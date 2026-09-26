//! Helpers shared by the account runtime bridges (Claude, Codex, OpenCode, Pi).

use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::account_runtime_events::serialize_runtime_event;
use crate::codex_bridge::AccountWorkerEvent;

/// Normalizes a requested tool approval policy to `review`, `open`, or
/// `guarded`. Anything else, including no policy, is `guarded`.
///
/// The OpenCode and Pi bridges match the requested policy exactly instead, so
/// a missing or unrecognized policy is not treated as `guarded` there.
pub(crate) fn account_runtime_policy(value: Option<&str>) -> &'static str {
    match value.map(str::trim) {
        Some("review") => "review",
        Some("open") => "open",
        _ => "guarded",
    }
}

/// Whether a turn may run tools at all: never in plan mode, and under `review`
/// only with a standing grant or a way to ask the user.
pub(crate) fn tools_allowed(
    plan_mode: bool,
    review: bool,
    approval_granted: bool,
    interactive_approval: bool,
) -> bool {
    !plan_mode && (!review || approval_granted || interactive_approval)
}

pub(crate) fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn compact_json(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(value.to_string())
}

pub(crate) fn cli_path_warning(label: &str, command: &str, install: &str) -> String {
    format!("{label} CLI was not found on PATH. Apps launched from the Dock or Finder do not inherit your shell PATH, so on macOS and Linux milim also reads your login shell's PATH and looks in the usual install directories (`~/.local/bin`, Homebrew, `~/.bun/bin`, and asdf/mise/volta shims). Install it with `{install}`, or use Locate binary... in Providers to choose the `{command}` executable.")
}

pub(crate) fn is_cli_path_warning(message: &str) -> bool {
    message.contains("CLI was not found on PATH")
}

/// A native runtime event that may also report Managed Worker progress.
pub(crate) trait AccountWorkerEventSource: Serialize {
    fn account_worker_event(&self) -> Option<AccountWorkerEvent>;
}

/// Serializes a runtime event, first publishing its Worker update, if any.
pub(crate) fn runtime_event_with_worker<T: AccountWorkerEventSource>(
    value: &T,
    worker_events: &Option<UnboundedSender<AccountWorkerEvent>>,
) -> Value {
    if let Some(event) = value.account_worker_event() {
        publish_worker(worker_events, event);
    }
    serialize_runtime_event(value)
}

pub(crate) fn publish_worker(
    worker_events: &Option<UnboundedSender<AccountWorkerEvent>>,
    event: AccountWorkerEvent,
) {
    if let Some(worker_events) = worker_events {
        let _ = worker_events.send(event);
    }
}
