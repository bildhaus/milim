use super::*;

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
) -> Result<Response, ApiError> {
    let bridge = mobile_bridge(&st)?;
    let key = companion_device_key(&headers)?.to_string();
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

const MOBILE_COMPANION_HTML: &str = include_str!("mobile.html");

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
    include_str!("../../../../apps/desktop/public/milim-wordmark.svg");
const MOBILE_COMPANION_ICON_PNG: &[u8] =
    include_bytes!("../../../../apps/desktop/src-tauri/icons/icon.png");
