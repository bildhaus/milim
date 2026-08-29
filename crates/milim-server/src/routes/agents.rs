use super::*;

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
    let approval = match context.tool_context.tool_approval_policy.as_deref() {
        Some("review") => ToolApprovalPolicy::Review,
        Some("open") => ToolApprovalPolicy::Open,
        _ => ToolApprovalPolicy::Guarded,
    };
    let policy = ToolRunPolicy {
        approval,
        approval_granted: context.tool_context.tool_approval_grant,
        interactive_approval: context.tool_context.interactive_tool_approval,
        sandbox_enabled: context.tool_context.sandbox_enabled,
        computer_use_enabled: context.tool_context.computer_use_enabled,
        preview_tools_enabled: context.tool_context.preview_tools_enabled,
        experimental_hashline_patch: context.tool_context.experimental_hashline_patch,
        plan_mode: context.tool_context.plan_mode,
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
    if !policy.plan_mode {
        register_skill_tools(
            &mut registry,
            st,
            &context.skill_mode,
            &context.enabled_skills,
        );
        if registry.contains("preview_open_url") {
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
                "Milim account-runtime tools require a loopback server address".into(),
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
        .expect("account runtime tool store poisoned")
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
                .expect("account runtime tool store poisoned")
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
    "write_file",
    "edit_file",
    "patch_file",
    "shell",
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
    "http_fetch",
    "current_time",
    "echo",
];
const PLAN_MODE_READ_ONLY_TOOL_NAMES: &[&str] = &[
    "read_file",
    "list_dir",
    "list_agents",
    "linked_thread_list",
    "linked_thread_read",
];
const MAX_CHILD_THREAD_WAIT_MS: u64 = 300_000;
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
        }
    }
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
    let approval = match string_extra(req, "tool_approval_policy").as_deref() {
        Some("review") => ToolApprovalPolicy::Review,
        Some("open") => ToolApprovalPolicy::Open,
        _ => ToolApprovalPolicy::Guarded,
    };
    ToolRunPolicy {
        approval,
        approval_granted: bool_extra(req, "tool_approval_grant"),
        interactive_approval: bool_extra(req, "interactive_tool_approval"),
        sandbox_enabled: bool_extra(req, "sandbox_enabled"),
        computer_use_enabled: bool_extra(req, "computer_use_enabled"),
        preview_tools_enabled: bool_extra(req, "preview_tools_enabled"),
        experimental_hashline_patch: bool_extra(req, "experimental_hashline_patch"),
        plan_mode: bool_extra(req, "plan_mode"),
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
        worker_context: worker_context_from_request(req),
        linked_thread_grants: Vec::new(),
    }
}

fn worker_context_from_request(req: &ChatCompletionRequest) -> Option<String> {
    let instructions = req
        .messages
        .iter()
        .filter(|message| message.role == "system")
        .map(ChatMessage::text_content)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let request = req
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(ChatMessage::text_content)
        .unwrap_or_default();
    let context = [
        (!request.trim().is_empty()).then(|| format!("Current request:\n{request}")),
        (!instructions.trim().is_empty())
            .then(|| format!("Resolved instructions and skills:\n{instructions}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n");
    (!context.is_empty()).then(|| context.chars().take(32_000).collect())
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
    reg.scoped_for_run()
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

fn add_workspace_notice_if_needed(messages: &mut Vec<ChatMessage>, workspace_unavailable: bool) {
    if !workspace_unavailable {
        return;
    }
    let insert_at = messages
        .iter()
        .position(|message| message.role != "system")
        .unwrap_or(messages.len());
    messages.insert(
        insert_at,
        ChatMessage::text("system", WORKSPACE_UNAVAILABLE_SYSTEM_PROMPT),
    );
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
fn agent_registry_with_memory(
    st: &AppState,
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
    run_context: &RunContext,
) -> ToolRegistry {
    agent_registry_for_mode_with_context(st, "all", &[], memory, policy, run_context)
}

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
    (policy.approval_granted
        || policy.interactive_approval
        || policy.approval != ToolApprovalPolicy::Review)
        && !policy.plan_mode
}

#[derive(Deserialize)]
pub(crate) struct ToolApprovalDecision {
    decision: String,
    #[serde(default)]
    response: Option<Value>,
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
    match st
        .tool_approvals
        .resolve_with_response(&id, approved, req.response)
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
        reg = reg.without(&["read_file", "list_dir", "read_file_anchors"]);
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
    let normalized = milim_agents::normalize_tool_mode(tool_mode, enabled_tools);
    let inherited = match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => all.filtered(enabled_tools),
        _ => all,
    };
    let mut reg = inherited.clone();
    if let (Some(memory), Some(supervisor)) = (memory, st.threads.as_ref()) {
        if tools_available(policy) && child_thread_tools_allowed(supervisor, &memory) {
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
    }
    match normalized.as_str() {
        "none" => ToolRegistry::new(),
        "custom" if enabled_tools.is_empty() => ToolRegistry::new(),
        "custom" => reg.filtered(enabled_tools),
        _ => reg,
    }
}

fn register_linked_thread_tools(
    registry: &mut ToolRegistry,
    state: AppState,
    control: Arc<crate::control::RunManager>,
    context: AgentMemoryContext,
    policy: &ToolRunPolicy,
) {
    let Some(origin_thread_id) = context.thread_id.clone() else {
        return;
    };
    let grants = context.linked_thread_grants.clone();
    registry.register(Arc::new(LinkedThreadListTool {
        control: control.clone(),
        origin_thread_id: origin_thread_id.clone(),
        grants: grants.clone(),
    }));
    registry.register(Arc::new(LinkedThreadReadTool {
        control: control.clone(),
        grants: grants.clone(),
    }));
    if !policy.plan_mode && policy.approval != ToolApprovalPolicy::Guarded {
        let origin_run_id = context.message_id.clone();
        let destinations = grants
            .iter()
            .map(|grant| {
                format!(
                    "{} [{}] in {} using {}/{}",
                    grant.title,
                    grant.target_thread_id,
                    grant.project.as_deref().unwrap_or("unknown project"),
                    grant.model.as_deref().unwrap_or("unknown model"),
                    grant.runtime
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        registry.register(Arc::new(LinkedThreadSendTool {
            state,
            control: control.clone(),
            origin_thread_id: origin_thread_id.clone(),
            origin_run_id: origin_run_id.clone(),
            grants: grants.clone(),
            description: format!(
                "Send a message to a linked Milim thread. This starts idle work, steers a compatible active run, or queues a durable follow-up with the destination's own settings, then returns asynchronously through the mailbox. When this run depends on the reply, pass the returned exchange_id to linked_thread_wait. Destinations: {destinations}"
            ),
        }));
        if let Some(origin_run_id) = origin_run_id {
            registry.register(Arc::new(LinkedThreadWaitTool {
                control,
                origin_thread_id,
                origin_run_id,
                grants,
            }));
        }
    }
}

struct LinkedThreadListTool {
    control: Arc<crate::control::RunManager>,
    origin_thread_id: String,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[async_trait]
impl Tool for LinkedThreadListTool {
    fn name(&self) -> &str {
        "linked_thread_list"
    }

    fn description(&self) -> &str {
        "List the linked-thread grants frozen for this run and the origin thread's mailbox states."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        self.control
            .linked_thread_list(&self.origin_thread_id, &self.grants)
    }
}

struct LinkedThreadReadTool {
    control: Arc<crate::control::RunManager>,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[derive(Deserialize)]
struct LinkedThreadReadArgs {
    target_thread_id: String,
    #[serde(default)]
    after_seq: Option<u64>,
    #[serde(default = "default_linked_thread_read_limit")]
    limit: usize,
}

fn default_linked_thread_read_limit() -> usize {
    20
}

#[async_trait]
impl Tool for LinkedThreadReadTool {
    fn name(&self) -> &str {
        "linked_thread_read"
    }

    fn description(&self) -> &str {
        "Read canonical visible user/assistant messages from a linked thread up to this run's frozen sequence. Hidden prompts, reasoning, tool ledgers, and account-runtime history are excluded."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_thread_id": { "type": "string" },
                "after_seq": { "type": "integer", "minimum": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 }
            },
            "required": ["target_thread_id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadReadArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_read arguments: {error}"))
        })?;
        self.control.linked_thread_read(
            &self.grants,
            args.target_thread_id.trim(),
            args.after_seq,
            args.limit,
        )
    }
}

struct LinkedThreadSendTool {
    state: AppState,
    control: Arc<crate::control::RunManager>,
    origin_thread_id: String,
    origin_run_id: Option<String>,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
    description: String,
}

#[derive(Deserialize)]
struct LinkedThreadSendArgs {
    target_thread_id: String,
    message: String,
}

#[async_trait]
impl Tool for LinkedThreadSendTool {
    fn name(&self) -> &str {
        "linked_thread_send"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "target_thread_id": { "type": "string" },
                "message": { "type": "string", "minLength": 1 }
            },
            "required": ["target_thread_id", "message"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Exclusive
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadSendArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_send arguments: {error}"))
        })?;
        self.control
            .linked_thread_send(
                self.state.clone(),
                &self.origin_thread_id,
                self.origin_run_id.as_deref(),
                &self.grants,
                args.target_thread_id.trim(),
                &args.message,
            )
            .await
    }
}

struct LinkedThreadWaitTool {
    control: Arc<crate::control::RunManager>,
    origin_thread_id: String,
    origin_run_id: String,
    grants: Vec<crate::control::FrozenLinkedThreadGrantV1>,
}

#[derive(Deserialize)]
struct LinkedThreadWaitArgs {
    exchange_id: String,
    #[serde(default = "default_linked_thread_wait_ms")]
    timeout_ms: u64,
}

fn default_linked_thread_wait_ms() -> u64 {
    DEFAULT_LINKED_THREAD_WAIT_MS
}

#[async_trait]
impl Tool for LinkedThreadWaitTool {
    fn name(&self) -> &str {
        "linked_thread_wait"
    }

    fn description(&self) -> &str {
        "Wait for a specific linked_thread_send exchange only when this run depends on its reply. The wait is bounded, may be repeated after a timeout, and never discards a late reply."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "exchange_id": { "type": "string", "minLength": 1 },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 100,
                    "maximum": crate::control::MAX_LINKED_THREAD_WAIT_MS,
                    "default": DEFAULT_LINKED_THREAD_WAIT_MS
                }
            },
            "required": ["exchange_id"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> milim_tools::ToolConcurrency {
        milim_tools::ToolConcurrency::Parallel
    }

    fn environment_policy(&self) -> milim_tools::ProcessEnvironmentPolicy {
        milim_tools::ProcessEnvironmentPolicy::ConfiguredIntegrationSanitized
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: LinkedThreadWaitArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid linked_thread_wait arguments: {error}"))
        })?;
        self.control
            .linked_thread_wait(
                &self.origin_thread_id,
                &self.origin_run_id,
                &self.grants,
                args.exchange_id.trim(),
                args.timeout_ms,
            )
            .await
    }
}

