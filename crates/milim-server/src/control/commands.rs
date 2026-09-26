//! Command intake: idempotent receipts, per-command and per-thread locks,
//! destructive confirmations, dispatch, and Worker run commands.

use std::sync::Arc;
use std::time::{Duration, Instant};

use milim_control_contract::{
    ControlCommandKindV1, ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1,
};
use milim_core::{Error, Result};
use milim_storage::ControlCommandReceiptRecord;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{now_ms, ConfirmationGrant, RunManager, ThreadPatch};
use crate::AppState;

const CONFIRMATION_TTL: Duration = Duration::from_secs(5 * 60);

impl RunManager {
    pub async fn command(
        self: &Arc<Self>,
        state: AppState,
        device_id: Option<String>,
        mut command: ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let _admission = self.mutation_guard()?;
        validate_command_id(&command.command_id)?;
        let command_lock = self.lock_for_command(&command.command_id);
        let _command_guard = command_lock.lock().await;
        let (store, command_id) = (self.store.clone(), command.command_id.clone());
        let receipt =
            crate::blocking::run(move || store.control_command_receipt(&command_id)).await?;
        if let Some(receipt) = receipt {
            return serde_json::from_str(&receipt.result_json).map_err(|error| {
                Error::Other(format!("stored control command result is invalid: {error}"))
            });
        }
        let attachment_upload_ids =
            self.resolve_command_attachment_uploads(device_id.as_deref(), &mut command)?;
        if command.kind.destructive() && !self.consume_confirmation(&command) {
            return Ok(self.confirmation_result(&command));
        }
        let thread_lock = if matches!(
            command.kind,
            ControlCommandKindV1::TurnSend
                | ControlCommandKindV1::ThreadLinkAdd
                | ControlCommandKindV1::ThreadLinkRemove
                | ControlCommandKindV1::TurnSteer
                | ControlCommandKindV1::ContextInject
                | ControlCommandKindV1::TurnInboxDelete
                | ControlCommandKindV1::TurnRegenerate
                | ControlCommandKindV1::TurnQueueResume
                | ControlCommandKindV1::TurnQueueMove
                | ControlCommandKindV1::TurnQueueDelete
                | ControlCommandKindV1::WorkerContinueSolo
        ) {
            command
                .thread_id
                .as_deref()
                .map(|thread_id| self.lock_for_thread(thread_id))
        } else {
            None
        };
        let _thread_guard = match thread_lock.as_ref() {
            Some(lock) => Some(lock.lock().await),
            None => None,
        };
        let receipt_command = command_for_receipt(&command);
        let request_json = serde_json::to_string(&receipt_command)
            .map_err(|error| Error::Other(format!("serialize control command: {error}")))?;
        let result = self.apply_command(state, command.clone()).await;
        let result_json = serde_json::to_string(&result)
            .map_err(|error| Error::Other(format!("serialize control result: {error}")))?;
        let receipt = ControlCommandReceiptRecord {
            command_id: command.command_id,
            device_id,
            thread_id: result.thread_id.clone(),
            command_kind: command.kind.as_str().to_string(),
            request_json,
            result_json,
            created_at_ms: now_ms(),
        };
        let store = self.store.clone();
        crate::blocking::run(move || store.control_put_command_receipt(&receipt)).await?;
        if matches!(
            result.status,
            ControlCommandStatusV1::Accepted
                | ControlCommandStatusV1::Queued
                | ControlCommandStatusV1::Applied
        ) {
            self.consume_attachment_uploads(&attachment_upload_ids);
        }
        Ok(result)
    }

