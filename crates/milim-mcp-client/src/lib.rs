//! `milim-mcp-client` — a Model Context Protocol **client**.
//!
//! milim already speaks MCP as a *server* (exposing its own tools). This
//! crate is the other direction: it spawns external MCP servers (filesystem,
//! GitHub, Brave-search, …) over stdio, lists their tools, and wraps each one
//! as an [`milim_tools::Tool`] so the agent loop can call them like any builtin.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over the child's stdin/stdout
//! (the MCP stdio transport). A background reader task demuxes responses by id.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use milim_storage::{create_private_file, EncryptedStore};
use milim_tools::{atomic_write, ProcessEnvironmentPolicy, Tool, ToolEffect, ToolUiDescriptor};

const PROTOCOL_VERSION: &str = "2025-06-18";

/// How long to wait for a single JSON-RPC response.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A configured external MCP server (persisted to `mcp.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// Executable to spawn (e.g. `npx`, `uvx`, an absolute path).
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    #[serde(default = "default_true")]
    pub enabled: bool,
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

/// Public, serializable view of a server's state for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<McpEnvVarInfo>,
    pub enabled: bool,
    pub connected: bool,
    pub tool_count: usize,
    pub capabilities: McpCapabilities,
    pub missing_env: Vec<String>,
    pub error: Option<String>,
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
    pub error: Option<String>,
}

struct McpSecretStore {
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

    fn get(&self, server_id: &str, key: &str) -> Result<Option<String>> {
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

    fn put(&self, server_id: &str, key: &str, value: &str) -> Result<()> {
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

/// A live stdio connection to one MCP server.
pub struct McpClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    next_id: AtomicI64,
    capabilities: McpCapabilities,
    // Held so the child and its descendants stay alive with the client.
    _tree: ProcessTreeGuard,
    _child: Mutex<Child>,
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

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Other(format!("failed to spawn MCP server '{command}': {e}")))?;
        let tree =
            ProcessTreeGuard::attach(child.id().ok_or_else(|| {
                Error::Other(format!("MCP server '{command}' has no process id"))
            })?)
            .map_err(|e| Error::Other(format!("failed to contain MCP server '{command}': {e}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("MCP child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("MCP child has no stdout".into()))?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Reader task: demux responses to their waiting callers by id.
        let pending_r = pending.clone();
        let stdin_r = stdin.clone();
        tokio::spawn(async move {
            const MAX_LINE_BYTES: u64 = 8 * 1024 * 1024;
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                let read = (&mut reader)
                    .take(MAX_LINE_BYTES + 1)
                    .read_until(b'\n', &mut line)
                    .await;
                let Ok(read) = read else { break };
                if read == 0 {
                    break;
                }
                if line.len() as u64 > MAX_LINE_BYTES || !line.ends_with(b"\n") {
                    break;
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if msg.get("method").is_some() {
                    if msg.get("id").is_some() {
                        let response = server_request_response(&msg);
                        let _ = write_json(&stdin_r, &response).await;
                    }
                    continue;
                }
                if let Some(id) = msg.get("id").and_then(Value::as_i64) {
                    if let Some(tx) = pending_r.lock().await.remove(&id) {
                        let _ = tx.send(msg);
                    }
                }
                // Server-initiated notifications don't need responses.
            }
            let err = json!({ "error": { "message": "MCP connection closed" } });
            for tx in pending_r.lock().await.drain().map(|(_, tx)| tx) {
                let _ = tx.send(err.clone());
            }
        });

        let next_id = AtomicI64::new(1);
        let init = client_request(
            &stdin,
            &pending,
            &next_id,
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "extensions": {
                        "io.modelcontextprotocol/ui": {
                            "mimeTypes": ["text/html;profile=mcp-app"]
                        }
                    }
                },
                "clientInfo": { "name": "milim", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
        .await?;
        let capabilities = capabilities_from_initialize(&init);

        let client = Arc::new(McpClient {
            stdin,
            pending,
            next_id,
            capabilities,
            _tree: tree,
            _child: Mutex::new(child),
        });
        client
            .notify("notifications/initialized", json!({}))
            .await?;

        Ok(client)
    }

    pub fn capabilities(&self) -> McpCapabilities {
        self.capabilities.clone()
    }

    /// Send a JSON-RPC request and await its response (or time out).
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        client_request(&self.stdin, &self.pending, &self.next_id, method, params).await
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let line = format!(
            "{}\n",
            serde_json::to_string(&json!({
                "jsonrpc": "2.0", "method": method, "params": params
            }))?
        );
        write_line(&self.stdin, &line).await
    }

    /// `tools/list` — enumerate the server's tools.
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
                    effect: match (
                        t.pointer("/annotations/readOnlyHint")
                            .and_then(Value::as_bool),
                        t.pointer("/annotations/destructiveHint")
                            .and_then(Value::as_bool),
                    ) {
                        (_, Some(true)) => ToolEffect::Mutating,
                        (Some(true), _) => ToolEffect::ReadOnly,
                        _ => ToolEffect::Unknown,
                    },
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
        self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
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

async fn client_request(
    stdin: &Arc<Mutex<ChildStdin>>,
    pending: &Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    next_id: &AtomicI64,
    method: &str,
    params: Value,
) -> Result<Value> {
    let id = next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    pending.lock().await.insert(id, tx);

    let line = format!(
        "{}\n",
        serde_json::to_string(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }))?
    );
    if let Err(error) = write_line(stdin, &line).await {
        pending.lock().await.remove(&id);
        return Err(error);
    }

    let resp = tokio::time::timeout(REQUEST_TIMEOUT, rx).await;
    if resp.is_err() {
        pending.lock().await.remove(&id);
    }
    let resp = resp
        .map_err(|_| Error::Other(format!("MCP '{method}' timed out")))?
        .map_err(|_| Error::Other("MCP connection closed".into()))?;

    if let Some(err) = resp.get("error") {
        return Err(Error::Other(format!("MCP '{method}' error: {err}")));
    }
    Ok(resp.get("result").cloned().unwrap_or(Value::Null))
}

async fn write_json(stdin: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<()> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    write_line(stdin, &line).await
}

async fn write_line(stdin: &Arc<Mutex<ChildStdin>>, line: &str) -> Result<()> {
    let mut w = stdin.lock().await;
    w.write_all(line.as_bytes())
        .await
        .map_err(|e| Error::Other(format!("MCP write failed: {e}")))?;
    w.flush()
        .await
        .map_err(|e| Error::Other(format!("MCP flush failed: {e}")))?;
    Ok(())
}

fn server_request_response(req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    match req.get("method").and_then(Value::as_str) {
        Some("ping") => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "method not found" }
        }),
    }
}

