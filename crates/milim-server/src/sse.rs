//! Adapters that turn a backend [`EventStream`] into the two streamed wire
//! formats: OpenAI SSE (`data: {chunk}` … `data: [DONE]`) and Ollama NDJSON.

use std::convert::Infallible;

use axum::response::sse::Event;
use bytes::Bytes;
use futures::{pin_mut, Stream, StreamExt};
use milim_agents::AgentEvent;

use milim_core::api::anthropic;
use milim_core::api::ollama::{OllamaChatResponse, OllamaMessage};
use milim_core::api::openai::{
    ChatCompletionChunk, ChunkChoice, Delta, DeltaToolCall, ErrorEnvelope, Usage,
};
use milim_inference::{EventStream, StreamEvent, ToolCallAccumulator};
use serde_json::json;

use crate::translate::{anthropic_stop_reason, anthropic_usage, openai_usage_value};

/// Per-response identifiers shared across every emitted chunk.
#[derive(Clone)]
pub struct ChunkCtx {
    pub id: String,
    pub created: u64,
    pub model: String,
}

impl ChunkCtx {
    fn chunk(&self, choices: Vec<ChunkChoice>, usage: Option<Usage>) -> ChatCompletionChunk {
        ChatCompletionChunk {
            id: self.id.clone(),
            object: "chat.completion.chunk".to_string(),
            created: self.created,
            model: self.model.clone(),
            choices,
            usage,
        }
    }
}

fn event(chunk: &ChatCompletionChunk) -> Event {
    // serialization of a plain DTO cannot fail; fall back to an empty object.
    let json = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".to_string());
    Event::default().data(json)
}

/// Map a backend stream into OpenAI SSE events.
pub fn openai_sse(
    mut inner: EventStream,
    ctx: ChunkCtx,
    include_usage: bool,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let mut first = true;
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(d)) => {
                    let delta = Delta {
                        role: take_role(&mut first),
                        content: d.content,
                        reasoning_content: d.reasoning,
                        reasoning: None,
                        tool_calls: non_empty(d.tool_calls),
                    };
                    let chunk = ctx.chunk(
                        vec![ChunkChoice { index: 0, delta, finish_reason: None }],
                        None,
                    );
                    yield Ok(event(&chunk));
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    let delta = Delta { role: take_role(&mut first), ..Default::default() };
                    let chunk = ctx.chunk(
                        vec![ChunkChoice { index: 0, delta, finish_reason: Some(finish_reason) }],
                        None,
                    );
                    yield Ok(event(&chunk));
                    if include_usage {
                        let uchunk = ctx.chunk(vec![], None);
                        let mut value = serde_json::to_value(&uchunk).unwrap_or_default();
                        value["usage"] = openai_usage_value(&usage);
                        yield Ok(Event::default().data(value.to_string()));
                    }
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                Err(e) => {
                    let env = ErrorEnvelope::new(e.to_string(), e.code());
                    let json = serde_json::to_string(&env).unwrap_or_default();
                    yield Ok(Event::default().event("error").data(json));
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            }
        }
        // Stream ended without an explicit Done.
        yield Ok(Event::default().data("[DONE]"));
    }
}

/// Map a backend stream into Ollama NDJSON lines.
pub fn ollama_ndjson(
    mut inner: EventStream,
    model: String,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        let mut tools = ToolCallAccumulator::default();
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(d)) => {
                    for tc in d.tool_calls {
                        tools.push(tc);
                    }
                    if let Some(content) = d.content {
                        let resp = OllamaChatResponse {
                            model: model.clone(),
                            created_at: crate::rfc3339_now(),
                            message: OllamaMessage {
                                role: "assistant".to_string(),
                                content,
                                images: None,
                                tool_calls: None,
                                thinking: None,
                            },
                            done: false,
                            done_reason: None,
                            total_duration: None,
                            prompt_eval_count: None,
                            eval_count: None,
                        };
                        yield Ok(ndjson_line(&resp));
                    }
                    if let Some(thinking) = d.reasoning {
                        let resp = OllamaChatResponse {
                            model: model.clone(),
                            created_at: crate::rfc3339_now(),
                            message: OllamaMessage {
                                role: "assistant".to_string(),
                                content: String::new(),
                                images: None,
                                tool_calls: None,
                                thinking: Some(thinking),
                            },
                            done: false,
                            done_reason: None,
                            total_duration: None,
                            prompt_eval_count: None,
                            eval_count: None,
                        };
                        yield Ok(ndjson_line(&resp));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    let calls = tools.finish();
                    let resp = OllamaChatResponse {
                        model: model.clone(),
                        created_at: crate::rfc3339_now(),
                        message: OllamaMessage {
                            role: "assistant".to_string(),
                            content: String::new(),
                            images: None,
                            tool_calls: (!calls.is_empty()).then_some(calls),
                            thinking: None,
                        },
                        done: true,
                        done_reason: Some(finish_reason),
                        total_duration: Some(0),
                        prompt_eval_count: Some(usage.prompt_tokens),
                        eval_count: Some(usage.completion_tokens),
                    };
                    yield Ok(ndjson_line(&resp));
                    return;
                }
                Err(e) => {
                    let resp = OllamaChatResponse {
                        model: model.clone(),
                        created_at: crate::rfc3339_now(),
                        message: OllamaMessage {
                            role: "assistant".to_string(),
                            content: String::new(),
                            images: None,
                            tool_calls: None,
                            thinking: None,
                        },
                        done: true,
                        done_reason: Some(format!("error: {e}")),
                        total_duration: None,
                        prompt_eval_count: None,
                        eval_count: None,
                    };
                    yield Ok(ndjson_line(&resp));
                    return;
                }
            }
        }
    }
}

