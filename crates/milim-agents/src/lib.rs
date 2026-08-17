//! `milim-agents` - the tool-use agent loop.
//!
//! [`run_agent`] drives the core agentic cycle milim exposes via
//! `POST /agents/{id}/run`: ask the model with the available tools; if it emits
//! tool calls, execute them through the [`ToolRegistry`] and feed the results
//! back as `tool`-role messages; repeat until the model answers in plain text.

mod store;
mod threads;

pub use store::{
    normalize_skill_mode, normalize_tool_mode, AgentDef, AgentStore, AGENT_MIGRATIONS,
};
pub use threads::{
    thread_status_terminal, AgentThread, DelegationPolicy, ThreadEvent, ThreadStore, Worker,
    WorkerAccess, WorkerAgentSnapshot, WorkerPlanTask, WorkerRun, WorkerRunStatus, WorkerRuntime,
    THREAD_MIGRATIONS, THREAD_STATUS_DONE, THREAD_STATUS_ERROR, THREAD_STATUS_QUEUED,
    THREAD_STATUS_RUNNING, THREAD_STATUS_STOPPED,
};

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, ImageUrl, ReasoningEffort, Tool, ToolCall, ToolFunction,
    Usage,
};
use milim_core::{Error, Result};
use milim_inference::{
    CompletionRequest, EventStream, ModelService, SamplingParams, SharedService, StreamEvent,
    ToolCallAccumulator,
};
use milim_tools::{
    ProcessEnvironmentPolicy, ToolEffect, ToolExecutionSpec, ToolRegistry, ToolUiDescriptor,
};

const DEFAULT_AGENT_MAX_ITERATIONS: usize = 100;
const DEFAULT_INITIAL_STREAM_RETRY_BACKOFF_MS: u64 = 250;
const TOOL_REPLAY_MAX_LINES: usize = 2_000;
const TOOL_REPLAY_MAX_BYTES: usize = 50 * 1024;

/// Configuration for one agent loop run.
#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    /// Maximum number of model turns before the loop stops without executing
    /// another round of tool calls.
    pub max_iterations: usize,
    /// Backoff before retrying a failed initial streaming request once.
    pub initial_stream_retry_backoff: Duration,
    /// Interactive approval broker for consequential streamed tool calls.
    pub approval_broker: Option<Arc<ToolApprovalBroker>>,
    /// Durable boundary hook. A failed commit aborts the loop before another
    /// provider request can leave Milim.
    pub step_hook: Option<Arc<dyn AgentStepHook>>,
}

impl Default for AgentRunConfig {
    fn default() -> Self {
        Self {
            max_iterations: DEFAULT_AGENT_MAX_ITERATIONS,
            initial_stream_retry_backoff: Duration::from_millis(
                DEFAULT_INITIAL_STREAM_RETRY_BACKOFF_MS,
            ),
            approval_broker: None,
            step_hook: None,
        }
    }
}

#[async_trait::async_trait]
pub trait AgentStepHook: std::fmt::Debug + Send + Sync {
    async fn commit_tool_catalog(&self, _tools: &[ToolExecutionSpec]) -> Result<()> {
        Ok(())
    }

    async fn prepare_model_step(&self, step: usize, messages: &mut Vec<ChatMessage>) -> Result<()>;

    async fn commit_model_request(&self, step: usize, request: &CompletionRequest) -> Result<()>;

    async fn commit_model_response(
        &self,
        step: usize,
        content: &str,
        reasoning: &str,
        tool_calls: &[ToolCall],
        finish_reason: &str,
        usage: Usage,
    ) -> Result<()>;

    async fn commit_tool_result(
        &self,
        step: usize,
        call_id: Option<&str>,
        name: &str,
        result: &Value,
        model_content: &str,
    ) -> Result<()>;
}

#[derive(Debug)]
pub struct ToolApprovalBroker {
    pending: Mutex<HashMap<String, ApprovalEntry>>,
    external: Mutex<HashMap<String, ExternalApprovalMeta>>,
    notices: tokio::sync::broadcast::Sender<ApprovalNotice>,
}

pub const APPROVAL_DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);
pub const APPROVAL_RUNTIME_ACK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ApprovalEntry {
    sender: Option<tokio::sync::oneshot::Sender<ApprovalDecision>>,
    decision: Option<bool>,
    response_fingerprint: Option<u64>,
    snapshot: ApprovalSnapshot,
    updates: tokio::sync::watch::Sender<ApprovalSnapshot>,
}

#[derive(Debug, Clone)]
struct ExternalApprovalMeta {
    run_id: String,
    call_id: Option<String>,
    name: String,
    arguments: String,
    effect: ToolEffect,
}

