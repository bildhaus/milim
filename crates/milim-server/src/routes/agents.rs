use super::*;

mod agent_tools;
mod linked_thread_tools;
mod run_builder;
mod schedule_tools;
mod workers;

pub(crate) use agent_tools::*;
use linked_thread_tools::*;
use run_builder::*;
pub(crate) use schedule_tools::*;
pub(crate) use workers::*;

// ----- Agents -----

#[derive(Serialize)]
struct AgentRunResponse {
    id: String,
    object: &'static str,
    model: String,
    message: ChatMessage,
    steps: Vec<milim_agents::ToolStep>,
    iterations: usize,
    stopped_at_limit: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AgentMemoryContext {
    enabled: bool,
    model: String,
    thread_id: Option<String>,
    project_locator: Option<String>,
    project_label: Option<String>,
    message_id: Option<String>,
    delegation_policy: milim_agents::DelegationPolicy,
    worker_model: Option<String>,
    worker_context: Option<String>,
    linked_thread_grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct AccountRuntimeMilimContext {
    #[serde(default)]
    tool_context: AccountRuntimeToolContext,
    #[serde(default)]
    memory_context: AccountRuntimeMemoryContext,
    #[serde(default = "default_tool_mode")]
    tool_mode: String,
    #[serde(default)]
    enabled_tools: Vec<String>,
    #[serde(default = "default_skill_mode")]
    skill_mode: String,
    #[serde(default)]
    enabled_skills: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AccountRuntimeToolContext {
    parent_model: Option<String>,
    #[serde(default)]
    workspace: RequestValue,
    #[serde(default)]
    privacy_mode: RequestValue,
    tool_approval_policy: Option<String>,
    #[serde(default)]
    tool_approval_grant: bool,
    #[serde(default)]
    interactive_tool_approval: bool,
    #[serde(default)]
    sandbox_enabled: bool,
    #[serde(default)]
    computer_use_enabled: bool,
    #[serde(default)]
    preview_tools_enabled: bool,
    preview_runtime_key: Option<String>,
    #[serde(default)]
    experimental_hashline_patch: bool,
    #[serde(default)]
    plan_mode: bool,
    delegation_policy: Option<String>,
    worker_model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct AccountRuntimeMemoryContext {
    #[serde(default)]
    memory_enabled: bool,
    thread_id: Option<String>,
    project_locator: Option<String>,
    project_label: Option<String>,
    message_id: Option<String>,
    #[serde(default)]
    linked_thread_grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

fn default_tool_mode() -> String {
    "all".to_string()
}

fn default_skill_mode() -> String {
    "auto".to_string()
}

#[derive(Debug, Clone)]
pub(crate) struct AccountRuntimeToolEndpoint {
    pub run_id: String,
    pub url: String,
    pub authorization: String,
    pub tools: Vec<milim_tools::ToolSpec>,
}

pub(crate) fn account_runtime_tool_endpoint(
    st: &AppState,
    headers: &HeaderMap,
    context: Option<&AccountRuntimeMilimContext>,
    run_context: &RunContext,
    model: &str,
    prompt: &str,
) -> Result<Option<AccountRuntimeToolEndpoint>, ApiError> {
    let Some(context) = context else {
        return Ok(None);
    };
    let approval =
        ToolApprovalPolicy::from_requested(context.tool_context.tool_approval_policy.as_deref());
    let policy = ToolRunPolicy {
        approval,
        approval_granted: context.tool_context.tool_approval_grant,
        interactive_approval: context.tool_context.interactive_tool_approval,
        sandbox_enabled: context.tool_context.sandbox_enabled,
        computer_use_enabled: context.tool_context.computer_use_enabled,
        preview_tools_enabled: context.tool_context.preview_tools_enabled,
        experimental_hashline_patch: context.tool_context.experimental_hashline_patch,
        plan_mode: context.tool_context.plan_mode,
        // An account runtime keeps its own history, so this turn's prompt
        // cannot tell whether an earlier turn asked for these tools.
        schedule_tools: true,
        mcp_server_tools: true,
    };
    let memory = AgentMemoryContext {
        enabled: context.memory_context.memory_enabled,
        model: context
            .tool_context
            .parent_model
            .clone()
            .unwrap_or_else(|| model.to_string()),
        thread_id: context.memory_context.thread_id.clone(),
        project_locator: context.memory_context.project_locator.clone(),
        project_label: context.memory_context.project_label.clone(),
        message_id: context.memory_context.message_id.clone(),
        delegation_policy: match context.tool_context.delegation_policy.as_deref() {
            Some("off") => milim_agents::DelegationPolicy::Off,
            Some("auto") => milim_agents::DelegationPolicy::Auto,
            _ => milim_agents::DelegationPolicy::Ask,
        },
        worker_model: context.tool_context.worker_model.clone(),
        worker_context: Some(prompt.chars().take(32_000).collect()),
        linked_thread_grants: context.memory_context.linked_thread_grants.clone(),
    };
    let mut registry = agent_registry_for_mode_with_context(
        st,
        &context.tool_mode,
        &context.enabled_tools,
        Some(memory),
        &policy,
        run_context,
    )
    .without(DESKTOP_WORKSPACE_TOOL_NAMES);
    register_skill_tools(
        &mut registry,
        st,
        &context.skill_mode,
        &context.enabled_skills,
        run_context.workspace(),
    );
    if !policy.plan_mode && registry.contains("preview_open_url") {
        if let (Some(thread_id), Some(cwd)) = (
            context
                .tool_context
                .preview_runtime_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            run_context.workspace.clone(),
        ) {
            for tool in crate::preview_runtime::account_runtime_preview_tools(
                st.preview_runtime.clone(),
                thread_id.to_string(),
                cwd,
            ) {
                registry.register(tool);
            }
        }
    }
    if registry.is_empty() {
        return Ok(None);
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(loopback_host)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "milim account-runtime tools require a loopback server address".into(),
            ))
        })?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let token = uuid::Uuid::new_v4().to_string();
    let endpoint = AccountRuntimeToolEndpoint {
        run_id: run_id.clone(),
        url: format!("http://{host}/internal/account-runtime-tools/{run_id}/mcp"),
        authorization: format!("Bearer {token}"),
        tools: registry.list(),
    };
    st.account_runtime_tools
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            run_id,
            crate::state::AccountRuntimeToolSession {
                token,
                registry,
                review: approval == ToolApprovalPolicy::Review && !policy.approval_granted,
            },
        );
    Ok(Some(endpoint))
}

struct AccountRuntimeToolLease {
    sessions: Arc<Mutex<HashMap<String, crate::state::AccountRuntimeToolSession>>>,
    approvals: Arc<milim_agents::ToolApprovalBroker>,
    run_id: Option<String>,
}

impl Drop for AccountRuntimeToolLease {
    fn drop(&mut self) {
        if let Some(run_id) = &self.run_id {
            self.approvals.fail_run(
                run_id,
                "account runtime disconnected during approval delivery",
            );
            self.sessions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(run_id);
        }
    }
}

pub(crate) fn account_runtime_harness_stream<S>(
    stream: S,
    st: &AppState,
    endpoint: Option<&AccountRuntimeToolEndpoint>,
    relay_notices: bool,
) -> impl futures::Stream<Item = crate::account_runtime_events::HarnessEvent>
where
    S: futures::Stream<Item = crate::account_runtime_events::HarnessEvent>,
{
    use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};

    let run_id = endpoint.map(|endpoint| endpoint.run_id.clone());
    let sessions = st.account_runtime_tools.clone();
    let approvals = st.tool_approvals.clone();
    let mut notices = st.tool_approvals.subscribe();
    async_stream::stream! {
        let _lease = AccountRuntimeToolLease {
            sessions,
            approvals: approvals.clone(),
            run_id: run_id.clone(),
        };
        futures::pin_mut!(stream);
        loop {
            tokio::select! {
                event = stream.next() => match event {
                    Some(event) => {
                        if let Some(run_id) = run_id.as_deref() {
                            approvals.acknowledge_run(run_id);
                        }
                        yield event
                    },
                    None => break,
                },
                notice = notices.recv(), if relay_notices && run_id.is_some() => match notice {
                    Ok(notice) if Some(notice.run_id.as_str()) == run_id.as_deref() => {
                        let mut fields = serde_json::Map::new();
                        fields.insert("approval_id".to_string(), Value::String(notice.approval_id.clone()));
                        fields.insert(
                            "call_id".to_string(),
                            Value::String(notice.call_id.unwrap_or(notice.approval_id)),
                        );
                        let kind = match notice.state {
                            milim_agents::ApprovalState::Requested => {
                                fields.insert("name".to_string(), Value::String(notice.name));
                                fields.insert("arguments".to_string(), Value::String(notice.arguments));
                                fields.insert("effect".to_string(), serde_json::to_value(notice.effect).unwrap_or(Value::String("unknown".to_string())));
                                HarnessEventKind::ApprovalRequested
                            }
                            milim_agents::ApprovalState::Decided
                            | milim_agents::ApprovalState::Delivered => {
                                fields.insert("decision".to_string(), serde_json::to_value(notice.decision).unwrap_or(Value::String("deny".to_string())));
                                fields.insert(
                                    "status".to_string(),
                                    Value::String(if notice.state == milim_agents::ApprovalState::Decided {
                                        "decided"
                                    } else {
                                        "delivered"
                                    }.to_string()),
                                );
                                HarnessEventKind::ApprovalStatus
                            }
                            milim_agents::ApprovalState::Acknowledged => {
                                fields.insert("decision".to_string(), Value::String(notice.decision.unwrap_or("deny").to_string()));
                                HarnessEventKind::ApprovalResolved
                            }
                            milim_agents::ApprovalState::Failed
                            | milim_agents::ApprovalState::Canceled => {
                                if let Some(decision) = notice.decision {
                                    fields.insert("decision".to_string(), Value::String(decision.to_string()));
                                }
                                fields.insert(
                                    "message".to_string(),
                                    Value::String(notice.error.unwrap_or_else(|| "Approval delivery failed".to_string())),
                                );
                                HarnessEventKind::ApprovalFailed
                            }
                        };
                        yield HarnessEvent::new(kind, fields);
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[cfg(test)]
mod account_runtime_tool_tests {
    use super::*;

    #[test]
    fn gateway_requires_loopback_and_removes_finished_sessions() {
        assert_eq!(
            loopback_host("127.0.0.1:1234").as_deref(),
            Some("127.0.0.1:1234")
        );
        assert!(loopback_host("example.com:1234").is_none());

        let sessions = Arc::new(Mutex::new(HashMap::from([(
            "run".to_string(),
            crate::state::AccountRuntimeToolSession {
                token: "token".into(),
                registry: ToolRegistry::new(),
                review: false,
            },
        )])));
        drop(AccountRuntimeToolLease {
            sessions: sessions.clone(),
            approvals: Arc::new(milim_agents::ToolApprovalBroker::default()),
            run_id: Some("run".into()),
        });
        assert!(sessions.lock().unwrap().is_empty());
    }
}

const DESKTOP_WORKSPACE_TOOL_NAMES: &[&str] = &[
    "read_file",
    "read_file_anchors",
    "list_dir",
    "glob",
    "grep",
    "diagnostics",
    "write_file",
    "edit_file",
    "patch_file",
    "shell",
    "process_output",
    "process_kill",
];
const RUN_WORKSPACE_TOOL_NAMES: &[&str] = &["google_drive_transfer"];
pub(crate) const HASHLINE_TOOL_NAMES: &[&str] = &["read_file_anchors", "patch_file"];
const SANDBOX_TOOL_NAMES: &[&str] = &["run_command"];
const COMPUTER_TOOL_NAMES: &[&str] = &[
    "screenshot",
    "mouse_move",
    "mouse_click",
    "key_press",
    "type_text",
    "scroll",
];
const ACTIVE_PREVIEW_TOOL_NAMES: &[&str] = &[
    "preview_dom_snapshot",
    "preview_click",
    "preview_type_text",
    "preview_key_press",
    "preview_scroll",
];
const PREVIEW_OPEN_TOOL_NAMES: &[&str] = &["preview_open_url"];
const SCHEDULE_TOOL_NAMES: &[&str] = &[
    "schedule_create",
    "schedule_update",
    "schedule_list",
    "schedule_delete",
];
const MCP_SERVER_TOOL_NAMES: &[&str] = &[
    "mcp_server_list",
    "mcp_server_test",
    "mcp_server_save",
    "mcp_server_delete",
];
const CHILD_THREAD_TOOL_NAMES: &[&str] = &[
    "delegate_workers",
    "child_thread_spawn",
    "child_thread_list",
    "child_thread_read",
    "child_thread_wait",
    "child_thread_stop",
];
const CHILD_THREAD_READ_ONLY_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "glob",
    "grep",
    "diagnostics",
    "http_fetch",
    "web_search",
    "current_time",
    "echo",
];
const PLAN_MODE_READ_ONLY_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "glob",
    "grep",
    "diagnostics",
    "list_agents",
    "linked_thread_list",
    "linked_thread_read",
];
/// How long a Worker Run may execute. It is measured from when its Workers
/// start, so plan resolution and review-worktree setup do not count, and
/// each Worker schedules its tools on its own, so no Worker waits in another
/// run's tool queue.
const WORKER_RUN_TIMEOUT: Duration = Duration::from_secs(300);
/// Time `delegate_workers` may spend resolving its plan and creating review
/// worktrees before the Workers start.
const WORKER_SETUP_ALLOWANCE: Duration = Duration::from_secs(120);
/// Extra time past a tool's own wait so its timeout handling and cleanup run
/// before the pipeline deadline cancels the call.
const TOOL_WAIT_GRACE: Duration = Duration::from_secs(30);
/// Read-only workspace tools that need a selected working folder.
const WORKSPACE_READ_TOOL_NAMES: &[&str] =
    &["read_file", "list_dir", "glob", "grep", "diagnostics"];
const DEFAULT_LINKED_THREAD_WAIT_MS: u64 = 60_000;
const WORKSPACE_UNAVAILABLE_SYSTEM_PROMPT: &str = concat!(
    "No working folder is selected in Milim. Host filesystem and host shell tools are unavailable. ",
    "If the user asks to create a new file, web app, document, dataset, or other generated artifact ",
    "that is not tied to existing local project files, return it inline as a named fenced code block ",
    "such as ```html file=index.html ... ``` so Milim can capture it in the current chat's artifact panel. ",
    "For browser apps, use index.html plus sibling CSS/JS/TS/TSX files when that is clearer; ",
    "the preview resolves relative links and imports across those artifacts. ",
    "Ask them to pick a folder with the Folder chip only when they want you to read, write, edit, list, ",
    "run commands, or save directly against existing project files."
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolApprovalPolicy {
    Review,
    Guarded,
    Open,
}

impl ToolApprovalPolicy {
    /// Matches the requested policy exactly; anything else, including no
    /// policy, is `Guarded`.
    fn from_requested(value: Option<&str>) -> Self {
        match value {
            Some("review") => Self::Review,
            Some("open") => Self::Open,
            _ => Self::Guarded,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ToolRunPolicy {
    approval: ToolApprovalPolicy,
    approval_granted: bool,
    interactive_approval: bool,
    sandbox_enabled: bool,
    computer_use_enabled: bool,
    preview_tools_enabled: bool,
    experimental_hashline_patch: bool,
    plan_mode: bool,
    /// The conversation asked for scheduled automations, so the default tool
    /// mode includes the `schedule_*` tools.
    schedule_tools: bool,
    /// The conversation asked about MCP servers, so the default tool mode
    /// includes the `mcp_server_*` management tools.
    mcp_server_tools: bool,
}

impl Default for ToolRunPolicy {
    fn default() -> Self {
        Self {
            approval: ToolApprovalPolicy::Guarded,
            approval_granted: false,
            interactive_approval: false,
            sandbox_enabled: false,
            computer_use_enabled: false,
            preview_tools_enabled: false,
            experimental_hashline_patch: false,
            plan_mode: false,
            schedule_tools: false,
            mcp_server_tools: false,
        }
    }
}

impl ToolRunPolicy {
    /// Expose the management tool groups the conversation asks for. Every
    /// user message counts, so a group stays exposed for the rest of the
    /// thread instead of changing the tool list (and invalidating the
    /// provider's prompt cache) from turn to turn.
    fn with_management_tools_for<'a>(
        mut self,
        requests: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        for request in requests {
            self.schedule_tools |= requests_schedule_tools(request);
            self.mcp_server_tools |= requests_mcp_server_tools(request);
        }
        self
    }

    /// Tools the default tool mode leaves out. `current_time` duplicates the
    /// date each turn carries; the schedule and MCP-server groups are large
    /// and only useful when asked for; `list_agents` only helps choose an
    /// `agent_id` for delegation or a schedule, or while planning. A custom
    /// Agent that names one of these tools still gets it.
    fn hidden_default_tools(&self, delegation_available: bool) -> Vec<&'static str> {
        let mut hidden = vec!["current_time"];
        if !self.schedule_tools {
            hidden.extend(SCHEDULE_TOOL_NAMES);
        }
        if !self.mcp_server_tools {
            hidden.extend(MCP_SERVER_TOOL_NAMES);
        }
        if !self.plan_mode && !delegation_available && !self.schedule_tools {
            hidden.push("list_agents");
        }
        hidden
    }
}

fn requests_schedule_tools(text: &str) -> bool {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(
                r"(?i)\b(?:schedul\w*|automat(?:e|es|ion|ions)|cron\w*|recurring|periodic(?:ally)?|hourly|daily|nightly|weekly|monthly|every\s+(?:\d+\s+)?(?:second|minute|hour|day|weekday|week|month|morning|evening|night)s?)\b",
            )
            .expect("valid schedule request pattern")
        })
        .is_match(text)
}

fn requests_mcp_server_tools(text: &str) -> bool {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(r"(?i)\bmcp\b|model context protocol")
                .expect("valid MCP request pattern")
        })
        .is_match(text)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunContext {
    workspace: Option<PathBuf>,
    privacy_mode: crate::privacy::PrivacyMode,
}

#[derive(Clone, Debug, Default)]
enum RequestValue {
    #[default]
    Missing,
    Present(Value),
}

impl RequestValue {
    fn as_value(&self) -> Option<&Value> {
        match self {
            Self::Missing => None,
            Self::Present(value) => Some(value),
        }
    }
}

impl<'de> Deserialize<'de> for RequestValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::Present)
    }
}

