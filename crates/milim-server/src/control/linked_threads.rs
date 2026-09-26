//! Linked threads: frozen read/send grants, cross-thread reads, and the durable
//! mailbox that carries sends, waits, and replies.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use milim_control_contract::{FrozenLinkedThreadGrantV1, FrozenRunConfigV1, MailboxOriginV1};
use milim_core::{Error, Result};
use milim_storage::{ControlInboxRecord, ControlMailboxRecord, ControlQueuedTurnRecord};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::native_sessions::{runtime_adapter, runtime_model};
use super::run_config::resolve_frozen_config;
use super::views::{project_label, thread_summary, thread_title};
use super::{now_ms, parse_value, AcceptedTurnV1, RunManager, MAX_LINKED_THREAD_WAIT_MS};
use crate::AppState;

impl RunManager {
    pub(super) fn freeze_linked_thread_grants(
        &self,
        owner_thread_id: &str,
    ) -> Result<Vec<FrozenLinkedThreadGrantV1>> {
        Ok(self
            .store
            .control_thread_links(Some(owner_thread_id))?
            .into_iter()
            .map(|link| -> Result<Option<FrozenLinkedThreadGrantV1>> {
                let Some(target) = self.store.control_thread(&link.target_thread_id)? else {
                    return Ok(None);
                };
                let (epoch, max_timeline_seq) = self
                    .store
                    .control_timeline_max_seq(&target.id)?
                    .unwrap_or_else(|| (target.epoch.clone(), 0));
                let summary = thread_summary(&target, false, 0)?;
                Ok(Some(FrozenLinkedThreadGrantV1 {
                    target_thread_id: target.id,
                    title: summary.title,
                    workspace: summary.workspace.clone(),
                    project: summary.workspace.as_deref().and_then(project_label),
                    model: summary
                        .model
                        .as_deref()
                        .map(runtime_model)
                        .map(str::to_string),
                    runtime: runtime_adapter(summary.model.as_deref().unwrap_or_default())
                        .to_string(),
                    revision: target.revision,
                    epoch,
                    max_timeline_seq,
                }))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect())
    }

    pub(crate) fn linked_thread_list(
        &self,
        origin_thread_id: &str,
        grants: &[FrozenLinkedThreadGrantV1],
    ) -> Result<Value> {
        let mailbox = self
            .store
            .control_mailbox_for_origin(origin_thread_id)?
            .into_iter()
            .map(|exchange| {
                json!({
                    "exchange_id": exchange.id,
                    "target_thread_id": exchange.target_thread_id,
                    "status": exchange.status,
                    "created_at_ms": exchange.created_at_ms,
                    "updated_at_ms": exchange.updated_at_ms,
                    "consumed": exchange.consumed_at_ms.is_some(),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "linked_threads": grants, "mailbox": mailbox }))
    }

    pub(super) fn enrich_linked_thread_send_approval(
        &self,
        accepted: &AcceptedTurnV1,
        mut request: Value,
    ) -> Result<Value> {
        let Some(object) = request.as_object_mut() else {
            return Ok(request);
        };
        if object.get("name").and_then(Value::as_str) != Some("linked_thread_send") {
            return Ok(request);
        }
        let arguments = match object.get("arguments") {
            Some(Value::String(arguments)) => serde_json::from_str::<Value>(arguments).ok(),
            Some(arguments @ Value::Object(_)) => Some(arguments.clone()),
            _ => None,
        };
        let Some(arguments) = arguments else {
            return Ok(request);
        };
        let Some(target_thread_id) = arguments.get("target_thread_id").and_then(Value::as_str)
        else {
            return Ok(request);
        };
        let Some(grant) = accepted
            .config
            .linked_thread_grants
            .iter()
            .find(|grant| grant.target_thread_id == target_thread_id)
        else {
            return Ok(request);
        };
        let current = self
            .store
            .control_thread(target_thread_id)?
            .map(|thread| thread_summary(&thread, false, 0))
            .transpose()?;
        let queued_turns = self
            .store
            .control_queued_turns(Some(target_thread_id))?
            .len();
        let active_delivery = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(target_thread_id)
            .map(|run| if run.steering { "steer" } else { "queue" });
        let delivery = active_delivery.unwrap_or(if queued_turns > 0 { "queue" } else { "start" });
        let current_model = current
            .as_ref()
            .and_then(|thread| thread.model.as_deref())
            .map(runtime_model)
            .map(str::to_string)
            .or_else(|| grant.model.clone());
        let current_runtime = current
            .as_ref()
            .and_then(|thread| thread.model.as_deref())
            .map(runtime_adapter)
            .map(str::to_string)
            .unwrap_or_else(|| grant.runtime.clone());
        object.insert(
            "linked_thread_send".into(),
            json!({
                "destination_thread_id": target_thread_id,
                "destination_title": current.as_ref().map(|thread| thread.title.as_str()).unwrap_or(&grant.title),
                "destination_project": current.as_ref().and_then(|thread| thread.workspace.as_deref()).and_then(project_label).or_else(|| grant.project.clone()),
                "destination_workspace": current.as_ref().and_then(|thread| thread.workspace.clone()).or_else(|| grant.workspace.clone()),
                "destination_model": current_model,
                "destination_runtime": current_runtime,
                "message": arguments.get("message").cloned().unwrap_or(Value::Null),
                "delivery": delivery,
                "model_work_notice": if delivery == "steer" {
                    "Approval steers the destination's active run and may use its provider or account subscription."
                } else if delivery == "queue" {
                    "Approval queues model work in the destination and may use its provider or account subscription."
                } else {
                    "Approval starts model work in the destination and may use its provider or account subscription."
                },
            }),
        );
        Ok(request)
    }

    pub(crate) fn linked_thread_read(
        &self,
        grants: &[FrozenLinkedThreadGrantV1],
        target_thread_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Value> {
        let grant = grants
            .iter()
            .find(|grant| grant.target_thread_id == target_thread_id)
            .ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "thread {target_thread_id} is not granted to this run"
                ))
            })?;
        let limit = limit.clamp(1, 50);
        let records = self.store.control_visible_timeline_messages(
            target_thread_id,
            &grant.epoch,
            grant.max_timeline_seq,
            after_seq,
            500,
        )?;
        let mut messages = Vec::new();
        let mut next_after_seq = after_seq;
        let mut has_more = false;
        for record in records {
            let value = parse_value(&record.data_json)?;
            let Some(role) = value.get("role").and_then(Value::as_str) else {
                continue;
            };
            if !matches!(role, "user" | "assistant") {
                continue;
            }
            if messages.len() >= limit {
                has_more = true;
                break;
            }
            let attachments = value
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|attachment| {
                    json!({
                        "id": attachment.get("id").cloned().unwrap_or(Value::Null),
                        "name": attachment.get("name").cloned().unwrap_or(Value::Null),
                        "mime": attachment.get("mime").cloned().unwrap_or(Value::Null),
                        "size": attachment.get("size").cloned().unwrap_or(Value::Null),
                        "truncated": attachment.get("truncated").cloned().unwrap_or(Value::Bool(false)),
                    })
                })
                .collect::<Vec<_>>();
            let (content, content_truncated) = truncate_linked_content(
                value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                48 * 1024,
            );
            let message = json!({
                "id": value.get("id").cloned().unwrap_or_else(|| Value::String(record.item_id.clone())),
                "seq": record.seq,
                "role": role,
                "content": content,
                "content_truncated": content_truncated,
                "attachments": attachments,
                "mailbox_origin": value.get("mailboxOrigin").cloned().unwrap_or(Value::Null),
            });
            let mut candidate = messages.clone();
            candidate.push(message.clone());
            let envelope = json!({
                "thread": grant,
                "messages": candidate,
                "next_after_seq": record.seq,
                "has_more": true,
            });
            if serde_json::to_vec(&envelope)
                .map_err(|error| Error::Other(format!("serialize linked transcript: {error}")))?
                .len()
                > 64 * 1024
            {
                has_more = true;
                break;
            }
            messages.push(message);
            next_after_seq = Some(record.seq);
        }
        Ok(json!({
            "thread": grant,
            "messages": messages,
            "next_after_seq": next_after_seq,
            "has_more": has_more,
        }))
    }

    pub(crate) async fn linked_thread_send(
        self: &Arc<Self>,
        state: AppState,
        origin_thread_id: &str,
        origin_run_id: Option<&str>,
        grants: &[FrozenLinkedThreadGrantV1],
        target_thread_id: &str,
        message: &str,
    ) -> Result<Value> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::InvalidRequest(
                "linked_thread_send requires a message".into(),
            ));
        }
        let grant = grants
            .iter()
            .find(|grant| grant.target_thread_id == target_thread_id)
            .ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "thread {target_thread_id} is not granted to this run"
                ))
            })?;
        let origin = self
            .store
            .control_thread(origin_thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {origin_thread_id}")))?;
        let target = self
            .store
            .control_thread(target_thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {target_thread_id}")))?;
        let target_summary = thread_summary(&target, false, 0)?;
        if target_summary.archived_at_ms.is_some() {
            return Err(Error::InvalidRequest(format!(
                "linked thread {} is archived",
                target_summary.title
            )));
        }
        let origin_summary = thread_summary(&origin, false, 0)?;
        let exchange_id = Uuid::new_v4().to_string();
        let mailbox_origin = MailboxOriginV1 {
            exchange_id: exchange_id.clone(),
            origin_thread_id: origin_thread_id.to_string(),
            origin_title: origin_summary.title.clone(),
            origin_workspace: origin_summary.workspace.clone(),
            origin_project: origin_summary.workspace.as_deref().and_then(project_label),
        };
        let mut config = resolve_frozen_config(&state, &self.store, &target, Vec::new())?;
        config.linked_thread_grants = self.freeze_linked_thread_grants(target_thread_id)?;
        let accepted = AcceptedTurnV1 {
            text: format!(
                "Linked-thread mailbox message from \"{}\" (thread {}):\n{}",
                origin_summary.title, origin_thread_id, message
            ),
            client_message_id: None,
            display_text: Some(message.to_string()),
            config,
            append_user: true,
            mailbox_origin: Some(mailbox_origin.clone()),
            mailbox_context: Vec::new(),
            preview_runtime: None,
        };
        let active_delivery = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(target_thread_id)
            .map(|run| (run.run_id.clone(), run.steering));
        let busy = active_delivery.is_some();
        let steer_run_id = active_delivery
            .as_ref()
            .and_then(|(run_id, steering)| steering.then(|| run_id.clone()));
        let now = now_ms();
        let mut exchange = ControlMailboxRecord {
            id: exchange_id.clone(),
            origin_thread_id: origin_thread_id.to_string(),
            target_thread_id: target_thread_id.to_string(),
            origin_run_id: origin_run_id.map(str::to_string),
            target_run_id: steer_run_id.clone(),
            status: if busy && steer_run_id.is_none() {
                "queued"
            } else {
                "running"
            }
            .into(),
            request_json: json!({
                "message": message,
                "origin": mailbox_origin,
                "target": grant,
            })
            .to_string(),
            reply_json: None,
            created_at_ms: now,
            updated_at_ms: now,
            consumed_at_ms: None,
            projected_at_ms: None,
        };
        self.store.control_put_mailbox(&exchange)?;
        if let Some(run_id) = steer_run_id {
            if let Err(error) = self.store.control_put_inbox(&ControlInboxRecord {
                id: exchange_id.clone(),
                thread_id: target_thread_id.to_string(),
                target_run_id: Some(run_id.clone()),
                command_id: Some(format!("mailbox-{exchange_id}")),
                kind: "steer".into(),
                state: "pending".into(),
                payload_json: serde_json::to_string(&accepted)
                    .map_err(|error| Error::Other(format!("serialize mailbox steer: {error}")))?,
                created_at_ms: now,
                claimed_at_ms: None,
                resolved_at_ms: None,
            }) {
                let _ = self.store.control_delete_mailbox(&exchange_id);
                return Err(error);
            }
            self.persist_and_emit(
                origin_thread_id,
                origin_run_id,
                "mailbox_steered",
                json!({
                    "exchange_id": exchange_id,
                    "target_thread_id": target_thread_id,
                    "target_title": target_summary.title,
                    "target_run_id": run_id,
                    "status": "running",
                }),
            )?;
            self.emit(
                "mailbox.steered",
                Some(target_thread_id),
                Some(&target.epoch),
                None,
                json!({ "exchange_id": exchange_id, "run_id": run_id }),
            );
            Ok(json!({
                "exchange_id": exchange_id,
                "status": "steering",
                "run_id": run_id,
                "target": grant,
            }))
        } else if busy {
            let queue_id = format!("mailbox-{exchange_id}");
            if let Err(error) = self.store.control_enqueue_turn(&ControlQueuedTurnRecord {
                id: queue_id.clone(),
                thread_id: target_thread_id.to_string(),
                command_id: format!("mailbox-{exchange_id}"),
                request_json: serde_json::to_string(&accepted)
                    .map_err(|error| Error::Other(format!("serialize mailbox turn: {error}")))?,
                accepted_at_ms: now,
            }) {
                let _ = self.store.control_delete_mailbox(&exchange_id);
                return Err(error);
            }
            self.persist_and_emit(
                origin_thread_id,
                origin_run_id,
                "mailbox_queued",
                json!({
                    "exchange_id": exchange_id,
                    "target_thread_id": target_thread_id,
                    "target_title": target_summary.title,
                    "status": "queued",
                }),
            )?;
            self.emit(
                "mailbox.queued",
                Some(target_thread_id),
                Some(&target.epoch),
                None,
                json!({ "exchange_id": exchange_id, "queue_id": queue_id }),
            );
            Ok(json!({
                "exchange_id": exchange_id,
                "status": "queued",
                "queue_id": queue_id,
                "target": grant,
            }))
        } else {
            let run_id = match self.start_turn(state, target_thread_id.to_string(), accepted) {
                Ok(run_id) => run_id,
                Err(error) => {
                    let _ = self.store.control_delete_mailbox(&exchange_id);
                    return Err(error);
                }
            };
            exchange.target_run_id = Some(run_id.clone());
            exchange.updated_at_ms = now_ms();
            self.store.control_put_mailbox(&exchange)?;
            Ok(json!({
                "exchange_id": exchange_id,
                "status": "running",
                "run_id": run_id,
                "target": grant,
            }))
        }
    }

    pub(crate) async fn linked_thread_wait(
        &self,
        origin_thread_id: &str,
        origin_run_id: &str,
        grants: &[FrozenLinkedThreadGrantV1],
        exchange_id: &str,
        timeout_ms: u64,
    ) -> Result<Value> {
        let exchange_id = exchange_id.trim();
        if exchange_id.is_empty() {
            return Err(Error::InvalidRequest(
                "linked_thread_wait requires an exchange id".into(),
            ));
        }
        if !(100..=MAX_LINKED_THREAD_WAIT_MS).contains(&timeout_ms) {
            return Err(Error::InvalidRequest(format!(
                "linked_thread_wait timeout_ms must be between 100 and {MAX_LINKED_THREAD_WAIT_MS}"
            )));
        }

        let mut events = self.subscribe();
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let exchange = self
                .store
                .control_mailbox(exchange_id)?
                .ok_or_else(|| Error::NotFound(format!("mailbox exchange {exchange_id}")))?;
            let owned_by_run = exchange.origin_thread_id == origin_thread_id
                && exchange.origin_run_id.as_deref() == Some(origin_run_id)
                && grants
                    .iter()
                    .any(|grant| grant.target_thread_id == exchange.target_thread_id);
            if !owned_by_run {
                return Err(Error::InvalidRequest(
                    "mailbox exchange is not available to this linked-thread run".into(),
                ));
            }

            if matches!(exchange.status.as_str(), "replied" | "failed") {
                if exchange.consumed_at_ms.is_none()
                    && self.store.control_mark_mailbox_consumed(
                        exchange_id,
                        origin_thread_id,
                        origin_run_id,
                    )?
                {
                    let _ = self.persist_and_emit(
                        origin_thread_id,
                        Some(origin_run_id),
                        "mailbox_reply_consumed",
                        json!({
                            "exchange_id": exchange.id,
                            "target_thread_id": exchange.target_thread_id,
                            "status": exchange.status,
                            "source": "linked_thread_wait",
                        }),
                    );
                }
                return mailbox_wait_result(&exchange, false);
            }
            if exchange.status == "discarded" {
                return mailbox_wait_result(&exchange, false);
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return mailbox_wait_result(&exchange, true);
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Ok(event)) => {
                    if event
                        .data
                        .get("exchange_id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| id == exchange_id)
                    {
                        continue;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err(Error::Other("control event stream closed".into()))
                }
                Err(_) => continue,
            }
        }
    }

    pub(super) fn complete_mailbox_exchange(
        &self,
        target_run_id: &str,
        content: Option<&str>,
        failure: Option<&str>,
    ) -> Result<()> {
        let exchanges = self.store.control_mailboxes_for_target_run(target_run_id)?;
        let Some(first) = exchanges.first() else {
            return Ok(());
        };
        let pending_steer_ids = self
            .store
            .control_pending_inbox(Some(&first.target_thread_id))?
            .into_iter()
            .filter(|item| {
                item.kind == "steer" && item.target_run_id.as_deref() == Some(target_run_id)
            })
            .map(|item| item.id)
            .collect::<HashSet<_>>();
        for mut exchange in exchanges {
            if pending_steer_ids.contains(&exchange.id)
                || matches!(exchange.status.as_str(), "replied" | "failed" | "discarded")
            {
                continue;
            }
            let target = self.store.control_thread(&exchange.target_thread_id)?;
            let target_title = target
                .as_ref()
                .map(thread_title)
                .unwrap_or_else(|| "Deleted linked thread".into());
            let target_summary = target
                .as_ref()
                .map(|thread| thread_summary(thread, false, 0))
                .transpose()?;
            exchange.status = if failure.is_some() {
                "failed"
            } else {
                "replied"
            }
            .into();
            exchange.updated_at_ms = now_ms();
            exchange.reply_json = Some(
                json!({
                    "target_title": target_title,
                    "target_workspace": target_summary.as_ref().and_then(|summary| summary.workspace.clone()),
                    "target_project": target_summary.as_ref().and_then(|summary| summary.workspace.as_deref().and_then(project_label)),
                    "target_model": target_summary.as_ref().and_then(|summary| summary.model.clone()),
                    "target_runtime": target_summary.as_ref().map(|summary| runtime_adapter(summary.model.as_deref().unwrap_or_default())),
                    "content": content.unwrap_or_default(),
                    "error": failure,
                })
                .to_string(),
            );
            self.store.control_put_mailbox(&exchange)?;
            self.project_mailbox_reply(&exchange)?;
        }
        Ok(())
    }

    pub(super) fn fail_mailbox_exchange_by_id(
        &self,
        exchange_id: &str,
        failure: &str,
    ) -> Result<()> {
        let Some(mut exchange) = self.store.control_mailbox(exchange_id)? else {
            return Ok(());
        };
        if matches!(exchange.status.as_str(), "replied" | "failed" | "discarded") {
            return Ok(());
        }
        let target_title = self
            .store
            .control_thread(&exchange.target_thread_id)?
            .as_ref()
            .map(thread_title)
            .unwrap_or_else(|| "Unavailable linked thread".into());
        exchange.status = "failed".into();
        exchange.updated_at_ms = now_ms();
        exchange.reply_json = Some(
            json!({
                "target_title": target_title,
                "content": "",
                "error": failure,
            })
            .to_string(),
        );
        self.store.control_put_mailbox(&exchange)?;
        self.project_mailbox_reply(&exchange)
    }

    fn project_mailbox_reply(&self, exchange: &ControlMailboxRecord) -> Result<()> {
        if exchange.projected_at_ms.is_some() {
            return Ok(());
        }
        let Some(_) = self.store.control_thread(&exchange.origin_thread_id)? else {
            return Ok(());
        };
        let reply = exchange
            .reply_json
            .as_deref()
            .map(parse_value)
            .transpose()?
            .unwrap_or(Value::Null);
        self.persist_and_emit(
            &exchange.origin_thread_id,
            exchange.origin_run_id.as_deref(),
            "mailbox_reply",
            json!({
                "exchange_id": exchange.id,
                "target_thread_id": exchange.target_thread_id,
                "status": exchange.status,
                "reply": reply,
            }),
        )?;
        self.store.control_mark_mailbox_projected(&exchange.id)?;
        self.emit(
            if exchange.status == "replied" {
                "mailbox.replied"
            } else {
                "mailbox.failed"
            },
            Some(&exchange.origin_thread_id),
            None,
            None,
            json!({
                "exchange_id": exchange.id,
                "target_thread_id": exchange.target_thread_id,
            }),
        );
        Ok(())
    }

    pub(super) fn reconcile_mailbox_projections(&self) -> Result<usize> {
        let mut projected = 0;
        for thread in self.store.control_threads()? {
            for exchange in self.store.control_mailbox_for_origin(&thread.id)? {
                if matches!(exchange.status.as_str(), "replied" | "failed")
                    && exchange.projected_at_ms.is_none()
                {
                    self.project_mailbox_reply(&exchange)?;
                    projected += 1;
                }
            }
        }
        Ok(projected)
    }
}

