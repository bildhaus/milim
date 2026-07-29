use super::*;

// ----- Codex app-server bridge -----

#[derive(Deserialize)]
pub(crate) struct CodexAccountQuery {
    #[serde(default)]
    refresh: bool,
}

/// `GET /codex/account` - current Codex-managed auth state.
pub(crate) async fn codex_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<CodexAccountQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let account = crate::codex_bridge::account(query.refresh)
        .await
        .map_err(ApiError)?;
    Ok(Json(account).into_response())
}

/// `POST /codex/login/device` - start ChatGPT login and stream completion.
pub(crate) async fn codex_login_device(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Sse::new(crate::codex_bridge::login_device_stream())
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `POST /codex/login/chatgpt-device` - start ChatGPT device-code login.
pub(crate) async fn codex_login_chatgpt_device(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(
        Sse::new(crate::codex_bridge::login_chatgpt_device_code_stream())
            .keep_alive(KeepAlive::default())
            .into_response(),
    )
}

/// `POST /codex/login/api-key` - sign Codex in with an OpenAI API key.
pub(crate) async fn codex_login_api_key(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<crate::codex_bridge::CodexApiKeyLoginRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::login_api_key(req.api_key)
        .await
        .map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `POST /codex/logout` - clear Codex-managed auth.
pub(crate) async fn codex_logout(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::logout().await.map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `GET /codex/rate-limits` - read Codex account usage buckets.
pub(crate) async fn codex_rate_limits(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::rate_limits().await.map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `GET /codex/models` - list models exposed by the installed Codex app-server.
pub(crate) async fn codex_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::models().await.map_err(ApiError)?;
    Ok(Json(result).into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexThreadsQuery {
    cursor: Option<String>,
    search: Option<String>,
    #[serde(default)]
    archived: bool,
}

/// `GET /codex/threads` - page through recoverable Codex app-server threads.
pub(crate) async fn codex_threads(
    State(st): State<AppState>,
    Query(query): Query<CodexThreadsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::threads(query.cursor, query.search, query.archived)
        .await
        .map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `GET /codex/threads/{id}` - recover visible messages from one Codex thread.
pub(crate) async fn codex_thread_recover(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::codex_bridge::recover_thread(&id)
        .await
        .map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `POST /codex/run` - run a Codex turn as a separate account runtime.
pub(crate) async fn codex_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<crate::codex_bridge::CodexRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context =
        RunContext::from_account_runtime(&st, req.milim_context.as_ref(), req.cwd.as_deref())
            .map_err(ApiError)?;
    req.cwd = run_context.workspace_text();
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Codex requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&run_context, &req.prompt, "claude");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &run_context, &req.prompt, "Codex")
            .map_err(ApiError)?;
    account_runtime_images_for_remote(&run_context, &req.images, "Codex").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
        &run_context,
        req.model.as_deref().unwrap_or_default(),
        &req.prompt,
    )?;
    req.milim_mcp = endpoint.clone();
    let approvals = Some(st.tool_approvals.clone());
    Ok(Sse::new(account_runtime_stream(
        crate::codex_bridge::run_stream(req, redactions, approvals),
        &st,
        endpoint.as_ref(),
        true,
    ))
    .keep_alive(KeepAlive::default())
    .into_response())
}

/// `GET /claude/status` - current installed Claude CLI auth/runtime state.
pub(crate) async fn claude_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let status = crate::claude_bridge::status().await.map_err(ApiError)?;
    Ok(Json(status).into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeThreadsQuery {
    cursor: Option<String>,
    search: Option<String>,
}

/// `GET /claude/threads` - page through locally retained Claude CLI chats.
pub(crate) async fn claude_threads(
    State(st): State<AppState>,
    Query(query): Query<ClaudeThreadsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::claude_bridge::threads(query.cursor, query.search)
        .await
        .map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `GET /claude/threads/{id}` - import visible messages from one local Claude chat.
pub(crate) async fn claude_thread_import(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let result = crate::claude_bridge::import_thread(&id)
        .await
        .map_err(ApiError)?;
    Ok(Json(result).into_response())
}

/// `GET /opencode/status` - installed OpenCode CLI and configured-model state.
pub(crate) async fn opencode_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(crate::opencode_bridge::status().await.map_err(ApiError)?).into_response())
}

/// `GET /opencode/models` - models exposed by the installed OpenCode CLI.
pub(crate) async fn opencode_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(crate::opencode_bridge::models().await.map_err(ApiError)?).into_response())
}

/// `POST /opencode/run` - create or resume one OpenCode ACP session turn.
pub(crate) async fn opencode_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<crate::opencode_bridge::OpenCodeRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context =
        RunContext::from_account_runtime(&st, req.milim_context.as_ref(), req.cwd.as_deref())
            .map_err(ApiError)?;
    req.cwd = run_context.workspace_text();
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "OpenCode requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&run_context, &req.prompt, "agents");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &run_context, &req.prompt, "OpenCode")
            .map_err(ApiError)?;
    account_runtime_images_for_remote(&run_context, &req.images, "OpenCode").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
        &run_context,
        &req.model,
        &req.prompt,
    )?;
    req.milim_mcp = endpoint.clone();
    Ok(Sse::new(account_runtime_stream(
        crate::opencode_bridge::run_stream(req, redactions, Some(st.tool_approvals.clone())),
        &st,
        endpoint.as_ref(),
        true,
    ))
    .keep_alive(KeepAlive::default())
    .into_response())
}

