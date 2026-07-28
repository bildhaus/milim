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
        model: model.to_string(),
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
    };
    let mut registry = agent_registry_for_mode(
        st,
        &context.tool_mode,
        &context.enabled_tools,
        Some(memory),
        &policy,
    )
    .without(DESKTOP_WORKSPACE_TOOL_NAMES);
    if !policy.plan_mode {
        register_skill_tools(
            &mut registry,
            st,
            &context.skill_mode,
            &context.enabled_skills,
        );
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
    run_id: Option<String>,
}

impl Drop for AccountRuntimeToolLease {
    fn drop(&mut self) {
        if let Some(run_id) = &self.run_id {
            self.sessions
                .lock()
                .expect("account runtime tool store poisoned")
                .remove(run_id);
        }
    }
}

pub(crate) fn account_runtime_stream<S>(
    stream: S,
    st: &AppState,
    endpoint: Option<&AccountRuntimeToolEndpoint>,
    relay_notices: bool,
) -> impl futures::Stream<Item = std::result::Result<Event, Infallible>>
where
    S: futures::Stream<Item = std::result::Result<Event, Infallible>>,
{
    let run_id = endpoint.map(|endpoint| endpoint.run_id.clone());
    let sessions = st.account_runtime_tools.clone();
    let mut notices = st.tool_approvals.subscribe();
    async_stream::stream! {
        let _lease = AccountRuntimeToolLease { sessions, run_id: run_id.clone() };
        futures::pin_mut!(stream);
        loop {
            tokio::select! {
                event = stream.next() => match event {
                    Some(event) => yield event,
                    None => break,
                },
                notice = notices.recv(), if relay_notices && run_id.is_some() => match notice {
                    Ok(notice) if Some(notice.run_id.as_str()) == run_id.as_deref() => {
                        let value = if let Some(decision) = notice.decision {
                            json!({
                                "type": "tool_approval_resolved",
                                "approval_id": notice.approval_id,
                                "call_id": notice.call_id.unwrap_or_else(|| notice.approval_id.clone()),
                                "decision": decision,
                            })
                        } else {
                            json!({
                                "type": "tool_approval_required",
                                "approval_id": notice.approval_id,
                                "call_id": notice.call_id.unwrap_or_else(|| notice.approval_id.clone()),
                                "name": notice.name,
                                "arguments": notice.arguments,
                                "effect": notice.effect,
                            })
                        };
                        yield Ok(Event::default().data(value.to_string()));
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
const PREVIEW_TOOL_NAMES: &[&str] = &[
    "preview_dom_snapshot",
    "preview_click",
    "preview_type_text",
    "preview_key_press",
    "preview_scroll",
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
    "http_fetch",
    "current_time",
    "echo",
];
const PLAN_MODE_READ_ONLY_TOOL_NAMES: &[&str] = &["read_file", "list_dir"];
const MAX_CHILD_THREAD_WAIT_MS: u64 = 300_000;
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

fn workspace_is_selected(st: &AppState) -> bool {
    workspace_snapshot(st).is_some()
}

pub(crate) fn workspace_snapshot(st: &AppState) -> Option<PathBuf> {
    st.workspace.read().ok().and_then(|guard| guard.clone())
}

pub(crate) fn static_registry_for_run(st: &AppState) -> ToolRegistry {
    let reg = st.tools.as_deref().cloned().unwrap_or_default();
    workspace_snapshot(st)
        .map(|root| reg.scoped_to_workspace(&root))
        .unwrap_or(reg)
        .scoped_for_run()
}

fn registry_has_desktop_host_tools(reg: &ToolRegistry) -> bool {
    reg.contains("edit_file") || reg.contains("patch_file") || reg.contains("shell")
}

fn desktop_workspace_unavailable(st: &AppState) -> bool {
    !workspace_is_selected(st)
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
    let folder = workspace_snapshot(st);
    let context = crate::workspace_context::resolve(folder.as_deref());
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
) -> ToolRegistry {
    agent_registry_for_mode(st, "all", &[], memory, policy)
}

fn agent_base_registry_with_memory(
    st: &AppState,
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
) -> ToolRegistry {
    let mut reg = static_registry_for_run(st);
    let workspace_unavailable = desktop_workspace_unavailable(st);
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
        register_schedule_tools(&mut reg, store.clone(), workspace_snapshot(st));
    }
    if let (Some(memory), Some(store)) = (memory.clone(), st.memory.as_ref()) {
        if memory.enabled {
            reg.register(Arc::new(MemoryRegisterTool {
                store: store.clone(),
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
        reg = reg.without(PREVIEW_TOOL_NAMES);
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
    match st
        .tool_approvals
        .resolve_with_response(&id, approved, req.response)
    {
        milim_agents::ApprovalResolve::Resolved => Ok(StatusCode::NO_CONTENT.into_response()),
        milim_agents::ApprovalResolve::AlreadyResolved => Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "tool approval already resolved" })),
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

fn agent_registry_for_mode(
    st: &AppState,
    tool_mode: &str,
    enabled_tools: &[String],
    memory: Option<AgentMemoryContext>,
    policy: &ToolRunPolicy,
) -> ToolRegistry {
    let all = agent_base_registry_with_memory(st, memory.clone(), policy);
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
            register_child_thread_tools(
                &mut reg,
                st.clone(),
                supervisor.clone(),
                memory,
                child_registry_for_policy(st, policy, inherited),
                policy.approval == ToolApprovalPolicy::Open
                    || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted),
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

pub(crate) fn scheduled_agent_registry(
    st: &AppState,
    agent: &milim_agents::AgentDef,
) -> ToolRegistry {
    let mut registry = agent_registry_for_mode(
        st,
        &agent.tool_mode,
        &agent.enabled_tools,
        None,
        &ToolRunPolicy::default(),
    );
    register_skill_tools(&mut registry, st, &agent.skill_mode, &agent.enabled_skills);
    registry
}

pub(crate) struct MemoryRegisterTool {
    pub(crate) store: Arc<milim_memory::MemoryStore>,
    pub(crate) context: AgentMemoryContext,
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
    if context.delegation_policy != milim_agents::DelegationPolicy::Off {
        reg.register(Arc::new(DelegateWorkersTool {
            state,
            supervisor,
            context,
            child_tools,
            allow_write_review,
        }));
    }
}

fn child_read_only_registry(st: &AppState) -> ToolRegistry {
    let allowed: Vec<String> = CHILD_THREAD_READ_ONLY_TOOL_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    let mut reg = static_registry_for_run(st).filtered(&allowed);
    if desktop_workspace_unavailable(st) {
        reg = reg.without(&["read_file", "list_dir"]);
    }
    reg
}

fn worker_review_registry(st: &AppState) -> ToolRegistry {
    static_registry_for_run(st)
        .without(CHILD_THREAD_TOOL_NAMES)
        .without(SANDBOX_TOOL_NAMES)
        .without(COMPUTER_TOOL_NAMES)
        .without(PREVIEW_TOOL_NAMES)
}

fn child_registry_for_policy(
    st: &AppState,
    policy: &ToolRunPolicy,
    inherited: ToolRegistry,
) -> ToolRegistry {
    if policy.approval == ToolApprovalPolicy::Open
        || (policy.approval == ToolApprovalPolicy::Review && policy.approval_granted)
    {
        inherited.without(CHILD_THREAD_TOOL_NAMES)
    } else {
        child_read_only_registry(st)
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
    parent_model: &str,
    worker_model: Option<&str>,
    tasks: Vec<DelegateWorkerTaskArgs>,
) -> milim_core::Result<(Vec<milim_agents::WorkerPlanTask>, Vec<Option<String>>)> {
    if !(1..=4).contains(&tasks.len()) {
        return Err(Error::InvalidRequest(
            "delegate_workers requires 1 to 4 independent tasks".to_string(),
        ));
    }
    let available = state.service.list_models().await?;
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
        let model = resolve_worker_model(&available, requested_model, preferred_model)?;
        let agent_id = trim_optional_agent_id(task.agent_id);
        let system_prompt = if let Some(agent_id) = agent_id.as_deref() {
            let store = state
                .agents
                .as_ref()
                .ok_or_else(|| Error::InvalidRequest("named agents are not enabled".to_string()))?;
            let agent = store
                .get(agent_id)?
                .ok_or_else(|| Error::ModelNotFound(format!("agent {agent_id}")))?;
            (!agent.system_prompt.trim().is_empty()).then_some(agent.system_prompt)
        } else {
            None
        };
        let title = child_thread_title(task.title, &prompt);
        plan.push(milim_agents::WorkerPlanTask {
            id: uuid::Uuid::new_v4().to_string(),
            title,
            prompt,
            role: task.role,
            agent_id,
            model,
            access: task.access.unwrap_or_default(),
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

fn managed_worker_context(state: &AppState, base: Option<&str>) -> Option<String> {
    let mut sections = Vec::new();
    if let Some(base) = base.filter(|value| !value.trim().is_empty()) {
        sections.push(base.to_string());
    }
    if let Some(workspace) = workspace_snapshot(state) {
        let branch = git_text(&workspace, &["branch", "--show-current"]);
        sections.push(format!(
            "Workspace: {}{}",
            workspace.display(),
            branch
                .filter(|value| !value.is_empty())
                .map(|value| format!("\nBranch: {value}"))
                .unwrap_or_default(),
        ));
        let context = crate::workspace_context::resolve(Some(&workspace));
        if let Some(instructions) = crate::workspace_context::formatted(&context, None) {
            sections.push(instructions);
        }
    }
    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

async fn start_managed_worker_run(
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
    let task_args = run
        .tasks
        .iter()
        .map(|task| DelegateWorkerTaskArgs {
            prompt: task.prompt.clone(),
            title: Some(task.title.clone()),
            role: task.role.clone(),
            agent_id: task.agent_id.clone(),
            model: Some(task.model.clone()),
            access: Some(task.access),
        })
        .collect();
    let (resolved, prompts) = resolve_worker_plan(state, "default", None, task_args).await?;
    if resolved
        .iter()
        .map(|t| (&t.prompt, &t.model, &t.agent_id))
        .ne(run.tasks.iter().map(|t| (&t.prompt, &t.model, &t.agent_id)))
    {
        return Err(Error::InvalidRequest(
            "frozen worker plan no longer resolves exactly".to_string(),
        ));
    }
    let running = supervisor
        .store()
        .update_worker_run_status(&run.id, milim_agents::WorkerRunStatus::Running, None)?
        .ok_or_else(|| Error::ModelNotFound(format!("worker run {}", run.id)))?;
    let mut workers = Vec::with_capacity(running.tasks.len());
    for mut spec in worker_specs(&running, prompts) {
        let worker_tools = if spec.access == milim_agents::WorkerAccess::WriteReview {
            match create_worker_worktree(state).await {
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
        workers.push(supervisor.spawn(state.service.clone(), worker_tools, spec)?);
    }
    Ok((running, workers))
}

async fn create_worker_worktree(state: &AppState) -> Option<PathBuf> {
    let folder = workspace_snapshot(state)?;
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

fn schedule_worker_run_deadline(supervisor: Arc<ThreadSupervisor>, run_id: String) {
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
        "Delegate 1 to 4 genuinely independent tasks as one Worker Run. Do not delegate short or sequential work. Ask mode proposes a frozen plan for approval; Auto runs managed read-only workers and joins their results."
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
            &self.context.model,
            self.context.worker_model.as_deref(),
            args.tasks,
        )
        .await?;
        if self.context.delegation_policy != milim_agents::DelegationPolicy::Ask
            || !self.allow_write_review
        {
            for task in &mut tasks {
                task.access = milim_agents::WorkerAccess::ReadOnly;
            }
        }
        let run = self.supervisor.store().create_worker_run(
            &parent_id,
            self.context.message_id.as_deref(),
            self.context.delegation_policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            managed_worker_context(&self.state, self.context.worker_context.as_deref()).as_deref(),
        )?;
        if self.context.delegation_policy == milim_agents::DelegationPolicy::Ask {
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
) {
    reg.register(Arc::new(ScheduleCreateTool {
        store: store.clone(),
        workspace: workspace.map(|path| path.to_string_lossy().to_string()),
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
        let schedule = self.store.create_with_model_context(
            &name,
            &cron,
            trim_optional_agent_id(args.agent_id),
            &model,
            &prompt,
            args.attachments,
            args.enabled,
            self.workspace.clone(),
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
    let workspace_unavailable = desktop_workspace_unavailable(&st);
    let mut messages = req.messages;
    add_workspace_instructions(&mut messages, &st);
    add_workspace_notice_if_needed(&mut messages, workspace_unavailable);

    if want_stream {
        let mut registry = agent_registry_with_memory(&st, Some(memory), &tool_policy);
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &skill_mode, &enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            st.service.clone(),
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

    let mut tools = agent_registry_with_memory(&st, Some(memory), &tool_policy);
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &skill_mode, &enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        st.service.as_ref(),
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
    add_workspace_instructions(&mut messages, &st);
    let model = requested_model;
    let memory = AgentMemoryContext {
        model: model.clone(),
        ..memory
    };
    add_workspace_notice_if_needed(&mut messages, desktop_workspace_unavailable(&st));

    if want_stream {
        let mut registry = agent_registry_for_mode(
            &st,
            &agent.tool_mode,
            &agent.enabled_tools,
            Some(memory),
            &tool_policy,
        );
        if !tool_policy.plan_mode {
            register_skill_tools(&mut registry, &st, &agent.skill_mode, &agent.enabled_skills);
        }
        let tools = std::sync::Arc::new(registry);
        let stream = milim_agents::run_agent_stream_with_config(
            st.service.clone(),
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

    let mut tools = agent_registry_for_mode(
        &st,
        &agent.tool_mode,
        &agent.enabled_tools,
        Some(memory),
        &tool_policy,
    );
    if !tool_policy.plan_mode {
        register_skill_tools(&mut tools, &st, &agent.skill_mode, &agent.enabled_skills);
    }
    let outcome = milim_agents::run_agent_with_config(
        st.service.as_ref(),
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
        if let Ok(backfill) = supervisor.child_events_after(&id, last_seq, event_limit) {
            for (thread, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
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
                    if let Ok(backfill) = supervisor.child_events_after(&id, last_seq, event_limit) {
                        for (thread, event) in backfill {
                            last_seq = last_seq.max(event.seq);
                            let data = serde_json::to_string(&SupervisorEvent::ChildThreadEvent { thread, event })
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
    let (mut tasks, _) = resolve_worker_plan(&st, &default_model, req.model.as_deref(), req.tasks)
        .await
        .map_err(ApiError)?;
    for task in &mut tasks {
        task.access = milim_agents::WorkerAccess::ReadOnly;
    }
    // Native adapters normalize their own activity into this contract. This endpoint safely falls back to managed workers.
    let _requested_runtime = req.runtime;
    let supervisor = thread_supervisor(&st)?;
    let mut run = supervisor
        .store()
        .create_worker_run(
            &parent_id,
            req.parent_turn_id.as_deref(),
            req.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            None,
        )
        .map_err(ApiError)?;
    let mut workers = Vec::new();
    if req.policy == milim_agents::DelegationPolicy::Auto {
        (run, workers) =
            start_managed_worker_run(&st, &supervisor, &run, child_read_only_registry(&st))
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
    let (run, workers) =
        start_managed_worker_run(&st, &supervisor, &run, worker_review_registry(&st))
            .await
            .map_err(ApiError)?;
    schedule_worker_run_deadline(supervisor, run.id.clone());
    Ok(Json(json!({ "run": run, "workers": workers })).into_response())
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
        .create_worker_run(
            &source.parent_thread_id,
            source.parent_turn_id.as_deref(),
            source.policy,
            milim_agents::WorkerRuntime::Managed,
            tasks,
            source.context.as_deref(),
        )
        .map_err(ApiError)?;
    let (run, workers) =
        start_managed_worker_run(&st, &supervisor, &retry, worker_review_registry(&st))
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
    let worker = owned_worker(&supervisor, &id, &worker_id)?;
    let worktree = worker.worktree_path.clone().ok_or_else(|| {
        ApiError(Error::InvalidRequest(
            "worker has no review worktree".into(),
        ))
    })?;
    let root = workspace_snapshot(&st)
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
        if let Ok(backfill) = supervisor.worker_events_after(&id, last_seq, limit) {
            for (worker, event) in backfill {
                last_seq = last_seq.max(event.seq);
                let data = serde_json::to_string(&json!({"type":"worker_run_worker_event","run_id":id,"worker":worker,"event":event})).unwrap_or_else(|_| "{}".to_string());
                yield Ok::<Event, Infallible>(Event::default().data(data));
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
                    let data = serde_json::to_string(&json!({"type":kind,"run":run,"worker":event.thread(),"event":event})).unwrap_or_else(|_| "{}".to_string());
                    yield Ok::<Event, Infallible>(Event::default().data(data));
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
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
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    last_run: Option<i64>,
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
    Ok(Json(json!({ "schedules": schedules })).into_response())
}

/// `POST /schedules` — create a cron schedule.
/// `GET /schedules/events` - drain completed background schedule runs.
pub(crate) async fn schedule_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    Ok(Json(json!({ "events": st.schedule_runs.take() })).into_response())
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
        .create_with_model_context(
            &req.name,
            &req.cron,
            req.agent_id,
            &model,
            &req.prompt,
            req.attachments,
            true,
            workspace_snapshot(&st).map(|path| path.to_string_lossy().to_string()),
        )
        .map_err(ApiError)?;
    Ok(Json(schedule).into_response())
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
    let schedule = store
        .update(milim_automation::ScheduleUpdate {
            id: &id,
            name: &req.name,
            cron: &req.cron,
            agent_id: req.agent_id,
            model: &model,
            prompt: &req.prompt,
            attachments: req.attachments,
            enabled: req.enabled,
            workspace: current.workspace,
            created_unix: current.created_unix,
            last_run: req.last_run,
        })
        .map_err(ApiError)?;
    Ok(Json(schedule).into_response())
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
