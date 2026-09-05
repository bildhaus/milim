//! Outbound privacy gate for **remote** providers.
//!
//! The standalone `POST /privacy/scan` endpoint is on-demand only. This module
//! is the enforcement half: a runtime-settable gate that the [`ProviderRouter`]
//! consults *before* sending a request to a remote provider (local backends are
//! never scanned). In `Block` mode it fails closed — a request carrying PII is
//! refused. In `Redact` mode PII is replaced with reversible `[KIND_N]`
//! placeholders on the way out and restored in the streamed reply.
//!
//! [`ProviderRouter`]: crate::providers::ProviderRouter

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;

use milim_core::api::openai::{Content, ContentPart, DeltaFunction, DeltaToolCall, Model};
use milim_core::{Error, Result};
use milim_inference::{
    CompletionRequest, DeltaEvent, EventStream, ModelService, SharedService, StreamEvent,
};
use milim_privacy::{Detection, Redactor};

/// What the gate does to outbound requests bound for a remote provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PrivacyMode {
    /// No scanning — send verbatim (default).
    Off = 0,
    /// Replace PII with reversible placeholders; un-redact the reply.
    Redact = 1,
    /// Refuse the request if it contains any PII (fail-closed).
    Block = 2,
}

impl PrivacyMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "redact" => Self::Redact,
            "block" => Self::Block,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Redact => "redact",
            Self::Block => "block",
        }
    }
}

/// Shared, runtime-settable default privacy mode. The desktop syncs the active
/// UI setting via `POST /privacy/mode`; each new run snapshots that default
/// into its own fixed request-scoped gate.
pub struct PrivacyGate {
    mode: AtomicU8,
}

impl std::fmt::Debug for PrivacyGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivacyGate")
            .field("mode", &self.mode())
            .finish()
    }
}

impl Default for PrivacyGate {
    fn default() -> Self {
        Self {
            mode: AtomicU8::new(PrivacyMode::Off as u8),
        }
    }
}

impl PrivacyGate {
    pub fn from_env() -> Self {
        Self::default()
    }

    pub fn set(&self, mode: PrivacyMode) {
        self.mode.store(mode as u8, Ordering::Relaxed);
    }

    pub fn mode(&self) -> PrivacyMode {
        match self.mode.load(Ordering::Relaxed) {
            1 => PrivacyMode::Redact,
            2 => PrivacyMode::Block,
            _ => PrivacyMode::Off,
        }
    }

    pub fn scan_request(&self, req: &CompletionRequest) -> Vec<Detection> {
        scan_request(req)
    }

    pub fn redact_request(&self, req: &mut CompletionRequest) -> BTreeMap<String, String> {
        redact_request(req)
    }

    pub fn scan_text(&self, text: &str) -> Vec<Detection> {
        milim_privacy::scan(text)
    }

    pub fn redact_text(&self, text: &str) -> milim_privacy::Redaction {
        milim_privacy::redact(text)
    }

    pub fn is_clean_text(&self, text: &str) -> bool {
        milim_privacy::is_clean(text)
    }
}

/// Return a request-scoped view of a backend with a fixed privacy mode.
///
/// Local services remain untouched. Remote services are wrapped so callers
/// that do not use [`ProviderRouter`](crate::providers::ProviderRouter), such
/// as the standalone CLI fallback, receive the same outbound enforcement.
pub(crate) fn scoped_service(service: SharedService, mode: PrivacyMode) -> SharedService {
    if mode == PrivacyMode::Off || !service.requires_privacy_gate() {
        service
    } else {
        Arc::new(ScopedPrivacyService {
            inner: service,
            mode,
        })
    }
}

struct ScopedPrivacyService {
    inner: SharedService,
    mode: PrivacyMode,
}

#[async_trait]
impl ModelService for ScopedPrivacyService {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn list_models(&self) -> Result<Vec<Model>> {
        self.inner.list_models().await
    }

