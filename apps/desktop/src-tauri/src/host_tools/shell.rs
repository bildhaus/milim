//! `shell`, `process_output`, and `process_kill`: host commands that keep a
//! per-run working directory, plus background processes owned by the run.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use milim_tools::shell_command::{is_read_only, ShellDialect};
use milim_tools::{Tool, ToolConcurrency, ToolEffect};

use super::{
    arg_str, host_tool_scoping, inside_workspace, optional_bool, optional_u64, root_of, HostCtx,
    PathDisplay,
};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
/// Extra time the pipeline allows beyond a command's own timeout, so the
/// shell can kill the process tree and report partial output first.
const DEADLINE_GRACE: Duration = Duration::from_secs(15);
/// Bytes kept per output stream of a foreground command.
const MAX_OUTPUT: usize = 1024 * 1024;
/// Trailing stdout bytes searched for the working-directory marker.
const MARKER_TAIL: usize = 8192;
/// How long output readers may keep draining after the process tree ends.
const READER_DRAIN: Duration = Duration::from_secs(2);
/// Output retained per background process; older output is dropped.
const BACKGROUND_BUFFER: usize = 1024 * 1024;
/// Largest output chunk one `process_output` call returns.
const MAX_OUTPUT_CHUNK: usize = 512 * 1024;
const MAX_RUNNING_BACKGROUND: usize = 16;
const MAX_WAIT_SECS: u64 = 60;
/// How long `process_kill` waits for the exit status after killing.
const KILL_SETTLE: Duration = Duration::from_secs(2);

/// Shell state for one run: the working directory commands start in and the
/// background processes they started. Dropping it drops every
/// [`BackgroundProcess`], whose [`ProcessTreeGuard`] kills its process tree.
#[derive(Default)]
pub(super) struct ShellRunState {
    cwd: Mutex<Option<PathBuf>>,
    processes: Mutex<BTreeMap<String, Arc<BackgroundProcess>>>,
    next_process: AtomicU64,
}

impl ShellRunState {
    fn process(&self, id: &str) -> Result<Arc<BackgroundProcess>> {
        let processes = self
            .processes
            .lock()
            .map_err(|_| Error::Other("background process table poisoned".into()))?;
        processes.get(id).cloned().ok_or_else(|| {
            let known = processes.keys().cloned().collect::<Vec<_>>();
            Error::InvalidRequest(if known.is_empty() {
                format!("unknown process_id {id}; this run has no background processes")
            } else {
                format!(
                    "unknown process_id {id}; this run's processes are {}",
                    known.join(", ")
                )
            })
        })
    }
}

/// Milim's own credentials (remote API keys, OAuth client secrets, API
/// tokens) must not reach model-run commands. Only `MILIM_*` variables whose
/// names look secret are withheld; non-secret settings such as `MILIM_HOME`
/// and the user's unrelated environment stay inherited.
fn milim_secret_env_keys() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .filter_map(|(key, _)| {
            let name = key.to_str()?;
            let milim_owned = name
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("MILIM_"));
            (milim_owned && milim_mcp_client::secret_env_key(name)).then_some(key)
        })
        .collect()
}

