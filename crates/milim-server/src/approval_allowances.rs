//! Per-thread "Allow for this chat" approval rules.
//!
//! A rule is created only by an explicit `approval.resolve` with
//! `scope: "thread"`. It is keyed by tool name, except for command-bearing
//! requests, which are keyed by the exact command string so a shell tool is
//! never blanket-allowed. With `allowance_match: "prefix"`, a simple command
//! from a short list of common developer tools (`cargo test`, `npm run`,
//! `git diff`, ...) is keyed by its first two words instead, and covers later
//! simple commands that start with those words. Chained, piped, redirected,
//! or substituted commands never match a prefix rule. Rules live in canonical
//! user state rather than in the desktop-synchronized session JSON, so a
//! stale client snapshot cannot drop or invent them.

use milim_core::Result;
use milim_storage::UserDataStore;
use milim_tools::shell_command::{prefix_matches, simple_words, ShellDialect};
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

/// Programs whose `<program> <subcommand>` families can be allowed as a
/// prefix, with the subcommands that qualify.
const PREFIX_COMMANDS: &[(&str, &[&str])] = &[
    (
        "cargo",
        &[
            "bench", "build", "check", "clippy", "doc", "fmt", "nextest", "test", "tree",
        ],
    ),
    ("npm", &["run", "test"]),
    ("pnpm", &["build", "lint", "run", "test"]),
    ("yarn", &["build", "lint", "run", "test"]),
    ("bun", &["run", "test"]),
    ("go", &["build", "test", "vet"]),
    ("dotnet", &["build", "test"]),
    ("git", &["diff", "log", "show", "status"]),
];

/// POSIX shells whose `<shell> -c <script>` wrapper is looked through, as
/// account runtimes such as Codex report commands that way.
const WRAPPER_SHELLS: &[&str] = &["bash", "sh", "zsh"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ApprovalAllowance {
    /// Stable identity: `tool:<name>`, `command:<exact command>`, or
    /// `prefix:<leading words>`.
    pub key: String,
    /// Tool name as the runtime reported it when the rule was created.
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Leading words a later simple command must start with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
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
            prefix: None,
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
        prefix: None,
        created_at_ms: 0,
    })
}

/// The prefix rule "Allow for this chat" could create for this request, or
/// `None` when it is not a simple command of a listed developer tool.
pub(crate) fn prefix_allowance_for(
    kind: &str,
    name: &str,
    arguments: &str,
) -> Option<ApprovalAllowance> {
    if kind != "command" {
        return None;
    }
    let tool = name.trim();
    let command = command_string(arguments)?;
    if tool.is_empty() || command.chars().count() > MAX_COMMAND_CHARS {
        return None;
    }
    let dialect = dialect_for(tool);
    let words = simple_words(&effective_command(&command, dialect), dialect)?;
    let [program, subcommand, ..] = words.as_slice() else {
        return None;
    };
    PREFIX_COMMANDS
        .iter()
        .any(|(candidate, subcommands)| {
            candidate == program && subcommands.contains(&subcommand.as_str())
        })
        .then(|| ApprovalAllowance {
            key: format!("prefix:{program} {subcommand}"),
            tool: tool.to_string(),
            command: None,
            prefix: Some(format!("{program} {subcommand}")),
            created_at_ms: 0,
        })
}

fn dialect_for(tool: &str) -> ShellDialect {
    let lowered = tool.to_ascii_lowercase();
    if lowered.contains("powershell") {
        ShellDialect::PowerShell
    } else if lowered == "bash" {
        ShellDialect::Posix
    } else {
        ShellDialect::host()
    }
}

/// The script inside a `<shell> -c <script>` wrapper, or the command itself.
fn effective_command(command: &str, dialect: ShellDialect) -> String {
    if dialect == ShellDialect::Posix {
        if let Some(words) = simple_words(command, dialect) {
            if let [shell, flag, script] = words.as_slice() {
                let shell = shell.rsplit('/').next().unwrap_or(shell);
                if WRAPPER_SHELLS.contains(&shell) && matches!(flag.as_str(), "-c" | "-lc") {
                    return script.clone();
                }
            }
        }
    }
    command.to_string()
}

