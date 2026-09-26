//! Anthropic Messages API upstream backend.
//!
//! Translates the backend-neutral [`CompletionRequest`] into Anthropic's
//! `/v1/messages` format and maps Anthropic named SSE events back into the
//! neutral [`StreamEvent`] shape used by the rest of milim.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, DeltaFunction, DeltaToolCall, Model, ReasoningEffort, Tool,
    Usage,
};
use milim_core::provider_error::upstream_stream_error;
use milim_core::{Error, Result};

use crate::http_error::{http_status_error, stream_read_error, HttpFailure};
use crate::service::{
    normalize_finish_reason, CompletionRequest, DeltaEvent, EventStream, ModelService, StreamEvent,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Highest output budget sent when the request leaves `max_tokens` unset.
/// Model caps above it (128K on current Opus and Sonnet models) are not used
/// by default so one response cannot consume a whole output rate-limit window.
const DEFAULT_OUTPUT_CEILING: u32 = 32_000;

#[cfg(not(test))]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(50);

#[cfg(not(test))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Forwards generation to an Anthropic Messages-compatible endpoint.
#[derive(Debug, Clone)]
pub struct AnthropicBackend {
    label: String,
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
    /// Per-model output caps from the provider catalog (the Models API
    /// `max_tokens` field), keyed by model id.
    output_limits: Arc<RwLock<HashMap<String, u32>>>,
}

impl AnthropicBackend {
    /// Build a backend pointing at `base_url` (usually
    /// `https://api.anthropic.com/v1`) with an optional API key.
    pub fn new(
        label: impl Into<String>,
        base_url: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            label: label.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            client: default_client(),
            output_limits: Arc::default(),
        }
    }

    /// Seed per-model output token caps already recorded in the provider's
    /// model catalog, so the default output budget never exceeds them.
    pub fn with_model_output_limits(self, limits: impl IntoIterator<Item = (String, u32)>) -> Self {
        self.record_output_limits(limits);
        self
    }

    fn record_output_limits(&self, limits: impl IntoIterator<Item = (String, u32)>) {
        if let Ok(mut map) = self.output_limits.write() {
            map.extend(limits.into_iter().filter(|(_, limit)| *limit > 0));
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let rb = rb.header("anthropic-version", ANTHROPIC_VERSION);
        match &self.api_key {
            Some(k) if !k.is_empty() => rb.header("x-api-key", k),
            _ => rb,
        }
    }

    /// The output budget for a request: the caller's `max_tokens`, or a
    /// model-aware default, never above a cap the catalog or an earlier
    /// rejection reported for this model.
    fn max_tokens_for(&self, req: &CompletionRequest) -> u32 {
        let catalog = self
            .output_limits
            .read()
            .ok()
            .and_then(|limits| limits.get(&req.model).copied());
        let requested = req.sampling.max_tokens.unwrap_or_else(|| {
            catalog
                .map(|limit| limit.min(DEFAULT_OUTPUT_CEILING))
                .unwrap_or_else(|| default_output_tokens(&req.model))
        });
        [catalog, learned_output_limit(&self.base_url, &req.model)]
            .into_iter()
            .flatten()
            .fold(requested, u32::min)
    }

    fn build_body(&self, req: &CompletionRequest) -> Value {
        let (mut system, messages) = anthropic_messages(&req.messages);

        let mut body = json!({
            "model": req.model,
            "max_tokens": self.max_tokens_for(req),
            "messages": messages,
            "stream": true,
        });

        if let Some(last) = system.last_mut() {
            set_cache_breakpoint(last);
            body["system"] = Value::Array(system);
        }
        if let Some(t) = req.sampling.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(t) = req.sampling.top_p {
            body["top_p"] = json!(t);
        }
        if !req.sampling.stop.is_empty() {
            body["stop_sequences"] = json!(req.sampling.stop);
        }
        if !req.tools.is_empty() {
            let mut tools = anthropic_tools(&req.tools);
            if let Some(last) = tools.last_mut() {
                set_cache_breakpoint(last);
            }
            body["tools"] = Value::Array(tools);
        }
        if let Some(choice) = anthropic_tool_choice(req.tool_choice.as_ref()) {
            body["tool_choice"] = choice;
        }
        if let Some(effort) = anthropic_reasoning_effort(req.reasoning_effort, &req.model) {
            body["output_config"] = json!({ "effort": effort });
        }

        body
    }

    async fn send_messages(&self, body: &Value) -> Result<reqwest::Response> {
        self.auth(self.client.post(self.endpoint("messages")))
            .json(body)
            .send()
            .await
            .map_err(upstream)
    }
}

/// Output budget for a model with no catalog cap: a conservative table by
/// model family, well inside each family's real limit.
fn default_output_tokens(model: &str) -> u32 {
    let id = model.to_ascii_lowercase();
    if id.contains("claude-3-opus")
        || id.contains("claude-3-sonnet")
        || id.contains("claude-3-haiku")
    {
        4_096
    } else if id.contains("claude-3-5") {
        8_192
    } else if id.contains("haiku") {
        if id.contains("haiku-4") {
            DEFAULT_OUTPUT_CEILING
        } else {
            8_192
        }
    } else if id.contains("opus")
        || id.contains("sonnet")
        || id.contains("fable")
        || id.contains("mythos")
    {
        DEFAULT_OUTPUT_CEILING
    } else {
        16_000
    }
}

/// Output caps learned from `max_tokens` rejections, shared by every backend
/// in the process and keyed by endpoint and model.
fn learned_output_limits() -> &'static Mutex<HashMap<String, u32>> {
    static LIMITS: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    LIMITS.get_or_init(Mutex::default)
}

