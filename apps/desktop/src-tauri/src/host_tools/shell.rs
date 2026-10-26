//! `shell`, `process_output`, and `process_kill`: host commands that keep a
//! working directory and exported environment, plus background processes, for
//! the chat thread (or the run, without one) that started them.
//!
//! After each foreground command the shell prints a trailer (a marker, the
//! final working directory, and its environment) that is stripped from the
//! output. The environment is compared with the one the command started
//! from, and the variables it exported, changed, or unset are applied to
//! later commands of the thread or run.

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
/// Leading bytes kept per output stream of a foreground command.
const OUTPUT_HEAD: usize = 256 * 1024;
/// Trailing bytes kept per stream; output between head and tail is dropped,
/// so final errors and summaries always survive.
const OUTPUT_TAIL: usize = 256 * 1024;
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

/// Environment changes earlier commands made: a value to set, or `None` to
/// unset an inherited variable.
type EnvOverrides = BTreeMap<String, Option<String>>;

/// Shell state of a thread or run: the working directory commands start in,
/// the environment changes they exported, and the background processes they
/// started. Dropping it drops every [`BackgroundProcess`], whose
/// [`ProcessTreeGuard`] kills its process tree.
#[derive(Default)]
pub(super) struct ShellRunState {
    cwd: Mutex<Option<PathBuf>>,
    env: Mutex<EnvOverrides>,
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
                format!("unknown process_id {id}; there are no background processes")
            } else {
                format!(
                    "unknown process_id {id}; the background processes are {}",
                    known.join(", ")
                )
            })
        })
    }

    /// The id and command of every background process still running.
    fn running(&self) -> Vec<(String, String)> {
        self.processes
            .lock()
            .map(|processes| {
                processes
                    .iter()
                    .filter(|(_, process)| !process.output.exited())
                    .map(|(id, process)| (id.clone(), process.command.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Longest command shown in the turn note about running processes.
const NOTE_COMMAND_CHARS: usize = 160;

/// The turn note about background processes earlier turns left running.
fn running_processes_note(running: &[(String, String)]) -> Option<String> {
    if running.is_empty() {
        return None;
    }
    let mut note = String::from(
        "Background processes still running from earlier turns (read them with process_output, stop them with process_kill):",
    );
    for (id, command) in running {
        let command = command.split_whitespace().collect::<Vec<_>>().join(" ");
        let shown = if command.chars().count() > NOTE_COMMAND_CHARS {
            let cut = command
                .chars()
                .take(NOTE_COMMAND_CHARS - 3)
                .collect::<String>();
            format!("{cut}...")
        } else {
            command
        };
        let _ = write!(note, "\n- {id}: {shown}");
    }
    Some(note)
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

/// Variables the shell itself maintains, which never persist.
const VOLATILE_ENV: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", "PSModulePath"];

/// How an environment variable name compares: case-insensitively on Windows.
fn env_key(name: &str) -> String {
    if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.to_string()
    }
}

/// Whether a change to `name` carries over to later commands: ordinary
/// identifiers only, never the shell's own bookkeeping, `MILIM_*` variables
/// (milim's settings and withheld credentials), or `DYLD_*` variables, which
/// macOS strips from system shells, so they always look unset.
fn persists(name: &str) -> bool {
    let mut chars = name.chars();
    let identifier = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    let prefixed = |prefix: &str| {
        name.get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    identifier
        && !prefixed("MILIM_")
        && !prefixed("DYLD_")
        && !VOLATILE_ENV
            .iter()
            .any(|volatile| env_key(volatile) == env_key(name))
}

/// The environment a host command starts with before the run's overrides:
/// milim's own, minus withheld credentials, with the login shell's `PATH`
/// on POSIX. Commands are spawned from exactly this snapshot, so comparing
/// a command's final environment with it shows only the command's changes.
struct BaseEnv {
    vars: Vec<(OsString, OsString)>,
}

impl BaseEnv {
    fn capture() -> Self {
        let withheld = milim_secret_env_keys();
        let mut vars = std::env::vars_os()
            .filter(|(key, _)| !withheld.contains(key))
            .collect::<Vec<_>>();
        if !cfg!(windows) {
            vars.retain(|(key, _)| key != "PATH");
            vars.push(("PATH".into(), super::search::helper_search_path().clone()));
        }
        Self { vars }
    }

    /// The UTF-8 variables, keyed by [`env_key`], with their original names.
    fn utf8(&self) -> BTreeMap<String, (String, String)> {
        self.vars
            .iter()
            .filter_map(|(key, value)| Some((key.to_str()?, value.to_str()?)))
            .map(|(key, value)| (env_key(key), (key.to_string(), value.to_string())))
            .collect()
    }
}

/// The overrides that turn `base` into `captured`, the environment a
/// command finished with.
fn env_overrides(
    base: &BTreeMap<String, (String, String)>,
    captured: &BTreeMap<String, (String, String)>,
) -> EnvOverrides {
    let mut overrides = EnvOverrides::new();
    for (key, (name, value)) in captured {
        if persists(name) && base.get(key).map(|(_, base)| base) != Some(value) {
            overrides.insert(name.clone(), Some(value.clone()));
        }
    }
    for (key, (name, _)) in base {
        if persists(name) && !captured.contains_key(key) {
            overrides.insert(name.clone(), None);
        }
    }
    overrides
}

/// Parse `NAME=value` entries separated by NUL bytes.
fn parse_env(bytes: &[u8]) -> BTreeMap<String, (String, String)> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let (name, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
            (!name.is_empty()).then(|| (env_key(name), (name.to_string(), value.to_string())))
        })
        .collect()
}

/// The shell that runs host commands.
struct HostShell {
    program: PathBuf,
    /// Shown to the model: `bash`, `zsh`, `sh`, `PowerShell 7`, or
    /// `Windows PowerShell 5.1`.
    label: &'static str,
}

/// PowerShell 7 (`pwsh`) when installed, else Windows PowerShell 5.1, on
/// Windows. Elsewhere bash (the user's `$SHELL` when it is a bash), which is
/// what models write commands for; zsh or `sh` only when bash is missing.
fn host_shell() -> &'static HostShell {
    static SHELL: OnceLock<HostShell> = OnceLock::new();
    SHELL.get_or_init(|| {
        if cfg!(windows) {
            return match super::search::find_helper("pwsh") {
                Some(program) => HostShell {
                    program,
                    label: "PowerShell 7",
                },
                None => HostShell {
                    program: "powershell".into(),
                    label: "Windows PowerShell 5.1",
                },
            };
        }
        let login = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .filter(|shell| shell.is_file());
        let named = |name: &str| {
            login
                .clone()
                .filter(|shell| shell.file_name().is_some_and(|file| file == name))
        };
        let bash = named("bash")
            .or_else(|| Some(PathBuf::from("/bin/bash")).filter(|bash| bash.is_file()))
            .or_else(|| super::search::find_helper("bash"));
        match (bash, named("zsh")) {
            (Some(program), _) => HostShell {
                program,
                label: "bash",
            },
            (None, Some(program)) => HostShell {
                program,
                label: "zsh",
            },
            (None, None) => HostShell {
                program: "sh".into(),
                label: "sh",
            },
        }
    })
}

/// Makes PowerShell write UTF-8, whatever the console code page is.
const POWERSHELL_UTF8: &str = "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); $OutputEncoding = [Console]::OutputEncoding\n";

/// The host shell running `script` in its own process group, with the `base`
/// environment plus the run's `overrides`. POSIX shells get the login
/// shell's `PATH`, so a Finder- or Dock-launched app still finds `cargo`,
/// `npm`, and version-manager binaries.
fn shell_command(cwd: &Path, script: &str, base: &BaseEnv, overrides: &EnvOverrides) -> Command {
    let shell = host_shell();
    let mut cmd = Command::new(&shell.program);
    if cfg!(windows) {
        let script = format!("{POWERSHELL_UTF8}{script}");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    } else {
        cmd.args(["-c", script]);
    }
    cmd.current_dir(cwd);
    cmd.env_clear();
    cmd.envs(base.vars.iter().map(|(key, value)| (key, value)));
    for (name, value) in overrides {
        match value {
            Some(value) => cmd.env(name, value),
            None => cmd.env_remove(name),
        };
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

/// The marker that ends the trailer started by `marker`.
fn end_marker(marker: &str) -> String {
    marker.replacen("__MILIM_CWD_", "__MILIM_ENV_END_", 1)
}

/// Append the trailer: a line with `marker` and the final working
/// directory, then the environment as NUL-separated `NAME=value` entries and
/// the end marker. The script then exits with the command's own status.
fn wrap_with_cwd_marker(command: &str, marker: &str) -> String {
    let end = end_marker(marker);
    if cfg!(windows) {
        format!(
            "{command}\n$__milimOk = $?; $__milimCode = $LASTEXITCODE\n[Console]::Out.Write(\"`n{marker}\" + (Get-Location).ProviderPath + \"`n\")\nforeach ($__milimVar in [Environment]::GetEnvironmentVariables().GetEnumerator()) {{ [Console]::Out.Write([string]$__milimVar.Key + '=' + [string]$__milimVar.Value + [char]0) }}\n[Console]::Out.Write(\"{end}`n\")\nif (-not $__milimOk) {{ if ($__milimCode) {{ exit $__milimCode }} else {{ exit 1 }} }}\nexit 0\n"
        )
    } else {
        format!(
            "{command}\n__milim_status=$?\nprintf '\\n{marker}%s\\n' \"$(pwd -P)\"\nenv -0 2>/dev/null\nprintf '{end}\\n'\nexit $__milim_status\n"
        )
    }
}

/// Output of one stream: its first [`OUTPUT_HEAD`] bytes and last
/// [`OUTPUT_TAIL`] bytes, with the size of the dropped middle.
#[derive(Default)]
struct Captured {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    /// Bytes dropped between `head` and `tail`.
    omitted: u64,
}

impl Captured {
    fn push(&mut self, chunk: &[u8]) {
        let room = OUTPUT_HEAD.saturating_sub(self.head.len()).min(chunk.len());
        self.head.extend_from_slice(&chunk[..room]);
        self.tail.extend(&chunk[room..]);
        let excess = self.tail.len().saturating_sub(OUTPUT_TAIL);
        self.tail.drain(..excess);
        self.omitted += excess as u64;
    }

    /// The kept output, with a marker where the middle was dropped.
    fn text(&self) -> String {
        let tail = self.tail.iter().copied().collect::<Vec<_>>();
        if self.omitted == 0 {
            let mut all = self.head.clone();
            all.extend_from_slice(&tail);
            return String::from_utf8_lossy(&all).into_owned();
        }
        format!(
            "{}\n[... {} bytes of output omitted ...]\n{}",
            String::from_utf8_lossy(&self.head),
            self.omitted,
            String::from_utf8_lossy(&tail)
        )
    }
}

async fn capture(mut stream: impl AsyncRead + Unpin, sink: Arc<Mutex<Captured>>) {
    let mut buffer = [0_u8; 8192];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        let Ok(mut captured) = sink.lock() else {
            break;
        };
        captured.push(&buffer[..count]);
    }
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// What a command's trailer reported.
#[derive(Debug, Default, PartialEq)]
struct Trailer {
    /// The working directory the command finished in.
    dir: Option<String>,
    /// The environment it finished with, keyed by [`env_key`].
    env: Option<BTreeMap<String, (String, String)>>,
}

/// Remove the trailer from captured stdout and return what it reported. The
/// trailer ends the stream, so it sits in the tail unless nothing was
/// dropped (then head and tail join).
fn take_marker(captured: &mut Captured, marker: &str) -> Trailer {
    let mut tail = captured.tail.drain(..).collect::<Vec<_>>();
    if captured.omitted == 0 {
        captured.head.append(&mut tail);
        return strip_marker(&mut captured.head, marker);
    }
    let trailer = strip_marker(&mut tail, marker);
    captured.tail = tail.into();
    trailer
}

fn strip_marker(bytes: &mut Vec<u8>, marker: &str) -> Trailer {
    let Some(position) = find_last(bytes, marker.as_bytes()) else {
        return Trailer::default();
    };
    let mut end = bytes[position..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| position + offset + 1)
        .unwrap_or(bytes.len());
    let dir = String::from_utf8_lossy(&bytes[position + marker.len()..end])
        .trim_end_matches(['\n', '\r'])
        .to_string();
    let end_marker = end_marker(marker);
    let env = find_last(&bytes[end..], end_marker.as_bytes()).map(|offset| {
        let env = parse_env(&bytes[end..end + offset]);
        end = bytes[end + offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|newline| end + offset + newline + 1)
            .unwrap_or(bytes.len());
        env
    });
    let start = if position > 0 && bytes[position - 1] == b'\n' {
        position - 1
    } else {
        position
    };
    bytes.drain(start..end);
    Trailer {
        dir: (!dir.is_empty()).then_some(dir),
        // An empty dump means the shell could not list its environment.
        env: env.filter(|env| !env.is_empty()),
    }
}

struct Foreground {
    stdout: Captured,
    stderr: Captured,
    exit_code: Option<i32>,
    timed_out: bool,
    /// Working directory and environment the command finished with, when
    /// it reported them.
    trailer: Trailer,
}

async fn run_foreground(
    cwd: &Path,
    command: &str,
    timeout: Duration,
    base: &BaseEnv,
    overrides: &EnvOverrides,
) -> Result<Foreground> {
    let marker = unique_marker();
    let mut child = shell_command(
        cwd,
        &wrap_with_cwd_marker(command, &marker),
        base,
        overrides,
    )
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
        tokio::spawn(capture(stdout, stdout_sink.clone())),
        tokio::spawn(capture(stderr, stderr_sink.clone())),
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
    let trailer = take_marker(&mut stdout, &marker);
    Ok(Foreground {
        stdout,
        stderr,
        exit_code: status.and_then(|status| status.code()),
        timed_out,
        trailer,
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

/// A command started with `run_in_background`, owned by its thread or run.
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

/// Run a command in the host terminal. PowerShell on Windows, bash
/// elsewhere. Executes on the real machine - the agentic counterpart to the
/// Docker-sandboxed `run_command`.
pub struct ShellTool {
    pub(super) ctx: HostCtx,
}

impl ShellTool {
    /// The directory the next command starts in: the last command's working
    /// directory while it still exists and is allowed, else the workspace root.
    fn start_dir(&self) -> Result<PathBuf> {
        let root = root_of(&self.ctx.ws)?;
        let stored = self
            .ctx
            .run
            .shell()
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
        if let Ok(mut cwd) = self.ctx.run.shell().cwd.lock() {
            *cwd = allowed.then(|| dir.clone());
        }
        if allowed {
            Ok((dir, false))
        } else {
            Ok((root_of(&self.ctx.ws)?, true))
        }
    }

    /// The environment changes earlier commands of this thread or run
    /// exported.
    fn overrides(&self) -> EnvOverrides {
        self.ctx
            .run
            .shell()
            .env
            .lock()
            .map(|env| env.clone())
            .unwrap_or_default()
    }

    /// Persist the environment a command finished with, relative to `base`.
    /// Returns the variables this command changed, as `NAME` (set, or back
    /// to its inherited value) or `-NAME` (unset).
    fn settle_env(
        &self,
        base: &BTreeMap<String, (String, String)>,
        before: &EnvOverrides,
        captured: Option<BTreeMap<String, (String, String)>>,
    ) -> Vec<String> {
        let Some(captured) = captured else {
            return Vec::new();
        };
        let after = env_overrides(base, &captured);
        let mut changed = after
            .iter()
            .filter(|(name, value)| before.get(*name) != Some(value))
            .map(|(name, value)| match value {
                Some(_) => name.clone(),
                None => format!("-{name}"),
            })
            .collect::<Vec<_>>();
        changed.extend(
            before
                .keys()
                .filter(|name| !after.contains_key(*name))
                .cloned(),
        );
        if let Ok(mut env) = self.ctx.run.shell().env.lock() {
            *env = after;
        }
        changed
    }

    async fn start_background(&self, command: &str) -> Result<Value> {
        let shell = self.ctx.run.shell();
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
        let mut child = shell_command(&cwd, command, &BaseEnv::capture(), &self.overrides())
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

/// The `shell` tool description for the host shell named `label`.
fn shell_description(label: &str) -> String {
    let chaining = match label {
        "Windows PowerShell 5.1" => "This is Windows PowerShell 5.1: `&&` and `||` do not exist. Separate commands with `;` (the next one always runs) and continue only on success with `if ($LASTEXITCODE -eq 0) { ... }` or `if ($?) { ... }`.",
        "PowerShell 7" => "Chain with `&&` / `||` (run the next command only on success / failure) or `;` (always).",
        _ => "Chain with `&&` (stop at the first failure) or `;` (always continue).",
    };
    let export = if label.contains("PowerShell") {
        "`$env:NAME = 'value'`"
    } else {
        "`export NAME=value`"
    };
    format!(
        "Run a command on the host with {label}. The command is non-interactive: stdin is closed, so anything that prompts fails or waits until the timeout; pass flags such as --yes or --no-edit instead.\n\
         Use it for builds, tests, git, package managers, and other programs. Don't use it to read, search, or edit files when read_file, glob, grep, or edit_file can do it.\n\
         Commands start in the directory the previous command ended in (initially the working folder), so `cd` persists, and so do environment variables a command sets with {export} or unsets (MILIM_* variables excepted); a command that calls `exit` keeps the previous directory and environment. {chaining}\n\
         timeout_secs defaults to 120 (max 600). For servers, watchers, and other long-running processes set run_in_background, then use process_output and process_kill; do not background with `&`. Output beyond 512 KB per stream keeps its beginning and end.\n\
         Example: {{\"command\": \"cargo test -p core parser::\", \"timeout_secs\": 300}}"
    )
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
        static DESCRIPTION: OnceLock<String> = OnceLock::new();
        DESCRIPTION.get_or_init(|| shell_description(host_shell().label))
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "command":{"type":"string","description":"The command line to run. Quote paths and arguments that contain spaces or special characters."},
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
        let stderr = result.get("stderr")?.as_str()?;
        if !stderr.is_empty() {
            out.push_str("\n[stderr]\n");
            out.push_str(stderr.trim_end_matches('\n'));
        }
        if result["cwd_reset"].as_bool().unwrap_or(false) {
            out.push_str("\n[The command left the workspace, so the working directory was reset to the workspace root.]");
        } else if let Some(cwd) = result["cwd"].as_str().filter(|cwd| *cwd != ".") {
            let _ = write!(out, "\n[cwd: {cwd}]");
        }
        let changed = result["env_changed"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !changed.is_empty() {
            let (unset, set): (Vec<&str>, Vec<&str>) =
                changed.iter().partition(|name| name.starts_with('-'));
            let mut parts = Vec::new();
            if !set.is_empty() {
                parts.push(format!("set {}", set.join(", ")));
            }
            if !unset.is_empty() {
                let unset = unset.iter().map(|name| &name[1..]).collect::<Vec<_>>();
                parts.push(format!("unset {}", unset.join(", ")));
            }
            let _ = write!(out, "\n[env for later commands: {}]", parts.join("; "));
        }
        Some(out)
    }
    fn turn_context(&self) -> Option<String> {
        // Only a thread's state outlives a run; a run of its own starts empty.
        self.ctx.run.thread.get()?;
        running_processes_note(&self.ctx.run.shell().running())
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
        let base = BaseEnv::capture();
        let overrides = self.overrides();
        let finished = run_foreground(
            &start,
            command,
            Duration::from_secs(timeout),
            &base,
            &overrides,
        )
        .await?;
        let (cwd, cwd_reset) = self.settle_dir(start, finished.trailer.dir)?;
        let env_changed = self.settle_env(&base.utf8(), &overrides, finished.trailer.env);
        Ok(json!({
            "stdout": finished.stdout.text(),
            "stderr": finished.stderr.text(),
            "stdout_truncated": finished.stdout.omitted > 0,
            "stderr_truncated": finished.stderr.omitted > 0,
            "stdout_omitted_bytes": finished.stdout.omitted,
            "stderr_omitted_bytes": finished.stderr.omitted,
            "exit_code": finished.exit_code,
            "timed_out": finished.timed_out,
            "timeout_secs": timeout,
            "cwd": PathDisplay::new(&root).show(&cwd),
            "cwd_reset": cwd_reset,
            "env_changed": env_changed,
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
        "Return a background shell process's output since the last read, and its exit status once it has finished. wait_secs (0-60) waits for the process to exit before returning.\n\
         Use it to check on a process started by shell with run_in_background (a dev server's startup log, a long build). Don't poll it in a tight loop: pass wait_secs, or do other work between checks.\n\
         Example: {\"process_id\": \"bg-1\", \"wait_secs\": 10}"
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
        let process = self.ctx.run.shell().process(id)?;
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
        "Stop a background shell process and its child processes, returning any output not yet read.\n\
         Use it when you are done with a server or watcher started with run_in_background, or one that hangs. Background processes are also stopped when the run ends.\n\
         Example: {\"process_id\": \"bg-1\"}"
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
        let shell = self.ctx.run.shell();
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
        captured.push(format!("hello\n\n{marker}/tmp/work\n").as_bytes());
        assert_eq!(
            take_marker(&mut captured, marker).dir.as_deref(),
            Some("/tmp/work")
        );
        assert_eq!(captured.text(), "hello\n");

        let mut unterminated = Captured::default();
        unterminated.push(format!("no newline\n{marker}/tmp\n").as_bytes());
        assert_eq!(
            take_marker(&mut unterminated, marker).dir.as_deref(),
            Some("/tmp")
        );
        assert_eq!(unterminated.text(), "no newline");

        // The environment dump after the directory is parsed and removed.
        let end = end_marker(marker);
        assert!(!end.contains(marker));
        let mut with_env = Captured::default();
        with_env.push(format!("out\n\n{marker}/tmp\nA=1\0B=x=y\nz\0bad\0{end}\n").as_bytes());
        let trailer = take_marker(&mut with_env, marker);
        assert_eq!(trailer.dir.as_deref(), Some("/tmp"));
        let env = trailer.env.unwrap();
        assert_eq!(env[&env_key("A")], ("A".to_string(), "1".to_string()));
        assert_eq!(env[&env_key("B")].1, "x=y\nz");
        assert_eq!(env.len(), 2);
        assert_eq!(with_env.text(), "out\n");
        let mut empty_env = Captured::default();
        empty_env.push(format!("{marker}/tmp\n{end}\n").as_bytes());
        assert_eq!(
            take_marker(&mut empty_env, marker),
            Trailer {
                dir: Some("/tmp".into()),
                env: None
            }
        );

        // Past the head, the marker is found in the kept tail.
        let mut long = Captured::default();
        long.push(&vec![b'a'; OUTPUT_HEAD]);
        long.push(&vec![b'b'; OUTPUT_TAIL + 10]);
        long.push(format!("final error\n\n{marker}/tmp/deep\n").as_bytes());
        assert_eq!(
            take_marker(&mut long, marker).dir.as_deref(),
            Some("/tmp/deep")
        );
        let text = long.text();
        assert!(
            text.ends_with("final error\n"),
            "{}",
            &text[text.len() - 40..]
        );
        assert!(text.contains(&format!(
            "\n[... {} bytes of output omitted ...]\n",
            long.omitted
        )));
    }

    #[test]
    fn env_overrides_keep_exports_and_unsets_but_not_volatile_or_milim_names() {
        let entry =
            |name: &str, value: &str| (env_key(name), (name.to_string(), value.to_string()));
        let base = BTreeMap::from([
            entry("HOME", "/home/u"),
            entry("KEEP", "same"),
            entry("GONE", "x"),
            entry("PWD", "/a"),
        ]);
        let captured = BTreeMap::from([
            entry("HOME", "/home/u"),
            entry("KEEP", "changed"),
            entry("NEW_VAR", "1"),
            entry("PWD", "/b"),
            entry("SHLVL", "2"),
            entry("MILIM_TOKEN", "secret"),
            entry("BASH_FUNC_f%%", "() { :; }"),
        ]);
        assert_eq!(
            env_overrides(&base, &captured),
            EnvOverrides::from([
                ("GONE".to_string(), None),
                ("KEEP".to_string(), Some("changed".to_string())),
                ("NEW_VAR".to_string(), Some("1".to_string())),
            ])
        );
        assert!(persists("_private"));
        assert!(!persists("1ABC") && !persists("milim_home") && !persists("_"));
    }

    #[test]
    fn capture_keeps_the_head_and_the_tail_of_long_output() {
        let mut captured = Captured::default();
        captured.push(b"first line\n");
        for _ in 0..200 {
            captured.push(&[b'x'; 8192]);
        }
        captured.push(b"\nerror: the last line\n");
        assert_eq!(captured.head.len(), OUTPUT_HEAD);
        assert_eq!(captured.tail.len(), OUTPUT_TAIL);
        assert_eq!(
            captured.omitted as usize,
            11 + 200 * 8192 + 22 - OUTPUT_HEAD - OUTPUT_TAIL
        );
        let text = captured.text();
        assert!(text.starts_with("first line\n"));
        assert!(text.ends_with("error: the last line\n"));
    }

    #[test]
    fn descriptions_state_the_chaining_rules_of_each_shell() {
        assert!(shell_description("Windows PowerShell 5.1").contains("`&&` and `||` do not exist"));
        assert!(shell_description("PowerShell 7").contains("`&&` / `||`"));
        let bash = shell_description("bash");
        assert!(bash.contains("with bash") && bash.contains("stdin is closed"));
    }

    #[cfg(unix)]
    #[test]
    fn posix_commands_run_in_bash_with_the_login_path() {
        if host_shell().label != "bash" {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = runtime
            .block_on(async {
                shell_command(
                    Path::new("/"),
                    "printf '%s|%s' \"$BASH_VERSION\" \"$PATH\"",
                    &BaseEnv::capture(),
                    &EnvOverrides::new(),
                )
                .output()
                .await
            })
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let (version, path) = stdout.split_once('|').unwrap();
        assert!(!version.is_empty(), "not bash: {stdout}");
        assert_eq!(
            path,
            super::super::search::helper_search_path().to_str().unwrap()
        );
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