    async fn apply_command(
        self: &Arc<Self>,
        state: AppState,
        command: ControlCommandV1,
    ) -> ControlCommandResultV1 {
        let result = match command.kind {
            ControlCommandKindV1::ThreadCreate => self.create_thread(&command),
            ControlCommandKindV1::ThreadRename => self.patch_thread(&command, ThreadPatch::Rename),
            ControlCommandKindV1::ThreadArchive => {
                self.patch_thread(&command, ThreadPatch::Archive)
            }
            ControlCommandKindV1::ThreadDelete => self.delete_thread(&command),
            ControlCommandKindV1::ThreadSetModel => self.patch_thread(&command, ThreadPatch::Model),
            ControlCommandKindV1::ThreadSetAgent => self.patch_thread(&command, ThreadPatch::Agent),
            ControlCommandKindV1::ThreadSetExecutionSettings => {
                self.patch_thread(&command, ThreadPatch::Execution)
            }
            ControlCommandKindV1::ThreadSetAccountProfile => {
                self.patch_thread(&command, ThreadPatch::AccountProfile)
            }
            ControlCommandKindV1::ThreadLinkAdd => self.link_thread(&command, true),
            ControlCommandKindV1::ThreadLinkRemove => self.link_thread(&command, false),
            ControlCommandKindV1::MessageDelete => self.delete_message(&command),
            ControlCommandKindV1::ModelFavoritesSet => self.set_model_favorites(&command),
            ControlCommandKindV1::TurnSend => self.accept_turn(state, &command).await,
            ControlCommandKindV1::TurnSteer => self.steer_turn(&state, &command),
            ControlCommandKindV1::ContextInject => self.inject_context(&command),
            ControlCommandKindV1::TurnInboxDelete => self.delete_inbox_input(&command),
            ControlCommandKindV1::TurnStop => self.stop_turn(&command),
            ControlCommandKindV1::TurnRegenerate => {
                self.regenerate_turn(state, &command, None).await
            }
            ControlCommandKindV1::TurnQueueResume => self.resume_queued_turn(state, &command).await,
            ControlCommandKindV1::TurnQueueMove => self.move_queued_turn(&command),
            ControlCommandKindV1::TurnQueueDelete => self.delete_queued_turn(&command),
            ControlCommandKindV1::ApprovalResolve => self.resolve_approval(&state, &command).await,
            ControlCommandKindV1::WorkerStart => self.worker_start(&state, &command).await,
            ControlCommandKindV1::WorkerStop => self.worker_stop(&state, &command, false),
            ControlCommandKindV1::WorkerContinueSolo => {
                self.worker_continue_solo(state, &command).await
            }
        };
        match result {
            Ok(result) => result,
            Err(error) => ControlCommandResultV1 {
                command_id: command.command_id,
                status: if error.to_string().contains("revision conflict") {
                    ControlCommandStatusV1::Conflict
                } else {
                    ControlCommandStatusV1::Failed
                },
                thread_id: command.thread_id,
                revision: None,
                run_id: None,
                queue_id: None,
                confirmation_token: None,
                message: Some(error.to_string()),
                data: Value::Null,
            },
        }
    }