pub(crate) struct MemoryRegisterTool {
    pub(crate) store: Arc<milim_memory::MemoryStore>,
    pub(crate) context: AgentMemoryContext,
}

struct ListAgentsTool {
    store: Arc<milim_agents::AgentStore>,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "List reusable Milim Agents and compact tool/skill capability summaries. System prompts are never returned."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        let agents = self
            .store
            .list()?
            .into_iter()
            .map(|agent| {
                json!({
                    "id": agent.id,
                    "name": agent.name,
                    "description": agent.description,
                    "avatar": agent.avatar,
                    "tools": {
                        "mode": agent.tool_mode,
                        "count": agent.enabled_tools.len(),
                        "names": agent.enabled_tools,
                    },
                    "skills": {
                        "mode": agent.skill_mode,
                        "count": agent.enabled_skills.len(),
                        "names": agent.enabled_skills,
                    }
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "agents": agents }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryRegisterArgs {
    #[serde(default)]
    scope: Option<String>,
    content: String,
    #[serde(default)]
    title: Option<String>,
}

#[async_trait]
impl Tool for MemoryRegisterTool {
    fn name(&self) -> &str {
        "memory_register"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Save concise durable context to Personal or Project memory. Use this only for facts, decisions, preferences, and project context likely to help future turns."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["personal", "project"],
                    "description": "Where to store the memory. Defaults to project when a workspace folder exists, otherwise personal."
                },
                "content": { "type": "string", "description": "One or two sentences with the useful durable context." },
                "title": { "type": "string", "description": "Optional short human-readable title." }
            },
            "required": ["content"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: MemoryRegisterArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid memory_register arguments: {e}"))
        })?;
        let content = trim_required_tool_arg(args.content, "content")?;
        let title = args
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                content
                    .lines()
                    .next()
                    .unwrap_or("Memory")
                    .chars()
                    .take(80)
                    .collect()
            });
        let requested_scope = args
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| {
                if self.context.project_locator.is_some() {
                    "project".to_string()
                } else {
                    "personal".to_string()
                }
            });

        let (scope_kind, locator, label) = match requested_scope.as_str() {
            "project" => {
                let locator = self.context.project_locator.clone().ok_or_else(|| {
                    Error::InvalidRequest(
                        "project memory requires an active project folder".to_string(),
                    )
                })?;
                let label = self
                    .context
                    .project_label
                    .clone()
                    .unwrap_or_else(|| locator.clone());
                ("project".to_string(), locator, label)
            }
            "personal" => (
                "global".to_string(),
                "personal".to_string(),
                "Personal".to_string(),
            ),
            _ => {
                return Err(Error::InvalidRequest(
                    "memory_register scope must be personal or project".to_string(),
                ))
            }
        };

        let registration = self
            .store
            .register(
                &self.context.model,
                milim_memory::MemoryScopeInput {
                    kind: scope_kind,
                    label,
                    locator,
                },
                milim_memory::MemoryNodeInput {
                    kind: "fact".to_string(),
                    title,
                    body: content,
                    confidence: 0.85,
                    source: "agent".to_string(),
                },
                Vec::new(),
                milim_memory::MemoryEventInput {
                    thread_id: self.context.thread_id.clone().unwrap_or_default(),
                    message_id: self.context.message_id.clone().unwrap_or_default(),
                    summary: String::new(),
                },
            )
            .await?;
        Ok(json!({
            "ok": true,
            "memory": registration.node,
            "scope": registration.scope,
            "memory_notice": registration.notice
        }))
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
        reg = reg.without(&["read_file", "list_dir"]);
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