/// PowerShell on Windows, `sh -c` elsewhere, in its own process group.
fn shell_command(cwd: &Path, script: &str) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    };
    cmd.current_dir(cwd);
    for key in milim_secret_env_keys() {
        cmd.env_remove(key);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

/// A marker unlikely to appear in command output.
fn unique_marker() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    format!(
        "__MILIM_CWD_{:x}_{:x}_{nanos:x}__",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Append a line that prints `marker` and the final working directory, then
/// exits with the command's own status.
fn wrap_with_cwd_marker(command: &str, marker: &str) -> String {
    if cfg!(windows) {
        format!(
            "{command}\n$__milimOk = $?; $__milimCode = $LASTEXITCODE\n[Console]::Out.Write(\"`n{marker}\" + (Get-Location).ProviderPath + \"`n\")\nif (-not $__milimOk) {{ if ($__milimCode) {{ exit $__milimCode }} else {{ exit 1 }} }}\nexit 0\n"
        )
    } else {
        format!(
            "{command}\n__milim_status=$?\nprintf '\\n{marker}%s\\n' \"$(pwd -P)\"\nexit $__milim_status\n"
        )
    }
}

/// Output of one stream: the first bytes up to a limit plus the stream's tail.
#[derive(Default)]
struct Captured {
    kept: Vec<u8>,
    truncated: bool,
    tail: VecDeque<u8>,
}

async fn capture(mut stream: impl AsyncRead + Unpin, sink: Arc<Mutex<Captured>>, tail_len: usize) {
    let mut buffer = [0_u8; 8192];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        let Ok(mut captured) = sink.lock() else {
            break;
        };
        let chunk = &buffer[..count];
        let room = MAX_OUTPUT.saturating_sub(captured.kept.len());
        captured.kept.extend_from_slice(&chunk[..count.min(room)]);
        captured.truncated |= count > room;
        if tail_len > 0 {
            captured.tail.extend(chunk);
            let excess = captured.tail.len().saturating_sub(tail_len);
            captured.tail.drain(..excess);
        }
    }
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Remove the working-directory marker line from captured stdout and return
/// the directory it reported.
fn take_marker(captured: &mut Captured, marker: &str) -> Option<String> {
    let tail = captured.tail.iter().copied().collect::<Vec<_>>();
    let tail = String::from_utf8_lossy(&tail);
    let at = tail.rfind(marker)?;
    let dir = tail[at + marker.len()..]
        .split('\n')
        .next()?
        .trim_end_matches('\r')
        .to_string();
    if let Some(position) = find_last(&captured.kept, marker.as_bytes()) {
        let start = if position > 0 && captured.kept[position - 1] == b'\n' {
            position - 1
        } else {
            position
        };
        let end = captured.kept[position..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| position + offset + 1)
            .unwrap_or(captured.kept.len());
        captured.kept.drain(start..end);
    }
    (!dir.is_empty()).then_some(dir)
}

struct Foreground {
    stdout: Captured,
    stderr: Captured,
    exit_code: Option<i32>,
    timed_out: bool,
    /// Working directory the command finished in, when it reported one.
    final_dir: Option<String>,
}

async fn run_foreground(cwd: &Path, command: &str, timeout: Duration) -> Result<Foreground> {
    let marker = unique_marker();
    let mut child = shell_command(cwd, &wrap_with_cwd_marker(command, &marker))
        .spawn()
        .map_err(|error| Error::Other(format!("shell failed to start: {error}")))?;
    let mut guard = ProcessTreeGuard::attach(
        child
            .id()
            .ok_or_else(|| Error::Other("shell process id unavailable".into()))?,
    )
    .map_err(|error| Error::Other(format!("failed to contain shell process: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Other("shell stdout unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Other("shell stderr unavailable".into()))?;
    let stdout_sink = Arc::new(Mutex::new(Captured::default()));
    let stderr_sink = Arc::new(Mutex::new(Captured::default()));
    let readers = [
        tokio::spawn(capture(stdout, stdout_sink.clone(), MARKER_TAIL)),
        tokio::spawn(capture(stderr, stderr_sink.clone(), 0)),
    ];
    let (status, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => (Some(status?), false),
        Err(_) => {
            guard.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
            (None, true)
        }
    };
    guard.terminate();
    let abort = readers
        .iter()
        .map(|reader| reader.abort_handle())
        .collect::<Vec<_>>();
    if tokio::time::timeout(READER_DRAIN, join_readers(readers))
        .await
        .is_err()
    {
        abort.iter().for_each(|handle| handle.abort());
    }
    let take = |sink: Arc<Mutex<Captured>>| {
        sink.lock()
            .map(|mut captured| std::mem::take(&mut *captured))
            .unwrap_or_default()
    };
    let mut stdout = take(stdout_sink);
    let stderr = take(stderr_sink);
    let final_dir = take_marker(&mut stdout, &marker);
    Ok(Foreground {
        stdout,
        stderr,
        exit_code: status.and_then(|status| status.code()),
        timed_out,
        final_dir,
    })
}

async fn join_readers(readers: [tokio::task::JoinHandle<()>; 2]) {
    for reader in readers {
        let _ = reader.await;
    }
}

/// Buffered output of a background process, newest `BACKGROUND_BUFFER` bytes.
#[derive(Default)]
struct OutputRing {
    state: Mutex<RingState>,
}

#[derive(Default)]
struct RingState {
    data: VecDeque<u8>,
    /// Bytes ever written.
    total: u64,
    /// Position up to which output has been returned.
    cursor: u64,
    /// `Some(code)` once the process exited.
    exit: Option<Option<i32>>,
    killed: bool,
}

/// Output returned by one read of a background process.
struct Chunk {
    text: String,
    dropped: u64,
    exit: Option<Option<i32>>,
    killed: bool,
}

impl OutputRing {
    fn push(&self, bytes: &[u8]) {
        if let Ok(mut state) = self.state.lock() {
            state.data.extend(bytes);
            let excess = state.data.len().saturating_sub(BACKGROUND_BUFFER);
            state.data.drain(..excess);
            state.total += bytes.len() as u64;
        }
    }

    fn finish(&self, code: Option<i32>) {
        if let Ok(mut state) = self.state.lock() {
            state.exit = Some(code);
        }
    }

    fn mark_killed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.killed = true;
        }
    }

    fn exited(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.exit.is_some())
            .unwrap_or(true)
    }

    /// Output written since the previous read.
    fn take_new(&self) -> Chunk {
        let Ok(mut state) = self.state.lock() else {
            return Chunk {
                text: String::new(),
                dropped: 0,
                exit: Some(None),
                killed: false,
            };
        };
        let first = state.total - state.data.len() as u64;
        let start = state.cursor.max(first);
        let mut dropped = start - state.cursor;
        let mut bytes = state
            .data
            .iter()
            .skip((start - first) as usize)
            .copied()
            .collect::<Vec<_>>();
        if bytes.len() > MAX_OUTPUT_CHUNK {
            let excess = bytes.len() - MAX_OUTPUT_CHUNK;
            dropped += excess as u64;
            bytes.drain(..excess);
        }
        state.cursor = state.total;
        Chunk {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            dropped,
            exit: state.exit,
            killed: state.killed,
        }
    }
}

async fn pump(mut stream: impl AsyncRead + Unpin, ring: Arc<OutputRing>) {
    let mut buffer = [0_u8; 8192];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        ring.push(&buffer[..count]);
    }
}

