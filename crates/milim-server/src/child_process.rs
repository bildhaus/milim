use std::io;
#[cfg(windows)]
use std::path::PathBuf;
use std::process::{Output, Stdio};
use std::time::Duration;

use milim_core::proc::ProcessTreeGuard;
use tokio::process::Command;

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