fn mailbox_wait_result(exchange: &ControlMailboxRecord, timed_out: bool) -> Result<Value> {
    let reply = exchange
        .reply_json
        .as_deref()
        .map(parse_value)
        .transpose()?
        .unwrap_or(Value::Null);
    Ok(json!({
        "exchange_id": exchange.id,
        "target_thread_id": exchange.target_thread_id,
        "target_run_id": exchange.target_run_id,
        "status": exchange.status,
        "completed": matches!(exchange.status.as_str(), "replied" | "failed" | "discarded"),
        "timed_out": timed_out,
        "reply": reply,
    }))
}

pub(super) fn mailbox_context_from_record(exchange: &ControlMailboxRecord) -> Option<String> {
    let reply = exchange
        .reply_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())?;
    let source_title = reply
        .get("target_title")
        .and_then(Value::as_str)
        .unwrap_or("linked thread");
    let body = reply
        .get("content")
        .or_else(|| reply.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("The linked thread ended without a reply.");
    Some(format!(
        "Mailbox reply {} from {} (thread {}), status {}:\n{}",
        exchange.id, source_title, exchange.target_thread_id, exchange.status, body
    ))
}

fn truncate_linked_content(content: &str, max_bytes: usize) -> (String, bool) {
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }
    let mut end = max_bytes.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    (content[..end].to_string(), true)
}

pub(super) fn linked_run_context(
    config: &FrozenRunConfigV1,
    mailbox_context: &[String],
) -> Option<String> {
    if config.linked_thread_grants.is_empty() && mailbox_context.is_empty() {
        return None;
    }
    let mut sections = Vec::new();
    if !config.linked_thread_grants.is_empty() {
        let links = config
            .linked_thread_grants
            .iter()
            .map(|grant| {
                format!(
                    "- {} (id: {}, project: {}, model/runtime: {}/{}, frozen revision/epoch/seq: {}/{}/{})",
                    grant.title,
                    grant.target_thread_id,
                    grant.project.as_deref().unwrap_or("unknown"),
                    grant.model.as_deref().unwrap_or("unknown"),
                    grant.runtime,
                    grant.revision,
                    grant.epoch,
                    grant.max_timeline_seq
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(format!(
            "Linked-thread grants for this run (use linked_thread_* tools; transcripts are not injected automatically):\n{links}"
        ));
    }
    if !mailbox_context.is_empty() {
        sections.push(format!(
            "Mailbox replies claimed when this run started:\n{}",
            mailbox_context.join("\n\n")
        ));
    }
    Some(sections.join("\n\n"))
}