/// A command started with `run_in_background`, owned by its run.
struct BackgroundProcess {
    command: String,
    output: Arc<OutputRing>,
    guard: Mutex<Option<ProcessTreeGuard>>,
}

impl BackgroundProcess {
    fn kill(&self) {
        if let Some(mut guard) = self.guard.lock().ok().and_then(|mut guard| guard.take()) {
            guard.terminate();
        }
        self.output.mark_killed();
    }
}

fn chunk_result(id: &str, process: &BackgroundProcess, chunk: Chunk) -> Value {
    json!({
        "process_id": id,
        "command": process.command,
        "output": chunk.text,
        "dropped_bytes": chunk.dropped,
        "running": chunk.exit.is_none(),
        "exit_code": chunk.exit.flatten(),
        "killed": chunk.killed,
    })
}

/// Status line plus new output, shared by `process_output` and `process_kill`.
fn chunk_text(result: &Value) -> Option<String> {
    let id = result.get("process_id")?.as_str()?;
    let mut out = if result["running"].as_bool()? {
        format!("{id}: running")
    } else if result["killed"].as_bool().unwrap_or(false) {
        format!("{id}: killed")
    } else {
        match result["exit_code"].as_i64() {
            Some(code) => format!("{id}: exited, exit code: {code}"),
            None => format!("{id}: exited (terminated by a signal)"),
        }
    };
    let dropped = result["dropped_bytes"].as_u64().unwrap_or(0);
    if dropped > 0 {
        let _ = write!(out, "\n[{dropped} bytes of earlier output were dropped]");
    }
    let output = result.get("output")?.as_str()?;
    if output.is_empty() {
        out.push_str("\n(no new output)");
    } else {
        out.push('\n');
        out.push_str(output.trim_end_matches('\n'));
    }
    Some(out)
}

