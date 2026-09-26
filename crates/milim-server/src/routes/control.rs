use super::*;

use crate::control::{
    ControlAttachmentUploadV1, ControlCommandV1, ControlEventV1, EffectiveRunPreviewRequestV1,
    EffectiveRunPreviewV1, RunEventPageV1, RunInspectionV1, RunManager, TimelinePageV1,
};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

fn control_manager(st: &AppState) -> Result<Arc<RunManager>, ApiError> {
    st.control.as_ref().cloned().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "canonical control runtime is not available".to_string(),
        ))
    })
}

fn control_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

struct ControlIdentity {
    device_id: Option<String>,
    device_key: Option<String>,
}

fn control_identity(
    st: &AppState,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> Result<ControlIdentity, ApiError> {
    if let Some(key) = control_bearer_token(headers) {
        if let Some(device) = st
            .mobile_companion
            .as_ref()
            .and_then(|bridge| bridge.authenticate_device(key, now_unix()))
        {
            return Ok(ControlIdentity {
                device_id: Some(device.id),
                device_key: Some(key.to_string()),
            });
        }
    }
    if st.mobile_control_only {
        return Err(ApiError(Error::Unauthorized(
            "missing or invalid paired-device credential".to_string(),
        )));
    }
    authorize(st, headers, peer)?;
    Ok(ControlIdentity {
        device_id: None,
        device_key: None,
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct ControlTimelineQuery {
    after_seq: Option<u64>,
    before_seq: Option<u64>,
    tail: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ControlSocketQuery {
    ticket: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ControlAppearanceBackgroundQuery {
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ControlRunEventsQuery {
    after_seq: Option<u64>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ControlAttachmentUploadQuery {
    name: String,
    size: u64,
}

/// `GET /control/v1/bootstrap`
pub(crate) async fn control_bootstrap(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    Ok(Json(manager.bootstrap(&st).await.map_err(ApiError)?).into_response())
}

/// `GET /control/v1/appearance/background`
pub(crate) async fn control_appearance_background(
    State(st): State<AppState>,
    Query(query): Query<ControlAppearanceBackgroundQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let appearance = manager.appearance_snapshot();
    if query
        .revision
        .as_deref()
        .is_some_and(|revision| revision != appearance.revision)
    {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error": { "message": "appearance changed; refresh bootstrap" },
                "revision": appearance.revision,
            })),
        )
            .into_response());
    }
    let Some(asset) = manager.appearance_background_asset() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut response = asset.bytes.into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(asset.mime));
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!("\"{}\"", asset.revision)) {
        response.headers_mut().insert(ETAG, value);
    }
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

/// `GET /control/v1/threads/{id}/timeline`
pub(crate) async fn control_timeline(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ControlTimelineQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let limit = query.tail.or(query.limit).unwrap_or(100).clamp(1, 500);
    let tail = query.tail.is_some() || (query.after_seq.is_none() && query.before_seq.is_none());
    let thread_id = id.clone();
    let page: TimelinePageV1 = crate::blocking::run(move || {
        manager.timeline_page(&thread_id, query.after_seq, query.before_seq, tail, limit)
    })
    .await
    .map_err(ApiError)?
    .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    Ok(Json(page).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlApprovalAllowanceRevoke {
    /// Keys to revoke. Omitted revokes every allowance in the thread.
    #[serde(default)]
    keys: Option<Vec<String>>,
}

/// `GET /control/v1/threads/{id}/approval-allowances` — the thread's active
/// "Allow for this chat" rules.
pub(crate) async fn control_approval_allowances(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let allowances = manager.approval_allowances(&id).map_err(ApiError)?;
    Ok(Json(json!({ "thread_id": id, "allowances": allowances })).into_response())
}

/// `DELETE /control/v1/threads/{id}/approval-allowances` — revoke listed
/// rules (`{"keys": [...]}`) or, with no body, all of them.
pub(crate) async fn control_approval_allowances_revoke(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    body: Bytes,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let request: ControlApprovalAllowanceRevoke = if body.iter().all(u8::is_ascii_whitespace) {
        ControlApprovalAllowanceRevoke::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|error| ApiError(Error::InvalidRequest(format!("invalid body: {error}"))))?
    };
    let allowances = manager
        .revoke_approval_allowances(&id, request.keys.as_deref())
        .map_err(ApiError)?;
    Ok(Json(json!({ "thread_id": id, "allowances": allowances })).into_response())
}

/// `GET /control/v1/runs/{run_id}` — loaded only when an existing work
/// surface explicitly asks for details.
pub(crate) async fn control_run_inspection(
    State(st): State<AppState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let id = run_id.clone();
    let inspection: RunInspectionV1 = crate::blocking::run(move || manager.run_inspection(&id))
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("run {run_id}"))))?;
    Ok(Json(inspection).into_response())
}

