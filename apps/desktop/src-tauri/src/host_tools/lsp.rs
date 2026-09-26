//! A minimal Language Server Protocol client used only for diagnostics.
//!
//! Servers start lazily, one per workspace and server command, stay alive
//! across runs, and shut down after [`IDLE_SHUTDOWN`] without use. A
//! diagnostics request opens the file (or replaces its text), saves it, and
//! waits briefly for the server's `textDocument/publishDiagnostics` push for
//! that document. Messages are JSON-RPC 2.0 with `Content-Length` framing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, watch};
use tokio::time::Instant;

use milim_core::paths::Paths;
use milim_core::proc::ProcessTreeGuard;

/// How long an unused server stays alive.
const IDLE_SHUTDOWN: Duration = Duration::from_secs(10 * 60);
/// How often idle servers are checked.
const IDLE_CHECK: Duration = Duration::from_secs(60);
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(60);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
/// A server that failed to start is not retried for this long.
const RETRY_FAILED_AFTER: Duration = Duration::from_secs(5 * 60);
/// How long the `diagnostics` tool waits for a cold server to initialize.
pub(super) const READY_WAIT: Duration = Duration::from_secs(10);
/// How long the `diagnostics` tool waits for a diagnostics push.
pub(super) const PUSH_WAIT: Duration = Duration::from_secs(3);
/// Extra time an edit may spend on diagnostics, including a cold start.
const AFTER_EDIT_BUDGET: Duration = Duration::from_millis(1500);
/// Error diagnostics appended to an edit result.
const MAX_AFTER_EDIT: usize = 20;
/// Largest message body accepted from a server.
const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Servers used when their binary is found on the helper search path. Each
/// entry lists extensions and candidate commands, first found wins.
type BuiltinServer = (
    &'static [&'static str],
    &'static [(&'static str, &'static [&'static str])],
);
const BUILTIN_SERVERS: &[BuiltinServer] = &[
    (&[".rs"], &[("rust-analyzer", &[])]),
    (
        &[".ts", ".tsx", ".js", ".jsx"],
        &[("typescript-language-server", &["--stdio"])],
    ),
    (
        &[".py"],
        &[("pyright-langserver", &["--stdio"]), ("pylsp", &[])],
    ),
    (&[".go"], &[("gopls", &[])]),
];

/// Encode one JSON-RPC message with its `Content-Length` header.
pub(super) fn encode(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Splits a byte stream into JSON-RPC messages.
#[derive(Default)]
pub(super) struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete message, or `None` until more bytes arrive. A
    /// malformed frame is skipped and reported as an error.
    pub(super) fn next_message(&mut self) -> Option<std::result::Result<Value, String>> {
        let header_end = self
            .buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")?;
        let start = header_end + 4;
        let header = String::from_utf8_lossy(&self.buffer[..header_end]).into_owned();
        let length = header.split("\r\n").find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        });
        let Some(length) = length else {
            self.buffer.drain(..start);
            return Some(Err("message header has no Content-Length".into()));
        };
        if length > MAX_FRAME {
            self.buffer.clear();
            return Some(Err(format!("message of {length} bytes exceeds the limit")));
        }
        if self.buffer.len() < start + length {
            return None;
        }
        let frame = self.buffer.drain(..start + length).collect::<Vec<_>>();
        Some(
            serde_json::from_slice(&frame[start..])
                .map_err(|error| format!("invalid JSON-RPC message: {error}")),
        )
    }
}

/// The `lsp` section of `~/.milim/settings.json`.
#[derive(Clone, Debug, Default, Deserialize)]
pub(super) struct LspSettings {
    #[serde(default)]
    pub servers: Vec<ServerSetting>,
    #[serde(default)]
    pub diagnostics_after_edit: Option<bool>,
}