/// Run a command in the host terminal. PowerShell on Windows, `sh -c`
/// elsewhere. Executes on the real machine - the agentic counterpart to the
/// Docker-sandboxed `run_command`.
pub struct ShellTool {
    pub(super) ctx: HostCtx,
}

impl ShellTool {
    /// The directory the next command starts in: the run's last working
    /// directory while it still exists and is allowed, else the workspace root.
    fn start_dir(&self) -> Result<PathBuf> {
        let root = root_of(&self.ctx.ws)?;
        let stored = self
            .ctx
            .run
            .shell
            .cwd
            .lock()
            .ok()
            .and_then(|cwd| cwd.clone());
        Ok(stored
            .filter(|dir| dir.is_dir() && inside_workspace(&self.ctx.ws, dir))
            .unwrap_or(root))
    }

    /// Persist the directory a command finished in. Returns it and whether it
    /// was reset to the workspace root for leaving the workspace.
    fn settle_dir(&self, start: PathBuf, reported: Option<String>) -> Result<(PathBuf, bool)> {
        let Some(dir) = reported.map(PathBuf::from) else {
            return Ok((start, false));
        };
        let allowed = dir.is_dir() && inside_workspace(&self.ctx.ws, &dir);
        if let Ok(mut cwd) = self.ctx.run.shell.cwd.lock() {
            *cwd = allowed.then(|| dir.clone());
        }
        if allowed {
            Ok((dir, false))
        } else {
            Ok((root_of(&self.ctx.ws)?, true))
        }
    }

    async fn start_background(&self, command: &str) -> Result<Value> {
        let shell = &self.ctx.run.shell;
        let running = shell
            .processes
            .lock()
            .map(|processes| {
                processes
                    .values()
                    .filter(|process| !process.output.exited())
                    .count()
            })
            .unwrap_or(0);
        if running >= MAX_RUNNING_BACKGROUND {
            return Err(Error::InvalidRequest(format!(
                "{running} background processes are already running; stop one with process_kill first"
            )));
        }
        let cwd = self.start_dir()?;
        let mut child = shell_command(&cwd, command)
            .spawn()
            .map_err(|error| Error::Other(format!("shell failed to start: {error}")))?;
        let pid = child
            .id()
            .ok_or_else(|| Error::Other("shell process id unavailable".into()))?;
        let guard = ProcessTreeGuard::attach(pid)
            .map_err(|error| Error::Other(format!("failed to contain shell process: {error}")))?;
        let output = Arc::new(OutputRing::default());
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(pump(stdout, output.clone()));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(pump(stderr, output.clone()));
        }
        let waiter = output.clone();
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|status| status.code());
            waiter.finish(code);
        });
        let id = format!(
            "bg-{}",
            shell.next_process.fetch_add(1, Ordering::Relaxed) + 1
        );
        shell
            .processes
            .lock()
            .map_err(|_| Error::Other("background process table poisoned".into()))?
            .insert(
                id.clone(),
                Arc::new(BackgroundProcess {
                    command: command.to_string(),
                    output,
                    guard: Mutex::new(Some(guard)),
                }),
            );
        let root = root_of(&self.ctx.ws)?;
        Ok(json!({
            "process_id": id,
            "pid": pid,
            "command": command,
            "cwd": PathDisplay::new(&root).show(&cwd),
            "background": true,
        }))
    }
}

fn timeout_secs(args: &Value) -> Result<u64> {
    let secs = optional_u64(args, "timeout_secs", DEFAULT_TIMEOUT_SECS)?;
    if !(1..=MAX_TIMEOUT_SECS).contains(&secs) {
        return Err(Error::InvalidRequest(format!(
            "timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
        )));
    }
    Ok(secs)
}

