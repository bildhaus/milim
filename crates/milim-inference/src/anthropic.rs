//! Anthropic Messages API upstream backend.
//!
//! Translates the backend-neutral [`CompletionRequest`] into Anthropic's
//! `/v1/messages` format and maps Anthropic named SSE events back into the
//! neutral [`StreamEvent`] shape used by the rest of milim.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, DeltaFunction, DeltaToolCall, Model, ModelReasoningMetadata,
    ReasoningEffort, Tool, Usage,
};
use milim_core::provider_error::upstream_stream_error;
use milim_core::{Error, Result};

use crate::http_error::{http_status_error, stream_read_error, HttpFailure};
use crate::service::{
    normalize_finish_reason, CompletionRequest, DeltaEvent, EventStream, ModelService,
    SamplingParams, StreamEvent,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Highest output budget sent when the request leaves `max_tokens` unset.
/// Anthropic's output rate limits count generated tokens, not `max_tokens`,
/// so a roomy budget only leaves thinking and long tool inputs room to
/// finish. Model caps above it (128K on current models) are not used by
/// default.
const DEFAULT_OUTPUT_CEILING: u32 = 64_000;

/// Output budget for a model this adapter does not recognize.
const UNKNOWN_MODEL_OUTPUT: u32 = 16_000;

/// Smallest `budget_tokens` Anthropic accepts for manual extended thinking.
const MIN_THINKING_BUDGET: u32 = 1_024;

/// `output_config.effort` levels, lowest first.
const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

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

    /// Build the `/v1/messages` body. Thinking, effort, sampling, and
    /// `tool_choice` follow what the model family accepts (see
    /// [`claude_caps`]); a model this adapter does not recognize gets the
    /// caller's sampling and tool choice and no thinking configuration.
    fn build_body(&self, req: &CompletionRequest) -> Value {
        let caps = claude_caps(&req.model);
        let (mut system, messages) = anthropic_messages(&req.messages, &req.model);
        let max_tokens = self.max_tokens_for(req);
        let plan = caps.map(|caps| {
            thinking_plan(
                &caps,
                req.reasoning_effort.unwrap_or(ReasoningEffort::Auto),
                req.sampling.thinking_token_budget,
                max_tokens,
                !continues_turn_without_thinking(&messages),
            )
        });
        let thinking = plan.as_ref().is_some_and(|plan| plan.enabled);

        let mut body = json!({
            "model": req.model,
            "max_tokens": max_tokens,
            "messages": messages,
            "stream": true,
        });

        if let Some(last) = system.last_mut() {
            set_cache_breakpoint(last);
            body["system"] = Value::Array(system);
        }
        let (temperature, top_p) = sampling_params(&req.sampling, caps.as_ref(), thinking);
        if let Some(t) = temperature {
            body["temperature"] = json!(t);
        }
        if let Some(t) = top_p {
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
            // Manual extended thinking rejects forced tool use, and some
            // models reject it on every request.
            let allow_forced = caps.is_none_or(|caps| {
                caps.forced_tool_choice && !(thinking && caps.thinking == ThinkingMode::Budget)
            });
            if let Some(choice) = anthropic_tool_choice(req.tool_choice.as_ref(), allow_forced) {
                body["tool_choice"] = choice;
            }
        }
        if let Some(plan) = plan {
            if let Some(config) = plan.thinking {
                body["thinking"] = config;
            }
            if let Some(effort) = plan.effort {
                body["output_config"] = json!({ "effort": effort });
            }
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

/// Output budget for a model with no catalog cap: the family's output limit,
/// never above [`DEFAULT_OUTPUT_CEILING`].
fn default_output_tokens(model: &str) -> u32 {
    claude_caps(model).map_or(UNKNOWN_MODEL_OUTPUT, |caps| {
        caps.max_output.min(DEFAULT_OUTPUT_CEILING)
    })
}

/// A Claude model line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClaudeLine {
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Mythos,
}

/// How a Claude model takes thinking configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThinkingMode {
    /// No thinking (Claude 3 models before 3.7).
    Unsupported,
    /// Manual extended thinking, `{type: "enabled", budget_tokens}`, off
    /// unless requested.
    Budget,
    /// Adaptive thinking, off unless `{type: "adaptive"}` is sent.
    Adaptive,
    /// Adaptive thinking on by default; `{type: "disabled"}` turns it off.
    AdaptiveDefault,
    /// Adaptive thinking that cannot be turned off.
    AdaptiveAlways,
}

/// Which sampling parameters a model accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sampling {
    /// Non-default `temperature`, `top_p`, and `top_k` are rejected.
    Rejected,
    /// Accepted, but `temperature` and `top_p` not together (Claude 4).
    OneOf,
    Accepted,
}

/// What one Claude model family accepts, per Anthropic's model, thinking,
/// and effort docs. A model milim does not recognize has no entry and gets
/// no thinking or effort configuration.
#[derive(Clone, Copy, Debug)]
struct ClaudeCaps {
    thinking: ThinkingMode,
    /// `output_config.effort` levels the model accepts, lowest first; empty
    /// when the model rejects `effort`.
    efforts: &'static [&'static str],
    /// The level an omitted `effort` runs at.
    default_effort: &'static str,
    /// Whether thinking `display` defaults to `"omitted"`, so summarized
    /// thinking text has to be requested.
    omits_thinking_text: bool,
    sampling: Sampling,
    /// Whether `tool_choice` `any` or `tool` is accepted.
    forced_tool_choice: bool,
    max_output: u32,
    context_window: u32,
}

const ALL_EFFORTS: &[&str] = &EFFORT_LEVELS;
const EFFORTS_WITHOUT_XHIGH: &[&str] = &["low", "medium", "high", "max"];
const BASIC_EFFORTS: &[&str] = &["low", "medium", "high"];

/// Fable, Mythos, and Opus 5.5 and later: adaptive thinking always on.
const ALWAYS_THINKING: ClaudeCaps = ClaudeCaps {
    thinking: ThinkingMode::AdaptiveAlways,
    efforts: ALL_EFFORTS,
    default_effort: "high",
    omits_thinking_text: true,
    sampling: Sampling::Rejected,
    forced_tool_choice: false,
    max_output: 128_000,
    context_window: 1_000_000,
};

/// Claude Opus 5 and Sonnet 5: adaptive thinking on by default.
const DEFAULT_THINKING: ClaudeCaps = ClaudeCaps {
    thinking: ThinkingMode::AdaptiveDefault,
    forced_tool_choice: true,
    ..ALWAYS_THINKING
};