/// `POST /control/v1/threads/{id}/effective-run` — resolve the next run
/// without accepting a turn or mutating canonical state.
pub(crate) async fn control_effective_run_preview(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(request): Json<EffectiveRunPreviewRequestV1>,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let preview: EffectiveRunPreviewV1 = manager
        .effective_run_preview(&st, &id, request)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    Ok(Json(preview).into_response())
}

/// `PUT /control/v1/attachments/{id}` — stage one paired-device attachment
/// without copying its binary payload through the React Native JavaScript heap.
pub(crate) async fn control_attachment_upload(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ControlAttachmentUploadQuery>,
    headers: HeaderMap,
    peer: Peer,
    body: Bytes,
) -> Result<Response, ApiError> {
    let identity = control_identity(&st, &headers, peer_addr(peer))?;
    let device_id = identity.device_id.ok_or_else(|| {
        ApiError(Error::Unauthorized(
            "attachment uploads require a paired-device credential".to_string(),
        ))
    })?;
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream");
    let manager = control_manager(&st)?;
    let upload: ControlAttachmentUploadV1 = manager
        .put_attachment_upload(
            &device_id,
            &id,
            &query.name,
            mime,
            query.size,
            body.to_vec(),
        )
        .map_err(ApiError)?;
    Ok(Json(upload).into_response())
}