fn runs_in_background(args: &Value) -> bool {
    args.get("run_in_background")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        if cfg!(windows) {
            "Run a PowerShell command on the host. Commands start in the directory the previous command ended in (initially the working folder), so `cd` persists. timeout_secs defaults to 120 (max 600); run_in_background returns a process_id for process_output and process_kill."
        } else {
            "Run a shell command (sh -c) on the host. Commands start in the directory the previous command ended in (initially the working folder), so `cd` persists. timeout_secs defaults to 120 (max 600); run_in_background returns a process_id for process_output and process_kill."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "command":{"type":"string"},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":MAX_TIMEOUT_SECS,"description":"Kill the command's process tree after this many seconds. Default 120."},
            "run_in_background":{"type":"boolean","description":"Start the command and return a process_id immediately, for servers and long jobs. Default false."}
        },"required":["command"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }
    fn effect_for_call(&self, args: &Value) -> ToolEffect {
        match args.get("command").and_then(Value::as_str) {
            Some(command)
                if !runs_in_background(args) && is_read_only(command, ShellDialect::host()) =>
            {
                ToolEffect::ReadOnly
            }
            _ => ToolEffect::Command,
        }
    }
    fn deadline_for_call(&self, args: &Value) -> Option<Duration> {
        if runs_in_background(args) {
            return None;
        }
        let secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        Some(Duration::from_secs(secs) + DEADLINE_GRACE)
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        if result["background"].as_bool().unwrap_or(false) {
            let id = result.get("process_id")?.as_str()?;
            return Some(format!(
                "Started background process {id} (pid {}). Read its output with process_output and stop it with process_kill.",
                result["pid"]
            ));
        }
        let mut out = if result["timed_out"].as_bool().unwrap_or(false) {
            format!(
                "exit code: none (timed out after {} seconds; the process tree was killed)",
                result["timeout_secs"]
            )
        } else {
            match result["exit_code"].as_i64() {
                Some(code) => format!("exit code: {code}"),
                None => "exit code: none (terminated by a signal)".to_string(),
            }
        };
        let stdout = result.get("stdout")?.as_str()?;
        if !stdout.is_empty() {
            out.push('\n');
            out.push_str(stdout.trim_end_matches('\n'));
        }
        if result["stdout_truncated"].as_bool().unwrap_or(false) {
            out.push_str("\n[stdout truncated at 1 MiB]");
        }
        let stderr = result.get("stderr")?.as_str()?;
        if !stderr.is_empty() {
            out.push_str("\n[stderr]\n");
            out.push_str(stderr.trim_end_matches('\n'));
        }
        if result["stderr_truncated"].as_bool().unwrap_or(false) {
            out.push_str("\n[stderr truncated at 1 MiB]");
        }
        if result["cwd_reset"].as_bool().unwrap_or(false) {
            out.push_str("\n[The command left the workspace, so the working directory was reset to the workspace root.]");
        } else if let Some(cwd) = result["cwd"].as_str().filter(|cwd| *cwd != ".") {
            let _ = write!(out, "\n[cwd: {cwd}]");
        }
        Some(out)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let command = arg_str(&args, "command")?;
        if optional_bool(&args, "run_in_background")? {
            return self.start_background(command).await;
        }
        let timeout = timeout_secs(&args)?;
        let root = root_of(&self.ctx.ws)?;
        let start = self.start_dir()?;
        let finished = run_foreground(&start, command, Duration::from_secs(timeout)).await?;
        let (cwd, cwd_reset) = self.settle_dir(start, finished.final_dir)?;
        Ok(json!({
            "stdout": String::from_utf8_lossy(&finished.stdout.kept),
            "stderr": String::from_utf8_lossy(&finished.stderr.kept),
            "stdout_truncated": finished.stdout.truncated,
            "stderr_truncated": finished.stderr.truncated,
            "exit_code": finished.exit_code,
            "timed_out": finished.timed_out,
            "timeout_secs": timeout,
            "cwd": PathDisplay::new(&root).show(&cwd),
            "cwd_reset": cwd_reset,
        }))
    }
}