fn is_shell_like(name: &str) -> bool {
    name.contains("shell") || name.contains("bash") || name.contains("terminal")
}

fn command_value(arguments: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(arguments).ok()?;
    value
        .get("command")
        .or_else(|| value.get("cmd"))
        .or_else(|| value.pointer("/input/command"))
        .cloned()
}

/// The command line a request would run, when it is a single string.
fn command_string(arguments: &str) -> Option<String> {
    let text = command_value(arguments)?.as_str()?.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// The exact command a request would run, if its arguments carry one.
fn command_text(arguments: &str) -> Option<String> {
    let command = command_value(arguments)?;
    let text = match &command {
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
    if let Some(exact) = allowances.iter().find(|item| item.key == candidate.key) {
        return Some(exact.clone());
    }
    if kind != "command" {
        return None;
    }
    let command = command_string(arguments)?;
    let dialect = dialect_for(name.trim());
    let command = effective_command(&command, dialect);
    allowances
        .iter()
        .find(|item| {
            item.prefix
                .as_deref()
                .is_some_and(|prefix| prefix_matches(prefix, &command, dialect))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn prefix_rules_cover_arguments_but_never_chaining() {
        let rule = prefix_allowance_for("command", "shell", r#"{"command":"cargo test -p core"}"#)
            .expect("prefix rule");
        assert_eq!(rule.key, "prefix:cargo test");
        assert_eq!(rule.prefix.as_deref(), Some("cargo test"));
        let rules = vec![rule];
        for covered in [
            "cargo test",
            "cargo test --release -- parser",
            "cargo  test 2>&1",
        ] {
            let arguments = json!({ "command": covered }).to_string();
            assert!(
                matching(&rules, "command", "shell", &arguments).is_some(),
                "{covered}"
            );
        }
        for refused in [
            "cargo test && rm -rf target",
            "cargo test; curl example.invalid",
            "cargo test | tee out.txt",
            "cargo test > out.txt",
            "cargo test $(whoami)",
            "cargo build",
            "cargo",
        ] {
            let arguments = json!({ "command": refused }).to_string();
            assert!(
                matching(&rules, "command", "shell", &arguments).is_none(),
                "{refused}"
            );
        }
        // Account runtimes wrap commands in a shell; the script is what counts.
        assert!(matching(
            &rules,
            "command",
            "command",
            r#"{"command":"/bin/zsh -lc 'cargo test --workspace'"}"#
        )
        .is_some());
        assert!(matching(
            &rules,
            "command",
            "command",
            r#"{"command":"/bin/zsh -lc 'cargo test && rm -rf ~'"}"#
        )
        .is_none());
        assert!(matching(
            &rules,
            "file_change",
            "shell",
            r#"{"command":"cargo test"}"#
        )
        .is_none());
    }

    #[test]
    fn prefix_rules_need_a_simple_command_of_a_listed_tool() {
        assert!(
            prefix_allowance_for("command", "shell", r#"{"command":"rm -rf target"}"#).is_none()
        );
        assert!(
            prefix_allowance_for("command", "shell", r#"{"command":"git push origin"}"#).is_none()
        );
        assert!(
            prefix_allowance_for("command", "shell", r#"{"command":"cargo test && ls"}"#).is_none()
        );
        assert!(
            prefix_allowance_for("command", "shell", r#"{"command":["cargo","test"]}"#).is_none()
        );
        assert!(
            prefix_allowance_for("file_change", "shell", r#"{"command":"cargo test"}"#).is_none()
        );
        assert_eq!(
            prefix_allowance_for("command", "Bash", r#"{"command":"pnpm test --filter ui"}"#)
                .and_then(|rule| rule.prefix),
            Some("pnpm test".into())
        );
        assert_eq!(
            prefix_allowance_for(
                "command",
                "command",
                r#"{"command":"bash -c 'npm run lint'"}"#
            )
            .and_then(|rule| rule.prefix),
            Some("npm run".into())
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