/// `GET /control/v1/runs/{run_id}/events` — bounded, forward-only ledger
/// pagination for the nested run-details surface.
pub(crate) async fn control_run_events(
    State(st): State<AppState>,
    Path(run_id): Path<String>,
    Query(query): Query<ControlRunEventsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let id = run_id.clone();
    let page: RunEventPageV1 = crate::blocking::run(move || {
        manager.run_event_page(&id, query.after_seq, query.limit.unwrap_or(100))
    })
    .await
    .map_err(ApiError)?
    .ok_or_else(|| ApiError(Error::ModelNotFound(format!("run {run_id}"))))?;
    Ok(Json(page).into_response())
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlRunReplayRequest {
    /// Model step to replay. Defaults to the run's last model step.
    #[serde(default)]
    step: Option<usize>,
    /// Provider or local model to send to instead of the recorded one.
    #[serde(default)]
    model: Option<String>,
    /// Return the reconstructed request without calling a model.
    #[serde(default)]
    dry_run: bool,
}

/// One side of a replay comparison.
#[derive(Debug, Serialize)]
struct ReplayResponseV1 {
    model: String,
    content: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    finish_reason: String,
    usage: Value,
    latency_ms: Option<u64>,
    first_token_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ReplayDiffV1 {
    same_finish_reason: bool,
    same_tool_call_count: bool,
    same_tool_names: bool,
    same_tool_arguments: bool,
    /// Character-bigram Dice similarity of the visible text, `0..1`.
    text_similarity: f64,
}

/// A recorded model step, reconstructed from the run ledger.
struct StoredModelStep {
    step: usize,
    request: Value,
    privacy: crate::privacy::PrivacyMode,
    response: Option<ReplayResponseV1>,
}

const ACCOUNT_RUNTIME_PREFIXES: [&str; 4] = ["codex:", "claude:", "opencode:", "pi:"];

fn ledger_step_number(step_id: Option<&str>) -> Option<usize> {
    step_id?.strip_prefix("step-")?.parse().ok()
}

fn stored_artifact(
    store: &milim_storage::UserDataStore,
    run_id: &str,
    data: &Value,
) -> Result<Value, Error> {
    let digest = data
        .get("artifact_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Other("run event is missing artifact_digest".into()))?;
    let artifact = store
        .control_run_artifact(run_id, digest)?
        .ok_or_else(|| Error::Other(format!("run artifact {digest} is missing")))?;
    serde_json::from_str(&artifact.data_json)
        .map_err(|error| Error::Other(format!("invalid stored run artifact: {error}")))
}

/// Load the exact provider request, response, and timing a run recorded for
/// one model step.
fn stored_model_step(
    store: &milim_storage::UserDataStore,
    run_id: &str,
    step: Option<usize>,
) -> Result<StoredModelStep, Error> {
    let events = store.control_run_events_by_types(
        run_id,
        &[
            "model_request_resolved",
            "model_response_committed",
            "model_timing",
            "harness_request_committed",
        ],
    )?;
    let requests = events
        .iter()
        .filter(|event| event.event_type == "model_request_resolved")
        .filter_map(|event| Some((ledger_step_number(event.step_id.as_deref())?, event)))
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return Err(Error::InvalidRequest(
            if events
                .iter()
                .any(|event| event.event_type == "harness_request_committed")
            {
                "account-runtime runs record a harness boundary, not a provider request; they cannot be replayed"
            } else {
                "run has no recorded model request to replay"
            }
            .into(),
        ));
    }
    let step = match step {
        Some(step) => step,
        None => requests.iter().map(|(step, _)| *step).max().unwrap_or(1),
    };
    let (_, request_event) = requests
        .iter()
        .rev()
        .find(|(candidate, _)| *candidate == step)
        .ok_or_else(|| {
            Error::InvalidRequest(format!("run has no model request for step {step}"))
        })?;
    let request_data: Value = serde_json::from_str(&request_event.data_json)
        .map_err(|error| Error::Other(format!("invalid stored run event: {error}")))?;
    let request = stored_artifact(store, run_id, &request_data)?;
    let privacy = crate::privacy::PrivacyMode::parse(
        request_data
            .get("privacy")
            .and_then(Value::as_str)
            .unwrap_or("off"),
    );
    let in_step = |event_type: &str| {
        events.iter().rev().find(|event| {
            event.event_type == event_type
                && ledger_step_number(event.step_id.as_deref()) == Some(step)
        })
    };
    let timing = in_step("model_timing")
        .and_then(|event| serde_json::from_str::<Value>(&event.data_json).ok());
    let response = match in_step("model_response_committed") {
        Some(event) => {
            let data: Value = serde_json::from_str(&event.data_json)
                .map_err(|error| Error::Other(format!("invalid stored run event: {error}")))?;
            let artifact = stored_artifact(store, run_id, &data)?;
            let text = |key: &str| {
                artifact
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let timing_ms = |key: &str| timing.as_ref()?.get(key)?.as_u64();
            Some(ReplayResponseV1 {
                model: request
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                content: text("content"),
                reasoning: text("reasoning"),
                tool_calls: serde_json::from_value(
                    artifact
                        .get("tool_calls")
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                )
                .unwrap_or_default(),
                finish_reason: text("finish_reason"),
                usage: artifact.get("usage").cloned().unwrap_or(Value::Null),
                latency_ms: timing_ms("duration_ms").or_else(|| {
                    u64::try_from(event.created_at_ms - request_event.created_at_ms).ok()
                }),
                first_token_ms: timing_ms("first_token_ms"),
            })
        }
        None => None,
    };
    Ok(StoredModelStep {
        step,
        request,
        privacy,
        response,
    })
}

/// Re-send a reconstructed request through the same privacy-scoped service
/// normal runs use. Tool calls are returned, never executed.
async fn replay_model_request(
    st: &AppState,
    privacy: crate::privacy::PrivacyMode,
    request: CompletionRequest,
) -> Result<ReplayResponseV1, Error> {
    let context = RunContext::from_control(st, None, privacy.as_str())?;
    let service = service_for_run(st, &context);
    let model = request.model.clone();
    let started = Instant::now();
    let mut stream = service.stream(request).await?;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tools = ToolCallAccumulator::default();
    let mut first_token_ms = None;
    let mut done = None;
    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::Delta(delta) => {
                if first_token_ms.is_none()
                    && (delta.content.is_some()
                        || delta.reasoning.is_some()
                        || !delta.tool_calls.is_empty())
                {
                    first_token_ms = Some(elapsed_ms(started));
                }
                content.push_str(delta.content.as_deref().unwrap_or_default());
                reasoning.push_str(delta.reasoning.as_deref().unwrap_or_default());
                for call in delta.tool_calls {
                    tools.push(call);
                }
            }
            StreamEvent::Done {
                finish_reason,
                usage,
            } => {
                done = Some((finish_reason, usage));
                break;
            }
        }
    }
    let (finish_reason, usage) =
        done.ok_or_else(|| Error::Other("provider stream ended without a terminal event".into()))?;
    Ok(ReplayResponseV1 {
        model,
        content,
        reasoning,
        tool_calls: tools.finish(),
        finish_reason,
        usage: serde_json::to_value(usage).unwrap_or(Value::Null),
        latency_ms: Some(elapsed_ms(started)),
        first_token_ms,
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn stricter_privacy(
    a: crate::privacy::PrivacyMode,
    b: crate::privacy::PrivacyMode,
) -> crate::privacy::PrivacyMode {
    if (b as u8) > (a as u8) {
        b
    } else {
        a
    }
}

fn bigram_similarity(a: &str, b: &str) -> f64 {
    fn bigrams(text: &str) -> HashMap<(char, char), usize> {
        let chars = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
            .chars()
            .collect::<Vec<_>>();
        let mut counts = HashMap::new();
        for pair in chars.windows(2) {
            *counts.entry((pair[0], pair[1])).or_default() += 1;
        }
        counts
    }
    let (left, right) = (bigrams(a), bigrams(b));
    let total = left.values().sum::<usize>() + right.values().sum::<usize>();
    if total == 0 {
        return if a.trim() == b.trim() { 1.0 } else { 0.0 };
    }
    let shared = left
        .iter()
        .map(|(pair, count)| (*count).min(right.get(pair).copied().unwrap_or(0)))
        .sum::<usize>();
    (2 * shared) as f64 / total as f64
}

fn replay_diff(original: &ReplayResponseV1, replay: &ReplayResponseV1) -> ReplayDiffV1 {
    let names = |response: &ReplayResponseV1| {
        response
            .tool_calls
            .iter()
            .map(|call| call.function.name.clone())
            .collect::<Vec<_>>()
    };
    let arguments = |response: &ReplayResponseV1| {
        response
            .tool_calls
            .iter()
            .map(|call| {
                serde_json::from_str::<Value>(&call.function.arguments)
                    .unwrap_or_else(|_| Value::String(call.function.arguments.clone()))
            })
            .collect::<Vec<_>>()
    };
    let same_tool_names = names(original) == names(replay);
    ReplayDiffV1 {
        same_finish_reason: original.finish_reason == replay.finish_reason,
        same_tool_call_count: original.tool_calls.len() == replay.tool_calls.len(),
        same_tool_names,
        same_tool_arguments: same_tool_names && arguments(original) == arguments(replay),
        text_similarity: bigram_similarity(&original.content, &replay.content),
    }
}

async fn replay_run(
    st: &AppState,
    store: Arc<milim_storage::UserDataStore>,
    run_id: String,
    request: ControlRunReplayRequest,
) -> Result<Option<Value>, Error> {
    let model = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string);
    if model.as_deref().is_some_and(|model| {
        ACCOUNT_RUNTIME_PREFIXES.iter().any(|prefix| {
            model
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        })
    }) {
        return Err(Error::InvalidRequest(
            "replay sends provider requests; choose a provider or local model, not an account runtime".into(),
        ));
    }
    let step = request.step;
    let lookup = run_id.clone();
    let stored = crate::blocking::run(move || {
        if store.control_run(&lookup)?.is_none() {
            return Ok(None);
        }
        stored_model_step(&store, &lookup, step).map(Some)
    })
    .await?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    // A replay never runs with a weaker gate than the run recorded or the
    // current default.
    let privacy = stricter_privacy(stored.privacy, st.privacy.mode());
    let mut provider_request = crate::control::completion_request_from_value(&stored.request)?;
    if let Some(model) = model {
        provider_request.model = model;
    }
    if request.dry_run {
        return Ok(Some(json!({
            "run_id": run_id,
            "step": stored.step,
            "dry_run": true,
            "privacy": privacy.as_str(),
            "request": crate::control::completion_request_value(&provider_request)?,
        })));
    }
    let replay = replay_model_request(st, privacy, provider_request).await?;
    let diff = stored
        .response
        .as_ref()
        .map(|original| replay_diff(original, &replay));
    Ok(Some(json!({
        "run_id": run_id,
        "step": stored.step,
        "dry_run": false,
        "privacy": privacy.as_str(),
        "original": stored.response,
        "replay": replay,
        "diff": diff,
    })))
}

/// `POST /control/v1/runs/{run_id}/replay` — re-send one recorded model step
/// to the same or another model without executing tools, and compare the
/// responses. `dry_run` returns the reconstructed request only.
pub(crate) async fn control_run_replay(
    State(st): State<AppState>,
    Path(run_id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    body: Bytes,
) -> Result<Response, ApiError> {
    control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let request: ControlRunReplayRequest = if body.iter().all(u8::is_ascii_whitespace) {
        ControlRunReplayRequest::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|error| ApiError(Error::InvalidRequest(format!("invalid body: {error}"))))?
    };
    let replay = replay_run(&st, manager.store().clone(), run_id.clone(), request)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("run {run_id}"))))?;
    Ok(Json(replay).into_response())
}