impl RunContext {
    fn current(st: &AppState) -> Self {
        Self {
            workspace: workspace_snapshot(st),
            privacy_mode: st.privacy.mode(),
        }
    }

    pub(crate) fn from_request(
        st: &AppState,
        req: &ChatCompletionRequest,
    ) -> milim_core::Result<Self> {
        Self::from_values(
            st,
            req.extra.get("workspace"),
            req.extra.get("privacy_mode"),
        )
    }

    fn from_values(
        st: &AppState,
        workspace: Option<&Value>,
        privacy_mode: Option<&Value>,
    ) -> milim_core::Result<Self> {
        let workspace = match workspace {
            None => workspace_snapshot(st),
            Some(Value::Null) => None,
            Some(Value::String(path)) if !path.trim().is_empty() => {
                Some(canonical_workspace(PathBuf::from(path.trim()))?)
            }
            Some(Value::String(_)) => {
                return Err(Error::InvalidRequest(
                    "workspace must be a non-empty path or null".to_string(),
                ))
            }
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "workspace must be a string path or null".to_string(),
                ))
            }
        };
        let privacy_mode = match privacy_mode {
            None => st.privacy.mode(),
            Some(Value::String(mode)) => explicit_privacy_mode(mode)?,
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "privacy_mode must be off, redact, or block".to_string(),
                ))
            }
        };
        Ok(Self {
            workspace,
            privacy_mode,
        })
    }

    pub(crate) fn from_account_runtime(
        st: &AppState,
        context: Option<&AccountRuntimeMilimContext>,
        cwd: Option<&str>,
    ) -> milim_core::Result<Self> {
        let cwd = cwd
            .map(str::trim)
            .filter(|cwd| !cwd.is_empty())
            .map(|cwd| Value::String(cwd.to_string()));
        let workspace = context
            .and_then(|context| context.tool_context.workspace.as_value())
            .or(cwd.as_ref());
        let privacy_mode = context.and_then(|context| context.tool_context.privacy_mode.as_value());
        Self::from_values(st, workspace, privacy_mode)
    }

    pub(crate) fn from_control(
        st: &AppState,
        workspace: Option<&str>,
        privacy_mode: &str,
    ) -> milim_core::Result<Self> {
        let workspace = workspace.map(|value| Value::String(value.to_string()));
        let privacy_mode = Value::String(privacy_mode.to_string());
        Self::from_values(st, workspace.as_ref(), Some(&privacy_mode))
    }

    fn from_worker_run(run: &milim_agents::WorkerRun) -> milim_core::Result<Self> {
        let privacy_mode = run
            .privacy_mode
            .as_deref()
            .ok_or_else(legacy_worker_run_context_error)
            .and_then(explicit_privacy_mode)?;
        let workspace = run
            .workspace
            .as_deref()
            .map(PathBuf::from)
            .map(canonical_workspace)
            .transpose()?;
        Ok(Self {
            workspace,
            privacy_mode,
        })
    }

    pub(crate) fn workspace(&self) -> Option<&FsPath> {
        self.workspace.as_deref()
    }

    pub(crate) fn privacy_mode(&self) -> crate::privacy::PrivacyMode {
        self.privacy_mode
    }

    pub(crate) fn workspace_text(&self) -> Option<String> {
        self.workspace
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
    }
}

