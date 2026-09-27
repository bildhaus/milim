//! OpenAI-compatible Chat Completions DTOs.
//!
//! Ported from milim's `OpenAIAPI.swift`. Optional fields are omitted on
//! serialize (`skip_serializing_if`) to match OpenAI's wire output, and unknown
//! request fields are captured in `extra` for forward-compatibility.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Auto,
    None,
    Minimal,
    Low,
    Medium,
    High,
    On,
    Xhigh,
    Max,
}

impl ReasoningEffort {
    pub fn is_auto(self) -> bool {
        self == Self::Auto
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::On => "on",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

/// `POST /v1/chat/completions` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Newer OpenAI alias for `max_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<StringOrArray>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,

    /// Any extra/non-standard fields (e.g. `session_id`, `enable_thinking`).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ChatCompletionRequest {
    /// Effective max-token budget across the two spellings.
    pub fn effective_max_tokens(&self) -> Option<u32> {
        self.max_completion_tokens.or(self.max_tokens)
    }

    /// Whether the client asked for a streamed response.
    pub fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// A `string` or `[string]` (used by `stop`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    String(String),
    Array(Vec<String>),
}

impl StringOrArray {
    /// Normalize to a vector of stop sequences.
    pub fn into_vec(self) -> Vec<String> {
        match self {
            StringOrArray::String(s) => vec![s],
            StringOrArray::Array(v) => v,
        }
    }
}

/// Options controlling streamed responses.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StreamOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>,
}

/// A single chat message (request or response).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Content>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Opaque provider continuation data for an assistant turn, such as
    /// Anthropic thinking blocks with their signatures, OpenAI Responses
    /// reasoning items, or Gemini thought signatures. It is keyed by the
    /// adapter that produced it (for example `{"anthropic": ...}`); each
    /// adapter replays only its own key and ignores the rest. It must be
    /// stored and replayed byte-exact, never redacted or rewritten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
}

impl ChatMessage {
    /// Construct a plain text message.
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(Content::Text(content.into())),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            provider_state: None,
        }
    }

    /// Flatten content to a plain string (joining text parts, ignoring media).
    pub fn text_content(&self) -> String {
        match &self.content {
            Some(Content::Text(s)) => s.clone(),
            Some(Content::Parts(parts)) => parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
            None => String::new(),
        }
    }
}

/// Message content: a bare string or an array of typed parts (multimodal).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Parts(Vec<ContentPart>),
}

/// One part of a multimodal message, tagged by `type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: ImageUrl,
    },
    InputAudio {
        input_audio: Value,
    },
    #[serde(other)]
    Unknown,
}

/// An image reference within a content part.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A tool the model may call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: ToolFunction,
}

/// A function-tool definition (name + JSON-schema parameters).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
}

/// A completed tool call emitted by the assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default = "function_type")]
    pub kind: String,
    pub function: FunctionCall,
}

/// The function name + serialized JSON arguments of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    /// Arguments as a JSON string (OpenAI sends a string, not an object).
    pub arguments: String,
}

fn function_type() -> String {
    "function".to_string()
}

// ----- Non-streaming response -----

/// `chat.completion` response object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String, // "chat.completion"
    pub created: u64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
}

/// One completion choice (non-streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Token accounting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// Provider-reported billed cost in USD. OpenRouter sends this as
    /// `usage.cost`; Milim normalizes it to `cost_usd` everywhere else.
    #[serde(default, alias = "cost", skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Prompt tokens the provider served from its prompt cache. Already
    /// counted in `prompt_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Prompt tokens the provider wrote to its prompt cache. Already counted
    /// in `prompt_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
}

impl Usage {
    pub fn new(prompt: u32, completion: u32) -> Self {
        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            cost_usd: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
        }
    }

    /// Add another response's cache token counts to this running total.
    pub fn add_cache_tokens(&mut self, other: &Usage) {
        fn sum(a: Option<u32>, b: Option<u32>) -> Option<u32> {
            match (a, b) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
            }
        }
        self.cache_read_tokens = sum(self.cache_read_tokens, other.cache_read_tokens);
        self.cache_write_tokens = sum(self.cache_write_tokens, other.cache_write_tokens);
    }
}

// ----- Streaming response -----

/// `chat.completion.chunk` streamed object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String, // "chat.completion.chunk"
    pub created: u64,
    pub model: String,
    pub choices: Vec<ChunkChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// One streamed choice carrying a delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkChoice {
    pub index: u32,
    pub delta: Delta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Incremental content for a streamed choice.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Delta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

/// A streamed tool-call fragment (arguments arrive incrementally).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaToolCall {
    /// Some OpenAI-compatible servers omit `index`; it then defaults to 0.
    #[serde(default)]
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub function: DeltaFunction,
}

/// Incremental function name/arguments for a streamed tool call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeltaFunction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

// ----- Models listing -----

fn model_object() -> String {
    "model".to_string()
}

