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

#[derive(Debug, Deserialize)]
pub(crate) struct HarnessMetricsParams {
    /// Rolling window ending now. Ignored when `since_ms` is set.
    #[serde(default)]
    days: Option<u32>,
    #[serde(default)]
    since_ms: Option<i64>,
    #[serde(default)]
    until_ms: Option<i64>,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// `GET /usage/harness?days=7&runtime=provider&model=...` - run-ledger health:
/// runs by status, model step latency and time to first token, per-tool error
/// rates, approval waits, steps and cost per run, and retries.
pub(crate) async fn usage_harness(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<HarnessMetricsParams>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st
        .control
        .as_ref()
        .map(|control| control.store().clone())
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "Harness metrics require the desktop's canonical store.".into(),
            ))
        })?;
    let days = query.days.unwrap_or(7);
    if days == 0 || days > milim_storage::USAGE_MAX_DAYS {
        return Err(ApiError(Error::InvalidRequest(format!(
            "metrics range must be between 1 and {} days",
            milim_storage::USAGE_MAX_DAYS
        ))));
    }
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(i64::MAX);
    let until_ms = query.until_ms.unwrap_or(now_ms);
    let since_ms = query
        .since_ms
        .unwrap_or_else(|| until_ms.saturating_sub(i64::from(days) * 86_400_000));
    let request = milim_storage::HarnessMetricsQuery {
        since_ms,
        until_ms,
        runtime: query.runtime,
        model: query.model,
    };
    let metrics = tokio::task::spawn_blocking(move || store.harness_metrics(&request))
        .await
        .map_err(|e| ApiError(Error::Other(format!("metrics task failed: {e}"))))?
        .map_err(ApiError)?;
    Ok(Json(metrics).into_response())
}