fn learned_output_limit(base_url: &str, model: &str) -> Option<u32> {
    learned_output_limits()
        .lock()
        .ok()?
        .get(&format!("{base_url} {model}"))
        .copied()
}

fn remember_output_limit(base_url: &str, model: &str, limit: u32) {
    if let Ok(mut limits) = learned_output_limits().lock() {
        limits.insert(format!("{base_url} {model}"), limit);
    }
}

/// Why Anthropic rejected a request's `max_tokens`, with the value to retry.
#[derive(Debug, PartialEq, Eq)]
enum MaxTokensRejection {
    /// `max_tokens: 64000 > 32000, which is the maximum allowed number of
    /// output tokens for <model>` - a per-model cap worth remembering.
    ModelLimit(u32),
    /// `input length and max_tokens exceed context limit: 190000 + 32000 >
    /// 200000` - only this request's remaining context.
    ContextRemaining(u32),
}

fn max_tokens_rejection(body: &str) -> Option<MaxTokensRejection> {
    let text = body.to_ascii_lowercase();
    let at = text.find("max_tokens")?;
    let rest = &text[at..];
    let leading_number = |s: &str| -> Option<u32> {
        let digits: String = s
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    };
    if let Some(limit_at) = rest.find("context limit:") {
        let expr = &rest[limit_at + "context limit:".len()..];
        let input = leading_number(expr)?;
        let (_, limit) = expr.split_once('>')?;
        let remaining = leading_number(limit)?.checked_sub(input)?;
        return (remaining > 0).then_some(MaxTokensRejection::ContextRemaining(remaining));
    }
    if !rest.contains("maximum") {
        return None;
    }
    let (_, limit) = rest.split_once('>')?;
    leading_number(limit)
        .filter(|limit| *limit > 0)
        .map(MaxTokensRejection::ModelLimit)
}

fn default_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
        .read_timeout(DEFAULT_READ_TIMEOUT)
        .build()
        .expect("valid reqwest client timeout configuration")
}

#[async_trait]
impl ModelService for AnthropicBackend {
    fn name(&self) -> &str {
        &self.label
    }

    fn requires_privacy_gate(&self) -> bool {
        true
    }