    async fn stream(&self, mut req: CompletionRequest) -> Result<EventStream> {
        match self.mode {
            PrivacyMode::Off => self.inner.stream(req).await,
            PrivacyMode::Block => {
                if request_has_image_parts(&req) {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate: outbound message contains image data, which the privacy gate cannot scan. Switch the gate to Off to send images to a remote provider.".to_string(),
                    ));
                }
                let detections = scan_request(&req);
                if detections.is_empty() {
                    self.inner.stream(req).await
                } else {
                    Err(Error::InvalidRequest(format!(
                        "blocked by the privacy gate: outbound message contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote provider.",
                        kinds_summary(&detections),
                        detections.len()
                    )))
                }
            }
            PrivacyMode::Redact => {
                if request_has_image_parts(&req) {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate: outbound message contains image data, which the privacy gate cannot redact. Switch the gate to Off to send images to a remote provider.".to_string(),
                    ));
                }
                let map = redact_request(&mut req);
                let inner = self.inner.stream(req).await?;
                Ok(if map.is_empty() {
                    inner
                } else {
                    unredact_stream(inner, map)
                })
            }
        }
    }

    async fn ollama_keep_alive(
        &self,
        model: &str,
        keep_alive: Option<serde_json::Value>,
    ) -> Result<bool> {
        self.inner.ollama_keep_alive(model, keep_alive).await
    }

    async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        match self.mode {
            PrivacyMode::Off => self.inner.embed(model, inputs).await,
            PrivacyMode::Block => {
                let detections = inputs
                    .iter()
                    .flat_map(|input| milim_privacy::scan(input))
                    .collect::<Vec<_>>();
                if detections.is_empty() {
                    self.inner.embed(model, inputs).await
                } else {
                    Err(Error::InvalidRequest(format!(
                        "blocked by the privacy gate: embedding input contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote provider.",
                        kinds_summary(&detections),
                        detections.len()
                    )))
                }
            }
            PrivacyMode::Redact => {
                let inputs = inputs
                    .into_iter()
                    .map(|input| milim_privacy::redact(&input).text)
                    .collect();
                self.inner.embed(model, inputs).await
            }
        }
    }
}

/// Scan the same text fields that Redact transforms, including structured tool
/// arguments and schema descriptions. Routing/model identifiers remain intact.
pub fn scan_request(req: &CompletionRequest) -> Vec<Detection> {
    let mut detections = Vec::new();
    let mut request = req.clone();
    transform_request_text(&mut request, &mut |text| {
        detections.extend(milim_privacy::scan(text));
        text.to_string()
    });
    detections
}

pub fn request_has_image_parts(req: &CompletionRequest) -> bool {
    req.messages.iter().any(|m| {
        matches!(
            &m.content,
            Some(Content::Parts(parts))
                if parts
                    .iter()
                    .any(|p| matches!(p, ContentPart::ImageUrl { .. }))
        )
    })
}