/// A tool definition as reported by an MCP server.
#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: ToolEffect,
    pub model_visible: bool,
    pub app_visible: bool,
    pub ui_resource_uri: Option<String>,
    pub raw: Value,
}

/// An [`milim_tools::Tool`] that proxies to a remote MCP tool. Exposed under a
/// prefixed name (`<server>__<tool>`) to avoid colliding with builtins; calls
/// use the original server-side name.
pub struct McpTool {
    client: Arc<McpClient>,
    exposed_name: String,
    remote_name: String,
    description: String,
    schema: Value,
    effect: ToolEffect,
    aliases: Vec<String>,
    ui: Option<ToolUiDescriptor>,
}

struct McpAppCallTool {
    client: Arc<McpClient>,
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

    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        self.client.call_tool(&self.name, args).await
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
        self.client.call_tool(&self.remote_name, args).await
    }
}

struct McpListResourcesTool {
    client: Arc<McpClient>,
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
        let resources = self.client.list_resources().await?;
        let resource_templates = self.client.list_resource_templates().await?;
        Ok(json!({ "resources": resources, "resourceTemplates": resource_templates }))
    }
}

struct McpReadResourceTool {
    client: Arc<McpClient>,
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
        self.client.read_resource(uri).await
    }
}

struct McpListPromptsTool {
    client: Arc<McpClient>,
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
        Ok(json!({ "prompts": self.client.list_prompts().await? }))
    }
}