/// Claude Opus 4.7 and 4.8: adaptive thinking on request.
const ADAPTIVE_THINKING: ClaudeCaps = ClaudeCaps {
    thinking: ThinkingMode::Adaptive,
    ..DEFAULT_THINKING
};

/// Claude Opus 4.6 and Sonnet 4.6: adaptive thinking on request, summarized
/// by default, sampling accepted while thinking is off, no `xhigh`.
const ADAPTIVE_THINKING_4_6: ClaudeCaps = ClaudeCaps {
    efforts: EFFORTS_WITHOUT_XHIGH,
    omits_thinking_text: false,
    sampling: Sampling::OneOf,
    ..ADAPTIVE_THINKING
};

/// Claude 4 models before 4.6 (and Sonnet 3.7): manual extended thinking.
const BUDGET_THINKING: ClaudeCaps = ClaudeCaps {
    thinking: ThinkingMode::Budget,
    efforts: &[],
    omits_thinking_text: false,
    sampling: Sampling::OneOf,
    max_output: 64_000,
    context_window: 200_000,
    ..ADAPTIVE_THINKING
};

/// Claude 3 models before 3.7.
const NO_THINKING: ClaudeCaps = ClaudeCaps {
    thinking: ThinkingMode::Unsupported,
    sampling: Sampling::Accepted,
    max_output: 4_096,
    ..BUDGET_THINKING
};

/// The capability entry for a Claude model id, or `None` for a model this
/// adapter does not recognize. Unreleased versions of a known line inherit
/// the newest known version's entry.
fn claude_caps(model: &str) -> Option<ClaudeCaps> {
    use ClaudeLine::*;
    let (line, version) = parse_claude_model(model)?;
    let caps = match (line, version) {
        // Claude Mythos Preview has no `xhigh`.
        (Fable | Mythos, None) => ClaudeCaps {
            efforts: EFFORTS_WITHOUT_XHIGH,
            ..ALWAYS_THINKING
        },
        (Fable | Mythos, Some(version)) => ClaudeCaps {
            // Forced tool use was rejected starting with 5.1.
            forced_tool_choice: version < (5, 1),
            ..ALWAYS_THINKING
        },
        (Opus, Some(version)) if version >= (5, 5) => ClaudeCaps {
            default_effort: "medium",
            ..ALWAYS_THINKING
        },
        (Opus, Some(version)) if version >= (5, 0) => DEFAULT_THINKING,
        (Sonnet, Some(version)) if version >= (5, 0) => DEFAULT_THINKING,
        (Opus, Some(version)) if version >= (4, 7) => ADAPTIVE_THINKING,
        (Opus | Sonnet, Some((4, 6))) => ADAPTIVE_THINKING_4_6,
        (Opus, Some((4, 5))) => ClaudeCaps {
            efforts: BASIC_EFFORTS,
            ..BUDGET_THINKING
        },
        (Opus, Some((4, _))) => ClaudeCaps {
            max_output: 32_000,
            ..BUDGET_THINKING
        },
        (Sonnet, Some(version)) if version >= (3, 7) => BUDGET_THINKING,
        (Haiku, Some(version)) if version >= (4, 5) => BUDGET_THINKING,
        (_, Some((3, 5))) => ClaudeCaps {
            max_output: 8_192,
            ..NO_THINKING
        },
        (_, Some((3, _))) => NO_THINKING,
        _ => return None,
    };
    Some(caps)
}

/// Parse a Claude model id into its line and `(major, minor)` version.
/// Accepts first-party ids (`claude-opus-4-5-20251101`, `claude-3-5-sonnet`),
/// dotted forms (`claude-sonnet-4.5`), and the prefixes and suffixes cloud
/// platforms add (`anthropic.claude-…-v1:0`, `claude-…@20251101`).
fn parse_claude_model(model: &str) -> Option<(ClaudeLine, Option<(u32, u32)>)> {
    let id = model
        .to_ascii_lowercase()
        .replace(['.', '@', ':', '_'], "-");
    let rest = &id[id.find("claude-")? + "claude-".len()..];
    let mut tokens = rest.split('-').filter(|token| !token.is_empty()).peekable();
    let line = |token: &str| match token {
        "opus" => Some(ClaudeLine::Opus),
        "sonnet" => Some(ClaudeLine::Sonnet),
        "haiku" => Some(ClaudeLine::Haiku),
        "fable" => Some(ClaudeLine::Fable),
        "mythos" => Some(ClaudeLine::Mythos),
        _ => None,
    };
    // Version parts are one or two digits; longer runs are date suffixes.
    let part = |token: &str| {
        (token.len() <= 2)
            .then(|| token.parse::<u32>().ok())
            .flatten()
    };

    let first = tokens.next()?;
    if let Some(line) = line(first) {
        let Some(major) = tokens.next().and_then(part) else {
            return Some((line, None));
        };
        let minor = tokens.next().and_then(part).unwrap_or(0);
        return Some((line, Some((major, minor))));
    }
    let major = part(first)?;
    let next = tokens.next()?;
    let (minor, line_token) = match part(next) {
        Some(minor) => (minor, tokens.next()?),
        None => (0, next),
    };
    Some((line(line_token)?, Some((major, minor))))
}

/// Reasoning controls for a recognized Claude model, in the shape provider
/// catalogs show. `None` for models this adapter sends no thinking to.
pub fn claude_reasoning_metadata(model: &str) -> Option<ModelReasoningMetadata> {
    let caps = claude_caps(model)?;
    let efforts = caps
        .efforts
        .iter()
        .filter_map(|level| effort_variant(level));
    let (supported_efforts, default_effort, default_enabled, mandatory) = match caps.thinking {
        ThinkingMode::Unsupported => return None,
        ThinkingMode::AdaptiveAlways => (
            efforts.collect(),
            effort_variant(caps.default_effort),
            true,
            true,
        ),
        ThinkingMode::Adaptive | ThinkingMode::AdaptiveDefault => (
            std::iter::once(ReasoningEffort::None)
                .chain(efforts)
                .collect(),
            effort_variant(caps.default_effort),
            true,
            false,
        ),
        // Manual thinking stays off unless a level is chosen.
        ThinkingMode::Budget => (
            vec![
                ReasoningEffort::None,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
            ],
            None,
            false,
            false,
        ),
    };
    Some(ModelReasoningMetadata {
        supported_efforts,
        default_effort,
        default_enabled: Some(default_enabled),
        mandatory: Some(mandatory),
    })
}

/// Default context window for a recognized Claude model.
pub fn claude_context_window(model: &str) -> Option<u32> {
    claude_caps(model).map(|caps| caps.context_window)
}

/// Default output token limit for a recognized Claude model.
pub fn claude_max_output_tokens(model: &str) -> Option<u32> {
    claude_caps(model).map(|caps| caps.max_output)
}

