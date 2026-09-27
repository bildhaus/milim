//! Queued turns and inbox inputs.

use std::sync::Arc;

use milim_control_contract::{ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1};
use milim_core::{Error, Result};
use milim_storage::ControlInboxRecord;
use serde_json::{json, Value};
use uuid::Uuid;

use super::commands::{required_payload_string, required_thread_id};
use super::{now_ms, AcceptedTurnV1, RunManager};
use crate::AppState;

impl RunManager {
    pub(super) fn inject_context(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let text = required_payload_string(&command.payload, "text")?;
        let thread = self
            .store
            .control_thread(&thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
        let inbox_id = Uuid::new_v4().to_string();
        self.store.control_put_inbox(&ControlInboxRecord {
            id: inbox_id.clone(),
            thread_id: thread_id.clone(),
            target_run_id: None,
            command_id: Some(command.command_id.clone()),
            kind: "inject".into(),
            state: "pending".into(),
            payload_json: json!({ "text": text }).to_string(),
            created_at_ms: now_ms(),
            claimed_at_ms: None,
            resolved_at_ms: None,
        })?;
        self.emit(
            "turn.inbox_updated",
            Some(&thread_id),
            Some(&thread.epoch),
            None,
            json!({ "inbox_id": inbox_id, "kind": "inject", "state": "pending" }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id),
            revision: Some(thread.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "inbox_id": inbox_id }),
        })
    }

    pub(super) fn delete_inbox_input(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let inbox_id = required_payload_string(&command.payload, "inbox_id")?;
        let belongs_to_thread = self
            .store
            .control_pending_inbox(Some(&thread_id))?
            .iter()
            .any(|item| item.id == inbox_id);
        if !belongs_to_thread || !self.store.control_cancel_inbox(&inbox_id)? {
            return Err(Error::InvalidRequest(
                "inbox input was already claimed, removed, or belongs to another thread".into(),
            ));
        }
        self.emit(
            "turn.inbox_updated",
            Some(&thread_id),
            None,
            None,
            json!({ "inbox_id": inbox_id, "state": "cancelled" }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id),
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "inbox_id": inbox_id }),
        })
    }

    pub(super) async fn resume_queued_turn(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let queue_id = required_payload_string(&command.payload, "queue_id")?;
        let interrupt_active = command
            .payload
            .get("interrupt_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        {
            let active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(run) = active.get(&thread_id) {
                if !interrupt_active {
                    return Err(Error::InvalidRequest(
                        "stop the active turn before resuming a queued turn".into(),
                    ));
                }
                if !self
                    .store
                    .control_queued_turns(Some(&thread_id))?
                    .iter()
                    .any(|turn| turn.id == queue_id)
                {
                    return Err(Error::NotFound(format!("queued turn {queue_id}")));
                }
                let mut interrupts = self
                    .queue_interrupts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(pending) = interrupts.get(&thread_id) {
                    if pending != &queue_id {
                        return Err(Error::InvalidRequest(
                            "another queued turn is already interrupting this thread".into(),
                        ));
                    }
                } else {
                    interrupts.insert(thread_id.clone(), queue_id.clone());
                }
                let _ = run.stop.send(true);
                return Ok(ControlCommandResultV1 {
                    command_id: command.command_id.clone(),
                    status: ControlCommandStatusV1::Accepted,
                    thread_id: Some(thread_id),
                    revision: None,
                    run_id: Some(run.run_id.clone()),
                    queue_id: Some(queue_id),
                    confirmation_token: None,
                    message: None,
                    data: json!({ "interrupting": true }),
                });
            }
        }
        let run_id = self.start_queued_turn(state, thread_id.clone(), &queue_id, true)?;
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id),
            revision: None,
            run_id: Some(run_id),
            queue_id: Some(queue_id),
            confirmation_token: None,
            message: None,
            data: json!({ "interrupting": false }),
        })
    }

    pub(super) fn delete_queued_turn(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let queue_id = required_payload_string(&command.payload, "queue_id")?;
        let belongs_to_thread = self
            .store
            .control_queued_turns(Some(&thread_id))?
            .iter()
            .any(|turn| turn.id == queue_id);
        if !belongs_to_thread || !self.store.control_cancel_inbox(&queue_id)? {
            return Err(Error::NotFound(format!("queued turn {queue_id}")));
        }
        self.emit(
            "turn.queue_deleted",
            Some(&thread_id),
            self.store
                .control_thread(&thread_id)?
                .as_ref()
                .map(|thread| thread.epoch.as_str()),
            None,
            json!({ "queue_id": queue_id }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id),
            revision: None,
            run_id: None,
            queue_id: Some(queue_id),
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    pub(super) fn move_queued_turn(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        let queue_id = required_payload_string(&command.payload, "queue_id")?;
        let target_id = required_payload_string(&command.payload, "target_id")?;
        let position = required_payload_string(&command.payload, "position")?;
        let after = match position.as_str() {
            "before" => false,
            "after" => true,
            _ => {
                return Err(Error::InvalidRequest(
                    "payload.position must be before or after".into(),
                ))
            }
        };
        if !self
            .store
            .control_move_queued_turn(&thread_id, &queue_id, &target_id, after)?
        {
            return Err(Error::NotFound(format!(
                "queued turn {queue_id} or target {target_id}"
            )));
        }
        self.emit(
            "turn.queue_moved",
            Some(&thread_id),
            self.store
                .control_thread(&thread_id)?
                .as_ref()
                .map(|thread| thread.epoch.as_str()),
            None,
            json!({ "queue_id": queue_id, "target_id": target_id, "position": position }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id),
            revision: None,
            run_id: None,
            queue_id: Some(queue_id),
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    pub(super) fn start_queued_turn(
        self: &Arc<Self>,
        state: AppState,
        thread_id: String,
        queue_id: &str,
        emit_resumed: bool,
    ) -> Result<String> {
        let _admission = self.mutation_guard()?;
        let queued = self
            .store
            .control_queued_turns(Some(&thread_id))?
            .into_iter()
            .find(|turn| turn.id == queue_id)
            .ok_or_else(|| Error::NotFound(format!("queued turn {queue_id}")))?;
        let mut accepted = serde_json::from_str::<AcceptedTurnV1>(&queued.request_json)
            .map_err(|error| Error::Other(format!("stored queued turn is invalid: {error}")))?;
        if !self.store.control_remove_queued_turn(queue_id)? {
            return Err(Error::NotFound(format!("queued turn {queue_id}")));
        }
        let mailbox_exchange_id = accepted
            .mailbox_origin
            .as_ref()
            .map(|origin| origin.exchange_id.clone());
        let started = self
            .refresh_queued_turn_config(&state, &thread_id, &mut accepted)
            .and_then(|()| self.start_turn(state, thread_id.clone(), accepted));
        let run_id = match started {
            Ok(run_id) => run_id,
            Err(error) => {
                match mailbox_exchange_id {
                    // The sender is waiting on this exchange; fail it rather
                    // than leave it queued with no turn behind it.
                    Some(exchange_id) => {
                        let _ = self.fail_mailbox_exchange_by_id(
                            &exchange_id,
                            "The linked thread could not start its queued turn.",
                        );
                    }
                    None => {
                        let _ = self.store.control_enqueue_turn(&queued);
                    }
                }
                return Err(error);
            }
        };
        if emit_resumed {
            self.emit(
                "turn.queue_resumed",
                Some(&thread_id),
                self.store
                    .control_thread(&thread_id)?
                    .as_ref()
                    .map(|thread| thread.epoch.as_str()),
                None,
                json!({ "queue_id": queue_id, "run_id": run_id }),
            );
        }
        Ok(run_id)
    }

    /// A queued turn runs with the thread's settings as they are when it
    /// starts, not when it was queued, so a model, approval, or privacy
    /// change made while it waited applies to it. Its attachments are kept.
    fn refresh_queued_turn_config(
        &self,
        state: &AppState,
        thread_id: &str,
        accepted: &mut AcceptedTurnV1,
    ) -> Result<()> {
        let thread = self
            .store
            .control_thread(thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {thread_id}")))?;
        let attachments = std::mem::take(&mut accepted.config.attachments);
        accepted.config = self.resolve_turn_config(state, &thread, attachments, "sending")?;
        Ok(())
    }

    pub(super) fn drain_queue(self: &Arc<Self>, state: AppState, thread_id: String) {
        let Some(next) = self
            .store
            .control_queued_turns(Some(&thread_id))
            .ok()
            .and_then(|mut turns| (!turns.is_empty()).then(|| turns.remove(0)))
        else {
            return;
        };
        let _ = self.start_queued_turn(state, thread_id, &next.id, false);
    }
}