/// `GET /pi/status` - installed Pi CLI and configured-model state.
pub(crate) async fn pi_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(crate::pi_bridge::status().await.map_err(ApiError)?).into_response())
}

/// `GET /pi/models` - models exposed by the installed Pi CLI.
pub(crate) async fn pi_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(crate::pi_bridge::models().await.map_err(ApiError)?).into_response())
}

/// `POST /pi/run` - create or resume one Pi RPC session turn.
pub(crate) async fn pi_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<crate::pi_bridge::PiRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context =
        RunContext::from_account_runtime(&st, req.milim_context.as_ref(), req.cwd.as_deref())
            .map_err(ApiError)?;
    req.cwd = run_context.workspace_text();
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Pi requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&run_context, &req.prompt, "native");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &run_context, &req.prompt, "Pi")
            .map_err(ApiError)?;
    account_runtime_images_for_remote(&run_context, &req.images, "Pi").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
        &run_context,
        &req.model,
        &req.prompt,
    )?;
    req.milim_mcp = endpoint.clone();
    Ok(Sse::new(account_runtime_stream(
        crate::pi_bridge::run_stream(req, redactions, Some(st.tool_approvals.clone())),
        &st,
        endpoint.as_ref(),
        true,
    ))
    .keep_alive(KeepAlive::default())
    .into_response())
}

/// `GET /account-runtimes/updates` - installed and latest versions for user-owned CLIs.
pub(crate) async fn account_runtime_updates(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(crate::account_runtime_update::statuses().await).into_response())
}

/// `POST /account-runtimes/{runtime}/update` - run the CLI's own updater.
pub(crate) async fn account_runtime_update(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(runtime): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let runtime = crate::account_runtime_update::AccountRuntime::parse(&runtime)
        .ok_or_else(|| ApiError(Error::InvalidRequest("Unknown account runtime".into())))?;
    Ok(Json(
        crate::account_runtime_update::update(runtime)
            .await
            .map_err(ApiError)?,
    )
    .into_response())
}

/// `POST /claude/run` - run an installed Claude CLI turn as a separate account runtime.
pub(crate) async fn claude_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<crate::claude_bridge::ClaudeRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context =
        RunContext::from_account_runtime(&st, req.milim_context.as_ref(), req.cwd.as_deref())
            .map_err(ApiError)?;
    req.cwd = run_context.workspace_text();
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Claude requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&run_context, &req.prompt, "agents");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &run_context, &req.prompt, "Claude")
            .map_err(ApiError)?;
    account_runtime_images_for_remote(&run_context, &req.images, "Claude").map_err(ApiError)?;
    req.prompt = prompt;
    req.interactive_tool_approval = crate::claude_bridge::claude_interactive_tool_approval(&req);
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
        &run_context,
        req.model.as_deref().unwrap_or_default(),
        &req.prompt,
    )?;
    req.milim_mcp = endpoint.clone();
    let approvals = if req.interactive_tool_approval {
        let run_id = endpoint
            .as_ref()
            .map(|endpoint| endpoint.run_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let host = headers
            .get(HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("127.0.0.1:7377");
        let host = loopback_host(host).ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "Claude Review requires a loopback Milim server address".to_string(),
            ))
        })?;
        req.approval_run_id = Some(run_id.clone());
        req.approval_mcp_url = Some(format!(
            "http://{host}/internal/claude-approvals/{run_id}/mcp"
        ));
        req.approval_mcp_authorization = headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        Some(st.tool_approvals.clone())
    } else {
        None
    };
    Ok(Sse::new(account_runtime_stream(
        crate::claude_bridge::run_stream(req, redactions, approvals),
        &st,
        endpoint.as_ref(),
        false,
    ))
    .keep_alive(KeepAlive::default())
    .into_response())
}

