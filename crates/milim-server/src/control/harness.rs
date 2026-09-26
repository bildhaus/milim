//! Account runtime runs (Codex, Claude, OpenCode, Pi) through the harness
//! bridges, and the prompts sent to them.

use axum::http::{HeaderMap, HeaderValue};
use futures::StreamExt;
use milim_agents::AgentStepHook as _;
use milim_core::api::openai::Usage;
use milim_core::{Error, Result};
use milim_storage::ControlApprovalRecord;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use super::delta::{flush_deltas, DELTA_FLUSH_BYTES, DELTA_FLUSH_INTERVAL};
use super::journal::RunJournal;
use super::metrics::response_metrics_value;
use super::{
    control_account_images, frozen_harness_instructions, linked_run_context,
    managed_preview_runtime_context, normalized_approval_kind, now_ms, AcceptedTurnV1, RunManager,
    RunOutcome,
};
use crate::AppState;

impl RunManager {
    pub(super) async fn run_harness(
        &self,
        state: &AppState,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let messages = self.store.control_messages(thread_id)?;
        let prompt = account_runtime_prompt(
            &messages,
            accepted.config.native_session_id.as_deref(),
            accepted.config.native_session_cursor.as_deref(),
            &accepted.text,
        );
        let prompt = if let Some(context) =
            linked_run_context(&accepted.config, &accepted.mailbox_context)
        {
            format!("{context}\n\n{prompt}")
        } else {
            prompt
        };
        let instructions = [
            Some(frozen_harness_instructions(&accepted.config)).filter(|value| !value.is_empty()),
            managed_preview_runtime_context(&accepted.preview_runtime),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n");
        let (prompt, developer_instructions) =
            account_runtime_harness_prompt(&accepted.config.adapter, prompt, instructions);
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            HeaderValue::from_str(&format!("127.0.0.1:{}", state.config.port))
                .map_err(|error| Error::Other(format!("invalid control host header: {error}")))?,
        );
        let request = crate::routes::HarnessRunRequest {
            prompt,
            developer_instructions,
            images: control_account_images(&accepted.config.attachments),
            model: accepted.config.model.clone(),
            cwd: accepted.config.workspace.clone(),
            reasoning_effort: accepted.config.reasoning_effort.clone(),
            native_session_id: accepted.config.native_session_id.clone(),
            persist_session: Some(true),
            tool_approval_policy: Some(accepted.config.approval_mode.clone()),
            tool_approval_grant: false,
            interactive_tool_approval: accepted.config.approval_mode == "review",
            plan_mode: accepted.config.plan_mode,
            allow_session_recovery: false,
            account_profile_id: Some(accepted.config.account_profile_id.clone()),
            milim_context: Some(json!({
                "tool_context": {
                    "parent_model": format!("{}:{}", accepted.config.adapter, accepted.config.model),
                    "workspace": accepted.config.workspace,
                    "privacy_mode": accepted.config.privacy,
                    "tool_approval_policy": accepted.config.approval_mode,
                    "tool_approval_grant": false,
                    "interactive_tool_approval": accepted.config.approval_mode == "review",
                    "sandbox_enabled": accepted.config.sandbox,
                    "computer_use_enabled": accepted.config.computer_use,
                    "preview_tools_enabled": false,
                    "plan_mode": accepted.config.plan_mode,
                    "delegation_policy": accepted.config.delegation_policy,
                    "worker_model": accepted.config.worker_model,
                },
                "memory_context": {
                    "memory_enabled": accepted.config.memory,
                    "thread_id": thread_id,
                    "project_locator": accepted.config.workspace,
                    "linked_thread_grants": accepted.config.linked_thread_grants,
                },
                "tool_mode": accepted.config.tool_mode,
                "enabled_tools": accepted.config.enabled_tools,
                "skill_mode": accepted.config.skill_mode,
                "enabled_skills": accepted.config.enabled_skills,
            })),
        };
        let journal = RunJournal {
            store: self.store.clone(),
            privacy: state.privacy.clone(),
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
        };
        let boundary_request = json!({
            "adapter": accepted.config.adapter,
            "model": request.model,
            "prompt": journal.privacy_processed_text(&request.prompt)?,
            "cwd": request.cwd,
            "reasoning_effort": request.reasoning_effort,
            "native_session_id": request.native_session_id,
            "persist_session": request.persist_session,
            "account_profile_id": accepted.config.account_profile_id,
            "environment_policy": "AccountRuntimeInherited",
            "images": request.images.iter().map(|image| json!({
                "media_type": image.media_type,
                "digest": format!("sha256:{:x}", Sha256::digest(image.data.as_bytes())),
                "reference": "control-attachment",
            })).collect::<Vec<_>>(),
        });
        let request_digest = journal.put_artifact("harness_boundary_request", &boundary_request)?;
        journal.append_event(
            1,
            "harness_request_committed",
            json!({
                "artifact_digest": request_digest,
                "visibility": "harness_boundary",
            }),
        )?;
        let mut stream = crate::routes::account_harness_stream(
            state,
            &headers,
            &accepted.config.adapter,
            request,
        )
        .map_err(|error| error.0)?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        let mut emitted_first_delta = false;
        let mut final_usage: Option<Usage> = None;
        let mut reported_cost_usd: Option<f64> = None;
        let mut bound_session_id = accepted.config.native_session_id.clone();
        let mut session_recovery_required = false;
        loop {
            let event = tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        flush_deltas(
                            self,
                            thread_id,
                            run_id,
                            &mut pending_text,
                            &mut pending_reasoning,
                        )?;
                        return Ok(RunOutcome::Cancelled);
                    }
                    None
                }
                event = tokio::time::timeout(DELTA_FLUSH_INTERVAL, stream.next()) => {
                    match event {
                        Ok(event) => event,
                        Err(_) => {
                            flush_deltas(
                                self,
                                thread_id,
                                run_id,
                                &mut pending_text,
                                &mut pending_reasoning,
                            )?;
                            continue;
                        }
                    }
                },
            };
            let Some(event) = event else {
                break;
            };
            let value = serde_json::to_value(&event)
                .map_err(|error| Error::Other(format!("serialize harness event: {error}")))?;
            if let Some(usage) = value
                .get("usage")
                .filter(|usage| !usage.is_null())
                .and_then(|usage| serde_json::from_value::<Usage>(usage.clone()).ok())
            {
                final_usage = Some(usage);
            }
            if let Some(cost) = value
                .get("cost_usd")
                .and_then(Value::as_f64)
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
            {
                reported_cost_usd = Some(cost);
            }
            let event_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("runtime_notice");
            let timeline_type = event_type;
            let mut timeline_value = value.clone();
            if event_type == "session_recovery_required" {
                let recovery_session_id = bound_session_id.clone().or_else(|| {
                    value
                        .get("native_session_id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                });
                if let Some(expected) = recovery_session_id.as_deref() {
                    bound_session_id = self.clear_native_session_binding(
                        thread_id,
                        &accepted.config.adapter,
                        &accepted.config.account_profile_id,
                        expected,
                    )?;
                }
                session_recovery_required = true;
            } else if !session_recovery_required {
                if let Some(native_session_id) = value
                    .get("native_session_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if bound_session_id.as_deref() != Some(native_session_id) {
                        bound_session_id = self.persist_native_session_binding(
                            thread_id,
                            run_id,
                            &accepted.config.adapter,
                            &accepted.config.account_profile_id,
                            bound_session_id.as_deref(),
                            native_session_id,
                        )?;
                    }
                }
            }
            let mut is_delta = false;
            match event_type {
                "text_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                        if !pending_reasoning.is_empty() {
                            flush_deltas(
                                self,
                                thread_id,
                                run_id,
                                &mut pending_text,
                                &mut pending_reasoning,
                            )?;
                        }
                        pending_text.push_str(text);
                        is_delta = true;
                    }
                }
                "reasoning_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        reasoning.push_str(text);
                        if !pending_text.is_empty() {
                            flush_deltas(
                                self,
                                thread_id,
                                run_id,
                                &mut pending_text,
                                &mut pending_reasoning,
                            )?;
                        }
                        pending_reasoning.push_str(text);
                        is_delta = true;
                    }
                }
                "approval_requested" => {
                    if let Some(id) = value.get("approval_id").and_then(Value::as_str) {
                        let kind = normalized_approval_kind(
                            value
                                .get("request_kind")
                                .and_then(Value::as_str)
                                .unwrap_or("command"),
                        );
                        let request = value
                            .get("request")
                            .filter(|request| !request.is_null())
                            .cloned()
                            .unwrap_or_else(|| value.clone());
                        let request = self.enrich_linked_thread_send_approval(accepted, request)?;
                        self.store.control_put_approval(&ControlApprovalRecord {
                            id: id.to_string(),
                            run_id: run_id.to_string(),
                            thread_id: thread_id.to_string(),
                            kind: kind.to_string(),
                            request_json: request.to_string(),
                            status: "pending".into(),
                            decision_json: None,
                            created_at_ms: now_ms(),
                            resolved_at_ms: None,
                        })?;
                        let name = value
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let arguments = value
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(key) = self.auto_resolve_allowed_approval(
                            state, thread_id, id, kind, name, arguments,
                        )? {
                            timeline_value["auto_approved"] =
                                json!({ "scope": "thread", "allowance": key });
                        } else if let Some(prefix) =
                            crate::approval_allowances::prefix_allowance_for(kind, name, arguments)
                                .and_then(|rule| rule.prefix)
                        {
                            timeline_value["allowance_prefix"] = json!(prefix);
                        }
                    }
                }
                _ => {}
            }
            if is_delta {
                if !emitted_first_delta
                    || pending_text.len() + pending_reasoning.len() >= DELTA_FLUSH_BYTES
                {
                    flush_deltas(
                        self,
                        thread_id,
                        run_id,
                        &mut pending_text,
                        &mut pending_reasoning,
                    )?;
                    emitted_first_delta = true;
                }
                continue;
            }
            flush_deltas(
                self,
                thread_id,
                run_id,
                &mut pending_text,
                &mut pending_reasoning,
            )?;
            if matches!(event_type, "approval_requested" | "approval_resolved") {
                journal.append_event(1, event_type, journal.privacy_processed_value(&value)?)?;
            }
            self.persist_and_emit(thread_id, Some(run_id), timeline_type, timeline_value)?;
            if event.is_terminal() {
                if event_type == "turn_completed" {
                    let committed_usage = final_usage.unwrap_or_default();
                    journal
                        .commit_model_response(
                            1,
                            &content,
                            &reasoning,
                            &[],
                            "stop",
                            committed_usage,
                        )
                        .await?;
                    let metrics = response_metrics_value(
                        state,
                        &self.store,
                        run_id,
                        &accepted.config.model,
                        final_usage,
                        reported_cost_usd,
                    )
                    .await?;
                    let assistant_message_id = self.complete_assistant_message(
                        thread_id,
                        run_id,
                        content,
                        reasoning,
                        Some(metrics),
                    )?;
                    if let Some(native_session_id) = bound_session_id.as_deref() {
                        self.persist_native_session_cursor(
                            thread_id,
                            &accepted.config.adapter,
                            &accepted.config.account_profile_id,
                            native_session_id,
                            &assistant_message_id,
                        )?;
                    }
                    return Ok(RunOutcome::Completed);
                }
                if event_type == "turn_cancelled" {
                    return Ok(RunOutcome::Cancelled);
                }
                return Err(Error::Other(
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("account runtime failed")
                        .to_string(),
                ));
            }
        }
        Err(Error::Other(
            "account runtime ended without a terminal event".into(),
        ))
    }
}