fn child_thread_parent_id(context: &AgentMemoryContext) -> milim_core::Result<String> {
    context
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            Error::InvalidRequest("child threads require a parent thread id".to_string())
        })
}

fn child_thread_title(title: Option<String>, prompt: &str) -> String {
    title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| prompt.chars().take(80).collect())
}

fn worker_run_event_name(status: milim_agents::WorkerRunStatus) -> &'static str {
    match status {
        milim_agents::WorkerRunStatus::Proposed => "proposed",
        milim_agents::WorkerRunStatus::Running => "started",
        milim_agents::WorkerRunStatus::Done | milim_agents::WorkerRunStatus::Partial => "done",
        milim_agents::WorkerRunStatus::Stopped | milim_agents::WorkerRunStatus::Error => "error",
    }
}

fn worker_run_notice(run: &milim_agents::WorkerRun, workers: &[milim_agents::Worker]) -> Value {
    json!({ "event": worker_run_event_name(run.status), "run": run, "workers": workers, "message": run.error })
}

struct DelegateWorkersTool {
    state: AppState,
    supervisor: Arc<ThreadSupervisor>,
    context: AgentMemoryContext,
    child_tools: ToolRegistry,
    allow_write_review: bool,
    auto_approve_workers: bool,
    run_context: RunContext,
}

#[derive(Debug, Clone, Deserialize)]
struct DelegateWorkerTaskArgs {
    prompt: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    access: Option<milim_agents::WorkerAccess>,
}

#[derive(Debug, Deserialize)]
struct DelegateWorkersArgs {
    tasks: Vec<DelegateWorkerTaskArgs>,
}

fn worker_model_is_available(available: &[Model], model: &str) -> bool {
    match crate::providers::provider_model_route(model) {
        Some((provider_id, model_id)) => available.iter().any(|candidate| {
            candidate.id == model_id && candidate.provider_id.as_deref() == Some(&provider_id)
        }),
        None => available.iter().any(|candidate| candidate.id == model),
    }
}

fn account_runtime_worker_target(model: &str) -> Option<(&'static str, &str)> {
    let model = model.trim();
    for (adapter, prefix) in [
        ("codex", "codex:"),
        ("claude", "claude:"),
        ("opencode", "opencode:"),
        ("pi", "pi:"),
    ] {
        if model
            .get(..prefix.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        {
            let runtime_model = model.get(prefix.len()..)?.trim();
            return (!runtime_model.is_empty()).then_some((adapter, runtime_model));
        }
    }
    None
}

fn resolve_account_runtime_worker_model(requested: &str, preferred_model: &str) -> Option<String> {
    let requested = requested.trim();
    if account_runtime_worker_target(requested).is_some() {
        return Some(requested.to_string());
    }
    let (adapter, _) = account_runtime_worker_target(preferred_model)?;
    (!requested.is_empty() && !requested.contains(':')).then(|| format!("{adapter}:{requested}"))
}

fn resolve_worker_model(
    available: &[Model],
    requested: &str,
    preferred_model: &str,
) -> milim_core::Result<String> {
    let requested = requested.trim();
    if worker_model_is_available(available, requested) {
        return Ok(requested.to_string());
    }
    if requested.contains('/') || requested.starts_with("provider:") {
        return Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is not available"
        )));
    }

    let (preferred_provider, preferred_id) =
        match crate::providers::provider_model_route(preferred_model) {
            Some((provider_id, model_id)) => (Some(provider_id), model_id),
            None => (None, preferred_model.trim().to_string()),
        };
    if let Some((namespace, _)) = preferred_id.split_once('/') {
        let model_id = format!("{namespace}/{requested}");
        if let Some(provider_id) = preferred_provider.as_deref() {
            let routed = crate::providers::provider_model_id(provider_id, &model_id);
            if worker_model_is_available(available, &routed) {
                return Ok(routed);
            }
        } else if worker_model_is_available(available, &model_id) {
            return Ok(model_id);
        }
    }

    let mut matches = available
        .iter()
        .filter(|candidate| {
            candidate
                .id
                .rsplit_once('/')
                .is_some_and(|(_, name)| name == requested)
        })
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    match matches.as_slice() {
        [model] => Ok(model.clone()),
        [] => Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is not available"
        ))),
        _ => Err(Error::InvalidRequest(format!(
            "worker model '{requested}' is ambiguous; use a full provider/model id"
        ))),
    }
}

async fn resolve_worker_plan(
    state: &AppState,
    run_context: &RunContext,
    parent_model: &str,
    worker_model: Option<&str>,
    tasks: Vec<DelegateWorkerTaskArgs>,
) -> milim_core::Result<(Vec<milim_agents::WorkerPlanTask>, Vec<Option<String>>)> {
    if !(1..=4).contains(&tasks.len()) {
        return Err(Error::InvalidRequest(
            "delegate_workers requires 1 to 4 independent tasks".to_string(),
        ));
    }
    let mut available = None;
    let mut plan = Vec::with_capacity(tasks.len());
    let mut system_prompts = Vec::with_capacity(tasks.len());
    for task in tasks {
        let prompt = trim_required_tool_arg(task.prompt, "tasks[].prompt")?;
        let preferred_model = worker_model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(parent_model.trim());
        let requested_model = task
            .model
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(preferred_model);
        let model = if let Some(model) =
            resolve_account_runtime_worker_model(requested_model, preferred_model)
        {
            model
        } else {
            if available.is_none() {
                available = Some(service_for_run(state, run_context).list_models().await?);
            }
            resolve_worker_model(
                available.as_deref().unwrap_or_default(),
                requested_model,
                preferred_model,
            )?
        };
        let agent_id = trim_optional_agent_id(task.agent_id);
        let agent_snapshot = if let Some(agent_id) = agent_id.as_deref() {
            let store = state
                .agents
                .as_ref()
                .ok_or_else(|| Error::InvalidRequest("named agents are not enabled".to_string()))?;
            let agent = store
                .get(agent_id)?
                .ok_or_else(|| Error::ModelNotFound(format!("agent {agent_id}")))?;
            Some(milim_agents::WorkerAgentSnapshot {
                id: agent.id,
                name: agent.name,
                description: agent.description,
                system_prompt: agent.system_prompt,
                tool_mode: agent.tool_mode,
                enabled_tools: agent.enabled_tools,
                skill_mode: agent.skill_mode,
                enabled_skills: agent.enabled_skills,
                avatar: agent.avatar,
            })
        } else {
            None
        };
        let system_prompt = agent_snapshot.as_ref().and_then(|agent| {
            (!agent.system_prompt.trim().is_empty()).then(|| agent.system_prompt.clone())
        });
        let title = child_thread_title(task.title, &prompt);
        let account_runtime = account_runtime_worker_target(&model).is_some();
        plan.push(milim_agents::WorkerPlanTask {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            prompt,
            role: task.role,
            agent_id,
            agent_snapshot,
            model,
            access: if account_runtime {
                milim_agents::WorkerAccess::ReadOnly
            } else {
                task.access.unwrap_or_default()
            },
        });
        system_prompts.push(system_prompt);
    }
    Ok((plan, system_prompts))
}