fn canonical_workspace(path: PathBuf) -> milim_core::Result<PathBuf> {
    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        Error::InvalidRequest(format!("invalid workspace {}: {error}", path.display()))
    })?;
    if !canonical.is_dir() {
        return Err(Error::InvalidRequest(format!(
            "workspace is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn explicit_privacy_mode(mode: &str) -> milim_core::Result<crate::privacy::PrivacyMode> {
    match mode {
        "off" => Ok(crate::privacy::PrivacyMode::Off),
        "redact" => Ok(crate::privacy::PrivacyMode::Redact),
        "block" => Ok(crate::privacy::PrivacyMode::Block),
        _ => Err(Error::InvalidRequest(
            "privacy_mode must be off, redact, or block".to_string(),
        )),
    }
}

fn legacy_worker_run_context_error() -> Error {
    Error::InvalidRequest(
        "this worker run predates origin context and cannot be approved, retried, or applied; create a new run"
            .to_string(),
    )
}

pub(crate) fn service_for_run(
    st: &AppState,
    context: &RunContext,
) -> milim_inference::SharedService {
    st.providers
        .as_ref()
        .map(|providers| {
            Arc::new(providers.router_with_privacy(context.privacy_mode))
                as milim_inference::SharedService
        })
        .unwrap_or_else(|| crate::privacy::scoped_service(st.service.clone(), context.privacy_mode))
}

#[cfg(test)]
mod run_context_tests {
    use super::*;
    use milim_inference::test_backend::TestBackend;
    use milim_storage::{Database, UserDataStore};

    struct WorkspaceProbe;

    struct ScopedWorkspaceProbe {
        workspace: PathBuf,
    }

    struct FullAccessWorkspaceProbe {
        workspace: PathBuf,
    }

    struct ThreadProbe(Option<String>);

    #[derive(Clone, Default)]
    struct RecordingRemoteBackend {
        prompts: Arc<Mutex<Vec<String>>>,
        embeddings: Arc<Mutex<Vec<Vec<String>>>>,
    }

    struct NamedProbe(&'static str);

    #[async_trait]
    impl ModelService for RecordingRemoteBackend {
        fn name(&self) -> &str {
            "recording-remote"
        }

        fn requires_privacy_gate(&self) -> bool {
            true
        }

        async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
            Ok(vec![Model::local("recording-model", 0)])
        }

        async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
            self.prompts.lock().unwrap().push(req.last_user_text());
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn embed(
            &self,
            _model: &str,
            inputs: Vec<String>,
        ) -> milim_core::Result<Vec<Vec<f32>>> {
            self.embeddings.lock().unwrap().push(inputs.clone());
            Ok(inputs
                .iter()
                .map(|input| vec![input.len() as f32])
                .collect())
        }
    }

    #[async_trait]
    impl Tool for NamedProbe {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "No-op test tool."
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Ok(Value::Null)
        }
    }

    #[async_trait]
    impl Tool for ThreadProbe {
        fn name(&self) -> &str {
            "thread_probe"
        }

        fn description(&self) -> &str {
            "Return the task bound to this run."
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        fn scoped_to_thread(&self, thread_id: &str) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(Self(Some(thread_id.to_string()))))
        }

        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Ok(json!({"thread_id": self.0}))
        }
    }

    #[async_trait]
    impl Tool for WorkspaceProbe {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Return the workspace bound to this run."
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        fn scoped_to_workspace(&self, root: &FsPath) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(ScopedWorkspaceProbe {
                workspace: root.to_path_buf(),
            }))
        }

        fn with_full_access(&self, cwd: &FsPath) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(FullAccessWorkspaceProbe {
                workspace: cwd.to_path_buf(),
            }))
        }

        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Err(Error::InvalidRequest(
                "workspace probe was not scoped".to_string(),
            ))
        }
    }

    #[async_trait]
    impl Tool for FullAccessWorkspaceProbe {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Return the full-access working directory bound to this run."
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Ok(json!({"workspace": self.workspace, "full_access": true}))
        }
    }

    #[async_trait]
    impl Tool for ScopedWorkspaceProbe {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Return the workspace bound to this run."
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Ok(json!({"workspace": self.workspace}))
        }
    }

    fn temp_workspace_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "milim-run-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn request(value: Value) -> ChatCompletionRequest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn explicit_and_legacy_request_contexts_snapshot_once() {
        let root = temp_workspace_root();
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        );
        *state.workspace.write().unwrap() = Some(first.clone());

        let legacy =
            RunContext::from_request(&state, &request(json!({"model":"test-echo","messages":[]})))
                .unwrap();
        let explicit = RunContext::from_request(
            &state,
            &request(json!({
                "model":"test-echo",
                "messages":[],
                "workspace":second,
                "privacy_mode":"block"
            })),
        )
        .unwrap();

        *state.workspace.write().unwrap() = None;
        state.privacy.set(crate::privacy::PrivacyMode::Redact);
        assert_eq!(legacy.workspace, Some(first));
        assert_eq!(legacy.privacy_mode, crate::privacy::PrivacyMode::Off);
        assert_eq!(
            explicit.workspace,
            Some(std::fs::canonicalize(second).unwrap())
        );
        assert_eq!(explicit.privacy_mode, crate::privacy::PrivacyMode::Block);

        let null_workspace = RunContext::from_request(
            &state,
            &request(json!({
                "model":"test-echo",
                "messages":[],
                "workspace":null,
                "privacy_mode":"off"
            })),
        )
        .unwrap();
        assert!(null_workspace.workspace.is_none());
        assert!(RunContext::from_request(
            &state,
            &request(json!({
                "model":"test-echo",
                "messages":[],
                "workspace":root.join("missing"),
                "privacy_mode":"off"
            }))
        )
        .is_err());
        assert!(RunContext::from_request(
            &state,
            &request(json!({
                "model":"test-echo",
                "messages":[],
                "workspace":null,
                "privacy_mode":"invalid"
            }))
        )
        .is_err());

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn standalone_remote_service_uses_captured_privacy_mode() {
        let backend = RecordingRemoteBackend::default();
        let state = AppState::new(
            Arc::new(backend.clone()),
            milim_core::config::ServerConfiguration::default(),
        );
        let block_context = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Block,
        };
        let block_error = service_for_run(&state, &block_context)
            .stream(CompletionRequest {
                model: "recording-model".to_string(),
                messages: vec![ChatMessage::text("user", "email person@example.com")],
                tools: Vec::new(),
                tool_choice: None,
                response_format: None,
                prompt: None,
                suffix: None,
                sampling: Default::default(),
                reasoning_effort: None,
            })
            .await
            .err()
            .expect("captured block mode should reject PII");
        assert!(block_error
            .to_string()
            .contains("blocked by the privacy gate"));
        assert!(backend.prompts.lock().unwrap().is_empty());

        let redact_context = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Redact,
        };
        service_for_run(&state, &redact_context)
            .embed(
                "recording-model",
                vec!["email person@example.com".to_string()],
            )
            .await
            .unwrap();
        let embeddings = backend.embeddings.lock().unwrap();
        assert_eq!(embeddings.len(), 1);
        assert_eq!(embeddings[0], ["email [EMAIL_1]"]);
    }

    #[test]
    fn worker_review_without_workspace_hides_workspace_tools() {
        let mut tools = ToolRegistry::new();
        for name in [
            "read_file",
            "shell",
            "google_drive_transfer",
            "current_time",
        ] {
            tools.register(Arc::new(NamedProbe(name)));
        }
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        *state.workspace.write().unwrap() = Some(PathBuf::from("later-selected-workspace"));

        let no_workspace = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let review = worker_review_registry(&state, &no_workspace);
        assert!(!review.contains("read_file"));
        assert!(!review.contains("shell"));
        assert!(!review.contains("google_drive_transfer"));
        assert!(review.contains("current_time"));

        let captured_workspace = RunContext {
            workspace: Some(PathBuf::from("captured-workspace")),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let review = worker_review_registry(&state, &captured_workspace);
        assert!(review.contains("read_file"));
        assert!(review.contains("shell"));
        assert!(review.contains("google_drive_transfer"));
    }

    #[tokio::test]
    async fn memory_registration_uses_run_scoped_privacy() {
        let backend = RecordingRemoteBackend::default();
        let memory = milim_memory::MemoryStore::new(
            milim_storage::Database::open_in_memory().unwrap(),
            Arc::new(backend.clone()),
        )
        .unwrap();
        let state = AppState::new(
            Arc::new(backend.clone()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_memory(memory);
        state.privacy.set(crate::privacy::PrivacyMode::Off);
        let run_context = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Redact,
        };
        let policy = ToolRunPolicy {
            approval: ToolApprovalPolicy::Open,
            ..Default::default()
        };
        let registry = agent_base_registry_with_memory(
            &state,
            Some(AgentMemoryContext {
                enabled: true,
                model: "recording-model".to_string(),
                ..Default::default()
            }),
            &policy,
            &run_context,
        );

        registry
            .call(
                "memory_register",
                json!({"scope":"personal","content":"Contact person@example.com"}),
            )
            .await
            .unwrap();

        let embeddings = backend.embeddings.lock().unwrap();
        assert_eq!(embeddings.len(), 1);
        assert!(!embeddings[0][0].contains("person@example.com"));
        assert!(embeddings[0][0].contains("[EMAIL_"));
    }

    #[test]
    fn linked_thread_tools_follow_plan_guarded_review_open_and_custom_modes() {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        let control = crate::control::RunManager::new(store, "Tool fixture").unwrap();
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_control(control);
        let run_context = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let memory = AgentMemoryContext {
            thread_id: Some("origin".into()),
            message_id: Some("run-origin".into()),
            linked_thread_grants: vec![crate::control::FrozenLinkedThreadGrantV1 {
                target_thread_id: "target".into(),
                title: "Target".into(),
                workspace: Some("C:/projects/target".into()),
                project: Some("target".into()),
                model: Some("mock-echo".into()),
                runtime: "mock".into(),
                revision: 4,
                epoch: "epoch-target".into(),
                max_timeline_seq: 8,
            }],
            ..Default::default()
        };

        let plan = agent_base_registry_with_memory(
            &state,
            Some(memory.clone()),
            &ToolRunPolicy {
                plan_mode: true,
                ..Default::default()
            },
            &run_context,
        );
        assert!(plan.contains("linked_thread_list"));
        assert!(plan.contains("linked_thread_read"));
        assert!(!plan.contains("linked_thread_send"));
        assert!(!plan.contains("linked_thread_wait"));

        let guarded = agent_base_registry_with_memory(
            &state,
            Some(memory.clone()),
            &ToolRunPolicy::default(),
            &run_context,
        );
        assert!(guarded.contains("linked_thread_list"));
        assert!(guarded.contains("linked_thread_read"));
        assert!(!guarded.contains("linked_thread_send"));
        assert!(!guarded.contains("linked_thread_wait"));

        let review = agent_base_registry_with_memory(
            &state,
            Some(memory.clone()),
            &ToolRunPolicy {
                approval: ToolApprovalPolicy::Review,
                interactive_approval: true,
                ..Default::default()
            },
            &run_context,
        );
        assert!(review.contains("linked_thread_send"));
        assert!(review.contains("linked_thread_wait"));

        let open = ToolRunPolicy {
            approval: ToolApprovalPolicy::Open,
            ..Default::default()
        };
        let open_registry =
            agent_base_registry_with_memory(&state, Some(memory.clone()), &open, &run_context);
        assert!(open_registry.contains("linked_thread_send"));
        assert!(open_registry.contains("linked_thread_wait"));
        let custom = agent_registry_for_mode_with_context(
            &state,
            "custom",
            &["linked_thread_read".into()],
            Some(memory),
            &open,
            &run_context,
        );
        assert!(custom.contains("linked_thread_read"));
        assert!(!custom.contains("linked_thread_list"));
        assert!(!custom.contains("linked_thread_send"));
        assert!(!custom.contains("linked_thread_wait"));

        let account_context: AccountRuntimeMilimContext = serde_json::from_value(json!({
            "tool_context": {
                "tool_approval_policy": "open",
                "privacy_mode": "off"
            },
            "memory_context": {
                "thread_id": "origin",
                "message_id": "run-origin",
                "linked_thread_grants": [{
                    "target_thread_id": "target",
                    "title": "Target",
                    "workspace": "C:/projects/target",
                    "project": "target",
                    "model": "mock-echo",
                    "runtime": "mock",
                    "revision": 4,
                    "epoch": "epoch-target",
                    "max_timeline_seq": 8
                }]
            }
        }))
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "127.0.0.1:7377".parse().unwrap());
        let endpoint = account_runtime_tool_endpoint(
            &state,
            &headers,
            Some(&account_context),
            &run_context,
            "mock-echo",
            "fixture",
        )
        .map_err(|error| error.0)
        .unwrap()
        .unwrap();
        let names = endpoint
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"linked_thread_list"));
        assert!(names.contains(&"linked_thread_read"));
        assert!(names.contains(&"linked_thread_send"));
        assert!(names.contains(&"linked_thread_wait"));
    }

    #[test]
    fn account_runtime_context_prefers_explicit_fields_and_captures_legacy_cwd() {
        let root = temp_workspace_root();
        let legacy = root.join("legacy");
        let explicit = root.join("explicit");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&explicit).unwrap();
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        );
        state.privacy.set(crate::privacy::PrivacyMode::Redact);
        let context: AccountRuntimeMilimContext = serde_json::from_value(json!({
            "tool_context": {
                "workspace": explicit,
                "privacy_mode": "block"
            }
        }))
        .unwrap();

        let captured = RunContext::from_account_runtime(
            &state,
            Some(&context),
            Some(legacy.to_string_lossy().as_ref()),
        )
        .unwrap();
        assert_eq!(
            captured.workspace,
            Some(std::fs::canonicalize(&explicit).unwrap())
        );
        assert_eq!(captured.privacy_mode, crate::privacy::PrivacyMode::Block);

        let legacy_context =
            RunContext::from_account_runtime(&state, None, Some(legacy.to_string_lossy().as_ref()))
                .unwrap();
        state.privacy.set(crate::privacy::PrivacyMode::Off);
        assert_eq!(
            legacy_context.workspace,
            Some(std::fs::canonicalize(&legacy).unwrap())
        );
        assert_eq!(
            legacy_context.privacy_mode,
            crate::privacy::PrivacyMode::Redact
        );

        let null_context: AccountRuntimeMilimContext = serde_json::from_value(json!({
            "tool_context": {
                "workspace": null,
                "privacy_mode": "off"
            }
        }))
        .unwrap();
        assert!(RunContext::from_account_runtime(
            &state,
            Some(&null_context),
            Some(legacy.to_string_lossy().as_ref()),
        )
        .unwrap()
        .workspace
        .is_none());

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn account_runtime_preview_tools_bind_to_the_active_project() {
        let root = temp_workspace_root();
        std::fs::create_dir_all(&root).unwrap();
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(NamedProbe("preview_open_url")));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        let context: AccountRuntimeMilimContext = serde_json::from_value(json!({
            "tool_context": {
                "workspace": root,
                "privacy_mode": "off",
                "tool_approval_policy": "open",
                "preview_runtime_key": "project-test"
            }
        }))
        .unwrap();
        let run_context = RunContext::from_account_runtime(&state, Some(&context), None).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(HOST, "127.0.0.1:7377".parse().unwrap());

        let endpoint = account_runtime_tool_endpoint(
            &state,
            &headers,
            Some(&context),
            &run_context,
            "test-model",
            "preview this",
        )
        .map_err(|error| error.0)
        .unwrap()
        .unwrap();
        let names = endpoint
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"preview_prepare_app"));
        assert!(names.contains(&"preview_start_app"));

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn simultaneous_two_iteration_runs_keep_workspace_origin_100_times() {
        let root = temp_workspace_root();
        let left = root.join("left");
        let right = root.join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::write(left.join("AGENTS.md"), "LEFT_ONLY").unwrap();
        std::fs::write(right.join("AGENTS.md"), "RIGHT_ONLY").unwrap();
        let left = std::fs::canonicalize(left).unwrap();
        let right = std::fs::canonicalize(right).unwrap();

        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(WorkspaceProbe));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        let left_context = RunContext {
            workspace: Some(left.clone()),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let right_context = RunContext {
            workspace: Some(right.clone()),
            privacy_mode: crate::privacy::PrivacyMode::Block,
        };
        let left_tools = static_registry_for_context(&state, &left_context);
        let right_tools = static_registry_for_context(&state, &right_context);
        let service = TestBackend::new();
        let mut left_messages = vec![ChatMessage::text("user", "/tool left")];
        let mut right_messages = vec![ChatMessage::text("user", "/tool right")];
        add_workspace_instructions_for(&mut left_messages, Some(&left));
        add_workspace_instructions_for(&mut right_messages, Some(&right));
        assert!(left_messages[0].text_content().contains("LEFT_ONLY"));
        assert!(!left_messages[0].text_content().contains("RIGHT_ONLY"));
        assert!(right_messages[0].text_content().contains("RIGHT_ONLY"));
        assert!(!right_messages[0].text_content().contains("LEFT_ONLY"));

        for _ in 0..100 {
            let (left_run, right_run) = tokio::join!(
                milim_agents::run_agent(
                    &service,
                    &left_tools,
                    "test-echo",
                    left_messages.clone(),
                    None
                ),
                milim_agents::run_agent(
                    &service,
                    &right_tools,
                    "test-echo",
                    right_messages.clone(),
                    None
                )
            );
            let left_run = left_run.unwrap();
            let right_run = right_run.unwrap();
            assert_eq!(left_run.iterations, 2);
            assert_eq!(right_run.iterations, 2);
            assert_eq!(
                left_run.steps[0].result["workspace"],
                json!(left.to_string_lossy())
            );
            assert_eq!(
                right_run.steps[0].result["workspace"],
                json!(right.to_string_lossy())
            );
            assert_eq!(left_context.privacy_mode, crate::privacy::PrivacyMode::Off);
            assert_eq!(
                right_context.privacy_mode,
                crate::privacy::PrivacyMode::Block
            );
        }

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn open_registry_gives_parent_and_workers_full_access() {
        let root = temp_workspace_root();
        std::fs::create_dir_all(&root).unwrap();
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(WorkspaceProbe));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        let run_context = RunContext {
            workspace: Some(root.clone()),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let open = ToolRunPolicy {
            approval: ToolApprovalPolicy::Open,
            ..Default::default()
        };
        let parent = agent_base_registry_with_memory(&state, None, &open, &run_context);
        assert_eq!(
            parent.call("echo", json!({})).await.unwrap()["full_access"],
            true
        );
        let worker = child_registry_for_policy(&state, &open, parent, &run_context).read_only();
        assert_eq!(
            worker.call("echo", json!({})).await.unwrap()["full_access"],
            true
        );

        let review = ToolRunPolicy {
            approval: ToolApprovalPolicy::Review,
            approval_granted: true,
            ..Default::default()
        };
        let scoped = agent_base_registry_with_memory(&state, None, &review, &run_context);
        assert!(scoped
            .call("echo", json!({}))
            .await
            .unwrap()
            .get("full_access")
            .is_none());

        std::fs::remove_dir_all(root).ok();
    }

    #[tokio::test]
    async fn registry_binds_task_owned_tools_to_the_originating_thread() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ThreadProbe(None)));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        let registry = agent_base_registry_with_memory(
            &state,
            Some(AgentMemoryContext {
                thread_id: Some("origin-thread".to_string()),
                ..Default::default()
            }),
            &ToolRunPolicy {
                approval: ToolApprovalPolicy::Open,
                ..Default::default()
            },
            &RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
        );

        assert_eq!(
            registry.call("thread_probe", json!({})).await.unwrap()["thread_id"],
            "origin-thread"
        );
    }

    #[test]
    fn worker_run_create_fields_distinguish_omitted_from_null() {
        let omitted: WorkerRunCreateRequest = serde_json::from_value(json!({
            "parent_thread_id":"parent",
            "tasks":[]
        }))
        .unwrap();
        let explicit_null: WorkerRunCreateRequest = serde_json::from_value(json!({
            "parent_thread_id":"parent",
            "workspace":null,
            "privacy_mode":"off",
            "tasks":[]
        }))
        .unwrap();

        assert!(omitted.workspace.as_value().is_none());
        assert_eq!(explicit_null.workspace.as_value(), Some(&Value::Null));
        assert_eq!(
            explicit_null.privacy_mode.as_value(),
            Some(&Value::String("off".to_string()))
        );
    }

    #[test]
    fn legacy_worker_run_origin_is_rejected_for_mutating_reentry() {
        let mut run = milim_agents::WorkerRun {
            id: "legacy".to_string(),
            parent_thread_id: "parent".to_string(),
            parent_turn_id: None,
            policy: milim_agents::DelegationPolicy::Ask,
            runtime: milim_agents::WorkerRuntime::Managed,
            status: milim_agents::WorkerRunStatus::Proposed,
            tasks: Vec::new(),
            context: None,
            workspace: None,
            privacy_mode: None,
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
            finished_at: None,
        };

        let error = RunContext::from_worker_run(&run).unwrap_err().to_string();
        assert!(error.contains("cannot be approved, retried, or applied"));

        run.privacy_mode = Some("off".to_string());
        run.workspace = Some(temp_workspace_root().to_string_lossy().to_string());
        let error = RunContext::from_worker_run(&run).unwrap_err().to_string();
        assert!(error.contains("invalid workspace"));
    }
}

