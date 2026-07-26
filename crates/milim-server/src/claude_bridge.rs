//! Thin local bridge to the user's installed official `claude` CLI.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::response::sse::Event;
use futures::{Stream, StreamExt};
use milim_agents::ToolApprovalBroker;
use milim_core::api::openai::{ReasoningEffort, Usage};
use milim_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::codex_bridge::{
    AccountImage, AccountNativeWorkerLifecycle, AccountNativeWorkerState, AccountWorkerEvent,
};
use crate::privacy::Unredactor;

const CLAUDE_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const CLAUDE_MODEL_ALIASES: &[&str] = &["sonnet", "opus", "haiku", "fable"];
const CLAUDE_PROJECT_DIR_NAME_LIMIT: usize = 200;

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<AccountImage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub tool_approval_policy: Option<String>,
    #[serde(default)]
    pub tool_approval_grant: bool,
    #[serde(default)]
    pub interactive_tool_approval: bool,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub allow_session_recovery: bool,
    #[serde(default)]
    pub milim_context: Option<crate::routes::AccountRuntimeMilimContext>,
    #[serde(skip)]
    pub milim_mcp: Option<crate::routes::AccountRuntimeToolEndpoint>,
    #[serde(skip)]
    pub approval_run_id: Option<String>,
    #[serde(skip)]
    pub approval_mcp_url: Option<String>,
    #[serde(skip)]
    pub approval_mcp_authorization: Option<String>,
    #[serde(skip)]
    approval_mcp_config: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeStreamEvent {
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    RateLimit {
        limit: Value,
    },
    Done {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Warning {
        message: String,
    },
    SessionRecoveryRequired {
        message: String,
    },
    ToolApprovalRequired {
        approval_id: String,
        call_id: String,
        name: String,
        arguments: String,
        effect: milim_tools::ToolEffect,
    },
    ToolApprovalResolved {
        approval_id: String,
        call_id: String,
        decision: &'static str,
    },
}

#[derive(Debug, Clone)]
struct ClaudeToolState {
    name: String,
    detail: Option<String>,
}

pub(crate) async fn status() -> Result<Value> {
    let mut command = claude_command();
    command.arg("auth").arg("status");
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = match tokio::time::timeout(CLAUDE_STATUS_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            let error = claude_spawn_error_message(&e);
            let warning = is_cli_path_warning(&error);
            return Ok(json!({
                "available": false,
                "authenticated": false,
                "models": [],
                "error": error,
                "warning": warning
            }));
        }
        Err(_) => {
            return Ok(json!({
                "available": true,
                "authenticated": false,
                "models": [],
                "error": "`claude auth status` timed out"
            }));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let auth = serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| json!({ "raw": stdout }));
    let authenticated = output.status.success()
        && auth
            .get("loggedIn")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Ok(json!({
        "available": true,
        "authenticated": authenticated,
        "auth": auth,
        "models": if authenticated { CLAUDE_MODEL_ALIASES } else { &[] as &[&str] },
        "model_capabilities": if authenticated {
            json!(CLAUDE_MODEL_ALIASES.iter().map(|model| (model.to_string(), json!({ "image_input": true }))).collect::<serde_json::Map<_, _>>())
        } else {
            json!({})
        },
        "error": if output.status.success() { Value::Null } else { Value::String(stderr) },
    }))
}

pub(crate) fn run_stream(
    mut req: ClaudeRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let _approval_config = match ClaudeApprovalConfig::materialize(&req) {
            Ok(config) => config,
            Err(error) => {
                yield sse_event(&ClaudeStreamEvent::Error {
                    message: error.to_string(),
                    usage: None,
                    cost_usd: None,
                });
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        if let Some(config) = &_approval_config {
            req.approval_mcp_config = Some(config.path.clone());
        }
        let approval_run_id = req.approval_run_id.clone();
        let mut approval_rx = approval_broker.as_ref().map(|broker| broker.subscribe());
        let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let stream = run_stream_with_worker_events(req, redactions, Some(worker_tx));
        futures::pin_mut!(stream);
        loop {
            let (event, notice) = if let Some(receiver) = approval_rx.as_mut() {
                tokio::select! {
                    event = stream.next() => (event, None),
                    notice = receiver.recv() => (None, Some(notice)),
                }
            } else {
                (stream.next().await, None)
            };
            match (event, notice) {
                (Some(event), _) => yield event,
                (None, Some(Ok(notice))) if Some(notice.run_id.as_str()) == approval_run_id.as_deref() => {
                    let call_id = notice.call_id.unwrap_or_else(|| notice.approval_id.clone());
                    if let Some(decision) = notice.decision {
                        yield sse_event(&ClaudeStreamEvent::ToolApprovalResolved {
                            approval_id: notice.approval_id,
                            call_id,
                            decision,
                        });
                    } else {
                        yield sse_event(&ClaudeStreamEvent::ToolApprovalRequired {
                            approval_id: notice.approval_id,
                            call_id,
                            name: notice.name,
                            arguments: notice.arguments,
                            effect: notice.effect,
                        });
                    }
                }
                (None, Some(Ok(_)))
                | (None, Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(_)))) => {}
                (None, Some(Err(tokio::sync::broadcast::error::RecvError::Closed))) => approval_rx = None,
                (None, None) => break,
            }
            while let Ok(worker) = worker_rx.try_recv() {
                if matches!(worker, AccountWorkerEvent::NativeWorker { .. }) {
                    yield sse_event(&worker);
                }
            }
        }
    }
}

