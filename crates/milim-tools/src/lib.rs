//! `milim-tools` — the tool registry and built-in tools.
//!
//! A [`Tool`] is an async function with a JSON schema; the [`ToolRegistry`]
//! holds them and is exposed two ways:
//!   - to MCP/HTTP clients via the server's `/mcp/tools` + `/mcp/call`,
//!   - to the agent loop for autonomous tool use.

mod builtins;
mod fs;
mod html;
pub mod shell_command;
mod todo;
mod web_search;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use milim_core::{Error, Result};

pub use builtins::{CurrentTimeTool, EchoTool, HttpFetchTool, RenderChartTool};
pub use fs::{
    atomic_write, fs_tools, read_file_result, read_text_range, resolve_workspace_path, ListDirTool,
    ReadFileTool, WriteFileTool, PATH_DESCRIPTION,
};
pub use todo::{TodoItem, TodoStatus, TodoWriteTool};
pub use web_search::{
    WebSearchApi, WebSearchApiSource, WebSearchProvider, WebSearchQueryFilter, WebSearchTool,
};

/// A callable tool exposed to agents and MCP clients.
#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name (the call identifier).
    fn name(&self) -> &str;
    /// One-line description for the model.
    fn description(&self) -> &str;
    /// JSON Schema for the tool's arguments.
    fn input_schema(&self) -> Value;
    /// The externally visible effect used by approval policy.
    fn effect(&self) -> ToolEffect {
        ToolEffect::Unknown
    }
    /// The effect of one concrete call. Tools whose consequence depends on
    /// their arguments (for example a shell running `git status`) narrow it
    /// here; approval policy and scheduling use this value.
    fn effect_for_call(&self, _args: &Value) -> ToolEffect {
        self.effect()
    }
    /// Deadline for one concrete call. `None` keeps the pipeline default.
    fn deadline_for_call(&self, _args: &Value) -> Option<Duration> {
        None
    }
    /// Whether the tool only waits on other runs (delegated Workers, linked
    /// thread replies). Such calls take no scheduler permits: holding them
    /// while waiting would block the very runs they wait on from executing
    /// their own tools.
    fn waits_on_other_runs(&self) -> bool {
        false
    }
    /// Read-only is necessary but not sufficient for concurrency. Tools must
    /// opt in after proving their implementation is parallel-safe.
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Exclusive
    }
    /// Environment boundary for any process the tool may launch.
    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::HostShellInherited
    }
    /// Optional interactive UI associated with this tool call.
    fn ui(&self) -> Option<ToolUiDescriptor> {
        None
    }
    /// Result projected through the existing generic registry call path.
    fn call_result(&self, result: &Value) -> Value {
        result.clone()
    }
    /// Result projected into the model-visible tool reply.
    fn model_result(&self, result: &Value) -> Value {
        result.clone()
    }
    /// Plain-text projection for the model. When `Some`, the agent loop sends
    /// this text verbatim instead of the JSON-encoded [`Tool::model_result`],
    /// so command output and file contents keep real newlines.
    fn model_text(&self, _result: &Value) -> Option<String> {
        None
    }
    /// Previous names accepted for persisted custom-agent selections.
    fn aliases(&self) -> Vec<String> {
        Vec::new()
    }
    /// Return a copy bound to one run's immutable workspace, when applicable.
    fn scoped_to_workspace(&self, _root: &Path) -> Option<Arc<dyn Tool>> {
        None
    }
    /// Return a copy with unrestricted host access and a fixed working directory.
    fn with_full_access(&self, _cwd: &Path) -> Option<Arc<dyn Tool>> {
        None
    }
    /// Return a copy bound to the task that originated the run, when applicable.
    fn scoped_to_thread(&self, _thread_id: &str) -> Option<Arc<dyn Tool>> {
        None
    }
    /// Return a copy with mutable UI targets captured for one run.
    fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
        None
    }
    /// Execute with the given arguments.
    async fn invoke(&self, args: Value) -> Result<Value>;
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    ReadOnly,
    Mutating,
    Command,
    Unknown,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolConcurrency {
    Parallel,
    Exclusive,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
pub enum ProcessEnvironmentPolicy {
    AccountRuntimeInherited,
    HostShellInherited,
    ConfiguredIntegrationSanitized,
    SandboxSanitized,
}

#[derive(Debug, Clone, Default)]
pub struct ToolExecutionContext {
    pub run_id: Option<String>,
    pub workspace: Option<PathBuf>,
    pub explicit_environment_grants: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolExecutionRequest {
    pub name: String,
    pub arguments: Value,
    pub deadline: Duration,
    pub output_limit_bytes: usize,
}

impl ToolExecutionRequest {
    pub fn new(name: impl Into<String>, arguments: Value) -> Self {
        Self {
            name: name.into(),
            arguments,
            deadline: Duration::from_secs(120),
            output_limit_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub raw: Value,
    pub effect: ToolEffect,
    pub concurrency: ToolConcurrency,
    pub environment_policy: ProcessEnvironmentPolicy,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolExecutionSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: ToolEffect,
    pub concurrency: ToolConcurrency,
    pub environment_policy: ProcessEnvironmentPolicy,
}

pub struct ToolExecutionPipeline;

/// Tool calls running at once across every run in the process.
const PROCESS_TOOL_LIMIT: u32 = 16;
/// Tool calls running at once within one run. An exclusive call takes all of
/// them, so it never overlaps another call of the same run.
const RUN_TOOL_LIMIT: u32 = 4;

/// Model-visible text budget for one tool reply. Tools that page their output
/// (`read_file`, `grep`, `http_fetch`) stay inside it so the agent loop's
/// head+tail replay cut (50 KiB / 2000 lines) never removes the middle of a
/// page while its continuation hint points past the gap.
pub const MODEL_TEXT_BUDGET_BYTES: usize = 40 * 1024;
/// Line counterpart of [`MODEL_TEXT_BUDGET_BYTES`], leaving room for hints.
pub const MODEL_TEXT_BUDGET_LINES: usize = 1000;

fn process_tool_permits() -> &'static Arc<tokio::sync::Semaphore> {
    static LIMIT: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    LIMIT.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(PROCESS_TOOL_LIMIT as usize)))
}

impl ToolExecutionPipeline {
    async fn execute(
        tool: Arc<dyn Tool>,
        request: ToolExecutionRequest,
        run_permits: Arc<tokio::sync::Semaphore>,
        _context: ToolExecutionContext,
    ) -> Result<ToolExecutionResult> {
        let effect = tool.effect_for_call(&request.arguments);
        let deadline = tool
            .deadline_for_call(&request.arguments)
            .unwrap_or(request.deadline);
        let concurrency = match (effect, tool.concurrency()) {
            (ToolEffect::ReadOnly, ToolConcurrency::Parallel) => ToolConcurrency::Parallel,
            _ => ToolConcurrency::Exclusive,
        };
        // Exclusivity is per run: an exclusive call holds its own run's
        // permits, while the process-wide semaphore only caps how many calls
        // run at once, so a long command in one run never stalls another.
        let (permits, process_permits) = if tool.waits_on_other_runs() {
            (0, 0)
        } else if concurrency == ToolConcurrency::Parallel {
            (1, 1)
        } else {
            (RUN_TOOL_LIMIT, 1)
        };
        let _run_guard = run_permits
            .acquire_many_owned(permits)
            .await
            .map_err(|_| Error::Other("tool run scheduler closed".into()))?;
        let _process_guard = process_tool_permits()
            .clone()
            .acquire_many_owned(process_permits)
            .await
            .map_err(|_| Error::Other("tool process scheduler closed".into()))?;
        let started = Instant::now();
        let raw = tokio::time::timeout(deadline, tool.invoke(request.arguments))
            .await
            .map_err(|_| {
                Error::Other(format!(
                    "tool {} exceeded its {:?} deadline",
                    request.name, deadline
                ))
            })??;
        Ok(ToolExecutionResult {
            raw,
            effect,
            concurrency,
            environment_policy: tool.environment_policy(),
            elapsed: started.elapsed(),
        })
    }
}

/// Bound an encoded tool result to `limit` bytes. Oversized string fields are
/// cut to their head and tail around an omission marker, largest first, so
/// the result keeps its shape; only a result that still does not fit becomes
/// a JSON preview. A top-level `image` (see `split_tool_image` in the agent
/// loop) is left whole: it travels to the model as its own message.
fn normalize_tool_output(mut value: Value, limit: usize) -> Value {
    let image = value
        .as_object_mut()
        .and_then(|object| object.remove("image"));
    let mut value = bound_encoded(value, limit.max(1024));
    if let (Some(image), Some(object)) = (image, value.as_object_mut()) {
        object.insert("image".into(), image);
    }
    value
}

/// Room [`cut_middle`]'s omission marker takes in an encoded string.
const CUT_MARKER_BYTES: usize = 48;

fn bound_encoded(value: Value, limit: usize) -> Value {
    let encoded_len = |value: &Value| serde_json::to_vec(value).map_or(0, |encoded| encoded.len());
    let original = encoded_len(&value);
    if original <= limit {
        return value;
    }
    // Cut every string longer than one shared cap, so a short final error
    // survives next to a long log. JSON escaping can make one pass fall
    // short; later passes cut the original again with a larger target.
    let mut reduction = original - limit;
    for _ in 0..4 {
        let mut shrunk = value.clone();
        let mut strings = Vec::new();
        collect_strings(&mut shrunk, &mut strings);
        let Some(cap) = string_cap(&strings, reduction) else {
            break;
        };
        for text in strings {
            if text.len() > cap + CUT_MARKER_BYTES {
                *text = cut_middle(text, cap);
            }
        }
        let size = encoded_len(&shrunk);
        if size <= limit {
            return shrunk;
        }
        reduction += size - limit;
    }
    let encoded = serde_json::to_vec(&value).unwrap_or_default();
    let preview = String::from_utf8_lossy(&encoded[..limit.min(encoded.len())]);
    serde_json::json!({
        "truncated": true,
        "original_bytes": original,
        "preview": preview,
    })
}

fn collect_strings<'a>(value: &'a mut Value, out: &mut Vec<&'a mut String>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter_mut().for_each(|item| collect_strings(item, out)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// The largest per-string length whose cuts save at least `reduction` bytes,
/// or `None` when even empty strings would not.
fn string_cap(strings: &[&mut String], reduction: usize) -> Option<usize> {
    let saved = |cap: usize| {
        strings
            .iter()
            .map(|text| text.len().saturating_sub(cap + CUT_MARKER_BYTES))
            .sum::<usize>()
    };
    let longest = strings.iter().map(|text| text.len()).max()?;
    if saved(0) < reduction {
        return None;
    }
    let (mut low, mut high) = (0, longest);
    while low < high {
        let middle = (low + high).div_ceil(2);
        if saved(middle) >= reduction {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    Some(low)
}

/// Keep about `keep` bytes of `text`, half from each end, around a marker
/// naming how much was left out.
pub fn cut_middle(text: &str, keep: usize) -> String {
    if text.len() <= keep {
        return text.to_string();
    }
    let mut head = keep / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - keep / 2;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{}\n[… {} bytes omitted …]\n{}",
        &text[..head],
        tail - head,
        &text[tail..]
    )
}

/// Interactive UI metadata carried with an agent tool event.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolUiDescriptor {
    McpApp {
        server_id: String,
        resource_uri: String,
        tool: Value,
    },
    NativeChart,
}

/// One tool invocation split into model-visible and app-visible results.
#[derive(Debug, Clone)]
pub struct ToolAgentResult {
    pub result: Value,
    /// Plain-text model projection from [`Tool::model_text`], when provided.
    pub model_text: Option<String>,
    pub app_result: Option<Value>,
    pub ui: Option<ToolUiDescriptor>,
}

static TOOL_OUTPUT_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Register the process-wide directory where oversized tool output is saved
/// for later ranged reads. The first registration wins.
pub fn set_tool_output_root(root: PathBuf) {
    let _ = TOOL_OUTPUT_ROOT.set(root);
}

/// Directory holding saved oversized tool output, when one is registered.
/// File tools may read absolute paths inside it even when workspace-scoped.
pub fn tool_output_root() -> Option<&'static Path> {
    TOOL_OUTPUT_ROOT.get().map(PathBuf::as_path)
}

