//! Milim Agent runs: the tool loop streamed through the canonical ledger.

use std::sync::Arc;

use futures::StreamExt;
use milim_core::api::openai::ChatMessage;
use milim_core::{Error, Result};
use milim_storage::ControlApprovalRecord;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::delta::{DeltaBuffer, DELTA_FLUSH_INTERVAL};
use super::journal::RunJournal;
use super::linked_threads::linked_run_context;
use super::metrics::{provider_context_window, provider_pricing, response_metrics_value};
use super::preview_runtime::managed_preview_runtime_context;
use super::provider::control_chat_messages;
use super::run_config::{
    compose_labeled_instructions, frozen_run_instructions, parse_reasoning_effort,
    sampling_from_generation,
};
use super::{now_ms, AcceptedTurnV1, RunManager, RunOutcome};
use crate::AppState;

impl RunManager {
    pub(super) async fn run_agent(
        &self,
        state: &AppState,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let agent = accepted
            .config
            .agent
            .as_ref()
            .map(|snapshot| milim_agents::AgentDef {
                id: snapshot.id.clone(),
                name: snapshot.name.clone(),
                description: snapshot.description.clone(),
                system_prompt: compose_labeled_instructions(
                    "Milim global instructions",
                    &accepted.config.global_instructions,
                    "Agent instructions",
                    &snapshot.system_prompt,
                ),
                model: String::new(),
                tool_mode: snapshot.tool_mode.clone(),
                enabled_tools: snapshot.enabled_tools.clone(),
                skill_mode: snapshot.skill_mode.clone(),
                enabled_skills: snapshot.enabled_skills.clone(),
                avatar: snapshot.avatar.clone(),
            })
            .unwrap_or_else(|| milim_agents::AgentDef {
                id: "control-default".into(),
                name: "Milim".into(),
                description: "Canonical provider chat".into(),
                system_prompt: frozen_run_instructions(&accepted.config),
                model: String::new(),
                tool_mode: accepted.config.tool_mode.clone(),
                enabled_tools: accepted.config.enabled_tools.clone(),
                skill_mode: accepted.config.skill_mode.clone(),
                enabled_skills: accepted.config.enabled_skills.clone(),
                avatar: "sparkles".into(),
            });
        let mut messages = control_chat_messages(&self.store, thread_id)?;
        if let Some(context) = managed_preview_runtime_context(&accepted.preview_runtime) {
            messages.insert(0, ChatMessage::text("system", context));
        }
        if let Some(context) = linked_run_context(&accepted.config, &accepted.mailbox_context) {
            messages.insert(0, ChatMessage::text("system", context));
        }
        let reasoning_effort = accepted
            .config
            .reasoning_effort
            .as_deref()
            .and_then(parse_reasoning_effort);
        let journal = Arc::new(RunJournal {
            store: self.store.clone(),
            privacy: state.privacy.clone(),
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
        });
        let mut stream = crate::routes::control_agent_stream(
            state,
            &agent,
            &accepted.config.model,
            messages,
            accepted.config.workspace.as_deref(),
            &accepted.config.privacy,
            &accepted.config.approval_mode,
            accepted.config.plan_mode,
            accepted.config.sandbox,
            accepted.config.computer_use,
            accepted.config.memory,
            &accepted.config.delegation_policy,
            &accepted.config.worker_model,
            thread_id,
            run_id,
            accepted.config.linked_thread_grants.clone(),
            reasoning_effort,
            sampling_from_generation(&accepted.config.generation, thread_id),
            accepted.config.run_limits.as_ref(),
            provider_pricing(state, &accepted.config.model).await,
            provider_context_window(state, &accepted.config.model).await,
            journal.clone(),
        )?;
        let mut deltas = DeltaBuffer::new(self, thread_id, run_id);
        loop {
            let event = tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        deltas.flush()?;
                        return Ok(RunOutcome::Cancelled);
                    }
                    None
                }
                event = tokio::time::timeout(DELTA_FLUSH_INTERVAL, stream.next()) => {
                    match event {
                        Ok(event) => event,
                        Err(_) => {
                            deltas.flush()?;
                            continue;
                        }
                    }
                },
            };
            let Some(event) = event else {
                break;
            };
            let mut value = serde_json::to_value(&event)
                .map_err(|error| Error::Other(format!("serialize Agent event: {error}")))?;
            let event_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("agent_event")
                .to_string();
            if matches!(
                &event,
                milim_agents::AgentEvent::ToolApprovalRequired { .. }
                    | milim_agents::AgentEvent::ToolApprovalResolved { .. }
            ) {
                journal.append_event(0, &event_type, journal.privacy_processed_value(&value)?)?;
            }
            match &event {
                milim_agents::AgentEvent::Token { text } => {
                    deltas.push_text(text);
                    deltas.flush_if_due()?;
                    continue;
                }
                milim_agents::AgentEvent::Reasoning { text } => {
                    deltas.push_reasoning(text);
                    deltas.flush_if_due()?;
                    continue;
                }
                milim_agents::AgentEvent::ToolApprovalRequired {
                    approval_id,
                    name,
                    arguments,
                    effect,
                    environment_policy,
                    ..
                } => {
                    deltas.flush()?;
                    let approval_request = self.enrich_linked_thread_send_approval(
                        accepted,
                        json!({
                            "approval_id": approval_id,
                            "name": name,
                            "arguments": arguments,
                            "effect": effect,
                            "environment_policy": environment_policy,
                            "environment_notice": matches!(
                                environment_policy,
                                milim_tools::ProcessEnvironmentPolicy::HostShellInherited
                            ).then_some(
                                "This host tool inherits your user environment; developer credentials may be accessible."
                            ),
                        }),
                    )?;
                    self.store.control_put_approval(&ControlApprovalRecord {
                        id: approval_id.clone(),
                        run_id: run_id.to_string(),
                        thread_id: thread_id.to_string(),
                        kind: "command".into(),
                        request_json: approval_request.to_string(),
                        status: "pending".into(),
                        decision_json: None,
                        created_at_ms: now_ms(),
                        resolved_at_ms: None,
                    })?;
                    if let Some(key) = self.auto_resolve_allowed_approval(
                        state,
                        thread_id,
                        approval_id,
                        "command",
                        name,
                        arguments,
                    )? {
                        value["auto_approved"] = json!({ "scope": "thread", "allowance": key });
                    } else if let Some(prefix) =
                        crate::approval_allowances::prefix_allowance_for("command", name, arguments)
                            .and_then(|rule| rule.prefix)
                    {
                        value["allowance_prefix"] = json!(prefix);
                    }
                }
                milim_agents::AgentEvent::ProviderRetry {
                    discarded_content_bytes,
                    discarded_reasoning_bytes,
                    ..
                } => {
                    deltas.flush()?;
                    deltas.truncate_for_retry(*discarded_content_bytes, *discarded_reasoning_bytes);
                }
                milim_agents::AgentEvent::ToolApprovalResolved {
                    approval_id,
                    reason: Some(reason),
                    ..
                } => {
                    deltas.flush()?;
                    // The loop denied the request itself (e.g. it timed out);
                    // close the stored approval so it no longer shows pending.
                    if let Some(mut durable) = self.store.control_approval(approval_id)? {
                        if durable.status == "pending" {
                            durable.status = "expired".into();
                            durable.decision_json =
                                Some(json!({ "decision": "deny", "reason": reason }).to_string());
                            durable.resolved_at_ms = Some(now_ms());
                            self.store.control_put_approval(&durable)?;
                        }
                    }
                }
                milim_agents::AgentEvent::Done {
                    usage,
                    stopped_at_limit,
                    ..
                } => {
                    deltas.flush()?;
                    self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
                    let metrics = response_metrics_value(
                        state,
                        &self.store,
                        run_id,
                        &accepted.config.model,
                        Some(*usage),
                        None,
                    )
                    .await?;
                    let (content, reasoning) = deltas.into_output();
                    self.complete_assistant_message(
                        thread_id,
                        run_id,
                        content,
                        reasoning,
                        Some(metrics),
                    )?;
                    return Ok(if *stopped_at_limit {
                        RunOutcome::Limited
                    } else {
                        RunOutcome::Completed
                    });
                }
                milim_agents::AgentEvent::Error { message } => {
                    deltas.flush()?;
                    self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
                    return Err(Error::Other(message.clone()));
                }
                _ => {
                    deltas.flush()?;
                }
            }
            self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
        }
        Err(Error::Other(
            "Agent stream ended without a terminal event".into(),
        ))
    }
}
