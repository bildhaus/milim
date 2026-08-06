//! Thin ACP bridge to the user-installed OpenCode CLI.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use futures::Stream;
use milim_agents::ToolApprovalBroker;
use milim_core::api::openai::Usage;
use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::account_runtime_events::{canonicalize_runtime_stream, HarnessEvent};
use crate::codex_bridge::AccountImage;
use crate::privacy::Unredactor;

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_GRACE: Duration = Duration::from_millis(750);
const SAFE_PERMISSIONS: &[&str] = &[
    "read",
    "glob",
    "grep",
    "list",
    "lsp",
    "skill",
    "webfetch",
    "websearch",
];

#[derive(Debug, Deserialize)]
pub(crate) struct OpenCodeRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<AccountImage>,
    pub model: String,
    #[serde(default)]
    pub cwd: Option<String>,
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
    pub milim_context: Option<crate::routes::AccountRuntimeMilimContext>,
    #[serde(skip)]
    pub milim_mcp: Option<crate::routes::AccountRuntimeToolEndpoint>,
}

pub(crate) async fn status() -> Result<Value> {
    let version = command_output(&["--version"]).await;
    match version {
        Ok(version) => match model_catalog().await {
            Ok((models, model_capabilities)) => Ok(json!({
                "available": true,
                "authenticated": !models.is_empty(),
                "version": version.trim(),
                "models": models,
                "model_capabilities": model_capabilities,
                "error": if models.is_empty() { Value::String("OpenCode has no configured models.".into()) } else { Value::Null },
            })),
            Err(error) => Ok(json!({
                "available": true,
                "authenticated": false,
                "version": version.trim(),
                "models": [],
                "error": error.to_string(),
            })),
        },
        Err(error) => Ok(json!({
            "available": false,
            "authenticated": false,
            "models": [],
            "error": error.to_string(),
        })),
    }
}

pub(crate) async fn models() -> Result<Value> {
    let (models, model_capabilities) = model_catalog().await?;
    Ok(json!({ "models": models, "model_capabilities": model_capabilities }))
}

