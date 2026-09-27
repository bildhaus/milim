//! OpenAI's Responses API (`/v1/responses`) for OpenAI's own reasoning models.
//!
//! Chat Completions drops a reasoning model's chain of thought between tool
//! calls, and newer models no longer call functions there at all. Requests to
//! api.openai.com for the o-series and gpt-5 and later therefore use the
//! Responses API statelessly (`store: false`). Each turn's encrypted reasoning
//! items come back as `provider_state = {"openai_responses": {..}}` and are
//! replayed, in order, ahead of that turn's message and function calls.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use milim_core::api::openai::{ChatMessage, DeltaFunction, DeltaToolCall, ReasoningEffort};
use milim_core::provider_error::upstream_stream_error;
use milim_core::{Error, Result};

use crate::service::{normalize_finish_reason, CompletionRequest, DeltaEvent, StreamEvent};

/// This adapter's key in `ChatMessage::provider_state`.
const STATE_KEY: &str = "openai_responses";

/// The API's floor for `max_output_tokens`.
const MIN_OUTPUT_TOKENS: u32 = 16;

/// Whether `model` is one of OpenAI's reasoning models: the o-series,
/// `gpt-5` and later (not the `-chat` and other non-reasoning variants), and
/// `codex-*`. A routing prefix such as `openai/` is ignored.
pub(crate) fn is_openai_reasoning_model(model: &str) -> bool {
    let id = model.trim().to_ascii_lowercase();
    let id = id.rsplit('/').next().unwrap_or_default();
    if [
        "-chat",
        "-search",
        "-audio",
        "-realtime",
        "-transcribe",
        "-tts",
        "-image",
    ]
    .iter()
    .any(|variant| id.contains(variant))
    {
        return false;
    }
    let o_series = ["o1", "o3", "o4"].iter().any(|series| {
        id.strip_prefix(series)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
    });
    let gpt_major = id
        .strip_prefix("gpt-")
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|major| major.parse::<u32>().ok());
    o_series || gpt_major.is_some_and(|major| major >= 5) || id.starts_with("codex-")
}

/// Whether a 400 from the Responses API rejected `reasoning.summary`, which
/// OpenAI allows only for verified organizations.
pub(crate) fn rejects_reasoning_summary(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    body.contains("reasoning.summary") || (body.contains("verif") && body.contains("summar"))
}

#[derive(Debug, Serialize)]
struct ResponsesBody {
    model: String,
    input: Vec<Value>,
    stream: bool,
    store: bool,
    include: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Reasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    effort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'static str>,
}

/// Build the streaming `/v1/responses` body. Reasoning models reject
/// temperature and top_p, and the Responses API has no stop sequences, seed,
/// or penalties, so those sampling options are not sent.
pub(crate) fn build_body(
    req: &CompletionRequest,
    summaries: bool,
    prompt_cache_key: Option<String>,
) -> Result<Value> {
    let effort = match req.reasoning_effort {
        None | Some(ReasoningEffort::Auto | ReasoningEffort::On) => None,
        Some(effort) => Some(effort.as_str()),
    };
    let summary = summaries.then_some("auto");
    let body = ResponsesBody {
        model: req.model.clone(),
        input: input_items(&req.model, &req.messages)?,
        stream: true,
        store: false,
        include: ["reasoning.encrypted_content"],
        reasoning: (effort.is_some() || summary.is_some()).then_some(Reasoning { effort, summary }),
        max_output_tokens: req
            .sampling
            .max_tokens
            .map(|max| max.max(MIN_OUTPUT_TOKENS)),
        tools: crate::remote::responses_tools(&req.tools),
        tool_choice: req.tool_choice.as_ref().map(tool_choice),
        text: req
            .response_format
            .as_ref()
            .and_then(text_format)
            .map(|format| json!({ "format": format })),
        prompt_cache_key,
    };
    serde_json::to_value(body).map_err(Into::into)
}

fn input_items(model: &str, messages: &[ChatMessage]) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "tool" => {
                let call_id = message.tool_call_id.clone().ok_or_else(|| {
                    Error::InvalidRequest(
                        "OpenAI Responses requires tool messages to include tool_call_id"
                            .to_string(),
                    )
                })?;
                out.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": message.text_content(),
                }));
            }
            "assistant" => assistant_items(model, message, &mut out),
            role => {
                if let Some(content) = crate::remote::responses_message_content(message)? {
                    out.push(json!({ "type": "message", "role": role, "content": content }));
                }
            }
        }
    }
    Ok(out)
}