fn effort_variant(level: &str) -> Option<ReasoningEffort> {
    match level {
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" => Some(ReasoningEffort::Xhigh),
        "max" => Some(ReasoningEffort::Max),
        _ => None,
    }
}

/// The `thinking` config and `output_config.effort` for one request.
#[derive(Debug, PartialEq)]
struct ThinkingPlan {
    thinking: Option<Value>,
    effort: Option<&'static str>,
    /// Whether the model thinks on this request, which rules out sampling
    /// parameters and, in manual mode, forced tool use.
    enabled: bool,
}

/// Map milim's reasoning effort onto a model's thinking controls.
///
/// `Auto` and `On` leave depth to the model: adaptive thinking at the
/// model's default effort, or no thinking on a manual-thinking model unless
/// the generation settings carry a thinking budget. `None` turns thinking
/// off where the model allows it and asks for the lowest effort where it
/// does not. Named levels map to the nearest level the model accepts, never
/// above the request unless it is below every accepted level.
///
/// Manual thinking needs `budget_tokens` of at least 1,024 below
/// `max_tokens` (half of it at most, so the answer has room), and a request
/// that continues an assistant turn must keep that turn's thinking mode, so
/// `can_start_thinking` is false when the turn so far ran without it.
fn thinking_plan(
    caps: &ClaudeCaps,
    effort: ReasoningEffort,
    budget_setting: Option<u32>,
    max_tokens: u32,
    can_start_thinking: bool,
) -> ThinkingPlan {
    let level = match effort {
        ReasoningEffort::Minimal | ReasoningEffort::Low => Some(0),
        ReasoningEffort::Medium => Some(1),
        ReasoningEffort::High => Some(2),
        ReasoningEffort::Xhigh => Some(3),
        ReasoningEffort::Max => Some(4),
        ReasoningEffort::Auto | ReasoningEffort::On | ReasoningEffort::None => None,
    };
    let effort_level = level.and_then(|level| supported_effort(caps.efforts, level));
    let adaptive = || {
        let mut config = json!({ "type": "adaptive" });
        if caps.omits_thinking_text {
            config["display"] = json!("summarized");
        }
        config
    };
    let off = ThinkingPlan {
        thinking: None,
        effort: None,
        enabled: false,
    };
    match caps.thinking {
        ThinkingMode::Unsupported => off,
        ThinkingMode::AdaptiveAlways => ThinkingPlan {
            thinking: Some(adaptive()),
            effort: if effort == ReasoningEffort::None {
                caps.efforts.first().copied()
            } else {
                effort_level
            },
            enabled: true,
        },
        ThinkingMode::AdaptiveDefault if effort == ReasoningEffort::None => ThinkingPlan {
            thinking: Some(json!({ "type": "disabled" })),
            ..off
        },
        ThinkingMode::Adaptive if effort == ReasoningEffort::None => off,
        ThinkingMode::Adaptive | ThinkingMode::AdaptiveDefault => ThinkingPlan {
            thinking: Some(adaptive()),
            effort: effort_level,
            enabled: true,
        },
        ThinkingMode::Budget => {
            let target = match effort {
                ReasoningEffort::None => None,
                ReasoningEffort::Auto => budget_setting,
                ReasoningEffort::Minimal => Some(budget_setting.unwrap_or(MIN_THINKING_BUDGET)),
                ReasoningEffort::Low => Some(budget_setting.unwrap_or(4_096)),
                ReasoningEffort::Medium | ReasoningEffort::On => {
                    Some(budget_setting.unwrap_or(8_192))
                }
                ReasoningEffort::High => Some(budget_setting.unwrap_or(16_384)),
                ReasoningEffort::Xhigh => Some(budget_setting.unwrap_or(24_576)),
                ReasoningEffort::Max => Some(budget_setting.unwrap_or(32_000)),
            };
            let budget = target
                .map(|budget| budget.min(max_tokens / 2))
                .filter(|budget| *budget >= MIN_THINKING_BUDGET && can_start_thinking);
            ThinkingPlan {
                thinking: budget
                    .map(|budget| json!({ "type": "enabled", "budget_tokens": budget })),
                effort: effort_level,
                enabled: budget.is_some(),
            }
        }
    }
}

/// The highest accepted effort level at or below `requested` (an index into
/// [`EFFORT_LEVELS`]), else the lowest accepted level.
fn supported_effort(accepted: &'static [&'static str], requested: usize) -> Option<&'static str> {
    let rank = |level: &str| EFFORT_LEVELS.iter().position(|known| *known == level);
    accepted
        .iter()
        .copied()
        .rfind(|level| rank(level).is_some_and(|rank| rank <= requested))
        .or_else(|| accepted.first().copied())
}

/// Whether the request continues an assistant turn (its newest user turn
/// returns tool results) whose last assistant message did not open with a
/// thinking block. Manual extended thinking requires that opening, and a
/// turn keeps one thinking mode throughout.
fn continues_turn_without_thinking(messages: &[Value]) -> bool {
    let [.., assistant, user] = messages else {
        return false;
    };
    let returns_tool_results = user["role"] == "user"
        && user["content"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|b| block_type(b) == Some("tool_result")));
    if !returns_tool_results || assistant["role"] != "assistant" {
        return false;
    }
    let opening = assistant["content"]
        .as_array()
        .and_then(|blocks| blocks.first())
        .and_then(block_type);
    !matches!(opening, Some("thinking" | "redacted_thinking"))
}

