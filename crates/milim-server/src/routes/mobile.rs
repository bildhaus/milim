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

/// `GET /mobile` - unauthenticated native-client discovery probe. It exposes
/// identity and protocol only; all state and mutations remain authenticated.
pub(crate) async fn mobile_companion_probe(
    State(st): State<AppState>,
) -> Result<Response, ApiError> {
    mobile_bridge(&st)?;
    let control = st.control.as_ref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "canonical control runtime is not available".to_string(),
        ))
    })?;
    let host = control.host();
    Ok(Json(json!({
        "service": "milim-mobile-control",
        "host_id": host.host_id,
        "host_name": host.display_name,
        "protocol": {
            "min": crate::control::CONTROL_PROTOCOL_MIN,
            "max": crate::control::CONTROL_PROTOCOL_MAX,
        },
    }))
    .into_response())
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

/// `DELETE /mobile/device` - revoke the currently authenticated phone. This
/// is deliberately scoped to the caller and is safe to expose on the narrow
/// mobile router; revoking another device remains desktop-only.
pub(crate) async fn mobile_companion_device_revoke_self(
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
    let status = bridge.revoke_device(&device.id, now_unix());
    Ok(Json(json!({
        "revoked": true,
        "device_id": device.id,
        "status": status,
    }))
    .into_response())
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
