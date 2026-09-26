//! Anthropic Messages API DTOs (`POST /anthropic/v1/messages`).
//!
//! Ported from milim's `AnthropicAPI.swift`. Streaming uses typed SSE events
//! (`message_start`, `content_block_*`, `message_delta`, `message_stop`) which
//! the server constructs directly; this module covers the request and the
//! non-streaming response.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `POST /anthropic/v1/messages` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<SystemContent>,
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl MessagesRequest {
    pub fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// System prompt: a bare string or an array of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

impl SystemContent {
    pub fn plain_text(&self) -> String {
        match self {
            SystemContent::Text(t) => t.clone(),
            SystemContent::Blocks(b) => blocks_text(b),
        }
    }
}

/// A message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

/// Message content: a bare string or an array of content blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

/// A request-side content block, tagged by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        source: Value,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    #[serde(other)]
    Unknown,
}

/// A tool definition (Anthropic uses `input_schema`, not `parameters`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

/// Non-streaming `message` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagesResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String, // "message"
    pub role: String, // "assistant"
    pub content: Vec<ResponseBlock>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>,
    pub usage: Usage,
}

/// A response-side content block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

/// Anthropic token accounting. `input_tokens` excludes the cached portions.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
}

/// Join the text of all text blocks with newlines.
fn blocks_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Flatten any tool-result `content` (string or block array) into text.
pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| {
                if i.get("type").and_then(Value::as_str) == Some("text") {
                    i.get("text").and_then(Value::as_str)
                } else {
                    i.as_str()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_and_block_content() {
        let req: MessagesRequest = serde_json::from_str(
            r#"{"model":"claude","max_tokens":100,"messages":[
                {"role":"user","content":"hi"},
                {"role":"assistant","content":[{"type":"text","text":"hello"}]}
            ]}"#,
        )
        .unwrap();
        assert_eq!(req.messages.len(), 2);
        assert!(matches!(req.messages[0].content, MessageContent::Text(_)));
        assert!(matches!(req.messages[1].content, MessageContent::Blocks(_)));
    }

    #[test]
    fn parses_tool_use_and_result_blocks() {
        let req: MessagesRequest = serde_json::from_str(
            r#"{"model":"c","max_tokens":50,"messages":[
                {"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"f","input":{"x":1}}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"42"}]}
            ]}"#,
        )
        .unwrap();
        if let MessageContent::Blocks(b) = &req.messages[0].content {
            assert!(matches!(b[0], ContentBlock::ToolUse { .. }));
        } else {
            panic!("expected blocks");
        }
    }

    #[test]
    fn system_plain_text() {
        let s = SystemContent::Blocks(vec![
            ContentBlock::Text { text: "a".into() },
            ContentBlock::Text { text: "b".into() },
        ]);
        assert_eq!(s.plain_text(), "a\nb");
    }
}