/// Saved tool output older than this is removed at startup.
pub const TOOL_OUTPUT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Prune saved output older than [`TOOL_OUTPUT_RETENTION`] under `root`, then
/// register it as the process-wide tool output root. Pruning is best-effort.
pub fn init_tool_output_root(root: PathBuf) {
    prune_tool_output(&root, TOOL_OUTPUT_RETENTION);
    set_tool_output_root(root);
}

/// Remove files older than `max_age` under `root` (one level of run
/// directories deep) and drop run directories left empty. Returns the number
/// of removed files.
pub fn prune_tool_output(root: &Path, max_age: Duration) -> usize {
    let Some(cutoff) = std::time::SystemTime::now().checked_sub(max_age) else {
        return 0;
    };
    let expired = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified < cutoff)
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(files) = std::fs::read_dir(&path) {
                for file in files.flatten() {
                    let file = file.path();
                    if file.is_file() && expired(&file) && std::fs::remove_file(&file).is_ok() {
                        removed += 1;
                    }
                }
            }
            // Fails harmlessly while the directory still holds recent output.
            let _ = std::fs::remove_dir(&path);
        } else if path.is_file() && expired(&path) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// A serializable description of a tool (for `/mcp/tools` and tool listings).
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub effect: ToolEffect,
}

