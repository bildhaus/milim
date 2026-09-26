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
    pub global_instructions: String,
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
    #[serde(default = "default_account_profile_id")]
    pub account_profile_id: String,
    #[serde(default)]
    pub account_profile_label: String,
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
