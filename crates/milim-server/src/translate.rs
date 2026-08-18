//! Translation between wire DTOs and the backend-neutral request type.

use base64::Engine;
use milim_core::api::anthropic::{
    self, ContentBlock, MessageContent, MessagesRequest, ResponseBlock,
};
use milim_core::api::ollama::{OllamaChatRequest, OllamaMessage};
use milim_core::api::openai::{
    ChatCompletionRequest, ChatMessage, Content, ContentPart, FunctionCall, ImageUrl,
    ReasoningEffort, Tool, ToolCall, ToolFunction,
};
use milim_core::{Error, Result};
use milim_inference::{CompletionRequest, SamplingParams};
use serde_json::Value;

/// Build a neutral request from an OpenAI Chat Completions request.
pub fn openai_to_completion(req: ChatCompletionRequest) -> CompletionRequest {
    let sampling = SamplingParams {
        temperature: req.temperature,
        top_p: req.top_p,
        max_tokens: req.effective_max_tokens(),
        stop: req.stop.clone().map(|s| s.into_vec()).unwrap_or_default(),
        seed: req.seed,
        frequency_penalty: req.frequency_penalty,
        presence_penalty: req.presence_penalty,
        top_k: req
            .extra
            .get("top_k")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        min_p: req
            .extra
            .get("min_p")
            .and_then(Value::as_f64)
            .map(|v| v as f32),
        repetition_penalty: req
            .extra
            .get("repetition_penalty")
            .and_then(Value::as_f64)
            .map(|v| v as f32),
        thinking_token_budget: req
            .extra
            .get("thinking_token_budget")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    };
    CompletionRequest {
        model: req.model,
        messages: req.messages,
        tools: req.tools.unwrap_or_default(),
        tool_choice: req.tool_choice,
        response_format: req.response_format,
        prompt: None,
        suffix: None,
        sampling,
        reasoning_effort: req.reasoning_effort,
    }
}

/// Build a neutral request from an Ollama `/api/chat` request.
pub fn ollama_to_completion(req: OllamaChatRequest) -> CompletionRequest {
    let opts = req.options.unwrap_or(Value::Null);
    let sampling = SamplingParams {
        temperature: opt_f32(&opts, "temperature"),
        top_p: opt_f32(&opts, "top_p"),
        max_tokens: opt_u32(&opts, "num_predict"),
        stop: opt_stops(&opts),
        seed: opt_i64(&opts, "seed"),
        frequency_penalty: opt_f32(&opts, "frequency_penalty"),
        presence_penalty: opt_f32(&opts, "presence_penalty"),
        ..Default::default()
    };
    CompletionRequest {
        model: req.model,
        messages: req.messages.into_iter().map(ollama_message).collect(),
        tools: req.tools.unwrap_or_default(),
        tool_choice: None,
        response_format: req.format.map(ollama_format_to_response_format),
        prompt: None,
        suffix: None,
        sampling,
        reasoning_effort: ollama_think_effort(req.think.as_ref()),
    }
}

fn ollama_message(m: OllamaMessage) -> ChatMessage {
    let content = match m.images {
        Some(images) if !images.is_empty() => {
            let mut parts = Vec::new();
            if !m.content.is_empty() {
                parts.push(ContentPart::Text { text: m.content });
            }
            parts.extend(images.into_iter().map(|image| ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: ollama_image_url(image),
                    detail: None,
                },
            }));
            Content::Parts(parts)
        }
        _ => Content::Text(m.content),
    };
    ChatMessage {
        role: m.role,
        content: Some(content),
        name: None,
        tool_calls: m.tool_calls,
        tool_call_id: None,
        reasoning_content: m.thinking,
    }
}

fn ollama_image_url(image: String) -> String {
    if image.trim_start().starts_with("data:image/") {
        image
    } else {
        format!("data:image/png;base64,{image}")
    }
}

pub(crate) fn ollama_think_effort(think: Option<&Value>) -> Option<ReasoningEffort> {
    match think? {
        Value::Bool(true) => Some(ReasoningEffort::Medium),
        Value::Bool(false) => Some(ReasoningEffort::None),
        Value::String(value) => serde_json::from_value(Value::String(value.clone())).ok(),
        _ => None,
    }
}

