use std::io;
#[cfg(windows)]
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use milim_core::proc::ProcessTreeGuard;
use tokio::process::Command;

/// Account runtimes are trusted user-owned CLIs. Snapshot the complete user
/// environment, then let each adapter layer its own explicit overrides on top.
pub(crate) fn account_runtime_inherited(mut command: Command) -> Command {
    // `Command` inherits the parent environment by default. Snapshot the
    // remaining variables explicitly, but preserve a PATH already supplied by
    // `cli_path::command`; GUI launches need that enriched PATH for helpers
    // such as `node`, and applying the parent's minimal PATH here would replace
    // it.
    command.envs(
        std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().eq_ignore_ascii_case("PATH")),
    );
    command
}

/// Run a short-lived Milim-owned command without letting cancellation orphan it.
pub(crate) async fn output(mut command: Command) -> io::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    #[cfg(unix)]
    command.process_group(0);

    let child = command.spawn()?;
    let _tree = ProcessTreeGuard::attach(
        child
            .id()
            .ok_or_else(|| io::Error::other("child process id was unavailable"))?,
    )?;
    child.wait_with_output().await
}

pub(crate) async fn wait_or_kill(child: &mut tokio::process::Child, grace: Duration) {
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        let _ = child.kill().await;
    }
}

#[cfg(windows)]
pub(crate) fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn account_runtime_processes_inherit_the_user_environment() {
        let key = format!("MILIM_ACCOUNT_ENV_PROOF_{}", std::process::id());
        let value = "account-environment-visible";
        std::env::set_var(&key, value);
        let command = if cfg!(windows) {
            let mut command = Command::new("powershell");
            let script = format!("[Console]::Write($env:{key})");
            command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            command
        } else {
            let mut command = Command::new("sh");
            let script = format!("printf '%s' \"${key}\"");
            command.args(["-c", &script]);
            command
        };
        let output = output(account_runtime_inherited(command)).await.unwrap();
        std::env::remove_var(&key);
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), value);
    }

    #[tokio::test]
    async fn account_runtime_preserves_an_explicit_child_path() {
        #[cfg(windows)]
        let (expected, mut command) = {
            let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let mut command = Command::new(powershell);
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Write($env:PATH)",
            ]);
            (r"C:\milim\account-runtime\bin", command)
        };
        #[cfg(not(windows))]
        let (expected, mut command) = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "printf '%s' \"$PATH\""]);
            ("/milim/account-runtime/bin", command)
        };
        command.env("PATH", expected);
        let output = output(account_runtime_inherited(command)).await.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}