/// A name-indexed set of tools.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
    aliases: BTreeMap<String, String>,
    run_permits: Arc<tokio::sync::Semaphore>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self {
            tools: BTreeMap::new(),
            aliases: BTreeMap::new(),
            run_permits: Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize)),
        }
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-populated with the built-in tools.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        #[cfg(debug_assertions)]
        r.register(Arc::new(EchoTool));
        r.register(Arc::new(CurrentTimeTool));
        r.register(Arc::new(HttpFetchTool));
        r.register(Arc::new(RenderChartTool));
        r.register(Arc::new(WebSearchTool::default()));
        r.register(Arc::new(TodoWriteTool::default()));
        r
    }

    /// Add a tool. Existing names win so later registries cannot shadow them.
    pub fn register(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) || self.aliases.contains_key(&name) {
            return self;
        }
        for alias in tool.aliases() {
            if alias != name
                && !self.tools.contains_key(&alias)
                && !self.aliases.contains_key(&alias)
            {
                self.aliases.insert(alias, name.clone());
            }
        }
        self.tools.insert(name, tool);
        self
    }

    /// Add a tool and report a collision to callers handling untrusted names.
    pub fn try_register(&mut self, tool: Arc<dyn Tool>) -> Result<&mut Self> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) || self.aliases.contains_key(&name) {
            return Err(Error::InvalidRequest(format!(
                "duplicate tool name: {name}"
            )));
        }
        let aliases = tool.aliases();
        for alias in aliases {
            if alias != name
                && !self.tools.contains_key(&alias)
                && !self.aliases.contains_key(&alias)
            {
                self.aliases.insert(alias, name.clone());
            }
        }
        self.tools.insert(name, tool);
        Ok(self)
    }

    /// Register the sandboxed filesystem tools rooted at `root`.
    pub fn register_fs(&mut self, root: impl Into<PathBuf>) -> &mut Self {
        for tool in fs::fs_tools(root) {
            self.register(tool);
        }
        self
    }

    /// Bind workspace-aware tools to the root captured when a run starts.
    pub fn scoped_to_workspace(&self, root: &Path) -> Self {
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .map(|(name, tool)| {
                    (
                        name.clone(),
                        tool.scoped_to_workspace(root)
                            .unwrap_or_else(|| tool.clone()),
                    )
                })
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Give host-aware tools unrestricted access while fixing their working directory.
    pub fn with_full_access(&self, cwd: &Path) -> Self {
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .map(|(name, tool)| {
                    (
                        name.clone(),
                        tool.with_full_access(cwd).unwrap_or_else(|| tool.clone()),
                    )
                })
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    pub fn scoped_for_run(&self) -> Self {
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .map(|(name, tool)| {
                    (
                        name.clone(),
                        tool.scoped_for_run().unwrap_or_else(|| tool.clone()),
                    )
                })
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize)),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Bind tools that route task-owned effects to the task that originated the run.
    pub fn scoped_to_thread(&self, thread_id: &str) -> Self {
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .map(|(name, tool)| {
                    (
                        name.clone(),
                        tool.scoped_to_thread(thread_id)
                            .unwrap_or_else(|| tool.clone()),
                    )
                })
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Whether a tool with `name` is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name) || self.aliases.contains_key(name)
    }

    /// Return a registry containing only the named tools.
    pub fn filtered(&self, allowed: &[String]) -> Self {
        let allowed: HashSet<&str> = allowed.iter().map(String::as_str).collect();
        let canonical: HashSet<&str> = allowed
            .iter()
            .filter_map(|name| self.aliases.get(*name).map(String::as_str))
            .chain(allowed.iter().copied())
            .collect();
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .filter(|(name, _)| canonical.contains(name.as_str()))
                .map(|(name, tool)| (name.clone(), tool.clone()))
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Return a registry excluding the named tools.
    pub fn without(&self, denied: &[&str]) -> Self {
        if denied.is_empty() {
            return self.clone();
        }
        let denied: HashSet<&str> = denied.iter().copied().collect();
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .filter(|(name, _)| !denied.contains(name.as_str()))
                .map(|(name, tool)| (name.clone(), tool.clone()))
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Keep only tools that declare themselves read-only.
    pub fn read_only(&self) -> Self {
        let mut registry = Self {
            tools: self
                .tools
                .iter()
                .filter(|(_, tool)| tool.effect() == ToolEffect::ReadOnly)
                .map(|(name, tool)| (name.clone(), tool.clone()))
                .collect(),
            aliases: self.aliases.clone(),
            run_permits: self.run_permits.clone(),
        };
        registry.retain_valid_aliases();
        registry
    }

    /// Specs for all tools, ordered by name.
    pub fn list(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
                effect: t.effect(),
            })
            .collect()
    }

    /// Complete model and execution metadata for run-ledger composition.
    pub fn execution_specs(&self) -> Vec<ToolExecutionSpec> {
        self.tools
            .values()
            .map(|tool| {
                let effect = tool.effect();
                ToolExecutionSpec {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    input_schema: tool.input_schema(),
                    effect,
                    concurrency: match (effect, tool.concurrency()) {
                        (ToolEffect::ReadOnly, ToolConcurrency::Parallel) => {
                            ToolConcurrency::Parallel
                        }
                        _ => ToolConcurrency::Exclusive,
                    },
                    environment_policy: tool.environment_policy(),
                }
            })
            .collect()
    }

    /// Invoke a tool by name.
    pub async fn call(&self, name: &str, args: Value) -> Result<Value> {
        let tool = self.tool(name)?;
        let request = ToolExecutionRequest::new(name, args);
        let limit = request.output_limit_bytes;
        let result = ToolExecutionPipeline::execute(
            tool.clone(),
            request,
            self.run_permits.clone(),
            ToolExecutionContext::default(),
        )
        .await?;
        Ok(normalize_tool_output(tool.call_result(&result.raw), limit))
    }

    /// Invoke a tool while preserving private UI data outside model context.
    pub async fn call_for_agent(&self, name: &str, args: Value) -> Result<ToolAgentResult> {
        let tool = self.tool(name)?;
        let request = ToolExecutionRequest::new(name, args);
        let limit = request.output_limit_bytes;
        let result = ToolExecutionPipeline::execute(
            tool.clone(),
            request,
            self.run_permits.clone(),
            ToolExecutionContext::default(),
        )
        .await?;
        let raw = result.raw;
        let ui = tool.ui();
        Ok(ToolAgentResult {
            result: normalize_tool_output(tool.model_result(&raw), limit),
            // Rendered from the complete result: the agent loop bounds the
            // text itself and saves the full version for later reads.
            model_text: tool.model_text(&raw),
            app_result: ui.is_some().then(|| normalize_tool_output(raw, limit)),
            ui,
        })
    }

    /// Interactive UI metadata for a tool before it is invoked.
    pub fn ui(&self, name: &str) -> Option<ToolUiDescriptor> {
        self.tool(name).ok()?.ui()
    }

    /// Input schema of one tool, resolving aliases the same way as calls.
    pub fn input_schema(&self, name: &str) -> Option<Value> {
        self.tool(name).ok().map(|tool| tool.input_schema())
    }

    /// The registered name `name` resolves to (itself, or the tool an alias
    /// such as a renamed MCP tool's earlier name points at). Policy keyed by
    /// tool name (hooks, approval allowances) can match on this.
    pub fn canonical_name(&self, name: &str) -> Option<String> {
        self.tool(name).ok().map(|tool| tool.name().to_string())
    }

    /// Every other name the tool called `name` answers to: its canonical
    /// name and all its aliases, excluding `name` itself.
    pub fn other_names(&self, name: &str) -> Vec<String> {
        let Some(canonical) = self.canonical_name(name) else {
            return Vec::new();
        };
        std::iter::once(canonical.clone())
            .chain(
                self.aliases
                    .iter()
                    .filter(|(_, target)| **target == canonical)
                    .map(|(alias, _)| alias.clone()),
            )
            .filter(|other| other != name)
            .collect()
    }

    /// Canonical names of all registered tools, ordered by name.
    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Effect declared by a tool, resolving aliases the same way as calls.
    pub fn effect(&self, name: &str) -> Option<ToolEffect> {
        self.tool(name).ok().map(|tool| tool.effect())
    }

    /// Effect of one concrete call, resolving aliases the same way as calls.
    pub fn effect_for_call(&self, name: &str, args: &Value) -> Option<ToolEffect> {
        self.tool(name).ok().map(|tool| tool.effect_for_call(args))
    }

    pub fn environment_policy(&self, name: &str) -> Option<ProcessEnvironmentPolicy> {
        self.tool(name).ok().map(|tool| tool.environment_policy())
    }

    fn tool(&self, name: &str) -> Result<Arc<dyn Tool>> {
        let name = self.aliases.get(name).map(String::as_str).unwrap_or(name);
        self.tools
            .get(name)
            .cloned()
            .ok_or_else(|| Error::InvalidRequest(format!("unknown tool: {name}")))
    }

    fn retain_valid_aliases(&mut self) {
        self.aliases
            .retain(|_, canonical| self.tools.contains_key(canonical));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AliasTool;

    struct TimedTool {
        name: String,
        effect: ToolEffect,
        concurrency: ToolConcurrency,
        delay: Duration,
        current: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl Tool for TimedTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "scheduler fixture"
        }

        fn input_schema(&self) -> Value {
            json!({"type": "object"})
        }

        fn effect(&self) -> ToolEffect {
            self.effect
        }

        fn concurrency(&self) -> ToolConcurrency {
            self.concurrency
        }

        async fn invoke(&self, _args: Value) -> Result<Value> {
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(current, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            if self.fail {
                Err(Error::Other("fixture failed".into()))
            } else {
                Ok(json!({"name": self.name, "payload": "x".repeat(2048)}))
            }
        }
    }

    fn timed_tool(
        name: &str,
        effect: ToolEffect,
        concurrency: ToolConcurrency,
        delay: Duration,
    ) -> (Arc<TimedTool>, Arc<AtomicUsize>) {
        let maximum = Arc::new(AtomicUsize::new(0));
        (
            Arc::new(TimedTool {
                name: name.into(),
                effect,
                concurrency,
                delay,
                current: Arc::new(AtomicUsize::new(0)),
                maximum: maximum.clone(),
                fail: false,
            }),
            maximum,
        )
    }

    #[async_trait]
    impl Tool for AliasTool {
        fn name(&self) -> &str {
            "canonical"
        }
        fn description(&self) -> &str {
            "alias test"
        }
        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }
        fn aliases(&self) -> Vec<String> {
            vec!["legacy".to_string()]
        }
        async fn invoke(&self, _args: Value) -> Result<Value> {
            Ok(json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn registry_lists_and_calls() {
        let reg = ToolRegistry::with_builtins();
        let names: Vec<String> = reg.list().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "current_time",
                "echo",
                "http_fetch",
                "render_chart",
                "todo_write",
                "web_search"
            ]
        ); // BTreeMap → sorted

        let out = reg.call("echo", json!({"text": "hi"})).await.unwrap();
        assert_eq!(out["echoed"]["text"], "hi");

        let t = reg.call("current_time", json!({})).await.unwrap();
        assert!(t["unix"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let reg = ToolRegistry::with_builtins();
        assert!(reg.call("nope", json!({})).await.is_err());
    }

    #[test]
    fn registry_can_exclude_tools() {
        let reg = ToolRegistry::with_builtins();
        assert!(reg.contains("echo"));

        let filtered = reg.without(&["echo"]);
        let names: Vec<String> = filtered.list().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "current_time",
                "http_fetch",
                "render_chart",
                "todo_write",
                "web_search"
            ]
        );
        assert!(!filtered.contains("echo"));
    }

    #[test]
    fn empty_allow_list_exposes_nothing() {
        assert!(ToolRegistry::with_builtins().filtered(&[]).is_empty());
    }

    #[test]
    fn duplicate_registration_keeps_the_first_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        assert!(registry.try_register(Arc::new(EchoTool)).is_err());
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn legacy_aliases_filter_and_call_the_canonical_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(AliasTool));
        let filtered = registry.filtered(&["legacy".to_string()]);
        assert_eq!(filtered.list()[0].name, "canonical");
        assert_eq!(
            registry.canonical_name("legacy").as_deref(),
            Some("canonical")
        );
        assert_eq!(registry.canonical_name("missing"), None);
        assert_eq!(registry.other_names("canonical"), vec!["legacy"]);
        assert_eq!(registry.other_names("legacy"), vec!["canonical"]);
        assert!(registry.other_names("missing").is_empty());
        assert_eq!(
            filtered.call("legacy", json!({})).await.unwrap()["ok"],
            true
        );
    }

    #[tokio::test]
    async fn pipeline_enforces_four_parallel_calls_per_run() {
        let (tool, maximum) = timed_tool(
            "parallel",
            ToolEffect::ReadOnly,
            ToolConcurrency::Parallel,
            Duration::from_millis(20),
        );
        let mut registry = ToolRegistry::new();
        registry.register(tool);
        let registry = registry.scoped_for_run();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..12 {
            let registry = registry.clone();
            tasks.spawn(async move { registry.call("parallel", json!({})).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        assert!(maximum.load(Ordering::SeqCst) <= RUN_TOOL_LIMIT as usize);
    }

    #[tokio::test]
    async fn mutating_and_unknown_tools_are_exclusive_even_if_they_opt_into_parallel() {
        for effect in [
            ToolEffect::Mutating,
            ToolEffect::Command,
            ToolEffect::Unknown,
        ] {
            let (tool, _) = timed_tool(
                "exclusive",
                effect,
                ToolConcurrency::Parallel,
                Duration::ZERO,
            );
            let result = ToolExecutionPipeline::execute(
                tool,
                ToolExecutionRequest::new("exclusive", json!({})),
                Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize)),
                ToolExecutionContext::default(),
            )
            .await
            .unwrap();
            assert_eq!(result.concurrency, ToolConcurrency::Exclusive);
        }
    }

    #[tokio::test]
    async fn exclusive_pipeline_calls_do_not_overlap() {
        let (tool, maximum) = timed_tool(
            "exclusive",
            ToolEffect::Command,
            ToolConcurrency::Parallel,
            Duration::from_millis(20),
        );
        let permits = Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize));
        let left = ToolExecutionPipeline::execute(
            tool.clone(),
            ToolExecutionRequest::new("exclusive", json!({})),
            permits.clone(),
            ToolExecutionContext::default(),
        );
        let right = ToolExecutionPipeline::execute(
            tool,
            ToolExecutionRequest::new("exclusive", json!({})),
            permits,
            ToolExecutionContext::default(),
        );
        let (left, right) = tokio::join!(left, right);
        left.unwrap();
        right.unwrap();
        assert_eq!(maximum.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn prune_tool_output_removes_only_expired_files() {
        let root = std::env::temp_dir().join(format!("milim-prune-{}", std::process::id()));
        let run = root.join("run-1");
        std::fs::create_dir_all(&run).unwrap();
        let old = run.join("old.txt");
        std::fs::write(&old, "old").unwrap();
        let fresh_run = root.join("run-2");
        std::fs::create_dir_all(&fresh_run).unwrap();
        std::fs::write(fresh_run.join("fresh.txt"), "fresh").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(fresh_run.join("fresh.txt"), "fresh").unwrap();
        // Everything written before the sleep is older than the 10ms cutoff.
        let removed = prune_tool_output(&root, Duration::from_millis(10));
        assert_eq!(removed, 1);
        assert!(!old.exists());
        assert!(!run.exists(), "an emptied run directory is removed");
        assert!(fresh_run.join("fresh.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn pipeline_enforces_deadline_without_cancelling_a_sibling() {
        let (slow, _) = timed_tool(
            "slow",
            ToolEffect::ReadOnly,
            ToolConcurrency::Parallel,
            Duration::from_millis(50),
        );
        let (good, _) = timed_tool(
            "good",
            ToolEffect::ReadOnly,
            ToolConcurrency::Parallel,
            Duration::from_millis(1),
        );
        let permits = Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize));
        let mut request = ToolExecutionRequest::new("slow", json!({}));
        request.deadline = Duration::from_millis(5);
        let slow_call = ToolExecutionPipeline::execute(
            slow,
            request,
            permits.clone(),
            ToolExecutionContext::default(),
        );
        let good_call = ToolExecutionPipeline::execute(
            good,
            ToolExecutionRequest::new("good", json!({})),
            permits,
            ToolExecutionContext::default(),
        );
        let (slow_result, good_result) = tokio::join!(slow_call, good_call);
        assert!(slow_result.unwrap_err().to_string().contains("deadline"));
        assert_eq!(good_result.unwrap().raw["name"], "good");
    }

    #[tokio::test]
    async fn exclusive_calls_in_one_run_do_not_block_other_runs() {
        let (slow, _) = timed_tool(
            "slow",
            ToolEffect::Command,
            ToolConcurrency::Exclusive,
            Duration::from_millis(400),
        );
        let (fast, _) = timed_tool(
            "fast",
            ToolEffect::Mutating,
            ToolConcurrency::Exclusive,
            Duration::ZERO,
        );
        let mut registry = ToolRegistry::new();
        registry.register(slow);
        registry.register(fast);
        let first_run = registry.scoped_for_run();
        let second_run = registry.scoped_for_run();
        let slow_call = tokio::spawn(async move { first_run.call("slow", json!({})).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let started = Instant::now();
        second_run.call("fast", json!({})).await.unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "another run's exclusive call waited {:?}",
            started.elapsed()
        );
        assert!(!slow_call.is_finished());
        slow_call.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn exclusive_calls_across_runs_share_the_process_cap() {
        let (tool, maximum) = timed_tool(
            "exclusive",
            ToolEffect::Command,
            ToolConcurrency::Exclusive,
            Duration::from_millis(30),
        );
        let mut registry = ToolRegistry::new();
        registry.register(tool);
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..(PROCESS_TOOL_LIMIT as usize + 8) {
            let run = registry.scoped_for_run();
            tasks.spawn(async move { run.call("exclusive", json!({})).await });
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        let maximum = maximum.load(Ordering::SeqCst);
        assert!(maximum > 1, "separate runs ran exclusive calls together");
        assert!(maximum <= PROCESS_TOOL_LIMIT as usize);
    }

    #[test]
    fn oversized_results_keep_their_shape_and_both_ends_of_long_strings() {
        let value = json!({
            "exit_code": 1,
            "stdout": format!("start{}end-of-stdout", "x".repeat(4000)),
            "stderr": format!("first error{}final error", "y".repeat(3000)),
        });
        let bounded = normalize_tool_output(value, 2048);
        assert!(serde_json::to_vec(&bounded).unwrap().len() <= 2048);
        assert_eq!(bounded["exit_code"], 1);
        let stdout = bounded["stdout"].as_str().unwrap();
        assert!(stdout.starts_with("start") && stdout.ends_with("end-of-stdout"));
        assert!(stdout.contains("bytes omitted"), "{stdout}");
        let stderr = bounded["stderr"].as_str().unwrap();
        assert!(stderr.starts_with("first error") && stderr.ends_with("final error"));

        let image = json!({"image": {"mime": "image/png", "data": "A".repeat(5000)}, "ok": true});
        assert_eq!(normalize_tool_output(image.clone(), 1024), image);

        let wide = Value::Array((0..400).map(|index| json!({ "n": index })).collect());
        let preview = normalize_tool_output(wide, 1024);
        assert_eq!(preview["truncated"], true);
    }

    struct LongOutputTool;

    #[async_trait]
    impl Tool for LongOutputTool {
        fn name(&self) -> &str {
            "long"
        }
        fn description(&self) -> &str {
            "returns more than the output limit"
        }
        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }
        fn model_text(&self, result: &Value) -> Option<String> {
            result["text"].as_str().map(str::to_string)
        }
        async fn invoke(&self, _args: Value) -> Result<Value> {
            Ok(
                json!({ "text": format!("{}\nfinal error: build failed", "line\n".repeat(400_000)) }),
            )
        }
    }

    #[tokio::test]
    async fn text_projection_sees_the_complete_result() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(LongOutputTool));
        let result = registry.call_for_agent("long", json!({})).await.unwrap();
        let text = result.model_text.unwrap();
        assert!(text.ends_with("final error: build failed"));
        assert!(text.len() > 1024 * 1024);
        let visible = result.result["text"].as_str().unwrap();
        assert!(visible.len() < 1024 * 1024);
        assert!(visible.ends_with("final error: build failed"));
    }

    struct DelegatingTool {
        child: Arc<TimedTool>,
    }

    #[async_trait]
    impl Tool for DelegatingTool {
        fn name(&self) -> &str {
            "delegate"
        }
        fn description(&self) -> &str {
            "waits on another run's tool"
        }
        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }
        fn waits_on_other_runs(&self) -> bool {
            true
        }
        async fn invoke(&self, _args: Value) -> Result<Value> {
            // The child run has its own run scheduler but shares the
            // process-wide one with the waiting parent.
            let child_permits = Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize));
            ToolExecutionPipeline::execute(
                self.child.clone(),
                ToolExecutionRequest::new("child", json!({})),
                child_permits,
                ToolExecutionContext::default(),
            )
            .await
            .map(|result| result.raw)
        }
    }

    #[tokio::test]
    async fn waiting_tool_does_not_block_the_run_it_waits_on() {
        let (child, _) = timed_tool(
            "child",
            ToolEffect::Command,
            ToolConcurrency::Exclusive,
            Duration::from_millis(1),
        );
        let parent_permits = Arc::new(tokio::sync::Semaphore::new(RUN_TOOL_LIMIT as usize));
        let call = ToolExecutionPipeline::execute(
            Arc::new(DelegatingTool { child }),
            ToolExecutionRequest::new("delegate", json!({})),
            parent_permits,
            ToolExecutionContext::default(),
        );
        let result = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .expect("the delegated child ran while its parent waited");
        assert!(result.is_ok());
    }
}
