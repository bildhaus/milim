//! The [`ModelService`] trait and the backend-neutral generation types.

use async_trait::async_trait;
use futures::Stream;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use milim_core::api::openai::{
    ChatMessage, Content, DeltaToolCall, FunctionCall, Model, ReasoningEffort, Tool, ToolCall,
    Usage,
};
use milim_core::Result;

/// Decoding/sampling parameters and other per-run generation options,
/// normalized across wire formats.
#[derive(Debug, Clone, Default)]
pub struct SamplingParams {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop: Vec<String>,
    pub seed: Option<i64>,
    pub frequency_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    /// vLLM-compatible extension controls. They remain absent for providers
    /// that should use their own model defaults.
    pub top_k: Option<i32>,
    pub min_p: Option<f32>,
    pub repetition_penalty: Option<f32>,
    pub thinking_token_budget: Option<u32>,
    /// Stable prompt-cache routing key, such as the canonical thread id.
    /// Backends that support one (OpenAI's `prompt_cache_key`) send a hash
    /// of it; without it they derive a key from the leading messages.
    pub prompt_cache_key: Option<String>,
}

/// A backend-neutral generation request. The server builds this from an
/// OpenAI/Ollama/Anthropic wire request; backends consume it.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<Tool>,
    pub tool_choice: Option<serde_json::Value>,
    pub response_format: Option<serde_json::Value>,
    pub prompt: Option<String>,
    pub suffix: Option<String>,
    pub sampling: SamplingParams,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl CompletionRequest {
    /// The most recent user message text, if any.
    pub fn last_user_text(&self) -> String {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.text_content())
            .unwrap_or_default()
    }
}

/// One incremental generation event delta.
#[derive(Debug, Clone, Default)]
pub struct DeltaEvent {
    /// Visible assistant text.
    pub content: Option<String>,
    /// Reasoning / thinking channel text (kept separate from `content`).
    pub reasoning: Option<String>,
    /// Streamed tool-call fragments.
    pub tool_calls: Vec<DeltaToolCall>,
    /// Opaque continuation data for the assistant turn (see
    /// `ChatMessage::provider_state`). An adapter sends the complete value
    /// for the turn, normally once just before `Done`; a later value
    /// replaces an earlier one.
    pub provider_state: Option<serde_json::Value>,
}

impl DeltaEvent {
    /// Convenience: a delta carrying only visible text.
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            content: Some(s.into()),
            ..Default::default()
        }
    }

    /// True if this delta carries nothing.
    pub fn is_empty(&self) -> bool {
        self.content.is_none()
            && self.reasoning.is_none()
            && self.tool_calls.is_empty()
            && self.provider_state.is_none()
    }
}

/// An item in a generation stream.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental content/reasoning/tool-call data.
    Delta(DeltaEvent),
    /// Terminal event with the normalized finish reason (see
    /// [`normalize_finish_reason`]) and token accounting.
    Done { finish_reason: String, usage: Usage },
}

/// Map a provider's raw stop reason onto the normalized finish reasons every
/// backend reports on [`StreamEvent::Done`]: `stop`, `length`, `tool_calls`,
/// `content_filter`, `context_window_exceeded`, or `error`. A missing or
/// unrecognized reason is `stop`.
pub fn normalize_finish_reason(raw: Option<&str>) -> &'static str {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return "stop";
    };
    match raw.to_ascii_lowercase().as_str() {
        // OpenAI `length`, Anthropic `max_tokens`, Gemini `MAX_TOKENS`.
        "length" | "max_tokens" | "max_output_tokens" => "length",
        // The prompt filled the context window: continuing would overflow
        // again, so the agent loop compacts instead.
        "model_context_window_exceeded" | "context_window_exceeded" | "context_length_exceeded" => {
            "context_window_exceeded"
        }
        "tool_calls" | "tool_use" | "function_call" => "tool_calls",
        "content_filter" | "refusal" | "safety" | "recitation" | "blocklist"
        | "prohibited_content" | "spii" | "image_safety" | "language" => "content_filter",
        "error" | "abort" | "malformed_function_call" | "unexpected_tool_call" => "error",
        _ => "stop",
    }
}