pub(crate) fn run_stream(
    req: OpenCodeRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = HarnessEvent> {
    let initial_session_id = req.session_id.clone();
    canonicalize_runtime_stream(
        native_event_stream(req, redactions, approval_broker),
        initial_session_id,
    )
}

fn native_event_stream(
    req: OpenCodeRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = Value> {
    async_stream::stream! {
        let mut proc = match OpenCodeProcess::start(&req).await {
            Ok(proc) => proc,
            Err(error) => {
                yield json!({ "type": "error", "message": error.to_string() });
                return;
            }
        };
        let mut content = Unredactor::new(redactions.clone());
        let mut reasoning = Unredactor::new(redactions);
        let session = match proc.open_session(&req).await {
            Ok(session) => session,
            Err(error) => {
                yield json!({ "type": "error", "message": error.to_string() });
                return;
            }
        };
        yield json!({ "type": "session", "session_id": session, "model": req.model });
        let mut tool_calls: BTreeMap<String, (String, String)> = BTreeMap::new();
        let mut approval_ack: Option<(milim_agents::PendingApproval, String, &'static str)> = None;

        let prompt_id = match proc.send_request("session/prompt", Some(prompt_params(&req, &session))).await {
            Ok(id) => id,
            Err(error) => {
                yield json!({ "type": "error", "message": error.to_string() });
                return;
            }
        };
        loop {
            let read = if approval_ack.is_some() {
                match tokio::time::timeout(
                    milim_agents::APPROVAL_RUNTIME_ACK_TIMEOUT,
                    proc.read_value(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        if let Some((pending, call_id, decision)) = approval_ack.take() {
                            let message = "OpenCode did not resume after the approval decision".to_string();
                            pending.fail(message.clone());
                            yield json!({
                                "type": "tool_approval_failed", "approval_id": pending.id,
                                "call_id": call_id, "decision": decision, "message": message
                            });
                            yield json!({ "type": "error", "message": message });
                        }
                        break;
                    }
                }
            } else {
                proc.read_value().await
            };
            let message = match read {
                Ok(value) => value,
                Err(error) => {
                    if let Some((pending, call_id, decision)) = approval_ack.take() {
                        pending.fail(error.to_string());
                        yield json!({
                            "type": "tool_approval_failed", "approval_id": pending.id,
                            "call_id": call_id, "decision": decision, "message": error.to_string()
                        });
                    }
                    yield json!({ "type": "error", "message": error.to_string() });
                    break;
                }
            };
            if let Some((pending, call_id, decision)) = approval_ack.take() {
                pending.acknowledge();
                yield json!({
                    "type": "tool_approval_resolved", "approval_id": pending.id,
                    "call_id": call_id, "decision": decision
                });
            }
            if message.get("id") == Some(&prompt_id) {
                if let Some(error) = message.get("error") {
                    yield json!({ "type": "error", "message": rpc_error(error) });
                } else {
                    yield json!({ "type": "done", "status": "done" });
                }
                break;
            }
            let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
            let params = message.get("params").unwrap_or(&Value::Null);
            if method == "session/request_permission" {
                let Some(id) = message.get("id").cloned() else { continue };
                let call = params.get("toolCall").unwrap_or(&Value::Null);
                let call_id = string_at(call, &["toolCallId"]).unwrap_or_else(|| "opencode-tool".into());
                let name = string_at(call, &["title"]).unwrap_or_else(|| "OpenCode tool".into());
                let arguments = call.get("rawInput").cloned().unwrap_or(Value::Null).to_string();
                let interactive = req.interactive_tool_approval && !req.tool_approval_grant;
                let mut pending_delivery = None;
                let approved = if interactive {
                    let Some(broker) = approval_broker.as_ref() else {
                        let _ = proc.respond(id, permission_response(params, false)).await;
                        yield json!({ "type": "error", "message": "OpenCode Review approval broker is unavailable." });
                        break;
                    };
                    let mut pending = broker.request();
                    yield json!({
                        "type": "tool_approval_required", "approval_id": pending.id,
                        "call_id": call_id, "name": name, "arguments": arguments, "effect": "unknown"
                    });
                    let decision = pending.wait().await.approved;
                    yield json!({
                        "type": "tool_approval_status", "approval_id": pending.id,
                        "call_id": call_id, "decision": if decision { "approve" } else { "deny" },
                        "status": "decided"
                    });
                    pending_delivery = Some(pending);
                    decision
                } else {
                    req.tool_approval_grant || req.tool_approval_policy.as_deref() == Some("open")
                };
                if let Err(error) = proc.respond(id, permission_response(params, approved)).await {
                    if let Some(pending) = pending_delivery.take() {
                        pending.fail(error.to_string());
                        yield json!({
                            "type": "tool_approval_failed", "approval_id": pending.id,
                            "call_id": call_id, "decision": if approved { "approve" } else { "deny" },
                            "message": error.to_string()
                        });
                    }
                    yield json!({ "type": "error", "message": error.to_string() });
                    break;
                }
                if let Some(pending) = pending_delivery {
                    let decision = if approved { "approve" } else { "deny" };
                    if let Err(message) = pending.deliver() {
                        yield json!({
                            "type": "tool_approval_failed", "approval_id": pending.id,
                            "call_id": call_id, "decision": decision, "message": message
                        });
                        yield json!({ "type": "error", "message": message });
                        break;
                    }
                    yield json!({
                        "type": "tool_approval_status", "approval_id": pending.id,
                        "call_id": call_id, "decision": decision, "status": "delivered"
                    });
                    approval_ack = Some((pending, call_id, decision));
                }
                continue;
            }
            if method != "session/update" {
                if let Some(id) = message.get("id").cloned() {
                    if let Err(error) = proc
                        .respond_error(id, -32601, "Method not found")
                        .await
                    {
                        yield json!({ "type": "error", "message": error.to_string() });
                        break;
                    }
                    yield json!({
                        "type": "warning",
                        "message": format!("OpenCode requested unsupported method {method}")
                    });
                }
                continue;
            }
            let update = params.get("update").unwrap_or(&Value::Null);
            for event in open_code_update_events(
                update,
                &mut content,
                &mut reasoning,
                &mut tool_calls,
            ) {
                yield event;
            }
        }
        proc.finish().await;
    }
}

fn open_code_update_events(
    update: &Value,
    content: &mut Unredactor,
    reasoning: &mut Unredactor,
    tool_calls: &mut BTreeMap<String, (String, String)>,
) -> Vec<Value> {
    let mut events = Vec::new();
    match update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "agent_message_chunk" => {
            if let Some(text) = string_at(update, &["content", "text"]) {
                let text = content.push(&text);
                if !text.is_empty() {
                    events.push(json!({ "type": "token", "text": text }));
                }
            }
        }
        "agent_thought_chunk" => {
            if let Some(text) = string_at(update, &["content", "text"]) {
                let text = reasoning.push(&text);
                if !text.is_empty() {
                    events.push(json!({ "type": "reasoning", "text": text }));
                }
            }
        }
        "tool_call" | "tool_call_update" => {
            let id = string_at(update, &["toolCallId"]).unwrap_or_else(|| "opencode-tool".into());
            let previous = tool_calls.get(&id).cloned();
            let name = string_at(update, &["title"])
                .or_else(|| string_at(update, &["kind"]))
                .or_else(|| previous.as_ref().map(|(name, _)| name.clone()))
                .unwrap_or_else(|| "OpenCode tool".into());
            let detail = tool_input_detail(update)
                .or_else(|| previous.as_ref().map(|(_, detail)| detail.clone()));
            let status = string_at(update, &["status"]).unwrap_or_else(|| "running".into());
            let result = tool_result(update);
            let error = matches!(status.as_str(), "failed" | "error")
                .then(|| tool_error_detail(update, result.as_ref()))
                .flatten();
            if matches!(status.as_str(), "completed" | "done" | "failed" | "error") {
                tool_calls.remove(&id);
            } else if let Some(detail) = detail.as_ref() {
                tool_calls.insert(id.clone(), (name.clone(), detail.clone()));
            }
            events.push(json!({
                "type": "tool", "id": id, "name": name, "status": status,
                "detail": detail, "result": result, "error": error
            }));
        }
        "usage_update" => {
            if let Some(usage) = usage_from_update(update) {
                events.push(json!({ "type": "done", "status": "running", "usage": usage }));
            }
        }
        "plan" | "current_mode_update" | "available_commands_update" | "" => {}
        kind => events.push(json!({
            "type": "protocol_notice",
            "kind": "unsupported_update",
            "message": format!("OpenCode emitted unsupported session update {kind}")
        })),
    }
    events
}

fn prompt_params(req: &OpenCodeRunRequest, session_id: &str) -> Value {
    let mut prompt = vec![json!({ "type": "text", "text": req.prompt })];
    prompt.extend(req.images.iter().map(|image| {
        json!({
            "type": "image", "mimeType": image.media_type, "data": image.data,
        })
    }));
    json!({ "sessionId": session_id, "prompt": prompt })
}

fn permission_response(params: &Value, approved: bool) -> Value {
    let options = params.get("options").and_then(Value::as_array);
    let kind = if approved {
        "allow_once"
    } else {
        "reject_once"
    };
    let option = options
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("kind").and_then(Value::as_str) == Some(kind))
        })
        .and_then(|item| item.get("optionId"))
        .cloned();
    option
        .map(|option_id| json!({ "outcome": { "outcome": "selected", "optionId": option_id } }))
        .unwrap_or_else(|| json!({ "outcome": { "outcome": "cancelled" } }))
}

