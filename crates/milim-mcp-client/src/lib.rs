//! `milim-mcp-client` — a Model Context Protocol **client**.
//!
//! milim already speaks MCP as a *server* (exposing its own tools). This
//! crate is the other direction: it connects external MCP servers
//! (filesystem, GitHub, Brave-search, …) over stdio or Streamable HTTP, lists
//! their tools, and wraps each one as an [`milim_tools::Tool`] so the agent
//! loop can call them like any builtin.
//!
//! The [`McpHub`] owns every configured server: it connects them in parallel,
//! refreshes tools on `notifications/tools/list_changed`, keeps server log
//! messages for diagnostics, reconnects dropped connections with backoff, and
//! runs OAuth sign-in for HTTP servers that require it.

mod oauth;
mod transport;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::mpsc;

use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use milim_storage::{create_private_file, EncryptedStore};
use milim_tools::{atomic_write, ProcessEnvironmentPolicy, Tool, ToolEffect, ToolUiDescriptor};

use transport::{BearerSource, HttpTransport, Link, SendError, StdioTransport};

pub(crate) const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long to wait for a single non-tool JSON-RPC response.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Default per-server `tools/call` timeout.
pub const DEFAULT_CALL_TIMEOUT_SECS: u64 = 60;
/// Longest configurable per-server `tools/call` timeout.
pub const MAX_CALL_TIMEOUT_SECS: u64 = 600;
/// Budget for one server to start, initialize, and list its tools.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
/// Pipeline headroom so the MCP call timeout reports before the tool deadline.
const DEADLINE_GRACE: Duration = Duration::from_secs(5);
/// Reconnect delays after a live connection drops; the last one repeats.
const RECONNECT_BACKOFF_SECS: [u64; 5] = [1, 2, 5, 15, 60];
const MAX_LOG_ENTRIES: usize = 50;
const MAX_LOG_CHARS: usize = 2_000;
/// Secret-store key prefix for encrypted HTTP header values.
const HEADER_SECRET_PREFIX: &str = "header:";

/// How milim reaches a configured MCP server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportKind {
    /// Spawn `command args…` and speak newline-delimited JSON-RPC.
    #[default]
    Stdio,
    /// Streamable HTTP, falling back to legacy HTTP+SSE.
    Http,
}

/// A configured external MCP server (persisted to `mcp.json`). Configs saved
/// before HTTP support have no `type` and load as stdio.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default, rename = "type")]
    pub transport: McpTransportKind,
    /// Executable to spawn (e.g. `npx`, `uvx`, an absolute path).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    /// Streamable HTTP endpoint (for `type: "http"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Extra HTTP headers. Accepts a `{name: value}` map or the same entry
    /// list as `env`; secret values live in the encrypted secret store.
    #[serde(
        default,
        deserialize_with = "deserialize_headers",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub headers: Vec<McpEnvVar>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Tool annotations are server-declared and untrusted. Only when the user
    /// opts in do `readOnlyHint` tools skip approval as read-only.
    #[serde(default)]
    pub trust_read_only_hints: bool,
    /// Per-server `tools/call` timeout (default 60s, max 10 minutes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_timeout_secs: Option<u64>,
    /// OAuth client id for HTTP servers without dynamic client registration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            transport: McpTransportKind::Stdio,
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            url: None,
            headers: Vec::new(),
            enabled: true,
            trust_read_only_hints: false,
            call_timeout_secs: None,
            oauth_client_id: None,
        }
    }
}

impl McpServerConfig {
    /// Effective `tools/call` timeout, clamped to 1s..=10min.
    pub fn call_timeout(&self) -> Duration {
        Duration::from_secs(
            self.call_timeout_secs
                .unwrap_or(DEFAULT_CALL_TIMEOUT_SECS)
                .clamp(1, MAX_CALL_TIMEOUT_SECS),
        )
    }

    fn validate(&self) -> Result<()> {
        match self.transport {
            McpTransportKind::Stdio if self.command.trim().is_empty() => Err(
                Error::InvalidRequest("stdio MCP servers require a command".into()),
            ),
            McpTransportKind::Stdio => Ok(()),
            McpTransportKind::Http => http_url(self).map(|_| ()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpEnvVar {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing)]
    pub has_value: bool,
}

fn default_true() -> bool {
    true
}

fn deserialize_headers<'de, D>(deserializer: D) -> std::result::Result<Vec<McpEnvVar>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Headers {
        List(Vec<McpEnvVar>),
        Map(BTreeMap<String, String>),
    }
    Ok(match Option::<Headers>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(Headers::List(items)) => items,
        Some(Headers::Map(map)) => map
            .into_iter()
            .map(|(key, value)| McpEnvVar {
                key,
                value: Some(value),
                secret: false,
                required: false,
                has_value: false,
            })
            .collect(),
    })
}

fn http_url(cfg: &McpServerConfig) -> Result<Url> {
    let raw = cfg.url.as_deref().map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return Err(Error::InvalidRequest(
            "HTTP MCP servers require a URL".into(),
        ));
    }
    let url = Url::parse(raw)
        .map_err(|error| Error::InvalidRequest(format!("invalid MCP server URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::InvalidRequest(
            "MCP server URL must be http(s)".into(),
        ));
    }
    Ok(url)
}

/// Public, serializable view of a server's state for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub transport: McpTransportKind,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<McpEnvVarInfo>,
    pub url: Option<String>,
    pub headers: Vec<McpEnvVarInfo>,
    pub enabled: bool,
    pub connected: bool,
    pub status: McpConnectionState,
    pub tool_count: usize,
    /// Tools whose server declares `readOnlyHint: true`, trusted or not.
    pub declared_read_only_tools: usize,
    pub trust_read_only_hints: bool,
    pub call_timeout_secs: u64,
    pub oauth_client_id: Option<String>,
    pub auth: McpAuthInfo,
    pub reconnect_attempt: u32,
    pub retry_in_secs: Option<u64>,
    pub capabilities: McpCapabilities,
    pub missing_env: Vec<String>,
    pub error: Option<String>,
    pub logs: Vec<McpLogEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpEnvVarInfo {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub secret: bool,
    pub required: bool,
    pub has_value: bool,
}

/// Connection lifecycle of one configured server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConnectionState {
    #[default]
    Disconnected,
    Disabled,
    Connecting,
    Connected,
    Reconnecting,
    AuthRequired,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthStatus {
    NotRequired,
    Required,
    SignedIn,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpAuthInfo {
    pub status: McpAuthStatus,
    pub flow: Option<McpAuthFlow>,
}

/// One browser sign-in attempt. `status` is `pending`, `complete`, or `error`.
#[derive(Debug, Clone, Serialize)]
pub struct McpAuthFlow {
    pub id: String,
    pub status: String,
    pub url: Option<String>,
    pub error: Option<String>,
}

/// A server-sent `notifications/message` (or a milim connection event).
#[derive(Debug, Clone, Serialize)]
pub struct McpLogEntry {
    pub at_ms: u64,
    pub level: String,
    pub logger: Option<String>,
    pub message: String,
}

impl McpLogEntry {
    fn new(level: &str, message: impl Into<String>) -> Self {
        Self {
            at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            level: level.to_string(),
            logger: None,
            message: truncate_chars(&message.into(), MAX_LOG_CHARS),
        }
    }

    fn from_params(params: Option<&Value>) -> Option<Self> {
        let params = params?;
        let message = match params.get("data")? {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let mut entry = Self::new(
            params
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("info"),
            message,
        );
        entry.logger = params
            .get("logger")
            .and_then(Value::as_str)
            .map(str::to_string);
        Some(entry)
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    format!("{}…", value.chars().take(limit).collect::<String>())
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct McpCapabilities {
    pub tools: bool,
    pub resources: bool,
    pub prompts: bool,
    pub apps: bool,
}

fn capabilities_from_initialize(result: &Value) -> McpCapabilities {
    let caps = &result["capabilities"];
    McpCapabilities {
        tools: caps.get("tools").is_some(),
        resources: caps.get("resources").is_some(),
        prompts: caps.get("prompts").is_some(),
        apps: caps
            .pointer("/extensions/io.modelcontextprotocol~1ui")
            .is_some(),
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct McpTestResult {
    pub ok: bool,
    pub connected: bool,
    pub tool_count: usize,
    pub capabilities: McpCapabilities,
    pub missing_env: Vec<String>,
    pub auth_required: bool,
    pub error: Option<String>,
}

pub(crate) struct McpSecretStore {
    data_path: PathBuf,
    enc: EncryptedStore,
    lock: StdMutex<()>,
}

impl McpSecretStore {
    fn open(dir: &Path) -> Result<Self> {
        let key_path = dir.join("mcp-secrets.key");
        let key = read_or_make_key(&key_path)?;
        Self::open_with_encryption(dir, EncryptedStore::from_key(&key))
    }

    fn open_with_encryption(dir: &Path, enc: EncryptedStore) -> Result<Self> {
        Ok(Self {
            data_path: dir.join("mcp-secrets.enc"),
            enc,
            lock: StdMutex::new(()),
        })
    }

    pub(crate) fn get(&self, server_id: &str, key: &str) -> Result<Option<String>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::Other("MCP secret lock poisoned".into()))?;
        let all = self.read_all()?;
        Ok(all
            .get(server_id)
            .and_then(|server| server.get(key))
            .cloned())
    }

    fn has(&self, server_id: &str, key: &str) -> bool {
        self.get(server_id, key)
            .ok()
            .flatten()
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    }

    pub(crate) fn put(&self, server_id: &str, key: &str, value: &str) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::Other("MCP secret lock poisoned".into()))?;
        let mut all = self.read_all()?;
        all.entry(server_id.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
        self.write_all(&all)
    }

    fn delete(&self, server_id: &str, key: &str) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::Other("MCP secret lock poisoned".into()))?;
        let mut all = self.read_all()?;
        if let Some(server) = all.get_mut(server_id) {
            server.remove(key);
            if server.is_empty() {
                all.remove(server_id);
            }
        }
        self.write_all(&all)
    }

    fn delete_server(&self, server_id: &str) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| Error::Other("MCP secret lock poisoned".into()))?;
        let mut all = self.read_all()?;
        all.remove(server_id);
        self.write_all(&all)
    }

    fn read_all(&self) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
        if !self.data_path.exists() {
            return Ok(BTreeMap::new());
        }
        let encrypted = std::fs::read(&self.data_path)?;
        let decrypted = self.enc.decrypt(&encrypted)?;
        serde_json::from_slice(&decrypted).map_err(Into::into)
    }

    fn write_all(&self, all: &BTreeMap<String, BTreeMap<String, String>>) -> Result<()> {
        if let Some(parent) = self.data_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec(all)?;
        atomic_write(&self.data_path, &self.enc.encrypt(&data)?)?;
        Ok(())
    }
}

fn read_or_make_key(path: &Path) -> Result<[u8; 32]> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        Ok(bytes) => {
            return Err(Error::Other(format!(
                "invalid MCP encryption key length: expected 32 bytes, got {}",
                bytes.len()
            )))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let key = EncryptedStore::random_key();
    create_private_file(path, &key)?;
    Ok(key)
}

// ----- Client -----

enum Transport {
    Stdio(Box<StdioTransport>),
    Http(Arc<HttpTransport>),
}

impl Transport {
    async fn send(&self, message: &Value) -> std::result::Result<(), SendError> {
        match self {
            Transport::Stdio(stdio) => stdio.send(message).await.map_err(SendError::Other),
            Transport::Http(http) => http.send(message).await,
        }
    }
}

/// A live connection to one MCP server.
pub struct McpClient {
    link: Arc<Link>,
    transport: Transport,
    next_id: AtomicI64,
    capabilities: McpCapabilities,
    notifications: StdMutex<Option<mpsc::UnboundedReceiver<Value>>>,
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Transport::Http(http) = &self.transport {
            http.shutdown();
        }
        self.link.close("MCP client disconnected");
    }
}

impl McpClient {
    /// Spawn `command args…` and complete the MCP `initialize` handshake.
    pub async fn connect(command: &str, args: &[String]) -> Result<Arc<McpClient>> {
        Self::connect_with_env(command, args, None, &HashMap::new()).await
    }

