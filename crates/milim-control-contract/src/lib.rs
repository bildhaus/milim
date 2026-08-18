use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const CONTROL_PROTOCOL_MIN: u16 = 1;
pub const CONTROL_PROTOCOL_MAX: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ControlProtocolRangeV1 {
    pub min: u16,
    pub max: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub run_ledger: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub run_inspection: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub steering: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub context_injection: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model_favorites: Option<bool>,
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
            run_ledger: Some(true),
            run_inspection: Some(true),
            steering: Some(true),
            context_injection: Some(true),
            model_favorites: Some(true),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ControlAttachmentV1 {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub data_url: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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
    pub adapter: String,
}

fn default_control_tool_mode() -> String {
    "all".into()
}

fn default_control_skill_mode() -> String {
    "all".into()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct RunCapabilitiesV1 {
    pub ledger: bool,
    pub inspectable: bool,
    pub steering: bool,
    pub visibility: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunSnapshotV1 {
    pub id: String,
    pub thread_id: String,
    pub status: String,
    pub adapter: String,
    pub config: FrozenRunConfigV1,
    #[serde(default)]
    pub capabilities: RunCapabilitiesV1,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub error: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PendingApprovalV1 {
    pub id: String,
    pub run_id: String,
    pub thread_id: String,
    pub kind: String,
    pub request: Value,
    pub status: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QueuedTurnV1 {
    pub id: String,
    pub thread_id: String,
    pub command_id: String,
    pub accepted_at_ms: i64,
    pub display_text: String,
    pub attachments: Vec<ControlAttachmentV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PendingInputV1 {
    pub id: String,
    pub thread_id: String,
    pub target_run_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
pub struct AppearanceGlassV1 {
    pub enabled: bool,
    pub blur_radius: f64,
    pub opacity_primary: f64,
    pub opacity_secondary: f64,
    pub edge_light: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
pub struct AppearanceBordersV1 {
    pub card_radius: f64,
    pub input_radius: f64,
    pub border_opacity: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
pub struct AppearanceTypographyV1 {
    pub font_family: String,
    pub mono_family: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ControlBootstrapV1 {
    pub protocol: ControlProtocolRangeV1,
    pub host_id: String,
    pub host_name: String,
    pub capabilities: ControlCapabilitiesV1,
    #[serde(default)]
    pub appearance: AppearanceSnapshotV1,
    pub threads: Vec<ThreadSummaryV1>,
    pub models: Vec<Value>,
    #[serde(default)]
    pub favorite_model_ids: Vec<String>,
    pub agents: Vec<AgentSummaryV1>,
    pub active_runs: Vec<RunSnapshotV1>,
    pub queued_turns: Vec<QueuedTurnV1>,
    #[serde(default)]
    pub pending_inputs: Vec<PendingInputV1>,
    pub pending_approvals: Vec<PendingApprovalV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct TimelineItemV1 {
    pub id: String,
    pub thread_id: String,
    pub epoch: String,
    pub seq: u64,
    pub run_id: Option<String>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub item_type: String,
    pub data: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ControlEventV1 {
    pub event_id: String,
    pub host_id: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub epoch: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seq: Option<u64>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub event_type: String,
    pub data: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub enum ControlCommandStatusV1 {
    #[serde(rename = "applied")]
    #[ts(rename = "applied")]
    Applied,
    #[serde(rename = "accepted")]
    #[ts(rename = "accepted")]
    Accepted,
    #[serde(rename = "queued")]
    #[ts(rename = "queued")]
    Queued,
    #[serde(rename = "needs_confirmation")]
    #[ts(rename = "needs_confirmation")]
    NeedsConfirmation,
    #[serde(rename = "conflict")]
    #[ts(rename = "conflict")]
    Conflict,
    #[serde(rename = "failed")]
    #[ts(rename = "failed")]
    Failed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub enum ControlCommandKindV1 {
    #[serde(rename = "thread.create")]
    #[ts(rename = "thread.create")]
    ThreadCreate,
    #[serde(rename = "thread.rename")]
    #[ts(rename = "thread.rename")]
    ThreadRename,
    #[serde(rename = "thread.archive")]
    #[ts(rename = "thread.archive")]
    ThreadArchive,
    #[serde(rename = "thread.delete")]
    #[ts(rename = "thread.delete")]
    ThreadDelete,
    #[serde(rename = "thread.set_model")]
    #[ts(rename = "thread.set_model")]
    ThreadSetModel,
    #[serde(rename = "thread.set_agent")]
    #[ts(rename = "thread.set_agent")]
    ThreadSetAgent,
    #[serde(rename = "message.delete")]
    #[ts(rename = "message.delete")]
    MessageDelete,
    #[serde(rename = "model_favorites.set")]
    #[ts(rename = "model_favorites.set")]
    ModelFavoritesSet,
    #[serde(rename = "turn.send")]
    #[ts(rename = "turn.send")]
    TurnSend,
    #[serde(rename = "turn.steer")]
    #[ts(rename = "turn.steer")]
    TurnSteer,
    #[serde(rename = "context.inject")]
    #[ts(rename = "context.inject")]
    ContextInject,
    #[serde(rename = "turn.inbox_delete")]
    #[ts(rename = "turn.inbox_delete")]
    TurnInboxDelete,
    #[serde(rename = "turn.stop")]
    #[ts(rename = "turn.stop")]
    TurnStop,
    #[serde(rename = "turn.regenerate")]
    #[ts(rename = "turn.regenerate")]
    TurnRegenerate,
    #[serde(rename = "turn.queue_resume")]
    #[ts(rename = "turn.queue_resume")]
    TurnQueueResume,
    #[serde(rename = "turn.queue_move")]
    #[ts(rename = "turn.queue_move")]
    TurnQueueMove,
    #[serde(rename = "turn.queue_delete")]
    #[ts(rename = "turn.queue_delete")]
    TurnQueueDelete,
    #[serde(rename = "approval.resolve")]
    #[ts(rename = "approval.resolve")]
    ApprovalResolve,
    #[serde(rename = "worker.start")]
    #[ts(rename = "worker.start")]
    WorkerStart,
    #[serde(rename = "worker.continue_solo")]
    #[ts(rename = "worker.continue_solo")]
    WorkerContinueSolo,
    #[serde(rename = "worker.stop")]
    #[ts(rename = "worker.stop")]
    WorkerStop,
}

impl ControlCommandKindV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThreadCreate => "thread.create",
            Self::ThreadRename => "thread.rename",
            Self::ThreadArchive => "thread.archive",
            Self::ThreadDelete => "thread.delete",
            Self::ThreadSetModel => "thread.set_model",
            Self::ThreadSetAgent => "thread.set_agent",
            Self::MessageDelete => "message.delete",
            Self::ModelFavoritesSet => "model_favorites.set",
            Self::TurnSend => "turn.send",
            Self::TurnSteer => "turn.steer",
            Self::ContextInject => "context.inject",
            Self::TurnInboxDelete => "turn.inbox_delete",
            Self::TurnStop => "turn.stop",
            Self::TurnRegenerate => "turn.regenerate",
            Self::TurnQueueResume => "turn.queue_resume",
            Self::TurnQueueMove => "turn.queue_move",
            Self::TurnQueueDelete => "turn.queue_delete",
            Self::ApprovalResolve => "approval.resolve",
            Self::WorkerStart => "worker.start",
            Self::WorkerContinueSolo => "worker.continue_solo",
            Self::WorkerStop => "worker.stop",
        }
    }

    pub fn destructive(self) -> bool {
        matches!(self, Self::ThreadDelete | Self::MessageDelete)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ControlCommandV1 {
    pub command_id: String,
    pub kind: ControlCommandKindV1,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub confirmation_token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ControlCommandResultV1 {
    pub command_id: String,
    pub status: ControlCommandStatusV1,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<u64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub run_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub queue_id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub confirmation_token: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct ResolvedRunCompositionV1 {
    pub visibility: String,
    pub adapter: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub workspace: Option<String>,
    pub environment_policy: String,
    pub prompt_sections: Vec<Value>,
    pub tools: Vec<Value>,
    pub policies: Value,
    pub attachments: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunEventV1 {
    pub id: String,
    pub run_id: String,
    pub seq: u64,
    pub step_id: Option<String>,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub event_type: String,
    pub data: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunEventPageV1 {
    pub run_id: String,
    pub after_seq: Option<u64>,
    pub next_seq: Option<u64>,
    pub has_more: bool,
    pub events: Vec<RunEventV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct RunInspectionV1 {
    pub run: RunSnapshotV1,
    pub composition: Option<ResolvedRunCompositionV1>,
}