/// One user-configured language server.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct ServerSetting {
    #[serde(default)]
    pub extensions: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn load_user_settings() -> LspSettings {
    let path = Paths::resolve().settings_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return LspSettings::default();
    };
    let section = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|settings| settings.get("lsp").cloned());
    match section.map(serde_json::from_value::<LspSettings>) {
        Some(Ok(settings)) => settings,
        Some(Err(error)) => {
            tracing::warn!(
                "ignoring invalid lsp settings in {}: {error}",
                path.display()
            );
            LspSettings::default()
        }
        None => LspSettings::default(),
    }
}

fn normalize_extension(extension: &str) -> String {
    let extension = extension.trim().to_ascii_lowercase();
    if extension.starts_with('.') {
        extension
    } else {
        format!(".{extension}")
    }
}

fn extension_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(normalize_extension)
}

fn language_id(path: &Path) -> String {
    let extension = extension_of(path).unwrap_or_default();
    match extension.as_str() {
        ".rs" => "rust",
        ".ts" => "typescript",
        ".tsx" => "typescriptreact",
        ".js" => "javascript",
        ".jsx" => "javascriptreact",
        ".py" => "python",
        ".go" => "go",
        other => other.trim_start_matches('.'),
    }
    .to_string()
}

/// A server command resolved to a binary.
#[derive(Clone, Debug)]
struct ServerLaunch {
    name: String,
    program: PathBuf,
    args: Vec<String>,
}

impl ServerLaunch {
    fn key(&self) -> String {
        std::iter::once(self.program.display().to_string())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join("\0")
    }
}

/// Resolve a command name or path to an existing binary.
fn find_program(command: &str) -> Option<PathBuf> {
    let direct = Path::new(command);
    if direct.components().count() > 1 {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    let candidates = if cfg!(windows) {
        vec![
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
            command.to_string(),
        ]
    } else {
        vec![command.to_string()]
    };
    std::env::split_paths(super::search::helper_search_path()).find_map(|dir| {
        candidates
            .iter()
            .map(|file| dir.join(file))
            .find(|candidate| candidate.is_file())
    })
}

fn launch(command: &str, args: &[String]) -> Option<ServerLaunch> {
    let program = find_program(command)?;
    let name = Path::new(command)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.to_string());
    Some(ServerLaunch {
        name,
        program,
        args: args.to_vec(),
    })
}

/// Model-visible reason no server applies to `extension`.
fn no_server_message(extension: &str) -> String {
    let hint = BUILTIN_SERVERS
        .iter()
        .find(|(extensions, _)| extensions.contains(&extension))
        .map(|(_, commands)| {
            commands
                .iter()
                .map(|(command, _)| *command)
                .collect::<Vec<_>>()
                .join(" or ")
        })
        .unwrap_or_else(|| "a language server".to_string());
    format!(
        "no language server found for {extension}; install {hint} or configure one under lsp.servers in ~/.milim/settings.json"
    )
}

