//! Canonical desktop/mobile control protocol and server-owned run lifecycle.
//!
//! `/control/v1` is deliberately separate from the legacy child-thread API.
//! The durable user session tables remain authoritative; this module adds
//! sequencing, command idempotency, queues, and live replication around them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, HeaderValue};
use base64::Engine as _;
use futures::StreamExt;
use milim_agents::AgentStepHook as _;
pub use milim_control_contract::*;
use milim_core::api::openai::{ChatMessage, ReasoningEffort, ToolCall, Usage};
use milim_core::{Error, Result};
use milim_inference::{CompletionRequest, SamplingParams, StreamEvent};
use milim_storage::{
    ControlApprovalRecord, ControlCommandReceiptRecord, ControlHostRecord, ControlInboxRecord,
    ControlMailboxRecord, ControlQueuedTurnRecord, ControlRunArtifactRecord, ControlRunRecord,
    ControlThreadRecord, ControlThreadLinkRecord, ControlTimelineRecord, UserDataStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, watch, Mutex as AsyncMutex};
use uuid::Uuid;

use crate::routes::{service_for_run, RunContext};
use crate::AppState;

const CONTROL_EVENT_CAPACITY: usize = 1_024;
const CONFIRMATION_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_CONTROL_ATTACHMENTS: usize = 6;
const MAX_CONTROL_ATTACHMENT_NAME_CHARS: usize = 140;
const MAX_CONTROL_ATTACHMENT_MIME_CHARS: usize = 120;
const MAX_CONTROL_ATTACHMENT_CONTENT_CHARS: usize = 128 * 1024;
const MAX_CONTROL_ATTACHMENT_DATA_URL_CHARS: usize = 3 * 1024 * 1024;
const MAX_CONTROL_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024;
const APPEARANCE_STATE_KEY: &str = "milim.appearanceSnapshot";
const CUSTOM_THEMES_STATE_KEY: &str = "milim.customThemes";
const MAX_APPEARANCE_BACKGROUND_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODEL_FAVORITES: usize = 256;
const MAX_MODEL_FAVORITE_ID_CHARS: usize = 512;
pub const MODEL_FAVORITES_SETTINGS_KEY: &str = "milim.settings";
pub const MODEL_FAVORITES_EVENT_TYPE: &str = "model_favorites.updated";

