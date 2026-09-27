//! Run ledger: the per-run journal that records model requests, responses,
//! tool results, and composition artifacts, plus the Agent step hook.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use milim_control_contract::ResolvedRunCompositionV1;
use milim_core::api::openai::{ChatMessage, Content, ContentPart, ToolCall, Usage};
use milim_core::{Error, Result};
use milim_inference::CompletionRequest;
use milim_storage::{ControlRunArtifactRecord, UserDataStore};
use regex::{Captures, Regex};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::preview_runtime::managed_preview_runtime_context;
use super::provider::with_image_attachments;
use super::replay::completion_request_value;
use super::{now_ms, parse_value, AcceptedTurnV1};

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod ledger_tests;

pub(super) struct RunJournal {
    pub(super) store: Arc<UserDataStore>,
    pub(super) privacy: Arc<crate::privacy::PrivacyGate>,
    pub(super) privacy_mode: crate::privacy::PrivacyMode,
    pub(super) thread_id: String,
    pub(super) run_id: String,
    /// The latest committed model step as the model saw it; see [`SentStep`].
    pub(super) sent_step: Mutex<Option<SentStep>>,
}

/// The latest model step exactly as the Agent loop sent it and the model
/// answered.
///
/// The ledger keeps a privacy-processed, credential-scrubbed copy of each
/// step for inspection and replay. Rebuilding the next step from that copy
/// would show the model text it never saw: masked credentials, redaction
/// placeholders the outbound privacy gate would otherwise apply consistently
/// itself, and a changed prefix that defeats prompt caching. So the journal
/// keeps the exact step in memory and rebuilds from it. Each part is recorded
/// only after its ledger commit succeeds, so a step still contains only what
/// the ledger committed; a journal without the copy falls back to the ledger.
#[derive(Debug, Default)]
pub(super) struct SentStep {
    step: usize,
    messages: Vec<ChatMessage>,
    response_committed: bool,
    assistant: Option<ChatMessage>,
    tool_results: Vec<ChatMessage>,
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
    pub(super) fn new(
        store: Arc<UserDataStore>,
        privacy: Arc<crate::privacy::PrivacyGate>,
        privacy_mode: crate::privacy::PrivacyMode,
        thread_id: &str,
        run_id: &str,
    ) -> Self {
        Self {
            store,
            privacy,
            privacy_mode,
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
            sent_step: Mutex::default(),
        }
    }

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

    fn sent_step(&self) -> std::sync::MutexGuard<'_, Option<SentStep>> {
        self.sent_step
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The next step's messages from the exact copy of the previous one, if
    /// this journal committed that step's request and response itself.
    fn rebuild_sent_messages_for_step(
        &self,
        step: usize,
        memory_cache: &[ChatMessage],
    ) -> Option<Vec<ChatMessage>> {
        let sent = self.sent_step();
        let sent = sent
            .as_ref()
            .filter(|sent| sent.step + 1 == step && sent.response_committed)?;
        let had_tool_calls = sent
            .assistant
            .as_ref()
            .is_some_and(|message| message.tool_calls.is_some());
        let mut messages = sent.messages.clone();
        messages.extend(sent.assistant.clone());
        messages.extend(sent.tool_results.iter().cloned());
        messages.extend(image_follow_ups(memory_cache, had_tool_calls));
        Some(messages)
    }