/// `POST /control/v1/commands`
pub(crate) async fn control_command(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(command): Json<ControlCommandV1>,
) -> Result<Response, ApiError> {
    let identity = control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    Ok(Json(
        manager
            .command(st.clone(), identity.device_id, command)
            .await
            .map_err(ApiError)?,
    )
    .into_response())
}

/// `POST /control/v1/socket-ticket`
pub(crate) async fn control_socket_ticket(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    let identity = control_identity(&st, &headers, peer_addr(peer))?;
    let manager = control_manager(&st)?;
    let (ticket, expires_in_seconds) = manager.issue_socket_ticket(identity.device_key);
    Ok(Json(json!({
        "ticket": ticket,
        "expires_in_seconds": expires_in_seconds,
        "single_use": true,
    }))
    .into_response())
}

/// `GET /control/v1/ws?ticket=...`
pub(crate) async fn control_socket(
    State(st): State<AppState>,
    Query(query): Query<ControlSocketQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let manager = control_manager(&st)?;
    let ticket = manager
        .take_socket_ticket(query.ticket.trim())
        .ok_or_else(|| {
            ApiError(Error::Unauthorized(
                "invalid, expired, or already-used control socket ticket".to_string(),
            ))
        })?;
    if let Some(key) = ticket.device_key.as_deref() {
        let valid = st
            .mobile_companion
            .as_ref()
            .and_then(|bridge| bridge.authenticate_device(key, now_unix()))
            .is_some();
        if !valid {
            return Err(ApiError(Error::Unauthorized(
                "paired device was revoked".to_string(),
            )));
        }
    }
    Ok(ws
        .on_upgrade(move |socket| control_socket_loop(socket, st, manager, ticket.device_key))
        .into_response())
}