#[cfg(test)]
mod worker_model_tests {
    use super::*;

    fn provider_model(provider_id: &str, model_id: &str) -> Model {
        let mut model = Model::local(model_id, 0);
        model.provider_id = Some(provider_id.to_string());
        model
    }

    #[test]
    fn resolves_worker_model_aliases_without_guessing_invalid_ids() {
        let available = vec![
            provider_model("openrouter", "openai/gpt-5.4"),
            Model::local("anthropic/claude-sonnet", 0),
        ];

        assert_eq!(
            resolve_worker_model(&available, "openai/gpt-5.4", "unused").unwrap(),
            "openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "provider:openrouter:openai/gpt-5.4", "unused")
                .unwrap(),
            "provider:openrouter:openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "gpt-5.4", "openai/parent").unwrap(),
            "openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "gpt-5.4", "provider:openrouter:openai/gpt-5.4")
                .unwrap(),
            "provider:openrouter:openai/gpt-5.4"
        );
        assert_eq!(
            resolve_worker_model(&available, "claude-sonnet", "local-parent").unwrap(),
            "anthropic/claude-sonnet"
        );

        let namespace_preferred = vec![
            Model::local("openai/shared", 0),
            Model::local("other/shared", 0),
        ];
        assert_eq!(
            resolve_worker_model(&namespace_preferred, "shared", "openai/parent").unwrap(),
            "openai/shared"
        );

        assert!(
            resolve_worker_model(&namespace_preferred, "shared", "local-parent")
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(resolve_worker_model(&available, "missing", "local-parent")
            .unwrap_err()
            .to_string()
            .contains("not available"));
        assert!(
            resolve_worker_model(&available, "openai/missing", "local-parent")
                .unwrap_err()
                .to_string()
                .contains("not available")
        );
    }

    #[test]
    fn account_runtime_workers_inherit_runtime_without_reusing_provider_routing() {
        assert_eq!(
            resolve_account_runtime_worker_model("codex:gpt-5.6", "codex:gpt-5.6"),
            Some("codex:gpt-5.6".to_string())
        );
        assert_eq!(
            resolve_account_runtime_worker_model("gpt-5.5", "codex:gpt-5.6"),
            Some("codex:gpt-5.5".to_string())
        );
        assert_eq!(
            resolve_account_runtime_worker_model(
                "provider:openrouter:openai/gpt-5.6",
                "codex:gpt-5.6"
            ),
            None
        );
        assert_eq!(
            account_runtime_worker_target("pi:openai-codex/gpt-5.3-codex"),
            Some(("pi", "openai-codex/gpt-5.3-codex"))
        );
    }

    #[tokio::test]
    async fn account_runtime_worker_plan_freezes_inherited_runtime_as_read_only() {
        let state = AppState::new(
            Arc::new(milim_inference::test_backend::TestBackend::new()),
            milim_core::config::ServerConfiguration::default(),
        );
        let (tasks, _) = resolve_worker_plan(
            &state,
            &RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
            "opencode:openai/gpt-5.6",
            None,
            vec![DelegateWorkerTaskArgs {
                prompt: "Inspect the code.".to_string(),
                title: None,
                role: None,
                agent_id: None,
                model: None,
                access: Some(milim_agents::WorkerAccess::WriteReview),
            }],
        )
        .await
        .unwrap();

        assert_eq!(tasks[0].model, "opencode:openai/gpt-5.6");
        assert_eq!(tasks[0].access, milim_agents::WorkerAccess::ReadOnly);
    }

    #[test]
    fn account_runtime_worker_request_uses_a_fresh_guarded_session() {
        let spec = ChildRunSpec {
            parent_id: "parent".to_string(),
            title: "Worker".to_string(),
            model: "claude:sonnet".to_string(),
            agent_id: None,
            system_prompt: Some("Inspect the implementation.".to_string()),
            prompt: "Find the bug.".to_string(),
            run_id: Some("run".to_string()),
            runtime: milim_agents::WorkerRuntime::Managed,
            access: milim_agents::WorkerAccess::ReadOnly,
            worktree_path: None,
        };
        let (adapter, request) = account_worker_harness_request(
            &spec,
            &RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
        )
        .unwrap();

        assert_eq!(adapter, "claude");
        assert_eq!(request.model, "sonnet");
        assert_eq!(request.native_session_id, None);
        assert_eq!(request.persist_session, Some(false));
        assert_eq!(request.tool_approval_policy.as_deref(), Some("guarded"));
        assert!(!request.interactive_tool_approval);
        assert_eq!(
            request
                .milim_context
                .as_ref()
                .and_then(|value| value.pointer("/tool_context/delegation_policy"))
                .and_then(Value::as_str),
            Some("off")
        );
        assert_eq!(
            request
                .milim_context
                .as_ref()
                .and_then(|value| value.get("tool_mode"))
                .and_then(Value::as_str),
            Some("none")
        );
        assert!(request.prompt.contains("Do not delegate more work"));
    }

    #[tokio::test]
    async fn account_runtime_worker_events_normalize_into_the_existing_worker_stream() {
        use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};
        use serde_json::Map;

        let mut delta = Map::new();
        delta.insert("text".to_string(), Value::String("done".to_string()));
        let stream: AccountHarnessStream = Box::pin(futures::stream::iter(vec![
            HarnessEvent::new(HarnessEventKind::TextDelta, delta),
            HarnessEvent::new(HarnessEventKind::TurnCompleted, Map::new()),
        ]));
        let events = account_worker_events(stream).collect::<Vec<_>>().await;

        assert!(matches!(
            events.first(),
            Some(milim_agents::AgentEvent::Token { text }) if text == "done"
        ));
        assert!(matches!(
            events.get(1),
            Some(milim_agents::AgentEvent::Final { content }) if content == "done"
        ));
        assert!(matches!(
            events.get(2),
            Some(milim_agents::AgentEvent::Done { iterations: 1, .. })
        ));
    }
}