struct McpGetPromptTool {
    client: Arc<McpClient>,
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
        self.client.get_prompt(name, arguments).await
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

fn missing_env(cfg: &McpServerConfig, secrets: Option<&McpSecretStore>) -> Vec<String> {
    cfg.env
        .iter()
        .filter(|item| item.required)
        .filter_map(|item| {
            let key = env_key(&item.key);
            if key.is_empty() {
                return None;
            }
            let has_value = if item.secret {
                item.value
                    .as_ref()
                    .map(|value| !value.is_empty())
                    .unwrap_or(false)
                    || secrets
                        .map(|store| store.has(&cfg.id, &key))
                        .unwrap_or(false)
            } else {
                item.value
                    .as_ref()
                    .map(|value| !value.is_empty())
                    .unwrap_or(false)
            };
            (!has_value).then_some(key)
        })
        .collect()
}

fn resolved_env(
    cfg: &McpServerConfig,
    secrets: Option<&McpSecretStore>,
) -> Result<HashMap<String, String>> {
    let missing = missing_env(cfg, secrets);
    if !missing.is_empty() {
        return Err(Error::InvalidRequest(format!(
            "missing required env: {}",
            missing.join(", ")
        )));
    }
    let mut out = HashMap::new();
    for item in &cfg.env {
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
                .or_else(|| secrets.and_then(|store| store.get(&cfg.id, &key).ok().flatten()));
            if let Some(value) = value {
                out.insert(key, value);
            }
        } else if let Some(value) = item.value.as_ref().filter(|value| !value.is_empty()) {
            out.insert(key, value.clone());
        }
    }
    Ok(out)
}

// ----- Hub -----

struct HubState {
    configs: Vec<McpServerConfig>,
    clients: HashMap<String, Arc<McpClient>>,
    tools: HashMap<String, Vec<Arc<dyn Tool>>>,
    tool_defs: HashMap<String, HashMap<String, McpToolDef>>,
    errors: HashMap<String, String>,
}