fn usage_from_update(update: &Value) -> Option<Usage> {
    let usage = update
        .get("usage")
        .or_else(|| update.get("_meta").and_then(|meta| meta.get("usage")));
    if usage.is_none() {
        let used = update.get("used").and_then(Value::as_u64)? as u32;
        return Some(Usage {
            prompt_tokens: used,
            completion_tokens: 0,
            total_tokens: used,
        });
    }
    let usage = usage?;
    Some(Usage {
        prompt_tokens: usage
            .get("inputTokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: usage
            .get("outputTokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total_tokens: usage
            .get("totalTokens")
            .or_else(|| usage.get("total_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    })
}

fn tool_input_detail(update: &Value) -> Option<String> {
    update.get("rawInput").and_then(value_detail)
}

fn tool_result(update: &Value) -> Option<Value> {
    update
        .get("rawOutput")
        .or_else(|| update.get("content"))
        .filter(|value| !value.is_null())
        .cloned()
}

fn tool_error_detail(update: &Value, result: Option<&Value>) -> Option<String> {
    update
        .get("error")
        .and_then(value_detail)
        .or_else(|| result.and_then(value_detail))
}

fn value_detail(value: &Value) -> Option<String> {
    if value.is_null() {
        return None;
    }
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| Some(value.to_string()))
}

struct OpenCodeProcess {
    tree: Option<ProcessTreeGuard>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    active_session: Option<String>,
    cwd: PathBuf,
    finished: bool,
}

impl OpenCodeProcess {
    async fn start(req: &OpenCodeRunRequest) -> Result<Self> {
        let cwd = opencode_cwd(req)?;
        let overlay = merged_policy_overlay(req)?;
        if req.tool_approval_policy.as_deref() != Some("open")
            || req.plan_mode
            || !has_workspace(req)
        {
            preflight_policy(&cwd, &overlay).await?;
        }
        let mut command = opencode_command();
        command
            .arg("acp")
            .arg("--cwd")
            .arg(&cwd)
            .current_dir(&cwd)
            .env("OPENCODE_CONFIG_CONTENT", serde_json::to_string(&overlay)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| Error::Upstream(format!("OpenCode is unavailable: {error}")))?;
        let tree =
            ProcessTreeGuard::attach(child.id().ok_or_else(|| {
                Error::Upstream("OpenCode ACP process id is unavailable.".into())
            })?)
            .map_err(|error| Error::Upstream(format!("failed to contain OpenCode ACP: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Upstream("OpenCode ACP stdin is unavailable.".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Upstream("OpenCode ACP stdout is unavailable.".into()))?;
        let mut process = Self {
            tree: Some(tree),
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
            active_session: None,
            cwd,
            finished: false,
        };
        let initialized = process
            .request(
                "initialize",
                Some(json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {},
                    "clientInfo": { "name": "milim", "version": env!("CARGO_PKG_VERSION") }
                })),
            )
            .await?;
        if initialized.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
            return Err(Error::Upstream(
                "OpenCode does not support ACP protocol version 1.".into(),
            ));
        }
        if !req.images.is_empty()
            && initialized
                .pointer("/agentCapabilities/promptCapabilities/image")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(Error::InvalidRequest(
                "This OpenCode ACP runtime does not advertise image prompt support.".into(),
            ));
        }
        Ok(process)
    }

    async fn open_session(&mut self, req: &OpenCodeRunRequest) -> Result<String> {
        let mcp_servers = req
            .milim_mcp
            .as_ref()
            .map(|endpoint| {
                vec![json!({
                    "type": "http",
                    "name": "milim",
                    "url": endpoint.url,
                    "headers": [{ "name": "Authorization", "value": endpoint.authorization }]
                })]
            })
            .unwrap_or_default();
        let (method, params) = if let Some(session_id) =
            req.session_id.as_deref().filter(|id| !id.trim().is_empty())
        {
            (
                "session/resume",
                json!({ "sessionId": session_id, "cwd": self.cwd, "mcpServers": mcp_servers }),
            )
        } else {
            (
                "session/new",
                json!({ "cwd": self.cwd, "mcpServers": mcp_servers }),
            )
        };
        let result = self.request(method, Some(params)).await?;
        let session_id = req
            .session_id
            .clone()
            .filter(|id| !id.trim().is_empty())
            .or_else(|| string_at(&result, &["sessionId"]))
            .ok_or_else(|| Error::Upstream("OpenCode did not return an ACP session id.".into()))?;
        self.set_config(&session_id, "model", &req.model).await?;
        let mode = if req.plan_mode { "plan" } else { "build" };
        if config_has_value(&result, "mode", mode) {
            self.set_config(&session_id, "mode", mode).await?;
        }
        self.active_session = Some(session_id.clone());
        Ok(session_id)
    }

    async fn set_config(&mut self, session_id: &str, config_id: &str, value: &str) -> Result<()> {
        self.request(
            "session/set_config_option",
            Some(json!({
                "sessionId": session_id, "configId": config_id, "value": value,
            })),
        )
        .await
        .map(|_| ())
    }

    async fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.send_request(method, params).await?;
        loop {
            let message = self.read_value().await?;
            if message.get("id") != Some(&id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(Error::Upstream(rpc_error(error)));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = json!(self.next_id);
        self.next_id += 1;
        let mut message = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(params) = params {
            message["params"] = params;
        }
        self.write_value(&message).await?;
        Ok(id)
    }

    async fn respond(&mut self, id: Value, result: Value) -> Result<()> {
        self.write_value(&json!({ "jsonrpc": "2.0", "id": id, "result": result }))
            .await
    }

    async fn respond_error(&mut self, id: Value, code: i64, message: &str) -> Result<()> {
        self.write_value(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }))
        .await
    }

    async fn write_value(&mut self, value: &Value) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Upstream("OpenCode ACP stdin is closed.".into()))?;
        stdin
            .write_all(serde_json::to_string(value)?.as_bytes())
            .await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn read_value(&mut self) -> Result<Value> {
        let line = self.stdout.next_line().await?.ok_or_else(|| {
            let status = self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten());
            Error::Upstream(
                status
                    .map(|status| format!("OpenCode ACP exited with {status}."))
                    .unwrap_or_else(|| "OpenCode ACP closed stdout.".into()),
            )
        })?;
        serde_json::from_str(&line)
            .map_err(|error| Error::Upstream(format!("OpenCode ACP emitted invalid JSON: {error}")))
    }

    async fn finish(&mut self) {
        self.finished = true;
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            crate::child_process::wait_or_kill(&mut child, CANCEL_GRACE).await;
        }
        self.tree.take();
    }
}

