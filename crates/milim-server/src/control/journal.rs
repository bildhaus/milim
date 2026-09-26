//! Run ledger: the per-run journal that records model requests, responses,
//! tool results, and composition artifacts, plus the Agent step hook.

use std::sync::Arc;

use milim_control_contract::ResolvedRunCompositionV1;
use milim_core::api::openai::{ChatMessage, ToolCall, Usage};
use milim_core::{Error, Result};
use milim_inference::CompletionRequest;
use milim_storage::{ControlRunArtifactRecord, UserDataStore};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::preview_runtime::managed_preview_runtime_context;
use super::replay::completion_request_value;
use super::{now_ms, parse_value, AcceptedTurnV1};

pub(super) struct RunJournal {
    pub(super) store: Arc<UserDataStore>,
    pub(super) privacy: Arc<crate::privacy::PrivacyGate>,
    pub(super) privacy_mode: crate::privacy::PrivacyMode,
    pub(super) thread_id: String,
    pub(super) run_id: String,
}

impl std::fmt::Debug for RunJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunJournal")
            .field("thread_id", &self.thread_id)
            .field("run_id", &self.run_id)
            .field("privacy_mode", &self.privacy_mode)
            .finish_non_exhaustive()
    }
}

impl RunJournal {
    pub(super) fn append_event(&self, step: usize, event_type: &str, data: Value) -> Result<()> {
        self.store.control_append_run_event(
            &self.run_id,
            &Uuid::new_v4().to_string(),
            Some(&format!("step-{step}")),
            event_type,
            &data.to_string(),
        )?;
        Ok(())
    }

    pub(super) fn put_artifact(&self, kind: &str, data: &Value) -> Result<String> {
        let encoded = serde_json::to_vec(data)
            .map_err(|error| Error::Other(format!("serialize run artifact: {error}")))?;
        let byte_len = u64::try_from(encoded.len()).unwrap_or(u64::MAX);
        let digest = format!("sha256:{:x}", Sha256::digest(&encoded));
        self.store
            .control_put_run_artifact(&ControlRunArtifactRecord {
                run_id: self.run_id.clone(),
                digest: digest.clone(),
                kind: kind.into(),
                data_json: String::from_utf8(encoded)
                    .map_err(|error| Error::Other(format!("encode run artifact: {error}")))?,
                byte_len,
                created_at_ms: now_ms(),
            })?;
        Ok(digest)
    }