struct ClaudeApprovalConfig {
    path: PathBuf,
}

impl ClaudeApprovalConfig {
    fn materialize(req: &ClaudeRunRequest) -> Result<Option<Self>> {
        let needs_approval = req.interactive_tool_approval
            && account_runtime_policy(req.tool_approval_policy.as_deref()) == "review";
        if !needs_approval && req.milim_mcp.is_none() {
            return Ok(None);
        }
        let path = std::env::temp_dir().join(format!(
            "milim-claude-approval-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut servers = serde_json::Map::new();
        if let Some(endpoint) = &req.milim_mcp {
            servers.insert(
                "milim".into(),
                json!({
                    "type": "http",
                    "url": endpoint.url,
                    "headers": { "Authorization": endpoint.authorization }
                }),
            );
        }
        if needs_approval {
            let url = req.approval_mcp_url.as_deref().ok_or_else(|| {
                Error::InvalidRequest(
                    "Claude Review mode requires Milim's interactive approval endpoint".to_string(),
                )
            })?;
            let mut server = json!({ "type": "http", "url": url });
            if let Some(authorization) = clean_optional(req.approval_mcp_authorization.as_deref()) {
                server["headers"] = json!({ "Authorization": authorization });
            }
            servers.insert("milim_approval".into(), server);
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({ "mcpServers": servers }))?,
        )
        .map_err(|error| {
            Error::Other(format!(
                "failed to create Claude approval configuration: {error}"
            ))
        })?;
        Ok(Some(Self { path }))
    }
}

impl Drop for ClaudeApprovalConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run_stream_with_worker_events(
    req: ClaudeRunRequest,
    redactions: BTreeMap<String, String>,
    worker_events: Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let mut retried_locked_session = false;
        loop {
            let mut command = claude_command();
            for arg in claude_run_args(&req) {
                command.arg(arg);
            }
            if worker_events.is_some() {
                for denied in ["Agent", "Task"] {
                    command.arg("--disallowedTools").arg(denied);
                }
            }
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            if let Some(cwd) = clean_optional(req.cwd.as_deref()) {
                command.current_dir(cwd);
            }
            #[cfg(windows)]
            command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    let message = claude_spawn_error_message(&e);
                    if is_cli_path_warning(&message) {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Warning { message }, &worker_events);
                    } else {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                            message,
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            };
            let Some(mut stdin) = child.stdin.take() else {
                yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                    message: "claude stdin was not available".to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            };
            let input = match claude_stream_input(&req) {
                Ok(input) => input,
                Err(e) => {
                    yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                        message: e.to_string(),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            };
            if let Err(e) = stdin.write_all(format!("{input}\n").as_bytes()).await {
                yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                    message: format!("failed to send Claude multimodal input: {e}"),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
            let _ = stdin.shutdown().await;
            let Some(stdout) = child.stdout.take() else {
                yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                    message: "claude stdout was not available".to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            };
            let stderr = child.stderr.take();
            let stderr_task = tokio::spawn(async move {
                let mut text = String::new();
                if let Some(stderr) = stderr {
                    let _ = BufReader::new(stderr).read_to_string(&mut text).await;
                }
                text
            });

            let mut lines = BufReader::new(stdout).lines();
            let mut content = Unredactor::new(redactions.clone());
            let mut reasoning = Unredactor::new(redactions.clone());
            let mut tools = BTreeMap::new();
            let mut saw_terminal_event = false;
            let mut locked_session_error = None;

            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&line) {
                            if let Some(lifecycle) = claude_native_worker_event(&value) {
                                publish_worker(&worker_events, AccountWorkerEvent::NativeWorker { lifecycle });
                            }
                        }
                        for event in handle_line(&line, &mut content, &mut reasoning, &mut tools, &mut saw_terminal_event) {
                            if matches!(&event, ClaudeStreamEvent::Error { message, .. } if claude_session_in_use_error(message) && !retried_locked_session)
                            {
                                if let ClaudeStreamEvent::Error { message, .. } = event {
                                    locked_session_error = Some(message);
                                }
                            } else {
                                yield sse_event_with_worker(&event, &worker_events);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                            message: format!("claude stream failed: {e}"),
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                        yield Ok(Event::default().data("[DONE]"));
                        return;
                    }
                }

            }

            let status = child.wait().await;
            let stderr = stderr_task.await.unwrap_or_default();
            let tail = content.flush();
            if !tail.is_empty() {
                yield sse_event_with_worker(&ClaudeStreamEvent::Token { text: tail }, &worker_events);
            }
            let rtail = reasoning.flush();
            if !rtail.is_empty() {
                yield sse_event_with_worker(&ClaudeStreamEvent::Reasoning { text: rtail }, &worker_events);
            }

            match status {
                Ok(status) if status.success() => {
                    if let Some(message) = locked_session_error {
                        if maybe_recover_locked_session(&req, &mut retried_locked_session).await {
                            yield sse_event_with_worker(&ClaudeStreamEvent::Warning {
                                message: "Claude session was already in use; Milim stopped the matching local Claude CLI process and retried.".to_string(),
                            }, &worker_events);
                            continue;
                        }
                        yield locked_session_error_event(&req, message);
                    } else if !saw_terminal_event {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Done {
                            status: "completed".to_string(),
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    break;
                }
                Ok(status) => {
                    let message = locked_session_error
                        .unwrap_or_else(|| first_error(&stderr, &format!("claude exited with {status}")));
                    if claude_session_in_use_error(&message)
                        && maybe_recover_locked_session(&req, &mut retried_locked_session).await
                    {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Warning {
                            message: "Claude session was already in use; Milim stopped the matching local Claude CLI process and retried.".to_string(),
                        }, &worker_events);
                        continue;
                    }
                    if claude_session_in_use_error(&message) {
                        yield locked_session_error_event(&req, message);
                    } else {
                        yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                            message,
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    break;
                }
                Err(e) => {
                    yield sse_event_with_worker(&ClaudeStreamEvent::Error {
                        message: format!("claude exit status failed: {e}"),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    break;
                }
            }
        }
        yield Ok(Event::default().data("[DONE]"));
    }
}

async fn maybe_recover_locked_session(
    req: &ClaudeRunRequest,
    retried_locked_session: &mut bool,
) -> bool {
    if *retried_locked_session || !req.allow_session_recovery {
        return false;
    }
    let Some(session_id) = clean_optional(req.session_id.as_deref()) else {
        return false;
    };
    if terminate_claude_session_processes(&session_id).await {
        *retried_locked_session = true;
        true
    } else {
        false
    }
}

fn locked_session_error_event(
    req: &ClaudeRunRequest,
    message: String,
) -> std::result::Result<Event, Infallible> {
    if req.allow_session_recovery {
        return sse_event(&ClaudeStreamEvent::Error {
            message,
            usage: None,
            cost_usd: None,
        });
    }
    sse_event(&ClaudeStreamEvent::SessionRecoveryRequired {
        message: format!(
            "This Claude session appears to be in use by another Claude CLI process. Milim can try to stop the matching local Claude process and retry, or you can cancel and resume manually. Claude reported: {message}"
        ),
    })
}

fn handle_line(
    line: &str,
    content: &mut Unredactor,
    reasoning: &mut Unredactor,
    tools: &mut BTreeMap<String, ClaudeToolState>,
    saw_terminal_event: &mut bool,
) -> Vec<ClaudeStreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if value.get("type").and_then(Value::as_str) == Some("stream_event") {
        let event = value.get("event").unwrap_or(&Value::Null);
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let block = event.get("content_block").unwrap_or(&Value::Null);
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    if let Some(event) = claude_tool_start_event(block, tools) {
                        out.push(event);
                    }
                }
            }
            Some("content_block_delta") => {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            let text = content.push(text);
                            if !text.is_empty() {
                                out.push(ClaudeStreamEvent::Token { text });
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            let text = reasoning.push(text);
                            if !text.is_empty() {
                                out.push(ClaudeStreamEvent::Reasoning { text });
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    } else if value.get("type").and_then(Value::as_str) == Some("user") {
        if let Some(blocks) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let Some(id) = string_field(block, "tool_use_id") else {
                    continue;
                };
                let Some(tool) = tools.remove(&id) else {
                    continue;
                };
                out.push(claude_tool_end_event(
                    id,
                    tool,
                    block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ));
            }
        }
    } else if value.get("type").and_then(Value::as_str) == Some("result") {
        *saw_terminal_event = true;
        let tail = content.flush();
        if !tail.is_empty() {
            out.push(ClaudeStreamEvent::Token { text: tail });
        }
        let rtail = reasoning.flush();
        if !rtail.is_empty() {
            out.push(ClaudeStreamEvent::Reasoning { text: rtail });
        }
        let status = value
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_string();
        let is_error = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.extend(close_claude_tools(tools, is_error));
        let usage = usage_from_claude_result(&value);
        let cost_usd = value.get("total_cost_usd").and_then(Value::as_f64);
        if is_error {
            let message = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("claude run failed")
                .to_string();
            out.push(ClaudeStreamEvent::Error {
                message,
                usage,
                cost_usd,
            });
        } else {
            out.push(ClaudeStreamEvent::Done {
                status,
                usage,
                cost_usd,
            });
        }
    } else if value.get("type").and_then(Value::as_str) == Some("rate_limit_event") {
        out.push(ClaudeStreamEvent::RateLimit {
            limit: claude_rate_limit(value.get("rate_limit_info").unwrap_or(&Value::Null)),
        });
    } else if value.get("type").and_then(Value::as_str) == Some("tool_progress") {
        if let Some(event) = claude_tool_progress_event(&value, tools) {
            out.push(event);
        }
    }
    out
}

fn claude_tool_start_event(
    block: &Value,
    tools: &mut BTreeMap<String, ClaudeToolState>,
) -> Option<ClaudeStreamEvent> {
    let id = string_field(block, "id")?;
    let name = string_field(block, "name").unwrap_or_else(|| "tool".to_string());
    if tools.contains_key(&id) {
        return None;
    }
    let detail = compact_json(block.get("input"));
    tools.insert(
        id.clone(),
        ClaudeToolState {
            name: name.clone(),
            detail: detail.clone(),
        },
    );
    Some(ClaudeStreamEvent::Tool {
        id,
        name: name.clone(),
        status: "running".to_string(),
        label: Some(format!("Using {name}")),
        detail,
        icon: Some(claude_tool_icon(&name).to_string()),
    })
}

fn claude_tool_progress_event(
    value: &Value,
    tools: &mut BTreeMap<String, ClaudeToolState>,
) -> Option<ClaudeStreamEvent> {
    let id = string_field(value, "tool_use_id")?;
    let name = string_field(value, "tool_name").unwrap_or_else(|| "tool".to_string());
    if tools.contains_key(&id) {
        return None;
    }
    tools.insert(
        id.clone(),
        ClaudeToolState {
            name: name.clone(),
            detail: None,
        },
    );
    Some(ClaudeStreamEvent::Tool {
        id,
        name: name.clone(),
        status: "running".to_string(),
        label: Some(format!("Using {name}")),
        detail: None,
        icon: Some(claude_tool_icon(&name).to_string()),
    })
}

fn close_claude_tools(
    tools: &mut BTreeMap<String, ClaudeToolState>,
    is_error: bool,
) -> Vec<ClaudeStreamEvent> {
    std::mem::take(tools)
        .into_iter()
        .map(|(id, tool)| claude_tool_end_event(id, tool, is_error))
        .collect()
}

fn claude_tool_end_event(id: String, tool: ClaudeToolState, is_error: bool) -> ClaudeStreamEvent {
    ClaudeStreamEvent::Tool {
        id,
        name: tool.name.clone(),
        status: if is_error { "error" } else { "done" }.to_string(),
        label: Some(if is_error {
            format!("{} failed", tool.name)
        } else {
            format!("Used {}", tool.name)
        }),
        detail: tool.detail,
        icon: Some(claude_tool_icon(&tool.name).to_string()),
    }
}

fn claude_tool_icon(name: &str) -> &'static str {
    match name {
        "Read" | "Grep" | "Glob" | "Write" | "Edit" | "MultiEdit" => "file",
        "Bash" => "command",
        _ => "tool",
    }
}