impl Drop for OpenCodeProcess {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let Some(mut child) = self.child.take() else {
            return;
        };
        let Some(mut stdin) = self.stdin.take() else {
            let _ = child.start_kill();
            return;
        };
        let session = self.active_session.take();
        let tree = self.tree.take();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let _ = child.start_kill();
            return;
        };
        runtime.spawn(async move {
            let _tree = tree;
            if let Some(session_id) = session {
                let cancel = json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": session_id },
                });
                if let Ok(line) = serde_json::to_vec(&cancel) {
                    let _ = stdin.write_all(&line).await;
                    let _ = stdin.write_all(b"\n").await;
                    let _ = stdin.flush().await;
                }
            }
            drop(stdin);
            crate::child_process::wait_or_kill(&mut child, CANCEL_GRACE).await;
        });
    }
}

async fn command_output(args: &[&str]) -> Result<String> {
    let mut command = opencode_command();
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = tokio::time::timeout(STATUS_TIMEOUT, crate::child_process::output(command))
        .await
        .map_err(|_| Error::Upstream("OpenCode command timed out.".into()))?
        .map_err(|error| Error::Upstream(format!("OpenCode is unavailable: {error}")))?;
    if !output.status.success() {
        return Err(Error::Upstream(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn model_catalog() -> Result<(Vec<String>, BTreeMap<String, Value>)> {
    let output = match command_output(&["models", "--verbose", "--pure"]).await {
        Ok(output) => output,
        Err(_) => command_output(&["models"]).await?,
    };
    Ok((parse_models(&output), parse_model_capabilities(&output)))
}

async fn preflight_policy(cwd: &std::path::Path, overlay: &Value) -> Result<()> {
    let expected = overlay.get("permission").cloned().unwrap_or(Value::Null);
    let mut command = opencode_command();
    command
        .arg("debug")
        .arg("config")
        .current_dir(cwd)
        .env("OPENCODE_CONFIG_CONTENT", serde_json::to_string(overlay)?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = tokio::time::timeout(STATUS_TIMEOUT, crate::child_process::output(command))
        .await
        .map_err(|_| Error::Upstream("OpenCode permission preflight timed out.".into()))?
        .map_err(|error| {
            Error::Upstream(format!("OpenCode permission preflight failed: {error}"))
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Error::Upstream(if detail.is_empty() {
            "OpenCode permission preflight failed.".into()
        } else {
            format!("OpenCode permission preflight failed: {detail}")
        }));
    }
    let config: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        Error::Upstream("OpenCode permission preflight returned invalid configuration.".into())
    })?;
    if config.get("permission") != Some(&expected) {
        return Err(Error::InvalidRequest(
            "Managed OpenCode permissions override Milim's selected safety mode.".into(),
        ));
    }
    Ok(())
}

fn policy_overlay(req: &OpenCodeRunRequest) -> Value {
    let has_workspace = has_workspace(req);
    let restrictive =
        !has_workspace || req.plan_mode || req.tool_approval_policy.as_deref() == Some("guarded");
    let mut permissions = serde_json::Map::new();
    if req.tool_approval_policy.as_deref() == Some("open") && !req.plan_mode && has_workspace {
        return json!({ "permission": "allow", "default_agent": "build" });
    }
    permissions.insert("*".into(), json!(if restrictive { "deny" } else { "ask" }));
    if has_workspace {
        for name in SAFE_PERMISSIONS {
            permissions.insert((*name).into(), json!("allow"));
        }
    }
    if req.milim_mcp.is_some() {
        permissions.insert("milim_*".into(), json!("allow"));
    }
    json!({
        "permission": permissions,
        "default_agent": if req.plan_mode { "plan" } else { "build" },
    })
}

fn opencode_cwd(req: &OpenCodeRunRequest) -> Result<PathBuf> {
    if let Some(cwd) = req.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
        return Ok(PathBuf::from(cwd));
    }
    let cwd = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("opencode");
    std::fs::create_dir_all(&cwd)?;
    Ok(cwd)
}

fn has_workspace(req: &OpenCodeRunRequest) -> bool {
    req.cwd.as_deref().is_some_and(|cwd| !cwd.trim().is_empty())
}

fn merged_policy_overlay(req: &OpenCodeRunRequest) -> Result<Value> {
    let mut merged = std::env::var("OPENCODE_CONFIG_CONTENT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| serde_json::from_str::<Value>(&value))
        .transpose()
        .map_err(|error| {
            Error::InvalidRequest(format!(
                "Existing OPENCODE_CONFIG_CONTENT is invalid JSON: {error}"
            ))
        })?
        .unwrap_or_else(|| json!({}));
    let overlay = policy_overlay(req);
    let target = merged.as_object_mut().ok_or_else(|| {
        Error::InvalidRequest("Existing OPENCODE_CONFIG_CONTENT must be a JSON object.".into())
    })?;
    for key in ["permission", "default_agent"] {
        if let Some(value) = overlay.get(key) {
            target.insert(key.to_string(), value.clone());
        }
    }
    Ok(merged)
}

fn parse_models(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty() && line.contains('/') && !line.contains(char::is_whitespace)
        })
        .map(str::to_string)
        .collect()
}

fn parse_model_capabilities(output: &str) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    let mut id: Option<&str> = None;
    let mut lines = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let is_id =
            !trimmed.is_empty() && trimmed.contains('/') && !trimmed.contains(char::is_whitespace);
        if is_id {
            insert_model_capabilities(&mut result, id, &lines);
            id = Some(trimmed);
            lines.clear();
        } else if id.is_some() {
            lines.push(line);
        }
    }
    insert_model_capabilities(&mut result, id, &lines);
    result
}