/// A pinned, boxed stream of [`StreamEvent`]s.
pub type EventStream = Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>;

/// The fully-assembled result of a non-streaming completion.
#[derive(Debug, Clone)]
pub struct CompletionOutput {
    pub message: ChatMessage,
    pub finish_reason: String,
    pub usage: Usage,
}

/// A backend capable of listing models, generating, and embedding.
///
/// Backends only implement [`stream`](ModelService::stream); the default
/// [`complete`](ModelService::complete) assembles a full message from it.
#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait]
pub trait ModelService: Send + Sync {
    /// Stable backend label (e.g. `"test"`, `"openai"`, `"ollama"`).
    fn name(&self) -> &str;

    /// Whether requests to this backend cross the machine boundary and must
    /// pass through Milim's outbound privacy gate.
    fn requires_privacy_gate(&self) -> bool {
        false
    }

    /// Models this backend can serve.
    async fn list_models(&self) -> Result<Vec<Model>>;

    /// Stream a generation as [`StreamEvent`]s.
    async fn stream(&self, req: CompletionRequest) -> Result<EventStream>;

    /// Handle Ollama's empty-prompt `keep_alive` lifecycle request when a
    /// backend has a native Ollama API to control. Returns `false` when the
    /// backend accepted the compatibility shape but has no lifecycle to manage.
    async fn ollama_keep_alive(
        &self,
        _model: &str,
        _keep_alive: Option<serde_json::Value>,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Compute embeddings for `inputs`.
    async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>>;

    /// Assemble a full, non-streamed completion by draining [`stream`].
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionOutput> {
        use futures::StreamExt;

        let mut stream = self.stream(req).await?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tools = ToolCallAccumulator::default();
        let mut finish_reason = "stop".to_string();
        let mut usage = Usage::default();
        let mut provider_state = None;

        while let Some(ev) = stream.next().await {
            match ev? {
                StreamEvent::Delta(d) => {
                    if let Some(c) = d.content {
                        content.push_str(&c);
                    }
                    if let Some(r) = d.reasoning {
                        reasoning.push_str(&r);
                    }
                    for tc in d.tool_calls {
                        tools.push(tc);
                    }
                    if d.provider_state.is_some() {
                        provider_state = d.provider_state;
                    }
                }
                StreamEvent::Done {
                    finish_reason: fr,
                    usage: u,
                } => {
                    finish_reason = fr;
                    usage = u;
                }
            }
        }

        let tool_calls = tools.finish();
        let has_tools = !tool_calls.is_empty();
        let message = ChatMessage {
            role: "assistant".to_string(),
            // Per OpenAI semantics, content is null when only tool calls exist.
            content: if content.is_empty() && has_tools {
                None
            } else {
                Some(Content::Text(content))
            },
            name: None,
            tool_calls: if has_tools { Some(tool_calls) } else { None },
            tool_call_id: None,
            reasoning_content: if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            provider_state,
        };

        Ok(CompletionOutput {
            message,
            finish_reason,
            usage,
        })
    }
}

/// A shared, type-erased backend handle.
pub type SharedService = Arc<dyn ModelService>;

/// Reassembles streamed [`DeltaToolCall`] fragments into full [`ToolCall`]s.
///
/// OpenAI streams a tool call's `id`/`name` on the first delta and the
/// `arguments` JSON across subsequent deltas, keyed by `index`.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: BTreeMap<u32, PartialCall>,
}

#[derive(Debug, Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

impl ToolCallAccumulator {
    /// Fold one streamed fragment in.
    pub fn push(&mut self, d: DeltaToolCall) {
        let entry = self.calls.entry(d.index).or_default();
        if let Some(id) = d.id {
            entry.id = Some(id);
        }
        if let Some(name) = d.function.name {
            // Some servers repeat the full name (or a growing prefix of it) on
            // every chunk; only a genuine continuation is appended.
            if name.starts_with(entry.name.as_str()) {
                entry.name = name;
            } else {
                entry.name.push_str(&name);
            }
        }
        if let Some(args) = d.function.arguments {
            entry.arguments.push_str(&args);
        }
    }