/// Read new output from a background process started by `shell`.
pub struct ProcessOutputTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for ProcessOutputTool {
    fn name(&self) -> &str {
        "process_output"
    }
    fn description(&self) -> &str {
        "Return a background shell process's output since the last read, and its exit status once it has finished. wait_secs (0-60) waits for the process to exit before returning."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "process_id":{"type":"string"},
            "wait_secs":{"type":"integer","minimum":0,"maximum":MAX_WAIT_SECS,"description":"Seconds to wait for the process to exit. Default 0."}
        },"required":["process_id"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn deadline_for_call(&self, args: &Value) -> Option<Duration> {
        let wait = args
            .get("wait_secs")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(MAX_WAIT_SECS);
        Some(Duration::from_secs(wait) + DEADLINE_GRACE)
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        chunk_text(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "process_id")?;
        let wait = optional_u64(&args, "wait_secs", 0)?;
        if wait > MAX_WAIT_SECS {
            return Err(Error::InvalidRequest(format!(
                "wait_secs must be at most {MAX_WAIT_SECS}"
            )));
        }
        let process = self.ctx.run.shell.process(id)?;
        let deadline = Instant::now() + Duration::from_secs(wait);
        while !process.output.exited() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(remaining.min(Duration::from_millis(100))).await;
        }
        Ok(chunk_result(id, &process, process.output.take_new()))
    }
}

/// Stop a background shell process and its children.
pub struct ProcessKillTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for ProcessKillTool {
    fn name(&self) -> &str {
        "process_kill"
    }
    fn description(&self) -> &str {
        "Stop a background shell process and its child processes, returning any output not yet read."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"process_id":{"type":"string"}},"required":["process_id"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        chunk_text(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "process_id")?;
        let shell = &self.ctx.run.shell;
        let process = shell.process(id)?;
        if !process.output.exited() {
            process.kill();
            let deadline = Instant::now() + KILL_SETTLE;
            while !process.output.exited() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        if let Ok(mut processes) = shell.processes.lock() {
            processes.remove(id);
        }
        Ok(chunk_result(id, &process, process.output.take_new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_stripped_from_output_and_reports_the_directory() {
        let marker = "__MILIM_CWD_test__";
        let mut captured = Captured::default();
        let stream = format!("hello\n\n{marker}/tmp/work\n");
        captured.kept.extend_from_slice(stream.as_bytes());
        captured.tail.extend(stream.as_bytes());
        assert_eq!(
            take_marker(&mut captured, marker).as_deref(),
            Some("/tmp/work")
        );
        assert_eq!(captured.kept, b"hello\n");

        let mut unterminated = Captured::default();
        let stream = format!("no newline\n{marker}/tmp\n");
        unterminated.kept.extend_from_slice(stream.as_bytes());
        unterminated.tail.extend(stream.as_bytes());
        assert_eq!(
            take_marker(&mut unterminated, marker).as_deref(),
            Some("/tmp")
        );
        assert_eq!(unterminated.kept, b"no newline");
    }

    #[test]
    fn output_ring_keeps_the_newest_bytes_and_tracks_reads() {
        let ring = OutputRing::default();
        ring.push(b"first ");
        let chunk = ring.take_new();
        assert_eq!(chunk.text, "first ");
        assert!(chunk.exit.is_none());
        ring.push(&vec![b'x'; BACKGROUND_BUFFER + 10]);
        ring.finish(Some(3));
        let chunk = ring.take_new();
        assert_eq!(
            chunk.dropped,
            (BACKGROUND_BUFFER + 10 - MAX_OUTPUT_CHUNK) as u64
        );
        assert_eq!(chunk.text.len(), MAX_OUTPUT_CHUNK);
        assert_eq!(chunk.exit, Some(Some(3)));
        assert!(ring.take_new().text.is_empty());
    }
}
