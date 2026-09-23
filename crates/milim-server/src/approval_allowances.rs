//! Per-thread "Allow for this chat" approval rules.
//!
//! A rule is created only by an explicit `approval.resolve` with
//! `scope: "thread"`. It is keyed by tool name, except for command-bearing
//! requests, which are keyed by the exact command string so a shell tool is
//! never blanket-allowed. Rules live in canonical user state rather than in
//! the desktop-synchronized session JSON, so a stale client snapshot cannot
//! drop or invent them.

use milim_core::Result;
use milim_storage::UserDataStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const STATE_KEY_PREFIX: &str = "milim.approvalAllowances.";
const MAX_ALLOWANCES_PER_THREAD: usize = 64;
const MAX_COMMAND_CHARS: usize = 4_096;
pub(crate) const ALLOWANCES_EVENT_TYPE: &str = "approval_allowances.updated";

/// Tool names that run arbitrary shell input. Without an exact command they
/// cannot be allowed for a chat.
const SHELL_TOOL_NAMES: &[&str] = &[
    "shell",
    "bash",
    "command",
    "sh",
    "zsh",
    "powershell",
    "terminal",
    "exec",
    "exec_command",
    "run_command",
    "run_shell",
    "shell_command",
    "execute_command",
    "local_shell",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalAllowance {
    /// Stable identity: `tool:<name>` or `command:<exact command>`.
    pub key: String,
    /// Tool name as the runtime reported it when the rule was created.
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub created_at_ms: i64,
}

/// Approval kinds that can carry a chat-wide allowance. Permission
/// elevations and MCP forms/links always stay one-shot.
pub(crate) fn scope_supported(kind: &str) -> bool {
    matches!(kind, "command" | "file_change")
}

/// Derive the rule an approval would create, or `None` when the request
/// cannot be safely allowed for the whole chat.
pub(crate) fn allowance_for(kind: &str, name: &str, arguments: &str) -> Option<ApprovalAllowance> {
    if !scope_supported(kind) {
        return None;
    }
    let tool = name.trim();
    if tool.is_empty() {
        return None;
    }
    if let Some(command) = command_text(arguments) {
        if command.chars().count() > MAX_COMMAND_CHARS {
            return None;
        }
        return Some(ApprovalAllowance {
            key: format!("command:{command}"),
            tool: tool.to_string(),
            command: Some(command),
            created_at_ms: 0,
        });
    }
    let lowered = tool.to_ascii_lowercase();
    if SHELL_TOOL_NAMES.contains(&lowered.as_str()) || is_shell_like(&lowered) {
        return None;
    }
    Some(ApprovalAllowance {
        key: format!("tool:{lowered}"),
        tool: tool.to_string(),
        command: None,
        created_at_ms: 0,
    })
}

fn is_shell_like(name: &str) -> bool {
    name.contains("shell") || name.contains("bash") || name.contains("terminal")
}

/// The exact command a request would run, if its arguments carry one.
fn command_text(arguments: &str) -> Option<String> {
    let value: Value = serde_json::from_str(arguments).ok()?;
    let command = value
        .get("command")
        .or_else(|| value.get("cmd"))
        .or_else(|| value.pointer("/input/command"))?;
    let text = match command {
        Value::String(text) => text.trim().to_string(),
        // Argument vectors keep their exact boundaries: `["a b"]` and
        // `["a", "b"]` are different commands.
        Value::Array(parts) if !parts.is_empty() && parts.iter().all(Value::is_string) => {
            serde_json::to_string(parts).ok()?
        }
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn state_key(thread_id: &str) -> String {
    format!("{STATE_KEY_PREFIX}{thread_id}")
}

pub(crate) fn load(store: &UserDataStore, thread_id: &str) -> Result<Vec<ApprovalAllowance>> {
    Ok(store
        .get_json(&state_key(thread_id))?
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn save(store: &UserDataStore, thread_id: &str, allowances: &[ApprovalAllowance]) -> Result<()> {
    if allowances.is_empty() {
        store.delete_json(&state_key(thread_id))?;
        return Ok(());
    }
    let value = serde_json::to_string(allowances)
        .map_err(|error| milim_core::Error::Other(format!("serialize allowances: {error}")))?;
    store.set_json(&state_key(thread_id), &value)
}

/// Record a rule. Returns the thread's allowances when they changed.
pub(crate) fn record(
    store: &UserDataStore,
    thread_id: &str,
    mut allowance: ApprovalAllowance,
    now_ms: i64,
) -> Result<Option<Vec<ApprovalAllowance>>> {
    let mut allowances = load(store, thread_id)?;
    if allowances.iter().any(|item| item.key == allowance.key) {
        return Ok(None);
    }
    allowance.created_at_ms = now_ms;
    allowances.push(allowance);
    if allowances.len() > MAX_ALLOWANCES_PER_THREAD {
        let overflow = allowances.len() - MAX_ALLOWANCES_PER_THREAD;
        allowances.drain(..overflow);
    }
    save(store, thread_id, &allowances)?;
    Ok(Some(allowances))
}

/// Remove the listed rules, or every rule when `keys` is `None`. Returns the
/// remaining allowances.
pub(crate) fn revoke(
    store: &UserDataStore,
    thread_id: &str,
    keys: Option<&[String]>,
) -> Result<Vec<ApprovalAllowance>> {
    let mut allowances = load(store, thread_id)?;
    match keys {
        None => allowances.clear(),
        Some(keys) => allowances.retain(|item| !keys.contains(&item.key)),
    }
    save(store, thread_id, &allowances)?;
    Ok(allowances)
}

/// The rule that pre-approves this request, if any.
pub(crate) fn matching(
    allowances: &[ApprovalAllowance],
    kind: &str,
    name: &str,
    arguments: &str,
) -> Option<ApprovalAllowance> {
    let candidate = allowance_for(kind, name, arguments)?;
    allowances
        .iter()
        .find(|item| item.key == candidate.key)
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_requests_are_keyed_by_exact_command() {
        let rule =
            allowance_for("command", "shell", r#"{"command":"cargo test"}"#).expect("command rule");
        assert_eq!(rule.key, "command:cargo test");
        assert_eq!(rule.command.as_deref(), Some("cargo test"));
        let rules = vec![rule];
        assert!(matching(&rules, "command", "shell", r#"{"command":"cargo test"}"#).is_some());
        assert!(matching(&rules, "command", "Bash", r#"{"command":"cargo test"}"#).is_some());
        assert!(matching(
            &rules,
            "command",
            "shell",
            r#"{"command":"cargo test --release"}"#
        )
        .is_none());
        assert!(matching(&rules, "command", "shell", r#"{"command":"rm -rf /"}"#).is_none());
        assert_eq!(
            allowance_for("command", "command", r#"{"command":["git","status"]}"#)
                .map(|rule| rule.key),
            Some(r#"command:["git","status"]"#.into())
        );
    }

    #[test]
    fn shell_tools_without_a_command_are_never_blanket_allowed() {
        assert!(allowance_for("command", "shell", "{}").is_none());
        assert!(allowance_for("command", "Bash", "not json").is_none());
        assert!(allowance_for("command", "local_shell_v2", "{}").is_none());
    }

    #[test]
    fn other_tools_are_keyed_by_name_and_elevations_stay_one_shot() {
        let rule =
            allowance_for("command", "write_file", r#"{"path":"a.txt"}"#).expect("tool rule");
        assert_eq!(rule.key, "tool:write_file");
        assert!(matching(&[rule], "command", "Write_File", r#"{"path":"b.txt"}"#).is_some());
        assert_eq!(
            allowance_for("file_change", "file_change", "{}").map(|rule| rule.key),
            Some("tool:file_change".into())
        );
        assert!(allowance_for("permission_elevation", "permissions", "{}").is_none());
        assert!(allowance_for("mcp_form", "MCP github", "{}").is_none());
    }
}