/// An assistant turn as input items: its saved reasoning items, its text (with
/// the saved `phase`), then its function calls.
fn assistant_items(model: &str, message: &ChatMessage, out: &mut Vec<Value>) {
    let text = message.text_content();
    let calls = message.tool_calls.as_deref().unwrap_or_default();
    if text.is_empty() && calls.is_empty() {
        // A reasoning item may not be replayed without the item it led to.
        return;
    }
    let state = replay_state(model, message);
    if let Some(items) = state
        .and_then(|state| state.get("items"))
        .and_then(Value::as_array)
    {
        out.extend(items.iter().cloned());
    }
    if !text.is_empty() {
        let mut item = json!({ "type": "message", "role": "assistant", "content": text });
        if let Some(phase) = state.and_then(|state| state.get("phase")) {
            item["phase"] = phase.clone();
        }
        out.push(item);
    }
    for (position, call) in calls.iter().enumerate() {
        out.push(json!({
            "type": "function_call",
            "call_id": call.id.clone().unwrap_or_else(|| format!("call_{position}")),
            "name": call.function.name,
            "arguments": call.function.arguments,
        }));
    }
}

/// This adapter's saved state for an assistant turn, when the same model
/// produced it; encrypted reasoning does not carry over between models.
fn replay_state<'a>(model: &str, message: &'a ChatMessage) -> Option<&'a Value> {
    let state = message.provider_state.as_ref()?.get(STATE_KEY)?;
    (state.get("model").and_then(Value::as_str) == Some(model)).then_some(state)
}

/// Chat Completions names a forced function as `{"function": {"name": ..}}`;
/// the Responses API takes the name at the top level.
fn tool_choice(choice: &Value) -> Value {
    match choice.pointer("/function/name") {
        Some(name) if choice.get("type").and_then(Value::as_str) == Some("function") => {
            json!({ "type": "function", "name": name })
        }
        _ => choice.clone(),
    }
}

/// Chat Completions' `response_format` as a Responses `text.format`.
fn text_format(format: &Value) -> Option<Value> {
    match format.get("type").and_then(Value::as_str)? {
        "json_schema" => {
            let mut out = format
                .get("json_schema")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            out.insert("type".to_string(), json!("json_schema"));
            Some(Value::Object(out))
        }
        _ => Some(format.clone()),
    }
}

/// Folds Responses stream events into milim stream events for one turn.
pub(crate) struct ResponsesTurn {
    label: String,
    model: String,
    /// Completed reasoning items, in output order, kept byte-exact.
    reasoning_items: Vec<Value>,
    /// The `phase` of the assistant message item, when the model set one.
    phase: Option<Value>,
    /// Function calls by output index, and whether their arguments streamed.
    calls: BTreeMap<u32, bool>,
    reasoning_started: bool,
    finished: bool,
}

impl ResponsesTurn {
    pub(crate) fn new(label: &str, model: &str) -> Self {
        Self {
            label: label.to_string(),
            model: model.to_string(),
            reasoning_items: Vec::new(),
            phase: None,
            calls: BTreeMap::new(),
            reasoning_started: false,
            finished: false,
        }
    }

