//! Anthropic Messages API upstream backend.
//!
//! Translates the backend-neutral [`CompletionRequest`] into Anthropic's
//! `/v1/messages` format and maps Anthropic named SSE events back into the
//! neutral [`StreamEvent`] shape used by the rest of milim.

use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, DeltaFunction, DeltaToolCall, Model, ReasoningEffort, Tool,
    Usage,
};
use milim_core::{Error, Result};

use crate::service::{CompletionRequest, DeltaEvent, EventStream, ModelService, StreamEvent};

const ANTHROPIC_VERSION: &str = "2023-06-01";

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

    fn build_body(&self, req: &CompletionRequest) -> Value {
        let system = system_text(&req.messages);
        let messages = req
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(message_to_anthropic)
            .collect::<Vec<_>>();

        let mut body = json!({
            "model": req.model,
            "max_tokens": req.sampling.max_tokens.unwrap_or(1024),
            "messages": messages,
            "stream": true,
        });

        if let Some(system) = system {
            body["system"] = Value::String(system);
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
            body["tools"] = json!(anthropic_tools(&req.tools));
        }
        if let Some(choice) = anthropic_tool_choice(req.tool_choice.as_ref()) {
            body["tool_choice"] = choice;
        }
        if let Some(effort) = anthropic_reasoning_effort(req.reasoning_effort, &req.model) {
            body["output_config"] = json!({ "effort": effort });
        }

        body
    }
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
        Ok(parsed
            .data
            .into_iter()
            .map(|m| Model {
                id: m.id,
                object: "model".to_string(),
                created: 0,
                owned_by: self.label.clone(),
                provider_id: None,
                context_length: None,
                max_prompt_tokens: None,
                max_completion_tokens: None,
                pricing: None,
                reasoning: None,
                capabilities: None,
                architecture: None,
            })
            .collect())
    }

    async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
        crate::image_input::validate_request_images(&req)?;
        let body = self.build_body(&req);
        let resp = self
            .auth(self.client.post(self.endpoint("messages")))
            .json(&body)
            .send()
            .await
            .map_err(upstream)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "{} messages -> {status}: {text}",
                self.label
            )));
        }

        let stream = async_stream::stream! {
            let mut bytes = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = AnthropicStreamState::default();

            'outer: while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(b) => b,
                    Err(e) => {
                        yield Err(upstream(e));
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
                        AnthropicLine::Error(e) => {
                            yield Err(Error::Upstream(e));
                            return;
                        }
                        AnthropicLine::Ignore => {}
                    }
                }
            }

            yield Ok(StreamEvent::Done {
                finish_reason: anthropic_stop_to_openai(&state.stop_reason),
                usage: Usage::new(state.input_tokens, state.output_tokens),
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
}

#[derive(Debug, Default)]
struct AnthropicStreamState {
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: Option<String>,
}

enum AnthropicLine {
    Delta(DeltaEvent),
    Done,
    Error(String),
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
                state.input_tokens = opt_u32(usage, "input_tokens").unwrap_or(0);
                state.output_tokens = opt_u32(usage, "output_tokens").unwrap_or(0);
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
                state.output_tokens =
                    opt_u32(usage, "output_tokens").unwrap_or(state.output_tokens);
            }
            AnthropicLine::Ignore
        }
        Some("message_stop") => AnthropicLine::Done,
        Some("error") => AnthropicLine::Error(
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Anthropic upstream stream error")
                .to_string(),
        ),
        _ => AnthropicLine::Ignore,
    }
}

fn system_text(messages: &[ChatMessage]) -> Option<String> {
    let text = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(ChatMessage::text_content)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(text)
}

fn message_to_anthropic(msg: &ChatMessage) -> Value {
    if msg.role == "tool" {
        return json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                "content": msg.text_content()
            }]
        });
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

    if blocks.len() == 1 && blocks[0].get("type").and_then(Value::as_str) == Some("text") {
        json!({ "role": role, "content": blocks[0]["text"].clone() })
    } else {
        json!({ "role": role, "content": blocks })
    }
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

fn anthropic_stop_to_openai(reason: &Option<String>) -> String {
    match reason.as_deref() {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        Some("end_turn") | Some("stop_sequence") | None => "stop",
        Some(other) => other,
    }
    .to_string()
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
