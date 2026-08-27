#![recursion_limit = "256"]

//! `milim-server` — the axum HTTP server exposing the public API contract.
//!
//! Mirrors milim's OpenAI/Ollama-compatible surface so existing clients work
//! unchanged: streamed and non-streamed chat completions, model listing, and
//! embeddings, with bearer auth + loopback trust, CORS, and a body-size cap.

mod account_runtime_events;
mod account_runtime_update;
mod auth;
mod child_process;
mod claude_bridge;
#[cfg(not(windows))]
mod cli_path;
mod codex_bridge;
pub mod companion;
pub mod control;
mod error;
pub mod google_workspace;
pub mod mcp_bridge;
pub mod media_library;
mod opencode_bridge;
mod pi_bridge;
pub mod preview_runtime;
pub mod privacy;
pub mod providers;
mod routes;
mod sse;
mod state;
pub mod threads;
mod translate;
mod workspace_context;

use std::future::Future;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::routing::{delete, get, post, put};
use axum::Router;

use milim_control_contract::{
    ControlCommandKindV1, ControlCommandStatusV1, ControlCommandV1, ThreadOriginV1,
};
use milim_core::{api::openai::ChatMessage, Error, Result};
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

pub use state::AppState;

/// Assemble the application router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.max_request_body_bytes;
    let cors = build_cors(&state.config.allowed_origins);

    Router::new()
        .route("/health", get(routes::health))
        // Native mobile pairing, credentials, and control-v1 administration.
        .route("/mobile", get(routes::mobile_companion_probe))
        .route("/mobile/", get(routes::mobile_companion_probe))
        .route("/mobile/status", get(routes::mobile_companion_status))
        .route("/mobile/enabled", post(routes::mobile_companion_enabled))
        .route("/mobile/pairing", post(routes::mobile_companion_pairing))
        .route("/mobile/pair", post(routes::mobile_companion_pair))
        .route(
            "/mobile/pair-requests",
            post(routes::mobile_companion_pairing_request_create),
        )
        .route(
            "/mobile/pair-requests/{id}",
            get(routes::mobile_companion_pairing_request_status)
                .delete(routes::mobile_companion_pairing_request_cancel),
        )
        .route(
            "/mobile/pair-requests/{id}/claim",
            post(routes::mobile_companion_pairing_request_claim),
        )
        .route(
            "/mobile/pair-requests/{id}/decision",
            post(routes::mobile_companion_pairing_request_decision),
        )
        .route(
            "/mobile/device/status",
            get(routes::mobile_companion_device_status),
        )
        .route(
            "/mobile/device",
            delete(routes::mobile_companion_device_revoke_self),
        )
        .route(
            "/mobile/devices/{id}",
            delete(routes::mobile_companion_device_revoke),
        )
        // Canonical desktop/native-mobile control contract. This namespace is
        // intentionally distinct from legacy Worker `/threads/*` routes.
        .route("/control/v1/bootstrap", get(routes::control_bootstrap))
        .route(
            "/control/v1/appearance/background",
            get(routes::control_appearance_background),
        )
        .route(
            "/control/v1/threads/{id}/timeline",
            get(routes::control_timeline),
        )
        .route(
            "/control/v1/threads/{id}/effective-run",
            post(routes::control_effective_run_preview),
        )
        .route(
            "/control/v1/runs/{run_id}",
            get(routes::control_run_inspection),
        )
        .route(
            "/control/v1/runs/{run_id}/events",
            get(routes::control_run_events),
        )
        .route("/control/v1/commands", post(routes::control_command))
        .route(
            "/control/v1/socket-ticket",
            post(routes::control_socket_ticket),
        )
        .route("/control/v1/ws", get(routes::control_socket))
        // Model listing (OpenAI + Ollama)
        .route("/v1/models", get(routes::openai_models))
        .route("/models", get(routes::openai_models))
        // Provider registry (OpenAI-compatible remotes)
        .route(
            "/providers",
            get(routes::providers_list).post(routes::provider_upsert),
        )
        .route("/providers/discover", get(routes::providers_discover))
        .route(
            "/providers/{id}/models/verify",
            post(routes::provider_model_verify),
        )
        .route("/providers/{id}", delete(routes::provider_delete))
        // Desktop-selected Google Drive, Docs, Sheets, and Slides.
        .route(
            "/google-workspace/status",
            get(google_workspace::status_route),
        )
        .route(
            "/google-workspace/picker",
            post(google_workspace::picker_start_route),
        )
        .route(
            "/google-workspace/picker/{id}",
            get(google_workspace::picker_status_route),
        )
        .route(
            "/google-workspace/files",
            get(google_workspace::file_list_route),
        )
        .route(
            "/google-workspace/files/{id}",
            delete(google_workspace::file_remove_route),
        )
        .route(
            "/google-workspace/disconnect",
            post(google_workspace::disconnect_route),
        )
        .route(
            "/google-workspace/preview/{id}",
            get(google_workspace::preview_route),
        )
        .route(
            "/google-workspace/content/{id}",
            get(google_workspace::content_route),
        )
        .route(
            "/google-workspace/sheets/{id}/edit",
            post(google_workspace::sheet_edit_route),
        )
        .route(
            "/google-workspace/docs/{id}/edit",
            post(google_workspace::doc_edit_route),
        )
        .route(
            "/google-workspace/slides/{id}/edit",
            post(google_workspace::slide_edit_route),
        )
        // Media generation through encrypted remote provider credentials
        .route("/media/models", get(routes::media_models))
        .route("/media/model-schema", get(routes::media_model_schema))
        .route("/media/status", get(routes::media_status))
        .route("/media/content", get(routes::media_content))
        .route("/media/generate", post(routes::media_generate))
        .route("/media/library", get(routes::media_library_list))
        .route(
            "/media/library/{id}/refresh",
            post(routes::media_library_refresh),
        )
        .route(
            "/media/library/{id}/content/{index}",
            get(routes::media_library_content),
        )
        .route("/media/library/{id}", delete(routes::media_library_delete))
        // Host working folder (drives the filesystem/shell tools)
        .route(
            "/workspace",
            get(routes::workspace_get).post(routes::workspace_set),
        )
        .route("/workspace/git", get(routes::workspace_git_status))
        .route("/workspace/context", get(routes::workspace_context))
        .route("/workspace/git/action", post(routes::workspace_git_action))
        // Managed preview apps for no-folder chat artifacts.
        .route("/preview-apps/{thread_id}", get(routes::preview_app_get))
        .route(
            "/preview-apps/{thread_id}/stage",
            post(routes::preview_app_stage),
        )
        .route(
            "/preview-apps/{thread_id}/preflight",
            post(routes::preview_app_preflight),
        )
        .route(
            "/preview-apps/{thread_id}/start",
            post(routes::preview_app_start),
        )
        .route(
            "/preview-apps/{thread_id}/static",
            post(routes::preview_app_static),
        )
        .route(
            "/preview-apps/{thread_id}/stop",
            post(routes::preview_app_stop),
        )
        .route(
            "/preview-apps/{thread_id}/restart",
            post(routes::preview_app_restart),
        )
        .route(
            "/preview-apps/{thread_id}/logs",
            get(routes::preview_app_logs),
        )
        // Computer-use gate (screen capture + mouse/keyboard)
        .route(
            "/computer",
            get(routes::computer_get).post(routes::computer_set),
        )
        .route("/api/tags", get(routes::ollama_tags))
        // Chat completions
        .route("/v1/chat/completions", post(routes::openai_chat))
        .route("/chat/completions", post(routes::openai_chat))
        .route("/v1/completions", post(routes::openai_completions))
        .route("/completions", post(routes::openai_completions))
        .route("/v1/responses", post(routes::openai_responses))
        .route("/api/chat", post(routes::ollama_chat))
        .route("/api/generate", post(routes::ollama_generate))
        // Anthropic Messages
        .route("/anthropic/v1/messages", post(routes::anthropic_messages))
        .route("/anthropic/messages", post(routes::anthropic_messages))
        .route("/v1/messages", post(routes::anthropic_messages))
        // Codex app-server bridge (separate from OpenAI-compatible providers)
        .route("/codex/account", get(routes::codex_account))
        .route("/codex/login/device", post(routes::codex_login_device))
        .route(
            "/codex/login/chatgpt-device",
            post(routes::codex_login_chatgpt_device),
        )
        .route("/codex/login/api-key", post(routes::codex_login_api_key))
        .route("/codex/logout", post(routes::codex_logout))
        .route("/codex/rate-limits", get(routes::codex_rate_limits))
        .route("/codex/models", get(routes::codex_models))
        .route("/codex/threads", get(routes::codex_threads))
        .route("/codex/threads/{id}", get(routes::codex_thread_recover))
        .route("/codex/run", post(routes::codex_run))
        // Installed Claude CLI bridge (separate from Anthropic API-key providers)
        .route("/claude/status", get(routes::claude_status))
        .route("/claude/threads", get(routes::claude_threads))
        .route("/claude/threads/{id}", get(routes::claude_thread_import))
        .route("/claude/run", post(routes::claude_run))
        // User-installed OpenCode ACP runtime
        .route("/opencode/status", get(routes::opencode_status))
        .route("/opencode/models", get(routes::opencode_models))
        .route("/opencode/run", post(routes::opencode_run))
        // User-installed Pi JSONL RPC runtime
        .route("/pi/status", get(routes::pi_status))
        .route("/pi/models", get(routes::pi_models))
        .route("/pi/run", post(routes::pi_run))
        .route("/harnesses/{id}/run", post(routes::harness_run))
        .route(
            "/account-runtimes/updates",
            get(routes::account_runtime_updates),
        )
        .route(
            "/account-runtimes/{runtime}/update",
            post(routes::account_runtime_update),
        )
        .route(
            "/internal/claude-approvals/{run_id}/mcp",
            post(routes::claude_approval_mcp),
        )
        .route(
            "/internal/account-runtime-tools/{run_id}/mcp",
            post(routes::account_runtime_tool_mcp),
        )
        // MCP tools (server bridge: exposes our tools to MCP clients)
        .route("/mcp/tools", get(routes::mcp_tools))
        .route("/mcp/call", post(routes::mcp_call))
        .route(
            "/mcp/apps/resources/read",
            post(routes::mcp_app_resource_read),
        )
        .route("/mcp/apps/tools/call", post(routes::mcp_app_tool_call))
        .route("/mcp/apps/views/{id}", get(routes::mcp_app_view))
        // MCP client: external MCP servers whose tools we consume
        .route(
            "/mcp/servers",
            get(routes::mcp_servers_list).post(routes::mcp_server_upsert),
        )
        .route("/mcp/servers/test", post(routes::mcp_server_test_draft))
        .route(
            "/mcp/servers/{id}/test",
            post(routes::mcp_server_test_saved),
        )
        .route("/mcp/servers/{id}", delete(routes::mcp_server_delete))
        // Agents (server-side tool-use loop + named agents)
        .route("/agents/run", post(routes::agents_run))
        .route(
            "/tool-approvals/{id}",
            get(routes::tool_approval_status).post(routes::tool_approval_resolve),
        )
        .route(
            "/agents",
            get(routes::agents_list).post(routes::agent_create),
        )
        .route(
            "/agents/{id}",
            get(routes::agent_get)
                .put(routes::agent_update)
                .delete(routes::agent_delete),
        )
        .route("/agents/{id}/run", post(routes::agent_run_by_id))
        .route(
            "/threads/{id}",
            get(routes::thread_get).delete(routes::thread_delete),
        )
        .route("/threads/{id}/children", get(routes::thread_children))
        .route("/threads/{id}/events", get(routes::thread_events))
        .route("/threads/{id}/stop", post(routes::thread_stop))
        .route(
            "/worker-runs",
            get(routes::worker_runs_list).post(routes::worker_run_create),
        )
        .route(
            "/worker-runs/{id}",
            get(routes::worker_run_get).delete(routes::worker_run_delete),
        )
        .route("/worker-runs/{id}/events", get(routes::worker_run_events))
        .route("/worker-runs/{id}/start", post(routes::worker_run_start))
        .route("/worker-runs/{id}/stop", post(routes::worker_run_stop))
        .route(
            "/worker-runs/{id}/tasks/{task_id}/retry",
            post(routes::worker_run_task_retry),
        )
        .route(
            "/worker-runs/{id}/workers/{worker_id}/stop",
            post(routes::worker_run_worker_stop),
        )
        .route(
            "/worker-runs/{id}/workers/{worker_id}/diff",
            get(routes::worker_run_worker_diff),
        )
        .route(
            "/worker-runs/{id}/workers/{worker_id}/apply",
            post(routes::worker_run_worker_apply),
        )
        // Memory / RAG
        .route("/memory/ingest", post(routes::memory_ingest))
        .route("/memory/search", post(routes::memory_search))
        .route("/memory/register", post(routes::memory_register))
        .route("/memory/graph/search", post(routes::memory_graph_search))
        .route("/memory/scopes", get(routes::memory_scopes))
        .route("/memory/nodes", get(routes::memory_nodes))
        .route(
            "/memory/nodes/{id}",
            put(routes::memory_node_update).delete(routes::memory_node_delete),
        )
        .route(
            "/memory/nodes/{id}/archive",
            post(routes::memory_node_archive),
        )
        // Privacy filter
        .route("/privacy/scan", post(routes::privacy_scan))
        .route(
            "/privacy/mode",
            get(routes::privacy_mode_get).post(routes::privacy_mode_set),
        )
        // Sandbox (isolated command execution)
        .route("/sandbox/run", post(routes::sandbox_run))
        // Skills
        .route(
            "/skills",
            get(routes::skills_list).post(routes::skill_create),
        )
        .route("/skills/select", post(routes::skills_select))
        .route(
            "/skills/{id}",
            get(routes::skill_get)
                .put(routes::skill_update)
                .delete(routes::skill_delete),
        )
        // Schedules (cron)
        .route(
            "/schedules",
            get(routes::schedules_list).post(routes::schedule_create),
        )
        .route(
            "/schedules/{id}",
            put(routes::schedule_update).delete(routes::schedule_delete),
        )
        // Embeddings
        .route("/v1/embeddings", post(routes::openai_embeddings))
        .route("/embeddings", post(routes::openai_embeddings))
        .route("/api/embed", post(routes::ollama_embeddings))
        .route("/api/embeddings", post(routes::ollama_embeddings))
        // Middleware (applied outermost-first)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(RequestBodyLimitLayer::new(body_limit))
        .with_state(state)
}