    async fn worker_start(
        &self,
        state: &AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let run_id = required_payload_string(&command.payload, "run_id")?;
        let data = crate::routes::control_worker_run_start(state, &run_id).await?;
        let thread_id = data
            .get("run")
            .and_then(|run| run.get("parent_thread_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| command.thread_id.clone());
        if let Some(thread_id) = thread_id.as_deref() {
            self.emit(
                "worker.updated",
                Some(thread_id),
                self.store
                    .control_thread(thread_id)?
                    .as_ref()
                    .map(|thread| thread.epoch.as_str()),
                None,
                data.clone(),
            );
        }
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id,
            revision: None,
            run_id: Some(run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data,
        })
    }

    fn worker_stop(
        &self,
        state: &AppState,
        command: &ControlCommandV1,
        continue_solo: bool,
    ) -> Result<ControlCommandResultV1> {
        let run_id = required_payload_string(&command.payload, "run_id")?;
        let mut data = crate::routes::control_worker_run_stop(state, &run_id)?;
        if continue_solo {
            data["continue_solo"] = Value::Bool(true);
        }
        let thread_id = data
            .get("run")
            .and_then(|run| run.get("parent_thread_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| command.thread_id.clone());
        if let Some(thread_id) = thread_id.as_deref() {
            self.emit(
                "worker.updated",
                Some(thread_id),
                self.store
                    .control_thread(thread_id)?
                    .as_ref()
                    .map(|thread| thread.epoch.as_str()),
                None,
                data.clone(),
            );
        }
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id,
            revision: None,
            run_id: Some(run_id),
            queue_id: None,
            confirmation_token: None,
            message: continue_solo.then(|| {
                "Worker run stopped; the client may continue the parent turn with delegation disabled."
                    .to_string()
            }),
            data,
        })
    }

    async fn worker_continue_solo(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let stopped = self.worker_stop(&state, command, true)?;
        let thread_id = stopped
            .thread_id
            .clone()
            .ok_or_else(|| Error::Other("Worker run has no parent thread".into()))?;
        let mut continue_command = command.clone();
        continue_command.kind = ControlCommandKindV1::TurnRegenerate;
        continue_command.thread_id = Some(thread_id);
        continue_command.expected_revision = None;
        continue_command.payload = Value::Null;
        let mut resumed = self
            .regenerate_turn(state, &continue_command, Some("off"))
            .await?;
        resumed.data = json!({
            "continued_solo": true,
            "worker": stopped.data,
        });
        Ok(resumed)
    }

    /// The lease removes the command's entry once the command and any
    /// concurrent same-id retry have finished.
    fn lock_for_command(&self, command_id: &str) -> crate::keyed_lock::KeyedLockLease<'_> {
        crate::keyed_lock::KeyedLockLease::acquire(&self.command_locks, command_id)
    }

    fn lock_for_thread(&self, thread_id: &str) -> crate::keyed_lock::KeyedLockLease<'_> {
        crate::keyed_lock::KeyedLockLease::acquire(&self.thread_locks, thread_id)
    }

    fn confirmation_result(&self, command: &ControlCommandV1) -> ControlCommandResultV1 {
        let mut confirmations = self
            .confirmations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        confirmations.retain(|_, grant| grant.expires_at > Instant::now());
        let grant = confirmations
            .entry(command.command_id.clone())
            .or_insert_with(|| ConfirmationGrant {
                token: Uuid::new_v4().to_string(),
                expires_at: Instant::now() + CONFIRMATION_TTL,
            });
        ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::NeedsConfirmation,
            thread_id: command.thread_id.clone(),
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: Some(grant.token.clone()),
            message: Some("Confirm this destructive action before it expires.".into()),
            data: json!({ "expires_in_seconds": CONFIRMATION_TTL.as_secs() }),
        }
    }

    fn consume_confirmation(&self, command: &ControlCommandV1) -> bool {
        let Some(provided) = command.confirmation_token.as_deref() else {
            return false;
        };
        let mut confirmations = self
            .confirmations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        confirmations.retain(|_, grant| grant.expires_at > Instant::now());
        confirmations
            .remove(&command.command_id)
            .is_some_and(|grant| grant.token == provided)
    }
}

fn validate_command_id(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 160 {
        return Err(Error::InvalidRequest(
            "command_id must contain 1 to 160 characters".into(),
        ));
    }
    Ok(())
}

fn command_for_receipt(command: &ControlCommandV1) -> ControlCommandV1 {
    let mut sanitized = command.clone();
    if sanitized.kind == ControlCommandKindV1::ApprovalResolve {
        if let Some(payload) = sanitized.payload.as_object_mut() {
            payload.remove("response");
        }
    }
    sanitized.confirmation_token = None;
    sanitized
}

pub(super) fn required_thread_id(command: &ControlCommandV1) -> Result<&str> {
    command
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::InvalidRequest(format!("{} requires thread_id", command.kind.as_str()))
        })
}

pub(super) fn required_payload_string(payload: &Value, key: &str) -> Result<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::InvalidRequest(format!("payload.{key} must be a non-empty string")))
}
