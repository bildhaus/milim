use super::*;

use crate::custom_commands::{self, CustomCommand};

// ----- Custom slash commands (Markdown prompt templates) -----

#[derive(Debug, Deserialize)]
pub(crate) struct CustomCommandsQuery {
    /// Thread workspace. Omitted uses the current host workspace; empty lists
    /// user commands only.
    #[serde(default)]
    workspace: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CustomCommandExpandRequest {
    #[serde(default)]
    workspace: Option<String>,
    name: String,
    #[serde(default)]
    arguments: String,
}

fn custom_command_workspace(
    st: &AppState,
    workspace: Option<&str>,
) -> milim_core::Result<Option<PathBuf>> {
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

async fn discover_custom_commands(
    workspace: Option<PathBuf>,
) -> Result<Vec<CustomCommand>, ApiError> {
    tokio::task::spawn_blocking(move || {
        custom_commands::discover(workspace.as_deref(), custom_commands::home_dir().as_deref())
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("custom command task failed: {error}"))))
}

/// `GET /commands?workspace=<path>` - list user and project slash commands.
pub(crate) async fn custom_commands_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Query(query): Query<CustomCommandsQuery>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let workspace = custom_command_workspace(&st, query.workspace.as_deref()).map_err(ApiError)?;
    let commands = discover_custom_commands(workspace).await?;
    Ok(Json(json!({ "commands": commands })).into_response())
}

/// `POST /commands/expand` - fill one command template with its arguments.
pub(crate) async fn custom_commands_expand(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CustomCommandExpandRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let workspace = custom_command_workspace(&st, req.workspace.as_deref()).map_err(ApiError)?;
    let name = req.name.clone();
    let command = tokio::task::spawn_blocking(move || {
        custom_commands::find(
            workspace.as_deref(),
            custom_commands::home_dir().as_deref(),
            &name,
        )
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("custom command task failed: {error}"))))?
    .ok_or_else(|| {
        ApiError(Error::NotFound(format!(
            "custom command not found: {}",
            req.name.trim()
        )))
    })?;
    let prompt = custom_commands::expand(&command.template, &req.arguments);
    Ok(Json(json!({
        "name": command.name,
        "source": command.source,
        "path": command.path,
        "prompt": prompt,
    }))
    .into_response())
}