fn list_object() -> String {
    "list".to_string()
}

/// Deserialize helper: treat `null` (or a missing field) as `T::default()`.
/// Lets us parse permissive upstreams — e.g. Ollama returns `"data": null`
/// when no models are pulled.
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `GET /v1/models` response. Lenient on parse: providers vary (Ollama sends
/// `data: null`; some omit `object`), so missing/null fields default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsResponse {
    #[serde(default = "list_object")]
    pub object: String, // "list"
    #[serde(default, deserialize_with = "null_default")]
    pub data: Vec<Model>,
}

/// One entry in the models list. Only `id` is required when parsing upstream
/// lists — OpenRouter and friends omit `object`/`owned_by`/`created`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    #[serde(default = "model_object")]
    pub object: String, // "model"
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub owned_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(
        default,
        alias = "contextLength",
        alias = "context_window",
        alias = "contextWindow",
        alias = "max_model_len",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_length: Option<u32>,
    #[serde(
        default,
        alias = "maxPromptTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_prompt_tokens: Option<u32>,
    #[serde(
        default,
        alias = "maxCompletionTokens",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ModelReasoningMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ModelCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<ModelArchitecture>,
}

/// Optional provider-supplied pricing, currently used by OpenRouter. Each
/// value is a USD price per token, as a decimal string.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<String>,
    /// Price of a prompt token served from the provider's prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cache_read: Option<String>,
    /// Price of a prompt token written to the provider's prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cache_write: Option<String>,
}

impl ModelPricing {
    /// Estimated USD cost of one response's usage. Cache reads and writes
    /// (already counted in `prompt_tokens`) are charged at their own prices
    /// when the provider publishes them, else at the prompt price. `None`
    /// when the prompt or completion price is missing or invalid.
    pub fn estimate_cost_usd(&self, usage: &Usage) -> Option<f64> {
        fn price(value: Option<&str>) -> Option<f64> {
            value?
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|price| price.is_finite() && *price >= 0.0)
        }
        let prompt = price(self.prompt.as_deref())?;
        let completion = price(self.completion.as_deref())?;
        let cache_read = usage
            .cache_read_tokens
            .unwrap_or(0)
            .min(usage.prompt_tokens);
        let cache_write = usage
            .cache_write_tokens
            .unwrap_or(0)
            .min(usage.prompt_tokens - cache_read);
        let uncached = usage.prompt_tokens - cache_read - cache_write;
        let cost = f64::from(uncached) * prompt
            + f64::from(cache_read) * price(self.input_cache_read.as_deref()).unwrap_or(prompt)
            + f64::from(cache_write) * price(self.input_cache_write.as_deref()).unwrap_or(prompt)
            + f64::from(usage.completion_tokens) * completion;
        (cost.is_finite() && cost >= 0.0).then_some(cost)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelReasoningMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_efforts: Vec<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<ReasoningEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandatory: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_output: Option<bool>,
    #[serde(default, alias = "toolUse", skip_serializing_if = "Option::is_none")]
    pub tool_use: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelArchitecture {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modalities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
}

impl Model {
    pub fn local(id: impl Into<String>, created: u64) -> Self {
        Self {
            id: id.into(),
            object: "model".to_string(),
            created,
            owned_by: "milim".to_string(),
            provider_id: None,
            context_length: Some(4096),
            max_prompt_tokens: None,
            max_completion_tokens: None,
            pricing: None,
            reasoning: None,
            capabilities: None,
            architecture: None,
        }
    }
}

// ----- Error envelope -----

/// OpenAI-style error envelope: `{ "error": { ... } }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

/// The inner error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ErrorEnvelope {
    pub fn new(message: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            error: ErrorBody {
                message: message.into(),
                kind: kind.into(),
                code: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_cache_tokens_are_optional_on_the_wire() {
        let legacy: Usage =
            serde_json::from_str(r#"{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}"#)
                .unwrap();
        assert_eq!(legacy.cache_read_tokens, None);
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::json!({"prompt_tokens":10,"completion_tokens":2,"total_tokens":12})
        );

        let mut total = Usage {
            cache_read_tokens: Some(8),
            ..Usage::new(10, 2)
        };
        total.add_cache_tokens(&Usage {
            cache_write_tokens: Some(5),
            ..Usage::new(6, 1)
        });
        total.add_cache_tokens(&Usage::new(1, 1));
        assert_eq!(total.cache_read_tokens, Some(8));
        assert_eq!(total.cache_write_tokens, Some(5));
        assert_eq!(serde_json::to_value(total).unwrap()["cache_read_tokens"], 8);
    }

    #[test]
    fn parses_string_content() {
        let json = r#"{"role":"user","content":"hello"}"#;
        let m: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(m.text_content(), "hello");
    }

    #[test]
    fn parses_multimodal_content() {
        let json = r#"{"role":"user","content":[
            {"type":"text","text":"look:"},
            {"type":"image_url","image_url":{"url":"http://x/y.png"}}
        ]}"#;
        let m: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(m.text_content(), "look:");
        match m.content.unwrap() {
            Content::Parts(p) => assert_eq!(p.len(), 2),
            _ => panic!("expected parts"),
        }
    }