    async fn list_models(&self) -> Result<Vec<Model>> {
        let resp = self
            .auth(self.client.get(self.endpoint("models")))
            .send()
            .await
            .map_err(upstream)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "{} GET /models -> {}",
                self.label,
                resp.status()
            )));
        }

        let parsed: AnthropicModelsResponse = resp.json().await.map_err(upstream)?;
        self.record_output_limits(
            parsed
                .data
                .iter()
                .filter_map(|m| Some((m.id.clone(), m.max_tokens?))),
        );
        Ok(parsed
            .data
            .into_iter()
            .map(|m| Model {
                id: m.id,
                object: "model".to_string(),
                created: 0,
                owned_by: self.label.clone(),
                provider_id: None,
                context_length: m.max_input_tokens,
                max_prompt_tokens: m.max_input_tokens,
                max_completion_tokens: m.max_tokens,
                pricing: None,
                reasoning: None,
                capabilities: None,
                architecture: None,
            })
            .collect())
    }

    async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
        crate::image_input::validate_request_images(&req)?;
        let mut body = self.build_body(&req);
        let mut resp = self.send_messages(&body).await?;

        // A `max_tokens` above what the model or the remaining context allows
        // is retried once with the allowed value.
        if resp.status() == reqwest::StatusCode::BAD_REQUEST {
            let failure = HttpFailure::read(resp).await;
            let sent = body["max_tokens"].as_u64().unwrap_or(0);
            let retry_with = match max_tokens_rejection(&failure.body) {
                Some(MaxTokensRejection::ModelLimit(limit)) if u64::from(limit) < sent => {
                    remember_output_limit(&self.base_url, &req.model, limit);
                    Some(limit)
                }
                Some(MaxTokensRejection::ContextRemaining(limit)) if u64::from(limit) < sent => {
                    Some(limit)
                }
                _ => None,
            };
            let Some(limit) = retry_with else {
                return Err(failure.into_error(&self.label, "messages"));
            };
            body["max_tokens"] = json!(limit);
            resp = self.send_messages(&body).await?;
        }

        if !resp.status().is_success() {
            return Err(http_status_error(&self.label, "messages", resp).await);
        }

        let label = self.label.clone();
        let stream = async_stream::stream! {
            let mut bytes = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = AnthropicStreamState::default();

            'outer: while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(stream_read_error(&label, e));
                        return;
                    }
                };
                buf.extend_from_slice(&chunk);

                while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                    let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                    let line = String::from_utf8_lossy(&line_bytes);
                    match parse_sse_line(line.trim_end(), &mut state) {
                        AnthropicLine::Delta(d) => {
                            if !d.is_empty() {
                                yield Ok(StreamEvent::Delta(d));
                            }
                        }
                        AnthropicLine::Done => break 'outer,
                        AnthropicLine::Error { kind, message } => {
                            yield Err(upstream_stream_error(
                                &label,
                                "messages",
                                None,
                                kind.as_deref(),
                                &message,
                            ));
                            return;
                        }
                        AnthropicLine::Ignore => {}
                    }
                }
            }

            yield Ok(StreamEvent::Done {
                finish_reason: normalize_finish_reason(state.stop_reason.as_deref()).to_string(),
                usage: state.usage(),
            });
        };

        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, _inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        Err(Error::InvalidRequest(
            "Anthropic does not expose an embeddings endpoint for this provider".to_string(),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    #[serde(default)]
    data: Vec<AnthropicModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicModel {
    id: String,
    #[serde(default)]
    max_input_tokens: Option<u32>,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Debug, Default)]
struct AnthropicStreamState {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    stop_reason: Option<String>,
}

impl AnthropicStreamState {
    /// Fold a `usage` object from `message_start` or `message_delta`; fields
    /// a later event omits keep their earlier value.
    fn record_usage(&mut self, usage: &Value) {
        for (key, slot) in [
            ("input_tokens", &mut self.input_tokens),
            ("output_tokens", &mut self.output_tokens),
            ("cache_read_input_tokens", &mut self.cache_read_tokens),
            ("cache_creation_input_tokens", &mut self.cache_write_tokens),
        ] {
            if let Some(value) = opt_u32(usage, key) {
                *slot = value;
            }
        }
    }

    /// Anthropic's `input_tokens` excludes cached tokens; the neutral
    /// `prompt_tokens` includes them so totals compare across providers.
    fn usage(&self) -> Usage {
        let prompt = self.input_tokens + self.cache_read_tokens + self.cache_write_tokens;
        Usage {
            cache_read_tokens: (self.cache_read_tokens > 0).then_some(self.cache_read_tokens),
            cache_write_tokens: (self.cache_write_tokens > 0).then_some(self.cache_write_tokens),
            ..Usage::new(prompt, self.output_tokens)
        }
    }
}

enum AnthropicLine {
    Delta(DeltaEvent),
    Done,
    Error {
        kind: Option<String>,
        message: String,
    },
    Ignore,
}

fn parse_sse_line(line: &str, state: &mut AnthropicStreamState) -> AnthropicLine {
    let Some(data) = line.strip_prefix("data:") else {
        return AnthropicLine::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return AnthropicLine::Ignore;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return AnthropicLine::Ignore;
    };
    match v.get("type").and_then(Value::as_str) {
        Some("message_start") => {
            if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                state.record_usage(usage);
            }
            AnthropicLine::Ignore
        }
        Some("content_block_start") => {
            let Some(block) = v.get("content_block") else {
                return AnthropicLine::Ignore;
            };
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return AnthropicLine::Ignore;
            }
            let index = opt_u32(&v, "index").unwrap_or(0);
            let mut delta = DeltaEvent::default();
            delta.tool_calls.push(DeltaToolCall {
                index,
                id: block.get("id").and_then(Value::as_str).map(str::to_string),
                kind: Some("function".to_string()),
                function: DeltaFunction {
                    name: block
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    arguments: block.get("input").and_then(non_empty_json_string),
                },
            });
            AnthropicLine::Delta(delta)
        }
        Some("content_block_delta") => {
            let Some(delta_v) = v.get("delta") else {
                return AnthropicLine::Ignore;
            };
            match delta_v.get("type").and_then(Value::as_str) {
                Some("text_delta") => AnthropicLine::Delta(DeltaEvent {
                    content: delta_v
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    ..Default::default()
                }),
                Some("thinking_delta") => AnthropicLine::Delta(DeltaEvent {
                    reasoning: delta_v
                        .get("thinking")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    ..Default::default()
                }),
                Some("input_json_delta") => {
                    let mut delta = DeltaEvent::default();
                    delta.tool_calls.push(DeltaToolCall {
                        index: opt_u32(&v, "index").unwrap_or(0),
                        id: None,
                        kind: None,
                        function: DeltaFunction {
                            name: None,
                            arguments: delta_v
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        },
                    });
                    AnthropicLine::Delta(delta)
                }
                _ => AnthropicLine::Ignore,
            }
        }
        Some("message_delta") => {
            if let Some(reason) = v
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
            {
                state.stop_reason = Some(reason.to_string());
            }
            if let Some(usage) = v.get("usage") {
                state.record_usage(usage);
            }
            AnthropicLine::Ignore
        }
        Some("message_stop") => AnthropicLine::Done,
        Some("error") => {
            let error = v.get("error");
            let field = |key: &str| {
                error
                    .and_then(|e| e.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            AnthropicLine::Error {
                kind: field("type"),
                message: field("message").unwrap_or_default(),
            }
        }
        _ => AnthropicLine::Ignore,
    }
}

/// Translate neutral messages into Anthropic's top-level `system` blocks and
/// alternating `messages`.
///
/// Only the leading run of system messages becomes `system`, one text block
/// each so the prefix can be cached. A later system message becomes user text
/// wrapped in `<system-reminder>` at its original position. Consecutive
/// same-role turns are merged, with `tool_result` blocks kept ahead of other
/// user content as Anthropic requires. The last block of the newest user turn
/// carries a cache breakpoint so each agent step reuses the prefix before it.
fn anthropic_messages(messages: &[ChatMessage]) -> (Vec<Value>, Vec<Value>) {
    let leading = messages.iter().take_while(|m| m.role == "system").count();
    let system = messages[..leading]
        .iter()
        .map(ChatMessage::text_content)
        .filter(|text| !text.trim().is_empty())
        .map(|text| json!({ "type": "text", "text": text }))
        .collect();

    let mut turns: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for msg in &messages[leading..] {
        let (role, blocks) = if msg.role == "system" {
            let text = msg.text_content();
            if text.trim().is_empty() {
                continue;
            }
            (
                "user",
                vec![json!({ "type": "text", "text": system_reminder(&text) })],
            )
        } else {
            message_blocks(msg)
        };
        if blocks.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((last_role, last_blocks)) if *last_role == role => {
                last_blocks.extend(blocks);
                if role == "user" {
                    last_blocks.sort_by_key(|block| block_type(block) != Some("tool_result"));
                }
            }
            _ => turns.push((role, blocks)),
        }
    }

    if let Some((_, blocks)) = turns.iter_mut().rev().find(|(role, _)| *role == "user") {
        if let Some(block) = blocks.iter_mut().rev().find(|b| !is_empty_block(b)) {
            set_cache_breakpoint(block);
        }
    }

    let messages = turns
        .into_iter()
        .map(|(role, blocks)| json!({ "role": role, "content": turn_content(blocks) }))
        .collect();
    (system, messages)
}