async fn control_socket_loop(
    mut socket: WebSocket,
    state: AppState,
    manager: Arc<RunManager>,
    device_key: Option<String>,
) {
    let mut events = manager.subscribe();
    let mut auth_check = tokio::time::interval(Duration::from_secs(5));
    auth_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = auth_check.tick() => {
                if let Some(key) = device_key.as_deref() {
                    let valid = state
                        .mobile_companion
                        .as_ref()
                        .and_then(|bridge| bridge.authenticate_device(key, now_unix()))
                        .is_some();
                    if !valid {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    _ => {}
                }
            }
            event = events.recv() => {
                let event = match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => ControlEventV1 {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        host_id: manager.host().host_id,
                        thread_id: None,
                        epoch: None,
                        seq: None,
                        event_type: "sync.required".to_string(),
                        data: json!({ "reason": "event_gap", "skipped": skipped }),
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                };
                let Ok(text) = serde_json::to_string(&event) else {
                    continue;
                };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use milim_core::api::openai::Model;
    use milim_core::config::ServerConfiguration;
    use milim_inference::{DeltaEvent, EventStream, ModelService};
    use milim_storage::{ControlRunArtifactRecord, ControlRunRecord, Database, UserDataStore};
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers every request with a fixed reply and one tool call, and counts
    /// how many requests actually left the process.
    #[derive(Clone)]
    struct ScriptedBackend {
        remote: bool,
        calls: Arc<AtomicUsize>,
        models: Arc<Mutex<Vec<String>>>,
    }

    impl ScriptedBackend {
        fn new(remote: bool) -> Self {
            Self {
                remote,
                calls: Arc::new(AtomicUsize::new(0)),
                models: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl ModelService for ScriptedBackend {
        fn name(&self) -> &str {
            "scripted"
        }

        fn requires_privacy_gate(&self) -> bool {
            self.remote
        }

        async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
            Ok(vec![Model::local("scripted", 0)])
        }

        async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.models.lock().unwrap().push(req.model.clone());
            let delta =
                |content: Option<&str>, tool: Option<milim_core::api::openai::DeltaToolCall>| {
                    StreamEvent::Delta(DeltaEvent {
                        content: content.map(str::to_string),
                        tool_calls: tool.into_iter().collect(),
                        ..Default::default()
                    })
                };
            let events = vec![
                Ok(delta(Some("I will read "), None)),
                Ok(delta(Some("the file."), None)),
                Ok(delta(
                    None,
                    Some(milim_core::api::openai::DeltaToolCall {
                        index: 0,
                        id: Some("call-new".into()),
                        kind: Some("function".into()),
                        function: milim_core::api::openai::DeltaFunction {
                            name: Some("read_file".into()),
                            arguments: Some("{\"path\": \"a.txt\"}".into()),
                        },
                    }),
                )),
                Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".into(),
                    usage: Usage::new(10, 5),
                }),
            ];
            Ok(Box::pin(futures::stream::iter(events)))
        }

        async fn embed(
            &self,
            _model: &str,
            _inputs: Vec<String>,
        ) -> milim_core::Result<Vec<Vec<f32>>> {
            Ok(Vec::new())
        }
    }

    fn put_artifact(store: &UserDataStore, run_id: &str, kind: &str, data: &Value) -> String {
        let encoded = serde_json::to_vec(data).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(&encoded));
        store
            .control_put_run_artifact(&ControlRunArtifactRecord {
                run_id: run_id.into(),
                digest: digest.clone(),
                kind: kind.into(),
                data_json: String::from_utf8(encoded).unwrap(),
                byte_len: 0,
                created_at_ms: 1,
            })
            .unwrap();
        digest
    }

    fn request(text: &str) -> CompletionRequest {
        CompletionRequest {
            model: "recorded-model".into(),
            messages: vec![
                ChatMessage::text("system", "fixture instructions"),
                ChatMessage::text("user", text),
            ],
            tools: serde_json::from_value(json!([{
                "type": "function",
                "function": {"name": "read_file", "description": "Read", "parameters": {"type": "object"}}
            }]))
            .unwrap(),
            tool_choice: Some(json!("auto")),
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams {
                temperature: Some(0.7),
                max_tokens: Some(512),
                stop: vec!["END".into()],
                ..SamplingParams::default()
            },
            reasoning_effort: Some(ReasoningEffort::High),
        }
    }

    fn record_step(
        store: &UserDataStore,
        run_id: &str,
        step: usize,
        text: &str,
        privacy: &str,
    ) -> Vec<u8> {
        let value = crate::control::completion_request_value(&request(text)).unwrap();
        let digest = put_artifact(store, run_id, "provider_request", &value);
        let step_id = format!("step-{step}");
        store
            .control_append_run_event(
                run_id,
                &uuid::Uuid::new_v4().to_string(),
                Some(&step_id),
                "model_request_resolved",
                &json!({"artifact_digest": digest, "privacy": privacy}).to_string(),
            )
            .unwrap();
        let response = json!({
            "content": "I will read the file.",
            "reasoning": "",
            "tool_calls": [{
                "id": "call-old",
                "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}
            }],
            "finish_reason": "tool_calls",
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        });
        let digest = put_artifact(store, run_id, "provider_response", &response);
        store
            .control_append_run_event(
                run_id,
                &uuid::Uuid::new_v4().to_string(),
                Some(&step_id),
                "model_response_committed",
                &json!({"artifact_digest": digest, "finish_reason": "tool_calls"}).to_string(),
            )
            .unwrap();
        serde_json::to_vec(&value).unwrap()
    }

    fn fixture(backend: ScriptedBackend) -> (AppState, Arc<UserDataStore>) {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        for (id, adapter) in [("run-1", "provider"), ("run-codex", "codex")] {
            store
                .control_put_run(&ControlRunRecord {
                    id: id.into(),
                    thread_id: "thread-1".into(),
                    status: "completed".into(),
                    adapter: adapter.into(),
                    request_json: "{}".into(),
                    agent_snapshot_json: None,
                    native_session_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                    completed_at_ms: Some(2),
                    error_json: None,
                })
                .unwrap();
        }
        let state = AppState::new(Arc::new(backend), ServerConfiguration::default());
        (state, store)
    }

    fn replay_request(value: Value) -> ControlRunReplayRequest {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn dry_run_reconstructs_the_stored_request_byte_for_byte() {
        let (state, store) = fixture(ScriptedBackend::new(false));
        record_step(&store, "run-1", 1, "first step", "off");
        let second = record_step(&store, "run-1", 2, "second step", "off");

        let dry = replay_run(
            &state,
            store.clone(),
            "run-1".into(),
            replay_request(json!({"dry_run": true})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(dry["step"], 2, "defaults to the last model step");
        assert_eq!(serde_json::to_vec(&dry["request"]).unwrap(), second);

        let first = replay_run(
            &state,
            store.clone(),
            "run-1".into(),
            replay_request(json!({"dry_run": true, "step": 1, "model": "other-model"})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(first["request"]["model"], "other-model");
        assert_eq!(first["request"]["messages"][1]["content"], "first step");
        assert!(replay_run(
            &state,
            store.clone(),
            "missing".into(),
            ControlRunReplayRequest::default()
        )
        .await
        .unwrap()
        .is_none());
        assert!(replay_run(
            &state,
            store,
            "run-1".into(),
            replay_request(json!({"step": 9}))
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn replay_resends_without_tools_executing_and_compares_responses() {
        let backend = ScriptedBackend::new(false);
        let (state, store) = fixture(backend.clone());
        record_step(&store, "run-1", 1, "read a.txt", "off");

        let result = replay_run(
            &state,
            store.clone(),
            "run-1".into(),
            replay_request(json!({"model": "replacement-model"})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*backend.models.lock().unwrap(), vec!["replacement-model"]);
        assert_eq!(result["replay"]["model"], "replacement-model");
        assert_eq!(result["replay"]["content"], "I will read the file.");
        assert_eq!(
            result["replay"]["tool_calls"][0]["function"]["name"],
            "read_file"
        );
        assert_eq!(result["replay"]["usage"]["total_tokens"], 15);
        assert!(result["replay"]["latency_ms"].is_u64());
        assert_eq!(result["original"]["model"], "recorded-model");
        assert_eq!(result["diff"]["same_tool_names"], true);
        assert_eq!(result["diff"]["same_tool_arguments"], true, "{result}");
        assert_eq!(result["diff"]["same_finish_reason"], true);
        assert_eq!(result["diff"]["text_similarity"], 1.0);
        // Replay reads the ledger; it never appends to it.
        assert_eq!(
            store.control_run_events("run-1", None, 50).unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn replay_respects_the_recorded_and_current_privacy_gates() {
        let backend = ScriptedBackend::new(true);
        let (state, store) = fixture(backend.clone());
        record_step(&store, "run-1", 1, "mail private@example.com", "block");
        let blocked = replay_run(
            &state,
            store.clone(),
            "run-1".into(),
            ControlRunReplayRequest::default(),
        )
        .await;
        assert!(blocked.unwrap_err().to_string().contains("privacy gate"));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);

        let backend = ScriptedBackend::new(true);
        let (state, store) = fixture(backend.clone());
        record_step(&store, "run-1", 1, "mail private@example.com", "off");
        state.privacy.set(crate::privacy::PrivacyMode::Block);
        let dry = replay_run(
            &state,
            store.clone(),
            "run-1".into(),
            replay_request(json!({"dry_run": true})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(dry["privacy"], "block");
        assert!(replay_run(
            &state,
            store,
            "run-1".into(),
            ControlRunReplayRequest::default()
        )
        .await
        .is_err());
        assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn account_runtime_runs_and_targets_are_rejected() {
        let (state, store) = fixture(ScriptedBackend::new(false));
        let digest = put_artifact(&store, "run-codex", "harness_boundary_request", &json!({}));
        store
            .control_append_run_event(
                "run-codex",
                "event-codex",
                Some("step-1"),
                "harness_request_committed",
                &json!({"artifact_digest": digest}).to_string(),
            )
            .unwrap();
        let error = replay_run(
            &state,
            store.clone(),
            "run-codex".into(),
            ControlRunReplayRequest::default(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("account-runtime"));

        record_step(&store, "run-1", 1, "hello", "off");
        assert!(replay_run(
            &state,
            store,
            "run-1".into(),
            replay_request(json!({"model": "codex:gpt-5"}))
        )
        .await
        .is_err());
    }

    #[test]
    fn text_similarity_is_symmetric_and_bounded() {
        assert_eq!(bigram_similarity("", ""), 1.0);
        assert_eq!(bigram_similarity("same text", "Same   text"), 1.0);
        assert_eq!(bigram_similarity("abc", "xyz"), 0.0);
        let partial = bigram_similarity("read the file", "read a file");
        assert!(partial > 0.4 && partial < 1.0);
        assert_eq!(partial, bigram_similarity("read a file", "read the file"));
    }
}
