use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use milim_core::{Error, Result};
use serde_json::{json, Value};
use tokio::process::Command;

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountRuntime {
    Codex,
    Claude,
    OpenCode,
    Pi,
}

impl AccountRuntime {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    fn update_args(self) -> &'static [&'static str] {
        match self {
            Self::Codex => &["update"],
            Self::Claude => &["update"],
            Self::OpenCode => &["upgrade", "--pure"],
            Self::Pi => &["update", "self", "--no-approve"],
        }
    }
}

pub(crate) async fn statuses() -> Value {
    let (codex, claude, opencode, pi) = tokio::join!(
        status(AccountRuntime::Codex),
        status(AccountRuntime::Claude),
        status(AccountRuntime::OpenCode),
        status(AccountRuntime::Pi),
    );
    json!({
        "runtimes": {
            "codex": codex,
            "claude": claude,
            "opencode": opencode,
            "pi": pi,
        }
    })
}

pub(crate) async fn update(runtime: AccountRuntime) -> Result<Value> {
    let previous_version = installed_version(runtime).await?;
    let mut command = runtime_command(runtime.command());
    command
        .args(runtime.update_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = tokio::time::timeout(UPDATE_TIMEOUT, command.output())
        .await
        .map_err(|_| Error::Upstream(format!("{} update timed out.", runtime.id())))?
        .map_err(|error| {
            Error::Upstream(format!("Could not start {} update: {error}", runtime.id()))
        })?;
    let detail = command_detail(&output.stdout, &output.stderr);
    if !output.status.success() {
        return Err(Error::Upstream(if detail.is_empty() {
            format!("{} update failed with {}.", runtime.id(), output.status)
        } else {
            detail
        }));
    }
    let version = installed_version(runtime).await?;
    Ok(json!({
        "runtime": runtime.id(),
        "previous_version": previous_version,
        "version": version,
        "updated": previous_version != version,
        "message": detail,
    }))
}

async fn status(runtime: AccountRuntime) -> Value {
    match installed_version(runtime).await {
        Ok(version) => json!({ "available": true, "version": version }),
        Err(error) => json!({
            "available": false,
            "version": Value::Null,
            "error": error.to_string(),
        }),
    }
}

async fn installed_version(runtime: AccountRuntime) -> Result<String> {
    let mut command = runtime_command(runtime.command());
    command
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .map_err(|_| Error::Upstream(format!("{} version check timed out.", runtime.id())))?
        .map_err(|error| {
            Error::Upstream(format!("{} CLI is unavailable: {error}", runtime.id()))
        })?;
    if !output.status.success() {
        return Err(Error::Upstream(command_detail(
            &output.stdout,
            &output.stderr,
        )));
    }
    normalize_version(&command_detail(&output.stdout, &output.stderr))
        .ok_or_else(|| Error::Upstream(format!("{} returned no version.", runtime.id())))
}

fn normalize_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .map(|part| {
            part.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '-')
                .to_string()
        })
        .filter(|version| !version.is_empty())
}

fn command_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    truncate(if stdout.is_empty() { stderr } else { stdout }, 4_000)
}

fn truncate(mut value: String, max_chars: usize) -> String {
    let Some((index, _)) = value.char_indices().nth(max_chars) else {
        return value;
    };
    value.truncate(index);
    value.push('…');
    value
}

#[cfg(windows)]
fn runtime_command(name: &str) -> Command {
    if let Some(path) = find_on_path(&format!("{name}.cmd")) {
        let mut command = Command::new("cmd");
        command.arg("/D").arg("/S").arg("/C").arg(path);
        return command;
    }
    if let Some(path) = find_on_path(&format!("{name}.exe")) {
        return Command::new(path);
    }
    Command::new(name)
}

#[cfg(not(windows))]
fn runtime_command(name: &str) -> Command {
    Command::new(name)
}

#[cfg(windows)]
fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_commands_use_runtime_owned_updaters() {
        assert_eq!(AccountRuntime::Codex.update_args(), ["update"]);
        assert_eq!(AccountRuntime::Claude.update_args(), ["update"]);
        assert_eq!(
            AccountRuntime::OpenCode.update_args(),
            ["upgrade", "--pure"]
        );
        assert_eq!(
            AccountRuntime::Pi.update_args(),
            ["update", "self", "--no-approve"]
        );
    }

    #[test]
    fn versions_are_normalized_for_cards() {
        assert_eq!(
            normalize_version("codex-cli 0.144.3\n").as_deref(),
            Some("0.144.3")
        );
        assert_eq!(
            normalize_version("2.1.215 (Claude Code)").as_deref(),
            Some("2.1.215")
        );
        assert_eq!(normalize_version("no version"), None);
    }
}