fn system_reminder(text: &str) -> String {
    format!("<system-reminder>\n{}\n</system-reminder>", text.trim())
}

fn set_cache_breakpoint(block: &mut Value) {
    block["cache_control"] = json!({ "type": "ephemeral" });
}

fn block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

fn is_empty_block(block: &Value) -> bool {
    match block_type(block) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(|text| text.trim().is_empty()),
        Some("tool_result") => match block.get("content") {
            Some(Value::String(text)) => text.trim().is_empty(),
            Some(Value::Array(parts)) => parts.is_empty(),
            _ => true,
        },
        _ => false,
    }
}

/// A lone plain text block is sent as a string; anything else as blocks.
fn turn_content(mut blocks: Vec<Value>) -> Value {
    let plain_text = blocks.len() == 1
        && block_type(&blocks[0]) == Some("text")
        && blocks[0].as_object().is_some_and(|block| block.len() == 2);
    if plain_text {
        blocks.swap_remove(0)["text"].take()
    } else {
        Value::Array(blocks)
    }
}

fn message_blocks(msg: &ChatMessage) -> (&'static str, Vec<Value>) {
    if msg.role == "tool" {
        return (
            "user",
            vec![json!({
                "type": "tool_result",
                "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                "content": msg.text_content()
            })],
        );
    }

    let role = if msg.role == "assistant" {
        "assistant"
    } else {
        "user"
    };
    let mut blocks = content_blocks(msg);
    if let Some(calls) = &msg.tool_calls {
        for call in calls {
            let input = serde_json::from_str::<Value>(&call.function.arguments)
                .unwrap_or_else(|_| Value::Object(Default::default()));
            blocks.push(json!({
                "type": "tool_use",
                "id": call.id.clone().unwrap_or_default(),
                "name": call.function.name,
                "input": input
            }));
        }
    }

    (role, blocks)
}