fn worker_specs(
    run: &milim_agents::WorkerRun,
    system_prompts: Vec<Option<String>>,
) -> Vec<ChildRunSpec> {
    run.tasks
        .iter()
        .cloned()
        .zip(system_prompts)
        .map(|(task, system_prompt)| {
            let system_prompt = [run.context.as_deref(), system_prompt.as_deref()]
                .into_iter()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            ChildRunSpec {
                parent_id: run.parent_thread_id.clone(),
                title: task.title,
                model: task.model,
                agent_id: task.agent_id,
                system_prompt: (!system_prompt.is_empty()).then_some(system_prompt),
                prompt: task.prompt,
                run_id: Some(run.id.clone()),
                runtime: run.runtime,
                access: task.access,
                worktree_path: None,
            }
        })
        .collect()
}

fn account_worker_harness_request(
    spec: &ChildRunSpec,
    run_context: &RunContext,
) -> milim_core::Result<(String, HarnessRunRequest)> {
    let (adapter, model) = account_runtime_worker_target(&spec.model).ok_or_else(|| {
        Error::InvalidRequest(format!(
            "worker model '{}' is not an account runtime",
            spec.model
        ))
    })?;
    let mut instructions = vec![
        "You are a Milim Worker. Complete only the delegated task and return a concise final report. Do not delegate more work. Your workspace access is read-only."
            .to_string(),
    ];
    if let Some(system_prompt) = spec
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        instructions.insert(0, system_prompt.to_string());
    }
    let prompt = format!(
        "System instructions:\n{}\n\n{}",
        instructions.join("\n\n"),
        spec.prompt
    );
    Ok((
        adapter.to_string(),
        HarnessRunRequest {
            prompt,
            images: Vec::new(),
            model: model.to_string(),
            cwd: spec
                .worktree_path
                .clone()
                .or_else(|| run_context.workspace_text()),
            reasoning_effort: None,
            native_session_id: None,
            persist_session: Some(false),
            tool_approval_policy: Some("guarded".to_string()),
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: Some(json!({
                "tool_context": {
                    "parent_model": spec.model,
                    "workspace": run_context.workspace_text(),
                    "privacy_mode": run_context.privacy_mode.as_str(),
                    "tool_approval_policy": "guarded",
                    "delegation_policy": "off",
                },
                "tool_mode": "none",
                "skill_mode": "auto",
            })),
        },
    ))
}

fn account_worker_agent_stream(
    state: &AppState,
    run_context: &RunContext,
    spec: ChildRunSpec,
) -> milim_core::Result<crate::threads::ChildAgentStream> {
    let (adapter, request) = account_worker_harness_request(&spec, run_context)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        HOST,
        HeaderValue::from_str(&format!("127.0.0.1:{}", state.config.port))
            .map_err(|error| Error::Other(format!("invalid Worker host header: {error}")))?,
    );
    let stream =
        account_harness_stream(state, &headers, &adapter, request).map_err(|error| error.0)?;
    Ok(Box::pin(account_worker_events(stream)))
}

fn account_worker_events(
    mut stream: AccountHarnessStream,
) -> impl futures::Stream<Item = milim_agents::AgentEvent> + Send {
    async_stream::stream! {
        let mut content = String::new();
        let mut usage = Usage::new(0, 0);
        let mut terminal = false;
        while let Some(event) = stream.next().await {
            let value = serde_json::to_value(event).unwrap_or_else(|error| {
                json!({"type":"turn_failed","message":format!("serialize Worker harness event: {error}")})
            });
            if let Some(next_usage) = value
                .get("usage")
                .filter(|value| !value.is_null())
                .and_then(|value| serde_json::from_value::<Usage>(value.clone()).ok())
            {
                usage = next_usage;
                yield milim_agents::AgentEvent::UsageDelta { usage: next_usage };
            }
            match value.get("type").and_then(Value::as_str).unwrap_or_default() {
                "text_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                        yield milim_agents::AgentEvent::Token { text: text.to_string() };
                    }
                }
                "reasoning_delta" => {
                    if let Some(text) = value.get("text").and_then(Value::as_str) {
                        yield milim_agents::AgentEvent::Reasoning { text: text.to_string() };
                    }
                }
                "tool_started" => {
                    let call_id = value.get("id").and_then(Value::as_str).map(str::to_string);
                    let name = value.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
                    let arguments = value
                        .get("arguments")
                        .or_else(|| value.get("input"))
                        .map(|value| value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string()))
                        .unwrap_or_else(|| "{}".to_string());
                    yield milim_agents::AgentEvent::ToolCall {
                        call_id,
                        name,
                        arguments,
                        mcp_app: None,
                    };
                }
                "tool_finished" => {
                    let call_id = value.get("id").and_then(Value::as_str).map(str::to_string);
                    let name = value.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
                    let result = value
                        .get("result")
                        .or_else(|| value.get("output"))
                        .cloned()
                        .unwrap_or(Value::Null);
                    yield milim_agents::AgentEvent::ToolResult {
                        call_id,
                        name,
                        result,
                        mcp_app: None,
                        mcp_app_result: None,
                    };
                }
                "turn_completed" => {
                    if content.trim().is_empty() {
                        content = value
                            .get("content")
                            .or_else(|| value.get("text"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                    }
                    yield milim_agents::AgentEvent::Final { content: content.clone() };
                    yield milim_agents::AgentEvent::Done {
                        iterations: 1,
                        stopped_at_limit: false,
                        usage,
                    };
                    terminal = true;
                    break;
                }
                "turn_failed" | "turn_cancelled" => {
                    let message = value
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("account-runtime Worker failed")
                        .to_string();
                    yield milim_agents::AgentEvent::Error { message };
                    terminal = true;
                    break;
                }
                _ => {}
            }
        }
        if !terminal {
            yield milim_agents::AgentEvent::Error {
                message: "account-runtime Worker ended without a terminal event".to_string(),
            };
        }
    }
}

fn managed_worker_context(workspace: Option<&FsPath>, base: Option<&str>) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(base) = base.filter(|value| !value.trim().is_empty()) {
        sections.push(base.to_string());
    }
    if let Some(workspace) = workspace {
        let branch = git_text(workspace, &["branch", "--show-current"]);
        sections.push(format!(
            "Workspace: {}{}",
            workspace.display(),
            branch
                .filter(|value| !value.is_empty())
                .map(|value| format!("\nBranch: {value}"))
                .unwrap_or_default(),
        ));
        let context = crate::workspace_context::resolve(Some(workspace));
        if let Some(instructions) = crate::workspace_context::formatted(&context, None) {
            sections.push(instructions);
        }
    }
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