/// The `temperature` and `top_p` a request may carry. Unrecognized models
/// get the caller's values; `top_k` is never sent.
fn sampling_params(
    sampling: &SamplingParams,
    caps: Option<&ClaudeCaps>,
    thinking: bool,
) -> (Option<f32>, Option<f32>) {
    let Some(caps) = caps else {
        return (sampling.temperature, sampling.top_p);
    };
    match caps.sampling {
        Sampling::Rejected => (None, None),
        // With thinking on, `temperature` is rejected and `top_p` is only
        // accepted between 0.95 and 1.
        _ if thinking => (
            None,
            sampling.top_p.filter(|top_p| (0.95..=1.0).contains(top_p)),
        ),
        Sampling::OneOf => (
            sampling.temperature,
            sampling.top_p.filter(|_| sampling.temperature.is_none()),
        ),
        Sampling::Accepted => (sampling.temperature, sampling.top_p),
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
        let mut retried_max_tokens = false;
        let mut stripped_thinking = false;

        // A `max_tokens` above what the model or the remaining context allows
        // is retried once with the allowed value. Replayed thinking blocks
        // the API refuses (edited history, another endpoint's signature) are
        // dropped once, so the turn continues without that reasoning.
        let resp = loop {
            let resp = self.send_messages(&body).await?;
            if resp.status() != reqwest::StatusCode::BAD_REQUEST {
                break resp;
            }
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
            match retry_with {
                Some(limit) if !retried_max_tokens => {
                    retried_max_tokens = true;
                    body["max_tokens"] = json!(limit);
                    fit_thinking_budget(&mut body);
                }
                _ if !stripped_thinking && is_thinking_replay_rejection(&failure.body) => {
                    let stripped = strip_thinking_blocks(&mut body);
                    if stripped.is_empty() {
                        return Err(failure.into_error(&self.label, "messages"));
                    }
                    remember_refused_thinking(&stripped);
                    stripped_thinking = true;
                }
                _ => return Err(failure.into_error(&self.label, "messages")),
            }
        };

        if !resp.status().is_success() {
            return Err(http_status_error(&self.label, "messages", resp).await);
        }

        let label = self.label.clone();
        let model = req.model.clone();
        let stream = async_stream::stream! {
            let mut bytes = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = AnthropicStreamState::default();
            let mut completed = false;

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
                        AnthropicLine::Done => {
                            completed = true;
                            break 'outer;
                        }
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

            if !completed {
                yield Err(Error::Other(
                    "provider stream ended before a completion event".into(),
                ));
                return;
            }
            if let Some(provider_state) = state.provider_state(&model) {
                yield Ok(StreamEvent::Delta(DeltaEvent {
                    provider_state: Some(provider_state),
                    ..Default::default()
                }));
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
    /// Completed `thinking` and `redacted_thinking` blocks, in response
    /// order, exactly as streamed.
    thinking_blocks: Vec<Value>,
    /// For each captured block, how many of the blocks milim replays (one
    /// text block, then each `tool_use`) came before it in the response.
    thinking_positions: Vec<u32>,
    /// The thinking block being streamed: content index, replay position,
    /// and the block so far.
    open_thinking: Option<(u32, u32, Value)>,
    saw_text: bool,
    tool_uses: u32,
}

impl AnthropicStreamState {
    fn replay_position(&self) -> u32 {
        u32::from(self.saw_text) + self.tool_uses
    }

    fn open_thinking_block(&mut self, index: u32, block: &Value) {
        let position = self.replay_position();
        self.open_thinking = Some((index, position, block.clone()));
    }

    /// Append a `thinking_delta` or `signature_delta` fragment to the open
    /// thinking block's `field`.
    fn extend_thinking(&mut self, index: u32, field: &str, fragment: &str) {
        let Some((open, _, block)) = self.open_thinking.as_mut() else {
            return;
        };
        if *open != index {
            return;
        }
        let mut text = block
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        text.push_str(fragment);
        block[field] = Value::String(text);
    }

    /// Keep the open thinking block when it closes, if the API signed it
    /// (a `thinking` signature or `redacted_thinking` data); unsigned
    /// blocks cannot be replayed.
    fn close_block(&mut self, index: u32) {
        if !self
            .open_thinking
            .as_ref()
            .is_some_and(|(open, _, _)| *open == index)
        {
            return;
        }
        let Some((_, position, block)) = self.open_thinking.take() else {
            return;
        };
        let signed_field = match block_type(&block) {
            Some("thinking") => "signature",
            Some("redacted_thinking") => "data",
            _ => return,
        };
        let signed = block
            .get(signed_field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty());
        if signed {
            self.thinking_blocks.push(block);
            self.thinking_positions.push(position);
        }
    }

    /// The turn's continuation data: `{"anthropic": {"model", "blocks",
    /// "positions"?}}`, with `positions` present only when a block did not
    /// open the response. `None` when the response had no thinking blocks.
    fn provider_state(&self, model: &str) -> Option<Value> {
        if self.thinking_blocks.is_empty() {
            return None;
        }
        let mut state = json!({ "model": model, "blocks": self.thinking_blocks });
        if self.thinking_positions.iter().any(|position| *position > 0) {
            state["positions"] = json!(self.thinking_positions);
        }
        Some(json!({ "anthropic": state }))
    }

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
            let index = opt_u32(&v, "index").unwrap_or(0);
            match block_type(block) {
                Some("tool_use") => {}
                Some("thinking" | "redacted_thinking") => {
                    state.open_thinking_block(index, block);
                    return AnthropicLine::Ignore;
                }
                _ => return AnthropicLine::Ignore,
            }
            state.tool_uses += 1;
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
            let index = opt_u32(&v, "index").unwrap_or(0);
            match delta_v.get("type").and_then(Value::as_str) {
                Some("text_delta") => {
                    let text = delta_v.get("text").and_then(Value::as_str);
                    if text.is_some_and(|text| !text.is_empty()) {
                        state.saw_text = true;
                    }
                    AnthropicLine::Delta(DeltaEvent {
                        content: text.map(str::to_string),
                        ..Default::default()
                    })
                }
                Some("thinking_delta") => {
                    let thinking = delta_v.get("thinking").and_then(Value::as_str);
                    state.extend_thinking(index, "thinking", thinking.unwrap_or_default());
                    AnthropicLine::Delta(DeltaEvent {
                        reasoning: thinking.map(str::to_string),
                        ..Default::default()
                    })
                }
                Some("signature_delta") => {
                    let signature = delta_v.get("signature").and_then(Value::as_str);
                    state.extend_thinking(index, "signature", signature.unwrap_or_default());
                    AnthropicLine::Ignore
                }
                Some("input_json_delta") => {
                    let mut delta = DeltaEvent::default();
                    delta.tool_calls.push(DeltaToolCall {
                        index,
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
        Some("content_block_stop") => {
            state.close_block(opt_u32(&v, "index").unwrap_or(0));
            AnthropicLine::Ignore
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
/// Assistant turns replay the thinking blocks `model` produced for them.
fn anthropic_messages(messages: &[ChatMessage], model: &str) -> (Vec<Value>, Vec<Value>) {
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
            message_blocks(msg, model)
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

fn message_blocks(msg: &ChatMessage, model: &str) -> (&'static str, Vec<Value>) {
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
    if role == "assistant" && !blocks.is_empty() {
        blocks = with_replayed_thinking(blocks, msg, model);
    }

    (role, blocks)
}

/// Put back the thinking blocks recorded in `provider_state.anthropic` for
/// this assistant message, unchanged and at their original positions among
/// the text and `tool_use` blocks. Blocks are replayed only to the model
/// that produced them; the API rejects or ignores other models' signatures.
fn with_replayed_thinking(blocks: Vec<Value>, msg: &ChatMessage, model: &str) -> Vec<Value> {
    let Some(state) = msg
        .provider_state
        .as_ref()
        .and_then(|state| state.get("anthropic"))
        .filter(|state| state.get("model").and_then(Value::as_str) == Some(model))
    else {
        return blocks;
    };
    let positions = state.get("positions").and_then(Value::as_array);
    let mut thinking = state
        .get("blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, block)| {
            matches!(block_type(block), Some("thinking" | "redacted_thinking"))
                && !is_refused_thinking(block)
        })
        .map(|(i, block)| {
            let position = positions
                .and_then(|positions| positions.get(i))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            (position, block.clone())
        })
        .peekable();

    let mut replayed = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.into_iter().enumerate() {
        while let Some((_, thought)) = thinking.next_if(|(position, _)| *position <= index as u64) {
            replayed.push(thought);
        }
        replayed.push(block);
    }
    replayed.extend(thinking.map(|(_, thought)| thought));
    replayed
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

/// Map an OpenAI-style (`"auto"`, `"none"`, `"required"`, `{type:
/// "function", …}`) or native Anthropic `tool_choice` onto Anthropic's
/// object form. A forced choice (`any`, `tool`) becomes `auto` when the
/// model or its thinking mode does not allow forcing; anything unrecognized
/// is dropped so the model default (`auto`) applies.
fn anthropic_tool_choice(choice: Option<&Value>, allow_forced: bool) -> Option<Value> {
    let choice = choice?;
    let mapped = match choice {
        Value::String(mode) => match mode.as_str() {
            "auto" => json!({ "type": "auto" }),
            "none" => json!({ "type": "none" }),
            "required" | "any" => json!({ "type": "any" }),
            _ => return None,
        },
        Value::Object(_) => match block_type(choice)? {
            "function" => {
                let name = choice
                    .pointer("/function/name")
                    .or_else(|| choice.get("name"))
                    .and_then(Value::as_str)?;
                json!({ "type": "tool", "name": name })
            }
            "required" => json!({ "type": "any" }),
            "auto" | "none" | "any" | "tool" => choice.clone(),
            _ => return None,
        },
        _ => return None,
    };
    if allow_forced || !matches!(block_type(&mapped), Some("any" | "tool")) {
        return Some(mapped);
    }
    let mut auto = json!({ "type": "auto" });
    if let Some(flag) = mapped.get("disable_parallel_tool_use") {
        auto["disable_parallel_tool_use"] = flag.clone();
    }
    Some(auto)
}

/// After `max_tokens` is lowered, keep a manual thinking budget below it,
/// or drop thinking when no valid budget fits.
fn fit_thinking_budget(body: &mut Value) {
    let max_tokens = body["max_tokens"].as_u64().unwrap_or(0);
    let Some(budget) = body
        .pointer("/thinking/budget_tokens")
        .and_then(Value::as_u64)
    else {
        return;
    };
    if budget < max_tokens {
        return;
    }
    let fitted = max_tokens / 2;
    if fitted >= u64::from(MIN_THINKING_BUDGET) {
        body["thinking"]["budget_tokens"] = json!(fitted);
    } else if let Some(body) = body.as_object_mut() {
        body.remove("thinking");
    }
}

/// Whether a 400 body rejects replayed thinking blocks: an invalid or
/// conversation-bound signature, or blocks that no longer match what the
/// model produced.
fn is_thinking_replay_rejection(body: &str) -> bool {
    let text = body.to_ascii_lowercase();
    text.contains("thinking")
        && ["signature", "cannot be modified", "different conversation"]
            .iter()
            .any(|needle| text.contains(needle))
}

/// Remove every `thinking` and `redacted_thinking` block from a request's
/// messages, dropping assistant messages left empty. Returns the removed
/// blocks.
fn strip_thinking_blocks(body: &mut Value) -> Vec<Value> {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return Vec::new();
    };
    let mut stripped = Vec::new();
    for message in messages.iter_mut() {
        let Some(content) = message
            .get_mut("content")
            .filter(|content| content.is_array())
        else {
            continue;
        };
        let Value::Array(blocks) = content.take() else {
            continue;
        };
        let (thinking, kept): (Vec<Value>, Vec<Value>) = blocks
            .into_iter()
            .partition(|block| matches!(block_type(block), Some("thinking" | "redacted_thinking")));
        // The same shape the next request builds without these blocks.
        *content = if thinking.is_empty() {
            Value::Array(kept)
        } else {
            turn_content(kept)
        };
        stripped.extend(thinking);
    }
    messages.retain(|message| {
        message["content"]
            .as_array()
            .is_none_or(|blocks| !blocks.is_empty())
    });
    stripped
}

/// Thinking blocks the API refused on replay, by signature, shared by every
/// backend in the process. Later requests leave them out instead of being
/// refused again; blocks produced after the refusal replay normally.
fn refused_thinking() -> &'static Mutex<HashSet<u64>> {
    static REFUSED: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();
    REFUSED.get_or_init(Mutex::default)
}

/// A hash of a thinking block's signature (or `redacted_thinking` data).
fn thinking_block_key(block: &Value) -> Option<u64> {
    let signed = block
        .get("signature")
        .or_else(|| block.get("data"))
        .and_then(Value::as_str)?;
    let mut hasher = DefaultHasher::new();
    signed.hash(&mut hasher);
    Some(hasher.finish())
}

fn remember_refused_thinking(blocks: &[Value]) {
    if let Ok(mut refused) = refused_thinking().lock() {
        if refused.len() > 10_000 {
            refused.clear();
        }
        refused.extend(blocks.iter().filter_map(thinking_block_key));
    }
}

fn is_refused_thinking(block: &Value) -> bool {
    thinking_block_key(block).is_some_and(|key| {
        refused_thinking()
            .lock()
            .is_ok_and(|refused| refused.contains(&key))
    })
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
        assert_eq!(default_output_tokens("claude-opus-4-8"), 64_000);
        assert_eq!(default_output_tokens("claude-sonnet-5"), 64_000);
        assert_eq!(default_output_tokens("claude-haiku-4-5"), 64_000);
        assert_eq!(default_output_tokens("claude-opus-4-1-20250805"), 32_000);
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
        assert_eq!(backend.build_body(&request)["max_tokens"], 64_000);

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
        let (system, messages) = anthropic_messages(&messages, "claude-opus-5");
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

    #[test]
    fn recognizes_claude_ids_across_platform_spellings() {
        use ClaudeLine::*;
        for (id, expected) in [
            ("claude-opus-5-5", Some((Opus, Some((5, 5))))),
            ("claude-fable-5-1", Some((Fable, Some((5, 1))))),
            ("claude-sonnet-4-20250514", Some((Sonnet, Some((4, 0))))),
            ("claude-opus-4-5-20251101", Some((Opus, Some((4, 5))))),
            ("claude-3-5-sonnet-20241022", Some((Sonnet, Some((3, 5))))),
            ("claude-3-haiku-20240307", Some((Haiku, Some((3, 0))))),
            (
                "us.anthropic.claude-haiku-4-5-20251001-v1:0",
                Some((Haiku, Some((4, 5)))),
            ),
            ("claude-opus-4-5@20251101", Some((Opus, Some((4, 5))))),
            ("anthropic/claude-sonnet-4.6", Some((Sonnet, Some((4, 6))))),
            ("claude-mythos-preview", Some((Mythos, None))),
            ("gpt-5", None),
            ("custom-sonnet-proxy", None),
        ] {
            assert_eq!(parse_claude_model(id), expected, "{id}");
        }
    }

    #[test]
    fn capability_table_covers_each_family() {
        let caps = |id: &str| claude_caps(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!(
            caps("claude-opus-5-5").thinking,
            ThinkingMode::AdaptiveAlways
        );
        assert_eq!(caps("claude-opus-5-5").default_effort, "medium");
        assert!(!caps("claude-opus-5-5").forced_tool_choice);
        assert!(!caps("claude-fable-5-1").forced_tool_choice);
        assert!(caps("claude-fable-5").forced_tool_choice);
        assert_eq!(
            caps("claude-opus-5").thinking,
            ThinkingMode::AdaptiveDefault
        );
        assert_eq!(
            caps("claude-sonnet-5").thinking,
            ThinkingMode::AdaptiveDefault
        );
        assert_eq!(caps("claude-opus-4-8").thinking, ThinkingMode::Adaptive);
        assert_eq!(caps("claude-opus-4-7").sampling, Sampling::Rejected);
        assert_eq!(caps("claude-opus-4-7").efforts, ALL_EFFORTS);
        assert_eq!(caps("claude-sonnet-4-6").efforts, EFFORTS_WITHOUT_XHIGH);
        assert_eq!(caps("claude-sonnet-4-6").sampling, Sampling::OneOf);
        assert_eq!(caps("claude-opus-4-5").thinking, ThinkingMode::Budget);
        assert_eq!(caps("claude-opus-4-5").efforts, BASIC_EFFORTS);
        assert!(caps("claude-haiku-4-5").efforts.is_empty());
        assert!(caps("claude-sonnet-4-5").efforts.is_empty());
        assert_eq!(
            caps("claude-3-5-sonnet-20241022").thinking,
            ThinkingMode::Unsupported
        );
        assert_eq!(caps("claude-haiku-4-5").context_window, 200_000);
        assert_eq!(caps("claude-sonnet-4-6").context_window, 1_000_000);
        // Unreleased versions inherit the newest known entry for their line.
        assert_eq!(caps("claude-opus-6").thinking, ThinkingMode::AdaptiveAlways);
        assert_eq!(
            caps("claude-sonnet-5-5").thinking,
            ThinkingMode::AdaptiveDefault
        );
        assert!(claude_caps("claude-sonnet").is_none());
    }

    fn family_body(model: &str, effort: Option<ReasoningEffort>) -> Value {
        let mut request = req(model, vec![ChatMessage::text("user", "hi")]);
        request.reasoning_effort = effort;
        request.sampling.temperature = Some(0.5);
        request.sampling.top_p = Some(0.9);
        AnthropicBackend::new("a", "http://families.test/v1", None).build_body(&request)
    }

    /// `(thinking, effort, temperature sent, top_p sent)` for one request.
    fn controls(body: &Value) -> (Value, Value, bool, bool) {
        (
            body.get("thinking").cloned().unwrap_or(Value::Null),
            body.pointer("/output_config/effort")
                .cloned()
                .unwrap_or(Value::Null),
            body.get("temperature").is_some(),
            body.get("top_p").is_some(),
        )
    }

    #[test]
    fn request_controls_follow_each_model_family() {
        use ReasoningEffort::*;
        let summarized = json!({ "type": "adaptive", "display": "summarized" });
        let cases = [
            // Always-thinking models: effort is the only control, never sampling.
            (
                "claude-opus-5-5",
                Some(Auto),
                summarized.clone(),
                json!(null),
                false,
                false,
            ),
            (
                "claude-opus-5-5",
                Some(None),
                summarized.clone(),
                json!("low"),
                false,
                false,
            ),
            (
                "claude-fable-5-1",
                Some(Xhigh),
                summarized.clone(),
                json!("xhigh"),
                false,
                false,
            ),
            (
                "claude-mythos-preview",
                Some(Xhigh),
                summarized.clone(),
                json!("high"),
                false,
                false,
            ),
            // Thinking on by default; `None` disables it.
            (
                "claude-opus-5",
                Some(None),
                json!({ "type": "disabled" }),
                json!(null),
                false,
                false,
            ),
            (
                "claude-sonnet-5",
                Some(High),
                summarized.clone(),
                json!("high"),
                false,
                false,
            ),
            // Thinking on request; `None` sends nothing.
            (
                "claude-opus-4-8",
                Option::None,
                summarized.clone(),
                json!(null),
                false,
                false,
            ),
            (
                "claude-opus-4-8",
                Some(None),
                json!(null),
                json!(null),
                false,
                false,
            ),
            (
                "claude-opus-4-7",
                Some(Max),
                summarized.clone(),
                json!("max"),
                false,
                false,
            ),
            // 4.6: summarized by default, no xhigh, sampling only without thinking.
            (
                "claude-sonnet-4-6",
                Some(Xhigh),
                json!({ "type": "adaptive" }),
                json!("high"),
                false,
                false,
            ),
            (
                "claude-sonnet-4-6",
                Some(None),
                json!(null),
                json!(null),
                true,
                false,
            ),
            // Manual thinking: budgets, effort only on Opus 4.5.
            (
                "claude-opus-4-5-20251101",
                Some(Max),
                json!({ "type": "enabled", "budget_tokens": 32_000 }),
                json!("high"),
                false,
                false,
            ),
            (
                "claude-haiku-4-5",
                Some(Auto),
                json!(null),
                json!(null),
                true,
                false,
            ),
            (
                "claude-haiku-4-5",
                Some(Medium),
                json!({ "type": "enabled", "budget_tokens": 8_192 }),
                json!(null),
                false,
                false,
            ),
            (
                "claude-sonnet-4-20250514",
                Some(High),
                json!({ "type": "enabled", "budget_tokens": 16_384 }),
                json!(null),
                false,
                false,
            ),
            // No thinking: everything the caller set passes through.
            (
                "claude-3-5-sonnet-20241022",
                Some(High),
                json!(null),
                json!(null),
                true,
                true,
            ),
            ("glm-4.6", Some(High), json!(null), json!(null), true, true),
        ];
        for (model, effort, thinking, level, temperature, top_p) in cases {
            assert_eq!(
                controls(&family_body(model, effort)),
                (thinking, level, temperature, top_p),
                "{model} {effort:?}"
            );
        }
    }

    #[test]
    fn manual_thinking_budget_fits_the_request() {
        let backend = AnthropicBackend::new("a", "http://budget-fit.test/v1", None);
        let mut request = req("claude-haiku-4-5", vec![ChatMessage::text("user", "hi")]);
        request.reasoning_effort = Some(ReasoningEffort::High);
        request.sampling.max_tokens = Some(4_000);
        assert_eq!(
            backend.build_body(&request)["thinking"]["budget_tokens"],
            2_000
        );
        // No valid budget fits below a small `max_tokens`.
        request.sampling.max_tokens = Some(2_000);
        assert!(backend.build_body(&request).get("thinking").is_none());

        // A thinking budget from the generation settings turns manual
        // thinking on under `Auto` and replaces the level's default.
        request.sampling.max_tokens = None;
        request.reasoning_effort = Some(ReasoningEffort::Auto);
        request.sampling.thinking_token_budget = Some(3_000);
        assert_eq!(
            backend.build_body(&request)["thinking"]["budget_tokens"],
            3_000
        );

        let mut body = json!({
            "max_tokens": 8_000,
            "thinking": { "type": "enabled", "budget_tokens": 16_384 }
        });
        fit_thinking_budget(&mut body);
        assert_eq!(body["thinking"]["budget_tokens"], 4_000);
        body["max_tokens"] = json!(1_500);
        fit_thinking_budget(&mut body);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn manual_thinking_waits_for_a_turn_that_started_without_it() {
        let messages = vec![
            ChatMessage::text("user", "Read it."),
            assistant_calls(&["call_a"]),
            tool_result("call_a", "contents"),
        ];
        let mut request = req("claude-haiku-4-5", messages);
        request.reasoning_effort = Some(ReasoningEffort::High);
        let backend = AnthropicBackend::new("a", "http://turns.test/v1", None);
        assert!(backend.build_body(&request).get("thinking").is_none());

        // The same turn with its thinking block replayed keeps thinking on.
        request.messages[1].provider_state = Some(json!({
            "anthropic": {
                "model": "claude-haiku-4-5",
                "blocks": [{ "type": "thinking", "thinking": "Plan.", "signature": "sig" }]
            }
        }));
        let body = backend.build_body(&request);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["messages"][1]["content"][0]["type"], "thinking");

        // Adaptive thinking has no such requirement.
        request.model = "claude-sonnet-4-6".to_string();
        request.messages[1].provider_state = None;
        assert_eq!(backend.build_body(&request)["thinking"]["type"], "adaptive");
    }

    #[test]
    fn maps_tool_choice_shapes_and_downgrades_forced_choices() {
        let map = |choice: Value, forced: bool| anthropic_tool_choice(Some(&choice), forced);
        assert_eq!(map(json!("auto"), true), Some(json!({ "type": "auto" })));
        assert_eq!(map(json!("none"), true), Some(json!({ "type": "none" })));
        assert_eq!(map(json!("required"), true), Some(json!({ "type": "any" })));
        assert_eq!(
            map(json!("required"), false),
            Some(json!({ "type": "auto" }))
        );
        assert_eq!(
            map(
                json!({ "type": "function", "function": { "name": "read" } }),
                true
            ),
            Some(json!({ "type": "tool", "name": "read" }))
        );
        assert_eq!(
            map(json!({ "type": "function", "name": "read" }), true),
            Some(json!({ "type": "tool", "name": "read" }))
        );
        assert_eq!(
            map(json!({ "type": "none" }), false),
            Some(json!({ "type": "none" }))
        );
        assert_eq!(
            map(
                json!({ "type": "any", "disable_parallel_tool_use": true }),
                false
            ),
            Some(json!({ "type": "auto", "disable_parallel_tool_use": true }))
        );
        assert_eq!(map(json!("sometimes"), true), None);
        assert_eq!(map(json!({ "type": "mystery" }), true), None);
    }

    #[test]
    fn forced_tool_choice_depends_on_model_and_thinking_mode() {
        let tool = Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: "read".to_string(),
                description: None,
                parameters: None,
            },
        };
        let backend = AnthropicBackend::new("a", "http://tool-choice.test/v1", None);
        let choice_for = |model: &str, effort: ReasoningEffort, tools: bool| {
            let mut request = req(model, vec![ChatMessage::text("user", "hi")]);
            request.reasoning_effort = Some(effort);
            request.tool_choice = Some(json!({
                "type": "function",
                "function": { "name": "read" }
            }));
            if tools {
                request.tools = vec![tool.clone()];
            }
            backend.build_body(&request).get("tool_choice").cloned()
        };
        let forced = Some(json!({ "type": "tool", "name": "read" }));
        let auto = Some(json!({ "type": "auto" }));
        // Adaptive thinking accepts forced tool use on most models.
        assert_eq!(
            choice_for("claude-opus-4-8", ReasoningEffort::Auto, true),
            forced
        );
        // These models reject it on every request.
        assert_eq!(
            choice_for("claude-opus-5-5", ReasoningEffort::Low, true),
            auto
        );
        assert_eq!(
            choice_for("claude-fable-5-1", ReasoningEffort::Auto, true),
            auto
        );
        // Manual thinking rejects it; without thinking it stays forced.
        assert_eq!(
            choice_for("claude-haiku-4-5", ReasoningEffort::High, true),
            auto
        );
        assert_eq!(
            choice_for("claude-haiku-4-5", ReasoningEffort::None, true),
            forced
        );
        // No tool choice without tools.
        assert_eq!(
            choice_for("claude-haiku-4-5", ReasoningEffort::None, false),
            None
        );
    }

    fn stream_lines(state: &mut AnthropicStreamState, lines: &[&str]) -> Vec<DeltaEvent> {
        lines
            .iter()
            .filter_map(|line| match parse_sse_line(line, state) {
                AnthropicLine::Delta(delta) => Some(delta),
                _ => None,
            })
            .collect()
    }

    const THINKING_TOOL_STREAM: &[&str] = &[
        r#"data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Check the \"config\"."}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" Then read."}}"#,
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBsig+A/="}}"#,
        r#"data: {"type":"content_block_stop","index":0}"#,
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"EmwKAhgBEgy3va3pzix/LafPsn4a"}}"#,
        r#"data: {"type":"content_block_stop","index":1}"#,
        r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"text","text":""}}"#,
        r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"text_delta","text":"Reading."}}"#,
        r#"data: {"type":"content_block_stop","index":2}"#,
        r#"data: {"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}"#,
        r#"data: {"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
        r#"data: {"type":"content_block_stop","index":3}"#,
        r#"data: {"type":"content_block_start","index":4,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"data: {"type":"content_block_delta","index":4,"delta":{"type":"thinking_delta","thinking":"Now the second file."}}"#,
        r#"data: {"type":"content_block_delta","index":4,"delta":{"type":"signature_delta","signature":"sig-C"}}"#,
        r#"data: {"type":"content_block_stop","index":4}"#,
        r#"data: {"type":"content_block_start","index":5,"content_block":{"type":"tool_use","id":"toolu_2","name":"read","input":{}}}"#,
        r#"data: {"type":"content_block_stop","index":5}"#,
        r#"data: {"type":"content_block_start","index":6,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
        r#"data: {"type":"content_block_delta","index":6,"delta":{"type":"thinking_delta","thinking":"cut off"}}"#,
        r#"data: {"type":"content_block_stop","index":6}"#,
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":40}}"#,
    ];

    #[test]
    fn captures_signed_thinking_blocks_in_order() {
        let mut state = AnthropicStreamState::default();
        let deltas = stream_lines(&mut state, THINKING_TOOL_STREAM);
        let reasoning: String = deltas
            .iter()
            .filter_map(|d| d.reasoning.as_deref())
            .collect();
        assert_eq!(
            reasoning,
            "Check the \"config\". Then read.Now the second file.cut off"
        );
        assert!(deltas.iter().all(|d| d.provider_state.is_none()));

        // The unsigned block at the end is not kept.
        assert_eq!(
            state.provider_state("claude-opus-5-5"),
            Some(json!({
                "anthropic": {
                    "model": "claude-opus-5-5",
                    "blocks": [
                        {
                            "type": "thinking",
                            "thinking": "Check the \"config\". Then read.",
                            "signature": "EqQBsig+A/="
                        },
                        { "type": "redacted_thinking", "data": "EmwKAhgBEgy3va3pzix/LafPsn4a" },
                        { "type": "thinking", "thinking": "Now the second file.", "signature": "sig-C" }
                    ],
                    "positions": [0, 0, 2]
                }
            }))
        );
        assert_eq!(
            AnthropicStreamState::default().provider_state("claude-opus-5-5"),
            None
        );
    }

    #[test]
    fn replays_thinking_blocks_unchanged_for_the_same_model() {
        let mut state = AnthropicStreamState::default();
        stream_lines(&mut state, THINKING_TOOL_STREAM);
        let provider_state = state.provider_state("claude-opus-5-5").unwrap();
        let expected_blocks = provider_state["anthropic"]["blocks"].clone();

        let mut assistant = assistant_calls(&["toolu_1", "toolu_2"]);
        assistant.content = Some(Content::Text("Reading.".to_string()));
        assistant.provider_state = Some(provider_state);
        let messages = vec![
            ChatMessage::text("user", "Read both."),
            assistant,
            tool_result("toolu_1", "one"),
            tool_result("toolu_2", "two"),
        ];

        let (_, replayed) = anthropic_messages(&messages, "claude-opus-5-5");
        let content = replayed[1]["content"].as_array().unwrap();
        let kinds: Vec<_> = content.iter().map(|b| block_type(b).unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "thinking",
                "redacted_thinking",
                "text",
                "tool_use",
                "thinking",
                "tool_use"
            ]
        );
        assert_eq!(content[0], expected_blocks[0]);
        assert_eq!(content[1], expected_blocks[1]);
        assert_eq!(content[4], expected_blocks[2]);
        assert_eq!(
            serde_json::to_string(&content[0]).unwrap(),
            serde_json::to_string(&expected_blocks[0]).unwrap()
        );

        // Another model never receives these signatures.
        let (_, other) = anthropic_messages(&messages, "claude-opus-5");
        assert_eq!(
            other[1]["content"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(block_type)
                .collect::<Vec<_>>(),
            ["text", "tool_use", "tool_use"]
        );
    }

    #[test]
    fn strips_replayed_thinking_after_a_signature_rejection() {
        assert!(is_thinking_replay_rejection(
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"messages.1.content.0: Invalid `signature` in `thinking` block. The block is bound to a different conversation."}}"#
        ));
        assert!(is_thinking_replay_rejection(
            "messages.1.content.0: `thinking` or `redacted_thinking` blocks in the latest assistant message cannot be modified."
        ));
        assert!(!is_thinking_replay_rejection(
            "messages: roles must alternate"
        ));

        let mut body = json!({
            "messages": [
                { "role": "user", "content": "Go." },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "Plan.", "signature": "sig" },
                    { "type": "text", "text": "Done." }
                ] },
                { "role": "assistant", "content": [
                    { "type": "redacted_thinking", "data": "abc" }
                ] },
                { "role": "user", "content": "Next." }
            ]
        });
        assert_eq!(strip_thinking_blocks(&mut body).len(), 2);
        assert_eq!(
            body["messages"],
            json!([
                { "role": "user", "content": "Go." },
                { "role": "assistant", "content": "Done." },
                { "role": "user", "content": "Next." }
            ])
        );
        assert!(strip_thinking_blocks(&mut body).is_empty());
    }

    #[test]
    fn reasoning_metadata_matches_the_capability_table() {
        use ReasoningEffort::*;
        let opus = claude_reasoning_metadata("claude-opus-5-5").unwrap();
        assert_eq!(opus.supported_efforts, [Low, Medium, High, Xhigh, Max]);
        assert_eq!(opus.default_effort, Some(Medium));
        assert_eq!(opus.mandatory, Some(true));

        let sonnet = claude_reasoning_metadata("claude-sonnet-4-6").unwrap();
        assert_eq!(sonnet.supported_efforts, [None, Low, Medium, High, Max]);
        assert_eq!(sonnet.default_effort, Some(High));
        assert_eq!(sonnet.mandatory, Some(false));

        let haiku = claude_reasoning_metadata("claude-haiku-4-5").unwrap();
        assert_eq!(haiku.supported_efforts, [None, Low, Medium, High]);
        assert_eq!(haiku.default_enabled, Some(false));

        assert!(claude_reasoning_metadata("claude-3-haiku-20240307").is_none());
        assert!(claude_reasoning_metadata("gpt-5").is_none());
        assert_eq!(claude_context_window("claude-opus-4-5"), Some(200_000));
        assert_eq!(claude_max_output_tokens("claude-opus-4-1"), Some(32_000));
    }
}
