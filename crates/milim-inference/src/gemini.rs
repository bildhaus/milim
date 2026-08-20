//! Gemini API upstream backend.
//!
//! Translates the backend-neutral [`CompletionRequest`] into Gemini's
//! `generateContent` request shape and maps Gemini SSE responses back into the
//! neutral [`StreamEvent`] shape used by the rest of milim.

use std::collections::HashMap;
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

#[cfg(not(test))]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(50);

#[cfg(not(test))]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(test)]
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(100);

/// Forwards generation to a Gemini API endpoint.
#[derive(Debug, Clone)]
pub struct GeminiBackend {
    label: String,
    base_url: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl GeminiBackend {
    /// Build a backend pointing at `base_url` (usually
    /// `https://generativelanguage.googleapis.com/v1beta`) with an optional API
    /// key.
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
        match &self.api_key {
            Some(k) if !k.is_empty() => rb.header("x-goog-api-key", k),
            _ => rb,
        }
    }

    fn build_body(&self, req: &CompletionRequest) -> Result<Value> {
        let mut body = json!({
            "contents": build_contents(&req.messages)?,
        });

        if let Some(system) = system_text(&req.messages) {
            body["systemInstruction"] = json!({
                "parts": [{ "text": system }]
            });
        }

        let generation_config = generation_config(req);
        if !generation_config.is_null() {
            body["generationConfig"] = generation_config;
        }

        if !req.tools.is_empty() {
            body["tools"] = json!([{
                "functionDeclarations": gemini_tools(&req.tools)
            }]);
        }

        if let Some(tool_config) = gemini_tool_config(req.tool_choice.as_ref()) {
            body["toolConfig"] = tool_config;
        }

        Ok(body)
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
impl ModelService for GeminiBackend {
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

        let parsed: GeminiModelsResponse = resp.json().await.map_err(upstream)?;
        Ok(parsed
            .models
            .into_iter()
            .map(|m| Model {
                id: model_id(&m.name).to_string(),
                object: "model".to_string(),
                created: 0,
                owned_by: self.label.clone(),
                provider_id: None,
                context_length: m.input_token_limit,
                max_prompt_tokens: m.input_token_limit,
                max_completion_tokens: m.output_token_limit,
                pricing: None,
                reasoning: None,
                capabilities: None,
                architecture: None,
            })
            .collect())
    }

    async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
        crate::image_input::validate_request_images(&req)?;
        let body = self.build_body(&req)?;
        let endpoint = format!(
            "{}?alt=sse",
            self.endpoint(&format!("{}:streamGenerateContent", model_path(&req.model)))
        );
        let resp = self
            .auth(self.client.post(endpoint))
            .json(&body)
            .send()
            .await
            .map_err(upstream)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "{} streamGenerateContent -> {status}: {text}",
                self.label
            )));
        }

        let stream = async_stream::stream! {
            let mut bytes = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = GeminiStreamState::default();

            while let Some(chunk) = bytes.next().await {
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
                        GeminiLine::Delta(d) => {
                            if !d.is_empty() {
                                yield Ok(StreamEvent::Delta(d));
                            }
                        }
                        GeminiLine::Error(e) => {
                            yield Err(Error::Upstream(e));
                            return;
                        }
                        GeminiLine::Ignore => {}
                    }
                }
            }

            yield Ok(StreamEvent::Done {
                finish_reason: gemini_finish_to_openai(&state.finish_reason, state.saw_tool_call),
                usage: state.usage,
            });
        };

        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, _inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        Err(Error::InvalidRequest(
            "Gemini embeddings are not implemented for this provider".to_string(),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModel>,
}

#[derive(Debug, Deserialize)]
struct GeminiModel {
    name: String,
    #[serde(default, rename = "inputTokenLimit")]
    input_token_limit: Option<u32>,
    #[serde(default, rename = "outputTokenLimit")]
    output_token_limit: Option<u32>,
}

#[derive(Debug, Default)]
struct GeminiStreamState {
    usage: Usage,
    finish_reason: Option<String>,
    saw_tool_call: bool,
    next_tool_index: u32,
}

enum GeminiLine {
    Delta(DeltaEvent),
    Error(String),
    Ignore,
}

fn parse_sse_line(line: &str, state: &mut GeminiStreamState) -> GeminiLine {
    let Some(data) = line.strip_prefix("data:") else {
        return GeminiLine::Ignore;
    };
    let data = data.trim();
    if data.is_empty() {
        return GeminiLine::Ignore;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else {
        return GeminiLine::Ignore;
    };

    if let Some(error) = v
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        return GeminiLine::Error(error.to_string());
    }

    if let Some(usage) = v.get("usageMetadata") {
        state.usage = Usage {
            prompt_tokens: opt_u32(usage, "promptTokenCount").unwrap_or(0),
            completion_tokens: opt_u32(usage, "candidatesTokenCount").unwrap_or(0),
            total_tokens: opt_u32(usage, "totalTokenCount").unwrap_or_else(|| {
                opt_u32(usage, "promptTokenCount").unwrap_or(0)
                    + opt_u32(usage, "candidatesTokenCount").unwrap_or(0)
            }),
            cost_usd: None,
        };
    }

    let mut delta = DeltaEvent::default();
    if let Some(candidates) = v.get("candidates").and_then(Value::as_array) {
        for candidate in candidates {
            if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
                state.finish_reason = Some(reason.to_string());
            }
            let Some(parts) = candidate
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    delta.content.get_or_insert_with(String::new).push_str(text);
                }
                if let Some(call) = part.get("functionCall") {
                    let index = state.next_tool_index;
                    state.next_tool_index += 1;
                    state.saw_tool_call = true;
                    delta.tool_calls.push(DeltaToolCall {
                        index,
                        id: Some(format!("gemini_call_{index}")),
                        kind: Some("function".to_string()),
                        function: DeltaFunction {
                            name: call.get("name").and_then(Value::as_str).map(str::to_string),
                            arguments: call.get("args").map(|args| args.to_string()),
                        },
                    });
                }
            }
        }
    }

    if delta.is_empty() {
        GeminiLine::Ignore
    } else {
        GeminiLine::Delta(delta)
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

fn build_contents(messages: &[ChatMessage]) -> Result<Vec<Value>> {
    let mut tool_names = HashMap::new();
    messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| message_to_gemini(m, &mut tool_names))
        .collect()
}

