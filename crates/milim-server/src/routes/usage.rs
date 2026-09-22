use super::*;

// ----- Usage dashboard -----

#[derive(Debug, Deserialize)]
pub(crate) struct UsageSummaryQuery {
    #[serde(default)]
    days: Option<u32>,
    /// `Date#getTimezoneOffset()` of the viewer, so days follow their calendar.
    #[serde(default)]
    tz_offset_minutes: Option<i32>,
}

/// `GET /usage/summary?days=30&tz_offset_minutes=0` - tokens and cost by
/// day, model, provider/runtime, and project from the canonical message store.
pub(crate) async fn usage_summary(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<UsageSummaryQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .control
        .as_ref()
        .map(|control| control.store().clone())
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "Usage requires the desktop's canonical store.".into(),
            ))
        })?;
    let days = query.days.unwrap_or(30);
    let tz_offset_minutes = query.tz_offset_minutes.unwrap_or(0);
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX);
    let summary =
        tokio::task::spawn_blocking(move || store.usage_summary(days, now_ms, tz_offset_minutes))
            .await
            .map_err(|e| ApiError(Error::Other(format!("usage task failed: {e}"))))?
            .map_err(ApiError)?;
    Ok(Json(summary).into_response())
}
