use std::io;
use std::process::{Output, Stdio};

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