/// Assemble the phone-facing companion router only. This is intentionally
/// narrower than the local API so it can be exposed through Tailscale Serve.
pub fn build_mobile_companion_router(state: AppState) -> Router {
    let body_limit = state.config.max_request_body_bytes;
    let mut state = state;
    state.mobile_control_only = true;

    Router::new()
        .route("/mobile", get(routes::mobile_companion_probe))
        .route("/mobile/", get(routes::mobile_companion_probe))
        .route("/mobile/pair", post(routes::mobile_companion_pair))
        .route(
            "/mobile/pair-requests",
            post(routes::mobile_companion_pairing_request_create),
        )
        .route(
            "/mobile/pair-requests/{id}",
            get(routes::mobile_companion_pairing_request_status)
                .delete(routes::mobile_companion_pairing_request_cancel),
        )
        .route(
            "/mobile/pair-requests/{id}/claim",
            post(routes::mobile_companion_pairing_request_claim),
        )
        .route(
            "/mobile/device/status",
            get(routes::mobile_companion_device_status),
        )
        .route(
            "/mobile/device",
            delete(routes::mobile_companion_device_revoke_self),
        )
        .route("/control/v1/bootstrap", get(routes::control_bootstrap))
        .route(
            "/control/v1/appearance/background",
            get(routes::control_appearance_background),
        )
        .route(
            "/control/v1/threads/{id}/timeline",
            get(routes::control_timeline),
        )
        .route(
            "/control/v1/threads/{id}/effective-run",
            post(routes::control_effective_run_preview),
        )
        .route(
            "/control/v1/runs/{run_id}",
            get(routes::control_run_inspection),
        )
        .route(
            "/control/v1/runs/{run_id}/events",
            get(routes::control_run_events),
        )
        .route("/control/v1/commands", post(routes::control_command))
        .route(
            "/control/v1/socket-ticket",
            post(routes::control_socket_ticket),
        )
        .route("/control/v1/ws", get(routes::control_socket))
        .layer(TraceLayer::new_for_http())
        .layer(RequestBodyLimitLayer::new(body_limit))
        .with_state(state)
}

