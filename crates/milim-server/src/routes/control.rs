use super::*;

use crate::control::{ControlCommandV1, ControlEventV1, RunManager, TimelinePageV1};
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
    let page: TimelinePageV1 = manager
        .timeline_page(&id, query.after_seq, query.before_seq, tail, limit)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    Ok(Json(page).into_response())
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