#[derive(Debug, Clone)]
pub struct ApprovalNotice {
    pub run_id: String,
    pub approval_id: String,
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: String,
    pub effect: ToolEffect,
    pub state: ApprovalState,
    pub decision: Option<&'static str>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Requested,
    Decided,
    Delivered,
    Acknowledged,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApprovalSnapshot {
    pub approval_id: String,
    pub state: ApprovalState,
    pub decision: Option<&'static str>,
    pub error: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ApprovalResolve {
    Resolved,
    AlreadyResolved,
    Conflict,
    Failed,
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub response: Option<Value>,
}

pub struct PendingApproval {
    pub id: String,
    receiver: tokio::sync::oneshot::Receiver<ApprovalDecision>,
    broker: Weak<ToolApprovalBroker>,
}

impl ToolApprovalBroker {
    pub fn request(self: &Arc<Self>) -> PendingApproval {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let now = approval_now_ms();
        let snapshot = ApprovalSnapshot {
            approval_id: id.clone(),
            state: ApprovalState::Requested,
            decision: None,
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let (updates, _) = tokio::sync::watch::channel(snapshot.clone());
        let mut pending = self.pending.lock().expect("tool approval broker poisoned");
        // ponytail: resolved ids are diagnostic only; discard them once the small cap is reached.
        if pending.len() >= 2048 {
            let removed = pending
                .iter()
                .filter_map(|(id, entry)| {
                    approval_state_terminal(entry.snapshot.state).then_some(id.clone())
                })
                .collect::<Vec<_>>();
            for id in &removed {
                pending.remove(id);
            }
            let mut external = self.external.lock().expect("tool approval broker poisoned");
            for id in removed {
                external.remove(&id);
            }
        }
        pending.insert(
            id.clone(),
            ApprovalEntry {
                sender: Some(sender),
                decision: None,
                response_fingerprint: None,
                snapshot,
                updates,
            },
        );
        PendingApproval {
            id,
            receiver,
            broker: Arc::downgrade(self),
        }
    }

    pub fn resolve(&self, id: &str, approved: bool) -> ApprovalResolve {
        self.resolve_with_response(id, approved, None)
    }

    pub fn resolve_with_response(
        &self,
        id: &str,
        approved: bool,
        response: Option<Value>,
    ) -> ApprovalResolve {
        let response_fingerprint = approval_response_fingerprint(&response);
        let decision = ApprovalDecision { approved, response };
        let (result, snapshot) = {
            let mut pending = self.pending.lock().expect("tool approval broker poisoned");
            let Some(entry) = pending.get_mut(id) else {
                return ApprovalResolve::Missing;
            };
            if entry.snapshot.state != ApprovalState::Requested {
                let result = if entry.decision == Some(approved)
                    && entry.response_fingerprint == Some(response_fingerprint)
                {
                    ApprovalResolve::AlreadyResolved
                } else if matches!(
                    entry.snapshot.state,
                    ApprovalState::Failed | ApprovalState::Canceled
                ) {
                    ApprovalResolve::Failed
                } else {
                    ApprovalResolve::Conflict
                };
                return result;
            }
            let Some(sender) = entry.sender.take() else {
                return ApprovalResolve::Failed;
            };
            entry.decision = Some(approved);
            entry.response_fingerprint = Some(response_fingerprint);
            if sender.send(decision).is_err() {
                transition_entry(
                    entry,
                    ApprovalState::Failed,
                    Some("approval receiver disconnected before delivery".to_string()),
                );
                (ApprovalResolve::Failed, Some(entry.snapshot.clone()))
            } else {
                transition_entry(entry, ApprovalState::Decided, None);
                (ApprovalResolve::Resolved, Some(entry.snapshot.clone()))
            }
        };
        if let Some(snapshot) = snapshot {
            self.publish_notice(id, &snapshot);
        }
        result
    }

    pub fn request_external(
        self: &Arc<Self>,
        run_id: String,
        call_id: Option<String>,
        name: String,
        arguments: String,
        effect: ToolEffect,
    ) -> PendingApproval {
        let pending = self.request();
        let meta = ExternalApprovalMeta {
            run_id: run_id.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
            effect,
        };
        self.external
            .lock()
            .expect("tool approval broker poisoned")
            .insert(pending.id.clone(), meta);
        let _ = self.notices.send(ApprovalNotice {
            run_id,
            approval_id: pending.id.clone(),
            call_id,
            name,
            arguments,
            effect,
            state: ApprovalState::Requested,
            decision: None,
            error: None,
        });
        pending
    }

    pub fn snapshot(&self, id: &str) -> Option<ApprovalSnapshot> {
        self.pending
            .lock()
            .expect("tool approval broker poisoned")
            .get(id)
            .map(|entry| entry.snapshot.clone())
    }

    pub async fn wait_for_delivery(&self, id: &str, timeout: Duration) -> Option<ApprovalSnapshot> {
        let mut updates = self
            .pending
            .lock()
            .expect("tool approval broker poisoned")
            .get(id)
            .map(|entry| entry.updates.subscribe())?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let snapshot = updates.borrow().clone();
            if matches!(
                snapshot.state,
                ApprovalState::Delivered
                    | ApprovalState::Acknowledged
                    | ApprovalState::Failed
                    | ApprovalState::Canceled
            ) {
                return Some(snapshot);
            }
            match tokio::time::timeout_at(deadline, updates.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return self.snapshot(id),
                Err(_) => {
                    self.fail(id, "runtime did not accept the approval decision in time");
                    return self.snapshot(id);
                }
            }
        }
    }

    pub fn mark_delivered(&self, id: &str) -> Option<ApprovalSnapshot> {
        self.transition(id, ApprovalState::Delivered, None)
    }

    pub fn acknowledge(&self, id: &str) -> Option<ApprovalSnapshot> {
        self.transition(id, ApprovalState::Acknowledged, None)
    }

    pub fn fail(&self, id: &str, error: impl Into<String>) -> Option<ApprovalSnapshot> {
        self.transition(id, ApprovalState::Failed, Some(error.into()))
    }

    pub fn acknowledge_run(&self, run_id: &str) {
        let ids = self.external_ids(run_id);
        for id in ids {
            if self
                .snapshot(&id)
                .is_some_and(|snapshot| snapshot.state == ApprovalState::Delivered)
            {
                self.acknowledge(&id);
            }
        }
    }

    pub fn fail_run(&self, run_id: &str, error: &str) {
        for id in self.external_ids(run_id) {
            if self
                .snapshot(&id)
                .is_some_and(|snapshot| !approval_state_terminal(snapshot.state))
            {
                self.fail(&id, error.to_string());
            }
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ApprovalNotice> {
        self.notices.subscribe()
    }

    fn transition(
        &self,
        id: &str,
        state: ApprovalState,
        error: Option<String>,
    ) -> Option<ApprovalSnapshot> {
        let snapshot = {
            let mut pending = self.pending.lock().expect("tool approval broker poisoned");
            let entry = pending.get_mut(id)?;
            let allowed = match state {
                ApprovalState::Delivered => entry.snapshot.state == ApprovalState::Decided,
                ApprovalState::Acknowledged => entry.snapshot.state == ApprovalState::Delivered,
                ApprovalState::Failed | ApprovalState::Canceled => {
                    !approval_state_terminal(entry.snapshot.state)
                }
                ApprovalState::Requested | ApprovalState::Decided => false,
            };
            if !allowed {
                return Some(entry.snapshot.clone());
            }
            if matches!(state, ApprovalState::Failed | ApprovalState::Canceled) {
                entry.sender.take();
            }
            transition_entry(entry, state, error);
            entry.snapshot.clone()
        };
        self.publish_notice(id, &snapshot);
        Some(snapshot)
    }

    fn external_ids(&self, run_id: &str) -> Vec<String> {
        self.external
            .lock()
            .expect("tool approval broker poisoned")
            .iter()
            .filter_map(|(id, meta)| (meta.run_id == run_id).then_some(id.clone()))
            .collect()
    }

    fn publish_notice(&self, id: &str, snapshot: &ApprovalSnapshot) {
        let meta = {
            let mut external = self.external.lock().expect("tool approval broker poisoned");
            if approval_state_terminal(snapshot.state) {
                external.remove(id)
            } else {
                external.get(id).cloned()
            }
        };
        if let Some(meta) = meta {
            let _ = self.notices.send(ApprovalNotice {
                run_id: meta.run_id,
                approval_id: id.to_string(),
                call_id: meta.call_id,
                name: meta.name,
                arguments: meta.arguments,
                effect: meta.effect,
                state: snapshot.state,
                decision: snapshot.decision,
                error: snapshot.error.clone(),
            });
        }
    }
}

impl Default for ToolApprovalBroker {
    fn default() -> Self {
        let (notices, _) = tokio::sync::broadcast::channel(64);
        Self {
            pending: Mutex::new(HashMap::new()),
            external: Mutex::new(HashMap::new()),
            notices,
        }
    }
}

impl PendingApproval {
    pub async fn wait(&mut self) -> ApprovalDecision {
        match (&mut self.receiver).await {
            Ok(decision) => decision,
            Err(_) => {
                if let Some(broker) = self.broker.upgrade() {
                    broker.fail(&self.id, "approval request was canceled");
                }
                ApprovalDecision {
                    approved: false,
                    response: None,
                }
            }
        }
    }

    pub fn mark_delivered(&self) -> Option<ApprovalSnapshot> {
        self.broker.upgrade()?.mark_delivered(&self.id)
    }

    pub fn deliver(&self) -> std::result::Result<ApprovalSnapshot, String> {
        let snapshot = self
            .mark_delivered()
            .ok_or_else(|| "approval transaction expired".to_string())?;
        if matches!(
            snapshot.state,
            ApprovalState::Delivered | ApprovalState::Acknowledged
        ) {
            Ok(snapshot)
        } else {
            Err(snapshot
                .error
                .unwrap_or_else(|| "approval decision is no longer deliverable".to_string()))
        }
    }

    pub fn acknowledge(&self) -> Option<ApprovalSnapshot> {
        self.broker.upgrade()?.acknowledge(&self.id)
    }

    pub fn fail(&self, error: impl Into<String>) -> Option<ApprovalSnapshot> {
        self.broker.upgrade()?.fail(&self.id, error)
    }
}

impl Drop for PendingApproval {
    fn drop(&mut self) {
        let Some(broker) = self.broker.upgrade() else {
            return;
        };
        match broker.snapshot(&self.id).map(|snapshot| snapshot.state) {
            Some(ApprovalState::Requested) => {
                broker.transition(
                    &self.id,
                    ApprovalState::Canceled,
                    Some("approval request was abandoned".to_string()),
                );
            }
            Some(ApprovalState::Decided) => {
                broker.fail(&self.id, "approval delivery was interrupted");
            }
            _ => {}
        }
    }
}

fn transition_entry(entry: &mut ApprovalEntry, state: ApprovalState, error: Option<String>) {
    entry.snapshot.state = state;
    entry.snapshot.decision = entry
        .decision
        .map(|approved| if approved { "approve" } else { "deny" });
    entry.snapshot.error = error;
    entry.snapshot.updated_at_ms = approval_now_ms();
    entry.updates.send_replace(entry.snapshot.clone());
}

fn approval_state_terminal(state: ApprovalState) -> bool {
    matches!(
        state,
        ApprovalState::Acknowledged | ApprovalState::Failed | ApprovalState::Canceled
    )
}

fn approval_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn approval_response_fingerprint(response: &Option<Value>) -> u64 {
    // ponytail: process-local idempotency only; use a durable digest if approvals cross restarts.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    response.as_ref().map(Value::to_string).hash(&mut hasher);
    hasher.finish()
}

impl AgentRunConfig {
    fn max_iterations(&self) -> usize {
        self.max_iterations.max(1)
    }
}

/// One executed tool call within a run.
#[derive(Debug, Clone, Serialize)]
pub struct ToolStep {
    pub name: String,
    pub arguments: String,
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_app: Option<ToolUiDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_app_result: Option<Value>,
}

/// The result of an agent run.
#[derive(Debug, Clone, Serialize)]
pub struct AgentOutcome {
    /// The final assistant message.
    pub message: ChatMessage,
    /// Tool calls executed along the way, in order.
    pub steps: Vec<ToolStep>,
    /// Number of model turns taken.
    pub iterations: usize,
    /// True when the run stopped because it reached the configured iteration limit.
    pub stopped_at_limit: bool,
}

/// Run the tool-use loop until the model answers.
pub async fn run_agent(
    service: &dyn ModelService,
    tools: &ToolRegistry,
    model: &str,
    messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
) -> Result<AgentOutcome> {
    run_agent_with_config(
        service,
        tools,
        model,
        messages,
        reasoning_effort,
        AgentRunConfig::default(),
    )
    .await
}

/// Run the tool-use loop with explicit loop configuration.
pub async fn run_agent_with_config(
    service: &dyn ModelService,
    tools: &ToolRegistry,
    model: &str,
    mut messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
    config: AgentRunConfig,
) -> Result<AgentOutcome> {
    let core_tools = tools_to_core(tools);
    let max_iterations = config.max_iterations();
    let mut steps = Vec::new();

    let mut iteration = 0;
    loop {
        let req = CompletionRequest {
            model: model.to_string(),
            messages: messages.clone(),
            tools: core_tools.clone(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: SamplingParams::default(),
            reasoning_effort,
        };
        let out = service.complete(req).await?;
        iteration += 1;

        let calls = out.message.tool_calls.clone().unwrap_or_default();
        if calls.is_empty() {
            return Ok(AgentOutcome {
                message: out.message,
                steps,
                iterations: iteration,
                stopped_at_limit: false,
            });
        }
        if iteration >= max_iterations {
            return Ok(AgentOutcome {
                message: limit_message(max_iterations),
                steps,
                iterations: iteration,
                stopped_at_limit: true,
            });
        }

        // Record the assistant's tool-call turn, then execute each call.
        messages.push(out.message);
        let mut pending_images: Vec<ChatMessage> = Vec::new();
        for call in calls {
            let executed =
                execute_tool_call(tools, &call.function.name, &call.function.arguments).await;
            let visible = executed.visible;
            steps.push(ToolStep {
                name: call.function.name.clone(),
                arguments: call.function.arguments.clone(),
                result: visible.clone(),
                mcp_app: executed.ui,
                mcp_app_result: executed.app_result,
            });
            messages.push(ChatMessage {
                role: "tool".to_string(),
                content: Some(Content::Text(tool_replay_content(&visible))),
                name: None,
                tool_calls: None,
                tool_call_id: call.id.clone(),
                reasoning_content: None,
            });
            if let Some(uri) = executed.image_uri {
                pending_images.push(image_user_message(&call.function.name, uri));
            }
        }
        // Image results ride in follow-up user messages, pushed after every
        // tool reply (OpenAI requires each tool_call_id answered before any
        // other role appears).
        messages.extend(pending_images);
    }
}

/// A streamed event from [`run_agent_stream`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// The run started; carries the model that will actually run (for a named
    /// agent this is the agent's own model, not the requested one).
    Start { model: String },
    /// A chunk of visible assistant text.
    Token { text: String },
    /// A chunk of non-answer reasoning/thinking text.
    Reasoning { text: String },
    /// Usage for one completed model request inside the agent loop.
    UsageDelta { usage: Usage },
    /// The agent decided to call a tool.
    ToolCall {
        call_id: Option<String>,
        name: String,
        arguments: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp_app: Option<ToolUiDescriptor>,
    },
    ToolApprovalRequired {
        approval_id: String,
        call_id: Option<String>,
        name: String,
        arguments: String,
        effect: ToolEffect,
        environment_policy: ProcessEnvironmentPolicy,
    },
    ToolApprovalResolved {
        approval_id: String,
        call_id: Option<String>,
        decision: &'static str,
    },
    /// The result of executing a tool.
    ToolResult {
        call_id: Option<String>,
        name: String,
        result: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp_app: Option<ToolUiDescriptor>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mcp_app_result: Option<Value>,
    },
    /// A memory registration tool created a durable graph memory.
    MemoryRegistered {
        id: String,
        node_id: String,
        scope_kind: String,
        scope_label: String,
        summary: String,
        created_at: String,
    },
    /// A child thread was spawned by the parent run.
    ChildThreadStarted { thread: AgentThread },
    /// A child thread reached a terminal success state.
    ChildThreadDone { thread: AgentThread },
    /// A child thread reached a terminal error state.
    ChildThreadError {
        thread: AgentThread,
        message: String,
    },
    WorkerRunProposed {
        run: WorkerRun,
        workers: Vec<Worker>,
    },
    WorkerRunStarted {
        run: WorkerRun,
        workers: Vec<Worker>,
    },
    WorkerRunDone {
        run: WorkerRun,
        workers: Vec<Worker>,
    },
    WorkerRunError {
        run: WorkerRun,
        workers: Vec<Worker>,
        message: String,
    },
    /// The final assistant answer.
    Final { content: String },
    /// Terminal event with the turn count and whether the configured iteration
    /// limit stopped the loop before a final model answer.
    Done {
        iterations: usize,
        stopped_at_limit: bool,
        usage: Usage,
    },
    /// An error occurred mid-run.
    Error { message: String },
}

/// Stream the tool-use loop as [`AgentEvent`]s (errors are folded into
/// `AgentEvent::Error` so the stream itself never fails).
pub fn run_agent_stream(
    service: SharedService,
    tools: Arc<ToolRegistry>,
    model: String,
    messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
) -> impl Stream<Item = AgentEvent> + Send {
    run_agent_stream_with_config(
        service,
        tools,
        model,
        messages,
        reasoning_effort,
        AgentRunConfig::default(),
    )
}

/// Stream the tool-use loop with explicit loop configuration.
pub fn run_agent_stream_with_config(
    service: SharedService,
    tools: Arc<ToolRegistry>,
    model: String,
    messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
    config: AgentRunConfig,
) -> impl Stream<Item = AgentEvent> + Send {
    async_stream::stream! {
        let core_tools = tools_to_core(&tools);
        let max_iterations = config.max_iterations();
        let retry_backoff = config.initial_stream_retry_backoff;
        let mut messages = messages;
        let mut total_usage = Usage::default();

        if let Some(hook) = config.step_hook.as_ref() {
            if let Err(e) = hook.commit_tool_catalog(&tools.execution_specs()).await {
                yield AgentEvent::Error { message: e.to_string() };
                return;
            }
        }

        yield AgentEvent::Start { model: model.clone() };

        let mut iteration = 0;
        loop {
            let step = iteration + 1;
            if let Some(hook) = config.step_hook.as_ref() {
                if let Err(e) = hook.prepare_model_step(step, &mut messages).await {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            }
            let req = CompletionRequest {
                model: model.clone(),
                messages: messages.clone(),
                tools: core_tools.clone(),
                tool_choice: None,
                response_format: None,
                prompt: None,
                suffix: None,
                sampling: SamplingParams::default(),
                reasoning_effort,
            };
            if let Some(hook) = config.step_hook.as_ref() {
                if let Err(e) = hook.commit_model_request(step, &req).await {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            }
            let mut stream = match stream_with_initial_retry(&service, req, retry_backoff).await {
                Ok(s) => s,
                Err(e) => {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            };

            let mut content = String::new();
            let mut reasoning = String::new();
            let mut step_usage = Usage::default();
            let mut finish_reason = "stream_ended".to_string();
            let mut tool_acc = ToolCallAccumulator::default();
            while let Some(ev) = stream.next().await {
                match ev {
                    Ok(StreamEvent::Delta(d)) => {
                        if let Some(c) = d.content {
                            content.push_str(&c);
                            yield AgentEvent::Token { text: c };
                        }
                        if let Some(r) = d.reasoning {
                            reasoning.push_str(&r);
                            yield AgentEvent::Reasoning { text: r };
                        }
                        for tc in d.tool_calls {
                            tool_acc.push(tc);
                        }
                    }
                    Ok(StreamEvent::Done { usage, finish_reason: reason }) => {
                        step_usage = usage;
                        finish_reason = reason;
                        add_usage(&mut total_usage, usage);
                        yield AgentEvent::UsageDelta { usage };
                    }
                    Err(e) => {
                        yield AgentEvent::Error { message: e.to_string() };
                        return;
                    }
                }
            }
            iteration += 1;

            let calls = tool_acc.finish();
            if let Some(hook) = config.step_hook.as_ref() {
                if let Err(e) = hook
                    .commit_model_response(
                        step,
                        &content,
                        &reasoning,
                        &calls,
                        &finish_reason,
                        step_usage,
                    )
                    .await
                {
                    yield AgentEvent::Error { message: e.to_string() };
                    return;
                }
            }
            if calls.is_empty() {
                yield AgentEvent::Final { content };
                yield AgentEvent::Done { iterations: iteration, stopped_at_limit: false, usage: total_usage };
                return;
            }
            if iteration >= max_iterations {
                let content = limit_message_text(max_iterations);
                yield AgentEvent::Final { content };
                yield AgentEvent::Done { iterations: iteration, stopped_at_limit: true, usage: total_usage };
                return;
            }

            // Record the assistant's tool-call turn.
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: (!content.is_empty()).then_some(Content::Text(content)),
                name: None,
                tool_calls: Some(calls.clone()),
                tool_call_id: None,
                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
            });

            let mut pending_images: Vec<ChatMessage> = Vec::new();
            let mut prepared_calls = Vec::new();
            for call in calls {
                yield AgentEvent::ToolCall {
                    call_id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                    mcp_app: tools.ui(&call.function.name),
                };
                let effect = tools.effect(&call.function.name).unwrap_or(ToolEffect::Unknown);
                let environment_policy = tools
                    .environment_policy(&call.function.name)
                    .unwrap_or(ProcessEnvironmentPolicy::HostShellInherited);
                let approved = if effect != ToolEffect::ReadOnly {
                    if let Some(broker) = config.approval_broker.as_ref() {
                        let mut pending = broker.request();
                        yield AgentEvent::ToolApprovalRequired {
                            approval_id: pending.id.clone(),
                            call_id: call.id.clone(),
                            name: call.function.name.clone(),
                            arguments: call.function.arguments.clone(),
                            effect,
                            environment_policy,
                        };
                        let approved = pending.wait().await.approved;
                        let _ = pending.deliver();
                        pending.acknowledge();
                        yield AgentEvent::ToolApprovalResolved {
                            approval_id: pending.id.clone(),
                            call_id: call.id.clone(),
                            decision: if approved { "approve" } else { "deny" },
                        };
                        approved
                    } else {
                        true
                    }
                } else {
                    true
                };
                prepared_calls.push((call, approved));
            }
            // Calls enter the fixed registry pipeline in model order. The
            // pipeline's fair exclusive barriers prevent mutating/command/MCP
            // calls from overlapping, while explicitly parallel-safe reads
            // can use at most four slots. `buffered` preserves result order
            // and one failure remains an independent model-visible result.
            let executions = futures::stream::iter(prepared_calls.into_iter().map(|(call, approved)| {
                let tools = tools.clone();
                async move {
                    let executed = if approved {
                        execute_tool_call(
                            tools.as_ref(),
                            &call.function.name,
                            &call.function.arguments,
                        )
                        .await
                    } else {
                        denied_tool_call(tools.as_ref(), &call.function.name)
                    };
                    (call, executed)
                }
            }))
            .buffered(4)
            .collect::<Vec<_>>()
            .await;
            for (call, executed) in executions {
                let visible = executed.visible;
                let model_content = tool_replay_content(&visible);
                if let Some(hook) = config.step_hook.as_ref() {
                    if let Err(e) = hook
                        .commit_tool_result(
                            step,
                            call.id.as_deref(),
                            &call.function.name,
                            &visible,
                            &model_content,
                        )
                        .await
                    {
                        yield AgentEvent::Error { message: e.to_string() };
                        return;
                    }
                }
                yield AgentEvent::ToolResult {
                    call_id: call.id.clone(),
                    name: call.function.name.clone(),
                    result: visible.clone(),
                    mcp_app: executed.ui,
                    mcp_app_result: executed.app_result,
                };
                if let Some(ev) = executed.memory_event {
                    yield ev;
                }
                if let Some(ev) = executed.child_event {
                    yield ev;
                }
                if let Some(ev) = executed.worker_event {
                    let waiting_for_approval = matches!(&ev, AgentEvent::WorkerRunProposed { .. });
                    yield ev;
                    if waiting_for_approval {
                        yield AgentEvent::Done { iterations: iteration, stopped_at_limit: false, usage: total_usage };
                        return;
                    }
                }
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(Content::Text(model_content)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: call.id.clone(),
                    reasoning_content: None,
                });
                if let Some(uri) = executed.image_uri {
                    pending_images.push(image_user_message(&call.function.name, uri));
                }
            }
            // Image results follow the tool replies as user messages (keeps
            // each tool_call_id answered before any other role, per OpenAI).
            messages.extend(pending_images);
        }
    }
}

fn denied_tool_call(tools: &ToolRegistry, name: &str) -> ExecutedToolResult {
    ExecutedToolResult {
        visible: json!({ "error": "Tool call denied by user", "denied": true }),
        image_uri: None,
        ui: tools.ui(name),
        app_result: None,
        memory_event: None,
        child_event: None,
        worker_event: None,
    }
}

fn limit_message(max_iterations: usize) -> ChatMessage {
    ChatMessage::text("assistant", limit_message_text(max_iterations))
}

fn limit_message_text(max_iterations: usize) -> String {
    format!("Agent stopped after reaching the iteration limit ({max_iterations} model turns).")
}

async fn stream_with_initial_retry(
    service: &SharedService,
    req: CompletionRequest,
    backoff: Duration,
) -> Result<EventStream> {
    let first_error = match service.stream(req.clone()).await {
        Ok(stream) => return Ok(stream),
        Err(error) => error,
    };

    if !backoff.is_zero() {
        tokio::time::sleep(backoff).await;
    }

    service.stream(req).await.map_err(|retry_error| {
        Error::Inference(format!(
            "initial stream failed after retry: {first_error}; retry failed: {retry_error}"
        ))
    })
}

fn add_usage(total: &mut Usage, usage: Usage) {
    total.prompt_tokens += usage.prompt_tokens;
    total.completion_tokens += usage.completion_tokens;
    total.total_tokens += usage.total_tokens;
}

fn memory_registered_event(result: &Value) -> Option<AgentEvent> {
    let notice = result.get("memory_notice")?.as_object()?;
    Some(AgentEvent::MemoryRegistered {
        id: notice.get("id")?.as_str()?.to_string(),
        node_id: notice.get("node_id")?.as_str()?.to_string(),
        scope_kind: notice.get("scope_kind")?.as_str()?.to_string(),
        scope_label: notice.get("scope_label")?.as_str()?.to_string(),
        summary: notice.get("summary")?.as_str()?.to_string(),
        created_at: notice.get("created_at")?.as_str()?.to_string(),
    })
}

fn child_thread_event(result: &Value) -> Option<AgentEvent> {
    let notice = result.get("child_thread_notice")?.as_object()?;
    let thread: AgentThread = serde_json::from_value(notice.get("thread")?.clone()).ok()?;
    match notice.get("event")?.as_str()? {
        "started" => Some(AgentEvent::ChildThreadStarted { thread }),
        "done" => Some(AgentEvent::ChildThreadDone { thread }),
        "error" => {
            let message = notice
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| thread.error.clone())
                .unwrap_or_else(|| "child thread failed".to_string());
            Some(AgentEvent::ChildThreadError { thread, message })
        }
        _ => None,
    }
}

fn worker_run_event(result: &Value) -> Option<AgentEvent> {
    let notice = result.get("worker_run_notice")?.as_object()?;
    let run: WorkerRun = serde_json::from_value(notice.get("run")?.clone()).ok()?;
    let workers: Vec<Worker> = serde_json::from_value(notice.get("workers")?.clone()).ok()?;
    match notice.get("event")?.as_str()? {
        "proposed" => Some(AgentEvent::WorkerRunProposed { run, workers }),
        "started" => Some(AgentEvent::WorkerRunStarted { run, workers }),
        "done" => Some(AgentEvent::WorkerRunDone { run, workers }),
        "error" => Some(AgentEvent::WorkerRunError {
            message: notice
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("worker run failed")
                .to_string(),
            run,
            workers,
        }),
        _ => None,
    }
}

fn tool_replay_content(result: &Value) -> String {
    truncate_tool_replay_text(&result.to_string())
}

fn truncate_tool_replay_text(text: &str) -> String {
    let total_lines = text.split('\n').count();
    if total_lines <= TOOL_REPLAY_MAX_LINES && text.len() <= TOOL_REPLAY_MAX_BYTES {
        return text.to_string();
    }

    let mut preview = String::new();
    let mut kept_lines = 0;
    let mut hit_bytes = false;
    for (index, line) in text.split('\n').enumerate() {
        if kept_lines >= TOOL_REPLAY_MAX_LINES {
            break;
        }
        let prefix = if index == 0 { "" } else { "\n" };
        let needed = prefix.len() + line.len();
        if preview.len() + needed > TOOL_REPLAY_MAX_BYTES {
            hit_bytes = true;
            let remaining = TOOL_REPLAY_MAX_BYTES.saturating_sub(preview.len() + prefix.len());
            if remaining > 0 {
                preview.push_str(prefix);
                preview.push_str(prefix_by_bytes(line, remaining));
            }
            break;
        }
        preview.push_str(prefix);
        preview.push_str(line);
        kept_lines += 1;
    }

    let (removed, unit) = if hit_bytes {
        (text.len().saturating_sub(preview.len()), "bytes")
    } else {
        (total_lines.saturating_sub(kept_lines), "lines")
    };
    format!("{preview}\n\n...tool result truncated for replay: omitted {removed} {unit}.")
}

fn prefix_by_bytes(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = 0;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    &text[..end]
}

/// Split a tool result into (visible_json, optional image data-URI).
///
/// A tool may return an `image` object `{ "mime": ..., "data": <base64> }`
/// (e.g. `screenshot`, or an MCP image tool). The image is removed from the
/// visible JSON - so multi-MB base64 blobs never reach the UI, logs, or the
/// `tool` message - and returned as a `data:` URI to attach as a follow-up
/// image message that vision models can actually see.
fn split_tool_image(mut result: Value) -> (Value, Option<String>) {
    let Some(obj) = result.as_object_mut() else {
        return (result, None);
    };
    let Some(img) = obj.remove("image") else {
        return (result, None);
    };
    let Some(data) = img.get("data").and_then(Value::as_str) else {
        return (result, None);
    };
    let mime = img
        .get("mime")
        .and_then(Value::as_str)
        .unwrap_or("image/png");
    let uri = format!("data:{mime};base64,{data}");
    (result, Some(uri))
}

struct ExecutedToolResult {
    visible: Value,
    image_uri: Option<String>,
    ui: Option<ToolUiDescriptor>,
    app_result: Option<Value>,
    memory_event: Option<AgentEvent>,
    child_event: Option<AgentEvent>,
    worker_event: Option<AgentEvent>,
}

async fn execute_tool_call(
    tools: &ToolRegistry,
    name: &str,
    arguments: &str,
) -> ExecutedToolResult {
    let args = serde_json::from_str(arguments).unwrap_or(Value::Null);
    let fallback_ui = tools.ui(name);
    let invoked = match tools.call_for_agent(name, args).await {
        Ok(value) => value,
        Err(error) => milim_tools::ToolAgentResult {
            result: json!({ "error": error.to_string() }),
            app_result: fallback_ui.as_ref().map(|_| {
                json!({
                    "content": [{ "type": "text", "text": error.to_string() }],
                    "isError": true
                })
            }),
            ui: fallback_ui,
        },
    };
    let (visible, image_uri) = split_tool_image(invoked.result);
    let memory_event = memory_registered_event(&visible);
    let child_event = child_thread_event(&visible);
    let worker_event = worker_run_event(&visible);
    ExecutedToolResult {
        visible: limit_visible_tool_result(visible),
        image_uri,
        ui: invoked.ui,
        app_result: invoked.app_result.map(limit_app_tool_result),
        memory_event,
        child_event,
        worker_event,
    }
}

fn limit_app_tool_result(result: Value) -> Value {
    const MAX_BYTES: usize = 1024 * 1024;
    match serde_json::to_vec(&result) {
        Ok(encoded) if encoded.len() <= MAX_BYTES => result,
        Ok(encoded) => json!({
            "content": [{
                "type": "text",
                "text": format!("MCP App result exceeded the {MAX_BYTES}-byte limit ({} bytes)", encoded.len())
            }],
            "isError": true
        }),
        Err(_) => json!({
            "content": [{ "type": "text", "text": "MCP App result could not be encoded" }],
            "isError": true
        }),
    }
}

fn limit_visible_tool_result(result: Value) -> Value {
    const MAX_VISIBLE_BYTES: usize = 1024 * 1024;
    let Ok(encoded) = serde_json::to_vec(&result) else {
        return json!({ "error": "tool result could not be encoded" });
    };
    if encoded.len() <= MAX_VISIBLE_BYTES {
        return result;
    }
    let preview = String::from_utf8_lossy(&encoded[..MAX_VISIBLE_BYTES]).to_string();
    json!({
        "truncated": true,
        "original_bytes": encoded.len(),
        "preview": preview
    })
}

/// A user message carrying an image a tool returned, so the model sees it next
/// turn. Encoded as an OpenAI `image_url` data-URI part (passed through to
/// OpenAI-compatible vision models verbatim; non-vision backends ignore it).
fn image_user_message(tool: &str, data_uri: String) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: Some(Content::Parts(vec![
            ContentPart::Text {
                text: format!("Image returned by the `{tool}` tool:"),
            },
            ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: data_uri,
                    detail: None,
                },
            },
        ])),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        reasoning_content: None,
    }
}