pub(crate) async fn start_managed_worker_run(
    state: &AppState,
    supervisor: &ThreadSupervisor,
    run: &milim_agents::WorkerRun,
    tools: ToolRegistry,
) -> milim_core::Result<(milim_agents::WorkerRun, Vec<milim_agents::Worker>)> {
    if run.status != milim_agents::WorkerRunStatus::Proposed {
        return Err(Error::InvalidRequest(
            "worker run is not awaiting approval".to_string(),
        ));
    }
    let run_context = RunContext::from_worker_run(run)?;
    let service = service_for_run(state, &run_context);
    if run
        .tasks
        .iter()
        .any(|task| task.agent_id.is_some() && task.agent_snapshot.is_none())
    {
        return Err(Error::InvalidRequest(
            "this proposed Worker plan predates frozen Agent snapshots; create a new proposal"
                .to_string(),
        ));
    }
    let prompts = run
        .tasks
        .iter()
        .map(|task| {
            task.agent_snapshot.as_ref().and_then(|agent| {
                (!agent.system_prompt.trim().is_empty()).then(|| agent.system_prompt.clone())
            })
        })
        .collect();
    let running = supervisor
        .store()
        .update_worker_run_status(&run.id, milim_agents::WorkerRunStatus::Running, None)?
        .ok_or_else(|| Error::ModelNotFound(format!("worker run {}", run.id)))?;
    let mut workers = Vec::with_capacity(running.tasks.len());
    for mut spec in worker_specs(&running, prompts) {
        let worker_tools = if spec.access == milim_agents::WorkerAccess::WriteReview {
            match create_worker_worktree(run_context.workspace.clone()).await {
                Some(path) => {
                    spec.worktree_path = Some(path.to_string_lossy().to_string());
                    tools.scoped_to_workspace(&path)
                }
                None => {
                    spec.access = milim_agents::WorkerAccess::ReadOnly;
                    tools.read_only()
                }
            }
        } else {
            tools.read_only()
        };
        if account_runtime_worker_target(&spec.model).is_some() {
            let state = state.clone();
            let run_context = run_context.clone();
            let factory: crate::threads::ChildStreamFactory =
                Arc::new(move |spec| account_worker_agent_stream(&state, &run_context, spec));
            workers.push(supervisor.spawn_stream(factory, spec)?);
        } else {
            workers.push(supervisor.spawn(service.clone(), worker_tools, spec)?);
        }
    }
    Ok((running, workers))
}

async fn create_worker_worktree(folder: Option<PathBuf>) -> Option<PathBuf> {
    let folder = folder?;
    tokio::task::spawn_blocking(move || {
        let status = workspace_git_status_blocking(Some(folder));
        if !status.is_repo {
            return None;
        }
        let root = PathBuf::from(status.root.as_deref()?);
        let checkpoint =
            workspace_git_checkpoint_action(&root, &status, Some("worker-run-base".to_string()))
                .checkpoint?;
        let worktree_root = milim_core::paths::Paths::resolve()
            .root()
            .join("runtime")
            .join("hot-swap");
        let created =
            workspace_git_create_retry_worktree_action(&root, Some(checkpoint), &worktree_root);
        created
            .ok
            .then_some(created.worktree)
            .flatten()
            .map(PathBuf::from)
    })
    .await
    .ok()
    .flatten()
}

pub(crate) fn schedule_worker_run_deadline(supervisor: Arc<ThreadSupervisor>, run_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(MAX_CHILD_THREAD_WAIT_MS)).await;
        let still_running = supervisor
            .worker_run(&run_id)
            .ok()
            .flatten()
            .is_some_and(|run| run.status == milim_agents::WorkerRunStatus::Running);
        if still_running {
            let _ = supervisor.stop_run(&run_id, "worker run exceeded the five-minute deadline");
        }
    });
}

#[async_trait]
impl Tool for DelegateWorkersTool {
    fn name(&self) -> &str {
        "delegate_workers"
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn description(&self) -> &str {
        "Delegate 1 to 4 genuinely independent tasks as one Worker Run. Do not delegate short or sequential work. Ask mode proposes a frozen plan unless tool approval is Open; Open and Auto start eligible workers immediately."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object", "properties": { "tasks": { "type": "array", "minItems": 1, "maxItems": 4,
                "items": { "type": "object", "properties": {
                    "prompt": {"type":"string"}, "title":{"type":"string"}, "role":{"type":"string"},
                    "agent_id":{"type":["string","null"]}, "model":{"type":"string"},
                    "access":{"type":"string","enum":["read_only","write_review"]}
                }, "required":["prompt"], "additionalProperties":false }
            } }, "required":["tasks"], "additionalProperties":false
        })
    }
    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: DelegateWorkersArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid delegate_workers arguments: {error}"))
        })?;
        let parent_id = child_thread_parent_id(&self.context)?;
        let (mut tasks, _) = resolve_worker_plan(
            &self.state,
            &self.run_context,
            &self.context.model,
            self.context.worker_model.as_deref(),
            args.tasks,
        )
        .await?;
        let delegation_policy = if self.auto_approve_workers {
            milim_agents::DelegationPolicy::Auto
        } else {
            self.context.delegation_policy
        };
        if !self.allow_write_review
            || (!self.auto_approve_workers
                && delegation_policy != milim_agents::DelegationPolicy::Ask)
        {
            for task in &mut tasks {
                task.access = milim_agents::WorkerAccess::ReadOnly;
            }
        }
        let worker_context = managed_worker_context(
            self.run_context.workspace.as_deref(),
            self.context.worker_context.as_deref(),
        );
        let run = self.supervisor.store().create_worker_run_with_origin(
            &parent_id,
            self.context.message_id.as_deref(),
            delegation_policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            worker_context.as_deref(),
            self.run_context.workspace_text().as_deref(),
            self.run_context.privacy_mode.as_str(),
        )?;
        if delegation_policy == milim_agents::DelegationPolicy::Ask {
            return Ok(
                json!({ "ok": true, "run": run, "workers": [], "worker_run_notice": worker_run_notice(&run, &[]) }),
            );
        }
        let (mut run, _) = start_managed_worker_run(
            &self.state,
            &self.supervisor,
            &run,
            self.child_tools.clone(),
        )
        .await?;
        run = self
            .supervisor
            .wait_run(&run.id, MAX_CHILD_THREAD_WAIT_MS)
            .await?
            .unwrap_or(run);
        if run.status == milim_agents::WorkerRunStatus::Running {
            run = self
                .supervisor
                .stop_run(&run.id, "worker run exceeded the five-minute deadline")?
                .unwrap_or(run);
        }
        let workers = self.supervisor.workers_for_run(&run.id)?;
        Ok(
            json!({ "ok": true, "run": run, "workers": workers, "worker_run_notice": worker_run_notice(&run, &workers) }),
        )
    }
}

pub(crate) fn register_schedule_tools(
    reg: &mut ToolRegistry,
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<PathBuf>,
    privacy: &str,
) {
    reg.register(Arc::new(ScheduleCreateTool {
        store: store.clone(),
        workspace: workspace.map(|path| path.to_string_lossy().to_string()),
        privacy: privacy.to_string(),
    }));
    reg.register(Arc::new(ScheduleUpdateTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleListTool {
        store: store.clone(),
    }));
    reg.register(Arc::new(ScheduleDeleteTool { store }));
}

struct ScheduleCreateTool {
    store: Arc<milim_automation::ScheduleStore>,
    workspace: Option<String>,
    privacy: String,
}

struct ScheduleUpdateTool {
    store: Arc<milim_automation::ScheduleStore>,
}

struct ScheduleListTool {
    store: Arc<milim_automation::ScheduleStore>,
}

struct ScheduleDeleteTool {
    store: Arc<milim_automation::ScheduleStore>,
}

