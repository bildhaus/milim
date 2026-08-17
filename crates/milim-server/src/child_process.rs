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
    command.envs(std::env::vars_os());
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
}
