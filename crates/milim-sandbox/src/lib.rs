//! `milim-sandbox` — run agent commands in an isolated container.
//!
//! Phase 4. milim isolates agent code in a Linux VM via Apple's
//! Containerization framework (macOS-only). The cross-platform replacement runs
//! each command through Docker in a `--rm --network none` container.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use milim_core::{Error, Result};

/// Options for a sandboxed run.
#[derive(Debug, Clone)]
pub struct RunOpts {
    /// Allow network access inside the container (off by default).
    pub network: bool,
    /// Wall-clock timeout.
    pub timeout: Duration,
    /// Optional working directory inside the container.
    pub workdir: Option<String>,
    /// Memory cap (e.g. `"512m"`); None uses the daemon default.
    pub memory: Option<String>,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self {
            network: false,
            timeout: Duration::from_secs(60),
            workdir: None,
            memory: Some("512m".to_string()),
        }
    }
}

/// The captured result of a sandboxed run.
#[derive(Debug, Clone, Serialize)]
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub exit_code: Option<i32>,
}

/// Runs commands via the `docker` CLI.
#[derive(Debug, Clone)]
pub struct DockerBackend {
    docker_bin: String,
}

impl Default for DockerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DockerBackend {
    pub fn new() -> Self {
        Self {
            docker_bin: non_empty_env("MILIM_DOCKER_BIN").unwrap_or_else(|| "docker".to_string()),
        }
    }

    /// Whether the Docker daemon is reachable.
    pub async fn available(&self) -> bool {
        let mut version_cmd = Command::new(&self.docker_bin);
        version_cmd
            .args(["version", "--format", "{{.Server.Version}}"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        version_cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        if !command_succeeds_within(&mut version_cmd, Duration::from_secs(5)).await {
            return false;
        }

        let mut probe_cmd = Command::new(&self.docker_bin);
        probe_cmd
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--memory",
                "512m",
                "alpine",
                "true",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        probe_cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        command_succeeds_within(&mut probe_cmd, Duration::from_secs(5)).await
    }
}

async fn command_succeeds_within(command: &mut Command, timeout: Duration) -> bool {
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(_)) => false,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            false
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

impl DockerBackend {
    /// Run `command` inside `image`, returning captured output.
    pub async fn run(
        &self,
        image: &str,
        command: &[String],
        opts: &RunOpts,
    ) -> Result<SandboxOutput> {
        const MAX_OUTPUT: usize = 1024 * 1024;
        static NEXT_CONTAINER: AtomicU64 = AtomicU64::new(0);
        let container_name = format!(
            "milim-sandbox-{}-{}",
            std::process::id(),
            NEXT_CONTAINER.fetch_add(1, Ordering::Relaxed)
        );
        let mut args: Vec<String> = vec![
            "run".into(),
            "--rm".into(),
            "--name".into(),
            container_name.clone(),
            "--pids-limit".into(),
            "128".into(),
            "--cpus".into(),
            "1".into(),
            "--read-only".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--tmpfs".into(),
            "/tmp:rw,noexec,nosuid,size=64m".into(),
        ];
        if !opts.network {
            args.push("--network".into());
            args.push("none".into());
        }
        if let Some(mem) = &opts.memory {
            args.push("--memory".into());
            args.push(mem.clone());
        }
        if let Some(dir) = &opts.workdir {
            args.push("--workdir".into());
            args.push(dir.clone());
        }
        args.push(image.to_string());
        args.extend(command.iter().cloned());

        let mut cmd = Command::new(&self.docker_bin);
        cmd.args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Other(format!("docker run failed to start: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("docker stdout unavailable".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Other("docker stderr unavailable".into()))?;
        let stdout_task = tokio::spawn(read_bounded(stdout, MAX_OUTPUT));
        let stderr_task = tokio::spawn(read_bounded(stderr, MAX_OUTPUT));
        let mut guard = ContainerGuard::new(self.docker_bin.clone(), container_name);
        let status = match tokio::time::timeout(opts.timeout, child.wait()).await {
            Ok(result) => {
                result.map_err(|error| Error::Other(format!("docker run failed: {error}")))?
            }
            Err(_) => {
                guard.terminate().await;
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(Error::Other(format!(
                    "sandbox run timed out after {:?}",
                    opts.timeout
                )));
            }
        };
        guard.disarm();
        let (stdout, stdout_truncated) = stdout_task
            .await
            .map_err(|error| Error::Other(format!("docker stdout task failed: {error}")))??;
        let (stderr, stderr_truncated) = stderr_task
            .await
            .map_err(|error| Error::Other(format!("docker stderr task failed: {error}")))??;

        Ok(SandboxOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            stdout_truncated,
            stderr_truncated,
            exit_code: status.code(),
        })
    }
}

async fn read_bounded<R>(mut reader: R, limit: usize) -> Result<(Vec<u8>, bool)>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok((kept, truncated))
}

struct ContainerGuard {
    docker_bin: String,
    name: Option<String>,
}

impl ContainerGuard {
    fn new(docker_bin: String, name: String) -> Self {
        Self {
            docker_bin,
            name: Some(name),
        }
    }

    fn disarm(&mut self) {
        self.name = None;
    }

    async fn terminate(&mut self) {
        let Some(name) = self.name.take() else { return };
        let mut command = Command::new(&self.docker_bin);
        command.args(["rm", "-f", &name]);
        #[cfg(windows)]
        command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        let _ = tokio::time::timeout(Duration::from_secs(10), command.output()).await;
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let Some(name) = self.name.take() else { return };
        let mut command = std::process::Command::new(&self.docker_bin);
        command.args(["rm", "-f", &name]);
        let _ = milim_core::proc::hide_console(&mut command).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn builds_default_opts() {
        let o = RunOpts::default();
        assert!(!o.network);
        assert_eq!(o.memory.as_deref(), Some("512m"));
    }

    #[tokio::test]
    async fn available_requires_a_runnable_linux_container() {
        let fake_docker = fake_docker_that_cannot_run_containers();
        let docker = DockerBackend {
            docker_bin: fake_docker.to_string_lossy().into_owned(),
        };

        assert!(!docker.available().await);
    }

    // Gated on a reachable Docker daemon.
    #[tokio::test]
    async fn runs_echo_in_alpine() {
        let docker = DockerBackend::new();
        if !docker.available().await {
            eprintln!("docker not available; skipping sandbox test");
            return;
        }
        let out = docker
            .run(
                "alpine",
                &["echo".into(), "hello-sandbox".into()],
                &RunOpts::default(),
            )
            .await
            .unwrap();
        assert!(
            out.stdout.contains("hello-sandbox"),
            "stdout: {:?}",
            out.stdout
        );
        assert_eq!(out.exit_code, Some(0));
    }

    fn fake_docker_that_cannot_run_containers() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "milim-fake-docker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        #[cfg(windows)]
        {
            let path = dir.join("docker.cmd");
            fs::write(
                &path,
                "@echo off\r\nif \"%1\"==\"version\" (echo 29.0.0& exit /b 0)\r\nif \"%1\"==\"run\" (echo cannot run linux image 1>&2& exit /b 125)\r\nexit /b 1\r\n",
            )
            .unwrap();
            path
        }

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            let path = dir.join("docker");
            fs::write(
                &path,
                "#!/bin/sh\nif [ \"$1\" = \"version\" ]; then echo 29.0.0; exit 0; fi\nif [ \"$1\" = \"run\" ]; then echo cannot run linux image >&2; exit 125; fi\nexit 1\n",
            )
            .unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).unwrap();
            path
        }
    }
}