    fn rebuild_messages_for_step(
        &self,
        step: usize,
        memory_cache: &[ChatMessage],
    ) -> Result<Option<Vec<ChatMessage>>> {
        if step <= 1 {
            return Ok(None);
        }
        if let Some(messages) = self.rebuild_sent_messages_for_step(step, memory_cache) {
            return Ok(Some(messages));
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
        let text = |key: &str| response.get(key).and_then(Value::as_str).unwrap_or("");
        let assistant = replayed_assistant_message(
            text("content"),
            text("reasoning"),
            tool_calls,
            response
                .get("provider_state")
                .filter(|state| !state.is_null())
                .cloned(),
        );
        let had_tool_calls = assistant
            .as_ref()
            .is_some_and(|message| message.tool_calls.is_some());
        messages.extend(assistant);
        for event in previous
            .iter()
            .filter(|event| event.event_type == "tool_result_committed")
        {
            let result = self.event_artifact_value(&event.data_json)?;
            let model_content = result
                .get("model_content")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Other("stored tool result has no model_content".into()))?;
            messages.push(tool_result_message(
                result.get("call_id").and_then(Value::as_str),
                model_content,
            ));
        }
        messages.extend(image_follow_ups(memory_cache, had_tool_calls));
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

/// The assistant turn a later step replays for a committed response. A step
/// without tool calls only continues after an output-limit cut off or a stop
/// hook; its text replays as plain assistant text, and a turn with no text at
/// all is dropped rather than sent empty.
fn replayed_assistant_message(
    content: &str,
    reasoning: &str,
    tool_calls: Vec<ToolCall>,
    provider_state: Option<Value>,
) -> Option<ChatMessage> {
    if content.is_empty() && tool_calls.is_empty() {
        return None;
    }
    Some(ChatMessage {
        role: "assistant".into(),
        content: (!content.is_empty()).then(|| Content::Text(content.to_string())),
        name: None,
        tool_calls: (!tool_calls.is_empty()).then_some(tool_calls),
        tool_call_id: None,
        reasoning_content: (!reasoning.is_empty()).then(|| reasoning.to_string()),
        provider_state,
    })
}

fn tool_result_message(call_id: Option<&str>, model_content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: Some(Content::Text(model_content.to_string())),
        name: None,
        tool_calls: None,
        tool_call_id: call_id.map(str::to_string),
        reasoning_content: None,
        provider_state: None,
    }
}

/// Binary tool images are referenced rather than duplicated in the ledger,
/// so the image follow-ups of the last tool-call turn come from the loop's
/// in-process messages. Only a step that ran tools can have produced them.
fn image_follow_ups(memory_cache: &[ChatMessage], had_tool_calls: bool) -> Vec<ChatMessage> {
    let Some(last_tool_call) = memory_cache
        .iter()
        .rposition(|message| message.role == "assistant" && message.tool_calls.is_some())
        .filter(|_| had_tool_calls)
    else {
        return Vec::new();
    };
    memory_cache[last_tool_call + 1..]
        .iter()
        .filter(|message| {
            message.role == "user"
                && matches!(
                    message.content.as_ref(),
                    Some(Content::Parts(parts))
                        if parts.iter().any(|part| matches!(part, ContentPart::ImageUrl { .. }))
                )
        })
        .cloned()
        .collect()
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
                    if credential_field(key, value) {
                        Ok((key.clone(), Value::String(REDACTED_CREDENTIAL.into())))
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

const REDACTED_CREDENTIAL: &str = "[REDACTED_CREDENTIAL]";

/// A JSON field that holds a credential: a non-empty string under a
/// credential name. Objects under such names (a tool schema's `password`
/// property, for example) are walked like any other value.
fn credential_field(key: &str, value: &Value) -> bool {
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
    ) && value.as_str().is_some_and(|text| !text.is_empty())
}

/// The credential shapes the ledger scrubs. Each rule marks only the secret
/// span, so the surrounding text survives and a JSON document embedded in a
/// string (tool-call arguments) stays valid JSON.
#[derive(Clone, Copy)]
enum CredentialRule {
    /// A PEM private key block, replaced whole.
    PrivateKey,
    /// A provider token with a recognizable prefix, replaced whole.
    PrefixedToken,
    /// The value of an `Authorization` header.
    AuthorizationHeader,
    /// A bearer token outside an `Authorization` header.
    Bearer,
    /// The value of a credential-named `key=value` or `"key": "value"` pair.
    KeyValue,
}

fn credential_rules() -> &'static [(CredentialRule, Regex)] {
    static RULES: OnceLock<Vec<(CredentialRule, Regex)>> = OnceLock::new();
    RULES.get_or_init(|| {
        vec![
            (
                CredentialRule::PrivateKey,
                Regex::new(
                    r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
                )
                .unwrap(),
            ),
            (
                CredentialRule::PrefixedToken,
                Regex::new(
                    r"(?:sk-(?:proj-|ant-|or-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}|xox[abposr]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{35})",
                )
                .unwrap(),
            ),
            (
                CredentialRule::AuthorizationHeader,
                Regex::new(
                    r#"(?i)authorization(?:\\?["'])?\s*[:=]\s*(?:\\?["'])?(?:(bearer|basic|token|bot|digest)\s+)?([A-Za-z0-9._~+/-]{8,}=*)"#,
                )
                .unwrap(),
            ),
            (
                CredentialRule::Bearer,
                Regex::new(r"(?i)bearer\s+([A-Za-z0-9._~+/-]{16,}=*)").unwrap(),
            ),
            (
                CredentialRule::KeyValue,
                Regex::new(
                    r#"(?i)(?:api[_-]?key|device[_-]?key|secret|access[_-]?token|refresh[_-]?token|auth[_-]?token|password|passwd)(?:\\?["'])?\s*[:=]\s*(?:\\?["'])?([^\s"'\\&;,]{8,})"#,
                )
                .unwrap(),
            ),
        ]
    })
}

/// Whether a value in credential position looks like a secret rather than a
/// reference (`$TOKEN`, `${KEY}`, `<your key>`), a marker (`[EMAIL_1]`), or
/// an ordinary word.
fn secret_like(value: &str) -> bool {
    !value.starts_with(['$', '{', '<', '%', '*', '['])
        && value.len() >= 8
        && (value.len() >= 20 || value.bytes().any(|byte| byte.is_ascii_digit()))
}

/// Whether a match at `start` begins a token rather than continuing a word,
/// counting an escape like `\n` inside JSON-encoded text as a separator.
fn starts_token(text: &str, start: usize) -> bool {
    let before = &text.as_bytes()[..start];
    match before {
        [] => true,
        [.., b'\\', b'n' | b'r' | b't'] => true,
        [.., last] => !(last.is_ascii_alphanumeric() || matches!(last, b'_' | b'-')),
    }
}

fn credential_span(
    rule: CredentialRule,
    text: &str,
    captures: &Captures<'_>,
) -> Option<Range<usize>> {
    let whole = captures.get(0)?;
    match rule {
        CredentialRule::PrivateKey => Some(whole.range()),
        CredentialRule::PrefixedToken => {
            let token = whole.as_str();
            (starts_token(text, whole.start())
                && (!token.starts_with("sk-") || token.bytes().any(|byte| byte.is_ascii_digit())))
            .then(|| whole.range())
        }
        CredentialRule::AuthorizationHeader => {
            let value = captures.get(2)?;
            (captures.get(1).is_some() || secret_like(value.as_str())).then(|| value.range())
        }
        CredentialRule::Bearer => {
            let value = captures.get(1)?;
            (starts_token(text, whole.start()) && secret_like(value.as_str()))
                .then(|| value.range())
        }
        CredentialRule::KeyValue => {
            let value = captures.get(1)?;
            secret_like(value.as_str()).then(|| value.range())
        }
    }
}

/// Replace each credential span in `text` with a marker, leaving the rest of
/// the text untouched.
fn scrub_credential_text(text: &str) -> String {
    let mut scrubbed = Cow::Borrowed(text);
    for (rule, regex) in credential_rules() {
        let spans = regex
            .captures_iter(&scrubbed)
            .filter_map(|captures| credential_span(*rule, &scrubbed, &captures))
            .collect::<Vec<_>>();
        if spans.is_empty() {
            continue;
        }
        let mut next = String::with_capacity(scrubbed.len());
        let mut cursor = 0;
        for span in spans {
            next.push_str(&scrubbed[cursor..span.start]);
            next.push_str(REDACTED_CREDENTIAL);
            cursor = span.end;
        }
        next.push_str(&scrubbed[cursor..]);
        scrubbed = Cow::Owned(next);
    }
    scrubbed.into_owned()
}

fn scrub_credential_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(scrub_credential_text(text)),
        Value::Array(values) => Value::Array(values.iter().map(scrub_credential_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    if credential_field(key, value) {
                        (key.clone(), Value::String(REDACTED_CREDENTIAL.into()))
                    } else {
                        (key.clone(), scrub_credential_value(value))
                    }
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// The model-visible user message for a steering input. Its image
/// attachments ride along exactly as they do on a replayed user turn.
fn steering_chat_message(accepted: &AcceptedTurnV1) -> Result<ChatMessage> {
    let mut message = json!({
        "role": "user",
        "content": accepted.text,
        "attachments": accepted.config.attachments,
    });
    with_image_attachments(&mut message);
    serde_json::from_value(message)
        .map_err(|error| Error::Other(format!("invalid steering message: {error}")))
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
                    messages.push(steering_chat_message(&accepted)?);
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
        )?;
        *self.sent_step() = Some(SentStep {
            step,
            messages: request.messages.clone(),
            ..SentStep::default()
        });
        Ok(())
    }

    async fn commit_model_response(
        &self,
        step: usize,
        content: &str,
        reasoning: &str,
        tool_calls: &[ToolCall],
        finish_reason: &str,
        usage: Usage,
        provider_state: Option<&Value>,
    ) -> Result<()> {
        let stored_tool_calls = self.privacy_processed_value(
            &serde_json::to_value(tool_calls)
                .map_err(|error| Error::Other(format!("serialize tool calls: {error}")))?,
        )?;
        let mut response = json!({
            "content": self.privacy_processed_text(content)?,
            "reasoning": self.privacy_processed_text(reasoning)?,
            "tool_calls": stored_tool_calls,
            "finish_reason": finish_reason,
            "usage": usage,
        });
        // Opaque provider continuation data (thinking signatures, encrypted
        // reasoning) is only valid byte-exact, so it bypasses privacy
        // processing and credential scrubbing.
        if let Some(state) = provider_state {
            response["provider_state"] = state.clone();
        }
        let digest = self.put_artifact("provider_response", &response)?;
        self.append_event(
            step,
            "model_response_committed",
            json!({
                "artifact_digest": digest,
                "finish_reason": finish_reason,
                "usage": usage,
            }),
        )?;
        if let Some(sent) = self.sent_step().as_mut().filter(|sent| sent.step == step) {
            sent.response_committed = true;
            sent.assistant = replayed_assistant_message(
                content,
                reasoning,
                tool_calls.to_vec(),
                provider_state.cloned(),
            );
        }
        Ok(())
    }

    async fn commit_tool_result(
        &self,
        step: usize,
        call_id: Option<&str>,
        name: &str,
        result: &Value,
        model_content: &str,
    ) -> Result<()> {
        let stored_content = self.privacy_processed_text(model_content)?;
        let model_content_bytes = stored_content.len();
        let artifact = json!({
            "call_id": call_id,
            "name": name,
            "result": self.privacy_processed_value(result)?,
            "model_content": stored_content,
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
        )?;
        if let Some(sent) = self
            .sent_step()
            .as_mut()
            .filter(|sent| sent.step == step && sent.response_committed)
        {
            sent.tool_results
                .push(tool_result_message(call_id, model_content));
        }
        Ok(())
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