fn insert_model_capabilities(
    result: &mut BTreeMap<String, Value>,
    id: Option<&str>,
    lines: &[&str],
) {
    let (Some(id), Ok(raw)) = (id, serde_json::from_str::<Value>(&lines.join("\n"))) else {
        return;
    };
    let variants = raw.get("variants").and_then(Value::as_object);
    let efforts = ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .into_iter()
        .filter(|effort| {
            variants.is_some_and(|variants| {
                variants.values().any(|variant| {
                    variant.get("reasoningEffort").and_then(Value::as_str) == Some(*effort)
                })
            })
        })
        .collect::<Vec<_>>();
    result.insert(
        id.to_string(),
        json!({
            "display_name": raw.get("name").and_then(Value::as_str),
            "context_length": raw.pointer("/limit/context").and_then(Value::as_u64),
            "max_prompt_tokens": raw.pointer("/limit/input").and_then(Value::as_u64),
            "max_completion_tokens": raw.pointer("/limit/output").and_then(Value::as_u64),
            "image_input": raw.pointer("/capabilities/input/image").and_then(Value::as_bool),
            "tool_use": raw.pointer("/capabilities/toolcall").and_then(Value::as_bool),
            "supported_efforts": efforts,
        }),
    );
}

fn config_has_value(result: &Value, id: &str, value: &str) -> bool {
    result
        .get("configOptions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|item| item.get("options"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|item| item.get("value").and_then(Value::as_str) == Some(value))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |cursor, key| cursor.get(*key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn rpc_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("OpenCode ACP request failed.")
        .to_string()
}

#[cfg(windows)]
fn opencode_command() -> Command {
    if let Some(path) = crate::child_process::find_on_path("opencode.cmd") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(path);
        return command;
    }
    Command::new("opencode")
}