pub(crate) fn loopback_host(value: &str) -> Option<String> {
    let value = value.trim();
    let url = reqwest::Url::parse(&format!("http://{value}")).ok()?;
    let host = url.host_str()?;
    (host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()))
    .then(|| value.to_string())
}

pub(crate) async fn claude_approval_mcp(
    State(st): State<AppState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(request): Json<Value>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "milim-approval", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({ "tools": [{
            "name": "request_tool_approval",
            "description": "Ask the Milim user to approve one Claude tool call.",
            "inputSchema": {
                "type": "object",
                "properties": { "tool_name": { "type": "string" }, "input": { "type": "object" } },
                "required": ["tool_name", "input"]
            }
        }] }),
        "tools/call" => {
            let params = request.get("params").unwrap_or(&Value::Null);
            if params.get("name").and_then(Value::as_str) != Some("request_tool_approval") {
                return Ok(Json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": "unknown approval tool" }
                }))
                .into_response());
            }
            let args = params.get("arguments").unwrap_or(&Value::Null);
            let name = args
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let input = args.get("input").cloned().unwrap_or(Value::Null);
            let arguments = serde_json::to_string(&input).unwrap_or_else(|_| "{}".to_string());
            let effect = match name.as_str() {
                "Bash" | "PowerShell" => ToolEffect::Command,
                "Edit" | "Write" | "NotebookEdit" => ToolEffect::Mutating,
                _ => ToolEffect::Unknown,
            };
            let mut pending = st
                .tool_approvals
                .request_external(run_id, None, name, arguments, effect);
            let mut approved = pending.wait().await.approved;
            let delivery_error = pending.deliver().err();
            if delivery_error.is_some() {
                approved = false;
            }
            let payload = if approved {
                json!({ "behavior": "allow", "updatedInput": input })
            } else {
                json!({
                    "behavior": "deny",
                    "message": delivery_error
                        .as_deref()
                        .unwrap_or("Tool call denied by user")
                })
            };
            json!({
                "content": [{ "type": "text", "text": payload.to_string() }],
                "isError": false
            })
        }
        _ if method.starts_with("notifications/") => {
            return Ok(StatusCode::ACCEPTED.into_response())
        }
        _ => {
            return Ok(Json(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }))
            .into_response())
        }
    };
    Ok(Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response())
}

pub(crate) async fn account_runtime_tool_mcp(
    State(st): State<AppState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(request): Json<Value>,
) -> Result<Response, ApiError> {
    if !peer_addr(peer).is_some_and(|addr| addr.ip().is_loopback()) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let session = st
        .account_runtime_tools
        .lock()
        .expect("account runtime tool store poisoned")
        .get(&run_id)
        .cloned();
    let Some(session) = session else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    if authorization != format!("Bearer {}", session.token) {
        return Ok(StatusCode::UNAUTHORIZED.into_response());
    }
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if method.starts_with("notifications/") {
        return Ok(StatusCode::ACCEPTED.into_response());
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "milim", "version": env!("CARGO_PKG_VERSION") }
        }),
        "tools/list" => json!({
            "tools": session.registry.list().into_iter().map(|tool| json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
                "annotations": { "readOnlyHint": tool.effect == ToolEffect::ReadOnly },
            })).collect::<Vec<_>>()
        }),
        "tools/call" => {
            let params = request.get("params").unwrap_or(&Value::Null);
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let Some(effect) = session.registry.effect(name) else {
                return Ok(Json(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": format!("unknown tool: {name}") }
                }))
                .into_response());
            };
            if session.review && effect != ToolEffect::ReadOnly {
                let mut pending = st.tool_approvals.request_external(
                    run_id.clone(),
                    request
                        .pointer("/params/_meta/toolCallId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    name.to_string(),
                    serde_json::to_string(&args).unwrap_or_else(|_| "{}".into()),
                    effect,
                );
                let approved = pending.wait().await.approved;
                let delivery_error = pending.deliver().err();
                if let Some(error) = delivery_error {
                    json!({
                        "content": [{ "type": "text", "text": error }],
                        "isError": true
                    })
                } else if !approved {
                    json!({
                        "content": [{ "type": "text", "text": "Tool call denied by user." }],
                        "isError": true
                    })
                } else {
                    account_runtime_mcp_call(&session.registry, name, args).await
                }
            } else {
                account_runtime_mcp_call(&session.registry, name, args).await
            }
        }
        _ => {
            return Ok(Json(json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }))
            .into_response())
        }
    };
    Ok(Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response())
}

