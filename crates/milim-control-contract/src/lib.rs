use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const CONTROL_PROTOCOL_MIN: u16 = 1;
pub const CONTROL_PROTOCOL_MAX: u16 = 1;
pub const CONTROL_MAX_ATTACHMENTS: usize = 12;
pub const CONTROL_MAX_ATTACHMENT_BYTES: u64 = 2 * 1024 * 1024;
pub const CONTROL_MAX_ATTACHMENT_CONTENT_CHARS: usize = 128 * 1024;

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
    pub attachment_uploads: Option<bool>,
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
    pub effective_run_preview: Option<bool>,
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
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_links: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_origins: Option<bool>,
    /// The host records `ask_user` and `exit_plan_mode` decisions in the
    /// timeline and accepts `turn.send` answers to them.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user_decisions: Option<bool>,
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
            attachment_uploads: Some(true),
            run_ledger: Some(true),
            run_inspection: Some(true),
            effective_run_preview: Some(true),
            steering: Some(true),
            context_injection: Some(true),
            model_favorites: Some(true),
            thread_links: Some(true),
            thread_origins: Some(true),
            user_decisions: Some(true),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ThreadLinkV1 {
    pub owner_thread_id: String,
    pub target_thread_id: String,
    pub target_title: String,
    pub target_workspace: Option<String>,
    pub target_project: Option<String>,
    pub target_model: Option<String>,
    pub target_runtime: String,
    pub target_archived_at_ms: Option<i64>,
    pub target_busy: bool,
    pub target_queued_turns: usize,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct FrozenLinkedThreadGrantV1 {
    pub target_thread_id: String,
    pub title: String,
    pub workspace: Option<String>,
    pub project: Option<String>,
    pub model: Option<String>,
    pub runtime: String,
    pub revision: u64,
    pub epoch: String,
    pub max_timeline_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct MailboxOriginV1 {
    pub exchange_id: String,
    pub origin_thread_id: String,
    pub origin_title: String,
    pub origin_workspace: Option<String>,
    pub origin_project: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(tag = "kind", rename_all = "snake_case")]
pub enum ThreadOriginV1 {
    Schedule {
        schedule_id: String,
        schedule_name: String,
        occurrence_unix: i64,
    },
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
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub origin: Option<ThreadOriginV1>,
    pub busy: bool,
    pub queued_turns: usize,
    #[serde(default)]
    pub linked_threads: Vec<ThreadLinkV1>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct ControlAttachmentUploadV1 {
    pub upload_id: String,
    pub expires_at_ms: i64,
}

/// Provider sampling controls frozen at turn acceptance time. Missing values
/// deliberately mean "use the model/server default".
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, TS)]
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct RunLimitsV1 {
    pub max_steps: Option<u32>,
    pub max_seconds: Option<u32>,
    pub max_cost_usd: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
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
    #[serde(default)]
    pub native_session_cursor: Option<String>,
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub generation: GenerationSettingsV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub run_limits: Option<RunLimitsV1>,
    pub adapter: String,
    /// Which signed-in account of `adapter` this turn runs as. `default` is
    /// the runtime's own configuration home; other values name a profile.
    #[serde(default = "default_account_profile_id")]
    pub account_profile_id: String,
    /// Display name for `account_profile_id`, frozen so a later rename does
    /// not rewrite what an accepted run reported.
    #[serde(default)]
    pub account_profile_label: String,
    #[serde(default)]
    pub linked_thread_grants: Vec<FrozenLinkedThreadGrantV1>,
    #[serde(default)]
    pub claimed_mailbox_ids: Vec<String>,
}

fn default_account_profile_id() -> String {
    "default".into()
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
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mailbox_origin: Option<MailboxOriginV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct PendingInputV1 {
    pub id: String,
    pub thread_id: String,
    pub target_run_id: Option<String>,
    pub kind: String,
    pub state: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub display_text: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attachments: Option<Vec<ControlAttachmentV1>>,
    pub created_at_ms: i64,
}

/// Timeline item type of a decision a run handed to the user. Its data is
/// `{ "type": "user_decision_requested", "call_id": ..., "decision": UserDecisionV1 }`.
pub const USER_DECISION_REQUESTED_ITEM: &str = "user_decision_requested";
/// Timeline item type that closes a decision; its data is a
/// [`UserDecisionResolvedV1`] and its `run_id` is the deciding run's.
pub const USER_DECISION_RESOLVED_ITEM: &str = "user_decision_resolved";
pub const USER_DECISION_MAX_QUESTIONS: usize = 4;
pub const USER_DECISION_MIN_OPTIONS: usize = 2;
pub const USER_DECISION_MAX_OPTIONS: usize = 4;
pub const USER_DECISION_MAX_HEADER_CHARS: usize = 12;
pub const USER_DECISION_MAX_QUESTION_CHARS: usize = 1_000;
pub const USER_DECISION_MAX_LABEL_CHARS: usize = 80;
pub const USER_DECISION_MAX_DESCRIPTION_CHARS: usize = 400;
pub const USER_DECISION_MAX_PLAN_CHARS: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UserDecisionKindV1 {
    AskUser,
    ExitPlanMode,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct UserQuestionOptionV1 {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct UserQuestionV1 {
    pub question: String,
    pub header: String,
    pub options: Vec<UserQuestionOptionV1>,
    #[serde(default)]
    pub multi_select: bool,
}

/// A structured decision a run ended on: questions for the user, or a plan
/// to approve. The answer arrives as the thread's next `turn.send`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct UserDecisionV1 {
    pub id: String,
    pub kind: UserDecisionKindV1,
    #[serde(default)]
    pub questions: Vec<UserQuestionV1>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub plan: Option<String>,
    /// The decision as Markdown. It ends the run's assistant message so
    /// clients without decision cards (and later model turns) still see it.
    #[serde(default)]
    pub fallback_text: String,
}

impl UserDecisionV1 {
    pub fn ask_user(id: String, questions: Vec<UserQuestionV1>) -> Result<Self, String> {
        if questions.is_empty() || questions.len() > USER_DECISION_MAX_QUESTIONS {
            return Err(format!(
                "questions must contain 1 to {USER_DECISION_MAX_QUESTIONS} items"
            ));
        }
        let mut questions = questions;
        for (index, question) in questions.iter_mut().enumerate() {
            let at = format!("questions[{index}]");
            question.question = bounded(
                &question.question,
                &format!("{at}.question"),
                USER_DECISION_MAX_QUESTION_CHARS,
            )?;
            question.header = bounded(
                &question.header,
                &format!("{at}.header"),
                USER_DECISION_MAX_HEADER_CHARS,
            )?;
            if question.options.len() < USER_DECISION_MIN_OPTIONS
                || question.options.len() > USER_DECISION_MAX_OPTIONS
            {
                return Err(format!(
                    "{at}.options must contain {USER_DECISION_MIN_OPTIONS} to {USER_DECISION_MAX_OPTIONS} items"
                ));
            }
            let mut labels = Vec::with_capacity(question.options.len());
            for (option_index, option) in question.options.iter_mut().enumerate() {
                let at = format!("{at}.options[{option_index}]");
                option.label = bounded(
                    &option.label,
                    &format!("{at}.label"),
                    USER_DECISION_MAX_LABEL_CHARS,
                )?;
                option.description = option.description.trim().to_string();
                if option.description.chars().count() > USER_DECISION_MAX_DESCRIPTION_CHARS {
                    return Err(format!(
                        "{at}.description must be at most {USER_DECISION_MAX_DESCRIPTION_CHARS} characters"
                    ));
                }
                if labels.contains(&option.label.to_lowercase()) {
                    return Err(format!("{at}.label repeats another option's label"));
                }
                labels.push(option.label.to_lowercase());
            }
        }
        let mut decision = Self {
            id,
            kind: UserDecisionKindV1::AskUser,
            questions,
            plan: None,
            fallback_text: String::new(),
        };
        decision.fallback_text = decision.render_fallback();
        Ok(decision)
    }

    pub fn exit_plan_mode(id: String, plan: &str) -> Result<Self, String> {
        let plan = plan.trim();
        if plan.is_empty() {
            return Err("plan must not be empty".into());
        }
        if plan.chars().count() > USER_DECISION_MAX_PLAN_CHARS {
            return Err(format!(
                "plan must be at most {USER_DECISION_MAX_PLAN_CHARS} characters"
            ));
        }
        let mut decision = Self {
            id,
            kind: UserDecisionKindV1::ExitPlanMode,
            questions: Vec::new(),
            plan: Some(plan.to_string()),
            fallback_text: String::new(),
        };
        decision.fallback_text = decision.render_fallback();
        Ok(decision)
    }

    fn render_fallback(&self) -> String {
        match self.kind {
            UserDecisionKindV1::AskUser => {
                let mut text = String::from("**Questions for you**\n");
                for (index, question) in self.questions.iter().enumerate() {
                    text.push_str(&format!(
                        "\n{}. **{}**: {}{}\n",
                        index + 1,
                        question.header,
                        question.question,
                        if question.multi_select {
                            " (choose any)"
                        } else {
                            ""
                        }
                    ));
                    for option in &question.options {
                        if option.description.is_empty() {
                            text.push_str(&format!("   - {}\n", option.label));
                        } else {
                            text.push_str(&format!(
                                "   - {}: {}\n",
                                option.label, option.description
                            ));
                        }
                    }
                }
                text.push_str("\nReply with your answers, or in your own words.");
                text
            }
            UserDecisionKindV1::ExitPlanMode => format!(
                "**Proposed plan**\n\n{}\n\nReply to approve this plan, or say what to change.",
                self.plan.as_deref().unwrap_or_default()
            ),
        }
    }
}

fn bounded(value: &str, field: &str, max_chars: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.chars().count() > max_chars {
        return Err(format!("{field} must be at most {max_chars} characters"));
    }
    Ok(value.to_string())
}

/// How a `turn.send` answers a pending decision.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UserDecisionResponseV1 {
    /// Answers to an `ask_user` decision.
    Answer,
    /// Approves an `exit_plan_mode` plan; the host turns plan mode off for
    /// the thread before the answering turn runs.
    ApprovePlan,
    /// Feedback on an `exit_plan_mode` plan; plan mode stays on.
    RevisePlan,
}

impl UserDecisionResponseV1 {
    pub fn fits(self, kind: UserDecisionKindV1) -> bool {
        match self {
            Self::Answer => kind == UserDecisionKindV1::AskUser,
            Self::ApprovePlan | Self::RevisePlan => kind == UserDecisionKindV1::ExitPlanMode,
        }
    }
}

/// `turn.send` `payload.answers_decision`: the pending decision the turn's
/// text answers. A stale or second answer is rejected with `conflict`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct UserDecisionAnswerV1 {
    pub decision_id: String,
    pub response: UserDecisionResponseV1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum UserDecisionStatusV1 {
    Answered,
    /// Another turn started before the decision was answered.
    Superseded,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct UserDecisionResolvedV1 {
    pub decision_id: String,
    pub status: UserDecisionStatusV1,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub response: Option<UserDecisionResponseV1>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub command_id: Option<String>,
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
    #[serde(rename = "thread.set_execution_settings")]
    #[ts(rename = "thread.set_execution_settings")]
    ThreadSetExecutionSettings,
    #[serde(rename = "thread.set_account_profile")]
    #[ts(rename = "thread.set_account_profile")]
    ThreadSetAccountProfile,
    #[serde(rename = "thread.link.add")]
    #[ts(rename = "thread.link.add")]
    ThreadLinkAdd,
    #[serde(rename = "thread.link.remove")]
    #[ts(rename = "thread.link.remove")]
    ThreadLinkRemove,
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
            Self::ThreadSetExecutionSettings => "thread.set_execution_settings",
            Self::ThreadSetAccountProfile => "thread.set_account_profile",
            Self::ThreadLinkAdd => "thread.link.add",
            Self::ThreadLinkRemove => "thread.link.remove",
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
    #[serde(default)]
    pub generation: GenerationSettingsV1,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub native_session_boundary: Option<String>,
    pub workspace: Option<String>,
    pub environment_policy: String,
    #[serde(default)]
    pub explicit_environment_grants: Vec<Value>,
    pub prompt_sections: Vec<Value>,
    pub tools: Vec<Value>,
    pub policies: Value,
    pub attachments: Vec<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
pub struct EffectiveRunPreviewRequestV1 {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<ControlAttachmentV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct EffectiveRunPreviewV1 {
    pub thread_id: String,
    pub thread_revision: u64,
    pub resolved_at_ms: i64,
    pub composition: ResolvedRunCompositionV1,
    #[serde(default)]
    pub warnings: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn option(label: &str) -> UserQuestionOptionV1 {
        UserQuestionOptionV1 {
            label: label.into(),
            description: format!("{label} description"),
        }
    }

    fn question(header: &str, options: Vec<UserQuestionOptionV1>) -> UserQuestionV1 {
        UserQuestionV1 {
            question: "Which database?".into(),
            header: header.into(),
            options,
            multi_select: false,
        }
    }

    #[test]
    fn ask_user_validates_bounds_and_renders_a_readable_fallback() {
        let decision = UserDecisionV1::ask_user(
            "d1".into(),
            vec![question(
                " Storage ",
                vec![option("SQLite"), option("Postgres")],
            )],
        )
        .unwrap();
        assert_eq!(decision.questions[0].header, "Storage");
        assert!(decision
            .fallback_text
            .contains("1. **Storage**: Which database?"));
        assert!(decision
            .fallback_text
            .contains("   - SQLite: SQLite description"));

        for (questions, needle) in [
            (Vec::new(), "1 to 4"),
            (vec![question("Storage", vec![option("One")])], "2 to 4"),
            (
                vec![question(
                    "A header too long",
                    vec![option("A"), option("B")],
                )],
                "at most 12",
            ),
            (
                vec![question("Dup", vec![option("A"), option("a")])],
                "repeats",
            ),
            (
                vec![question(" ", vec![option("A"), option("B")])],
                "must not be empty",
            ),
        ] {
            let error = UserDecisionV1::ask_user("d".into(), questions).unwrap_err();
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn exit_plan_mode_keeps_the_plan_and_responses_fit_their_kind() {
        let decision = UserDecisionV1::exit_plan_mode("p1".into(), "\n1. Edit a.rs\n").unwrap();
        assert_eq!(decision.plan.as_deref(), Some("1. Edit a.rs"));
        assert!(decision
            .fallback_text
            .starts_with("**Proposed plan**\n\n1. Edit a.rs"));
        assert!(UserDecisionV1::exit_plan_mode("p2".into(), "  ").is_err());
        assert!(UserDecisionResponseV1::ApprovePlan.fits(UserDecisionKindV1::ExitPlanMode));
        assert!(!UserDecisionResponseV1::Answer.fits(UserDecisionKindV1::ExitPlanMode));
        assert!(!UserDecisionResponseV1::RevisePlan.fits(UserDecisionKindV1::AskUser));
        let wire = serde_json::to_value(&decision).unwrap();
        assert_eq!(wire["kind"], "exit_plan_mode");
        assert!(wire.get("questions").is_some());
    }
}
