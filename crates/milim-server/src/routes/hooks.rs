use super::*;

use crate::user_hooks;

// ----- User hooks (settings.json) and project hook trust -----

#[derive(Debug, Deserialize)]
pub(crate) struct HooksQuery {
    /// Thread workspace. Omitted uses the current host workspace; empty shows
    /// user hooks only.
    #[serde(default)]
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HooksTrustRequest {
    workspace: String,
    /// Hash of the reviewed `hooks` config, from `GET /hooks` or a timeline
    /// notice.
    config_hash: String,
    #[serde(default = "default_trusted")]
    trusted: bool,
}

fn default_trusted() -> bool {
    true
}

fn hooks_workspace(st: &AppState, workspace: Option<&str>) -> milim_core::Result<Option<PathBuf>> {
    match workspace.map(str::trim) {
        None => Ok(workspace_snapshot(st)),
        Some("") => Ok(None),
        Some(path) => {
            let canonical = std::fs::canonicalize(path).map_err(|error| {
                Error::InvalidRequest(format!("invalid workspace {path}: {error}"))
            })?;
            if !canonical.is_dir() {
                return Err(Error::InvalidRequest(format!(
                    "workspace is not a directory: {}",
                    canonical.display()
                )));
            }
            Ok(Some(canonical))
        }
    }
}

/// `GET /hooks?workspace=<path>` - user hooks plus the workspace's project
/// hooks and whether they are trusted.
pub(crate) async fn hooks_status(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<HooksQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let workspace = hooks_workspace(&st, query.workspace.as_deref()).map_err(ApiError)?;
    let status = tokio::task::spawn_blocking(move || user_hooks::status(workspace.as_deref()))
        .await
        .map_err(|error| ApiError(Error::Other(format!("hook status task failed: {error}"))))?;
    Ok(Json(status).into_response())
}

/// `POST /hooks/trust` - trust (or stop trusting) a workspace's reviewed
/// project hooks.
pub(crate) async fn hooks_trust(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<HooksTrustRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let workspace = hooks_workspace(&st, Some(&req.workspace))
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::InvalidRequest("workspace is required".into())))?;
    let status = tokio::task::spawn_blocking(move || {
        user_hooks::set_trust(&workspace, req.config_hash.trim(), req.trusted)
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("hook trust task failed: {error}"))))?
    .map_err(ApiError)?;
    Ok(Json(status).into_response())
}
