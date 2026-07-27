//! JSONL RPC bridge to the user-installed Pi coding agent.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use axum::response::sse::Event;
use futures::Stream;
use milim_agents::ToolApprovalBroker;
use milim_core::api::openai::Usage;
use milim_core::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::codex_bridge::AccountImage;
use crate::privacy::Unredactor;

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_GRACE: Duration = Duration::from_millis(750);
const SAFE_TOOLS: &str = "read,grep,find,ls";
const APPROVAL_PREFIX: &str = "milim-tool-approval:";

#[derive(Debug, Deserialize)]
pub(crate) struct PiRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<AccountImage>,
    pub model: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub persist_session: Option<bool>,
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
        Ok(version) => match discover_models().await {
            Ok(models) => {
                let providers = provider_count(&models);
                Ok(json!({
                    "available": true,
                    "authenticated": !models.is_empty(),
                    "version": version.trim(),
                    "provider_count": providers,
                    "models": models,
                    "error": if models.is_empty() { Value::String("Pi has no authenticated or configured models. Run Pi and use /login, then refresh.".into()) } else { Value::Null },
                }))
            }
            Err(error) => Ok(json!({
                "available": true,
                "authenticated": false,
                "version": version.trim(),
                "provider_count": 0,
                "models": [],
                "error": error.to_string(),
            })),
        },
        Err(error) => Ok(json!({
            "available": false,
            "authenticated": false,
            "provider_count": 0,
            "models": [],
            "error": error.to_string(),
        })),
    }
}

pub(crate) async fn models() -> Result<Value> {
    Ok(json!({ "models": discover_models().await? }))
}