async fn account_runtime_mcp_call(registry: &ToolRegistry, name: &str, args: Value) -> Value {
    match registry.call_for_agent(name, args).await {
        Ok(result) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string(&result.result).unwrap_or_else(|_| "null".into()) }],
            "isError": false
        }),
        Err(error) => json!({
            "content": [{ "type": "text", "text": error.to_string() }],
            "isError": true
        }),
    }
}

fn account_runtime_workspace_prompt(
    run_context: &RunContext,
    prompt: &str,
    family: &str,
) -> String {
    let context = crate::workspace_context::resolve(run_context.workspace());
    match crate::workspace_context::formatted(&context, Some(family)) {
        Some(instructions) => format!("{instructions}\n\nUser request:\n{prompt}"),
        None => prompt.to_string(),
    }
}

fn account_runtime_prompt_for_remote(
    st: &AppState,
    run_context: &RunContext,
    prompt: &str,
    runtime: &str,
) -> milim_core::Result<(String, BTreeMap<String, String>)> {
    match run_context.privacy_mode() {
        PrivacyMode::Off => Ok((prompt.to_string(), BTreeMap::new())),
        PrivacyMode::Block => {
            let detections = st.privacy.scan_text(prompt);
            if detections.is_empty() {
                Ok((prompt.to_string(), BTreeMap::new()))
            } else {
                Err(Error::InvalidRequest(format!(
                    "blocked by the privacy gate: {runtime} prompt contains {} ({} item(s)). Switch the gate to Redact or Off to send this to {runtime}.",
                    kinds_summary(&detections),
                    detections.len()
                )))
            }
        }
        PrivacyMode::Redact => {
            let redaction = st.privacy.redact_text(prompt);
            Ok((redaction.text, redaction.map))
        }
    }
}

fn account_runtime_images_for_remote(
    run_context: &RunContext,
    images: &[crate::codex_bridge::AccountImage],
    runtime: &str,
) -> milim_core::Result<()> {
    if images.is_empty() {
        return Ok(());
    }
    if run_context.privacy_mode() != PrivacyMode::Off {
        return Err(Error::InvalidRequest(format!(
            "blocked by the privacy gate: {runtime} image pixels can only be sent in Privacy Off because images cannot be safely redacted"
        )));
    }
    crate::codex_bridge::validate_account_images(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_inference::test_backend::TestBackend;

    #[test]
    fn account_runtime_privacy_uses_the_captured_mode() {
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        );
        state.privacy.set(PrivacyMode::Block);
        let blocked = RunContext::from_account_runtime(&state, None, None).unwrap();
        state.privacy.set(PrivacyMode::Off);

        assert!(account_runtime_prompt_for_remote(
            &state,
            &blocked,
            "contact person@example.com",
            "test runtime",
        )
        .is_err());

        let allowed = RunContext::from_account_runtime(&state, None, None).unwrap();
        state.privacy.set(PrivacyMode::Block);
        assert_eq!(
            account_runtime_prompt_for_remote(
                &state,
                &allowed,
                "contact person@example.com",
                "test runtime",
            )
            .unwrap()
            .0,
            "contact person@example.com"
        );
    }
}
