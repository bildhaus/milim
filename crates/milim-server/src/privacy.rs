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

use milim_core::api::openai::{ChatMessage, Content, ContentPart, Model};
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

/// All PII detections across every message's visible text (for `Block` mode).
pub fn scan_request(req: &CompletionRequest) -> Vec<Detection> {
    req.messages
        .iter()
        .flat_map(|m| milim_privacy::scan(&m.text_content()))
        .collect()
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

/// Redact PII in every message in place, returning the reversal map (empty if
/// nothing was redacted).
pub fn redact_request(req: &mut CompletionRequest) -> BTreeMap<String, String> {
    let mut r = Redactor::new();
    for m in &mut req.messages {
        redact_message(m, &mut r);
    }
    r.into_map()
}

fn redact_message(m: &mut ChatMessage, r: &mut Redactor) {
    match m.content.as_mut() {
        Some(Content::Text(t)) => *t = r.redact(t),
        Some(Content::Parts(parts)) => {
            for p in parts.iter_mut() {
                if let ContentPart::Text { text } = p {
                    *text = r.redact(text);
                }
            }
        }
        None => {}
    }
}

/// Wrap a remote backend's reply stream so `[KIND_N]` placeholders are restored
/// to their originals. Buffers a trailing partial `[PLACEHOLDER` that might span
/// two deltas, flushing it at the terminal event.
pub fn unredact_stream(inner: EventStream, map: BTreeMap<String, String>) -> EventStream {
    Box::pin(async_stream::stream! {
        let mut inner = inner;
        let mut content = Unredactor::new(map.clone());
        let mut reasoning = Unredactor::new(map);
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
                    yield Ok(StreamEvent::Delta(d));
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    let tail = content.flush();
                    let rtail = reasoning.flush();
                    if !tail.is_empty() || !rtail.is_empty() {
                        yield Ok(StreamEvent::Delta(DeltaEvent {
                            content: (!tail.is_empty()).then_some(tail),
                            reasoning: (!rtail.is_empty()).then_some(rtail),
                            ..Default::default()
                        }));
                    }
                    yield Ok(StreamEvent::Done { finish_reason, usage });
                }
                Err(e) => yield Err(e),
            }
        }
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
}