/// Serve on a freshly-bound socket at `addr`.
pub async fn serve(state: AppState, addr: SocketAddr) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    serve_listener(state, listener).await
}

/// Serve on an already-bound listener (lets callers learn the port first).
pub async fn serve_listener(
    state: AppState,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    serve_listener_with_graceful_shutdown(state, listener, shutdown_signal()).await
}

/// Serve on an already-bound listener with a caller-provided shutdown signal.
pub async fn serve_listener_with_graceful_shutdown<S>(
    state: AppState,
    listener: tokio::net::TcpListener,
    shutdown: S,
) -> std::io::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    let app = build_router(state.clone());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(with_graceful_shutdown(state, shutdown))
    .await
}

/// Await a shutdown signal and stop active work before the server exits.
pub async fn with_graceful_shutdown<S>(state: AppState, shutdown: S)
where
    S: Future<Output = ()>,
{
    shutdown.await;
    if let Some(threads) = state.threads.as_ref() {
        if let Err(err) = threads.stop_running_children("stopped by server shutdown") {
            tracing::warn!("failed to stop child threads during shutdown: {err}");
        }
        if let Err(err) = threads
            .store()
            .update_non_terminal_worker_runs("stopped by server shutdown")
        {
            tracing::warn!("failed to stop worker runs during shutdown: {err}");
        }
    }
    if let Err(err) = state.preview_runtime.stop_all().await {
        tracing::warn!("failed to stop preview apps during shutdown: {err}");
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::warn!("failed to install Ctrl-C shutdown handler: {err}");
        std::future::pending::<()>().await;
    }
}

