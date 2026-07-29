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
    PreviewStaticStartRequest,
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

mod account_runtimes;
mod agents;
mod harnesses;
mod inference;
mod mcp;
mod media;
mod mobile;
mod workspace;

pub(crate) use account_runtimes::*;
pub(crate) use agents::*;
pub(crate) use harnesses::*;
pub(crate) use inference::*;
pub(crate) use mcp::*;
pub(crate) use media::*;
pub(crate) use mobile::*;
pub(crate) use workspace::*;

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