/// A short, de-duplicated human summary of detected kinds, e.g. `email, ip`.
pub fn kinds_summary(dets: &[Detection]) -> String {
    let mut kinds: Vec<String> = dets
        .iter()
        .filter_map(|d| {
            serde_json::to_value(d.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
        })
        .collect();
    kinds.sort();
    kinds.dedup();
    kinds.join(", ")
}

/// Redact all outbound text with one reversible map per request.
pub fn redact_request(req: &mut CompletionRequest) -> BTreeMap<String, String> {
    let mut redactor = Redactor::new();
    transform_request_text(req, &mut |text| redactor.redact(text));
    redactor.into_map()
}

fn transform_json_text(value: &mut serde_json::Value, transform: &mut impl FnMut(&str) -> String) {
    match value {
        serde_json::Value::String(text) => *text = transform(text),
        serde_json::Value::Array(values) => {
            for value in values {
                transform_json_text(value, transform);
            }
        }
        serde_json::Value::Object(values) => {
            // Argument dictionaries and schema property names can themselves
            // carry PII. Transform keys and values with the same map so schema
            // `required` entries and the eventual tool arguments stay aligned.
            *values = std::mem::take(values)
                .into_iter()
                .map(|(key, mut value)| {
                    let key = transform(&key);
                    transform_json_text(&mut value, transform);
                    (key, value)
                })
                .collect();
        }
        _ => {}
    }
}

fn transform_request_text(req: &mut CompletionRequest, transform: &mut impl FnMut(&str) -> String) {
    for message in &mut req.messages {
        match message.content.as_mut() {
            Some(Content::Text(text)) => *text = transform(text),
            Some(Content::Parts(parts)) => {
                for part in parts {
                    if let ContentPart::Text { text } = part {
                        *text = transform(text);
                    }
                }
            }
            None => {}
        }
        if let Some(text) = message.reasoning_content.as_mut() {
            *text = transform(text);
        }
        if let Some(calls) = message.tool_calls.as_mut() {
            for call in calls {
                let arguments = &mut call.function.arguments;
                if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(arguments) {
                    transform_json_text(&mut value, transform);
                    *arguments = value.to_string();
                } else {
                    // Preserve an upstream malformed payload for its normal validation,
                    // while still preventing raw detected text from crossing the gate.
                    *arguments = transform(arguments);
                }
            }
        }
    }
    for tool in &mut req.tools {
        if let Some(text) = tool.function.description.as_mut() {
            *text = transform(text);
        }
        if let Some(value) = tool.function.parameters.as_mut() {
            transform_json_text(value, transform);
        }
    }
    for value in [&mut req.response_format, &mut req.tool_choice]
        .into_iter()
        .flatten()
    {
        transform_json_text(value, transform);
    }
    for text in [&mut req.prompt, &mut req.suffix].into_iter().flatten() {
        *text = transform(text);
    }
    for text in &mut req.sampling.stop {
        *text = transform(text);
    }
}

/// Wrap a remote backend's reply stream so `[KIND_N]` placeholders are restored
/// to their originals. Buffers a trailing partial `[PLACEHOLDER` that might span
/// two deltas, flushing it at the terminal event.
pub fn unredact_stream(inner: EventStream, map: BTreeMap<String, String>) -> EventStream {
    Box::pin(async_stream::stream! {
        let mut inner = inner;
        let mut content = Unredactor::new(map.clone());
        let mut reasoning = Unredactor::new(map.clone());
        let mut arguments = BTreeMap::<u32, String>::new();
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(mut d)) => {
                    if let Some(c) = d.content.take() {
                        let out = content.push(&c);
                        d.content = (!out.is_empty()).then_some(out);
                    }
                    if let Some(rc) = d.reasoning.take() {
                        let out = reasoning.push(&rc);
                        d.reasoning = (!out.is_empty()).then_some(out);
                    }
                    // Tool argument fragments can split both placeholders and JSON
                    // escapes. Buffer by call index, then restore parsed string values.
                    for call in &mut d.tool_calls {
                        if let Some(fragment) = call.function.arguments.take() {
                            arguments.entry(call.index).or_default().push_str(&fragment);
                        }
                    }
                    yield Ok(StreamEvent::Delta(d));
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    let mut tool_calls = Vec::new();
                    for (index, raw) in std::mem::take(&mut arguments) {
                        let mut value = match serde_json::from_str::<serde_json::Value>(&raw) {
                            Ok(value) => value,
                            Err(error) => {
                                yield Err(Error::InvalidRequest(format!("Cannot restore private tool arguments: invalid JSON ({error})")));
                                return;
                            }
                        };
                        transform_json_text(&mut value, &mut |text| milim_privacy::unredact(text, &map));
                        tool_calls.push(DeltaToolCall {
                            index, id: None, kind: None,
                            function: DeltaFunction { name: None, arguments: Some(value.to_string()) },
                        });
                    }
                    let tail = content.flush();
                    let rtail = reasoning.flush();
                    if !tail.is_empty() || !rtail.is_empty() || !tool_calls.is_empty() {
                        yield Ok(StreamEvent::Delta(DeltaEvent {
                            content: (!tail.is_empty()).then_some(tail),
                            reasoning: (!rtail.is_empty()).then_some(rtail),
                            tool_calls,
                        }));
                    }
                    yield Ok(StreamEvent::Done { finish_reason, usage });
                    return;
                }
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
        // Consumers otherwise interpret EOF as a complete model step and could
        // execute already-emitted tool names with arguments still buffered here.
        yield Err(Error::Other("Provider stream ended before completion while restoring the private response.".into()));
    })
}

/// Buffered un-redactor: restores originals from the map, holding back any
/// trailing `[…` that has no closing `]` yet (it might complete in a later delta).
pub(crate) struct Unredactor {
    map: BTreeMap<String, String>,
    buf: String,
}

impl Unredactor {
    pub(crate) fn new(map: BTreeMap<String, String>) -> Self {
        Self {
            map,
            buf: String::new(),
        }
    }

    pub(crate) fn push(&mut self, s: &str) -> String {
        self.buf.push_str(s);
        // Emit everything up to the last unclosed '[' (a placeholder start that
        // might still be completing); keep that tail buffered.
        let cut = match self.buf.rfind('[') {
            Some(i) if !self.buf[i..].contains(']') => i,
            _ => self.buf.len(),
        };
        let head: String = self.buf.drain(..cut).collect();
        milim_privacy::unredact(&head, &self.map)
    }