    fn privacy_processed_request(&self, request: &CompletionRequest) -> Result<Value> {
        ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        }
        .resolve_request(request)
    }

    pub(super) fn privacy_processed_text(&self, text: &str) -> Result<String> {
        ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        }
        .resolve_text(text)
    }

    pub(super) fn privacy_processed_value(&self, value: &Value) -> Result<Value> {
        ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        }
        .resolve_value(value)
    }

    fn artifact_value(&self, digest: &str) -> Result<Value> {
        let artifact = self
            .store
            .control_run_artifact(&self.run_id, digest)?
            .ok_or_else(|| Error::Other(format!("run artifact {digest} is missing")))?;
        parse_value(&artifact.data_json)
    }

    fn event_artifact_value(&self, data_json: &str) -> Result<Value> {
        let data = parse_value(data_json)?;
        let digest = data
            .get("artifact_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Other("run event is missing artifact_digest".into()))?;
        self.artifact_value(digest)
    }

    fn rebuild_messages_for_step(
        &self,
        step: usize,
        memory_cache: &[ChatMessage],
    ) -> Result<Option<Vec<ChatMessage>>> {
        if step <= 1 {
            return Ok(None);
        }
        let previous_step_id = format!("step-{}", step - 1);
        let events = self
            .store
            .control_run_events_for_step(&self.run_id, &previous_step_id)?;
        let previous = events.iter().collect::<Vec<_>>();
        let request_event = previous
            .iter()
            .rev()
            .find(|event| event.event_type == "model_request_resolved")
            .ok_or_else(|| {
                Error::Other(format!(
                    "cannot rebuild step {step}: previous model request is missing"
                ))
            })?;
        let request = self.event_artifact_value(&request_event.data_json)?;
        let mut messages: Vec<ChatMessage> = serde_json::from_value(
            request
                .get("messages")
                .cloned()
                .ok_or_else(|| Error::Other("stored provider request has no messages".into()))?,
        )
        .map_err(|error| Error::Other(format!("decode stored provider messages: {error}")))?;

        let response_event = previous
            .iter()
            .rev()
            .find(|event| event.event_type == "model_response_committed")
            .ok_or_else(|| {
                Error::Other(format!(
                    "cannot rebuild step {step}: previous model response is missing"
                ))
            })?;
        let response = self.event_artifact_value(&response_event.data_json)?;
        let tool_calls: Vec<ToolCall> = serde_json::from_value(
            response
                .get("tool_calls")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|error| Error::Other(format!("decode stored provider tool calls: {error}")))?;
        let content = response
            .get("content")
            .and_then(Value::as_str)
            .filter(|content| !content.is_empty())
            .map(|content| milim_core::api::openai::Content::Text(content.to_string()));
        // A step without tool calls only continues after an output-limit cut
        // off; its partial text replays as plain assistant text, and a turn
        // with no text at all is dropped rather than sent empty.
        let had_tool_calls = !tool_calls.is_empty();
        if content.is_some() || had_tool_calls {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content,
                name: None,
                tool_calls: had_tool_calls.then_some(tool_calls),
                tool_call_id: None,
                reasoning_content: response
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .filter(|reasoning| !reasoning.is_empty())
                    .map(str::to_string),
            });
        }
        for event in previous
            .iter()
            .filter(|event| event.event_type == "tool_result_committed")
        {
            let result = self.event_artifact_value(&event.data_json)?;
            let model_content = result
                .get("model_content")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("stored tool result has no model_content".into()))?;
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(milim_core::api::openai::Content::Text(
                    model_content.to_string(),
                )),
                name: None,
                tool_calls: None,
                tool_call_id: result
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                reasoning_content: None,
            });
        }

        // Binary tool images are referenced rather than duplicated in the
        // ledger. Keep only those image follow-ups from the in-process cache;
        // all text and JSON above is rebuilt from SQLite. Only a step that
        // ran tools can have produced new image follow-ups.
        if let Some(last_tool_call) = memory_cache
            .iter()
            .rposition(|message| message.role == "assistant" && message.tool_calls.is_some())
            .filter(|_| had_tool_calls)
        {
            messages.extend(
                memory_cache[last_tool_call + 1..]
                    .iter()
                    .filter(|message| {
                        message.role == "user"
                            && matches!(
                                message.content.as_ref(),
                                Some(milim_core::api::openai::Content::Parts(parts))
                                    if parts.iter().any(|part| matches!(part, milim_core::api::openai::ContentPart::ImageUrl { .. }))
                            )
                    })
                    .cloned(),
            );
        }
        Ok(Some(messages))
    }

    pub(super) fn commit_failure(&self, step: usize, error: &Error) -> Result<()> {
        let (message, privacy_rejected) = match self.privacy_processed_text(&error.to_string()) {
            Ok(message) => (message, false),
            Err(_) => ("[REJECTED_BY_PRIVACY_BLOCK]".to_string(), true),
        };
        self.append_event(
            step,
            "run_error_committed",
            json!({
                "code": error.code(),
                "message": message,
                "privacy_rejected": privacy_rejected,
            }),
        )
    }
}

pub(super) struct ModelInputResolver<'a> {
    pub(super) privacy: &'a crate::privacy::PrivacyGate,
    pub(super) privacy_mode: crate::privacy::PrivacyMode,
}

