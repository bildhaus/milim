//! Turn lifecycle: acceptance, steering, regeneration, the server-owned run
//! task, completion, and stop.

use std::sync::Arc;

use milim_control_contract::{ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1};
use milim_core::{Error, Result};
use milim_storage::{ControlInboxRecord, ControlQueuedTurnRecord, ControlRunRecord};
use serde_json::{json, Value};
use tokio::sync::watch;
use uuid::Uuid;

use super::attachments::validate_control_attachments;
use super::commands::{required_payload_string, required_thread_id};
use super::journal::RunJournal;
use super::linked_threads::mailbox_context_from_record;
use super::preview_runtime::{preview_runtime_from_payload, sanitize_managed_preview_runtime};
use super::run_config::{resolve_frozen_config, thread_agent_id};
use super::views::{run_snapshot, thread_summary};
use super::{now_ms, AcceptedTurnV1, ActiveRun, RunManager, RunOutcome, TurnSendPayloadV1};
use crate::AppState;

/// Prefix of the id the renderer assigns to an assistant turn it projects
/// purely from stream events (a run that persisted no assistant message).
const STREAM_PLACEHOLDER_PREFIX: &str = "control-stream-";

impl RunManager {
    pub(super) async fn accept_turn(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let payload: TurnSendPayloadV1 =
            serde_json::from_value(command.payload.clone()).map_err(|error| {
                Error::InvalidRequest(format!("invalid turn.send payload: {error}"))
            })?;
        if payload.text.trim().is_empty() && payload.attachments.is_empty() {
            return Err(Error::InvalidRequest(
                "turn.send requires text or at least one attachment".into(),
            ));
        }
        if payload
            .client_message_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty() || id.chars().count() > 200)
        {
            return Err(Error::InvalidRequest(
                "client_message_id must contain 1 to 200 characters".into(),
            ));
        }
        validate_control_attachments(&payload.attachments)?;
        let thread = self
            .store
            .control_thread(&thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
        if let Some(expected) = command.expected_revision {
            if expected != thread.revision {
                return Err(Error::InvalidRequest(format!(
                    "thread revision conflict: expected {expected}, current {}",
                    thread.revision
                )));
            }
        }
        let mut config = resolve_frozen_config(&state, &self.store, &thread, payload.attachments)?;
        config.linked_thread_grants = self.freeze_linked_thread_grants(&thread_id)?;
        let bound_agent_id = thread_agent_id(&thread);
        if config.agent.is_none() {
            if let Some(agent_id) = bound_agent_id {
                return Err(Error::InvalidRequest(format!(
                    "thread is bound to missing Agent {agent_id}; replace or clear the binding before sending"
                )));
            }
        }
        let accepted = AcceptedTurnV1 {
            text: payload.text,
            client_message_id: payload.client_message_id,
            display_text: payload.display_text,
            config,
            append_user: true,
            mailbox_origin: None,
            mailbox_context: Vec::new(),
            preview_runtime: sanitize_managed_preview_runtime(payload.preview_runtime),
        };
        let busy = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&thread_id);
        if busy {
            let queue_id = Uuid::new_v4().to_string();
            self.store.control_enqueue_turn(&ControlQueuedTurnRecord {
                id: queue_id.clone(),
                thread_id: thread_id.clone(),
                command_id: command.command_id.clone(),
                request_json: serde_json::to_string(&accepted)
                    .map_err(|error| Error::Other(format!("serialize accepted turn: {error}")))?,
                accepted_at_ms: now_ms(),
            })?;
            self.emit(
                "turn.queued",
                Some(&thread_id),
                Some(&thread.epoch),
                None,
                json!({ "queue_id": queue_id, "command_id": command.command_id }),
            );
            return Ok(ControlCommandResultV1 {
                command_id: command.command_id.clone(),
                status: ControlCommandStatusV1::Queued,
                thread_id: Some(thread_id),
                revision: Some(thread.revision),
                run_id: None,
                queue_id: Some(queue_id),
                confirmation_token: None,
                message: None,
                data: Value::Null,
            });
        }
        let run_id = self.start_turn(state, thread_id.clone(), accepted)?;
        let run = self
            .store
            .control_run(&run_id)?
            .map(run_snapshot)
            .transpose()?;
        let run_capabilities = run.as_ref().map(|run| run.capabilities.clone());
        let native_session_id = run
            .as_ref()
            .and_then(|run| run.config.native_session_id.clone());
        let revision = self
            .store
            .control_thread(&thread_id)?
            .map(|value| value.revision);
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id),
            revision,
            run_id: Some(run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({
                "capabilities": run_capabilities,
                "native_session_id": native_session_id,
            }),
        })
    }

    pub(super) fn steer_turn(
        &self,
        state: &AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let requested_run_id = required_payload_string(&command.payload, "run_id")?;
        let payload: TurnSendPayloadV1 =
            serde_json::from_value(command.payload.clone()).map_err(|error| {
                Error::InvalidRequest(format!("invalid turn.steer payload: {error}"))
            })?;
        if payload.text.trim().is_empty() && payload.attachments.is_empty() {
            return Err(Error::InvalidRequest(
                "turn.steer requires text or at least one attachment".into(),
            ));
        }
        validate_control_attachments(&payload.attachments)?;
        {
            let active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(run) = active.get(&thread_id) else {
                return Err(Error::InvalidRequest("thread has no active turn".into()));
            };
            if run.run_id != requested_run_id {
                return Err(Error::InvalidRequest(
                    "turn.steer run_id does not match the active run".into(),
                ));
            }
            if !run.steering {
                return Err(Error::InvalidRequest(
                    "active runtime does not support steering".into(),
                ));
            }
        }
        let thread = self
            .store
            .control_thread(&thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
        let accepted = AcceptedTurnV1 {
            text: payload.text,
            client_message_id: payload.client_message_id,
            display_text: payload.display_text,
            config: resolve_frozen_config(state, &self.store, &thread, payload.attachments)?,
            append_user: true,
            mailbox_origin: None,
            mailbox_context: Vec::new(),
            preview_runtime: sanitize_managed_preview_runtime(payload.preview_runtime),
        };
        let inbox_id = Uuid::new_v4().to_string();
        self.store.control_put_inbox(&ControlInboxRecord {
            id: inbox_id.clone(),
            thread_id: thread_id.clone(),
            target_run_id: Some(requested_run_id.clone()),
            command_id: Some(command.command_id.clone()),
            kind: "steer".into(),
            state: "pending".into(),
            payload_json: serde_json::to_string(&accepted)
                .map_err(|error| Error::Other(format!("serialize steering input: {error}")))?,
            created_at_ms: now_ms(),
            claimed_at_ms: None,
            resolved_at_ms: None,
        })?;
        self.emit(
            "turn.inbox_updated",
            Some(&thread_id),
            Some(&thread.epoch),
            None,
            json!({ "inbox_id": inbox_id, "kind": "steer", "state": "pending" }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id),
            revision: Some(thread.revision),
            run_id: Some(requested_run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "inbox_id": inbox_id }),
        })
    }

    pub(super) async fn regenerate_turn(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
        delegation_policy: Option<&str>,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        if self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&thread_id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before regenerating".into(),
            ));
        }
        let thread = self
            .store
            .control_thread(&thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
        if let Some(expected) = command.expected_revision {
            if expected != thread.revision {
                return Err(Error::InvalidRequest(format!(
                    "thread revision conflict: expected {expected}, current {}",
                    thread.revision
                )));
            }
        }
        let messages = self
            .store
            .control_messages(&thread_id)?
            .into_iter()
            .filter_map(|raw| serde_json::from_str::<Value>(&raw).ok())
            .collect::<Vec<_>>();
        let user = messages
            .iter()
            .rev()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
            .ok_or_else(|| Error::InvalidRequest("thread has no user turn to regenerate".into()))?;
        let text = user
            .get("promptContent")
            .or_else(|| user.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let display_text = user
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string);
        let attachments = user
            .get("attachments")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| Error::Other(format!("stored attachments are invalid: {error}")))?
            .unwrap_or_default();
        if let Some(assistant_id) = messages
            .iter()
            .rev()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .and_then(|message| message.get("id"))
            .and_then(Value::as_str)
        {
            if self
                .store
                .control_delete_message(&thread_id, assistant_id)?
            {
                self.persist_and_emit(
                    &thread_id,
                    None,
                    "message_deleted",
                    json!({ "message_id": assistant_id, "reason": "regenerate" }),
                )?;
            }
        }
        // A failed or cancelled run persists no assistant message; the renderer
        // shows its error from stream events under a placeholder id. Retire that
        // placeholder too so the regenerated reply replaces it after reload.
        if let Some(last_run_id) = self.last_run_id_in_timeline(&thread_id)? {
            let has_assistant = messages.iter().any(|message| {
                message.get("role").and_then(Value::as_str) == Some("assistant")
                    && message.get("runId").and_then(Value::as_str) == Some(last_run_id.as_str())
            });
            if !has_assistant {
                self.persist_and_emit(
                    &thread_id,
                    None,
                    "message_deleted",
                    json!({
                        "message_id": stream_placeholder_message_id(&last_run_id),
                        "reason": "regenerate",
                    }),
                )?;
            }
        }
        let mut config = resolve_frozen_config(&state, &self.store, &thread, attachments)?;
        config.linked_thread_grants = self.freeze_linked_thread_grants(&thread_id)?;
        if let Some(policy) = delegation_policy {
            config.delegation_policy = policy.to_string();
        }
        if config.agent.is_none() {
            if let Some(agent_id) = thread_agent_id(&thread) {
                return Err(Error::InvalidRequest(format!(
                    "thread is bound to missing Agent {agent_id}; replace or clear the binding before regenerating"
                )));
            }
        }
        let accepted = AcceptedTurnV1 {
            text,
            client_message_id: None,
            display_text,
            config,
            append_user: false,
            mailbox_origin: None,
            mailbox_context: Vec::new(),
            preview_runtime: preview_runtime_from_payload(&command.payload)?,
        };
        let run_id = self.start_turn(state, thread_id.clone(), accepted)?;
        let run = self
            .store
            .control_run(&run_id)?
            .map(run_snapshot)
            .transpose()?;
        let run_capabilities = run.as_ref().map(|run| run.capabilities.clone());
        let native_session_id = run
            .as_ref()
            .and_then(|run| run.config.native_session_id.clone());
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id.clone()),
            revision: self
                .store
                .control_thread(&thread_id)?
                .map(|thread| thread.revision),
            run_id: Some(run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({
                "regenerated": true,
                "capabilities": run_capabilities,
                "native_session_id": native_session_id,
            }),
        })
    }

    /// The run id of the most recent run recorded in a thread's timeline.
    fn last_run_id_in_timeline(&self, thread_id: &str) -> Result<Option<String>> {
        let Some(page) = self
            .store
            .control_timeline_page(thread_id, None, None, true, 100)?
        else {
            return Ok(None);
        };
        Ok(page
            .items
            .iter()
            .rev()
            .find(|item| item.item_type == "run_status")
            .and_then(|item| item.run_id.clone()))
    }

    /// Whether `message_id` names the renderer's stream placeholder for a
    /// finished run that belongs to `thread_id`.
    pub(super) fn is_stream_placeholder_for_thread(
        &self,
        thread_id: &str,
        message_id: &str,
    ) -> Result<bool> {
        let Some(run_id) = message_id.strip_prefix(STREAM_PLACEHOLDER_PREFIX) else {
            return Ok(false);
        };
        Ok(self
            .store
            .control_run(run_id)?
            .is_some_and(|run| run.thread_id == thread_id && run.completed_at_ms.is_some()))
    }

    pub(super) fn start_turn(
        self: &Arc<Self>,
        state: AppState,
        thread_id: String,
        mut accepted: AcceptedTurnV1,
    ) -> Result<String> {
        let _admission = self.mutation_guard()?;
        if let Some(origin) = accepted.mailbox_origin.as_ref() {
            let target = self
                .store
                .control_thread(&thread_id)?
                .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
            if thread_summary(&target, false, 0)?.archived_at_ms.is_some() {
                self.fail_mailbox_exchange_by_id(
                    &origin.exchange_id,
                    "The linked destination was archived before its queued turn could start.",
                )?;
                return Err(Error::InvalidRequest(
                    "linked destination is archived".into(),
                ));
            }
        }
        if self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&thread_id)
        {
            return Err(Error::InvalidRequest(
                "thread already has an active turn".into(),
            ));
        }
        self.refresh_native_session_for_start(&thread_id, &mut accepted.config)?;
        let run_id = Uuid::new_v4().to_string();
        let (stop, stop_rx) = watch::channel(false);
        {
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active.contains_key(&thread_id) {
                return Err(Error::InvalidRequest(
                    "thread already has an active turn".into(),
                ));
            }
            active.insert(
                thread_id.clone(),
                ActiveRun {
                    run_id: run_id.clone(),
                    steering: accepted.config.agent.is_some()
                        || accepted.config.adapter == "provider",
                    stop,
                },
            );
        }
        let claimed_mail = match self
            .store
            .control_claim_mailbox_replies(&thread_id, &run_id, 20)
        {
            Ok(items) => items,
            Err(error) => {
                self.active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&thread_id);
                return Err(error);
            }
        };
        accepted.config.claimed_mailbox_ids =
            claimed_mail.iter().map(|item| item.id.clone()).collect();
        accepted.mailbox_context = claimed_mail
            .iter()
            .filter_map(mailbox_context_from_record)
            .collect();
        let now = now_ms();
        let mut run_record = ControlRunRecord {
            id: run_id.clone(),
            thread_id: thread_id.clone(),
            status: "accepted".into(),
            adapter: accepted.config.adapter.clone(),
            request_json: serde_json::to_string(&accepted)
                .map_err(|error| Error::Other(format!("serialize run snapshot: {error}")))?,
            agent_snapshot_json: accepted
                .config
                .agent
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|error| Error::Other(format!("serialize Agent snapshot: {error}")))?,
            native_session_json: accepted
                .config
                .native_session_id
                .as_ref()
                .map(|value| json!({ "id": value }).to_string()),
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            error_json: None,
        };
        self.store.control_put_run(&run_record)?;
        if let Some(origin) = accepted.mailbox_origin.as_ref() {
            if let Some(mut exchange) = self.store.control_mailbox(&origin.exchange_id)? {
                exchange.status = "running".into();
                exchange.target_run_id = Some(run_id.clone());
                exchange.updated_at_ms = now_ms();
                self.store.control_put_mailbox(&exchange)?;
                let _ = self.persist_and_emit(
                    &exchange.origin_thread_id,
                    exchange.origin_run_id.as_deref(),
                    "mailbox_running",
                    json!({
                        "exchange_id": exchange.id,
                        "target_thread_id": exchange.target_thread_id,
                        "target_run_id": run_id,
                        "status": "running",
                    }),
                );
                self.emit(
                    "mailbox.running",
                    Some(&thread_id),
                    None,
                    None,
                    json!({ "exchange_id": origin.exchange_id, "run_id": run_id }),
                );
            }
        }
        for exchange in &claimed_mail {
            let _ = self.persist_and_emit(
                &thread_id,
                Some(&run_id),
                "mailbox_reply_consumed",
                json!({
                    "exchange_id": exchange.id,
                    "target_thread_id": exchange.target_thread_id,
                    "status": exchange.status,
                }),
            );
        }
        let journal = RunJournal {
            store: self.store.clone(),
            privacy: state.privacy.clone(),
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
        };
        if let Err(error) = journal.commit_composition(&accepted) {
            self.active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&thread_id);
            run_record.status = "failed".into();
            run_record.updated_at_ms = now_ms();
            run_record.completed_at_ms = Some(run_record.updated_at_ms);
            run_record.error_json =
                Some(milim_core::provider_error::run_error_value(&error).to_string());
            let _ = self.store.control_put_run(&run_record);
            return Err(error);
        }
        if accepted.append_user {
            let user_message_id = accepted
                .client_message_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let user_message = json!({
                "id": user_message_id,
                "role": "user",
                "content": accepted.display_text.as_deref().unwrap_or(&accepted.text),
                "promptContent": accepted.text,
                "attachments": accepted.config.attachments,
                "runId": run_id,
                "mailboxOrigin": accepted.mailbox_origin.clone(),
            });
            if accepted.client_message_id.is_some() {
                self.persist_adopted_message_and_event(
                    &thread_id,
                    &run_id,
                    user_message,
                    "accepted_input_projected",
                    json!({"source": "turn.send", "adopted_optimistic_message": true}),
                )?;
            } else {
                self.persist_message_and_event(
                    &thread_id,
                    &run_id,
                    user_message,
                    None,
                    "accepted_input_projected",
                    json!({"source": "turn.send"}),
                )?;
            }
        }
        let manager = self.clone();
        let spawned_run_id = run_id.clone();
        tokio::spawn(async move {
            manager
                .run_turn(state, thread_id, spawned_run_id, accepted, stop_rx)
                .await;
        });
        Ok(run_id)
    }

    async fn run_turn(
        self: Arc<Self>,
        state: AppState,
        thread_id: String,
        run_id: String,
        accepted: AcceptedTurnV1,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut run = match self.store.control_run(&run_id).ok().flatten() {
            Some(run) => run,
            None => return,
        };
        run.status = "running".into();
        run.updated_at_ms = now_ms();
        let _ = self.store.control_put_run(&run);
        self.emit(
            "run.updated",
            Some(&thread_id),
            self.store
                .control_thread(&thread_id)
                .ok()
                .flatten()
                .as_ref()
                .map(|thread| thread.epoch.as_str()),
            None,
            json!({ "run_id": run_id, "status": "running" }),
        );
        self.checkpoint_turn_workspace(&thread_id, &run_id, &accepted.config)
            .await;

        let outcome = if accepted.config.agent.is_some() || accepted.config.adapter == "provider" {
            self.run_agent(&state, &thread_id, &run_id, &accepted, &mut stop)
                .await
        } else if accepted.config.adapter == "mock" {
            self.run_mock(&thread_id, &run_id, &accepted, &mut stop)
                .await
        } else if matches!(
            accepted.config.adapter.as_str(),
            "codex" | "claude" | "opencode" | "pi"
        ) {
            self.run_harness(&state, &thread_id, &run_id, &accepted, &mut stop)
                .await
        } else {
            self.run_provider(&state, &thread_id, &run_id, &accepted, &mut stop)
                .await
        };

        if let Err(error) = &outcome {
            let journal = RunJournal {
                store: self.store.clone(),
                privacy: state.privacy.clone(),
                privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
                thread_id: thread_id.clone(),
                run_id: run_id.clone(),
            };
            let _ = journal.commit_failure(0, error);
        }

        self.turn_checkpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&run_id);
        let limited = matches!(&outcome, Ok(RunOutcome::Limited));
        let (status, error) = match outcome {
            Ok(RunOutcome::Completed | RunOutcome::Limited) => ("completed", None),
            Ok(RunOutcome::Cancelled) => ("cancelled", None),
            Err(error) => (
                "failed",
                Some(milim_core::provider_error::run_error_value(&error)),
            ),
        };
        run.status = status.into();
        run.updated_at_ms = now_ms();
        run.completed_at_ms = Some(run.updated_at_ms);
        run.error_json = error.as_ref().map(Value::to_string);
        let _ = self.store.control_put_run(&run);
        let _ = self.persist_and_emit(
            &thread_id,
            Some(&run_id),
            "run_status",
            json!({ "run_id": run_id, "status": status, "error": error }),
        );
        if status != "completed" {
            let failure = error
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or(if status == "cancelled" {
                    "The linked thread run was cancelled."
                } else {
                    "The linked thread run failed."
                });
            let _ = self.complete_mailbox_exchange(&run_id, None, Some(failure));
        }
        let _ = self.store.control_retarget_pending_steers(&run_id);
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&thread_id);
        let interrupt_queue_id = self
            .queue_interrupts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&thread_id);
        if let Some(queue_id) = interrupt_queue_id {
            let _ = self.start_queued_turn(state, thread_id, &queue_id, true);
        } else if status != "cancelled" && !limited {
            self.drain_queue(state, thread_id);
        }
    }

    pub(super) fn complete_assistant_message(
        &self,
        thread_id: &str,
        run_id: &str,
        content: String,
        reasoning: String,
        metrics: Option<Value>,
    ) -> Result<String> {
        let mailbox_content = content.clone();
        let message_id = Uuid::new_v4().to_string();
        let mut message = json!({
            "id": message_id,
            "role": "assistant",
            "content": content,
            "reasoning": reasoning,
            "runId": run_id,
            "ledgerVersion": 1,
            "metrics": metrics,
        });
        if let Some(checkpoint) = self
            .turn_checkpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(run_id)
        {
            message["workspaceCheckpoint"] = checkpoint.clone();
        }
        self.persist_message_and_event(
            thread_id,
            run_id,
            message,
            None,
            "assistant_message_projected",
            json!({"ledger_version": 1}),
        )?;
        self.complete_mailbox_exchange(run_id, Some(&mailbox_content), None)?;
        Ok(message_id)
    }

    pub(super) fn stop_turn(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?;
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run = active.get(thread_id);
        if let Some(run) = run {
            let _ = run.stop.send(true);
        }
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id.to_string()),
            revision: self
                .store
                .control_thread(thread_id)?
                .map(|thread| thread.revision),
            run_id: run.map(|run| run.run_id.clone()),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({
                "queued_turns_preserved": true,
                "already_stopped": run.is_none(),
            }),
        })
    }
}

pub(super) fn stream_placeholder_message_id(run_id: &str) -> String {
    format!("{STREAM_PLACEHOLDER_PREFIX}{run_id}")
}
