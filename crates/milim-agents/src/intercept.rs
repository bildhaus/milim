//! Interception points around the agent loop.
//!
//! A [`ToolInterceptor`] sees each turn before its first model step, each
//! valid tool call before approval and execution, each executed call's
//! result, and the final answer before the loop finishes. milim's user hooks
//! implement it; the loop only applies the decisions and reports the
//! interceptor's [`HookActivity`] as [`crate::AgentEvent::Hook`] events.

use serde::Serialize;
use serde_json::Value;

use milim_core::api::openai::ChatMessage;

/// Stop-hook continuations one run accepts before it finishes anyway.
pub const MAX_STOP_CONTINUATIONS: usize = 3;

/// A validated tool call as an interceptor sees it.
#[derive(Debug, Clone, Copy)]
pub struct InterceptedCall<'a> {
    pub call_id: Option<&'a str>,
    pub name: &'a str,
    /// The tool's other registered names (canonical name and aliases), so a
    /// matcher written for an earlier name still matches.
    pub other_names: &'a [String],
    pub arguments: &'a Value,
}

/// One interceptor action, such as a user hook run, for the run timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookActivity {
    /// `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, or `Stop`.
    pub event: String,
    /// What ran, e.g. the hook command.
    pub hook: String,
    /// Where the hook was configured (`user` or `project`).
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// For example `allow`, `deny`, `approve`, `feedback`, `block`,
    /// `continue`, `error`, `timeout`, or `skipped`.
    pub outcome: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Present when project hooks were skipped until the user trusts them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust: Option<HookTrustRequest>,
}

/// What the user must trust for skipped project hooks to run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookTrustRequest {
    pub workspace: String,
    pub config_hash: String,
}

/// Result of [`ToolInterceptor::before_turn`].
#[derive(Debug, Default)]
pub struct TurnInterception {
    /// Extra model context, added as a system message before the first step.
    pub context: Vec<String>,
    /// Stops the turn before any model request, with this reason.
    pub block: Option<String>,
    pub activity: Vec<HookActivity>,
}

/// What happens to a tool call after [`ToolInterceptor::before_tool`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolDecision {
    /// The normal approval policy applies.
    #[default]
    Continue,
    /// Run without an interactive approval. The tool must already be in the
    /// run's registry, so policy filtering (Guarded, Plan) still applies.
    Approve,
    /// Do not run the call; the model sees this reason.
    Deny(String),
}

/// Result of [`ToolInterceptor::before_tool`].
#[derive(Debug, Default)]
pub struct ToolInterception {
    pub decision: ToolDecision,
    pub activity: Vec<HookActivity>,
}

/// Result of [`ToolInterceptor::after_tool`].
#[derive(Debug, Default)]
pub struct ResultInterception {
    /// Text appended to the model-visible tool result as `[hook] ...`.
    pub feedback: Vec<String>,
    pub activity: Vec<HookActivity>,
}

/// Result of [`ToolInterceptor::on_stop`].
#[derive(Debug, Default)]
pub struct StopInterception {
    /// Keep the run going with this note to the model. At most
    /// [`MAX_STOP_CONTINUATIONS`] are honored per run.
    pub continue_with: Option<String>,
    pub activity: Vec<HookActivity>,
}

#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait::async_trait]
pub trait ToolInterceptor: std::fmt::Debug + Send + Sync {
    /// Runs once per turn, before its first model step.
    async fn before_turn(&self, _messages: &[ChatMessage]) -> TurnInterception {
        TurnInterception::default()
    }

    /// Runs for each valid tool call before approval and execution.
    async fn before_tool(&self, _call: &InterceptedCall<'_>) -> ToolInterception {
        ToolInterception::default()
    }

    /// Runs after each executed tool call, before its result is committed.
    async fn after_tool(&self, _call: &InterceptedCall<'_>, _result: &Value) -> ResultInterception {
        ResultInterception::default()
    }

    /// Runs when the model answered without tool calls and the loop is about
    /// to finish. `continuations` counts earlier stop continuations.
    async fn on_stop(&self, _final_content: &str, _continuations: usize) -> StopInterception {
        StopInterception::default()
    }
}

/// The model-visible tool content with interceptor feedback appended.
pub(crate) fn with_feedback(mut content: String, feedback: &[String]) -> String {
    for text in feedback {
        content.push_str("\n\n[hook] ");
        content.push_str(text);
    }
    content
}

/// The system message carrying turn context from interceptors.
pub(crate) fn context_message(context: &[String]) -> Option<ChatMessage> {
    let text = context
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then(|| {
        ChatMessage::text(
            "system",
            format!(
                "<system-reminder>\nUserPromptSubmit hook context:\n{text}\n</system-reminder>"
            ),
        )
    })
}

/// The user note that continues a run after a stop hook objected.
pub(crate) fn stop_feedback_message(feedback: &str) -> ChatMessage {
    ChatMessage::text("user", format!("[Stop hook feedback]\n{}", feedback.trim()))
}