/// Map the registry's tools into OpenAI `Tool` definitions for the request.
fn tools_to_core(tools: &ToolRegistry) -> Vec<Tool> {
    tools
        .list()
        .into_iter()
        .map(|s| Tool {
            kind: "function".to_string(),
            function: ToolFunction {
                name: s.name,
                description: Some(s.description),
                parameters: Some(s.input_schema),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use milim_core::api::openai::{DeltaFunction, DeltaToolCall, Model};
    use milim_inference::test_backend::TestBackend;
    use milim_inference::DeltaEvent;

    struct LoopingToolBackend;

    #[async_trait]
    impl ModelService for LoopingToolBackend {
        fn name(&self) -> &str {
            "looping-tool"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(vec![Model::local("test-loop", 0)])
        }

        async fn stream(&self, _req: CompletionRequest) -> Result<EventStream> {
            let stream = async_stream::stream! {
                yield Ok(StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call_loop".to_string()),
                        kind: Some("function".to_string()),
                        function: DeltaFunction {
                            name: Some("missing_tool".to_string()),
                            arguments: Some("{}".to_string()),
                        },
                    }],
                    ..Default::default()
                }));
                yield Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".to_string(),
                    usage: Usage::new(1, 1),
                });
            };
            Ok(Box::pin(stream))
        }

        async fn embed(&self, _model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(inputs.into_iter().map(|_| vec![0.0]).collect())
        }
    }

    struct FlakyStreamBackend {
        attempts: Arc<AtomicUsize>,
    }

    struct CountingBackend {
        calls: Arc<AtomicUsize>,
    }

    struct OrderedToolsBackend {
        calls: Arc<AtomicUsize>,
        observed_tool_ids: Arc<Mutex<Vec<String>>>,
    }

    struct DelayTool {
        name: &'static str,
        delay: Duration,
    }

    #[async_trait]
    impl milim_tools::Tool for DelayTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "ordered result fixture"
        }

        fn input_schema(&self) -> Value {
            json!({"type":"object"})
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        fn concurrency(&self) -> milim_tools::ToolConcurrency {
            milim_tools::ToolConcurrency::Parallel
        }

        async fn invoke(&self, _args: Value) -> Result<Value> {
            tokio::time::sleep(self.delay).await;
            Ok(json!({"tool": self.name}))
        }
    }

    #[async_trait]
    impl ModelService for OrderedToolsBackend {
        fn name(&self) -> &str {
            "ordered-tools"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(vec![Model::local("ordered-tools", 0)])
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                let stream = async_stream::stream! {
                    yield Ok(StreamEvent::Delta(DeltaEvent {
                        tool_calls: vec![
                            DeltaToolCall {
                                index: 0,
                                id: Some("call-slow".into()),
                                kind: Some("function".into()),
                                function: DeltaFunction {
                                    name: Some("slow".into()),
                                    arguments: Some("{}".into()),
                                },
                            },
                            DeltaToolCall {
                                index: 1,
                                id: Some("call-fast".into()),
                                kind: Some("function".into()),
                                function: DeltaFunction {
                                    name: Some("fast".into()),
                                    arguments: Some("{}".into()),
                                },
                            },
                        ],
                        ..Default::default()
                    }));
                    yield Ok(StreamEvent::Done {
                        finish_reason: "tool_calls".into(),
                        usage: Usage::new(1, 1),
                    });
                };
                return Ok(Box::pin(stream));
            }
            *self.observed_tool_ids.lock().unwrap() = req
                .messages
                .iter()
                .filter(|message| message.role == "tool")
                .filter_map(|message| message.tool_call_id.clone())
                .collect();
            let stream = async_stream::stream! {
                yield Ok(StreamEvent::Delta(DeltaEvent::text("done")));
                yield Ok(StreamEvent::Done {
                    finish_reason: "stop".into(),
                    usage: Usage::new(1, 1),
                });
            };
            Ok(Box::pin(stream))
        }

        async fn embed(&self, _model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(inputs.into_iter().map(|_| vec![0.0]).collect())
        }
    }

    #[async_trait]
    impl ModelService for CountingBackend {
        fn name(&self) -> &str {
            "counting"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            TestBackend::new().list_models().await
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            TestBackend::new().stream(req).await
        }

        async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            TestBackend::new().embed(model, inputs).await
        }
    }

    #[derive(Debug)]
    struct FailingStepHook {
        fail_request: bool,
        fail_response: bool,
    }

    #[async_trait]
    impl AgentStepHook for FailingStepHook {
        async fn prepare_model_step(
            &self,
            _step: usize,
            _messages: &mut Vec<ChatMessage>,
        ) -> Result<()> {
            Ok(())
        }

        async fn commit_model_request(
            &self,
            _step: usize,
            _request: &CompletionRequest,
        ) -> Result<()> {
            if self.fail_request {
                Err(Error::Other("pre-request ledger commit failed".into()))
            } else {
                Ok(())
            }
        }

        async fn commit_model_response(
            &self,
            _step: usize,
            _content: &str,
            _reasoning: &str,
            _tool_calls: &[ToolCall],
            _finish_reason: &str,
            _usage: Usage,
        ) -> Result<()> {
            if self.fail_response {
                Err(Error::Other("post-response ledger commit failed".into()))
            } else {
                Ok(())
            }
        }

        async fn commit_tool_result(
            &self,
            _step: usize,
            _call_id: Option<&str>,
            _name: &str,
            _result: &Value,
            _model_content: &str,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl ModelService for FlakyStreamBackend {
        fn name(&self) -> &str {
            "flaky-stream"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            TestBackend::new().list_models().await
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(Error::Inference(
                    "temporary stream open failure".to_string(),
                ));
            }
            TestBackend::new().stream(req).await
        }

        async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            TestBackend::new().embed(model, inputs).await
        }
    }

    #[tokio::test]
    async fn runs_a_two_step_tool_loop() {
        let service = TestBackend::new();
        let tools = ToolRegistry::with_builtins();
        let messages = vec![ChatMessage::text("user", "/tool please")];

        let outcome = run_agent(&service, &tools, "test-echo", messages, None)
            .await
            .unwrap();

        // The test backend calls `echo` once, the loop runs it, then answers.
        assert_eq!(outcome.iterations, 2);
        assert!(!outcome.stopped_at_limit);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].name, "echo");
        assert_eq!(outcome.steps[0].result["echoed"]["text"], "test");
        assert!(outcome.message.text_content().contains("Echo:"));
    }

    #[tokio::test]
    async fn stops_when_iteration_cap_is_hit() {
        let service = LoopingToolBackend;
        let tools = ToolRegistry::new();
        let messages = vec![ChatMessage::text("user", "keep calling tools")];
        let outcome = run_agent_with_config(
            &service,
            &tools,
            "test-loop",
            messages,
            None,
            AgentRunConfig {
                max_iterations: 2,
                initial_stream_retry_backoff: Duration::ZERO,
                approval_broker: None,
                step_hook: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(outcome.iterations, 2);
        assert!(outcome.stopped_at_limit);
        assert_eq!(outcome.steps.len(), 1);
        assert!(outcome.message.text_content().contains("iteration limit"));
    }

    #[tokio::test]
    async fn approval_broker_tracks_delivery_and_idempotent_resolution() {
        let broker = Arc::new(ToolApprovalBroker::default());
        let mut pending = broker.request();
        let id = pending.id.clone();
        assert_eq!(
            broker.resolve_with_response(&id, true, Some(json!({ "name": "Milim" }))),
            ApprovalResolve::Resolved
        );
        assert_eq!(
            broker.resolve_with_response(&id, true, Some(json!({ "name": "Milim" }))),
            ApprovalResolve::AlreadyResolved
        );
        assert_eq!(broker.resolve(&id, false), ApprovalResolve::Conflict);
        let decision = pending.wait().await;
        assert!(decision.approved);
        assert_eq!(decision.response, Some(json!({ "name": "Milim" })));
        assert_eq!(broker.snapshot(&id).unwrap().state, ApprovalState::Decided);
        pending.mark_delivered();
        assert_eq!(
            broker
                .wait_for_delivery(&id, Duration::from_millis(10))
                .await
                .unwrap()
                .state,
            ApprovalState::Delivered
        );
        pending.acknowledge();
        assert_eq!(
            broker.snapshot(&id).unwrap().state,
            ApprovalState::Acknowledged
        );
        drop(pending);

        let abandoned = broker.request();
        let abandoned_id = abandoned.id.clone();
        drop(abandoned);
        assert_eq!(broker.resolve(&abandoned_id, true), ApprovalResolve::Failed);
        assert_eq!(
            broker.snapshot(&abandoned_id).unwrap().state,
            ApprovalState::Canceled
        );
    }

    #[tokio::test]
    async fn external_approval_publishes_pending_and_resolution_notices() {
        let broker = Arc::new(ToolApprovalBroker::default());
        let mut notices = broker.subscribe();
        let mut pending = broker.request_external(
            "run-1".to_string(),
            Some("call-1".to_string()),
            "shell".to_string(),
            r#"{"command":"cargo test"}"#.to_string(),
            ToolEffect::Command,
        );
        let requested = notices.recv().await.unwrap();
        assert_eq!(requested.run_id, "run-1");
        assert_eq!(requested.call_id.as_deref(), Some("call-1"));
        assert_eq!(requested.decision, None);
        assert_eq!(requested.state, ApprovalState::Requested);

        assert_eq!(
            broker.resolve(&pending.id, false),
            ApprovalResolve::Resolved
        );
        let decided = notices.recv().await.unwrap();
        assert_eq!(decided.decision, Some("deny"));
        assert_eq!(decided.state, ApprovalState::Decided);
        assert!(!pending.wait().await.approved);
        pending.mark_delivered();
        assert_eq!(
            notices.recv().await.unwrap().state,
            ApprovalState::Delivered
        );
        broker.acknowledge_run("run-1");
        assert_eq!(
            notices.recv().await.unwrap().state,
            ApprovalState::Acknowledged
        );
    }

    #[tokio::test]
    async fn stream_retries_initial_open_error_once() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let service: SharedService = Arc::new(FlakyStreamBackend {
            attempts: attempts.clone(),
        });
        let tools = Arc::new(ToolRegistry::new());
        let messages = vec![ChatMessage::text("user", "hello")];
        let mut stream = Box::pin(run_agent_stream_with_config(
            service,
            tools,
            "test-echo".into(),
            messages,
            None,
            AgentRunConfig {
                max_iterations: 100,
                initial_stream_retry_backoff: Duration::ZERO,
                approval_broker: None,
                step_hook: None,
            },
        ));

        let mut saw_final = false;
        let mut saw_done = false;
        let mut saw_error = false;
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::Final { content } => {
                    saw_final = true;
                    assert_eq!(content, "Echo: hello");
                }
                AgentEvent::Done { .. } => saw_done = true,
                AgentEvent::Error { .. } => saw_error = true,
                _ => {}
            }
        }

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert!(saw_final);
        assert!(saw_done);
        assert!(!saw_error);
    }

    #[tokio::test]
    async fn failed_pre_request_commit_prevents_provider_execution() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service: SharedService = Arc::new(CountingBackend {
            calls: calls.clone(),
        });
        let mut stream = Box::pin(run_agent_stream_with_config(
            service,
            Arc::new(ToolRegistry::new()),
            "test-echo".into(),
            vec![ChatMessage::text("user", "hello")],
            None,
            AgentRunConfig {
                step_hook: Some(Arc::new(FailingStepHook {
                    fail_request: true,
                    fail_response: false,
                })),
                ..AgentRunConfig::default()
            },
        ));
        let mut error = None;
        while let Some(event) = stream.next().await {
            if let AgentEvent::Error { message } = event {
                error = Some(message);
            }
        }
        assert!(error.unwrap().contains("pre-request ledger commit failed"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn failed_post_response_commit_prevents_tools_and_another_model_step() {
        let calls = Arc::new(AtomicUsize::new(0));
        let service: SharedService = Arc::new(CountingBackend {
            calls: calls.clone(),
        });
        let mut stream = Box::pin(run_agent_stream_with_config(
            service,
            Arc::new(ToolRegistry::with_builtins()),
            "test-echo".into(),
            vec![ChatMessage::text("user", "/tool please")],
            None,
            AgentRunConfig {
                step_hook: Some(Arc::new(FailingStepHook {
                    fail_request: false,
                    fail_response: true,
                })),
                ..AgentRunConfig::default()
            },
        ));
        let mut saw_tool_result = false;
        let mut error = None;
        while let Some(event) = stream.next().await {
            match event {
                AgentEvent::ToolResult { .. } => saw_tool_result = true,
                AgentEvent::Error { message } => error = Some(message),
                _ => {}
            }
        }
        assert!(error
            .unwrap()
            .contains("post-response ledger commit failed"));
        assert!(!saw_tool_result);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn parallel_tool_results_preserve_model_call_order() {
        let observed_tool_ids = Arc::new(Mutex::new(Vec::new()));
        let service: SharedService = Arc::new(OrderedToolsBackend {
            calls: Arc::new(AtomicUsize::new(0)),
            observed_tool_ids: observed_tool_ids.clone(),
        });
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DelayTool {
            name: "slow",
            delay: Duration::from_millis(30),
        }));
        registry.register(Arc::new(DelayTool {
            name: "fast",
            delay: Duration::from_millis(1),
        }));
        let mut stream = Box::pin(run_agent_stream(
            service,
            Arc::new(registry),
            "ordered-tools".into(),
            vec![ChatMessage::text("user", "run both")],
            None,
        ));
        let mut result_order = Vec::new();
        while let Some(event) = stream.next().await {
            if let AgentEvent::ToolResult { name, .. } = event {
                result_order.push(name);
            }
        }
        assert_eq!(result_order, vec!["slow", "fast"]);
        assert_eq!(
            *observed_tool_ids.lock().unwrap(),
            vec!["call-slow", "call-fast"]
        );
    }

    #[tokio::test]
    async fn streams_tool_loop_events() {
        let service: SharedService = Arc::new(TestBackend::new());
        let tools = Arc::new(ToolRegistry::with_builtins());
        let messages = vec![ChatMessage::text("user", "/tool please")];
        let mut stream = Box::pin(run_agent_stream(
            service,
            tools,
            "test-echo".into(),
            messages,
            None,
        ));

        let mut kinds = Vec::new();
        while let Some(ev) = stream.next().await {
            kinds.push(match ev {
                AgentEvent::Start { .. } => "start",
                AgentEvent::Token { .. } => "token",
                AgentEvent::Reasoning { .. } => "reasoning",
                AgentEvent::UsageDelta { .. } => "usage_delta",
                AgentEvent::ToolCall { .. } => "tool_call",
                AgentEvent::ToolResult { .. } => "tool_result",
                AgentEvent::ToolApprovalRequired { .. } => "tool_approval_required",
                AgentEvent::ToolApprovalResolved { .. } => "tool_approval_resolved",
                AgentEvent::MemoryRegistered { .. } => "memory_registered",
                AgentEvent::ChildThreadStarted { .. } => "child_thread_started",
                AgentEvent::ChildThreadDone { .. } => "child_thread_done",
                AgentEvent::ChildThreadError { .. } => "child_thread_error",
                AgentEvent::WorkerRunProposed { .. } => "worker_run_proposed",
                AgentEvent::WorkerRunStarted { .. } => "worker_run_started",
                AgentEvent::WorkerRunDone { .. } => "worker_run_done",
                AgentEvent::WorkerRunError { .. } => "worker_run_error",
                AgentEvent::Final { .. } => "final",
                AgentEvent::Done { .. } => "done",
                AgentEvent::Error { .. } => "error",
            });
        }
        assert_eq!(kinds.first(), Some(&"start"));
        assert!(kinds.contains(&"tool_call"));
        assert!(kinds.contains(&"tool_result"));
        assert!(kinds.contains(&"final"));
        assert_eq!(kinds.last(), Some(&"done"));
    }

    #[tokio::test]
    async fn streams_usage_summed_across_model_turns() {
        let service: SharedService = Arc::new(TestBackend::new());
        let tools = Arc::new(ToolRegistry::with_builtins());
        let messages = vec![ChatMessage::text("user", "/tool please")];
        let mut stream = Box::pin(run_agent_stream(
            service,
            tools,
            "test-echo".into(),
            messages,
            None,
        ));

        let mut usage = None;
        let mut deltas = Vec::new();
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::UsageDelta { usage: u } => deltas.push(u),
                AgentEvent::Done { usage: u, .. } => usage = Some(u),
                _ => {}
            }
        }

        let usage = usage.expect("agent stream should finish with usage");
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 7);
        assert_eq!(usage.total_tokens, 12);
        assert_eq!(deltas.len(), 2);
        let summed = deltas
            .into_iter()
            .fold(Usage::default(), |mut total, usage| {
                add_usage(&mut total, usage);
                total
            });
        assert_eq!(summed.prompt_tokens, usage.prompt_tokens);
        assert_eq!(summed.completion_tokens, usage.completion_tokens);
        assert_eq!(summed.total_tokens, usage.total_tokens);
    }

    #[test]
    fn split_tool_image_extracts_and_strips() {
        let result = json!({"path":"x.png","width":100,"image":{"mime":"image/png","data":"AAAA"}});
        let (visible, uri) = split_tool_image(result);
        assert_eq!(uri.as_deref(), Some("data:image/png;base64,AAAA"));
        assert!(
            visible.get("image").is_none(),
            "image must be stripped from visible result"
        );
        assert_eq!(visible["path"], "x.png");
    }

    #[test]
    fn split_tool_image_passthrough_without_image() {
        let (visible, uri) = split_tool_image(json!({"ok": true}));
        assert!(uri.is_none());
        assert_eq!(visible["ok"], true);
    }

    #[test]
    fn tool_replay_keeps_small_results() {
        let text = r#"{"ok":true}"#;
        assert_eq!(truncate_tool_replay_text(text), text);
    }

    #[test]
    fn tool_replay_truncates_large_results_by_bytes() {
        let text = "a".repeat(TOOL_REPLAY_MAX_BYTES + 10);
        let replay = truncate_tool_replay_text(&text);
        assert!(replay.starts_with(&"a".repeat(TOOL_REPLAY_MAX_BYTES)));
        assert!(replay.contains("tool result truncated for replay"));
        assert!(replay.contains("omitted 10 bytes"));
    }

    #[test]
    fn tool_replay_truncates_large_results_by_lines() {
        let text = (0..=TOOL_REPLAY_MAX_LINES)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let replay = truncate_tool_replay_text(&text);
        assert!(replay.contains("tool result truncated for replay"));
        assert!(replay.contains("omitted 1 lines"));
        assert!(!replay.ends_with(&TOOL_REPLAY_MAX_LINES.to_string()));
    }

    #[test]
    fn tool_replay_truncates_on_utf8_boundary() {
        let text = "é".repeat((TOOL_REPLAY_MAX_BYTES / "é".len()) + 10);
        let replay = truncate_tool_replay_text(&text);
        assert!(replay.contains("tool result truncated for replay"));
        assert!(replay.is_char_boundary(TOOL_REPLAY_MAX_BYTES));
    }

    #[test]
    fn image_user_message_is_multimodal() {
        let m = image_user_message("screenshot", "data:image/png;base64,AAAA".into());
        assert_eq!(m.role, "user");
        match m.content.unwrap() {
            Content::Parts(p) => {
                assert_eq!(p.len(), 2);
                assert!(matches!(p[1], ContentPart::ImageUrl { .. }));
            }
            _ => panic!("expected multimodal parts"),
        }
    }

    #[tokio::test]
    async fn answers_directly_without_tools() {
        let service = TestBackend::new();
        let tools = ToolRegistry::new();
        let messages = vec![ChatMessage::text("user", "hello")];
        let outcome = run_agent(&service, &tools, "test-echo", messages, None)
            .await
            .unwrap();
        assert_eq!(outcome.iterations, 1);
        assert!(outcome.steps.is_empty());
        assert_eq!(outcome.message.text_content(), "Echo: hello");
    }

    #[tokio::test]
    async fn forwards_reasoning_effort_to_model_turns() {
        let service = TestBackend::new();
        let tools = ToolRegistry::new();
        let messages = vec![ChatMessage::text("user", "hello")];
        let _ = run_agent(
            &service,
            &tools,
            "test-echo",
            messages,
            Some(ReasoningEffort::High),
        )
        .await
        .unwrap();
        assert_eq!(service.last_reasoning_effort(), Some(ReasoningEffort::High));
    }
}
