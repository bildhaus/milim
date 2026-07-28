#[cfg(windows)]
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

    fn latest_url(self) -> &'static str {
        match self {
            Self::Codex => "https://registry.npmjs.org/%40openai%2Fcodex/latest",
            Self::Claude => "https://registry.npmjs.org/%40anthropic-ai%2Fclaude-code/latest",
            Self::OpenCode => "https://registry.npmjs.org/opencode-ai/latest",
            Self::Pi => "https://registry.npmjs.org/%40earendil-works%2Fpi-coding-agent/latest",
        }
    }
}

pub(crate) async fn statuses() -> Value {
    let client = reqwest::Client::new();
    let (codex, claude, opencode, pi) = tokio::join!(
        status(&client, AccountRuntime::Codex),
        status(&client, AccountRuntime::Claude),
        status(&client, AccountRuntime::OpenCode),
        status(&client, AccountRuntime::Pi),
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

async fn status(client: &reqwest::Client, runtime: AccountRuntime) -> Value {
    match installed_version(runtime).await {
        Ok(version) => match latest_version(client, runtime).await {
            Ok(latest_version) => {
                let update_available = version_is_newer(&version, &latest_version);
                json!({
                    "available": true,
                    "version": version,
                    "latest_version": latest_version,
                    "update_available": update_available,
                    "update_error": if update_available.is_none() {
                        Value::String("Could not compare installed and latest versions.".into())
                    } else {
                        Value::Null
                    },
                })
            }
            Err(error) => json!({
                "available": true,
                "version": version,
                "latest_version": Value::Null,
                "update_available": Value::Null,
                "update_error": error.to_string(),
            }),
        },
        Err(error) => json!({
            "available": false,
            "version": Value::Null,
            "latest_version": Value::Null,
            "update_available": Value::Null,
            "error": error.to_string(),
        }),
    }
}

async fn latest_version(client: &reqwest::Client, runtime: AccountRuntime) -> Result<String> {
    tokio::time::timeout(VERSION_TIMEOUT, async {
        let response = client
            .get(runtime.latest_url())
            .send()
            .await
            .map_err(|error| {
                Error::Upstream(format!(
                    "Could not check the latest {} version: {error}",
                    runtime.id()
                ))
            })?;
        if !response.status().is_success() {
            return Err(Error::Upstream(format!(
                "Latest {} version check returned {}.",
                runtime.id(),
                response.status()
            )));
        }
        let payload: Value = response.json().await.map_err(|error| {
            Error::Upstream(format!(
                "Latest {} version response was invalid: {error}",
                runtime.id()
            ))
        })?;
        payload
            .get("version")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                Error::Upstream(format!(
                    "Latest {} version response had no version.",
                    runtime.id()
                ))
            })
    })
    .await
    .map_err(|_| Error::Upstream(format!("Latest {} version check timed out.", runtime.id())))?
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

fn version_is_newer(installed: &str, latest: &str) -> Option<bool> {
    let (installed_parts, installed_pre) = version_parts(installed)?;
    let (latest_parts, latest_pre) = version_parts(latest)?;
    for index in 0..installed_parts.len().max(latest_parts.len()) {
        let installed_part = installed_parts.get(index).copied().unwrap_or(0);
        let latest_part = latest_parts.get(index).copied().unwrap_or(0);
        if installed_part != latest_part {
            return Some(latest_part > installed_part);
        }
    }
    match (installed_pre, latest_pre) {
        (Some(_), None) => Some(true),
        (None, Some(_)) => Some(false),
        (None, None) => Some(false),
        (Some(installed), Some(latest)) if installed == latest => Some(false),
        (Some(_), Some(_)) => None,
    }
}

fn version_parts(version: &str) -> Option<(Vec<u64>, Option<&str>)> {
    let version = version.trim_start_matches('v').split('+').next()?;
    let (core, prerelease) = version
        .split_once('-')
        .map_or((version, None), |(core, prerelease)| {
            (core, Some(prerelease))
        });
    let parts = core
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<Vec<u64>, _>>()
        .ok()?;
    (!parts.is_empty()).then_some((parts, prerelease))
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

    #[test]
    fn update_availability_compares_release_versions() {
        assert_eq!(version_is_newer("1.18.7", "1.18.9"), Some(true));
        assert_eq!(version_is_newer("2.1.220", "2.1.220"), Some(false));
        assert_eq!(version_is_newer("0.146.0", "0.145.0"), Some(false));
        assert_eq!(version_is_newer("1.0.0-beta.1", "1.0.0"), Some(true));
        assert_eq!(version_is_newer("unknown", "1.0.0"), None);
    }
}