    async fn connect_with_env(
        command: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &HashMap<String, String>,
    ) -> Result<Arc<McpClient>> {
        // On Windows, route through `cmd /C` so PATHEXT shims (npx.cmd, uvx.cmd)
        // resolve; elsewhere spawn the executable directly.
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        } else {
            Command::new(command)
        };
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
            cmd.current_dir(cwd);
        }
        cmd.env_clear().envs(base_child_env()).envs(env);
        #[cfg(unix)]
        cmd.process_group(0);
        // Don't flash a console window when spawning the server on Windows.
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);

        let child = cmd
            .spawn()
            .map_err(|e| Error::Other(format!("failed to spawn MCP server '{command}': {e}")))?;
        let tree =
            ProcessTreeGuard::attach(child.id().ok_or_else(|| {
                Error::Other(format!("MCP server '{command}' has no process id"))
            })?)
            .map_err(|e| Error::Other(format!("failed to contain MCP server '{command}': {e}")))?;
        let (link, notifications) = Link::new();
        let transport = Transport::Stdio(Box::new(StdioTransport::start(child, tree, &link)?));
        Self::initialize(link, transport, notifications).await
    }

    /// Connect a Streamable HTTP server (or a legacy HTTP+SSE one).
    async fn connect_http(
        http: reqwest::Client,
        url: Url,
        headers: HeaderMap,
        auth: Option<Arc<dyn BearerSource>>,
    ) -> Result<Arc<McpClient>> {
        let (link, notifications) = Link::new();
        let transport = Transport::Http(HttpTransport::new(http, url, headers, auth, &link));
        Self::initialize(link, transport, notifications).await
    }

    async fn initialize(
        link: Arc<Link>,
        transport: Transport,
        notifications: mpsc::UnboundedReceiver<Value>,
    ) -> Result<Arc<McpClient>> {
        let next_id = AtomicI64::new(1);
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "extensions": {
                    "io.modelcontextprotocol/ui": {
                        "mimeTypes": ["text/html;profile=mcp-app"]
                    }
                }
            },
            "clientInfo": { "name": "milim", "version": env!("CARGO_PKG_VERSION") }
        });
        let init = match exchange(
            &link,
            &transport,
            &next_id,
            "initialize",
            params.clone(),
            REQUEST_TIMEOUT,
        )
        .await
        {
            // Servers that only speak the legacy HTTP+SSE transport reject the
            // POST; retry the handshake over their SSE endpoint.
            Err(SendError::Status(status, _))
                if matches!(status.as_u16(), 400 | 404 | 405)
                    && matches!(transport, Transport::Http(_)) =>
            {
                if let Transport::Http(http) = &transport {
                    http.open_legacy().await?;
                }
                exchange(
                    &link,
                    &transport,
                    &next_id,
                    "initialize",
                    params,
                    REQUEST_TIMEOUT,
                )
                .await?
            }
            other => other?,
        };
        if let (Transport::Http(http), Some(version)) = (
            &transport,
            init.get("protocolVersion").and_then(Value::as_str),
        ) {
            http.set_protocol_version(version);
        }
        let client = Arc::new(McpClient {
            link,
            transport,
            next_id,
            capabilities: capabilities_from_initialize(&init),
            notifications: StdMutex::new(Some(notifications)),
        });
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        if let Transport::Http(http) = &client.transport {
            if !http.is_legacy() {
                http.open_listen_stream();
            }
        }
        Ok(client)
    }

    pub fn capabilities(&self) -> McpCapabilities {
        self.capabilities.clone()
    }

    /// Why the connection closed, if it has.
    pub fn closed_reason(&self) -> Option<String> {
        self.link.closed_reason()
    }

    /// Server notifications (taken once, by the hub's supervisor).
    fn take_notifications(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.notifications
            .lock()
            .expect("mcp client poisoned")
            .take()
    }

    /// Send a JSON-RPC request and await its response (or time out).
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
            .await
    }

    async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        exchange(
            &self.link,
            &self.transport,
            &self.next_id,
            method,
            params,
            timeout,
        )
        .await
        .map_err(Into::into)
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.transport
            .send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
            .map_err(Into::into)
    }

    /// `tools/list` — enumerate the server's tools. Effects are computed
    /// without trusting `readOnlyHint`; the hub applies per-server trust.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>> {
        if !self.capabilities.tools {
            return Ok(Vec::new());
        }
        let tools = self.list_paged("tools/list", "tools").await?;
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name").and_then(Value::as_str)?.to_string();
                let ui = self
                    .capabilities
                    .apps
                    .then(|| t.pointer("/_meta/ui"))
                    .flatten();
                let visibility = ui
                    .and_then(|ui| ui.get("visibility"))
                    .and_then(Value::as_array);
                let visible = |target: &str| {
                    visibility
                        .map(|items| items.iter().any(|item| item.as_str() == Some(target)))
                        .unwrap_or(true)
                };
                Some(McpToolDef {
                    name,
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                    effect: annotation_effect(&t, false),
                    declared_read_only: t
                        .pointer("/annotations/readOnlyHint")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    model_visible: !self.capabilities.apps || visible("model"),
                    app_visible: self.capabilities.apps && visible("app"),
                    ui_resource_uri: ui
                        .and_then(|ui| ui.get("resourceUri"))
                        .and_then(Value::as_str)
                        .filter(|uri| uri.starts_with("ui://"))
                        .map(str::to_string),
                    raw: t,
                })
            })
            .collect())
    }

    /// `tools/call` — invoke a tool by its server-side name.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value> {
        self.call_tool_with_timeout(
            name,
            arguments,
            Duration::from_secs(DEFAULT_CALL_TIMEOUT_SECS),
        )
        .await
    }

    async fn call_tool_with_timeout(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.request_with_timeout(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
            timeout,
        )
        .await
    }

    pub async fn list_resources(&self) -> Result<Vec<Value>> {
        if !self.capabilities.resources {
            return Ok(Vec::new());
        }
        self.list_paged("resources/list", "resources").await
    }

    pub async fn list_resource_templates(&self) -> Result<Vec<Value>> {
        if !self.capabilities.resources {
            return Ok(Vec::new());
        }
        self.list_paged("resources/templates/list", "resourceTemplates")
            .await
    }

    pub async fn read_resource(&self, uri: &str) -> Result<Value> {
        self.request("resources/read", json!({ "uri": uri })).await
    }

    pub async fn list_prompts(&self) -> Result<Vec<Value>> {
        if !self.capabilities.prompts {
            return Ok(Vec::new());
        }
        self.list_paged("prompts/list", "prompts").await
    }

    pub async fn get_prompt(&self, name: &str, arguments: Value) -> Result<Value> {
        self.request(
            "prompts/get",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    async fn list_paged(&self, method: &str, key: &str) -> Result<Vec<Value>> {
        const MAX_PAGES: usize = 100;
        const MAX_ITEMS: usize = 10_000;
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        for _ in 0..MAX_PAGES {
            let params = cursor
                .as_ref()
                .map(|c| json!({ "cursor": c }))
                .unwrap_or_else(|| json!({}));
            let result = self.request(method, params).await?;
            out.extend(
                result
                    .get(key)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            if out.len() > MAX_ITEMS {
                return Err(Error::Other(format!(
                    "MCP '{method}' returned more than {MAX_ITEMS} items"
                )));
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(out);
            }
            if !seen.insert(cursor.clone().unwrap_or_default()) {
                return Err(Error::Other(format!(
                    "MCP '{method}' repeated a pagination cursor"
                )));
            }
        }
        Err(Error::Other(format!(
            "MCP '{method}' exceeded {MAX_PAGES} pages"
        )))
    }
}

fn base_child_env() -> HashMap<String, String> {
    const KEYS: &[&str] = &[
        "PATH",
        "Path",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "LANG",
        "LC_ALL",
    ];
    KEYS.iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| ((*key).to_string(), value))
        })
        .collect()
}

/// Send one request over `transport` and await its response by id.
async fn exchange(
    link: &Link,
    transport: &Transport,
    next_id: &AtomicI64,
    method: &str,
    params: Value,
    timeout: Duration,
) -> std::result::Result<Value, SendError> {
    if let Some(reason) = link.closed_reason() {
        return Err(SendError::Other(Error::Other(format!(
            "MCP connection closed: {reason}"
        ))));
    }
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let rx = link.register(id);
    let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let outcome = tokio::time::timeout(timeout, async {
        transport.send(&message).await?;
        rx.await
            .map_err(|_| SendError::Other(Error::Other("MCP connection closed".into())))
    })
    .await;
    let response = match outcome {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            link.forget(id);
            return Err(error);
        }
        Err(_) => {
            link.forget(id);
            if method != "initialize" {
                let cancel = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": { "requestId": id, "reason": "timed out" }
                });
                let _ = tokio::time::timeout(Duration::from_secs(2), transport.send(&cancel)).await;
            }
            return Err(SendError::Other(Error::Other(format!(
                "MCP '{method}' timed out after {}s",
                timeout.as_secs()
            ))));
        }
    };
    if response.get("id").is_none() {
        if let Some(reason) = link.closed_reason() {
            return Err(SendError::Other(Error::Other(format!(
                "MCP connection closed: {reason}"
            ))));
        }
    }
    if let Some(err) = response.get("error") {
        return Err(SendError::Other(Error::Other(format!(
            "MCP '{method}' error: {err}"
        ))));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

/// Map MCP tool annotations to an approval effect. Annotations are declared
/// by the server and untrusted, so `readOnlyHint` makes a tool read-only only
/// when the user trusts this server's hints.
pub fn annotation_effect(tool: &Value, trust_read_only_hints: bool) -> ToolEffect {
    match (
        tool.pointer("/annotations/readOnlyHint")
            .and_then(Value::as_bool),
        tool.pointer("/annotations/destructiveHint")
            .and_then(Value::as_bool),
    ) {
        (_, Some(true)) => ToolEffect::Mutating,
        (Some(true), _) if trust_read_only_hints => ToolEffect::ReadOnly,
        _ => ToolEffect::Unknown,
    }
}

fn apply_trust(defs: &mut [McpToolDef], trust_read_only_hints: bool) {
    for def in defs {
        def.effect = annotation_effect(&def.raw, trust_read_only_hints);
    }
}

/// A tool definition as reported by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: ToolEffect,
    /// The server declared `readOnlyHint: true` (display only; untrusted).
    pub declared_read_only: bool,
    pub model_visible: bool,
    pub app_visible: bool,
    pub ui_resource_uri: Option<String>,
    pub raw: Value,
}

/// The current connection for one configured server, shared by its tools so
/// they fail fast while disconnected and follow reconnects.
struct ServerSlot {
    name: String,
    call_timeout: Duration,
    client: RwLock<Option<Arc<McpClient>>>,
}

impl ServerSlot {
    fn new(cfg: &McpServerConfig) -> Arc<Self> {
        Arc::new(Self {
            name: cfg.name.clone(),
            call_timeout: cfg.call_timeout(),
            client: RwLock::new(None),
        })
    }

    fn set(&self, client: Option<Arc<McpClient>>) {
        *self.client.write().expect("mcp slot poisoned") = client;
    }

    fn peek(&self) -> Option<Arc<McpClient>> {
        self.client.read().expect("mcp slot poisoned").clone()
    }

    fn client(&self) -> Result<Arc<McpClient>> {
        match self.peek() {
            Some(client) if client.closed_reason().is_none() => Ok(client),
            _ => Err(Error::Other(format!(
                "MCP server '{}' is disconnected; milim reconnects it automatically, or use Reconnect in MCP Servers",
                self.name
            ))),
        }
    }
}

/// An [`milim_tools::Tool`] that proxies to a remote MCP tool. Exposed under a
/// prefixed name (`<server>__<tool>`) to avoid colliding with builtins; calls
/// use the original server-side name.
pub struct McpTool {
    slot: Arc<ServerSlot>,
    exposed_name: String,
    remote_name: String,
    description: String,
    schema: Value,
    effect: ToolEffect,
    aliases: Vec<String>,
    ui: Option<ToolUiDescriptor>,
}

struct McpAppCallTool {
    slot: Arc<ServerSlot>,
    name: String,
    description: String,
    schema: Value,
    effect: ToolEffect,
}

#[async_trait]
impl Tool for McpAppCallTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn effect(&self) -> ToolEffect {
        self.effect
    }

    fn deadline_for_call(&self, _args: &Value) -> Option<Duration> {
        Some(self.slot.call_timeout + DEADLINE_GRACE)
    }

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        self.slot
            .client()?
            .call_tool_with_timeout(&self.name, args, self.slot.call_timeout)
            .await
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.exposed_name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn input_schema(&self) -> Value {
        self.schema.clone()
    }
    fn effect(&self) -> ToolEffect {
        self.effect
    }
    fn deadline_for_call(&self, _args: &Value) -> Option<Duration> {
        Some(self.slot.call_timeout + DEADLINE_GRACE)
    }
    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }
    fn ui(&self) -> Option<ToolUiDescriptor> {
        self.ui.clone()
    }
    fn call_result(&self, result: &Value) -> Value {
        lift_mcp_image(result.clone())
    }
    fn model_result(&self, result: &Value) -> Value {
        let mut result = lift_mcp_image(result.clone());
        if self.ui.is_some() {
            if let Some(object) = result.as_object_mut() {
                object.remove("structuredContent");
                object.remove("_meta");
            }
        }
        result
    }
    fn aliases(&self) -> Vec<String> {
        self.aliases.clone()
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        self.slot
            .client()?
            .call_tool_with_timeout(&self.remote_name, args, self.slot.call_timeout)
            .await
    }
}

struct McpListResourcesTool {
    slot: Arc<ServerSlot>,
    name: String,
}

#[async_trait]
impl Tool for McpListResourcesTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "List resources and resource templates exposed by this MCP server."
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, _args: Value) -> Result<Value> {
        let client = self.slot.client()?;
        let resources = client.list_resources().await?;
        let resource_templates = client.list_resource_templates().await?;
        Ok(json!({ "resources": resources, "resourceTemplates": resource_templates }))
    }
}

struct McpReadResourceTool {
    slot: Arc<ServerSlot>,
    name: String,
}

#[async_trait]
impl Tool for McpReadResourceTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Read a resource from this MCP server by URI."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{"uri":{"type":"string","description":"Resource URI to read."}},
            "required":["uri"],
            "additionalProperties":false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let uri = args
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("uri is required".into()))?;
        self.slot.client()?.read_resource(uri).await
    }
}

struct McpListPromptsTool {
    slot: Arc<ServerSlot>,
    name: String,
}

#[async_trait]
impl Tool for McpListPromptsTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "List prompts exposed by this MCP server."
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, _args: Value) -> Result<Value> {
        Ok(json!({ "prompts": self.slot.client()?.list_prompts().await? }))
    }
}

struct McpGetPromptTool {
    slot: Arc<ServerSlot>,
    name: String,
}