/// Serve only the phone-facing companion surface on an already-bound listener.
pub async fn serve_mobile_companion_listener(
    state: AppState,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    serve_mobile_companion_listener_with_graceful_shutdown(state, listener, std::future::pending())
        .await
}

/// Serve only the phone-facing companion surface with caller-controlled shutdown.
pub async fn serve_mobile_companion_listener_with_graceful_shutdown<S>(
    state: AppState,
    listener: tokio::net::TcpListener,
    shutdown: S,
) -> std::io::Result<()>
where
    S: Future<Output = ()> + Send + 'static,
{
    let app = build_mobile_companion_router(state);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

/// Build a CORS layer. An empty allow-list yields a permissive dev policy
/// (loopback tooling); a non-empty list restricts to those origins.
fn build_cors(origins: &[String]) -> CorsLayer {
    if origins.is_empty() {
        return CorsLayer::new();
    }
    let allow = origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect::<Vec<axum::http::HeaderValue>>();
    CorsLayer::new()
        .allow_origin(allow)
        .allow_methods(Any)
        .allow_headers(Any)
}

/// Current unix time in whole seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current time formatted as RFC-3339 (for Ollama `created_at`).
pub fn rfc3339_now() -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Generate a response id like `chatcmpl-<hex>`.
pub fn gen_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

pub(crate) fn agent_skill_messages(
    state: &AppState,
    agent: &milim_agents::AgentDef,
    query: &str,
) -> Vec<ChatMessage> {
    let Some(store) = state.skills.as_ref() else {
        return Vec::new();
    };
    let skills = match milim_agents::normalize_skill_mode(&agent.skill_mode, &agent.enabled_skills)
        .as_str()
    {
        "none" => Vec::new(),
        "custom" => store
            .select_filtered(query, 3, Some(&agent.enabled_skills))
            .unwrap_or_default(),
        _ => store.select(query, 3).unwrap_or_default(),
    };
    skill_instruction_message(&skills).into_iter().collect()
}

fn skill_instruction_message(skills: &[milim_skills::SkillDef]) -> Option<ChatMessage> {
    const MAX_SKILL_CHARS: usize = 12_000;
    let enabled = skills
        .iter()
        .filter(|skill| skill.enabled)
        .collect::<Vec<_>>();
    let mut blocks = Vec::new();
    for skill in &enabled {
        let block = format!(
            "## {}\nWhen to use: {}\nInstructions:\n{}",
            skill.name, skill.description, skill.instructions
        );
        if !blocks.is_empty()
            && blocks.iter().map(String::len).sum::<usize>() + (blocks.len() * 2) + block.len()
                > MAX_SKILL_CHARS
        {
            continue;
        }
        blocks.push(block);
    }
    let omitted = enabled.len().saturating_sub(blocks.len());
    let mut body = blocks.join("\n\n");
    if omitted > 0 {
        body.push_str(&format!(
            "\n\n[{omitted} additional skill{} omitted by the prompt budget]",
            if omitted == 1 { "" } else { "s" }
        ));
    }
    if body.trim().is_empty() {
        return None;
    }
    Some(ChatMessage::text(
        "system",
        format!(
            "Use these installed skills when relevant. Follow their instructions only if they help with the user's current request.\n\n{body}"
        ),
    ))
}

/// Dispatch every schedule occurrence due at `now_unix` through the canonical
/// control ledger. The durable command receipts make retries idempotent.
pub async fn fire_due(state: &AppState, now_unix: i64) -> Result<usize> {
    let Some(schedules) = state.schedules.clone() else {
        return Ok(0);
    };
    let due = schedules.due(now_unix)?;
    let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut jobs = tokio::task::JoinSet::new();
    for occurrence in due {
        let state = state.clone();
        let schedules = schedules.clone();
        let limit = limit.clone();
        jobs.spawn(async move {
            let _permit = limit.acquire_owned().await.ok();
            let schedule_id = occurrence.schedule.id.clone();
            let handled = fire_schedule(state, occurrence).await?;
            if handled {
                schedules.mark_ran(&schedule_id, now_unix)?;
            }
            Ok::<bool, Error>(handled)
        });
    }

    let mut fired = 0;
    let mut first_error = None;
    while let Some(result) = jobs.join_next().await {
        match result {
            Ok(Ok(true)) => fired += 1,
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                tracing::warn!("scheduled run dispatch failed: {error}");
                first_error.get_or_insert(error);
            }
            Err(error) => {
                tracing::warn!("scheduled run task failed: {error}");
                first_error.get_or_insert_with(|| Error::Other(error.to_string()));
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(fired),
    }
}

async fn fire_schedule(state: AppState, occurrence: milim_automation::DueSchedule) -> Result<bool> {
    let schedule = &occurrence.schedule;
    let control = state.control.clone().ok_or_else(|| {
        Error::Other("scheduled runs require the canonical control manager".to_string())
    })?;
    let model = if schedule.model.trim().is_empty() {
        schedule
            .agent_id
            .as_deref()
            .and_then(|id| state.agents.as_ref()?.get(id).ok().flatten())
            .map(|agent| agent.model)
            .unwrap_or_default()
    } else {
        schedule.model.trim().to_string()
    };
    let thread_id = format!("schedule-{}-{}", schedule.id, occurrence.scheduled_for);
    let origin = ThreadOriginV1::Schedule {
        schedule_id: schedule.id.clone(),
        schedule_name: schedule.name.clone(),
        occurrence_unix: occurrence.scheduled_for,
    };
    let created = control
        .command(
            state.clone(),
            Some("system:scheduler".to_string()),
            ControlCommandV1 {
                command_id: format!(
                    "schedule:{}:{}:create",
                    schedule.id, occurrence.scheduled_for
                ),
                kind: ControlCommandKindV1::ThreadCreate,
                thread_id: None,
                expected_revision: None,
                payload: serde_json::json!({
                    "id": thread_id,
                    "title": format!("Schedule: {}", schedule.name),
                    "settings": {
                        "model": model,
                        "folder": schedule.workspace,
                        "activeAgentId": schedule.agent_id,
                        "toolApproval": "guarded",
                        "memory": false,
                        "delegationPolicy": "off",
                        "computerUse": false,
                        "sandbox": false,
                        "planMode": false,
                        "privacy": "off"
                    },
                    "origin": origin,
                }),
                confirmation_token: None,
            },
        )
        .await?;
    if created.status == ControlCommandStatusV1::Failed {
        return Ok(false);
    }

    if let Err(error) =
        milim_automation::message_with_attachments(&schedule.prompt, &schedule.attachments)
    {
        control.record_schedule_error(&thread_id, &error.to_string())?;
        return Ok(true);
    }
    let prompt = milim_automation::prompt_with_attachments(&schedule.prompt, &schedule.attachments);

    let attachments = schedule
        .attachments
        .iter()
        .map(|attachment| {
            serde_json::json!({
                "id": attachment.id,
                "name": attachment.name,
                "mime": attachment.mime,
                "size": attachment.size,
                "content": attachment.content,
                "data_url": attachment.data_url,
                "truncated": attachment.truncated,
            })
        })
        .collect::<Vec<_>>();
    let sent = control
        .command(
            state,
            Some("system:scheduler".to_string()),
            ControlCommandV1 {
                command_id: format!("schedule:{}:{}:send", schedule.id, occurrence.scheduled_for),
                kind: ControlCommandKindV1::TurnSend,
                thread_id: Some(thread_id.clone()),
                expected_revision: None,
                payload: serde_json::json!({
                    "text": prompt,
                    "display_text": schedule.prompt,
                    "attachments": attachments,
                }),
                confirmation_token: None,
            },
        )
        .await?;
    if sent.status == ControlCommandStatusV1::Failed {
        control.record_schedule_error(
            &thread_id,
            sent.message
                .as_deref()
                .unwrap_or("The scheduled turn could not be accepted."),
        )?;
    }
    Ok(true)
}

/// Run the background scheduler loop (checks for due schedules every 30s).
pub async fn scheduler_loop(state: AppState) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        ticker.tick().await;
        match fire_due(&state, now_unix() as i64).await {
            Ok(n) if n > 0 => tracing::info!("scheduler fired {n} run(s)"),
            Ok(_) => {}
            Err(e) => tracing::warn!("schedule fire_due failed: {e}"),
        }
    }
}