    #[test]
    fn unknown_request_fields_are_preserved() {
        let json = r#"{"model":"m","messages":[],"session_id":"abc","enable_thinking":true}"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.extra.get("session_id").unwrap(), "abc");
        assert_eq!(
            req.extra.get("enable_thinking").unwrap(),
            &Value::Bool(true)
        );
    }

    #[test]
    fn models_response_tolerates_null_data() {
        // Ollama returns this when no models are pulled.
        let m: ModelsResponse = serde_json::from_str(r#"{"object":"list","data":null}"#).unwrap();
        assert!(m.data.is_empty());
    }

    #[test]
    fn model_parses_with_only_id() {
        // OpenRouter et al. omit object/owned_by/created.
        let m: Model = serde_json::from_str(r#"{"id":"anthropic/claude-3.5-sonnet"}"#).unwrap();
        assert_eq!(m.id, "anthropic/claude-3.5-sonnet");
        assert_eq!(m.object, "model");
        assert_eq!(m.owned_by, "");
        assert!(m.pricing.is_none());
    }

    #[test]
    fn model_parses_context_limits() {
        let m: Model = serde_json::from_str(
            r#"{"id":"openrouter/test","context_length":32768,"max_prompt_tokens":30000,"max_completion_tokens":2048}"#,
        )
        .unwrap();
        assert_eq!(m.context_length, Some(32768));
        assert_eq!(m.max_prompt_tokens, Some(30000));
        assert_eq!(m.max_completion_tokens, Some(2048));
    }

    #[test]
    fn model_parses_vllm_context_length() {
        let m: Model =
            serde_json::from_str(r#"{"id":"local/test","owned_by":"vllm","max_model_len":196608}"#)
                .unwrap();
        assert_eq!(m.context_length, Some(196608));
    }

    #[test]
    fn pricing_charges_cache_tokens_at_cache_prices() {
        let model: Model = serde_json::from_str(
            r#"{"id":"anthropic/claude-sonnet-5","pricing":{"prompt":"0.000002","completion":"0.00001","input_cache_read":"0.0000002","input_cache_write":"0.0000025"}}"#,
        )
        .unwrap();
        let pricing = model.pricing.unwrap();
        let usage = Usage {
            cache_read_tokens: Some(8_000),
            cache_write_tokens: Some(1_000),
            ..Usage::new(10_000, 500)
        };
        // 1,000 uncached + 8,000 read + 1,000 written + 500 completion.
        let expected =
            1_000.0 * 0.000002 + 8_000.0 * 0.0000002 + 1_000.0 * 0.0000025 + 500.0 * 0.00001;
        let cost = pricing.estimate_cost_usd(&usage).unwrap();
        assert!((cost - expected).abs() < 1e-12, "{cost}");

        // Without published cache prices, cached tokens cost the prompt price.
        let plain = ModelPricing {
            prompt: Some("0.000002".into()),
            completion: Some("0.00001".into()),
            ..Default::default()
        };
        let cost = plain.estimate_cost_usd(&usage).unwrap();
        assert!((cost - (10_000.0 * 0.000002 + 500.0 * 0.00001)).abs() < 1e-12);
        assert_eq!(ModelPricing::default().estimate_cost_usd(&usage), None);
    }

    #[test]
    fn model_parses_reasoning_metadata() {
        let m: Model = serde_json::from_str(
            r#"{"id":"openrouter/test","reasoning":{"supported_efforts":["low","medium","high"],"default_effort":"medium","default_enabled":true,"mandatory":false}}"#,
        )
        .unwrap();
        let reasoning = m.reasoning.unwrap();
        assert_eq!(
            reasoning.supported_efforts,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High
            ]
        );
        assert_eq!(reasoning.default_effort, Some(ReasoningEffort::Medium));
        assert_eq!(reasoning.default_enabled, Some(true));
        assert_eq!(reasoning.mandatory, Some(false));
    }

    #[test]
    fn max_tokens_alias_resolves() {
        let json = r#"{"model":"m","messages":[],"max_completion_tokens":256}"#;
        let req: ChatCompletionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.effective_max_tokens(), Some(256));
    }

    #[test]
    fn omits_none_fields_on_serialize() {
        let chunk = ChatCompletionChunk {
            id: "x".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "m".into(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: Delta {
                    content: Some("hi".into()),
                    ..Default::default()
                },
                finish_reason: None,
            }],
            usage: None,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        assert!(!json.contains("usage"));
        assert!(!json.contains("finish_reason"));
        assert!(!json.contains("role"));
        assert!(json.contains("\"content\":\"hi\""));
    }
}