#[async_trait]
impl Tool for McpGetPromptTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Get a prompt from this MCP server by name."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type":"object",
            "properties":{
                "name":{"type":"string","description":"Prompt name."},
                "arguments":{"type":"object","description":"Prompt arguments.","additionalProperties":true}
            },
            "required":["name"],
            "additionalProperties":false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("name is required".into()))?;
        let arguments = args.get("arguments").cloned().unwrap_or_else(|| json!({}));
        self.slot.client()?.get_prompt(name, arguments).await
    }
}

/// If an MCP tool result carries an image content block
/// (`{type:"image", data, mimeType}`), surface it as a top-level `image` field
/// so the agent loop forwards it to vision models (same path as `screenshot`).
fn lift_mcp_image(mut result: Value) -> Value {
    let mut img = None;
    if let Some(items) = result.get_mut("content").and_then(Value::as_array_mut) {
        for item in items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            if object.get("type").and_then(Value::as_str) != Some("image") {
                continue;
            }
            let data = object
                .remove("data")
                .and_then(|value| value.as_str().map(str::to_string));
            if img.is_none() {
                let mime = object
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("image/png");
                img = data.map(|data| json!({ "mime": mime, "data": data }));
            }
            object.insert("dataOmitted".to_string(), Value::Bool(true));
        }
    }
    if let (Some(img), Some(obj)) = (img, result.as_object_mut()) {
        obj.insert("image".to_string(), img);
    }
    result
}

/// Stable provider-safe namespace derived from the persisted server id.
fn prefix_for(cfg: &McpServerConfig) -> String {
    let base = if cfg.id.trim().is_empty() {
        &cfg.name
    } else {
        &cfg.id
    };
    let hash = base.as_bytes().iter().fold(0x811c9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    });
    format!("mcp_{hash:08x}")
}

fn legacy_prefix_for(cfg: &McpServerConfig) -> String {
    let base = if cfg.name.trim().is_empty() {
        &cfg.id
    } else {
        &cfg.name
    };
    base.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn legacy_exposed_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}__{name}")
    }
}

fn safe_tool_component(value: &str, max: usize) -> String {
    let component: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(max)
        .collect();
    let component = component.trim_matches('_');
    if component.is_empty() {
        "tool".to_string()
    } else {
        component.to_string()
    }
}

fn exposed_tool_name(prefix: &str, name: &str) -> String {
    format!("{prefix}__tool_{}", safe_tool_component(name, 45))
}

fn exposed_meta_name(prefix: &str, name: &str) -> String {
    format!("{prefix}__{name}")
}

fn env_key(key: &str) -> String {
    key.trim().to_string()
}

pub fn secret_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASS",
        "AUTH",
        "CREDENTIAL",
        "PRIVATE",
        "BEARER",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
}

fn normalized_env(mut env: Vec<McpEnvVar>) -> Vec<McpEnvVar> {
    env.drain(..)
        .filter_map(|mut item| {
            item.key = env_key(&item.key);
            if item.key.is_empty() {
                return None;
            }
            if item.secret {
                item.value = None;
            }
            item.has_value = item
                .value
                .as_ref()
                .map(|value| !value.is_empty())
                .unwrap_or(false);
            Some(item)
        })
        .collect()
}

/// Environment variables and HTTP headers share one entry shape; secret
/// header values are stored under a prefixed key.
fn secret_key(prefix: &str, key: &str) -> String {
    format!("{prefix}{key}")
}

fn missing_items(
    server_id: &str,
    items: &[McpEnvVar],
    prefix: &str,
    secrets: Option<&McpSecretStore>,
) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.required)
        .filter_map(|item| {
            let key = env_key(&item.key);
            if key.is_empty() {
                return None;
            }
            let has_value = item
                .value
                .as_ref()
                .map(|value| !value.is_empty())
                .unwrap_or(false)
                || (item.secret
                    && secrets
                        .map(|store| store.has(server_id, &secret_key(prefix, &key)))
                        .unwrap_or(false));
            (!has_value).then_some(key)
        })
        .collect()
}

fn missing_env(cfg: &McpServerConfig, secrets: Option<&McpSecretStore>) -> Vec<String> {
    match cfg.transport {
        McpTransportKind::Stdio => missing_items(&cfg.id, &cfg.env, "", secrets),
        McpTransportKind::Http => {
            missing_items(&cfg.id, &cfg.headers, HEADER_SECRET_PREFIX, secrets)
        }
    }
}

fn resolved_items(
    server_id: &str,
    items: &[McpEnvVar],
    prefix: &str,
    secrets: Option<&McpSecretStore>,
    what: &str,
) -> Result<HashMap<String, String>> {
    let missing = missing_items(server_id, items, prefix, secrets);
    if !missing.is_empty() {
        return Err(Error::InvalidRequest(format!(
            "missing required {what}: {}",
            missing.join(", ")
        )));
    }
    let mut out = HashMap::new();
    for item in items {
        let key = env_key(&item.key);
        if key.is_empty() {
            continue;
        }
        if item.secret {
            let value = item
                .value
                .as_ref()
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| {
                    secrets.and_then(|store| {
                        store
                            .get(server_id, &secret_key(prefix, &key))
                            .ok()
                            .flatten()
                    })
                });
            if let Some(value) = value {
                out.insert(key, value);
            }
        } else if let Some(value) = item.value.as_ref().filter(|value| !value.is_empty()) {
            out.insert(key, value.clone());
        }
    }
    Ok(out)
}

fn header_map(values: HashMap<String, String>) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    for (key, value) in values {
        let name = HeaderName::from_bytes(key.as_bytes())
            .map_err(|_| Error::InvalidRequest(format!("invalid HTTP header name: {key}")))?;
        let value = HeaderValue::from_str(&value)
            .map_err(|_| Error::InvalidRequest(format!("invalid value for HTTP header {key}")))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn items_info(
    server_id: &str,
    items: &[McpEnvVar],
    prefix: &str,
    secrets: Option<&McpSecretStore>,
) -> Vec<McpEnvVarInfo> {
    items
        .iter()
        .map(|item| {
            let key = env_key(&item.key);
            let has_value = if item.secret {
                secrets
                    .map(|store| store.has(server_id, &secret_key(prefix, &key)))
                    .unwrap_or(false)
            } else {
                item.value
                    .as_ref()
                    .map(|value| !value.is_empty())
                    .unwrap_or(false)
            };
            McpEnvVarInfo {
                key,
                value: if item.secret {
                    None
                } else {
                    item.value.clone()
                },
                secret: item.secret,
                required: item.required,
                has_value,
            }
        })
        .collect()
}

fn persist_secret_items(
    secrets: Option<&McpSecretStore>,
    server_id: &str,
    items: &mut [McpEnvVar],
    prefix: &str,
) -> Result<()> {
    for item in items {
        if !item.secret {
            continue;
        }
        item.key = env_key(&item.key);
        if item.key.is_empty() {
            continue;
        }
        let Some(value) = item.value.take() else {
            continue;
        };
        let store =
            secrets.ok_or_else(|| Error::Other("MCP secret store is not available".to_string()))?;
        let key = secret_key(prefix, &item.key);
        if value.is_empty() {
            store.delete(server_id, &key)?;
        } else {
            store.put(server_id, &key, &value)?;
        }
    }
    Ok(())
}

// ----- Hub -----

#[derive(Default)]
struct ServerRuntime {
    generation: u64,
    slot: Option<Arc<ServerSlot>>,
    tools: Vec<Arc<dyn Tool>>,
    tool_defs: HashMap<String, McpToolDef>,
    capabilities: McpCapabilities,
    state: McpConnectionState,
    error: Option<String>,
    reconnect_attempt: u32,
    next_retry_at: Option<Instant>,
    auth_required: bool,
    auth_flow: Option<McpAuthFlow>,
    auth_task: Option<tokio::task::AbortHandle>,
    logs: VecDeque<McpLogEntry>,
}

impl ServerRuntime {
    fn push_log(&mut self, entry: McpLogEntry) {
        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logs.pop_front();
        }
        self.logs.push_back(entry);
    }

    fn clear_connection(&mut self) {
        if let Some(slot) = &self.slot {
            slot.set(None);
        }
        self.tools.clear();
        self.tool_defs.clear();
        self.capabilities = McpCapabilities::default();
    }
}

struct HubState {
    configs: Vec<McpServerConfig>,
    servers: HashMap<String, ServerRuntime>,
}

struct HubInner {
    path: PathBuf,
    load_error: Option<String>,
    secrets: Option<Arc<McpSecretStore>>,
    http: reqwest::Client,
    generations: AtomicU64,
    state: RwLock<HubState>,
}

/// Manages the set of configured MCP servers, their live connections, and the
/// merged set of proxy tools. Persists configs to `<dir>/mcp.json`.
pub struct McpHub {
    inner: Arc<HubInner>,
}

impl McpHub {
    /// Open the hub, loading any persisted server configs (does not connect).
    pub fn open(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref();
        let secrets = McpSecretStore::open(dir);
        Self::open_with_secrets(dir, secrets)
    }

    pub fn open_with_encryption(dir: impl AsRef<Path>, encryption: EncryptedStore) -> Self {
        let dir = dir.as_ref();
        let secrets = McpSecretStore::open_with_encryption(dir, encryption);
        Self::open_with_secrets(dir, secrets)
    }

    pub fn open_without_secrets(dir: impl AsRef<Path>, reason: impl Into<String>) -> Self {
        let error = Error::Other(reason.into());
        Self::open_with_secrets(dir.as_ref(), Err(error))
    }

    fn open_with_secrets(dir: &Path, secrets: Result<McpSecretStore>) -> Self {
        let path = dir.join("mcp.json");
        let (configs, load_error) = match std::fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<Value>(&data).and_then(|value| {
                serde_json::from_value::<Vec<McpServerConfig>>(
                    value.get("servers").cloned().ok_or_else(|| {
                        serde_json::Error::io(std::io::Error::other("missing servers"))
                    })?,
                )
            }) {
                Ok(configs) => (configs, None),
                Err(error) => (Vec::new(), Some(format!("invalid mcp.json: {error}"))),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
            Err(error) => (
                Vec::new(),
                Some(format!("failed to read mcp.json: {error}")),
            ),
        };
        let configs = configs
            .into_iter()
            .map(|mut cfg| {
                cfg.env = normalized_env(cfg.env);
                cfg.headers = normalized_env(cfg.headers);
                cfg
            })
            .collect();
        let secrets = match secrets {
            Ok(store) => Some(Arc::new(store)),
            Err(e) => {
                tracing::warn!("MCP secret store unavailable: {e}");
                None
            }
        };
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self {
            inner: Arc::new(HubInner {
                path,
                load_error,
                secrets,
                http,
                generations: AtomicU64::new(0),
                state: RwLock::new(HubState {
                    configs,
                    servers: HashMap::new(),
                }),
            }),
        }
    }

    /// Connect every enabled configured server in parallel. Each server has
    /// its own connect timeout, and per-server failures are recorded (not
    /// fatal) so one slow or broken server can't block the others.
    pub async fn connect_all(&self) {
        let configs: Vec<McpServerConfig> = {
            let st = self.inner.read();
            st.configs.iter().filter(|c| c.enabled).cloned().collect()
        };
        futures::future::join_all(
            configs
                .into_iter()
                .map(|cfg| self.inner.connect_config(cfg)),
        )
        .await;
    }