#[cfg(not(windows))]
fn opencode_command() -> Command {
    crate::cli_path::command("opencode")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(policy: &str, plan_mode: bool) -> OpenCodeRunRequest {
        OpenCodeRunRequest {
            prompt: "test".into(),
            images: vec![],
            model: "provider/model".into(),
            cwd: Some(".".into()),
            session_id: None,
            tool_approval_policy: Some(policy.into()),
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode,
            milim_context: None,
            milim_mcp: None,
        }
    }

    #[test]
    fn no_folder_request_deserializes_and_denies_native_tools() {
        let req: OpenCodeRunRequest =
            serde_json::from_value(json!({ "prompt": "test", "model": "provider/model" })).unwrap();
        assert!(req.cwd.is_none());
        assert_eq!(policy_overlay(&req)["permission"]["*"], "deny");
        assert!(policy_overlay(&req)["permission"].get("read").is_none());
    }

    #[test]
    fn models_are_bounded_to_plain_provider_ids() {
        assert_eq!(
            parse_models("openai/gpt-5\n heading text \nanthropic/sonnet"),
            vec!["openai/gpt-5", "anthropic/sonnet"]
        );
    }

    #[test]
    fn verbose_models_keep_capabilities() {
        let output = r#"opencode/north-mini-code-free
{
  "name": "North Mini Code Free",
  "limit": { "context": 256000, "output": 64000 },
  "capabilities": { "toolcall": true, "input": { "image": false } },
  "variants": {
    "none": { "reasoningEffort": "none" },
    "high": { "reasoningEffort": "high" }
  }
}"#;
        let metadata = parse_model_capabilities(output);
        let model = &metadata["opencode/north-mini-code-free"];
        assert_eq!(model["display_name"], "North Mini Code Free");
        assert_eq!(model["context_length"], 256000);
        assert_eq!(model["max_completion_tokens"], 64000);
        assert_eq!(model["image_input"], false);
        assert_eq!(model["tool_use"], true);
        assert_eq!(model["supported_efforts"], json!(["none", "high"]));
    }

    #[test]
    fn tool_updates_keep_full_inputs_and_outputs() {
        let command = format!("powershell -Command \"{}\"", "x".repeat(140));
        assert_eq!(
            tool_input_detail(&json!({ "rawInput": { "command": command } })),
            Some(command)
        );
        let update = json!({
            "status": "failed",
            "rawOutput": { "error": "permission denied" }
        });
        let result = tool_result(&update);
        assert_eq!(result, Some(json!({ "error": "permission denied" })));
        assert_eq!(
            tool_error_detail(&update, result.as_ref()),
            Some("{\"error\":\"permission denied\"}".into())
        );
    }

    #[test]
    fn policy_overlay_keeps_safe_tools_readable() {
        let guarded = policy_overlay(&request("guarded", false));
        assert_eq!(guarded["permission"]["*"], "deny");
        assert_eq!(guarded["permission"]["read"], "allow");
        assert_eq!(
            policy_overlay(&request("review", false))["permission"]["*"],
            "ask"
        );
        assert_eq!(
            policy_overlay(&request("open", false))["permission"],
            "allow"
        );
        assert_eq!(
            policy_overlay(&request("open", true))["default_agent"],
            "plan"
        );
    }

    #[test]
    fn approvals_are_one_shot() {
        let params = json!({ "options": [
            { "optionId": "once", "kind": "allow_once" },
            { "optionId": "always", "kind": "allow_always" },
            { "optionId": "reject", "kind": "reject_once" }
        ] });
        assert_eq!(
            permission_response(&params, true)["outcome"]["optionId"],
            "once"
        );
        assert_eq!(
            permission_response(&params, false)["outcome"]["optionId"],
            "reject"
        );
    }

    #[test]
    fn prompt_and_standard_usage_follow_acp_v1_shapes() {
        let mut req = request("guarded", false);
        req.images.push(AccountImage {
            media_type: "image/png".into(),
            data: "aGVsbG8=".into(),
        });
        let prompt = prompt_params(&req, "session-1");
        assert_eq!(prompt["sessionId"], "session-1");
        assert_eq!(prompt["prompt"][1]["type"], "image");
        assert_eq!(prompt["prompt"][1]["mimeType"], "image/png");

        let usage = usage_from_update(&json!({
            "sessionUpdate": "usage_update",
            "used": 321,
            "size": 8_192,
        }))
        .expect("standard ACP usage update");
        assert_eq!(usage.prompt_tokens, 321);
        assert_eq!(usage.total_tokens, 321);
    }

    #[test]
    fn replays_versioned_acp_update_fixture_without_payload_diagnostics() {
        let fixture = include_str!(
            "../tests/fixtures/account-runtimes/opencode-cli-1.18.9-acp-session-updates.jsonl"
        );
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let events = fixture
            .lines()
            .flat_map(|line| {
                let message: Value = serde_json::from_str(line).unwrap();
                open_code_update_events(
                    message.pointer("/params/update").unwrap(),
                    &mut content,
                    &mut reasoning,
                    &mut tools,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.get("type").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            [
                "token",
                "reasoning",
                "tool",
                "tool",
                "done",
                "protocol_notice"
            ]
        );
        assert!(!events
            .last()
            .unwrap()
            .to_string()
            .contains("privatePayload"));
        assert!(tools.is_empty());
    }
}