pub(crate) fn run_stream(
    req: PiRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let (provider, model_id) = match split_model(&req.model) {
            Some(parts) => parts,
            None => {
                yield sse(&json!({ "type": "error", "message": "Pi model must use provider/model format." }));
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        let session_id = req.session_id.clone().filter(|id| !id.trim().is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut proc = match PiProcess::start_run(&req, &provider, &model_id, &session_id).await {
            Ok(proc) => proc,
            Err(error) => {
                yield sse(&json!({ "type": "error", "message": error.to_string() }));
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        if let Err(error) = proc.verify_model(&provider, &model_id).await {
            yield sse(&json!({ "type": "error", "message": error.to_string() }));
            proc.finish().await;
            yield Ok(Event::default().data("[DONE]"));
            return;
        }
        yield sse(&json!({ "type": "session", "session_id": session_id, "model": req.model }));

        let mut prompt = json!({ "id": "milim-prompt", "type": "prompt", "message": req.prompt });
        if !req.images.is_empty() {
            prompt["images"] = Value::Array(req.images.iter().map(|image| json!({
                "type": "image", "mimeType": image.media_type, "data": image.data,
            })).collect());
        }
        if let Err(error) = proc.write_value(&prompt).await {
            yield sse(&json!({ "type": "error", "message": error.to_string() }));
            yield Ok(Event::default().data("[DONE]"));
            return;
        }

        let mut content = Unredactor::new(redactions.clone());
        let mut reasoning = Unredactor::new(redactions);
        let mut tool_calls: HashMap<String, (String, String, &'static str)> = HashMap::new();
        let mut final_usage: Option<Usage> = None;
        let mut finished = false;
        while !finished {
            let message = match proc.read_value().await {
                Ok(value) => value,
                Err(error) => {
                    yield sse(&json!({ "type": "error", "message": error.to_string() }));
                    break;
                }
            };
            match message.get("type").and_then(Value::as_str).unwrap_or_default() {
                "response" if message.get("id").and_then(Value::as_str) == Some("milim-prompt") => {
                    if message.get("success").and_then(Value::as_bool) == Some(false) {
                        yield sse(&json!({ "type": "error", "message": message.get("error").and_then(Value::as_str).unwrap_or("Pi rejected the prompt.") }));
                        break;
                    }
                }
                "message_update" => {
                    let event = message.get("assistantMessageEvent").unwrap_or(&Value::Null);
                    match event.get("type").and_then(Value::as_str).unwrap_or_default() {
                        "text_delta" => if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                            let text = content.push(delta);
                            if !text.is_empty() { yield sse(&json!({ "type": "token", "text": text })); }
                        },
                        "thinking_delta" => if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                            let text = reasoning.push(delta);
                            if !text.is_empty() { yield sse(&json!({ "type": "reasoning", "text": text })); }
                        },
                        "toolcall_end" => {
                            if let Some((call_id, name, arguments, effect)) = tool_call_from_delta(event) {
                                tool_calls.insert(call_id, (name, arguments, effect));
                            }
                        }
                        "error" => {
                            let detail = event.get("error").and_then(Value::as_str)
                                .or_else(|| event.pointer("/partial/errorMessage").and_then(Value::as_str))
                                .or_else(|| message.pointer("/message/errorMessage").and_then(Value::as_str))
                                .or_else(|| event.get("reason").and_then(Value::as_str))
                                .unwrap_or("Pi model request failed.");
                            yield sse(&json!({ "type": "error", "message": detail }));
                        }
                        _ => {}
                    }
                }
                "message_end" => {
                    if let Some(usage) = usage_from_message(message.get("message").unwrap_or(&Value::Null)) {
                        final_usage = Some(usage);
                    }
                }
                "tool_execution_start" => {
                    let call_id = message.get("toolCallId").and_then(Value::as_str).unwrap_or("pi-tool").to_string();
                    let name = message.get("toolName").and_then(Value::as_str).unwrap_or("Pi tool").to_string();
                    let arguments = message.get("args").cloned().unwrap_or(Value::Null).to_string();
                    let effect = tool_effect(&name);
                    tool_calls.insert(call_id.clone(), (name.clone(), arguments.clone(), effect));
                    yield sse(&json!({ "type": "tool", "id": call_id, "name": name, "status": "running", "detail": arguments }));
                }
                "tool_execution_end" => {
                    let call_id = message.get("toolCallId").and_then(Value::as_str).unwrap_or("pi-tool").to_string();
                    let name = message.get("toolName").and_then(Value::as_str).unwrap_or("Pi tool").to_string();
                    let status = if message.get("isError").and_then(Value::as_bool) == Some(true) { "error" } else { "done" };
                    yield sse(&json!({ "type": "tool", "id": call_id, "name": name, "status": status }));
                }
                "extension_ui_request" => {
                    let title = message.get("title").and_then(Value::as_str).unwrap_or_default();
                    if message.get("method").and_then(Value::as_str) != Some("confirm") || !title.starts_with(APPROVAL_PREFIX) {
                        continue;
                    }
                    let request_id = message.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                    let call_id = title.trim_start_matches(APPROVAL_PREFIX).to_string();
                    let (name, arguments, effect) = tool_calls.get(&call_id).cloned().unwrap_or_else(|| ("Pi tool".into(), "null".into(), "unknown"));
                    let interactive = req.interactive_tool_approval && !req.tool_approval_grant;
                    let approved = if interactive {
                        let Some(broker) = approval_broker.as_ref() else {
                            let _ = proc.write_value(&json!({ "type": "extension_ui_response", "id": request_id, "confirmed": false })).await;
                            yield sse(&json!({ "type": "error", "message": "Pi Review approval broker is unavailable." }));
                            break;
                        };
                        let mut pending = broker.request();
                        yield sse(&json!({
                            "type": "tool_approval_required", "approval_id": pending.id,
                            "call_id": call_id, "name": name, "arguments": arguments, "effect": effect
                        }));
                        let decision = pending.wait().await.approved;
                        yield sse(&json!({
                            "type": "tool_approval_resolved", "approval_id": pending.id,
                            "call_id": call_id, "decision": if decision { "approve" } else { "deny" }
                        }));
                        decision
                    } else {
                        req.tool_approval_grant || req.tool_approval_policy.as_deref() == Some("open")
                    };
                    if let Err(error) = proc.write_value(&json!({ "type": "extension_ui_response", "id": request_id, "confirmed": approved })).await {
                        yield sse(&json!({ "type": "error", "message": error.to_string() }));
                        break;
                    }
                }
                "extension_error" => {
                    let message = message.get("error").and_then(Value::as_str).or_else(|| message.get("message").and_then(Value::as_str)).unwrap_or("Pi extension failed.");
                    yield sse(&json!({ "type": "warning", "message": message }));
                }
                "agent_settled" => {
                    yield sse(&json!({ "type": "done", "status": "done", "usage": final_usage }));
                    finished = true;
                }
                _ => {}
            }
        }
        proc.finish().await;
        yield Ok(Event::default().data("[DONE]"));
    }
}

async fn discover_models() -> Result<Vec<Value>> {
    let mut proc = PiProcess::start_discovery().await?;
    proc.write_value(&json!({ "id": "milim-models", "type": "get_available_models" }))
        .await?;
    let result = tokio::time::timeout(STATUS_TIMEOUT, async {
        loop {
            let message = proc.read_value().await?;
            if message.get("id").and_then(Value::as_str) != Some("milim-models") {
                continue;
            }
            if message.get("success").and_then(Value::as_bool) != Some(true) {
                return Err(Error::Upstream(
                    message
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Pi model discovery failed.")
                        .to_string(),
                ));
            }
            let models = message
                .pointer("/data/models")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            return Ok(normalize_models(&models));
        }
    })
    .await
    .map_err(|_| Error::Upstream("Pi model discovery timed out.".into()))?;
    proc.finish().await;
    result
}

fn normalize_models(models: &[Value]) -> Vec<Value> {
    models.iter().filter_map(|model| {
        let provider = model.get("provider")?.as_str()?.trim();
        let model_id = model.get("id")?.as_str()?.trim();
        if provider.is_empty() || model_id.is_empty() { return None; }
        let input = model.get("input").and_then(Value::as_array);
        Some(json!({
            "id": format!("{provider}/{model_id}"),
            "provider": provider,
            "model_id": model_id,
            "name": model.get("name").and_then(Value::as_str).unwrap_or(model_id),
            "context_length": model.get("contextWindow").and_then(Value::as_u64),
            "max_completion_tokens": model.get("maxTokens").and_then(Value::as_u64),
            "reasoning": model.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
            "image_input": input.map(|items| items.iter().any(|item| item.as_str() == Some("image"))).unwrap_or(false),
        }))
    }).collect()
}

fn provider_count(models: &[Value]) -> usize {
    models
        .iter()
        .filter_map(|model| model.get("provider").and_then(Value::as_str))
        .collect::<HashSet<_>>()
        .len()
}

fn split_model(model: &str) -> Option<(String, String)> {
    let (provider, id) = model.trim().split_once('/')?;
    if provider.is_empty() || id.is_empty() {
        None
    } else {
        Some((provider.to_string(), id.to_string()))
    }
}

fn tool_effect(name: &str) -> &'static str {
    match name {
        "read" | "grep" | "find" | "ls" => "read_only",
        "bash" => "command",
        "write" | "edit" => "mutating",
        _ => "unknown",
    }
}

fn tool_call_from_delta(event: &Value) -> Option<(String, String, String, &'static str)> {
    let call = event.get("toolCall")?;
    let call_id = call
        .get("id")
        .or_else(|| call.get("toolCallId"))?
        .as_str()?
        .to_string();
    let name = call
        .get("name")
        .or_else(|| call.get("toolName"))?
        .as_str()?
        .to_string();
    let arguments = call
        .get("arguments")
        .or_else(|| call.get("args"))
        .cloned()
        .unwrap_or(Value::Null)
        .to_string();
    let effect = tool_effect(&name);
    Some((call_id, name, arguments, effect))
}

fn usage_from_message(message: &Value) -> Option<Usage> {
    let usage = message.get("usage")?;
    let prompt_tokens = usage.get("input").and_then(Value::as_u64).unwrap_or(0)
        + usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0)
        + usage.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0);
    let completion_tokens = usage.get("output").and_then(Value::as_u64).unwrap_or(0);
    let total_tokens = usage
        .get("totalTokens")
        .and_then(Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);
    Some(Usage {
        prompt_tokens: prompt_tokens as u32,
        completion_tokens: completion_tokens as u32,
        total_tokens: total_tokens as u32,
    })
}