/// Build a neutral request from an Anthropic Messages request.
pub fn anthropic_to_completion(req: MessagesRequest) -> Result<CompletionRequest> {
    let mut messages: Vec<ChatMessage> = Vec::new();

    if let Some(system) = &req.system {
        messages.push(ChatMessage::text("system", system.plain_text()));
    }

    for m in req.messages {
        match m.content {
            MessageContent::Text(t) => messages.push(ChatMessage::text(m.role, t)),
            MessageContent::Blocks(blocks) => {
                let mut content_parts = Vec::new();
                let mut tool_calls: Vec<ToolCall> = Vec::new();
                let mut tool_results: Vec<(String, String)> = Vec::new();

                for b in blocks {
                    match b {
                        ContentBlock::Text { text } => {
                            content_parts.push(ContentPart::Text { text });
                        }
                        ContentBlock::Image { source } => {
                            content_parts.push(anthropic_image_part(source)?);
                        }
                        ContentBlock::ToolUse { id, name, input } => tool_calls.push(ToolCall {
                            id: Some(id),
                            kind: "function".to_string(),
                            function: FunctionCall {
                                name,
                                arguments: input.to_string(),
                            },
                        }),
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => tool_results.push((tool_use_id, anthropic::value_to_text(&content))),
                        ContentBlock::Unknown => {}
                    }
                }

                if !content_parts.is_empty() || !tool_calls.is_empty() {
                    let content = match content_parts.as_slice() {
                        [] => None,
                        [ContentPart::Text { text }] => Some(Content::Text(text.clone())),
                        _ => Some(Content::Parts(content_parts)),
                    };
                    messages.push(ChatMessage {
                        role: m.role.clone(),
                        content,
                        name: None,
                        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
                        tool_call_id: None,
                        reasoning_content: None,
                    });
                }
                // Tool results map to OpenAI `tool` role messages.
                for (id, content) in tool_results {
                    messages.push(ChatMessage {
                        role: "tool".to_string(),
                        content: Some(Content::Text(content)),
                        name: None,
                        tool_calls: None,
                        tool_call_id: Some(id),
                        reasoning_content: None,
                    });
                }
            }
        }
    }

    let sampling = SamplingParams {
        temperature: req.temperature,
        top_p: req.top_p,
        max_tokens: Some(req.max_tokens),
        stop: req.stop_sequences.unwrap_or_default(),
        seed: None,
        frequency_penalty: None,
        presence_penalty: None,
        ..Default::default()
    };

    let tools = req
        .tools
        .unwrap_or_default()
        .into_iter()
        .map(|t| Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: t.name,
                description: t.description,
                parameters: t.input_schema,
            },
        })
        .collect();

    Ok(CompletionRequest {
        model: req.model,
        messages,
        tools,
        tool_choice: req.tool_choice,
        response_format: None,
        prompt: None,
        suffix: None,
        sampling,
        reasoning_effort: None,
    })
}

fn anthropic_image_part(source: Value) -> Result<ContentPart> {
    const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
    let source_type = source.get("type").and_then(Value::as_str).ok_or_else(|| {
        Error::InvalidRequest("Anthropic image source type is required".to_string())
    })?;
    let url = match source_type {
        "base64" => {
            let media_type = source
                .get("media_type")
                .and_then(Value::as_str)
                .filter(|value| {
                    matches!(
                        *value,
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                    )
                })
                .ok_or_else(|| {
                    Error::InvalidRequest(
                        "Anthropic base64 images must use PNG, JPEG, WebP, or GIF".to_string(),
                    )
                })?;
            let data = source
                .get("data")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    Error::InvalidRequest("Anthropic image data is required".to_string())
                })?;
            if data.len() > MAX_IMAGE_BYTES * 4 / 3 + 8 {
                return Err(Error::InvalidRequest(
                    "Anthropic images must be no larger than 2 MB".to_string(),
                ));
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| {
                    Error::InvalidRequest("Anthropic image data is not valid base64".to_string())
                })?;
            if decoded.is_empty() || decoded.len() > MAX_IMAGE_BYTES {
                return Err(Error::InvalidRequest(
                    "Anthropic images must contain 1 byte to 2 MB of data".to_string(),
                ));
            }
            if !crate::codex_bridge::image_bytes_match_media_type(media_type, &decoded) {
                return Err(Error::InvalidRequest(format!(
                    "Anthropic image bytes do not match {media_type}"
                )));
            }
            format!("data:{media_type};base64,{data}")
        }
        "url" => source
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            .map(ToString::to_string)
            .ok_or_else(|| {
                Error::InvalidRequest(
                    "Anthropic URL images require an http:// or https:// URL".to_string(),
                )
            })?,
        other => {
            return Err(Error::InvalidRequest(format!(
                "unsupported Anthropic image source type '{other}'"
            )))
        }
    };
    Ok(ContentPart::ImageUrl {
        image_url: ImageUrl { url, detail: None },
    })
}

pub fn ollama_format_to_response_format(format: Value) -> Value {
    match format {
        Value::String(s) if s == "json" => serde_json::json!({ "type": "json_object" }),
        Value::Object(map) => serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "ollama_schema",
                "schema": Value::Object(map),
            }
        }),
        other => other,
    }
}

/// Convert an assembled assistant message into Anthropic response blocks.
pub fn anthropic_response_blocks(msg: &ChatMessage) -> Vec<ResponseBlock> {
    let mut blocks = Vec::new();
    let text = msg.text_content();
    if !text.is_empty() {
        blocks.push(ResponseBlock::Text { text });
    }
    if let Some(calls) = &msg.tool_calls {
        for tc in calls {
            let input: Value = serde_json::from_str(&tc.function.arguments)
                .unwrap_or_else(|_| Value::Object(Default::default()));
            blocks.push(ResponseBlock::ToolUse {
                id: tc.id.clone().unwrap_or_default(),
                name: tc.function.name.clone(),
                input,
            });
        }
    }
    blocks
}