#[derive(Debug, Deserialize)]
struct ScheduleCreateToolArgs {
    name: String,
    cron: String,
    prompt: String,
    #[serde(default)]
    attachments: Vec<milim_automation::ScheduleAttachment>,
    #[serde(default)]
    agent_id: Option<String>,
    model: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ScheduleUpdateToolArgs {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cron: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    attachments: Option<Vec<milim_automation::ScheduleAttachment>>,
    #[serde(default)]
    agent_id: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ScheduleListToolArgs {
    #[serde(default)]
    enabled_only: bool,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ScheduleDeleteToolArgs {
    id: String,
}

pub(crate) fn trim_required_tool_arg(value: String, name: &str) -> milim_core::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(Error::InvalidRequest(format!("{name} is required")));
    }
    Ok(value)
}

fn trim_optional_agent_id(agent_id: Option<String>) -> Option<String> {
    agent_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn find_schedule(
    store: &milim_automation::ScheduleStore,
    id: &str,
) -> milim_core::Result<milim_automation::Schedule> {
    store
        .list()?
        .into_iter()
        .find(|schedule| schedule.id == id)
        .ok_or_else(|| Error::ModelNotFound(format!("schedule {id}")))
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    fn name(&self) -> &str {
        "schedule_create"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Create a cron automation that runs a saved agent prompt. Use this when the user asks to schedule, automate, run periodically, or create a cron from chat. Cron expressions must use six fields: sec min hour day month dow."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short human-readable automation name." },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string", "description": "Self-contained prompt to run each time the automation fires." },
                "attachments": {
                    "type": "array",
                    "description": "Optional file attachments whose text content should be included when the automation runs.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Optional named agent id. Omit for the default agent." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean", "description": "Whether the automation should start enabled. Defaults to true." }
            },
            "required": ["name", "cron", "prompt", "model"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleCreateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_create arguments: {e}"))
        })?;
        let name = trim_required_tool_arg(args.name, "name")?;
        let cron = trim_required_tool_arg(args.cron, "cron")?;
        let prompt = trim_required_tool_arg(args.prompt, "prompt")?;
        let model = provider_schedule_model(trim_required_tool_arg(args.model, "model")?)?;
        let schedule = self.store.create_with_run_context(
            &name,
            &cron,
            trim_optional_agent_id(args.agent_id),
            &model,
            &prompt,
            args.attachments,
            args.enabled,
            self.workspace.clone(),
            &self.privacy,
            "local",
        )?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleUpdateTool {
    fn name(&self) -> &str {
        "schedule_update"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Update an existing cron automation by id. Use null agent_id to clear the named agent and omit fields that should stay unchanged."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to update." },
                "name": { "type": "string" },
                "cron": { "type": "string", "description": "Six-field cron expression: sec min hour day month dow." },
                "prompt": { "type": "string" },
                "attachments": {
                    "type": "array",
                    "description": "Replacement file attachments for the automation.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "name": { "type": "string" },
                            "mime": { "type": "string" },
                            "size": { "type": "integer" },
                            "content": { "type": "string" },
                            "dataUrl": { "type": "string" },
                            "truncated": { "type": "boolean" },
                            "sourcePath": { "type": "string" }
                        },
                        "required": ["name"],
                        "additionalProperties": false
                    }
                },
                "agent_id": { "type": ["string", "null"], "description": "Named agent id, or null to clear." },
                "model": { "type": "string", "description": "Model id for unattended runs." },
                "enabled": { "type": "boolean" }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleUpdateToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_update arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let current = find_schedule(&self.store, &id)?;
        let name = args
            .name
            .map(|value| trim_required_tool_arg(value, "name"))
            .transpose()?
            .unwrap_or_else(|| current.name.clone());
        let cron = args
            .cron
            .map(|value| trim_required_tool_arg(value, "cron"))
            .transpose()?
            .unwrap_or_else(|| current.cron.clone());
        let prompt = args
            .prompt
            .map(|value| trim_required_tool_arg(value, "prompt"))
            .transpose()?
            .unwrap_or_else(|| current.prompt.clone());
        let model = args
            .model
            .map(|value| trim_required_tool_arg(value, "model"))
            .transpose()?
            .unwrap_or_else(|| current.model.clone());
        let model = provider_schedule_model(model)?;
        let attachments = args
            .attachments
            .unwrap_or_else(|| current.attachments.clone());
        let agent_id = match args.agent_id {
            None => current.agent_id.clone(),
            Some(Value::Null) => None,
            Some(Value::String(value)) => trim_optional_agent_id(Some(value)),
            Some(_) => {
                return Err(Error::InvalidRequest(
                    "agent_id must be a string or null".to_string(),
                ))
            }
        };
        let schedule = self.store.update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &name,
            cron: &cron,
            agent_id,
            model: &model,
            prompt: &prompt,
            attachments,
            enabled: args.enabled.unwrap_or(current.enabled),
            workspace: current.workspace,
            privacy: current.privacy,
            timezone_mode: current.timezone_mode,
            created_unix: current.created_unix,
            last_run: current.last_run,
        })?;
        Ok(json!({ "ok": true, "schedule": schedule }))
    }
}

#[async_trait]
impl Tool for ScheduleListTool {
    fn name(&self) -> &str {
        "schedule_list"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn description(&self) -> &str {
        "List saved cron automations."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "enabled_only": { "type": "boolean", "description": "Only return enabled schedules." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
            },
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleListToolArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid schedule_list arguments: {error}"))
        })?;
        let mut schedules = self.store.list()?;
        if args.enabled_only {
            schedules.retain(|schedule| schedule.enabled);
        }
        if let Some(limit) = args.limit {
            schedules.truncate(limit.clamp(1, 50));
        }
        Ok(json!({ "ok": true, "schedules": schedules }))
    }
}