#[cfg(test)]
mod native_run_context_tests {
    use super::*;
    use milim_inference::test_backend::TestBackend;
    use milim_storage::Database;

    struct ReadOnlyProbe(&'static str);

    #[async_trait]
    impl Tool for ReadOnlyProbe {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "read-only probe"
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }
        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }
        async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
            Ok(json!({}))
        }
    }

    fn state_with(names: &[&'static str], workspace: Option<PathBuf>) -> AppState {
        let mut tools = ToolRegistry::new();
        for name in names {
            tools.register(Arc::new(ReadOnlyProbe(name)));
        }
        AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools)
        .with_workspace(Arc::new(std::sync::RwLock::new(workspace)))
    }

    #[test]
    fn plan_and_guarded_modes_keep_glob_and_grep() {
        let names = [
            "read_file",
            "list_dir",
            "glob",
            "grep",
            "diagnostics",
            "shell",
        ];
        let workspace = std::env::temp_dir();
        let state = state_with(&names, Some(workspace.clone()));
        let run_context = RunContext {
            workspace: Some(workspace),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let plan = agent_base_registry_with_memory(
            &state,
            None,
            &ToolRunPolicy {
                approval: ToolApprovalPolicy::Open,
                plan_mode: true,
                ..Default::default()
            },
            &run_context,
        );
        let plan_names: Vec<String> = plan.list().into_iter().map(|tool| tool.name).collect();
        assert_eq!(
            plan_names,
            ["diagnostics", "glob", "grep", "list_dir", "read_file"]
        );

        let guarded =
            agent_base_registry_with_memory(&state, None, &ToolRunPolicy::default(), &run_context);
        assert!(guarded.contains("glob"));
        assert!(guarded.contains("grep"));
        assert!(guarded.contains("diagnostics"));

        let mut edit_tools = ToolRegistry::new();
        edit_tools.register(Arc::new(ReadOnlyProbe("diagnostics")));
        edit_tools.register(Arc::new(ReadOnlyProbe("glob")));
        edit_tools.register(Arc::new(ReadOnlyProbe("grep")));
        edit_tools.register(Arc::new(ReadOnlyProbe("edit_file")));
        let no_folder = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(edit_tools);
        let unscoped = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let plan_without_folder = agent_base_registry_with_memory(
            &no_folder,
            None,
            &ToolRunPolicy {
                plan_mode: true,
                ..Default::default()
            },
            &unscoped,
        );
        assert!(!plan_without_folder.contains("glob"));
        assert!(!plan_without_folder.contains("grep"));
        assert!(!plan_without_folder.contains("diagnostics"));
    }

    #[test]
    fn long_waiting_tools_outlast_their_own_wait() {
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        );
        let delegate = DelegateWorkersTool {
            state,
            supervisor: Arc::new(ThreadSupervisor::new(
                milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
            )),
            context: AgentMemoryContext::default(),
            child_tools: ToolRegistry::new(),
            allow_write_review: false,
            auto_approve_workers: false,
            run_context: RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
        };
        let deadline = delegate.deadline_for_call(&json!({})).unwrap();
        assert!(deadline > WORKER_RUN_TIMEOUT + WORKER_SETUP_ALLOWANCE);
        assert_eq!(deadline, Duration::from_secs(450));

        let store = Arc::new(
            milim_storage::UserDataStore::new(Database::open_in_memory().unwrap()).unwrap(),
        );
        let wait = LinkedThreadWaitTool {
            control: crate::control::RunManager::new(store, "Deadline fixture").unwrap(),
            origin_thread_id: "origin".into(),
            origin_run_id: "run".into(),
            grants: Vec::new(),
        };
        assert_eq!(
            wait.deadline_for_call(&json!({ "exchange_id": "x" })),
            Some(Duration::from_millis(DEFAULT_LINKED_THREAD_WAIT_MS) + TOOL_WAIT_GRACE)
        );
        assert_eq!(
            wait.deadline_for_call(&json!({ "exchange_id": "x", "timeout_ms": 90_000 })),
            Some(
                Duration::from_millis(crate::control::MAX_LINKED_THREAD_WAIT_MS) + TOOL_WAIT_GRACE
            )
        );
    }

    #[test]
    fn parent_context_is_framed_as_background_for_workers() {
        let block = parent_context_block("  Implement slugify.\n");
        assert!(block.starts_with("<parent_context>\n"));
        assert!(block.contains("do not carry out the parent's request yourself"));
        assert!(block.ends_with("Implement slugify.\n</parent_context>"));
    }

    #[test]
    fn native_workers_get_the_base_prompt_for_their_own_tools_and_an_environment() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(ReadOnlyProbe("read_file")));
        let mut spec = ChildRunSpec {
            parent_id: "parent-1".to_string(),
            title: "Worker".to_string(),
            model: "model-x".to_string(),
            agent_id: None,
            system_prompt: None,
            prompt: "Inspect.".to_string(),
            run_id: Some("run-1".to_string()),
            runtime: milim_agents::WorkerRuntime::Managed,
            access: milim_agents::WorkerAccess::ReadOnly,
            worktree_path: None,
            account_profile_id: None,
            base_prompt: None,
            environment: None,
        };
        assert!(add_native_worker_context(&mut spec, &registry, None).is_none());
        let base = spec.base_prompt.as_deref().unwrap();
        assert!(base.starts_with("You are milim's coding agent"));
        assert!(base.contains("read_file"));
        assert!(
            !base.contains("edit_file"),
            "guidance covers only the Worker's tools"
        );
        assert!(!base.contains("# Plan mode"));
        let environment = spec.environment.as_deref().unwrap();
        assert!(environment.starts_with("<environment>"));
        assert!(environment.contains("Model: model-x"));
        assert!(environment.contains("Today's date: "));

        let mut bare = spec.clone();
        bare.base_prompt = None;
        bare.environment = None;
        add_native_worker_context(&mut bare, &ToolRegistry::new(), None);
        assert!(bare.base_prompt.is_none() && bare.environment.is_none());
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
    }

    fn git_ok(dir: &FsPath, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn control_spec<'a>(
        run_context: &'a RunContext,
        instructions: crate::workspace_context::InstructionLayers,
        messages: Vec<ChatMessage>,
    ) -> NativeRunSpec<'a> {
        NativeRunSpec {
            kind: NativeRunKind::Control,
            model: "model-x",
            run_id: "run-1",
            thread_id: Some("thread-1"),
            run_context,
            policy: ToolRunPolicy {
                approval: ToolApprovalPolicy::Open,
                ..Default::default()
            },
            streamed: true,
            tool_mode: "all",
            enabled_tools: &[],
            skill_mode: "auto",
            enabled_skills: &[],
            skills_resolved: false,
            instructions,
            memory: AgentMemoryContext::default(),
            messages,
            turn_context: Vec::new(),
        }
    }

    fn leading_system(messages: &[ChatMessage]) -> Vec<String> {
        messages
            .iter()
            .take_while(|message| message.role == "system")
            .map(ChatMessage::text_content)
            .collect()
    }

    #[test]
    fn native_run_prefix_stays_byte_identical_when_only_the_turn_changes() {
        let workspace = temp_dir("milim-native-prefix");
        std::fs::write(workspace.join("AGENTS.md"), "Run the tests.").unwrap();
        let git = git_ok(&workspace, &["init", "-q", "-b", "main"])
            && git_ok(&workspace, &["config", "user.email", "test@example.com"])
            && git_ok(&workspace, &["config", "user.name", "Test"])
            && git_ok(&workspace, &["add", "AGENTS.md"])
            && git_ok(&workspace, &["commit", "-q", "-m", "first commit"]);
        let state = state_with(&["read_file"], None);
        let run_context = RunContext {
            workspace: Some(workspace.clone()),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let instructions = crate::workspace_context::InstructionLayers {
            milim: "Be brief.".into(),
            ..Default::default()
        };

        let first = build_native_run(
            &state,
            control_spec(
                &run_context,
                instructions.clone(),
                vec![ChatMessage::text("user", "first request")],
            ),
            Default::default(),
        );
        std::fs::write(workspace.join("new.txt"), "changed").unwrap();
        let second = build_native_run(
            &state,
            NativeRunSpec {
                turn_context: vec![ChatMessage::text(
                    "system",
                    "Linked chat mail: reply ready.",
                )],
                ..control_spec(
                    &run_context,
                    instructions,
                    vec![
                        ChatMessage::text("user", "first request"),
                        ChatMessage::text("assistant", "done"),
                        ChatMessage::text("user", "second request"),
                    ],
                )
            },
            Default::default(),
        );

        // The first turn's context directly follows the prefix; from the
        // second turn on, history separates them.
        let prefix = leading_system(&second.messages);
        assert_eq!(
            leading_system(&first.messages)[..prefix.len()],
            prefix[..],
            "git changes and per-turn context must not touch the cached prefix"
        );
        assert!(prefix[0].starts_with("You are milim's coding agent"));
        assert!(prefix[1].starts_with("# Instructions"));
        assert!(prefix[1].contains("Be brief.") && prefix[1].contains("Run the tests."));
        assert!(prefix.last().unwrap().starts_with("<environment>"));
        assert!(prefix.iter().all(|text| !text.contains("Today's date:")));

        let roles = |run: &NativeRun| {
            run.messages[prefix.len()..]
                .iter()
                .map(|message| message.role.as_str().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(roles(&first), ["system", "user"]);
        assert_eq!(roles(&second), ["user", "assistant", "system", "user"]);
        let turn_context = |run: &NativeRun| run.messages[run.messages.len() - 2].text_content();
        let first_turn = turn_context(&first);
        let second_turn = turn_context(&second);
        assert_eq!(first.turn_context.as_deref(), Some(first_turn.as_str()));
        assert_eq!(second.turn_context.as_deref(), Some(second_turn.as_str()));
        assert!(first_turn.starts_with("Context for this turn"));
        assert!(first_turn.contains("Today's date: "));
        assert!(second_turn.contains("Linked chat mail: reply ready."));
        if git {
            assert!(first_turn.contains("Git status: clean"), "{first_turn}");
            assert!(
                second_turn.contains("Git status (1 changed paths):"),
                "{second_turn}"
            );
            assert!(second_turn.contains("first commit"));
        }
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn every_native_run_kind_gets_base_prompt_environment_instructions_and_hooks() {
        let workspace = temp_dir("milim-native-parity");
        std::fs::write(workspace.join("AGENTS.md"), "REPO_RULE").unwrap();
        std::fs::create_dir_all(workspace.join(".milim")).unwrap();
        // Untrusted project hooks still attach, to report that they were skipped.
        std::fs::write(
            workspace.join(".milim").join("settings.json"),
            r#"{"hooks": {"Stop": [{"command": "true"}]}}"#,
        )
        .unwrap();
        let state = state_with(&["read_file"], None);
        let run_context = RunContext {
            workspace: Some(workspace.clone()),
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let layers = crate::workspace_context::InstructionLayers {
            milim: "CUSTOM_RULE".into(),
            ..Default::default()
        };
        let check = |kind: &str, messages: &[ChatMessage], hooks: bool| {
            let texts: Vec<String> = messages.iter().map(ChatMessage::text_content).collect();
            assert!(
                texts[0].starts_with("You are milim's coding agent"),
                "{kind}: {texts:?}"
            );
            for needle in [
                "REPO_RULE",
                "CUSTOM_RULE",
                "<environment>",
                "Today's date: ",
            ] {
                assert!(
                    texts.iter().any(|text| text.contains(needle)),
                    "{kind} is missing {needle}: {texts:?}"
                );
            }
            assert!(hooks, "{kind} runs without the user's hooks");
        };

        let control = build_native_run(
            &state,
            control_spec(
                &run_context,
                layers.clone(),
                vec![ChatMessage::text("user", "work")],
            ),
            Default::default(),
        );
        check(
            "control",
            &control.messages,
            control.config.interceptor.is_some(),
        );

        for (tool_mode, instructions) in [
            ("all", Default::default()),
            (
                "custom",
                crate::workspace_context::InstructionLayers {
                    agent: "AGENT_RULE".into(),
                    ..Default::default()
                },
            ),
        ] {
            let enabled = ["read_file".to_string()];
            let http = build_native_run(
                &state,
                NativeRunSpec {
                    kind: NativeRunKind::Http,
                    tool_mode,
                    enabled_tools: &enabled,
                    instructions,
                    streamed: false,
                    messages: vec![
                        ChatMessage::text("system", "CUSTOM_RULE"),
                        ChatMessage::text("user", "work"),
                    ],
                    ..control_spec(&run_context, Default::default(), Vec::new())
                },
                Default::default(),
            );
            check(tool_mode, &http.messages, http.config.interceptor.is_some());
        }

        let mut worker_run = milim_agents::WorkerRun {
            id: "worker-run".to_string(),
            parent_thread_id: "thread-1".to_string(),
            parent_turn_id: None,
            policy: milim_agents::DelegationPolicy::Auto,
            runtime: milim_agents::WorkerRuntime::Managed,
            status: milim_agents::WorkerRunStatus::Running,
            tasks: Vec::new(),
            context: None,
            workspace: None,
            privacy_mode: Some("off".to_string()),
            error: None,
            created_at: String::new(),
            updated_at: String::new(),
            finished_at: None,
        };
        worker_run.tasks.push(milim_agents::WorkerPlanTask {
            id: "task".to_string(),
            title: "Worker".to_string(),
            prompt: "Inspect.".to_string(),
            role: Some("reviewer".to_string()),
            agent_id: None,
            agent_snapshot: None,
            model: "model-x".to_string(),
            access: milim_agents::WorkerAccess::ReadOnly,
        });
        worker_run.context = managed_worker_context(
            Some(&workspace),
            worker_context("work", &layers, &[], None).as_deref(),
        );
        let mut spec = worker_specs(&worker_run, vec![None]).pop().unwrap();
        let tools = state_with(&["read_file"], None)
            .tools
            .as_deref()
            .cloned()
            .unwrap();
        let hooks = add_native_worker_context(&mut spec, &tools, Some(&workspace));
        let messages = crate::threads::worker_messages(&spec);
        check("worker", &messages, hooks.is_some());
        assert!(messages.iter().any(|message| message
            .text_content()
            .contains("Your role for this task: reviewer")));
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn default_tools_hide_management_groups_until_the_conversation_asks() {
        let root = temp_dir("milim-default-tools");
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(ReadOnlyProbe("current_time")));
        tools.register(Arc::new(ReadOnlyProbe("read_file")));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools)
        .with_schedules(
            milim_automation::ScheduleStore::new(Database::open_in_memory().unwrap()).unwrap(),
        )
        .with_agents(milim_agents::AgentStore::new(Database::open_in_memory().unwrap()).unwrap())
        .with_mcp(Arc::new(milim_mcp_client::McpHub::open(&root)));
        let run_context = RunContext {
            workspace: None,
            privacy_mode: crate::privacy::PrivacyMode::Off,
        };
        let open = ToolRunPolicy {
            approval: ToolApprovalPolicy::Open,
            ..Default::default()
        };
        let names = |policy: ToolRunPolicy, mode: &str, enabled: &[String]| {
            agent_registry_for_mode_with_context(&state, mode, enabled, None, &policy, &run_context)
                .list()
                .into_iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>()
        };

        let plain = names(
            open.with_management_tools_for(["fix the parser"]),
            "all",
            &[],
        );
        assert_eq!(plain, ["read_file"]);

        let scheduled = names(
            open.with_management_tools_for(["fix the parser", "Run this every 5 minutes"]),
            "all",
            &[],
        );
        for name in SCHEDULE_TOOL_NAMES.iter().chain(&["list_agents"]) {
            assert!(scheduled.iter().any(|tool| tool == name), "{scheduled:?}");
        }
        assert!(!scheduled.iter().any(|tool| tool.starts_with("mcp_server_")));

        let mcp = names(
            open.with_management_tools_for(["Add the GitHub MCP server"]),
            "all",
            &[],
        );
        for name in MCP_SERVER_TOOL_NAMES {
            assert!(mcp.iter().any(|tool| tool == name), "{mcp:?}");
        }
        assert!(!mcp.iter().any(|tool| tool.starts_with("schedule_")));

        let custom = names(
            open,
            "custom",
            &["current_time".to_string(), "schedule_list".to_string()],
        );
        assert_eq!(custom, ["current_time", "schedule_list"]);

        assert!(requests_schedule_tools("set up a cron job"));
        assert!(requests_schedule_tools("what automations do I have?"));
        assert!(!requests_schedule_tools(
            "add automated tests for the parser"
        ));
        assert!(requests_mcp_server_tools(
            "connect a Model Context Protocol server"
        ));
        assert!(!requests_mcp_server_tools("the mcpx binary"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn workers_get_their_own_run_state() {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(milim_tools::TodoWriteTool::default()));
        let state = AppState::new(
            Arc::new(TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        )
        .with_tools(tools);
        let parent = agent_base_registry_with_memory(
            &state,
            Some(AgentMemoryContext {
                thread_id: Some(format!("parent-{}", uuid::Uuid::new_v4())),
                ..Default::default()
            }),
            &ToolRunPolicy {
                approval: ToolApprovalPolicy::Open,
                ..Default::default()
            },
            &RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
        );
        let todos = |count: usize| {
            json!({ "todos": (0..count)
                .map(|index| json!({ "content": format!("step {index}"), "status": "pending" }))
                .collect::<Vec<_>>() })
        };
        let previous = |result: Value| result["previous_count"].as_u64().unwrap();

        parent.call("todo_write", todos(2)).await.unwrap();
        let first = worker_tools(&parent, None);
        let second = worker_tools(&parent, None);
        assert_eq!(
            previous(first.call("todo_write", todos(1)).await.unwrap()),
            0
        );
        assert_eq!(
            previous(second.call("todo_write", todos(3)).await.unwrap()),
            0
        );
        assert_eq!(
            previous(parent.call("todo_write", todos(2)).await.unwrap()),
            2,
            "Workers leave the parent's checklist alone"
        );
    }
}

fn string_extra(req: &ChatCompletionRequest, key: &str) -> Option<String> {
    req.extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn string_list_extra(req: &ChatCompletionRequest, key: &str) -> Vec<String> {
    req.extra
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn bool_extra(req: &ChatCompletionRequest, key: &str) -> bool {
    req.extra.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn tool_run_policy_from_request(req: &ChatCompletionRequest) -> ToolRunPolicy {
    let approval =
        ToolApprovalPolicy::from_requested(string_extra(req, "tool_approval_policy").as_deref());
    ToolRunPolicy {
        approval,
        approval_granted: bool_extra(req, "tool_approval_grant"),
        interactive_approval: bool_extra(req, "interactive_tool_approval"),
        sandbox_enabled: bool_extra(req, "sandbox_enabled"),
        computer_use_enabled: bool_extra(req, "computer_use_enabled"),
        preview_tools_enabled: bool_extra(req, "preview_tools_enabled"),
        experimental_hashline_patch: bool_extra(req, "experimental_hashline_patch"),
        plan_mode: bool_extra(req, "plan_mode"),
        ..Default::default()
    }
}

fn memory_context_from_request(req: &ChatCompletionRequest, model: String) -> AgentMemoryContext {
    AgentMemoryContext {
        enabled: bool_extra(req, "memory_enabled"),
        model,
        thread_id: string_extra(req, "thread_id"),
        project_locator: string_extra(req, "project_locator"),
        project_label: string_extra(req, "project_label"),
        message_id: string_extra(req, "message_id"),
        delegation_policy: match string_extra(req, "delegation_policy").as_deref() {
            Some("off") => milim_agents::DelegationPolicy::Off,
            Some("auto") => milim_agents::DelegationPolicy::Auto,
            _ => milim_agents::DelegationPolicy::Ask,
        },
        worker_model: string_extra(req, "worker_model"),
        worker_context: None,
        linked_thread_grants: Vec::new(),
    }
}

pub(crate) fn workspace_snapshot(st: &AppState) -> Option<PathBuf> {
    st.workspace.read().ok().and_then(|guard| guard.clone())
}

pub(crate) fn static_registry_for_run(st: &AppState) -> ToolRegistry {
    static_registry_for_context(st, &RunContext::current(st))
}

fn static_registry_for_context(st: &AppState, context: &RunContext) -> ToolRegistry {
    static_registry_for_context_with_access(st, context, false)
}

fn static_registry_for_context_with_access(
    st: &AppState,
    context: &RunContext,
    full_access: bool,
) -> ToolRegistry {
    let reg = st.tools.as_deref().cloned().unwrap_or_default();
    let mut reg = context
        .workspace
        .as_deref()
        .map(|root| {
            if full_access {
                reg.with_full_access(root)
            } else {
                reg.scoped_to_workspace(root)
            }
        })
        .unwrap_or(reg);
    if context.workspace.is_none() {
        reg = reg.without(RUN_WORKSPACE_TOOL_NAMES);
    }
    let mut reg = reg.scoped_for_run();
    if reg.contains("web_search") {
        reg = reg.without(&["web_search"]);
        reg.register(Arc::new(web_search_for_context(st, context)));
    }
    reg
}

struct ProviderWebSearchSource(Arc<crate::providers::ProviderRegistry>);

#[async_trait]
impl milim_tools::WebSearchApiSource for ProviderWebSearchSource {
    async fn api(&self) -> Option<milim_tools::WebSearchApi> {
        self.0.web_search_api().await
    }
}

/// `web_search` bound to the configured search provider and the run's
/// privacy mode, which applies to the query before it leaves the machine.
fn web_search_for_context(st: &AppState, context: &RunContext) -> milim_tools::WebSearchTool {
    let mut tool = milim_tools::WebSearchTool::default();
    if let Some(providers) = st.providers.clone() {
        tool = tool.with_api_source(Arc::new(ProviderWebSearchSource(providers)));
    }
    let mode = context.privacy_mode;
    if mode != crate::privacy::PrivacyMode::Off {
        tool = tool.with_query_filter(Arc::new(move |query: &str| {
            crate::privacy::gate_outbound_tool_text(mode, query, "web search query")
        }));
    }
    tool
}

fn registry_has_desktop_host_tools(reg: &ToolRegistry) -> bool {
    reg.contains("edit_file") || reg.contains("patch_file") || reg.contains("shell")
}

fn desktop_workspace_unavailable_for(st: &AppState, workspace: Option<&FsPath>) -> bool {
    workspace.is_none()
        && st
            .tools
            .as_ref()
            .map(|reg| registry_has_desktop_host_tools(reg))
            .unwrap_or(false)
}

pub(crate) fn add_workspace_instructions(messages: &mut Vec<ChatMessage>, st: &AppState) {
    add_workspace_instructions_for(messages, workspace_snapshot(st).as_deref());
}

pub(crate) fn add_workspace_instructions_for(
    messages: &mut Vec<ChatMessage>,
    workspace: Option<&FsPath>,
) {
    let context = crate::workspace_context::resolve(workspace);
    let Some(instructions) = crate::workspace_context::formatted(&context, None) else {
        return;
    };
    messages.insert(0, ChatMessage::text("system", instructions));
}

/// The effective tool registry for an agent run: the static tools (builtins,
/// host fs/shell, Docker sandbox) plus any tools exposed by connected MCP
/// servers. Rebuilt per-run (cheap clone) so newly-added MCP servers are
/// picked up without restarting the app.
fn agent_base_registry_with_memory(
    st: &AppState,
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
    run_context: &RunContext,
) -> ToolRegistry {
    let mut reg = static_registry_for_context_with_access(
        st,
        run_context,
        policy.approval == ToolApprovalPolicy::Open && !policy.plan_mode,
    );
    if let Some(thread_id) = memory
        .as_ref()
        .and_then(|context| context.thread_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        reg = reg.scoped_to_thread(thread_id);
    }
    if let Some(store) = st.agents.as_ref() {
        reg.register(Arc::new(ListAgentsTool {
            store: store.clone(),
        }));
    }
    if let (Some(memory), Some(control)) = (memory.clone(), st.control.as_ref()) {
        if memory.thread_id.is_some() && !memory.linked_thread_grants.is_empty() {
            register_linked_thread_tools(&mut reg, st.clone(), control.clone(), memory, policy);
        }
    }
    let workspace_unavailable =
        desktop_workspace_unavailable_for(st, run_context.workspace.as_deref());
    if policy.plan_mode {
        return plan_mode_registry(
            reg,
            workspace_unavailable,
            policy.experimental_hashline_patch,
        );
    }
    if let Some(hub) = &st.mcp {
        register_mcp_server_tools(&mut reg, hub.clone());
        for tool in hub.tools() {
            if let Err(error) = reg.try_register(tool) {
                tracing::warn!("skipping colliding MCP tool: {error}");
            }
        }
    }
    if let Some(store) = st.schedules.as_ref() {
        register_schedule_tools(
            &mut reg,
            store.clone(),
            run_context.workspace.clone(),
            run_context.privacy_mode.as_str(),
        );
    }
    if let (Some(memory), Some(store)) = (memory.clone(), st.memory.as_ref()) {
        if memory.enabled {
            reg.register(Arc::new(MemoryRegisterTool {
                store: Arc::new(store.with_embedder(service_for_run(st, run_context))),
                context: memory,
            }));
        }
    }
    if workspace_unavailable && registry_has_desktop_host_tools(&reg) {
        reg = reg.without(DESKTOP_WORKSPACE_TOOL_NAMES);
    }
    if !policy.sandbox_enabled {
        reg = reg.without(SANDBOX_TOOL_NAMES);
    }
    if !policy.computer_use_enabled {
        reg = reg.without(COMPUTER_TOOL_NAMES);
    }
    if !policy.preview_tools_enabled {
        reg = reg.without(ACTIVE_PREVIEW_TOOL_NAMES);
    }
    if !policy.experimental_hashline_patch {
        reg = reg.without(HASHLINE_TOOL_NAMES);
    }
    if policy.approval == ToolApprovalPolicy::Review
        && !policy.approval_granted
        && !policy.interactive_approval
    {
        reg = ToolRegistry::new();
    } else if policy.approval == ToolApprovalPolicy::Guarded {
        reg = reg.read_only();
    }
    reg
}

fn tools_available(policy: &ToolRunPolicy) -> bool {
    crate::account_runtime_common::tools_allowed(
        policy.plan_mode,
        policy.approval == ToolApprovalPolicy::Review,
        policy.approval_granted,
        policy.interactive_approval,
    )
}

#[derive(Deserialize)]
pub(crate) struct ToolApprovalDecision {
    decision: String,
    #[serde(default)]
    response: Option<Value>,
    /// `once` (default) or `thread` for "Allow for this chat".
    #[serde(default)]
    scope: Option<String>,
    /// `exact` (default) or `prefix` for a `thread` command allowance.
    #[serde(default)]
    allowance_match: Option<String>,
}

pub(crate) async fn tool_approval_status(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    match st.tool_approvals.snapshot(&id) {
        Some(snapshot) => Ok(Json(snapshot).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "tool approval not found or expired" })),
        )
            .into_response()),
    }
}

pub(crate) async fn tool_approval_resolve(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ToolApprovalDecision>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let approved = match req.decision.trim() {
        "approve" => true,
        "deny" => false,
        _ => {
            return Err(ApiError(Error::InvalidRequest(
                "decision must be approve or deny".to_string(),
            )))
        }
    };
    if let Some(control) = st
        .control
        .as_ref()
        .filter(|control| control.owns_approval(&id))
    {
        let result = control
            .command(
                st.clone(),
                None,
                crate::control::ControlCommandV1 {
                    command_id: format!("desktop-approval-{}", uuid::Uuid::new_v4()),
                    kind: crate::control::ControlCommandKindV1::ApprovalResolve,
                    thread_id: None,
                    expected_revision: None,
                    payload: json!({
                        "approval_id": id,
                        "decision": req.decision,
                        "response": req.response,
                        "scope": req.scope,
                        "allowance_match": req.allowance_match,
                    }),
                    confirmation_token: None,
                },
            )
            .await
            .map_err(ApiError)?;
        return match result.status {
            crate::control::ControlCommandStatusV1::Applied => {
                Ok(Json(result.data).into_response())
            }
            crate::control::ControlCommandStatusV1::Conflict => Ok((
                StatusCode::CONFLICT,
                Json(json!({ "error": result.message.unwrap_or_else(|| "approval conflict".into()) })),
            )
                .into_response()),
            _ => Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": result.message.unwrap_or_else(|| "tool approval failed".into()) })),
            )
                .into_response()),
        };
    }
    let scope = if req.scope.as_deref() == Some("thread") {
        milim_agents::ApprovalScope::Thread
    } else {
        milim_agents::ApprovalScope::Once
    };
    match st
        .tool_approvals
        .resolve_with_scope(&id, approved, req.response, scope)
    {
        milim_agents::ApprovalResolve::Resolved
        | milim_agents::ApprovalResolve::AlreadyResolved => {
            let Some(snapshot) = st
                .tool_approvals
                .wait_for_delivery(&id, milim_agents::APPROVAL_DELIVERY_TIMEOUT)
                .await
            else {
                return Ok((
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": "tool approval not found or expired" })),
                )
                    .into_response());
            };
            if matches!(
                snapshot.state,
                milim_agents::ApprovalState::Delivered | milim_agents::ApprovalState::Acknowledged
            ) {
                Ok(Json(snapshot).into_response())
            } else {
                Ok((
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": snapshot.error.as_deref().unwrap_or("tool approval delivery failed"),
                        "approval": snapshot,
                    })),
                )
                    .into_response())
            }
        }
        milim_agents::ApprovalResolve::Conflict => Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "tool approval was resolved with a different decision" })),
        )
            .into_response()),
        milim_agents::ApprovalResolve::Failed => Ok((
            StatusCode::GONE,
            Json(json!({
                "error": st
                    .tool_approvals
                    .snapshot(&id)
                    .and_then(|snapshot| snapshot.error)
                    .unwrap_or_else(|| "tool approval is no longer deliverable".to_string())
            })),
        )
            .into_response()),
        milim_agents::ApprovalResolve::Missing => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "tool approval not found or expired" })),
        )
            .into_response()),
    }
}