fn content_blocks(msg: &ChatMessage) -> Vec<Value> {
    match &msg.content {
        Some(Content::Text(text)) if !text.is_empty() => {
            vec![json!({ "type": "text", "text": text })]
        }
        Some(Content::Parts(parts)) => parts.iter().filter_map(part_to_anthropic).collect(),
        _ => Vec::new(),
    }
}

fn part_to_anthropic(part: &ContentPart) -> Option<Value> {
    match part {
        ContentPart::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentPart::ImageUrl { image_url } => {
            let url = &image_url.url;
            if let Some((media_type, data)) = parse_data_url(url) {
                Some(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": media_type,
                        "data": data
                    }
                }))
            } else {
                Some(json!({
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": url
                    }
                }))
            }
        }
        _ => None,
    }
}

fn parse_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    Some((media_type, data))
}

fn anthropic_tools(tools: &[Tool]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            let mut tool = serde_json::Map::new();
            tool.insert("name".to_string(), Value::String(t.function.name.clone()));
            if let Some(description) = &t.function.description {
                tool.insert(
                    "description".to_string(),
                    Value::String(description.clone()),
                );
            }
            tool.insert(
                "input_schema".to_string(),
                t.function
                    .parameters
                    .clone()
                    .unwrap_or_else(|| json!({"type":"object"})),
            );
            Value::Object(tool)
        })
        .collect()
}

fn anthropic_tool_choice(choice: Option<&Value>) -> Option<Value> {
    let choice = choice?;
    match choice.get("type").and_then(Value::as_str) {
        Some("function") => choice
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map(|name| json!({ "type": "tool", "name": name })),
        Some("required") => Some(json!({ "type": "any" })),
        Some("none") => None,
        Some("auto" | "any" | "tool") => Some(choice.clone()),
        _ => Some(choice.clone()),
    }
}

fn anthropic_reasoning_effort(
    effort: Option<ReasoningEffort>,
    model: &str,
) -> Option<&'static str> {
    if !anthropic_supports_effort(model) {
        return None;
    }
    match effort? {
        ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
        ReasoningEffort::Xhigh => Some("xhigh"),
        ReasoningEffort::Max => Some("max"),
        ReasoningEffort::Auto
        | ReasoningEffort::None
        | ReasoningEffort::Minimal
        | ReasoningEffort::On => None,
    }
}