// Wire declarations live in `milim-control-contract`. Keeping the former
// declarations compiled out for this cutover makes the ownership move easy to
// audit while all server call sites use the canonical crate above.
#[cfg(any())]
mod legacy_control_contract {
    use super::*;

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ControlProtocolRangeV1 {
        pub min: u16,
        pub max: u16,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlCapabilitiesV1 {
        pub timeline_sync: bool,
        pub queued_turns: bool,
        pub approvals: bool,
        pub agents: bool,
        pub workers: bool,
        pub attachments: bool,
        pub websocket_tickets: bool,
        pub lan_discovery: bool,
        pub push_notifications: bool,
        pub inline_branches: bool,
        pub appearance_assets: bool,
    }

    impl Default for ControlCapabilitiesV1 {
        fn default() -> Self {
            Self {
                timeline_sync: true,
                queued_turns: true,
                approvals: true,
                agents: true,
                workers: true,
                attachments: true,
                websocket_tickets: true,
                lan_discovery: true,
                push_notifications: false,
                inline_branches: false,
                appearance_assets: true,
            }
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ThreadSummaryV1 {
        pub id: String,
        pub title: String,
        pub revision: u64,
        pub epoch: String,
        pub updated_at_ms: i64,
        pub archived_at_ms: Option<i64>,
        pub model: Option<String>,
        #[serde(default)]
        pub reasoning_effort_overrides: HashMap<String, String>,
        pub agent_id: Option<String>,
        pub workspace: Option<String>,
        pub busy: bool,
        pub queued_turns: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct AgentSummaryV1 {
        pub id: String,
        pub name: String,
        pub description: String,
        pub avatar: String,
        pub tool_mode: String,
        pub enabled_tool_count: usize,
        pub skill_mode: String,
        pub enabled_skill_count: usize,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct AgentSnapshotV1 {
        pub id: String,
        pub name: String,
        pub description: String,
        pub avatar: String,
        pub system_prompt: String,
        pub tool_mode: String,
        pub enabled_tools: Vec<String>,
        pub skill_mode: String,
        pub enabled_skills: Vec<String>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct FrozenRunConfigV1 {
        pub model: String,
        #[serde(default)]
        pub instructions: String,
        pub workspace: Option<String>,
        pub privacy: String,
        pub approval_mode: String,
        pub plan_mode: bool,
        pub sandbox: bool,
        pub computer_use: bool,
        pub memory: bool,
        pub delegation_policy: String,
        pub worker_model: String,
        pub agent: Option<AgentSnapshotV1>,
        #[serde(default = "default_control_tool_mode")]
        pub tool_mode: String,
        pub enabled_tools: Vec<String>,
        #[serde(default = "default_control_skill_mode")]
        pub skill_mode: String,
        pub enabled_skills: Vec<String>,
        pub attachments: Vec<ControlAttachmentV1>,
        pub native_session_id: Option<String>,
        pub reasoning_effort: Option<String>,
        #[serde(default)]
        pub generation: GenerationSettingsV1,
        pub adapter: String,
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct GenerationSettingsV1 {
        pub max_tokens: Option<u32>,
        pub temperature: Option<f32>,
        pub top_p: Option<f32>,
        pub seed: Option<i64>,
        #[serde(default)]
        pub stop: Vec<String>,
        pub frequency_penalty: Option<f32>,
        pub presence_penalty: Option<f32>,
        pub top_k: Option<i32>,
        pub min_p: Option<f32>,
        pub repetition_penalty: Option<f32>,
        pub thinking_token_budget: Option<u32>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlAttachmentV1 {
        pub id: String,
        pub name: String,
        pub mime: String,
        pub size: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data_url: Option<String>,
        #[serde(default)]
        pub truncated: bool,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct RunSnapshotV1 {
        pub id: String,
        pub thread_id: String,
        pub status: String,
        pub adapter: String,
        pub config: FrozenRunConfigV1,
        pub created_at_ms: i64,
        pub updated_at_ms: i64,
        pub completed_at_ms: Option<i64>,
        pub error: Option<Value>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct PendingApprovalV1 {
        pub id: String,
        pub run_id: String,
        pub thread_id: String,
        pub kind: String,
        pub request: Value,
        pub status: String,
        pub created_at_ms: i64,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct QueuedTurnV1 {
        pub id: String,
        pub thread_id: String,
        pub command_id: String,
        pub accepted_at_ms: i64,
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceColorsV1 {
        pub primary_text: String,
        pub secondary_text: String,
        pub tertiary_text: String,
        pub placeholder_text: String,
        pub bg_primary: String,
        pub bg_secondary: String,
        pub bg_tertiary: String,
        pub sidebar_bg: String,
        pub accent: String,
        pub accent_light: String,
        pub border_primary: String,
        pub border_secondary: String,
        pub focus_border: String,
        pub success: String,
        pub warning: String,
        pub error: String,
        pub info: String,
        pub card_bg: String,
        pub card_border: String,
        pub input_bg: String,
        pub input_border: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceGlassV1 {
        pub enabled: bool,
        pub blur_radius: f64,
        pub opacity_primary: f64,
        pub opacity_secondary: f64,
        pub edge_light: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceBackgroundV1 {
        pub has_image: bool,
        pub image_opacity: f64,
        pub image_blur: f64,
        pub overlay_color: Option<String>,
        pub overlay_opacity: f64,
        #[serde(default = "default_appearance_background_fit")]
        pub fit: String,
        #[serde(default = "default_appearance_background_treatment")]
        pub treatment: String,
    }

    fn default_appearance_background_fit() -> String {
        "cover".into()
    }

    fn default_appearance_background_treatment() -> String {
        "clear".into()
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceBordersV1 {
        pub card_radius: f64,
        pub input_radius: f64,
        pub border_opacity: f64,
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceTypographyV1 {
        pub font_family: String,
        pub mono_family: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
    pub struct AppearanceSnapshotV1 {
        pub revision: String,
        pub theme_id: String,
        pub name: String,
        pub is_dark: bool,
        pub colors: AppearanceColorsV1,
        pub glass: AppearanceGlassV1,
        pub background: AppearanceBackgroundV1,
        pub borders: AppearanceBordersV1,
        pub typography: AppearanceTypographyV1,
    }

    impl Default for AppearanceSnapshotV1 {
        fn default() -> Self {
            Self {
                revision: "builtin-mono-dark".into(),
                theme_id: "mono-dark".into(),
                name: "Mono Dark".into(),
                is_dark: true,
                colors: AppearanceColorsV1 {
                    primary_text: "#ededf0".into(),
                    secondary_text: "#a0a0a8".into(),
                    tertiary_text: "#71717a".into(),
                    placeholder_text: "#71717a".into(),
                    bg_primary: "#0d0d0f".into(),
                    bg_secondary: "#161618".into(),
                    bg_tertiary: "#1f1f23".into(),
                    sidebar_bg: "#0a0a0c".into(),
                    accent: "#ededf0".into(),
                    accent_light: "#c8c8d0".into(),
                    border_primary: "#262629".into(),
                    border_secondary: "#323237".into(),
                    focus_border: "#55555e".into(),
                    success: "#34d399".into(),
                    warning: "#fbbf24".into(),
                    error: "#f87171".into(),
                    info: "#a0a0a8".into(),
                    card_bg: "#161618".into(),
                    card_border: "#262629".into(),
                    input_bg: "#161618".into(),
                    input_border: "#323237".into(),
                },
                glass: AppearanceGlassV1 {
                    enabled: false,
                    blur_radius: 24.0,
                    opacity_primary: 1.0,
                    opacity_secondary: 1.0,
                    edge_light: "rgba(255,255,255,0.08)".into(),
                },
                background: AppearanceBackgroundV1 {
                    has_image: false,
                    image_opacity: 1.0,
                    image_blur: 0.0,
                    overlay_color: None,
                    overlay_opacity: 0.0,
                    fit: default_appearance_background_fit(),
                    treatment: default_appearance_background_treatment(),
                },
                borders: AppearanceBordersV1 {
                    card_radius: 12.0,
                    input_radius: 10.0,
                    border_opacity: 1.0,
                },
                typography: AppearanceTypographyV1 {
                    font_family: "system-ui, sans-serif".into(),
                    mono_family: "ui-monospace, monospace".into(),
                },
            }
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlBootstrapV1 {
        pub protocol: ControlProtocolRangeV1,
        pub host_id: String,
        pub host_name: String,
        pub capabilities: ControlCapabilitiesV1,
        #[serde(default)]
        pub appearance: AppearanceSnapshotV1,
        pub threads: Vec<ThreadSummaryV1>,
        pub models: Vec<Value>,
        pub agents: Vec<AgentSummaryV1>,
        pub active_runs: Vec<RunSnapshotV1>,
        pub queued_turns: Vec<QueuedTurnV1>,
        pub pending_approvals: Vec<PendingApprovalV1>,
    }

    pub(crate) struct AppearanceBackgroundAsset {
        pub revision: String,
        pub mime: &'static str,
        pub bytes: Vec<u8>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct TimelineItemV1 {
        pub id: String,
        pub thread_id: String,
        pub epoch: String,
        pub seq: u64,
        pub run_id: Option<String>,
        #[serde(rename = "type")]
        pub item_type: String,
        pub data: Value,
        pub created_at_ms: i64,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct TimelinePageV1 {
        pub thread_id: String,
        pub epoch: String,
        pub first_seq: Option<u64>,
        pub last_seq: Option<u64>,
        pub has_older: bool,
        pub has_newer: bool,
        pub before_seq: Option<u64>,
        pub after_seq: Option<u64>,
        pub items: Vec<TimelineItemV1>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlEventV1 {
        pub event_id: String,
        pub host_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub epoch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub seq: Option<u64>,
        #[serde(rename = "type")]
        pub event_type: String,
        pub data: Value,
    }

    #[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
    pub enum ControlCommandStatusV1 {
        #[serde(rename = "applied")]
        Applied,
        #[serde(rename = "accepted")]
        Accepted,
        #[serde(rename = "queued")]
        Queued,
        #[serde(rename = "needs_confirmation")]
        NeedsConfirmation,
        #[serde(rename = "conflict")]
        Conflict,
        #[serde(rename = "failed")]
        Failed,
    }

    #[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
    pub enum ControlCommandKindV1 {
        #[serde(rename = "thread.create")]
        ThreadCreate,
        #[serde(rename = "thread.rename")]
        ThreadRename,
        #[serde(rename = "thread.archive")]
        ThreadArchive,
        #[serde(rename = "thread.delete")]
        ThreadDelete,
        #[serde(rename = "thread.set_model")]
        ThreadSetModel,
        #[serde(rename = "thread.set_agent")]
        ThreadSetAgent,
        #[serde(rename = "message.delete")]
        MessageDelete,
        #[serde(rename = "turn.send")]
        TurnSend,
        #[serde(rename = "turn.stop")]
        TurnStop,
        #[serde(rename = "turn.regenerate")]
        TurnRegenerate,
        #[serde(rename = "turn.queue_resume")]
        TurnQueueResume,
        #[serde(rename = "turn.queue_delete")]
        TurnQueueDelete,
        #[serde(rename = "approval.resolve")]
        ApprovalResolve,
        #[serde(rename = "worker.start")]
        WorkerStart,
        #[serde(rename = "worker.continue_solo")]
        WorkerContinueSolo,
        #[serde(rename = "worker.stop")]
        WorkerStop,
    }

    impl ControlCommandKindV1 {
        fn as_str(self) -> &'static str {
            match self {
                Self::ThreadCreate => "thread.create",
                Self::ThreadRename => "thread.rename",
                Self::ThreadArchive => "thread.archive",
                Self::ThreadDelete => "thread.delete",
                Self::ThreadSetModel => "thread.set_model",
                Self::ThreadSetAgent => "thread.set_agent",
                Self::MessageDelete => "message.delete",
                Self::TurnSend => "turn.send",
                Self::TurnStop => "turn.stop",
                Self::TurnRegenerate => "turn.regenerate",
                Self::TurnQueueResume => "turn.queue_resume",
                Self::TurnQueueDelete => "turn.queue_delete",
                Self::ApprovalResolve => "approval.resolve",
                Self::WorkerStart => "worker.start",
                Self::WorkerContinueSolo => "worker.continue_solo",
                Self::WorkerStop => "worker.stop",
            }
        }

        fn destructive(self) -> bool {
            matches!(self, Self::ThreadDelete | Self::MessageDelete)
        }
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlCommandV1 {
        pub command_id: String,
        pub kind: ControlCommandKindV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub expected_revision: Option<u64>,
        #[serde(default)]
        pub payload: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub confirmation_token: Option<String>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct ControlCommandResultV1 {
        pub command_id: String,
        pub status: ControlCommandStatusV1,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub thread_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub revision: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub run_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub queue_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub confirmation_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub message: Option<String>,
        #[serde(default)]
        pub data: Value,
    }
}

pub(crate) struct AppearanceBackgroundAsset {
    pub revision: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
struct TurnSendPayloadV1 {
    text: String,
    #[serde(default)]
    display_text: Option<String>,
    #[serde(default)]
    attachments: Vec<ControlAttachmentV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AcceptedTurnV1 {
    text: String,
    #[serde(default)]
    display_text: Option<String>,
    config: FrozenRunConfigV1,
    #[serde(default = "control_default_true")]
    append_user: bool,
    #[serde(default)]
    mailbox_origin: Option<MailboxOriginV1>,
}

struct ActiveRun {
    run_id: String,
    steering: bool,
    stop: watch::Sender<bool>,
}

struct RunJournal {
    store: Arc<UserDataStore>,
    privacy: Arc<crate::privacy::PrivacyGate>,
    privacy_mode: crate::privacy::PrivacyMode,
    thread_id: String,
    run_id: String,
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
    fn append_event(&self, step: usize, event_type: &str, data: Value) -> Result<()> {
        self.store.control_append_run_event(
            &self.run_id,
            &Uuid::new_v4().to_string(),
            Some(&format!("step-{step}")),
            event_type,
            &data.to_string(),
        )?;
        Ok(())
    }

    fn put_artifact(&self, kind: &str, data: &Value) -> Result<String> {
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

    fn privacy_processed_text(&self, text: &str) -> Result<String> {
        ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        }
        .resolve_text(text)
    }

    fn privacy_processed_value(&self, value: &Value) -> Result<Value> {
        ModelInputResolver {
            privacy: &self.privacy,
            privacy_mode: self.privacy_mode,
        }
        .resolve_value(value)
    }

    fn artifact_value(&self, digest: &str) -> Result<Value> {
        let artifact = self
            .store
            .control_run_artifacts(&self.run_id)?
            .into_iter()
            .find(|artifact| artifact.digest == digest)
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
        let mut events = Vec::new();
        let mut after_seq = None;
        loop {
            let page = self
                .store
                .control_run_events(&self.run_id, after_seq, 500)?;
            let page_len = page.len();
            after_seq = page.last().map(|event| event.seq);
            events.extend(page);
            if page_len < 500 {
                break;
            }
        }
        let previous = events
            .iter()
            .filter(|event| event.step_id.as_deref() == Some(previous_step_id.as_str()))
            .collect::<Vec<_>>();
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
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: response
                .get("content")
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
                .map(|content| milim_core::api::openai::Content::Text(content.to_string())),
            name: None,
            tool_calls: Some(tool_calls),
            tool_call_id: None,
            reasoning_content: response
                .get("reasoning")
                .and_then(Value::as_str)
                .filter(|reasoning| !reasoning.is_empty())
                .map(str::to_string),
        });
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
        // all text and JSON above is rebuilt from SQLite.
        if let Some(last_tool_call) = memory_cache
            .iter()
            .rposition(|message| message.role == "assistant" && message.tool_calls.is_some())
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

    fn commit_failure(&self, step: usize, error: &Error) -> Result<()> {
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

struct ModelInputResolver<'a> {
    privacy: &'a crate::privacy::PrivacyGate,
    privacy_mode: crate::privacy::PrivacyMode,
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
    fn commit_composition(&self, accepted: &AcceptedTurnV1) -> Result<()> {
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
        let composition = json!({
            "visibility": visibility,
            "adapter": accepted.config.adapter,
            "model": accepted.config.model,
            "reasoning_effort": accepted.config.reasoning_effort,
            "generation": accepted.config.generation,
            "native_session_boundary": accepted.config.native_session_id,
            "workspace": accepted.config.workspace,
            "environment_policy": environment_policy,
            "explicit_environment_grants": [],
            "prompt_sections": [
                {
                    "kind": "instructions",
                    "provenance": "frozen_run_config",
                    "content": self.privacy_processed_text(&accepted.config.instructions)?,
                },
                {
                    "kind": "user",
                    "provenance": "accepted_turn",
                    "content": self.privacy_processed_text(&accepted.text)?,
                }
            ],
            "tools": accepted.config.enabled_tools.iter().map(|name| json!({
                "name": name,
                "provenance": "frozen_run_config",
            })).collect::<Vec<_>>(),
            "policies": {
                "privacy": accepted.config.privacy,
                "approval": accepted.config.approval_mode,
                "tool_mode": accepted.config.tool_mode,
                "plan_mode": accepted.config.plan_mode,
                "sandbox": accepted.config.sandbox,
                "computer_use": accepted.config.computer_use,
                "delegation": accepted.config.delegation_policy,
            },
            "attachments": attachments,
        });
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
                    messages.push(ChatMessage::text("user", accepted.text.clone()));
                    let message = json!({
                        "id": Uuid::new_v4().to_string(),
                        "role": "user",
                        "content": accepted.display_text.as_deref().unwrap_or(&accepted.text),
                        "promptContent": accepted.text,
                        "attachments": accepted.config.attachments,
                        "runId": self.run_id,
                        "steering": true,
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
}

struct ConfirmationGrant {
    token: String,
    expires_at: Instant,
}

#[derive(Clone)]
pub(crate) struct SocketTicket {
    pub device_key: Option<String>,
    pub expires_at: Instant,
}

pub struct RunManager {
    store: Arc<UserDataStore>,
    host: RwLock<ControlHostRecord>,
    active: Mutex<HashMap<String, ActiveRun>>,
    command_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    thread_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    confirmations: Mutex<HashMap<String, ConfirmationGrant>>,
    socket_tickets: Mutex<HashMap<String, SocketTicket>>,
    events: broadcast::Sender<ControlEventV1>,
}

impl RunManager {
    pub fn new(store: Arc<UserDataStore>, display_name: impl AsRef<str>) -> Result<Arc<Self>> {
        let host = store
            .ensure_control_host(&format!("host-{}", Uuid::new_v4()), display_name.as_ref())?;
        store.reconcile_control_startup()?;
        let (events, _) = broadcast::channel(CONTROL_EVENT_CAPACITY);
        let manager = Arc::new(Self {
            store,
            host: RwLock::new(host),
            active: Mutex::new(HashMap::new()),
            command_locks: Mutex::new(HashMap::new()),
            thread_locks: Mutex::new(HashMap::new()),
            confirmations: Mutex::new(HashMap::new()),
            socket_tickets: Mutex::new(HashMap::new()),
            events,
        });
        manager.backfill_message_timelines()?;
        Ok(manager)
    }

    fn backfill_message_timelines(&self) -> Result<usize> {
        let mut seeded = 0;
        for thread in self.store.control_threads()? {
            let session: Value = serde_json::from_str(&thread.session_json).unwrap_or(Value::Null);
            let base_timestamp = session
                .get("createdAt")
                .or_else(|| session.get("created_at_ms"))
                .and_then(Value::as_i64)
                .unwrap_or(thread.updated_at_ms);
            let messages = self
                .store
                .control_messages(&thread.id)?
                .into_iter()
                .enumerate()
                .filter_map(|(index, raw)| {
                    history_timeline_message(&thread.id, index, &raw, base_timestamp)
                })
                .collect::<Vec<_>>();
            seeded += self
                .store
                .control_seed_message_timeline_if_empty(&thread.id, &messages)?;
        }
        Ok(seeded)
    }

    pub fn host(&self) -> ControlHostRecord {
        self.host
            .read()
            .expect("control host store poisoned")
            .clone()
    }

    pub fn refresh_host(&self) -> Result<ControlHostRecord> {
        let current = self.host();
        let restored = self
            .store
            .ensure_control_host(&current.host_id, &current.display_name)?;
        *self.host.write().expect("control host store poisoned") = restored.clone();
        Ok(restored)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ControlEventV1> {
        self.events.subscribe()
    }

    pub fn appearance_snapshot(&self) -> AppearanceSnapshotV1 {
        self.store
            .get_json(APPEARANCE_STATE_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default()
    }

    pub fn publish_appearance(&self) {
        self.emit(
            "appearance.updated",
            None,
            None,
            None,
            json!({ "appearance": self.appearance_snapshot() }),
        );
    }

    pub fn publish_model_catalog(&self) {
        self.emit("models.updated", None, None, None, json!({}));
    }

    pub fn model_favorites(&self) -> Vec<String> {
        self.store
            .get_json(MODEL_FAVORITES_SETTINGS_KEY)
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str::<Value>(&value).ok())
            .and_then(|value| value.get("state")?.get("favorites").cloned())
            .and_then(|value| value.as_array().cloned())
            .map(|values| normalized_model_favorite_ids(&values, false).unwrap_or_default())
            .unwrap_or_default()
    }

    pub fn publish_model_favorites(&self) {
        self.emit(
            MODEL_FAVORITES_EVENT_TYPE,
            None,
            None,
            None,
            json!({ "favorite_model_ids": self.model_favorites() }),
        );
    }

    pub(crate) fn appearance_background_asset(&self) -> Option<AppearanceBackgroundAsset> {
        let appearance = self.appearance_snapshot();
        if !appearance.background.has_image {
            return None;
        }
        let themes = self
            .store
            .get_json(CUSTOM_THEMES_STATE_KEY)
            .ok()
            .flatten()?;
        let themes: Value = serde_json::from_str(&themes).ok()?;
        let source = themes
            .as_array()?
            .iter()
            .find(|theme| theme.get("id").and_then(Value::as_str) == Some(&appearance.theme_id))?
            .get("background")?
            .get("image")?
            .as_str()?;
        let (mime, bytes) = decode_appearance_background(source)?;
        Some(AppearanceBackgroundAsset {
            revision: appearance.revision,
            mime,
            bytes,
        })
    }

    pub fn owns_approval(&self, approval_id: &str) -> bool {
        self.store
            .control_approval(approval_id)
            .ok()
            .flatten()
            .is_some()
    }

    pub fn issue_socket_ticket(&self, device_key: Option<String>) -> (String, u64) {
        let ticket = Uuid::new_v4().to_string();
        let ttl = Duration::from_secs(30);
        let mut tickets = self
            .socket_tickets
            .lock()
            .expect("control socket ticket store poisoned");
        tickets.retain(|_, value| value.expires_at > Instant::now());
        tickets.insert(
            ticket.clone(),
            SocketTicket {
                device_key,
                expires_at: Instant::now() + ttl,
            },
        );
        (ticket, ttl.as_secs())
    }

    pub(crate) fn take_socket_ticket(&self, ticket: &str) -> Option<SocketTicket> {
        let mut tickets = self
            .socket_tickets
            .lock()
            .expect("control socket ticket store poisoned");
        tickets.retain(|_, value| value.expires_at > Instant::now());
        tickets.remove(ticket)
    }

    pub async fn bootstrap(&self, state: &AppState) -> Result<ControlBootstrapV1> {
        let threads = self.store.control_threads()?;
        let queued = self.store.control_queued_turns(None)?;
        let queued_counts = queued
            .iter()
            .fold(HashMap::<String, usize>::new(), |mut map, item| {
                *map.entry(item.thread_id.clone()).or_default() += 1;
                map
            });
        let thread_summaries = {
            let active = self
                .active
                .lock()
                .expect("control active run store poisoned");
            threads
                .iter()
                .map(|thread| {
                    thread_summary(
                        thread,
                        active.contains_key(&thread.id),
                        *queued_counts.get(&thread.id).unwrap_or(&0),
                    )
                })
                .collect::<Result<Vec<_>>>()?
        };
        // A temporarily unavailable provider must not prevent a controller
        // from opening existing threads, stopping work, or resolving an
        // approval. Model discovery can recover on the next bootstrap.
        let models = state
            .service
            .list_models()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| Error::Other(format!("serialize models: {error}")))?;
        let agents = state
            .agents
            .as_ref()
            .map(|store| store.list())
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|agent| AgentSummaryV1 {
                id: agent.id,
                name: agent.name,
                description: agent.description,
                avatar: agent.avatar,
                tool_mode: agent.tool_mode,
                enabled_tool_count: agent.enabled_tools.len(),
                skill_mode: agent.skill_mode,
                enabled_skill_count: agent.enabled_skills.len(),
            })
            .collect();
        let active_runs = self
            .store
            .control_runs(true)?
            .into_iter()
            .map(run_snapshot)
            .collect::<Result<Vec<_>>>()?;
        let queued_turns = queued
            .into_iter()
            .map(queued_turn)
            .collect::<Result<Vec<_>>>()?;
        let pending_inputs = self
            .store
            .control_pending_inbox(None)?
            .into_iter()
            .filter(|item| item.kind != "followup")
            .map(pending_input)
            .collect();
        let pending_approvals = self
            .store
            .control_pending_approvals()?
            .into_iter()
            .map(pending_approval)
            .collect::<Result<Vec<_>>>()?;
        Ok(ControlBootstrapV1 {
            protocol: ControlProtocolRangeV1 {
                min: CONTROL_PROTOCOL_MIN,
                max: CONTROL_PROTOCOL_MAX,
            },
            host_id: self.host().host_id,
            host_name: self.host().display_name,
            capabilities: ControlCapabilitiesV1::default(),
            appearance: self.appearance_snapshot(),
            threads: thread_summaries,
            models,
            favorite_model_ids: self.model_favorites(),
            agents,
            active_runs,
            queued_turns,
            pending_inputs,
            pending_approvals,
        })
    }

    pub fn timeline_page(
        &self,
        thread_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        tail: bool,
        limit: usize,
    ) -> Result<Option<TimelinePageV1>> {
        self.store
            .control_timeline_page(thread_id, after_seq, before_seq, tail, limit)?
            .map(|page| {
                let items = page
                    .items
                    .into_iter()
                    .map(timeline_item)
                    .collect::<Result<Vec<_>>>()?;
                Ok(TimelinePageV1 {
                    thread_id: thread_id.to_string(),
                    epoch: page.epoch,
                    first_seq: page.first_seq,
                    last_seq: page.last_seq,
                    has_older: page.has_older,
                    has_newer: page.has_newer,
                    before_seq: page.has_older.then_some(page.first_seq).flatten(),
                    after_seq: page.has_newer.then_some(page.last_seq).flatten(),
                    items,
                })
            })
            .transpose()
    }

    pub fn run_inspection(&self, run_id: &str) -> Result<Option<RunInspectionV1>> {
        let Some(run) = self.store.control_run(run_id)? else {
            return Ok(None);
        };
        let composition = self
            .store
            .control_run_artifacts(run_id)?
            .into_iter()
            .find(|artifact| artifact.kind == "run_composition")
            .map(|artifact| {
                serde_json::from_str::<ResolvedRunCompositionV1>(&artifact.data_json).map_err(
                    |error| Error::Other(format!("stored run composition is invalid: {error}")),
                )
            })
            .transpose()?;
        Ok(Some(RunInspectionV1 {
            run: run_snapshot(run)?,
            composition,
        }))
    }

    pub fn run_event_page(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Option<RunEventPageV1>> {
        if self.store.control_run(run_id)?.is_none() {
            return Ok(None);
        }
        let limit = limit.clamp(1, 200);
        let mut records =
            self.store
                .control_run_events(run_id, after_seq, limit.saturating_add(1))?;
        let has_more = records.len() > limit;
        records.truncate(limit);
        let artifacts = self
            .store
            .control_run_artifacts(run_id)?
            .into_iter()
            .map(|artifact| (artifact.digest, artifact.data_json))
            .collect::<HashMap<_, _>>();
        let events = records
            .into_iter()
            .map(|record| {
                let mut data = parse_value(&record.data_json)?;
                if let Some(object) = data.as_object_mut() {
                    if let Some(artifact) = object
                        .get("artifact_digest")
                        .and_then(Value::as_str)
                        .and_then(|digest| artifacts.get(digest))
                    {
                        object.insert("artifact".into(), parse_value(artifact)?);
                    }
                }
                Ok(RunEventV1 {
                    id: record.event_id,
                    run_id: record.run_id,
                    seq: record.seq,
                    step_id: record.step_id,
                    event_type: record.event_type,
                    data,
                    created_at_ms: record.created_at_ms,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_seq = has_more
            .then(|| events.last().map(|event| event.seq))
            .flatten();
        Ok(Some(RunEventPageV1 {
            run_id: run_id.to_string(),
            after_seq,
            next_seq,
            has_more,
            events,
        }))
    }

    pub async fn command(
        self: &Arc<Self>,
        state: AppState,
        device_id: Option<String>,
        command: ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        validate_command_id(&command.command_id)?;
        let command_lock = self.lock_for_command(&command.command_id);
        let _command_guard = command_lock.lock().await;
        if let Some(receipt) = self.store.control_command_receipt(&command.command_id)? {
            return serde_json::from_str(&receipt.result_json).map_err(|error| {
                Error::Other(format!("stored control command result is invalid: {error}"))
            });
        }
        if command.kind.destructive() && !self.consume_confirmation(&command) {
            return Ok(self.confirmation_result(&command));
        }
        let thread_lock = if matches!(
            command.kind,
            ControlCommandKindV1::TurnSend
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
        self.store
            .control_put_command_receipt(&ControlCommandReceiptRecord {
                command_id: command.command_id,
                device_id,
                thread_id: result.thread_id.clone(),
                command_kind: command.kind.as_str().to_string(),
                request_json,
                result_json,
                created_at_ms: now_ms(),
            })?;
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

    fn create_thread(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let id = command
            .payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if let Some(thread) = self.store.control_thread(&id)? {
            return Ok(ControlCommandResultV1 {
                command_id: command.command_id.clone(),
                status: ControlCommandStatusV1::Applied,
                thread_id: Some(thread.id),
                revision: Some(thread.revision),
                run_id: None,
                queue_id: None,
                confirmation_token: None,
                message: None,
                data: Value::Null,
            });
        }
        let title = command
            .payload
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New chat");
        let now = now_ms();
        let mut session = json!({
            "id": id,
            "title": title,
            "createdAt": now,
            "updatedAt": now,
            "settings": command.payload.get("settings").cloned().unwrap_or_else(|| json!({}))
        });
        if let Some(project) = command.payload.get("project") {
            session["project"] = project.clone();
        }
        let thread = self.store.control_create_thread(
            &id,
            &session.to_string(),
            &Uuid::new_v4().to_string(),
        )?;
        self.emit_thread_changed(&thread, "thread.created");
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread.id),
            revision: Some(thread.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    fn set_model_favorites(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let values = command
            .payload
            .get("favorite_model_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::InvalidRequest("payload.favorite_model_ids must be an array".into())
            })?;
        let favorite_model_ids = normalized_model_favorite_ids(values, true)?;
        let mut root = self
            .store
            .get_json(MODEL_FAVORITES_SETTINGS_KEY)?
            .map(|value| {
                serde_json::from_str::<Value>(&value)
                    .map_err(|error| Error::Other(format!("invalid stored settings JSON: {error}")))
            })
            .transpose()?
            .unwrap_or_else(|| json!({ "state": {}, "version": 0 }));
        let state = root
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored settings are not an object".into()))?
            .entry("state")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored settings state is not an object".into()))?;
        state.insert("favorites".into(), json!(favorite_model_ids));
        self.store
            .set_json(MODEL_FAVORITES_SETTINGS_KEY, &root.to_string())?;
        self.publish_model_favorites();
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: None,
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "favorite_model_ids": favorite_model_ids }),
        })
    }

    fn patch_thread(
        &self,
        command: &ControlCommandV1,
        patch: ThreadPatch,
    ) -> Result<ControlCommandResultV1> {
        let id = required_thread_id(command)?;
        let current = self
            .store
            .control_thread(id)?
            .ok_or_else(|| Error::NotFound(format!("thread {id}")))?;
        if let Some(expected) = command.expected_revision {
            if expected != current.revision {
                return Err(Error::InvalidRequest(format!(
                    "thread revision conflict: expected {expected}, current {}",
                    current.revision
                )));
            }
        }
        let mut value: Value = serde_json::from_str(&current.session_json)
            .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored thread is not an object".into()))?;
        match patch {
            ThreadPatch::Rename => {
                let title = required_payload_string(&command.payload, "title")?;
                object.insert("title".into(), Value::String(title));
            }
            ThreadPatch::Archive => {
                let archived = command
                    .payload
                    .get("archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if archived {
                    object.insert("archivedAt".into(), Value::from(now_ms()));
                } else {
                    object.remove("archivedAt");
                }
            }
            ThreadPatch::Model => {
                let model = command
                    .payload
                    .get("model")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::InvalidRequest("payload.model must be a string".into()))?
                    .trim()
                    .to_string();
                let reasoning_effort = match command.payload.get("reasoning_effort") {
                    Some(value) => {
                        let raw = value.as_str().ok_or_else(|| {
                            Error::InvalidRequest(
                                "payload.reasoning_effort must be a supported string".into(),
                            )
                        })?;
                        Some(parse_reasoning_effort(raw).ok_or_else(|| {
                            Error::InvalidRequest(format!(
                                "unsupported payload.reasoning_effort: {raw}"
                            ))
                        })?)
                    }
                    None => None,
                };
                let settings = settings_object(object)?;
                settings.insert("model".into(), Value::String(model.clone()));
                if !model.is_empty() {
                    if let Some(reasoning_effort) = reasoning_effort {
                        let overrides = settings
                            .entry("reasoningEffortOverrides")
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                            .ok_or_else(|| {
                                Error::Other(
                                    "stored reasoning effort overrides are not an object".into(),
                                )
                            })?;
                        overrides
                            .insert(model, Value::String(reasoning_effort.as_str().to_string()));
                    }
                }
            }
            ThreadPatch::Agent => {
                let agent = command
                    .payload
                    .get("agent_id")
                    .cloned()
                    .unwrap_or(Value::Null);
                if !agent.is_null() && !agent.is_string() {
                    return Err(Error::InvalidRequest(
                        "agent_id must be a string or null".into(),
                    ));
                }
                settings_object(object)?.insert("activeAgentId".into(), agent);
            }
        }
        object.insert("updatedAt".into(), Value::from(now_ms()));
        let updated = self
            .store
            .control_update_thread(id, &value.to_string(), command.expected_revision)?
            .ok_or_else(|| Error::NotFound(format!("thread {id}")))?;
        self.emit_thread_changed(&updated, "thread.updated");
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(updated.id),
            revision: Some(updated.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    fn delete_thread(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let id = required_thread_id(command)?;
        if self
            .active
            .lock()
            .expect("control active run store poisoned")
            .contains_key(id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before deleting this thread".into(),
            ));
        }
        if !self.store.control_delete_thread(id)? {
            return Err(Error::NotFound(format!("thread {id}")));
        }
        self.emit(
            "thread.deleted",
            Some(id),
            None,
            None,
            json!({ "thread_id": id }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(id.to_string()),
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    fn delete_message(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?;
        if self
            .active
            .lock()
            .expect("control active run store poisoned")
            .contains_key(thread_id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before deleting a message".into(),
            ));
        }
        let message_id = required_payload_string(&command.payload, "message_id")?;
        if !self.store.control_delete_message(thread_id, &message_id)? {
            return Err(Error::NotFound(format!("message {message_id}")));
        }
        self.persist_and_emit(
            thread_id,
            None,
            "message_deleted",
            json!({ "message_id": message_id }),
        )?;
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id.to_string()),
            revision: self
                .store
                .control_thread(thread_id)?
                .map(|thread| thread.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    async fn accept_turn(
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
        let config = resolve_frozen_config(&state, &thread, payload.attachments)?;
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
            display_text: payload.display_text,
            config,
            append_user: true,
            mailbox_origin: None,
        };
        let busy = self
            .active
            .lock()
            .expect("control active run store poisoned")
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
        let run_capabilities = self
            .store
            .control_run(&run_id)?
            .map(run_snapshot)
            .transpose()?
            .map(|run| run.capabilities);
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
            data: json!({ "capabilities": run_capabilities }),
        })
    }

    fn steer_turn(
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
                .expect("control active run store poisoned");
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
            display_text: payload.display_text,
            config: resolve_frozen_config(state, &thread, payload.attachments)?,
            append_user: true,
            mailbox_origin: None,
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

    fn inject_context(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
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

    fn delete_inbox_input(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
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

    async fn regenerate_turn(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
        delegation_policy: Option<&str>,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        if self
            .active
            .lock()
            .expect("control active run store poisoned")
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
        let mut config = resolve_frozen_config(&state, &thread, attachments)?;
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
            display_text,
            config,
            append_user: false,
            mailbox_origin: None,
        };
        let run_id = self.start_turn(state, thread_id.clone(), accepted)?;
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
            data: json!({ "regenerated": true }),
        })
    }

    fn start_turn(
        self: &Arc<Self>,
        state: AppState,
        thread_id: String,
        accepted: AcceptedTurnV1,
    ) -> Result<String> {
        let run_id = Uuid::new_v4().to_string();
        let (stop, stop_rx) = watch::channel(false);
        {
            let mut active = self
                .active
                .lock()
                .expect("control active run store poisoned");
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
                .expect("control active run store poisoned")
                .remove(&thread_id);
            run_record.status = "failed".into();
            run_record.updated_at_ms = now_ms();
            run_record.completed_at_ms = Some(run_record.updated_at_ms);
            run_record.error_json =
                Some(json!({ "code": error.code(), "message": error.to_string() }).to_string());
            let _ = self.store.control_put_run(&run_record);
            return Err(error);
        }
        if accepted.append_user {
            let user_message_id = Uuid::new_v4().to_string();
            let user_message = json!({
                "id": user_message_id,
                "role": "user",
                "content": accepted.display_text.as_deref().unwrap_or(&accepted.text),
                "promptContent": accepted.text,
                "attachments": accepted.config.attachments,
                "runId": run_id,
            });
            self.persist_message_and_event(
                &thread_id,
                &run_id,
                user_message,
                None,
                "accepted_input_projected",
                json!({"source": "turn.send"}),
            )?;
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

        let (status, error) = match outcome {
            Ok(RunOutcome::Completed) => ("completed", None),
            Ok(RunOutcome::Cancelled) => ("cancelled", None),
            Err(error) => (
                "failed",
                Some(json!({ "code": error.code(), "message": error.to_string() })),
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
        let _ = self.store.control_retarget_pending_steers(&run_id);
        self.active
            .lock()
            .expect("control active run store poisoned")
            .remove(&thread_id);
        if status != "cancelled" {
            self.drain_queue(state, thread_id);
        }
    }

    async fn run_mock(
        &self,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let response = format!("Echo: {}", accepted.text.trim());
        let mut content = String::new();
        for chunk in response.as_bytes().chunks(4) {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    let text = String::from_utf8_lossy(chunk).to_string();
                    content.push_str(&text);
                    self.persist_and_emit(thread_id, Some(run_id), "assistant_delta", json!({ "text": text }))?;
                }
            }
        }
        self.complete_assistant_message(thread_id, run_id, content, String::new(), None)?;
        Ok(RunOutcome::Completed)
    }

    async fn run_provider(
        &self,
        state: &AppState,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let messages = control_chat_messages(&self.store, thread_id)?;
        let context = RunContext::from_control(
            state,
            accepted.config.workspace.as_deref(),
            &accepted.config.privacy,
        )?;
        let service = service_for_run(state, &context);
        let reasoning_effort = accepted
            .config
            .reasoning_effort
            .as_deref()
            .and_then(parse_reasoning_effort);
        let request = CompletionRequest {
            model: accepted.config.model.clone(),
            messages,
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: sampling_from_generation(&accepted.config.generation),
            reasoning_effort,
        };
        let journal = RunJournal {
            store: self.store.clone(),
            privacy: state.privacy.clone(),
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
        };
        journal.commit_model_request(1, &request).await?;
        let mut stream = service.stream(request).await?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        let mut last_flush = Instant::now();
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(StreamEvent::Delta(delta))) => {
                            if let Some(text) = delta.content {
                                content.push_str(&text);
                                pending_text.push_str(&text);
                            }
                            if let Some(text) = delta.reasoning {
                                reasoning.push_str(&text);
                                pending_reasoning.push_str(&text);
                            }
                            if pending_text.len() + pending_reasoning.len() >= 256
                                || last_flush.elapsed() >= Duration::from_millis(250)
                            {
                                flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                                last_flush = Instant::now();
                            }
                        }
                        Some(Ok(StreamEvent::Done { finish_reason, usage })) => {
                            flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                            journal
                                .commit_model_response(
                                    1,
                                    &content,
                                    &reasoning,
                                    &[],
                                    &finish_reason,
                                    usage,
                                )
                                .await?;
                            self.complete_assistant_message(
                                thread_id,
                                run_id,
                                content,
                                reasoning,
                                Some(json!({ "finish_reason": finish_reason, "usage": usage })),
                            )?;
                            return Ok(RunOutcome::Completed);
                        }
                        Some(Err(error)) => return Err(error),
                        None => {
                            flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                            return Err(Error::Other("provider stream ended without a terminal event".into()));
                        }
                    }
                }
            }
        }
    }

    async fn run_agent(
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
                system_prompt: snapshot.system_prompt.clone(),
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
                system_prompt: accepted.config.instructions.clone(),
                model: String::new(),
                tool_mode: accepted.config.tool_mode.clone(),
                enabled_tools: accepted.config.enabled_tools.clone(),
                skill_mode: accepted.config.skill_mode.clone(),
                enabled_skills: accepted.config.enabled_skills.clone(),
                avatar: "sparkles".into(),
            });
        let messages = control_chat_messages(&self.store, thread_id)?;
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
            reasoning_effort,
            sampling_from_generation(&accepted.config.generation),
            journal.clone(),
        )?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        loop {
            let event = tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                        return Ok(RunOutcome::Cancelled);
                    }
                    None
                }
                event = tokio::time::timeout(Duration::from_millis(250), stream.next()) => {
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
                    content.push_str(text);
                    pending_text.push_str(text);
                    if pending_text.len() + pending_reasoning.len() >= 256 {
                        flush_deltas(
                            self,
                            thread_id,
                            run_id,
                            &mut pending_text,
                            &mut pending_reasoning,
                        )?;
                    }
                    continue;
                }
                milim_agents::AgentEvent::Reasoning { text } => {
                    reasoning.push_str(text);
                    pending_reasoning.push_str(text);
                    if pending_text.len() + pending_reasoning.len() >= 256 {
                        flush_deltas(
                            self,
                            thread_id,
                            run_id,
                            &mut pending_text,
                            &mut pending_reasoning,
                        )?;
                    }
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
                    flush_deltas(
                        self,
                        thread_id,
                        run_id,
                        &mut pending_text,
                        &mut pending_reasoning,
                    )?;
                    self.store.control_put_approval(&ControlApprovalRecord {
                        id: approval_id.clone(),
                        run_id: run_id.to_string(),
                        thread_id: thread_id.to_string(),
                        kind: "command".into(),
                        request_json: json!({
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
                        })
                        .to_string(),
                        status: "pending".into(),
                        decision_json: None,
                        created_at_ms: now_ms(),
                        resolved_at_ms: None,
                    })?;
                }
                milim_agents::AgentEvent::Done { .. } => {
                    flush_deltas(
                        self,
                        thread_id,
                        run_id,
                        &mut pending_text,
                        &mut pending_reasoning,
                    )?;
                    self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
                    self.complete_assistant_message(thread_id, run_id, content, reasoning, None)?;
                    return Ok(RunOutcome::Completed);
                }
                milim_agents::AgentEvent::Error { message } => {
                    flush_deltas(
                        self,
                        thread_id,
                        run_id,
                        &mut pending_text,
                        &mut pending_reasoning,
                    )?;
                    self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
                    return Err(Error::Other(message.clone()));
                }
                _ => {
                    flush_deltas(
                        self,
                        thread_id,
                        run_id,
                        &mut pending_text,
                        &mut pending_reasoning,
                    )?;
                }
            }
            self.persist_and_emit(thread_id, Some(run_id), &event_type, value)?;
        }
        Err(Error::Other(
            "Agent stream ended without a terminal event".into(),
        ))
    }

    async fn run_harness(
        &self,
        state: &AppState,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let messages = self.store.control_messages(thread_id)?;
        let transcript_prompt = messages
            .iter()
            .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
            .filter_map(|message| {
                let role = message.get("role")?.as_str()?;
                let content = message
                    .get("promptContent")
                    .or_else(|| message.get("content"))?
                    .as_str()?;
                Some(format!("{}:\n{}", uppercase_role(role), content))
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let prompt = if accepted.config.native_session_id.is_some() {
            // Once an account runtime owns the native history, replaying the
            // visible Milim transcript would duplicate context in that
            // session. The frozen current turn remains sufficient.
            accepted.text.clone()
        } else {
            transcript_prompt
        };
        let prompt = if accepted.config.instructions.trim().is_empty() {
            prompt
        } else {
            format!(
                "System instructions:\n{}\n\n{}",
                accepted.config.instructions.trim(),
                prompt
            )
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::HOST,
            HeaderValue::from_str(&format!("127.0.0.1:{}", state.config.port))
                .map_err(|error| Error::Other(format!("invalid control host header: {error}")))?,
        );
        let request = crate::routes::HarnessRunRequest {
            prompt,
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
            milim_context: Some(json!({
                "tool_context": {
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
        loop {
            let event = tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        return Ok(RunOutcome::Cancelled);
                    }
                    None
                }
                event = stream.next() => event,
            };
            let Some(event) = event else {
                break;
            };
            let value = serde_json::to_value(&event)
                .map_err(|error| Error::Other(format!("serialize harness event: {error}")))?;
            let event_type = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("runtime_notice");
            let mut timeline_type = event_type;
            let mut timeline_value = value.clone();
            match event_type {
                "text_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                        timeline_type = "assistant_delta";
                        timeline_value = json!({ "text": text, "reasoning": "" });
                    }
                }
                "reasoning_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        reasoning.push_str(text);
                        timeline_type = "assistant_delta";
                        timeline_value = json!({ "text": "", "reasoning": text });
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
                    }
                }
                _ => {}
            }
            if matches!(event_type, "approval_requested" | "approval_resolved") {
                journal.append_event(1, event_type, journal.privacy_processed_value(&value)?)?;
            }
            self.persist_and_emit(thread_id, Some(run_id), timeline_type, timeline_value)?;
            if event.is_terminal() {
                if event_type == "turn_completed" {
                    journal
                        .commit_model_response(
                            1,
                            &content,
                            &reasoning,
                            &[],
                            "stop",
                            Usage::default(),
                        )
                        .await?;
                    self.complete_assistant_message(thread_id, run_id, content, reasoning, None)?;
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

    fn complete_assistant_message(
        &self,
        thread_id: &str,
        run_id: &str,
        content: String,
        reasoning: String,
        metrics: Option<Value>,
    ) -> Result<()> {
        let message = json!({
            "id": Uuid::new_v4().to_string(),
            "role": "assistant",
            "content": content,
            "reasoning": reasoning,
            "runId": run_id,
            "ledgerVersion": 1,
            "metrics": metrics,
        });
        self.persist_message_and_event(
            thread_id,
            run_id,
            message,
            None,
            "assistant_message_projected",
            json!({"ledger_version": 1}),
        )?;
        Ok(())
    }

    fn stop_turn(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?;
        let active = self
            .active
            .lock()
            .expect("control active run store poisoned");
        let Some(run) = active.get(thread_id) else {
            return Err(Error::InvalidRequest("thread has no active turn".into()));
        };
        let _ = run.stop.send(true);
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id.to_string()),
            revision: self
                .store
                .control_thread(thread_id)?
                .map(|thread| thread.revision),
            run_id: Some(run.run_id.clone()),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({ "queued_turns_preserved": true }),
        })
    }

    async fn resume_queued_turn(
        self: &Arc<Self>,
        state: AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?.to_string();
        if self
            .active
            .lock()
            .expect("control active run store poisoned")
            .contains_key(&thread_id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before resuming a queued turn".into(),
            ));
        }
        let queue_id = required_payload_string(&command.payload, "queue_id")?;
        let queued = self
            .store
            .control_queued_turns(Some(&thread_id))?
            .into_iter()
            .find(|turn| turn.id == queue_id)
            .ok_or_else(|| Error::NotFound(format!("queued turn {queue_id}")))?;
        let accepted = serde_json::from_str::<AcceptedTurnV1>(&queued.request_json)
            .map_err(|error| Error::Other(format!("stored queued turn is invalid: {error}")))?;
        if !self.store.control_remove_queued_turn(&queue_id)? {
            return Err(Error::NotFound(format!("queued turn {queue_id}")));
        }
        let run_id = match self.start_turn(state, thread_id.clone(), accepted) {
            Ok(run_id) => run_id,
            Err(error) => {
                let _ = self.store.control_enqueue_turn(&queued);
                return Err(error);
            }
        };
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
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Accepted,
            thread_id: Some(thread_id),
            revision: None,
            run_id: Some(run_id),
            queue_id: Some(queue_id),
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    fn delete_queued_turn(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
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

    fn move_queued_turn(&self, command: &ControlCommandV1) -> Result<ControlCommandResultV1> {
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

    async fn resolve_approval(
        &self,
        state: &AppState,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let approval_id = required_payload_string(&command.payload, "approval_id")?;
        let decision = required_payload_string(&command.payload, "decision")?;
        let approved = match decision.as_str() {
            "approve" => true,
            "deny" => false,
            _ => {
                return Err(Error::InvalidRequest(
                    "payload.decision must be approve or deny".into(),
                ))
            }
        };
        let response = command.payload.get("response").cloned();
        let Some(mut durable) = self.store.control_approval(&approval_id)? else {
            return Err(Error::NotFound(format!("approval {approval_id}")));
        };
        let resolved = state
            .tool_approvals
            .resolve_with_response(&approval_id, approved, response);
        if resolved == milim_agents::ApprovalResolve::Conflict {
            return Ok(ControlCommandResultV1 {
                command_id: command.command_id.clone(),
                status: ControlCommandStatusV1::Conflict,
                thread_id: Some(durable.thread_id),
                revision: None,
                run_id: Some(durable.run_id),
                queue_id: None,
                confirmation_token: None,
                message: Some("approval was already resolved with a different decision".into()),
                data: Value::Null,
            });
        }
        if matches!(
            resolved,
            milim_agents::ApprovalResolve::Missing | milim_agents::ApprovalResolve::Failed
        ) {
            return Err(Error::InvalidRequest(
                "approval is no longer deliverable to its runtime".into(),
            ));
        }
        let snapshot = state
            .tool_approvals
            .wait_for_delivery(&approval_id, milim_agents::APPROVAL_DELIVERY_TIMEOUT)
            .await
            .ok_or_else(|| Error::NotFound(format!("approval {approval_id}")))?;
        if !matches!(
            snapshot.state,
            milim_agents::ApprovalState::Delivered | milim_agents::ApprovalState::Acknowledged
        ) {
            return Err(Error::Upstream(
                snapshot
                    .error
                    .unwrap_or_else(|| "approval delivery failed".into()),
            ));
        }
        durable.status = if approved { "approved" } else { "denied" }.into();
        durable.decision_json = Some(json!({ "decision": decision }).to_string());
        durable.resolved_at_ms = Some(now_ms());
        self.store.control_put_approval(&durable)?;
        self.persist_and_emit(
            &durable.thread_id,
            Some(&durable.run_id),
            "approval_resolved",
            json!({
                "approval_id": approval_id,
                "decision": decision,
                "status": snapshot.state,
            }),
        )?;
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(durable.thread_id),
            revision: None,
            run_id: Some(durable.run_id),
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: serde_json::to_value(snapshot)
                .map_err(|error| Error::Other(format!("serialize approval: {error}")))?,
        })
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

    fn drain_queue(self: &Arc<Self>, state: AppState, thread_id: String) {
        let Some(next) = self
            .store
            .control_queued_turns(Some(&thread_id))
            .ok()
            .and_then(|mut turns| (!turns.is_empty()).then(|| turns.remove(0)))
        else {
            return;
        };
        let Ok(accepted) = serde_json::from_str::<AcceptedTurnV1>(&next.request_json) else {
            let _ = self.store.control_discard_inbox(&next.id);
            return;
        };
        if self
            .store
            .control_remove_queued_turn(&next.id)
            .unwrap_or(false)
            && self.start_turn(state, thread_id, accepted).is_err()
        {
            let _ = self.store.control_enqueue_turn(&next);
        }
    }

    pub async fn shutdown(&self, timeout: Duration) {
        {
            let active = self
                .active
                .lock()
                .expect("control active run store poisoned");
            for run in active.values() {
                let _ = run.stop.send(true);
            }
        }
        let wait = async {
            loop {
                if self
                    .active
                    .lock()
                    .expect("control active run store poisoned")
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        if tokio::time::timeout(timeout, wait).await.is_err() {
            let _ = self.store.reconcile_control_startup();
        }
    }

    fn lock_for_command(&self, command_id: &str) -> Arc<AsyncMutex<()>> {
        self.command_locks
            .lock()
            .expect("control command lock store poisoned")
            .entry(command_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn lock_for_thread(&self, thread_id: &str) -> Arc<AsyncMutex<()>> {
        self.thread_locks
            .lock()
            .expect("control thread lock store poisoned")
            .entry(thread_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn persist_and_emit(
        &self,
        thread_id: &str,
        run_id: Option<&str>,
        item_type: &str,
        data: Value,
    ) -> Result<ControlTimelineRecord> {
        let record = self.store.control_append_timeline(
            thread_id,
            &Uuid::new_v4().to_string(),
            run_id,
            item_type,
            &data.to_string(),
        )?;
        self.emit(
            "timeline.appended",
            Some(thread_id),
            Some(&record.epoch),
            Some(record.seq),
            json!({ "item": timeline_item(record.clone())? }),
        );
        Ok(record)
    }

    fn persist_message_and_event(
        &self,
        thread_id: &str,
        run_id: &str,
        message: Value,
        step_id: Option<&str>,
        event_type: &str,
        event_data: Value,
    ) -> Result<ControlTimelineRecord> {
        let item_id = message
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("message projection is missing id".into()))?;
        let (record, _) = self.store.control_commit_message_projection_and_event(
            thread_id,
            run_id,
            item_id,
            &message.to_string(),
            &Uuid::new_v4().to_string(),
            step_id,
            event_type,
            &event_data.to_string(),
        )?;
        self.emit(
            "timeline.appended",
            Some(thread_id),
            Some(&record.epoch),
            Some(record.seq),
            json!({ "item": timeline_item(record.clone())? }),
        );
        Ok(record)
    }

    fn emit_thread_changed(&self, thread: &ControlThreadRecord, event_type: &str) {
        self.emit(
            event_type,
            Some(&thread.id),
            Some(&thread.epoch),
            None,
            json!({ "revision": thread.revision }),
        );
    }

    fn emit(
        &self,
        event_type: &str,
        thread_id: Option<&str>,
        epoch: Option<&str>,
        seq: Option<u64>,
        data: Value,
    ) {
        let _ = self.events.send(ControlEventV1 {
            event_id: Uuid::new_v4().to_string(),
            host_id: self.host().host_id,
            thread_id: thread_id.map(str::to_string),
            epoch: epoch.map(str::to_string),
            seq,
            event_type: event_type.to_string(),
            data,
        });
    }

    fn confirmation_result(&self, command: &ControlCommandV1) -> ControlCommandResultV1 {
        let mut confirmations = self
            .confirmations
            .lock()
            .expect("control confirmation store poisoned");
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
            .expect("control confirmation store poisoned");
        confirmations.retain(|_, grant| grant.expires_at > Instant::now());
        confirmations
            .remove(&command.command_id)
            .is_some_and(|grant| grant.token == provided)
    }
}

fn validate_control_attachments(attachments: &[ControlAttachmentV1]) -> Result<()> {
    if attachments.len() > MAX_CONTROL_ATTACHMENTS {
        return Err(Error::InvalidRequest(format!(
            "a turn may contain at most {MAX_CONTROL_ATTACHMENTS} attachments"
        )));
    }
    for attachment in attachments {
        if attachment.id.trim().is_empty() || attachment.name.trim().is_empty() {
            return Err(Error::InvalidRequest(
                "attachments require stable IDs and names".into(),
            ));
        }
        if attachment.name.chars().count() > MAX_CONTROL_ATTACHMENT_NAME_CHARS
            || attachment.mime.chars().count() > MAX_CONTROL_ATTACHMENT_MIME_CHARS
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} metadata is too long",
                attachment.name
            )));
        }
        if attachment.size > MAX_CONTROL_ATTACHMENT_BYTES {
            return Err(Error::InvalidRequest(format!(
                "attachment {} exceeds the 2 MiB limit",
                attachment.name
            )));
        }
        if attachment
            .content
            .as_ref()
            .is_some_and(|value| value.chars().count() > MAX_CONTROL_ATTACHMENT_CONTENT_CHARS)
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} text exceeds the 128 KiB limit",
                attachment.name
            )));
        }
        if attachment
            .data_url
            .as_ref()
            .is_some_and(|value| value.len() > MAX_CONTROL_ATTACHMENT_DATA_URL_CHARS)
        {
            return Err(Error::InvalidRequest(format!(
                "attachment {} image payload exceeds the wire limit",
                attachment.name
            )));
        }
        if attachment.content.is_none() && attachment.data_url.is_none() {
            return Err(Error::InvalidRequest(format!(
                "attachment {} has no content",
                attachment.name
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ThreadPatch {
    Rename,
    Archive,
    Model,
    Agent,
}

enum RunOutcome {
    Completed,
    Cancelled,
}

fn flush_deltas(
    manager: &RunManager,
    thread_id: &str,
    run_id: &str,
    text: &mut String,
    reasoning: &mut String,
) -> Result<()> {
    if text.is_empty() && reasoning.is_empty() {
        return Ok(());
    }
    manager.persist_and_emit(
        thread_id,
        Some(run_id),
        "assistant_delta",
        json!({ "text": std::mem::take(text), "reasoning": std::mem::take(reasoning) }),
    )?;
    Ok(())
}

fn resolve_frozen_config(
    state: &AppState,
    thread: &ControlThreadRecord,
    attachments: Vec<ControlAttachmentV1>,
) -> Result<FrozenRunConfigV1> {
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    let settings = value.get("settings").and_then(Value::as_object);
    let instructions = settings
        .and_then(|settings| settings.get("instructions"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let selected_model = value
        .get("worker")
        .and_then(Value::as_object)
        .and_then(|worker| worker.get("model"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            settings
                .and_then(|settings| settings.get("model"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| Error::InvalidRequest("thread has no selected model".into()))?
        .to_string();
    let workspace = settings
        .and_then(|settings| settings.get("folder"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let privacy = setting_string(settings, "privacy", "off");
    let approval_mode = setting_string(settings, "toolApproval", "review");
    let plan_mode = settings
        .and_then(|settings| settings.get("planMode"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let sandbox = settings
        .and_then(|settings| settings.get("sandbox"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let computer_use = settings
        .and_then(|settings| settings.get("computerUse"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let memory = settings
        .and_then(|settings| settings.get("memory"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let delegation_policy = setting_string(settings, "delegationPolicy", "ask");
    let worker_model = setting_string(settings, "workerModel", "");
    let agent_id = settings
        .and_then(|settings| settings.get("activeAgentId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let agent = agent_id
        .and_then(|id| {
            state
                .agents
                .as_ref()
                .and_then(|store| store.get(id).ok().flatten())
        })
        .map(|agent| AgentSnapshotV1 {
            id: agent.id,
            name: agent.name,
            description: agent.description,
            avatar: agent.avatar,
            system_prompt: agent.system_prompt,
            tool_mode: agent.tool_mode,
            enabled_tools: agent.enabled_tools,
            skill_mode: agent.skill_mode,
            enabled_skills: agent.enabled_skills,
        });
    let adapter = runtime_adapter(&selected_model).to_string();
    let model = runtime_model(&selected_model).to_string();
    let native_session_id = value
        .get("accountRuntime")
        .and_then(Value::as_object)
        .and_then(|runtime| match adapter.as_str() {
            "codex" => runtime.get("codexThreadId"),
            "claude" => runtime.get("claudeSessionId"),
            "opencode" => runtime.get("opencodeSessionId"),
            "pi" => runtime.get("piSessionId"),
            _ => None,
        })
        .and_then(Value::as_str)
        .map(str::to_string);
    let reasoning_effort = settings
        .and_then(|settings| settings.get("reasoningEffortOverrides"))
        .and_then(Value::as_object)
        .and_then(|overrides| overrides.get(&selected_model))
        .and_then(Value::as_str)
        .map(str::to_string);
    let generation = settings
        .and_then(|settings| settings.get("generationOverrides"))
        .and_then(Value::as_object)
        .and_then(|overrides| overrides.get(&selected_model))
        .map(normalize_generation_settings)
        .unwrap_or_default();
    let enabled_tools = agent
        .as_ref()
        .map(|agent| agent.enabled_tools.clone())
        .unwrap_or_default();
    let tool_mode = agent
        .as_ref()
        .map(|agent| agent.tool_mode.clone())
        .unwrap_or_else(default_control_tool_mode);
    let enabled_skills = agent
        .as_ref()
        .map(|agent| agent.enabled_skills.clone())
        .unwrap_or_default();
    let skill_mode = agent
        .as_ref()
        .map(|agent| agent.skill_mode.clone())
        .unwrap_or_else(default_control_skill_mode);
    Ok(FrozenRunConfigV1 {
        model,
        instructions,
        workspace,
        privacy,
        approval_mode,
        plan_mode,
        sandbox,
        computer_use,
        memory,
        delegation_policy,
        worker_model,
        agent,
        tool_mode,
        enabled_tools,
        skill_mode,
        enabled_skills,
        attachments,
        native_session_id,
        reasoning_effort,
        generation,
        adapter,
        linked_thread_grants: Vec::new(),
        claimed_mailbox_ids: Vec::new(),
    })
}

fn normalize_generation_settings(value: &Value) -> GenerationSettingsV1 {
    let value = value.as_object();
    let bounded_f32 = |key: &str, min: f64, max: f64, include_min: bool| {
        value
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
            .filter(|number| {
                number.is_finite()
                    && if include_min {
                        *number >= min
                    } else {
                        *number > min
                    }
                    && *number <= max
            })
            .map(|number| number as f32)
    };
    let stop = value
        .and_then(|value| value.get("stop"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty() && item.chars().count() <= 256)
        .take(8)
        .map(str::to_string)
        .collect();
    GenerationSettingsV1 {
        max_tokens: value
            .and_then(|value| value.get("maxTokens"))
            .and_then(Value::as_u64)
            .filter(|number| (1..=1_000_000).contains(number))
            .and_then(|number| u32::try_from(number).ok()),
        temperature: bounded_f32("temperature", 0.0, 2.0, true),
        top_p: bounded_f32("topP", 0.0, 1.0, false),
        seed: value
            .and_then(|value| value.get("seed"))
            .and_then(Value::as_i64),
        stop,
        frequency_penalty: bounded_f32("frequencyPenalty", -2.0, 2.0, true),
        presence_penalty: bounded_f32("presencePenalty", -2.0, 2.0, true),
        top_k: value
            .and_then(|value| value.get("topK"))
            .and_then(Value::as_i64)
            .filter(|number| *number == -1 || (1..=1_000_000).contains(number))
            .and_then(|number| i32::try_from(number).ok()),
        min_p: bounded_f32("minP", 0.0, 1.0, true),
        repetition_penalty: bounded_f32("repetitionPenalty", 0.0, 2.0, false),
        thinking_token_budget: value
            .and_then(|value| value.get("thinkingTokenBudget"))
            .and_then(Value::as_u64)
            .filter(|number| *number <= 1_000_000)
            .and_then(|number| u32::try_from(number).ok()),
    }
}

fn sampling_from_generation(generation: &GenerationSettingsV1) -> SamplingParams {
    SamplingParams {
        temperature: generation.temperature,
        top_p: generation.top_p,
        max_tokens: generation.max_tokens,
        stop: generation.stop.clone(),
        seed: generation.seed,
        frequency_penalty: generation.frequency_penalty,
        presence_penalty: generation.presence_penalty,
        top_k: generation.top_k,
        min_p: generation.min_p,
        repetition_penalty: generation.repetition_penalty,
        thinking_token_budget: generation.thinking_token_budget,
    }
}

fn runtime_adapter(model: &str) -> &str {
    let model = model.trim();
    if model.eq_ignore_ascii_case("mock-echo") {
        "mock"
    } else if model
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("codex:"))
    {
        "codex"
    } else if model
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("claude:"))
    {
        "claude"
    } else if model
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
    {
        "opencode"
    } else if model
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pi:"))
    {
        "pi"
    } else {
        "provider"
    }
}

fn runtime_model(model: &str) -> &str {
    match runtime_adapter(model) {
        "codex" => model.get(6..).unwrap_or(model).trim(),
        "claude" => model.get(7..).unwrap_or(model).trim(),
        "opencode" => model.get(9..).unwrap_or(model).trim(),
        "pi" => model.get(3..).unwrap_or(model).trim(),
        _ => model.trim(),
    }
}

fn thread_summary(
    thread: &ControlThreadRecord,
    busy: bool,
    queued_turns: usize,
) -> Result<ThreadSummaryV1> {
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    let settings = value.get("settings").and_then(Value::as_object);
    Ok(ThreadSummaryV1 {
        id: thread.id.clone(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("New chat")
            .to_string(),
        revision: thread.revision,
        epoch: thread.epoch.clone(),
        updated_at_ms: thread.updated_at_ms,
        archived_at_ms: value.get("archivedAt").and_then(Value::as_i64),
        model: settings
            .and_then(|settings| settings.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string),
        reasoning_effort_overrides: settings
            .and_then(|settings| settings.get("reasoningEffortOverrides"))
            .and_then(Value::as_object)
            .map(|overrides| {
                overrides
                    .iter()
                    .filter_map(|(model, effort)| {
                        let effort = effort.as_str()?;
                        parse_reasoning_effort(effort)?;
                        Some((model.clone(), effort.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        agent_id: settings
            .and_then(|settings| settings.get("activeAgentId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        workspace: settings
            .and_then(|settings| settings.get("folder"))
            .and_then(Value::as_str)
            .map(str::to_string),
        busy,
        queued_turns,
        linked_threads: Vec::new(),
    })
}

fn run_snapshot(run: ControlRunRecord) -> Result<RunSnapshotV1> {
    let accepted: AcceptedTurnV1 = serde_json::from_str(&run.request_json)
        .map_err(|error| Error::Other(format!("invalid stored run snapshot: {error}")))?;
    let visibility = if matches!(run.adapter.as_str(), "codex" | "claude" | "opencode" | "pi") {
        "harness_boundary"
    } else {
        "model_visible"
    };
    let steering = accepted.config.agent.is_some() || accepted.config.adapter == "provider";
    Ok(RunSnapshotV1 {
        id: run.id,
        thread_id: run.thread_id,
        status: run.status,
        adapter: run.adapter,
        config: accepted.config,
        capabilities: RunCapabilitiesV1 {
            ledger: true,
            inspectable: true,
            steering,
            visibility: visibility.into(),
        },
        created_at_ms: run.created_at_ms,
        updated_at_ms: run.updated_at_ms,
        completed_at_ms: run.completed_at_ms,
        error: run.error_json.as_deref().map(parse_value).transpose()?,
    })
}

fn pending_input(item: ControlInboxRecord) -> PendingInputV1 {
    PendingInputV1 {
        id: item.id,
        thread_id: item.thread_id,
        target_run_id: item.target_run_id,
        kind: item.kind,
        state: item.state,
        created_at_ms: item.created_at_ms,
    }
}

fn queued_turn(turn: ControlQueuedTurnRecord) -> Result<QueuedTurnV1> {
    let accepted = serde_json::from_str::<AcceptedTurnV1>(&turn.request_json)
        .map_err(|error| Error::Other(format!("stored queued turn is invalid: {error}")))?;
    Ok(QueuedTurnV1 {
        id: turn.id,
        thread_id: turn.thread_id,
        command_id: turn.command_id,
        accepted_at_ms: turn.accepted_at_ms,
        display_text: accepted.display_text.unwrap_or(accepted.text),
        attachments: accepted.config.attachments,
        mailbox_origin: accepted.mailbox_origin,
    })
}

fn pending_approval(approval: ControlApprovalRecord) -> Result<PendingApprovalV1> {
    Ok(PendingApprovalV1 {
        id: approval.id,
        run_id: approval.run_id,
        thread_id: approval.thread_id,
        kind: approval.kind,
        request: parse_value(&approval.request_json)?,
        status: approval.status,
        created_at_ms: approval.created_at_ms,
    })
}

fn timeline_item(record: ControlTimelineRecord) -> Result<TimelineItemV1> {
    Ok(TimelineItemV1 {
        id: record.item_id,
        thread_id: record.thread_id,
        epoch: record.epoch,
        seq: record.seq,
        run_id: record.run_id,
        item_type: record.item_type,
        data: parse_value(&record.data_json)?,
        created_at_ms: record.created_at_ms,
    })
}

fn history_timeline_message(
    thread_id: &str,
    index: usize,
    raw: &str,
    base_timestamp: i64,
) -> Option<(String, String, i64)> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let role = value.get("role")?.as_str()?.to_string();
    if !matches!(role.as_str(), "user" | "assistant" | "system") {
        return None;
    }
    let message_id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("legacy-{thread_id}-{index}"));
    let stream_parts = value
        .get("streamParts")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .or_else(|| value.get("promptContent").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| history_stream_text(stream_parts, "text"));
    let reasoning = value
        .get("reasoning")
        .or_else(|| value.get("reasoningContent"))
        .or_else(|| value.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| history_stream_text(stream_parts, "thinking"));
    let created_at_ms = value
        .get("createdAt")
        .or_else(|| value.get("created_at_ms"))
        .or_else(|| value.get("timestamp"))
        .and_then(Value::as_i64)
        .or_else(|| {
            value
                .get("metrics")
                .and_then(|metrics| metrics.get("startedAt"))
                .and_then(Value::as_i64)
        })
        .or_else(|| {
            value
                .get("run")
                .and_then(|run| run.get("startedAt"))
                .and_then(Value::as_i64)
        })
        .unwrap_or_else(|| base_timestamp.saturating_add(index as i64));
    let object = value.as_object_mut()?;
    object.insert("id".into(), Value::String(message_id));
    object.insert("role".into(), Value::String(role));
    object.insert("content".into(), Value::String(content));
    object.insert("reasoning".into(), Value::String(reasoning));
    Some((
        format!("history:{thread_id}:{index}"),
        Value::Object(object.clone()).to_string(),
        created_at_ms,
    ))
}

fn history_stream_text(parts: Option<&[Value]>, kind: &str) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some(kind))
        .filter_map(|part| part.get("content").and_then(Value::as_str))
        .collect::<String>()
}

fn parse_value(value: &str) -> Result<Value> {
    serde_json::from_str(value)
        .map_err(|error| Error::Other(format!("invalid stored control JSON: {error}")))
}

fn completion_request_value(request: &CompletionRequest) -> Result<Value> {
    Ok(json!({
        "model": request.model,
        "messages": request.messages,
        "tools": request.tools,
        "tool_choice": request.tool_choice,
        "response_format": request.response_format,
        "prompt": request.prompt,
        "suffix": request.suffix,
        "sampling": {
            "temperature": request.sampling.temperature,
            "top_p": request.sampling.top_p,
            "max_tokens": request.sampling.max_tokens,
            "stop": request.sampling.stop,
            "seed": request.sampling.seed,
            "frequency_penalty": request.sampling.frequency_penalty,
            "presence_penalty": request.sampling.presence_penalty,
            "top_k": request.sampling.top_k,
            "min_p": request.sampling.min_p,
            "repetition_penalty": request.sampling.repetition_penalty,
            "thinking_token_budget": request.sampling.thinking_token_budget,
        },
        "reasoning_effort": request.reasoning_effort,
    }))
}

fn control_chat_messages(store: &UserDataStore, thread_id: &str) -> Result<Vec<ChatMessage>> {
    store
        .control_projected_messages(thread_id)?
        .into_iter()
        .map(|raw| {
            let mut value: Value = serde_json::from_str(&raw)
                .map_err(|error| Error::Other(format!("invalid stored message: {error}")))?;
            if let Some(prompt) = value
                .get("promptContent")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                value["content"] = Value::String(prompt);
            }
            let image_parts = value
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|attachment| {
                    let url = attachment.get("data_url")?.as_str()?;
                    attachment
                        .get("mime")?
                        .as_str()?
                        .starts_with("image/")
                        .then(|| json!({ "type": "image_url", "image_url": { "url": url } }))
                })
                .collect::<Vec<_>>();
            if !image_parts.is_empty() {
                let text = value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut parts = vec![json!({ "type": "text", "text": text })];
                parts.extend(image_parts);
                value["content"] = Value::Array(parts);
            }
            serde_json::from_value(value)
                .map_err(|error| Error::Other(format!("invalid control chat message: {error}")))
        })
        .collect()
}

fn control_account_images(
    attachments: &[ControlAttachmentV1],
) -> Vec<crate::codex_bridge::AccountImage> {
    attachments
        .iter()
        .filter_map(|attachment| {
            let data_url = attachment.data_url.as_deref()?;
            if !attachment.mime.starts_with("image/") {
                return None;
            }
            let (_, data) = data_url.split_once(',')?;
            Some(crate::codex_bridge::AccountImage {
                media_type: attachment.mime.clone(),
                data: data.to_string(),
            })
        })
        .collect()
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

fn required_thread_id(command: &ControlCommandV1) -> Result<&str> {
    command
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            Error::InvalidRequest(format!("{} requires thread_id", command.kind.as_str()))
        })
}

fn required_payload_string(payload: &Value, key: &str) -> Result<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::InvalidRequest(format!("payload.{key} must be a non-empty string")))
}

fn normalized_model_favorite_ids(values: &[Value], strict: bool) -> Result<Vec<String>> {
    if values.len() > MAX_MODEL_FAVORITES {
        return Err(Error::InvalidRequest(format!(
            "favorite_model_ids supports at most {MAX_MODEL_FAVORITES} models"
        )));
    }
    let mut normalized = Vec::new();
    for value in values {
        let Some(id) = value.as_str() else {
            if strict {
                return Err(Error::InvalidRequest(
                    "favorite_model_ids must contain only strings".into(),
                ));
            }
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if id.chars().count() > MAX_MODEL_FAVORITE_ID_CHARS {
            if strict {
                return Err(Error::InvalidRequest(format!(
                    "favorite model ids must contain at most {MAX_MODEL_FAVORITE_ID_CHARS} characters"
                )));
            }
            continue;
        }
        if !normalized.iter().any(|existing| existing == id) {
            normalized.push(id.to_string());
        }
    }
    Ok(normalized)
}

fn settings_object(object: &mut Map<String, Value>) -> Result<&mut Map<String, Value>> {
    if !object.contains_key("settings") {
        object.insert("settings".into(), json!({}));
    }
    object
        .get_mut("settings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::Other("stored thread settings are not an object".into()))
}

fn setting_string(settings: Option<&Map<String, Value>>, key: &str, fallback: &str) -> String {
    settings
        .and_then(|settings| settings.get(key))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn thread_agent_id(thread: &ControlThreadRecord) -> Option<String> {
    serde_json::from_str::<Value>(&thread.session_json)
        .ok()?
        .get("settings")?
        .get("activeAgentId")?
        .as_str()
        .map(str::to_string)
}

fn parse_reasoning_effort(value: &str) -> Option<ReasoningEffort> {
    serde_json::from_value(Value::String(value.to_string())).ok()
}

fn decode_appearance_background(source: &str) -> Option<(&'static str, Vec<u8>)> {
    let source = source.trim();
    let source = source.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let source = match source.as_bytes() {
        [b'\'', .., b'\''] | [b'"', .., b'"'] if source.len() >= 2 => &source[1..source.len() - 1],
        _ => source,
    };
    let data = source.strip_prefix("data:")?;
    let (metadata, payload) = data.split_once(',')?;
    let mut metadata = metadata.split(';');
    let source_mime = metadata.next()?.trim().to_ascii_lowercase();
    if !metadata.any(|part| part.eq_ignore_ascii_case("base64")) {
        return None;
    }
    let (mime, signature_matches): (&'static str, fn(&[u8]) -> bool) = match source_mime.as_str() {
        "image/jpeg" | "image/jpg" => {
            ("image/jpeg", |bytes| bytes.starts_with(&[0xff, 0xd8, 0xff]))
        }
        "image/png" => ("image/png", |bytes| bytes.starts_with(b"\x89PNG\r\n\x1a\n")),
        "image/gif" => ("image/gif", |bytes| {
            bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
        }),
        "image/webp" => ("image/webp", |bytes| {
            bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
        }),
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    if bytes.is_empty()
        || bytes.len() > MAX_APPEARANCE_BACKGROUND_BYTES
        || !signature_matches(&bytes)
    {
        return None;
    }
    Some((mime, bytes))
}

fn uppercase_role(role: &str) -> &'static str {
    match role {
        "system" => "System",
        "assistant" => "Assistant",
        _ => "User",
    }
}

fn normalized_approval_kind(kind: &str) -> &str {
    match kind {
        "command" => "command",
        "file_change" => "file_change",
        "permissions" | "permission" => "permission_elevation",
        "mcp_form" => "mcp_form",
        "mcp_url" => "mcp_url",
        _ => "unsupported",
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn default_control_tool_mode() -> String {
    "all".to_string()
}

fn default_control_skill_mode() -> String {
    "auto".to_string()
}

fn control_default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::config::ServerConfiguration;
    use milim_inference::test_backend::TestBackend;
    use milim_storage::Database;

    fn manager_and_state() -> (Arc<RunManager>, AppState) {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        let manager = RunManager::new(store, "Fixture desktop").unwrap();
        let service = Arc::new(TestBackend::new());
        let state =
            AppState::new(service, ServerConfiguration::default()).with_control(manager.clone());
        (manager, state)
    }

    fn create_command(command_id: &str, model: &str) -> ControlCommandV1 {
        ControlCommandV1 {
            command_id: command_id.into(),
            kind: ControlCommandKindV1::ThreadCreate,
            thread_id: None,
            expected_revision: None,
            payload: json!({
                "id": "thread-fixture",
                "title": "Fixture",
                "settings": { "model": model, "privacy": "off", "toolApproval": "review" }
            }),
            confirmation_token: None,
        }
    }

    fn journal_fixture(mode: crate::privacy::PrivacyMode) -> (Arc<UserDataStore>, RunJournal) {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"fixture"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        let journal = RunJournal {
            store: store.clone(),
            privacy: Arc::new(crate::privacy::PrivacyGate::default()),
            privacy_mode: mode,
            thread_id: "thread-1".into(),
            run_id: "run-1".into(),
        };
        (store, journal)
    }

    #[test]
    fn checked_in_protocol_fixtures_decode() {
        let bootstrap = serde_json::from_str::<ControlBootstrapV1>(include_str!(
            "../../../contracts/control-v1/bootstrap.json"
        ))
        .unwrap();
        assert_eq!(bootstrap.appearance.theme_id, "fixture-custom");
        serde_json::from_str::<ControlCommandV1>(include_str!(
            "../../../contracts/control-v1/command-turn-send.json"
        ))
        .unwrap();
        serde_json::from_str::<ControlCommandResultV1>(include_str!(
            "../../../contracts/control-v1/command-result.json"
        ))
        .unwrap();
        serde_json::from_str::<ControlEventV1>(include_str!(
            "../../../contracts/control-v1/event.json"
        ))
        .unwrap();
        serde_json::from_str::<TimelinePageV1>(include_str!(
            "../../../contracts/control-v1/timeline.json"
        ))
        .unwrap();
        serde_json::from_str::<PendingApprovalV1>(include_str!(
            "../../../contracts/control-v1/approval.json"
        ))
        .unwrap();
        let pairing: Value =
            serde_json::from_str(include_str!("../../../contracts/control-v1/pairing.json"))
                .unwrap();
        assert_eq!(pairing["host_id"], "host-fixture");
    }

    #[tokio::test]
    async fn thread_create_is_idempotent_by_thread_id() {
        let (manager, state) = manager_and_state();
        let first = manager
            .command(
                state.clone(),
                None,
                create_command("create-first", "openai:gpt-5"),
            )
            .await
            .unwrap();
        let second = manager
            .command(
                state.clone(),
                None,
                create_command("create-retry", "openrouter:other-model"),
            )
            .await
            .unwrap();

        assert_eq!(first.status, ControlCommandStatusV1::Applied);
        assert_eq!(second.status, ControlCommandStatusV1::Applied);
        assert_eq!(second.revision, first.revision);
        let bootstrap = manager.bootstrap(&state).await.unwrap();
        assert_eq!(bootstrap.threads.len(), 1);
        assert_eq!(bootstrap.threads[0].model.as_deref(), Some("openai:gpt-5"));
    }

    #[tokio::test]
    async fn run_ledger_scrubs_credentials_before_any_artifact_is_persisted() {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"fixture"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        let journal = RunJournal {
            store: store.clone(),
            privacy: Arc::new(crate::privacy::PrivacyGate::default()),
            privacy_mode: crate::privacy::PrivacyMode::Off,
            thread_id: "thread-1".into(),
            run_id: "run-1".into(),
        };
        let sentinel = "sentinel-device-credential-9381";
        let request = CompletionRequest {
            model: "fixture".into(),
            messages: vec![ChatMessage::text(
                "user",
                format!("Authorization: Bearer {sentinel}"),
            )],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams::default(),
            reasoning_effort: None,
        };
        journal.commit_model_request(1, &request).await.unwrap();
        journal
            .commit_tool_result(
                1,
                Some("call-1"),
                "fixture",
                &json!({"device_key": sentinel, "result": format!("sk-{sentinel}")}),
                &format!("sk-{sentinel}"),
            )
            .await
            .unwrap();

        let artifacts = store.control_run_artifacts("run-1").unwrap();
        assert!(!artifacts.is_empty());
        let stored = serde_json::to_string(&artifacts).unwrap();
        assert!(!stored.contains(sentinel));
        assert!(stored.contains("REDACTED_CREDENTIAL"));
        let events =
            serde_json::to_string(&store.control_run_events("run-1", None, 50).unwrap()).unwrap();
        assert!(!events.contains(sentinel));
    }

    #[tokio::test]
    async fn clean_run_ledger_reconstructs_provider_request_byte_for_byte() {
        let (store, journal) = journal_fixture(crate::privacy::PrivacyMode::Off);
        let request = CompletionRequest {
            model: "fixture-model".into(),
            messages: vec![
                ChatMessage::text("system", "follow the fixture"),
                ChatMessage::text("user", "hello"),
            ],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams::default(),
            reasoning_effort: Some(ReasoningEffort::High),
        };
        let expected = serde_json::to_vec(&completion_request_value(&request).unwrap()).unwrap();
        journal.commit_model_request(1, &request).await.unwrap();
        let artifact = store
            .control_run_artifacts("run-1")
            .unwrap()
            .into_iter()
            .find(|artifact| artifact.kind == "provider_request")
            .unwrap();
        assert_eq!(artifact.data_json.as_bytes(), expected);
    }

    #[tokio::test]
    async fn subsequent_model_step_rebuilds_text_and_tool_context_from_sqlite() {
        let (_store, journal) = journal_fixture(crate::privacy::PrivacyMode::Off);
        let request = CompletionRequest {
            model: "fixture-model".into(),
            messages: vec![
                ChatMessage::text("system", "ledger authority"),
                ChatMessage::text("user", "read the fixture"),
            ],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams::default(),
            reasoning_effort: None,
        };
        let tool_calls: Vec<ToolCall> = serde_json::from_value(json!([{
            "id": "call-1",
            "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}
        }]))
        .unwrap();
        journal.commit_model_request(1, &request).await.unwrap();
        journal
            .commit_model_response(
                1,
                "I will read it.",
                "",
                &tool_calls,
                "tool_calls",
                Usage::default(),
            )
            .await
            .unwrap();
        journal
            .commit_tool_result(
                1,
                Some("call-1"),
                "read_file",
                &json!({"content": "durable result"}),
                "{\"content\":\"durable result\"}",
            )
            .await
            .unwrap();

        let mut memory_cache = vec![ChatMessage::text("user", "poisoned memory cache")];
        journal
            .prepare_model_step(2, &mut memory_cache)
            .await
            .unwrap();
        assert_eq!(memory_cache.len(), 4);
        assert_eq!(memory_cache[0].text_content(), "ledger authority");
        assert_eq!(memory_cache[1].text_content(), "read the fixture");
        assert_eq!(memory_cache[2].text_content(), "I will read it.");
        assert_eq!(
            memory_cache[2].tool_calls.as_ref().unwrap()[0]
                .function
                .name,
            "read_file"
        );
        assert_eq!(
            memory_cache[3].text_content(),
            "{\"content\":\"durable result\"}"
        );
        assert_eq!(memory_cache[3].tool_call_id.as_deref(), Some("call-1"));
        assert!(memory_cache
            .iter()
            .all(|message| message.text_content() != "poisoned memory cache"));
    }

    #[tokio::test]
    async fn privacy_block_rejection_leaves_no_request_ledger_rows() {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"fixture"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        let journal = RunJournal {
            store: store.clone(),
            privacy: Arc::new(crate::privacy::PrivacyGate::default()),
            privacy_mode: crate::privacy::PrivacyMode::Block,
            thread_id: "thread-1".into(),
            run_id: "run-1".into(),
        };
        let request = CompletionRequest {
            model: "fixture".into(),
            messages: vec![ChatMessage::text("user", "private@example.com")],
            tools: vec![],
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams::default(),
            reasoning_effort: None,
        };
        assert!(journal.commit_model_request(1, &request).await.is_err());
        assert!(store.control_run_artifacts("run-1").unwrap().is_empty());
        assert!(store
            .control_run_events("run-1", None, 50)
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn model_patch_atomically_persists_thread_reasoning_effort() {
        let (manager, state) = manager_and_state();
        let created = manager
            .command(
                state.clone(),
                None,
                create_command("create-reasoning", "codex:gpt-5"),
            )
            .await
            .unwrap();
        let changed = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "set-reasoning".into(),
                    kind: ControlCommandKindV1::ThreadSetModel,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: created.revision,
                    payload: json!({"model": "codex:gpt-5", "reasoning_effort": "high"}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(changed.status, ControlCommandStatusV1::Applied);

        let bootstrap = manager.bootstrap(&state).await.unwrap();
        assert_eq!(
            bootstrap.threads[0]
                .reasoning_effort_overrides
                .get("codex:gpt-5")
                .map(String::as_str),
            Some("high")
        );
        let thread = manager
            .store
            .control_thread("thread-fixture")
            .unwrap()
            .unwrap();
        let frozen = resolve_frozen_config(&state, &thread, vec![]).unwrap();
        assert_eq!(frozen.model, "gpt-5");
        assert_eq!(frozen.reasoning_effort.as_deref(), Some("high"));

        let explicit_auto = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "set-reasoning-auto".into(),
                    kind: ControlCommandKindV1::ThreadSetModel,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: changed.revision,
                    payload: json!({"model": "codex:gpt-5", "reasoning_effort": "auto"}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(explicit_auto.status, ControlCommandStatusV1::Applied);
        let session: Value = serde_json::from_str(
            &manager
                .store
                .control_thread("thread-fixture")
                .unwrap()
                .unwrap()
                .session_json,
        )
        .unwrap();
        assert_eq!(
            session["settings"]["reasoningEffortOverrides"]["codex:gpt-5"],
            "auto"
        );

        let cleared = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "clear-model".into(),
                    kind: ControlCommandKindV1::ThreadSetModel,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: explicit_auto.revision,
                    payload: json!({"model": ""}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(cleared.status, ControlCommandStatusV1::Applied);
        let bootstrap = manager.bootstrap(&state).await.unwrap();
        assert_eq!(bootstrap.threads[0].model, None);
        let thread = manager
            .store
            .control_thread("thread-fixture")
            .unwrap()
            .unwrap();
        assert!(resolve_frozen_config(&state, &thread, vec![]).is_err());
    }

    #[test]
    fn frozen_config_prefers_the_worker_model_for_legacy_child_threads() {
        let (manager, state) = manager_and_state();
        let thread = manager
            .store
            .control_create_thread(
                "legacy-child",
                &json!({
                    "id": "legacy-child",
                    "settings": { "model": "provider:stale:old-model" },
                    "worker": { "model": "provider:current:new-model" }
                })
                .to_string(),
                "epoch-1",
            )
            .unwrap();

        let frozen = resolve_frozen_config(&state, &thread, vec![]).unwrap();
        assert_eq!(frozen.model, "provider:current:new-model");
    }

    #[test]
    fn generation_settings_are_normalized_and_mapped_to_sampling() {
        let generation = normalize_generation_settings(&json!({
            "maxTokens": 4096,
            "temperature": 0.4,
            "topP": 0.95,
            "seed": 7,
            "stop": [" END ", "", "x".repeat(257)],
            "frequencyPenalty": -0.25,
            "presencePenalty": 0.5,
            "topK": 40,
            "minP": 0.1,
            "repetitionPenalty": 1.05,
            "thinkingTokenBudget": 2048
        }));
        let sampling = sampling_from_generation(&generation);

        assert_eq!(sampling.max_tokens, Some(4096));
        assert_eq!(sampling.temperature, Some(0.4));
        assert_eq!(sampling.top_p, Some(0.95));
        assert_eq!(sampling.seed, Some(7));
        assert_eq!(sampling.stop, ["END"]);
        assert_eq!(sampling.frequency_penalty, Some(-0.25));
        assert_eq!(sampling.presence_penalty, Some(0.5));
        assert_eq!(sampling.top_k, Some(40));
        assert_eq!(sampling.min_p, Some(0.1));
        assert_eq!(sampling.repetition_penalty, Some(1.05));
        assert_eq!(sampling.thinking_token_budget, Some(2048));

        let invalid = normalize_generation_settings(&json!({
            "temperature": 3,
            "topP": 0,
            "topK": 0,
            "repetitionPenalty": 0
        }));
        assert!(invalid.temperature.is_none());
        assert!(invalid.top_p.is_none());
        assert!(invalid.top_k.is_none());
        assert!(invalid.repetition_penalty.is_none());
    }

    #[test]
    fn existing_desktop_transcript_is_backfilled_before_clients_connect() {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .set_sessions_snapshot(
                &json!({
                    "state": {
                        "activeId": "existing-thread",
                        "sessions": [{
                            "id": "existing-thread",
                            "title": "Existing transcript",
                            "createdAt": 100,
                            "updatedAt": 200,
                            "messages": [{
                                "id": "user-1",
                                "role": "user",
                                "content": "hello"
                            }, {
                                "id": "assistant-1",
                                "role": "assistant",
                                "content": "welcome back",
                                "streamParts": [{"kind": "thinking", "content": "brief thought"}]
                            }]
                        }]
                    },
                    "version": 0
                })
                .to_string(),
            )
            .unwrap();

        let manager = RunManager::new(store, "Fixture desktop").unwrap();
        let page = manager
            .timeline_page("existing-thread", None, None, true, 100)
            .unwrap()
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].data["id"], "user-1");
        assert_eq!(page.items[0].data["content"], "hello");
        assert_eq!(page.items[1].data["id"], "assistant-1");
        assert_eq!(page.items[1].data["content"], "welcome back");
        assert_eq!(page.items[1].data["reasoning"], "brief thought");
    }

    #[test]
    fn appearance_snapshot_is_durable_and_published_live() {
        let (manager, _) = manager_and_state();
        let mut receiver = manager.subscribe();
        let mut appearance = AppearanceSnapshotV1 {
            revision: "fixture-revision".into(),
            theme_id: "fixture-custom".into(),
            ..AppearanceSnapshotV1::default()
        };
        appearance.colors.accent = "#ff00aa".into();
        manager
            .store
            .set_json(
                APPEARANCE_STATE_KEY,
                &serde_json::to_string(&appearance).unwrap(),
            )
            .unwrap();

        assert_eq!(manager.appearance_snapshot(), appearance);
        manager.publish_appearance();
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.event_type, "appearance.updated");
        assert_eq!(event.data["appearance"]["colors"]["accent"], "#ff00aa");
    }

    #[test]
    fn model_catalog_updates_are_published_live() {
        let (manager, _) = manager_and_state();
        let mut receiver = manager.subscribe();

        manager.publish_model_catalog();

        let event = receiver.try_recv().unwrap();
        assert_eq!(event.event_type, "models.updated");
        assert_eq!(event.thread_id, None);
    }

    #[tokio::test]
    async fn model_favorites_round_trip_through_bootstrap_command_and_live_event() {
        let (manager, state) = manager_and_state();
        manager
            .store
            .set_json(
                MODEL_FAVORITES_SETTINGS_KEY,
                r#"{"state":{"favorites":["codex:gpt-5"],"browserStorageMode":"private"},"version":0}"#,
            )
            .unwrap();
        assert_eq!(
            manager.bootstrap(&state).await.unwrap().favorite_model_ids,
            vec!["codex:gpt-5"]
        );

        let mut receiver = manager.subscribe();
        let result = manager
            .command(
                state.clone(),
                Some("device-1".into()),
                ControlCommandV1 {
                    command_id: "set-model-favorites".into(),
                    kind: ControlCommandKindV1::ModelFavoritesSet,
                    thread_id: None,
                    expected_revision: None,
                    payload: json!({
                        "favorite_model_ids": [" claude:opus ", "claude:opus", "provider:model"]
                    }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.status, ControlCommandStatusV1::Applied);
        assert_eq!(
            result.data["favorite_model_ids"],
            json!(["claude:opus", "provider:model"])
        );
        let event = receiver.try_recv().unwrap();
        assert_eq!(event.event_type, MODEL_FAVORITES_EVENT_TYPE);
        assert_eq!(
            event.data["favorite_model_ids"],
            json!(["claude:opus", "provider:model"])
        );
        assert_eq!(
            manager.bootstrap(&state).await.unwrap().favorite_model_ids,
            vec!["claude:opus", "provider:model"]
        );
        let persisted: Value = serde_json::from_str(
            &manager
                .store
                .get_json(MODEL_FAVORITES_SETTINGS_KEY)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(persisted["state"]["browserStorageMode"], "private");
    }

    #[tokio::test]
    async fn model_favorites_reject_non_string_entries_without_mutating_settings() {
        let (manager, state) = manager_and_state();
        manager
            .store
            .set_json(
                MODEL_FAVORITES_SETTINGS_KEY,
                r#"{"state":{"favorites":["codex:gpt-5"]},"version":0}"#,
            )
            .unwrap();
        let result = manager
            .command(
                state,
                Some("device-1".into()),
                ControlCommandV1 {
                    command_id: "invalid-model-favorites".into(),
                    kind: ControlCommandKindV1::ModelFavoritesSet,
                    thread_id: None,
                    expected_revision: None,
                    payload: json!({"favorite_model_ids": ["claude:opus", 42]}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.status, ControlCommandStatusV1::Failed);
        assert_eq!(manager.model_favorites(), vec!["codex:gpt-5"]);
    }

    #[test]
    fn appearance_background_asset_is_bounded_and_scoped_to_the_active_theme() {
        let (manager, _) = manager_and_state();
        let mut appearance = AppearanceSnapshotV1 {
            revision: "background-revision".into(),
            theme_id: "custom-active".into(),
            ..AppearanceSnapshotV1::default()
        };
        appearance.background.has_image = true;
        manager
            .store
            .set_json(
                APPEARANCE_STATE_KEY,
                &serde_json::to_string(&appearance).unwrap(),
            )
            .unwrap();
        manager
            .store
            .set_json(
                CUSTOM_THEMES_STATE_KEY,
                &json!([{
                    "id": "custom-other",
                    "background": { "image": "url(data:image/png;base64,AAAA)" }
                }, {
                    "id": "custom-active",
                    "background": { "image": "url(data:image/png;base64,iVBORw0KGgo=)" }
                }])
                .to_string(),
            )
            .unwrap();

        let asset = manager.appearance_background_asset().unwrap();
        assert_eq!(asset.revision, "background-revision");
        assert_eq!(asset.mime, "image/png");
        assert_eq!(asset.bytes, b"\x89PNG\r\n\x1a\n");
        assert!(decode_appearance_background("url(https://example.test/background.png)").is_none());
        assert!(decode_appearance_background("linear-gradient(red, blue)").is_none());
    }

    #[test]
    fn attachment_limits_are_rejected_before_run_acceptance() {
        assert_eq!(
            normalized_approval_kind("permissions"),
            "permission_elevation"
        );
        assert_eq!(normalized_approval_kind("mcp_form"), "mcp_form");
        assert_eq!(normalized_approval_kind("future_schema"), "unsupported");
        let oversized = ControlAttachmentV1 {
            id: "attachment-1".into(),
            name: "large.png".into(),
            mime: "image/png".into(),
            size: MAX_CONTROL_ATTACHMENT_BYTES + 1,
            content: None,
            data_url: Some("data:image/png;base64,AA==".into()),
            truncated: false,
        };
        let error = validate_control_attachments(&[oversized]).unwrap_err();
        assert!(error.to_string().contains("2 MiB"));

        let empty = ControlAttachmentV1 {
            id: "attachment-2".into(),
            name: "empty.txt".into(),
            mime: "text/plain".into(),
            size: 0,
            content: None,
            data_url: None,
            truncated: false,
        };
        assert!(validate_control_attachments(&[empty]).is_err());
    }

    #[tokio::test]
    async fn mock_turn_is_server_owned_durable_and_idempotent() {
        let (manager, state) = manager_and_state();
        let created = manager
            .command(state.clone(), None, create_command("create-1", "mock-echo"))
            .await
            .unwrap();
        assert_eq!(created.status, ControlCommandStatusV1::Applied);

        let send = ControlCommandV1 {
            command_id: "send-1".into(),
            kind: ControlCommandKindV1::TurnSend,
            thread_id: Some("thread-fixture".into()),
            expected_revision: created.revision,
            payload: json!({ "text": "hello", "attachments": [] }),
            confirmation_token: None,
        };
        let accepted = manager
            .command(state.clone(), Some("phone-1".into()), send.clone())
            .await
            .unwrap();
        assert_eq!(accepted.status, ControlCommandStatusV1::Accepted);
        let duplicate = manager
            .command(state.clone(), Some("phone-1".into()), send)
            .await
            .unwrap();
        assert_eq!(duplicate.run_id, accepted.run_id);

        for _ in 0..100 {
            if manager.store.control_runs(true).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(manager.store.control_runs(true).unwrap().is_empty());
        let page = manager
            .timeline_page("thread-fixture", None, None, true, 100)
            .unwrap()
            .unwrap();
        assert!(page
            .items
            .iter()
            .any(|item| item.item_type == "assistant_delta"));
        assert!(page
            .items
            .iter()
            .any(|item| { item.item_type == "message" && item.data["role"] == "assistant" }));
        let messages = manager.store.control_messages("thread-fixture").unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn destructive_confirmation_is_one_time_and_final_result_is_idempotent() {
        let (manager, state) = manager_and_state();
        manager
            .command(state.clone(), None, create_command("create-1", "mock-echo"))
            .await
            .unwrap();
        let mut delete = ControlCommandV1 {
            command_id: "delete-1".into(),
            kind: ControlCommandKindV1::ThreadDelete,
            thread_id: Some("thread-fixture".into()),
            expected_revision: None,
            payload: Value::Null,
            confirmation_token: None,
        };
        let challenge = manager
            .command(state.clone(), None, delete.clone())
            .await
            .unwrap();
        assert_eq!(challenge.status, ControlCommandStatusV1::NeedsConfirmation);
        delete.confirmation_token = challenge.confirmation_token;
        let applied = manager
            .command(state.clone(), None, delete.clone())
            .await
            .unwrap();
        assert_eq!(applied.status, ControlCommandStatusV1::Applied);
        let retry = manager.command(state, None, delete).await.unwrap();
        assert_eq!(retry.status, ControlCommandStatusV1::Applied);
    }

    #[tokio::test]
    async fn stop_preserves_queue_until_an_explicit_resume() {
        let (manager, state) = manager_and_state();
        let created = manager
            .command(state.clone(), None, create_command("create-1", "mock-echo"))
            .await
            .unwrap();
        let first = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "send-first".into(),
                    kind: ControlCommandKindV1::TurnSend,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: created.revision,
                    payload: json!({ "text": "first", "attachments": [] }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(first.status, ControlCommandStatusV1::Accepted);
        let queued = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "send-second".into(),
                    kind: ControlCommandKindV1::TurnSend,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({
                        "text": "second",
                        "display_text": "Second shown",
                        "attachments": []
                    }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(queued.status, ControlCommandStatusV1::Queued);
        let third = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "send-third".into(),
                    kind: ControlCommandKindV1::TurnSend,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({ "text": "third", "attachments": [] }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(third.status, ControlCommandStatusV1::Queued);
        let moved = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "move-third".into(),
                    kind: ControlCommandKindV1::TurnQueueMove,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({
                        "queue_id": third.queue_id,
                        "target_id": queued.queue_id,
                        "position": "before"
                    }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(moved.status, ControlCommandStatusV1::Applied);
        let pending = manager
            .store
            .control_queued_turns(Some("thread-fixture"))
            .unwrap();
        assert_eq!(pending[0].id, third.queue_id.clone().unwrap());
        let deleted = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "delete-third".into(),
                    kind: ControlCommandKindV1::TurnQueueDelete,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({ "queue_id": third.queue_id }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(deleted.status, ControlCommandStatusV1::Applied);
        let bootstrap = manager.bootstrap(&state).await.unwrap();
        assert_eq!(bootstrap.queued_turns.len(), 1);
        assert_eq!(bootstrap.queued_turns[0].display_text, "Second shown");
        assert!(bootstrap.queued_turns[0].attachments.is_empty());
        manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "stop-first".into(),
                    kind: ControlCommandKindV1::TurnStop,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: Value::Null,
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        for _ in 0..100 {
            if manager.store.control_runs(true).unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let pending = manager
            .store
            .control_queued_turns(Some("thread-fixture"))
            .unwrap();
        assert_eq!(pending.len(), 1);
        let resumed = manager
            .command(
                state,
                None,
                ControlCommandV1 {
                    command_id: "resume-second".into(),
                    kind: ControlCommandKindV1::TurnQueueResume,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({ "queue_id": pending[0].id }),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(resumed.status, ControlCommandStatusV1::Accepted);
        assert!(manager
            .store
            .control_queued_turns(Some("thread-fixture"))
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn inbox_injection_is_durable_deletable_and_does_not_wake_an_idle_thread() {
        let (manager, state) = manager_and_state();
        manager
            .command(
                state.clone(),
                None,
                create_command("create-inject", "mock-echo"),
            )
            .await
            .unwrap();
        let injected = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "inject-1".into(),
                    kind: ControlCommandKindV1::ContextInject,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({"text": "quiet context"}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(injected.status, ControlCommandStatusV1::Accepted);
        assert!(manager.store.control_runs(true).unwrap().is_empty());
        let bootstrap = manager.bootstrap(&state).await.unwrap();
        assert!(bootstrap.active_runs.is_empty());
        assert_eq!(bootstrap.pending_inputs.len(), 1);
        assert_eq!(bootstrap.pending_inputs[0].kind, "inject");

        let inbox_id = injected.data["inbox_id"].as_str().unwrap();
        let deleted = manager
            .command(
                state.clone(),
                None,
                ControlCommandV1 {
                    command_id: "delete-inject-1".into(),
                    kind: ControlCommandKindV1::TurnInboxDelete,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({"inbox_id": inbox_id}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(deleted.status, ControlCommandStatusV1::Applied);
        assert!(manager
            .bootstrap(&state)
            .await
            .unwrap()
            .pending_inputs
            .is_empty());

        let conflict = manager
            .command(
                state,
                None,
                ControlCommandV1 {
                    command_id: "delete-inject-conflict".into(),
                    kind: ControlCommandKindV1::TurnInboxDelete,
                    thread_id: Some("thread-fixture".into()),
                    expected_revision: None,
                    payload: json!({"inbox_id": inbox_id}),
                    confirmation_token: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(conflict.status, ControlCommandStatusV1::Failed);
    }

    #[tokio::test]
    async fn inbox_steering_requires_the_exact_active_steer_capable_run() {
        let (manager, state) = manager_and_state();
        manager
            .command(
                state.clone(),
                None,
                create_command("create-steer", "mock-echo"),
            )
            .await
            .unwrap();
        manager
            .store
            .control_put_run(&ControlRunRecord {
                id: "run-active".into(),
                thread_id: "thread-fixture".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: serde_json::to_string(&AcceptedTurnV1 {
                    text: "active".into(),
                    display_text: None,
                    config: resolve_frozen_config(
                        &state,
                        &manager
                            .store
                            .control_thread("thread-fixture")
                            .unwrap()
                            .unwrap(),
                        vec![],
                    )
                    .unwrap(),
                    append_user: true,
                })
                .unwrap(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        let (stop, _stop_rx) = watch::channel(false);
        manager.active.lock().unwrap().insert(
            "thread-fixture".into(),
            ActiveRun {
                run_id: "run-active".into(),
                steering: false,
                stop: stop.clone(),
            },
        );

        let steer = |command_id: &str, run_id: &str| ControlCommandV1 {
            command_id: command_id.into(),
            kind: ControlCommandKindV1::TurnSteer,
            thread_id: Some("thread-fixture".into()),
            expected_revision: None,
            payload: json!({"run_id": run_id, "text": "adjust", "attachments": []}),
            confirmation_token: None,
        };
        let unsupported = manager
            .command(
                state.clone(),
                None,
                steer("steer-unsupported", "run-active"),
            )
            .await
            .unwrap();
        assert_eq!(unsupported.status, ControlCommandStatusV1::Failed);

        manager
            .active
            .lock()
            .unwrap()
            .get_mut("thread-fixture")
            .unwrap()
            .steering = true;
        let mismatched = manager
            .command(state.clone(), None, steer("steer-mismatch", "run-other"))
            .await
            .unwrap();
        assert_eq!(mismatched.status, ControlCommandStatusV1::Failed);
        let accepted = manager
            .command(state, None, steer("steer-accepted", "run-active"))
            .await
            .unwrap();
        assert_eq!(accepted.status, ControlCommandStatusV1::Accepted);
        let pending = manager
            .store
            .control_pending_inbox(Some("thread-fixture"))
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "steer");
        assert_eq!(pending[0].target_run_id.as_deref(), Some("run-active"));
    }
}
