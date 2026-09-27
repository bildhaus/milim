//! Delegated Worker Runs: planning, `delegate_workers`, and starting native
//! and account-runtime Workers.

use super::*;

pub(super) fn child_thread_parent_id(context: &AgentMemoryContext) -> milim_core::Result<String> {
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

pub(super) fn child_thread_title(title: Option<String>, prompt: &str) -> String {
    title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| prompt.chars().take(80).collect())
}

pub(super) fn worker_run_event_name(status: milim_agents::WorkerRunStatus) -> &'static str {
    match status {
        milim_agents::WorkerRunStatus::Proposed => "proposed",
        milim_agents::WorkerRunStatus::Running => "started",
        milim_agents::WorkerRunStatus::Done | milim_agents::WorkerRunStatus::Partial => "done",
        milim_agents::WorkerRunStatus::Stopped | milim_agents::WorkerRunStatus::Error => "error",
    }
}

pub(super) fn worker_run_notice(
    run: &milim_agents::WorkerRun,
    workers: &[milim_agents::Worker],
) -> Value {
    json!({ "event": worker_run_event_name(run.status), "run": run, "workers": workers, "message": run.error })
}

pub(super) struct DelegateWorkersTool {
    pub(super) state: AppState,
    pub(super) supervisor: Arc<ThreadSupervisor>,
    pub(super) context: AgentMemoryContext,
    pub(super) child_tools: ToolRegistry,
    pub(super) allow_write_review: bool,
    pub(super) auto_approve_workers: bool,
    pub(super) run_context: RunContext,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DelegateWorkerTaskArgs {
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) agent_id: Option<String>,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default)]
    pub(super) access: Option<milim_agents::WorkerAccess>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DelegateWorkersArgs {
    tasks: Vec<DelegateWorkerTaskArgs>,
}

pub(super) fn worker_model_is_available(available: &[Model], model: &str) -> bool {
    match crate::providers::provider_model_route(model) {
        Some((provider_id, model_id)) => available.iter().any(|candidate| {
            candidate.id == model_id && candidate.provider_id.as_deref() == Some(&provider_id)
        }),
        None => available.iter().any(|candidate| candidate.id == model),
    }
}

