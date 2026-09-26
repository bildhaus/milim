//! Workspace checkpoints taken before a turn runs.

use milim_control_contract::FrozenRunConfigV1;
use serde_json::{json, Value};

use super::RunManager;

impl RunManager {
    /// Checkpoint the thread's Git workspace before a run that may change
    /// files, whichever client started it, and record the outcome on the
    /// run's timeline as a `workspace_checkpoint` item.
    pub(super) async fn checkpoint_turn_workspace(
        &self,
        thread_id: &str,
        run_id: &str,
        config: &FrozenRunConfigV1,
    ) {
        let Some(folder) = turn_checkpoint_folder(config) else {
            return;
        };
        let label = run_id.to_string();
        let outcome = tokio::task::spawn_blocking(move || {
            crate::routes::turn_workspace_checkpoint(&folder, &label)
        })
        .await;
        let data = match outcome {
            Ok(Ok(checkpoint)) => {
                let checkpoint = serde_json::to_value(checkpoint).unwrap_or(Value::Null);
                self.turn_checkpoints
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(run_id.to_string(), checkpoint.clone());
                json!({ "status": "created", "checkpoint": checkpoint })
            }
            Ok(Err(skip)) => json!({
                "status": "skipped",
                "reason": if skip.not_git { "not_git" } else { "error" },
                "message": skip.message,
            }),
            Err(error) => json!({
                "status": "skipped",
                "reason": "error",
                "message": format!("Workspace checkpoint failed: {error}"),
            }),
        };
        let _ = self.persist_and_emit(thread_id, Some(run_id), "workspace_checkpoint", data);
    }
}

/// The folder to checkpoint before a run, or `None` when the run cannot
/// change files: Plan mode, no folder, or a provider turn without tools.
pub(super) fn turn_checkpoint_folder(config: &FrozenRunConfigV1) -> Option<std::path::PathBuf> {
    if config.plan_mode {
        return None;
    }
    let tool_mode = config
        .agent
        .as_ref()
        .map_or(config.tool_mode.as_str(), |agent| agent.tool_mode.as_str());
    let may_change_files = match config.adapter.as_str() {
        "codex" | "claude" | "opencode" | "pi" => true,
        "provider" => tool_mode != "none",
        _ => false,
    };
    let folder = config.workspace.as_deref().map(str::trim)?;
    (may_change_files && !folder.is_empty()).then(|| std::path::PathBuf::from(folder))
}