fn message_to_gemini(msg: &ChatMessage, tool_names: &mut HashMap<String, String>) -> Result<Value> {
    if msg.role == "tool" {
        let name = msg
            .name
            .clone()
            .or_else(|| {
                msg.tool_call_id
                    .as_ref()
                    .and_then(|id| tool_names.get(id).cloned())
            })
            .or_else(|| msg.tool_call_id.clone())
            .unwrap_or_else(|| "tool_result".to_string());
        return Ok(json!({
            "role": "user",
            "parts": [{
                "functionResponse": {
                    "name": name,
                    "response": tool_response_value(&msg.text_content())
                }
            }]
        }));
    }

    let role = if msg.role == "assistant" {
        "model"
    } else {
        "user"
    };
    let mut parts = content_parts(msg)?;

    if let Some(calls) = &msg.tool_calls {
        for call in calls {
            if let Some(id) = &call.id {
                tool_names.insert(id.clone(), call.function.name.clone());
            }
            let args = serde_json::from_str::<Value>(&call.function.arguments)
                .unwrap_or_else(|_| Value::Object(Default::default()));
            parts.push(json!({
                "functionCall": {
                    "name": call.function.name,
                    "args": args
                }
            }));
        }
    }

    if parts.is_empty() {
        parts.push(json!({ "text": "" }));
    }
    Ok(json!({ "role": role, "parts": parts }))
}

fn content_parts(msg: &ChatMessage) -> Result<Vec<Value>> {
    match &msg.content {
        Some(Content::Text(text)) if !text.is_empty() => Ok(vec![json!({ "text": text })]),
        Some(Content::Parts(parts)) => parts
            .iter()
            .filter_map(|part| part_to_gemini(part).transpose())
            .collect(),
        _ => Ok(Vec::new()),
    }
}

fn part_to_gemini(part: &ContentPart) -> Result<Option<Value>> {
    match part {
        ContentPart::Text { text } => Ok(Some(json!({ "text": text }))),
        ContentPart::ImageUrl { image_url } => {
            let url = &image_url.url;
            if let Some((mime_type, data)) = parse_data_url(url) {
                Ok(Some(json!({
                    "inline_data": {
                        "mime_type": mime_type,
                        "data": data
                    }
                })))
            } else if is_gemini_file_uri(url) {
                Ok(Some(json!({
                    "file_data": {
                        "file_uri": url
                    }
                })))
            } else {
                Err(Error::InvalidRequest(
                    "Gemini image URLs must be inline data URLs or Gemini Files API URIs; arbitrary HTTP image URLs are not supported"
                        .to_string(),
                ))
            }
        }
        _ => Ok(None),
    }
}

fn is_gemini_file_uri(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://generativelanguage.googleapis.com/") else {
        return false;
    };
    let path = rest.split_once('?').map(|(path, _)| path).unwrap_or(rest);
    ["v1beta/files/", "v1/files/"]
        .iter()
        .any(|prefix| path.strip_prefix(prefix).is_some_and(|id| !id.is_empty()))
}

fn tool_response_value(text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(v) if v.is_object() => v,
        _ => json!({ "content": text }),
    }
}

fn parse_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    Some((media_type, data))
}