/// One diagnostic, with 1-based line and column.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(super) struct Diagnostic {
    pub line: u64,
    pub column: u64,
    pub severity: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl Diagnostic {
    fn from_lsp(value: &Value) -> Option<Self> {
        let start = value.get("range")?.get("start")?;
        let severity = match value.get("severity").and_then(Value::as_u64) {
            Some(2) => "warning",
            Some(3) => "info",
            Some(4) => "hint",
            _ => "error",
        };
        Some(Self {
            line: start.get("line")?.as_u64()? + 1,
            column: start.get("character")?.as_u64()? + 1,
            severity,
            message: value.get("message")?.as_str()?.to_string(),
            source: value
                .get("source")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

/// `path:line:col severity message` on one line.
pub(super) fn render_line(
    path: &str,
    line: u64,
    column: u64,
    severity: &str,
    message: &str,
) -> String {
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{path}:{line}:{column} {severity} {message}")
}

/// The `Diagnostics after edit:` block for an edit result that carries
/// `path` and `diagnostics`.
pub(super) fn render_after_edit(result: &Value) -> Option<String> {
    let path = result.get("path")?.as_str()?;
    let lines = result
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(|diagnostic| {
            Some(render_line(
                path,
                diagnostic.get("line")?.as_u64()?,
                diagnostic.get("column")?.as_u64()?,
                diagnostic.get("severity")?.as_str()?,
                diagnostic.get("message")?.as_str()?,
            ))
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| format!("Diagnostics after edit:\n{}", lines.join("\n")))
}

/// Append the after-edit diagnostics block to an edit tool's model text.
pub(super) fn with_after_edit(text: Option<String>, result: &Value) -> Option<String> {
    let mut text = text?;
    if let Some(block) = render_after_edit(result) {
        text.push_str("\n\n");
        text.push_str(&block);
    }
    Some(text)
}

/// A `file://` URI for an absolute path.
pub(super) fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix(r"\\?\")
        .unwrap_or(&text)
        .replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// A comparison key for a document URI, tolerant of how servers re-encode it.
fn uri_key(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&uri[index + 1..index + 3], 16) {
                decoded.push(byte);
                index += 3;
                continue;
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    let key = String::from_utf8_lossy(&decoded).into_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

/// Diagnostics for one file from the `diagnostics` tool.
pub(super) struct FileDiagnostics {
    pub server: String,
    pub diagnostics: Vec<Diagnostic>,
    /// The server had not reported on the current text in time.
    pub partial: bool,
}

enum ServerTable {
    /// `~/.milim/settings.json` plus the built-in servers.
    UserSettings,
    /// Exactly these servers, without built-ins (tests).
    Fixed(LspSettings),
}

type ServerMap = Mutex<HashMap<(PathBuf, String), Arc<LspServer>>>;

/// Language servers shared by every run, keyed by workspace and command.
pub(crate) struct LspManager {
    table: ServerTable,
    idle: Duration,
    servers: Arc<ServerMap>,
}

impl LspManager {
    /// The process-wide manager. Unit tests get one with no servers so they
    /// never start a real language server.
    pub(super) fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<LspManager>> = OnceLock::new();
        GLOBAL
            .get_or_init(|| {
                Arc::new(Self::with_table(if cfg!(test) {
                    ServerTable::Fixed(LspSettings::default())
                } else {
                    ServerTable::UserSettings
                }))
            })
            .clone()
    }

    fn with_table(table: ServerTable) -> Self {
        Self {
            table,
            idle: IDLE_SHUTDOWN,
            servers: Arc::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn fixed(settings: LspSettings) -> Arc<Self> {
        Arc::new(Self::with_table(ServerTable::Fixed(settings)))
    }

    fn settings(&self) -> LspSettings {
        match &self.table {
            ServerTable::UserSettings => load_user_settings(),
            ServerTable::Fixed(settings) => settings.clone(),
        }
    }

    fn launch_for(&self, settings: &LspSettings, extension: &str) -> Option<ServerLaunch> {
        let configured = settings
            .servers
            .iter()
            .filter(|server| {
                server
                    .extensions
                    .iter()
                    .any(|candidate| normalize_extension(candidate) == extension)
            })
            .find_map(|server| launch(&server.command, &server.args));
        if configured.is_some() || !matches!(self.table, ServerTable::UserSettings) {
            return configured;
        }
        BUILTIN_SERVERS
            .iter()
            .filter(|(extensions, _)| extensions.contains(&extension))
            .flat_map(|(_, commands)| commands.iter())
            .find_map(|(command, args)| {
                launch(
                    command,
                    &args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
                )
            })
    }

    /// The running server for `root` and `launch`, starting it when needed.
    fn server(
        &self,
        root: &Path,
        launch: &ServerLaunch,
    ) -> std::result::Result<Arc<LspServer>, String> {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let key = (root.clone(), launch.key());
        let mut servers = self
            .servers
            .lock()
            .map_err(|_| "language server table poisoned".to_string())?;
        let idle = self.idle;
        servers.retain(|_, server| server.idle_for() < idle);
        if let Some(server) = servers.get(&key) {
            match server.state() {
                ServerState::Failed { reason, at } if at.elapsed() < RETRY_FAILED_AFTER => {
                    return Err(reason);
                }
                ServerState::Failed { .. } => {
                    servers.remove(&key);
                }
                _ => return Ok(server.clone()),
            }
        }
        let server = LspServer::spawn(&root, launch)?;
        servers.insert(key.clone(), server.clone());
        tokio::spawn(watch_idle(
            Arc::downgrade(&server),
            Arc::downgrade(&self.servers),
            key,
            idle,
        ));
        Ok(server)
    }

    /// Diagnostics for `path`. The error is the model-visible reason none
    /// are available.
    pub(super) async fn file_diagnostics(
        &self,
        root: &Path,
        path: &Path,
        ready_wait: Duration,
        push_wait: Duration,
    ) -> std::result::Result<FileDiagnostics, String> {
        let settings = self.settings();
        let extension = extension_of(path).ok_or_else(|| {
            format!(
                "{} has no file extension, so no language server applies",
                path.display()
            )
        })?;
        let launch = self
            .launch_for(&settings, &extension)
            .ok_or_else(|| no_server_message(&extension))?;
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let server = self.server(root, &launch)?;
        if !server.wait_ready(Instant::now() + ready_wait).await? {
            return Ok(FileDiagnostics {
                server: launch.name,
                diagnostics: Vec::new(),
                partial: true,
            });
        }
        let (diagnostics, fresh) = server
            .diagnose(path, &text, Instant::now() + push_wait)
            .await?;
        Ok(FileDiagnostics {
            server: launch.name,
            diagnostics,
            partial: !fresh,
        })
    }

    /// Error diagnostics for a file an edit just wrote, within a tight
    /// budget. `None` when no server applies, it is still starting, it did
    /// not report in time, or after-edit diagnostics are turned off. A cold
    /// server keeps starting in the background for later calls.
    pub(super) async fn after_edit(
        &self,
        root: &Path,
        path: &Path,
        text: &str,
    ) -> Option<Vec<Diagnostic>> {
        let deadline = Instant::now() + AFTER_EDIT_BUDGET;
        let settings = self.settings();
        if settings.diagnostics_after_edit == Some(false) {
            return None;
        }
        let launch = self.launch_for(&settings, &extension_of(path)?)?;
        let server = self.server(root, &launch).ok()?;
        if !server.wait_ready(deadline).await.ok()? {
            return None;
        }
        let (diagnostics, fresh) = server.diagnose(path, text, deadline).await.ok()?;
        fresh.then(|| {
            diagnostics
                .into_iter()
                .filter(|diagnostic| diagnostic.severity == "error")
                .take(MAX_AFTER_EDIT)
                .collect()
        })
    }

    #[cfg(test)]
    fn server_count(&self) -> usize {
        self.servers
            .lock()
            .map(|servers| servers.len())
            .unwrap_or(0)
    }
}

/// Shut a server down once it has been idle long enough.
async fn watch_idle(
    server: Weak<LspServer>,
    servers: Weak<ServerMap>,
    key: (PathBuf, String),
    idle: Duration,
) {
    loop {
        tokio::time::sleep(IDLE_CHECK.min(idle)).await;
        let Some(current) = server.upgrade() else {
            return;
        };
        if current.idle_for() < idle {
            continue;
        }
        if let Some(servers) = servers.upgrade() {
            if let Ok(mut servers) = servers.lock() {
                if servers
                    .get(&key)
                    .is_some_and(|entry| Arc::ptr_eq(entry, &current))
                {
                    servers.remove(&key);
                }
            }
        }
        current.shutdown().await;
        return;
    }
}

#[derive(Clone, Debug)]
enum ServerState {
    Starting,
    Ready,
    Failed {
        reason: String,
        at: std::time::Instant,
    },
}

/// What the server last reported for one document.
#[derive(Default)]
struct Document {
    /// Last version sent; 0 means not open.
    version: i64,
    /// Diagnostics pushes received for the document.
    publishes: u64,
    diagnostics: Vec<Diagnostic>,
}

struct LspServer {
    name: String,
    root: PathBuf,
    stdin: tokio::sync::Mutex<ChildStdin>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, oneshot::Sender<Value>>>,
    documents: Mutex<HashMap<String, Document>>,
    /// Bumped on every diagnostics push.
    publishes: watch::Sender<u64>,
    state: watch::Sender<ServerState>,
    last_used: Mutex<std::time::Instant>,
    /// Kept so the child is reaped with the server; dropping it kills it.
    _child: Child,
    /// Kills the server's process tree when the server drops.
    _guard: Option<ProcessTreeGuard>,
}

impl LspServer {
    fn spawn(root: &Path, launch: &ServerLaunch) -> std::result::Result<Arc<Self>, String> {
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .current_dir(root)
            .env("PATH", super::search::helper_search_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start {}: {error}", launch.name))?;
        let guard = child
            .id()
            .and_then(|pid| ProcessTreeGuard::attach(pid).ok());
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(format!("{} has no stdio pipes", launch.name));
        };
        let server = Arc::new(Self {
            name: launch.name.clone(),
            root: root.to_path_buf(),
            stdin: tokio::sync::Mutex::new(stdin),
            next_id: AtomicI64::new(1),
            pending: Mutex::default(),
            documents: Mutex::default(),
            publishes: watch::channel(0).0,
            state: watch::channel(ServerState::Starting).0,
            last_used: Mutex::new(std::time::Instant::now()),
            _child: child,
            _guard: guard,
        });
        tokio::spawn(read_loop(stdout, Arc::downgrade(&server)));
        tokio::spawn(initialize(server.clone()));
        Ok(server)
    }

    fn state(&self) -> ServerState {
        self.state.borrow().clone()
    }

    fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.state.send_if_modified(|state| {
            if matches!(state, ServerState::Failed { .. }) {
                return false;
            }
            *state = ServerState::Failed {
                reason: reason.clone(),
                at: std::time::Instant::now(),
            };
            true
        });
    }

    fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = std::time::Instant::now();
        }
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .map(|last_used| last_used.elapsed())
            .unwrap_or_default()
    }

    /// Whether the server finished initializing by `deadline`.
    async fn wait_ready(&self, deadline: Instant) -> std::result::Result<bool, String> {
        let mut state = self.state.subscribe();
        loop {
            match state.borrow_and_update().clone() {
                ServerState::Ready => return Ok(true),
                ServerState::Failed { reason, .. } => return Err(reason),
                ServerState::Starting => {}
            }
            match tokio::time::timeout_at(deadline, state.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(format!("{} stopped", self.name)),
                Err(_) => return Ok(false),
            }
        }
    }

    async fn send(&self, message: Value) -> std::result::Result<(), String> {
        let bytes = encode(&message);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&bytes)
            .await
            .and(stdin.flush().await)
            .map_err(|error| format!("{} is not accepting input: {error}", self.name))
    }

    async fn notify(&self, method: &str, params: Value) -> std::result::Result<(), String> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> std::result::Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, sender);
        }
        let forget = || {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
        };
        if let Err(error) = self
            .send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await
        {
            forget();
            return Err(error);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(response)) => match response.get("error").filter(|error| !error.is_null()) {
                Some(error) => Err(format!(
                    "{} {method} failed: {}",
                    self.name,
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                )),
                None => Ok(response.get("result").cloned().unwrap_or(Value::Null)),
            },
            Ok(Err(_)) => Err(format!("{} exited", self.name)),
            Err(_) => {
                forget();
                Err(format!("{} did not answer {method} in time", self.name))
            }
        }
    }

    /// Handle one message from the server.
    async fn dispatch(&self, message: Value) {
        let method = message.get("method").and_then(Value::as_str);
        let id = message.get("id").filter(|id| !id.is_null());
        match (method, id) {
            (None, Some(id)) => {
                let sender = id.as_i64().and_then(|id| {
                    self.pending
                        .lock()
                        .ok()
                        .and_then(|mut pending| pending.remove(&id))
                });
                if let Some(sender) = sender {
                    let _ = sender.send(message);
                }
            }
            // Requests from the server must be answered or some servers
            // stall; milim has no settings to offer, so every answer is empty.
            (Some(method), Some(id)) => {
                let result = if method == "workspace/configuration" {
                    let items = message["params"]["items"]
                        .as_array()
                        .map(Vec::len)
                        .unwrap_or(0);
                    Value::Array(vec![Value::Null; items])
                } else {
                    Value::Null
                };
                let _ = self
                    .send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
                    .await;
            }
            (Some("textDocument/publishDiagnostics"), None) => {
                self.record_publish(&message["params"]);
            }
            _ => {}
        }
    }

    fn record_publish(&self, params: &Value) {
        let Some(uri) = params.get("uri").and_then(Value::as_str) else {
            return;
        };
        let diagnostics = params
            .get("diagnostics")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Diagnostic::from_lsp).collect())
            .unwrap_or_default();
        if let Ok(mut documents) = self.documents.lock() {
            let document = documents.entry(uri_key(uri)).or_default();
            document.publishes += 1;
            document.diagnostics = diagnostics;
        }
        self.publishes.send_modify(|count| *count += 1);
    }

    /// Send `text` as the document's current content and wait until
    /// `deadline` for the server to report on it. Returns the latest
    /// diagnostics and whether they describe this text.
    async fn diagnose(
        &self,
        path: &Path,
        text: &str,
        deadline: Instant,
    ) -> std::result::Result<(Vec<Diagnostic>, bool), String> {
        self.touch();
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let uri = file_uri(&path);
        let key = uri_key(&uri);
        let mut publishes = self.publishes.subscribe();
        let (before, method, params) = {
            let mut documents = self
                .documents
                .lock()
                .map_err(|_| "diagnostics table poisoned".to_string())?;
            let document = documents.entry(key.clone()).or_default();
            let before = document.publishes;
            document.version += 1;
            if document.version == 1 {
                (
                    before,
                    "textDocument/didOpen",
                    json!({ "textDocument": {
                        "uri": uri,
                        "languageId": language_id(&path),
                        "version": 1,
                        "text": text,
                    }}),
                )
            } else {
                (
                    before,
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": document.version },
                        "contentChanges": [{ "text": text }],
                    }),
                )
            }
        };
        self.notify(method, params).await?;
        // Some servers (rust-analyzer's cargo check) only report on save.
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": uri }, "text": text }),
        )
        .await?;
        loop {
            if let Ok(documents) = self.documents.lock() {
                if let Some(document) = documents
                    .get(&key)
                    .filter(|document| document.publishes > before)
                {
                    return Ok((document.diagnostics.clone(), true));
                }
            }
            match tokio::time::timeout_at(deadline, publishes.changed()).await {
                Ok(Ok(())) => {}
                _ => break,
            }
        }
        let cached = self
            .documents
            .lock()
            .ok()
            .and_then(|documents| {
                documents
                    .get(&key)
                    .map(|document| document.diagnostics.clone())
            })
            .unwrap_or_default();
        Ok((cached, false))
    }

    /// Ask the server to exit; the process tree is killed when it drops.
    async fn shutdown(&self) {
        if matches!(self.state(), ServerState::Ready) {
            let _ = self
                .request("shutdown", Value::Null, SHUTDOWN_TIMEOUT)
                .await;
            let _ = self.notify("exit", Value::Null).await;
        }
        self.fail("the language server was shut down");
    }
}