fn sse_event<T: Serialize>(value: &T) -> std::result::Result<Event, Infallible> {
    Ok(Event::default().data(serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())))
}

fn sse_event_with_worker(
    value: &ClaudeStreamEvent,
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
) -> std::result::Result<Event, Infallible> {
    if let Some(event) = account_worker_event_from_claude(value) {
        publish_worker(worker_events, event);
    }
    sse_event(value)
}

fn publish_worker(
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
    event: AccountWorkerEvent,
) {
    if let Some(worker_events) = worker_events {
        let _ = worker_events.send(event);
    }
}

fn account_worker_event_from_claude(value: &ClaudeStreamEvent) -> Option<AccountWorkerEvent> {
    match value {
        ClaudeStreamEvent::Token { text } => Some(AccountWorkerEvent::Token { text: text.clone() }),
        ClaudeStreamEvent::Reasoning { text } => {
            Some(AccountWorkerEvent::Reasoning { text: text.clone() })
        }
        ClaudeStreamEvent::Tool {
            id,
            name,
            status,
            label,
            detail,
            icon,
        } => Some(AccountWorkerEvent::Tool {
            id: id.clone(),
            name: name.clone(),
            status: status.clone(),
            label: label.clone(),
            detail: detail.clone(),
            icon: icon.clone(),
        }),
        ClaudeStreamEvent::Done {
            status,
            usage,
            cost_usd,
        } => Some(AccountWorkerEvent::Done {
            status: status.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        ClaudeStreamEvent::Error {
            message,
            usage,
            cost_usd,
        } => Some(AccountWorkerEvent::Error {
            message: message.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        ClaudeStreamEvent::Warning { message }
        | ClaudeStreamEvent::SessionRecoveryRequired { message } => {
            Some(AccountWorkerEvent::Warning {
                message: message.clone(),
            })
        }
        ClaudeStreamEvent::RateLimit { .. }
        | ClaudeStreamEvent::ToolApprovalRequired { .. }
        | ClaudeStreamEvent::ToolApprovalResolved { .. } => None,
    }
}

fn first_error(stderr: &str, fallback: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn claude_session_in_use_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("session id") && lower.contains("already in use")
}

fn safe_session_id_for_process_match(session_id: &str) -> Option<&str> {
    let session_id = session_id.trim();
    if session_id.len() < 8 || session_id.len() > 128 {
        return None;
    }
    if session_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        Some(session_id)
    } else {
        None
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn process_matches_claude_session(name: &str, command_line: &str, session_id: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    matches!(name.as_str(), "claude" | "claude.exe" | "node" | "node.exe")
        && command_line.contains(session_id)
}

async fn terminate_claude_session_processes(session_id: &str) -> bool {
    let Some(session_id) = safe_session_id_for_process_match(session_id) else {
        return false;
    };
    let killed_from_registry = terminate_claude_registry_session_process(session_id).await;
    let killed_from_command_line = terminate_claude_session_processes_impl(session_id).await;
    killed_from_registry || killed_from_command_line
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeSessionRegistryEntry {
    pid: u32,
    path: PathBuf,
}

async fn terminate_claude_registry_session_process(session_id: &str) -> bool {
    let Some(entry) = find_claude_session_registry_entry(session_id) else {
        return false;
    };
    let killed = terminate_process_id(entry.pid).await;
    if killed {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    killed
}

fn find_claude_session_registry_entry(session_id: &str) -> Option<ClaudeSessionRegistryEntry> {
    for dir in claude_session_registry_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let Some(entry) = claude_session_registry_entry_from_value(&value, path) else {
                continue;
            };
            if value.get("sessionId").and_then(Value::as_str) == Some(session_id) {
                return Some(entry);
            }
        }
    }
    None
}

fn claude_session_registry_entry_from_value(
    value: &Value,
    path: PathBuf,
) -> Option<ClaudeSessionRegistryEntry> {
    let pid = value
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())?;
    let session_id = value.get("sessionId").and_then(Value::as_str)?.trim();
    safe_session_id_for_process_match(session_id)?;
    Some(ClaudeSessionRegistryEntry { pid, path })
}

fn claude_session_registry_dirs() -> Vec<PathBuf> {
    claude_home_dirs()
        .into_iter()
        .map(|dir| dir.join("sessions"))
        .collect()
}

#[cfg(windows)]
async fn terminate_process_id(pid: u32) -> bool {
    let mut command = Command::new("taskkill");
    command.arg("/PID").arg(pid.to_string()).arg("/F");
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    command
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
async fn terminate_process_id(pid: u32) -> bool {
    Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
async fn terminate_claude_session_processes_impl(session_id: &str) -> bool {
    let current_pid = std::process::id();
    let script = format!(
        "$sid = '{session_id}'; $self = {current_pid}; Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne $self -and $_.CommandLine -like \"*$sid*\" -and ($_.Name -ieq 'claude.exe' -or $_.Name -ieq 'claude' -or $_.Name -ieq 'node.exe' -or $_.Name -ieq 'node') }} | ForEach-Object {{ Write-Output $_.ProcessId; Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"
    );
    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script);
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    match command.output().await {
        Ok(output) if output.status.success() => {
            let killed = !String::from_utf8_lossy(&output.stdout).trim().is_empty();
            if killed {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            killed
        }
        _ => false,
    }
}

#[cfg(not(windows))]
async fn terminate_claude_session_processes_impl(session_id: &str) -> bool {
    let output = match Command::new("ps")
        .arg("-axo")
        .arg("pid=,comm=,args=")
        .output()
        .await
    {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };
    let current_pid = std::process::id();
    let mut killed = false;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim_start();
        let Some((pid_text, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_text.trim().parse::<u32>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let rest = rest.trim_start();
        let Some((name, command_line)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        if !process_matches_claude_session(name, command_line, session_id) {
            continue;
        }
        if Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
        {
            killed = true;
        }
    }
    if killed {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    killed
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn claude_native_worker_event(value: &Value) -> Option<AccountNativeWorkerLifecycle> {
    if value.get("type").and_then(Value::as_str) != Some("system") {
        return None;
    }
    let subtype = value.get("subtype").and_then(Value::as_str)?;
    let status = match subtype {
        "agent_started" => "running",
        "agent_completed" => "completed",
        "agent_failed" => "error",
        "agent_stopped" => "stopped",
        _ => return None,
    };
    let agent_id = string_field(value, "agent_id")?;
    Some(AccountNativeWorkerLifecycle {
        runtime: "claude".to_string(),
        call_id: string_field(value, "tool_use_id").unwrap_or_else(|| agent_id.clone()),
        operation: "native_agent".to_string(),
        status: status.to_string(),
        parent_runtime_id: string_field(value, "parent_agent_id"),
        worker_runtime_ids: vec![agent_id.clone()],
        workers: vec![AccountNativeWorkerState {
            runtime_id: agent_id,
            status: status.to_string(),
            message: string_field(value, "message"),
        }],
        prompt: string_field(value, "prompt").or_else(|| string_field(value, "description")),
        model: string_field(value, "model"),
    })
}

fn claude_run_args(req: &ClaudeRunRequest) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
    ];
    if let Some(session_id) = clean_optional(req.session_id.as_deref()) {
        if claude_project_session_exists(req, &session_id) {
            args.extend(["--resume".to_string(), session_id]);
        } else {
            args.extend(["--session-id".to_string(), session_id]);
        }
    } else {
        args.push("--no-session-persistence".to_string());
    }
    args.extend([
        "--permission-mode".to_string(),
        claude_permission_mode(req).to_string(),
    ]);
    if let Some(path) = &req.approval_mcp_config {
        args.extend([
            "--mcp-config".to_string(),
            path.to_string_lossy().into_owned(),
        ]);
        if req.milim_mcp.is_some() {
            args.extend(["--allowedTools".to_string(), "mcp__milim__*".to_string()]);
        }
        if req.approval_mcp_url.is_some() {
            args.extend([
                "--permission-prompt-tool".to_string(),
                "mcp__milim_approval__request_tool_approval".to_string(),
            ]);
        }
    }
    for denied in claude_denied_tools(req) {
        args.extend(["--disallowedTools".to_string(), denied.to_string()]);
    }
    if let Some(model) = clean_optional(req.model.as_deref()) {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(effort) = claude_effort(req.reasoning_effort) {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    args
}

fn claude_stream_input(req: &ClaudeRunRequest) -> Result<Value> {
    let mut content = Vec::new();
    if !req.prompt.trim().is_empty() {
        content.push(json!({ "type": "text", "text": req.prompt }));
    }
    for image in &req.images {
        if !matches!(
            image.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) || image.data.is_empty()
        {
            return Err(Error::InvalidRequest(
                "Claude images must contain PNG, JPEG, WebP, or GIF base64 data".to_string(),
            ));
        }
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": image.data,
            }
        }));
    }
    Ok(json!({
        "type": "user",
        "session_id": "",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": Value::Null,
    }))
}

fn claude_project_session_exists(req: &ClaudeRunRequest, session_id: &str) -> bool {
    let Some(session_id) = safe_session_id_for_process_match(session_id) else {
        return false;
    };
    let Some(cwd) = claude_session_cwd(req) else {
        return false;
    };
    for projects_dir in claude_projects_dirs() {
        let project_name = claude_project_dir_name(cwd.to_string_lossy().as_ref());
        let session_file = projects_dir
            .join(&project_name)
            .join(format!("{session_id}.jsonl"));
        if session_file.is_file() {
            return true;
        }
        if project_name.len() <= CLAUDE_PROJECT_DIR_NAME_LIMIT {
            continue;
        }
        let prefix = format!("{}-", &project_name[..CLAUDE_PROJECT_DIR_NAME_LIMIT]);
        let Ok(entries) = std::fs::read_dir(&projects_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix)
                && entry.path().join(format!("{session_id}.jsonl")).is_file()
            {
                return true;
            }
        }
    }
    false
}

fn claude_session_cwd(req: &ClaudeRunRequest) -> Option<PathBuf> {
    if let Some(cwd) = clean_optional(req.cwd.as_deref()) {
        return Some(normalize_claude_cwd(Path::new(&cwd)));
    }
    std::env::current_dir()
        .ok()
        .map(|path| normalize_claude_cwd(&path))
}

fn normalize_claude_cwd(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn claude_project_dir_name(cwd: &str) -> String {
    let normalized: String = cwd
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    if normalized.len() <= CLAUDE_PROJECT_DIR_NAME_LIMIT {
        return normalized;
    }
    normalized
}

fn claude_projects_dirs() -> Vec<PathBuf> {
    claude_home_dirs()
        .into_iter()
        .map(|dir| dir.join("projects"))
        .collect()
}

fn claude_home_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(profile).join(".claude"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let dir = PathBuf::from(home).join(".claude");
        if !dirs.iter().any(|existing| existing == &dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn account_runtime_policy(value: Option<&str>) -> &str {
    match value.map(str::trim) {
        Some("review") => "review",
        Some("open") => "open",
        _ => "guarded",
    }
}

fn claude_tools_allowed(req: &ClaudeRunRequest) -> bool {
    !req.plan_mode
        && match account_runtime_policy(req.tool_approval_policy.as_deref()) {
            "review" => req.tool_approval_grant || req.interactive_tool_approval,
            _ => true,
        }
}

fn claude_permission_mode(req: &ClaudeRunRequest) -> &'static str {
    if req.plan_mode {
        "plan"
    } else if req.interactive_tool_approval
        && account_runtime_policy(req.tool_approval_policy.as_deref()) == "review"
    {
        "manual"
    } else if !claude_tools_allowed(req)
        || account_runtime_policy(req.tool_approval_policy.as_deref()) == "guarded"
    {
        "dontAsk"
    } else {
        "bypassPermissions"
    }
}

fn claude_denied_tools(req: &ClaudeRunRequest) -> Vec<&'static str> {
    if req.plan_mode {
        Vec::new()
    } else if !claude_tools_allowed(req) {
        vec!["*"]
    } else if account_runtime_policy(req.tool_approval_policy.as_deref()) == "guarded" {
        vec!["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"]
    } else {
        Vec::new()
    }
}

fn claude_effort(effort: Option<ReasoningEffort>) -> Option<&'static str> {
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

fn usage_from_claude_result(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let prompt = opt_u32(usage, "input_tokens").unwrap_or(0)
        + opt_u32(usage, "cache_creation_input_tokens").unwrap_or(0)
        + opt_u32(usage, "cache_read_input_tokens").unwrap_or(0);
    let completion = opt_u32(usage, "output_tokens").unwrap_or(0);
    if prompt == 0 && completion == 0 {
        return None;
    }
    Some(Usage::new(prompt, completion))
}

fn claude_rate_limit(info: &Value) -> Value {
    json!({
        "provider": "Local Claude CLI",
        "status": info.get("status").and_then(Value::as_str),
        "kind": info.get("rateLimitType").and_then(Value::as_str),
        "reset_at": info.get("resetsAt").and_then(Value::as_i64),
        "raw": info,
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn compact_json(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(compact(&value.to_string(), 110))
}

fn compact(value: &str, limit: usize) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > limit {
        let prefix = text
            .chars()
            .take(limit.saturating_sub(3))
            .collect::<String>();
        format!("{prefix}...")
    } else {
        text
    }
}

fn claude_spawn_error_message(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        return cli_path_warning("Claude CLI", "claude");
    }
    format!("failed to start `claude`: {error}. Install Anthropic's official Claude CLI and sign in with `claude auth login`.")
}

fn cli_path_warning(label: &str, command: &str) -> String {
    format!("{label} CLI was not found on PATH. If this is macOS, apps launched from Finder or Dock may not inherit your shell PATH, so Milim may not see `{command}` even when Terminal can. Launch Milim from a terminal or add the CLI install folder to PATH for GUI apps.")
}

fn is_cli_path_warning(message: &str) -> bool {
    message.contains("CLI was not found on PATH")
}

fn opt_u32(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn claude_command() -> Command {
    Command::new("claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_delta() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn text_delta_does_not_close_open_tool() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let started = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"README.md"}}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(
            matches!(started.first(), Some(ClaudeStreamEvent::Tool { status, label: Some(label), .. }) if status == "running" && label == "Using Read")
        );

        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"I'll continue"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(
            matches!(events.first(), Some(ClaudeStreamEvent::Token { text }) if text == "I'll continue")
        );
        assert!(tools.contains_key("toolu_1"));

        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"still working"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Reasoning { text }) if text == "still working"
        ));
        assert!(tools.contains_key("toolu_1"));
    }

    #[test]
    fn tool_results_close_only_the_matching_call() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::from([
            (
                "toolu_1".to_string(),
                ClaudeToolState {
                    name: "Read".to_string(),
                    detail: None,
                },
            ),
            (
                "toolu_2".to_string(),
                ClaudeToolState {
                    name: "Edit".to_string(),
                    detail: None,
                },
            ),
        ]);
        let mut done = false;
        let success = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#;
        let failure = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"failed","is_error":true}]}}"#;

        let events = handle_line(success, &mut content, &mut reasoning, &mut tools, &mut done);
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_1" && status == "done"
        ));
        assert_eq!(events.len(), 1);
        assert!(tools.contains_key("toolu_2"));

        let events = handle_line(failure, &mut content, &mut reasoning, &mut tools, &mut done);
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_2" && status == "error"
        ));
        assert!(tools.is_empty());
        assert!(
            handle_line(failure, &mut content, &mut reasoning, &mut tools, &mut done,).is_empty()
        );
    }

    #[test]
    fn final_result_closes_tools_without_structured_results() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::from([(
            "toolu_1".to_string(),
            ClaudeToolState {
                name: "Read".to_string(),
                detail: None,
            },
        )]);
        let mut done = false;
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );

        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_1" && status == "done"
        ));
        assert!(matches!(
            events.last(),
            Some(ClaudeStreamEvent::Done { .. })
        ));
        assert!(tools.is_empty());
    }

    #[test]
    fn flushes_before_result_done() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        assert!(handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"["}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        )
        .is_empty());
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(events.first(), Some(ClaudeStreamEvent::Token { text }) if text == "["));
        assert!(
            matches!(events.last(), Some(ClaudeStreamEvent::Done { status, .. }) if status == "end_turn")
        );
    }

    #[test]
    fn parses_result_usage_and_limit_events() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","total_cost_usd":0.01,"usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":4}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.last(),
            Some(ClaudeStreamEvent::Done {
                usage: Some(Usage { prompt_tokens: 15, completion_tokens: 4, total_tokens: 19 }),
                cost_usd: Some(cost),
                ..
            }) if (*cost - 0.01).abs() < f64::EPSILON
        ));

        let events = handle_line(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1782660000,"rateLimitType":"five_hour"}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::RateLimit { .. })
        ));
    }

    #[test]
    fn status_models_require_authentication() {
        assert!(CLAUDE_MODEL_ALIASES.contains(&"sonnet"));
        assert!(CLAUDE_MODEL_ALIASES.contains(&"haiku"));
    }

    #[test]
    fn maps_supported_effort_flags() {
        assert_eq!(claude_effort(Some(ReasoningEffort::Low)), Some("low"));
        assert_eq!(claude_effort(Some(ReasoningEffort::Xhigh)), Some("xhigh"));
        assert_eq!(claude_effort(Some(ReasoningEffort::Minimal)), None);
        assert_eq!(claude_effort(Some(ReasoningEffort::Auto)), None);
    }

    #[test]
    fn persistent_run_args_use_session_id_without_turn_cap() {
        let args = claude_run_args(&ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: Some("sonnet".into()),
            cwd: None,
            reasoning_effort: Some(ReasoningEffort::High),
            session_id: Some("11111111-1111-4111-8111-111111111111".into()),
            tool_approval_policy: Some("open".into()),
            tool_approval_grant: true,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        });
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--session-id"
                && pair[1] == "11111111-1111-4111-8111-111111111111"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--effort" && pair[1] == "high"));
        assert!(!args.iter().any(|arg| arg == "--max-turns"));
        assert!(!args.iter().any(|arg| arg == "--no-session-persistence"));

        let args = claude_run_args(&ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: None,
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: None,
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        });
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(!args.iter().any(|arg| arg == "--max-turns"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--input-format", "stream-json"]));
    }

    #[test]
    fn claude_stream_input_preserves_image_only_turns() {
        let req = ClaudeRunRequest {
            prompt: String::new(),
            images: vec![AccountImage {
                media_type: "image/png".to_string(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC".to_string(),
            }],
            model: Some("sonnet".to_string()),
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: None,
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        };
        let input = claude_stream_input(&req).unwrap();
        assert_eq!(input["type"], "user");
        assert_eq!(input["message"]["content"].as_array().unwrap().len(), 1);
        assert_eq!(input["message"]["content"][0]["type"], "image");
        assert_eq!(
            input["message"]["content"][0]["source"]["media_type"],
            "image/png"
        );
    }

    #[test]
    fn claude_project_dir_names_match_cli_sanitizer() {
        assert_eq!(
            claude_project_dir_name("C:\\Users\\USER\\Documents\\DEV\\screenmeister"),
            "C--Users-USER-Documents-DEV-screenmeister"
        );
        assert_eq!(
            claude_project_dir_name("/Users/omer/Documents/DEV/milim"),
            "-Users-omer-Documents-DEV-milim"
        );
    }

    #[test]
    fn maps_milim_tool_modes_to_claude_permissions() {
        let mut req = ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: None,
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: Some("guarded".into()),
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        };
        assert_eq!(claude_permission_mode(&req), "dontAsk");
        assert_eq!(
            claude_denied_tools(&req),
            vec!["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"]
        );

        req.tool_approval_policy = Some("open".into());
        assert_eq!(claude_permission_mode(&req), "bypassPermissions");
        assert!(claude_denied_tools(&req).is_empty());

        req.tool_approval_policy = Some("review".into());
        req.interactive_tool_approval = true;
        assert_eq!(claude_permission_mode(&req), "manual");
        assert!(claude_denied_tools(&req).is_empty());

        req.plan_mode = true;
        assert_eq!(claude_permission_mode(&req), "plan");
    }

    #[test]
    fn maps_only_explicit_claude_agent_lineage() {
        let event = claude_native_worker_event(&json!({
            "type": "system",
            "subtype": "agent_started",
            "agent_id": "agent-2",
            "parent_agent_id": "agent-1",
            "tool_use_id": "toolu-1",
            "description": "review the parser",
            "model": "sonnet"
        }))
        .expect("native worker event");
        assert_eq!(event.call_id, "toolu-1");
        assert_eq!(event.status, "running");
        assert_eq!(event.parent_runtime_id.as_deref(), Some("agent-1"));
        assert_eq!(event.worker_runtime_ids, vec!["agent-2"]);

        let tool_use = json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": {
                    "type": "tool_use",
                    "id": "toolu-2",
                    "name": "Agent",
                    "input": { "description": "review" }
                }
            }
        });
        assert!(claude_native_worker_event(&tool_use).is_none());

        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            &tool_use.to_string(),
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { name, .. }) if name == "Agent"
        ));
    }

    #[test]
    fn spawn_not_found_is_path_warning() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = claude_spawn_error_message(&error);
        assert!(is_cli_path_warning(&message));
        assert!(message.contains("claude"));
        assert!(message.contains("macOS"));
    }

    #[test]
    fn detects_locked_claude_session_errors() {
        assert!(claude_session_in_use_error(
            "Error: Session ID 374c9003-5446-4eba-841a-78bf02c93b95 is already in use."
        ));
        assert!(!claude_session_in_use_error(
            "claude exited with exit status: 1"
        ));
    }

    #[test]
    fn process_kill_matching_is_narrow() {
        let sid = "374c9003-5446-4eba-841a-78bf02c93b95";
        assert_eq!(safe_session_id_for_process_match(sid), Some(sid));
        assert_eq!(safe_session_id_for_process_match("bad'id"), None);
        assert!(process_matches_claude_session(
            "node.exe",
            &format!("node claude.js --session-id {sid}"),
            sid,
        ));
        assert!(process_matches_claude_session(
            "claude",
            &format!("claude -p hi --session-id {sid}"),
            sid,
        ));
        assert!(!process_matches_claude_session(
            "powershell.exe",
            &format!("powershell {sid}"),
            sid,
        ));
    }

    #[test]
    fn parses_claude_session_registry_entries() {
        let path = PathBuf::from("44856.json");
        let value = json!({
            "pid": 44856,
            "sessionId": "baee7712-0151-4f0c-a6d3-9b77207b6575",
            "cwd": "C:\\Users\\USER\\Documents\\DEV\\screenmeister",
        });
        assert_eq!(
            claude_session_registry_entry_from_value(&value, path.clone()),
            Some(ClaudeSessionRegistryEntry { pid: 44856, path }),
        );

        let bad = json!({ "pid": 44856, "sessionId": "bad'id" });
        assert_eq!(
            claude_session_registry_entry_from_value(&bad, PathBuf::new()),
            None
        );
    }
}