fn run_arguments(
    req: &PiRunRequest,
    provider: &str,
    model: &str,
    session_id: &str,
    extension: Option<&Path>,
) -> Vec<String> {
    let mut args = vec!["--mode", "rpc", "--offline", "--no-extensions"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if req.persist_session.unwrap_or(true) {
        args.extend(["--session-id".into(), session_id.into()]);
    } else {
        args.push("--no-session".into());
    }
    args.extend([
        "--provider".into(),
        provider.into(),
        "--model".into(),
        model.into(),
    ]);
    if let Some(effort) = pi_thinking(req.reasoning_effort.as_deref()) {
        args.extend(["--thinking".into(), effort]);
    }
    let restrictive = req.plan_mode
        || req.tool_approval_policy.as_deref() == Some("guarded")
        || (req.tool_approval_policy.as_deref() == Some("review")
            && !req.interactive_tool_approval
            && !req.tool_approval_grant);
    let proxy_tools = req
        .milim_mcp
        .as_ref()
        .map(|endpoint| {
            endpoint
                .tools
                .iter()
                .map(|tool| format!("milim_{}", tool.name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if req.cwd.is_none() {
        args.push("--no-context-files".into());
        if proxy_tools.is_empty() {
            args.push("--no-tools".into());
        } else if restrictive {
            args.extend(["--tools".into(), proxy_tools.join(",")]);
        } else {
            args.push("--no-builtin-tools".into());
        }
    } else if restrictive {
        let mut tools = vec![SAFE_TOOLS.to_string()];
        tools.extend(proxy_tools);
        args.extend(["--tools".into(), tools.join(",")]);
    }
    if let Some(path) = extension {
        args.extend(["--extension".into(), path.to_string_lossy().into_owned()]);
    }
    args
}

fn parse_rpc_line(line: &str) -> Result<Value> {
    serde_json::from_str(line)
        .map_err(|error| Error::Upstream(format!("Pi RPC emitted invalid JSON: {error}")))
}

struct PiProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    extension_path: Option<PathBuf>,
    finished: bool,
}

impl PiProcess {
    async fn start_discovery() -> Result<Self> {
        let args = [
            "--mode",
            "rpc",
            "--no-session",
            "--offline",
            "--no-extensions",
            "--no-context-files",
        ]
        .map(str::to_string);
        Self::start(&args, None, None).await
    }

    async fn start_run(
        req: &PiRunRequest,
        provider: &str,
        model: &str,
        session_id: &str,
    ) -> Result<Self> {
        let review = req.tool_approval_policy.as_deref() == Some("review")
            && req.interactive_tool_approval
            && !req.tool_approval_grant
            && !req.plan_mode;
        let extension = if review || req.milim_mcp.is_some() {
            Some(write_runtime_extension(req, review)?)
        } else {
            None
        };
        let args = run_arguments(req, provider, model, session_id, extension.as_deref());
        Self::start(&args, req.cwd.as_deref().map(Path::new), extension).await
    }

    async fn start(
        args: &[String],
        cwd: Option<&Path>,
        extension_path: Option<PathBuf>,
    ) -> Result<Self> {
        let mut command = pi_command();
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        #[cfg(windows)]
        command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        let mut child = command
            .spawn()
            .map_err(|error| Error::Upstream(format!("Pi CLI is unavailable: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Upstream("Pi RPC stdin is unavailable.".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Upstream("Pi RPC stdout is unavailable.".into()))?;
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            extension_path,
            finished: false,
        })
    }

    async fn write_value(&mut self, value: &Value) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| Error::Upstream("Pi RPC stdin is closed.".into()))?;
        stdin
            .write_all(serde_json::to_string(value)?.as_bytes())
            .await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn verify_model(&mut self, provider: &str, model: &str) -> Result<()> {
        self.write_value(&json!({ "id": "milim-state", "type": "get_state" }))
            .await?;
        tokio::time::timeout(STATUS_TIMEOUT, async {
            loop {
                let response = self.read_value().await?;
                if response.get("id").and_then(Value::as_str) != Some("milim-state") {
                    continue;
                }
                if response.get("success").and_then(Value::as_bool) != Some(true) {
                    return Err(Error::Upstream(
                        response
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Pi could not report its selected model.")
                            .to_string(),
                    ));
                }
                let selected_provider = response
                    .pointer("/data/model/provider")
                    .and_then(Value::as_str);
                let selected_model = response.pointer("/data/model/id").and_then(Value::as_str);
                if selected_provider != Some(provider) || selected_model != Some(model) {
                    return Err(Error::Upstream(format!(
                        "Pi selected {}/{} instead of the requested {provider}/{model}.",
                        selected_provider.unwrap_or("unknown"),
                        selected_model.unwrap_or("unknown")
                    )));
                }
                return Ok(());
            }
        })
        .await
        .map_err(|_| Error::Upstream("Pi model verification timed out.".into()))?
    }

    async fn read_value(&mut self) -> Result<Value> {
        let line = self.stdout.next_line().await?.ok_or_else(|| {
            let status = self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten());
            Error::Upstream(
                status
                    .map(|status| format!("Pi RPC exited with {status}."))
                    .unwrap_or_else(|| "Pi RPC closed stdout.".into()),
            )
        })?;
        parse_rpc_line(&line)
    }

    async fn finish(&mut self) {
        self.finished = true;
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            if tokio::time::timeout(CANCEL_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
        remove_extension(self.extension_path.take());
    }
}

impl Drop for PiProcess {
    fn drop(&mut self) {
        remove_extension(self.extension_path.take());
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
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            let _ = child.start_kill();
            return;
        };
        runtime.spawn(async move {
            if let Ok(line) = serde_json::to_vec(&json!({ "type": "abort" })) {
                let _ = stdin.write_all(&line).await;
                let _ = stdin.write_all(b"\n").await;
                let _ = stdin.flush().await;
            }
            drop(stdin);
            if tokio::time::timeout(CANCEL_GRACE, child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        });
    }
}

async fn command_output(args: &[&str]) -> Result<String> {
    let mut command = pi_command();
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = tokio::time::timeout(STATUS_TIMEOUT, command.output())
        .await
        .map_err(|_| Error::Upstream("Pi command timed out.".into()))?
        .map_err(|error| Error::Upstream(format!("Pi CLI is unavailable: {error}")))?;
    if !output.status.success() {
        return Err(Error::Upstream(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn pi_thinking(effort: Option<&str>) -> Option<String> {
    match effort? {
        "none" => Some("off".into()),
        "minimal" | "low" | "medium" | "high" | "xhigh" => effort.map(str::to_string),
        _ => None,
    }
}

fn write_runtime_extension(req: &PiRunRequest, review: bool) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("milim-pi-approval-{}.mjs", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let endpoint = req.milim_mcp.as_ref();
    let config = endpoint
        .map(|endpoint| {
            json!({
                "url": endpoint.url,
                "authorization": endpoint.authorization,
                "tools": endpoint.tools,
            })
        })
        .unwrap_or(Value::Null);
    let source = format!(
        r#"
const config = {config};
export default function (pi) {{
  if (config) for (const tool of config.tools) {{
    pi.registerTool({{
      name: "milim_" + tool.name,
      label: tool.name,
      description: tool.description,
      parameters: tool.input_schema,
      async execute(toolCallId, params, signal) {{
        const response = await fetch(config.url, {{
          method: "POST",
          headers: {{ "Content-Type": "application/json", "Authorization": config.authorization }},
          body: JSON.stringify({{ jsonrpc: "2.0", id: toolCallId, method: "tools/call", params: {{ name: tool.name, arguments: params, _meta: {{ toolCallId }} }} }}),
          signal,
        }});
        const body = await response.json();
        if (!response.ok || body.error) throw new Error(body.error?.message || "Milim tool call failed");
        return {{ content: body.result.content, details: {{}} }};
      }},
    }});
  }}
  if ({review}) pi.on("tool_call", async (event, ctx) => {{
    if (event.toolName.startsWith("milim_") || ["read", "grep", "find", "ls"].includes(event.toolName)) return;
    const approved = await ctx.ui.confirm(
      "milim-tool-approval:" + event.toolCallId,
      "Approve this " + event.toolName + " call?",
    );
    if (!approved) return {{ block: true, reason: "Denied by user" }};
  }});
}}
"#,
        config = config,
        review = review
    );
    file.write_all(source.as_bytes())?;
    Ok(path)
}

fn remove_extension(path: Option<PathBuf>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

fn sse(value: &Value) -> std::result::Result<Event, Infallible> {
    Ok(Event::default()
        .data(serde_json::to_string(value).unwrap_or_else(|_| {
            "{\"type\":\"error\",\"message\":\"serialization failed\"}".into()
        })))
}

#[cfg(windows)]
fn pi_command() -> Command {
    if let Some(path) = find_on_path("pi.cmd") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(path);
        return command;
    }
    Command::new("pi")
}

#[cfg(not(windows))]
fn pi_command() -> Command {
    crate::cli_path::command("pi")
}

#[cfg(windows)]
fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(policy: &str) -> PiRunRequest {
        PiRunRequest {
            prompt: "test".into(),
            images: vec![],
            model: "openai-codex/gpt".into(),
            cwd: Some(".".into()),
            reasoning_effort: Some("high".into()),
            session_id: None,
            persist_session: Some(true),
            tool_approval_policy: Some(policy.into()),
            tool_approval_grant: false,
            interactive_tool_approval: policy == "review",
            plan_mode: false,
            milim_context: None,
            milim_mcp: None,
        }
    }

    #[test]
    fn models_keep_pi_metadata() {
        let models = normalize_models(&[json!({
            "provider": "openrouter", "id": "anthropic/claude-sonnet", "name": "Sonnet",
            "contextWindow": 200000, "maxTokens": 64000, "reasoning": true, "input": ["text", "image"]
        })]);
        assert_eq!(models[0]["id"], "openrouter/anthropic/claude-sonnet");
        assert_eq!(models[0]["context_length"], 200000);
        assert_eq!(models[0]["image_input"], true);
        assert_eq!(provider_count(&models), 1);
    }

    #[test]
    fn model_and_reasoning_values_are_bounded() {
        assert_eq!(
            split_model("openrouter/openai/gpt"),
            Some(("openrouter".into(), "openai/gpt".into()))
        );
        assert_eq!(split_model("missing"), None);
        assert_eq!(pi_thinking(Some("none")).as_deref(), Some("off"));
        assert_eq!(pi_thinking(Some("max")), None);
    }

    #[test]
    fn tool_effects_match_milim_approval_modes() {
        assert_eq!(tool_effect("read"), "read_only");
        assert_eq!(tool_effect("bash"), "command");
        assert_eq!(tool_effect("edit"), "mutating");
        assert_eq!(tool_effect("custom"), "unknown");
        assert_eq!(
            tool_call_from_delta(&json!({ "toolCall": {
                "id": "call-1", "name": "bash", "arguments": { "command": "echo ok" }
            }})),
            Some((
                "call-1".into(),
                "bash".into(),
                "{\"command\":\"echo ok\"}".into(),
                "command"
            ))
        );
    }

    #[test]
    fn pi_usage_includes_cache_tokens() {
        let usage = usage_from_message(&json!({ "usage": {
            "input": 10, "output": 5, "cacheRead": 3, "cacheWrite": 2, "totalTokens": 20
        }}))
        .expect("usage");
        assert_eq!(usage.prompt_tokens, 15);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 20);
    }

    #[test]
    fn session_and_approval_arguments_are_isolated() {
        let guarded = run_arguments(
            &request("guarded"),
            "openai-codex",
            "gpt",
            "session-1",
            None,
        );
        assert!(guarded
            .windows(2)
            .any(|pair| pair == ["--session-id", "session-1"]));
        assert!(guarded
            .windows(2)
            .any(|pair| pair == ["--tools", SAFE_TOOLS]));
        assert!(guarded
            .windows(2)
            .any(|pair| pair == ["--thinking", "high"]));

        let mut ephemeral = request("open");
        ephemeral.persist_session = Some(false);
        let args = run_arguments(
            &ephemeral,
            "openrouter",
            "anthropic/claude",
            "ignored",
            None,
        );
        assert!(args.contains(&"--no-session".to_string()));
        assert!(!args.contains(&"--tools".to_string()));
        assert!(!args.contains(&"--session-id".to_string()));

        let extension = Path::new("milim-review.mjs");
        let review = run_arguments(
            &request("review"),
            "openai-codex",
            "gpt",
            "session-2",
            Some(extension),
        );
        assert!(review
            .windows(2)
            .any(|pair| pair == ["--extension", "milim-review.mjs"]));
        assert!(!review.contains(&"--tools".to_string()));

        let mut no_workspace = request("open");
        no_workspace.cwd = None;
        let args = run_arguments(&no_workspace, "openai-codex", "gpt", "session-3", None);
        assert!(args.contains(&"--no-tools".to_string()));
        assert!(args.contains(&"--no-context-files".to_string()));
    }

    #[test]
    fn malformed_rpc_lines_fail_closed() {
        assert!(parse_rpc_line("not json").is_err());
        assert_eq!(
            parse_rpc_line(r#"{"type":"agent_settled"}"#).unwrap()["type"],
            "agent_settled"
        );
    }
}