    pub(crate) fn flush(&mut self) -> String {
        let out = milim_privacy::unredact(&self.buf, &self.map);
        self.buf.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::ChatMessage;

    #[test]
    fn unredactor_handles_split_placeholders() {
        let mut map = BTreeMap::new();
        map.insert("[EMAIL_1]".to_string(), "a@b.com".to_string());
        let mut u = Unredactor::new(map);
        // Placeholder split across three pushes is still restored once complete.
        let mut out = String::new();
        out.push_str(&u.push("contact [EMA"));
        out.push_str(&u.push("IL_1"));
        out.push_str(&u.push("] now"));
        out.push_str(&u.flush());
        assert_eq!(out, "contact a@b.com now");
    }

    #[test]
    fn block_mode_scan_finds_pii() {
        let mut req = CompletionRequest {
            model: "gpt".into(),
            messages: vec![ChatMessage::text("user", "email me at a@b.com")],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        };
        assert!(!scan_request(&req).is_empty());
        let map = redact_request(&mut req);
        assert!(
            scan_request(&req).is_empty(),
            "redacted request must be clean"
        );
        assert_eq!(map["[EMAIL_1]"], "a@b.com");
    }

    #[test]
    fn privacy_gate_uses_regex_only() {
        let gate = PrivacyGate::default();
        let req = CompletionRequest {
            model: "gpt".into(),
            messages: vec![ChatMessage::text("user", "Ada Lovelace")],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        };
        assert!(
            gate.scan_request(&req).is_empty(),
            "names are not regex PII"
        );
    }
    fn structured_request() -> CompletionRequest {
        use milim_core::api::openai::{FunctionCall, Tool, ToolCall, ToolFunction};
        let mut message = ChatMessage::text("assistant", "No personal information here");
        message.reasoning_content = Some("contact reasoning@example.com".into());
        message.tool_calls = Some(vec![ToolCall {
            id: Some("call-1".into()),
            kind: "function".into(),
            function: FunctionCall {
                name: "lookup".into(),
                arguments: r#"{"nested":[{"email":"person\u0040example.com"}]}"#.into(),
            },
        }]);
        CompletionRequest {
            model: "test".into(),
            messages: vec![message],
            tools: vec![Tool {
                kind: "function".into(),
                function: ToolFunction {
                    name: "lookup".into(),
                    description: Some("Ask owner@example.com".into()),
                    parameters: Some(
                        serde_json::json!({"type":"object","properties":{"email":{"type":"string","description":"example schema@example.com"}}}),
                    ),
                },
            }],
            tool_choice: None,
            response_format: None,
            prompt: Some("prompt@example.com".into()),
            suffix: Some("suffix@example.com".into()),
            sampling: Default::default(),
            reasoning_effort: None,
        }
    }

    #[test]
    fn structured_outbound_fields_are_scanned_and_redacted_as_valid_json() {
        let mut request = structured_request();
        assert_eq!(scan_request(&request).len(), 6);
        let map = redact_request(&mut request);
        assert!(scan_request(&request).is_empty());
        assert_eq!(map.len(), 6);
        let message = &request.messages[0];
        let arguments: serde_json::Value =
            serde_json::from_str(&message.tool_calls.as_ref().unwrap()[0].function.arguments)
                .unwrap();
        let placeholder = arguments["nested"][0]["email"].as_str().unwrap();
        assert_eq!(map[placeholder], "person@example.com");
        let wire = serde_json::to_value(message).unwrap();
        assert!(!wire.to_string().contains("example.com"));
        assert_eq!(
            message.tool_calls.as_ref().unwrap()[0].function.name,
            "lookup"
        );
    }

    #[tokio::test]
    async fn streamed_tool_arguments_restore_split_placeholders_and_json_escapes_by_call() {
        let map = BTreeMap::from([
            ("[EMAIL_1]".into(), "quoted\"\\\n@example.com".into()),
            ("[IP_1]".into(), "192.168.1.2".into()),
        ]);
        let fragment = |index, arguments: &str| {
            Ok(StreamEvent::Delta(DeltaEvent {
                tool_calls: vec![DeltaToolCall {
                    index,
                    id: None,
                    kind: None,
                    function: DeltaFunction {
                        name: None,
                        arguments: Some(arguments.into()),
                    },
                }],
                ..Default::default()
            }))
        };
        let events = vec![
            fragment(0, r#"{"email":"[EMA"#),
            fragment(1, r#"{"host":"[IP_"#),
            fragment(0, r#"IL_1]","literal":"a\\b"}"#),
            fragment(1, r#"1]"}"#),
            Ok(StreamEvent::Done {
                finish_reason: "tool_calls".into(),
                usage: Default::default(),
            }),
        ];
        let output = unredact_stream(Box::pin(futures::stream::iter(events)), map.clone())
            .collect::<Vec<_>>()
            .await;
        let mut calls = BTreeMap::new();
        for event in output {
            if let StreamEvent::Delta(delta) = event.unwrap() {
                for call in delta.tool_calls {
                    if let Some(arguments) = call.function.arguments {
                        assert!(calls
                            .insert(
                                call.index,
                                serde_json::from_str::<serde_json::Value>(&arguments).unwrap()
                            )
                            .is_none());
                    }
                }
            }
        }
        assert_eq!(calls[&0]["email"], map["[EMAIL_1]"]);
        assert_eq!(calls[&0]["literal"], "a\\b");
        assert_eq!(calls[&1]["host"], "192.168.1.2");
    }

    #[tokio::test]
    async fn malformed_private_tool_arguments_fail_before_terminal_dispatch() {
        let input = vec![
            Ok(StreamEvent::Delta(DeltaEvent {
                tool_calls: vec![DeltaToolCall {
                    index: 0,
                    id: None,
                    kind: None,
                    function: DeltaFunction {
                        name: Some("lookup".into()),
                        arguments: Some("{broken".into()),
                    },
                }],
                ..Default::default()
            })),
            Ok(StreamEvent::Done {
                finish_reason: "tool_calls".into(),
                usage: Default::default(),
            }),
        ];
        let output = unredact_stream(Box::pin(futures::stream::iter(input)), BTreeMap::new())
            .collect::<Vec<_>>()
            .await;
        assert!(output.last().unwrap().is_err());
        assert!(!output
            .iter()
            .any(|event| matches!(event, Ok(StreamEvent::Done { .. }))));
    }
    #[test]
    fn private_json_dictionary_keys_round_trip_with_values() {
        let original = serde_json::json!({"person@example.com": {"email": "person@example.com"}});
        let mut value = original.clone();
        let mut redactor = Redactor::new();
        transform_json_text(&mut value, &mut |text| redactor.redact(text));
        assert!(!value.to_string().contains("example.com"));
        let map = redactor.into_map();
        transform_json_text(&mut value, &mut |text| milim_privacy::unredact(text, &map));
        assert_eq!(value, original);
    }
    fn private_tool_fragment(arguments: Option<&str>) -> Result<StreamEvent> {
        Ok(StreamEvent::Delta(DeltaEvent {
            tool_calls: vec![DeltaToolCall {
                index: 0,
                id: Some("call-1".into()),
                kind: Some("function".into()),
                function: DeltaFunction {
                    name: Some("lookup".into()),
                    arguments: arguments.map(str::to_string),
                },
            }],
            ..Default::default()
        }))
    }

    #[tokio::test]
    async fn private_tool_stream_eof_fails_with_name_only_or_buffered_arguments() {
        for arguments in [None, Some(r#"{"email":"[EMAIL_1]"}"#)] {
            let input = vec![private_tool_fragment(arguments)];
            let output = unredact_stream(
                Box::pin(futures::stream::iter(input)),
                BTreeMap::from([("[EMAIL_1]".into(), "person@example.com".into())]),
            )
            .collect::<Vec<_>>()
            .await;
            assert!(output.last().unwrap().is_err());
            assert!(!output
                .iter()
                .any(|event| matches!(event, Ok(StreamEvent::Done { .. }))));
            assert!(!output.iter().any(|event| matches!(event, Ok(StreamEvent::Delta(delta)) if delta.tool_calls.iter().any(|call| call.function.arguments.is_some()))));
        }
    }

    #[tokio::test]
    async fn private_tool_stream_error_is_terminal_even_if_upstream_keeps_yielding() {
        let input = vec![
            private_tool_fragment(Some(r#"{"email":"[EMAIL_1]"}"#)),
            Err(Error::Other("upstream failed".into())),
            Ok(StreamEvent::Done {
                finish_reason: "tool_calls".into(),
                usage: Default::default(),
            }),
        ];
        let output = unredact_stream(
            Box::pin(futures::stream::iter(input)),
            BTreeMap::from([("[EMAIL_1]".into(), "person@example.com".into())]),
        )
        .collect::<Vec<_>>()
        .await;
        assert_eq!(output.len(), 2);
        assert!(output[1]
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("upstream failed"));
    }

    #[tokio::test]
    async fn private_stream_ignores_fragments_after_terminal_completion() {
        let input = vec![
            Ok(StreamEvent::Done {
                finish_reason: "stop".into(),
                usage: Default::default(),
            }),
            private_tool_fragment(Some("{}")),
        ];
        let output = unredact_stream(Box::pin(futures::stream::iter(input)), BTreeMap::new())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(output.len(), 1);
        assert!(matches!(output[0], Ok(StreamEvent::Done { .. })));
    }
}