impl ModelInputResolver<'_> {
    fn resolve_request(&self, request: &CompletionRequest) -> Result<Value> {
        let mut processed = request.clone();
        match self.privacy_mode {
            crate::privacy::PrivacyMode::Off => {}
            crate::privacy::PrivacyMode::Block => {
                if crate::privacy::request_has_image_parts(&processed)
                    || !self.privacy.scan_request(&processed).is_empty()
                {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate before run-ledger persistence".into(),
                    ));
                }
            }
            crate::privacy::PrivacyMode::Redact => {
                if crate::privacy::request_has_image_parts(&processed) {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate before run-ledger persistence: image data cannot be redacted".into(),
                    ));
                }
                self.privacy.redact_request(&mut processed);
            }
        }
        completion_request_value(&processed).map(|value| scrub_credential_value(&value))
    }

    fn resolve_text(&self, text: &str) -> Result<String> {
        let processed = match self.privacy_mode {
            crate::privacy::PrivacyMode::Off => text.to_string(),
            crate::privacy::PrivacyMode::Redact => self.privacy.redact_text(text).text,
            crate::privacy::PrivacyMode::Block => {
                if self.privacy.is_clean_text(text) {
                    text.to_string()
                } else {
                    return Err(Error::InvalidRequest(
                        "blocked by the privacy gate before run-ledger persistence".into(),
                    ));
                }
            }
        };
        Ok(scrub_credential_text(&processed))
    }

    fn resolve_value(&self, value: &Value) -> Result<Value> {
        match value {
            Value::String(text) => self.resolve_text(text).map(Value::String),
            Value::Array(values) => values
                .iter()
                .map(|value| self.resolve_value(value))
                .collect::<Result<Vec<_>>>()
                .map(Value::Array),
            Value::Object(values) => values
                .iter()
                .map(|(key, value)| {
                    if credential_field_name(key) {
                        Ok((key.clone(), Value::String("[REDACTED_CREDENTIAL]".into())))
                    } else {
                        self.resolve_value(value).map(|value| (key.clone(), value))
                    }
                })
                .collect::<Result<Map<String, Value>>>()
                .map(Value::Object),
            _ => Ok(value.clone()),
        }
    }
}

impl RunJournal {
    pub(super) fn commit_composition(&self, accepted: &AcceptedTurnV1) -> Result<()> {
        let resolver = ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        };
        let composition = resolved_run_composition(accepted, &resolver)?;
        let visibility = composition.visibility.clone();
        let composition = serde_json::to_value(composition)
            .map_err(|error| Error::Other(format!("serialize run composition: {error}")))?;
        let digest = self.put_artifact("run_composition", &composition)?;
        self.store.control_append_run_event(
            &self.run_id,
            &Uuid::new_v4().to_string(),
            None,
            "run_composition_resolved",
            &json!({ "artifact_digest": digest, "visibility": visibility }).to_string(),
        )?;
        Ok(())
    }
}

pub(super) fn resolved_run_composition(
    accepted: &AcceptedTurnV1,
    resolver: &ModelInputResolver<'_>,
) -> Result<ResolvedRunCompositionV1> {
    let visibility = if matches!(
        accepted.config.adapter.as_str(),
        "codex" | "claude" | "opencode" | "pi"
    ) {
        "harness_boundary"
    } else {
        "model_visible"
    };
    let environment_policy = if visibility == "harness_boundary" {
        "AccountRuntimeInherited"
    } else {
        "MilimProviderBoundary"
    };
    let attachments = accepted
        .config
        .attachments
        .iter()
        .map(|attachment| {
            let identity = attachment
                .data_url
                .as_deref()
                .or(attachment.content.as_deref())
                .unwrap_or_default();
            json!({
                "id": attachment.id,
                "name": attachment.name,
                "mime": attachment.mime,
                "size": attachment.size,
                "digest": format!("sha256:{:x}", Sha256::digest(identity.as_bytes())),
                "reference": format!("control-attachment:{}", attachment.id),
                "truncated": attachment.truncated,
            })
        })
        .collect::<Vec<_>>();
    let (instruction_kind, instruction_provenance, instruction_content) =
        if let Some(agent) = accepted.config.agent.as_ref() {
            (
                "agent_instructions",
                format!("agent:{}", agent.id),
                resolver.resolve_text(&agent.system_prompt)?,
            )
        } else {
            (
                "thread_instructions",
                "frozen_run_config".to_string(),
                resolver.resolve_text(&accepted.config.instructions)?,
            )
        };
    Ok(ResolvedRunCompositionV1 {
        visibility: visibility.to_string(),
        adapter: accepted.config.adapter.clone(),
        model: accepted.config.model.clone(),
        reasoning_effort: accepted.config.reasoning_effort.clone(),
        generation: accepted.config.generation.clone(),
        native_session_boundary: accepted.config.native_session_id.clone(),
        workspace: accepted.config.workspace.clone(),
        environment_policy: environment_policy.to_string(),
        explicit_environment_grants: Vec::new(),
        prompt_sections: vec![
            json!({
                "kind": "global_instructions",
                "provenance": "frozen_run_config",
                "content": resolver.resolve_text(&accepted.config.global_instructions)?,
            }),
            json!({
                "kind": instruction_kind,
                "provenance": instruction_provenance,
                "content": instruction_content,
            }),
            json!({
                "kind": "user",
                "provenance": "accepted_turn",
                "content": resolver.resolve_text(&accepted.text)?,
            }),
        ],
        tools: accepted
            .config
            .enabled_tools
            .iter()
            .map(|name| {
                json!({
                    "name": name,
                    "provenance": "frozen_run_config",
                })
            })
            .collect::<Vec<_>>(),
        policies: json!({
            "privacy": accepted.config.privacy,
            "approval": accepted.config.approval_mode,
            "tool_mode": accepted.config.tool_mode,
            "skill_mode": accepted.config.skill_mode,
            "enabled_skills": accepted.config.enabled_skills,
            "memory": accepted.config.memory,
            "plan_mode": accepted.config.plan_mode,
            "sandbox": accepted.config.sandbox,
            "computer_use": accepted.config.computer_use,
            "delegation": accepted.config.delegation_policy,
            "linked_thread_grants": accepted.config.linked_thread_grants,
        }),
        attachments,
    })
}

