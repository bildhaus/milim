//! `milim-tools` — the tool registry and built-in tools.
//!
//! A [`Tool`] is an async function with a JSON schema; the [`ToolRegistry`]
//! holds them and is exposed two ways:
//!   - to MCP/HTTP clients via the server's `/mcp/tools` + `/mcp/call`,
//!   - to the agent loop for autonomous tool use.

mod builtins;
mod fs;

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
    atomic_write, fs_tools, read_text_range, resolve_workspace_path, ListDirTool, ReadFileTool,
    WriteFileTool,
};

/// A callable tool exposed to agents and MCP clients.
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

const PROCESS_TOOL_LIMIT: u32 = 16;
const RUN_TOOL_LIMIT: u32 = 4;

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
        let effect = tool.effect();
        let concurrency = match (effect, tool.concurrency()) {
            (ToolEffect::ReadOnly, ToolConcurrency::Parallel) => ToolConcurrency::Parallel,
            _ => ToolConcurrency::Exclusive,
        };
        let permits = if concurrency == ToolConcurrency::Parallel {
            1
        } else {
            RUN_TOOL_LIMIT
        };
        let process_permits = if concurrency == ToolConcurrency::Parallel {
            1
        } else {
            PROCESS_TOOL_LIMIT
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
        let raw = tokio::time::timeout(request.deadline, tool.invoke(request.arguments))
            .await
            .map_err(|_| {
                Error::Other(format!(
                    "tool {} exceeded its {:?} deadline",
                    request.name, request.deadline
                ))
            })??;
        let raw = normalize_tool_output(raw, request.output_limit_bytes);
        Ok(ToolExecutionResult {
            raw,
            effect,
            concurrency,
            environment_policy: tool.environment_policy(),
            elapsed: started.elapsed(),
        })
    }
}

fn normalize_tool_output(value: Value, limit: usize) -> Value {
    let encoded = serde_json::to_vec(&value).unwrap_or_default();
    if encoded.len() <= limit.max(1024) {
        return value;
    }
    let preview = String::from_utf8_lossy(&encoded[..limit.max(1024).min(encoded.len())]);
    serde_json::json!({
        "truncated": true,
        "original_bytes": encoded.len(),
        "preview": preview,
    })
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
    pub app_result: Option<Value>,
    pub ui: Option<ToolUiDescriptor>,
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
        let result = ToolExecutionPipeline::execute(
            tool.clone(),
            ToolExecutionRequest::new(name, args),
            self.run_permits.clone(),
            ToolExecutionContext::default(),
        )
        .await?;
        Ok(tool.call_result(&result.raw))
    }

    /// Invoke a tool while preserving private UI data outside model context.
    pub async fn call_for_agent(&self, name: &str, args: Value) -> Result<ToolAgentResult> {
        let tool = self.tool(name)?;
        let result = ToolExecutionPipeline::execute(
            tool.clone(),
            ToolExecutionRequest::new(name, args),
            self.run_permits.clone(),
            ToolExecutionContext::default(),
        )
        .await?;
        let raw = result.raw;
        let ui = tool.ui();
        Ok(ToolAgentResult {
            result: tool.model_result(&raw),
            app_result: ui.is_some().then_some(raw),
            ui,
        })
    }

    /// Interactive UI metadata for a tool before it is invoked.
    pub fn ui(&self, name: &str) -> Option<ToolUiDescriptor> {
        self.tool(name).ok()?.ui()
    }

    /// Effect declared by a tool, resolving aliases the same way as calls.
    pub fn effect(&self, name: &str) -> Option<ToolEffect> {
        self.tool(name).ok().map(|tool| tool.effect())
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
            vec!["current_time", "echo", "http_fetch", "render_chart"]
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
        assert_eq!(names, vec!["current_time", "http_fetch", "render_chart"]);
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

    #[tokio::test]
    async fn pipeline_enforces_deadline_and_output_limit_without_cancelling_a_sibling() {
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
        let mut good_request = ToolExecutionRequest::new("good", json!({}));
        good_request.output_limit_bytes = 1024;
        let good_call = ToolExecutionPipeline::execute(
            good,
            good_request,
            permits,
            ToolExecutionContext::default(),
        );
        let (slow_result, good_result) = tokio::join!(slow_call, good_call);
        assert!(slow_result.unwrap_err().to_string().contains("deadline"));
        assert_eq!(good_result.unwrap().raw["truncated"], true);
    }
}