pub(super) fn account_runtime_worker_target(model: &str) -> Option<(&'static str, &str)> {
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

pub(super) fn resolve_account_runtime_worker_model(
    requested: &str,
    preferred_model: &str,
) -> Option<String> {
    let requested = requested.trim();
    if account_runtime_worker_target(requested).is_some() {
        return Some(requested.to_string());
    }
    let (adapter, _) = account_runtime_worker_target(preferred_model)?;
    (!requested.is_empty() && !requested.contains(':')).then(|| format!("{adapter}:{requested}"))
}

pub(super) fn resolve_worker_model(
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

pub(super) async fn resolve_worker_plan(
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

pub(super) fn worker_specs(
    run: &milim_agents::WorkerRun,
    system_prompts: Vec<Option<String>>,
) -> Vec<ChildRunSpec> {
    run.tasks
        .iter()
        .cloned()
        .zip(system_prompts)
        .map(|(task, system_prompt)| {
            let context = run
                .context
                .as_deref()
                .filter(|context| !context.trim().is_empty())
                .map(parent_context_block);
            let role = task
                .role
                .as_deref()
                .map(str::trim)
                .filter(|role| !role.is_empty())
                .map(|role| format!("Your role for this task: {role}"));
            let system_prompt = [
                context.as_deref(),
                system_prompt.as_deref(),
                role.as_deref(),
            ]
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
                account_profile_id: None,
                base_prompt: None,
                environment: None,
            }
        })
        .collect()
}

/// Frame the delegating chat's request as background. Without the framing a
/// Worker reads the parent's goal ("implement X") as its own assignment and
/// ignores the narrower delegated task in its user message.
pub(super) fn parent_context_block(context: &str) -> String {
    format!(
        "<parent_context>\nBackground from the chat that delegated this task. Use it to \
         understand the goal, but do not carry out the parent's request yourself: your \
         assignment is only the delegated task in the user message.\n\n{}\n</parent_context>",
        context.trim()
    )
}

/// Give a native Worker what every native run starts with (see
/// [`build_native_run`]): the base prompt built from the Worker's own tools
/// and the environment of the folder it works in (its review worktree when it
/// has one). Returns the user's hooks for that folder. The Worker's
/// instructions travel in the Worker Run's context.
pub(super) fn add_native_worker_context(
    spec: &mut ChildRunSpec,
    tools: &ToolRegistry,
    workspace: Option<&FsPath>,
) -> Option<Arc<dyn milim_agents::ToolInterceptor>> {
    if !tools.is_empty() {
        spec.base_prompt = Some(crate::agent_prompt::base_system_prompt(tools, false));
        let environment = crate::workspace_context::RunEnvironment::capture(workspace, &spec.model);
        spec.environment = Some(format!(
            "{}\n\n{}",
            environment.render_stable(),
            turn_environment(&environment)
        ));
    }
    crate::user_hooks::interceptor(
        workspace,
        spec.run_id.as_deref().unwrap_or(&spec.parent_id),
        Some(&spec.parent_id),
    )
}

pub(super) fn account_worker_harness_request(
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
    let instructions = instructions.join("\n\n");
    let (prompt, developer_instructions) = if adapter == "codex" {
        (spec.prompt.clone(), Some(instructions))
    } else {
        (
            format!("System instructions:\n{instructions}\n\n{}", spec.prompt),
            None,
        )
    };
    Ok((
        adapter.to_string(),
        HarnessRunRequest {
            prompt,
            developer_instructions,
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
            account_profile_id: spec.account_profile_id.clone(),
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

pub(super) fn account_worker_agent_stream(
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
    Ok(Box::pin(account_worker_events(adapter, stream)))
}

pub(super) fn account_worker_events(
    harness_id: String,
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
        } else {
            // Dropping the stream kills the runtime; let it exit cleanly (for
            // Claude, releasing its session lock) within the shared bound.
            crate::routes::drain_harness_after_terminal(&harness_id, stream);
        }
    }
}

pub(super) fn managed_worker_context(
    workspace: Option<&FsPath>,
    base: Option<&str>,
) -> Option<String> {
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
        if let Some((adapter, _)) = account_runtime_worker_target(&spec.model) {
            spec.account_profile_id = state
                .control
                .as_ref()
                .map(|control| control.thread_account_profile(&spec.parent_id, adapter));
        }
        let worktree = if spec.access == milim_agents::WorkerAccess::WriteReview {
            let worktree = create_worker_worktree(run_context.workspace.clone()).await;
            match &worktree {
                Some(path) => spec.worktree_path = Some(path.to_string_lossy().to_string()),
                None => spec.access = milim_agents::WorkerAccess::ReadOnly,
            }
            worktree
        } else {
            None
        };
        let worker_tools = worker_tools(&tools, worktree.as_deref());
        if account_runtime_worker_target(&spec.model).is_some() {
            let state = state.clone();
            let run_context = run_context.clone();
            let factory: crate::threads::ChildStreamFactory =
                Arc::new(move |spec| account_worker_agent_stream(&state, &run_context, spec));
            workers.push(supervisor.spawn_stream(factory, spec)?);
        } else {
            let workspace = spec
                .worktree_path
                .as_deref()
                .map(PathBuf::from)
                .or_else(|| run_context.workspace.clone());
            let hooks = add_native_worker_context(&mut spec, &worker_tools, workspace.as_deref());
            workers.push(supervisor.spawn_with_hooks(
                service.clone(),
                worker_tools,
                hooks,
                spec,
            )?);
        }
    }
    Ok((running, workers))
}

/// One Worker's tools: bound to its review worktree when it has one and
/// read-only otherwise. Each Worker gets its own run state (checklist,
/// file-freshness records, shell working directory, background processes,
/// and tool queue) instead of sharing the delegating run's.
pub(super) fn worker_tools(tools: &ToolRegistry, worktree: Option<&FsPath>) -> ToolRegistry {
    match worktree {
        Some(path) => tools.scoped_to_workspace(path),
        None => tools.read_only(),
    }
    .scoped_for_run()
}

pub(super) async fn create_worker_worktree(folder: Option<PathBuf>) -> Option<PathBuf> {
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

/// Stop a Worker Run still running [`WORKER_RUN_TIMEOUT`] after its Workers
/// started. Scheduled when the Workers start, so it also holds when whoever
/// started them stops waiting.
pub(crate) fn schedule_worker_run_deadline(supervisor: Arc<ThreadSupervisor>, run_id: String) {
    tokio::spawn(async move {
        tokio::time::sleep(WORKER_RUN_TIMEOUT).await;
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
    fn deadline_for_call(&self, _args: &Value) -> Option<Duration> {
        Some(WORKER_RUN_TIMEOUT + WORKER_SETUP_ALLOWANCE + TOOL_WAIT_GRACE)
    }
    fn waits_on_other_runs(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "Run 1 to 4 substantial, independent tasks in parallel as Workers and return their final reports. Workers start fresh: they don't see this conversation or anything you have read, so write each task's prompt as a self-contained brief with the goal, the relevant files, any constraints, and what to report back. Do short or sequential work yourself. Workers are stopped after five minutes of execution. When this chat's delegation setting is Ask and tool approval is not Open, the plan is proposed to the user to start instead, and the call returns without reports."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 4,
                    "items": {
                        "type": "object",
                        "properties": {
                            "prompt": { "type": "string", "description": "Self-contained brief: the goal, relevant files and context, constraints, and what to report back." },
                            "title": { "type": "string", "description": "Short label for the Worker. Defaults to the start of the prompt." },
                            "role": { "type": "string", "description": "Expertise the Worker should bring, such as \"security reviewer\"." },
                            "agent_id": { "type": ["string", "null"], "description": "Run the task as this saved Milim Agent, with its instructions (ids from list_agents)." },
                            "model": { "type": "string", "description": "Model for this Worker. Defaults to the chat's Worker model, or its own model." },
                            "access": {
                                "type": "string",
                                "enum": ["read_only", "write_review"],
                                "description": "read_only (default) investigates and reports. write_review edits in a separate git worktree whose changes the user reviews and applies; when tool approval does not allow edits, the task runs read_only."
                            }
                        },
                        "required": ["prompt"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["tasks"],
            "additionalProperties": false
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
        schedule_worker_run_deadline(self.supervisor.clone(), run.id.clone());
        run = self
            .supervisor
            .wait_run(&run.id, WORKER_RUN_TIMEOUT.as_millis() as u64)
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
            account_profile_id: Some("work".to_string()),
            base_prompt: None,
            environment: None,
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

        let mut codex_spec = spec;
        codex_spec.model = "codex:gpt-5.6".to_string();
        let (adapter, request) = account_worker_harness_request(
            &codex_spec,
            &RunContext {
                workspace: None,
                privacy_mode: crate::privacy::PrivacyMode::Off,
            },
        )
        .unwrap();
        assert_eq!(adapter, "codex");
        assert_eq!(request.prompt, "Find the bug.");
        assert!(request
            .developer_instructions
            .as_deref()
            .is_some_and(|instructions| instructions.contains("Do not delegate more work")));
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
        let events = account_worker_events("codex".to_string(), stream)
            .collect::<Vec<_>>()
            .await;

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