fn anthropic_supports_effort(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.contains("claude-4") || id.contains("claude-sonnet-4") || id.contains("claude-opus-4")
}

fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).map(|n| n as u32)
}

fn non_empty_json_string(v: &Value) -> Option<String> {
    if v.is_object() && v.as_object().is_some_and(|o| o.is_empty()) {
        None
    } else {
        Some(v.to_string())
    }
}

fn upstream(e: impl std::fmt::Display) -> Error {
    Error::Upstream(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::{FunctionCall, ToolCall, ToolFunction};
    use milim_core::provider_error::retry_hint;

    fn req(model: &str, messages: Vec<ChatMessage>) -> CompletionRequest {
        CompletionRequest {
            model: model.to_string(),
            messages,
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        }
    }

    fn tool_result(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            tool_call_id: Some(id.to_string()),
            ..ChatMessage::text("tool", text)
        }
    }

    fn assistant_calls(ids: &[&str]) -> ChatMessage {
        ChatMessage {
            content: None,
            tool_calls: Some(
                ids.iter()
                    .map(|id| ToolCall {
                        id: Some(id.to_string()),
                        kind: "function".to_string(),
                        function: FunctionCall {
                            name: "read".to_string(),
                            arguments: "{}".to_string(),
                        },
                    })
                    .collect(),
            ),
            ..ChatMessage::text("assistant", "")
        }
    }

    #[test]
    fn default_output_budget_follows_model_family() {
        assert_eq!(default_output_tokens("claude-opus-4-8"), 32_000);
        assert_eq!(default_output_tokens("claude-sonnet-5"), 32_000);
        assert_eq!(default_output_tokens("claude-haiku-4-5"), 32_000);
        assert_eq!(default_output_tokens("claude-3-5-haiku-latest"), 8_192);
        assert_eq!(default_output_tokens("claude-3-haiku-20240307"), 4_096);
        assert_eq!(default_output_tokens("some-compatible-model"), 16_000);
    }

    #[test]
    fn output_budget_respects_request_catalog_and_learned_caps() {
        let backend = AnthropicBackend::new("anthropic", "http://budget.test/v1", None)
            .with_model_output_limits([
                ("claude-opus-5".to_string(), 128_000),
                ("tiny".to_string(), 2_048),
            ]);
        let mut request = req("claude-opus-5", vec![ChatMessage::text("user", "hi")]);
        assert_eq!(backend.build_body(&request)["max_tokens"], 32_000);

        request.model = "tiny".to_string();
        assert_eq!(backend.build_body(&request)["max_tokens"], 2_048);
        request.sampling.max_tokens = Some(4_000);
        assert_eq!(backend.build_body(&request)["max_tokens"], 2_048);

        request.model = "claude-sonnet-4-6".to_string();
        assert_eq!(backend.build_body(&request)["max_tokens"], 4_000);
        remember_output_limit("http://budget.test/v1", "claude-sonnet-4-6", 1_000);
        assert_eq!(backend.build_body(&request)["max_tokens"], 1_000);
    }

    #[test]
    fn parses_max_tokens_rejections() {
        assert_eq!(
            max_tokens_rejection(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"max_tokens: 64000 > 32000, which is the maximum allowed number of output tokens for claude-opus-4-20250514"}}"#
            ),
            Some(MaxTokensRejection::ModelLimit(32_000))
        );
        assert_eq!(
            max_tokens_rejection(
                "input length and `max_tokens` exceed context limit: 190000 + 32000 > 200000, decrease input length or `max_tokens` and try again"
            ),
            Some(MaxTokensRejection::ContextRemaining(10_000))
        );
        assert_eq!(
            max_tokens_rejection("`max_tokens` must be greater than `thinking.budget_tokens`"),
            None
        );
        assert_eq!(
            max_tokens_rejection("messages: text content blocks must be non-empty"),
            None
        );
    }

    #[test]
    fn keeps_only_leading_system_messages_as_cached_system_blocks() {
        let messages = vec![
            ChatMessage::text("system", "Base prompt."),
            ChatMessage::text("system", "Project rules."),
            ChatMessage::text("user", "Fix the bug."),
            ChatMessage::text("assistant", "Looking."),
            ChatMessage::text("system", "Plan mode is off."),
            ChatMessage::text("user", "Go ahead."),
        ];
        let backend = AnthropicBackend::new("anthropic", "https://api.anthropic.com/v1", None);
        let body = backend.build_body(&req("claude-opus-5", messages));

        assert_eq!(
            body["system"],
            json!([
                { "type": "text", "text": "Base prompt." },
                { "type": "text", "text": "Project rules.", "cache_control": { "type": "ephemeral" } }
            ])
        );
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(
            messages[0],
            json!({ "role": "user", "content": "Fix the bug." })
        );
        assert_eq!(
            messages[1],
            json!({ "role": "assistant", "content": "Looking." })
        );
        assert_eq!(
            messages[2],
            json!({
                "role": "user",
                "content": [
                    { "type": "text", "text": "<system-reminder>\nPlan mode is off.\n</system-reminder>" },
                    { "type": "text", "text": "Go ahead.", "cache_control": { "type": "ephemeral" } }
                ]
            })
        );
    }

    #[test]
    fn merges_tool_results_and_reminders_into_one_user_turn() {
        let messages = vec![
            ChatMessage::text("user", "Read both files."),
            assistant_calls(&["call_a", "call_b"]),
            tool_result("call_a", "alpha"),
            ChatMessage::text("system", "Context is 80% full."),
            tool_result("call_b", ""),
        ];
        let (system, messages) = anthropic_messages(&messages);
        assert!(system.is_empty());
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"][1]["id"], "call_b");

        let turn = &messages[2];
        assert_eq!(turn["role"], "user");
        let blocks = turn["content"].as_array().unwrap();
        let kinds: Vec<_> = blocks.iter().map(|b| b["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["tool_result", "tool_result", "text"]);
        assert_eq!(blocks[0]["tool_use_id"], "call_a");
        assert_eq!(blocks[1]["tool_use_id"], "call_b");
        // The empty tool result is skipped; the breakpoint lands on the last
        // non-empty block of the newest user turn, and nowhere else.
        assert_eq!(blocks[2]["cache_control"]["type"], "ephemeral");
        assert!(blocks[1].get("cache_control").is_none());
        assert_eq!(
            serde_json::to_string(&messages)
                .unwrap()
                .matches("cache_control")
                .count(),
            1
        );
    }

    #[test]
    fn caches_last_tool_definition_within_breakpoint_limit() {
        let tool = |name: &str| Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: name.to_string(),
                description: None,
                parameters: None,
            },
        };
        let mut request = req(
            "claude-opus-5",
            vec![
                ChatMessage::text("system", "Base."),
                ChatMessage::text("user", "Hi"),
            ],
        );
        request.tools = vec![tool("read"), tool("write")];
        let body =
            AnthropicBackend::new("a", "https://api.anthropic.com/v1", None).build_body(&request);
        assert!(body["tools"][0].get("cache_control").is_none());
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
        assert_eq!(body.to_string().matches("cache_control").count(), 3);
    }

    #[test]
    fn usage_includes_cached_prompt_tokens() {
        let mut state = AnthropicStreamState::default();
        parse_sse_line(
            r#"data: {"type":"message_start","message":{"usage":{"input_tokens":20,"cache_read_input_tokens":1000,"cache_creation_input_tokens":300,"output_tokens":1}}}"#,
            &mut state,
        );
        parse_sse_line(
            r#"data: {"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":50}}"#,
            &mut state,
        );
        let usage = state.usage();
        assert_eq!(usage.prompt_tokens, 1_320);
        assert_eq!(usage.completion_tokens, 50);
        assert_eq!(usage.total_tokens, 1_370);
        assert_eq!(usage.cache_read_tokens, Some(1_000));
        assert_eq!(usage.cache_write_tokens, Some(300));
        assert_eq!(
            normalize_finish_reason(state.stop_reason.as_deref()),
            "length"
        );
    }

    #[test]
    fn stream_error_events_become_retryable_upstream_errors() {
        let mut state = AnthropicStreamState::default();
        let line = parse_sse_line(
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            &mut state,
        );
        let AnthropicLine::Error { kind, message } = line else {
            panic!("expected an error line");
        };
        let error = upstream_stream_error("anthropic", "messages", None, kind.as_deref(), &message);
        let hint = retry_hint(&error).unwrap();
        assert!(hint.retryable);
    }
}