/// Spawn the background scheduler loop on the currently entered Tokio runtime.
pub fn spawn_scheduler(state: AppState) {
    tokio::spawn(scheduler_loop(state));
}

#[cfg(test)]
mod tests {
    use super::*;

    use milim_core::config::ServerConfiguration;
    use milim_inference::test_backend::TestBackend;
    use milim_storage::{Database, UserDataStore};
    use std::sync::Arc;

    fn control_state() -> (AppState, Arc<control::RunManager>, Arc<UserDataStore>) {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        let control = control::RunManager::new(store.clone(), "Schedule fixture").unwrap();
        let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
            .with_control(control.clone());
        (state, control, store)
    }

    fn occurrence(schedule: milim_automation::Schedule) -> milim_automation::DueSchedule {
        milim_automation::DueSchedule {
            schedule,
            scheduled_for: 3_600,
        }
    }

    #[tokio::test]
    async fn scheduled_occurrence_creates_one_canonical_thread_and_run() {
        let (state, control, store) = control_state();
        let schedule = milim_automation::Schedule {
            id: "daily-review".to_string(),
            name: "Daily review".to_string(),
            cron: "0 0 * * * *".to_string(),
            agent_id: None,
            model: "test-echo".to_string(),
            prompt: "hello".to_string(),
            attachments: Vec::new(),
            enabled: true,
            workspace: None,
            created_unix: 0,
            last_run: None,
        };

        assert!(fire_schedule(state.clone(), occurrence(schedule.clone()))
            .await
            .unwrap());
        assert!(fire_schedule(state.clone(), occurrence(schedule))
            .await
            .unwrap());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let bootstrap = control.bootstrap(&state).await.unwrap();
        assert_eq!(bootstrap.threads.len(), 1);
        assert_eq!(bootstrap.threads[0].title, "Schedule: Daily review");
        assert!(matches!(
            bootstrap.threads[0].origin,
            Some(ThreadOriginV1::Schedule { ref schedule_id, occurrence_unix: 3_600, .. })
                if schedule_id == "daily-review"
        ));
        let runs = store.control_runs(false).unwrap();
        assert_eq!(runs.len(), 1);
    }

    #[tokio::test]
    async fn invalid_schedule_input_is_recorded_in_the_canonical_timeline() {
        let (state, control, _) = control_state();
        let schedule = milim_automation::Schedule {
            id: "legacy-image".to_string(),
            name: "Legacy image".to_string(),
            cron: "0 0 * * * *".to_string(),
            agent_id: None,
            model: "test-echo".to_string(),
            prompt: "describe it".to_string(),
            attachments: vec![milim_automation::ScheduleAttachment {
                id: "image".to_string(),
                name: "missing.png".to_string(),
                mime: "image/png".to_string(),
                size: 75,
                content: None,
                data_url: None,
                truncated: false,
                source_path: None,
            }],
            enabled: true,
            workspace: None,
            created_unix: 0,
            last_run: None,
        };

        assert!(fire_schedule(state, occurrence(schedule)).await.unwrap());
        let timeline = control
            .timeline_page("schedule-legacy-image-3600", None, None, true, 20)
            .unwrap()
            .unwrap();
        assert!(timeline.items.iter().any(|item| {
            item.item_type == "runtime_notice"
                && item.data["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("reattach it"))
        }));
    }
}
