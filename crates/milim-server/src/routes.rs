//! HTTP handlers for the OpenAI- and Ollama-compatible endpoints.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HOST,
    USER_AGENT,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use bytes::Bytes;
use futures::{future::join_all, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;

use crate::auth::authorize;
use crate::companion::{
    MobileCompanionBridge, MobilePairRequest, MobileRelayRequest, MobileThreadUpdateRequest,
};
use crate::error::ApiError;
use crate::media_library::{
    MediaDownloadSource, MediaLibraryItem, MediaLibraryMediaItem, MediaLibraryUpdate,
    NewMediaLibraryItem,
};
use crate::preview_runtime::{
    PreviewAppPreflightRequest, PreviewAppStageRequest, PreviewAppStartRequest,
};
use crate::privacy::{kinds_summary, PrivacyMode};
use crate::sse::{agent_sse, anthropic_sse, ollama_ndjson, openai_sse, ChunkCtx};
use crate::state::{AppState, McpAppViewDocument};
use crate::threads::{missing_threads_error, ChildRunSpec, SupervisorEvent, ThreadSupervisor};
use crate::translate::{
    anthropic_response_blocks, anthropic_stop_reason, anthropic_to_completion,
    ollama_format_to_response_format, ollama_think_effort, ollama_to_completion,
    openai_to_completion,
};
use crate::{gen_id, now_unix, rfc3339_now};
use milim_core::api::anthropic::{self, MessagesRequest, MessagesResponse};
use milim_core::api::ollama::{
    OllamaChatRequest, OllamaChatResponse, OllamaMessage, OllamaModelDetails, OllamaModelTag,
    OllamaTagsResponse,
};
use milim_core::api::openai::{
    ChatCompletionRequest, ChatCompletionResponse, ChatMessage, Choice, Content, ContentPart,
    FunctionCall, ImageUrl, Model, ModelsResponse, ReasoningEffort, StringOrArray,
    Tool as OpenAiTool, ToolCall, ToolFunction, Usage,
};
use milim_core::Error;
use milim_inference::remote::RemoteBackend;
use milim_inference::{
    CompletionRequest, EventStream, ModelService, SamplingParams, StreamEvent, ToolCallAccumulator,
};
use milim_tools::{Tool, ToolEffect, ToolRegistry};

// axum 0.8 routes `Option<T>` through `OptionalFromRequestParts`, which
// `ConnectInfo` does not implement -- so extract it directly. `serve_listener`
// always attaches connect-info, so this is present for every request.
type Peer = ConnectInfo<SocketAddr>;

fn peer_addr(peer: Peer) -> Option<SocketAddr> {
    Some(peer.0)
}

fn agent_run_config_from_request(req: &ChatCompletionRequest) -> milim_agents::AgentRunConfig {
    let mut config = milim_agents::AgentRunConfig::default();
    if let Some(max_iterations) = req
        .extra
        .get("agent_max_iterations")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    {
        config.max_iterations = max_iterations.max(1);
    }
    config
}

/// `GET /health`
pub(crate) async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "milim" }))
}

fn mobile_bridge(st: &AppState) -> Result<Arc<MobileCompanionBridge>, ApiError> {
    st.mobile_companion.as_ref().cloned().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "mobile companion bridge is not available".to_string(),
        ))
    })
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

fn companion_device_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    bearer_token(headers).ok_or_else(|| {
        ApiError(Error::Unauthorized(
            "missing mobile companion device key".to_string(),
        ))
    })
}

#[derive(Deserialize)]
pub(crate) struct MobileCompanionEnabledRequest {
    enabled: bool,
}

/// `GET /mobile`
pub(crate) async fn mobile_companion_page() -> Html<&'static str> {
    Html(MOBILE_COMPANION_HTML)
}

/// `GET /mobile/manifest.webmanifest`
pub(crate) async fn mobile_companion_manifest() -> Response {
    (
        [(CONTENT_TYPE, "application/manifest+json")],
        MOBILE_COMPANION_MANIFEST,
    )
        .into_response()
}

/// `GET /mobile/sw.js`
pub(crate) async fn mobile_companion_service_worker() -> Response {
    (
        [(CONTENT_TYPE, "application/javascript")],
        MOBILE_COMPANION_SERVICE_WORKER,
    )
        .into_response()
}

/// `GET /mobile/icon.svg`
pub(crate) async fn mobile_companion_icon() -> Response {
    ([(CONTENT_TYPE, "image/svg+xml")], MOBILE_COMPANION_ICON).into_response()
}

/// `GET /mobile/icon.png`
pub(crate) async fn mobile_companion_icon_png() -> Response {
    ([(CONTENT_TYPE, "image/png")], MOBILE_COMPANION_ICON_PNG).into_response()
}

/// `GET /mobile/wordmark.svg`
pub(crate) async fn mobile_companion_wordmark() -> Response {
    ([(CONTENT_TYPE, "image/svg+xml")], MOBILE_COMPANION_WORDMARK).into_response()
}

/// `GET /mobile/status`
pub(crate) async fn mobile_companion_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    Ok(Json(bridge.status(now_unix())).into_response())
}

/// `POST /mobile/enabled`
pub(crate) async fn mobile_companion_enabled(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MobileCompanionEnabledRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    Ok(Json(bridge.set_enabled(req.enabled, now_unix())).into_response())
}

/// `POST /mobile/pairing`
pub(crate) async fn mobile_companion_pairing(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    let pairing = bridge.start_pairing(now_unix()).map_err(|message| {
        ApiError(Error::InvalidRequest(format!(
            "could not start mobile pairing: {message}"
        )))
    })?;
    Ok(Json(pairing).into_response())
}

/// `POST /mobile/pair`
pub(crate) async fn mobile_companion_pair(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MobilePairRequest>,
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let pair = bridge
        .pair_device(req, now_unix(), user_agent)
        .map_err(|message| ApiError(Error::Unauthorized(message)))?;
    Ok(Json(pair).into_response())
}

/// `GET /mobile/device/status`
pub(crate) async fn mobile_companion_device_status(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let key = companion_device_key(&headers)?;
    let device = bridge.authenticate_device(key, now_unix()).ok_or_else(|| {
        ApiError(Error::Unauthorized(
            "invalid mobile companion device key".to_string(),
        ))
    })?;
    Ok(Json(json!({ "connected": true, "device": device })).into_response())
}

/// `POST /mobile/relay`
pub(crate) async fn mobile_companion_relay(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<MobileRelayRequest>,
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let key = companion_device_key(&headers)?;
    let event = bridge
        .submit_relay(key, req, now_unix())
        .map_err(|message| ApiError(Error::Unauthorized(message)))?;
    Ok(Json(json!({ "ok": true, "event": event })).into_response())
}

/// `GET /mobile/thread`
pub(crate) async fn mobile_companion_thread(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let key = companion_device_key(&headers)?;
    let thread = bridge
        .thread_for_device(key, now_unix())
        .map_err(|message| ApiError(Error::Unauthorized(message)))?;
    Ok(Json(json!({ "thread": thread })).into_response())
}

/// `GET /mobile/thread/events`
pub(crate) async fn mobile_companion_thread_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let key = bearer_token(&headers)
        .map(str::to_string)
        .or_else(|| query.get("key").cloned())
        .ok_or_else(|| {
            ApiError(Error::Unauthorized(
                "missing mobile companion device key".to_string(),
            ))
        })?;
    bridge
        .authenticate_device(&key, now_unix())
        .ok_or_else(|| {
            ApiError(Error::Unauthorized(
                "invalid mobile companion device key".to_string(),
            ))
        })?;
    let mut updates = bridge.subscribe_thread();
    let stream = async_stream::stream! {
        match bridge.thread_for_device(&key, now_unix()) {
            Ok(thread) => yield Ok::<Event, Infallible>(Event::default().data(json!({ "thread": thread }).to_string())),
            Err(_) => return,
        }
        loop {
            if updates.changed().await.is_err() {
                break;
            }
            match bridge.thread_for_device(&key, now_unix()) {
                Ok(thread) => yield Ok::<Event, Infallible>(Event::default().data(json!({ "thread": thread }).to_string())),
                Err(_) => break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `POST /mobile/thread`
pub(crate) async fn mobile_companion_thread_update(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MobileThreadUpdateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    let thread = bridge.update_thread(req, now_unix());
    Ok(Json(json!({ "thread": thread })).into_response())
}

/// `GET /mobile/events`
pub(crate) async fn mobile_companion_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    Ok(Json(json!({ "events": bridge.take_events() })).into_response())
}

/// `DELETE /mobile/devices/{id}`
pub(crate) async fn mobile_companion_device_revoke(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let bridge = mobile_bridge(&st)?;
    Ok(Json(bridge.revoke_device(&id, now_unix())).into_response())
}

const MOBILE_COMPANION_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover" />
  <title>Milim Mobile Companion</title>
  <link rel="manifest" href="/mobile/manifest.webmanifest" />
  <link rel="icon" href="/mobile/icon.png" type="image/png" />
  <link rel="apple-touch-icon" href="/mobile/icon.png" />
  <meta name="theme-color" content="#0d0d0f" />
  <meta name="mobile-web-app-capable" content="yes" />
  <meta name="apple-mobile-web-app-capable" content="yes" />
  <meta name="apple-mobile-web-app-title" content="Milim" />
  <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent" />
  <style>
    :root { color-scheme: dark; font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #0d0d0f; color: #ededf0; overflow-x: hidden; }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      min-height: 100svh;
      overflow-x: hidden;
      background:
        linear-gradient(180deg, rgba(255,255,255,.035), transparent 180px),
        repeating-linear-gradient(90deg, rgba(255,255,255,.018) 0 1px, transparent 1px 88px),
        #0d0d0f;
    }
    main { width: min(100%, 560px); min-height: 100svh; margin: 0 auto; display: grid; grid-template-rows: auto 1fr auto; gap: 14px; padding: max(14px, env(safe-area-inset-top)) 12px max(14px, env(safe-area-inset-bottom)); }
    header { min-height: 44px; display: flex; align-items: center; justify-content: space-between; gap: 12px; border-bottom: 1px solid #262629; }
    .brand { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
    .wordmark { font-size: 16px; font-weight: 760; letter-spacing: -.03em; }
    h1 { margin: 0; color: #a0a0a8; font-size: 12px; font-weight: 520; letter-spacing: 0; }
    .status { display: inline-flex; align-items: center; gap: 6px; border: 1px solid #323237; background: #161618; border-radius: 8px; padding: 5px 8px; font-size: 11px; color: #a0a0a8; white-space: nowrap; }
    .status::before { content: ""; width: 6px; height: 6px; border-radius: 999px; background: #fbbf24; box-shadow: 0 0 0 3px rgba(251,191,36,.12); }
    .status.ok::before { background: #34d399; box-shadow: 0 0 0 3px rgba(52,211,153,.12); }
    .status.stale::before { background: #fb7185; box-shadow: 0 0 0 3px rgba(251,113,133,.12); }
    .panel { display: grid; gap: 16px; align-content: start; align-self: start; border: 1px solid #262629; border-radius: 8px; background: rgba(22,22,24,.96); padding: 14px; box-shadow: inset 0 1px 0 rgba(255,255,255,.05), 0 18px 50px rgba(0,0,0,.28); animation: panel-in .18s ease-out; }
    .panel-head { display: grid; gap: 5px; }
    h2 { margin: 0; font-size: clamp(28px, 9vw, 44px); line-height: .95; letter-spacing: -.055em; }
    .panel-head p { margin: 0; color: #a0a0a8; font-size: 13px; line-height: 1.45; }
    label { display: grid; gap: 7px; font-size: 12px; color: #a0a0a8; }
    input, textarea { width: 100%; border: 1px solid #323237; border-radius: 8px; background: #0d0d0f; color: #ededf0; padding: 12px; font: inherit; outline: none; transition: border-color .15s, box-shadow .15s, transform .15s; }
    input:focus, textarea:focus { border-color: #55555e; box-shadow: 0 0 0 2px rgba(237,237,240,.12); }
    input::placeholder, textarea::placeholder { color: #71717a; }
    textarea { min-height: 96px; resize: vertical; line-height: 1.5; }
    .top-actions { display: flex; gap: 8px; align-items: center; }
    .mini-btn { min-height: 30px; padding: 0 9px; background: transparent; color: #ededf0; border-color: #323237; font-size: 12px; }
    .mini-btn.danger { color: var(--error); }
    .thread-panel { min-height: calc(100svh - 142px); grid-template-rows: auto auto auto auto minmax(0, 1fr) auto auto auto; }
    .thread-head { display: grid; grid-template-columns: minmax(0, 1fr) auto auto; align-items: start; gap: 8px; border-bottom: 1px solid #262629; padding-bottom: 12px; }
    .thread-head h2 { font-size: 22px; line-height: 1.05; letter-spacing: -.04em; }
    .thread-head p { margin: 4px 0 0; color: #71717a; font-size: 12px; }
    .thread-state { flex: none; border: 1px solid #323237; border-radius: 7px; padding: 4px 7px; color: #a0a0a8; font-size: 11px; }
    .thread-switch { min-height: 28px; padding: 0 9px; background: transparent; color: #ededf0; border-color: #323237; font-size: 12px; }
    .thread-list { max-height: 34svh; overflow: auto; display: grid; gap: 6px; border-bottom: 1px solid #262629; padding-bottom: 12px; scrollbar-width: thin; }
    .worker-run-mobile { display: grid; gap: 9px; border: 1px solid #323237; border-radius: 8px; padding: 10px; background: #0d0d0f; }
    .worker-run-mobile-head, .worker-run-mobile-actions { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
    .worker-run-mobile-tasks { display: grid; gap: 6px; color: #a0a0a8; font-size: 11px; }
    .worker-run-mobile-task { display: flex; justify-content: space-between; gap: 8px; }
    .worker-run-mobile-actions button { min-height: 32px; padding: 0 9px; font-size: 11px; }
    .thread-item { min-height: 0; display: grid; gap: 3px; text-align: left; border-color: #262629; background: #0d0d0f; color: #ededf0; padding: 9px 10px; }
    .thread-item.active { background: #ededf0; color: #0d0d0f; border-color: transparent; }
    .thread-item small { color: #71717a; font-size: 11px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .thread-item.active small { color: #3f3f46; }
    .thread-messages { min-height: 0; overflow: auto; display: grid; align-content: start; gap: 10px; padding: 2px 1px 4px; scrollbar-width: thin; }
    select { width: 100%; border: 1px solid #323237; border-radius: 8px; background: #0d0d0f; color: #ededf0; padding: 10px; font: inherit; }
    .thread-search { margin-bottom: -6px; }
    .msg-wrap { display: grid; gap: 4px; max-width: 88%; }
    .msg-wrap.user { justify-self: end; }
    .msg-wrap.assistant { justify-self: start; }
    .msg { width: fit-content; max-width: 100%; border: 1px solid #262629; border-radius: 8px; padding: 9px 10px; white-space: pre-wrap; overflow-wrap: anywhere; font-size: 13px; line-height: 1.45; }
    .msg.user { justify-self: end; background: #ededf0; color: #0d0d0f; border-color: transparent; }
    .msg.assistant { justify-self: start; background: #0d0d0f; color: #ededf0; }
    .msg-actions { display: flex; gap: 5px; opacity: .82; }
    .msg-actions button { min-height: 26px; padding: 0 7px; background: transparent; color: #a0a0a8; border-color: #323237; font-size: 11px; }
    .empty-thread { align-self: center; justify-self: center; color: #71717a; text-align: center; font-size: 13px; line-height: 1.45; }
    .compose { display: grid; gap: 8px; }
    .actions { display: grid; grid-template-columns: 1fr auto; gap: 8px; }
    .pair-actions { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; }
    .pair-link-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; }
    .pair-link-block { display: grid; gap: 7px; }
    .pair-scanner { width: 100%; aspect-ratio: 4 / 3; object-fit: cover; border: 1px solid #323237; border-radius: 8px; background: #050507; }
    .attachment-tray { display: flex; flex-wrap: wrap; gap: 6px; }
    .attachment-pill { display: inline-flex; align-items: center; gap: 6px; max-width: 100%; border: 1px solid #323237; border-radius: 7px; padding: 5px 7px; background: #0d0d0f; color: #a0a0a8; font-size: 11px; }
    .attachment-pill span { max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .settings-panel { display: grid; gap: 8px; border-top: 1px solid #262629; padding-top: 10px; color: #a0a0a8; font-size: 12px; }
    button { min-height: 44px; border: 1px solid transparent; border-radius: 7px; padding: 0 12px; font: inherit; font-weight: 650; color: #0d0d0f; background: #ededf0; cursor: pointer; transition: transform .15s, border-color .15s, background .15s, color .15s; }
    button:hover { transform: translateY(-1px); }
    button:active { transform: translateY(0) scale(.99); }
    button.secondary { background: transparent; color: #ededf0; border-color: #323237; }
    button.secondary:hover { background: #1f1f23; border-color: #55555e; }
    button:disabled { opacity: .45; }
    .notice { min-height: 20px; margin: 0; color: #71717a; font-size: 12px; line-height: 1.45; }
    .error { color: #f87171; }
    .hidden { display: none; }
    footer { display: grid; gap: 8px; color: #71717a; font-size: 12px; line-height: 1.45; }
    .limits { display: flex; flex-wrap: wrap; gap: 6px; }
    .limits span { border: 1px solid #262629; border-radius: 7px; padding: 5px 7px; background: rgba(22,22,24,.72); }
    .install { justify-self: start; min-height: 34px; background: transparent; color: #ededf0; border-color: #323237; font-size: 12px; }
    @keyframes panel-in {
      from { opacity: 0; transform: translateY(6px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes drawer-fade {
      from { opacity: 0; }
      to { opacity: 1; }
    }
    @media (prefers-reduced-motion: reduce) {
      .panel, .thread-drawer-backdrop { animation: none; }
      input, textarea, button, .thread-drawer { transition: none; }
    }
    /* Milim mobile shell: mirror desktop primitives without pulling in React. */
    :root {
      --bg-primary: #0d0d0f;
      --bg-secondary: #161618;
      --bg-tertiary: #1f1f23;
      --panel-bg: #161618;
      --sidebar-bg: #0a0a0c;
      --input-bg: #161618;
      --input-border: #323237;
      --border-primary: #262629;
      --border-secondary: #323237;
      --focus-border: #55555e;
      --glass-edge: rgba(255,255,255,0.08);
      --primary-text: #ededf0;
      --secondary-text: #a0a0a8;
      --tertiary-text: #71717a;
      --placeholder-text: #71717a;
      --accent: #ededf0;
      --accent-light: #c8c8d0;
      --accent-contrast: #10131a;
      --accent-soft: rgba(237, 237, 240, .14);
      --accent-15: rgba(237, 237, 240, .16);
      --accent-glow: rgba(237, 237, 240, .38);
      --chip-bg: #1f1f23;
      --chip-hover: #323237;
      --error: #f87171;
      --warning: #fbbf24;
      --success: #34d399;
      --blur: 0px;
      --bg-image: none;
      --bg-image-opacity: 1;
      --bg-image-blur: 0px;
      --overlay-color: #000000;
      --overlay-opacity: 0;
      --topbar-bg: var(--bg-primary);
      --app-height: 100svh;
      --viewport-top: 0px;
      --composer-height: 0px;
      --message-actions-inset: 36px;
      --shadow-soft: none;
      --shadow-strong: 0 18px 40px rgba(0,0,0,.52), 0 0 0 1px rgba(255,255,255,.04);
      --panel-row-hover: color-mix(in srgb, var(--chip-bg) 68%, transparent);
      --panel-row-active: color-mix(in srgb, var(--chip-bg) 88%, transparent);
      --panel-muted-bg: color-mix(in srgb, var(--bg-tertiary) 38%, transparent);
      --popover-bg: var(--panel-bg);
      --popover-border: var(--border-primary);
      --popover-blur: var(--blur);
      --card-radius: 12px;
      --input-radius: 10px;
      --chip-radius: 8px;
      --font: "Segoe UI Variable", "Segoe UI", ui-sans-serif, system-ui, sans-serif;
      --mono: "Cascadia Mono", "SFMono-Regular", Consolas, ui-monospace, monospace;
      font-family: var(--font);
      background: var(--bg-primary);
      color: var(--primary-text);
    }
    html {
      width: 100%;
      height: 100%;
      overflow: hidden;
      overscroll-behavior: none;
    }
    body {
      position: fixed;
      inset: 0;
      width: 100%;
      height: 100%;
      max-height: 100%;
      overflow: hidden;
      overscroll-behavior: none;
      background: var(--bg-primary);
      color: var(--primary-text);
      touch-action: manipulation;
      -webkit-font-smoothing: antialiased;
      text-rendering: optimizeLegibility;
    }
    body::before {
      content: "";
      position: fixed;
      inset: 0;
      z-index: 0;
      pointer-events: none;
      background-color: var(--bg-primary);
      background-image: var(--bg-image);
      background-size: cover;
      background-position: center;
      background-repeat: no-repeat;
      opacity: var(--bg-image-opacity, 1);
      filter: blur(var(--bg-image-blur, 0px));
      transform: scale(1.06);
    }
    body::after {
      content: "";
      position: fixed;
      inset: 0;
      z-index: 0;
      pointer-events: none;
      background: var(--overlay-color);
      opacity: var(--overlay-opacity);
    }
    body.bg-fit-cover::before {
      background-size: cover;
      background-repeat: no-repeat;
      background-position: center;
    }
    body.bg-fit-contain::before {
      background-size: contain;
      background-repeat: no-repeat;
      background-position: center;
      transform: none;
    }
    body.bg-fit-center::before {
      background-size: auto;
      background-repeat: no-repeat;
      background-position: center;
      transform: none;
    }
    body.bg-fit-tile::before {
      background-size: auto;
      background-repeat: repeat;
      background-position: top left;
      transform: none;
    }
    body.bg-treatment-blur::before {
      filter: blur(calc(var(--bg-image-blur, 0px) + 8px));
      transform: scale(1.09);
    }
    body.bg-treatment-mono::before {
      filter: grayscale(1) blur(var(--bg-image-blur, 0px));
    }
    body.bg-fit-tile.bg-treatment-blur::before {
      transform: none;
    }
    body.bg-treatment-dim::after {
      opacity: calc(var(--overlay-opacity) + 0.18);
    }
    main {
      position: relative;
      z-index: 1;
      width: min(100%, 760px);
      height: var(--app-height);
      min-height: var(--app-height);
      gap: 0;
      padding: env(safe-area-inset-top) 0 0;
      grid-template-rows: 44px minmax(0, 1fr);
      background: color-mix(in srgb, var(--bg-primary) 88%, transparent);
      border-inline: 1px solid var(--border-primary);
      box-shadow: 0 24px 80px rgba(0,0,0,.28);
      transform: translateY(var(--viewport-top));
      overflow: hidden;
      overscroll-behavior: none;
    }
    header {
      height: 44px;
      min-height: 44px;
      padding: 0 8px;
      border-bottom: 1px solid var(--border-primary);
      background: var(--topbar-bg);
      backdrop-filter: blur(var(--blur));
    }
    .top-actions { min-width: 0; }
    .brand { align-items: center; }
    .wordmark {
      width: 22px;
      height: 18px;
      background: currentColor;
      color: var(--primary-text);
      -webkit-mask: url("/mobile/wordmark.svg") center / contain no-repeat;
      mask: url("/mobile/wordmark.svg") center / contain no-repeat;
    }
    h1 { color: var(--tertiary-text); font-size: 11px; font-weight: 650; }
    .status {
      display: none;
      height: 28px;
      max-width: 72px;
      overflow: hidden;
      text-overflow: ellipsis;
      border-color: var(--border-primary);
      background: transparent;
      border-radius: 8px;
      color: var(--secondary-text);
      font-family: var(--mono);
      font-variant-numeric: tabular-nums;
    }
    .status::before { background: var(--warning); box-shadow: 0 0 0 3px color-mix(in srgb, var(--warning) 18%, transparent); }
    .status.ok::before { background: var(--success); box-shadow: 0 0 0 3px color-mix(in srgb, var(--success) 18%, transparent); }
    .status.stale::before { background: var(--error); box-shadow: 0 0 0 3px color-mix(in srgb, var(--error) 18%, transparent); }
    .panel {
      align-self: stretch;
      min-height: 0;
      border: 0;
      border-radius: 0;
      padding: 0;
      background: transparent;
      box-shadow: none;
      animation: none;
    }
    #pairPanel {
      align-content: center;
      justify-items: center;
      overflow: auto;
      padding: 18px 14px calc(18px + env(safe-area-inset-bottom));
      background: transparent;
    }
    .pair-card {
      width: min(100%, 300px);
      display: grid;
      gap: 12px;
      padding: 13px;
      border: 1px solid var(--border-primary);
      border-radius: var(--card-radius);
      background: var(--panel-bg);
      box-shadow: inset 0 1px 0 var(--glass-edge);
    }
    #pairPanel .panel-head { gap: 5px; }
    #pairPanel h2 { max-width: none; font-size: 22px; line-height: 1.05; letter-spacing: -.04em; text-wrap: balance; }
    #pairPanel .panel-head p { max-width: 36ch; color: var(--secondary-text); font-size: 12.5px; }
    .pair-actions,
    .pair-link-row { grid-template-columns: 1fr; }
    .pair-actions,
    .pair-link-row { width: 100%; }
    .pair-link-row button { justify-self: stretch; }
    .thread-panel {
      position: relative;
      height: calc(var(--app-height) - 44px - env(safe-area-inset-top));
      min-height: 0;
      grid-template-rows: auto auto minmax(0, 1fr) auto auto;
      gap: 0;
      overflow: hidden;
    }
    .thread-head {
      min-height: 50px;
      padding: 8px 10px;
      border-bottom: 1px solid var(--border-primary);
      background: var(--sidebar-bg);
      backdrop-filter: blur(var(--blur));
    }
    .thread-head h2 {
      max-width: 100%;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-size: 17px;
      font-weight: 720;
      letter-spacing: -.02em;
    }
    .thread-head p {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--tertiary-text);
      font-family: var(--mono);
      font-size: 11px;
    }
    .thread-state,
    .thread-switch,
    .mini-btn {
      min-height: 30px;
      border-radius: 8px;
      border-color: transparent;
      background: transparent;
      color: var(--secondary-text);
      font-size: 12px;
    }
    .thread-state {
      border-color: var(--border-primary);
      background: transparent;
      color: var(--tertiary-text);
      font-family: var(--mono);
      font-size: 10.5px;
    }
    .thread-switch:hover,
    .mini-btn:hover {
      background: var(--chip-hover);
      color: var(--primary-text);
    }
    .mini-btn.danger { color: #fca5a5; }
    select,
    input,
    textarea {
      border-color: var(--border-primary);
      background: var(--input-bg);
      color: var(--primary-text);
      border-radius: 8px;
      font-size: 16px;
    }
    input:focus,
    textarea:focus,
    select:focus {
      border-color: var(--border-secondary);
      box-shadow: 0 0 0 3px var(--accent-soft);
    }
    .thread-search {
      padding: 7px 10px;
      margin: 0;
      border-bottom: 1px solid var(--border-primary);
      background: var(--bg-secondary);
    }
    .thread-drawer-backdrop {
      position: fixed;
      inset: 0;
      z-index: 20;
      background: color-mix(in srgb, var(--bg-primary) 58%, transparent);
      backdrop-filter: blur(2px);
      animation: drawer-fade .16s ease-out;
    }
    .thread-drawer {
      position: fixed;
      z-index: 21;
      top: calc(env(safe-area-inset-top) + 44px);
      bottom: 0;
      left: max(0px, calc((100vw - 760px) / 2));
      width: min(86vw, 340px);
      display: grid;
      grid-template-rows: auto auto auto minmax(0, 1fr);
      border-right: 1px solid var(--border-primary);
      background:
        linear-gradient(180deg, var(--sidebar-bg), color-mix(in srgb, var(--sidebar-bg) 92%, var(--bg-secondary))),
        var(--sidebar-bg);
      box-shadow: 18px 0 48px rgba(0,0,0,.34), inset -1px 0 0 var(--glass-edge);
      transform: translateX(-105%);
      transition: transform .2s ease;
      pointer-events: none;
    }
    .thread-drawer.open {
      transform: translateX(0);
      pointer-events: auto;
    }
    .thread-drawer-head {
      min-height: 48px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 10px;
      padding: 8px 8px 8px 12px;
      border-bottom: 1px solid var(--border-primary);
      background: var(--panel-bg);
    }
    .thread-drawer-head strong {
      display: block;
      color: var(--primary-text);
      font-size: 13px;
      font-weight: 760;
    }
    .thread-drawer-head span {
      display: block;
      color: var(--tertiary-text);
      font-size: 11px;
      margin-top: 2px;
    }
    .thread-drawer-head button {
      min-height: 30px;
      width: auto;
      padding: 0 9px;
    }
    .thread-drawer-actions {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 6px;
      padding: 7px 8px;
      border-bottom: 1px solid var(--border-primary);
      background: var(--sidebar-bg);
    }
    .thread-list {
      min-height: 0;
      max-height: none;
      overflow: auto;
      display: grid;
      align-content: start;
      gap: 10px;
      padding: 10px 8px 14px;
      border: 0;
      background: transparent;
      scrollbar-width: thin;
    }
    .thread-group { display: grid; gap: 5px; }
    .thread-group-head {
      display: grid;
      gap: 2px;
      padding: 7px 8px 3px;
      color: var(--tertiary-text);
      font-size: 10px;
      font-weight: 760;
      letter-spacing: .06em;
      text-transform: uppercase;
    }
    .thread-group-head span {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      color: var(--tertiary-text);
      font-family: var(--mono);
      font-size: 10px;
      font-weight: 520;
      letter-spacing: 0;
      text-transform: none;
    }
    .thread-item {
      min-height: 34px;
      padding: 5px 7px 5px 10px;
      border-color: transparent;
      border-radius: 8px;
      background: transparent;
      color: var(--secondary-text);
    }
    .thread-item.active {
      border-color: var(--border-primary);
      background: var(--panel-row-active);
      color: var(--primary-text);
    }
    .thread-item:hover { background: var(--chip-hover); }
    .thread-item.child { margin-left: 14px; }
    .thread-item small { color: var(--tertiary-text); font-family: var(--mono); }
    .thread-item.active small { color: var(--tertiary-text); }
    .thread-messages {
      min-height: 0;
      overflow-y: auto;
      overflow-x: hidden;
      overscroll-behavior: contain;
      -webkit-overflow-scrolling: touch;
      padding: 14px 12px calc(16px + var(--composer-height) + var(--message-actions-inset));
      scroll-padding-bottom: calc(16px + var(--composer-height) + var(--message-actions-inset));
      gap: 12px;
      background:
        linear-gradient(180deg, rgba(255,255,255,.018), transparent 160px),
        color-mix(in srgb, var(--bg-primary) 88%, transparent);
      backdrop-filter: blur(var(--blur));
    }
    .msg-wrap { max-width: 100%; gap: 4px; }
    .msg-wrap.user { max-width: 82%; }
    .msg {
      min-width: 0;
      border-color: var(--border-primary);
      border-radius: 8px;
      background: color-mix(in srgb, var(--panel-bg) 72%, var(--bg-primary));
      color: var(--primary-text);
      font-size: 13px;
      line-height: 1.58;
      overflow-wrap: anywhere;
      backdrop-filter: blur(var(--blur));
    }
    .msg.user {
      border-color: var(--accent-soft);
      background: var(--accent-15);
      color: var(--primary-text);
    }
    .msg.assistant {
      width: 100%;
      border-color: transparent;
      background: transparent;
      padding-inline: 2px;
      backdrop-filter: none;
    }
    .msg > :first-child { margin-top: 0; }
    .msg > :last-child { margin-bottom: 0; }
    .msg p { margin: 0 0 8px; }
    .msg h1,
    .msg h2,
    .msg h3,
    .msg h4,
    .msg h5,
    .msg h6 {
      margin: 10px 0 6px;
      font-size: 14px;
      line-height: 1.3;
      letter-spacing: 0;
    }
    .msg ul,
    .msg ol { margin: 6px 0; padding-left: 18px; }
    .msg li { margin: 3px 0; }
    .msg blockquote {
      margin: 8px 0;
      padding-left: 10px;
      border-left: 2px solid var(--border-secondary);
      color: var(--secondary-text);
    }
    .msg pre {
      max-width: 100%;
      overflow: auto;
      margin: 8px 0;
      padding: 9px 10px;
      border: 1px solid var(--border-primary);
      border-radius: 8px;
      background: var(--bg-secondary);
      font-family: var(--mono);
      font-size: 12px;
      line-height: 1.45;
      white-space: pre;
    }
    .msg code {
      border-radius: 5px;
      background: var(--chip-bg);
      padding: 1px 4px;
      font-family: var(--mono);
      font-size: .92em;
    }
    .msg pre code { background: transparent; padding: 0; }
    .msg table {
      display: block;
      max-width: 100%;
      overflow: auto;
      border-collapse: collapse;
      margin: 8px 0;
      font-size: 12px;
    }
    .msg th,
    .msg td {
      border: 1px solid var(--border-primary);
      padding: 5px 7px;
      text-align: left;
      vertical-align: top;
    }
    .msg th { background: var(--bg-secondary); }
    .msg a { color: var(--accent); text-decoration: underline; text-underline-offset: 2px; }
    .msg-actions button {
      min-height: 26px;
      border-radius: 6px;
      background: transparent;
      color: var(--tertiary-text);
      border-color: transparent;
      font-size: 11px;
    }
    .msg-actions button:hover { background: var(--chip-hover); color: var(--secondary-text); }
    .compose {
      align-self: end;
      margin: 6px 8px max(4px, env(safe-area-inset-bottom));
      padding: 6px 7px;
      gap: 4px;
      border: 1px solid var(--border-primary);
      border-radius: var(--input-radius);
      background: var(--panel-bg);
      box-shadow: inset 0 1px 0 var(--glass-edge);
      backdrop-filter: blur(var(--blur));
    }
    .composer-model-select {
      min-height: 30px;
      height: 30px;
      padding: 0 28px 0 8px;
      border-radius: 7px;
      background: var(--chip-bg);
      color: var(--secondary-text);
      font-size: 12px;
      font-weight: 650;
    }
    textarea {
      height: 44px;
      min-height: 36px;
      max-height: 26svh;
      resize: vertical;
      padding: 4px 2px;
      border: 0;
      background: transparent;
      line-height: 1.4;
    }
    .compose .actions {
      display: grid;
      grid-template-columns: 32px minmax(0, 1fr) 32px;
      align-items: center;
      gap: 6px;
      margin-top: 0;
    }
    .compose .actions button {
      width: 32px;
      min-height: 32px;
      padding: 0;
    }
    .compose .actions button span { display: none; }
    .send-stack {
      display: grid;
      width: 32px;
      height: 32px;
    }
    .send-stack button {
      grid-area: 1 / 1;
    }
    .attachment-tray:empty { display: none; }
    .compose .notice {
      min-width: 0;
      min-height: 0;
      padding: 0 4px;
      overflow: hidden;
      color: var(--tertiary-text);
      font-size: 11px;
      text-align: center;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .compose .notice:empty { display: block; }
    .compose .notice.error { color: var(--error); }
    button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 6px;
      min-height: 32px;
      border-radius: 8px;
      color: var(--accent-contrast);
      background: var(--accent);
      transition: transform .14s ease, background .14s ease, border-color .14s ease;
    }
    button.secondary {
      color: var(--secondary-text);
      background: transparent;
      border-color: var(--border-primary);
    }
    button.secondary:hover {
      background: var(--chip-hover);
      border-color: var(--border-secondary);
      color: var(--primary-text);
    }
    button:focus-visible {
      outline: 2px solid rgba(220,230,248,.62);
      outline-offset: 2px;
    }
    .button-icon {
      width: 14px;
      height: 14px;
      flex: none;
    }
    .attachment-pill,
    .limits span {
      border-color: var(--border-primary);
      background: var(--chip-bg);
      color: var(--secondary-text);
    }
    .settings-panel {
      margin: 0 10px 10px;
      padding: 10px;
      border: 1px solid var(--border-primary);
      border-radius: 10px;
      background: var(--panel-bg);
      box-shadow: inset 0 1px 0 var(--glass-edge);
      backdrop-filter: blur(var(--blur));
    }
    .notice,
    .empty-thread,
    footer {
      color: var(--tertiary-text);
    }
    .notice { padding: 0 10px; }
    .notice:empty { display: none; }
    footer { display: none; }
    @media (max-width: 430px) {
      .thread-head { grid-template-columns: minmax(0, 1fr) auto; }
      .thread-state { display: none; }
    }
    @media (min-width: 520px) {
      .status { display: inline-flex; }
    }
  </style>
</head>
<body class="theme-dark bg-fit-cover bg-treatment-clear">
  <main>
    <header>
      <div class="brand" aria-label="Milim Relay">
        <span class="wordmark" role="img" aria-label="milim"></span>
        <h1>Relay</h1>
      </div>
      <div class="top-actions">
        <button id="settingsToggle" class="mini-btn" type="button">Device</button>
        <span id="status" class="status">Disconnected</span>
      </div>
    </header>

    <section id="pairPanel" class="panel hidden">
      <div class="pair-card">
        <div class="panel-head">
          <h2>Pair this phone</h2>
          <p>Give this device a name. Milim will show it on incoming relay events.</p>
        </div>
        <label>
          Device name
          <input id="deviceName" autocomplete="name" placeholder="My phone" />
        </label>
        <div class="pair-actions">
          <button id="pairButton" type="button">Pair this device</button>
          <button id="scanPairButton" class="secondary" type="button">Scan desktop QR</button>
        </div>
        <video id="pairScanner" class="pair-scanner hidden" playsinline muted></video>
        <button id="stopScanButton" class="secondary hidden" type="button">Stop camera</button>
        <div class="pair-link-block">
          <label for="pairLinkInput">Pairing link</label>
          <div class="pair-link-row">
            <input id="pairLinkInput" inputmode="url" autocomplete="off" placeholder="Paste pairing link" />
            <button id="pairLinkButton" class="secondary" type="button">Use link</button>
          </div>
        </div>
        <p id="pairNotice" class="notice"></p>
      </div>
    </section>

    <section id="relayPanel" class="panel thread-panel hidden">
      <div class="thread-head">
        <div>
          <h2 id="threadTitle">Milim</h2>
          <p id="threadMeta">Waiting for desktop thread...</p>
        </div>
        <button id="threadsToggle" class="thread-switch" type="button">Threads</button>
        <span id="threadState" class="thread-state">Idle</span>
      </div>
      <div id="threadDrawerBackdrop" class="thread-drawer-backdrop hidden"></div>
      <aside id="threadDrawer" class="thread-drawer" aria-hidden="true" aria-label="Thread sidebar">
        <div class="thread-drawer-head">
          <div>
            <strong>Threads</strong>
            <span>Desktop sidebar</span>
          </div>
          <button id="threadDrawerClose" class="mini-btn" type="button">Close</button>
        </div>
        <div class="thread-drawer-actions">
          <button id="newThreadButton" class="mini-btn" type="button">New</button>
          <button id="renameButton" class="mini-btn" type="button">Rename</button>
        </div>
        <label class="thread-search" id="threadSearchWrap">
          Search threads
          <input id="threadSearch" placeholder="Search by title, model, or project" />
        </label>
        <div id="threadList" class="thread-list"></div>
      </aside>
      <section id="workerRunPanel" class="worker-run-mobile hidden" aria-label="Worker run">
        <div class="worker-run-mobile-head"><strong id="workerRunTitle">Workers</strong><span id="workerRunStatus"></span></div>
        <div id="workerRunTasks" class="worker-run-mobile-tasks"></div>
        <div class="worker-run-mobile-actions">
          <button id="workerRunStart" type="button">Run workers</button>
          <button id="workerRunSolo" class="secondary" type="button">Continue solo</button>
          <button id="workerRunStop" class="danger hidden" type="button">Stop</button>
        </div>
      </section>
      <div id="threadMessages" class="thread-messages"></div>
      <form id="relayForm" class="compose">
        <select id="modelSelect" class="composer-model-select" aria-label="Model"></select>
        <textarea id="relayText" autofocus aria-label="Message" placeholder="Ask from your phone..."></textarea>
        <div id="attachmentTray" class="attachment-tray"></div>
        <input id="fileInput" class="hidden" type="file" multiple accept="image/*,.txt,.md,.json,.csv,.log" />
        <div class="actions">
          <button id="attachButton" class="secondary" type="button">Attach</button>
          <p id="relayNotice" class="notice"></p>
          <span class="send-stack">
            <button id="sendButton" type="submit">Send</button>
            <button id="stopButton" class="danger hidden" type="button">Stop</button>
          </span>
        </div>
      </form>
      <div id="settingsPanel" class="settings-panel hidden">
        <div id="deviceStatus">No device details yet.</div>
        <button id="forgetButton" class="secondary danger" type="button">Forget this phone</button>
        <button id="archiveButton" class="secondary" type="button">Archive thread</button>
        <button id="deleteThreadButton" class="secondary danger" type="button">Delete thread</button>
      </div>
    </section>

    <footer>
      <div class="limits">
        <span>Live desktop mirror</span>
        <span>Thread controls</span>
        <span>Phone uploads</span>
      </div>
      <button id="installButton" class="install hidden" type="button">Add to Home Screen</button>
      <span>Actions run through the paired desktop app.</span>
    </footer>
  </main>
  <script>
    const params = new URLSearchParams(location.search);
    const MAX_TEXT_ATTACHMENT_BYTES = 128 * 1024;
    const MAX_IMAGE_PREVIEW_BYTES = 2 * 1024 * 1024;
    const MAX_ATTACHMENTS = 6;
    const state = { thread: null, device: null, attachments: [], query: "", source: null, streamLive: false, lastUpdate: 0 };
    const themeColorMeta = document.querySelector('meta[name="theme-color"]');
    const themeCssVarPattern = /^--[a-z0-9-]+$/;
    const backgroundFits = new Set(["cover", "contain", "tile", "center"]);
    const backgroundTreatments = new Set(["clear", "dim", "blur", "mono"]);
    const buttonIcons = {
      archive: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 4h16v5H4Z"/><path d="M6 9v9a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V9"/><path d="M10 13h4"/></svg>`,
      arrowUp: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M6 11l6-6 6 6"/></svg>`,
      camera: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 8h3l2-3h6l2 3h3v11H4Z"/><circle cx="12" cy="13" r="3"/></svg>`,
      copy: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h8"/></svg>`,
      gear: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 13.5a1.7 1.7 0 0 0 .3 1.9l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.9-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 0 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.9.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.9 1.7 1.7 0 0 0-1.5-1H3a2 2 0 0 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.9.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 0 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.9-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.9V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 0 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"/></svg>`,
      link: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.1 0l2-2a5 5 0 0 0-7.1-7.1l-1.1 1.1"/><path d="M14 11a5 5 0 0 0-7.1 0l-2 2a5 5 0 0 0 7.1 7.1l1.1-1.1"/></svg>`,
      paperclip: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 11.5 12.5 20a4.5 4.5 0 0 1-6.4-6.4l8.5-8.5a3 3 0 0 1 4.3 4.3l-8.6 8.5a1.5 1.5 0 0 1-2.1-2.1l7.8-7.8"/></svg>`,
      pencil: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/></svg>`,
      plus: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 5v14M5 12h14"/></svg>`,
      refresh: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-2.6-6.4M21 4v5h-5"/></svg>`,
      sidebar: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/></svg>`,
      smartphone: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="2" width="10" height="20" rx="2"/><path d="M11 18h2"/></svg>`,
      square: `<svg class="button-icon" viewBox="0 0 24 24" fill="currentColor" stroke="none"><rect x="6" y="6" width="12" height="12" rx="2.5"/></svg>`,
      trash: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13"/></svg>`,
      x: `<svg class="button-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6 6 18"/></svg>`,
    };
    const store = {
      get key() { return localStorage.getItem("milim.mobile.deviceKey") || ""; },
      set key(value) { value ? localStorage.setItem("milim.mobile.deviceKey", value) : localStorage.removeItem("milim.mobile.deviceKey"); },
      get id() { return localStorage.getItem("milim.mobile.deviceId") || ""; },
      set id(value) { value ? localStorage.setItem("milim.mobile.deviceId", value) : localStorage.removeItem("milim.mobile.deviceId"); },
      get name() { return localStorage.getItem("milim.mobile.deviceName") || ""; },
      set name(value) { value ? localStorage.setItem("milim.mobile.deviceName", value) : localStorage.removeItem("milim.mobile.deviceName"); },
    };
    const statusEl = document.getElementById("status");
    const pairPanel = document.getElementById("pairPanel");
    const relayPanel = document.getElementById("relayPanel");
    const pairNotice = document.getElementById("pairNotice");
    const relayNotice = document.getElementById("relayNotice");
    const relayText = document.getElementById("relayText");
    const relayForm = document.getElementById("relayForm");
    const threadTitle = document.getElementById("threadTitle");
    const threadMeta = document.getElementById("threadMeta");
    const threadState = document.getElementById("threadState");
    const threadsToggle = document.getElementById("threadsToggle");
    const threadDrawer = document.getElementById("threadDrawer");
    const threadDrawerBackdrop = document.getElementById("threadDrawerBackdrop");
    const threadDrawerClose = document.getElementById("threadDrawerClose");
    const threadSearch = document.getElementById("threadSearch");
    const threadList = document.getElementById("threadList");
    const threadMessages = document.getElementById("threadMessages");
    const modelSelect = document.getElementById("modelSelect");
    const sendButton = document.getElementById("sendButton");
    const stopButton = document.getElementById("stopButton");
    const workerRunPanel = document.getElementById("workerRunPanel");
    const workerRunTitle = document.getElementById("workerRunTitle");
    const workerRunStatus = document.getElementById("workerRunStatus");
    const workerRunTasks = document.getElementById("workerRunTasks");
    const workerRunStart = document.getElementById("workerRunStart");
    const workerRunSolo = document.getElementById("workerRunSolo");
    const workerRunStop = document.getElementById("workerRunStop");
    const attachmentTray = document.getElementById("attachmentTray");
    const fileInput = document.getElementById("fileInput");
    const installButton = document.getElementById("installButton");
    const deviceName = document.getElementById("deviceName");
    const pairLinkInput = document.getElementById("pairLinkInput");
    const scanPairButton = document.getElementById("scanPairButton");
    const stopScanButton = document.getElementById("stopScanButton");
    const pairScanner = document.getElementById("pairScanner");
    const settingsPanel = document.getElementById("settingsPanel");
    const deviceStatus = document.getElementById("deviceStatus");
    let installPrompt = null;
    let activePairId = params.get("pair_id") || "";
    let activePairSecret = params.get("secret") || "";
    let scannerStream = null;
    let scannerFrame = 0;
    deviceName.value = store.name || "";
    [
      ["settingsToggle", "gear", "Device"],
      ["pairButton", "smartphone", "Pair this device"],
      ["scanPairButton", "camera", "Scan desktop QR"],
      ["stopScanButton", "x", "Stop camera"],
      ["pairLinkButton", "link", "Use link"],
      ["threadsToggle", "sidebar", "Threads"],
      ["threadDrawerClose", "x", "Close"],
      ["newThreadButton", "plus", "New"],
      ["stopButton", "square", "Stop"],
      ["renameButton", "pencil", "Rename"],
      ["sendButton", "arrowUp", "Send"],
      ["attachButton", "paperclip", "Attach"],
      ["forgetButton", "smartphone", "Forget this phone"],
      ["archiveButton", "archive", "Archive thread"],
      ["deleteThreadButton", "trash", "Delete thread"],
      ["installButton", "smartphone", "Add to Home Screen"],
    ].forEach(([id, icon, label]) => setButtonIcon(document.getElementById(id), icon, label));

    if ("serviceWorker" in navigator) {
      navigator.serviceWorker.register("/mobile/sw.js", { scope: "/mobile" }).catch(() => {});
    }
    function syncVisualViewport() {
      const viewport = window.visualViewport;
      const height = Math.max(320, Math.floor(viewport?.height || window.innerHeight || document.documentElement.clientHeight));
      const top = Math.max(0, Math.floor(viewport?.offsetTop || 0));
      document.documentElement.style.setProperty("--app-height", `${height}px`);
      document.documentElement.style.setProperty("--viewport-top", `${top}px`);
      syncComposerInset();
      if (document.activeElement === relayText) {
        requestAnimationFrame(() => relayText.scrollIntoView({ block: "nearest" }));
      }
    }
    function syncComposerInset() {
      document.documentElement.style.setProperty("--composer-height", `${Math.ceil(relayForm.getBoundingClientRect().height || 0)}px`);
    }
    syncVisualViewport();
    if ("ResizeObserver" in window) new ResizeObserver(syncComposerInset).observe(relayForm);
    window.visualViewport?.addEventListener("resize", syncVisualViewport);
    window.visualViewport?.addEventListener("scroll", syncVisualViewport);
    window.addEventListener("resize", syncVisualViewport);
    relayText.addEventListener("focus", syncVisualViewport);
    relayText.addEventListener("blur", () => window.setTimeout(syncVisualViewport, 120));
    window.addEventListener("beforeinstallprompt", (event) => {
      event.preventDefault();
      installPrompt = event;
      installButton.classList.remove("hidden");
    });
    let lastTouchEnd = 0;
    document.addEventListener("gesturestart", (event) => event.preventDefault());
    document.addEventListener("gesturechange", (event) => event.preventDefault());
    document.addEventListener("touchmove", (event) => {
      if (event.touches.length > 1) event.preventDefault();
    }, { passive: false });
    document.addEventListener("touchend", (event) => {
      const now = Date.now();
      if (now - lastTouchEnd < 320) event.preventDefault();
      lastTouchEnd = now;
    }, { passive: false });
    installButton.addEventListener("click", async () => {
      if (!installPrompt) return;
      installPrompt.prompt();
      await installPrompt.userChoice.catch(() => {});
      installPrompt = null;
      installButton.classList.add("hidden");
    });

    function show(panel) {
      pairPanel.classList.toggle("hidden", panel !== "pair");
      relayPanel.classList.toggle("hidden", panel !== "relay");
    }
    function setButtonIcon(button, icon, label) {
      if (!button) return;
      button.innerHTML = `${buttonIcons[icon] || ""}<span>${label}</span>`;
      button.setAttribute("aria-label", label);
      button.title = label;
    }
    function setStatus(text, ok, stale = false) {
      statusEl.textContent = text;
      statusEl.classList.toggle("ok", ok && !stale);
      statusEl.classList.toggle("stale", stale);
      statusEl.style.color = ok ? "var(--success)" : stale ? "var(--error)" : "var(--warning)";
    }
    function setChoiceClass(prefix, value, allowed, fallback) {
      const next = allowed.has(value) ? value : fallback;
      for (const option of allowed) document.body.classList.toggle(`${prefix}-${option}`, option === next);
    }
    function applyThemeSnapshot(theme) {
      if (!theme || typeof theme !== "object") return;
      const vars = theme.css_vars && typeof theme.css_vars === "object" ? theme.css_vars : {};
      for (const [key, value] of Object.entries(vars)) {
        if (!themeCssVarPattern.test(key) || typeof value !== "string") continue;
        document.documentElement.style.setProperty(key, value);
      }
      const isDark = theme.is_dark !== false;
      document.documentElement.dataset.dark = String(isDark);
      document.documentElement.style.colorScheme = isDark ? "dark" : "light";
      document.body.classList.toggle("theme-dark", isDark);
      document.body.classList.toggle("theme-light", !isDark);
      setChoiceClass("bg-fit", theme.background_fit, backgroundFits, "cover");
      setChoiceClass("bg-treatment", theme.background_treatment, backgroundTreatments, "clear");
      if (typeof vars["--bg-primary"] === "string" && vars["--bg-primary"]) {
        themeColorMeta?.setAttribute("content", vars["--bg-primary"]);
      }
    }
    async function api(path, options = {}) {
      const headers = new Headers(options.headers || {});
      if (store.key) headers.set("Authorization", `Bearer ${store.key}`);
      if (options.body && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
      const response = await fetch(path, { ...options, headers });
      if (!response.ok) {
        let message = `HTTP ${response.status}`;
        try {
          const data = await response.json();
          message = data.error?.message || data.message || message;
        } catch {}
        throw new Error(message);
      }
      return await response.json();
    }
    async function relayAction(action, text = "", attachments = []) {
      return await api("/mobile/relay", {
        method: "POST",
        body: JSON.stringify({ action, text, attachments }),
      });
    }
    function showRelayError(error) {
      relayNotice.textContent = String(error.message || error);
      relayNotice.className = "notice error";
    }
    function renderDevice() {
      const device = state.device;
      deviceStatus.textContent = device
        ? `Paired as ${device.name}. Last seen ${new Date((device.last_seen_at || 0) * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}.`
        : "No device details yet.";
    }
    function renderModelSelect(thread) {
      modelSelect.replaceChildren();
      const models = Array.isArray(thread?.models) ? thread.models : [];
      const current = thread?.model || "";
      const seen = new Set();
      for (const model of current ? [{ id: current, provider: "Current" }, ...models] : models) {
        if (!model.id || seen.has(model.id)) continue;
        seen.add(model.id);
        const option = document.createElement("option");
        option.value = model.id;
        option.textContent = model.provider ? `${model.id} (${model.provider})` : model.id;
        modelSelect.append(option);
      }
      modelSelect.disabled = modelSelect.options.length === 0;
      if (current) modelSelect.value = current;
    }
    function threadSearchText(group, item) {
      return [
        group?.label,
        group?.subtitle,
        item?.title,
        item?.model,
        item?.project_label,
        item?.project_path,
      ].filter(Boolean).join(" ").toLowerCase();
    }
    function groupedThreads(thread) {
      const groups = Array.isArray(thread?.groups) && thread.groups.length
        ? thread.groups
        : [{ id: "threads", label: "Threads", threads: Array.isArray(thread?.threads) ? thread.threads : [] }];
      const query = state.query.trim().toLowerCase();
      return groups.map((group) => {
        const threads = Array.isArray(group.threads) ? group.threads : [];
        const visible = query
          ? threads.filter((item) => threadSearchText(group, item).includes(query))
          : threads;
        return { ...group, threads: visible };
      }).filter((group) => group.threads.length);
    }
    function renderThreadList(thread) {
      threadList.replaceChildren();
      const groups = groupedThreads(thread);
      if (!groups.length) {
        const empty = document.createElement("div");
        empty.className = "empty-thread";
        empty.textContent = state.query.trim() ? "No matching threads." : "No desktop threads yet.";
        threadList.append(empty);
        return;
      }
      for (const group of groups) {
        const section = document.createElement("section");
        section.className = "thread-group";
        const head = document.createElement("div");
        head.className = "thread-group-head";
        const label = document.createElement("strong");
        label.textContent = group.label || "Threads";
        head.append(label);
        if (group.subtitle) {
          const subtitle = document.createElement("span");
          subtitle.textContent = group.subtitle;
          head.append(subtitle);
        }
        section.append(head);
        for (const item of group.threads) {
          const button = document.createElement("button");
          button.type = "button";
          button.className = `thread-item ${item.id === thread?.session_id ? "active" : ""} ${item.parent_id ? "child" : ""}`;
          const title = document.createElement("span");
          title.textContent = item.title || "New chat";
          const meta = document.createElement("small");
          const project = group.id === "pinned" && item.project_label ? item.project_label : "";
          meta.textContent = [project, item.model, item.busy ? "Running" : ""].filter(Boolean).join(" / ");
          button.append(title, meta);
          button.addEventListener("click", () => switchThread(item.id));
          section.append(button);
        }
        threadList.append(section);
      }
    }
    function safeHref(raw) {
      try {
        const url = new URL(raw, location.href);
        return ["http:", "https:", "mailto:", "tel:"].includes(url.protocol) ? url.href : "";
      } catch {
        return "";
      }
    }
    function appendInline(parent, text) {
      let i = 0;
      const addText = (value) => { if (value) parent.append(document.createTextNode(value)); };
      while (i < text.length) {
        if (text.startsWith("`", i)) {
          const end = text.indexOf("`", i + 1);
          if (end > i + 1) {
            const code = document.createElement("code");
            code.textContent = text.slice(i + 1, end);
            parent.append(code);
            i = end + 1;
            continue;
          }
        }
        if (text.startsWith("**", i)) {
          const end = text.indexOf("**", i + 2);
          if (end > i + 2) {
            const strong = document.createElement("strong");
            appendInline(strong, text.slice(i + 2, end));
            parent.append(strong);
            i = end + 2;
            continue;
          }
        }
        if (text.startsWith("*", i)) {
          const end = text.indexOf("*", i + 1);
          if (end > i + 1) {
            const em = document.createElement("em");
            appendInline(em, text.slice(i + 1, end));
            parent.append(em);
            i = end + 1;
            continue;
          }
        }
        if (text.startsWith("[", i)) {
          const close = text.indexOf("](", i + 1);
          const end = close > i ? text.indexOf(")", close + 2) : -1;
          const href = end > close ? safeHref(text.slice(close + 2, end)) : "";
          if (href) {
            const link = document.createElement("a");
            link.href = href;
            link.target = "_blank";
            link.rel = "noreferrer";
            appendInline(link, text.slice(i + 1, close));
            parent.append(link);
            i = end + 1;
            continue;
          }
        }
        const next = ["`", "**", "*", "["]
          .map((marker) => text.indexOf(marker, i + 1))
          .filter((index) => index > i)
          .sort((a, b) => a - b)[0] ?? text.length;
        addText(text.slice(i, next));
        i = next;
      }
    }
    function tableCells(line) {
      return line.trim().replace(/^\||\|$/g, "").split("|").map((cell) => cell.trim());
    }
    function isTableSeparator(line) {
      return /^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?\s*$/.test(line);
    }
    function isListLine(line, ordered) {
      return ordered ? /^\s*\d+[.)]\s+/.test(line) : /^\s*[-*+]\s+/.test(line);
    }
    function isBlockStart(lines, index) {
      const line = lines[index] || "";
      return /^```/.test(line)
        || /^#{1,6}\s+/.test(line)
        || /^>\s?/.test(line)
        || isListLine(line, true)
        || isListLine(line, false)
        || (line.includes("|") && isTableSeparator(lines[index + 1] || ""));
    }
    function renderMarkdown(target, source) {
      const lines = String(source || "").replace(/\r\n?/g, "\n").split("\n");
      target.replaceChildren();
      let i = 0;
      while (i < lines.length) {
        const line = lines[i];
        if (!line.trim()) { i += 1; continue; }
        if (line.startsWith("```")) {
          const codeLines = [];
          i += 1;
          while (i < lines.length && !lines[i].startsWith("```")) codeLines.push(lines[i++]);
          if (i < lines.length) i += 1;
          const pre = document.createElement("pre");
          const code = document.createElement("code");
          code.textContent = codeLines.join("\n");
          pre.append(code);
          target.append(pre);
          continue;
        }
        const heading = /^(#{1,6})\s+(.+)$/.exec(line);
        if (heading) {
          const el = document.createElement(`h${heading[1].length}`);
          appendInline(el, heading[2]);
          target.append(el);
          i += 1;
          continue;
        }
        if (line.includes("|") && isTableSeparator(lines[i + 1] || "")) {
          const headers = tableCells(line);
          i += 2;
          const table = document.createElement("table");
          const thead = table.createTHead();
          const headerRow = thead.insertRow();
          headers.forEach((header) => {
            const th = document.createElement("th");
            appendInline(th, header);
            headerRow.append(th);
          });
          const body = table.createTBody();
          while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
            const row = body.insertRow();
            tableCells(lines[i++]).forEach((cell) => {
              const td = row.insertCell();
              appendInline(td, cell);
            });
          }
          target.append(table);
          continue;
        }
        if (/^>\s?/.test(line)) {
          const quote = document.createElement("blockquote");
          const parts = [];
          while (i < lines.length && /^>\s?/.test(lines[i])) parts.push(lines[i++].replace(/^>\s?/, ""));
          appendInline(quote, parts.join(" "));
          target.append(quote);
          continue;
        }
        if (isListLine(line, true) || isListLine(line, false)) {
          const ordered = isListLine(line, true);
          const list = document.createElement(ordered ? "ol" : "ul");
          while (i < lines.length && isListLine(lines[i], ordered)) {
            const li = document.createElement("li");
            appendInline(li, lines[i++].replace(ordered ? /^\s*\d+[.)]\s+/ : /^\s*[-*+]\s+/, ""));
            list.append(li);
          }
          target.append(list);
          continue;
        }
        const paragraph = [];
        while (i < lines.length && lines[i].trim() && !isBlockStart(lines, i)) paragraph.push(lines[i++]);
        const p = document.createElement("p");
        appendInline(p, paragraph.join(" "));
        target.append(p);
      }
      if (!target.childNodes.length && source) target.textContent = source;
    }
    function renderWorkerRun(run) {
      workerRunPanel.classList.toggle("hidden", !run);
      workerRunTasks.replaceChildren();
      if (!run) return;
      workerRunTitle.textContent = `${(run.tasks || []).length} worker${(run.tasks || []).length === 1 ? "" : "s"}`;
      workerRunStatus.textContent = run.status || "unknown";
      for (const task of run.tasks || []) {
        const row = document.createElement("div");
        row.className = "worker-run-mobile-task";
        const title = document.createElement("span");
        title.textContent = task.title || "Worker";
        const status = document.createElement("span");
        status.textContent = task.status || "queued";
        row.append(title, status);
        workerRunTasks.append(row);
      }
      const proposed = run.status === "proposed";
      const running = run.status === "running";
      workerRunStart.classList.toggle("hidden", !proposed);
      workerRunSolo.classList.toggle("hidden", !proposed);
      workerRunStop.classList.toggle("hidden", !running);
      workerRunStart.dataset.runId = run.id || "";
      workerRunSolo.dataset.runId = run.id || "";
      workerRunStop.dataset.runId = run.id || "";
    }
    function renderThread(thread) {
      state.thread = thread;
      applyThemeSnapshot(thread?.theme);
      threadMessages.replaceChildren();
      renderModelSelect(thread);
      renderWorkerRun(thread?.worker_run);
      if (!thread) {
        threadTitle.textContent = "No active thread";
        threadMeta.textContent = "Open Milim on desktop to publish the current chat.";
        threadState.textContent = "Offline";
        stopButton.disabled = true;
        stopButton.classList.add("hidden");
        sendButton.classList.remove("hidden");
        renderThreadList(null);
        const empty = document.createElement("div");
        empty.className = "empty-thread";
        empty.textContent = "Waiting for the active desktop thread.";
        threadMessages.append(empty);
        return;
      }
      threadTitle.textContent = thread.title || "Current thread";
      threadMeta.textContent = [thread.model, new Date(thread.updated_at * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })].filter(Boolean).join(" / ");
      threadState.textContent = thread.busy ? "Running" : "Idle";
      stopButton.disabled = !thread.busy;
      stopButton.classList.toggle("hidden", !thread.busy);
      sendButton.classList.toggle("hidden", thread.busy);
      renderThreadList(thread);
      const messages = Array.isArray(thread.messages) ? thread.messages : [];
      if (!messages.length) {
        const empty = document.createElement("div");
        empty.className = "empty-thread";
        empty.textContent = "This thread is empty. Send a message from your phone.";
        threadMessages.append(empty);
        return;
      }
      const latestAssistantIndex = messages.reduce((latest, message, index) => message.role === "assistant" ? index : latest, -1);
      messages.forEach((message, index) => {
        const role = message.role === "user" ? "user" : "assistant";
        const wrap = document.createElement("div");
        wrap.className = `msg-wrap ${role}`;
        const item = document.createElement("article");
        item.className = `msg ${role}`;
        renderMarkdown(item, message.content || "");
        const actions = document.createElement("div");
        actions.className = "msg-actions";
        const copy = document.createElement("button");
        copy.type = "button";
        setButtonIcon(copy, "copy", "Copy");
        copy.addEventListener("click", () => navigator.clipboard?.writeText(message.content || "").catch(() => {}));
        const del = document.createElement("button");
        del.type = "button";
        setButtonIcon(del, "trash", "Delete");
        del.addEventListener("click", () => relayAction("delete_message", String(index)).catch(showRelayError));
        actions.append(copy, del);
        if (index === latestAssistantIndex) {
          const regen = document.createElement("button");
          regen.type = "button";
          setButtonIcon(regen, "refresh", "Regenerate");
          regen.addEventListener("click", () => relayAction("regenerate").catch(showRelayError));
          actions.append(regen);
        }
        wrap.append(item, actions);
        threadMessages.append(wrap);
      });
      threadMessages.scrollTop = threadMessages.scrollHeight;
    }
    async function refreshThread() {
      if (!store.key || relayPanel.classList.contains("hidden")) return;
      try {
        const data = await api("/mobile/thread");
        state.lastUpdate = Date.now();
        renderThread(data.thread);
      } catch (error) {
        renderThread(null);
        showRelayError(error);
      }
    }
    function connectThreadStream() {
      if (state.source) state.source.close();
      state.streamLive = false;
      if (!store.key || !("EventSource" in window)) return;
      const source = new EventSource(`/mobile/thread/events?key=${encodeURIComponent(store.key)}`);
      state.source = source;
      source.onopen = () => {
        state.streamLive = true;
        setStatus("Live", true);
      };
      source.onmessage = (event) => {
        try {
          const data = JSON.parse(event.data);
          state.lastUpdate = Date.now();
          renderThread(data.thread);
          setStatus("Live", true);
        } catch {}
      };
      source.onerror = () => {
        state.streamLive = false;
        setStatus("Reconnecting", false, true);
      };
    }
    function checkHealth() {
      if (relayPanel.classList.contains("hidden") || !state.lastUpdate) return;
      if (Date.now() - state.lastUpdate > 10000) setStatus("Stale", false, true);
    }
    function openThreadDrawer() {
      threadDrawer.classList.add("open");
      threadDrawer.setAttribute("aria-hidden", "false");
      threadDrawerBackdrop.classList.remove("hidden");
    }
    function closeThreadDrawer() {
      threadDrawer.classList.remove("open");
      threadDrawer.setAttribute("aria-hidden", "true");
      threadDrawerBackdrop.classList.add("hidden");
    }
    async function switchThread(sessionId) {
      if (!sessionId) return;
      relayNotice.textContent = "Switching thread...";
      relayNotice.className = "notice";
      try {
        await relayAction("switch_thread", sessionId);
        closeThreadDrawer();
        window.setTimeout(refreshThread, 800);
      } catch (error) {
        showRelayError(error);
      }
    }
    async function checkDevice() {
      if (!store.key) return false;
      try {
        const data = await api("/mobile/device/status");
        state.device = data.device;
        renderDevice();
        setStatus("Connected", true);
        show("relay");
        connectThreadStream();
        await refreshThread();
        return true;
      } catch (error) {
        setStatus("Pairing needed", false);
        return false;
      }
    }
    function applyPairingLink(raw) {
      const value = String(raw || "").trim().replaceAll("&amp;", "&");
      if (!value) throw new Error("Paste a Milim pairing link first.");
      let url;
      try {
        url = new URL(value, location.href);
      } catch {
        throw new Error("That does not look like a Milim pairing link.");
      }
      const pairId = url.searchParams.get("pair_id") || "";
      const secret = url.searchParams.get("secret") || "";
      if (!pairId || !secret) throw new Error("That QR code did not include a Milim pairing token.");
      if (url.origin !== location.origin) {
        pairNotice.textContent = "Opening that pairing link...";
        pairNotice.className = "notice";
        location.href = url.href;
        return false;
      }
      activePairId = pairId;
      activePairSecret = secret;
      pairLinkInput.value = url.href;
      return true;
    }
    async function usePairingLink(raw) {
      try {
        if (!applyPairingLink(raw)) return;
        await pair();
      } catch (error) {
        pairNotice.textContent = String(error.message || error);
        pairNotice.className = "notice error";
      }
    }
    function stopPairScanner() {
      if (scannerFrame) cancelAnimationFrame(scannerFrame);
      scannerFrame = 0;
      if (scannerStream) {
        scannerStream.getTracks().forEach((track) => track.stop());
        scannerStream = null;
      }
      pairScanner.pause();
      pairScanner.srcObject = null;
      pairScanner.classList.add("hidden");
      stopScanButton.classList.add("hidden");
      scanPairButton.disabled = false;
    }
    async function startPairScanner() {
      pairNotice.textContent = "Opening camera...";
      pairNotice.className = "notice";
      if (!window.isSecureContext) {
        pairNotice.textContent = "Camera scanning needs HTTPS or localhost. Use the HTTPS Tailscale URL, or paste the pairing link here.";
        pairNotice.className = "notice error";
        return;
      }
      if (!navigator.mediaDevices?.getUserMedia) {
        pairNotice.textContent = "This browser cannot open the camera from the relay app. Paste the pairing link instead.";
        pairNotice.className = "notice error";
        return;
      }
      if (!("BarcodeDetector" in window)) {
        pairNotice.textContent = "This browser cannot scan QR codes here. Paste the pairing link instead.";
        pairNotice.className = "notice error";
        return;
      }
      const BarcodeDetectorCtor = window.BarcodeDetector;
      let detector;
      try {
        detector = new BarcodeDetectorCtor({ formats: ["qr_code"] });
      } catch {
        detector = new BarcodeDetectorCtor();
      }
      stopPairScanner();
      scanPairButton.disabled = true;
      try {
        scannerStream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: { ideal: "environment" } },
          audio: false,
        });
        pairScanner.srcObject = scannerStream;
        pairScanner.classList.remove("hidden");
        stopScanButton.classList.remove("hidden");
        await pairScanner.play();
        pairNotice.textContent = "Point your camera at the Milim desktop QR.";
        const scan = async () => {
          if (!scannerStream) return;
          try {
            const codes = await detector.detect(pairScanner);
            const value = codes[0]?.rawValue || "";
            if (value) {
              stopPairScanner();
              await usePairingLink(value);
              return;
            }
          } catch {}
          scannerFrame = requestAnimationFrame(scan);
        };
        scannerFrame = requestAnimationFrame(scan);
      } catch {
        stopPairScanner();
        pairNotice.textContent = "Could not open the camera. Paste the pairing link instead.";
        pairNotice.className = "notice error";
      }
    }
    async function pair() {
      pairNotice.textContent = "Pairing...";
      pairNotice.className = "notice";
      if (!activePairId || !activePairSecret) {
        pairNotice.textContent = "Scan the desktop QR or paste a pairing link first.";
        pairNotice.className = "notice error";
        return;
      }
      try {
        const data = await api("/mobile/pair", {
          method: "POST",
          body: JSON.stringify({
            pair_id: activePairId,
            secret: activePairSecret,
            device_name: deviceName.value.trim() || "Phone",
          }),
        });
        store.key = data.device_key;
        store.id = data.device_id;
        store.name = data.device_name;
        pairNotice.textContent = "Paired.";
        if (location.search) history.replaceState(null, "", "/mobile");
        await checkDevice();
      } catch (error) {
        const message = String(error.message || error);
        pairNotice.textContent = message.includes("invalid pairing token") || message.includes("expired")
          ? "This pairing link expired. Start a fresh pairing from Milim desktop Settings, then scan the new QR or paste its link here."
          : message;
        pairNotice.className = "notice error";
      }
    }
    function attachmentId() {
      return crypto.randomUUID ? crypto.randomUUID() : `att-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    }
    function fileDataUrl(file) {
      return new Promise((resolve, reject) => {
        if (!file.type.startsWith("image/")) return resolve(undefined);
        if (!["image/png", "image/jpeg", "image/webp", "image/gif"].includes(file.type)) {
          return reject(new Error(`${file.name || "Image"} must be PNG, JPEG, WebP, or GIF.`));
        }
        if (!file.size || file.size > MAX_IMAGE_PREVIEW_BYTES) {
          return reject(new Error(`${file.name || "Image"} must be no larger than 2 MB.`));
        }
        const reader = new FileReader();
        reader.onload = () => typeof reader.result === "string"
          ? resolve(reader.result)
          : reject(new Error(`${file.name || "Image"} could not be read as image data.`));
        reader.onerror = () => reject(new Error(`${file.name || "Image"} could not be read as image data.`));
        reader.readAsDataURL(file);
      });
    }
    async function fileAttachment(file) {
      const mime = file.type || "application/octet-stream";
      const textLike = mime.startsWith("text/") || ["application/json", "application/xml", "application/javascript"].includes(mime);
      const [content, dataUrl] = await Promise.all([
        textLike ? file.slice(0, MAX_TEXT_ATTACHMENT_BYTES).text() : undefined,
        fileDataUrl(file),
      ]);
      return {
        id: attachmentId(),
        name: file.name || "attachment",
        mime,
        size: file.size,
        content,
        dataUrl,
        truncated: textLike ? file.size > MAX_TEXT_ATTACHMENT_BYTES : false,
      };
    }
    function renderAttachments() {
      attachmentTray.replaceChildren();
      for (const attachment of state.attachments) {
        const pill = document.createElement("button");
        pill.type = "button";
        pill.className = "attachment-pill";
        pill.title = "Remove attachment";
        const label = document.createElement("span");
        label.textContent = attachment.name;
        pill.append(label, document.createTextNode("x"));
        pill.addEventListener("click", () => {
          state.attachments = state.attachments.filter((item) => item.id !== attachment.id);
          renderAttachments();
        });
        attachmentTray.append(pill);
      }
    }
    async function addFiles(files) {
      const incoming = Array.from(files || []).slice(0, MAX_ATTACHMENTS - state.attachments.length);
      if (!incoming.length) return;
      try {
        state.attachments = [...state.attachments, ...(await Promise.all(incoming.map(fileAttachment)))].slice(0, MAX_ATTACHMENTS);
        renderAttachments();
      } catch (error) {
        relayNotice.textContent = String(error.message || error);
        relayNotice.className = "notice error";
      }
    }
    async function relay(action) {
      const text = relayText.value.trim();
      const attachments = state.attachments;
      if (!text && !attachments.length && action !== "new_thread") return;
      relayNotice.textContent = action === "new_thread" ? "Creating thread..." : "Sending...";
      relayNotice.className = "notice";
      try {
        await relayAction(action, text, attachments);
        relayText.value = "";
        state.attachments = [];
        renderAttachments();
        relayNotice.textContent = action === "new_thread" ? "New thread requested." : "Sent to desktop.";
        window.setTimeout(refreshThread, 800);
      } catch (error) {
        showRelayError(error);
      }
    }
    document.getElementById("pairButton").addEventListener("click", pair);
    scanPairButton.addEventListener("click", startPairScanner);
    stopScanButton.addEventListener("click", stopPairScanner);
    document.getElementById("pairLinkButton").addEventListener("click", () => usePairingLink(pairLinkInput.value));
    pairLinkInput.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        usePairingLink(pairLinkInput.value);
      }
    });
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) stopPairScanner();
    });
    document.getElementById("settingsToggle").addEventListener("click", () => settingsPanel.classList.toggle("hidden"));
    document.getElementById("newThreadButton").addEventListener("click", () => {
      closeThreadDrawer();
      relay("new_thread");
    });
    document.getElementById("stopButton").addEventListener("click", () => relayAction("stop").catch(showRelayError));
    workerRunStart.addEventListener("click", () => relayAction("worker_run_start", workerRunStart.dataset.runId || "").catch(showRelayError));
    workerRunSolo.addEventListener("click", () => relayAction("worker_run_continue_solo", workerRunSolo.dataset.runId || "").catch(showRelayError));
    workerRunStop.addEventListener("click", () => relayAction("worker_run_stop", workerRunStop.dataset.runId || "").catch(showRelayError));
    document.getElementById("renameButton").addEventListener("click", () => {
      const next = prompt("Rename thread", state.thread?.title || "");
      if (next) {
        closeThreadDrawer();
        relayAction("rename_thread", next).catch(showRelayError);
      }
    });
    document.getElementById("archiveButton").addEventListener("click", () => {
      if (state.thread && confirm("Archive this thread?")) relayAction("archive_thread", state.thread.session_id).catch(showRelayError);
    });
    document.getElementById("deleteThreadButton").addEventListener("click", () => {
      if (state.thread && confirm("Delete this thread?")) relayAction("delete_thread", state.thread.session_id).catch(showRelayError);
    });
    document.getElementById("forgetButton").addEventListener("click", () => {
      if (state.source) state.source.close();
      store.key = "";
      store.id = "";
      store.name = "";
      state.device = null;
      setStatus("Pairing needed", false);
      show("pair");
    });
    modelSelect.addEventListener("change", () => {
      if (modelSelect.value) relayAction("set_model", modelSelect.value).catch(showRelayError);
    });
    fileInput.addEventListener("change", () => addFiles(fileInput.files).finally(() => { fileInput.value = ""; }));
    document.getElementById("attachButton").addEventListener("click", () => fileInput.click());
    relayForm.addEventListener("submit", (event) => {
      event.preventDefault();
      relay("send");
    });
    threadsToggle.addEventListener("click", () => openThreadDrawer());
    threadDrawerClose.addEventListener("click", () => closeThreadDrawer());
    threadDrawerBackdrop.addEventListener("click", () => closeThreadDrawer());
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeThreadDrawer();
    });
    threadSearch.addEventListener("input", () => {
      state.query = threadSearch.value;
      renderThreadList(state.thread);
    });
    window.setInterval(() => {
      checkHealth();
      if (!state.streamLive) refreshThread();
    }, 5000);
    (async () => {
      if (await checkDevice()) return;
      if (activePairId && activePairSecret) {
        setStatus("Pairing", false);
        show("pair");
        await pair();
      } else {
        setStatus("Pair this phone", false);
        show("pair");
        pairNotice.textContent = "Start pairing from Milim desktop Settings, then scan the QR here. If camera scanning is unavailable, paste the pairing link instead.";
      }
    })();
  </script>
</body>
</html>"##;

const MOBILE_COMPANION_MANIFEST: &str = r##"{
  "id": "/mobile",
  "name": "Milim Relay",
  "short_name": "Milim",
  "description": "Mobile companion for the active Milim desktop thread.",
  "start_url": "/mobile",
  "scope": "/mobile",
  "display": "standalone",
  "background_color": "#0d0d0f",
  "theme_color": "#0d0d0f",
  "icons": [
    {
      "src": "/mobile/icon.svg",
      "sizes": "any",
      "type": "image/svg+xml",
      "purpose": "any maskable"
    },
    {
      "src": "/mobile/icon.png",
      "sizes": "512x512",
      "type": "image/png",
      "purpose": "any maskable"
    }
  ]
}"##;

const MOBILE_COMPANION_SERVICE_WORKER: &str = r##"self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));
"##;

const MOBILE_COMPANION_ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
  <rect width="512" height="512" rx="104" fill="#0d0d0f"/>
  <rect x="40" y="40" width="432" height="432" rx="72" fill="#161618" stroke="#323237" stroke-width="8"/>
  <path fill="#ededf0" d="M120 326V178h44l4 18c12-15 28-23 48-23 24 0 41 10 52 29 13-19 32-29 56-29 42 0 68 28 68 76v77h-48v-73c0-24-11-37-31-37-21 0-34 15-34 40v70h-48v-74c0-23-11-36-31-36-21 0-34 15-34 40v70h-46Z"/>
</svg>"##;

const MOBILE_COMPANION_WORDMARK: &str =
    include_str!("../../../apps/desktop/public/milim-wordmark.svg");
const MOBILE_COMPANION_ICON_PNG: &[u8] =
    include_bytes!("../../../apps/desktop/src-tauri/icons/icon.png");

/// `GET /v1/models` and `/models`
pub(crate) async fn openai_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let data = st.service.list_models().await.map_err(ApiError)?;
    Ok(Json(ModelsResponse {
        object: "list".to_string(),
        data,
    })
    .into_response())
}

/// `GET /api/tags` (Ollama)
pub(crate) async fn ollama_tags(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let tags = st
        .service
        .list_models()
        .await
        .map_err(ApiError)?
        .into_iter()
        .map(|m: Model| OllamaModelTag {
            name: m.id.clone(),
            model: m.id,
            modified_at: rfc3339_now(),
            size: 0,
            digest: String::new(),
            details: OllamaModelDetails {
                format: "gguf".to_string(),
                ..Default::default()
            },
        })
        .collect();
    Ok(Json(OllamaTagsResponse { models: tags }).into_response())
}

/// `POST /v1/chat/completions` and `/chat/completions`
pub(crate) async fn openai_chat(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let include_usage = req
        .stream_options
        .as_ref()
        .and_then(|o| o.include_usage)
        .unwrap_or(false);

    let mut creq = openai_to_completion(req);
    add_workspace_instructions(&mut creq.messages, &st);
    let ctx = ChunkCtx {
        id: gen_id("chatcmpl"),
        created: now_unix(),
        model: model.clone(),
    };

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let stream = openai_sse(inner, ctx, include_usage);
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let out = st.service.complete(creq).await.map_err(ApiError)?;
        let resp = ChatCompletionResponse {
            id: ctx.id,
            object: "chat.completion".to_string(),
            created: ctx.created,
            model,
            choices: vec![Choice {
                index: 0,
                message: out.message,
                finish_reason: Some(out.finish_reason),
            }],
            usage: out.usage,
            system_fingerprint: None,
        };
        Ok(Json(resp).into_response())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PromptInput {
    Text(String),
    Many(Vec<String>),
}

impl PromptInput {
    fn text(self) -> String {
        match self {
            Self::Text(text) => text,
            Self::Many(items) => items.join("\n"),
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LegacyCompletionRequest {
    model: String,
    prompt: PromptInput,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    stop: Option<StringOrArray>,
    #[serde(default)]
    frequency_penalty: Option<f32>,
    #[serde(default)]
    presence_penalty: Option<f32>,
    #[serde(default)]
    seed: Option<i64>,
    #[serde(default)]
    echo: Option<bool>,
}

impl LegacyCompletionRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// `POST /v1/completions` and `/completions`
pub(crate) async fn openai_completions(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<LegacyCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let prompt = req.prompt.clone().text();
    let echo = req.echo.unwrap_or(false);
    let creq = legacy_completion_to_completion(req);
    let id = gen_id("cmpl");
    let created = now_unix();

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        return Ok(Sse::new(completion_sse(inner, id, created, model))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    let text = if echo {
        format!("{prompt}{}", out.message.text_content())
    } else {
        out.message.text_content()
    };
    Ok(Json(json!({
        "id": id,
        "object": "text_completion",
        "created": created,
        "model": model,
        "choices": [{
            "text": text,
            "index": 0,
            "logprobs": null,
            "finish_reason": out.finish_reason,
        }],
        "usage": out.usage,
    }))
    .into_response())
}

fn legacy_completion_to_completion(req: LegacyCompletionRequest) -> CompletionRequest {
    let prompt = req.prompt.text();
    CompletionRequest {
        model: req.model,
        messages: vec![ChatMessage::text("user", prompt.clone())],
        tools: Vec::new(),
        tool_choice: None,
        response_format: None,
        prompt: Some(prompt),
        suffix: req.suffix,
        sampling: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_tokens,
            stop: req.stop.map(|s| s.into_vec()).unwrap_or_default(),
            seed: req.seed,
            frequency_penalty: req.frequency_penalty,
            presence_penalty: req.presence_penalty,
        },
        reasoning_effort: None,
    }
}

fn completion_sse(
    mut inner: EventStream,
    id: String,
    created: u64,
    model: String,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    if let Some(text) = delta.content {
                        yield Ok(Event::default().data(json!({
                            "id": id,
                            "object": "text_completion",
                            "created": created,
                            "model": model,
                            "choices": [{
                                "text": text,
                                "index": 0,
                                "logprobs": null,
                                "finish_reason": null,
                            }],
                        }).to_string()));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    yield Ok(Event::default().data(json!({
                        "id": id,
                        "object": "text_completion",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "text": "",
                            "index": 0,
                            "logprobs": null,
                            "finish_reason": finish_reason,
                        }],
                        "usage": usage,
                    }).to_string()));
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                Err(e) => {
                    yield Ok(Event::default().event("error").data(json!({
                        "error": { "message": e.to_string(), "type": e.code() }
                    }).to_string()));
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            }
        }
        yield Ok(Event::default().data("[DONE]"));
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ResponsesInput {
    Text(String),
    Items(Vec<Value>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResponsesRequest {
    model: String,
    input: ResponsesInput,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
    #[serde(default)]
    tools: Vec<Value>,
    #[serde(default)]
    tool_choice: Option<Value>,
    #[serde(default)]
    reasoning: Option<Value>,
    #[serde(default)]
    text: Option<Value>,
    #[serde(default)]
    previous_response_id: Option<String>,
}

impl ResponsesRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(false)
    }
}

/// `POST /v1/responses`
pub(crate) async fn openai_responses(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ResponsesRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let response_id = gen_id("resp");
    let created = now_unix();
    let text = response_text_value(req.text.clone());
    let tools = req.tools.clone();
    let tool_choice = req.tool_choice.clone().unwrap_or_else(|| json!("auto"));
    let reasoning = req
        .reasoning
        .clone()
        .unwrap_or_else(|| json!({ "effort": null, "summary": null }));
    let previous_response_id = req.previous_response_id.clone();
    let response_shape = ResponseShape {
        id: response_id,
        created,
        model,
        text,
        tools,
        tool_choice,
        reasoning,
        previous_response_id,
    };
    let creq = responses_to_completion(req).map_err(ApiError)?;

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        return Ok(Sse::new(responses_sse(inner, response_shape))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    Ok(Json(response_json(
        &response_shape,
        response_output_items(&out.message),
        Some(out.usage),
    ))
    .into_response())
}

fn responses_to_completion(req: ResponsesRequest) -> Result<CompletionRequest, Error> {
    let text = response_text_value(req.text);
    let mut messages = response_input_messages(req.input)?;
    if let Some(instructions) = req.instructions.filter(|s| !s.is_empty()) {
        messages.insert(0, ChatMessage::text("system", instructions));
    }
    Ok(CompletionRequest {
        model: req.model,
        messages,
        tools: response_tools(req.tools)?,
        tool_choice: req.tool_choice,
        response_format: response_format_from_responses_text(&text),
        prompt: None,
        suffix: None,
        sampling: SamplingParams {
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_output_tokens,
            ..Default::default()
        },
        reasoning_effort: response_reasoning_effort(req.reasoning.as_ref()),
    })
}

fn response_input_messages(input: ResponsesInput) -> Result<Vec<ChatMessage>, Error> {
    match input {
        ResponsesInput::Text(text) => Ok(vec![ChatMessage::text("user", text)]),
        ResponsesInput::Items(items) => items.into_iter().map(response_input_item).collect(),
    }
}

fn response_input_item(item: Value) -> Result<ChatMessage, Error> {
    if let Some(text) = item.as_str() {
        return Ok(ChatMessage::text("user", text));
    }
    let kind = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message");
    match kind {
        "function_call_output" => Ok(ChatMessage {
            role: "tool".to_string(),
            content: Some(Content::Text(
                item.get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )),
            name: None,
            tool_calls: None,
            tool_call_id: item
                .get("call_id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            reasoning_content: None,
        }),
        "function_call" => Ok(ChatMessage {
            role: "assistant".to_string(),
            content: None,
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                kind: "function".to_string(),
                function: FunctionCall {
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                },
            }]),
            tool_call_id: None,
            reasoning_content: None,
        }),
        _ => {
            let role = item
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            let content = item
                .get("content")
                .map(response_content)
                .transpose()?
                .unwrap_or_else(|| Content::Text(String::new()));
            Ok(ChatMessage {
                role,
                content: Some(content),
                name: None,
                tool_calls: None,
                tool_call_id: None,
                reasoning_content: None,
            })
        }
    }
}

fn response_content(value: &Value) -> Result<Content, Error> {
    if let Some(text) = value.as_str() {
        return Ok(Content::Text(text.to_string()));
    }
    let parts = value.as_array().ok_or_else(|| {
        Error::InvalidRequest("Responses message content must be a string or array".to_string())
    })?;
    let mut out = Vec::new();
    for part in parts {
        match part.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("text") | Some("output_text") => {
                out.push(ContentPart::Text {
                    text: part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            Some("input_image") | Some("image_url") => {
                if let Some(url) = part
                    .get("image_url")
                    .and_then(Value::as_str)
                    .or_else(|| part.pointer("/image_url/url").and_then(Value::as_str))
                {
                    out.push(ContentPart::ImageUrl {
                        image_url: ImageUrl {
                            url: url.to_string(),
                            detail: part
                                .get("detail")
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    Ok(Content::Parts(out))
}

fn response_tools(tools: Vec<Value>) -> Result<Vec<OpenAiTool>, Error> {
    let mut out = Vec::new();
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        if tool.get("function").is_some() {
            out.push(serde_json::from_value(tool).map_err(|e| {
                Error::InvalidRequest(format!("invalid Responses function tool: {e}"))
            })?);
            continue;
        }
        let name = tool.get("name").and_then(Value::as_str).ok_or_else(|| {
            Error::InvalidRequest("Responses function tool is missing name".to_string())
        })?;
        out.push(OpenAiTool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: name.to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                parameters: tool.get("parameters").cloned(),
            },
        });
    }
    Ok(out)
}

fn response_reasoning_effort(reasoning: Option<&Value>) -> Option<ReasoningEffort> {
    let effort = reasoning?.get("effort")?.as_str()?;
    serde_json::from_value(Value::String(effort.to_string())).ok()
}

fn response_text_value(text: Option<Value>) -> Value {
    text.unwrap_or_else(|| json!({ "format": { "type": "text" } }))
}

fn response_format_from_responses_text(text: &Value) -> Option<Value> {
    let format = text.get("format")?;
    match format.get("type").and_then(Value::as_str) {
        Some("text") | None => None,
        Some("json_schema") if format.get("json_schema").is_none() => Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": format
                    .get("name")
                    .cloned()
                    .unwrap_or_else(|| Value::String("response".to_string())),
                "strict": format.get("strict").cloned().unwrap_or(Value::Bool(false)),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
            }
        })),
        _ => Some(format.clone()),
    }
}

fn response_output_items(message: &ChatMessage) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(calls) = &message.tool_calls {
        for call in calls {
            out.push(json!({
                "type": "function_call",
                "id": call.id.clone().unwrap_or_else(|| gen_id("fc")),
                "call_id": call.id.clone().unwrap_or_else(|| gen_id("call")),
                "name": call.function.name.clone(),
                "arguments": call.function.arguments.clone(),
                "status": "completed",
            }));
        }
    }
    let text = message.text_content();
    if !text.is_empty() || out.is_empty() {
        out.push(response_message_item(&text));
    }
    out
}

fn response_message_item(text: &str) -> Value {
    json!({
        "type": "message",
        "id": gen_id("msg"),
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": text,
            "annotations": [],
        }],
    })
}

#[derive(Debug, Clone)]
struct ResponseShape {
    id: String,
    created: u64,
    model: String,
    text: Value,
    tools: Vec<Value>,
    tool_choice: Value,
    reasoning: Value,
    previous_response_id: Option<String>,
}

fn response_json(shape: &ResponseShape, output: Vec<Value>, usage: Option<Usage>) -> Value {
    let usage = usage.map(|u| {
        json!({
            "input_tokens": u.prompt_tokens,
            "output_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens,
        })
    });
    json!({
        "id": shape.id.clone(),
        "object": "response",
        "created_at": shape.created,
        "status": "completed",
        "completed_at": now_unix(),
        "error": null,
        "incomplete_details": null,
        "instructions": null,
        "max_output_tokens": null,
        "model": shape.model.clone(),
        "output": output,
        "parallel_tool_calls": true,
        "previous_response_id": shape.previous_response_id.clone(),
        "reasoning": shape.reasoning.clone(),
        "store": false,
        "temperature": null,
        "text": shape.text.clone(),
        "tool_choice": shape.tool_choice.clone(),
        "tools": shape.tools.clone(),
        "top_p": null,
        "truncation": "disabled",
        "usage": usage,
        "user": null,
        "metadata": {},
    })
}

fn responses_sse(
    mut inner: EventStream,
    shape: ResponseShape,
) -> impl futures::Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        yield named_response_event("response.created", json!({
            "type": "response.created",
            "response": response_json(
                &shape,
                Vec::new(),
                None,
            )
        }));

        let mut content = String::new();
        let mut reasoning_text = String::new();
        let mut text_started = false;
        let mut tool_calls = ToolCallAccumulator::default();

        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    for call in delta.tool_calls {
                        tool_calls.push(call);
                    }
                    if let Some(reasoning_delta) = delta.reasoning {
                        reasoning_text.push_str(&reasoning_delta);
                        yield named_response_event("response.reasoning_text.delta", json!({
                            "type": "response.reasoning_text.delta",
                            "delta": reasoning_delta,
                        }));
                    }
                    if let Some(chunk) = delta.content {
                        if !text_started {
                            text_started = true;
                            yield named_response_event("response.output_item.added", json!({
                                "type": "response.output_item.added",
                                "output_index": 0,
                                "item": { "type": "message", "id": gen_id("msg"), "status": "in_progress", "role": "assistant", "content": [] }
                            }));
                            yield named_response_event("response.content_part.added", json!({
                                "type": "response.content_part.added",
                                "output_index": 0,
                                "content_index": 0,
                                "part": { "type": "output_text", "text": "", "annotations": [] }
                            }));
                        }
                        content.push_str(&chunk);
                        yield named_response_event("response.output_text.delta", json!({
                            "type": "response.output_text.delta",
                            "output_index": 0,
                            "content_index": 0,
                            "delta": chunk,
                        }));
                    }
                }
                Ok(StreamEvent::Done { usage, .. }) => {
                    let mut message = ChatMessage::text("assistant", content.clone());
                    let calls = tool_calls.finish();
                    if !calls.is_empty() {
                        message.tool_calls = Some(calls);
                    }
                    if !reasoning_text.is_empty() {
                        message.reasoning_content = Some(reasoning_text);
                    }
                    yield named_response_event("response.output_text.done", json!({
                        "type": "response.output_text.done",
                        "output_index": 0,
                        "content_index": 0,
                        "text": content,
                    }));
                    let output = response_output_items(&message);
                    yield named_response_event("response.completed", json!({
                        "type": "response.completed",
                        "response": response_json(
                            &shape,
                            output,
                            Some(usage),
                        )
                    }));
                    return;
                }
                Err(e) => {
                    yield named_response_event("response.failed", json!({
                        "type": "response.failed",
                        "response": {
                            "id": shape.id.clone(),
                            "object": "response",
                            "created_at": shape.created,
                            "status": "failed",
                            "error": { "message": e.to_string(), "type": e.code() },
                            "model": shape.model.clone(),
                        }
                    }));
                    return;
                }
            }
        }
    }
}

fn named_response_event(name: &str, value: Value) -> Result<Event, Infallible> {
    Ok(Event::default().event(name).data(value.to_string()))
}

/// `POST /api/chat` (Ollama)
pub(crate) async fn ollama_chat(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OllamaChatRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let mut creq = ollama_to_completion(req);
    add_workspace_instructions(&mut creq.messages, &st);

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let body = Body::from_stream(ollama_ndjson(inner, model));
        Ok(Response::builder()
            .header(CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .expect("valid ndjson response"))
    } else {
        let out = st.service.complete(creq).await.map_err(ApiError)?;
        let resp = OllamaChatResponse {
            model,
            created_at: rfc3339_now(),
            message: OllamaMessage {
                role: "assistant".to_string(),
                content: out.message.text_content(),
                images: None,
                tool_calls: out.message.tool_calls,
                thinking: out.message.reasoning_content,
            },
            done: true,
            done_reason: Some(out.finish_reason),
            total_duration: Some(0),
            prompt_eval_count: Some(out.usage.prompt_tokens),
            eval_count: Some(out.usage.completion_tokens),
        };
        Ok(Json(resp).into_response())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaGenerateRequest {
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    suffix: Option<String>,
    #[serde(default)]
    system: Option<String>,
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    raw: Option<bool>,
    #[serde(default)]
    format: Option<Value>,
    #[serde(default)]
    keep_alive: Option<Value>,
    #[serde(default)]
    options: Option<Value>,
    #[serde(default)]
    think: Option<Value>,
}

impl OllamaGenerateRequest {
    fn wants_stream(&self) -> bool {
        self.stream.unwrap_or(true)
    }
}

/// `POST /api/generate` (Ollama)
pub(crate) async fn ollama_generate(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OllamaGenerateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    if req.prompt.is_empty() && req.keep_alive.is_some() {
        let keep_alive = req.keep_alive.clone();
        st.service
            .ollama_keep_alive(&model, keep_alive.as_ref().cloned())
            .await
            .map_err(ApiError)?;
        let done_reason = ollama_keep_alive_done_reason(keep_alive.as_ref());
        let value = ollama_keep_alive_response(&model, done_reason);
        if want_stream {
            let body = Body::from(ollama_generate_line(value));
            return Ok(Response::builder()
                .header(CONTENT_TYPE, "application/x-ndjson")
                .body(body)
                .expect("valid ndjson response"));
        }
        return Ok(Json(value).into_response());
    }
    let creq = ollama_generate_to_completion(req);

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let body = Body::from_stream(ollama_generate_ndjson(inner, model));
        return Ok(Response::builder()
            .header(CONTENT_TYPE, "application/x-ndjson")
            .body(body)
            .expect("valid ndjson response"));
    }

    let out = st.service.complete(creq).await.map_err(ApiError)?;
    Ok(Json(json!({
        "model": model,
        "created_at": rfc3339_now(),
        "response": out.message.text_content(),
        "thinking": out.message.reasoning_content,
        "done": true,
        "done_reason": out.finish_reason,
        "total_duration": 0,
        "load_duration": 0,
        "prompt_eval_count": out.usage.prompt_tokens,
        "prompt_eval_duration": 0,
        "eval_count": out.usage.completion_tokens,
        "eval_duration": 0,
    }))
    .into_response())
}

fn ollama_generate_to_completion(req: OllamaGenerateRequest) -> CompletionRequest {
    let opts = req.options.unwrap_or(Value::Null);
    let mut messages = Vec::new();
    if !req.raw.unwrap_or(false) {
        if let Some(system) = req.system.filter(|s| !s.is_empty()) {
            messages.push(ChatMessage::text("system", system));
        }
    }
    messages.push(ChatMessage::text("user", req.prompt.clone()));
    CompletionRequest {
        model: req.model,
        messages,
        tools: Vec::new(),
        tool_choice: None,
        response_format: req.format.map(ollama_format_to_response_format),
        prompt: Some(req.prompt),
        suffix: req.suffix,
        sampling: SamplingParams {
            temperature: opt_f32(&opts, "temperature"),
            top_p: opt_f32(&opts, "top_p"),
            max_tokens: opt_u32(&opts, "num_predict"),
            stop: opt_stops(&opts),
            seed: opt_i64(&opts, "seed"),
            frequency_penalty: opt_f32(&opts, "frequency_penalty"),
            presence_penalty: opt_f32(&opts, "presence_penalty"),
        },
        reasoning_effort: ollama_think_effort(req.think.as_ref()),
    }
}

fn ollama_keep_alive_done_reason(keep_alive: Option<&Value>) -> &'static str {
    match keep_alive {
        Some(Value::Number(n)) if n.as_i64() == Some(0) || n.as_u64() == Some(0) => "unload",
        Some(Value::String(s)) if s.trim() == "0" => "unload",
        _ => "load",
    }
}

fn ollama_keep_alive_response(model: &str, done_reason: &str) -> Value {
    json!({
        "model": model,
        "created_at": rfc3339_now(),
        "response": "",
        "done": true,
        "done_reason": done_reason,
        "total_duration": 0,
        "load_duration": 0,
        "prompt_eval_count": 0,
        "prompt_eval_duration": 0,
        "eval_count": 0,
        "eval_duration": 0,
    })
}

fn ollama_generate_ndjson(
    mut inner: EventStream,
    model: String,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        while let Some(ev) = inner.next().await {
            match ev {
                Ok(StreamEvent::Delta(delta)) => {
                    if let Some(content) = delta.content {
                        yield Ok(ollama_generate_line(json!({
                            "model": model,
                            "created_at": rfc3339_now(),
                            "response": content,
                            "done": false,
                        })));
                    }
                    if let Some(thinking) = delta.reasoning {
                        yield Ok(ollama_generate_line(json!({
                            "model": model,
                            "created_at": rfc3339_now(),
                            "response": "",
                            "thinking": thinking,
                            "done": false,
                        })));
                    }
                }
                Ok(StreamEvent::Done { finish_reason, usage }) => {
                    yield Ok(ollama_generate_line(json!({
                        "model": model,
                        "created_at": rfc3339_now(),
                        "response": "",
                        "done": true,
                        "done_reason": finish_reason,
                        "total_duration": 0,
                        "load_duration": 0,
                        "prompt_eval_count": usage.prompt_tokens,
                        "prompt_eval_duration": 0,
                        "eval_count": usage.completion_tokens,
                        "eval_duration": 0,
                    })));
                    return;
                }
                Err(e) => {
                    yield Ok(ollama_generate_line(json!({
                        "model": model,
                        "created_at": rfc3339_now(),
                        "response": "",
                        "done": true,
                        "done_reason": format!("error: {e}"),
                    })));
                    return;
                }
            }
        }
    }
}

fn ollama_generate_line(value: Value) -> Bytes {
    let mut line = value.to_string();
    line.push('\n');
    Bytes::from(line)
}

fn opt_f32(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32)
}

fn opt_u32(v: &Value, key: &str) -> Option<u32> {
    v.get(key).and_then(|x| x.as_u64()).map(|x| x as u32)
}

fn opt_i64(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(|x| x.as_i64())
}

fn opt_stops(v: &Value) -> Vec<String> {
    match v.get("stop") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// `POST /anthropic/v1/messages`
pub(crate) async fn anthropic_messages(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MessagesRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let creq = anthropic_to_completion(req).map_err(ApiError)?;
    let id = gen_id("msg");

    if want_stream {
        let inner = st.service.stream(creq).await.map_err(ApiError)?;
        let stream = anthropic_sse(inner, id, model);
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    } else {
        let out = st.service.complete(creq).await.map_err(ApiError)?;
        let resp = MessagesResponse {
            id,
            kind: "message".to_string(),
            role: "assistant".to_string(),
            content: anthropic_response_blocks(&out.message),
            model,
            stop_reason: Some(anthropic_stop_reason(&out.finish_reason)),
            stop_sequence: None,
            usage: anthropic::Usage {
                input_tokens: out.usage.prompt_tokens,
                output_tokens: out.usage.completion_tokens,
            },
        };
        Ok(Json(resp).into_response())
    }
}

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
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Codex requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&st, req.cwd.as_deref(), &req.prompt, "claude");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &req.prompt, "Codex").map_err(ApiError)?;
    account_runtime_images_for_remote(&st, &req.images, "Codex").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
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
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "OpenCode requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(
        &st,
        req.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()),
        &req.prompt,
        "agents",
    );
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &req.prompt, "OpenCode").map_err(ApiError)?;
    account_runtime_images_for_remote(&st, &req.images, "OpenCode").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
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
    req.cwd = req.cwd.take().filter(|cwd| !cwd.trim().is_empty());
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Pi requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&st, req.cwd.as_deref(), &req.prompt, "native");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &req.prompt, "Pi").map_err(ApiError)?;
    account_runtime_images_for_remote(&st, &req.images, "Pi").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
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

/// `GET /account-runtimes/updates` - installed versions for user-owned CLIs.
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
    if req.prompt.trim().is_empty() && req.images.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "Claude requires a prompt or at least one image".to_string(),
        )));
    }
    req.prompt = account_runtime_workspace_prompt(&st, req.cwd.as_deref(), &req.prompt, "agents");
    let (prompt, redactions) =
        account_runtime_prompt_for_remote(&st, &req.prompt, "Claude").map_err(ApiError)?;
    account_runtime_images_for_remote(&st, &req.images, "Claude").map_err(ApiError)?;
    req.prompt = prompt;
    let endpoint = account_runtime_tool_endpoint(
        &st,
        &headers,
        req.milim_context.as_ref(),
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

fn loopback_host(value: &str) -> Option<String> {
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
            let approved = pending.wait().await.approved;
            let payload = if approved {
                json!({ "behavior": "allow", "updatedInput": input })
            } else {
                json!({ "behavior": "deny", "message": "Tool call denied by user" })
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
                if !pending.wait().await.approved {
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
    st: &AppState,
    cwd: Option<&str>,
    prompt: &str,
    family: &str,
) -> String {
    let folder = cwd.map(PathBuf::from).or_else(|| workspace_snapshot(st));
    let context = crate::workspace_context::resolve(folder.as_deref());
    match crate::workspace_context::formatted(&context, Some(family)) {
        Some(instructions) => format!("{instructions}\n\nUser request:\n{prompt}"),
        None => prompt.to_string(),
    }
}

fn account_runtime_prompt_for_remote(
    st: &AppState,
    prompt: &str,
    runtime: &str,
) -> milim_core::Result<(String, BTreeMap<String, String>)> {
    match st.privacy.mode() {
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
    st: &AppState,
    images: &[crate::codex_bridge::AccountImage],
    runtime: &str,
) -> milim_core::Result<()> {
    if images.is_empty() {
        return Ok(());
    }
    if st.privacy.mode() != PrivacyMode::Off {
        return Err(Error::InvalidRequest(format!(
            "blocked by the privacy gate: {runtime} image pixels can only be sent in Privacy Off because images cannot be safely redacted"
        )));
    }
    crate::codex_bridge::validate_account_images(images)
}

// ----- MCP (tools) -----

fn mcp_registry(st: &AppState) -> ToolRegistry {
    let mut reg = static_registry_for_run(st);
    register_skill_tools(&mut reg, st, "auto", &[]);
    if let Some(hub) = &st.mcp {
        register_mcp_server_tools(&mut reg, hub.clone());
        for tool in hub.tools() {
            if let Err(error) = reg.try_register(tool) {
                tracing::warn!("skipping colliding MCP tool: {error}");
            }
        }
    }
    reg.without(HASHLINE_TOOL_NAMES)
}

const MAX_SKILL_READ_CHARS: usize = 40_000;

#[derive(Clone)]
struct MilimSkillScope {
    allowed_ids: Option<Arc<HashSet<String>>>,
}

struct MilimSkillSearchTool {
    store: Arc<milim_skills::SkillStore>,
    scope: MilimSkillScope,
}

struct MilimSkillReadTool {
    store: Arc<milim_skills::SkillStore>,
    scope: MilimSkillScope,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilimSkillSearchArgs {
    query: String,
    #[serde(default = "default_skill_select_limit")]
    limit: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MilimSkillReadArgs {
    id: String,
}

fn register_skill_tools(
    registry: &mut ToolRegistry,
    st: &AppState,
    skill_mode: &str,
    enabled_skills: &[String],
) {
    let Some(store) = st.skills.as_ref().cloned() else {
        return;
    };
    let allowed_ids = match skill_mode {
        "none" => return,
        "custom" if enabled_skills.is_empty() => return,
        "custom" => Some(Arc::new(enabled_skills.iter().cloned().collect())),
        _ => None,
    };
    let scope = MilimSkillScope { allowed_ids };
    registry.register(Arc::new(MilimSkillSearchTool {
        store: store.clone(),
        scope: scope.clone(),
    }));
    registry.register(Arc::new(MilimSkillReadTool { store, scope }));
}

fn skill_allowed(scope: &MilimSkillScope, skill: &milim_skills::SkillDef) -> bool {
    skill.enabled
        && scope
            .allowed_ids
            .as_ref()
            .is_none_or(|ids| ids.contains(&skill.id))
}

fn compact_skill_description(value: &str) -> String {
    const MAX_CHARS: usize = 220;
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= MAX_CHARS {
        return normalized;
    }
    format!(
        "{}...",
        normalized
            .chars()
            .take(MAX_CHARS)
            .collect::<String>()
            .trim_end()
    )
}

#[async_trait]
impl Tool for MilimSkillSearchTool {
    fn name(&self) -> &str {
        "milim_skill_search"
    }

    fn description(&self) -> &str {
        "Find relevant enabled Milim skills without loading their instruction bodies."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Current task or capability to match." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MilimSkillSearchArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid milim_skill_search arguments: {error}"))
        })?;
        let query = trim_required_tool_arg(args.query, "query")?;
        let allowed = self
            .scope
            .allowed_ids
            .as_ref()
            .map(|ids| ids.iter().cloned().collect::<Vec<_>>());
        let skills =
            self.store
                .select_filtered(&query, args.limit.clamp(1, 10), allowed.as_deref())?;
        Ok(json!({
            "skills": skills.into_iter().map(|skill| json!({
                "id": skill.id,
                "name": skill.name,
                "description": compact_skill_description(&skill.description),
            })).collect::<Vec<_>>()
        }))
    }
}

#[async_trait]
impl Tool for MilimSkillReadTool {
    fn name(&self) -> &str {
        "milim_skill_read"
    }

    fn description(&self) -> &str {
        "Load the complete instructions for one enabled Milim skill selected by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Skill id returned by milim_skill_search or the turn's skill candidates." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MilimSkillReadArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid milim_skill_read arguments: {error}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let skill = self
            .store
            .get(&id)?
            .filter(|skill| skill_allowed(&self.scope, skill))
            .ok_or_else(|| Error::ModelNotFound(format!("skill {id}")))?;
        if skill.instructions.chars().count() > MAX_SKILL_READ_CHARS {
            return Err(Error::InvalidRequest(format!(
                "skill {} exceeds the {} character read limit; move detailed material into referenced files",
                skill.name, MAX_SKILL_READ_CHARS
            )));
        }
        Ok(json!({
            "id": skill.id,
            "name": skill.name,
            "description": skill.description,
            "instructions": skill.instructions,
        }))
    }
}

fn mcp_catalog_registry(st: &AppState) -> ToolRegistry {
    let mut registry = mcp_registry(st);
    if let Some(store) = st.schedules.as_ref() {
        register_schedule_tools(&mut registry, store.clone(), workspace_snapshot(st));
    }
    if let Some(store) = st.memory.as_ref() {
        registry.register(Arc::new(MemoryRegisterTool {
            store: store.clone(),
            context: AgentMemoryContext::default(),
        }));
    }
    if let Some(supervisor) = st.threads.as_ref() {
        register_child_thread_tools(
            &mut registry,
            st.clone(),
            supervisor.clone(),
            AgentMemoryContext::default(),
            ToolRegistry::new(),
            false,
        );
    }
    registry
}

/// `GET /mcp/tools` — list available tools.
#[derive(Default, Deserialize)]
pub(crate) struct McpToolsQuery {
    #[serde(default)]
    callable: bool,
}

pub(crate) async fn mcp_tools(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<McpToolsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let tools = if query.callable {
        mcp_registry(&st).read_only().list()
    } else {
        mcp_catalog_registry(&st).list()
    };
    Ok(Json(json!({ "tools": tools })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct McpCallRequest {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// `POST /mcp/call` — invoke a tool by name.
pub(crate) async fn mcp_call(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpCallRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let registry = mcp_registry(&st).read_only();
    if registry.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "no tools registered".to_string(),
        )));
    }
    let result = registry
        .call(&req.name, req.arguments)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({ "result": result })).into_response())
}

const MCP_APP_HTML_MIME: &str = "text/html;profile=mcp-app";
const MCP_APP_HTML_LIMIT: usize = 5 * 1024 * 1024;
const MCP_APP_RESULT_LIMIT: usize = 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct McpAppResourceReadRequest {
    server_id: String,
    uri: String,
    #[serde(default)]
    render: bool,
}

#[derive(Deserialize)]
pub(crate) struct McpAppToolCallRequest {
    server_id: String,
    name: String,
    #[serde(default)]
    arguments: Value,
    #[serde(default)]
    approval: Option<String>,
    #[serde(default)]
    approval_granted: bool,
}

/// `POST /mcp/apps/resources/read` — fetch one server-advertised MCP App document.
pub(crate) async fn mcp_app_resource_read(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpAppResourceReadRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP is not configured".to_string())))?;
    let result = hub
        .read_app_resource(&req.server_id, &req.uri)
        .await
        .map_err(ApiError)?;
    let document = validate_mcp_app_resource(&result, &req.uri).map_err(ApiError)?;
    let view_path = req.render.then(|| {
        let id = st.mcp_app_views.insert(document);
        format!("/mcp/apps/views/{id}")
    });
    let result = (!req.render).then_some(result);
    Ok(Json(json!({ "result": result, "view_path": view_path })).into_response())
}

/// Ephemeral capability URL for one validated App document.
pub(crate) async fn mcp_app_view(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let document = st.mcp_app_views.take(&id).ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "MCP App view expired or does not exist".to_string(),
        ))
    })?;
    let mut response = Html(document.html).into_response();
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(&document.csp)
            .map_err(|_| ApiError(Error::Other("invalid generated MCP App CSP".to_string())))?,
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `POST /mcp/apps/tools/call` — invoke an app-visible tool on its fixed server.
pub(crate) async fn mcp_app_tool_call(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpAppToolCallRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP is not configured".to_string())))?;
    let tool = hub.app_tool(&req.server_id, &req.name).ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "tool is not app-visible on this MCP server".to_string(),
        ))
    })?;
    enforce_mcp_app_approval(req.approval.as_deref(), req.approval_granted, tool.effect)
        .map_err(ApiError)?;
    let result = hub
        .call_app_tool(&req.server_id, &req.name, req.arguments)
        .await
        .map_err(ApiError)?;
    let size = serde_json::to_vec(&result)
        .map_err(|error| {
            ApiError(Error::Other(format!(
                "MCP App result encoding failed: {error}"
            )))
        })?
        .len();
    if size > MCP_APP_RESULT_LIMIT {
        return Err(ApiError(Error::InvalidRequest(
            "MCP App tool result exceeds the 1 MiB limit".to_string(),
        )));
    }
    Ok(Json(json!({ "result": result })).into_response())
}

fn enforce_mcp_app_approval(
    approval: Option<&str>,
    approval_granted: bool,
    effect: ToolEffect,
) -> Result<(), Error> {
    match approval {
        Some("review") if !approval_granted => Err(Error::InvalidRequest(
            "this MCP App tool call requires approval".to_string(),
        )),
        Some("review") => Ok(()),
        Some("open") => Ok(()),
        _ if effect == ToolEffect::ReadOnly => Ok(()),
        _ => Err(Error::InvalidRequest(
            "guarded mode only allows read-only MCP App tools".to_string(),
        )),
    }
}

fn validate_mcp_app_resource(
    result: &Value,
    requested_uri: &str,
) -> Result<McpAppViewDocument, Error> {
    if !requested_uri.starts_with("ui://") {
        return Err(Error::InvalidRequest(
            "MCP App resources must use ui:// URIs".to_string(),
        ));
    }
    let content = result
        .get("contents")
        .and_then(Value::as_array)
        .and_then(|contents| {
            contents
                .iter()
                .find(|content| content.get("uri").and_then(Value::as_str) == Some(requested_uri))
        })
        .ok_or_else(|| {
            Error::InvalidRequest(
                "MCP App resource response did not contain the requested URI".to_string(),
            )
        })?;
    if content.get("mimeType").and_then(Value::as_str) != Some(MCP_APP_HTML_MIME) {
        return Err(Error::InvalidRequest(format!(
            "MCP App resource must use MIME type {MCP_APP_HTML_MIME}"
        )));
    }
    let html = if let Some(text) = content.get("text").and_then(Value::as_str) {
        text.to_string()
    } else if let Some(blob) = content.get("blob").and_then(Value::as_str) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(blob)
            .map_err(|_| {
                Error::InvalidRequest("MCP App resource blob is not valid base64".to_string())
            })?;
        String::from_utf8(bytes).map_err(|_| {
            Error::InvalidRequest("MCP App resource blob is not UTF-8 HTML".to_string())
        })?
    } else {
        return Err(Error::InvalidRequest(
            "MCP App resource must contain text or blob HTML".to_string(),
        ));
    };
    if html.len() > MCP_APP_HTML_LIMIT {
        return Err(Error::InvalidRequest(
            "MCP App HTML exceeds the 5 MiB limit".to_string(),
        ));
    }
    Ok(McpAppViewDocument {
        html,
        csp: mcp_app_content_security_policy(content),
    })
}

fn mcp_app_content_security_policy(content: &Value) -> String {
    let csp = content.pointer("/_meta/ui/csp").unwrap_or(&Value::Null);
    let resources = csp.get("resourceDomains");
    [
        "default-src 'none'".to_string(),
        format!(
            "script-src {}",
            mcp_app_csp_sources(resources, &["'unsafe-inline'"])
        ),
        format!(
            "style-src {}",
            mcp_app_csp_sources(resources, &["'unsafe-inline'"])
        ),
        format!(
            "img-src {}",
            mcp_app_csp_sources(resources, &["data:", "blob:"])
        ),
        format!("font-src {}", mcp_app_csp_sources(resources, &["data:"])),
        format!(
            "media-src {}",
            mcp_app_csp_sources(resources, &["data:", "blob:"])
        ),
        format!(
            "connect-src {}",
            mcp_app_csp_sources(csp.get("connectDomains"), &[])
        ),
        format!(
            "frame-src {}",
            mcp_app_csp_sources(csp.get("frameDomains"), &[])
        ),
        format!(
            "base-uri {}",
            mcp_app_csp_sources(csp.get("baseUriDomains"), &[])
        ),
        "form-action 'none'".to_string(),
        "object-src 'none'".to_string(),
    ]
    .join("; ")
}

fn mcp_app_csp_sources(value: Option<&Value>, defaults: &[&str]) -> String {
    let mut sources = defaults
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if let Some(values) = value.and_then(Value::as_array) {
        sources.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| valid_mcp_app_origin(value))
                .map(str::to_string),
        );
    }
    if sources.is_empty() {
        "'none'".to_string()
    } else {
        sources.join(" ")
    }
}

fn valid_mcp_app_origin(value: &str) -> bool {
    if value
        .chars()
        .any(|character| character.is_whitespace() || matches!(character, ';' | '\'' | '"'))
    {
        return false;
    }
    let Some((scheme, authority)) = value.split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https" | "ws" | "wss")
        && !authority.is_empty()
        && !authority.contains(['/', '?', '#'])
}

#[cfg(test)]
mod mcp_app_tests {
    use super::*;

    #[test]
    fn app_resource_requires_matching_uri_mime_and_size() {
        let valid = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":MCP_APP_HTML_MIME,
            "text":"<h1>Chart</h1>",
            "_meta":{"ui":{"csp":{
                "connectDomains":["https://api.example.com","https://bad.example;script-src *"],
                "resourceDomains":["https://cdn.example.com"]
            }}}
        }]});
        let document = validate_mcp_app_resource(&valid, "ui://fixture/chart").unwrap();
        assert!(document.csp.contains("connect-src https://api.example.com"));
        assert!(document
            .csp
            .contains("script-src 'unsafe-inline' https://cdn.example.com"));
        assert!(!document.csp.contains("bad.example"));
        let wrong_mime = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":"text/html",
            "text":"<h1>Chart</h1>"
        }]});
        assert!(validate_mcp_app_resource(&wrong_mime, "ui://fixture/chart").is_err());
        let oversized = json!({"contents":[{
            "uri":"ui://fixture/chart",
            "mimeType":MCP_APP_HTML_MIME,
            "text":"x".repeat(MCP_APP_HTML_LIMIT + 1)
        }]});
        assert!(validate_mcp_app_resource(&oversized, "ui://fixture/chart").is_err());
        assert!(validate_mcp_app_resource(&valid, "https://fixture/chart").is_err());
    }

    #[test]
    fn app_approval_enforces_review_guarded_and_open() {
        assert!(enforce_mcp_app_approval(Some("review"), false, ToolEffect::ReadOnly).is_err());
        assert!(enforce_mcp_app_approval(Some("review"), true, ToolEffect::Mutating).is_ok());
        assert!(enforce_mcp_app_approval(Some("guarded"), false, ToolEffect::ReadOnly).is_ok());
        assert!(enforce_mcp_app_approval(Some("guarded"), false, ToolEffect::Mutating).is_err());
        assert!(enforce_mcp_app_approval(Some("open"), false, ToolEffect::Mutating).is_ok());
    }
}

// ----- Workspace (host working folder for filesystem/shell tools) -----

#[derive(Deserialize)]
pub(crate) struct WorkspaceSet {
    #[serde(default)]
    folder: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitStatus {
    state: String,
    folder: Option<String>,
    is_repo: bool,
    root: Option<String>,
    branch: Option<String>,
    head: Option<String>,
    upstream: Option<String>,
    remote: Option<String>,
    ahead: u32,
    behind: u32,
    staged: u32,
    unstaged: u32,
    untracked: u32,
    conflicts: u32,
    insertions: u32,
    deletions: u32,
    has_changes: bool,
    changed_file_count: u32,
    changed_files: Vec<WorkspaceGitFileChange>,
    branches: Vec<WorkspaceGitBranch>,
    recent_commits: Vec<WorkspaceGitCommit>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitFileChange {
    status: String,
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitBranch {
    name: String,
    current: bool,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitCommit {
    hash: String,
    subject: String,
}

#[derive(Deserialize)]
pub(crate) struct WorkspaceGitActionRequest {
    action: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    checkpoint: Option<String>,
    #[serde(default)]
    stage_all: bool,
    #[serde(default)]
    staged_only: bool,
    #[serde(default)]
    diff_scope: Option<String>,
    #[serde(default)]
    diff_base: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    worktree: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    pull_request: Option<u64>,
    #[serde(default)]
    review_action: Option<String>,
    #[serde(default)]
    merge_method: Option<String>,
    #[serde(default)]
    expected_head: Option<String>,
    #[serde(default)]
    repository: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceGitActionResponse {
    ok: bool,
    action: String,
    command: String,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    message: String,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    undo_checkpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflicts: Option<Vec<String>>,
    pull_request: Option<Value>,
}

#[derive(Debug, Default)]
struct GitChangeCounts {
    staged: u32,
    unstaged: u32,
    untracked: u32,
    conflicts: u32,
}

/// `GET /workspace` — the current host working folder (or null).
pub(crate) async fn workspace_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let cur = st
        .workspace
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|p| p.to_string_lossy().to_string());
    Ok(Json(json!({ "folder": cur })).into_response())
}

/// `POST /workspace` — set (or clear, with empty/null) the host working folder
/// that the filesystem/shell tools operate within.
pub(crate) async fn workspace_set(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkspaceSet>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = req
        .folder
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from)
        .map(|path| {
            let canonical = std::fs::canonicalize(&path).map_err(|error| {
                ApiError(Error::InvalidRequest(format!(
                    "invalid workspace {}: {error}",
                    path.display()
                )))
            })?;
            if !canonical.is_dir() {
                return Err(ApiError(Error::InvalidRequest(format!(
                    "workspace is not a directory: {}",
                    canonical.display()
                ))));
            }
            Ok(canonical)
        })
        .transpose()?;
    if let Ok(mut g) = st.workspace.write() {
        *g = folder.clone();
    }
    Ok(Json(json!({ "folder": folder.map(|p| p.to_string_lossy().to_string()) })).into_response())
}

/// `GET /preview-apps/{thread_id}` - status for a managed preview app.
pub(crate) async fn preview_app_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.status(&thread_id)?).into_response())
}

/// `POST /preview-apps/{thread_id}/stage` - stage no-folder named artifact files.
pub(crate) async fn preview_app_stage(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Json(req): Json<PreviewAppStageRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.stage(&thread_id, &req.files)?).into_response())
}

/// `POST /preview-apps/{thread_id}/preflight` - inspect commands without staging or executing.
pub(crate) async fn preview_app_preflight(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Json(req): Json<PreviewAppPreflightRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.preflight(&thread_id, &req)?).into_response())
}

/// `POST /preview-apps/{thread_id}/start` - install and start the preview app.
pub(crate) async fn preview_app_start(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    req: Option<Json<PreviewAppStartRequest>>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let request = req.map(|Json(req)| req).unwrap_or_default();
    Ok(Json(st.preview_runtime.start(&thread_id, &request)?).into_response())
}

/// `POST /preview-apps/{thread_id}/stop` - stop the preview app process tree.
pub(crate) async fn preview_app_stop(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.stop(&thread_id).await?).into_response())
}

/// `POST /preview-apps/{thread_id}/restart` - stop, then start the preview app.
pub(crate) async fn preview_app_restart(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    req: Option<Json<PreviewAppStartRequest>>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let request = req.map(|Json(req)| req).unwrap_or_default();
    Ok(Json(st.preview_runtime.restart(&thread_id, &request).await?).into_response())
}

/// `GET /preview-apps/{thread_id}/logs` - recent preview app logs.
pub(crate) async fn preview_app_logs(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(thread_id): Path<String>,
    Query(query): Query<PreviewAppLogsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(st.preview_runtime.logs_after(&thread_id, query.after_seq)?).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct PreviewAppLogsQuery {
    after_seq: Option<u64>,
}

/// `GET /workspace/git` - Git status for the current host working folder.
pub(crate) async fn workspace_git_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = st.workspace.read().ok().and_then(|g| g.clone());
    let status = tokio::task::spawn_blocking(move || workspace_git_status_blocking(folder))
        .await
        .map_err(|e| ApiError(Error::Other(format!("git status task failed: {e}"))))?;
    Ok(Json(status).into_response())
}

/// `GET /workspace/context` - resolved repository instructions and stable identity.
pub(crate) async fn workspace_context(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let folder = workspace_snapshot(&st);
    let context =
        tokio::task::spawn_blocking(move || crate::workspace_context::resolve(folder.as_deref()))
            .await
            .map_err(|error| {
                ApiError(Error::Other(format!(
                    "workspace context task failed: {error}"
                )))
            })?;
    Ok(Json(context).into_response())
}

/// `POST /workspace/git/action` - run a narrow, guarded Git sidebar action.
pub(crate) async fn workspace_git_action(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(mut req): Json<WorkspaceGitActionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let action = req.action.trim().to_string();
    if !matches!(
        action.as_str(),
        "diff"
            | "fetch"
            | "pull"
            | "push"
            | "publish"
            | "commit"
            | "commit_push"
            | "checkout_branch"
            | "create_branch"
            | "checkpoint"
            | "restore_checkpoint"
            | "create_retry_worktree"
            | "apply_retry_worktree"
            | "remove_retry_worktree"
            | "create_thread_worktree"
            | "remove_thread_worktree"
            | "pr_list"
            | "pr_view"
            | "pr_status"
            | "pr_create"
            | "pr_ready"
            | "pr_comment"
            | "pr_review"
            | "pr_merge"
    ) {
        return Err(ApiError(Error::InvalidRequest(format!(
            "unsupported git action: {action}"
        ))));
    }

    let active_folder = st.workspace.read().ok().and_then(|g| g.clone());
    let folder = if action.starts_with("pr_") {
        req.folder
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or(active_folder)
    } else {
        if req
            .folder
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(ApiError(Error::InvalidRequest(
                "folder override is only supported for pull request actions".to_string(),
            )));
        }
        active_folder
    };
    let hot_swap_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("hot-swap");
    let thread_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("threads");
    req.action = action;
    let result = tokio::task::spawn_blocking(move || {
        workspace_git_action_blocking(folder, req, hot_swap_root, thread_root)
    })
    .await
    .map_err(|e| ApiError(Error::Other(format!("git action task failed: {e}"))))?;
    Ok(Json(result).into_response())
}

fn workspace_git_status_blocking(folder: Option<PathBuf>) -> WorkspaceGitStatus {
    let Some(folder) = folder else {
        return workspace_git_status_message(
            "no_folder",
            None,
            false,
            "No working folder selected",
        );
    };
    let folder_text = folder.to_string_lossy().to_string();

    match std::fs::metadata(&folder) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                false,
                "Selected working folder is not a directory",
            )
        }
        Err(e) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                false,
                &format!("Failed to read working folder metadata: {e}"),
            )
        }
    }

    let inside = match git_output(&folder, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(_) => {
            return workspace_git_status_message(
                "not_git",
                Some(folder_text),
                false,
                "No Git repository found in the selected folder",
            )
        }
        Err(e) => return workspace_git_status_message("error", Some(folder_text), false, &e),
    };

    if inside.trim() != "true" {
        return workspace_git_status_message(
            "not_git",
            Some(folder_text),
            false,
            "No Git worktree found in the selected folder",
        );
    }

    let root_text = git_text(&folder, &["rev-parse", "--show-toplevel"])
        .unwrap_or_else(|| folder.to_string_lossy().to_string());
    let root = PathBuf::from(&root_text);
    let branch = git_text(&root, &["branch", "--show-current"]);
    let head = git_text(&root, &["rev-parse", "--short", "HEAD"]);
    let upstream = git_text(
        &root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    let remote = git_text(&root, &["remote", "get-url", "origin"]);
    let branches = workspace_git_branches(&root, branch.as_deref());
    let recent_commits = workspace_git_recent_commits(&root);
    let (ahead, behind) = if upstream.is_some() {
        git_text(
            &root,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        )
        .map(|text| parse_ahead_behind(&text))
        .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    let porcelain = match git_output(
        &root,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=normal",
        ],
    ) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(output) => {
            return workspace_git_status_message(
                "error",
                Some(folder_text),
                true,
                &format!("Failed to read git status: {}", output_error_text(&output)),
            )
        }
        Err(e) => return workspace_git_status_message("error", Some(folder_text), true, &e),
    };

    let counts = parse_git_porcelain_counts(&porcelain);
    let (changed_file_count, changed_files) = parse_git_porcelain_files(&porcelain);
    let (insertions, deletions) = git_text(&root, &["diff", "--shortstat", "HEAD"])
        .map(|text| parse_git_shortstat(&text))
        .unwrap_or((0, 0));
    let has_changes = counts.staged + counts.unstaged + counts.untracked + counts.conflicts > 0;

    WorkspaceGitStatus {
        state: "ready".to_string(),
        folder: Some(folder_text),
        is_repo: true,
        root: Some(root_text),
        branch,
        head,
        upstream,
        remote,
        ahead,
        behind,
        staged: counts.staged,
        unstaged: counts.unstaged,
        untracked: counts.untracked,
        conflicts: counts.conflicts,
        insertions,
        deletions,
        has_changes,
        changed_file_count,
        changed_files,
        branches,
        recent_commits,
        message: None,
    }
}

fn workspace_git_status_message(
    state: &str,
    folder: Option<String>,
    is_repo: bool,
    message: &str,
) -> WorkspaceGitStatus {
    WorkspaceGitStatus {
        state: state.to_string(),
        folder,
        is_repo,
        root: None,
        branch: None,
        head: None,
        upstream: None,
        remote: None,
        ahead: 0,
        behind: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        insertions: 0,
        deletions: 0,
        has_changes: false,
        changed_file_count: 0,
        changed_files: Vec::new(),
        branches: Vec::new(),
        recent_commits: Vec::new(),
        message: Some(message.to_string()),
    }
}

fn workspace_git_recent_commits(root: &FsPath) -> Vec<WorkspaceGitCommit> {
    let Ok(output) = git_output(root, &["log", "-n", "5", "--format=%h%x00%s"]) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (hash, subject) = line.split_once('\0')?;
            let hash = hash.trim();
            let subject = subject.trim();
            if hash.is_empty() || subject.is_empty() {
                return None;
            }
            Some(WorkspaceGitCommit {
                hash: hash.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

fn workspace_git_branches(root: &FsPath, current: Option<&str>) -> Vec<WorkspaceGitBranch> {
    let Ok(output) = git_output(
        root,
        &[
            "for-each-ref",
            "--format=%(refname:short)%00%(upstream:short)",
            "refs/heads",
        ],
    ) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut branches: Vec<WorkspaceGitBranch> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let upstream = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let (ahead, behind) = upstream
                .as_deref()
                .and_then(|upstream| {
                    let range = format!("{name}...{upstream}");
                    git_text(
                        root,
                        &["rev-list", "--left-right", "--count", range.as_str()],
                    )
                })
                .map(|text| parse_ahead_behind(&text))
                .unwrap_or((0, 0));
            Some(WorkspaceGitBranch {
                name: name.to_string(),
                current: current == Some(name),
                upstream,
                ahead,
                behind,
            })
        })
        .collect();

    branches.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| a.name.cmp(&b.name)));
    branches
}

fn git_output(cwd: &FsPath, args: &[&str]) -> std::result::Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    milim_core::proc::hide_console(&mut cmd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))
}

fn git_output_with_env(
    cwd: &FsPath,
    args: &[&str],
    envs: &[(&str, String)],
) -> std::result::Result<Output, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    milim_core::proc::hide_console(&mut cmd)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))
}

fn git_text(cwd: &FsPath, args: &[&str]) -> Option<String> {
    let output = git_output(cwd, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = output_text(&output);
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

fn output_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn output_error_text(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("git exited with {}", output.status)
    } else {
        stderr
    }
}

fn parse_ahead_behind(text: &str) -> (u32, u32) {
    let mut parts = text.split_whitespace();
    let ahead = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    let behind = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

fn parse_git_porcelain_counts(text: &str) -> GitChangeCounts {
    let mut counts = GitChangeCounts::default();
    for line in text.lines() {
        if line.starts_with("##") || line.len() < 2 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == '?' && y == '?' {
            counts.untracked += 1;
            continue;
        }
        if x == '!' && y == '!' {
            continue;
        }
        if is_git_conflict_status(x, y) {
            counts.conflicts += 1;
            continue;
        }
        if x != ' ' {
            counts.staged += 1;
        }
        if y != ' ' {
            counts.unstaged += 1;
        }
    }
    counts
}

fn is_git_conflict_status(x: char, y: char) -> bool {
    matches!(
        (x, y),
        ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
    )
}

fn parse_git_porcelain_files(text: &str) -> (u32, Vec<WorkspaceGitFileChange>) {
    let mut count = 0;
    let mut files = Vec::new();
    for line in text.lines() {
        if line.starts_with("##") || line.len() < 3 {
            continue;
        }
        let status = line[..2].trim().to_string();
        let path = line[3..].trim().to_string();
        if !status.is_empty() && !path.is_empty() {
            count += 1;
            files.push(WorkspaceGitFileChange { status, path });
        }
    }
    (count, files)
}

fn parse_git_shortstat(text: &str) -> (u32, u32) {
    let mut insertions = 0;
    let mut deletions = 0;
    for part in text.split(',').map(str::trim) {
        let mut words = part.split_whitespace();
        let Some(value) = words.next().and_then(|word| word.parse::<u32>().ok()) else {
            continue;
        };
        let Some(kind) = words.next() else {
            continue;
        };
        if kind.starts_with("insertion") {
            insertions = value;
        } else if kind.starts_with("deletion") {
            deletions = value;
        }
    }
    (insertions, deletions)
}

fn workspace_git_action_blocking(
    folder: Option<PathBuf>,
    request: WorkspaceGitActionRequest,
    hot_swap_root: PathBuf,
    thread_root: PathBuf,
) -> WorkspaceGitActionResponse {
    const OUTPUT_LIMIT: usize = 24_000;
    let WorkspaceGitActionRequest {
        action,
        folder: _,
        message,
        checkpoint,
        stage_all,
        staged_only,
        diff_scope,
        diff_base,
        branch,
        worktree,
        thread_id,
        force,
        title,
        body,
        base,
        draft,
        pull_request,
        review_action,
        merge_method,
        expected_head,
        repository,
    } = request;

    if action == "pr_list" {
        return workspace_git_pr_list_action();
    }
    if action == "pr_view" {
        return workspace_git_pr_view_action(repository, pull_request);
    }
    if matches!(
        action.as_str(),
        "pr_ready" | "pr_comment" | "pr_review" | "pr_merge"
    ) && repository.is_some()
    {
        return workspace_git_pr_mutation_action_global(
            repository,
            &action,
            pull_request,
            body,
            review_action,
            merge_method,
            expected_head,
        );
    }

    let status = workspace_git_status_blocking(folder);
    let Some(root) = status.root.as_ref().map(PathBuf::from) else {
        return workspace_git_action_message(
            &action,
            "",
            false,
            &git_state_label_for_action(&status),
        );
    };

    if action == "diff" {
        let scope = diff_scope
            .as_deref()
            .unwrap_or(if staged_only { "staged" } else { "all" });
        return workspace_git_diff_action(
            &root,
            &action,
            status.head.is_some(),
            scope,
            diff_base.as_deref(),
        );
    }
    if action == "checkpoint" {
        return workspace_git_checkpoint_action(&root, &status, message);
    }
    if action == "restore_checkpoint" {
        return workspace_git_restore_checkpoint_action(&root, checkpoint);
    }
    if action == "create_retry_worktree" {
        return workspace_git_create_retry_worktree_action(&root, checkpoint, &hot_swap_root);
    }
    if action == "apply_retry_worktree" {
        return workspace_git_apply_retry_worktree_action(
            &root,
            checkpoint,
            worktree,
            &hot_swap_root,
        );
    }
    if action == "remove_retry_worktree" {
        return workspace_git_remove_retry_worktree_action(&root, worktree, &hot_swap_root);
    }
    if action == "create_thread_worktree" {
        return workspace_git_create_thread_worktree_action(
            &root,
            &status,
            thread_id,
            &thread_root,
        );
    }
    if action == "remove_thread_worktree" {
        return workspace_git_remove_thread_worktree_action(&root, thread_id, force, &thread_root);
    }
    if action == "pr_status" {
        return workspace_git_pr_status_action(&root, &status);
    }
    if action == "pr_create" {
        return workspace_git_pr_create_action(&root, &status, title, body, base, draft);
    }
    if matches!(
        action.as_str(),
        "pr_ready" | "pr_comment" | "pr_review" | "pr_merge"
    ) {
        return workspace_git_pr_mutation_action(
            &root,
            &status,
            &action,
            pull_request,
            body,
            review_action,
            merge_method,
            expected_head,
        );
    }
    if matches!(action.as_str(), "commit" | "commit_push") {
        return workspace_git_commit_action(&root, &action, &status, message, stage_all);
    }

    let requested_branch = branch.unwrap_or_default().trim().to_string();
    let args: Vec<String> = match action.as_str() {
        "fetch" => vec!["fetch".into(), "--prune".into()],
        "checkout_branch" => {
            if requested_branch.is_empty() {
                return workspace_git_action_message(
                    &action,
                    "git checkout <branch>",
                    false,
                    "Branch name required.",
                );
            }
            vec!["checkout".into(), requested_branch.clone()]
        }
        "create_branch" => {
            if requested_branch.is_empty() {
                return workspace_git_action_message(
                    &action,
                    "git checkout -b <branch>",
                    false,
                    "Branch name required.",
                );
            }
            vec!["checkout".into(), "-b".into(), requested_branch.clone()]
        }
        "pull" => {
            if status.has_changes {
                return workspace_git_action_message(
                    &action,
                    "git pull --ff-only",
                    false,
                    "Pull requires a clean worktree.",
                );
            }
            if status.upstream.is_none() || status.behind == 0 {
                return workspace_git_action_message(
                    &action,
                    "git pull --ff-only",
                    false,
                    "Nothing to pull from an upstream branch.",
                );
            }
            vec!["pull".into(), "--ff-only".into()]
        }
        "push" => {
            if status.upstream.is_none() || status.ahead == 0 {
                return workspace_git_action_message(
                    &action,
                    "git push",
                    false,
                    "Nothing to push to an upstream branch.",
                );
            }
            vec!["push".into()]
        }
        "publish" => {
            let Some(branch) = status.branch.as_deref().filter(|s| !s.trim().is_empty()) else {
                return workspace_git_action_message(
                    &action,
                    "git push -u origin <branch>",
                    false,
                    "Publish requires a named branch.",
                );
            };
            if status.remote.is_none() || status.upstream.is_some() {
                return workspace_git_action_message(
                    &action,
                    &format!("git push -u origin {branch}"),
                    false,
                    "Publish requires a remote and no upstream.",
                );
            }
            vec![
                "push".into(),
                "-u".into(),
                "origin".into(),
                branch.to_string(),
            ]
        }
        _ => return workspace_git_action_message(&action, "", false, "Unsupported Git action."),
    };

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = format!("git {}", args.join(" "));
    match git_output(&root, &arg_refs) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let (stdout, stdout_truncated) = truncate_git_action_output(stdout, OUTPUT_LIMIT);
            let (stderr, stderr_truncated) = truncate_git_action_output(stderr, OUTPUT_LIMIT);
            let ok = output.status.success();
            let message = if ok {
                match action.as_str() {
                    "diff" if stdout.trim().is_empty() => "No diff to show.".to_string(),
                    "diff" => "Diff ready.".to_string(),
                    "fetch" => "Fetch complete.".to_string(),
                    "checkout_branch" => "Branch checked out.".to_string(),
                    "create_branch" => "Branch created.".to_string(),
                    "pull" => "Pull complete.".to_string(),
                    "push" => "Push complete.".to_string(),
                    "publish" => "Branch published.".to_string(),
                    _ => "Git action complete.".to_string(),
                }
            } else {
                output_error_text(&output)
            };
            WorkspaceGitActionResponse {
                ok,
                action,
                command,
                stdout,
                stderr,
                exit_code: output.status.code(),
                message,
                truncated: stdout_truncated || stderr_truncated,
                checkpoint: None,
                root: None,
                head: None,
                worktree: None,
                undo_checkpoint: None,
                conflicts: None,
                pull_request: None,
            }
        }
        Err(e) => workspace_git_action_message(&action, &command, false, &e),
    }
}

fn workspace_git_commit_action(
    root: &FsPath,
    action: &str,
    status: &WorkspaceGitStatus,
    message: Option<String>,
    stage_all: bool,
) -> WorkspaceGitActionResponse {
    let message = message.unwrap_or_default().trim().to_string();
    if message.is_empty() {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Commit message required.",
        );
    }
    if !status.has_changes {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Nothing to commit.",
        );
    }
    if status.conflicts > 0 {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "Resolve conflicts before committing.",
        );
    }
    if !stage_all && status.staged == 0 {
        return workspace_git_action_message(
            action,
            "git commit -m <message>",
            false,
            "No staged changes to commit.",
        );
    }

    let push_args: Option<Vec<String>> = if action == "commit_push" {
        if status.behind > 0 {
            return workspace_git_action_message(action, "git push", false, "Pull before pushing.");
        }
        if status.remote.is_none() {
            return workspace_git_action_message(
                action,
                "git push",
                false,
                "No remote configured.",
            );
        }
        if status.upstream.is_some() {
            Some(vec!["push".into()])
        } else {
            let Some(branch) = status.branch.as_deref().filter(|s| !s.trim().is_empty()) else {
                return workspace_git_action_message(
                    action,
                    "git push -u origin <branch>",
                    false,
                    "Publish requires a named branch.",
                );
            };
            Some(vec![
                "push".into(),
                "-u".into(),
                "origin".into(),
                branch.to_string(),
            ])
        }
    } else {
        None
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut commands = Vec::new();

    if stage_all {
        let args = ["add", "-A"];
        commands.push(git_command_text(&args));
        match git_output(root, &args) {
            Ok(output) if output.status.success() => {
                append_git_output(&mut stdout, &mut stderr, &output)
            }
            Ok(output) => {
                append_git_output(&mut stdout, &mut stderr, &output);
                return workspace_git_combined_response(
                    action,
                    &commands.join(" && "),
                    false,
                    stdout,
                    stderr,
                    output.status.code(),
                    output_error_text(&output),
                );
            }
            Err(e) => {
                return workspace_git_action_message(action, &commands.join(" && "), false, &e)
            }
        }
    }

    let commit_args = ["commit", "-m", message.as_str()];
    commands.push(git_command_text(&commit_args));
    match git_output(root, &commit_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                action,
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => return workspace_git_action_message(action, &commands.join(" && "), false, &e),
    }

    if let Some(args) = push_args {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        commands.push(git_command_text(&refs));
        match git_output(root, &refs) {
            Ok(output) if output.status.success() => {
                append_git_output(&mut stdout, &mut stderr, &output)
            }
            Ok(output) => {
                append_git_output(&mut stdout, &mut stderr, &output);
                return workspace_git_combined_response(
                    action,
                    &commands.join(" && "),
                    false,
                    stdout,
                    stderr,
                    output.status.code(),
                    output_error_text(&output),
                );
            }
            Err(e) => {
                return workspace_git_action_message(action, &commands.join(" && "), false, &e)
            }
        }
    }

    workspace_git_combined_response(
        action,
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        if action == "commit_push" {
            "Commit and push complete.".to_string()
        } else {
            "Commit complete.".to_string()
        },
    )
}

fn workspace_git_checkpoint_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    message: Option<String>,
) -> WorkspaceGitActionResponse {
    let Some(index_path) = git_text(root, &["rev-parse", "--git-path", "index"]) else {
        return workspace_git_action_message(
            "checkpoint",
            "git rev-parse --git-path index",
            false,
            "Failed to locate the Git index.",
        );
    };
    let index_path = {
        let path = PathBuf::from(index_path);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let temp_index = index_path.with_file_name(format!("milim-{}.index", gen_id("checkpoint")));
    let index_env = [("GIT_INDEX_FILE", temp_index.to_string_lossy().to_string())];
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut commands = Vec::new();

    let add_args = ["add", "-A", "--"];
    commands.push(format!(
        "GIT_INDEX_FILE={} {}",
        temp_index.display(),
        git_command_text(&add_args)
    ));
    match git_output_with_env(root, &add_args, &index_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            let _ = std::fs::remove_file(&temp_index);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp_index);
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e);
        }
    }

    let write_tree_args = ["write-tree"];
    commands.push(format!(
        "GIT_INDEX_FILE={} {}",
        temp_index.display(),
        git_command_text(&write_tree_args)
    ));
    let tree = match git_output_with_env(root, &write_tree_args, &index_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output);
            output_text(&output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            let _ = std::fs::remove_file(&temp_index);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp_index);
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e);
        }
    };
    let _ = std::fs::remove_file(&temp_index);

    let checkpoint_ref = format!("refs/milim/checkpoints/{}", gen_id("turn"));
    let checkpoint_label = message
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or("turn");
    let commit_message = format!("milim workspace checkpoint: {checkpoint_label}");
    let head = git_text(root, &["rev-parse", "HEAD"]);
    let mut commit_args = vec!["commit-tree", tree.as_str()];
    if let Some(head) = head.as_deref() {
        commit_args.push("-p");
        commit_args.push(head);
    }
    commit_args.push("-m");
    commit_args.push(commit_message.as_str());
    commands.push(git_command_text(&commit_args));
    let commit_env = [
        ("GIT_AUTHOR_NAME", "milim".to_string()),
        ("GIT_AUTHOR_EMAIL", "milim@example.invalid".to_string()),
        ("GIT_COMMITTER_NAME", "milim".to_string()),
        ("GIT_COMMITTER_EMAIL", "milim@example.invalid".to_string()),
    ];
    let commit = match git_output_with_env(root, &commit_args, &commit_env) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output);
            output_text(&output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e)
        }
    };

    let update_ref_args = ["update-ref", checkpoint_ref.as_str(), commit.as_str()];
    commands.push(git_command_text(&update_ref_args));
    match git_output(root, &update_ref_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message("checkpoint", &commands.join(" && "), false, &e)
        }
    }

    let mut response = workspace_git_combined_response(
        "checkpoint",
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        "Workspace checkpoint created.".to_string(),
    );
    response.checkpoint = Some(checkpoint_ref);
    response.root = Some(root.to_string_lossy().to_string());
    response.head = status.head.clone();
    response
}

fn workspace_git_restore_checkpoint_action(
    root: &FsPath,
    checkpoint: Option<String>,
) -> WorkspaceGitActionResponse {
    let checkpoint = checkpoint.unwrap_or_default().trim().to_string();
    if checkpoint.is_empty() {
        return workspace_git_action_message(
            "restore_checkpoint",
            "git read-tree --reset -u <checkpoint>",
            false,
            "Checkpoint ref required.",
        );
    }

    let treeish = format!("{checkpoint}^{{tree}}");
    let tree = match git_output(root, &["rev-parse", "--verify", treeish.as_str()]) {
        Ok(output) if output.status.success() => output_text(&output),
        Ok(output) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &format!("git rev-parse --verify {treeish}"),
                false,
                &output_error_text(&output),
            )
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &format!("git rev-parse --verify {treeish}"),
                false,
                &e,
            )
        }
    };

    let mut stdout = String::new();
    let mut stderr = String::new();
    let read_tree_args = ["read-tree", "--reset", "-u", tree.as_str()];
    let clean_args = ["clean", "-fd"];
    let commands = [
        git_command_text(&read_tree_args),
        git_command_text(&clean_args),
    ];

    match git_output(root, &read_tree_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                &e,
            )
        }
    }
    match git_output(root, &clean_args) {
        Ok(output) if output.status.success() => {
            append_git_output(&mut stdout, &mut stderr, &output)
        }
        Ok(output) => {
            append_git_output(&mut stdout, &mut stderr, &output);
            return workspace_git_combined_response(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                stdout,
                stderr,
                output.status.code(),
                output_error_text(&output),
            );
        }
        Err(e) => {
            return workspace_git_action_message(
                "restore_checkpoint",
                &commands.join(" && "),
                false,
                &e,
            )
        }
    }

    let mut response = workspace_git_combined_response(
        "restore_checkpoint",
        &commands.join(" && "),
        true,
        stdout,
        stderr,
        Some(0),
        "Workspace restored to checkpoint.".to_string(),
    );
    response.checkpoint = Some(checkpoint);
    response.root = Some(root.to_string_lossy().to_string());
    response.head = git_text(root, &["rev-parse", "--short", "HEAD"]);
    response
}

fn valid_milim_checkpoint(checkpoint: Option<String>) -> Option<String> {
    let checkpoint = checkpoint.unwrap_or_default().trim().to_string();
    if checkpoint.starts_with("refs/milim/checkpoints/") {
        Some(checkpoint)
    } else {
        None
    }
}

fn retry_worktree_path(
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(worktree.unwrap_or_default());
    let root = std::fs::canonicalize(hot_swap_root)
        .map_err(|e| format!("Hot Swap runtime is unavailable: {e}"))?;
    let path = std::fs::canonicalize(&requested)
        .map_err(|e| format!("Retry worktree is unavailable: {e}"))?;
    if path.starts_with(&root) {
        Ok(path)
    } else {
        Err("Retry worktree must be inside Milim's runtime directory.".to_string())
    }
}

fn git_common_dir(root: &FsPath) -> Option<PathBuf> {
    let raw = PathBuf::from(git_text(root, &["rev-parse", "--git-common-dir"])?);
    std::fs::canonicalize(if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    })
    .ok()
}

fn workspace_git_create_retry_worktree_action(
    root: &FsPath,
    checkpoint: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let Some(checkpoint) = valid_milim_checkpoint(checkpoint) else {
        return workspace_git_action_message(
            "create_retry_worktree",
            "git rev-parse --verify <checkpoint>",
            false,
            "A Milim workspace checkpoint is required.",
        );
    };
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message(
            "create_retry_worktree",
            "",
            false,
            &format!("Failed to create Hot Swap runtime directory: {e}"),
        );
    }
    let worktree = hot_swap_root.join(gen_id("retry"));
    let worktree_text = worktree.to_string_lossy().to_string();
    let args = [
        "worktree",
        "add",
        "--detach",
        worktree_text.as_str(),
        checkpoint.as_str(),
    ];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(e) => {
            return workspace_git_action_message(
                "create_retry_worktree",
                &git_command_text(&args),
                false,
                &e,
            )
        }
    };
    let ok = output.status.success();
    let mut response = workspace_git_combined_response(
        "create_retry_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Retry worktree created.".to_string()
        } else {
            output_error_text(&output)
        },
    );
    if ok {
        response.checkpoint = Some(checkpoint);
        response.root = Some(root.to_string_lossy().to_string());
        response.worktree = Some(worktree_text);
    }
    response
}

fn workspace_git_apply_retry_worktree_action(
    root: &FsPath,
    checkpoint: Option<String>,
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let Some(checkpoint) = valid_milim_checkpoint(checkpoint) else {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "git rev-parse --verify <checkpoint>",
            false,
            "A Milim workspace checkpoint is required.",
        );
    };
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message("apply_retry_worktree", "", false, &e.to_string());
    }
    let worktree = match retry_worktree_path(worktree, hot_swap_root) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("apply_retry_worktree", "", false, &message)
        }
    };
    if git_common_dir(root) != git_common_dir(&worktree) {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            "Retry worktree does not belong to the selected repository.",
        );
    }

    let retry_status = workspace_git_status_blocking(Some(worktree.clone()));
    let retry_checkpoint = workspace_git_checkpoint_action(
        &worktree,
        &retry_status,
        Some("hot-swap-retry-result".to_string()),
    );
    let Some(retry_ref) = retry_checkpoint.checkpoint else {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            &retry_checkpoint.message,
        );
    };

    let diff_args = [
        "diff",
        "--binary",
        checkpoint.as_str(),
        retry_ref.as_str(),
        "--",
    ];
    let diff = match git_output(&worktree, &diff_args) {
        Ok(output) if output.status.success() => output.stdout,
        Ok(output) => {
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&diff_args),
                false,
                &output_error_text(&output),
            )
        }
        Err(e) => {
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&diff_args),
                false,
                &e,
            )
        }
    };
    if diff.is_empty() {
        return workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&diff_args),
            true,
            "Retry workspace has no changes to apply.",
        );
    }
    let names = git_text(
        &worktree,
        &[
            "diff",
            "--name-only",
            checkpoint.as_str(),
            retry_ref.as_str(),
            "--",
        ],
    )
    .unwrap_or_default()
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_string)
    .collect::<Vec<_>>();
    let patch_path = hot_swap_root.join(format!("{}.patch", gen_id("apply")));
    if let Err(e) = std::fs::write(&patch_path, &diff) {
        return workspace_git_action_message(
            "apply_retry_worktree",
            "",
            false,
            &format!("Failed to prepare retry patch: {e}"),
        );
    }
    let patch_text = patch_path.to_string_lossy().to_string();
    let check_args = ["apply", "--check", "--binary", patch_text.as_str()];
    let check = git_output(root, &check_args);
    if !matches!(&check, Ok(output) if output.status.success()) {
        let _ = std::fs::remove_file(&patch_path);
        let detail = match check {
            Ok(output) => output_error_text(&output),
            Err(e) => e,
        };
        let mut response = workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&check_args),
            false,
            &format!("Retry changes conflict with the original workspace: {detail}"),
        );
        response.conflicts = Some(names);
        return response;
    }

    let original_status = workspace_git_status_blocking(Some(root.to_path_buf()));
    let undo = workspace_git_checkpoint_action(
        root,
        &original_status,
        Some("before-hot-swap-apply".to_string()),
    );
    let Some(undo_ref) = undo.checkpoint else {
        let _ = std::fs::remove_file(&patch_path);
        return workspace_git_action_message("apply_retry_worktree", "", false, &undo.message);
    };
    let apply_args = ["apply", "--binary", patch_text.as_str()];
    let output = match git_output(root, &apply_args) {
        Ok(output) => output,
        Err(e) => {
            let _ = std::fs::remove_file(&patch_path);
            return workspace_git_action_message(
                "apply_retry_worktree",
                &git_command_text(&apply_args),
                false,
                &e,
            );
        }
    };
    let _ = std::fs::remove_file(&patch_path);
    if !output.status.success() {
        let restored = workspace_git_restore_checkpoint_action(root, Some(undo_ref));
        return workspace_git_action_message(
            "apply_retry_worktree",
            &git_command_text(&apply_args),
            false,
            &format!(
                "Retry apply failed and the original workspace was {}: {}",
                if restored.ok {
                    "restored"
                } else {
                    "not restored"
                },
                output_error_text(&output)
            ),
        );
    }
    let mut response = workspace_git_combined_response(
        "apply_retry_worktree",
        &git_command_text(&apply_args),
        true,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Retry changes applied to the original workspace.".to_string(),
    );
    response.root = Some(root.to_string_lossy().to_string());
    response.worktree = Some(worktree.to_string_lossy().to_string());
    response.undo_checkpoint = Some(undo_ref);
    response
}

fn workspace_git_remove_retry_worktree_action(
    root: &FsPath,
    worktree: Option<String>,
    hot_swap_root: &FsPath,
) -> WorkspaceGitActionResponse {
    if let Err(e) = std::fs::create_dir_all(hot_swap_root) {
        return workspace_git_action_message("remove_retry_worktree", "", false, &e.to_string());
    }
    let worktree = match retry_worktree_path(worktree, hot_swap_root) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("remove_retry_worktree", "", false, &message)
        }
    };
    if git_common_dir(root) != git_common_dir(&worktree) {
        return workspace_git_action_message(
            "remove_retry_worktree",
            "",
            false,
            "Retry worktree does not belong to the selected repository.",
        );
    }
    let worktree_text = worktree.to_string_lossy().to_string();
    let args = ["worktree", "remove", "--force", worktree_text.as_str()];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(e) => {
            return workspace_git_action_message(
                "remove_retry_worktree",
                &git_command_text(&args),
                false,
                &e,
            )
        }
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        "remove_retry_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Retry worktree removed.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn valid_thread_runtime_id(thread_id: Option<String>) -> Result<String, String> {
    let value = thread_id.unwrap_or_default().trim().to_string();
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("A valid thread id is required.".to_string());
    }
    Ok(value)
}

fn workspace_git_create_thread_worktree_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    thread_id: Option<String>,
    thread_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let thread_id = match valid_thread_runtime_id(thread_id) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("create_thread_worktree", "", false, &message)
        }
    };
    if status.head.is_none() {
        return workspace_git_action_message(
            "create_thread_worktree",
            "git worktree add",
            false,
            "An initial commit is required before creating an isolated worktree.",
        );
    }
    if let Err(error) = std::fs::create_dir_all(thread_root) {
        return workspace_git_action_message(
            "create_thread_worktree",
            "",
            false,
            &format!("Failed to create the thread runtime directory: {error}"),
        );
    }
    let target = thread_root.join(&thread_id);
    if target.exists() {
        return workspace_git_action_message(
            "create_thread_worktree",
            "",
            false,
            "This thread already has a runtime worktree.",
        );
    }
    let short_id: String = thread_id.chars().take(8).collect();
    let branch = format!("milim/thread-{short_id}");
    let target_text = target.to_string_lossy().to_string();
    let args = [
        "worktree",
        "add",
        "-b",
        branch.as_str(),
        target_text.as_str(),
        "HEAD",
    ];
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message(
                "create_thread_worktree",
                &git_command_text(&args),
                false,
                &error,
            )
        }
    };
    let ok = output.status.success();
    let mut response = workspace_git_combined_response(
        "create_thread_worktree",
        &git_command_text(&args),
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            "Isolated thread worktree created. Uncommitted changes in the original checkout were not included.".to_string()
        } else {
            output_error_text(&output)
        },
    );
    if ok {
        response.root = Some(root.to_string_lossy().to_string());
        response.worktree = Some(target_text);
        response.head = git_text(root, &["rev-parse", "HEAD"]);
        response.stdout = branch;
    }
    response
}

fn workspace_git_remove_thread_worktree_action(
    root: &FsPath,
    thread_id: Option<String>,
    force: bool,
    thread_root: &FsPath,
) -> WorkspaceGitActionResponse {
    let thread_id = match valid_thread_runtime_id(thread_id) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("remove_thread_worktree", "", false, &message)
        }
    };
    let runtime_root = match std::fs::canonicalize(thread_root) {
        Ok(path) => path,
        Err(error) => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                "",
                false,
                &format!("Thread runtime is unavailable: {error}"),
            )
        }
    };
    let target = match std::fs::canonicalize(thread_root.join(thread_id)) {
        Ok(path) if path.starts_with(&runtime_root) => path,
        _ => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                "",
                false,
                "Thread worktree is outside Milim's runtime directory or no longer exists.",
            )
        }
    };
    if git_common_dir(root) != git_common_dir(&target) {
        return workspace_git_action_message(
            "remove_thread_worktree",
            "",
            false,
            "Thread worktree does not belong to the selected repository.",
        );
    }
    if !force {
        let dirty = git_text(&target, &["status", "--porcelain"]).unwrap_or_default();
        if !dirty.trim().is_empty() {
            return workspace_git_action_message(
                "remove_thread_worktree", "git worktree remove", false,
                "The isolated worktree has uncommitted changes. Confirm force-discard to delete it; its branch will be retained.",
            );
        }
    }
    let target_text = target.to_string_lossy().to_string();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(target_text.as_str());
    let output = match git_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message(
                "remove_thread_worktree",
                &git_command_text(&args),
                false,
                &error,
            )
        }
    };
    workspace_git_combined_response(
        "remove_thread_worktree",
        &git_command_text(&args),
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if output.status.success() {
            "Thread worktree removed. Its branch was retained.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn gh_output(root: &FsPath, args: &[&str]) -> Result<Output, String> {
    let mut command = Command::new("gh");
    command.current_dir(root).args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn gh_output_owned(root: &FsPath, args: &[String]) -> Result<Output, String> {
    let mut command = Command::new("gh");
    command.current_dir(root).args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn gh_output_global(args: &[String]) -> Result<Output, String> {
    let mut command = Command::new("gh");
    command.args(args);
    milim_core::proc::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Failed to run GitHub CLI: {error}"))
}

fn github_repository(value: Option<String>) -> Result<String, String> {
    let value = value.unwrap_or_default().trim().to_string();
    let mut parts = value.split('/');
    let valid_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(repository), None) if valid_part(owner) && valid_part(repository) => {
            Ok(value)
        }
        _ => Err("A GitHub repository in owner/name format is required.".to_string()),
    }
}

fn github_viewer_permission(output: Result<Output, String>) -> Option<String> {
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice::<Value>(&output.stdout)
        .ok()?
        .get("viewerPermission")?
        .as_str()
        .map(str::to_string)
}

fn workspace_git_pr_list_action() -> WorkspaceGitActionResponse {
    let fields = "number,title,url,state,isDraft,author,repository,updatedAt,commentsCount,body";
    let mut pull_requests = HashMap::<String, Value>::new();
    let mut errors = Vec::new();
    let mut successes = 0;

    for (flag, filter) in [
        ("authored", ["--author", "@me"]),
        ("reviewing", ["--review-requested", "@me"]),
    ] {
        let args = vec![
            "search".to_string(),
            "prs".to_string(),
            filter[0].to_string(),
            filter[1].to_string(),
            "--state".to_string(),
            "open".to_string(),
            "--sort".to_string(),
            "updated".to_string(),
            "--order".to_string(),
            "desc".to_string(),
            "--limit".to_string(),
            "100".to_string(),
            "--json".to_string(),
            fields.to_string(),
        ];
        let output = match gh_output_global(&args) {
            Ok(output) => output,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        if !output.status.success() {
            errors.push(output_error_text(&output));
            continue;
        }
        let items = match serde_json::from_slice::<Vec<Value>>(&output.stdout) {
            Ok(items) => items,
            Err(error) => {
                errors.push(format!("GitHub CLI returned invalid PR data: {error}"));
                continue;
            }
        };
        successes += 1;
        for mut item in items {
            let repository = item
                .get("repository")
                .and_then(|value| value.get("nameWithOwner"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let key = item
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if key.is_empty() || repository.is_empty() {
                continue;
            }
            if let Some(existing) = pull_requests.get_mut(&key) {
                existing[flag] = Value::Bool(true);
                continue;
            }
            item["repository"] = Value::String(repository);
            item["authored"] = Value::Bool(flag == "authored");
            item["reviewing"] = Value::Bool(flag == "reviewing");
            pull_requests.insert(key, item);
        }
    }

    if successes == 0 {
        return workspace_git_action_message("pr_list", "gh search prs", false, &errors.join("\n"));
    }

    let mut pull_requests = pull_requests.into_values().collect::<Vec<_>>();
    pull_requests.sort_by(|left, right| {
        right
            .get("updatedAt")
            .and_then(Value::as_str)
            .cmp(&left.get("updatedAt").and_then(Value::as_str))
    });
    let mut response = workspace_git_combined_response(
        "pr_list",
        "gh search prs",
        true,
        Value::Array(pull_requests).to_string(),
        errors.join("\n"),
        Some(0),
        if errors.is_empty() {
            "Pull requests loaded.".to_string()
        } else {
            "Pull requests loaded with one unavailable filter.".to_string()
        },
    );
    response.truncated = false;
    response
}

fn workspace_git_pr_view_action(
    repository: Option<String>,
    pull_request: Option<u64>,
) -> WorkspaceGitActionResponse {
    let repository = match github_repository(repository) {
        Ok(repository) => repository,
        Err(message) => {
            return workspace_git_action_message("pr_view", "gh pr view", false, &message)
        }
    };
    let number = match pull_request.filter(|number| *number > 0) {
        Some(number) => number,
        None => {
            return workspace_git_action_message(
                "pr_view",
                "gh pr view",
                false,
                "Pull request number is required.",
            )
        }
    };
    let fields = "number,title,url,state,isDraft,baseRefName,headRefName,headRefOid,body,author,reviewRequests,latestReviews,reviewDecision,mergeStateStatus,mergeable,comments,additions,deletions,changedFiles,updatedAt,files";
    let args = vec![
        "pr".to_string(),
        "view".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repository.clone(),
        "--json".to_string(),
        fields.to_string(),
    ];
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_global(&args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message("pr_view", &command, false, &error),
    };
    if !output.status.success() {
        return workspace_git_combined_response(
            "pr_view",
            &command,
            false,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            output_error_text(&output),
        );
    }
    let mut pull_request = match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(pull_request) => pull_request,
        Err(error) => {
            return workspace_git_action_message(
                "pr_view",
                &command,
                false,
                &format!("GitHub CLI returned invalid PR data: {error}"),
            )
        }
    };
    let check_args = vec![
        "pr".to_string(),
        "checks".to_string(),
        number.to_string(),
        "--repo".to_string(),
        repository.clone(),
        "--json".to_string(),
        "bucket,completedAt,description,event,link,name,startedAt,state,workflow".to_string(),
    ];
    match gh_output_global(&check_args) {
        Ok(checks) if checks.status.success() || checks.status.code() == Some(8) => {
            if let Ok(value) = serde_json::from_slice::<Value>(&checks.stdout) {
                pull_request["checks"] = value;
            }
        }
        Ok(checks) => {
            pull_request["checksError"] = Value::String(output_error_text(&checks));
        }
        Err(error) => {
            pull_request["checksError"] = Value::String(error);
        }
    }
    pull_request["exists"] = Value::Bool(true);
    pull_request["repository"] = Value::String(repository.clone());
    let permission_args = vec![
        "repo".to_string(),
        "view".to_string(),
        repository,
        "--json".to_string(),
        "viewerPermission".to_string(),
    ];
    if let Some(permission) = github_viewer_permission(gh_output_global(&permission_args)) {
        pull_request["viewerPermission"] = Value::String(permission);
    }
    let mut response = workspace_git_combined_response(
        "pr_view",
        &command,
        true,
        pull_request.to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Pull request found.".to_string(),
    );
    response.pull_request = Some(pull_request);
    response
}

fn workspace_git_pr_prerequisite(
    root: &FsPath,
    status: &WorkspaceGitStatus,
) -> Result<String, String> {
    let remote = git_text(root, &["remote", "get-url", "origin"])
        .ok_or_else(|| "A GitHub origin remote is required.".to_string())?;
    if !remote.to_ascii_lowercase().contains("github.com") {
        return Err("PR creation currently requires a GitHub origin remote.".to_string());
    }
    let branch = status
        .branch
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "A named branch is required.".to_string())?;
    if status.upstream.is_none() {
        return Err("Publish the branch before creating a PR.".to_string());
    }
    let auth = gh_output(root, &["auth", "status"])?;
    if !auth.status.success() {
        return Err("GitHub CLI is not authenticated. Run `gh auth login`.".to_string());
    }
    Ok(branch)
}

fn workspace_git_pr_status_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
) -> WorkspaceGitActionResponse {
    let branch = match workspace_git_pr_prerequisite(root, status) {
        Ok(branch) => branch,
        Err(message) => {
            return workspace_git_action_message(
                "pr_status",
                "gh pr list --head <branch>",
                false,
                &message,
            )
        }
    };
    let fields = "number,title,url,state,isDraft,baseRefName,headRefName,headRefOid,body,author,reviewRequests,latestReviews,reviewDecision,mergeStateStatus,mergeable,comments,additions,deletions,changedFiles,updatedAt";
    let args = vec![
        "pr".to_string(),
        "list".to_string(),
        "--head".to_string(),
        branch,
        "--state".to_string(),
        "all".to_string(),
        "--limit".to_string(),
        "10".to_string(),
        "--json".to_string(),
        fields.to_string(),
    ];
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_owned(root, &args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message("pr_status", &command, false, &error),
    };
    if !output.status.success() {
        return workspace_git_combined_response(
            "pr_status",
            &command,
            false,
            String::from_utf8_lossy(&output.stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
            output.status.code(),
            output_error_text(&output),
        );
    }

    let candidates = match serde_json::from_slice::<Vec<Value>>(&output.stdout) {
        Ok(candidates) => candidates,
        Err(error) => {
            return workspace_git_action_message(
                "pr_status",
                &command,
                false,
                &format!("GitHub CLI returned invalid PR data: {error}"),
            )
        }
    };
    let Some(mut pull_request) = select_pull_request_for_head(candidates, status.head.as_deref())
    else {
        let base = git_text(
            root,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .and_then(|value| value.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".to_string());
        let title = git_text(root, &["log", "-1", "--pretty=%s"]).unwrap_or_default();
        let mut response = workspace_git_combined_response(
            "pr_status",
            &command,
            true,
            json!({ "exists": false, "baseRefName": base, "title": title }).to_string(),
            String::new(),
            output.status.code(),
            "No pull request exists for this branch.".to_string(),
        );
        response.pull_request = None;
        return response;
    };

    if let Some(number) = pull_request.get("number").and_then(Value::as_u64) {
        let check_args = vec![
            "pr".to_string(),
            "checks".to_string(),
            number.to_string(),
            "--json".to_string(),
            "bucket,completedAt,description,event,link,name,startedAt,state,workflow".to_string(),
        ];
        match gh_output_owned(root, &check_args) {
            Ok(checks) if checks.status.success() || checks.status.code() == Some(8) => {
                if let Ok(value) = serde_json::from_slice::<Value>(&checks.stdout) {
                    pull_request["checks"] = value;
                }
            }
            Ok(checks) => {
                pull_request["checksError"] = Value::String(output_error_text(&checks));
            }
            Err(error) => {
                pull_request["checksError"] = Value::String(error);
            }
        }
    }
    pull_request["exists"] = Value::Bool(true);
    if let Some(permission) = github_viewer_permission(gh_output(
        root,
        &["repo", "view", "--json", "viewerPermission"],
    )) {
        pull_request["viewerPermission"] = Value::String(permission);
    }
    let mut response = workspace_git_combined_response(
        "pr_status",
        &command,
        true,
        pull_request.to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        "Pull request found.".to_string(),
    );
    response.pull_request = Some(pull_request);
    response
}

fn select_pull_request_for_head(candidates: Vec<Value>, head: Option<&str>) -> Option<Value> {
    let is_open = |candidate: &&Value| {
        candidate
            .get("state")
            .and_then(Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("open"))
    };
    let matches_head = |candidate: &&Value| {
        head.is_some_and(|head| {
            candidate
                .get("headRefOid")
                .and_then(Value::as_str)
                .is_some_and(|candidate_head| candidate_head == head)
        })
    };
    candidates
        .iter()
        .find(|candidate| is_open(candidate) && matches_head(candidate))
        .or_else(|| candidates.iter().find(is_open))
        .or_else(|| candidates.iter().find(matches_head))
        .or_else(|| candidates.first())
        .cloned()
}

fn workspace_git_pr_action_args(
    action: &str,
    pull_request: Option<u64>,
    body: Option<&str>,
    review_action: Option<&str>,
    merge_method: Option<&str>,
    expected_head: Option<&str>,
) -> Result<Vec<String>, String> {
    let number = pull_request
        .filter(|number| *number > 0)
        .ok_or_else(|| "Pull request number is required.".to_string())?
        .to_string();
    let mut args = vec!["pr".to_string()];
    match action {
        "pr_ready" => args.extend(["ready".to_string(), number]),
        "pr_comment" => {
            let body = body
                .map(str::trim)
                .filter(|body| !body.is_empty())
                .ok_or_else(|| "Comment body is required.".to_string())?;
            args.extend([
                "comment".to_string(),
                number,
                "--body".to_string(),
                body.to_string(),
            ]);
        }
        "pr_review" => {
            let review_action = review_action
                .map(str::trim)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let flag = match review_action.as_str() {
                "approve" => "--approve",
                "request_changes" => "--request-changes",
                "comment" => "--comment",
                _ => return Err("Choose approve, request changes, or comment.".to_string()),
            };
            let body = body.map(str::trim).unwrap_or_default();
            if review_action != "approve" && body.is_empty() {
                return Err("Review body is required.".to_string());
            }
            args.extend(["review".to_string(), number, flag.to_string()]);
            if !body.is_empty() {
                args.extend(["--body".to_string(), body.to_string()]);
            }
        }
        "pr_merge" => {
            let method = match merge_method.map(str::trim) {
                Some("merge") => "--merge",
                Some("squash") => "--squash",
                Some("rebase") => "--rebase",
                _ => return Err("Choose merge, squash, or rebase.".to_string()),
            };
            let expected_head = expected_head
                .map(str::trim)
                .filter(|head| !head.is_empty())
                .ok_or_else(|| "Current pull request head is required.".to_string())?;
            args.extend([
                "merge".to_string(),
                number,
                method.to_string(),
                "--match-head-commit".to_string(),
                expected_head.to_string(),
            ]);
        }
        _ => return Err("Unsupported pull request action.".to_string()),
    }
    Ok(args)
}

#[allow(clippy::too_many_arguments)]
fn workspace_git_pr_mutation_action_global(
    repository: Option<String>,
    action: &str,
    pull_request: Option<u64>,
    body: Option<String>,
    review_action: Option<String>,
    merge_method: Option<String>,
    expected_head: Option<String>,
) -> WorkspaceGitActionResponse {
    let repository = match github_repository(repository) {
        Ok(repository) => repository,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    let mut args = match workspace_git_pr_action_args(
        action,
        pull_request,
        body.as_deref(),
        review_action.as_deref(),
        merge_method.as_deref(),
        expected_head.as_deref(),
    ) {
        Ok(args) => args,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    args.extend(["--repo".to_string(), repository]);
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_global(&args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message(action, &command, false, &error),
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        action,
        &command,
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            match action {
                "pr_ready" => "Pull request marked ready for review.",
                "pr_comment" => "Comment added.",
                "pr_review" => "Review submitted.",
                "pr_merge" => "Pull request merged.",
                _ => "Pull request updated.",
            }
            .to_string()
        } else {
            output_error_text(&output)
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn workspace_git_pr_mutation_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    action: &str,
    pull_request: Option<u64>,
    body: Option<String>,
    review_action: Option<String>,
    merge_method: Option<String>,
    expected_head: Option<String>,
) -> WorkspaceGitActionResponse {
    if let Err(message) = workspace_git_pr_prerequisite(root, status) {
        return workspace_git_action_message(action, "gh pr", false, &message);
    }
    let args = match workspace_git_pr_action_args(
        action,
        pull_request,
        body.as_deref(),
        review_action.as_deref(),
        merge_method.as_deref(),
        expected_head.as_deref(),
    ) {
        Ok(args) => args,
        Err(message) => return workspace_git_action_message(action, "gh pr", false, &message),
    };
    let command = format!("gh {}", args.join(" "));
    let output = match gh_output_owned(root, &args) {
        Ok(output) => output,
        Err(error) => return workspace_git_action_message(action, &command, false, &error),
    };
    let ok = output.status.success();
    workspace_git_combined_response(
        action,
        &command,
        ok,
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if ok {
            match action {
                "pr_ready" => "Pull request marked ready for review.",
                "pr_comment" => "Comment added.",
                "pr_review" => "Review submitted.",
                "pr_merge" => "Pull request merged.",
                _ => "Pull request updated.",
            }
            .to_string()
        } else {
            output_error_text(&output)
        },
    )
}

fn workspace_git_pr_create_action(
    root: &FsPath,
    status: &WorkspaceGitStatus,
    title: Option<String>,
    body: Option<String>,
    base: Option<String>,
    draft: bool,
) -> WorkspaceGitActionResponse {
    let branch = match workspace_git_pr_prerequisite(root, status) {
        Ok(value) => value,
        Err(message) => {
            return workspace_git_action_message("pr_create", "gh pr create", false, &message)
        }
    };
    let title = title.unwrap_or_default().trim().to_string();
    let body = body.unwrap_or_default();
    let base = base.unwrap_or_default().trim().to_string();
    let base = if base.is_empty() {
        git_text(
            root,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .and_then(|value| value.strip_prefix("origin/").map(str::to_string))
        .unwrap_or_else(|| "main".to_string())
    } else {
        base
    };
    if title.is_empty() {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "PR title is required.",
        );
    }
    if branch == base {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "Create the PR from a non-default branch.",
        );
    }
    let ahead = git_text(
        root,
        &["rev-list", "--count", &format!("origin/{base}..HEAD")],
    )
    .and_then(|value| value.parse::<u64>().ok())
    .unwrap_or(0);
    if ahead == 0 {
        return workspace_git_action_message(
            "pr_create",
            "gh pr create",
            false,
            "Commit changes on this branch before creating a PR.",
        );
    }
    let mut args = vec![
        "pr",
        "create",
        "--title",
        title.as_str(),
        "--body",
        body.as_str(),
        "--base",
        base.as_str(),
        "--head",
        branch.as_str(),
    ];
    if draft {
        args.push("--draft");
    }
    let output = match gh_output(root, &args) {
        Ok(output) => output,
        Err(error) => {
            return workspace_git_action_message("pr_create", "gh pr create", false, &error)
        }
    };
    workspace_git_combined_response(
        "pr_create",
        "gh pr create",
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code(),
        if output.status.success() {
            "Draft pull request created.".to_string()
        } else {
            output_error_text(&output)
        },
    )
}

#[cfg(test)]
mod git_control_plane_tests {
    use super::*;

    #[test]
    fn pull_request_actions_validate_review_and_guard_merge_head() {
        assert_eq!(
            github_repository(Some("openai/codex".to_string())).as_deref(),
            Ok("openai/codex")
        );
        assert!(github_repository(Some("../codex".to_string())).is_err());
        assert!(
            workspace_git_pr_action_args("pr_comment", Some(12), Some(" "), None, None, None,)
                .is_err()
        );
        assert!(workspace_git_pr_action_args(
            "pr_review",
            Some(12),
            None,
            Some("request_changes"),
            None,
            None,
        )
        .is_err());

        let args = workspace_git_pr_action_args(
            "pr_merge",
            Some(12),
            None,
            None,
            Some("squash"),
            Some("abc123"),
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "pr",
                "merge",
                "12",
                "--squash",
                "--match-head-commit",
                "abc123"
            ]
        );
        assert!(!args.iter().any(|arg| arg == "--admin"));
    }

    #[test]
    fn pull_request_selection_prefers_open_matching_head() {
        let selected = select_pull_request_for_head(
            vec![
                json!({ "number": 1, "state": "MERGED", "headRefOid": "abc" }),
                json!({ "number": 2, "state": "OPEN", "headRefOid": "def" }),
                json!({ "number": 3, "state": "OPEN", "headRefOid": "abc" }),
            ],
            Some("abc"),
        )
        .unwrap();
        assert_eq!(selected["number"], 3);
    }

    #[test]
    fn thread_worktree_lifecycle_keeps_branch_and_guards_dirty_state() {
        let base = std::env::temp_dir().join(format!("milim-thread-worktree-{}", gen_id("test")));
        let repo = base.join("repo");
        let runtime = base.join("runtime");
        std::fs::create_dir_all(&repo).unwrap();
        assert!(git_output(&repo, &["init"]).unwrap().status.success());
        std::fs::write(repo.join("README.md"), "hello").unwrap();
        assert!(git_output(&repo, &["add", "README.md"])
            .unwrap()
            .status
            .success());
        assert!(git_output(
            &repo,
            &[
                "-c",
                "user.name=Milim Test",
                "-c",
                "user.email=milim@example.invalid",
                "commit",
                "-m",
                "initial",
            ]
        )
        .unwrap()
        .status
        .success());
        let status = workspace_git_status_blocking(Some(repo.clone()));
        let created = workspace_git_create_thread_worktree_action(
            &repo,
            &status,
            Some("thread_12345678".into()),
            &runtime,
        );
        assert!(created.ok, "{}", created.message);
        let worktree = PathBuf::from(created.worktree.unwrap());
        assert!(worktree.is_dir());
        std::fs::write(worktree.join("dirty.txt"), "dirty").unwrap();
        let blocked = workspace_git_remove_thread_worktree_action(
            &repo,
            Some("thread_12345678".into()),
            false,
            &runtime,
        );
        assert!(!blocked.ok);
        assert!(blocked.message.contains("uncommitted changes"));
        let removed = workspace_git_remove_thread_worktree_action(
            &repo,
            Some("thread_12345678".into()),
            true,
            &runtime,
        );
        assert!(removed.ok, "{}", removed.message);
        assert!(
            git_text(&repo, &["branch", "--list", "milim/thread-thread_1"])
                .unwrap_or_default()
                .contains("milim/thread-thread_1")
        );
        std::fs::remove_dir_all(base).ok();
    }

    #[test]
    fn thread_runtime_ids_reject_paths() {
        assert!(valid_thread_runtime_id(Some("../../outside".into())).is_err());
        assert_eq!(
            valid_thread_runtime_id(Some("abc_DEF-123".into())).unwrap(),
            "abc_DEF-123"
        );
    }
}

fn append_git_output(stdout: &mut String, stderr: &mut String, output: &Output) {
    stdout.push_str(&String::from_utf8_lossy(&output.stdout));
    stderr.push_str(&String::from_utf8_lossy(&output.stderr));
}

fn git_command_text(args: &[&str]) -> String {
    let rendered = args
        .iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                format!("{arg:?}")
            } else {
                (*arg).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {rendered}")
}

fn workspace_git_combined_response(
    action: &str,
    command: &str,
    ok: bool,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    message: String,
) -> WorkspaceGitActionResponse {
    const OUTPUT_LIMIT: usize = 24_000;
    let (stdout, stdout_truncated) = truncate_git_action_output(stdout, OUTPUT_LIMIT);
    let (stderr, stderr_truncated) = truncate_git_action_output(stderr, OUTPUT_LIMIT);
    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command: command.to_string(),
        stdout,
        stderr,
        exit_code,
        message,
        truncated: stdout_truncated || stderr_truncated,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn workspace_git_diff_action(
    root: &FsPath,
    action: &str,
    has_head: bool,
    scope: &str,
    base: Option<&str>,
) -> WorkspaceGitActionResponse {
    let mut temp_index = None;
    let mut index_env = Vec::new();
    let args: Vec<String> = match scope {
        "all" if has_head => ["diff", "--no-ext-diff", "--patch", "HEAD", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "all" | "unstaged" => ["diff", "--no-ext-diff", "--patch", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "staged" => ["diff", "--cached", "--no-ext-diff", "--patch", "--"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        "last_turn" => {
            let checkpoint =
                if let Some(candidate) = base.map(str::trim).filter(|value| !value.is_empty()) {
                    if !candidate.starts_with("refs/milim/checkpoints/")
                        || git_text(root, &["show-ref", "--verify", candidate]).is_none()
                    {
                        return workspace_git_action_message(
                            action,
                            "git diff <last-turn-checkpoint> --",
                            false,
                            "Select a valid Milim turn checkpoint.",
                        );
                    }
                    candidate.to_string()
                } else {
                    let Some(checkpoint) = git_text(
                        root,
                        &[
                            "for-each-ref",
                            "--sort=-refname",
                            "--sort=-creatordate",
                            "--count=1",
                            "--format=%(refname)",
                            "refs/milim/checkpoints/",
                        ],
                    ) else {
                        return workspace_git_action_message(
                            action,
                            "git diff <last-turn-checkpoint> --",
                            false,
                            "No turn checkpoint is available.",
                        );
                    };
                    checkpoint
                };
            let Some(index_path) = git_text(root, &["rev-parse", "--git-path", "index"]) else {
                return workspace_git_action_message(
                    action,
                    "git rev-parse --git-path index",
                    false,
                    "Failed to locate the Git index.",
                );
            };
            let index_path = PathBuf::from(index_path);
            let index_path = if index_path.is_absolute() {
                index_path
            } else {
                root.join(index_path)
            };
            let path = index_path.with_file_name(format!("milim-{}.index", gen_id("diff")));
            index_env.push(("GIT_INDEX_FILE", path.to_string_lossy().to_string()));
            let read_tree =
                git_output_with_env(root, &["read-tree", checkpoint.as_str()], &index_env);
            if !matches!(read_tree, Ok(ref output) if output.status.success()) {
                let _ = std::fs::remove_file(&path);
                return workspace_git_action_message(
                    action,
                    "git read-tree <last-turn-checkpoint>",
                    false,
                    "Failed to read the last turn checkpoint.",
                );
            }
            temp_index = Some(path);
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--patch".into(),
                checkpoint,
                "--".into(),
            ]
        }
        "commit" => {
            let candidate = base.unwrap_or_default().trim();
            if candidate.len() < 7 || !candidate.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return workspace_git_action_message(
                    action,
                    "git show <commit>",
                    false,
                    "Select a valid commit.",
                );
            }
            let verify = format!("{candidate}^{{commit}}");
            let Some(commit) = git_text(root, &["rev-parse", "--verify", verify.as_str()]) else {
                return workspace_git_action_message(
                    action,
                    "git show <commit>",
                    false,
                    "Commit not found.",
                );
            };
            vec![
                "show".into(),
                "--no-ext-diff".into(),
                "--format=".into(),
                "--patch".into(),
                commit,
                "--".into(),
            ]
        }
        "branch" => {
            let branch = base.unwrap_or_default().trim();
            if branch.is_empty() || branch.starts_with('-') {
                return workspace_git_action_message(
                    action,
                    "git diff <branch>...HEAD --",
                    false,
                    "Select a valid branch.",
                );
            }
            let reference = format!("refs/heads/{branch}");
            let verify = format!("{reference}^{{commit}}");
            if git_text(root, &["rev-parse", "--verify", verify.as_str()]).is_none() {
                return workspace_git_action_message(
                    action,
                    "git diff <branch>...HEAD --",
                    false,
                    "Branch not found.",
                );
            }
            vec![
                "diff".into(),
                "--no-ext-diff".into(),
                "--patch".into(),
                format!("{reference}...HEAD"),
                "--".into(),
            ]
        }
        _ => {
            return workspace_git_action_message(
                action,
                "git diff",
                false,
                "Unsupported diff scope.",
            )
        }
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let command = format!("git {}", args.join(" "));
    let output = match if index_env.is_empty() {
        git_output(root, &arg_refs)
    } else {
        git_output_with_env(root, &arg_refs, &index_env)
    } {
        Ok(output) => output,
        Err(e) => {
            if let Some(path) = temp_index.as_ref() {
                let _ = std::fs::remove_file(path);
            }
            return workspace_git_action_message(action, &command, false, &e);
        }
    };
    if let Some(path) = temp_index.as_ref() {
        let _ = std::fs::remove_file(path);
    }

    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() && matches!(scope, "all" | "unstaged" | "last_turn") {
        let excluded = if scope == "last_turn" {
            base_paths_from_diff_args(root, &args)
        } else {
            HashSet::new()
        };
        stdout.push_str(&untracked_git_diff(root, &excluded));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = output.status.success();
    let message = if ok {
        if stdout.trim().is_empty() {
            "No diff to show.".to_string()
        } else {
            "Diff ready.".to_string()
        }
    } else {
        output_error_text(&output)
    };

    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command,
        stdout,
        stderr,
        exit_code: output.status.code(),
        message,
        truncated: false,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn base_paths_from_diff_args(root: &FsPath, args: &[String]) -> HashSet<String> {
    let Some(base) = args.get(3) else {
        return HashSet::new();
    };
    let Ok(output) = git_output(root, &["ls-tree", "-r", "--name-only", "-z", base]) else {
        return HashSet::new();
    };
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect()
}

fn untracked_git_diff(root: &FsPath, excluded: &HashSet<String>) -> String {
    let Ok(output) = git_output(root, &["ls-files", "--others", "--exclude-standard", "-z"]) else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }

    let mut patch = String::new();
    for raw in output
        .stdout
        .split(|b| *b == 0)
        .filter(|item| !item.is_empty())
    {
        let path = String::from_utf8_lossy(raw).replace('\\', "/");
        if excluded.contains(&path) {
            continue;
        }
        let Some(file_patch) = untracked_file_patch(root, &path) else {
            continue;
        };
        if !patch.ends_with('\n') && !patch.is_empty() {
            patch.push('\n');
        }
        patch.push_str(&file_patch);
    }
    patch
}

fn untracked_file_patch(root: &FsPath, path: &str) -> Option<String> {
    let full_path = root.join(path);
    if !full_path.is_file() {
        return None;
    }
    let bytes = std::fs::read(full_path).ok()?;
    let mut patch = format!(
        "\ndiff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    match String::from_utf8(bytes) {
        Ok(text) => {
            let line_count = text.lines().count().max(1);
            patch.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
            if text.is_empty() {
                return Some(patch);
            }
            for line in text.split_inclusive('\n') {
                patch.push('+');
                patch.push_str(line);
                if !line.ends_with('\n') {
                    patch.push('\n');
                }
            }
        }
        Err(_) => {
            patch.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        }
    }
    Some(patch)
}

fn workspace_git_action_message(
    action: &str,
    command: &str,
    ok: bool,
    message: &str,
) -> WorkspaceGitActionResponse {
    WorkspaceGitActionResponse {
        ok,
        action: action.to_string(),
        command: command.to_string(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        message: message.to_string(),
        truncated: false,
        checkpoint: None,
        root: None,
        head: None,
        worktree: None,
        undo_checkpoint: None,
        conflicts: None,
        pull_request: None,
    }
}

fn truncate_git_action_output(text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}...\n[truncated]", &text[..end]), true)
}

fn git_state_label_for_action(status: &WorkspaceGitStatus) -> String {
    status
        .message
        .clone()
        .unwrap_or_else(|| "No Git repository is available.".to_string())
}

// ----- Computer use (screen capture + mouse/keyboard gate) -----

#[derive(Deserialize)]
pub(crate) struct ComputerSet {
    enabled: bool,
}

/// `GET /computer` — whether the computer-use layer is currently enabled.
pub(crate) async fn computer_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let enabled = st.computer_use.load(std::sync::atomic::Ordering::Relaxed);
    Ok(Json(json!({ "enabled": enabled })).into_response())
}

/// `POST /computer` — enable/disable the computer-use layer (mouse/keyboard).
pub(crate) async fn computer_set(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ComputerSet>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    st.computer_use
        .store(req.enabled, std::sync::atomic::Ordering::Relaxed);
    Ok(Json(json!({ "enabled": req.enabled })).into_response())
}

// ----- Providers -----

fn default_enabled() -> bool {
    true
}

#[derive(Deserialize)]
pub(crate) struct ProviderUpsert {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    kind: crate::providers::ProviderKind,
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderDiscovery {
    name: &'static str,
    kind: crate::providers::ProviderKind,
    base_url: &'static str,
    configured: bool,
    provider_id: Option<String>,
    reachable: bool,
    models: Vec<String>,
    error: Option<String>,
}

const LOCAL_PROVIDER_CANDIDATES: &[(&str, &str)] = &[
    ("Ollama (local)", "http://localhost:11434/v1"),
    ("LM Studio (local)", "http://localhost:1234/v1"),
];
const LOCAL_PROVIDER_PROBE_TIMEOUT: Duration = Duration::from_millis(900);

/// `GET /providers` — list configured providers (keys redacted to `has_key`).
pub(crate) async fn providers_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let list = match &st.providers {
        Some(r) => r.list().await,
        None => Vec::new(),
    };
    let safe: Vec<Value> = list
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id, "name": p.name, "kind": p.kind, "base_url": p.base_url,
                "enabled": p.enabled, "has_key": p.api_key.is_some(), "models": p.models,
                "pricing": p.pricing,
                "error": p.last_error,
            })
        })
        .collect();
    Ok(Json(json!({ "providers": safe })).into_response())
}

/// `GET /providers/discover` — probe well-known local OpenAI-compatible
/// endpoints and report whether they are reachable/configured.
pub(crate) async fn providers_discover(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let configured = match &st.providers {
        Some(reg) => reg.list().await,
        None => Vec::new(),
    };
    let out = join_all(LOCAL_PROVIDER_CANDIDATES.iter().map(|(name, base_url)| {
        let existing = configured.iter().find(|p| {
            p.kind == crate::providers::ProviderKind::OpenAiCompatible
                && p.base_url.trim_end_matches('/') == base_url.trim_end_matches('/')
        });
        let configured = existing.is_some();
        let provider_id = existing.map(|p| p.id.clone());
        async move {
            let backend = RemoteBackend::new(*name, *base_url, None);
            let (models, error) =
                match tokio::time::timeout(LOCAL_PROVIDER_PROBE_TIMEOUT, backend.list_models())
                    .await
                {
                    Ok(Ok(models)) => (models.into_iter().map(|m| m.id).collect(), None),
                    Ok(Err(err)) => (Vec::new(), Some(err.to_string())),
                    Err(_) => (
                        Vec::new(),
                        Some("local provider probe timed out".to_string()),
                    ),
                };
            ProviderDiscovery {
                name,
                kind: crate::providers::ProviderKind::OpenAiCompatible,
                base_url,
                configured,
                provider_id,
                reachable: error.is_none(),
                models,
                error,
            }
        }
    }))
    .await;

    Ok(Json(json!({ "providers": out })).into_response())
}

/// `POST /providers` — create (no id) or update (with id) a provider.
pub(crate) async fn provider_upsert(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ProviderUpsert>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let reg = st.providers.as_ref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "providers are not enabled".to_string(),
        ))
    })?;
    let cfg = crate::providers::Provider {
        id: req.id.unwrap_or_else(|| gen_id("prov")),
        name: req.name,
        kind: req.kind,
        base_url: req.base_url,
        api_key: req.api_key,
        enabled: req.enabled,
        models: Vec::new(),
        pricing: BTreeMap::new(),
        model_context: BTreeMap::new(),
        model_reasoning: BTreeMap::new(),
        model_capabilities: BTreeMap::new(),
        last_error: None,
    };
    let saved = reg.upsert(cfg).await.map_err(ApiError)?;
    Ok(Json(json!({
        "id": saved.id, "name": saved.name, "kind": saved.kind, "base_url": saved.base_url,
        "enabled": saved.enabled, "has_key": saved.api_key.is_some(), "models": saved.models,
        "pricing": saved.pricing,
        "error": saved.last_error,
    }))
    .into_response())
}

/// `DELETE /providers/{id}` — remove a provider.
pub(crate) async fn provider_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let reg = st.providers.as_ref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "providers are not enabled".to_string(),
        ))
    })?;
    Ok(Json(json!({ "deleted": reg.delete(&id).await.map_err(ApiError)? })).into_response())
}

// ----- Media generation -----

fn default_media_kind() -> String {
    "image".to_string()
}

#[derive(Deserialize)]
pub(crate) struct MediaGenerateRequest {
    #[serde(default)]
    provider_id: Option<String>,
    #[serde(default)]
    provider_kind: Option<crate::providers::ProviderKind>,
    model: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    prompt: String,
    #[serde(default)]
    input: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
pub(crate) struct MediaModelsQuery {
    provider_id: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
pub(crate) struct MediaModelSchemaQuery {
    provider_id: String,
    model: String,
    #[serde(default = "default_media_kind")]
    kind: String,
    #[serde(default)]
    refresh: bool,
}

#[derive(Deserialize)]
pub(crate) struct MediaStatusQuery {
    provider_id: String,
    id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    response_url: Option<String>,
    #[serde(default)]
    status_url: Option<String>,
    #[serde(default = "default_media_kind")]
    kind: String,
}

#[derive(Deserialize)]
pub(crate) struct MediaContentQuery {
    provider_id: String,
    id: String,
    #[serde(default)]
    index: usize,
}

#[derive(Default, Deserialize)]
pub(crate) struct MediaLibraryQuery {
    #[serde(default, alias = "q")]
    query: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "provider_id")]
    provider: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MediaItem {
    url: String,
    kind: String,
    mime: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    requires_auth: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Serialize)]
pub(crate) struct MediaPrivacyInfo {
    mode: &'static str,
    redacted: bool,
    detections: usize,
    kinds: String,
}

#[derive(Debug, Serialize)]
struct MediaModelInfo {
    id: String,
    name: String,
    description: String,
    output_modalities: Vec<String>,
    supported_parameters: Vec<String>,
    default_parameters: Value,
    pricing: Value,
}

#[derive(Debug, Serialize)]
struct MediaControlOption {
    label: String,
    value: Value,
}

#[derive(Debug, Serialize)]
struct MediaSchemaControl {
    key: String,
    label: String,
    kind: String,
    path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<Vec<MediaControlOption>>,
    #[serde(rename = "default", skip_serializing_if = "Option::is_none")]
    default_value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    placeholder: Option<String>,
}

struct MediaCacheEntry {
    created_at: Instant,
    value: Value,
}

const MEDIA_METADATA_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
static MEDIA_METADATA_CACHE: OnceLock<Mutex<HashMap<String, MediaCacheEntry>>> = OnceLock::new();

fn media_metadata_cache() -> &'static Mutex<HashMap<String, MediaCacheEntry>> {
    MEDIA_METADATA_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_media_cache(key: &str) -> Option<Value> {
    let cache = media_metadata_cache().lock().ok()?;
    let entry = cache.get(key)?;
    if entry.created_at.elapsed() <= MEDIA_METADATA_CACHE_TTL {
        Some(entry.value.clone())
    } else {
        None
    }
}

fn write_media_cache(key: String, value: &Value) {
    if let Ok(mut cache) = media_metadata_cache().lock() {
        cache.insert(
            key,
            MediaCacheEntry {
                created_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
}

fn media_cache_response(mut value: Value, cached: bool) -> Value {
    if let Value::Object(map) = &mut value {
        map.insert("cached".to_string(), Value::Bool(cached));
        map.insert(
            "cache_ttl_seconds".to_string(),
            json!(MEDIA_METADATA_CACHE_TTL.as_secs()),
        );
    }
    value
}

fn media_models_cache_key(
    provider: &crate::providers::Provider,
    kind: &str,
    query: &str,
) -> String {
    format!(
        "models:{}:{:?}:{}:{}",
        provider.id,
        provider.kind,
        provider.base_url.trim_end_matches('/'),
        kind
    ) + ":"
        + query
}

fn media_schema_cache_key(
    provider: &crate::providers::Provider,
    model: &str,
    kind: &str,
) -> String {
    format!(
        "schema:{}:{}:{}:{}",
        provider.id,
        provider.base_url.trim_end_matches('/'),
        kind,
        model
    )
}

fn validate_media_kind(kind: &str) -> Result<&str, ApiError> {
    match kind.trim() {
        "image" => Ok("image"),
        "video" => Ok("video"),
        "music" => Ok("music"),
        other => Err(ApiError(Error::InvalidRequest(format!(
            "unsupported media kind '{other}'; expected image, video, or music"
        )))),
    }
}

/// `GET /media/models` - list provider models that can produce the requested
/// media kind.
pub(crate) async fn media_models(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaModelsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    let kind = validate_media_kind(&query.kind)?;
    let model_query = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(kind);
    let cache_key = media_models_cache_key(&provider, kind, model_query);
    if !query.refresh {
        if let Some(cached) = read_media_cache(&cache_key) {
            return Ok(Json(media_cache_response(cached, true)).into_response());
        }
    }
    let models = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            let upstream =
                call_replicate_media_models(&provider.base_url, &key, model_query).await?;
            media_models_from_replicate(&upstream, kind)
        }
        crate::providers::ProviderKind::Fal => {
            let upstream =
                call_fal_media_models(&provider.base_url, &key, kind, model_query).await?;
            media_models_from_fal(&upstream, kind)
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            let upstream = call_openrouter_media_models(&provider.base_url, &key, kind).await?;
            filter_media_models(media_models_from_openrouter(&upstream, kind), model_query)
        }
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "selected provider does not expose media model metadata".to_string(),
            )))
        }
    };
    let response = json!({ "models": models });
    write_media_cache(cache_key, &response);
    Ok(Json(media_cache_response(response, false)).into_response())
}

/// `GET /media/model-schema` - return normalized UI controls for a selected
/// media model.
pub(crate) async fn media_model_schema(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaModelSchemaQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let model = query.model.trim();
    if model.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media model is required".to_string(),
        )));
    }
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let kind = validate_media_kind(&query.kind)?;
    let key = media_provider_key(&provider)?;
    let cache_key = media_schema_cache_key(&provider, model, kind);
    if !query.refresh {
        if let Some(cached) = read_media_cache(&cache_key) {
            return Ok(Json(media_cache_response(cached, true)).into_response());
        }
    }
    let (supported, controls, upstream) = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            let upstream = call_replicate_model_schema(&provider.base_url, &key, model).await?;
            let (supported, controls) = replicate_schema_controls(&upstream)?;
            (supported, controls, upstream)
        }
        crate::providers::ProviderKind::Fal => {
            let upstream = call_fal_model_schema(&provider.base_url, &key, model).await?;
            let (supported, controls) = fal_schema_controls(&upstream)?;
            (supported, controls, upstream)
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            let upstream = if kind == "video" {
                call_openrouter_video_models(&provider.base_url, &key).await?
            } else {
                call_openrouter_model_endpoints(&provider.base_url, &key, model).await?
            };
            let supported = if kind == "video" {
                openrouter_video_supported_parameters(&upstream, model)
            } else {
                openrouter_schema_supported_parameters(&upstream)
            };
            let controls = match kind {
                "video" => openrouter_video_schema_controls(&upstream, model, &supported),
                "music" => openrouter_music_schema_controls(&supported),
                _ => media_schema_controls(&supported),
            };
            (supported, controls, upstream)
        }
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "selected provider does not expose media model metadata".to_string(),
            )))
        }
    };

    let response = json!({
        "model": model,
        "provider_id": provider.id,
        "provider": provider.name,
        "kind": kind,
        "supported_parameters": supported,
        "controls": controls,
        "raw": upstream,
    });
    write_media_cache(cache_key, &response);
    Ok(Json(media_cache_response(response, false)).into_response())
}

/// `GET /media/status` - fetch the latest status for an asynchronous media run.
pub(crate) async fn media_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaStatusQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let id = query.id.trim();
    if id.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media generation id is required".to_string(),
        )));
    }
    let kind = validate_media_kind(&query.kind)?;
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    let upstream = fetch_media_status_upstream(&provider, &key, &query, kind).await?;
    let media = if is_openrouter_provider(&provider) && kind == "video" {
        openrouter_video_media_items(&provider.id, id, &upstream)
    } else {
        media_items_from_result(&upstream, kind)
    };
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("submitted")
        .to_string();
    let library_item = update_media_library(&st, None, &provider, id, &status, &urls, &media);
    Ok(Json(json!({
        "id": id,
        "object": "media.status",
        "kind": kind,
        "provider_id": provider.id,
        "provider": provider.name,
        "provider_kind": provider.kind,
        "model": query.model.unwrap_or_default(),
        "status": status,
        "output": upstream.get("output").cloned().unwrap_or(Value::Null),
        "media": media,
        "urls": urls,
        "library_id": library_item.as_ref().map(|item| item.id.as_str()),
        "save_state": library_item.as_ref().map(|item| item.save_state.as_str()),
        "raw": upstream,
    }))
    .into_response())
}

async fn fetch_media_status_upstream(
    provider: &crate::providers::Provider,
    key: &str,
    query: &MediaStatusQuery,
    kind: &str,
) -> Result<Value, ApiError> {
    let id = query.id.trim();
    match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            call_replicate_media_status(&provider.base_url, key, id).await
        }
        crate::providers::ProviderKind::Fal => {
            call_fal_media_status(
                &provider.base_url,
                key,
                id,
                query.model.as_deref(),
                query.response_url.as_deref(),
                query.status_url.as_deref(),
            )
            .await
        }
        crate::providers::ProviderKind::OpenAiCompatible
            if is_openrouter_provider(provider) && kind == "video" =>
        {
            call_openrouter_video_status(&provider.base_url, key, id).await
        }
        _ => Err(ApiError(Error::InvalidRequest(
            "selected provider does not expose media run status".to_string(),
        ))),
    }
}

/// `GET /media/content` - proxy authenticated OpenRouter video bytes without
/// exposing the provider credential to the desktop webview.
pub(crate) async fn media_content(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaContentQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    if query.index > 16 {
        return Err(ApiError(Error::InvalidRequest(
            "media content index is out of range".to_string(),
        )));
    }
    let id = query.id.trim();
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ApiError(Error::InvalidRequest(
            "media content id is invalid".to_string(),
        )));
    }
    let provider = select_media_provider(&st, Some(&query.provider_id), None)
        .await
        .map_err(ApiError)?;
    if !is_openrouter_provider(&provider) {
        return Err(ApiError(Error::InvalidRequest(
            "authenticated media content is only available for OpenRouter video".to_string(),
        )));
    }
    let key = media_provider_key(&provider)?;
    let url = format!(
        "{}/videos/{id}/content",
        provider.base_url.trim_end_matches('/')
    );
    let response = openrouter_request(reqwest::Client::new().get(url), &key)
        .query(&[("index", query.index)])
        .send()
        .await
        .map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter video content request failed: {error}"
            )))
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError(Error::Upstream(format!(
            "OpenRouter video content returned HTTP {status}: {body}"
        ))));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("video/mp4"));
    let content_length = response.headers().get(CONTENT_LENGTH).cloned();
    let mut proxied = Response::new(Body::from_stream(response.bytes_stream()));
    proxied.headers_mut().insert(CONTENT_TYPE, content_type);
    proxied.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    if let Some(content_length) = content_length {
        proxied.headers_mut().insert(CONTENT_LENGTH, content_length);
    }
    Ok(proxied)
}

/// `GET /media/library` - list durable locally saved media runs.
pub(crate) async fn media_library_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MediaLibraryQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    Ok(Json(library.list(
        &query.query,
        query.kind.as_deref().filter(|value| !value.is_empty()),
        query.provider.as_deref().filter(|value| !value.is_empty()),
        query.status.as_deref().filter(|value| !value.is_empty()),
        query.cursor.as_deref().filter(|value| !value.is_empty()),
        limit,
    ))
    .into_response())
}

/// `GET /media/library/{id}/content/{index}` - stream one authenticated local file.
pub(crate) async fn media_library_content(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path((id, index)): Path<(String, usize)>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let (path, mime) = library.content(&id, index).map_err(ApiError)?;
    let size = tokio::fs::metadata(&path).await.map_err(Error::from)?.len();
    let stream = async_stream::stream! {
        let mut file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(error) => {
                yield Err::<Bytes, std::io::Error>(error);
                return;
            }
        };
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = match file.read(&mut buffer).await {
                Ok(read) => read,
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(error);
                    return;
                }
            };
            if read == 0 {
                break;
            }
            yield Ok::<Bytes, std::io::Error>(Bytes::copy_from_slice(&buffer[..read]));
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if let Ok(value) = HeaderValue::from_str(&size.to_string()) {
        response.headers_mut().insert(CONTENT_LENGTH, value);
    }
    Ok(response)
}

/// `DELETE /media/library/{id}` - permanently remove a local media run and files.
pub(crate) async fn media_library_delete(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    Ok(Json(json!({ "deleted": library.delete(&id).map_err(ApiError)? })).into_response())
}

/// `POST /media/library/{id}/refresh` - refresh a pending run or retry its local save.
pub(crate) async fn media_library_refresh(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let library = st
        .media_library
        .as_ref()
        .ok_or_else(media_library_unavailable)?;
    let item = library
        .get(&id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("media library item {id}"))))?;
    if matches!(item.save_state.as_str(), "saving" | "ready") {
        return Ok(Json(item).into_response());
    }
    let provider = select_media_provider(&st, Some(&item.provider_id), None)
        .await
        .map_err(ApiError)?;
    let key = media_provider_key(&provider)?;
    if item.save_state == "failed" && !item.media.is_empty() {
        let updated = library
            .update(
                &id,
                MediaLibraryUpdate {
                    provider_run_id: item.provider_run_id.clone(),
                    status: item.status.clone(),
                    urls: item.urls.clone(),
                    media: item.media.clone(),
                },
            )
            .map_err(ApiError)?;
        spawn_media_save(library.clone(), &provider, &key, &updated);
        return Ok(Json(updated).into_response());
    }
    if item.provider_run_id.is_empty() {
        return Ok(Json(item).into_response());
    }
    let query = MediaStatusQuery {
        provider_id: item.provider_id.clone(),
        id: item.provider_run_id.clone(),
        model: Some(item.model.clone()),
        response_url: item.urls.get("response").cloned(),
        status_url: item.urls.get("status").cloned(),
        kind: item.kind.clone(),
    };
    let upstream = fetch_media_status_upstream(&provider, &key, &query, &item.kind).await?;
    let media = if is_openrouter_provider(&provider) && item.kind == "video" {
        openrouter_video_media_items(&provider.id, &item.provider_run_id, &upstream)
    } else {
        media_items_from_result(&upstream, &item.kind)
    };
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("submitted");
    let updated = update_media_library(
        &st,
        Some(&id),
        &provider,
        &item.provider_run_id,
        status,
        &urls,
        &media,
    )
    .ok_or_else(media_library_unavailable)?;
    Ok(Json(updated).into_response())
}

fn media_library_unavailable() -> ApiError {
    ApiError(Error::InvalidRequest(
        "media library is not enabled".to_string(),
    ))
}

/// `POST /media/generate` - submit a prompt to an enabled remote media
/// provider. The endpoint is intentionally provider-neutral at the UI boundary:
/// callers pass a model id, a prompt, and optional model-specific input fields.
pub(crate) async fn media_generate(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MediaGenerateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let model = req.model.trim();
    if model.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media model is required".to_string(),
        )));
    }
    let original_prompt = req.prompt.trim();
    if original_prompt.is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "media prompt is required".to_string(),
        )));
    }
    let kind = validate_media_kind(&req.kind)?.to_string();

    let provider = select_media_provider(&st, req.provider_id.as_deref(), req.provider_kind)
        .await
        .map_err(ApiError)?;
    let key = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(format!(
                "{} requires an API key",
                provider.name
            )))
        })?
        .to_string();
    let (prompt, privacy) = media_prompt_for_remote(&st, original_prompt).map_err(ApiError)?;

    let original_input = Value::Object(req.input.clone());
    let mut input = req.input;
    input.insert("prompt".to_string(), Value::String(prompt));

    let library_item = st
        .media_library
        .as_ref()
        .map(|library| {
            library.create(NewMediaLibraryItem {
                provider_id: provider.id.clone(),
                provider: provider.name.clone(),
                provider_kind: provider_kind_name(provider.kind).to_string(),
                kind: kind.clone(),
                model: model.to_string(),
                prompt: original_prompt.to_string(),
                input: original_input,
                privacy: serde_json::to_value(&privacy).unwrap_or(Value::Null),
            })
        })
        .transpose()
        .map_err(ApiError)?;

    let upstream_result = match provider.kind {
        crate::providers::ProviderKind::Replicate => {
            call_replicate_media(&provider.base_url, &key, model, input).await
        }
        crate::providers::ProviderKind::Fal => {
            call_fal_media(&provider.base_url, &key, model, input).await
        }
        crate::providers::ProviderKind::OpenAiCompatible if is_openrouter_provider(&provider) => {
            match kind.as_str() {
                "video" => {
                    call_openrouter_video_media(&provider.base_url, &key, model, input).await
                }
                "music" => {
                    call_openrouter_music_media(&provider.base_url, &key, model, input).await
                }
                _ => call_openrouter_image_media(&provider.base_url, &key, model, input).await,
            }
        }
        _ => Err(ApiError(Error::InvalidRequest(
            "selected provider is not a media provider".to_string(),
        ))),
    };
    let upstream = match upstream_result {
        Ok(upstream) => upstream,
        Err(error) => {
            if let (Some(library), Some(item)) = (&st.media_library, &library_item) {
                let _ = library.fail(&item.id, error.0.to_string());
            }
            return Err(error);
        }
    };
    let media = media_items_from_result(&upstream, &kind);
    let urls = media_urls_from_result(&provider.kind, &upstream);
    let id = upstream
        .get("id")
        .or_else(|| upstream.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let status = upstream
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if is_openrouter_provider(&provider) && kind != "video" {
            "completed"
        } else {
            "submitted"
        })
        .to_string();
    let library_item = update_media_library(
        &st,
        library_item.as_ref().map(|item| item.id.as_str()),
        &provider,
        &id,
        &status,
        &urls,
        &media,
    )
    .or(library_item);

    Ok(Json(json!({
        "id": id,
        "object": "media.generation",
        "provider_id": provider.id,
        "provider": provider.name,
        "provider_kind": provider.kind,
        "kind": kind,
        "model": model,
        "status": status,
        "output": upstream.get("output").cloned().unwrap_or(Value::Null),
        "media": media,
        "urls": urls,
        "library_id": library_item.as_ref().map(|item| item.id.as_str()),
        "save_state": library_item.as_ref().map(|item| item.save_state.as_str()),
        "privacy": privacy,
        "raw": upstream,
    }))
    .into_response())
}

fn provider_kind_name(kind: crate::providers::ProviderKind) -> &'static str {
    match kind {
        crate::providers::ProviderKind::OpenAiCompatible => "openai_compatible",
        crate::providers::ProviderKind::Anthropic => "anthropic",
        crate::providers::ProviderKind::Gemini => "gemini",
        crate::providers::ProviderKind::Replicate => "replicate",
        crate::providers::ProviderKind::Fal => "fal",
    }
}

fn update_media_library(
    st: &AppState,
    library_id: Option<&str>,
    provider: &crate::providers::Provider,
    provider_run_id: &str,
    status: &str,
    urls: &Value,
    media: &[MediaItem],
) -> Option<MediaLibraryItem> {
    let library = st.media_library.as_ref()?;
    let id = library_id.map(str::to_string).or_else(|| {
        library
            .find_by_run(&provider.id, provider_run_id)
            .map(|item| item.id)
    })?;
    let already_saving_or_ready = library
        .get(&id)
        .is_some_and(|item| matches!(item.save_state.as_str(), "saving" | "ready"));
    let item = library
        .update(
            &id,
            MediaLibraryUpdate {
                provider_run_id: provider_run_id.to_string(),
                status: status.to_string(),
                urls: media_urls_map(urls),
                media: media
                    .iter()
                    .map(|item| MediaLibraryMediaItem {
                        url: item.url.clone(),
                        source_url: item.url.clone(),
                        kind: item.kind.clone(),
                        mime: item.mime.clone(),
                        requires_auth: item.requires_auth,
                        file_name: None,
                        local_path: None,
                        size_bytes: None,
                    })
                    .collect(),
            },
        )
        .ok()?;
    if !already_saving_or_ready && !item.media.is_empty() {
        if let Ok(key) = media_provider_key(provider) {
            spawn_media_save(library.clone(), provider, &key, &item);
        }
    }
    Some(item)
}

fn media_urls_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn spawn_media_save(
    library: Arc<crate::media_library::MediaLibrary>,
    provider: &crate::providers::Provider,
    key: &str,
    item: &MediaLibraryItem,
) {
    let sources = item
        .media
        .iter()
        .enumerate()
        .map(|(index, media)| {
            let authenticated = media.requires_auth && is_openrouter_provider(provider);
            MediaDownloadSource {
                url: if authenticated {
                    format!(
                        "{}/videos/{}/content?index={index}",
                        provider.base_url.trim_end_matches('/'),
                        item.provider_run_id
                    )
                } else {
                    media.source_url.clone()
                },
                kind: media.kind.clone(),
                mime: media.mime.clone(),
                authorization: authenticated.then(|| format!("Bearer {key}")),
            }
        })
        .collect::<Vec<_>>();
    if sources.is_empty() {
        return;
    }
    let id = item.id.clone();
    tokio::spawn(async move {
        if let Err(error) = library.save(&id, sources).await {
            tracing::warn!(media_library_id = %id, "failed to save generated media: {error}");
        }
    });
}

async fn select_media_provider(
    st: &AppState,
    provider_id: Option<&str>,
    provider_kind: Option<crate::providers::ProviderKind>,
) -> milim_core::Result<crate::providers::Provider> {
    let reg = st
        .providers
        .as_ref()
        .ok_or_else(|| Error::InvalidRequest("providers are not enabled".to_string()))?;
    let providers = reg.list().await;
    let selected = providers.into_iter().find(|provider| {
        if !provider.enabled {
            return false;
        }
        if provider.kind.is_chat() && !is_openrouter_provider(provider) {
            return false;
        }
        if let Some(id) = provider_id {
            return provider.id == id;
        }
        if let Some(kind) = provider_kind {
            return provider.kind == kind;
        }
        true
    });
    selected.ok_or_else(|| {
        Error::InvalidRequest(
            "no enabled Replicate, fal, or OpenRouter media provider matched the request"
                .to_string(),
        )
    })
}

fn is_openrouter_provider(provider: &crate::providers::Provider) -> bool {
    if provider.kind != crate::providers::ProviderKind::OpenAiCompatible {
        return false;
    }
    provider.name.eq_ignore_ascii_case("openrouter")
        || provider
            .base_url
            .to_ascii_lowercase()
            .contains("openrouter.ai/")
}

fn media_provider_key(provider: &crate::providers::Provider) -> Result<String, ApiError> {
    provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(format!(
                "{} requires an API key",
                provider.name
            )))
        })
}

fn media_prompt_for_remote(
    st: &AppState,
    prompt: &str,
) -> milim_core::Result<(String, MediaPrivacyInfo)> {
    match st.privacy.mode() {
        PrivacyMode::Off => Ok((
            prompt.to_string(),
            MediaPrivacyInfo {
                mode: "off",
                redacted: false,
                detections: 0,
                kinds: String::new(),
            },
        )),
        PrivacyMode::Block => {
            let detections = st.privacy.scan_text(prompt);
            if detections.is_empty() {
                Ok((
                    prompt.to_string(),
                    MediaPrivacyInfo {
                        mode: "block",
                        redacted: false,
                        detections: 0,
                        kinds: String::new(),
                    },
                ))
            } else {
                Err(Error::InvalidRequest(format!(
                    "blocked by the privacy gate: media prompt contains {} ({} item(s)). Switch the gate to Redact or Off to send this to a remote media provider.",
                    kinds_summary(&detections),
                    detections.len()
                )))
            }
        }
        PrivacyMode::Redact => {
            let detections = st.privacy.scan_text(prompt);
            let redaction = st.privacy.redact_text(prompt);
            Ok((
                redaction.text,
                MediaPrivacyInfo {
                    mode: "redact",
                    redacted: !redaction.map.is_empty(),
                    detections: detections.len(),
                    kinds: kinds_summary(&detections),
                },
            ))
        }
    }
}

async fn call_replicate_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let url = format!("{}/predictions", base_url.trim_end_matches('/'));
    let mut body = serde_json::Map::new();
    if is_replicate_version_id(model) {
        body.insert("version".to_string(), Value::String(model.to_string()));
    } else {
        body.insert("model".to_string(), Value::String(model.to_string()));
    }
    body.insert("input".to_string(), Value::Object(input));
    post_media_json(
        reqwest::Client::new()
            .post(url)
            .bearer_auth(api_key)
            .header("Prefer", "wait")
            .json(&Value::Object(body)),
    )
    .await
}

async fn call_replicate_media_status(
    base_url: &str,
    api_key: &str,
    id: &str,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/predictions/{}",
        base_url.trim_end_matches('/'),
        id.trim_start_matches('/')
    );
    get_media_json(reqwest::Client::new().get(url).bearer_auth(api_key)).await
}

async fn call_fal_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        model.trim_start_matches('/')
    );
    post_media_json(
        reqwest::Client::new()
            .post(url)
            .header("Authorization", format!("Key {api_key}"))
            .json(&Value::Object(input)),
    )
    .await
}

async fn call_fal_media_status(
    base_url: &str,
    api_key: &str,
    id: &str,
    model: Option<&str>,
    response_url: Option<&str>,
    status_url: Option<&str>,
) -> Result<Value, ApiError> {
    let url = response_url
        .or(status_url)
        .map(|url| validate_media_status_url(base_url, url))
        .transpose()?
        .unwrap_or_else(|| {
            let model = model.unwrap_or_default().trim_matches('/');
            format!(
                "{}/{}/requests/{}",
                base_url.trim_end_matches('/'),
                model,
                id.trim_start_matches('/')
            )
        });
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}")),
    )
    .await
}

async fn call_openrouter_image_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    // The media endpoint returns image artifacts; accepting a saved/custom
    // text co-output here breaks image-only OpenRouter endpoints such as Flux.
    let _ = input.remove("modalities");
    let modalities = json!(["image"]);
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let mut body = input;
    body.insert("model".to_string(), Value::String(model.to_string()));
    body.insert(
        "messages".to_string(),
        json!([{ "role": "user", "content": prompt }]),
    );
    body.insert("modalities".to_string(), modalities);
    body.insert("stream".to_string(), Value::Bool(false));

    post_media_json(
        openrouter_request(reqwest::Client::new().post(url), api_key).json(&Value::Object(body)),
    )
    .await
}

async fn call_openrouter_video_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    input.remove("modalities");
    input.remove("stream");
    input.insert("model".to_string(), Value::String(model.to_string()));
    input.insert("prompt".to_string(), Value::String(prompt));
    let url = format!("{}/videos", base_url.trim_end_matches('/'));
    post_media_json(
        openrouter_request(reqwest::Client::new().post(url), api_key).json(&Value::Object(input)),
    )
    .await
}

async fn call_openrouter_video_status(
    base_url: &str,
    api_key: &str,
    id: &str,
) -> Result<Value, ApiError> {
    let url = format!(
        "{}/videos/{}",
        base_url.trim_end_matches('/'),
        id.trim_start_matches('/')
    );
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_openrouter_music_media(
    base_url: &str,
    api_key: &str,
    model: &str,
    mut input: serde_json::Map<String, Value>,
) -> Result<Value, ApiError> {
    let prompt = input
        .remove("prompt")
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    input.remove("modalities");
    input.insert("model".to_string(), Value::String(model.to_string()));
    input.insert(
        "messages".to_string(),
        json!([{ "role": "user", "content": prompt }]),
    );
    input.insert("modalities".to_string(), json!(["text", "audio"]));
    input.insert("stream".to_string(), Value::Bool(true));
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let response = openrouter_request(reqwest::Client::new().post(url), api_key)
        .json(&Value::Object(input))
        .send()
        .await
        .map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter music request failed: {error}"
            )))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        ApiError(Error::Upstream(format!(
            "OpenRouter music response failed: {error}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "OpenRouter music returned HTTP {status}: {body}"
        ))));
    }
    openrouter_music_from_sse(&body)
}

fn openrouter_music_from_sse(body: &str) -> Result<Value, ApiError> {
    let mut id = String::new();
    let mut audio = String::new();
    let mut text = String::new();
    let mut transcript = String::new();
    for line in body.lines() {
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let chunk: Value = serde_json::from_str(data).map_err(|error| {
            ApiError(Error::Upstream(format!(
                "OpenRouter music stream returned invalid JSON: {error}"
            )))
        })?;
        if id.is_empty() {
            id = chunk
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        let Some(delta) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("delta"))
        else {
            continue;
        };
        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            text.push_str(content);
        }
        if let Some(audio_delta) = delta.get("audio") {
            if let Some(data) = audio_delta.get("data").and_then(Value::as_str) {
                audio.push_str(data);
            }
            if let Some(value) = audio_delta.get("transcript").and_then(Value::as_str) {
                transcript.push_str(value);
            }
        }
    }
    if audio.is_empty() {
        return Err(ApiError(Error::Upstream(
            "OpenRouter music stream completed without audio data".to_string(),
        )));
    }
    Ok(json!({
        "id": id,
        "status": "completed",
        "output": {
            "text": text,
            "transcript": transcript,
        },
        "audio": {
            "url": format!("data:audio/mpeg;base64,{audio}"),
            "content_type": "audio/mpeg"
        }
    }))
}

fn openrouter_request(builder: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    builder
        .bearer_auth(api_key)
        .header("HTTP-Referer", "https://milim.ai/")
        .header("X-OpenRouter-Title", "milim")
}

async fn call_openrouter_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
) -> Result<Value, ApiError> {
    if kind == "video" {
        return call_openrouter_video_models(base_url, api_key).await;
    }
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let output_kind = if kind == "music" { "audio" } else { kind };
    get_media_json(
        openrouter_request(reqwest::Client::new().get(url), api_key)
            .query(&[("output_modalities", output_kind)]),
    )
    .await
}

async fn call_openrouter_video_models(base_url: &str, api_key: &str) -> Result<Value, ApiError> {
    let url = format!("{}/videos/models", base_url.trim_end_matches('/'));
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_openrouter_model_endpoints(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let (author, slug) = model.split_once('/').ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "OpenRouter media model id must be in author/slug form".to_string(),
        ))
    })?;
    if author.is_empty() || slug.is_empty() || slug.contains('/') {
        return Err(ApiError(Error::InvalidRequest(
            "OpenRouter media model id must be in author/slug form".to_string(),
        )));
    }
    let url = format!(
        "{}/models/{author}/{slug}/endpoints",
        base_url.trim_end_matches('/')
    );
    get_media_json(openrouter_request(reqwest::Client::new().get(url), api_key)).await
}

async fn call_replicate_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
) -> Result<Value, ApiError> {
    let url = format!("{}/search", base_url.trim_end_matches('/'));
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .bearer_auth(api_key)
            .query(&[("query", kind), ("limit", "50")]),
    )
    .await
}

async fn call_replicate_model_schema(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let (owner, name) = model.split_once('/').ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "Replicate media model id must be in owner/name form".to_string(),
        ))
    })?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return Err(ApiError(Error::InvalidRequest(
            "Replicate media model id must be in owner/name form".to_string(),
        )));
    }
    let url = format!("{}/models/{owner}/{name}", base_url.trim_end_matches('/'));
    get_media_json(reqwest::Client::new().get(url).bearer_auth(api_key)).await
}

async fn call_fal_media_models(
    base_url: &str,
    api_key: &str,
    kind: &str,
    query: &str,
) -> Result<Value, ApiError> {
    let category = match kind {
        "image" => "text-to-image",
        "music" => "text-to-audio",
        _ => kind,
    };
    let url = format!("{}/models", fal_platform_base_url(base_url));
    let mut params = vec![
        ("limit", "50"),
        ("category", category),
        ("status", "active"),
    ];
    if (query != kind || kind == "music") && !query.trim().is_empty() {
        params.push(("q", query));
    }
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}"))
            .query(&params),
    )
    .await
}

async fn call_fal_model_schema(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Value, ApiError> {
    let url = format!("{}/models", fal_platform_base_url(base_url));
    get_media_json(
        reqwest::Client::new()
            .get(url)
            .header("Authorization", format!("Key {api_key}"))
            .query(&[("endpoint_id", model), ("expand", "openapi-3.0")]),
    )
    .await
}

fn fal_platform_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("127.0.0.1")
        || lower.contains("localhost")
        || lower.contains("[::1]")
        || lower.contains("api.fal.ai/")
    {
        trimmed.to_string()
    } else {
        "https://api.fal.ai/v1".to_string()
    }
}

fn validate_media_status_url(base_url: &str, url: &str) -> Result<String, ApiError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ApiError(Error::InvalidRequest(
            "media status URL is not a valid URL".to_string(),
        ))
    })?;
    let allowed = reqwest::Url::parse(base_url)
        .ok()
        .map(|base| parsed.scheme() == base.scheme() && parsed.host_str() == base.host_str())
        .unwrap_or(false)
        || matches!(
            parsed.host_str(),
            Some("queue.fal.run") | Some("fal.run") | Some("api.replicate.com")
        );
    if !allowed {
        return Err(ApiError(Error::InvalidRequest(
            "media status URL must match the selected provider".to_string(),
        )));
    }
    Ok(url.to_string())
}

async fn get_media_json(builder: reqwest::RequestBuilder) -> Result<Value, ApiError> {
    let response = builder.send().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider request failed: {e}"
        )))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider response failed: {e}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "media provider returned HTTP {status}: {body}"
        ))));
    }
    serde_json::from_str(&body).map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider returned invalid JSON: {e}"
        )))
    })
}

fn media_models_from_openrouter(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let upstream_modalities = openrouter_output_modalities(model);
            let descriptor = media_model_descriptor(model);
            let matches = match kind {
                // The dedicated /videos/models catalog is already scoped to
                // generation models and does not consistently repeat a video
                // output modality on every entry.
                "video" => true,
                "music" => {
                    upstream_modalities.iter().any(|item| item == "audio")
                        && openrouter_input_modalities(model)
                            .iter()
                            .all(|item| item != "audio")
                        && descriptor_is_music(&descriptor)
                }
                _ => upstream_modalities.iter().any(|item| item == kind),
            };
            if !matches {
                return None;
            }
            let id = model.get("id").and_then(Value::as_str)?.to_string();
            let output_modalities = vec![kind.to_string()];
            Some(MediaModelInfo {
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description: model
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                output_modalities,
                supported_parameters: openrouter_video_model_parameters(model),
                default_parameters: model
                    .get("default_parameters")
                    .cloned()
                    .unwrap_or(Value::Null),
                pricing: model
                    .get("pricing")
                    .or_else(|| model.get("pricing_skus"))
                    .cloned()
                    .unwrap_or(Value::Null),
                id,
            })
        })
        .collect()
}

fn media_models_from_replicate(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let model = item.get("model").unwrap_or(item);
            let openapi = replicate_model_openapi_schema(model);
            let id = replicate_model_id(model)?;
            let descriptor = format!(
                "{} {} {}",
                id,
                model
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                item.get("metadata").cloned().unwrap_or(Value::Null)
            )
            .to_ascii_lowercase();
            let output_schema =
                openapi.and_then(|schema| openapi_schema_by_name(schema, "Output"))?;
            if !schema_outputs_media_url(output_schema, openapi?, 0)
                || !descriptor_matches_media_kind(&descriptor, kind)
            {
                return None;
            }
            let output_modalities = vec![kind.to_string()];
            let controls = openapi
                .and_then(|openapi| {
                    openapi_input_schema(openapi)
                        .map(|schema| json_schema_controls(schema, openapi))
                })
                .unwrap_or_default();
            let description = item
                .get("metadata")
                .and_then(|metadata| metadata.get("generated_description"))
                .and_then(Value::as_str)
                .or_else(|| model.get("description").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            Some(MediaModelInfo {
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description,
                output_modalities,
                supported_parameters: supported_parameters_from_controls(&controls),
                default_parameters: defaults_from_controls(&controls),
                pricing: Value::Null,
                id,
            })
        })
        .collect()
}

fn media_models_from_fal(value: &Value, kind: &str) -> Vec<MediaModelInfo> {
    value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let metadata = model.get("metadata").unwrap_or(&Value::Null);
            let output_modalities = fal_output_modalities(model, metadata);
            if !output_modalities.iter().any(|item| item == kind) {
                return None;
            }
            let id = model
                .get("endpoint_id")
                .and_then(Value::as_str)?
                .to_string();
            let controls = model
                .get("openapi")
                .and_then(|openapi| {
                    openapi_input_schema(openapi)
                        .map(|schema| json_schema_controls(schema, openapi))
                })
                .unwrap_or_default();
            Some(MediaModelInfo {
                name: metadata
                    .get("display_name")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string(),
                description: metadata
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                output_modalities,
                supported_parameters: supported_parameters_from_controls(&controls),
                default_parameters: defaults_from_controls(&controls),
                pricing: Value::Null,
                id,
            })
        })
        .collect()
}

fn filter_media_models(models: Vec<MediaModelInfo>, query: &str) -> Vec<MediaModelInfo> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() || matches!(needle.as_str(), "image" | "video" | "music") {
        return models;
    }
    models
        .into_iter()
        .filter(|model| {
            model.id.to_ascii_lowercase().contains(&needle)
                || model.name.to_ascii_lowercase().contains(&needle)
                || model.description.to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

fn replicate_schema_controls(
    value: &Value,
) -> Result<(Vec<String>, Vec<MediaSchemaControl>), ApiError> {
    let openapi = replicate_model_openapi_schema(value).ok_or_else(|| {
        ApiError(Error::Upstream(
            "Replicate model response did not include an OpenAPI schema".to_string(),
        ))
    })?;
    let input_schema = openapi_input_schema(openapi).ok_or_else(|| {
        ApiError(Error::Upstream(
            "Replicate model schema did not include an Input schema".to_string(),
        ))
    })?;
    let controls = json_schema_controls(input_schema, openapi);
    Ok((supported_parameters_from_controls(&controls), controls))
}

fn fal_schema_controls(value: &Value) -> Result<(Vec<String>, Vec<MediaSchemaControl>), ApiError> {
    let openapi = value
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| models.first())
        .and_then(|model| model.get("openapi"))
        .or_else(|| value.get("openapi"))
        .ok_or_else(|| {
            ApiError(Error::Upstream(
                "fal model response did not include an OpenAPI schema".to_string(),
            ))
        })?;
    let input_schema = openapi_input_schema(openapi).ok_or_else(|| {
        ApiError(Error::Upstream(
            "fal model schema did not include an input schema".to_string(),
        ))
    })?;
    let controls = json_schema_controls(input_schema, openapi);
    Ok((supported_parameters_from_controls(&controls), controls))
}

fn replicate_model_id(model: &Value) -> Option<String> {
    let owner = model.get("owner").and_then(Value::as_str)?;
    let name = model.get("name").and_then(Value::as_str)?;
    Some(format!("{owner}/{name}"))
}

fn replicate_model_openapi_schema(model: &Value) -> Option<&Value> {
    model
        .get("latest_version")
        .and_then(|version| version.get("openapi_schema"))
}

fn fal_output_modalities(model: &Value, metadata: &Value) -> Vec<String> {
    let category = metadata
        .get("category")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if category == "text-to-image" || category.ends_with("to-image") || category == "image" {
        vec!["image".to_string()]
    } else if category == "text-to-video" || category.ends_with("to-video") || category == "video" {
        vec!["video".to_string()]
    } else if category == "text-to-audio" {
        let descriptor = format!("{} {}", model, metadata).to_ascii_lowercase();
        if descriptor_is_music(&descriptor) {
            vec!["music".to_string()]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    }
}

fn openapi_input_schema(openapi: &Value) -> Option<&Value> {
    if let Some(schema) = openapi_schema_by_name(openapi, "Input") {
        return Some(schema);
    }
    if let Some(paths) = openapi.get("paths").and_then(Value::as_object) {
        for operation in paths.values().filter_map(|path| path.get("post")) {
            if let Some(schema) = operation
                .get("requestBody")
                .and_then(|body| body.get("content"))
                .and_then(|content| content.get("application/json"))
                .and_then(|content| content.get("schema"))
                .and_then(|schema| resolve_schema_ref(openapi, schema))
            {
                return Some(schema);
            }
        }
    }
    openapi
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .and_then(|schemas| {
            schemas
                .iter()
                .find(|(name, schema)| {
                    name.to_ascii_lowercase().ends_with("input")
                        && schema
                            .get("properties")
                            .and_then(Value::as_object)
                            .is_some()
                })
                .map(|(_, schema)| schema)
        })
}

fn openapi_schema_by_name<'a>(openapi: &'a Value, suffix: &str) -> Option<&'a Value> {
    let suffix = suffix.to_ascii_lowercase();
    openapi
        .get("components")
        .and_then(|components| components.get("schemas"))
        .and_then(Value::as_object)
        .and_then(|schemas| {
            schemas
                .iter()
                .find(|(name, _)| name.to_ascii_lowercase() == suffix)
                .or_else(|| {
                    schemas
                        .iter()
                        .find(|(name, _)| name.to_ascii_lowercase().ends_with(&suffix))
                })
                .map(|(_, schema)| schema)
        })
}

fn resolve_schema_ref<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    let reference = schema.get("$ref").and_then(Value::as_str)?;
    let path = reference.strip_prefix("#/")?;
    let mut cursor = root;
    for segment in path.split('/') {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        cursor = cursor.get(&segment)?;
    }
    Some(cursor)
}

fn json_schema_controls(input_schema: &Value, root: &Value) -> Vec<MediaSchemaControl> {
    let Some(properties) = input_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut keys = input_schema
        .get("x-fal-order-properties")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| properties.keys().cloned().collect());
    for key in properties.keys() {
        if !keys.iter().any(|existing| existing == key) {
            keys.push(key.clone());
        }
    }
    keys.into_iter()
        .filter(|key| key != "prompt")
        .filter_map(|key| {
            properties
                .get(&key)
                .and_then(|schema| control_from_json_schema_property(&key, schema, root))
        })
        .collect()
}

fn control_from_json_schema_property(
    key: &str,
    schema: &Value,
    root: &Value,
) -> Option<MediaSchemaControl> {
    let label = schema
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| label_from_key(key));
    let description = schema_description(schema, root, 0);
    let default_value = schema.get("default").cloned();
    if let Some(options) = schema_enum_options(schema, root, 0) {
        return Some(select_control_values(
            key,
            &label,
            vec![key.to_string()],
            options,
            default_value,
            description,
        ));
    }
    let schema_type = schema_type(schema, root, 0)?;
    match schema_type.as_str() {
        "boolean" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "checkbox".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: None,
        }),
        "integer" | "number" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "number".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: schema_number(schema, root, "minimum", 0),
            max: schema_number(schema, root, "maximum", 0),
            step: if schema_type == "integer" {
                Some(1.0)
            } else {
                None
            },
            item_kind: None,
            placeholder: None,
        }),
        "array" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "array".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: schema_number(schema, root, "minItems", 0),
            max: schema_number(schema, root, "maxItems", 0),
            step: None,
            item_kind: schema_array_item_kind(schema, root, 0).map(str::to_string),
            placeholder: Some("one value per line".to_string()),
        }),
        "object" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: "json".to_string(),
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: Some("{ }".to_string()),
        }),
        "string" => Some(MediaSchemaControl {
            key: key.to_string(),
            label,
            kind: if schema_is_url(schema, root, key, 0) {
                "url".to_string()
            } else {
                "text".to_string()
            },
            path: vec![key.to_string()],
            description,
            options: None,
            default_value,
            min: None,
            max: None,
            step: None,
            item_kind: None,
            placeholder: None,
        }),
        _ => None,
    }
}

fn schema_enum_options(
    schema: &Value,
    root: &Value,
    depth: usize,
) -> Option<Vec<MediaControlOption>> {
    if depth > 8 {
        return None;
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let options = values
            .iter()
            .filter(|value| !value.is_null())
            .map(|value| MediaControlOption {
                label: media_option_label(value),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        if !options.is_empty() {
            return Some(options);
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(options) = schema_enum_options(item, root, depth + 1) {
                    return Some(options);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(options) = schema_enum_options(resolved, root, depth + 1) {
                        return Some(options);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_enum_options(resolved, root, depth + 1))
}

fn schema_type(schema: &Value, root: &Value, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    match schema.get("type") {
        Some(Value::String(kind)) if kind != "null" => return Some(kind.to_string()),
        Some(Value::Array(kinds)) => {
            if let Some(kind) = kinds
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| *kind != "null")
            {
                return Some(kind.to_string());
            }
        }
        _ => {}
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(kind) = schema_type(item, root, depth + 1) {
                    return Some(kind);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(kind) = schema_type(resolved, root, depth + 1) {
                        return Some(kind);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema).and_then(|resolved| schema_type(resolved, root, depth + 1))
}

fn schema_number(schema: &Value, root: &Value, key: &str, depth: usize) -> Option<f64> {
    if depth > 8 {
        return None;
    }
    if let Some(value) = schema.get(key).and_then(Value::as_f64) {
        return Some(value);
    }
    for group in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(group).and_then(Value::as_array) {
            for item in items {
                if let Some(value) = schema_number(item, root, key, depth + 1) {
                    return Some(value);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(value) = schema_number(resolved, root, key, depth + 1) {
                        return Some(value);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_number(resolved, root, key, depth + 1))
}

fn schema_description(schema: &Value, root: &Value, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        return Some(description.trim().to_string());
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            for item in items {
                if let Some(description) = schema_description(item, root, depth + 1) {
                    return Some(description);
                }
                if let Some(resolved) = resolve_schema_ref(root, item) {
                    if let Some(description) = schema_description(resolved, root, depth + 1) {
                        return Some(description);
                    }
                }
            }
        }
    }
    resolve_schema_ref(root, schema)
        .and_then(|resolved| schema_description(resolved, root, depth + 1))
}

fn schema_is_url(schema: &Value, root: &Value, key: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if schema.get("format").and_then(Value::as_str) == Some("uri") {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    if lower.ends_with("_url") || lower.ends_with(" url") || lower.contains("image_url") {
        return true;
    }
    for group in ["anyOf", "oneOf", "allOf"] {
        if let Some(items) = schema.get(group).and_then(Value::as_array) {
            if items
                .iter()
                .any(|item| schema_is_url(item, root, key, depth + 1))
            {
                return true;
            }
        }
    }
    resolve_schema_ref(root, schema)
        .map(|resolved| schema_is_url(resolved, root, key, depth + 1))
        .unwrap_or(false)
}

fn schema_array_item_kind(schema: &Value, root: &Value, depth: usize) -> Option<&'static str> {
    if depth > 8 {
        return None;
    }
    let items = schema.get("items")?;
    let resolved = resolve_schema_ref(root, items).unwrap_or(items);
    if schema_is_url(resolved, root, "", depth + 1) {
        return Some("url");
    }
    match schema_type(resolved, root, depth + 1).as_deref() {
        Some("integer") | Some("number") => Some("number"),
        Some("boolean") => Some("checkbox"),
        Some("object") | Some("array") => Some("json"),
        Some("string") => Some("text"),
        _ => None,
    }
}

fn supported_parameters_from_controls(controls: &[MediaSchemaControl]) -> Vec<String> {
    controls.iter().map(|control| control.key.clone()).collect()
}

fn defaults_from_controls(controls: &[MediaSchemaControl]) -> Value {
    let mut out = serde_json::Map::new();
    for control in controls {
        if let Some(value) = &control.default_value {
            out.insert(control.key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

fn media_option_label(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn label_from_key(key: &str) -> String {
    key.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn openrouter_output_modalities(model: &Value) -> Vec<String> {
    string_array(
        model
            .get("architecture")
            .and_then(|architecture| architecture.get("output_modalities")),
    )
}

fn openrouter_input_modalities(model: &Value) -> Vec<String> {
    string_array(
        model
            .get("architecture")
            .and_then(|architecture| architecture.get("input_modalities")),
    )
}

fn media_model_descriptor(model: &Value) -> String {
    format!(
        "{} {} {} {}",
        model.get("id").and_then(Value::as_str).unwrap_or_default(),
        model
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        model
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        model.get("tags").cloned().unwrap_or(Value::Null)
    )
    .to_ascii_lowercase()
}

fn descriptor_is_music(descriptor: &str) -> bool {
    let excluded = [
        "text-to-speech",
        "speech-to-text",
        "transcription",
        "voice cloning",
        "voice changer",
        "dubbing",
        "tts",
    ];
    let music = [
        "music",
        "song",
        "instrumental",
        "lyrics",
        "musicgen",
        "lyria",
        "melody",
    ];
    music.iter().any(|term| descriptor.contains(term))
        && !excluded.iter().any(|term| descriptor.contains(term))
}

fn descriptor_matches_media_kind(descriptor: &str, kind: &str) -> bool {
    match kind {
        "music" => descriptor_is_music(descriptor),
        "video" => [
            "video",
            "text-to-video",
            "veo",
            "kling",
            "runway",
            "wan",
            "hailuo",
        ]
        .iter()
        .any(|term| descriptor.contains(term)),
        "image" => [
            "image",
            "text-to-image",
            "flux",
            "sdxl",
            "stable diffusion",
            "recraft",
        ]
        .iter()
        .any(|term| descriptor.contains(term)),
        _ => false,
    }
}

fn openrouter_video_model_parameters(model: &Value) -> Vec<String> {
    let mut out = string_array(model.get("supported_parameters"));
    for (field, parameter) in [
        ("supported_durations", "duration"),
        ("durations", "duration"),
        ("supported_resolutions", "resolution"),
        ("resolutions", "resolution"),
        ("supported_aspect_ratios", "aspect_ratio"),
        ("aspect_ratios", "aspect_ratio"),
        ("supported_sizes", "size"),
    ] {
        if model.get(field).is_some() && !out.iter().any(|item| item == parameter) {
            out.push(parameter.to_string());
        }
    }
    for parameter in ["generate_audio", "seed"] {
        if model.get(parameter).is_some() && !out.iter().any(|item| item == parameter) {
            out.push(parameter.to_string());
        }
    }
    out
}

fn schema_outputs_media_url(schema: &Value, root: &Value, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    if schema.get("format").and_then(Value::as_str) == Some("uri") {
        return true;
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        if properties
            .values()
            .any(|value| schema_outputs_media_url(value, root, depth + 1))
        {
            return true;
        }
    }
    if let Some(items) = schema.get("items") {
        if schema_outputs_media_url(items, root, depth + 1) {
            return true;
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if schema
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| schema_outputs_media_url(item, root, depth + 1))
            })
        {
            return true;
        }
    }
    resolve_schema_ref(root, schema)
        .is_some_and(|resolved| schema_outputs_media_url(resolved, root, depth + 1))
}

fn openrouter_schema_supported_parameters(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(endpoints) = value.get("endpoints").and_then(Value::as_array) {
        for endpoint in endpoints {
            push_unique_strings(&mut out, endpoint.get("supported_parameters"));
        }
    }
    push_unique_strings(&mut out, value.get("supported_parameters"));
    out
}

fn openrouter_video_model<'a>(value: &'a Value, model: &str) -> Option<&'a Value> {
    value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(model))
        })
}

fn openrouter_video_supported_parameters(value: &Value, model: &str) -> Vec<String> {
    openrouter_video_model(value, model)
        .map(openrouter_video_model_parameters)
        .unwrap_or_default()
}

fn openrouter_video_values(model: &Value, fields: &[&str]) -> Vec<Value> {
    for field in fields {
        if let Some(values) = model.get(field).and_then(Value::as_array) {
            return values
                .iter()
                .filter(|value| value.is_string() || value.is_number())
                .cloned()
                .collect();
        }
    }
    Vec::new()
}

fn openrouter_video_schema_controls(
    value: &Value,
    model_id: &str,
    supported: &[String],
) -> Vec<MediaSchemaControl> {
    let Some(model) = openrouter_video_model(value, model_id) else {
        return Vec::new();
    };
    let mut controls = Vec::new();
    for (key, label, fields) in [
        (
            "duration",
            "Duration",
            &["supported_durations", "durations"][..],
        ),
        (
            "resolution",
            "Resolution",
            &["supported_resolutions", "resolutions"][..],
        ),
        (
            "aspect_ratio",
            "Aspect ratio",
            &["supported_aspect_ratios", "aspect_ratios"][..],
        ),
    ] {
        let values = openrouter_video_values(model, fields);
        if !values.is_empty() {
            controls.push(select_control_values(
                key,
                label,
                vec![key.to_string()],
                values
                    .iter()
                    .map(|value| MediaControlOption {
                        label: media_option_label(value),
                        value: value.clone(),
                    })
                    .collect(),
                values.first().cloned(),
                None,
            ));
        }
    }
    let has = |key: &str| supported.iter().any(|item| item == key);
    if has("generate_audio") || model.get("generate_audio").is_some() {
        controls.push(checkbox_control(
            "generate_audio",
            "Generate audio",
            ["generate_audio"],
            Some(json!(model
                .get("generate_audio")
                .and_then(Value::as_bool)
                .unwrap_or(false))),
        ));
    }
    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    controls
}

fn openrouter_music_schema_controls(supported: &[String]) -> Vec<MediaSchemaControl> {
    let has = |key: &str| supported.iter().any(|item| item == key);
    let mut controls = Vec::new();
    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    if has("temperature") {
        controls.push(number_control(
            "temperature",
            "Temperature",
            ["temperature"],
            Some(json!(0.7)),
            Some(0.0),
            Some(2.0),
            Some(0.1),
        ));
    }
    if has("top_p") {
        controls.push(number_control(
            "top_p",
            "Top P",
            ["top_p"],
            Some(json!(0.9)),
            Some(0.0),
            Some(1.0),
            Some(0.05),
        ));
    }
    controls
}

fn push_unique_strings(out: &mut Vec<String>, value: Option<&Value>) {
    for item in string_array(value) {
        if !out.iter().any(|existing| existing == &item) {
            out.push(item);
        }
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn media_schema_controls(supported: &[String]) -> Vec<MediaSchemaControl> {
    let has = |key: &str| supported.iter().any(|item| item == key);
    let mut controls = vec![select_control(
        "aspect_ratio",
        "Aspect ratio",
        ["image_config", "aspect_ratio"],
        ["1:1", "16:9", "9:16", "4:3", "3:4"],
        Some("1:1"),
    )];

    controls.push(select_control(
        "quality",
        "Quality",
        ["image_config", "quality"],
        ["auto", "low", "medium", "high"],
        Some("auto"),
    ));

    if has("seed") {
        controls.push(number_control(
            "seed",
            "Seed",
            ["seed"],
            None,
            Some(0.0),
            None,
            Some(1.0),
        ));
    }
    if has("temperature") {
        controls.push(number_control(
            "temperature",
            "Temperature",
            ["temperature"],
            Some(json!(0.7)),
            Some(0.0),
            Some(2.0),
            Some(0.1),
        ));
    }
    if has("top_p") {
        controls.push(number_control(
            "top_p",
            "Top P",
            ["top_p"],
            Some(json!(0.9)),
            Some(0.0),
            Some(1.0),
            Some(0.05),
        ));
    }
    controls
}

fn select_control<const N: usize, const M: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    options: [&str; M],
    default_value: Option<&str>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "select".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: Some(
            options
                .into_iter()
                .map(|value| MediaControlOption {
                    label: value.to_string(),
                    value: Value::String(value.to_string()),
                })
                .collect(),
        ),
        default_value: default_value.map(|value| Value::String(value.to_string())),
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn select_control_values(
    key: &str,
    label: &str,
    path: Vec<String>,
    options: Vec<MediaControlOption>,
    default_value: Option<Value>,
    description: Option<String>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "select".to_string(),
        path,
        description,
        options: Some(options),
        default_value,
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn checkbox_control<const N: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    default_value: Option<Value>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "checkbox".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: None,
        default_value,
        min: None,
        max: None,
        step: None,
        item_kind: None,
        placeholder: None,
    }
}

fn number_control<const N: usize>(
    key: &str,
    label: &str,
    path: [&str; N],
    default_value: Option<Value>,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
) -> MediaSchemaControl {
    MediaSchemaControl {
        key: key.to_string(),
        label: label.to_string(),
        kind: "number".to_string(),
        path: path.into_iter().map(str::to_string).collect(),
        description: None,
        options: None,
        default_value,
        min,
        max,
        step,
        item_kind: None,
        placeholder: None,
    }
}

async fn post_media_json(builder: reqwest::RequestBuilder) -> Result<Value, ApiError> {
    let response = builder.send().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider request failed: {e}"
        )))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider response failed: {e}"
        )))
    })?;
    if !status.is_success() {
        return Err(ApiError(Error::Upstream(format!(
            "media provider returned HTTP {status}: {body}"
        ))));
    }
    serde_json::from_str(&body).map_err(|e| {
        ApiError(Error::Upstream(format!(
            "media provider returned invalid JSON: {e}"
        )))
    })
}

fn is_replicate_version_id(model: &str) -> bool {
    model.len() == 64 && model.bytes().all(|b| b.is_ascii_hexdigit())
}

fn media_urls_from_result(kind: &crate::providers::ProviderKind, value: &Value) -> Value {
    let mut urls = serde_json::Map::new();
    if let Some(source) = value.get("urls").and_then(Value::as_object) {
        for (key, value) in source {
            urls.insert(key.clone(), value.clone());
        }
    }
    if matches!(kind, crate::providers::ProviderKind::Fal) {
        for (source, target) in [
            ("response_url", "response"),
            ("status_url", "status"),
            ("cancel_url", "cancel"),
        ] {
            if let Some(url) = value.get(source).and_then(Value::as_str) {
                urls.insert(target.to_string(), Value::String(url.to_string()));
            }
        }
    }
    Value::Object(urls)
}

fn media_items_from_result(value: &Value, kind_hint: &str) -> Vec<MediaItem> {
    let mut urls = Vec::new();
    if let Some(output) = value.get("output") {
        collect_media_urls(output, &mut urls);
    }
    if let Some(images) = value.get("images") {
        collect_media_urls(images, &mut urls);
    }
    if let Some(video) = value.get("video") {
        collect_media_urls(video, &mut urls);
    }
    if let Some(audio) = value.get("audio") {
        collect_media_urls(audio, &mut urls);
    }
    if let Some(unsigned_urls) = value.get("unsigned_urls") {
        collect_media_urls(unsigned_urls, &mut urls);
    }
    if let Some(choices) = value.get("choices") {
        collect_media_urls(choices, &mut urls);
    }
    urls.sort();
    urls.dedup();
    urls.into_iter()
        .map(|url| MediaItem {
            kind: media_kind_from_url(&url, kind_hint).to_string(),
            mime: media_mime_from_url(&url).map(str::to_string),
            url,
            requires_auth: false,
        })
        .collect()
}

fn openrouter_video_media_items(provider_id: &str, id: &str, value: &Value) -> Vec<MediaItem> {
    let has_output = value
        .get("unsigned_urls")
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
        || value.get("output").is_some_and(|output| !output.is_null());
    if !has_output {
        return Vec::new();
    }
    vec![MediaItem {
        url: format!(
            "/media/content?provider_id={}&id={}&index=0",
            percent_encode_query(provider_id),
            percent_encode_query(id)
        ),
        kind: "video".to_string(),
        mime: Some("video/mp4".to_string()),
        requires_auth: true,
    }]
}

fn percent_encode_query(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

fn collect_media_urls(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.starts_with("http://")
                || text.starts_with("https://")
                || text.starts_with("data:image/")
                || text.starts_with("data:video/")
                || text.starts_with("data:audio/")
            {
                out.push(text.to_string());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_media_urls(value, out);
            }
        }
        Value::Object(map) => {
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                out.push(url.to_string());
            }
            for value in map.values() {
                collect_media_urls(value, out);
            }
        }
        _ => {}
    }
}

fn media_kind_from_url(url: &str, kind_hint: &str) -> &'static str {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("data:audio/")
        || [".mp3", ".wav", ".flac", ".ogg", ".m4a", ".aac", ".opus"]
            .iter()
            .any(|extension| lower.contains(extension))
    {
        "music"
    } else if lower.starts_with("data:video/")
        || lower.contains(".mp4")
        || lower.contains(".webm")
        || lower.contains(".mov")
    {
        "video"
    } else if matches!(kind_hint, "image" | "video" | "music") {
        match kind_hint {
            "video" => "video",
            "music" => "music",
            _ => "image",
        }
    } else {
        "image"
    }
}

fn media_mime_from_url(url: &str) -> Option<&'static str> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("data:image/png") || lower.contains(".png") {
        Some("image/png")
    } else if lower.starts_with("data:image/jpeg")
        || lower.contains(".jpg")
        || lower.contains(".jpeg")
    {
        Some("image/jpeg")
    } else if lower.starts_with("data:image/webp") || lower.contains(".webp") {
        Some("image/webp")
    } else if lower.starts_with("data:image/gif") || lower.contains(".gif") {
        Some("image/gif")
    } else if lower.starts_with("data:video/mp4") || lower.contains(".mp4") {
        Some("video/mp4")
    } else if lower.starts_with("data:video/webm") || lower.contains(".webm") {
        Some("video/webm")
    } else if lower.starts_with("data:video/quicktime") || lower.contains(".mov") {
        Some("video/quicktime")
    } else if lower.starts_with("data:audio/mpeg")
        || lower.starts_with("data:audio/mp3")
        || lower.contains(".mp3")
    {
        Some("audio/mpeg")
    } else if lower.starts_with("data:audio/wav")
        || lower.starts_with("data:audio/x-wav")
        || lower.contains(".wav")
    {
        Some("audio/wav")
    } else if lower.starts_with("data:audio/flac")
        || lower.starts_with("data:audio/x-flac")
        || lower.contains(".flac")
    {
        Some("audio/flac")
    } else if lower.starts_with("data:audio/ogg") || lower.contains(".ogg") {
        Some("audio/ogg")
    } else if lower.starts_with("data:audio/mp4")
        || lower.starts_with("data:audio/x-m4a")
        || lower.contains(".m4a")
    {
        Some("audio/mp4")
    } else if lower.starts_with("data:audio/aac") || lower.contains(".aac") {
        Some("audio/aac")
    } else if lower.starts_with("data:audio/opus") || lower.contains(".opus") {
        Some("audio/opus")
    } else {
        None
    }
}

// ----- MCP servers (external MCP client connections) -----

#[derive(Deserialize)]
pub(crate) struct McpServerUpsert {
    #[serde(default)]
    id: Option<String>,
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<milim_mcp_client::McpEnvVar>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

/// `GET /mcp/servers` — list configured MCP servers with connection status.
pub(crate) async fn mcp_servers_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let list = match &st.mcp {
        Some(hub) => hub.list(),
        None => Vec::new(),
    };
    Ok(Json(json!({ "servers": list })).into_response())
}

/// `POST /mcp/servers` — add or update an MCP server (connects immediately).
pub(crate) async fn mcp_server_upsert(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpServerUpsert>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = milim_mcp_client::McpServerConfig {
        id: req.id.unwrap_or_default(),
        name: req.name,
        command: req.command,
        args: req.args,
        cwd: req.cwd,
        env: req.env,
        enabled: req.enabled,
    };
    let saved = hub.upsert(cfg).await.map_err(ApiError)?;
    let info = hub.list().into_iter().find(|s| s.id == saved.id);
    Ok(Json(json!({ "server": info })).into_response())
}

/// `POST /mcp/servers/test` — test a draft MCP server without saving/enabling it.
pub(crate) async fn mcp_server_test_draft(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<McpServerUpsert>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = milim_mcp_client::McpServerConfig {
        id: req.id.unwrap_or_default(),
        name: req.name,
        command: req.command,
        args: req.args,
        cwd: req.cwd,
        env: req.env,
        enabled: req.enabled,
    };
    Ok(Json(hub.test_config(cfg).await).into_response())
}

/// `POST /mcp/servers/{id}/test` — test a saved MCP server without enabling it.
pub(crate) async fn mcp_server_test_saved(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    let cfg = hub
        .config(&id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("mcp server {id}"))))?;
    Ok(Json(hub.test_config(cfg).await).into_response())
}

/// `DELETE /mcp/servers/{id}` — remove an MCP server (disconnects it).
pub(crate) async fn mcp_server_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let hub = st
        .mcp
        .as_ref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("MCP client is not enabled".into())))?;
    Ok(Json(json!({ "deleted": hub.remove(&id).map_err(ApiError)? })).into_response())
}

fn register_mcp_server_tools(registry: &mut ToolRegistry, hub: Arc<milim_mcp_client::McpHub>) {
    registry.register(Arc::new(McpServerListTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerTestTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerSaveTool { hub: hub.clone() }));
    registry.register(Arc::new(McpServerDeleteTool { hub }));
}

struct McpServerListTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerTestTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerSaveTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

struct McpServerDeleteTool {
    hub: Arc<milim_mcp_client::McpHub>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolEnv {
    key: String,
    value: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolSecretEnv {
    key: String,
    #[serde(default = "default_true")]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerToolConfig {
    #[serde(default)]
    id: Option<String>,
    name: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: Vec<McpServerToolEnv>,
    #[serde(default)]
    secret_env: Vec<McpServerToolSecretEnv>,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct McpServerDeleteToolArgs {
    id: String,
}

impl McpServerToolConfig {
    fn into_config(
        self,
        hub: &milim_mcp_client::McpHub,
        update_must_exist: bool,
    ) -> milim_core::Result<milim_mcp_client::McpServerConfig> {
        let id = self
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if update_must_exist && id.as_deref().is_some_and(|id| hub.config(id).is_none()) {
            return Err(Error::ModelNotFound(format!(
                "mcp server {}",
                id.as_deref().unwrap_or_default()
            )));
        }

        let mut keys = HashSet::new();
        let mut env = Vec::with_capacity(self.env.len() + self.secret_env.len());
        for item in self.env {
            let key = trim_required_tool_arg(item.key, "env[].key")?;
            if milim_mcp_client::secret_env_key(&key) {
                return Err(Error::InvalidRequest(format!(
                    "environment variable {key} looks secret; declare it in secret_env without a value"
                )));
            }
            if !keys.insert(key.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate environment variable: {key}"
                )));
            }
            env.push(milim_mcp_client::McpEnvVar {
                key,
                value: Some(item.value),
                secret: false,
                required: item.required,
                has_value: false,
            });
        }
        for item in self.secret_env {
            let key = trim_required_tool_arg(item.key, "secret_env[].key")?;
            if !keys.insert(key.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate environment variable: {key}"
                )));
            }
            env.push(milim_mcp_client::McpEnvVar {
                key,
                value: None,
                secret: true,
                required: item.required,
                has_value: false,
            });
        }

        Ok(milim_mcp_client::McpServerConfig {
            id: id.unwrap_or_default(),
            name: trim_required_tool_arg(self.name, "name")?,
            command: trim_required_tool_arg(self.command, "command")?,
            args: self.args,
            cwd: self
                .cwd
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
            env,
            enabled: self.enabled,
        })
    }
}

fn mcp_server_config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Existing server id. Omit to create or test a new server." },
            "name": { "type": "string", "description": "Human-readable server name." },
            "command": { "type": "string", "description": "Executable to run, such as npx, uvx, node, or an absolute executable path." },
            "args": { "type": "array", "items": { "type": "string" }, "description": "Exact command arguments." },
            "cwd": { "type": ["string", "null"], "description": "Optional working directory. Use the active workspace for a locally-authored server." },
            "env": {
                "type": "array",
                "description": "Non-secret environment variables. Credential-looking keys are rejected.",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "value": { "type": "string" },
                        "required": { "type": "boolean" }
                    },
                    "required": ["key", "value"],
                    "additionalProperties": false
                }
            },
            "secret_env": {
                "type": "array",
                "description": "Secret environment variable placeholders. Values must be entered later in Milim's encrypted MCP Manager.",
                "items": {
                    "type": "object",
                    "properties": {
                        "key": { "type": "string" },
                        "required": { "type": "boolean", "description": "Defaults to true." }
                    },
                    "required": ["key"],
                    "additionalProperties": false
                }
            },
            "enabled": { "type": "boolean", "description": "Connect after saving. Defaults to true." }
        },
        "required": ["name", "command"],
        "additionalProperties": false
    })
}

#[async_trait]
impl Tool for McpServerListTool {
    fn name(&self) -> &str {
        "mcp_server_list"
    }

    fn description(&self) -> &str {
        "List Milim-managed MCP servers, connection status, capabilities, and sanitized environment metadata. Secret values are never returned."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        Ok(json!({ "ok": true, "servers": self.hub.list() }))
    }
}

#[async_trait]
impl Tool for McpServerTestTool {
    fn name(&self) -> &str {
        "mcp_server_test"
    }

    fn description(&self) -> &str {
        "Launch and test an MCP server configuration without saving it. Use an existing id to reuse its encrypted secret placeholders. Requires command approval."
    }

    fn input_schema(&self) -> Value {
        mcp_server_config_schema()
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerToolConfig = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_test arguments: {error}"))
        })?;
        Ok(json!(
            self.hub
                .test_config(args.into_config(&self.hub, false)?)
                .await
        ))
    }
}

#[async_trait]
impl Tool for McpServerSaveTool {
    fn name(&self) -> &str {
        "mcp_server_save"
    }

    fn description(&self) -> &str {
        "Create or fully replace a Milim-managed MCP server. Omit id to create; list first and include id to update. Connected tools become callable on the next chat turn. Requires command approval."
    }

    fn input_schema(&self) -> Value {
        mcp_server_config_schema()
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerToolConfig = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_save arguments: {error}"))
        })?;
        let saved = self.hub.upsert(args.into_config(&self.hub, true)?).await?;
        let server = self
            .hub
            .list()
            .into_iter()
            .find(|server| server.id == saved.id)
            .ok_or_else(|| Error::ModelNotFound(format!("mcp server {}", saved.id)))?;
        let tools_available_next_turn = server.connected && server.tool_count > 0;
        Ok(json!({
            "ok": true,
            "server": server,
            "tools_available_next_turn": tools_available_next_turn
        }))
    }
}

#[async_trait]
impl Tool for McpServerDeleteTool {
    fn name(&self) -> &str {
        "mcp_server_delete"
    }

    fn description(&self) -> &str {
        "Disconnect and permanently remove a Milim-managed MCP server by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "id": { "type": "string", "description": "Existing server id." } },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: McpServerDeleteToolArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid mcp_server_delete arguments: {error}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        if !self.hub.remove(&id)? {
            return Err(Error::ModelNotFound(format!("mcp server {id}")));
        }
        Ok(json!({ "ok": true, "deleted": true, "id": id }))
    }
}

#[cfg(test)]
mod mcp_server_tool_tests {
    use super::*;

    fn hub() -> (PathBuf, Arc<milim_mcp_client::McpHub>) {
        let root = std::env::temp_dir().join(format!(
            "milim-mcp-chat-tools-{}",
            uuid::Uuid::new_v4().simple()
        ));
        (root.clone(), Arc::new(milim_mcp_client::McpHub::open(root)))
    }

    #[tokio::test]
    async fn manages_servers_and_keeps_secret_values_hidden() {
        let (root, hub) = hub();
        let mut tools = ToolRegistry::new();
        register_mcp_server_tools(&mut tools, hub.clone());

        assert_eq!(tools.effect("mcp_server_list"), Some(ToolEffect::ReadOnly));
        assert_eq!(tools.effect("mcp_server_test"), Some(ToolEffect::Command));
        assert_eq!(tools.effect("mcp_server_save"), Some(ToolEffect::Command));
        assert_eq!(
            tools.effect("mcp_server_delete"),
            Some(ToolEffect::Mutating)
        );
        assert!(tools.read_only().contains("mcp_server_list"));
        assert!(!tools.read_only().contains("mcp_server_save"));

        let created = tools
            .call(
                "mcp_server_save",
                json!({
                    "name": "Local fixture",
                    "command": "missing-but-disabled",
                    "env": [{ "key": "BASE_URL", "value": "https://example.com" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        let id = created["server"]["id"].as_str().unwrap().to_string();
        assert_eq!(created["server"]["enabled"], false);
        assert_eq!(created["tools_available_next_turn"], false);

        let updated = tools
            .call(
                "mcp_server_save",
                json!({
                    "id": id,
                    "name": "Renamed fixture",
                    "command": "missing-but-disabled",
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        assert_eq!(updated["server"]["name"], "Renamed fixture");

        tools
            .call("mcp_server_delete", json!({ "id": id }))
            .await
            .unwrap();
        assert!(hub.list().is_empty());

        hub.upsert(milim_mcp_client::McpServerConfig {
            id: "secret-server".into(),
            name: "Secret fixture".into(),
            command: "missing-but-disabled".into(),
            args: Vec::new(),
            cwd: None,
            env: vec![milim_mcp_client::McpEnvVar {
                key: "API_TOKEN".into(),
                value: Some("encrypted-value".into()),
                secret: true,
                required: true,
                has_value: false,
            }],
            enabled: false,
        })
        .await
        .unwrap();
        tools
            .call(
                "mcp_server_save",
                json!({
                    "id": "secret-server",
                    "name": "Secret fixture updated",
                    "command": "missing-but-disabled",
                    "secret_env": [{ "key": "API_TOKEN" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap();
        let listed = tools.call("mcp_server_list", json!({})).await.unwrap();
        let secret = &listed["servers"][0]["env"][0];
        assert_eq!(secret["key"], "API_TOKEN");
        assert_eq!(secret["has_value"], true);
        assert!(secret.get("value").is_none());

        let rejected = tools
            .call(
                "mcp_server_save",
                json!({
                    "name": "Leaky fixture",
                    "command": "missing-but-disabled",
                    "env": [{ "key": "API_KEY", "value": "must-not-save" }],
                    "enabled": false
                }),
            )
            .await
            .unwrap_err();
        assert!(rejected.to_string().contains("declare it in secret_env"));

        let missing_update = tools
            .call(
                "mcp_server_save",
                json!({
                    "id": "missing",
                    "name": "Missing",
                    "command": "missing-but-disabled",
                    "enabled": false
                }),
            )
            .await
            .unwrap_err();
        assert!(missing_update.to_string().contains("mcp server missing"));

        drop(tools);
        drop(hub);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn tests_drafts_without_saving_them() {
        let (root, hub) = hub();
        let mut tools = ToolRegistry::new();
        register_mcp_server_tools(&mut tools, hub.clone());

        let missing_env = tools
            .call(
                "mcp_server_test",
                json!({
                    "name": "Missing secret",
                    "command": "unused",
                    "secret_env": [{ "key": "API_TOKEN" }]
                }),
            )
            .await
            .unwrap();
        assert_eq!(missing_env["ok"], false);
        assert_eq!(missing_env["missing_env"][0], "API_TOKEN");

        let failed = tools
            .call(
                "mcp_server_test",
                json!({
                    "name": "Missing command",
                    "command": "__milim_missing_mcp_command__"
                }),
            )
            .await
            .unwrap();
        assert_eq!(failed["ok"], false);
        assert!(!failed["error"].as_str().unwrap().is_empty());
        assert!(hub.list().is_empty());

        drop(tools);
        drop(hub);
        let _ = std::fs::remove_dir_all(root);
    }
}

// ----- Agents -----

#[derive(Serialize)]
struct AgentRunResponse {
    id: String,
    object: &'static str,
    model: String,
    message: ChatMessage,
    steps: Vec<milim_agents::ToolStep>,
    iterations: usize,
    stopped_at_limit: bool,
}

#[derive(Debug, Clone, Default)]
struct AgentMemoryContext {
    enabled: bool,
    model: String,
    thread_id: Option<String>,
    project_locator: Option<String>,
    project_label: Option<String>,
    message_id: Option<String>,
    delegation_policy: milim_agents::DelegationPolicy,
    worker_model: Option<String>,
    worker_context: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AccountRuntimeMilimContext {
    #[serde(default)]
    tool_context: AccountRuntimeToolContext,
    #[serde(default)]
    memory_context: AccountRuntimeMemoryContext,
    #[serde(default = "default_tool_mode")]
    tool_mode: String,
    #[serde(default)]
    enabled_tools: Vec<String>,
    #[serde(default = "default_skill_mode")]
    skill_mode: String,
    #[serde(default)]
    enabled_skills: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AccountRuntimeToolContext {
    tool_approval_policy: Option<String>,
    #[serde(default)]
    tool_approval_grant: bool,
    #[serde(default)]
    interactive_tool_approval: bool,
    #[serde(default)]
    sandbox_enabled: bool,
    #[serde(default)]
    computer_use_enabled: bool,
    #[serde(default)]
    preview_tools_enabled: bool,
    #[serde(default)]
    experimental_hashline_patch: bool,
    #[serde(default)]
    plan_mode: bool,
    delegation_policy: Option<String>,
    worker_model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AccountRuntimeMemoryContext {
    #[serde(default)]
    memory_enabled: bool,
    thread_id: Option<String>,
    project_locator: Option<String>,
    project_label: Option<String>,
    message_id: Option<String>,
}

fn default_tool_mode() -> String {
    "all".to_string()
}

fn default_skill_mode() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone)]
pub(crate) struct AccountRuntimeToolEndpoint {
    pub run_id: String,
    pub url: String,
    pub authorization: String,
    pub tools: Vec<milim_tools::ToolSpec>,
}

fn account_runtime_tool_endpoint(
    st: &AppState,
    headers: &HeaderMap,
    context: Option<&AccountRuntimeMilimContext>,
    model: &str,
    prompt: &str,
) -> Result<Option<AccountRuntimeToolEndpoint>, ApiError> {
    let Some(context) = context else {
        return Ok(None);
    };
    let approval = match context.tool_context.tool_approval_policy.as_deref() {
        Some("review") => ToolApprovalPolicy::Review,
        Some("open") => ToolApprovalPolicy::Open,
        _ => ToolApprovalPolicy::Guarded,
    };
    let policy = ToolRunPolicy {
        approval,
        approval_granted: context.tool_context.tool_approval_grant,
        interactive_approval: context.tool_context.interactive_tool_approval,
        sandbox_enabled: context.tool_context.sandbox_enabled,
        computer_use_enabled: context.tool_context.computer_use_enabled,
        preview_tools_enabled: context.tool_context.preview_tools_enabled,
        experimental_hashline_patch: context.tool_context.experimental_hashline_patch,
        plan_mode: context.tool_context.plan_mode,
    };
    let memory = AgentMemoryContext {
        enabled: context.memory_context.memory_enabled,
        model: model.to_string(),
        thread_id: context.memory_context.thread_id.clone(),
        project_locator: context.memory_context.project_locator.clone(),
        project_label: context.memory_context.project_label.clone(),
        message_id: context.memory_context.message_id.clone(),
        delegation_policy: match context.tool_context.delegation_policy.as_deref() {
            Some("off") => milim_agents::DelegationPolicy::Off,
            Some("auto") => milim_agents::DelegationPolicy::Auto,
            _ => milim_agents::DelegationPolicy::Ask,
        },
        worker_model: context.tool_context.worker_model.clone(),
        worker_context: Some(prompt.chars().take(32_000).collect()),
    };
    let mut registry = agent_registry_for_mode(
        st,
        &context.tool_mode,
        &context.enabled_tools,
        Some(memory),
        &policy,
    )
    .without(DESKTOP_WORKSPACE_TOOL_NAMES);
    if !policy.plan_mode {
        register_skill_tools(
            &mut registry,
            st,
            &context.skill_mode,
            &context.enabled_skills,
        );
    }
    if registry.is_empty() {
        return Ok(None);
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(loopback_host)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "Milim account-runtime tools require a loopback server address".into(),
            ))
        })?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let endpoint = AccountRuntimeToolEndpoint {
        run_id: run_id.clone(),
        url: format!("http://{host}/internal/account-runtime-tools/{run_id}/mcp"),
        authorization: format!("Bearer {token}"),
        tools: registry.list(),
    };
    st.account_runtime_tools
        .lock()
        .expect("account runtime tool store poisoned")
        .insert(
            run_id,
            crate::state::AccountRuntimeToolSession {
                token,
                registry,
                review: approval == ToolApprovalPolicy::Review && !policy.approval_granted,
            },
        );
    Ok(Some(endpoint))
}

struct AccountRuntimeToolLease {
    sessions: Arc<Mutex<HashMap<String, crate::state::AccountRuntimeToolSession>>>,
    run_id: Option<String>,
}

impl Drop for AccountRuntimeToolLease {
    fn drop(&mut self) {
        if let Some(run_id) = &self.run_id {
            self.sessions
                .lock()
                .expect("account runtime tool store poisoned")
                .remove(run_id);
        }
    }
}

fn account_runtime_stream<S>(
    stream: S,
    st: &AppState,
    endpoint: Option<&AccountRuntimeToolEndpoint>,
    relay_notices: bool,
) -> impl futures::Stream<Item = std::result::Result<Event, Infallible>>
where
    S: futures::Stream<Item = std::result::Result<Event, Infallible>>,
{
    let run_id = endpoint.map(|endpoint| endpoint.run_id.clone());
    let sessions = st.account_runtime_tools.clone();
    let mut notices = st.tool_approvals.subscribe();
    async_stream::stream! {
        let _lease = AccountRuntimeToolLease { sessions, run_id: run_id.clone() };
        futures::pin_mut!(stream);
        loop {
            tokio::select! {
                event = stream.next() => match event {
                    Some(event) => yield event,
                    None => break,
                },
                notice = notices.recv(), if relay_notices && run_id.is_some() => match notice {
                    Ok(notice) if Some(notice.run_id.as_str()) == run_id.as_deref() => {
                        let value = if let Some(decision) = notice.decision {
                            json!({
                                "type": "tool_approval_resolved",
                                "approval_id": notice.approval_id,
                                "call_id": notice.call_id.unwrap_or_else(|| notice.approval_id.clone()),
                                "decision": decision,
                            })
                        } else {
                            json!({
                                "type": "tool_approval_required",
                                "approval_id": notice.approval_id,
                                "call_id": notice.call_id.unwrap_or_else(|| notice.approval_id.clone()),
                                "name": notice.name,
                                "arguments": notice.arguments,
                                "effect": notice.effect,
                            })
                        };
                        yield Ok(Event::default().data(value.to_string()));
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod account_runtime_tool_tests {
    use super::*;
    use milim_storage::Database;

    #[test]
    fn gateway_requires_loopback_and_removes_finished_sessions() {
        assert_eq!(
            loopback_host("127.0.0.1:1234").as_deref(),
            Some("127.0.0.1:1234")
        );
        assert!(loopback_host("example.com:1234").is_none());

        let sessions = Arc::new(Mutex::new(HashMap::from([(
            "run".to_string(),
            crate::state::AccountRuntimeToolSession {
                token: "token".into(),
                registry: ToolRegistry::new(),
                review: false,
            },
        )])));
        drop(AccountRuntimeToolLease {
            sessions: sessions.clone(),
            run_id: Some("run".into()),
        });
        assert!(sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn lazy_skill_tools_enforce_custom_agent_scope() {
        let store =
            Arc::new(milim_skills::SkillStore::new(Database::open_in_memory().unwrap()).unwrap());
        let allowed = store
            .create(
                "Code Review",
                "Review source changes",
                "List findings first.",
            )
            .unwrap();
        let blocked = store
            .create("Deployment", "Deploy releases", "Push the release.")
            .unwrap();
        let scope = MilimSkillScope {
            allowed_ids: Some(Arc::new(HashSet::from([allowed.id.clone()]))),
        };
        let search = MilimSkillSearchTool {
            store: store.clone(),
            scope: scope.clone(),
        };
        let found = search
            .invoke(json!({ "query": "review source changes", "limit": 10 }))
            .await
            .unwrap();
        assert_eq!(found["skills"].as_array().unwrap().len(), 1);
        assert_eq!(found["skills"][0]["id"], allowed.id);

        let read = MilimSkillReadTool { store, scope };
        let loaded = read.invoke(json!({ "id": allowed.id })).await.unwrap();
        assert_eq!(loaded["instructions"], "List findings first.");
        assert!(read.invoke(json!({ "id": blocked.id })).await.is_err());
    }
}

const DESKTOP_WORKSPACE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "read_file_anchors",
    "list_dir",
    "write_file",
    "edit_file",
    "patch_file",
    "shell",
];
const HASHLINE_TOOL_NAMES: &[&str] = &["read_file_anchors", "patch_file"];
const SANDBOX_TOOL_NAMES: &[&str] = &["run_command"];
const COMPUTER_TOOL_NAMES: &[&str] = &[
    "screenshot",
    "mouse_move",
    "mouse_click",
    "key_press",
    "type_text",
    "scroll",
];
const PREVIEW_TOOL_NAMES: &[&str] = &[
    "preview_dom_snapshot",
    "preview_click",
    "preview_type_text",
    "preview_key_press",
    "preview_scroll",
];
const CHILD_THREAD_TOOL_NAMES: &[&str] = &[
    "delegate_workers",
    "child_thread_spawn",
    "child_thread_list",
    "child_thread_read",
    "child_thread_wait",
    "child_thread_stop",
];
const CHILD_THREAD_READ_ONLY_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "http_fetch",
    "current_time",
    "echo",
];
const PLAN_MODE_READ_ONLY_TOOL_NAMES: &[&str] = &["read_file", "list_dir"];
const MAX_CHILD_THREAD_WAIT_MS: u64 = 300_000;
const WORKSPACE_UNAVAILABLE_SYSTEM_PROMPT: &str = concat!(
    "No working folder is selected in Milim. Host filesystem and host shell tools are unavailable. ",
    "If the user asks to create a new file, web app, document, dataset, or other generated artifact ",
    "that is not tied to existing local project files, return it inline as a named fenced code block ",
    "such as ```html file=index.html ... ``` so Milim can capture it in the current chat's artifact panel. ",
    "For browser apps, use index.html plus sibling CSS/JS/TS/TSX files when that is clearer; ",
    "the preview resolves relative links and imports across those artifacts. ",
    "Ask them to pick a folder with the Folder chip only when they want you to read, write, edit, list, ",
    "run commands, or save directly against existing project files."
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolApprovalPolicy {
    Review,
    Guarded,
    Open,
}

#[derive(Clone, Copy, Debug)]
struct ToolRunPolicy {
    approval: ToolApprovalPolicy,
    approval_granted: bool,
    interactive_approval: bool,
    sandbox_enabled: bool,
    computer_use_enabled: bool,
    preview_tools_enabled: bool,
    experimental_hashline_patch: bool,
    plan_mode: bool,
}

impl Default for ToolRunPolicy {
    fn default() -> Self {
        Self {
            approval: ToolApprovalPolicy::Guarded,
            approval_granted: false,
            interactive_approval: false,
            sandbox_enabled: false,
            computer_use_enabled: false,
            preview_tools_enabled: false,
            experimental_hashline_patch: false,
            plan_mode: false,
        }
    }
}

fn string_extra(req: &ChatCompletionRequest, key: &str) -> Option<String> {
    req.extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_list_extra(req: &ChatCompletionRequest, key: &str) -> Vec<String> {
    req.extra
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn bool_extra(req: &ChatCompletionRequest, key: &str) -> bool {
    req.extra.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn tool_run_policy_from_request(req: &ChatCompletionRequest) -> ToolRunPolicy {
    let approval = match string_extra(req, "tool_approval_policy").as_deref() {
        Some("review") => ToolApprovalPolicy::Review,
        Some("open") => ToolApprovalPolicy::Open,
        _ => ToolApprovalPolicy::Guarded,
    };
    ToolRunPolicy {
        approval,
        approval_granted: bool_extra(req, "tool_approval_grant"),
        interactive_approval: bool_extra(req, "interactive_tool_approval"),
        sandbox_enabled: bool_extra(req, "sandbox_enabled"),
        computer_use_enabled: bool_extra(req, "computer_use_enabled"),
        preview_tools_enabled: bool_extra(req, "preview_tools_enabled"),
        experimental_hashline_patch: bool_extra(req, "experimental_hashline_patch"),
        plan_mode: bool_extra(req, "plan_mode"),
    }
}

fn memory_context_from_request(req: &ChatCompletionRequest, model: String) -> AgentMemoryContext {
    AgentMemoryContext {
        enabled: bool_extra(req, "memory_enabled"),
        model,
        thread_id: string_extra(req, "thread_id"),
        project_locator: string_extra(req, "project_locator"),
        project_label: string_extra(req, "project_label"),
        message_id: string_extra(req, "message_id"),
        delegation_policy: match string_extra(req, "delegation_policy").as_deref() {
            Some("off") => milim_agents::DelegationPolicy::Off,
            Some("auto") => milim_agents::DelegationPolicy::Auto,
            _ => milim_agents::DelegationPolicy::Ask,
        },
        worker_model: string_extra(req, "worker_model"),
        worker_context: worker_context_from_request(req),
    }
}

fn worker_context_from_request(req: &ChatCompletionRequest) -> Option<String> {
    let instructions = req
        .messages
        .iter()
        .filter(|message| message.role == "system")
        .map(ChatMessage::text_content)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let request = req
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(ChatMessage::text_content)
        .unwrap_or_default();
    let context = [
        (!request.trim().is_empty()).then(|| format!("Current request:\n{request}")),
        (!instructions.trim().is_empty())
            .then(|| format!("Resolved instructions and skills:\n{instructions}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n");
    (!context.is_empty()).then(|| context.chars().take(32_000).collect())
}

fn workspace_is_selected(st: &AppState) -> bool {
    workspace_snapshot(st).is_some()
}

fn workspace_snapshot(st: &AppState) -> Option<PathBuf> {
    st.workspace.read().ok().and_then(|guard| guard.clone())
}

fn static_registry_for_run(st: &AppState) -> ToolRegistry {
    let reg = st.tools.as_deref().cloned().unwrap_or_default();
    workspace_snapshot(st)
        .map(|root| reg.scoped_to_workspace(&root))
        .unwrap_or(reg)
        .scoped_for_run()
}

fn registry_has_desktop_host_tools(reg: &ToolRegistry) -> bool {
    reg.contains("edit_file") || reg.contains("patch_file") || reg.contains("shell")
}

fn desktop_workspace_unavailable(st: &AppState) -> bool {
    !workspace_is_selected(st)
        && st
            .tools
            .as_ref()
            .map(|reg| registry_has_desktop_host_tools(reg))
            .unwrap_or(false)
}

fn add_workspace_notice_if_needed(messages: &mut Vec<ChatMessage>, workspace_unavailable: bool) {
    if !workspace_unavailable {
        return;
    }
    let insert_at = messages
        .iter()
        .position(|message| message.role != "system")
        .unwrap_or(messages.len());
    messages.insert(
        insert_at,
        ChatMessage::text("system", WORKSPACE_UNAVAILABLE_SYSTEM_PROMPT),
    );
}

fn add_workspace_instructions(messages: &mut Vec<ChatMessage>, st: &AppState) {
    let folder = workspace_snapshot(st);
    let context = crate::workspace_context::resolve(folder.as_deref());
    let Some(instructions) = crate::workspace_context::formatted(&context, None) else {
        return;
    };
    messages.insert(0, ChatMessage::text("system", instructions));
}

/// The effective tool registry for an agent run: the static tools (builtins,
/// host fs/shell, Docker sandbox) plus any tools exposed by connected MCP
/// servers. Rebuilt per-run (cheap clone) so newly-added MCP servers are
/// picked up without restarting the app.
fn agent_registry_with_memory(
    st: &AppState,
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
) -> ToolRegistry {
    agent_registry_for_mode(st, "all", &[], memory, policy)
}

fn agent_base_registry_with_memory(
    st: &AppState,
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
) -> ToolRegistry {
    let mut reg = static_registry_for_run(st);
    let workspace_unavailable = desktop_workspace_unavailable(st);
    if policy.plan_mode {
        return plan_mode_registry(
            reg,
            workspace_unavailable,
            policy.experimental_hashline_patch,
        );
    }
    if let Some(hub) = &st.mcp {
        register_mcp_server_tools(&mut reg, hub.clone());
        for tool in hub.tools() {
            if let Err(error) = reg.try_register(tool) {
                tracing::warn!("skipping colliding MCP tool: {error}");
            }
        }
    }
    if let Some(store) = st.schedules.as_ref() {
        register_schedule_tools(&mut reg, store.clone(), workspace_snapshot(st));
    }
    if let (Some(memory), Some(store)) = (memory.clone(), st.memory.as_ref()) {
        if memory.enabled {
            reg.register(Arc::new(MemoryRegisterTool {
                store: store.clone(),
                context: memory,
            }));
        }
    }
    if workspace_unavailable && registry_has_desktop_host_tools(&reg) {
        reg = reg.without(DESKTOP_WORKSPACE_TOOL_NAMES);
    }
    if !policy.sandbox_enabled {
        reg = reg.without(SANDBOX_TOOL_NAMES);
    }
    if !policy.computer_use_enabled {
        reg = reg.without(COMPUTER_TOOL_NAMES);
    }
    if !policy.preview_tools_enabled {
        reg = reg.without(PREVIEW_TOOL_NAMES);
    }
    if !policy.experimental_hashline_patch {
        reg = reg.without(HASHLINE_TOOL_NAMES);
    }
    if policy.approval == ToolApprovalPolicy::Review
        && !policy.approval_granted
        && !policy.interactive_approval
    {
        reg = ToolRegistry::new();
    } else if policy.approval == ToolApprovalPolicy::Guarded {
        reg = reg.read_only();
    }
    reg
}

fn tools_available(policy: &ToolRunPolicy) -> bool {
    (policy.approval_granted
        || policy.interactive_approval
        || policy.approval != ToolApprovalPolicy::Review)
        && !policy.plan_mode
}

#[derive(Deserialize)]
pub(crate) struct ToolApprovalDecision {
    decision: String,
    #[serde(default)]
    response: Option<Value>,
}

pub(crate) async fn tool_approval_resolve(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ToolApprovalDecision>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let approved = match req.decision.trim() {
        "approve" => true,
        "deny" => false,
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "decision must be approve or deny".to_string(),
            )))
        }
    };
    match st
        .tool_approvals
        .resolve_with_response(&id, approved, req.response)
    {
        milim_agents::ApprovalResolve::Resolved => Ok(StatusCode::NO_CONTENT.into_response()),
        milim_agents::ApprovalResolve::AlreadyResolved => Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "tool approval already resolved" })),
        )
            .into_response()),
        milim_agents::ApprovalResolve::Missing => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "tool approval not found or expired" })),
        )
            .into_response()),
    }
}

fn plan_mode_registry(
    reg: ToolRegistry,
    workspace_unavailable: bool,
    anchored_reads_enabled: bool,
) -> ToolRegistry {
    let mut allowed: Vec<String> = PLAN_MODE_READ_ONLY_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    if anchored_reads_enabled {
        allowed.push("read_file_anchors".to_string());
    }
    let mut reg = reg.filtered(&allowed);
    if workspace_unavailable {
        reg = reg.without(&["read_file", "list_dir", "read_file_anchors"]);
    }
    reg
}

fn agent_registry_for_mode(
    st: &AppState,
    tool_mode: &str,
    enabled_tools: &[String],
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
) -> ToolRegistry {
    let all = agent_base_registry_with_memory(st, memory.clone(), policy);
    let normalized = milim_agents::normalize_tool_mode(tool_mode, enabled_tools);
    let inherited = match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => all.filtered(enabled_tools),
        _ => all,
    };
    let mut reg = inherited.clone();
    if let (Some(memory), Some(supervisor)) = (memory, st.threads.as_ref()) {
        if tools_available(policy) && child_thread_tools_allowed(supervisor, &memory) {
            register_child_thread_tools(
                &mut reg,
                st.clone(),
                supervisor.clone(),
                memory,
                child_registry_for_policy(st, policy, inherited),
                policy.approval == ToolApprovalPolicy::Open
                    || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted),
            );
        }
    }
    match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => reg.filtered(enabled_tools),
        _ => reg,
    }
}

pub(crate) fn scheduled_agent_registry(
    st: &AppState,
    agent: &milim_agents::AgentDef,
) -> ToolRegistry {
    let mut registry = agent_registry_for_mode(
        st,
        &agent.tool_mode,
        &agent.enabled_tools,
        None,
        &ToolRunPolicy::default(),
    );
    register_skill_tools(&mut registry, st, &agent.skill_mode, &agent.enabled_skills);
    registry
}

struct MemoryRegisterTool {
    store: Arc<milim_memory::MemoryStore>,
    context: AgentMemoryContext,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRegisterArgs {
    #[serde(default)]
    scope: Option<String>,
    content: String,
    #[serde(default)]
    title: Option<String>,
}

#[async_trait]
impl Tool for MemoryRegisterTool {
    fn name(&self) -> &str {
        "memory_register"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Save concise durable context to Personal or Project memory. Use this only for facts, decisions, preferences, and project context likely to help future turns."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["personal", "project"],
                    "description": "Where to store the memory. Defaults to project when a workspace folder exists, otherwise personal."
                },
                "content": { "type": "string", "description": "One or two sentences with the useful durable context." },
                "title": { "type": "string", "description": "Optional short human-readable title." }
            },
            "required": ["content"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MemoryRegisterArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid memory_register arguments: {e}"))
        })?;
        let content = trim_required_tool_arg(args.content, "content")?;
        let title = args
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                content
                    .lines()
                    .next()
                    .unwrap_or("Memory")
                    .chars()
                    .take(80)
                    .collect()
            });
        let requested_scope = args
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if self.context.project_locator.is_some() {
                    "project".to_string()
                } else {
                    "personal".to_string()
                }
            });

        let (scope_kind, locator, label) = match requested_scope.as_str() {
            "project" => {
                let locator = self.context.project_locator.clone().ok_or_else(|| {
                    Error::InvalidRequest(
                        "project memory requires an active project folder".to_string(),
                    )
                })?;
                let label = self
                    .context
                    .project_label
                    .clone()
                    .unwrap_or_else(|| locator.clone());
                ("project".to_string(), locator, label)
            }
            "personal" => (
                "global".to_string(),
                "personal".to_string(),
                "Personal".to_string(),
            ),
            _ => {
                return Err(Error::InvalidRequest(
                    "memory_register scope must be personal or project".to_string(),
                ))
            }
        };

        let registration = self
            .store
            .register(
                &self.context.model,
                milim_memory::MemoryScopeInput {
                    kind: scope_kind,
                    label,
                    locator,
                },
                milim_memory::MemoryNodeInput {
                    kind: "fact".to_string(),
                    title,
                    body: content,
                    confidence: 0.85,
                    source: "agent".to_string(),
                },
                Vec::new(),
                milim_memory::MemoryEventInput {
                    thread_id: self.context.thread_id.clone().unwrap_or_default(),
                    message_id: self.context.message_id.clone().unwrap_or_default(),
                    summary: String::new(),
                },
            )
            .await?;
        Ok(json!({
            "ok": true,
            "memory": registration.node,
            "scope": registration.scope,
            "memory_notice": registration.notice
        }))
    }
}

fn child_thread_tools_allowed(supervisor: &ThreadSupervisor, context: &AgentMemoryContext) -> bool {
    let Some(thread_id) = context.thread_id.as_deref() else {
        return false;
    };
    supervisor
        .get(thread_id)
        .map(|t| t.is_none())
        .unwrap_or(false)
}

fn register_child_thread_tools(
    reg: &mut ToolRegistry,
    state: AppState,
    supervisor: Arc<ThreadSupervisor>,
    context: AgentMemoryContext,
    child_tools: ToolRegistry,
    allow_write_review: bool,
) {
    if context.delegation_policy != milim_agents::DelegationPolicy::Off {
        reg.register(Arc::new(DelegateWorkersTool {
            state,
            supervisor,
            context,
            child_tools,
            allow_write_review,
        }));
    }
}

fn child_read_only_registry(st: &AppState) -> ToolRegistry {
    let allowed: Vec<String> = CHILD_THREAD_READ_ONLY_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut reg = static_registry_for_run(st).filtered(&allowed);
    if desktop_workspace_unavailable(st) {
        reg = reg.without(&["read_file", "list_dir"]);
    }
    reg
}

fn worker_review_registry(st: &AppState) -> ToolRegistry {
    static_registry_for_run(st)
        .without(CHILD_THREAD_TOOL_NAMES)
        .without(SANDBOX_TOOL_NAMES)
        .without(COMPUTER_TOOL_NAMES)
        .without(PREVIEW_TOOL_NAMES)
}

fn child_registry_for_policy(
    st: &AppState,
    policy: &ToolRunPolicy,
    inherited: ToolRegistry,
) -> ToolRegistry {
    if policy.approval == ToolApprovalPolicy::Open
        || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted)
    {
        inherited.without(CHILD_THREAD_TOOL_NAMES)
    } else {
        child_read_only_registry(st)
    }
}

fn child_thread_parent_id(context: &AgentMemoryContext) -> milim_core::Result<String> {
    context
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            Error::InvalidRequest("child threads require a parent thread id".to_string())
        })
}

fn child_thread_title(title: Option<String>, prompt: &str) -> String {
    title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| prompt.chars().take(80).collect())
}

fn worker_run_event_name(status: milim_agents::WorkerRunStatus) -> &'static str {
    match status {
        milim_agents::WorkerRunStatus::Proposed => "proposed",
        milim_agents::WorkerRunStatus::Running => "started",
        milim_agents::WorkerRunStatus::Done | milim_agents::WorkerRunStatus::Partial => "done",
        milim_agents::WorkerRunStatus::Stopped | milim_agents::WorkerRunStatus::Error => "error",
    }
}

fn worker_run_notice(run: &milim_agents::WorkerRun, workers: &[milim_agents::Worker]) -> Value {
    json!({ "event": worker_run_event_name(run.status), "run": run, "workers": workers, "message": run.error })
}

struct DelegateWorkersTool {
    state: AppState,
    supervisor: Arc<ThreadSupervisor>,
    context: AgentMemoryContext,
    child_tools: ToolRegistry,
    allow_write_review: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct DelegateWorkerTaskArgs {
    prompt: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    access: Option<milim_agents::WorkerAccess>,
}

#[derive(Debug, Deserialize)]
struct DelegateWorkersArgs {
    tasks: Vec<DelegateWorkerTaskArgs>,
}

fn worker_model_is_available(available: &[Model], model: &str) -> bool {
    match crate::providers::provider_model_route(model) {
        Some((provider_id, model_id)) => available.iter().any(|candidate| {
            candidate.id == model_id && candidate.provider_id.as_deref() == Some(&provider_id)
        }),
        None => available.iter().any(|candidate| candidate.id == model),
    }
}

fn resolve_worker_model(
    available: &[Model],
    requested: &str,
    preferred_model: &str,
) -> milim_core::Result<String> {
    let requested = requested.trim();
    if worker_model_is_available(available, requested) {
        return Ok(requested.to_string());
    }
    if requested.contains('/') || requested.starts_with("provider:") {
        return Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is not available"
        )));
    }

    let (preferred_provider, preferred_id) =
        match crate::providers::provider_model_route(preferred_model) {
            Some((provider_id, model_id)) => (Some(provider_id), model_id),
            None => (None, preferred_model.trim().to_string()),
        };
    if let Some((namespace, _)) = preferred_id.split_once('/') {
        let model_id = format!("{namespace}/{requested}");
        if let Some(provider_id) = preferred_provider.as_deref() {
            let routed = crate::providers::provider_model_id(provider_id, &model_id);
            if worker_model_is_available(available, &routed) {
                return Ok(routed);
            }
        } else if worker_model_is_available(available, &model_id) {
            return Ok(model_id);
        }
    }

    let mut matches = available
        .iter()
        .filter(|candidate| {
            candidate
                .id
                .rsplit_once('/')
                .is_some_and(|(_, name)| name == requested)
        })
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [model] => Ok(model.clone()),
        [] => Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is not available"
        ))),
        _ => Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is ambiguous; use a full provider/model id"
        ))),
    }
}

async fn resolve_worker_plan(
    state: &AppState,
    parent_model: &str,
    worker_model: Option<&str>,
    tasks: Vec<DelegateWorkerTaskArgs>,
) -> milim_core::Result<(Vec<milim_agents::WorkerPlanTask>, Vec<Option<String>>)> {
    if !(1..=4).contains(&tasks.len()) {
        return Err(Error::InvalidRequest(
            "delegate_workers requires 1 to 4 independent tasks".to_string(),
        ));
    }
    let available = state.service.list_models().await?;
    let mut plan = Vec::with_capacity(tasks.len());
    let mut system_prompts = Vec::with_capacity(tasks.len());
    for task in tasks {
        let prompt = trim_required_tool_arg(task.prompt, "tasks[].prompt")?;
        let preferred_model = worker_model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(parent_model.trim());
        let requested_model = task
            .model
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(preferred_model);
        let model = resolve_worker_model(&available, requested_model, preferred_model)?;
        let agent_id = trim_optional_agent_id(task.agent_id);
        let system_prompt = if let Some(agent_id) = agent_id.as_deref() {
            let store = state
                .agents
                .as_ref()
                .ok_or_else(|| Error::InvalidRequest("named agents are not enabled".to_string()))?;
            let agent = store
                .get(agent_id)?
                .ok_or_else(|| Error::ModelNotFound(format!("agent {agent_id}")))?;
            (!agent.system_prompt.trim().is_empty()).then_some(agent.system_prompt)
        } else {
            None
        };
        let title = child_thread_title(task.title, &prompt);
        plan.push(milim_agents::WorkerPlanTask {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            prompt,
            role: task.role,
            agent_id,
            model,
            access: task.access.unwrap_or_default(),
        });
        system_prompts.push(system_prompt);
    }
    Ok((plan, system_prompts))
}

#[cfg(test)]
mod worker_model_tests {
    use super::*;

    fn provider_model(provider_id: &str, model_id: &str) -> Model {
        let mut model = Model::local(model_id, 0);
        model.provider_id = Some(provider_id.to_string());
        model
    }

    #[test]
    fn resolves_worker_model_aliases_without_guessing_invalid_ids() {
        let available = vec![
            provider_model("openrouter", "openai/gpt-5.4"),
            Model::local("anthropic/claude-sonnet", 0),
        ];

        assert_eq!(
            resolve_worker_model(&available, "openai/gpt-5.4", "unused").unwrap(),
            "openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "provider:openrouter:openai/gpt-5.4", "unused")
                .unwrap(),
            "provider:openrouter:openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "gpt-5.4", "openai/parent").unwrap(),
            "openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "gpt-5.4", "provider:openrouter:openai/gpt-5.4")
                .unwrap(),
            "provider:openrouter:openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "claude-sonnet", "local-parent").unwrap(),
            "anthropic/claude-sonnet"
        );

        let namespace_preferred = vec![
            Model::local("openai/shared", 0),
            Model::local("other/shared", 0),
        ];
        assert_eq!(
            resolve_worker_model(&namespace_preferred, "shared", "openai/parent").unwrap(),
            "openai/shared"
        );

        assert!(
            resolve_worker_model(&namespace_preferred, "shared", "local-parent")
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(resolve_worker_model(&available, "missing", "local-parent")
            .unwrap_err()
            .to_string()
            .contains("not available"));
        assert!(
            resolve_worker_model(&available, "openai/missing", "local-parent")
                .unwrap_err()
                .to_string()
                .contains("not available")
        );
    }
}

fn worker_specs(
    run: &milim_agents::WorkerRun,
    system_prompts: Vec<Option<String>>,
) -> Vec<ChildRunSpec> {
    run.tasks
        .iter()
        .cloned()
        .zip(system_prompts)
        .map(|(task, system_prompt)| {
            let system_prompt = [run.context.as_deref(), system_prompt.as_deref()]
                .into_iter()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            ChildRunSpec {
                parent_id: run.parent_thread_id.clone(),
                title: task.title,
                model: task.model,
                agent_id: task.agent_id,
                system_prompt: (!system_prompt.is_empty()).then_some(system_prompt),
                prompt: task.prompt,
                run_id: Some(run.id.clone()),
                runtime: run.runtime,
                access: task.access,
                worktree_path: None,
            }
        })
        .collect()
}

fn managed_worker_context(state: &AppState, base: Option<&str>) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(base) = base.filter(|value| !value.trim().is_empty()) {
        sections.push(base.to_string());
    }
    if let Some(workspace) = workspace_snapshot(state) {
        let branch = git_text(&workspace, &["branch", "--show-current"]);
        sections.push(format!(
            "Workspace: {}{}",
            workspace.display(),
            branch
                .filter(|value| !value.is_empty())
                .map(|value| format!("\nBranch: {value}"))
                .unwrap_or_default(),
        ));
        let context = crate::workspace_context::resolve(Some(&workspace));
        if let Some(instructions) = crate::workspace_context::formatted(&context, None) {
            sections.push(instructions);
        }
    }
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

async fn start_managed_worker_run(
    state: &AppState,
    supervisor: &ThreadSupervisor,
    run: &milim_agents::WorkerRun,
    tools: ToolRegistry,
) -> milim_core::Result<(milim_agents::WorkerRun, Vec<milim_agents::Worker>)> {
    if run.status != milim_agents::WorkerRunStatus::Proposed {
        return Err(Error::InvalidRequest(
            "worker run is not awaiting approval".to_string(),
        ));
    }
    let task_args = run
        .tasks
        .iter()
        .map(|task| DelegateWorkerTaskArgs {
            prompt: task.prompt.clone(),
            title: Some(task.title.clone()),
            role: task.role.clone(),
            agent_id: task.agent_id.clone(),
            model: Some(task.model.clone()),
            access: Some(task.access),
        })
        .collect();
    let (resolved, prompts) = resolve_worker_plan(state, "default", None, task_args).await?;
    if resolved
        .iter()
        .map(|t| (&t.prompt, &t.model, &t.agent_id))
        .ne(run.tasks.iter().map(|t| (&t.prompt, &t.model, &t.agent_id)))
    {
        return Err(Error::InvalidRequest(
            "frozen worker plan no longer resolves exactly".to_string(),
        ));
    }
    let running = supervisor
        .store()
        .update_worker_run_status(&run.id, milim_agents::WorkerRunStatus::Running, None)?
        .ok_or_else(|| Error::ModelNotFound(format!("worker run {}", run.id)))?;
    let mut workers = Vec::with_capacity(running.tasks.len());
    for mut spec in worker_specs(&running, prompts) {
        let worker_tools = if spec.access == milim_agents::WorkerAccess::WriteReview {
            match create_worker_worktree(state).await {
                Some(path) => {
                    spec.worktree_path = Some(path.to_string_lossy().to_string());
                    tools.scoped_to_workspace(&path)
                }
                None => {
                    spec.access = milim_agents::WorkerAccess::ReadOnly;
                    tools.read_only()
                }
            }
        } else {
            tools.read_only()
        };
        workers.push(supervisor.spawn(state.service.clone(), worker_tools, spec)?);
    }
    Ok((running, workers))
}

async fn create_worker_worktree(state: &AppState) -> Option<PathBuf> {
    let folder = workspace_snapshot(state)?;
    tokio::task::spawn_blocking(move || {
        let status = workspace_git_status_blocking(Some(folder));
        if !status.is_repo {
            return None;
        }
        let root = PathBuf::from(status.root.as_deref()?);
        let checkpoint =
            workspace_git_checkpoint_action(&root, &status, Some("worker-run-base".to_string()))
                .checkpoint?;
        let worktree_root = milim_core::paths::Paths::resolve()
            .root()
            .join("runtime")
            .join("hot-swap");
        let created =
            workspace_git_create_retry_worktree_action(&root, Some(checkpoint), &worktree_root);
        created
            .ok
            .then_some(created.worktree)
            .flatten()
            .map(PathBuf::from)
    })
    .await
    .ok()
    .flatten()
}

fn schedule_worker_run_deadline(supervisor: Arc<ThreadSupervisor>, run_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(MAX_CHILD_THREAD_WAIT_MS)).await;
        let still_running = supervisor
            .worker_run(&run_id)
            .ok()
            .flatten()
            .is_some_and(|run| run.status == milim_agents::WorkerRunStatus::Running);
        if still_running {
            let _ = supervisor.stop_run(&run_id, "worker run exceeded the five-minute deadline");
        }
    });
}

#[async_trait]
impl Tool for DelegateWorkersTool {
    fn name(&self) -> &str {
        "delegate_workers"
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn description(&self) -> &str {
        "Delegate 1 to 4 genuinely independent tasks as one Worker Run. Do not delegate short or sequential work. Ask mode proposes a frozen plan for approval; Auto runs managed read-only workers and joins their results."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "properties": { "tasks": { "type": "array", "minItems": 1, "maxItems": 4,
                "items": { "type": "object", "properties": {
                    "prompt": {"type":"string"}, "title":{"type":"string"}, "role":{"type":"string"},
                    "agent_id":{"type":["string","null"]}, "model":{"type":"string"},
                    "access":{"type":"string","enum":["read_only","write_review"]}
                }, "required":["prompt"], "additionalProperties":false }
            } }, "required":["tasks"], "additionalProperties":false
        })
    }
    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: DelegateWorkersArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid delegate_workers arguments: {error}"))
        })?;
        let parent_id = child_thread_parent_id(&self.context)?;
        let (mut tasks, _) = resolve_worker_plan(
            &self.state,
            &self.context.model,
            self.context.worker_model.as_deref(),
            args.tasks,
        )
        .await?;
        if self.context.delegation_policy != milim_agents::DelegationPolicy::Ask
            || !self.allow_write_review
        {
            for task in &mut tasks {
                task.access = milim_agents::WorkerAccess::ReadOnly;
            }
        }
        let run = self.supervisor.store().create_worker_run(
            &parent_id,
            self.context.message_id.as_deref(),
            self.context.delegation_policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            managed_worker_context(&self.state, self.context.worker_context.as_deref()).as_deref(),
        )?;
        if self.context.delegation_policy == milim_agents::DelegationPolicy::Ask {
            return Ok(
                json!({ "ok": true, "run": run, "workers": [], "worker_run_notice": worker_run_notice(&run, &[]) }),
            );
        }
        let (mut run, _) = start_managed_worker_run(
            &self.state,
            &self.supervisor,
            &run,
            self.child_tools.clone(),
        )
        .await?;
        run = self
            .supervisor
            .wait_run(&run.id, MAX_CHILD_THREAD_WAIT_MS)
            .await?
            .unwrap_or(run);
        if run.status == milim_agents::WorkerRunStatus::Running {
            run = self
                .supervisor
                .stop_run(&run.id, "worker run exceeded the five-minute deadline")?
                .unwrap_or(run);
        }
        let workers = self.supervisor.workers_for_run(&run.id)?;
        Ok(
            json!({ "ok": true, "run": run, "workers": workers, "worker_run_notice": worker_run_notice(&run, &workers) }),
        )
    }
}

fn register_schedule_tools(
    reg: &mut ToolRegistry,
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<PathBuf>,
) {
    reg.register(Arc::new(ScheduleCreateTool {
        store: store.clone(),
        workspace: workspace.map(|path| path.to_string_lossy().to_string()),
    }));
    reg.register(Arc::new(ScheduleUpdateTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleListTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleDeleteTool { store }));
}

struct ScheduleCreateTool {
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<String>,
}

struct ScheduleUpdateTool {
    store: Arc<milim_automation::ScheduleStore>,
}

struct ScheduleListTool {
    store: Arc<milim_automation::ScheduleStore>,
}

struct ScheduleDeleteTool {
    store: Arc<milim_automation::ScheduleStore>,
}

#[derive(Debug, Deserialize)]
struct ScheduleCreateToolArgs {
    name: String,
    cron: String,
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ScheduleUpdateToolArgs {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cron: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<milim_automation::ScheduleAttachment>>,
    #[serde(default)]
    agent_id: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScheduleListToolArgs {
    #[serde(default)]
    enabled_only: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ScheduleDeleteToolArgs {
    id: String,
}

fn trim_required_tool_arg(value: String, name: &str) -> milim_core::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(Error::InvalidRequest(format!("{name} is required")));
    }
    Ok(value)
}

fn trim_optional_agent_id(agent_id: Option<String>) -> Option<String> {
    agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn find_schedule(
    store: &milim_automation::ScheduleStore,
    id: &str,
) -> milim_core::Result<milim_automation::Schedule> {
    store
        .list()?
        .into_iter()
        .find(|schedule| schedule.id == id)
        .ok_or_else(|| Error::ModelNotFound(format!("schedule {id}")))
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    fn name(&self) -> &str {
        "schedule_create"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Create a cron automation that runs a saved agent prompt. Use this when the user asks to schedule, automate, run periodically, or create a cron from chat. Cron expressions must use six fields: sec min hour day month dow."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short human-readable automation name." },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string", "description": "Self-contained prompt to run each time the automation fires." },
                "attachments": {
                    "type": "array",
                    "description": "Optional file attachments whose text content should be included when the automation runs.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Optional named agent id. Omit for the default agent." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean", "description": "Whether the automation should start enabled. Defaults to true." }
            },
            "required": ["name", "cron", "prompt", "model"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleCreateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_create arguments: {e}"))
        })?;
        let name = trim_required_tool_arg(args.name, "name")?;
        let cron = trim_required_tool_arg(args.cron, "cron")?;
        let prompt = trim_required_tool_arg(args.prompt, "prompt")?;
        let model = provider_schedule_model(trim_required_tool_arg(args.model, "model")?)?;
        let schedule = self.store.create_with_model_context(
            &name,
            &cron,
            trim_optional_agent_id(args.agent_id),
            &model,
            &prompt,
            args.attachments,
            args.enabled,
            self.workspace.clone(),
        )?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleUpdateTool {
    fn name(&self) -> &str {
        "schedule_update"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Update an existing cron automation by id. Use null agent_id to clear the named agent and omit fields that should stay unchanged."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to update." },
                "name": { "type": "string" },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string" },
                "attachments": {
                    "type": "array",
                    "description": "Replacement file attachments for the automation.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Named agent id, or null to clear." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean" }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleUpdateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_update arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let current = find_schedule(&self.store, &id)?;
        let name = args
            .name
            .map(|value| trim_required_tool_arg(value, "name"))
            .transpose()?
            .unwrap_or_else(|| current.name.clone());
        let cron = args
            .cron
            .map(|value| trim_required_tool_arg(value, "cron"))
            .transpose()?
            .unwrap_or_else(|| current.cron.clone());
        let prompt = args
            .prompt
            .map(|value| trim_required_tool_arg(value, "prompt"))
            .transpose()?
            .unwrap_or_else(|| current.prompt.clone());
        let model = args
            .model
            .map(|value| trim_required_tool_arg(value, "model"))
            .transpose()?
            .unwrap_or_else(|| current.model.clone());
        let model = provider_schedule_model(model)?;
        let attachments = args
            .attachments
            .unwrap_or_else(|| current.attachments.clone());
        let agent_id = match args.agent_id {
            None => current.agent_id.clone(),
            Some(Value::Null) => None,
            Some(Value::String(value)) => trim_optional_agent_id(Some(value)),
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "agent_id must be a string or null".to_string(),
                ))
            }
        };
        let schedule = self.store.update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &name,
            cron: &cron,
            agent_id,
            model: &model,
            prompt: &prompt,
            attachments,
            enabled: args.enabled.unwrap_or(current.enabled),
            workspace: current.workspace,
            created_unix: current.created_unix,
            last_run: current.last_run,
        })?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleListTool {
    fn name(&self) -> &str {
        "schedule_list"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "List saved cron automations."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "enabled_only": { "type": "boolean", "description": "Only return enabled schedules." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleListToolArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid schedule_list arguments: {error}"))
        })?;
        let mut schedules = self.store.list()?;
        if args.enabled_only {
            schedules.retain(|schedule| schedule.enabled);
        }
        if let Some(limit) = args.limit {
            schedules.truncate(limit.clamp(1, 50));
        }
        Ok(json!({ "ok": true, "schedules": schedules }))
    }
}

#[async_trait]
impl Tool for ScheduleDeleteTool {
    fn name(&self) -> &str {
        "schedule_delete"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Delete a saved cron automation by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to delete." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleDeleteToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_delete arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let deleted = self.store.delete(&id)?;
        if !deleted {
            return Err(Error::ModelNotFound(format!("schedule {id}")));
        }
        Ok(json!({ "ok": true, "deleted": true, "id": id }))
    }
}

/// `POST /agents/run` — run the tool-use loop server-side and return the final
/// message plus the tool steps taken.
pub(crate) async fn agents_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;

    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let reasoning_effort = req.reasoning_effort;
    let mut agent_config = agent_run_config_from_request(&req);
    let tool_policy = tool_run_policy_from_request(&req);
    if want_stream
        && tool_policy.approval == ToolApprovalPolicy::Review
        && tool_policy.interactive_approval
        && !tool_policy.approval_granted
    {
        agent_config.approval_broker = Some(st.tool_approvals.clone());
    }
    let memory = memory_context_from_request(&req, model.clone());
    let skill_mode = string_extra(&req, "skill_mode").unwrap_or_else(|| "auto".to_string());
    let enabled_skills = string_list_extra(&req, "enabled_skills");
    let workspace_unavailable = desktop_workspace_unavailable(&st);
    let mut messages = req.messages;
    add_workspace_instructions(&mut messages, &st);
    add_workspace_notice_if_needed(&mut messages, workspace_unavailable);

    if want_stream {
        let mut registry = agent_registry_with_memory(&st, Some(memory), &tool_policy);
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &skill_mode, &enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            st.service.clone(),
            tools,
            model,
            messages,
            reasoning_effort,
            agent_config,
        );
        return Ok(Sse::new(agent_sse(stream))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let mut tools = agent_registry_with_memory(&st, Some(memory), &tool_policy);
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &skill_mode, &enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        st.service.as_ref(),
        &tools,
        &model,
        messages,
        reasoning_effort,
        agent_config,
    )
    .await
    .map_err(ApiError)?;

    Ok(Json(AgentRunResponse {
        id: gen_id("agentrun"),
        object: "agent.run",
        model,
        message: outcome.message,
        steps: outcome.steps,
        iterations: outcome.iterations,
        stopped_at_limit: outcome.stopped_at_limit,
    })
    .into_response())
}

/// `GET /agents` — list named agents.
pub(crate) async fn agents_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let agents = match &st.agents {
        Some(store) => store.list().map_err(ApiError)?,
        None => Vec::new(),
    };
    Ok(Json(json!({ "agents": agents })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct CreateAgentRequest {
    name: String,
    model: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    tool_mode: String,
    #[serde(default)]
    enabled_tools: Vec<String>,
    #[serde(default)]
    skill_mode: String,
    #[serde(default)]
    enabled_skills: Vec<String>,
    #[serde(default)]
    avatar: String,
}

/// `POST /agents` — create a named agent.
pub(crate) async fn agent_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .create(
            &req.name,
            &req.model,
            &req.system_prompt,
            &req.tool_mode,
            req.enabled_tools,
            &req.skill_mode,
            req.enabled_skills,
            &req.avatar,
        )
        .map_err(ApiError)?;
    Ok(Json(agent).into_response())
}

/// `GET /agents/{id}` — fetch one agent.
pub(crate) async fn agent_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("agent {id}"))))?;
    Ok(Json(agent).into_response())
}

/// `POST /agents/{id}/run` — run a named agent's tool-use loop.
pub(crate) async fn agent_run_by_id(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("agent {id}"))))?;

    let want_stream = req.wants_stream();
    let requested_model = req.model.clone();
    let reasoning_effort = req.reasoning_effort;
    let mut agent_config = agent_run_config_from_request(&req);
    let tool_policy = tool_run_policy_from_request(&req);
    if want_stream
        && tool_policy.approval == ToolApprovalPolicy::Review
        && tool_policy.interactive_approval
        && !tool_policy.approval_granted
    {
        agent_config.approval_broker = Some(st.tool_approvals.clone());
    }
    let mut memory = memory_context_from_request(&req, requested_model.clone());
    let mut messages = Vec::new();
    if !agent.system_prompt.is_empty() {
        messages.push(ChatMessage::text("system", agent.system_prompt.clone()));
    }
    let skill_query = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(ChatMessage::text_content)
        .unwrap_or_default();
    if !bool_extra(&req, "skills_resolved") {
        messages.extend(crate::agent_skill_messages(&st, &agent, &skill_query));
    }
    let resolved_role = messages
        .iter()
        .filter(|message| message.role == "system")
        .map(ChatMessage::text_content)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !resolved_role.is_empty() {
        memory.worker_context = Some(
            [
                memory.worker_context.as_deref(),
                Some(resolved_role.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n"),
        );
    }
    messages.extend(req.messages);
    add_workspace_instructions(&mut messages, &st);
    let model = requested_model;
    let memory = AgentMemoryContext {
        model: model.clone(),
        ..memory
    };
    add_workspace_notice_if_needed(&mut messages, desktop_workspace_unavailable(&st));

    if want_stream {
        let mut registry = agent_registry_for_mode(
            &st,
            &agent.tool_mode,
            &agent.enabled_tools,
            Some(memory),
            &tool_policy,
        );
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &agent.skill_mode, &agent.enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            st.service.clone(),
            tools,
            model,
            messages,
            reasoning_effort,
            agent_config,
        );
        return Ok(Sse::new(agent_sse(stream))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let mut tools = agent_registry_for_mode(
        &st,
        &agent.tool_mode,
        &agent.enabled_tools,
        Some(memory),
        &tool_policy,
    );
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &agent.skill_mode, &agent.enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        st.service.as_ref(),
        &tools,
        &model,
        messages,
        reasoning_effort,
        agent_config,
    )
    .await
    .map_err(ApiError)?;

    Ok(Json(AgentRunResponse {
        id: gen_id("agentrun"),
        object: "agent.run",
        model,
        message: outcome.message,
        steps: outcome.steps,
        iterations: outcome.iterations,
        stopped_at_limit: outcome.stopped_at_limit,
    })
    .into_response())
}

/// `PUT /agents/{id}` — update (upsert) a named agent.
pub(crate) async fn agent_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = milim_agents::AgentDef {
        id,
        name: req.name,
        system_prompt: req.system_prompt,
        model: req.model,
        tool_mode: milim_agents::normalize_tool_mode(&req.tool_mode, &req.enabled_tools),
        enabled_tools: req.enabled_tools,
        skill_mode: milim_agents::normalize_skill_mode(&req.skill_mode, &req.enabled_skills),
        enabled_skills: req.enabled_skills,
        avatar: req.avatar,
    };
    store.upsert(&agent).map_err(ApiError)?;
    Ok(Json(agent).into_response())
}

/// `DELETE /agents/{id}` — remove a named agent.
pub(crate) async fn agent_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let removed = store.delete(&id).map_err(ApiError)?;
    Ok(Json(json!({ "deleted": removed })).into_response())
}

fn agents_store(st: &AppState) -> Result<&milim_agents::AgentStore, ApiError> {
    st.agents
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("agents are not enabled".to_string())))
}

fn thread_supervisor(st: &AppState) -> Result<Arc<ThreadSupervisor>, ApiError> {
    st.threads
        .as_ref()
        .cloned()
        .ok_or_else(|| ApiError(missing_threads_error()))
}

#[derive(Deserialize)]
pub(crate) struct ThreadReadQuery {
    #[serde(default)]
    include_events: bool,
    #[serde(default)]
    event_limit: Option<usize>,
    #[serde(default)]
    after_seq: Option<i64>,
}

const DEFAULT_THREAD_EVENT_LIMIT: usize = 1000;
const MAX_THREAD_EVENT_LIMIT: usize = 5000;

fn thread_event_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_THREAD_EVENT_LIMIT)
}

#[derive(Deserialize)]
pub(crate) struct ThreadChildrenQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct ThreadEventsQuery {
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    event_limit: Option<usize>,
}

/// `GET /threads/{id}` - inspect one child thread.
pub(crate) async fn thread_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadReadQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let thread = supervisor
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    if query.include_events {
        let limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
        let events = if let Some(after_seq) = query.after_seq {
            supervisor
                .events_after(&id, after_seq.max(0), limit)
                .map_err(ApiError)?
        } else {
            supervisor.events(&id, limit).map_err(ApiError)?
        };
        let event_count = supervisor.event_count(&id).map_err(ApiError)?;
        Ok(Json(json!({
            "thread": thread,
            "events": events,
            "event_count": event_count,
            "events_truncated": event_count > events.len()
        }))
        .into_response())
    } else {
        Ok(Json(json!({ "thread": thread })).into_response())
    }
}

/// `GET /threads/{id}/children` - list children for a parent thread id.
pub(crate) async fn thread_children(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadChildrenQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let threads = supervisor
        .children(
            &id,
            query.status.as_deref(),
            query.limit.unwrap_or(50).clamp(1, 50),
        )
        .map_err(ApiError)?;
    Ok(Json(json!({ "threads": threads })).into_response())
}

/// `GET /threads/{id}/events` - pushed child-thread supervisor events.
pub(crate) async fn thread_events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadEventsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let mut events = supervisor.subscribe();
    let event_limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
    let initial_after_seq = query.after_seq.unwrap_or(0).max(0);
    let stream = async_stream::stream! {
        let mut last_seq = initial_after_seq;
        if let Ok(backfill) = supervisor.child_events_after(&id, last_seq, event_limit) {
            for (thread, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
                    .unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.thread().parent_id != id {
                        continue;
                    }
                    if let SupervisorEvent::ChildThreadEvent { event: stored, .. } = &event {
                        if stored.seq <= last_seq {
                            continue;
                        }
                        last_seq = stored.seq;
                    }
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                    yield Ok::<Event, Infallible>(Event::default().data(data));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if let Ok(backfill) = supervisor.child_events_after(&id, last_seq, event_limit) {
                        for (thread, event) in backfill {
                            last_seq = last_seq.max(event.seq);
                            let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
                                .unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().data(data));
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `POST /threads/{id}/stop` - stop a running child thread.
pub(crate) async fn thread_stop(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let thread = supervisor
        .stop(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    Ok(Json(json!({ "thread": thread })).into_response())
}

/// `DELETE /threads/{id}` - delete child-thread rows under a parent or child id.
pub(crate) async fn thread_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let deleted = supervisor.delete_tree(&id).map_err(ApiError)?;
    Ok(Json(json!({ "deleted": deleted.len() })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct WorkerRunsListQuery {
    parent_thread_id: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct WorkerRunCreateRequest {
    parent_thread_id: String,
    #[serde(default)]
    parent_turn_id: Option<String>,
    #[serde(default)]
    policy: milim_agents::DelegationPolicy,
    #[serde(default)]
    runtime: milim_agents::WorkerRuntime,
    #[serde(default)]
    model: Option<String>,
    tasks: Vec<DelegateWorkerTaskArgs>,
}

pub(crate) async fn worker_runs_list(
    State(st): State<AppState>,
    Query(query): Query<WorkerRunsListQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let parent_id =
        trim_required_tool_arg(query.parent_thread_id, "parent_thread_id").map_err(ApiError)?;
    let supervisor = thread_supervisor(&st)?;
    let runs = supervisor
        .worker_runs(&parent_id, query.limit.unwrap_or(50).clamp(1, 200))
        .map_err(ApiError)?;
    let mut records = Vec::with_capacity(runs.len());
    for run in runs {
        let workers = supervisor.workers_for_run(&run.id).map_err(ApiError)?;
        records.push(json!({ "run": run, "workers": workers }));
    }
    Ok(Json(json!({ "runs": records })).into_response())
}

pub(crate) async fn worker_run_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkerRunCreateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    if req.policy == milim_agents::DelegationPolicy::Off {
        return Err(ApiError(Error::InvalidRequest(
            "delegation is off for this thread".to_string(),
        )));
    }
    let parent_id =
        trim_required_tool_arg(req.parent_thread_id, "parent_thread_id").map_err(ApiError)?;
    let default_model = if let Some(model) = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        model.to_string()
    } else {
        st.service
            .list_models()
            .await
            .map_err(ApiError)?
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| {
                ApiError(Error::InvalidRequest(
                    "no worker model is available".to_string(),
                ))
            })?
    };
    let (mut tasks, _) = resolve_worker_plan(&st, &default_model, req.model.as_deref(), req.tasks)
        .await
        .map_err(ApiError)?;
    for task in &mut tasks {
        task.access = milim_agents::WorkerAccess::ReadOnly;
    }
    // Native adapters normalize their own activity into this contract. This endpoint safely falls back to managed workers.
    let _requested_runtime = req.runtime;
    let supervisor = thread_supervisor(&st)?;
    let mut run = supervisor
        .store()
        .create_worker_run(
            &parent_id,
            req.parent_turn_id.as_deref(),
            req.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            None,
        )
        .map_err(ApiError)?;
    let mut workers = Vec::new();
    if req.policy == milim_agents::DelegationPolicy::Auto {
        (run, workers) =
            start_managed_worker_run(&st, &supervisor, &run, child_read_only_registry(&st))
                .await
                .map_err(ApiError)?;
        schedule_worker_run_deadline(supervisor.clone(), run.id.clone());
    }
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let workers = supervisor.workers_for_run(&id).map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    if matches!(
        run.status,
        milim_agents::WorkerRunStatus::Proposed | milim_agents::WorkerRunStatus::Running
    ) {
        return Err(ApiError(Error::InvalidRequest(
            "active worker runs must be stopped before deletion".to_string(),
        )));
    }
    let deleted = supervisor
        .store()
        .delete_worker_run(&id)
        .map_err(ApiError)?;
    Ok(Json(json!({ "deleted": deleted })).into_response())
}

pub(crate) async fn worker_run_start(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let (run, workers) =
        start_managed_worker_run(&st, &supervisor, &run, worker_review_registry(&st))
            .await
            .map_err(ApiError)?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

#[derive(Default, Deserialize)]
pub(crate) struct WorkerRunRetryRequest {
    #[serde(default)]
    model: Option<String>,
}

pub(crate) async fn worker_run_task_retry(
    State(st): State<AppState>,
    Path((id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkerRunRetryRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let source = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    if matches!(
        source.status,
        milim_agents::WorkerRunStatus::Proposed | milim_agents::WorkerRunStatus::Running
    ) {
        return Err(ApiError(Error::InvalidRequest(
            "worker task can be retried only after its run finishes".to_string(),
        )));
    }
    let task = source
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker task {task_id}"))))?;
    let requested_model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(&task.model);
    let (tasks, _) = resolve_worker_plan(
        &st,
        &task.model,
        None,
        vec![DelegateWorkerTaskArgs {
            prompt: task.prompt.clone(),
            title: Some(task.title.clone()),
            role: task.role.clone(),
            agent_id: task.agent_id.clone(),
            model: Some(requested_model.to_string()),
            access: Some(task.access),
        }],
    )
    .await
    .map_err(ApiError)?;
    let retry = supervisor
        .store()
        .create_worker_run(
            &source.parent_thread_id,
            source.parent_turn_id.as_deref(),
            source.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            source.context.as_deref(),
        )
        .map_err(ApiError)?;
    let (run, workers) =
        start_managed_worker_run(&st, &supervisor, &retry, worker_review_registry(&st))
            .await
            .map_err(ApiError)?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_stop(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .stop_run(&id, "stopped by parent")
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let workers = supervisor.workers_for_run(&id).map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

fn owned_worker(
    supervisor: &ThreadSupervisor,
    run_id: &str,
    worker_id: &str,
) -> Result<milim_agents::Worker, ApiError> {
    let worker = supervisor
        .get(worker_id)
        .map_err(ApiError)?
        .filter(|worker| worker.run_id.as_deref() == Some(run_id))
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker {worker_id}"))))?;
    Ok(worker)
}

pub(crate) async fn worker_run_worker_stop(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worker = supervisor
        .stop(&worker_id)
        .map_err(ApiError)?
        .unwrap_or(worker);
    let run = supervisor
        .store()
        .refresh_worker_run_status(&id)
        .map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "worker": worker })).into_response())
}

pub(crate) async fn worker_run_worker_diff(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worktree = worker
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "worker has no review worktree".into(),
            ))
        })?;
    let payload = tokio::task::spawn_blocking(move || {
        let status = workspace_git_status_blocking(Some(worktree.clone()));
        let checkpoint =
            workspace_git_checkpoint_action(&worktree, &status, Some("worker-review".to_string()));
        let diff = checkpoint
            .checkpoint
            .as_deref()
            .and_then(|reference| {
                git_text(&worktree, &["diff", "--binary", "HEAD", reference, "--"])
            })
            .unwrap_or_default();
        json!({ "worker_id": worker_id, "status": status, "diff": diff })
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("worker diff task failed: {error}"))))?;
    Ok(Json(payload).into_response())
}

pub(crate) async fn worker_run_worker_apply(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worktree = worker.worktree_path.clone().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "worker has no review worktree".into(),
        ))
    })?;
    let root = workspace_snapshot(&st)
        .ok_or_else(|| ApiError(Error::InvalidRequest("no working folder selected".into())))?;
    let worktree_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("hot-swap");
    let result = tokio::task::spawn_blocking(move || {
        let checkpoint = git_text(FsPath::new(&worktree), &["rev-parse", "HEAD"]);
        workspace_git_apply_retry_worktree_action(&root, checkpoint, Some(worktree), &worktree_root)
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("worker apply task failed: {error}"))))?;
    Ok(Json(result).into_response())
}

pub(crate) async fn worker_run_events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadEventsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    if supervisor.worker_run(&id).map_err(ApiError)?.is_none() {
        return Err(ApiError(Error::ModelNotFound(format!("worker run {id}"))));
    }
    let mut events = supervisor.subscribe();
    let limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
    let initial_after_seq = query.after_seq.unwrap_or(0).max(0);
    let stream = async_stream::stream! {
        let mut last_seq = initial_after_seq;
        if let Ok(backfill) = supervisor.worker_events_after(&id, last_seq, limit) {
            for (worker, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&json!({"type":"worker_run_worker_event","run_id":id,"worker":worker,"event":event})).unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }
        }
        if let Ok(Some(run)) = supervisor.store().refresh_worker_run_status(&id) {
            let workers = supervisor.workers_for_run(&id).unwrap_or_default();
            let kind = format!("worker_run_{}", worker_run_event_name(run.status));
            let data = serde_json::to_string(&json!({"type":kind,"run":run,"workers":workers})).unwrap_or_else(|_| "{}".to_string());
            yield Ok::<Event, Infallible>(Event::default().data(data));
        }
        loop {
            match events.recv().await {
                Ok(event) if event.thread().run_id.as_deref() == Some(id.as_str()) => {
                    if let SupervisorEvent::ChildThreadEvent { event: stored, .. } = &event {
                        if stored.seq <= last_seq { continue; }
                        last_seq = stored.seq;
                    }
                    let run = supervisor.store().refresh_worker_run_status(&id).ok().flatten();
                    let kind = match &event {
                        SupervisorEvent::ChildThreadStarted { .. } => "worker_run_worker_started",
                        SupervisorEvent::ChildThreadDone { .. } => "worker_run_worker_done",
                        SupervisorEvent::ChildThreadError { .. } => "worker_run_worker_error",
                        SupervisorEvent::ChildThreadStopped { .. } => "worker_run_worker_stopped",
                        SupervisorEvent::ChildThreadEvent { .. } => "worker_run_worker_event",
                    };
                    let data = serde_json::to_string(&json!({"type":kind,"run":run,"worker":event.thread(),"event":event})).unwrap_or_else(|_| "{}".to_string());
                    yield Ok::<Event, Infallible>(Event::default().data(data));
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

// ----- Schedules -----

#[derive(Deserialize)]
pub(crate) struct CreateScheduleRequest {
    name: String,
    cron: String,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateScheduleRequest {
    name: String,
    cron: String,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    last_run: Option<i64>,
}

fn provider_schedule_model(model: String) -> milim_core::Result<String> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("codex:")
        || lower.starts_with("claude:")
        || lower.starts_with("opencode:")
        || lower.starts_with("pi:")
    {
        return Err(Error::InvalidRequest(
            "schedules require a configured provider model; account runtimes are interactive only"
                .to_string(),
        ));
    }
    Ok(model)
}

fn default_true() -> bool {
    true
}

/// `GET /schedules` — list cron schedules.
pub(crate) async fn schedules_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let schedules = match &st.schedules {
        Some(store) => store.list().map_err(ApiError)?,
        None => Vec::new(),
    };
    Ok(Json(json!({ "schedules": schedules })).into_response())
}

/// `POST /schedules` — create a cron schedule.
/// `GET /schedules/events` - drain completed background schedule runs.
pub(crate) async fn schedule_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(json!({ "events": st.schedule_runs.take() })).into_response())
}

/// `POST /schedules` - create a cron schedule.
pub(crate) async fn schedule_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    let model =
        provider_schedule_model(trim_required_tool_arg(req.model, "model").map_err(ApiError)?)
            .map_err(ApiError)?;
    let schedule = store
        .create_with_model_context(
            &req.name,
            &req.cron,
            req.agent_id,
            &model,
            &req.prompt,
            req.attachments,
            true,
            workspace_snapshot(&st).map(|path| path.to_string_lossy().to_string()),
        )
        .map_err(ApiError)?;
    Ok(Json(schedule).into_response())
}

/// `DELETE /schedules/{id}` — remove a schedule.
/// `PUT /schedules/{id}` - update a cron schedule.
pub(crate) async fn schedule_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    let current = find_schedule(store, &id).map_err(ApiError)?;
    let model =
        provider_schedule_model(trim_required_tool_arg(req.model, "model").map_err(ApiError)?)
            .map_err(ApiError)?;
    let schedule = store
        .update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &req.name,
            cron: &req.cron,
            agent_id: req.agent_id,
            model: &model,
            prompt: &req.prompt,
            attachments: req.attachments,
            enabled: req.enabled,
            workspace: current.workspace,
            created_unix: current.created_unix,
            last_run: req.last_run,
        })
        .map_err(ApiError)?;
    Ok(Json(schedule).into_response())
}

pub(crate) async fn schedule_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    if store.delete(&id).map_err(ApiError)? {
        Ok(Json(json!({ "deleted": true })).into_response())
    } else {
        Err(ApiError(Error::ModelNotFound(format!("schedule {id}"))))
    }
}

// ----- Skills -----

/// `GET /skills` — list skills.
pub(crate) async fn skills_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let skills = match &st.skills {
        Some(store) => store.list().map_err(ApiError)?,
        None => Vec::new(),
    };
    Ok(Json(json!({ "skills": skills })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct CreateSkillRequest {
    #[serde(default)]
    skill_md: Option<String>,
    #[serde(default)]
    skill_url: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize)]
pub(crate) struct UpdateSkillRequest {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    source_url: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct SelectSkillsRequest {
    query: String,
    #[serde(default = "default_skill_select_limit")]
    limit: usize,
    #[serde(default)]
    ids: Option<Vec<String>>,
}

fn default_skill_select_limit() -> usize {
    3
}

/// `POST /skills` — create a skill from a `SKILL.md` or explicit fields.
pub(crate) async fn skill_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateSkillRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .skills
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("skills are not enabled".to_string())))?;
    let skill = if let Some(md) = req.skill_md {
        store
            .create_from_md_with_source(
                &md,
                req.enabled,
                req.source_kind.as_deref().unwrap_or("pasted"),
                req.source_url,
            )
            .map_err(ApiError)?
    } else if let Some(url) = req.skill_url {
        let raw_url = github_skill_raw_url(&url).map_err(ApiError)?;
        let md = fetch_skill_md(&raw_url).await.map_err(ApiError)?;
        store
            .create_from_md_with_source(&md, req.enabled, "github", Some(url))
            .map_err(ApiError)?
    } else {
        let name = req.name.ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "missing 'name', 'skill_md', or 'skill_url'".to_string(),
            ))
        })?;
        store
            .create_with_source(
                &name,
                req.description.as_deref().unwrap_or(""),
                req.instructions.as_deref().unwrap_or(""),
                req.enabled,
                req.source_kind.as_deref().unwrap_or("manual"),
                req.source_url,
            )
            .map_err(ApiError)?
    };
    Ok(Json(skill).into_response())
}

/// `POST /skills/select` - keyword-select enabled skills for a query.
pub(crate) async fn skills_select(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<SelectSkillsRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .skills
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("skills are not enabled".to_string())))?;
    let skills = store
        .select_filtered(&req.query, req.limit.clamp(1, 10), req.ids.as_deref())
        .map_err(ApiError)?;
    Ok(Json(json!({ "skills": skills })).into_response())
}

/// `GET /skills/{id}` — fetch one skill.
pub(crate) async fn skill_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .skills
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("skills are not enabled".to_string())))?;
    let skill = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("skill {id}"))))?;
    Ok(Json(skill).into_response())
}

/// `PUT /skills/{id}` - update one skill.
pub(crate) async fn skill_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<UpdateSkillRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .skills
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("skills are not enabled".to_string())))?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::InvalidRequest(
            "skill name is required".to_string(),
        )));
    }
    let skill = store
        .update(&milim_skills::SkillDef {
            id: id.clone(),
            name: req.name.trim().to_string(),
            description: req.description.trim().to_string(),
            instructions: req.instructions,
            enabled: req.enabled,
            source_kind: req.source_kind.unwrap_or_else(|| "manual".to_string()),
            source_url: req.source_url,
            updated_at: String::new(),
        })
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("skill {id}"))))?;
    Ok(Json(skill).into_response())
}

/// `DELETE /skills/{id}` - delete one skill.
pub(crate) async fn skill_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .skills
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("skills are not enabled".to_string())))?;
    Ok(Json(json!({ "deleted": store.delete(&id).map_err(ApiError)? })).into_response())
}

fn github_skill_raw_url(input: &str) -> Result<String, Error> {
    let parsed = reqwest::Url::parse(input.trim())
        .map_err(|e| Error::InvalidRequest(format!("invalid skill URL: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(Error::InvalidRequest("skill URL must be https".to_string()));
    }
    let host = parsed.host_str().unwrap_or_default();
    let segments: Vec<&str> = parsed
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    if host == "raw.githubusercontent.com" {
        if segments.len() >= 5 && segments.last() == Some(&"SKILL.md") {
            return Ok(parsed.to_string());
        }
        return Err(Error::InvalidRequest(
            "raw GitHub skill URL must point to SKILL.md".to_string(),
        ));
    }

    if host != "github.com" || segments.len() < 5 {
        return Err(Error::InvalidRequest(
            "only GitHub SKILL.md URLs are supported".to_string(),
        ));
    }

    let owner = segments[0];
    let repo = segments[1];
    let kind = segments[2];
    let branch = segments[3];
    let mut path = segments[4..].join("/");
    match kind {
        "blob" if path.ends_with("SKILL.md") => {}
        "tree" => {
            if !path.ends_with("SKILL.md") {
                path = format!("{}/SKILL.md", path.trim_end_matches('/'));
            }
        }
        _ => {
            return Err(Error::InvalidRequest(
                "GitHub skill URL must be a blob or tree path to SKILL.md".to_string(),
            ));
        }
    }
    Ok(format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}"
    ))
}

async fn fetch_skill_md(raw_url: &str) -> Result<String, Error> {
    let response = reqwest::Client::new()
        .get(raw_url)
        .header(USER_AGENT, "milim-skill-import")
        .send()
        .await
        .map_err(|e| Error::Other(format!("failed to fetch skill: {e}")))?;
    if !response.status().is_success() {
        return Err(Error::InvalidRequest(format!(
            "failed to fetch skill: HTTP {}",
            response.status()
        )));
    }
    let text = response
        .text()
        .await
        .map_err(|e| Error::Other(format!("failed to read skill: {e}")))?;
    if text.len() > 256 * 1024 {
        return Err(Error::InvalidRequest(
            "skill is too large; maximum is 256 KiB".to_string(),
        ));
    }
    if !text.contains("name:") {
        return Err(Error::InvalidRequest(
            "fetched file does not look like SKILL.md".to_string(),
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod skill_import_tests {
    use super::github_skill_raw_url;

    #[test]
    fn github_blob_skill_url_becomes_raw() {
        assert_eq!(
            github_skill_raw_url("https://github.com/acme/skills/blob/main/review/SKILL.md")
                .unwrap(),
            "https://raw.githubusercontent.com/acme/skills/main/review/SKILL.md"
        );
    }

    #[test]
    fn github_tree_skill_url_appends_skill_md() {
        assert_eq!(
            github_skill_raw_url("https://github.com/acme/skills/tree/main/review").unwrap(),
            "https://raw.githubusercontent.com/acme/skills/main/review/SKILL.md"
        );
    }

    #[test]
    fn rejects_non_github_or_non_skill_urls() {
        assert!(github_skill_raw_url("https://example.com/SKILL.md").is_err());
        assert!(
            github_skill_raw_url("https://github.com/acme/skills/blob/main/README.md").is_err()
        );
    }
}

// ----- Sandbox -----

#[derive(Deserialize)]
pub(crate) struct SandboxRunRequest {
    image: String,
    command: Vec<String>,
    #[serde(default)]
    network: bool,
}

/// `POST /sandbox/run` — run a command in an isolated container.
pub(crate) async fn sandbox_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<SandboxRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let backend = milim_sandbox::DockerBackend::new();
    let opts = milim_sandbox::RunOpts {
        network: req.network,
        ..Default::default()
    };
    let out = backend
        .run(&req.image, &req.command, &opts)
        .await
        .map_err(ApiError)?;
    Ok(Json(out).into_response())
}

// ----- Privacy -----

#[derive(Deserialize)]
pub(crate) struct PrivacyScanRequest {
    text: String,
}

/// `POST /privacy/scan` — detect + redact PII in `text`.
pub(crate) async fn privacy_scan(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<PrivacyScanRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let detections = st.privacy.scan_text(&req.text);
    let redaction = st.privacy.redact_text(&req.text);
    Ok(Json(json!({
        "clean": st.privacy.is_clean_text(&req.text),
        "detections": detections,
        "redacted": redaction.text,
        "map": redaction.map,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub(crate) struct PrivacyModeSet {
    mode: String,
}

/// `GET /privacy/mode` — the current outbound privacy gate mode.
pub(crate) async fn privacy_mode_get(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(json!({ "mode": st.privacy.mode().as_str() })).into_response())
}

/// `POST /privacy/mode` — set the outbound gate (`off` | `redact` | `block`).
/// Applies to requests routed to a remote provider; local backends are exempt.
pub(crate) async fn privacy_mode_set(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<PrivacyModeSet>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let mode = crate::privacy::PrivacyMode::parse(&req.mode);
    st.privacy.set(mode);
    Ok(Json(json!({ "mode": mode.as_str() })).into_response())
}

// ----- Memory -----

fn memory_store(st: &AppState) -> Result<&milim_memory::MemoryStore, ApiError> {
    st.memory
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("memory is not enabled".to_string())))
}

fn default_memory_model() -> String {
    "default".to_string()
}

fn default_top_k() -> usize {
    5
}

#[derive(Deserialize)]
pub(crate) struct MemoryIngestRequest {
    #[serde(default = "default_memory_model")]
    model: String,
    text: String,
}

/// `POST /memory/ingest` — embed and store a memory.
pub(crate) async fn memory_ingest(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MemoryIngestRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let mem = memory_store(&st)?;
    let id = mem.add(&req.model, &req.text).await.map_err(ApiError)?;
    Ok(Json(json!({ "id": id })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct MemorySearchRequest {
    #[serde(default = "default_memory_model")]
    model: String,
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

#[derive(Serialize)]
struct MemorySearchResponse {
    hits: Vec<milim_memory::MemoryHit>,
}

/// `POST /memory/search` — return the most similar stored memories.
pub(crate) async fn memory_search(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let mem = memory_store(&st)?;
    let hits = mem
        .search(&req.model, &req.query, req.top_k)
        .await
        .map_err(ApiError)?;
    Ok(Json(MemorySearchResponse { hits }).into_response())
}

#[derive(Deserialize)]
pub(crate) struct MemoryRegisterRequest {
    #[serde(default = "default_memory_model")]
    model: String,
    scope: milim_memory::MemoryScopeInput,
    node: milim_memory::MemoryNodeInput,
    #[serde(default)]
    edges: Vec<milim_memory::MemoryEdgeInput>,
    #[serde(default)]
    event: milim_memory::MemoryEventInput,
}

/// `POST /memory/register` — create a scoped graph memory.
pub(crate) async fn memory_register(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MemoryRegisterRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let mem = memory_store(&st)?;
    let registration = mem
        .register(&req.model, req.scope, req.node, req.edges, req.event)
        .await
        .map_err(ApiError)?;
    Ok(Json(registration).into_response())
}

#[derive(Deserialize)]
pub(crate) struct MemoryGraphSearchRequest {
    #[serde(default = "default_memory_model")]
    model: String,
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    #[serde(default)]
    scopes: Vec<milim_memory::MemoryScopeRef>,
    #[serde(default)]
    include_archived: bool,
}

#[derive(Serialize)]
struct MemoryGraphSearchResponse {
    hits: Vec<milim_memory::MemoryGraphHit>,
}

/// `POST /memory/graph/search` — hybrid lexical/semantic search over scoped memories.
pub(crate) async fn memory_graph_search(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MemoryGraphSearchRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let mem = memory_store(&st)?;
    let hits = mem
        .search_graph(
            &req.model,
            &req.query,
            &req.scopes,
            req.top_k,
            req.include_archived,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(MemoryGraphSearchResponse { hits }).into_response())
}

/// `GET /memory/scopes` — list thread/project/global memory scopes.
pub(crate) async fn memory_scopes(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let scopes = memory_store(&st)?.list_scopes().map_err(ApiError)?;
    Ok(Json(json!({ "scopes": scopes })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct MemoryNodesQuery {
    #[serde(default)]
    scope_kind: Option<String>,
    #[serde(default)]
    scope_locator: Option<String>,
    #[serde(default)]
    include_archived: bool,
    #[serde(default = "default_memory_node_limit")]
    limit: usize,
}

fn default_memory_node_limit() -> usize {
    100
}

/// `GET /memory/nodes` — list graph memory nodes, optionally scoped.
pub(crate) async fn memory_nodes(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<MemoryNodesQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let scope = match (query.scope_kind, query.scope_locator) {
        (Some(kind), Some(locator)) if !kind.trim().is_empty() && !locator.trim().is_empty() => {
            Some(milim_memory::MemoryScopeRef { kind, locator })
        }
        _ => None,
    };
    let nodes = memory_store(&st)?
        .list_nodes(scope, query.include_archived, query.limit)
        .map_err(ApiError)?;
    Ok(Json(json!({ "nodes": nodes })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct MemoryNodeUpdateRequest {
    #[serde(default = "default_memory_model")]
    model: String,
    #[serde(flatten)]
    update: milim_memory::MemoryNodeUpdate,
}

/// `PUT /memory/nodes/{id}` — update one graph memory node.
pub(crate) async fn memory_node_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<MemoryNodeUpdateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let node = memory_store(&st)?
        .update_node(&req.model, &id, req.update)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("memory node {id}"))))?;
    Ok(Json(node).into_response())
}

/// `DELETE /memory/nodes/{id}` — delete one graph memory node.
pub(crate) async fn memory_node_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let deleted = memory_store(&st)?.delete_node(&id).map_err(ApiError)?;
    Ok(Json(json!({ "deleted": deleted })).into_response())
}

/// `POST /memory/nodes/{id}/archive` — hide one graph memory node without deleting it.
pub(crate) async fn memory_node_archive(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let archived = memory_store(&st)?.archive_node(&id).map_err(ApiError)?;
    Ok(Json(json!({ "archived": archived })).into_response())
}

// ----- Embeddings -----

#[derive(Deserialize)]
#[serde(untagged)]
enum EmbedInput {
    One(String),
    Many(Vec<String>),
}

impl EmbedInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            EmbedInput::One(s) => vec![s],
            EmbedInput::Many(v) => v,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct OpenAiEmbeddingRequest {
    model: String,
    input: EmbedInput,
}

#[derive(Serialize)]
struct OpenAiEmbeddingItem {
    object: &'static str,
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Serialize)]
struct OpenAiEmbeddingResponse {
    object: &'static str,
    data: Vec<OpenAiEmbeddingItem>,
    model: String,
    usage: Usage,
}

/// `POST /v1/embeddings` and `/embeddings`
pub(crate) async fn openai_embeddings(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OpenAiEmbeddingRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let inputs = req.input.into_vec();
    let prompt_tokens = inputs
        .iter()
        .map(|s| s.split_whitespace().count() as u32)
        .sum();
    let vectors = st
        .service
        .embed(&req.model, inputs)
        .await
        .map_err(ApiError)?;
    let data = vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| OpenAiEmbeddingItem {
            object: "embedding",
            embedding,
            index,
        })
        .collect();
    Ok(Json(OpenAiEmbeddingResponse {
        object: "list",
        data,
        model: req.model,
        usage: Usage::new(prompt_tokens, 0),
    })
    .into_response())
}

#[derive(Deserialize)]
pub(crate) struct OllamaEmbedRequest {
    model: String,
    input: EmbedInput,
}

#[derive(Serialize)]
struct OllamaEmbedResponse {
    model: String,
    embeddings: Vec<Vec<f32>>,
}

/// `POST /api/embed` and `/api/embeddings` (Ollama)
pub(crate) async fn ollama_embeddings(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<OllamaEmbedRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let embeddings = st
        .service
        .embed(&req.model, req.input.into_vec())
        .await
        .map_err(ApiError)?;
    Ok(Json(OllamaEmbedResponse {
        model: req.model,
        embeddings,
    })
    .into_response())
}