#[async_trait]
impl Tool for ScheduleDeleteTool {
    fn name(&self) -> &str {
        "schedule_delete"
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    fn description(&self) -> &str {
        "Delete a saved cron automation by id."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Schedule id to delete." }
            },
            "required": ["id"],
            "additionalProperties": false
        })
    }

    async fn invoke(&self, args: Value) -> milim_core::Result<Value> {
        let args: ScheduleDeleteToolArgs = serde_json::from_value(args).map_err(|e| {
            Error::InvalidRequest(format!("invalid schedule_delete arguments: {e}"))
        })?;
        let id = trim_required_tool_arg(args.id, "id")?;
        let deleted = self.store.delete(&id)?;
        if !deleted {
            return Err(Error::ModelNotFound(format!("schedule {id}")));
        }
        Ok(json!({ "ok": true, "deleted": true, "id": id }))
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

    let run_context = RunContext::from_request(&st, &req).map_err(ApiError)?;
    let service = service_for_run(&st, &run_context);
    let model = req.model.clone();
    let want_stream = req.wants_stream();
    let reasoning_effort = req.reasoning_effort;
    let mut agent_config = agent_run_config_from_request(&req);
    let tool_policy = tool_run_policy_from_request(&req);
    if want_stream
        && tool_policy.approval == ToolApprovalPolicy::Review
        && tool_policy.interactive_approval
        && !tool_policy.approval_granted
    {
        agent_config.approval_broker = Some(st.tool_approvals.clone());
    }
    let memory = memory_context_from_request(&req, model.clone());
    let skill_mode = string_extra(&req, "skill_mode").unwrap_or_else(|| "auto".to_string());
    let enabled_skills = string_list_extra(&req, "enabled_skills");
    let workspace_unavailable =
        desktop_workspace_unavailable_for(&st, run_context.workspace.as_deref());
    let mut messages = req.messages;
    add_workspace_instructions_for(&mut messages, run_context.workspace.as_deref());
    add_workspace_notice_if_needed(&mut messages, workspace_unavailable);

    if want_stream {
        let mut registry =
            agent_registry_with_memory(&st, Some(memory), &tool_policy, &run_context);
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &skill_mode, &enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            service,
            tools,
            model,
            messages,
            reasoning_effort,
            agent_config,
        );
        return Ok(Sse::new(agent_sse(stream))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let mut tools = agent_registry_with_memory(&st, Some(memory), &tool_policy, &run_context);
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &skill_mode, &enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        service.as_ref(),
        &tools,
        &model,
        messages,
        reasoning_effort,
        agent_config,
    )
    .await
    .map_err(ApiError)?;

    Ok(Json(AgentRunResponse {
        id: gen_id("agentrun"),
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn control_agent_stream(
    st: &AppState,
    agent: &milim_agents::AgentDef,
    model: &str,
    mut messages: Vec<ChatMessage>,
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
    reasoning_effort: Option<ReasoningEffort>,
    sampling: SamplingParams,
    step_hook: Arc<dyn milim_agents::AgentStepHook>,
) -> milim_core::Result<ControlAgentStream> {
    let run_context = RunContext::from_control(st, workspace, privacy)?;
    let service = service_for_run(st, &run_context);
    let approval = match approval_mode {
        "review" => ToolApprovalPolicy::Review,
        "open" => ToolApprovalPolicy::Open,
        _ => ToolApprovalPolicy::Guarded,
    };
    let tool_policy = ToolRunPolicy {
        approval,
        approval_granted: false,
        interactive_approval: approval == ToolApprovalPolicy::Review,
        sandbox_enabled: sandbox,
        computer_use_enabled: computer_use,
        preview_tools_enabled: false,
        experimental_hashline_patch: false,
        plan_mode,
    };
    let mut agent_config = milim_agents::AgentRunConfig::default();
    if tool_policy.interactive_approval {
        agent_config.approval_broker = Some(st.tool_approvals.clone());
    }
    agent_config.step_hook = Some(step_hook);
    agent_config.sampling = sampling;
    let query = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(ChatMessage::text_content)
        .unwrap_or_default();
    let mut prefixed = Vec::new();
    if !agent.system_prompt.trim().is_empty() {
        prefixed.push(ChatMessage::text("system", agent.system_prompt.clone()));
    }
    prefixed.extend(crate::agent_skill_messages(st, agent, &query));
    prefixed.append(&mut messages);
    add_workspace_instructions_for(&mut prefixed, run_context.workspace());
    add_workspace_notice_if_needed(
        &mut prefixed,
        desktop_workspace_unavailable_for(st, run_context.workspace()),
    );
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
        worker_context: Some(query),
        linked_thread_grants,
    };
    let mut registry = agent_registry_for_mode_with_context(
        st,
        &agent.tool_mode,
        &agent.enabled_tools,
        Some(memory),
        &tool_policy,
        &run_context,
    );
    if !plan_mode {
        register_skill_tools(&mut registry, st, &agent.skill_mode, &agent.enabled_skills);
    }
    Ok(Box::pin(milim_agents::run_agent_stream_with_config(
        service,
        Arc::new(registry),
        model.to_string(),
        prefixed,
        reasoning_effort,
        agent_config,
    )))
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
    let run_context = RunContext::from_request(&st, &req).map_err(ApiError)?;
    let service = service_for_run(&st, &run_context);
    let store = agents_store(&st)?;
    let agent = store
        .get(&id)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("agent {id}"))))?;

    let want_stream = req.wants_stream();
    let requested_model = req.model.clone();
    let reasoning_effort = req.reasoning_effort;
    let mut agent_config = agent_run_config_from_request(&req);
    let tool_policy = tool_run_policy_from_request(&req);
    if want_stream
        && tool_policy.approval == ToolApprovalPolicy::Review
        && tool_policy.interactive_approval
        && !tool_policy.approval_granted
    {
        agent_config.approval_broker = Some(st.tool_approvals.clone());
    }
    let mut memory = memory_context_from_request(&req, requested_model.clone());
    let mut messages = Vec::new();
    if !agent.system_prompt.is_empty() {
        messages.push(ChatMessage::text("system", agent.system_prompt.clone()));
    }
    let skill_query = req
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(ChatMessage::text_content)
        .unwrap_or_default();
    if !bool_extra(&req, "skills_resolved") {
        messages.extend(crate::agent_skill_messages(&st, &agent, &skill_query));
    }
    let resolved_role = messages
        .iter()
        .filter(|message| message.role == "system")
        .map(ChatMessage::text_content)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !resolved_role.is_empty() {
        memory.worker_context = Some(
            [
                memory.worker_context.as_deref(),
                Some(resolved_role.as_str()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n"),
        );
    }
    messages.extend(req.messages);
    add_workspace_instructions_for(&mut messages, run_context.workspace.as_deref());
    let model = requested_model;
    let memory = AgentMemoryContext {
        model: model.clone(),
        ..memory
    };
    add_workspace_notice_if_needed(
        &mut messages,
        desktop_workspace_unavailable_for(&st, run_context.workspace.as_deref()),
    );

    if want_stream {
        let mut registry = agent_registry_for_mode_with_context(
            &st,
            &agent.tool_mode,
            &agent.enabled_tools,
            Some(memory),
            &tool_policy,
            &run_context,
        );
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &agent.skill_mode, &agent.enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            service,
            tools,
            model,
            messages,
            reasoning_effort,
            agent_config,
        );
        return Ok(Sse::new(agent_sse(stream))
            .keep_alive(KeepAlive::default())
            .into_response());
    }

    let mut tools = agent_registry_for_mode_with_context(
        &st,
        &agent.tool_mode,
        &agent.enabled_tools,
        Some(memory),
        &tool_policy,
        &run_context,
    );
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &agent.skill_mode, &agent.enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        service.as_ref(),
        &tools,
        &model,
        messages,
        reasoning_effort,
        agent_config,
    )
    .await
    .map_err(ApiError)?;

    Ok(Json(AgentRunResponse {
        id: gen_id("agentrun"),
        object: "agent.run",
        model,
        message: outcome.message,
        steps: outcome.steps,
        iterations: outcome.iterations,
        stopped_at_limit: outcome.stopped_at_limit,
    })
    .into_response())
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