    /// Add or update a server (by id), reconnecting it, then persist.
    pub async fn upsert(&self, mut cfg: McpServerConfig) -> Result<McpServerConfig> {
        if cfg.id.trim().is_empty() {
            cfg.id = format!("mcp-{}", uuid::Uuid::new_v4().simple());
        }
        cfg.validate()?;
        cfg.call_timeout_secs = cfg
            .call_timeout_secs
            .map(|secs| secs.clamp(1, MAX_CALL_TIMEOUT_SECS));
        let secrets = self.inner.secrets.as_deref();
        persist_secret_items(secrets, &cfg.id, &mut cfg.env, "")?;
        persist_secret_items(secrets, &cfg.id, &mut cfg.headers, HEADER_SECRET_PREFIX)?;
        cfg.env = normalized_env(cfg.env);
        cfg.headers = normalized_env(cfg.headers);
        let (previous, mut configs) = {
            let st = self.inner.read();
            (
                st.configs.iter().find(|c| c.id == cfg.id).cloned(),
                st.configs
                    .iter()
                    .filter(|c| c.id != cfg.id)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        };
        configs.push(cfg.clone());
        self.inner.save_configs(&configs)?;
        // Tokens are bound to the server URL; drop them when it changes.
        if previous
            .is_some_and(|previous| previous.transport != cfg.transport || previous.url != cfg.url)
        {
            if let Some(secrets) = secrets {
                let _ = secrets.delete(&cfg.id, oauth::OAUTH_SECRET_KEY);
            }
        }
        self.inner.write().configs = configs;
        self.inner.connect_config(cfg.clone()).await;
        Ok(cfg)
    }

    /// Remove a server (dropping its connection, which kills the child).
    pub fn remove(&self, id: &str) -> Result<bool> {
        let configs = {
            let st = self.inner.read();
            if !st.configs.iter().any(|c| c.id == id) {
                return Ok(false);
            }
            st.configs
                .iter()
                .filter(|c| c.id != id)
                .cloned()
                .collect::<Vec<_>>()
        };
        self.inner.save_configs(&configs)?;
        {
            let mut st = self.inner.write();
            st.configs = configs;
            if let Some(mut runtime) = st.servers.remove(id) {
                runtime.clear_connection();
                if let Some(task) = runtime.auth_task.take() {
                    task.abort();
                }
            }
        }
        if let Some(secrets) = &self.inner.secrets {
            let _ = secrets.delete_server(id);
        }
        Ok(true)
    }

    /// Drop and re-establish one server's connection now.
    pub async fn reconnect(&self, id: &str) -> Result<()> {
        let cfg = self
            .config(id)
            .ok_or_else(|| Error::NotFound(format!("mcp server {id}")))?;
        if !cfg.enabled {
            return Err(Error::InvalidRequest(
                "enable the MCP server before reconnecting it".into(),
            ));
        }
        self.inner.connect_config(cfg).await;
        Ok(())
    }

    pub fn config(&self, id: &str) -> Option<McpServerConfig> {
        self.inner
            .read()
            .configs
            .iter()
            .find(|cfg| cfg.id == id)
            .cloned()
    }

    pub async fn test_config(&self, mut cfg: McpServerConfig) -> McpTestResult {
        if cfg.id.trim().is_empty() {
            cfg.id = format!("mcp-test-{}", uuid::Uuid::new_v4().simple());
        }
        let secrets = self.inner.secrets.as_deref();
        let missing = missing_env(&cfg, secrets);
        if !missing.is_empty() {
            return McpTestResult {
                missing_env: missing.clone(),
                error: Some(format!("missing required values: {}", missing.join(", "))),
                ..McpTestResult::default()
            };
        }
        match connect_one(&cfg, self.inner.secrets.as_ref(), &self.inner.http).await {
            Ok((client, defs)) => {
                let slot = ServerSlot::new(&cfg);
                let capabilities = client.capabilities();
                match build_tools(&cfg, &slot, &defs, &capabilities) {
                    Ok(tools) => McpTestResult {
                        ok: true,
                        connected: true,
                        tool_count: tools.len(),
                        capabilities,
                        ..McpTestResult::default()
                    },
                    Err(e) => McpTestResult {
                        error: Some(e.to_string()),
                        ..McpTestResult::default()
                    },
                }
            }
            Err(e) => McpTestResult {
                auth_required: matches!(e, Error::Unauthorized(_)),
                error: Some(e.to_string()),
                ..McpTestResult::default()
            },
        }
    }

    /// All currently-available proxy tools across connected servers.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let state = self.inner.read();
        let mut ids = state.servers.keys().collect::<Vec<_>>();
        ids.sort();
        ids.into_iter()
            .flat_map(|id| state.servers[id].tools.iter().cloned())
            .collect()
    }

    /// Begin an OAuth sign-in for an HTTP server. The returned flow carries
    /// the authorization URL to open in the system browser; the hub finishes
    /// the exchange when the browser returns to the loopback redirect and
    /// then reconnects the server.
    pub async fn start_sign_in(&self, id: &str) -> Result<McpAuthFlow> {
        let cfg = self
            .config(id)
            .ok_or_else(|| Error::NotFound(format!("mcp server {id}")))?;
        if cfg.transport != McpTransportKind::Http {
            return Err(Error::InvalidRequest(
                "sign-in applies only to HTTP MCP servers".into(),
            ));
        }
        let secrets = self
            .inner
            .secrets
            .clone()
            .ok_or_else(|| Error::Other("MCP secret store is not available".into()))?;
        let url = http_url(&cfg)?;
        let headers = header_map(resolved_items(
            &cfg.id,
            &cfg.headers,
            HEADER_SECRET_PREFIX,
            Some(&secrets),
            "headers",
        )?)?;
        let http = self.inner.http.clone();
        let discovery = oauth::discover(&http, &url, &headers).await?;
        let callback = oauth::LoopbackCallback::bind().await?;
        let client = match cfg
            .oauth_client_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(client_id) => oauth::OAuthClient {
                client_id: client_id.to_string(),
                client_secret: None,
                token_endpoint_auth_method: None,
            },
            None => {
                let endpoint = discovery.metadata.registration_endpoint.as_ref().ok_or_else(|| {
                    Error::InvalidRequest(
                        "this server does not support dynamic client registration; enter an OAuth client ID for it in MCP Servers".into(),
                    )
                })?;
                oauth::register_client(
                    &http,
                    endpoint,
                    &callback.redirect_uri,
                    discovery.scope.as_deref(),
                )
                .await?
            }
        };
        let (verifier, challenge) = oauth::pkce_pair();
        let state = oauth::new_state();
        let authorization_url = oauth::authorization_url(
            &discovery,
            &client,
            &callback.redirect_uri,
            &state,
            &challenge,
        )?;
        let flow = McpAuthFlow {
            id: uuid::Uuid::new_v4().to_string(),
            status: "pending".into(),
            url: Some(authorization_url),
            error: None,
        };
        {
            let mut st = self.inner.write();
            let runtime = st.servers.entry(cfg.id.clone()).or_default();
            if let Some(task) = runtime.auth_task.take() {
                task.abort();
            }
            runtime.auth_flow = Some(flow.clone());
        }
        let weak = Arc::downgrade(&self.inner);
        let server_id = cfg.id.clone();
        let flow_id = flow.id.clone();
        let task = tokio::spawn(async move {
            let result =
                oauth::finish_sign_in(&http, callback, &discovery, &client, &state, &verifier)
                    .await
                    .and_then(|tokens| oauth::save_tokens(&secrets, &server_id, &tokens));
            let Some(inner) = weak.upgrade() else { return };
            if let Some(cfg) = inner.finish_sign_in(&server_id, &flow_id, result) {
                inner.connect_config(cfg).await;
            }
        });
        let mut st = self.inner.write();
        match st
            .servers
            .get_mut(&cfg.id)
            .filter(|runtime| runtime.auth_flow.as_ref().is_some_and(|f| f.id == flow.id))
        {
            Some(runtime) if !task.is_finished() => runtime.auth_task = Some(task.abort_handle()),
            _ => {}
        }
        Ok(flow)
    }

    /// Forget an HTTP server's OAuth tokens and reconnect it without them.
    pub async fn sign_out(&self, id: &str) -> Result<()> {
        let cfg = self
            .config(id)
            .ok_or_else(|| Error::NotFound(format!("mcp server {id}")))?;
        if let Some(secrets) = &self.inner.secrets {
            secrets.delete(id, oauth::OAUTH_SECRET_KEY)?;
        }
        {
            let mut st = self.inner.write();
            if let Some(runtime) = st.servers.get_mut(id) {
                if let Some(task) = runtime.auth_task.take() {
                    task.abort();
                }
                runtime.auth_flow = None;
            }
        }
        self.inner.connect_config(cfg).await;
        Ok(())
    }

    /// Read a resource from one negotiated MCP Apps server connection.
    pub async fn read_app_resource(&self, server_id: &str, uri: &str) -> Result<Value> {
        if !uri.starts_with("ui://") || !self.has_app_resource(server_id, uri) {
            return Err(Error::InvalidRequest(
                "resource was not advertised by an MCP App tool".to_string(),
            ));
        }
        let client = self.app_client(server_id)?;
        client.read_resource(uri).await
    }

    /// Whether a connected server advertised this `ui://` resource on a tool.
    pub fn has_app_resource(&self, server_id: &str, uri: &str) -> bool {
        self.inner
            .read()
            .servers
            .get(server_id)
            .is_some_and(|runtime| {
                runtime
                    .tool_defs
                    .values()
                    .any(|tool| tool.ui_resource_uri.as_deref() == Some(uri))
            })
    }

    /// Metadata for an app-callable tool on one server connection.
    pub fn app_tool(&self, server_id: &str, name: &str) -> Option<McpToolDef> {
        self.inner
            .read()
            .servers
            .get(server_id)?
            .tool_defs
            .get(name)
            .filter(|tool| tool.app_visible)
            .cloned()
    }

    /// One fixed-server MCP App tool for the central execution pipeline.
    pub fn app_tool_proxy(&self, server_id: &str, name: &str) -> Result<Arc<dyn Tool>> {
        let definition = self.app_tool(server_id, name).ok_or_else(|| {
            Error::InvalidRequest(format!("tool is not app-visible on MCP server: {name}"))
        })?;
        self.app_client(server_id)?;
        Ok(Arc::new(McpAppCallTool {
            slot: self.slot(server_id)?,
            name: definition.name,
            description: definition.description,
            schema: definition.input_schema,
            effect: definition.effect,
        }))
    }

    /// Call an app-visible tool on its originating server connection.
    pub async fn call_app_tool(
        &self,
        server_id: &str,
        name: &str,
        arguments: Value,
    ) -> Result<Value> {
        if self.app_tool(server_id, name).is_none() {
            return Err(Error::InvalidRequest(format!(
                "tool is not app-visible on MCP server: {name}"
            )));
        }
        let timeout = self.slot(server_id)?.call_timeout;
        self.app_client(server_id)?
            .call_tool_with_timeout(name, arguments, timeout)
            .await
    }

    fn slot(&self, server_id: &str) -> Result<Arc<ServerSlot>> {
        self.inner
            .read()
            .servers
            .get(server_id)
            .and_then(|runtime| runtime.slot.clone())
            .ok_or_else(|| Error::InvalidRequest("MCP server is not connected".to_string()))
    }

    fn app_client(&self, server_id: &str) -> Result<Arc<McpClient>> {
        let client = self
            .slot(server_id)?
            .peek()
            .filter(|client| client.closed_reason().is_none())
            .ok_or_else(|| Error::InvalidRequest("MCP server is not connected".to_string()))?;
        if !client.capabilities().apps {
            return Err(Error::InvalidRequest(
                "MCP server did not negotiate Apps support".to_string(),
            ));
        }
        Ok(client)
    }

    /// UI view of every configured server.
    pub fn list(&self) -> Vec<McpServerInfo> {
        let st = self.inner.read();
        let secrets = self.inner.secrets.as_deref();
        let now = Instant::now();
        st.configs
            .iter()
            .map(|c| {
                let runtime = st.servers.get(&c.id);
                let status = runtime
                    .map(|runtime| runtime.state)
                    .unwrap_or(if c.enabled {
                        McpConnectionState::Disconnected
                    } else {
                        McpConnectionState::Disabled
                    });
                let signed_in = c.transport == McpTransportKind::Http
                    && secrets.is_some_and(|store| store.has(&c.id, oauth::OAUTH_SECRET_KEY));
                let auth_status = if c.transport != McpTransportKind::Http {
                    McpAuthStatus::NotRequired
                } else if signed_in {
                    McpAuthStatus::SignedIn
                } else if runtime.is_some_and(|runtime| runtime.auth_required) {
                    McpAuthStatus::Required
                } else {
                    McpAuthStatus::NotRequired
                };
                McpServerInfo {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    transport: c.transport,
                    command: c.command.clone(),
                    args: c.args.clone(),
                    cwd: c.cwd.clone(),
                    env: items_info(&c.id, &c.env, "", secrets),
                    url: c.url.clone(),
                    headers: items_info(&c.id, &c.headers, HEADER_SECRET_PREFIX, secrets),
                    enabled: c.enabled,
                    connected: status == McpConnectionState::Connected,
                    status,
                    tool_count: runtime.map(|runtime| runtime.tools.len()).unwrap_or(0),
                    declared_read_only_tools: runtime
                        .map(|runtime| {
                            runtime
                                .tool_defs
                                .values()
                                .filter(|tool| tool.declared_read_only)
                                .count()
                        })
                        .unwrap_or(0),
                    trust_read_only_hints: c.trust_read_only_hints,
                    call_timeout_secs: c.call_timeout().as_secs(),
                    oauth_client_id: c.oauth_client_id.clone(),
                    auth: McpAuthInfo {
                        status: auth_status,
                        flow: runtime.and_then(|runtime| runtime.auth_flow.clone()),
                    },
                    reconnect_attempt: runtime
                        .map(|runtime| runtime.reconnect_attempt)
                        .unwrap_or(0),
                    retry_in_secs: runtime
                        .and_then(|runtime| runtime.next_retry_at)
                        .map(|at| at.saturating_duration_since(now).as_secs()),
                    capabilities: runtime
                        .map(|runtime| runtime.capabilities.clone())
                        .unwrap_or_default(),
                    missing_env: missing_env(c, secrets),
                    error: runtime.and_then(|runtime| runtime.error.clone()),
                    logs: runtime
                        .map(|runtime| runtime.logs.iter().cloned().collect())
                        .unwrap_or_default(),
                }
            })
            .collect()
    }
}

impl HubInner {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HubState> {
        self.state.read().expect("mcp hub poisoned")
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HubState> {
        self.state.write().expect("mcp hub poisoned")
    }