fn credential_field_name(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "authorization"
            | "apikey"
            | "accesstoken"
            | "refreshtoken"
            | "bearertoken"
            | "devicekey"
            | "clientsecret"
            | "password"
            | "secret"
    )
}

fn scrub_credential_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let markers = [
        "bearer ",
        "authorization:",
        "api_key=",
        "api-key=",
        "apikey=",
        "openai_api_key=",
        "anthropic_api_key=",
        "device_key=",
        "client_secret=",
        "sk-",
    ];
    if markers.iter().any(|marker| lower.contains(marker)) {
        "[REDACTED_CREDENTIAL]".into()
    } else {
        text.to_string()
    }
}

fn scrub_credential_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(scrub_credential_text(text)),
        Value::Array(values) => Value::Array(values.iter().map(scrub_credential_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    if credential_field_name(key) {
                        (key.clone(), Value::String("[REDACTED_CREDENTIAL]".into()))
                    } else {
                        (key.clone(), scrub_credential_value(value))
                    }
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

#[async_trait::async_trait]
impl milim_agents::AgentStepHook for RunJournal {
    fn output_scope(&self) -> Option<String> {
        Some(self.run_id.clone())
    }

    async fn commit_tool_catalog(&self, tools: &[milim_tools::ToolExecutionSpec]) -> Result<()> {
        let tools = self.privacy_processed_value(
            &serde_json::to_value(tools)
                .map_err(|error| Error::Other(format!("serialize effective tools: {error}")))?,
        )?;
        let digest = self.put_artifact("effective_tools", &tools)?;
        self.store.control_append_run_event(
            &self.run_id,
            &Uuid::new_v4().to_string(),
            None,
            "effective_tools_resolved",
            &json!({"artifact_digest": digest}).to_string(),
        )?;
        Ok(())
    }

    async fn prepare_model_step(&self, step: usize, messages: &mut Vec<ChatMessage>) -> Result<()> {
        if let Some(rebuilt) = self.rebuild_messages_for_step(step, messages)? {
            *messages = rebuilt;
        }
        let claimed = self
            .store
            .control_claim_step_inputs(&self.thread_id, &self.run_id)?;
        for item in &claimed {
            match item.kind.as_str() {
                "steer" => {
                    let accepted: AcceptedTurnV1 = serde_json::from_str(&item.payload_json)
                        .map_err(|error| {
                            Error::Other(format!("stored steering input is invalid: {error}"))
                        })?;
                    if let Some(context) =
                        managed_preview_runtime_context(&accepted.preview_runtime)
                    {
                        messages.push(ChatMessage::text("system", context));
                    }
                    messages.push(ChatMessage::text("user", accepted.text.clone()));
                    let message = json!({
                        "id": Uuid::new_v4().to_string(),
                        "role": "user",
                        "content": accepted.display_text.as_deref().unwrap_or(&accepted.text),
                        "promptContent": accepted.text,
                        "attachments": accepted.config.attachments,
                        "runId": self.run_id,
                        "steering": true,
                        "steeringInboxId": item.id,
                        "mailboxOrigin": accepted.mailbox_origin,
                    });
                    let message_id = message
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or(&item.id);
                    let step_id = format!("step-{step}");
                    self.store.control_commit_message_projection_and_event(
                        &self.thread_id,
                        &self.run_id,
                        message_id,
                        &message.to_string(),
                        &Uuid::new_v4().to_string(),
                        Some(&step_id),
                        "inbox_input_projected",
                        &json!({"inbox_id": item.id, "item_id": message_id}).to_string(),
                    )?;
                }
                "inject" => {
                    let payload = parse_value(&item.payload_json)?;
                    if let Some(text) = payload.get("text").and_then(Value::as_str) {
                        messages.push(ChatMessage::text(
                            "system",
                            format!("Injected context:\n{text}"),
                        ));
                    }
                }
                _ => {}
            }
        }
        if !claimed.is_empty() {
            self.append_event(
                step,
                "inbox_claimed",
                json!({
                    "items": claimed.iter().map(|item| json!({
                        "id": item.id,
                        "kind": item.kind,
                        "target_run_id": item.target_run_id,
                    })).collect::<Vec<_>>()
                }),
            )?;
        }
        Ok(())
    }

    async fn commit_model_request(&self, step: usize, request: &CompletionRequest) -> Result<()> {
        let data = self.privacy_processed_request(request)?;
        let digest = self.put_artifact("provider_request", &data)?;
        self.append_event(
            step,
            "model_request_resolved",
            json!({ "artifact_digest": digest, "privacy": self.privacy_mode.as_str() }),
        )
    }

    async fn commit_model_response(
        &self,
        step: usize,
        content: &str,
        reasoning: &str,
        tool_calls: &[ToolCall],
        finish_reason: &str,
        usage: Usage,
    ) -> Result<()> {
        let content = self.privacy_processed_text(content)?;
        let reasoning = self.privacy_processed_text(reasoning)?;
        let tool_calls = self.privacy_processed_value(
            &serde_json::to_value(tool_calls)
                .map_err(|error| Error::Other(format!("serialize tool calls: {error}")))?,
        )?;
        let response = json!({
            "content": content,
            "reasoning": reasoning,
            "tool_calls": tool_calls,
            "finish_reason": finish_reason,
            "usage": usage,
        });
        let digest = self.put_artifact("provider_response", &response)?;
        self.append_event(
            step,
            "model_response_committed",
            json!({
                "artifact_digest": digest,
                "finish_reason": finish_reason,
                "usage": usage,
            }),
        )
    }

    async fn commit_tool_result(
        &self,
        step: usize,
        call_id: Option<&str>,
        name: &str,
        result: &Value,
        model_content: &str,
    ) -> Result<()> {
        let result = self.privacy_processed_value(result)?;
        let model_content = self.privacy_processed_text(model_content)?;
        let model_content_bytes = model_content.len();
        let artifact = json!({
            "call_id": call_id,
            "name": name,
            "result": result,
            "model_content": model_content,
        });
        let digest = self.put_artifact("tool_result", &artifact)?;
        self.append_event(
            step,
            "tool_result_committed",
            json!({
                "artifact_digest": digest,
                "call_id": call_id,
                "name": name,
                "model_content_bytes": model_content_bytes,
            }),
        )
    }

    async fn commit_context_compaction(
        &self,
        step: usize,
        compaction: &milim_agents::ContextCompaction,
    ) -> Result<()> {
        self.append_event(
            step,
            "context_compacted",
            serde_json::to_value(compaction)
                .map_err(|error| Error::Other(format!("serialize context compaction: {error}")))?,
        )
    }

    async fn commit_model_timing(
        &self,
        step: usize,
        timing: &milim_agents::ModelStepTiming,
    ) -> Result<()> {
        self.append_event(
            step,
            "model_timing",
            json!({
                "step": step,
                "started_at_ms": timing.started_at_ms,
                "first_token_ms": timing.first_token_ms,
                "duration_ms": timing.duration_ms,
                "attempts": timing.attempts,
                "finish_reason": timing.finish_reason,
            }),
        )
    }

    async fn commit_tool_timing(
        &self,
        step: usize,
        call_id: Option<&str>,
        name: &str,
        duration_ms: u64,
        is_error: bool,
    ) -> Result<()> {
        self.append_event(
            step,
            "tool_timing",
            json!({
                "step": step,
                "call_id": call_id,
                "name": name,
                "duration_ms": duration_ms,
                "is_error": is_error,
            }),
        )
    }
}