/// Map a backend stream into Anthropic typed SSE events.
pub fn anthropic_sse(
    mut inner: EventStream,
    id: String,
    model: String,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        // message_start
        yield named(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": id,
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": { "input_tokens": 0, "output_tokens": 0 }
                }
            }),
        );

        let mut text_open = false;
        let mut index = 0u64;
        let mut tools = ToolCallAccumulator::default();
        let mut final_usage = anthropic::Usage::default();
        let mut stop_reason = "end_turn".to_string();

        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(d)) => {
                    for tc in d.tool_calls {
                        tools.push(tc);
                    }
                    if let Some(text) = d.content {
                        if !text_open {
                            yield named("content_block_start", json!({
                                "type": "content_block_start",
                                "index": index,
                                "content_block": { "type": "text", "text": "" }
                            }));
                            text_open = true;
                        }
                        yield named("content_block_delta", json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": { "type": "text_delta", "text": text }
                        }));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    stop_reason = anthropic_stop_reason(&finish_reason);
                    final_usage = anthropic_usage(&usage);
                    break;
                }
                Err(e) => {
                    yield named("error", json!({
                        "type": "error",
                        "error": { "type": e.code(), "message": e.to_string() }
                    }));
                    return;
                }
            }
        }

        if text_open {
            yield named("content_block_stop", json!({"type":"content_block_stop","index":index}));
            index += 1;
        }

        // Emit each tool call as its own tool_use block.
        for call in tools.finish() {
            yield named("content_block_start", json!({
                "type": "content_block_start",
                "index": index,
                "content_block": {
                    "type": "tool_use",
                    "id": call.id,
                    "name": call.function.name,
                    "input": {}
                }
            }));
            yield named("content_block_delta", json!({
                "type": "content_block_delta",
                "index": index,
                "delta": { "type": "input_json_delta", "partial_json": call.function.arguments }
            }));
            yield named("content_block_stop", json!({"type":"content_block_stop","index":index}));
            index += 1;
            stop_reason = "tool_use".to_string();
        }

        yield named("message_delta", json!({
            "type": "message_delta",
            "delta": { "stop_reason": stop_reason, "stop_sequence": null },
            "usage": final_usage
        }));
        yield named("message_stop", json!({"type":"message_stop"}));
    }
}

/// Build a named SSE event (`event:` + `data:`).
fn named(name: &str, value: serde_json::Value) -> Result<Event, Infallible> {
    Ok(Event::default().event(name).data(value.to_string()))
}

/// Map an agent-loop [`AgentEvent`] stream into SSE events, terminated by
/// `data: [DONE]`. Each event is emitted as its tagged JSON (`{"type":...}`).
pub fn agent_sse(
    inner: impl Stream<Item = AgentEvent> + Send + 'static,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        pin_mut!(inner);
        while let Some(ev) = inner.next().await {
            let terminal = matches!(&ev, AgentEvent::Done { .. } | AgentEvent::Error { .. });
            let json = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".to_string());
            yield Ok(Event::default().data(json));
            if terminal {
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        }
        yield Ok(Event::default().data("[DONE]"));
    }
}

fn ndjson_line(resp: &OllamaChatResponse) -> Bytes {
    let mut s = serde_json::to_string(resp).unwrap_or_else(|_| "{}".to_string());
    s.push('\n');
    Bytes::from(s)
}

/// On the first call, returns `Some("assistant")` and flips `first` off.
fn take_role(first: &mut bool) -> Option<String> {
    if *first {
        *first = false;
        Some("assistant".to_string())
    } else {
        None
    }
}

fn non_empty(v: Vec<DeltaToolCall>) -> Option<Vec<DeltaToolCall>> {
    (!v.is_empty()).then_some(v)
}