fn plan_mode_registry(
    reg: ToolRegistry,
    workspace_unavailable: bool,
    anchored_reads_enabled: bool,
) -> ToolRegistry {
    let mut allowed: Vec<String> = PLAN_MODE_READ_ONLY_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    if anchored_reads_enabled {
        allowed.push("read_file_anchors".to_string());
    }
    let mut reg = reg.filtered(&allowed);
    if workspace_unavailable {
        reg = reg
            .without(WORKSPACE_READ_TOOL_NAMES)
            .without(HASHLINE_TOOL_NAMES);
    }
    reg
}

fn agent_registry_for_mode_with_context(
    st: &AppState,
    tool_mode: &str,
    enabled_tools: &[String],
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
    run_context: &RunContext,
) -> ToolRegistry {
    let all = agent_base_registry_with_memory(st, memory.clone(), policy, run_context);
    let delegation = memory
        .zip(st.threads.as_ref())
        .filter(|(memory, supervisor)| {
            memory.delegation_policy != milim_agents::DelegationPolicy::Off
                && tools_available(policy)
                && child_thread_tools_allowed(supervisor, memory)
        });
    let normalized = milim_agents::normalize_tool_mode(tool_mode, enabled_tools);
    let inherited = match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => all.filtered(enabled_tools),
        _ => all.without(&policy.hidden_default_tools(delegation.is_some())),
    };
    let mut reg = inherited.clone();
    if let Some((memory, supervisor)) = delegation {
        register_child_thread_tools_with_context(
            &mut reg,
            st.clone(),
            supervisor.clone(),
            memory,
            child_registry_for_policy(st, policy, inherited, run_context),
            policy.approval == ToolApprovalPolicy::Open
                || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted),
            policy.approval == ToolApprovalPolicy::Open,
            run_context.clone(),
        );
    }
    match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => reg.filtered(enabled_tools),
        _ => reg,
    }
}