    /// True once the terminal event (`response.completed` or
    /// `response.incomplete`) was handled.
    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    /// Handle one stream event.
    pub(crate) fn handle(&mut self, event: &Value) -> Result<Vec<StreamEvent>> {
        let text = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_string);
        let delta = match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta" | "response.refusal.delta") => DeltaEvent {
                content: text("delta"),
                ..Default::default()
            },
            Some("response.reasoning_summary_text.delta" | "response.reasoning_text.delta") => {
                self.reasoning_started = true;
                DeltaEvent {
                    reasoning: text("delta"),
                    ..Default::default()
                }
            }
            // Keep separate summary parts from running together.
            Some("response.reasoning_summary_part.added") if self.reasoning_started => DeltaEvent {
                reasoning: Some("\n\n".to_string()),
                ..Default::default()
            },
            Some("response.output_item.added") => self.item_added(event),
            Some("response.function_call_arguments.delta") => {
                let index = output_index(event);
                self.calls.insert(index, true);
                DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index,
                        id: None,
                        kind: None,
                        function: DeltaFunction {
                            name: None,
                            arguments: text("delta"),
                        },
                    }],
                    ..Default::default()
                }
            }
            Some("response.output_item.done") => self.item_done(event),
            Some("response.completed") => {
                let reason = if self.calls.is_empty() {
                    "stop"
                } else {
                    "tool_calls"
                };
                return Ok(self.finish(reason, event));
            }
            Some("response.incomplete") => {
                let reason = event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str);
                return Ok(self.finish(normalize_finish_reason(reason), event));
            }
            Some("response.failed") => {
                let error = event.pointer("/response/error").unwrap_or(&Value::Null);
                return Err(self.stream_error(error));
            }
            Some("error") => {
                return Err(match event.get("error").filter(|error| error.is_object()) {
                    Some(error) => self.stream_error(error),
                    // The top-level form: `{"type":"error","code":..,"message":..}`.
                    None => self.stream_error(&json!({
                        "code": event.get("code"),
                        "message": event.get("message"),
                    })),
                });
            }
            _ => DeltaEvent::default(),
        };
        Ok(if delta.is_empty() {
            Vec::new()
        } else {
            vec![StreamEvent::Delta(delta)]
        })
    }

    fn item_added(&mut self, event: &Value) -> DeltaEvent {
        let item = event.get("item").unwrap_or(&Value::Null);
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return DeltaEvent::default();
        }
        let index = output_index(event);
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .filter(|arguments| !arguments.is_empty());
        self.calls.insert(index, arguments.is_some());
        DeltaEvent {
            tool_calls: vec![function_call_delta(index, item, arguments)],
            ..Default::default()
        }
    }

    fn item_done(&mut self, event: &Value) -> DeltaEvent {
        let item = event.get("item").unwrap_or(&Value::Null);
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => self.reasoning_items.push(item.clone()),
            Some("message") => {
                if let Some(phase) = item.get("phase").filter(|phase| !phase.is_null()) {
                    self.phase = Some(phase.clone());
                }
            }
            Some("function_call") => {
                let index = output_index(event);
                let arguments = item.get("arguments").and_then(Value::as_str);
                // A call whose `added` event or argument deltas were missed
                // is completed from the finished item.
                match self.calls.insert(index, true) {
                    None => {
                        return DeltaEvent {
                            tool_calls: vec![function_call_delta(index, item, arguments)],
                            ..Default::default()
                        }
                    }
                    Some(false) if arguments.is_some_and(|a| !a.is_empty()) => {
                        return DeltaEvent {
                            tool_calls: vec![DeltaToolCall {
                                index,
                                id: None,
                                kind: None,
                                function: DeltaFunction {
                                    name: None,
                                    arguments: arguments.map(str::to_string),
                                },
                            }],
                            ..Default::default()
                        };
                    }
                    Some(_) => {}
                }
            }
            _ => {}
        }
        DeltaEvent::default()
    }

    /// The terminal events: this turn's provider state, then `Done`.
    fn finish(&mut self, finish_reason: &str, event: &Value) -> Vec<StreamEvent> {
        self.finished = true;
        let mut events = Vec::new();
        if !self.reasoning_items.is_empty() || self.phase.is_some() {
            let mut state = json!({
                "model": self.model,
                "items": std::mem::take(&mut self.reasoning_items),
            });
            if let Some(phase) = self.phase.take() {
                state["phase"] = phase;
            }
            events.push(StreamEvent::Delta(DeltaEvent {
                provider_state: Some(json!({ STATE_KEY: state })),
                ..Default::default()
            }));
        }
        // Reasoning tokens are already part of `output_tokens`.
        events.push(StreamEvent::Done {
            finish_reason: finish_reason.to_string(),
            usage: crate::remote::response_usage(event),
        });
        events
    }

    fn stream_error(&self, error: &Value) -> Error {
        let text = |key: &str| error.get(key).and_then(Value::as_str);
        upstream_stream_error(
            &self.label,
            "responses",
            None,
            text("code").or_else(|| text("type")),
            text("message").unwrap_or_default(),
        )
    }
}

fn output_index(event: &Value) -> u32 {
    event
        .get("output_index")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32
}

fn function_call_delta(index: u32, item: &Value, arguments: Option<&str>) -> DeltaToolCall {
    let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    DeltaToolCall {
        index,
        id: text("call_id").or_else(|| text("id")),
        kind: Some("function".to_string()),
        function: DeltaFunction {
            name: text("name"),
            arguments: arguments.map(str::to_string),
        },
    }
}