/// Manages the set of configured MCP servers, their live connections, and the
/// merged set of proxy tools. Persists configs to `<dir>/mcp.json`.
pub struct McpHub {
    path: PathBuf,
    load_error: Option<String>,
    secrets: Option<McpSecretStore>,
    inner: RwLock<HubState>,
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
                cfg
            })
            .collect();
        let secrets = match secrets {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::warn!("MCP secret store unavailable: {e}");
                None
            }
        };
        Self {
            path,
            load_error,
            secrets,
            inner: RwLock::new(HubState {
                configs,
                clients: HashMap::new(),
                tools: HashMap::new(),
                tool_defs: HashMap::new(),
                errors: HashMap::new(),
            }),
        }
    }

    /// Connect every enabled configured server, populating tools. Per-server
    /// failures are recorded (not fatal) so one bad server can't block startup.
    pub async fn connect_all(&self) {
        let configs: Vec<McpServerConfig> = {
            let st = self.inner.read().expect("mcp hub poisoned");
            st.configs.iter().filter(|c| c.enabled).cloned().collect()
        };
        for cfg in configs {
            self.connect_into_state(&cfg).await;
        }
    }

    /// Connect one config and store the client+tools (or its error) in state.
    async fn connect_into_state(&self, cfg: &McpServerConfig) {
        match connect_one(cfg, self.secrets.as_ref()).await {
            Ok((client, tools, tool_defs)) => {
                let mut st = self.inner.write().expect("mcp hub poisoned");
                st.clients.insert(cfg.id.clone(), client);
                st.tools.insert(cfg.id.clone(), tools);
                st.tool_defs.insert(cfg.id.clone(), tool_defs);
                st.errors.remove(&cfg.id);
            }
            Err(e) => {
                tracing::warn!("MCP server '{}' failed to connect: {e}", cfg.name);
                let mut st = self.inner.write().expect("mcp hub poisoned");
                st.clients.remove(&cfg.id);
                st.tools.remove(&cfg.id);
                st.tool_defs.remove(&cfg.id);
                st.errors.insert(cfg.id.clone(), e.to_string());
            }
        }
    }

    /// Add or update a server (by id), reconnecting it, then persist.
    pub async fn upsert(&self, mut cfg: McpServerConfig) -> Result<McpServerConfig> {
        if cfg.id.trim().is_empty() {
            cfg.id = format!("mcp-{}", uuid::Uuid::new_v4().simple());
        }
        self.persist_secret_env(&mut cfg)?;
        cfg.env = normalized_env(cfg.env);
        let mut configs = {
            let st = self.inner.read().expect("mcp hub poisoned");
            st.configs
                .iter()
                .filter(|c| c.id != cfg.id)
                .cloned()
                .collect::<Vec<_>>()
        };
        configs.push(cfg.clone());
        self.save_configs(&configs)?;

        // Drop any prior connection/tools for this id first.
        {
            let mut st = self.inner.write().expect("mcp hub poisoned");
            st.configs = configs;
            st.clients.remove(&cfg.id);
            st.tools.remove(&cfg.id);
            st.tool_defs.remove(&cfg.id);
            st.errors.remove(&cfg.id);
        }
        if cfg.enabled {
            self.connect_into_state(&cfg).await;
        }
        Ok(cfg)
    }

    fn persist_secret_env(&self, cfg: &mut McpServerConfig) -> Result<()> {
        for env in &mut cfg.env {
            if !env.secret {
                continue;
            }
            env.key = env_key(&env.key);
            if env.key.is_empty() {
                continue;
            }
            let Some(value) = env.value.take() else {
                continue;
            };
            let store = self
                .secrets
                .as_ref()
                .ok_or_else(|| Error::Other("MCP secret store is not available".to_string()))?;
            if value.is_empty() {
                store.delete(&cfg.id, &env.key)?;
            } else {
                store.put(&cfg.id, &env.key, &value)?;
            }
        }
        Ok(())
    }

    /// Remove a server (dropping its connection, which kills the child).
    pub fn remove(&self, id: &str) -> Result<bool> {
        let (had, configs) = {
            let st = self.inner.read().expect("mcp hub poisoned");
            let had = st.configs.iter().any(|c| c.id == id);
            if !had {
                return Ok(false);
            }
            let configs = st
                .configs
                .iter()
                .filter(|c| c.id != id)
                .cloned()
                .collect::<Vec<_>>();
            (had, configs)
        };
        self.save_configs(&configs)?;
        let had = {
            let mut st = self.inner.write().expect("mcp hub poisoned");
            st.configs = configs;
            st.clients.remove(id);
            st.tools.remove(id);
            st.tool_defs.remove(id);
            st.errors.remove(id);
            had
        };
        if let Some(secrets) = &self.secrets {
            let _ = secrets.delete_server(id);
        }
        Ok(had)
    }

    pub fn config(&self, id: &str) -> Option<McpServerConfig> {
        self.inner
            .read()
            .expect("mcp hub poisoned")
            .configs
            .iter()
            .find(|cfg| cfg.id == id)
            .cloned()
    }

    pub async fn test_config(&self, mut cfg: McpServerConfig) -> McpTestResult {
        if cfg.id.trim().is_empty() {
            cfg.id = format!("mcp-test-{}", uuid::Uuid::new_v4().simple());
        }
        let missing = missing_env(&cfg, self.secrets.as_ref());
        if !missing.is_empty() {
            return McpTestResult {
                ok: false,
                connected: false,
                tool_count: 0,
                capabilities: McpCapabilities::default(),
                missing_env: missing.clone(),
                error: Some(format!("missing required env: {}", missing.join(", "))),
            };
        }
        match connect_one(&cfg, self.secrets.as_ref()).await {
            Ok((client, tools, _)) => McpTestResult {
                ok: true,
                connected: true,
                tool_count: tools.len(),
                capabilities: client.capabilities(),
                missing_env: Vec::new(),
                error: None,
            },
            Err(e) => McpTestResult {
                ok: false,
                connected: false,
                tool_count: 0,
                capabilities: McpCapabilities::default(),
                missing_env: Vec::new(),
                error: Some(e.to_string()),
            },
        }
    }

    /// All currently-available proxy tools across connected servers.
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let state = self.inner.read().expect("mcp hub poisoned");
        let mut ids = state.tools.keys().collect::<Vec<_>>();
        ids.sort();
        ids.into_iter()
            .flat_map(|id| state.tools[id].iter().cloned())
            .collect()
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
            .expect("mcp hub poisoned")
            .tool_defs
            .get(server_id)
            .is_some_and(|tools| {
                tools
                    .values()
                    .any(|tool| tool.ui_resource_uri.as_deref() == Some(uri))
            })
    }

    /// Metadata for an app-callable tool on one server connection.
    pub fn app_tool(&self, server_id: &str, name: &str) -> Option<McpToolDef> {
        self.inner
            .read()
            .expect("mcp hub poisoned")
            .tool_defs
            .get(server_id)?
            .get(name)
            .filter(|tool| tool.app_visible)
            .cloned()
    }

    /// One fixed-server MCP App tool for the central execution pipeline.
    pub fn app_tool_proxy(&self, server_id: &str, name: &str) -> Result<Arc<dyn Tool>> {
        let definition = self.app_tool(server_id, name).ok_or_else(|| {
            Error::InvalidRequest(format!("tool is not app-visible on MCP server: {name}"))
        })?;
        Ok(Arc::new(McpAppCallTool {
            client: self.app_client(server_id)?,
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
        self.app_client(server_id)?.call_tool(name, arguments).await
    }

    fn app_client(&self, server_id: &str) -> Result<Arc<McpClient>> {
        let client = self
            .inner
            .read()
            .expect("mcp hub poisoned")
            .clients
            .get(server_id)
            .cloned()
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
        let st = self.inner.read().expect("mcp hub poisoned");
        st.configs
            .iter()
            .map(|c| {
                let client = st.clients.get(&c.id);
                McpServerInfo {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    command: c.command.clone(),
                    args: c.args.clone(),
                    cwd: c.cwd.clone(),
                    env: self.env_info(c),
                    enabled: c.enabled,
                    connected: client.is_some(),
                    tool_count: st.tools.get(&c.id).map(Vec::len).unwrap_or(0),
                    capabilities: client.map(|c| c.capabilities()).unwrap_or_default(),
                    missing_env: missing_env(c, self.secrets.as_ref()),
                    error: st.errors.get(&c.id).cloned(),
                }
            })
            .collect()
    }

    fn env_info(&self, cfg: &McpServerConfig) -> Vec<McpEnvVarInfo> {
        cfg.env
            .iter()
            .map(|item| {
                let key = env_key(&item.key);
                let has_value = if item.secret {
                    self.secrets
                        .as_ref()
                        .map(|store| store.has(&cfg.id, &key))
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
}

/// Connect a config and build its prefixed proxy tools.
async fn connect_one(
    cfg: &McpServerConfig,
    secrets: Option<&McpSecretStore>,
) -> Result<(
    Arc<McpClient>,
    Vec<Arc<dyn Tool>>,
    HashMap<String, McpToolDef>,
)> {
    let env = resolved_env(cfg, secrets)?;
    let client =
        McpClient::connect_with_env(&cfg.command, &cfg.args, cfg.cwd.as_deref(), &env).await?;
    let defs = client.list_tools().await?;
    let tool_defs = defs
        .iter()
        .cloned()
        .map(|tool| (tool.name.clone(), tool))
        .collect();
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
            client: client.clone(),
            exposed_name: tool_name,
            remote_name: d.name.clone(),
            description: d.description,
            schema: d.input_schema,
            effect: d.effect,
            aliases,
            ui: d
                .ui_resource_uri
                .map(|resource_uri| ToolUiDescriptor::McpApp {
                    server_id: cfg.id.clone(),
                    resource_uri,
                    tool: d.raw,
                }),
        }) as Arc<dyn Tool>);
    }
    let caps = client.capabilities();
    if caps.resources {
        tools.push(Arc::new(McpListResourcesTool {
            client: client.clone(),
            name: exposed_meta_name(&prefix, "list_resources"),
        }));
        tools.push(Arc::new(McpReadResourceTool {
            client: client.clone(),
            name: exposed_meta_name(&prefix, "read_resource"),
        }));
    }
    if caps.prompts {
        tools.push(Arc::new(McpListPromptsTool {
            client: client.clone(),
            name: exposed_meta_name(&prefix, "list_prompts"),
        }));
        tools.push(Arc::new(McpGetPromptTool {
            client: client.clone(),
            name: exposed_meta_name(&prefix, "get_prompt"),
        }));
    }
    Ok((client, tools, tool_defs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_sanitizes() {
        let cfg = McpServerConfig {
            id: "x".into(),
            name: "GitHub MCP!".into(),
            command: "npx".into(),
            args: vec![],
            cwd: None,
            env: Vec::new(),
            enabled: true,
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

    #[tokio::test]
    async fn apps_fixture_negotiates_metadata_isolation_and_result_separation() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
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
            cwd: None,
            env: Vec::new(),
            enabled: true,
        };
        let (client, tools, defs) = connect_one(&cfg, None).await.unwrap();

        assert!(client.capabilities().apps);
        let chart = defs.get("show_chart").unwrap();
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
        let refresh = defs.get("refresh_chart").unwrap();
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

        let dir =
            std::env::temp_dir().join(format!("milim-mcp-apps-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = McpHub::open(&dir);
        {
            let mut state = hub.inner.write().expect("mcp hub poisoned");
            state.clients.insert(cfg.id.clone(), client);
            state.tool_defs.insert(cfg.id.clone(), defs);
        }
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
        let ping = server_request_response(&json!({"jsonrpc":"2.0","id":7,"method":"ping"}));
        assert_eq!(ping["result"], json!({}));

        let roots = server_request_response(&json!({"jsonrpc":"2.0","id":8,"method":"roots/list"}));
        assert_eq!(roots["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn stdio_client_reads_tools_resources_and_prompts() {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
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
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
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
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir =
            std::env::temp_dir().join(format!("milim-mcp-process-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
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
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let script = r#"const readline=require('readline');const rl=readline.createInterface({input:process.stdin});const send=(id,result)=>process.stdout.write(JSON.stringify({jsonrpc:'2.0',id,result})+'\n');rl.on('line',line=>{const m=JSON.parse(line);if(m.id===undefined)return;if(m.method==='initialize')return send(m.id,{protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'mock',version:'1'}});if(m.method==='tools/list')return send(m.id,{tools:[],nextCursor:'same'});});"#;
        let client = McpClient::connect("node", &["-e".to_string(), script.to_string()])
            .await
            .unwrap();
        let error = client.list_tools().await.unwrap_err().to_string();
        assert!(error.contains("repeated a pagination cursor"));
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
        let dir =
            std::env::temp_dir().join(format!("milim-mcp-corrupt-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        std::fs::write(&path, "{broken").unwrap();
        let hub = McpHub::open(&dir);
        let error = hub
            .upsert(McpServerConfig {
                id: "new".into(),
                name: "New".into(),
                command: "node".into(),
                args: Vec::new(),
                cwd: None,
                env: Vec::new(),
                enabled: false,
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_existing_secret_key_fails_closed() {
        let dir = std::env::temp_dir().join(format!("milim-mcp-key-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
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
                args: Vec::new(),
                cwd: None,
                env: Vec::new(),
                enabled: false,
            })
            .await
            .unwrap_err();

        assert_eq!(err.code(), "io_error");
        assert!(hub.list().is_empty());
        let _ = std::fs::remove_file(&blocker);
    }

    #[tokio::test]
    async fn remove_reports_save_failure_without_mutating_state() {
        let dir =
            std::env::temp_dir().join(format!("milim-mcp-remove-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = McpHub::open(&dir);
        hub.upsert(McpServerConfig {
            id: "keep".into(),
            name: "Keep".into(),
            command: "node".into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            enabled: false,
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
        let dir = std::env::temp_dir().join(format!("milim-mcp-env-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let hub = McpHub::open(&dir);
        let result = hub
            .test_config(McpServerConfig {
                id: "secret-server".into(),
                name: "Secret".into(),
                command: "node".into(),
                args: Vec::new(),
                cwd: None,
                env: vec![McpEnvVar {
                    key: "API_KEY".into(),
                    value: None,
                    secret: true,
                    required: true,
                    has_value: false,
                }],
                enabled: false,
            })
            .await;
        assert!(!result.ok);
        assert_eq!(result.missing_env, vec!["API_KEY"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