fn child_thread_tools_allowed(supervisor: &ThreadSupervisor, context: &AgentMemoryContext) -> bool {
    let Some(thread_id) = context.thread_id.as_deref() else {
        return false;
    };
    supervisor
        .get(thread_id)
        .map(|t| t.is_none())
        .unwrap_or(false)
}

pub(crate) fn register_child_thread_tools(
    reg: &mut ToolRegistry,
    state: AppState,
    supervisor: Arc<ThreadSupervisor>,
    context: AgentMemoryContext,
    child_tools: ToolRegistry,
    allow_write_review: bool,
) {
    let run_context = RunContext::current(&state);
    register_child_thread_tools_with_context(
        reg,
        state,
        supervisor,
        context,
        child_tools,
        allow_write_review,
        false,
        run_context,
    );
}

#[allow(clippy::too_many_arguments)]
fn register_child_thread_tools_with_context(
    reg: &mut ToolRegistry,
    state: AppState,
    supervisor: Arc<ThreadSupervisor>,
    context: AgentMemoryContext,
    child_tools: ToolRegistry,
    allow_write_review: bool,
    auto_approve_workers: bool,
    run_context: RunContext,
) {
    if context.delegation_policy != milim_agents::DelegationPolicy::Off {
        reg.register(Arc::new(DelegateWorkersTool {
            state,
            supervisor,
            context,
            child_tools,
            allow_write_review,
            auto_approve_workers,
            run_context,
        }));
    }
}

fn child_read_only_registry(st: &AppState, run_context: &RunContext) -> ToolRegistry {
    let allowed: Vec<String> = CHILD_THREAD_READ_ONLY_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut reg = static_registry_for_context(st, run_context).filtered(&allowed);
    if desktop_workspace_unavailable_for(st, run_context.workspace.as_deref()) {
        reg = reg.without(WORKSPACE_READ_TOOL_NAMES);
    }
    reg
}

pub(crate) fn worker_review_registry(st: &AppState, run_context: &RunContext) -> ToolRegistry {
    let mut reg = static_registry_for_context(st, run_context)
        .without(CHILD_THREAD_TOOL_NAMES)
        .without(SANDBOX_TOOL_NAMES)
        .without(COMPUTER_TOOL_NAMES)
        .without(ACTIVE_PREVIEW_TOOL_NAMES)
        .without(PREVIEW_OPEN_TOOL_NAMES);
    if desktop_workspace_unavailable_for(st, run_context.workspace.as_deref()) {
        reg = reg.without(DESKTOP_WORKSPACE_TOOL_NAMES);
    }
    reg
}

fn child_registry_for_policy(
    st: &AppState,
    policy: &ToolRunPolicy,
    inherited: ToolRegistry,
    run_context: &RunContext,
) -> ToolRegistry {
    if policy.approval == ToolApprovalPolicy::Open
        || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted)
    {
        inherited
            .without(CHILD_THREAD_TOOL_NAMES)
            .without(PREVIEW_OPEN_TOOL_NAMES)
    } else {
        child_read_only_registry(st, run_context)
    }
}

/// `POST /agents/run` — run the tool-use loop server-side and return the final
/// message plus the tool steps taken.
pub(crate) async fn agents_run(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let skill_mode = string_extra(&req, "skill_mode").unwrap_or_else(|| "auto".to_string());
    let enabled_skills = string_list_extra(&req, "enabled_skills");
    http_native_run(
        &st,
        req,
        "all",
        &[],
        &skill_mode,
        &enabled_skills,
        Default::default(),
    )
    .await
}

/// Run the tool-use loop for one HTTP agent request, as SSE when the caller
/// asked to stream and as a single JSON response otherwise.
async fn http_native_run(
    st: &AppState,
    req: ChatCompletionRequest,
    tool_mode: &str,
    enabled_tools: &[String],
    skill_mode: &str,
    enabled_skills: &[String],
    instructions: crate::workspace_context::InstructionLayers,
) -> Result<Response, ApiError> {
    let run_context = RunContext::from_request(st, &req).map_err(ApiError)?;
    let service = service_for_run(st, &run_context);
    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let reasoning_effort = req.reasoning_effort;
    let run_id = gen_id("agentrun");
    let thread_id = string_extra(&req, "thread_id");
    let config = agent_run_config_from_request(&req);
    let spec = NativeRunSpec {
        kind: NativeRunKind::Http,
        model: &model,
        run_id: &run_id,
        thread_id: thread_id.as_deref(),
        run_context: &run_context,
        policy: tool_run_policy_from_request(&req),
        streamed: want_stream,
        tool_mode,
        enabled_tools,
        skill_mode,
        enabled_skills,
        skills_resolved: bool_extra(&req, "skills_resolved"),
        instructions,
        memory: memory_context_from_request(&req, model.clone()),
        messages: req.messages,
        turn_context: Vec::new(),
    };
    let run = build_native_run(st, spec, config);

    if want_stream {
        let stream = milim_agents::run_agent_stream_with_config(
            service,
            Arc::new(run.tools),
            model,
            run.messages,
            reasoning_effort,
            run.config,
        );
        return Ok(Sse::new(agent_sse(stream))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let outcome = milim_agents::run_agent_with_config(
        service.as_ref(),
        &run.tools,
        &model,
        run.messages,
        reasoning_effort,
        run.config,
    )
    .await
    .map_err(ApiError)?;

    Ok(Json(AgentRunResponse {
        id: run_id,
        object: "agent.run",
        model,
        message: outcome.message,
        steps: outcome.steps,
        iterations: outcome.iterations,
        stopped_at_limit: outcome.stopped_at_limit,
    })
    .into_response())
}

pub(crate) type ControlAgentStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = milim_agents::AgentEvent> + Send>>;

/// A canonical thread turn through milim's tool loop. `instructions` are the
/// run's frozen instruction layers; `agent` contributes its tool and skill
/// selection. `messages` is the replayed thread history; `turn_context` is
/// this turn's own context (linked-thread mail, preview runtime), which
/// travels with the turn rather than in the cached prompt prefix. Returns the
/// stream and the per-turn context message as sent, for byte-stable replay.
#[allow(clippy::too_many_arguments)]
pub(crate) fn control_agent_stream(
    st: &AppState,
    agent: &milim_agents::AgentDef,
    model: &str,
    messages: Vec<ChatMessage>,
    turn_context: Vec<ChatMessage>,
    workspace: Option<&str>,
    privacy: &str,
    approval_mode: &str,
    plan_mode: bool,
    sandbox: bool,
    computer_use: bool,
    memory_enabled: bool,
    delegation_policy: &str,
    worker_model: &str,
    thread_id: &str,
    message_id: &str,
    linked_thread_grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
    instructions: crate::workspace_context::InstructionLayers,
    reasoning_effort: Option<ReasoningEffort>,
    sampling: SamplingParams,
    run_limits: Option<&crate::control::RunLimitsV1>,
    pricing: Option<milim_core::api::openai::ModelPricing>,
    context_window_tokens: Option<u32>,
    step_hook: Arc<dyn milim_agents::AgentStepHook>,
) -> milim_core::Result<(ControlAgentStream, Option<String>)> {
    let run_context = RunContext::from_control(st, workspace, privacy)?;
    let service = service_for_run(st, &run_context);
    let approval = ToolApprovalPolicy::from_requested(Some(approval_mode));
    let policy = ToolRunPolicy {
        approval,
        interactive_approval: approval == ToolApprovalPolicy::Review,
        sandbox_enabled: sandbox,
        computer_use_enabled: computer_use,
        plan_mode,
        ..Default::default()
    };
    let mut config = milim_agents::AgentRunConfig {
        step_hook: Some(step_hook),
        sampling,
        context_window_tokens,
        ..Default::default()
    };
    if let Some(limits) = run_limits {
        if let Some(steps) = limits.max_steps {
            config.max_iterations = steps as usize;
        }
        config.limits = milim_agents::AgentRunLimits {
            max_duration: limits
                .max_seconds
                .map(|seconds| std::time::Duration::from_secs(u64::from(seconds))),
            max_cost_usd: limits.max_cost_usd,
            pricing,
        };
    }
    let memory = AgentMemoryContext {
        enabled: memory_enabled,
        model: model.to_string(),
        thread_id: Some(thread_id.to_string()),
        project_locator: workspace.map(str::to_string),
        project_label: workspace.and_then(|value| {
            FsPath::new(value)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        }),
        message_id: Some(message_id.to_string()),
        delegation_policy: match delegation_policy {
            "off" => milim_agents::DelegationPolicy::Off,
            "auto" => milim_agents::DelegationPolicy::Auto,
            _ => milim_agents::DelegationPolicy::Ask,
        },
        worker_model: (!worker_model.trim().is_empty()).then(|| worker_model.to_string()),
        worker_context: None,
        linked_thread_grants,
    };
    let spec = NativeRunSpec {
        kind: NativeRunKind::Control,
        model,
        run_id: message_id,
        thread_id: Some(thread_id),
        run_context: &run_context,
        policy,
        streamed: true,
        tool_mode: &agent.tool_mode,
        enabled_tools: &agent.enabled_tools,
        skill_mode: &agent.skill_mode,
        enabled_skills: &agent.enabled_skills,
        skills_resolved: false,
        instructions,
        memory,
        messages,
        turn_context,
    };
    let run = build_native_run(st, spec, config);
    let stream: ControlAgentStream = Box::pin(milim_agents::run_agent_stream_with_config(
        service,
        Arc::new(run.tools),
        model.to_string(),
        run.messages,
        reasoning_effort,
        run.config,
    ));
    Ok((stream, run.turn_context))
}

/// `GET /agents` — list named agents.
pub(crate) async fn agents_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let agents = match &st.agents {
        Some(store) => store.list().map_err(ApiError)?,
        None => Vec::new(),
    };
    Ok(Json(json!({ "agents": agents })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct CreateAgentRequest {
    name: String,
    #[serde(default)]
    description: String,
    model: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    tool_mode: String,
    #[serde(default)]
    enabled_tools: Vec<String>,
    #[serde(default)]
    skill_mode: String,
    #[serde(default)]
    enabled_skills: Vec<String>,
    #[serde(default)]
    avatar: String,
}

/// `POST /agents` — create a named agent.
pub(crate) async fn agent_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .create(
            &req.name,
            &req.description,
            &req.model,
            &req.system_prompt,
            &req.tool_mode,
            req.enabled_tools,
            &req.skill_mode,
            req.enabled_skills,
            &req.avatar,
        )
        .map_err(ApiError)?;
    Ok(Json(agent).into_response())
}

/// `GET /agents/{id}` — fetch one agent.
pub(crate) async fn agent_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("agent {id}"))))?;
    Ok(Json(agent).into_response())
}