    fn save_configs(&self, configs: &[McpServerConfig]) -> Result<()> {
        if let Some(error) = &self.load_error {
            return Err(Error::Other(format!(
                "refusing to overwrite unreadable MCP configuration: {error}"
            )));
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_vec_pretty(&json!({ "servers": configs }))?;
        atomic_write(&self.path, &data)?;
        Ok(())
    }

    /// Start a fresh connection session: invalidate background tasks of the
    /// previous one and drop its connection.
    fn begin_session(&self, cfg: &McpServerConfig) -> u64 {
        let generation = self.generations.fetch_add(1, Ordering::Relaxed) + 1;
        let mut st = self.write();
        let runtime = st.servers.entry(cfg.id.clone()).or_default();
        runtime.clear_connection();
        runtime.slot = Some(ServerSlot::new(cfg));
        runtime.generation = generation;
        runtime.state = if cfg.enabled {
            McpConnectionState::Connecting
        } else {
            McpConnectionState::Disabled
        };
        runtime.error = None;
        runtime.reconnect_attempt = 0;
        runtime.next_retry_at = None;
        generation
    }

    /// Connect one config in a new session and record its client and tools
    /// (or its error).
    async fn connect_config(self: &Arc<Self>, cfg: McpServerConfig) {
        let generation = self.begin_session(&cfg);
        if !cfg.enabled {
            return;
        }
        let result = connect_one(&cfg, self.secrets.as_ref(), &self.http).await;
        self.install(&cfg, generation, result, false);
    }

    /// Store a connection attempt's outcome if its session is still current.
    /// Returns `false` when a reconnect loop should keep retrying.
    fn install(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
        generation: u64,
        result: Result<(Arc<McpClient>, Vec<McpToolDef>)>,
        retrying: bool,
    ) -> bool {
        let mut st = self.write();
        let Some(runtime) = st
            .servers
            .get_mut(&cfg.id)
            .filter(|runtime| runtime.generation == generation)
        else {
            return true;
        };
        match result {
            Ok((client, defs)) => {
                let Some(slot) = runtime.slot.clone() else {
                    return true;
                };
                let capabilities = client.capabilities();
                let tools = match build_tools(cfg, &slot, &defs, &capabilities) {
                    Ok(tools) => tools,
                    Err(e) => {
                        runtime.state = McpConnectionState::Error;
                        runtime.error = Some(e.to_string());
                        return true;
                    }
                };
                slot.set(Some(client.clone()));
                runtime.tools = tools;
                runtime.tool_defs = defs
                    .into_iter()
                    .map(|tool| (tool.name.clone(), tool))
                    .collect();
                runtime.capabilities = capabilities;
                runtime.state = McpConnectionState::Connected;
                runtime.error = None;
                runtime.reconnect_attempt = 0;
                runtime.next_retry_at = None;
                runtime.auth_required = false;
                drop(st);
                let link = client.link.clone();
                if let Some(notifications) = client.take_notifications() {
                    tokio::spawn(supervise(
                        Arc::downgrade(self),
                        cfg.id.clone(),
                        generation,
                        link,
                        notifications,
                    ));
                }
                true
            }
            Err(Error::Unauthorized(message)) => {
                runtime.state = McpConnectionState::AuthRequired;
                runtime.auth_required = true;
                runtime.error = Some(message);
                runtime.next_retry_at = None;
                true
            }
            Err(e) => {
                tracing::warn!("MCP server '{}' failed to connect: {e}", cfg.name);
                runtime.error = Some(e.to_string());
                if retrying {
                    runtime.state = McpConnectionState::Reconnecting;
                    false
                } else {
                    runtime.state = McpConnectionState::Error;
                    true
                }
            }
        }
    }

    fn push_log(&self, id: &str, entry: McpLogEntry) {
        if let Some(runtime) = self.write().servers.get_mut(id) {
            runtime.push_log(entry);
        }
    }

    async fn handle_notification(&self, id: &str, generation: u64, message: Value) {
        match message.get("method").and_then(Value::as_str) {
            Some("notifications/tools/list_changed") => {
                if let Err(e) = self.refresh_tools(id, generation).await {
                    self.push_log(
                        id,
                        McpLogEntry::new("warning", format!("Tool list refresh failed: {e}")),
                    );
                }
            }
            Some("notifications/message") => {
                if let Some(entry) = McpLogEntry::from_params(message.get("params")) {
                    self.push_log(id, entry);
                }
            }
            // Resource and prompt tools list on demand, so their
            // list_changed notifications need no cached refresh.
            _ => {}
        }
    }

    /// Re-list a live server's tools after `notifications/tools/list_changed`.
    async fn refresh_tools(&self, id: &str, generation: u64) -> Result<()> {
        let Some((cfg, slot, client)) = ({
            let st = self.read();
            let runtime = st
                .servers
                .get(id)
                .filter(|runtime| runtime.generation == generation);
            let cfg = st.configs.iter().find(|cfg| cfg.id == id).cloned();
            let slot = runtime.and_then(|runtime| runtime.slot.clone());
            let client = slot.as_ref().and_then(|slot| slot.peek());
            cfg.zip(slot)
                .zip(client)
                .map(|((cfg, slot), client)| (cfg, slot, client))
        }) else {
            return Ok(());
        };
        let mut defs = client.list_tools().await?;
        apply_trust(&mut defs, cfg.trust_read_only_hints);
        let tools = build_tools(&cfg, &slot, &defs, &client.capabilities())?;
        let mut st = self.write();
        if let Some(runtime) = st.servers.get_mut(id).filter(|runtime| {
            runtime.generation == generation && runtime.state == McpConnectionState::Connected
        }) {
            runtime.tools = tools;
            runtime.tool_defs = defs
                .into_iter()
                .map(|tool| (tool.name.clone(), tool))
                .collect();
        }
        Ok(())
    }

    /// Record a dropped connection. Returns whether to reconnect.
    fn connection_lost(&self, id: &str, generation: u64, reason: &str) -> bool {
        let mut st = self.write();
        let enabled = st.configs.iter().any(|cfg| cfg.id == id && cfg.enabled);
        let Some(runtime) = st
            .servers
            .get_mut(id)
            .filter(|runtime| runtime.generation == generation)
        else {
            return false;
        };
        tracing::warn!("MCP server '{id}' disconnected: {reason}");
        runtime.clear_connection();
        runtime.error = Some(reason.to_string());
        runtime.push_log(McpLogEntry::new(
            "warning",
            format!("Connection lost: {reason}"),
        ));
        runtime.state = if enabled {
            McpConnectionState::Reconnecting
        } else {
            McpConnectionState::Disabled
        };
        enabled
    }

    fn schedule_retry(&self, id: &str, generation: u64, attempt: u32, delay: Duration) -> bool {
        let mut st = self.write();
        let Some(runtime) = st.servers.get_mut(id).filter(|runtime| {
            runtime.generation == generation && runtime.state == McpConnectionState::Reconnecting
        }) else {
            return false;
        };
        runtime.reconnect_attempt = attempt;
        runtime.next_retry_at = Some(Instant::now() + delay);
        true
    }

    fn session_config(&self, id: &str, generation: u64) -> Option<McpServerConfig> {
        let st = self.read();
        st.servers
            .get(id)
            .filter(|runtime| runtime.generation == generation)?;
        st.configs
            .iter()
            .find(|cfg| cfg.id == id && cfg.enabled)
            .cloned()
    }

    /// Record a finished sign-in; returns the config to reconnect on success.
    fn finish_sign_in(
        &self,
        id: &str,
        flow_id: &str,
        result: Result<()>,
    ) -> Option<McpServerConfig> {
        let mut st = self.write();
        let cfg = st.configs.iter().find(|cfg| cfg.id == id).cloned();
        let runtime = st.servers.get_mut(id)?;
        let flow = runtime
            .auth_flow
            .as_mut()
            .filter(|flow| flow.id == flow_id)?;
        flow.url = None;
        runtime.auth_task = None;
        match result {
            Ok(()) => {
                flow.status = "complete".into();
                runtime.auth_required = false;
                cfg.filter(|cfg| cfg.enabled)
            }
            Err(e) => {
                flow.status = "error".into();
                flow.error = Some(e.to_string());
                None
            }
        }
    }
}

/// Watch one live connection: apply its notifications, and when it drops,
/// withdraw its tools and reconnect with backoff.
async fn supervise(
    hub: Weak<HubInner>,
    id: String,
    generation: u64,
    link: Arc<Link>,
    mut notifications: mpsc::UnboundedReceiver<Value>,
) {
    loop {
        tokio::select! {
            message = notifications.recv() => {
                let Some(message) = message else { break };
                let Some(inner) = hub.upgrade() else { return };
                inner.handle_notification(&id, generation, message).await;
            }
            _ = link.wait_closed() => break,
        }
    }
    let reason = link.wait_closed().await;
    drop(link);
    let Some(inner) = hub.upgrade() else { return };
    if !inner.connection_lost(&id, generation, &reason) {
        return;
    }
    drop(inner);
    reconnect_loop(hub, id, generation).await;
}

async fn reconnect_loop(hub: Weak<HubInner>, id: String, generation: u64) {
    let mut attempt: u32 = 0;
    loop {
        let delay = Duration::from_secs(
            RECONNECT_BACKOFF_SECS[(attempt as usize).min(RECONNECT_BACKOFF_SECS.len() - 1)],
        );
        attempt += 1;
        {
            let Some(inner) = hub.upgrade() else { return };
            if !inner.schedule_retry(&id, generation, attempt, delay) {
                return;
            }
        }
        tokio::time::sleep(delay).await;
        let Some(inner) = hub.upgrade() else { return };
        let Some(cfg) = inner.session_config(&id, generation) else {
            return;
        };
        let result = connect_one(&cfg, inner.secrets.as_ref(), &inner.http).await;
        if inner.install(&cfg, generation, result, true) {
            return;
        }
    }
}

/// Connect a config and list its tools (with per-server annotation trust),
/// bounded by the connect timeout.
async fn connect_one(
    cfg: &McpServerConfig,
    secrets: Option<&Arc<McpSecretStore>>,
    http: &reqwest::Client,
) -> Result<(Arc<McpClient>, Vec<McpToolDef>)> {
    cfg.validate()?;
    let connect = async {
        let client = match cfg.transport {
            McpTransportKind::Stdio => {
                let env = resolved_items(&cfg.id, &cfg.env, "", secrets.map(Arc::as_ref), "env")?;
                McpClient::connect_with_env(&cfg.command, &cfg.args, cfg.cwd.as_deref(), &env)
                    .await?
            }
            McpTransportKind::Http => {
                let headers = header_map(resolved_items(
                    &cfg.id,
                    &cfg.headers,
                    HEADER_SECRET_PREFIX,
                    secrets.map(Arc::as_ref),
                    "headers",
                )?)?;
                let auth = secrets.map(|store| {
                    Arc::new(oauth::TokenSource::new(
                        store.clone(),
                        cfg.id.clone(),
                        http.clone(),
                    )) as Arc<dyn BearerSource>
                });
                McpClient::connect_http(http.clone(), http_url(cfg)?, headers, auth).await?
            }
        };
        let mut defs = client.list_tools().await?;
        apply_trust(&mut defs, cfg.trust_read_only_hints);
        Ok((client, defs))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| {
            Error::Other(format!(
                "MCP server '{}' did not connect within {}s",
                cfg.name,
                CONNECT_TIMEOUT.as_secs()
            ))
        })?
}

/// Build the prefixed proxy tools for one server connection.
fn build_tools(
    cfg: &McpServerConfig,
    slot: &Arc<ServerSlot>,
    defs: &[McpToolDef],
    caps: &McpCapabilities,
) -> Result<Vec<Arc<dyn Tool>>> {
    let prefix = prefix_for(cfg);
    let legacy_prefix = legacy_prefix_for(cfg);
    let mut names = HashSet::new();
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for d in defs {
        if !d.model_visible {
            continue;
        }
        let tool_name = exposed_tool_name(&prefix, &d.name);
        let legacy_name = legacy_exposed_name(&legacy_prefix, &d.name);
        if !names.insert(tool_name.clone()) {
            return Err(Error::InvalidRequest(format!(
                "MCP server exposes colliding tool names after normalization: {}",
                d.name
            )));
        }
        let aliases = (legacy_name != tool_name)
            .then_some(legacy_name)
            .into_iter()
            .collect();
        tools.push(Arc::new(McpTool {
            slot: slot.clone(),
            exposed_name: tool_name,
            remote_name: d.name.clone(),
            description: d.description.clone(),
            schema: d.input_schema.clone(),
            effect: d.effect,
            aliases,
            ui: d
                .ui_resource_uri
                .clone()
                .map(|resource_uri| ToolUiDescriptor::McpApp {
                    server_id: cfg.id.clone(),
                    resource_uri,
                    tool: d.raw.clone(),
                }),
        }) as Arc<dyn Tool>);
    }
    if caps.resources {
        tools.push(Arc::new(McpListResourcesTool {
            slot: slot.clone(),
            name: exposed_meta_name(&prefix, "list_resources"),
        }));
        tools.push(Arc::new(McpReadResourceTool {
            slot: slot.clone(),
            name: exposed_meta_name(&prefix, "read_resource"),
        }));
    }
    if caps.prompts {
        tools.push(Arc::new(McpListPromptsTool {
            slot: slot.clone(),
            name: exposed_meta_name(&prefix, "list_prompts"),
        }));
        tools.push(Arc::new(McpGetPromptTool {
            slot: slot.clone(),
            name: exposed_meta_name(&prefix, "get_prompt"),
        }));
    }
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_available() -> bool {
        std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_ok()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("milim-mcp-{label}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn node_config(id: &str, script: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.into(),
            name: id.into(),
            command: "node".into(),
            args: vec!["-e".into(), script.into()],
            ..McpServerConfig::default()
        }
    }

    async fn wait_for(mut check: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if check() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        check()
    }

    #[test]
    fn prefix_sanitizes() {
        let cfg = McpServerConfig {
            id: "x".into(),
            name: "GitHub MCP!".into(),
            command: "npx".into(),
            ..McpServerConfig::default()
        };
        let prefix = prefix_for(&cfg);
        assert!(prefix.starts_with("mcp_"));
        assert_eq!(prefix.len(), 12);
    }

    #[test]
    fn parses_server_capabilities() {
        let caps = capabilities_from_initialize(&json!({
            "capabilities": {
                "tools": {},
                "resources": { "listChanged": true },
                "prompts": {},
                "extensions": { "io.modelcontextprotocol/ui": {} }
            }
        }));
        assert!(caps.tools);
        assert!(caps.resources);
        assert!(caps.prompts);
        assert!(caps.apps);
    }

    #[test]
    fn read_only_hints_are_untrusted_by_default() {
        let read_only = json!({"name":"search","annotations":{"readOnlyHint":true}});
        let destructive =
            json!({"name":"drop","annotations":{"readOnlyHint":true,"destructiveHint":true}});
        let plain = json!({"name":"plain"});
        assert_eq!(annotation_effect(&read_only, false), ToolEffect::Unknown);
        assert_eq!(annotation_effect(&read_only, true), ToolEffect::ReadOnly);
        assert_eq!(annotation_effect(&destructive, true), ToolEffect::Mutating);
        assert_eq!(annotation_effect(&destructive, false), ToolEffect::Mutating);
        assert_eq!(annotation_effect(&plain, true), ToolEffect::Unknown);
    }

    #[test]
    fn legacy_stdio_configs_load_unchanged_and_http_configs_round_trip() {
        let legacy: McpServerConfig = serde_json::from_value(json!({
            "id": "fs",
            "name": "Filesystem",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
            "env": [{ "key": "ROOT", "value": "/tmp" }],
            "enabled": true
        }))
        .unwrap();
        assert_eq!(legacy.transport, McpTransportKind::Stdio);
        assert!(!legacy.trust_read_only_hints);
        assert_eq!(legacy.call_timeout(), Duration::from_secs(60));
        assert!(legacy.headers.is_empty());
        let saved = serde_json::to_value(&legacy).unwrap();
        assert_eq!(saved["type"], "stdio");
        assert!(saved.get("url").is_none());
        assert!(saved.get("headers").is_none());

        let http: McpServerConfig = serde_json::from_value(json!({
            "name": "Remote",
            "type": "http",
            "url": "https://mcp.example.com/mcp",
            "headers": { "X-Team": "core" },
            "call_timeout_secs": 5000
        }))
        .unwrap();
        assert_eq!(http.transport, McpTransportKind::Http);
        assert_eq!(http.headers[0].key, "X-Team");
        assert_eq!(http.headers[0].value.as_deref(), Some("core"));
        assert_eq!(
            http.call_timeout(),
            Duration::from_secs(MAX_CALL_TIMEOUT_SECS)
        );
        let round_trip: McpServerConfig =
            serde_json::from_value(serde_json::to_value(&http).unwrap()).unwrap();
        assert_eq!(round_trip.headers[0].key, "X-Team");
        assert_eq!(
            round_trip.url.as_deref(),
            Some("https://mcp.example.com/mcp")
        );

        assert!(McpServerConfig {
            transport: McpTransportKind::Http,
            url: Some("ftp://example.com".into()),
            ..McpServerConfig::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn mcp_tools_extend_the_pipeline_deadline_to_the_call_timeout() {
        let cfg = McpServerConfig {
            call_timeout_secs: Some(300),
            ..McpServerConfig::default()
        };
        let slot = ServerSlot::new(&cfg);
        let defs = vec![McpToolDef {
            name: "slow".into(),
            description: String::new(),
            input_schema: json!({"type":"object"}),
            effect: ToolEffect::Unknown,
            declared_read_only: false,
            model_visible: true,
            app_visible: false,
            ui_resource_uri: None,
            raw: json!({"name":"slow"}),
        }];
        let tools = build_tools(&cfg, &slot, &defs, &McpCapabilities::default()).unwrap();
        assert_eq!(
            tools[0].deadline_for_call(&json!({})),
            Some(Duration::from_secs(305))
        );
    }

    #[tokio::test]
    async fn apps_fixture_negotiates_metadata_isolation_and_result_separation() {
        if !node_available() {
            return;
        }
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("apps_server.js");
        let cfg = McpServerConfig {
            id: "apps-a".into(),
            name: "Apps fixture".into(),
            command: "node".into(),
            args: vec![fixture.to_string_lossy().into_owned()],
            ..McpServerConfig::default()
        };
        let dir = temp_dir("apps-test");
        let hub = McpHub::open(&dir);
        hub.upsert(cfg).await.unwrap();
        let tools = hub.tools();
        let chart = hub.app_tool("apps-a", "show_chart").unwrap();
        assert!(chart.model_visible);
        assert!(chart.app_visible);
        assert_eq!(
            chart.ui_resource_uri.as_deref(),
            Some("ui://milim.test/chart")
        );
        assert_eq!(
            chart.raw["_meta"]["ui"]["resourceUri"],
            "ui://milim.test/chart"
        );
        let refresh = hub.app_tool("apps-a", "refresh_chart").unwrap();
        assert!(!refresh.model_visible);
        assert!(refresh.app_visible);
        assert!(!tools
            .iter()
            .any(|tool| tool.name().contains("refresh_chart")));

        let mut registry = milim_tools::ToolRegistry::new();
        for tool in &tools {
            registry.register(tool.clone());
        }
        let chart_name = tools
            .iter()
            .find(|tool| tool.ui().is_some())
            .unwrap()
            .name()
            .to_string();
        assert_eq!(
            tools
                .iter()
                .find(|tool| tool.name() == chart_name)
                .unwrap()
                .environment_policy(),
            ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
        );
        let generic = registry.call(&chart_name, json!({})).await.unwrap();
        assert!(generic.get("structuredContent").is_some());
        assert!(generic.get("image").is_some());
        assert_eq!(generic["content"][1]["dataOmitted"], true);
        let agent = registry
            .call_for_agent(&chart_name, json!({}))
            .await
            .unwrap();
        assert!(agent.result.get("structuredContent").is_none());
        assert!(agent.result.get("_meta").is_none());
        let app_result = agent.app_result.unwrap();
        assert!(app_result.get("structuredContent").is_some());
        assert!(app_result["content"][1].get("data").is_some());
        assert!(matches!(
            agent.ui,
            Some(ToolUiDescriptor::McpApp { server_id, .. }) if server_id == "apps-a"
        ));

        let resource = hub
            .read_app_resource("apps-a", "ui://milim.test/chart")
            .await
            .unwrap();
        assert_eq!(
            resource["contents"][0]["mimeType"],
            "text/html;profile=mcp-app"
        );
        assert!(hub
            .call_app_tool("apps-a", "refresh_chart", json!({}))
            .await
            .unwrap()
            .get("structuredContent")
            .is_some());
        assert!(hub
            .call_app_tool("other-server", "refresh_chart", json!({}))
            .await
            .is_err());
        assert!(hub
            .read_app_resource("apps-a", "ui://milim.test/not-advertised")
            .await
            .is_err());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn exposed_names_are_namespaced_and_provider_safe() {
        assert_eq!(
            exposed_tool_name("mcp_12345678", "Search-Web"),
            "mcp_12345678__tool_search_web"
        );
        assert_eq!(
            exposed_tool_name("mcp_12345678", "!!!"),
            "mcp_12345678__tool_tool"
        );
    }

    #[test]
    fn answers_server_ping_and_rejects_unadvertised_requests() {
        let ping =
            transport::server_request_response(&json!({"jsonrpc":"2.0","id":7,"method":"ping"}));
        assert_eq!(ping["result"], json!({}));

        let roots = transport::server_request_response(
            &json!({"jsonrpc":"2.0","id":8,"method":"roots/list"}),
        );
        assert_eq!(roots["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn stdio_client_reads_tools_resources_and_prompts() {
        if !node_available() {
            return;
        }

        let script = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});function send(id,result){process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n')}function error(id){process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,error:{code:-32601,message:'not found'}})+'\n')}rl.on('line',line=>{const msg=JSON.parse(line);if(msg.id===undefined)return;if(msg.method==='initialize')return send(msg.id,{protocolVersion:'2025-06-18',capabilities:{tools:{},resources:{},prompts:{}},serverInfo:{name:'mock',version:'1'}});if(msg.method==='tools/list')return send(msg.id,{tools:[{name:'echo',description:'Echo',inputSchema:{type:'object'}}]});if(msg.method==='tools/call')return send(msg.id,{content:[{type:'text',text:'ok'}]});if(msg.method==='resources/list')return send(msg.id,{resources:[{uri:'mock://note',name:'note',mimeType:'text/plain'}]});if(msg.method==='resources/templates/list')return send(msg.id,{resourceTemplates:[{uriTemplate:'mock://{name}',name:'template'}]});if(msg.method==='resources/read')return send(msg.id,{contents:[{uri:msg.params.uri,mimeType:'text/plain',text:'hello'}]});if(msg.method==='prompts/list')return send(msg.id,{prompts:[{name:'review',description:'Review'}]});if(msg.method==='prompts/get')return send(msg.id,{messages:[{role:'user',content:{type:'text',text:'review '+(msg.params.arguments.code||'')}}]});error(msg.id)});"#;

        let args = vec!["-e".to_string(), script.to_string()];
        let client = McpClient::connect("node", &args).await.unwrap();

        let caps = client.capabilities();
        assert!(caps.tools);
        assert!(caps.resources);
        assert!(caps.prompts);
        assert_eq!(client.list_tools().await.unwrap()[0].name, "echo");
        assert_eq!(
            client.list_resources().await.unwrap()[0]["uri"],
            "mock://note"
        );
        assert_eq!(
            client.list_resource_templates().await.unwrap()[0]["uriTemplate"],
            "mock://{name}"
        );
        assert_eq!(
            client.read_resource("mock://note").await.unwrap()["contents"][0]["text"],
            "hello"
        );
        assert_eq!(client.list_prompts().await.unwrap()[0]["name"], "review");
        assert_eq!(
            client
                .get_prompt("review", json!({ "code": "x" }))
                .await
                .unwrap()["messages"][0]["content"]["text"],
            "review x"
        );
        assert_eq!(
            client.call_tool("echo", json!({})).await.unwrap()["content"][0]["text"],
            "ok"
        );
    }

    #[tokio::test]
    async fn configured_mcp_environment_excludes_host_credentials_and_keeps_explicit_grants() {
        if !node_available() {
            return;
        }
        let host_key = format!("MILIM_MCP_HOST_SECRET_{}", std::process::id());
        let host_value = "must-not-cross-mcp-boundary";
        std::env::set_var(&host_key, host_value);
        let script = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});function send(id,result){process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n')}rl.on('line',line=>{const m=JSON.parse(line);if(m.method==='initialize')return send(m.id,{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'env-proof',version:'1'}});if(m.method==='tools/list')return send(m.id,{tools:[{name:'environment',description:'environment proof',inputSchema:{type:'object'}}]});if(m.method==='tools/call')return send(m.id,{content:[{type:'text',text:JSON.stringify({host:process.env[process.env.MILIM_PROBE_KEY]??null,grant:process.env.MCP_EXPLICIT_GRANT??null,path:Boolean(process.env.PATH||process.env.Path)})}]})});"#;
        let env = HashMap::from([
            ("MILIM_PROBE_KEY".to_string(), host_key.clone()),
            (
                "MCP_EXPLICIT_GRANT".to_string(),
                "configured-value".to_string(),
            ),
        ]);
        let client = McpClient::connect_with_env(
            "node",
            &["-e".to_string(), script.to_string()],
            None,
            &env,
        )
        .await
        .unwrap();
        let result = client.call_tool("environment", json!({})).await.unwrap();
        std::env::remove_var(&host_key);
        let proof: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(proof["host"], Value::Null);
        assert_eq!(proof["grant"], "configured-value");
        assert_eq!(proof["path"], true);
        assert!(!serde_json::to_string(&result).unwrap().contains(host_value));
    }

    #[tokio::test]
    async fn dropping_client_kills_server_descendants() {
        if !node_available() {
            return;
        }
        let dir = temp_dir("process-test");
        let pid_path = dir.join("child.pid");
        let script = r#"const fs=require('fs'),cp=require('child_process'),readline=require('readline');const rl=readline.createInterface({input:process.stdin});rl.on('line',line=>{const m=JSON.parse(line);if(m.method!=='initialize')return;const child=cp.spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});fs.writeFileSync(process.env.MILIM_CHILD_PID,String(child.pid));process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result:{protocolVersion:'2025-06-18',capabilities:{},serverInfo:{name:'tree-test',version:'1'}}})+'\n')});"#;
        let env = HashMap::from([(
            "MILIM_CHILD_PID".to_string(),
            pid_path.to_string_lossy().into_owned(),
        )]);
        let client = McpClient::connect_with_env(
            "node",
            &["-e".to_string(), script.to_string()],
            None,
            &env,
        )
        .await
        .unwrap();
        let pid: u32 = std::fs::read_to_string(&pid_path).unwrap().parse().unwrap();

        drop(client);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let mut running = true;
        while running && tokio::time::Instant::now() < deadline {
            running = std::process::Command::new("node")
                .args([
                    "-e",
                    "try{process.kill(Number(process.argv[1]),0)}catch{process.exit(1)}",
                    &pid.to_string(),
                ])
                .status()
                .is_ok_and(|status| status.success());
            #[cfg(target_os = "linux")]
            if running {
                running = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                    !stat
                        .rsplit_once(") ")
                        .is_some_and(|(_, rest)| rest.starts_with('Z'))
                });
            }
            if running {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        if running {
            #[cfg(windows)]
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
            #[cfg(unix)]
            let _ = std::process::Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
        let _ = std::fs::remove_dir_all(dir);
        assert!(!running, "MCP server descendant {pid} survived client drop");
    }

    #[tokio::test]
    async fn repeated_pagination_cursor_is_rejected() {
        if !node_available() {
            return;
        }
        let script = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});const send=(id,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n');rl.on('line',line=>{const m=JSON.parse(line);if(m.id===undefined)return;if(m.method==='initialize')return send(m.id,{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'mock',version:'1'}});if(m.method==='tools/list')return send(m.id,{tools:[],nextCursor:'same'});});"#;
        let client = McpClient::connect("node", &["-e".to_string(), script.to_string()])
            .await
            .unwrap();
        let error = client.list_tools().await.unwrap_err().to_string();
        assert!(error.contains("repeated a pagination cursor"));
    }

    // A stdio server whose tool list grows on `grow` (announced with
    // tools/list_changed plus a log message) and which exits on `crash`.
    const DYNAMIC_SERVER: &str = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});let count=1;const out=m=>process.stdout.write(JSON.stringify(m)+'\n');const send=(id,result)=>out({jsonrpc:'2.0',id,result});rl.on('line',line=>{const m=JSON.parse(line);if(m.id===undefined)return;if(m.method==='initialize')return send(m.id,{protocolVersion:'2025-06-18',capabilities:{tools:{listChanged:true},logging:{}},serverInfo:{name:'dynamic',version:'1'}});if(m.method==='tools/list'){const tools=[{name:'grow',inputSchema:{type:'object'}},{name:'crash',inputSchema:{type:'object'}}];for(let i=1;i<count;i++)tools.push({name:'extra_'+i,inputSchema:{type:'object'},annotations:{readOnlyHint:true}});return send(m.id,{tools})}if(m.method==='tools/call'){if(m.params.name==='crash')process.exit(3);if(m.params.name==='grow'){count++;out({jsonrpc:'2.0',method:'notifications/message',params:{level:'info',logger:'dynamic',data:'grew to '+count}});out({jsonrpc:'2.0',method:'notifications/tools/list_changed'})}return send(m.id,{content:[{type:'text',text:'ok'}]})}});"#;

    #[tokio::test]
    async fn tools_list_changed_refreshes_registered_tools_and_logs() {
        if !node_available() {
            return;
        }
        let dir = temp_dir("list-changed");
        let hub = McpHub::open(&dir);
        let cfg = hub
            .upsert(node_config("dynamic", DYNAMIC_SERVER))
            .await
            .unwrap();
        assert_eq!(hub.tools().len(), 2);
        let grow = hub
            .tools()
            .into_iter()
            .find(|tool| tool.name().ends_with("__tool_grow"))
            .unwrap();
        grow.invoke(json!({})).await.unwrap();
        assert!(wait_for(|| hub.tools().len() == 3, Duration::from_secs(5)).await);
        let extra = hub
            .tools()
            .into_iter()
            .find(|tool| tool.name().ends_with("__tool_extra_1"))
            .unwrap();
        assert_eq!(extra.effect(), ToolEffect::Unknown);
        let info = hub.list().into_iter().find(|s| s.id == cfg.id).unwrap();
        assert_eq!(info.declared_read_only_tools, 1);
        assert!(info.logs.iter().any(
            |entry| entry.message == "grew to 2" && entry.logger.as_deref() == Some("dynamic")
        ));

        hub.upsert(McpServerConfig {
            trust_read_only_hints: true,
            ..cfg
        })
        .await
        .unwrap();
        // A fresh process starts without extras; trusted hints now count.
        assert_eq!(hub.tools().len(), 2);
        let grow = hub
            .tools()
            .into_iter()
            .find(|tool| tool.name().ends_with("__tool_grow"))
            .unwrap();
        grow.invoke(json!({})).await.unwrap();
        assert!(wait_for(|| hub.tools().len() == 3, Duration::from_secs(5)).await);
        let extra = hub
            .tools()
            .into_iter()
            .find(|tool| tool.name().ends_with("__tool_extra_1"))
            .unwrap();
        assert_eq!(extra.effect(), ToolEffect::ReadOnly);
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn crashed_stdio_server_fails_fast_and_reconnects() {
        if !node_available() {
            return;
        }
        let dir = temp_dir("reconnect");
        let hub = McpHub::open(&dir);
        hub.upsert(node_config("crashy", DYNAMIC_SERVER))
            .await
            .unwrap();
        let tools = hub.tools();
        let crash = tools
            .iter()
            .find(|tool| tool.name().ends_with("__tool_crash"))
            .unwrap()
            .clone();
        let grow = tools
            .iter()
            .find(|tool| tool.name().ends_with("__tool_grow"))
            .unwrap()
            .clone();
        let error = crash.invoke(json!({})).await.unwrap_err().to_string();
        assert!(error.contains("connection closed"), "{error}");
        assert!(
            wait_for(
                || hub.list()[0].status == McpConnectionState::Reconnecting,
                Duration::from_secs(2)
            )
            .await
        );
        assert!(hub.tools().is_empty());
        let fast = grow.invoke(json!({})).await.unwrap_err().to_string();
        assert!(fast.contains("disconnected"), "{fast}");
        assert!(
            wait_for(
                || hub.list()[0].status == McpConnectionState::Connected,
                Duration::from_secs(6)
            )
            .await
        );
        assert!(hub.list()[0]
            .logs
            .iter()
            .any(|entry| entry.message.contains("Connection lost")));
        // Tools captured before the crash follow the reconnected client.
        grow.invoke(json!({})).await.unwrap();

        hub.remove("crashy").unwrap();
        assert!(hub.tools().is_empty());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn manual_reconnect_restarts_a_server() {
        if !node_available() {
            return;
        }
        let dir = temp_dir("manual-reconnect");
        let hub = McpHub::open(&dir);
        hub.upsert(node_config("manual", DYNAMIC_SERVER))
            .await
            .unwrap();
        let grow = hub
            .tools()
            .into_iter()
            .find(|tool| tool.name().ends_with("__tool_grow"))
            .unwrap();
        grow.invoke(json!({})).await.unwrap();
        assert!(wait_for(|| hub.tools().len() == 3, Duration::from_secs(5)).await);
        hub.reconnect("manual").await.unwrap();
        assert_eq!(hub.list()[0].status, McpConnectionState::Connected);
        assert_eq!(hub.tools().len(), 2);
        assert!(hub.reconnect("missing").await.is_err());
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn connect_all_runs_servers_in_parallel() {
        if !node_available() {
            return;
        }
        let dir = temp_dir("parallel");
        let slow = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});rl.on('line',line=>{const m=JSON.parse(line);if(m.id===undefined)return;setTimeout(()=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:m.id,result:m.method==='initialize'?{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'slow',version:'1'}}:{tools:[]}})+'\n'),1500)});"#;
        let configs: Vec<McpServerConfig> = (0..3)
            .map(|index| node_config(&format!("slow-{index}"), slow))
            .collect();
        std::fs::write(
            dir.join("mcp.json"),
            serde_json::to_vec(&json!({ "servers": configs })).unwrap(),
        )
        .unwrap();
        let hub = McpHub::open(&dir);
        let started = Instant::now();
        hub.connect_all().await;
        // Each server needs ~3s (initialize + tools/list); serially that
        // would be ~9s. The bound leaves room for slow process startup on
        // CI runners while still failing a serial connect.
        assert!(
            started.elapsed() < Duration::from_millis(6500),
            "{:?}",
            started.elapsed()
        );
        assert!(hub
            .list()
            .iter()
            .all(|server| server.status == McpConnectionState::Connected));
        drop(hub);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn lifts_mcp_image_content() {
        let result = json!({"content":[
            {"type":"text","text":"hi"},
            {"type":"image","data":"AAAA","mimeType":"image/png"}
        ]});
        let lifted = lift_mcp_image(result);
        assert_eq!(lifted["image"]["data"], "AAAA");
        assert_eq!(lifted["image"]["mime"], "image/png");
        assert!(lifted["content"][1].get("data").is_none());
        assert_eq!(lifted["content"][1]["dataOmitted"], true);
    }

    #[test]
    fn lift_mcp_image_noop_without_image() {
        let lifted = lift_mcp_image(json!({"content":[{"type":"text","text":"hi"}]}));
        assert!(lifted.get("image").is_none());
    }

    #[test]
    fn open_missing_is_empty() {
        let dir = std::env::temp_dir().join(format!("milim-mcp-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let hub = McpHub::open(&dir);
        assert!(hub.list().is_empty());
        assert!(hub.tools().is_empty());
    }

    #[tokio::test]
    async fn corrupt_config_is_preserved_and_cannot_be_overwritten() {
        let dir = temp_dir("corrupt-test");
        let path = dir.join("mcp.json");
        std::fs::write(&path, "{broken").unwrap();
        let hub = McpHub::open(&dir);
        let error = hub
            .upsert(McpServerConfig {
                id: "new".into(),
                name: "New".into(),
                command: "node".into(),
                enabled: false,
                ..McpServerConfig::default()
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_existing_secret_key_fails_closed() {
        let dir = temp_dir("key-test");
        let path = dir.join("mcp.key");
        std::fs::write(&path, [1_u8; 8]).unwrap();
        assert!(read_or_make_key(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), [1_u8; 8]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn upsert_reports_save_failure_without_mutating_state() {
        let blocker =
            std::env::temp_dir().join(format!("milim-mcp-blocker-{}", uuid::Uuid::new_v4()));
        let hub = McpHub::open(&blocker);
        std::fs::remove_dir_all(&blocker).unwrap();
        std::fs::write(&blocker, b"not a directory").unwrap();
        let err = hub
            .upsert(McpServerConfig {
                id: "broken".into(),
                name: "Broken".into(),
                command: "node".into(),
                enabled: false,
                ..McpServerConfig::default()
            })
            .await
            .unwrap_err();

        assert_eq!(err.code(), "io_error");
        assert!(hub.list().is_empty());
        let _ = std::fs::remove_file(&blocker);
    }

    #[tokio::test]
    async fn remove_reports_save_failure_without_mutating_state() {
        let dir = temp_dir("remove-test");
        let hub = McpHub::open(&dir);
        hub.upsert(McpServerConfig {
            id: "keep".into(),
            name: "Keep".into(),
            command: "node".into(),
            enabled: false,
            ..McpServerConfig::default()
        })
        .await
        .unwrap();
        let path = dir.join("mcp.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        let err = hub.remove("keep").unwrap_err();

        assert_eq!(err.code(), "io_error");
        assert!(hub.list().iter().any(|server| server.id == "keep"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_config_reports_missing_required_env() {
        let dir = temp_dir("env-test");
        let hub = McpHub::open(&dir);
        let result = hub
            .test_config(McpServerConfig {
                id: "secret-server".into(),
                name: "Secret".into(),
                command: "node".into(),
                env: vec![McpEnvVar {
                    key: "API_KEY".into(),
                    value: None,
                    secret: true,
                    required: true,
                    has_value: false,
                }],
                enabled: false,
                ..McpServerConfig::default()
            })
            .await;
        assert!(!result.ok);
        assert_eq!(result.missing_env, vec!["API_KEY"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn secret_http_headers_are_encrypted_and_hidden() {
        let dir = temp_dir("headers");
        let hub = McpHub::open(&dir);
        hub.upsert(McpServerConfig {
            id: "remote".into(),
            name: "Remote".into(),
            transport: McpTransportKind::Http,
            url: Some("https://mcp.example.com/mcp".into()),
            headers: vec![McpEnvVar {
                key: "Authorization".into(),
                value: Some("Bearer static-secret".into()),
                secret: true,
                required: true,
                has_value: false,
            }],
            enabled: false,
            ..McpServerConfig::default()
        })
        .await
        .unwrap();
        let saved = std::fs::read_to_string(dir.join("mcp.json")).unwrap();
        assert!(!saved.contains("static-secret"));
        let info = &hub.list()[0];
        assert_eq!(info.transport, McpTransportKind::Http);
        assert!(info.headers[0].has_value);
        assert!(info.headers[0].value.is_none());
        assert!(info.missing_env.is_empty());
        let secrets = hub.inner.secrets.clone().unwrap();
        let resolved = resolved_items(
            "remote",
            &hub.config("remote").unwrap().headers,
            HEADER_SECRET_PREFIX,
            Some(&secrets),
            "headers",
        )
        .unwrap();
        assert_eq!(resolved["Authorization"], "Bearer static-secret");
        drop(hub);
        let _ = std::fs::remove_dir_all(&dir);
    }

    mod http {
        use super::*;
        use axum::body::{Body, Bytes};
        use axum::extract::{Query, State};
        use axum::http::{HeaderMap as AxumHeaders, StatusCode};
        use axum::response::{IntoResponse, Response};
        use axum::routing::{get, post};
        use axum::Router;

        #[derive(Clone, Default)]
        struct Fake {
            sse: bool,
            base: Arc<StdMutex<String>>,
            /// Bearer token the MCP endpoint requires, if any.
            token: Option<String>,
            seen: Arc<StdMutex<Vec<Value>>>,
            challenge: Arc<StdMutex<Option<String>>>,
            token_requests: Arc<StdMutex<Vec<HashMap<String, String>>>>,
            legacy: Option<Arc<tokio::sync::Mutex<Option<mpsc::UnboundedSender<String>>>>>,
        }

        fn header(headers: &AxumHeaders, name: &str) -> Value {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null)
        }

        fn rpc_result(message: &Value) -> Value {
            let result = match message["method"].as_str().unwrap_or_default() {
                "initialize" => json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "fake-http", "version": "1" }
                }),
                "tools/list" => json!({ "tools": [
                    { "name": "echo", "inputSchema": { "type": "object" }, "annotations": { "readOnlyHint": true } }
                ]}),
                "tools/call" => json!({ "content": [{ "type": "text", "text": "ok" }] }),
                _ => json!({}),
            };
            json!({ "jsonrpc": "2.0", "id": message["id"], "result": result })
        }

        async fn mcp_post(State(fake): State<Fake>, headers: AxumHeaders, body: Bytes) -> Response {
            let message: Value = serde_json::from_slice(&body).unwrap();
            fake.seen.lock().unwrap().push(json!({
                "method": message["method"],
                "session": header(&headers, "mcp-session-id"),
                "protocol": header(&headers, "mcp-protocol-version"),
                "authorization": header(&headers, "authorization"),
                "accept": header(&headers, "accept"),
            }));
            if let Some(token) = &fake.token {
                if header(&headers, "authorization") != json!(format!("Bearer {token}")) {
                    let base = fake.base.lock().unwrap().clone();
                    return (
                        StatusCode::UNAUTHORIZED,
                        [(
                            "www-authenticate",
                            format!(
                                r#"Bearer resource_metadata="{base}/.well-known/oauth-protected-resource", scope="mcp""#
                            ),
                        )],
                    )
                        .into_response();
                }
            }
            if message.get("id").is_none() {
                return StatusCode::ACCEPTED.into_response();
            }
            let response = rpc_result(&message);
            let mut reply = if fake.sse {
                let log = json!({"jsonrpc":"2.0","method":"notifications/message","params":{"level":"info","data":"working"}});
                (
                    [("content-type", "text/event-stream")],
                    format!("event: message\ndata: {log}\n\nevent: message\ndata: {response}\n\n"),
                )
                    .into_response()
            } else {
                axum::Json(response).into_response()
            };
            if message["method"] == "initialize" {
                reply
                    .headers_mut()
                    .insert("mcp-session-id", "session-1".parse().unwrap());
            }
            reply
        }

        async fn serve(fake: Fake, router: Router) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            *fake.base.lock().unwrap() = base.clone();
            tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            base
        }

        fn mcp_router(fake: Fake) -> Router {
            Router::new()
                .route(
                    "/mcp",
                    post(mcp_post).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                )
                .with_state(fake)
        }

        async fn connect(base: &str) -> Arc<McpClient> {
            McpClient::connect_http(
                reqwest::Client::new(),
                Url::parse(&format!("{base}/mcp")).unwrap(),
                HeaderMap::new(),
                None,
            )
            .await
            .unwrap()
        }

        #[tokio::test]
        async fn json_responses_track_session_and_protocol_version() {
            let fake = Fake::default();
            let base = serve(fake.clone(), mcp_router(fake.clone())).await;
            let client = connect(&base).await;
            assert_eq!(client.list_tools().await.unwrap()[0].name, "echo");
            assert_eq!(
                client.call_tool("echo", json!({})).await.unwrap()["content"][0]["text"],
                "ok"
            );
            let Transport::Http(http) = &client.transport else {
                panic!("expected HTTP transport")
            };
            assert_eq!(http.session_id().as_deref(), Some("session-1"));
            let seen = fake.seen.lock().unwrap().clone();
            assert_eq!(seen[0]["method"], "initialize");
            assert!(seen[0]["session"].is_null());
            assert_eq!(seen[0]["accept"], "application/json, text/event-stream");
            for request in &seen[1..] {
                assert_eq!(request["session"], "session-1", "{request}");
                assert_eq!(request["protocol"], "2025-06-18", "{request}");
            }
            assert!(seen
                .iter()
                .any(|request| request["method"] == "notifications/initialized"));
        }

        #[tokio::test]
        async fn sse_responses_deliver_results_and_notifications() {
            let fake = Fake {
                sse: true,
                ..Fake::default()
            };
            let base = serve(fake.clone(), mcp_router(fake.clone())).await;
            let client = connect(&base).await;
            let mut notifications = client.take_notifications().unwrap();
            assert_eq!(
                client.call_tool("echo", json!({})).await.unwrap()["content"][0]["text"],
                "ok"
            );
            let log = tokio::time::timeout(Duration::from_secs(2), notifications.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(log["method"], "notifications/message");
        }

        #[tokio::test]
        async fn expired_session_closes_the_connection() {
            let fake = Fake::default();
            let expired = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let flag = expired.clone();
            let router = Router::new()
                .route(
                    "/mcp",
                    post(
                        move |state: State<Fake>, headers: AxumHeaders, body: Bytes| {
                            let flag = flag.clone();
                            async move {
                                if flag.load(Ordering::Relaxed) {
                                    return StatusCode::NOT_FOUND.into_response();
                                }
                                mcp_post(state, headers, body).await
                            }
                        },
                    )
                    .get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                )
                .with_state(fake.clone());
            let base = serve(fake, router).await;
            let client = connect(&base).await;
            expired.store(true, Ordering::Relaxed);
            assert!(client.call_tool("echo", json!({})).await.is_err());
            assert_eq!(
                client.closed_reason().as_deref(),
                Some("MCP session expired")
            );
        }

        #[tokio::test]
        async fn legacy_sse_servers_are_used_as_a_fallback() {
            let fake = Fake {
                legacy: Some(Arc::new(tokio::sync::Mutex::new(None))),
                ..Fake::default()
            };
            async fn stream(State(fake): State<Fake>) -> Response {
                let (tx, rx) = mpsc::unbounded_channel::<String>();
                tx.send("event: endpoint\ndata: /messages?sid=1\n\n".into())
                    .unwrap();
                *fake.legacy.as_ref().unwrap().lock().await = Some(tx);
                let body = futures::stream::unfold(rx, |mut rx| async move {
                    rx.recv()
                        .await
                        .map(|event| (Ok::<_, std::io::Error>(Bytes::from(event)), rx))
                });
                (
                    [("content-type", "text/event-stream")],
                    Body::from_stream(body),
                )
                    .into_response()
            }
            async fn message(
                State(fake): State<Fake>,
                Query(query): Query<HashMap<String, String>>,
                body: Bytes,
            ) -> StatusCode {
                assert_eq!(query.get("sid").map(String::as_str), Some("1"));
                let message: Value = serde_json::from_slice(&body).unwrap();
                if message.get("id").is_some() {
                    let response = rpc_result(&message);
                    if let Some(tx) = fake.legacy.as_ref().unwrap().lock().await.as_ref() {
                        let _ = tx.send(format!("event: message\ndata: {response}\n\n"));
                    }
                }
                StatusCode::ACCEPTED
            }
            let router = Router::new()
                .route(
                    "/sse",
                    get(stream).post(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                )
                .route("/messages", post(message))
                .with_state(fake.clone());
            let base = serve(fake, router).await;
            let client = McpClient::connect_http(
                reqwest::Client::new(),
                Url::parse(&format!("{base}/sse")).unwrap(),
                HeaderMap::new(),
                None,
            )
            .await
            .unwrap();
            assert_eq!(client.list_tools().await.unwrap()[0].name, "echo");
            assert_eq!(
                client.call_tool("echo", json!({})).await.unwrap()["content"][0]["text"],
                "ok"
            );
        }

        fn oauth_router(fake: Fake) -> Router {
            async fn resource_metadata(State(fake): State<Fake>) -> axum::Json<Value> {
                let base = fake.base.lock().unwrap().clone();
                axum::Json(json!({
                    "resource": format!("{base}/mcp"),
                    "authorization_servers": [format!("{base}/auth")],
                    "scopes_supported": ["mcp"]
                }))
            }
            async fn server_metadata(State(fake): State<Fake>) -> axum::Json<Value> {
                let base = fake.base.lock().unwrap().clone();
                axum::Json(json!({
                    "issuer": format!("{base}/auth"),
                    "authorization_endpoint": format!("{base}/auth/authorize"),
                    "token_endpoint": format!("{base}/auth/token"),
                    "registration_endpoint": format!("{base}/auth/register"),
                    "code_challenge_methods_supported": ["S256"]
                }))
            }
            async fn register(body: Bytes) -> (StatusCode, axum::Json<Value>) {
                let request: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["token_endpoint_auth_method"], "none");
                assert!(request["redirect_uris"][0]
                    .as_str()
                    .unwrap()
                    .starts_with("http://127.0.0.1:"));
                (
                    StatusCode::CREATED,
                    axum::Json(json!({ "client_id": "registered-client" })),
                )
            }
            async fn token(State(fake): State<Fake>, body: Bytes) -> Response {
                let form: HashMap<String, String> =
                    reqwest::Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&body)))
                        .unwrap()
                        .query_pairs()
                        .into_owned()
                        .collect();
                fake.token_requests.lock().unwrap().push(form.clone());
                let ok = match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code") => {
                        let challenge = fake.challenge.lock().unwrap().clone();
                        form.get("code").map(String::as_str) == Some("auth-code")
                            && form
                                .get("code_verifier")
                                .map(|verifier| oauth::pkce_challenge(verifier))
                                == challenge
                    }
                    Some("refresh_token") => {
                        form.get("refresh_token").map(String::as_str) == Some("refresh-1")
                    }
                    _ => false,
                };
                if !ok {
                    return (
                        StatusCode::BAD_REQUEST,
                        axum::Json(json!({ "error": "invalid_grant" })),
                    )
                        .into_response();
                }
                axum::Json(json!({
                    "access_token": "access-1",
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "refresh_token": "refresh-1"
                }))
                .into_response()
            }
            Router::new()
                .route(
                    "/mcp",
                    post(mcp_post).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
                )
                .route(
                    "/.well-known/oauth-protected-resource",
                    get(resource_metadata),
                )
                .route(
                    "/.well-known/oauth-authorization-server/auth",
                    get(server_metadata),
                )
                .route("/auth/register", post(register))
                .route("/auth/token", post(token))
                .with_state(fake)
        }

        #[tokio::test]
        async fn oauth_discovery_follows_the_resource_metadata_challenge() {
            let fake = Fake {
                token: Some("access-1".into()),
                ..Fake::default()
            };
            let base = serve(fake.clone(), oauth_router(fake)).await;
            let discovery = oauth::discover(
                &reqwest::Client::new(),
                &Url::parse(&format!("{base}/mcp")).unwrap(),
                &HeaderMap::new(),
            )
            .await
            .unwrap();
            assert_eq!(discovery.resource, format!("{base}/mcp"));
            assert_eq!(discovery.scope.as_deref(), Some("mcp"));
            assert_eq!(
                discovery.metadata.token_endpoint.as_str(),
                format!("{base}/auth/token")
            );
            assert!(discovery.metadata.registration_endpoint.is_some());
        }

        #[tokio::test]
        async fn oauth_sign_in_registers_exchanges_and_connects() {
            let fake = Fake {
                token: Some("access-1".into()),
                ..Fake::default()
            };
            let base = serve(fake.clone(), oauth_router(fake.clone())).await;
            let dir = temp_dir("oauth");
            let hub = McpHub::open(&dir);
            hub.upsert(McpServerConfig {
                id: "remote".into(),
                name: "Remote".into(),
                transport: McpTransportKind::Http,
                url: Some(format!("{base}/mcp")),
                ..McpServerConfig::default()
            })
            .await
            .unwrap();
            let info = hub.list().remove(0);
            assert_eq!(info.status, McpConnectionState::AuthRequired);
            assert_eq!(info.auth.status, McpAuthStatus::Required);

            let flow = hub.start_sign_in("remote").await.unwrap();
            let url = Url::parse(flow.url.as_deref().unwrap()).unwrap();
            assert_eq!(url.path(), "/auth/authorize");
            let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
            assert_eq!(query["client_id"], "registered-client");
            assert_eq!(query["code_challenge_method"], "S256");
            assert_eq!(query["resource"], format!("{base}/mcp"));
            assert_eq!(query["scope"], "mcp");
            *fake.challenge.lock().unwrap() = Some(query["code_challenge"].clone());

            // Play the browser: the authorization server redirects back.
            let callback = reqwest::get(format!(
                "{}?code=auth-code&state={}",
                query["redirect_uri"], query["state"]
            ))
            .await
            .unwrap();
            assert_eq!(callback.status(), reqwest::StatusCode::OK);
            assert!(
                wait_for(
                    || hub.list()[0].status == McpConnectionState::Connected,
                    Duration::from_secs(5)
                )
                .await,
                "{:?}",
                hub.list()[0]
            );
            let info = hub.list().remove(0);
            assert_eq!(info.auth.status, McpAuthStatus::SignedIn);
            assert_eq!(info.auth.flow.unwrap().status, "complete");
            assert_eq!(info.tool_count, 1);
            let exchange = fake.token_requests.lock().unwrap()[0].clone();
            assert_eq!(exchange["resource"], format!("{base}/mcp"));
            assert_eq!(exchange["client_id"], "registered-client");
            let saved = std::fs::read_to_string(dir.join("mcp.json")).unwrap();
            assert!(!saved.contains("access-1"));

            // A rejected token is refreshed once and retried.
            let secrets = hub.inner.secrets.clone().unwrap();
            let mut stored = oauth::load_tokens(&secrets, "remote").unwrap().unwrap();
            stored.access_token = "stale".into();
            oauth::save_tokens(&secrets, "remote", &stored).unwrap();
            let echo = hub.tools().remove(0);
            echo.invoke(json!({})).await.unwrap();
            assert!(fake
                .token_requests
                .lock()
                .unwrap()
                .iter()
                .any(|form| form["grant_type"] == "refresh_token"
                    && form["resource"] == format!("{base}/mcp")));

            hub.sign_out("remote").await.unwrap();
            let info = hub.list().remove(0);
            assert_eq!(info.status, McpConnectionState::AuthRequired);
            assert_eq!(info.auth.status, McpAuthStatus::Required);
            drop(hub);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