    /// True if no fragments were accumulated.
    pub fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Produce the assembled tool calls, ordered by their stream index.
    pub fn finish(self) -> Vec<ToolCall> {
        self.calls
            .into_iter()
            .map(|(idx, c)| ToolCall {
                id: Some(c.id.unwrap_or_else(|| format!("call_{idx}"))),
                kind: "function".to_string(),
                function: FunctionCall {
                    name: c.name,
                    arguments: c.arguments,
                },
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::DeltaFunction;

    #[test]
    fn accumulator_reassembles_streamed_arguments() {
        let mut acc = ToolCallAccumulator::default();
        acc.push(DeltaToolCall {
            index: 0,
            id: Some("call_abc".into()),
            kind: Some("function".into()),
            function: DeltaFunction {
                name: Some("get_weather".into()),
                arguments: Some("{\"loc".into()),
            },
        });
        acc.push(DeltaToolCall {
            index: 0,
            id: None,
            kind: None,
            function: DeltaFunction {
                name: None,
                arguments: Some("ation\":\"NYC\"}".into()),
            },
        });
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id.as_deref(), Some("call_abc"));
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"location\":\"NYC\"}");
    }

    #[test]
    fn normalizes_provider_finish_reasons() {
        for (raw, expected) in [
            (None, "stop"),
            (Some("end_turn"), "stop"),
            (Some("stop_sequence"), "stop"),
            (Some("STOP"), "stop"),
            (Some("max_tokens"), "length"),
            (Some("MAX_TOKENS"), "length"),
            (Some("length"), "length"),
            (
                Some("model_context_window_exceeded"),
                "context_window_exceeded",
            ),
            (Some("tool_use"), "tool_calls"),
            (Some("tool_calls"), "tool_calls"),
            (Some("refusal"), "content_filter"),
            (Some("SAFETY"), "content_filter"),
            (Some("content_filter"), "content_filter"),
            (Some("MALFORMED_FUNCTION_CALL"), "error"),
            (Some("error"), "error"),
            (Some("pause_turn"), "stop"),
        ] {
            assert_eq!(normalize_finish_reason(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn accumulator_keeps_names_that_servers_repeat_on_every_chunk() {
        let fragment = |name: &str, arguments: &str| DeltaToolCall {
            index: 0,
            id: Some("call_abc".into()),
            kind: Some("function".into()),
            function: DeltaFunction {
                name: Some(name.into()),
                arguments: Some(arguments.into()),
            },
        };
        let mut acc = ToolCallAccumulator::default();
        acc.push(fragment("get_weather", "{\"loc"));
        acc.push(fragment("get_weather", "ation\":\"NYC\"}"));
        let calls = acc.finish();
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].function.arguments, "{\"location\":\"NYC\"}");

        // A growing prefix replaces, a genuine fragment appends.
        let mut acc = ToolCallAccumulator::default();
        acc.push(fragment("get_", ""));
        acc.push(fragment("get_wea", ""));
        acc.push(fragment("ther", "{}"));
        assert_eq!(acc.finish()[0].function.name, "get_weather");
    }

    #[test]
    fn tool_call_fragments_without_an_index_still_parse() {
        let fragment: DeltaToolCall = serde_json::from_value(serde_json::json!({
            "id": "call_1",
            "function": { "name": "list_dir", "arguments": "{}" }
        }))
        .unwrap();
        assert_eq!(fragment.index, 0);
        assert_eq!(fragment.function.name.as_deref(), Some("list_dir"));
    }

    #[test]
    fn accumulator_synthesizes_missing_id() {
        let mut acc = ToolCallAccumulator::default();
        acc.push(DeltaToolCall {
            index: 2,
            id: None,
            kind: None,
            function: DeltaFunction {
                name: Some("f".into()),
                arguments: Some("{}".into()),
            },
        });
        let calls = acc.finish();
        assert_eq!(calls[0].id.as_deref(), Some("call_2"));
    }
}