/// Map an OpenAI finish reason to an Anthropic stop reason.
pub fn anthropic_stop_reason(finish: &str) -> String {
    match finish {
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        "stop" => "end_turn",
        other => other,
    }
    .to_string()
}

fn opt_f32(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as u32)
}

fn opt_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

/// Ollama's `stop` option may be a string or an array of strings.
fn opt_stops(v: &Value) -> Vec<String> {
    match v.get("stop") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEOMETRIC_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC";

    #[test]
    fn maps_openai_sampling() {
        let req: ChatCompletionRequest = serde_json::from_str(
            r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"temperature":0.7,"max_completion_tokens":128,"stop":["END"]}"#,
        )
        .unwrap();
        let c = openai_to_completion(req);
        assert_eq!(c.sampling.temperature, Some(0.7));
        assert_eq!(c.sampling.max_tokens, Some(128));
        assert_eq!(c.sampling.stop, vec!["END".to_string()]);
        assert_eq!(c.messages.len(), 1);
    }

    #[test]
    fn maps_openai_reasoning_effort() {
        let req: ChatCompletionRequest =
            serde_json::from_str(r#"{"model":"m","messages":[],"reasoning_effort":"high"}"#)
                .unwrap();
        let c = openai_to_completion(req);
        assert_eq!(
            c.reasoning_effort,
            Some(milim_core::api::openai::ReasoningEffort::High)
        );
    }

    #[test]
    fn maps_ollama_options() {
        let req: OllamaChatRequest = serde_json::from_str(
            r#"{"model":"llama3","messages":[{"role":"user","content":"hi"}],"options":{"temperature":0.2,"num_predict":64,"stop":"<END>"}}"#,
        )
        .unwrap();
        let c = ollama_to_completion(req);
        assert_eq!(c.sampling.temperature, Some(0.2));
        assert_eq!(c.sampling.max_tokens, Some(64));
        assert_eq!(c.sampling.stop, vec!["<END>".to_string()]);
    }

    #[test]
    fn maps_ollama_images_to_multimodal_parts() {
        let req: OllamaChatRequest = serde_json::from_str(
            r#"{"model":"llava","messages":[{"role":"user","content":"what is this?","images":["AAAA"]}]}"#,
        )
        .unwrap();
        let c = ollama_to_completion(req);
        let Some(Content::Parts(parts)) = &c.messages[0].content else {
            panic!("expected multimodal parts");
        };
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[0], ContentPart::Text { .. }));
        match &parts[1] {
            ContentPart::ImageUrl { image_url } => {
                assert_eq!(image_url.url, "data:image/png;base64,AAAA");
            }
            _ => panic!("expected image part"),
        }
    }

    #[test]
    fn maps_anthropic_base64_and_url_images_to_multimodal_parts() {
        let req: MessagesRequest = serde_json::from_value(serde_json::json!({
            "model": "claude-sonnet-4",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "Compare these shapes" },
                    { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": GEOMETRIC_PNG } },
                    { "type": "image", "source": { "type": "url", "url": "https://example.com/shape.webp" } }
                ]
            }]
        })).unwrap();
        let completion = anthropic_to_completion(req).unwrap();
        let Some(Content::Parts(parts)) = &completion.messages[0].content else {
            panic!("expected multimodal parts");
        };
        assert_eq!(parts.len(), 3);
        assert!(matches!(parts[0], ContentPart::Text { .. }));
        assert!(
            matches!(&parts[1], ContentPart::ImageUrl { image_url } if image_url.url == format!("data:image/png;base64,{GEOMETRIC_PNG}"))
        );
        assert!(
            matches!(&parts[2], ContentPart::ImageUrl { image_url } if image_url.url == "https://example.com/shape.webp")
        );
    }

    #[test]
    fn rejects_malformed_anthropic_image_sources() {
        let req: MessagesRequest = serde_json::from_value(serde_json::json!({
            "model": "claude-sonnet-4",
            "max_tokens": 64,
            "messages": [{
                "role": "user",
                "content": [{ "type": "image", "source": { "type": "base64", "media_type": "image/svg+xml", "data": "AAAA" } }]
            }]
        })).unwrap();
        assert!(anthropic_to_completion(req).is_err());
    }

    #[test]
    fn maps_structured_output_formats() {
        let req: ChatCompletionRequest = serde_json::from_str(
            r#"{"model":"m","messages":[],"response_format":{"type":"json_object"}}"#,
        )
        .unwrap();
        assert_eq!(
            openai_to_completion(req).response_format.unwrap()["type"],
            "json_object"
        );

        let req: OllamaChatRequest =
            serde_json::from_str(r#"{"model":"m","format":{"type":"object"}}"#).unwrap();
        let format = ollama_to_completion(req).response_format.unwrap();
        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["json_schema"]["schema"]["type"], "object");
    }
}