fn generation_config(req: &CompletionRequest) -> Value {
    let mut cfg = serde_json::Map::new();
    if let Some(max_tokens) = req.sampling.max_tokens {
        cfg.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = req.sampling.temperature {
        cfg.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = req.sampling.top_p {
        cfg.insert("topP".to_string(), json!(top_p));
    }
    if let Some(seed) = req.sampling.seed {
        cfg.insert("seed".to_string(), json!(seed));
    }
    if let Some(presence_penalty) = req.sampling.presence_penalty {
        cfg.insert("presencePenalty".to_string(), json!(presence_penalty));
    }
    if let Some(frequency_penalty) = req.sampling.frequency_penalty {
        cfg.insert("frequencyPenalty".to_string(), json!(frequency_penalty));
    }
    if !req.sampling.stop.is_empty() {
        cfg.insert("stopSequences".to_string(), json!(req.sampling.stop));
    }
    if let Some(thinking) = gemini_thinking_config(req.reasoning_effort, &req.model) {
        cfg.insert("thinkingConfig".to_string(), thinking);
    }
    if cfg.is_empty() {
        Value::Null
    } else {
        Value::Object(cfg)
    }
}

fn gemini_thinking_config(effort: Option<ReasoningEffort>, model: &str) -> Option<Value> {
    let effort = effort.filter(|e| !e.is_auto())?;
    let id = model.to_ascii_lowercase();
    if id.contains("gemini-3") {
        return match effort {
            ReasoningEffort::Minimal | ReasoningEffort::Low => {
                Some(json!({ "thinkingLevel": "LOW" }))
            }
            ReasoningEffort::Medium => Some(json!({ "thinkingLevel": "MEDIUM" })),
            ReasoningEffort::High | ReasoningEffort::Xhigh | ReasoningEffort::Max => {
                Some(json!({ "thinkingLevel": "HIGH" }))
            }
            ReasoningEffort::None | ReasoningEffort::Auto | ReasoningEffort::On => None,
        };
    }
    if id.contains("gemini-2.5") {
        return match effort {
            ReasoningEffort::None if id.contains("flash") => Some(json!({ "thinkingBudget": 0 })),
            ReasoningEffort::Minimal => Some(json!({ "thinkingBudget": 128 })),
            ReasoningEffort::Low => Some(json!({ "thinkingBudget": 1024 })),
            ReasoningEffort::Medium => Some(json!({ "thinkingBudget": 4096 })),
            ReasoningEffort::High => Some(json!({ "thinkingBudget": 8192 })),
            ReasoningEffort::Xhigh | ReasoningEffort::Max => {
                Some(json!({ "thinkingBudget": 16384 }))
            }
            ReasoningEffort::Auto | ReasoningEffort::None | ReasoningEffort::On => None,
        };
    }
    None
}

fn gemini_tools(tools: &[Tool]) -> Vec<Value> {
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
                "parameters".to_string(),
                t.function
                    .parameters
                    .clone()
                    .unwrap_or_else(|| json!({"type":"object"})),
            );
            Value::Object(tool)
        })
        .collect()
}

fn gemini_tool_config(choice: Option<&Value>) -> Option<Value> {
    let choice = choice?;
    let function_config = match choice.get("type").and_then(Value::as_str) {
        Some("function") => choice
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map(|name| {
                json!({
                    "mode": "ANY",
                    "allowedFunctionNames": [name]
                })
            })?,
        Some("required") => json!({ "mode": "ANY" }),
        Some("none") => json!({ "mode": "NONE" }),
        Some("auto") | None => json!({ "mode": "AUTO" }),
        Some(other) => json!({ "mode": other.to_ascii_uppercase() }),
    };
    Some(json!({
        "functionCallingConfig": function_config
    }))
}

fn model_path(model: &str) -> String {
    if model.starts_with("models/") {
        model.to_string()
    } else {
        format!("models/{model}")
    }
}

fn model_id(name: &str) -> &str {
    name.strip_prefix("models/").unwrap_or(name)
}

fn gemini_finish_to_openai(reason: &Option<String>, saw_tool_call: bool) -> String {
    if saw_tool_call {
        return "tool_calls".to_string();
    }
    match reason.as_deref() {
        Some("MAX_TOKENS") => "length",
        Some("STOP") | None => "stop",
        Some(other) => other,
    }
    .to_string()
}

fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(Value::as_u64).map(|n| n as u32)
}

fn upstream(e: impl std::fmt::Display) -> Error {
    Error::Upstream(e.to_string())
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use milim_core::api::openai::ImageUrl;

    #[test]
    fn rejects_arbitrary_http_image_urls() {
        let part = ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: "https://example.com/image.png".to_string(),
                detail: None,
            },
        };
        let error = part_to_gemini(&part).unwrap_err().to_string();
        assert!(error.contains("Gemini Files API URIs"));
    }

    #[test]
    fn accepts_genuine_gemini_files_api_uris() {
        let part = ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: "https://generativelanguage.googleapis.com/v1beta/files/abc123".to_string(),
                detail: None,
            },
        };
        let value = part_to_gemini(&part).unwrap().unwrap();
        assert_eq!(
            value["file_data"]["file_uri"],
            "https://generativelanguage.googleapis.com/v1beta/files/abc123"
        );
    }
}