/// `POST /agents/{id}/run` — run a named agent's tool-use loop.
pub(crate) async fn agent_run_by_id(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<ChatCompletionRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("agent {id}"))))?;
    http_native_run(
        &st,
        req,
        &agent.tool_mode,
        &agent.enabled_tools,
        &agent.skill_mode,
        &agent.enabled_skills,
        crate::workspace_context::InstructionLayers {
            agent: agent.system_prompt.clone(),
            ..Default::default()
        },
    )
    .await
}

/// `PUT /agents/{id}` — update (upsert) a named agent.
pub(crate) async fn agent_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateAgentRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let agent = milim_agents::AgentDef {
        id,
        name: req.name,
        description: req.description,
        system_prompt: req.system_prompt,
        model: req.model,
        tool_mode: milim_agents::normalize_tool_mode(&req.tool_mode, &req.enabled_tools),
        enabled_tools: req.enabled_tools,
        skill_mode: milim_agents::normalize_skill_mode(&req.skill_mode, &req.enabled_skills),
        enabled_skills: req.enabled_skills,
        avatar: req.avatar,
    };
    store.upsert(&agent).map_err(ApiError)?;
    Ok(Json(agent).into_response())
}

/// `DELETE /agents/{id}` — remove a named agent.
pub(crate) async fn agent_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = agents_store(&st)?;
    let removed = store.delete(&id).map_err(ApiError)?;
    Ok(Json(json!({ "deleted": removed })).into_response())
}

fn agents_store(st: &AppState) -> Result<&milim_agents::AgentStore, ApiError> {
    st.agents
        .as_deref()
        .ok_or_else(|| ApiError(Error::InvalidRequest("agents are not enabled".to_string())))
}

fn thread_supervisor(st: &AppState) -> Result<Arc<ThreadSupervisor>, ApiError> {
    st.threads
        .as_ref()
        .cloned()
        .ok_or_else(|| ApiError(missing_threads_error()))
}

#[derive(Deserialize)]
pub(crate) struct ThreadReadQuery {
    #[serde(default)]
    include_events: bool,
    #[serde(default)]
    event_limit: Option<usize>,
    #[serde(default)]
    after_seq: Option<i64>,
}

const DEFAULT_THREAD_EVENT_LIMIT: usize = 1000;
const MAX_THREAD_EVENT_LIMIT: usize = 5000;

fn thread_event_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_THREAD_EVENT_LIMIT)
}

#[derive(Deserialize)]
pub(crate) struct ThreadChildrenQuery {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct ThreadEventsQuery {
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    event_limit: Option<usize>,
}

fn canonical_child_event(thread: milim_agents::AgentThread) -> SupervisorEvent {
    match thread.status.as_str() {
        "done" => SupervisorEvent::ChildThreadDone { thread },
        "error" => {
            let message = thread
                .error
                .clone()
                .unwrap_or_else(|| "child thread failed".to_string());
            SupervisorEvent::ChildThreadError { thread, message }
        }
        "stopped" => {
            let message = thread
                .error
                .clone()
                .unwrap_or_else(|| "child thread stopped".to_string());
            SupervisorEvent::ChildThreadStopped { thread, message }
        }
        _ => SupervisorEvent::ChildThreadStarted { thread },
    }
}

/// `GET /threads/{id}` - inspect one child thread.
pub(crate) async fn thread_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadReadQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let thread = supervisor
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    if query.include_events {
        let limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
        let events = if let Some(after_seq) = query.after_seq {
            supervisor
                .events_after(&id, after_seq.max(0), limit)
                .map_err(ApiError)?
        } else {
            supervisor.events(&id, limit).map_err(ApiError)?
        };
        let event_count = supervisor.event_count(&id).map_err(ApiError)?;
        Ok(Json(json!({
            "thread": thread,
            "events": events,
            "event_count": event_count,
            "events_truncated": event_count > events.len()
        }))
        .into_response())
    } else {
        Ok(Json(json!({ "thread": thread })).into_response())
    }
}

/// `GET /threads/{id}/children` - list children for a parent thread id.
pub(crate) async fn thread_children(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadChildrenQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let threads = supervisor
        .children(
            &id,
            query.status.as_deref(),
            query.limit.unwrap_or(50).clamp(1, 50),
        )
        .map_err(ApiError)?;
    Ok(Json(json!({ "threads": threads })).into_response())
}

/// `GET /threads/{id}/events` - pushed child-thread supervisor events.
pub(crate) async fn thread_events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadEventsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let mut events = supervisor.subscribe();
    let event_limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
    let initial_after_seq = query.after_seq.unwrap_or(0).max(0);
    let stream = async_stream::stream! {
        let mut last_seq = initial_after_seq;
        while let Ok(backfill) = supervisor.child_events_after(&id, last_seq, event_limit) {
            let drained = backfill.len() < event_limit;
            let previous_seq = last_seq;
            for (thread, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
                    .unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }
            if drained || last_seq == previous_seq {
                break;
            }
        }
        if let Ok(children) = supervisor.children(&id, None, 200) {
            for thread in children.into_iter().filter(|thread| thread.run_id.is_none()) {
                let data = serde_json::to_string(&canonical_child_event(thread))
                    .unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }
        }
        loop {
            match events.recv().await {
                Ok(event) => {
                    if event.thread().parent_id != id {
                        continue;
                    }
                    if let SupervisorEvent::ChildThreadEvent { event: stored, .. } = &event {
                        if stored.seq <= last_seq {
                            continue;
                        }
                        last_seq = stored.seq;
                    }
                    let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                    yield Ok::<Event, Infallible>(Event::default().data(data));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    while let Ok(backfill) =
                        supervisor.child_events_after(&id, last_seq, event_limit)
                    {
                        let drained = backfill.len() < event_limit;
                        let previous_seq = last_seq;
                        for (thread, event) in backfill {
                            last_seq = last_seq.max(event.seq);
                            let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
                                .unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().data(data));
                        }
                        if drained || last_seq == previous_seq {
                            break;
                        }
                    }
                    if let Ok(children) = supervisor.children(&id, None, 200) {
                        for thread in children.into_iter().filter(|thread| thread.run_id.is_none()) {
                            let data = serde_json::to_string(&canonical_child_event(thread))
                                .unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().data(data));
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// `POST /threads/{id}/stop` - stop a running child thread.
pub(crate) async fn thread_stop(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let thread = supervisor
        .stop(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("thread {id}"))))?;
    Ok(Json(json!({ "thread": thread })).into_response())
}

/// `DELETE /threads/{id}` - delete child-thread rows under a parent or child id.
pub(crate) async fn thread_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let deleted = supervisor.delete_tree(&id).map_err(ApiError)?;
    Ok(Json(json!({ "deleted": deleted.len() })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct WorkerRunsListQuery {
    parent_thread_id: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct WorkerRunCreateRequest {
    parent_thread_id: String,
    #[serde(default)]
    parent_turn_id: Option<String>,
    #[serde(default)]
    policy: milim_agents::DelegationPolicy,
    #[serde(default)]
    runtime: milim_agents::WorkerRuntime,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    workspace: RequestValue,
    #[serde(default)]
    privacy_mode: RequestValue,
    tasks: Vec<DelegateWorkerTaskArgs>,
}

pub(crate) async fn worker_runs_list(
    State(st): State<AppState>,
    Query(query): Query<WorkerRunsListQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let parent_id =
        trim_required_tool_arg(query.parent_thread_id, "parent_thread_id").map_err(ApiError)?;
    let supervisor = thread_supervisor(&st)?;
    let runs = supervisor
        .worker_runs(&parent_id, query.limit.unwrap_or(50).clamp(1, 200))
        .map_err(ApiError)?;
    let mut records = Vec::with_capacity(runs.len());
    for run in runs {
        let workers = supervisor.workers_for_run(&run.id).map_err(ApiError)?;
        records.push(json!({ "run": run, "workers": workers }));
    }
    Ok(Json(json!({ "runs": records })).into_response())
}

pub(crate) async fn worker_run_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkerRunCreateRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let run_context =
        RunContext::from_values(&st, req.workspace.as_value(), req.privacy_mode.as_value())
            .map_err(ApiError)?;
    if req.policy == milim_agents::DelegationPolicy::Off {
        return Err(ApiError(Error::InvalidRequest(
            "delegation is off for this thread".to_string(),
        )));
    }
    let parent_id =
        trim_required_tool_arg(req.parent_thread_id, "parent_thread_id").map_err(ApiError)?;
    let default_model = if let Some(model) = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        model.to_string()
    } else {
        st.service
            .list_models()
            .await
            .map_err(ApiError)?
            .first()
            .map(|m| m.id.clone())
            .ok_or_else(|| {
                ApiError(Error::InvalidRequest(
                    "no worker model is available".to_string(),
                ))
            })?
    };
    let (mut tasks, _) = resolve_worker_plan(
        &st,
        &run_context,
        &default_model,
        req.model.as_deref(),
        req.tasks,
    )
    .await
    .map_err(ApiError)?;
    for task in &mut tasks {
        task.access = milim_agents::WorkerAccess::ReadOnly;
    }
    // Native adapters normalize their own activity into this contract. This endpoint safely falls back to managed workers.
    let _requested_runtime = req.runtime;
    let supervisor = thread_supervisor(&st)?;
    let worker_context = managed_worker_context(run_context.workspace.as_deref(), None);
    let mut run = supervisor
        .store()
        .create_worker_run_with_origin(
            &parent_id,
            req.parent_turn_id.as_deref(),
            req.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            worker_context.as_deref(),
            run_context.workspace_text().as_deref(),
            run_context.privacy_mode.as_str(),
        )
        .map_err(ApiError)?;
    let mut workers = Vec::new();
    if req.policy == milim_agents::DelegationPolicy::Auto {
        (run, workers) = start_managed_worker_run(
            &st,
            &supervisor,
            &run,
            child_read_only_registry(&st, &run_context),
        )
        .await
        .map_err(ApiError)?;
        schedule_worker_run_deadline(supervisor.clone(), run.id.clone());
    }
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_get(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let workers = supervisor.workers_for_run(&id).map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    if matches!(
        run.status,
        milim_agents::WorkerRunStatus::Proposed | milim_agents::WorkerRunStatus::Running
    ) {
        return Err(ApiError(Error::InvalidRequest(
            "active worker runs must be stopped before deletion".to_string(),
        )));
    }
    let deleted = supervisor
        .store()
        .delete_worker_run(&id)
        .map_err(ApiError)?;
    Ok(Json(json!({ "deleted": deleted })).into_response())
}

pub(crate) async fn worker_run_start(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let run_context = RunContext::from_worker_run(&run).map_err(ApiError)?;
    let (run, workers) = start_managed_worker_run(
        &st,
        &supervisor,
        &run,
        worker_review_registry(&st, &run_context),
    )
    .await
    .map_err(ApiError)?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn control_worker_run_start(st: &AppState, id: &str) -> milim_core::Result<Value> {
    let supervisor = st
        .threads
        .as_ref()
        .cloned()
        .ok_or_else(|| Error::InvalidRequest("child threads are not enabled".to_string()))?;
    let run = supervisor
        .worker_run(id)?
        .ok_or_else(|| Error::ModelNotFound(format!("worker run {id}")))?;
    let run_context = RunContext::from_worker_run(&run)?;
    let (run, workers) = start_managed_worker_run(
        st,
        &supervisor,
        &run,
        worker_review_registry(st, &run_context),
    )
    .await?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(json!({ "run": run, "workers": workers }))
}

pub(crate) fn control_worker_run_stop(st: &AppState, id: &str) -> milim_core::Result<Value> {
    let supervisor = st
        .threads
        .as_ref()
        .cloned()
        .ok_or_else(|| Error::InvalidRequest("child threads are not enabled".to_string()))?;
    let run = supervisor
        .stop_run(id, "stopped by control client")?
        .ok_or_else(|| Error::ModelNotFound(format!("worker run {id}")))?;
    let workers = supervisor.workers_for_run(id)?;
    Ok(json!({ "run": run, "workers": workers }))
}

#[derive(Default, Deserialize)]
pub(crate) struct WorkerRunRetryRequest {
    #[serde(default)]
    model: Option<String>,
}

pub(crate) async fn worker_run_task_retry(
    State(st): State<AppState>,
    Path((id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<WorkerRunRetryRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let source = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let run_context = RunContext::from_worker_run(&source).map_err(ApiError)?;
    if matches!(
        source.status,
        milim_agents::WorkerRunStatus::Proposed | milim_agents::WorkerRunStatus::Running
    ) {
        return Err(ApiError(Error::InvalidRequest(
            "worker task can be retried only after its run finishes".to_string(),
        )));
    }
    let task = source
        .tasks
        .iter()
        .find(|task| task.id == task_id)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker task {task_id}"))))?;
    let requested_model = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or(&task.model);
    let (tasks, _) = resolve_worker_plan(
        &st,
        &run_context,
        &task.model,
        None,
        vec![DelegateWorkerTaskArgs {
            prompt: task.prompt.clone(),
            title: Some(task.title.clone()),
            role: task.role.clone(),
            agent_id: task.agent_id.clone(),
            model: Some(requested_model.to_string()),
            access: Some(task.access),
        }],
    )
    .await
    .map_err(ApiError)?;
    let retry = supervisor
        .store()
        .create_worker_run_with_origin(
            &source.parent_thread_id,
            source.parent_turn_id.as_deref(),
            source.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            source.context.as_deref(),
            run_context.workspace_text().as_deref(),
            run_context.privacy_mode.as_str(),
        )
        .map_err(ApiError)?;
    let (run, workers) = start_managed_worker_run(
        &st,
        &supervisor,
        &retry,
        worker_review_registry(&st, &run_context),
    )
    .await
    .map_err(ApiError)?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

pub(crate) async fn worker_run_stop(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .stop_run(&id, "stopped by parent")
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let workers = supervisor.workers_for_run(&id).map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
}

fn owned_worker(
    supervisor: &ThreadSupervisor,
    run_id: &str,
    worker_id: &str,
) -> Result<milim_agents::Worker, ApiError> {
    let worker = supervisor
        .get(worker_id)
        .map_err(ApiError)?
        .filter(|worker| worker.run_id.as_deref() == Some(run_id))
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker {worker_id}"))))?;
    Ok(worker)
}

pub(crate) async fn worker_run_worker_stop(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worker = supervisor
        .stop(&worker_id)
        .map_err(ApiError)?
        .unwrap_or(worker);
    let run = supervisor
        .store()
        .refresh_worker_run_status(&id)
        .map_err(ApiError)?;
    Ok(Json(json!({ "run": run, "worker": worker })).into_response())
}

pub(crate) async fn worker_run_worker_diff(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worktree = worker
        .worktree_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| {
            ApiError(Error::InvalidRequest(
                "worker has no review worktree".into(),
            ))
        })?;
    let payload = tokio::task::spawn_blocking(move || {
        let status = workspace_git_status_blocking(Some(worktree.clone()));
        let checkpoint =
            workspace_git_checkpoint_action(&worktree, &status, Some("worker-review".to_string()));
        let diff = checkpoint
            .checkpoint
            .as_deref()
            .and_then(|reference| {
                git_text(&worktree, &["diff", "--binary", "HEAD", reference, "--"])
            })
            .unwrap_or_default();
        json!({ "worker_id": worker_id, "status": status, "diff": diff })
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("worker diff task failed: {error}"))))?;
    Ok(Json(payload).into_response())
}

pub(crate) async fn worker_run_worker_apply(
    State(st): State<AppState>,
    Path((id, worker_id)): Path<(String, String)>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    let run = supervisor
        .worker_run(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("worker run {id}"))))?;
    let run_context = RunContext::from_worker_run(&run).map_err(ApiError)?;
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worktree = worker.worktree_path.clone().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "worker has no review worktree".into(),
        ))
    })?;
    let root = run_context
        .workspace
        .ok_or_else(|| ApiError(Error::InvalidRequest("no working folder selected".into())))?;
    let worktree_root = milim_core::paths::Paths::resolve()
        .root()
        .join("runtime")
        .join("hot-swap");
    let result = tokio::task::spawn_blocking(move || {
        let checkpoint = git_text(FsPath::new(&worktree), &["rev-parse", "HEAD"]);
        workspace_git_apply_retry_worktree_action(&root, checkpoint, Some(worktree), &worktree_root)
    })
    .await
    .map_err(|error| ApiError(Error::Other(format!("worker apply task failed: {error}"))))?;
    Ok(Json(result).into_response())
}