async fn read_loop(mut stdout: ChildStdout, server: Weak<LspServer>) {
    let mut decoder = FrameDecoder::default();
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let count = match stdout.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        decoder.push(&buffer[..count]);
        while let Some(message) = decoder.next_message() {
            let Some(server) = server.upgrade() else {
                return;
            };
            match message {
                Ok(message) => server.dispatch(message).await,
                Err(error) => tracing::debug!("{}: {error}", server.name),
            }
        }
    }
    if let Some(server) = server.upgrade() {
        server.fail(format!("{} exited", server.name));
    }
}

async fn initialize(server: Arc<LspServer>) {
    let root_uri = file_uri(&server.root);
    let name = server
        .root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into());
    let params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "milim" },
        "rootUri": root_uri,
        "rootPath": server.root.to_string_lossy(),
        "workspaceFolders": [{ "uri": root_uri, "name": name }],
        "capabilities": {
            "textDocument": {
                "synchronization": { "didSave": true, "dynamicRegistration": false },
                "publishDiagnostics": { "relatedInformation": false },
            },
            "workspace": { "configuration": true, "workspaceFolders": true },
            "window": { "workDoneProgress": true },
        },
    });
    let started = match server
        .request("initialize", params, INITIALIZE_TIMEOUT)
        .await
    {
        Ok(_) => server.notify("initialized", json!({})).await,
        Err(error) => Err(error),
    };
    match started {
        Ok(()) => {
            server.state.send_if_modified(|state| {
                let starting = matches!(state, ServerState::Starting);
                if starting {
                    *state = ServerState::Ready;
                }
                starting
            });
        }
        Err(reason) => server.fail(reason),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_across_partial_and_joined_reads() {
        let first = json!({"jsonrpc": "2.0", "id": 1, "result": {"text": "héllo"}});
        let second = json!({"jsonrpc": "2.0", "method": "note", "params": null});
        let mut bytes = encode(&first);
        bytes.extend(encode(&second));

        let mut decoder = FrameDecoder::default();
        let mut decoded = Vec::new();
        for byte in &bytes {
            decoder.push(std::slice::from_ref(byte));
            while let Some(message) = decoder.next_message() {
                decoded.push(message.unwrap());
            }
        }
        assert_eq!(decoded, vec![first.clone(), second.clone()]);

        let mut decoder = FrameDecoder::default();
        decoder.push(&bytes);
        assert_eq!(decoder.next_message().unwrap().unwrap(), first);
        assert_eq!(decoder.next_message().unwrap().unwrap(), second);
        assert!(decoder.next_message().is_none());
    }

    #[test]
    fn frames_accept_extra_headers_and_skip_malformed_ones() {
        let mut decoder = FrameDecoder::default();
        decoder.push(b"content-length: 2\r\nContent-Type: application/vscode-jsonrpc\r\n\r\n{}");
        assert_eq!(decoder.next_message().unwrap().unwrap(), json!({}));
        decoder.push(b"X-Other: 1\r\n\r\nContent-Length: 4\r\n\r\nnull");
        assert!(decoder.next_message().unwrap().is_err());
        assert_eq!(decoder.next_message().unwrap().unwrap(), Value::Null);
        assert_eq!(
            String::from_utf8(encode(&json!({"a": 1}))).unwrap(),
            "Content-Length: 7\r\n\r\n{\"a\":1}"
        );
    }

    #[test]
    fn uris_encode_paths_and_compare_after_decoding() {
        let uri = file_uri(Path::new("/tmp/my project/a#b.rs"));
        assert_eq!(uri, "file:///tmp/my%20project/a%23b.rs");
        assert_eq!(uri_key(&uri), uri_key("file:///tmp/my project/a%23b.rs"));
    }

    #[test]
    fn lsp_diagnostics_become_one_based_lines() {
        let diagnostic = Diagnostic::from_lsp(&json!({
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 5}},
            "severity": 2,
            "message": "unused\nvariable",
            "source": "rustc"
        }))
        .unwrap();
        assert_eq!((diagnostic.line, diagnostic.column), (2, 5));
        assert_eq!(diagnostic.severity, "warning");
        assert_eq!(
            render_line("src/lib.rs", 2, 5, diagnostic.severity, &diagnostic.message),
            "src/lib.rs:2:5 warning unused variable"
        );
    }

    #[test]
    fn missing_servers_name_the_install_and_settings() {
        let manager = LspManager::fixed(LspSettings::default());
        assert!(manager.launch_for(&manager.settings(), ".rs").is_none());
        let message = no_server_message(".py");
        assert!(message.contains("pyright-langserver or pylsp"), "{message}");
        assert!(message.contains("lsp.servers in ~/.milim/settings.json"));
    }

    /// A POSIX sh language server: answers `initialize` and `shutdown`,
    /// asks for configuration once, and reports one error at line 2,
    /// column 5 for every opened or changed document.
    #[cfg(unix)]
    pub(in crate::host_tools) fn fake_server(dir: &Path) -> LspSettings {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-lsp.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
send() { printf 'Content-Length: %d\r\n\r\n%s' "${#1}" "$1"; }
while :; do
  len=0
  while IFS= read -r line; do
    line=$(printf '%s' "$line" | tr -d '\r')
    [ -z "$line" ] && break
    case "$line" in Content-Length:*) len=$(printf '%s' "${line#*:}" | tr -d ' ');; esac
  done
  [ "$len" -gt 0 ] 2>/dev/null || exit 0
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$body" in
    *'"method":"initialize"'*) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"capabilities\":{}}}" ;;
    *'"method":"initialized"'*) send '{"jsonrpc":"2.0","id":"cfg","method":"workspace/configuration","params":{"items":[{}]}}' ;;
    *'"method":"shutdown"'*) send "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
    *'"method":"exit"'*) exit 0 ;;
    *'"method":"textDocument/didOpen"'*|*'"method":"textDocument/didChange"'*)
      uri=$(printf '%s' "$body" | sed -n 's/.*"uri":"\([^"]*\)".*/\1/p')
      send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{\"uri\":\"$uri\",\"diagnostics\":[{\"range\":{\"start\":{\"line\":1,\"character\":4},\"end\":{\"line\":1,\"character\":5}},\"severity\":1,\"message\":\"expected semicolon\",\"source\":\"fake\"},{\"range\":{\"start\":{\"line\":0,\"character\":0},\"end\":{\"line\":0,\"character\":1}},\"severity\":2,\"message\":\"unused import\"}]}}" ;;
  esac
done
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        LspSettings {
            servers: vec![ServerSetting {
                extensions: vec!["rs".into()],
                command: script.display().to_string(),
                args: Vec::new(),
            }],
            diagnostics_after_edit: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn fake_server_publishes_diagnostics_after_did_open_and_idles_out() {
        let dir = std::env::temp_dir().join(format!("milim-lsp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("lib.rs");
        std::fs::write(&file, "use x;\nfn main() {}\n").unwrap();
        let manager = Arc::new(LspManager::with_table(ServerTable::Fixed(fake_server(
            &dir,
        ))));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let report = manager
                .file_diagnostics(&dir, &file, READY_WAIT, PUSH_WAIT)
                .await
                .unwrap();
            assert!(!report.partial);
            assert_eq!(report.server, "fake-lsp.sh");
            assert_eq!(report.diagnostics.len(), 2);
            assert_eq!(report.diagnostics[0].message, "expected semicolon");
            assert_eq!(
                (report.diagnostics[0].line, report.diagnostics[0].column),
                (2, 5)
            );

            // The second request reuses the server with didChange.
            let again = manager
                .file_diagnostics(&dir, &file, READY_WAIT, PUSH_WAIT)
                .await
                .unwrap();
            assert!(!again.partial);
            assert_eq!(manager.server_count(), 1);
        });
        let idle = Arc::new(LspManager {
            idle: Duration::ZERO,
            ..LspManager::with_table(ServerTable::Fixed(fake_server(&dir)))
        });
        runtime.block_on(async {
            let launch = idle.launch_for(&idle.settings(), ".rs").unwrap();
            let server = Arc::downgrade(&idle.server(&dir, &launch).unwrap());
            for _ in 0..100 {
                if idle.server_count() == 0 && server.upgrade().is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            assert_eq!(idle.server_count(), 0, "the idle server is removed");
            assert!(
                server.upgrade().is_none(),
                "and dropped, killing its process"
            );
        });
        let _ = std::fs::remove_dir_all(dir);
    }
}