pub(super) fn account_runtime_prompt(
    raw_messages: &[String],
    native_session_id: Option<&str>,
    native_session_cursor: Option<&str>,
    current_turn: &str,
) -> String {
    if native_session_id.is_some() && native_session_cursor.is_none() {
        return current_turn.to_string();
    }
    let messages = raw_messages
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .collect::<Vec<_>>();
    let start = native_session_cursor
        .and_then(|cursor| {
            messages
                .iter()
                .position(|message| message.get("id").and_then(Value::as_str) == Some(cursor))
        })
        .map(|index| index + 1)
        .unwrap_or_default();
    messages
        .iter()
        .skip(start)
        .filter_map(|message| {
            let role = message.get("role")?.as_str()?;
            let content = message
                .get("promptContent")
                .or_else(|| message.get("content"))?
                .as_str()?;
            Some(format!("{}:\n{}", uppercase_role(role), content))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn account_runtime_harness_prompt(
    adapter: &str,
    prompt: String,
    instructions: String,
) -> (String, Option<String>) {
    let instructions = instructions.trim();
    if instructions.is_empty() {
        return (prompt, None);
    }
    if adapter == "codex" {
        return (prompt, Some(instructions.to_string()));
    }
    (
        format!("System instructions:\n{instructions}\n\n{prompt}"),
        None,
    )
}

fn uppercase_role(role: &str) -> &'static str {
    match role {
        "system" => "System",
        "assistant" => "Assistant",
        _ => "User",
    }
}