pub(crate) async fn worker_run_events(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ThreadEventsQuery>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let supervisor = thread_supervisor(&st)?;
    if supervisor.worker_run(&id).map_err(ApiError)?.is_none() {
        return Err(ApiError(Error::ModelNotFound(format!("worker run {id}"))));
    }
    let mut events = supervisor.subscribe();
    let limit = thread_event_limit(query.event_limit.unwrap_or(DEFAULT_THREAD_EVENT_LIMIT));
    let initial_after_seq = query.after_seq.unwrap_or(0).max(0);
    let stream = async_stream::stream! {
        let mut last_seq = initial_after_seq;
        while let Ok(backfill) = supervisor.worker_events_after(&id, last_seq, limit) {
            let drained = backfill.len() < limit;
            let previous_seq = last_seq;
            for (worker, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&json!({"type":"worker_run_worker_event","run_id":id,"worker":worker,"event":event})).unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
            }
            if drained || last_seq == previous_seq {
                break;
            }
        }
        if let Ok(Some(run)) = supervisor.store().refresh_worker_run_status(&id) {
            let workers = supervisor.workers_for_run(&id).unwrap_or_default();
            let kind = format!("worker_run_{}", worker_run_event_name(run.status));
            let data = serde_json::to_string(&json!({"type":kind,"run":run,"workers":workers})).unwrap_or_else(|_| "{}".to_string());
            yield Ok::<Event, Infallible>(Event::default().data(data));
        }
        loop {
            match events.recv().await {
                Ok(event) if event.thread().run_id.as_deref() == Some(id.as_str()) => {
                    if let SupervisorEvent::ChildThreadEvent { event: stored, .. } = &event {
                        if stored.seq <= last_seq { continue; }
                        last_seq = stored.seq;
                    }
                    let run = supervisor.store().refresh_worker_run_status(&id).ok().flatten();
                    let kind = match &event {
                        SupervisorEvent::ChildThreadStarted { .. } => "worker_run_worker_started",
                        SupervisorEvent::ChildThreadDone { .. } => "worker_run_worker_done",
                        SupervisorEvent::ChildThreadError { .. } => "worker_run_worker_error",
                        SupervisorEvent::ChildThreadStopped { .. } => "worker_run_worker_stopped",
                        SupervisorEvent::ChildThreadEvent { .. } => "worker_run_worker_event",
                    };
                    let stored = match &event {
                        SupervisorEvent::ChildThreadEvent { event, .. } => Some(event),
                        _ => None,
                    };
                    let data = serde_json::to_string(&json!({"type":kind,"run":run,"worker":event.thread(),"event":stored})).unwrap_or_else(|_| "{}".to_string());
                    yield Ok::<Event, Infallible>(Event::default().data(data));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    while let Ok(backfill) =
                        supervisor.worker_events_after(&id, last_seq, limit)
                    {
                        let drained = backfill.len() < limit;
                        let previous_seq = last_seq;
                        for (worker, event) in backfill {
                            last_seq = last_seq.max(event.seq);
                            let data = serde_json::to_string(&json!({"type":"worker_run_worker_event","run_id":id,"worker":worker,"event":event})).unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().data(data));
                        }
                        if drained || last_seq == previous_seq {
                            break;
                        }
                    }
                    if let Ok(Some(run)) = supervisor.store().refresh_worker_run_status(&id) {
                        let workers = supervisor.workers_for_run(&id).unwrap_or_default();
                        let kind = format!("worker_run_{}", worker_run_event_name(run.status));
                        let data = serde_json::to_string(&json!({"type":kind,"run":run,"workers":workers})).unwrap_or_else(|_| "{}".to_string());
                        yield Ok::<Event, Infallible>(Event::default().data(data));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

// ----- Schedules -----

#[derive(Deserialize)]
pub(crate) struct CreateScheduleRequest {
    name: String,
    cron: String,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    workspace: RequestValue,
    #[serde(default)]
    privacy: Option<String>,
    #[serde(default)]
    timezone_mode: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct UpdateScheduleRequest {
    name: String,
    cron: String,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default)]
    workspace: RequestValue,
    #[serde(default)]
    privacy: Option<String>,
    #[serde(default)]
    timezone_mode: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

fn provider_schedule_model(model: String) -> milim_core::Result<String> {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("codex:")
        || lower.starts_with("claude:")
        || lower.starts_with("opencode:")
        || lower.starts_with("pi:")
    {
        return Err(Error::InvalidRequest(
            "schedules require a configured provider model; account runtimes are interactive only"
                .to_string(),
        ));
    }
    Ok(model)
}

pub(crate) fn default_true() -> bool {
    true
}

fn schedule_json(schedule: milim_automation::Schedule) -> milim_core::Result<Value> {
    let next_run_unix = milim_automation::schedule_next_run(&schedule)?;
    let mut value = serde_json::to_value(schedule)
        .map_err(|error| Error::Other(format!("serialize schedule: {error}")))?;
    value
        .as_object_mut()
        .ok_or_else(|| Error::Other("serialized schedule is not an object".to_string()))?
        .insert("next_run_unix".to_string(), json!(next_run_unix));
    Ok(value)
}

fn schedule_workspace(
    requested: RequestValue,
    fallback: Option<String>,
) -> milim_core::Result<Option<String>> {
    match requested {
        RequestValue::Missing => Ok(fallback),
        RequestValue::Present(Value::Null) => Ok(None),
        RequestValue::Present(Value::String(value)) => {
            Ok(Some(value.trim().to_string()).filter(|value| !value.is_empty()))
        }
        RequestValue::Present(_) => Err(Error::InvalidRequest(
            "workspace must be a string or null".to_string(),
        )),
    }
}

/// `GET /schedules` — list cron schedules.
pub(crate) async fn schedules_list(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let schedules = match &st.schedules {
        Some(store) => store.list().map_err(ApiError)?,
        None => Vec::new(),
    };
    let schedules = schedules
        .into_iter()
        .map(schedule_json)
        .collect::<milim_core::Result<Vec<_>>>()
        .map_err(ApiError)?;
    Ok(Json(json!({ "schedules": schedules })).into_response())
}

/// `POST /schedules` - create a cron schedule.
pub(crate) async fn schedule_create(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<CreateScheduleRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    let model =
        provider_schedule_model(trim_required_tool_arg(req.model, "model").map_err(ApiError)?)
            .map_err(ApiError)?;
    let schedule = store
        .create_with_run_context(
            &req.name,
            &req.cron,
            req.agent_id,
            &model,
            &req.prompt,
            req.attachments,
            req.enabled,
            schedule_workspace(
                req.workspace,
                workspace_snapshot(&st).map(|path| path.to_string_lossy().to_string()),
            )
            .map_err(ApiError)?,
            req.privacy.as_deref().unwrap_or("off"),
            req.timezone_mode.as_deref().unwrap_or("local"),
        )
        .map_err(ApiError)?;
    Ok(Json(schedule_json(schedule).map_err(ApiError)?).into_response())
}

/// `DELETE /schedules/{id}` — remove a schedule.
/// `PUT /schedules/{id}` - update a cron schedule.
pub(crate) async fn schedule_update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    let current = find_schedule(store, &id).map_err(ApiError)?;
    let model =
        provider_schedule_model(trim_required_tool_arg(req.model, "model").map_err(ApiError)?)
            .map_err(ApiError)?;
    let workspace =
        schedule_workspace(req.workspace, current.workspace.clone()).map_err(ApiError)?;
    let privacy = req.privacy.unwrap_or_else(|| current.privacy.clone());
    let timezone_mode = req
        .timezone_mode
        .unwrap_or_else(|| current.timezone_mode.clone());
    let schedule = store
        .update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &req.name,
            cron: &req.cron,
            agent_id: req.agent_id,
            model: &model,
            prompt: &req.prompt,
            attachments: req.attachments,
            enabled: req.enabled.unwrap_or(current.enabled),
            workspace,
            privacy,
            timezone_mode,
            created_unix: current.created_unix,
            last_run: current.last_run,
        })
        .map_err(ApiError)?;
    Ok(Json(schedule_json(schedule).map_err(ApiError)?).into_response())
}

pub(crate) async fn schedule_delete(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let store = st.schedules.as_deref().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "schedules are not enabled".to_string(),
        ))
    })?;
    if store.delete(&id).map_err(ApiError)? {
        Ok(Json(json!({ "deleted": true })).into_response())
    } else {
        Err(ApiError(Error::ModelNotFound(format!("schedule {id}"))))
    }
}
