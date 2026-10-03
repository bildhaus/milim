//! `milim-agents` - the tool-use agent loop.
//!
//! [`run_agent`] drives the core agentic cycle milim exposes via
//! `POST /agents/{id}/run`: ask the model with the available tools; if it emits
//! tool calls, execute them through the [`ToolRegistry`] and feed the results
//! back as `tool`-role messages; repeat until the model answers in plain text.

mod context;
mod intercept;
mod limits;
mod retry;
mod store;
mod threads;
mod tool_output;
pub use intercept::{
    HookActivity, HookTrustRequest, InterceptedCall, ResultInterception, StopInterception,
    ToolDecision, ToolInterception, ToolInterceptor, TurnInterception, MAX_STOP_CONTINUATIONS,
};
pub use limits::AgentRunLimits;

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
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};

use milim_core::api::openai::{
    ChatMessage, Content, ContentPart, ImageUrl, ReasoningEffort, Tool, ToolCall, ToolFunction,
    Usage,
};
use milim_core::{Error, Result};
use milim_inference::{
    CompletionRequest, ModelService, SamplingParams, SharedService, StreamEvent,
    ToolCallAccumulator,
};
use milim_tools::{
    ProcessEnvironmentPolicy, ToolEffect, ToolExecutionSpec, ToolRegistry, ToolUiDescriptor,
};

const DEFAULT_AGENT_MAX_ITERATIONS: usize = 100;
const DEFAULT_INITIAL_STREAM_RETRY_BACKOFF_MS: u64 = 250;
/// Default wait for an interactive tool approval before it is denied.
pub const DEFAULT_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Model-visible denial reason when an approval request expires.
pub const APPROVAL_TIMEOUT_MESSAGE: &str = "approval request timed out";
/// Consecutive output-limit recoveries before a cut-off answer is accepted.
const MAX_LENGTH_RECOVERIES: usize = 2;
const LENGTH_RECOVERY_NOTE: &str = "Your previous response was cut off at the output token limit. Continue from where you stopped, and keep individual tool calls smaller (for example, split a large file write into several edits).";

/// Configuration for one agent loop run.
#[derive(Debug, Clone)]
pub struct AgentRunConfig {
    /// Maximum number of model turns before the loop stops without executing
    /// another round of tool calls.
    pub max_iterations: usize,
    /// Base delay for exponential backoff between retries of a retryable
    /// provider failure (rate limits, overload, 5xx, connection errors).
    pub initial_stream_retry_backoff: Duration,
    /// Interactive approval broker for consequential streamed tool calls.
    pub approval_broker: Option<Arc<ToolApprovalBroker>>,
    /// Durable boundary hook. A failed commit aborts the loop before another
    /// provider request can leave Milim.
    pub step_hook: Option<Arc<dyn AgentStepHook>>,
    /// Sampling controls frozen for the whole agent run. The same values are
    /// reused for every model turn so tool loops cannot silently drift.
    pub sampling: SamplingParams,
    pub limits: AgentRunLimits,
    /// The model's context window. When known, older tool outputs are elided
    /// and old turns summarized before a step would overflow it. Either way,
    /// a prompt the provider rejects as too long is compacted and retried
    /// once, and its size bounds the window for the rest of the run.
    pub context_window_tokens: Option<u32>,
    /// How long an interactive approval may wait before it is denied.
    /// `None` waits indefinitely. A step's approvals are requested together
    /// and waited on concurrently; that wait does not count against
    /// `limits.max_duration`.
    pub approval_timeout: Option<Duration>,
    /// User hooks around turns, tool calls, and the final answer.
    pub interceptor: Option<Arc<dyn ToolInterceptor>>,
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
            sampling: SamplingParams::default(),
            limits: AgentRunLimits::default(),
            context_window_tokens: None,
            approval_timeout: Some(DEFAULT_APPROVAL_TIMEOUT),
            interceptor: None,
        }
    }
}

/// Timing of one model step, across all provider attempts.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelStepTiming {
    /// Unix milliseconds when the step's first provider request started.
    pub started_at_ms: u64,
    /// Milliseconds from the start of the successful attempt's request to
    /// its first streamed delta.
    pub first_token_ms: Option<u64>,
    /// Milliseconds from the first request to the end of the stream,
    /// including retries and backoff.
    pub duration_ms: u64,
    /// Provider requests made for the step (1 without retries).
    pub attempts: u32,
    pub finish_reason: String,
}

/// What in-run context management changed before a model step.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextCompaction {
    pub elided_tool_results: usize,
    pub summarized_messages: usize,
    pub estimated_tokens_before: usize,
    pub estimated_tokens_after: usize,
    /// The effective window: the configured one, or what a provider's
    /// context-length rejection showed. 0 when unknown.
    pub context_window_tokens: u32,
    /// Why summarizing older turns failed; tool results were elided instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_error: Option<String>,
}

#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait::async_trait]
pub trait AgentStepHook: std::fmt::Debug + Send + Sync {
    async fn commit_tool_catalog(&self, _tools: &[ToolExecutionSpec]) -> Result<()> {
        Ok(())
    }

    /// Directory name that groups this run's saved oversized tool output,
    /// usually the run or thread id.
    fn output_scope(&self) -> Option<String> {
        None
    }

    async fn prepare_model_step(&self, step: usize, messages: &mut Vec<ChatMessage>) -> Result<()>;

    async fn commit_model_request(&self, step: usize, request: &CompletionRequest) -> Result<()>;

    /// `provider_state` is the adapter's opaque continuation data for the
    /// turn (see `ChatMessage::provider_state`); persist it byte-exact.
    #[allow(clippy::too_many_arguments)]
    async fn commit_model_response(
        &self,
        step: usize,
        content: &str,
        reasoning: &str,
        tool_calls: &[ToolCall],
        finish_reason: &str,
        usage: Usage,
        provider_state: Option<&Value>,
    ) -> Result<()>;

    async fn commit_tool_result(
        &self,
        step: usize,
        call_id: Option<&str>,
        name: &str,
        result: &Value,
        model_content: &str,
    ) -> Result<()>;

    /// Record in-run context compaction. The compacted messages themselves
    /// are part of the next committed model request.
    async fn commit_context_compaction(
        &self,
        _step: usize,
        _compaction: &ContextCompaction,
    ) -> Result<()> {
        Ok(())
    }

    /// Record model step timing for metrics. Failures are ignored.
    async fn commit_model_timing(&self, _step: usize, _timing: &ModelStepTiming) -> Result<()> {
        Ok(())
    }

    /// Record one tool call's execution time for metrics. Failures are ignored.
    async fn commit_tool_timing(
        &self,
        _step: usize,
        _call_id: Option<&str>,
        _name: &str,
        _duration_ms: u64,
        _is_error: bool,
    ) -> Result<()> {
        Ok(())
    }
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

/// How far an approval reaches. `Thread` means Milim also recorded a
/// per-thread rule that approves later matching requests; runtimes still
/// receive a one-shot approval so that rule alone governs repeats.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    #[default]
    Once,
    Thread,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalDecision {
    pub approved: bool,
    pub response: Option<Value>,
    pub scope: ApprovalScope,
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
        self.resolve_with_scope(id, approved, response, ApprovalScope::Once)
    }

    /// Resolve with an explicit scope. A denial is always one-shot.
    pub fn resolve_with_scope(
        &self,
        id: &str,
        approved: bool,
        response: Option<Value>,
        scope: ApprovalScope,
    ) -> ApprovalResolve {
        let response_fingerprint = approval_response_fingerprint(&response);
        let scope = if approved { scope } else { ApprovalScope::Once };
        let decision = ApprovalDecision {
            approved,
            response,
            scope,
        };
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
                    scope: ApprovalScope::Once,
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

/// Run the tool-use loop with explicit loop configuration. This is the
/// streamed loop collected into one outcome: the same retries, context
/// management, hooks, and limits apply.
pub async fn run_agent_with_config(
    service: &dyn ModelService,
    tools: &ToolRegistry,
    model: &str,
    messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
    config: AgentRunConfig,
) -> Result<AgentOutcome> {
    let events = agent_loop(
        service,
        tools,
        model.to_string(),
        messages,
        reasoning_effort,
        config,
    );
    futures::pin_mut!(events);
    let mut steps = Vec::new();
    // Every announced call gets one result, in call order.
    let mut arguments = std::collections::VecDeque::new();
    let mut answer = None;
    while let Some(event) = events.next().await {
        match event {
            LoopEvent::Event(AgentEvent::ToolCall {
                arguments: call_arguments,
                ..
            }) => arguments.push_back(call_arguments),
            LoopEvent::Event(AgentEvent::ToolResult {
                name,
                result,
                mcp_app,
                mcp_app_result,
                ..
            }) => steps.push(ToolStep {
                name,
                arguments: arguments.pop_front().unwrap_or_default(),
                result,
                mcp_app,
                mcp_app_result,
            }),
            LoopEvent::Event(AgentEvent::Done {
                iterations,
                stopped_at_limit,
                ..
            }) => {
                return Ok(AgentOutcome {
                    // A run that stops to wait for a worker plan decision
                    // has no final answer yet.
                    message: answer.unwrap_or_else(|| ChatMessage::text("assistant", "")),
                    steps,
                    iterations,
                    stopped_at_limit,
                });
            }
            LoopEvent::Event(_) => {}
            LoopEvent::Answer(message) => answer = Some(message),
            LoopEvent::Limited {
                reason, iterations, ..
            } => {
                return Ok(AgentOutcome {
                    message: ChatMessage::text("assistant", reason),
                    steps,
                    iterations,
                    stopped_at_limit: true,
                });
            }
            LoopEvent::Failed { error, .. } => return Err(error),
        }
    }
    Err(Error::Other(
        "agent loop ended without a terminal event".into(),
    ))
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
    /// Harness-authored text shown to the user, such as a run-limit stop.
    /// It is not model output: consumers display it but must never replay
    /// it to a model as assistant text.
    Notice { text: String },
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
        /// Why the loop resolved the approval itself, e.g. `timed_out`.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// A retryable provider failure; the step is retried after `delay_ms`.
    /// A prompt rejected as too long for the context window is retried once
    /// after compacting older context (reason `context window exceeded`).
    /// Text and reasoning streamed by the failed attempt are discarded; the
    /// byte counts let consumers drop what they already accumulated.
    ProviderRetry {
        attempt: u32,
        delay_ms: u64,
        reason: String,
        discarded_content_bytes: usize,
        discarded_reasoning_bytes: usize,
    },
    /// Older context was compacted before a model step to stay inside the
    /// model's context window.
    ContextCompacted {
        elided_tool_results: usize,
        summarized_messages: usize,
        estimated_tokens_before: usize,
        estimated_tokens_after: usize,
        /// Why summarizing older turns failed; tool results were elided
        /// instead.
        #[serde(skip_serializing_if = "Option::is_none")]
        summary_error: Option<String>,
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
    /// A user hook ran, or project hooks were skipped until trusted.
    Hook(HookActivity),
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
        let events = agent_loop(
            service.as_ref(),
            tools.as_ref(),
            model,
            messages,
            reasoning_effort,
            config,
        );
        futures::pin_mut!(events);
        while let Some(event) = events.next().await {
            match event {
                LoopEvent::Event(event) => yield event,
                LoopEvent::Answer(message) => {
                    yield AgentEvent::Final { content: message.text_content() };
                }
                LoopEvent::Limited { reason, iterations, usage } => {
                    yield AgentEvent::Notice { text: format!("\n\n{reason} {CONTINUE_HINT}") };
                    yield AgentEvent::Done { iterations, stopped_at_limit: true, usage };
                }
                LoopEvent::Failed { message, .. } => yield AgentEvent::Error { message },
            }
        }
    }
}

/// Appended to a run-limit notice in streamed runs.
const CONTINUE_HINT: &str = "Send Continue to start another bounded run in this thread.";

/// One item from [`agent_loop`]: a public event, or a terminal outcome that
/// the streaming and collecting entry points present differently.
// Items move straight through, like `AgentEvent`s themselves; boxing the
// common `Event` variant would only add an allocation per streamed token.
#[allow(clippy::large_enum_variant)]
enum LoopEvent {
    Event(AgentEvent),
    /// The model's final answer, followed by `Done` (streamed as
    /// [`AgentEvent::Final`]).
    Answer(ChatMessage),
    /// A run limit stopped the loop (streamed as a notice, then `Done`).
    Limited {
        reason: String,
        iterations: usize,
        usage: Usage,
    },
    /// The run failed. `message` is what the stream shows; `error` keeps the
    /// underlying error for callers that return it.
    Failed {
        error: Error,
        message: String,
    },
}

impl LoopEvent {
    fn failed(error: Error) -> Self {
        Self::Failed {
            message: error.to_string(),
            error,
        }
    }
}

/// One tool call of a step, validated and on its way through approval.
struct PreparedCall {
    call: ToolCall,
    arguments: std::result::Result<Value, String>,
    approved: bool,
    denial: Option<String>,
}

/// The tool-use loop shared by [`run_agent_stream_with_config`] and
/// [`run_agent_with_config`].
fn agent_loop<'a>(
    service: &'a dyn ModelService,
    tools: &'a ToolRegistry,
    model: String,
    messages: Vec<ChatMessage>,
    reasoning_effort: Option<ReasoningEffort>,
    config: AgentRunConfig,
) -> impl Stream<Item = LoopEvent> + Send + 'a {
    async_stream::stream! {
        let core_tools = tools_to_core(tools);
        let max_iterations = config.max_iterations();
        let retry_backoff = config.initial_stream_retry_backoff;
        let output_scope = config.step_hook.as_ref().and_then(|hook| hook.output_scope());
        let mut messages = messages;
        // The user request that started this run. Compaction never
        // summarizes it.
        let mut anchor = messages.iter().rposition(|message| message.role == "user");
        let mut window = context::ContextWindow::new(config.context_window_tokens);
        let mut total_usage = Usage::default();
        let mut budget = limits::RunBudget::new(config.limits.clone());
        // Model-visible notes queued for the next step. They are appended
        // after `prepare_model_step`, which may rebuild `messages` from the
        // ledger, so the committed request carries them.
        let mut pending_notes: Vec<ChatMessage> = Vec::new();
        let mut length_recoveries = 0;
        let mut stop_continuations = 0;

        if let Some(hook) = config.step_hook.as_ref() {
            if let Err(e) = hook.commit_tool_catalog(&tools.execution_specs()).await {
                yield LoopEvent::failed(e);
                return;
            }
        }

        yield LoopEvent::Event(AgentEvent::Start { model: model.clone() });

        if let Some(interceptor) = config.interceptor.as_ref() {
            let turn = interceptor.before_turn(&messages).await;
            for activity in turn.activity {
                yield LoopEvent::Event(AgentEvent::Hook(activity));
            }
            if let Some(reason) = turn.block {
                let message = blocked_turn_message(&reason);
                yield LoopEvent::Failed { error: Error::InvalidRequest(message.clone()), message };
                return;
            }
            pending_notes.extend(intercept::context_message(&turn.context));
        }

        let mut iteration = 0;
        loop {
            let step = iteration + 1;
            if let Some(reason) = budget.reason() {
                yield LoopEvent::Limited { reason, iterations: iteration, usage: total_usage };
                return;
            }
            if let Some(hook) = config.step_hook.as_ref() {
                if let Err(e) = hook.prepare_model_step(step, &mut messages).await {
                    yield LoopEvent::failed(e);
                    return;
                }
            }
            messages.append(&mut pending_notes);

            // One model step. A retryable failure, whether opening the stream
            // or in the middle of it, discards the partial turn and retries
            // the same request. A prompt the provider rejects as too long is
            // compacted once, regardless of thresholds, and sent again.
            let started_at_ms = approval_now_ms();
            let step_started = Instant::now();
            let mut attempts: u32 = 0;
            let mut overflow: Option<Error> = None;
            let mut recovered_overflow = false;
            let mut request_estimate;
            let mut content;
            let mut reasoning;
            let mut step_usage;
            let mut finish_reason;
            let mut tool_acc;
            let mut first_token_ms;
            let mut provider_state: Option<Value>;
            'request: loop {
                // Context management runs on the exact messages about to be
                // committed and sent, so the ledger stays byte-exact.
                let forced = overflow.is_some();
                let compacted = compact_context(
                    service,
                    &model,
                    &mut messages,
                    &mut anchor,
                    &core_tools,
                    &window,
                    forced,
                    budget.reason().is_none(),
                    &config.sampling,
                    retry_backoff,
                )
                .await;
                if let Some(compacted) = &compacted {
                    if let Some(usage) = compacted.usage {
                        budget.record(usage);
                        add_usage(&mut total_usage, usage);
                        yield LoopEvent::Event(AgentEvent::UsageDelta { usage });
                    }
                    let compaction = compacted.record();
                    if let Some(hook) = config.step_hook.as_ref() {
                        if let Err(e) = hook.commit_context_compaction(step, &compaction).await {
                            yield LoopEvent::failed(e);
                            return;
                        }
                    }
                    yield LoopEvent::Event(AgentEvent::ContextCompacted {
                        elided_tool_results: compaction.elided_tool_results,
                        summarized_messages: compaction.summarized_messages,
                        estimated_tokens_before: compaction.estimated_tokens_before,
                        estimated_tokens_after: compaction.estimated_tokens_after,
                        summary_error: compaction.summary_error,
                    });
                }
                if let Some(error) = overflow.take() {
                    if !compacted.as_ref().is_some_and(Compacted::shrank) {
                        yield LoopEvent::Failed {
                            message: format!("The conversation no longer fits this model's context window and there is no older context left to compact: {error}"),
                            error,
                        };
                        return;
                    }
                    recovered_overflow = true;
                }

                let req = CompletionRequest {
                    model: model.clone(),
                    messages: messages.clone(),
                    tools: core_tools.clone(),
                    tool_choice: None,
                    response_format: None,
                    prompt: None,
                    suffix: None,
                    sampling: config.sampling.clone(),
                    reasoning_effort,
                };
                if let Some(reason) = budget.reason() {
                    yield LoopEvent::Limited { reason, iterations: iteration, usage: total_usage };
                    return;
                }
                if let Some(hook) = config.step_hook.as_ref() {
                    if let Err(e) = hook.commit_model_request(step, &req).await {
                        yield LoopEvent::failed(e);
                        return;
                    }
                }
                request_estimate = context::estimate_tokens(&req.messages, &core_tools);

                let mut request_attempts: u32 = 0;
                loop {
                    attempts += 1;
                    request_attempts += 1;
                    content = String::new();
                    reasoning = String::new();
                    provider_state = None;
                    step_usage = Usage::default();
                    finish_reason = String::new();
                    tool_acc = ToolCallAccumulator::default();
                    first_token_ms = None;
                    let mut saw_done = false;
                    let attempt_started = Instant::now();
                    let mut failure = match service.stream(req.clone()).await {
                        Err(error) => Some(error),
                        Ok(mut stream) => {
                            let mut failure = None;
                            while let Some(ev) = stream.next().await {
                                match ev {
                                    Ok(StreamEvent::Delta(d)) => {
                                        if first_token_ms.is_none() && !d.is_empty() {
                                            first_token_ms = Some(elapsed_ms(attempt_started));
                                        }
                                        if let Some(c) = d.content {
                                            content.push_str(&c);
                                            yield LoopEvent::Event(AgentEvent::Token { text: c });
                                        }
                                        if let Some(r) = d.reasoning {
                                            reasoning.push_str(&r);
                                            yield LoopEvent::Event(AgentEvent::Reasoning { text: r });
                                        }
                                        for tc in d.tool_calls {
                                            tool_acc.push(tc);
                                        }
                                        if d.provider_state.is_some() {
                                            provider_state = d.provider_state;
                                        }
                                    }
                                    Ok(StreamEvent::Done { usage, finish_reason: reason }) => {
                                        saw_done = true;
                                        step_usage = usage;
                                        finish_reason = reason;
                                        add_usage(&mut total_usage, usage);
                                        yield LoopEvent::Event(AgentEvent::UsageDelta { usage });
                                    }
                                    Err(e) => {
                                        failure = Some(e);
                                        break;
                                    }
                                }
                            }
                            failure
                        }
                    };
                    // A stream that ends without its completion event was cut
                    // off; what it produced is incomplete.
                    if failure.is_none() && !saw_done {
                        failure = Some(Error::Other(retry::STREAM_ENDED_EARLY.into()));
                    }
                    let overflowed = match &failure {
                        Some(error) => retry::context_overflow(&error.to_string()),
                        None => retry::context_overflow_finish(&finish_reason),
                    };
                    if overflowed {
                        let error = failure.unwrap_or_else(|| {
                            Error::Inference(format!(
                                "the model's context window filled up (finish reason `{finish_reason}`)"
                            ))
                        });
                        if recovered_overflow {
                            yield LoopEvent::Failed {
                                message: format!("The conversation no longer fits this model's context window, even after compacting older context: {error}"),
                                error,
                            };
                            return;
                        }
                        if saw_done {
                            budget.record(step_usage);
                        }
                        window.lower(window.estimate(&req.messages, &core_tools));
                        yield LoopEvent::Event(AgentEvent::ProviderRetry {
                            attempt: attempts,
                            delay_ms: 0,
                            reason: "context window exceeded".into(),
                            discarded_content_bytes: content.len(),
                            discarded_reasoning_bytes: reasoning.len(),
                        });
                        overflow = Some(error);
                        continue 'request;
                    }
                    let Some(error) = failure else {
                        break 'request;
                    };
                    // Tool calls are neither announced nor run until their
                    // stream completes, so a partial step of any kind is
                    // discarded and retried.
                    let message = error.to_string();
                    let retry = (request_attempts <= retry::MAX_PROVIDER_RETRIES)
                        .then(|| retry::retryable(&message))
                        .flatten();
                    let Some(retry) = retry else {
                        let message = if request_attempts > 1 {
                            format!("{message} (gave up after {request_attempts} attempts)")
                        } else {
                            message
                        };
                        yield LoopEvent::Failed { error, message };
                        return;
                    };
                    if saw_done {
                        budget.record(step_usage);
                    }
                    let delay = retry::backoff_delay(retry_backoff, request_attempts, retry.retry_after);
                    if budget.remaining_time().is_some_and(|remaining| delay >= remaining) {
                        yield LoopEvent::Failed {
                            message: format!("{message} (not retried: the run time limit would pass during the {}ms backoff)", delay.as_millis()),
                            error,
                        };
                        return;
                    }
                    yield LoopEvent::Event(AgentEvent::ProviderRetry {
                        attempt: attempts,
                        delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                        reason: retry.reason,
                        discarded_content_bytes: content.len(),
                        discarded_reasoning_bytes: reasoning.len(),
                    });
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    if let Some(reason) = budget.reason() {
                        yield LoopEvent::Limited { reason, iterations: iteration, usage: total_usage };
                        return;
                    }
                }
            }
            iteration += 1;
            budget.record(step_usage);
            window.calibrate(request_estimate, step_usage.prompt_tokens);
            let finish_reason = normalize_finish_reason(&finish_reason);
            let truncated = finish_reason == "length";

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
                        provider_state.as_ref(),
                    )
                    .await
                {
                    yield LoopEvent::failed(e);
                    return;
                }
                let timing = ModelStepTiming {
                    started_at_ms,
                    first_token_ms,
                    duration_ms: elapsed_ms(step_started),
                    attempts,
                    finish_reason: finish_reason.clone(),
                };
                let _ = hook.commit_model_timing(step, &timing).await;
            }
            if calls.is_empty() {
                if let Some(reason) = budget.reason() {
                    yield LoopEvent::Limited { reason, iterations: iteration, usage: total_usage };
                    return;
                }
                // A cut-off answer continues in another step instead of
                // ending the run mid-sentence.
                if truncated && length_recoveries < MAX_LENGTH_RECOVERIES && iteration < max_iterations {
                    length_recoveries += 1;
                    if !content.is_empty() {
                        messages.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: Some(Content::Text(content)),
                            name: None,
                            tool_calls: None,
                            tool_call_id: None,
                            reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                            provider_state,
                        });
                    }
                    pending_notes.push(ChatMessage::text("user", LENGTH_RECOVERY_NOTE));
                    continue;
                }
                if let Some(interceptor) = config.interceptor.as_ref() {
                    let stop = interceptor.on_stop(&content, stop_continuations).await;
                    for activity in stop.activity {
                        yield LoopEvent::Event(AgentEvent::Hook(activity));
                    }
                    if let Some(feedback) = stop.continue_with.filter(|_| {
                        stop_continuations < intercept::MAX_STOP_CONTINUATIONS && iteration < max_iterations
                    }) {
                        stop_continuations += 1;
                        if !content.is_empty() {
                            messages.push(ChatMessage {
                                role: "assistant".to_string(),
                                content: Some(Content::Text(content)),
                                name: None,
                                tool_calls: None,
                                tool_call_id: None,
                                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                                provider_state,
                            });
                        }
                        pending_notes.push(intercept::stop_feedback_message(&feedback));
                        continue;
                    }
                }
                yield LoopEvent::Answer(ChatMessage {
                    role: "assistant".to_string(),
                    content: Some(Content::Text(content)),
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                    provider_state,
                });
                yield LoopEvent::Event(AgentEvent::Done { iterations: iteration, stopped_at_limit: false, usage: total_usage });
                return;
            }
            if let Some(reason) = budget.reason().or_else(|| (iteration >= max_iterations).then(|| limit_message_text(max_iterations))) {
                yield LoopEvent::Limited { reason, iterations: iteration, usage: total_usage };
                return;
            }
            let unparseable = truncated
                && calls
                    .iter()
                    .any(|call| tool_output::parse_tool_arguments(&call.function.arguments).is_err());
            let recover_length = unparseable && length_recoveries < MAX_LENGTH_RECOVERIES;
            if recover_length {
                length_recoveries += 1;
            } else if !unparseable {
                length_recoveries = 0;
            }

            // Record the assistant's tool-call turn.
            messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: (!content.is_empty()).then_some(Content::Text(content)),
                name: None,
                tool_calls: Some(calls.clone()),
                tool_call_id: None,
                reasoning_content: (!reasoning.is_empty()).then_some(reasoning),
                provider_state,
            });

            // Announce every call, and every approval the step needs, before
            // waiting on any of them.
            let mut prepared_calls = Vec::new();
            let mut approvals = Vec::new();
            for call in calls {
                yield LoopEvent::Event(AgentEvent::ToolCall {
                    call_id: call.id.clone(),
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                    mcp_app: tools.ui(&call.function.name),
                });
                // Invalid calls are answered with an error and never run, so
                // they need no approval.
                let arguments = prepare_tool_arguments(
                    tools,
                    &call.function.name,
                    &call.function.arguments,
                    truncated,
                );
                let mut denial: Option<String> = None;
                let mut hook_approved = false;
                if let (Some(interceptor), Ok(args), None) =
                    (config.interceptor.as_ref(), &arguments, budget.reason())
                {
                    let other_names = tools.other_names(&call.function.name);
                    let interception = interceptor
                        .before_tool(&intercepted_call(&call, &other_names, args))
                        .await;
                    for activity in interception.activity {
                        yield LoopEvent::Event(AgentEvent::Hook(activity));
                    }
                    match interception.decision {
                        ToolDecision::Continue => {}
                        ToolDecision::Approve => hook_approved = true,
                        ToolDecision::Deny(reason) => denial = Some(reason),
                    }
                }
                let mut approved = budget.reason().is_none() && denial.is_none();
                if let (true, Ok(args)) = (approved, &arguments) {
                    let effect = tools
                        .effect_for_call(&call.function.name, args)
                        .unwrap_or(ToolEffect::Unknown);
                    let environment_policy = tools
                        .environment_policy(&call.function.name)
                        .unwrap_or(ProcessEnvironmentPolicy::HostShellInherited);
                    if let Some(broker) = config
                        .approval_broker
                        .as_ref()
                        .filter(|_| effect != ToolEffect::ReadOnly && !hook_approved)
                    {
                        let pending = broker.request();
                        yield LoopEvent::Event(AgentEvent::ToolApprovalRequired {
                            approval_id: pending.id.clone(),
                            call_id: call.id.clone(),
                            name: call.function.name.clone(),
                            arguments: call.function.arguments.clone(),
                            effect,
                            environment_policy,
                        });
                        approvals.push((prepared_calls.len(), pending));
                        approved = false;
                    }
                }
                prepared_calls.push(PreparedCall { call, arguments, approved, denial });
            }
            if !approvals.is_empty() {
                // The decisions are awaited together, in whatever order the
                // person answers them. Each one is acknowledged as soon as it
                // arrives, since its resolver waits for that acknowledgement;
                // the calls still run in call order below. Time spent waiting
                // for a person does not count against the run time limit.
                budget.pause_clock();
                let approval_timeout = config.approval_timeout;
                let count = approvals.len();
                let mut decisions = futures::stream::iter(approvals.into_iter().map(
                    |(index, mut pending): (usize, PendingApproval)| async move {
                        let decision = match approval_timeout {
                            Some(timeout) => tokio::time::timeout(timeout, pending.wait()).await.ok(),
                            None => Some(pending.wait().await),
                        };
                        if decision.is_some() {
                            let _ = pending.deliver();
                            pending.acknowledge();
                        }
                        (index, pending, decision)
                    },
                ))
                .buffer_unordered(count);
                while let Some((index, pending, decision)) = decisions.next().await {
                    let prepared = &mut prepared_calls[index];
                    match decision {
                        Some(decision) => {
                            prepared.approved = decision.approved;
                            yield LoopEvent::Event(AgentEvent::ToolApprovalResolved {
                                approval_id: pending.id.clone(),
                                call_id: prepared.call.id.clone(),
                                decision: if decision.approved { "approve" } else { "deny" },
                                reason: None,
                            });
                        }
                        None => {
                            pending.fail(APPROVAL_TIMEOUT_MESSAGE);
                            prepared.denial = Some(APPROVAL_TIMEOUT_MESSAGE.to_string());
                            yield LoopEvent::Event(AgentEvent::ToolApprovalResolved {
                                approval_id: pending.id.clone(),
                                call_id: prepared.call.id.clone(),
                                decision: "deny",
                                reason: Some("timed_out".into()),
                            });
                        }
                    }
                }
                budget.resume_clock();
            }
            // Calls enter the fixed registry pipeline in model order. The
            // pipeline's fair exclusive barriers prevent mutating/command/MCP
            // calls from overlapping, while explicitly parallel-safe reads
            // can use at most four slots. A time-bounded run uses one slot so
            // each tool checks its deadline before entering the pipeline.
            // `buffered` preserves result order
            // and one failure remains an independent model-visible result.
            let executions = futures::stream::iter(prepared_calls.into_iter().map(|prepared| {
                let budget = &budget;
                async move {
                    let PreparedCall { call, arguments, approved, denial } = prepared;
                    let started = Instant::now();
                    let executed = if let Some(reason) = budget.reason() {
                        let mut skipped = denied_tool_call(tools, &call.function.name, None);
                        skipped.visible = json!({ "skipped": true, "error": reason });
                        skipped
                    } else if !approved {
                        denied_tool_call(tools, &call.function.name, denial.as_deref())
                    } else {
                        match arguments {
                            Ok(args) => execute_tool_call(tools, &call.function.name, args).await,
                            Err(message) => tool_error_result(tools, &call.function.name, message),
                        }
                    };
                    (call, executed, elapsed_ms(started))
                }
            }))
            .buffered(if config.limits.max_duration.is_some() { 1 } else { 4 })
            .collect::<Vec<_>>()
            .await;
            // The step's results share one model-visible budget.
            let model_contents = tool_output::model_tool_contents(
                executions.iter().map(|(call, executed, _)| {
                    (executed.model_text.as_deref(), &executed.visible, call.id.as_deref())
                }),
                output_scope.as_deref(),
            );
            let mut pending_images: Vec<ChatMessage> = Vec::new();
            for ((call, executed, duration_ms), mut model_content) in executions.into_iter().zip(model_contents) {
                let visible = executed.visible;
                if let (Some(interceptor), true) = (config.interceptor.as_ref(), executed.attempted) {
                    let args = tool_output::parse_tool_arguments(&call.function.arguments).unwrap_or(Value::Null);
                    let other_names = tools.other_names(&call.function.name);
                    let after = interceptor
                        .after_tool(&intercepted_call(&call, &other_names, &args), &visible)
                        .await;
                    for activity in after.activity {
                        yield LoopEvent::Event(AgentEvent::Hook(activity));
                    }
                    model_content = intercept::with_feedback(model_content, &after.feedback);
                }
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
                        yield LoopEvent::failed(e);
                        return;
                    }
                    if executed.attempted {
                        let _ = hook
                            .commit_tool_timing(
                                step,
                                call.id.as_deref(),
                                &call.function.name,
                                duration_ms,
                                visible.get("error").is_some(),
                            )
                            .await;
                    }
                }
                yield LoopEvent::Event(AgentEvent::ToolResult {
                    call_id: call.id.clone(),
                    name: call.function.name.clone(),
                    result: visible.clone(),
                    mcp_app: executed.ui,
                    mcp_app_result: executed.app_result,
                });
                if let Some(ev) = executed.memory_event {
                    yield LoopEvent::Event(ev);
                }
                if let Some(ev) = executed.child_event {
                    yield LoopEvent::Event(ev);
                }
                if let Some(ev) = executed.worker_event {
                    let waiting_for_approval = matches!(&ev, AgentEvent::WorkerRunProposed { .. });
                    yield LoopEvent::Event(ev);
                    if waiting_for_approval {
                        yield LoopEvent::Event(AgentEvent::Done { iterations: iteration, stopped_at_limit: false, usage: total_usage });
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
                    provider_state: None,
                });
                if let Some(uri) = executed.image_uri {
                    pending_images.push(image_user_message(&call.function.name, uri));
                }
            }
            // Image results follow the tool replies as user messages (keeps
            // each tool_call_id answered before any other role, per OpenAI).
            messages.extend(pending_images);
            if recover_length {
                pending_notes.push(ChatMessage::text("user", LENGTH_RECOVERY_NOTE));
            }
        }
    }
}

fn intercepted_call<'a>(
    call: &'a ToolCall,
    other_names: &'a [String],
    arguments: &'a Value,
) -> InterceptedCall<'a> {
    InterceptedCall {
        call_id: call.id.as_deref(),
        name: &call.function.name,
        other_names,
        arguments,
    }
}

fn blocked_turn_message(reason: &str) -> String {
    format!("UserPromptSubmit hook blocked this turn: {}", reason.trim())
}

fn denied_tool_call(tools: &ToolRegistry, name: &str, reason: Option<&str>) -> ExecutedToolResult {
    ExecutedToolResult {
        visible: json!({ "error": reason.unwrap_or("Tool call denied by user"), "denied": true }),
        model_text: None,
        image_uri: None,
        ui: tools.ui(name),
        app_result: None,
        memory_event: None,
        child_event: None,
        worker_event: None,
        attempted: false,
    }
}

/// Validate one streamed call before it may run: the tool must exist and
/// its arguments must be JSON that fits the tool's input schema. The error
/// text is what the model sees.
fn prepare_tool_arguments(
    tools: &ToolRegistry,
    name: &str,
    arguments: &str,
    truncated: bool,
) -> std::result::Result<Value, String> {
    if !tools.contains(name) {
        let available = tools.names();
        return Err(if available.is_empty() {
            format!("Unknown tool `{name}`. No tools are available in this run.")
        } else {
            format!(
                "Unknown tool `{name}`. Available tools: {}.",
                available.join(", ")
            )
        });
    }
    let args = tool_output::parse_tool_arguments(arguments)
        .map_err(|error| tool_output::invalid_json_message(&error, truncated))?;
    if let Some(schema) = tools.input_schema(name) {
        tool_output::validate_tool_arguments(&schema, &args)
            .map_err(|message| format!("Invalid arguments for `{name}`: {message}"))?;
    }
    Ok(args)
}

/// Map provider-specific output-limit stop reasons onto `length`.
fn normalize_finish_reason(reason: &str) -> String {
    match reason {
        "max_tokens" | "max_output_tokens" | "MAX_TOKENS" => "length",
        other => other,
    }
    .to_string()
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// What one compaction pass changed before a model request.
struct Compacted {
    elided: usize,
    summarized: usize,
    estimated_before: usize,
    estimated_after: usize,
    window: Option<usize>,
    /// Usage of the summary request, when it succeeded.
    usage: Option<Usage>,
    summary_error: Option<String>,
}

impl Compacted {
    fn shrank(&self) -> bool {
        self.elided > 0 || self.summarized > 0
    }

    fn record(&self) -> ContextCompaction {
        ContextCompaction {
            elided_tool_results: self.elided,
            summarized_messages: self.summarized,
            estimated_tokens_before: self.estimated_before,
            estimated_tokens_after: self.estimated_after,
            context_window_tokens: self
                .window
                .map_or(0, |tokens| u32::try_from(tokens).unwrap_or(u32::MAX)),
            summary_error: self.summary_error.clone(),
        }
    }
}

/// Keep `messages` inside the context window. Above
/// [`context::PRUNE_THRESHOLD`] older tool results are elided; still above
/// [`context::SUMMARIZE_THRESHOLD`], older turns are summarized with one
/// extra request to the run's model. `force` (after the provider rejected
/// the prompt as too long) does both regardless of thresholds, keeping less
/// recent context. A failed summary falls back to eliding every older tool
/// result and is reported, never silent. `None` when nothing was done.
#[allow(clippy::too_many_arguments)]
async fn compact_context(
    service: &dyn ModelService,
    model: &str,
    messages: &mut Vec<ChatMessage>,
    anchor: &mut Option<usize>,
    tools: &[Tool],
    window: &context::ContextWindow,
    force: bool,
    may_summarize: bool,
    sampling: &SamplingParams,
    backoff: Duration,
) -> Option<Compacted> {
    let tokens = window.tokens();
    let over = |estimate: usize, share: f64| {
        tokens.is_some_and(|tokens| estimate > (tokens as f64 * share) as usize)
    };
    let before = window.estimate(messages, tools);
    if !force && !over(before, context::PRUNE_THRESHOLD) {
        return None;
    }
    let retain = if force {
        context::RETAIN_FORCED
    } else {
        context::RETAIN
    };
    let mut elided = context::elide_old_tool_results(messages, retain.tool_results);
    let mut after = window.estimate(messages, tools);
    let mut summarized = 0;
    let mut usage = None;
    let mut summary_error = None;
    if may_summarize && (force || over(after, context::SUMMARIZE_THRESHOLD)) {
        if let Some(plan) = context::summary_span(messages, *anchor, retain.turns) {
            let transcript = context::summary_transcript(messages, &plan, tokens);
            match summarize_messages(service, model, &transcript, sampling, backoff).await {
                Ok((summary, summary_usage)) => {
                    usage = Some(summary_usage);
                    summarized = context::apply_summary(messages, &plan, anchor, &summary);
                }
                Err(error) => {
                    summary_error = Some(error.to_string());
                    elided += context::elide_old_tool_results(messages, 0);
                }
            }
            after = window.estimate(messages, tools);
        }
    }
    (elided > 0 || summarized > 0 || summary_error.is_some()).then_some(Compacted {
        elided,
        summarized,
        estimated_before: before,
        estimated_after: after,
        window: tokens,
        usage,
        summary_error,
    })
}

/// Condense a transcript of older conversation with one request to the
/// run's own backend, under the same retry policy as model steps.
async fn summarize_messages(
    service: &dyn ModelService,
    model: &str,
    transcript: &str,
    sampling: &SamplingParams,
    backoff: Duration,
) -> Result<(String, Usage)> {
    let req = CompletionRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage::text("system", context::SUMMARY_INSTRUCTIONS),
            ChatMessage::text(
                "user",
                format!("Summarize this earlier part of the session:\n\n{transcript}"),
            ),
        ],
        tools: Vec::new(),
        tool_choice: None,
        response_format: None,
        prompt: None,
        suffix: None,
        sampling: summary_sampling(sampling),
        reasoning_effort: None,
    };
    let out = complete_with_retry(service, req, backoff).await?;
    let summary = out.message.text_content();
    if summary.trim().is_empty() {
        return Err(Error::Inference("context summary was empty".into()));
    }
    Ok((summary, out.usage))
}

/// Output floor for the summary request when the run caps output tokens.
const SUMMARY_MIN_OUTPUT_TOKENS: u32 = 4_096;

/// Sampling for the summary request: provider defaults plus the run's
/// prompt-cache key. The run's stop sequences never cut the summary short,
/// and a run output cap is raised to at least [`SUMMARY_MIN_OUTPUT_TOKENS`].
fn summary_sampling(run: &SamplingParams) -> SamplingParams {
    SamplingParams {
        max_tokens: run
            .max_tokens
            .map(|max_tokens| max_tokens.max(SUMMARY_MIN_OUTPUT_TOKENS)),
        prompt_cache_key: run.prompt_cache_key.clone(),
        ..SamplingParams::default()
    }
}

fn limit_message_text(max_iterations: usize) -> String {
    format!("Agent stopped after reaching the iteration limit ({max_iterations} model turns).")
}

/// Non-streaming model call under the same retry policy as model steps,
/// without progress events.
async fn complete_with_retry(
    service: &dyn ModelService,
    req: CompletionRequest,
    backoff: Duration,
) -> Result<milim_inference::CompletionOutput> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let error = match service.complete(req.clone()).await {
            Ok(out) => return Ok(out),
            Err(error) => error,
        };
        let retry = (attempts <= retry::MAX_PROVIDER_RETRIES)
            .then(|| retry::retryable(&error.to_string()))
            .flatten();
        let Some(retry) = retry else {
            return Err(error);
        };
        let delay = retry::backoff_delay(backoff, attempts, retry.retry_after);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

fn add_usage(total: &mut Usage, usage: Usage) {
    let had_usage = total.prompt_tokens > 0
        || total.completion_tokens > 0
        || total.total_tokens > 0
        || total.cost_usd.is_some();
    total.cost_usd = if had_usage {
        match (total.cost_usd, usage.cost_usd) {
            (Some(current), Some(next)) => Some(current + next),
            _ => None,
        }
    } else {
        usage.cost_usd
    };
    total.prompt_tokens += usage.prompt_tokens;
    total.completion_tokens += usage.completion_tokens;
    total.total_tokens += usage.total_tokens;
    total.add_cache_tokens(&usage);
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
    /// The tool's plain-text model projection, sent verbatim when present.
    model_text: Option<String>,
    image_uri: Option<String>,
    ui: Option<ToolUiDescriptor>,
    app_result: Option<Value>,
    memory_event: Option<AgentEvent>,
    child_event: Option<AgentEvent>,
    worker_event: Option<AgentEvent>,
    /// Whether execution was attempted (false for denied or skipped calls).
    attempted: bool,
}

async fn execute_tool_call(tools: &ToolRegistry, name: &str, args: Value) -> ExecutedToolResult {
    let invoked = match tools.call_for_agent(name, args).await {
        Ok(value) => value,
        Err(error) => return tool_error_result(tools, name, error.to_string()),
    };
    let (visible, image_uri) = split_tool_image(invoked.result);
    let memory_event = memory_registered_event(&visible);
    let child_event = child_thread_event(&visible);
    let worker_event = worker_run_event(&visible);
    ExecutedToolResult {
        visible: limit_visible_tool_result(visible),
        model_text: invoked.model_text,
        image_uri,
        ui: invoked.ui,
        app_result: invoked.app_result.map(limit_app_tool_result),
        memory_event,
        child_event,
        worker_event,
        attempted: true,
    }
}

/// A failed call as the model and any MCP App see it.
fn tool_error_result(tools: &ToolRegistry, name: &str, message: String) -> ExecutedToolResult {
    let ui = tools.ui(name);
    ExecutedToolResult {
        app_result: ui.as_ref().map(|_| {
            json!({
                "content": [{ "type": "text", "text": message }],
                "isError": true
            })
        }),
        visible: json!({ "error": message }),
        model_text: None,
        image_uri: None,
        ui,
        memory_event: None,
        child_event: None,
        worker_event: None,
        attempted: true,
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
        provider_state: None,
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
    use milim_inference::{DeltaEvent, EventStream};

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
            _provider_state: Option<&Value>,
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
                return Err(Error::Upstream(
                    "x chat/completions -> 503 Service Unavailable: temporary stream open failure"
                        .to_string(),
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
                ..Default::default()
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
    async fn spend_and_step_limits_stop_before_another_tool_is_executed() {
        for (max_iterations, limits, expected) in [
            (
                100,
                AgentRunLimits {
                    max_cost_usd: Some(0.01),
                    ..Default::default()
                },
                "no usable price",
            ),
            (1, AgentRunLimits::default(), "iteration limit"),
            (
                100,
                AgentRunLimits {
                    max_cost_usd: Some(0.01),
                    pricing: Some(milim_core::api::openai::ModelPricing {
                        prompt: Some("0.01".into()),
                        completion: Some("0.01".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                "spend threshold",
            ),
        ] {
            let events = run_agent_stream_with_config(
                Arc::new(LoopingToolBackend),
                Arc::new(ToolRegistry::new()),
                "test-loop".into(),
                vec![ChatMessage::text("user", "continue")],
                None,
                AgentRunConfig {
                    max_iterations,
                    limits,
                    ..Default::default()
                },
            )
            .collect::<Vec<_>>()
            .await;
            assert!(!events.iter().any(|event| matches!(
                event,
                AgentEvent::ToolCall { .. } | AgentEvent::ToolResult { .. }
            )));
            assert!(events.iter().any(
                |event| matches!(event, AgentEvent::Notice { text } if text.contains(expected) && text.contains(CONTINUE_HINT))
            ));
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::Token { text } if text.contains(CONTINUE_HINT))),
                "limit text is never streamed as model output"
            );
            assert!(matches!(
                events.last(),
                Some(AgentEvent::Done {
                    stopped_at_limit: true,
                    iterations: 1,
                    ..
                })
            ));
        }
    }

    #[tokio::test]
    async fn approval_wait_does_not_count_against_the_time_limit() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let broker = Arc::new(ToolApprovalBroker::default());
        let events = run_agent_stream_with_config(
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]),
            Arc::new(registry),
            "scripted".into(),
            vec![ChatMessage::text("user", "continue")],
            None,
            AgentRunConfig {
                approval_broker: Some(broker.clone()),
                limits: AgentRunLimits {
                    max_duration: Some(Duration::from_millis(100)),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        futures::pin_mut!(events);
        let mut results = Vec::new();
        let mut done = None;
        while let Some(event) = events.next().await {
            match event {
                AgentEvent::ToolApprovalRequired { approval_id, .. } => {
                    // A person answers well after the run time limit.
                    let broker = broker.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(150)).await;
                        broker.resolve(&approval_id, true);
                    });
                }
                AgentEvent::ToolResult { result, .. } => results.push(result),
                AgentEvent::Done {
                    stopped_at_limit, ..
                } => done = Some(stopped_at_limit),
                AgentEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the approved tool runs");
        assert_ne!(results[0]["skipped"], true);
        assert_eq!(done, Some(false));
    }

    #[tokio::test]
    async fn unknown_spend_on_a_final_answer_still_marks_the_limit() {
        let events = run_agent_stream_with_config(
            Arc::new(TestBackend::new()),
            Arc::new(ToolRegistry::new()),
            "test-echo".into(),
            vec![ChatMessage::text("user", "hello")],
            None,
            AgentRunConfig {
                limits: AgentRunLimits {
                    max_cost_usd: Some(1.0),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .collect::<Vec<_>>()
        .await;
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                stopped_at_limit: true,
                ..
            })
        ));
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
    async fn stream_retries_a_retryable_open_error() {
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
                ..Default::default()
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
    async fn time_limit_between_tools_finishes_started_work_and_skips_the_next_call() {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DelayTool {
            name: "slow",
            delay: Duration::from_millis(150),
        }));
        registry.register(Arc::new(DelayTool {
            name: "fast",
            delay: Duration::from_millis(1),
        }));
        let events = run_agent_stream_with_config(
            Arc::new(OrderedToolsBackend {
                calls: Arc::new(AtomicUsize::new(0)),
                observed_tool_ids: Arc::new(Mutex::new(Vec::new())),
            }),
            Arc::new(registry),
            "ordered-tools".into(),
            vec![ChatMessage::text("user", "run both")],
            None,
            AgentRunConfig {
                limits: AgentRunLimits {
                    max_duration: Some(Duration::from_millis(100)),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .collect::<Vec<_>>()
        .await;
        let results: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolResult { name, result, .. } => Some((name.as_str(), result)),
                _ => None,
            })
            .collect();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "slow");
        assert_ne!(
            results[0].1["skipped"], true,
            "the started tool must finish normally"
        );
        assert_eq!(results[1].0, "fast");
        assert_eq!(
            results[1].1["skipped"], true,
            "the second tool must not execute after the deadline"
        );
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                stopped_at_limit: true,
                ..
            })
        ));
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
                AgentEvent::Notice { .. } => "notice",
                AgentEvent::UsageDelta { .. } => "usage_delta",
                AgentEvent::ToolCall { .. } => "tool_call",
                AgentEvent::ToolResult { .. } => "tool_result",
                AgentEvent::ToolApprovalRequired { .. } => "tool_approval_required",
                AgentEvent::ToolApprovalResolved { .. } => "tool_approval_resolved",
                AgentEvent::ProviderRetry { .. } => "provider_retry",
                AgentEvent::ContextCompacted { .. } => "context_compacted",
                AgentEvent::Hook(_) => "hook",
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
    fn sums_provider_cost_only_when_every_model_turn_reports_it() {
        let mut total = Usage::default();
        add_usage(
            &mut total,
            Usage {
                cost_usd: Some(0.12),
                ..Usage::new(10, 2)
            },
        );
        add_usage(
            &mut total,
            Usage {
                cost_usd: Some(0.03),
                ..Usage::new(3, 1)
            },
        );
        assert!((total.cost_usd.unwrap() - 0.15).abs() < f64::EPSILON);

        add_usage(&mut total, Usage::new(2, 1));
        assert_eq!(
            total.cost_usd, None,
            "a missing billed-cost event must force catalog estimation for the whole run",
        );
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

    /// A mutating tool that records how often it actually ran.
    struct RecordingTool {
        name: &'static str,
        runs: Arc<AtomicUsize>,
        result: Value,
        model_text: Option<String>,
    }

    #[async_trait]
    impl milim_tools::Tool for RecordingTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "recording fixture"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}, "safe": {"type": "boolean"}},
            })
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::Mutating
        }

        fn effect_for_call(&self, args: &Value) -> ToolEffect {
            if args["safe"] == true {
                ToolEffect::ReadOnly
            } else {
                ToolEffect::Mutating
            }
        }

        fn model_text(&self, _result: &Value) -> Option<String> {
            self.model_text.clone()
        }

        async fn invoke(&self, _args: Value) -> Result<Value> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(self.result.clone())
        }
    }

    fn recording_tool(name: &'static str, runs: &Arc<AtomicUsize>) -> Arc<RecordingTool> {
        Arc::new(RecordingTool {
            name,
            runs: runs.clone(),
            result: json!({"ok": true}),
            model_text: None,
        })
    }

    struct WriteTool {
        runs: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl milim_tools::Tool for WriteTool {
        fn name(&self) -> &str {
            "write"
        }

        fn description(&self) -> &str {
            "schema fixture"
        }

        fn input_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
                "required": ["path"]
            })
        }

        fn effect(&self) -> ToolEffect {
            ToolEffect::ReadOnly
        }

        async fn invoke(&self, _args: Value) -> Result<Value> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"written": true}))
        }
    }

    enum Script {
        OpenError(String),
        Events(Vec<Result<StreamEvent>>),
    }

    fn text_step(text: &str, finish_reason: &str) -> Script {
        Script::Events(vec![
            Ok(StreamEvent::Delta(DeltaEvent::text(text))),
            Ok(StreamEvent::Done {
                finish_reason: finish_reason.into(),
                usage: Usage::new(1, 1),
            }),
        ])
    }

    fn tool_step(calls: &[(&str, &str, &str)], finish_reason: &str) -> Script {
        Script::Events(vec![
            Ok(StreamEvent::Delta(DeltaEvent {
                tool_calls: calls
                    .iter()
                    .enumerate()
                    .map(|(index, (id, name, arguments))| DeltaToolCall {
                        index: index as u32,
                        id: Some((*id).into()),
                        kind: Some("function".into()),
                        function: DeltaFunction {
                            name: Some((*name).into()),
                            arguments: Some((*arguments).into()),
                        },
                    })
                    .collect(),
                ..Default::default()
            })),
            Ok(StreamEvent::Done {
                finish_reason: finish_reason.into(),
                usage: Usage::new(1, 1),
            }),
        ])
    }

    /// Replays scripted provider responses in order (then answers "done")
    /// and records every request it received.
    struct ScriptedBackend {
        script: Mutex<std::collections::VecDeque<Script>>,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    impl ScriptedBackend {
        fn new(script: Vec<Script>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(script.into()),
                requests: Mutex::new(Vec::new()),
            })
        }

        fn requests(&self) -> Vec<CompletionRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ModelService for ScriptedBackend {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn list_models(&self) -> Result<Vec<Model>> {
            Ok(vec![Model::local("scripted", 0)])
        }

        async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
            self.requests.lock().unwrap().push(req);
            let next = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| text_step("done", "stop"));
            match next {
                Script::OpenError(message) => Err(Error::Upstream(message)),
                Script::Events(events) => Ok(Box::pin(futures::stream::iter(events))),
            }
        }

        async fn embed(&self, _model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(inputs.into_iter().map(|_| vec![0.0]).collect())
        }
    }

    /// (step, call_id, name, is_error)
    type ToolTimingRecord = (usize, Option<String>, String, bool);

    #[derive(Debug, Default)]
    struct RecordingHook {
        committed_requests: Mutex<Vec<String>>,
        model_timings: Mutex<Vec<(usize, ModelStepTiming)>>,
        tool_timings: Mutex<Vec<ToolTimingRecord>>,
        compactions: Mutex<Vec<ContextCompaction>>,
        tool_contents: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl AgentStepHook for RecordingHook {
        fn output_scope(&self) -> Option<String> {
            Some("run-fixture".into())
        }

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
            request: &CompletionRequest,
        ) -> Result<()> {
            self.committed_requests
                .lock()
                .unwrap()
                .push(serde_json::to_string(&request.messages).unwrap());
            Ok(())
        }

        async fn commit_model_response(
            &self,
            _step: usize,
            _content: &str,
            _reasoning: &str,
            _tool_calls: &[ToolCall],
            _finish_reason: &str,
            _usage: Usage,
            _provider_state: Option<&Value>,
        ) -> Result<()> {
            Ok(())
        }

        async fn commit_tool_result(
            &self,
            _step: usize,
            _call_id: Option<&str>,
            _name: &str,
            _result: &Value,
            model_content: &str,
        ) -> Result<()> {
            self.tool_contents
                .lock()
                .unwrap()
                .push(model_content.to_string());
            Ok(())
        }

        async fn commit_context_compaction(
            &self,
            _step: usize,
            compaction: &ContextCompaction,
        ) -> Result<()> {
            self.compactions.lock().unwrap().push(compaction.clone());
            Ok(())
        }

        async fn commit_model_timing(&self, step: usize, timing: &ModelStepTiming) -> Result<()> {
            self.model_timings
                .lock()
                .unwrap()
                .push((step, timing.clone()));
            Ok(())
        }

        async fn commit_tool_timing(
            &self,
            step: usize,
            call_id: Option<&str>,
            name: &str,
            _duration_ms: u64,
            is_error: bool,
        ) -> Result<()> {
            self.tool_timings.lock().unwrap().push((
                step,
                call_id.map(str::to_string),
                name.to_string(),
                is_error,
            ));
            Ok(())
        }
    }

    async fn run_scripted(
        backend: Arc<ScriptedBackend>,
        registry: ToolRegistry,
        messages: Vec<ChatMessage>,
        config: AgentRunConfig,
    ) -> Vec<AgentEvent> {
        run_agent_stream_with_config(
            backend,
            Arc::new(registry),
            "scripted".into(),
            messages,
            None,
            AgentRunConfig {
                initial_stream_retry_backoff: Duration::ZERO,
                ..config
            },
        )
        .collect::<Vec<_>>()
        .await
    }

    fn tool_results(events: &[AgentEvent]) -> Vec<Value> {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolResult { result, .. } => Some(result.clone()),
                _ => None,
            })
            .collect()
    }

    fn tool_messages(request: &CompletionRequest) -> Vec<String> {
        request
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .map(ChatMessage::text_content)
            .collect()
    }

    fn write_registry(runs: &Arc<AtomicUsize>) -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(WriteTool { runs: runs.clone() }));
        registry
    }

    #[tokio::test]
    async fn malformed_arguments_are_rejected_without_running_the_tool() {
        let runs = Arc::new(AtomicUsize::new(0));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[("call-1", "write", r#"{"path": "a.rs", "content": "fn ma"#)],
            "tool_calls",
        )]);
        let events = run_scripted(
            backend.clone(),
            write_registry(&runs),
            vec![ChatMessage::text("user", "write it")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        let error = tool_results(&events)[0]["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(error.contains("not valid JSON"), "{error}");
        assert!(
            error.contains("EOF"),
            "the parse error is included: {error}"
        );
        assert!(!error.contains("cut off"));
        let requests = backend.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            tool_messages(&requests[1]),
            vec![json!({ "error": error }).to_string()]
        );
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                stopped_at_limit: false,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn truncated_tool_arguments_explain_the_token_limit_and_add_a_note() {
        let runs = Arc::new(AtomicUsize::new(0));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[(
                "call-1",
                "write",
                r#"{"path": "a.rs", "content": "very long"#,
            )],
            "max_tokens",
        )]);
        let events = run_scripted(
            backend.clone(),
            write_registry(&runs),
            vec![ChatMessage::text("user", "write it")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        let error = tool_results(&events)[0]["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            error.contains("cut off at the output token limit"),
            "{error}"
        );
        assert!(error.contains("several smaller edits"));
        let follow_up = &backend.requests()[1].messages;
        let last = follow_up.last().unwrap();
        assert_eq!(last.role, "user");
        assert_eq!(last.text_content(), LENGTH_RECOVERY_NOTE);
        assert_eq!(follow_up[follow_up.len() - 2].role, "tool");
    }

    #[tokio::test]
    async fn schema_violations_and_empty_arguments_are_reported() {
        let runs = Arc::new(AtomicUsize::new(0));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[
                ("call-1", "write", r#"{"path": 5}"#),
                ("call-2", "write", "  "),
                ("call-3", "write", r#"{"path": "ok.rs"}"#),
            ],
            "tool_calls",
        )]);
        let events = run_scripted(
            backend,
            write_registry(&runs),
            vec![ChatMessage::text("user", "write it")],
            AgentRunConfig::default(),
        )
        .await;
        let results = tool_results(&events);
        assert_eq!(
            results[0]["error"],
            "Invalid arguments for `write`: Argument `path` must be string, got number."
        );
        assert_eq!(
            results[1]["error"],
            "Invalid arguments for `write`: Missing required argument: path."
        );
        assert_eq!(results[2]["written"], true);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn unknown_tools_list_the_available_names() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = write_registry(&runs);
        registry.register(recording_tool("read", &runs));
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "nope", "{}")], "tool_calls")]);
        let events = run_scripted(
            backend,
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(
            tool_results(&events)[0]["error"],
            "Unknown tool `nope`. Available tools: read, write."
        );
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cut_off_answers_continue_up_to_two_consecutive_times() {
        let backend = ScriptedBackend::new(vec![
            text_step("part one", "max_tokens"),
            text_step(" part two", "length"),
            text_step(" part three", "length"),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "write an essay")],
            AgentRunConfig::default(),
        )
        .await;
        let requests = backend.requests();
        assert_eq!(
            requests.len(),
            3,
            "two recoveries, then the answer is accepted"
        );
        let second = &requests[1].messages;
        assert_eq!(second.len(), 3);
        assert_eq!(second[1].role, "assistant");
        assert_eq!(second[1].text_content(), "part one");
        assert_eq!(second[2].text_content(), LENGTH_RECOVERY_NOTE);
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::Final { content } if content == " part three")
        ));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                iterations: 3,
                stopped_at_limit: false,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn model_text_is_sent_verbatim_and_json_renders_readably() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(RecordingTool {
            name: "shell",
            runs: runs.clone(),
            result: json!({"exit_code": 0, "stdout": "a\nb\n"}),
            model_text: None,
        }));
        registry.register(Arc::new(RecordingTool {
            name: "cat",
            runs: runs.clone(),
            result: json!({"content": "ignored"}),
            model_text: Some("line 1\n\"quoted\"".into()),
        }));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[("call-1", "shell", "{}"), ("call-2", "cat", "{}")],
            "tool_calls",
        )]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let expected = vec![
            "{\"exit_code\":0}\n\n--- stdout ---\na\nb".to_string(),
            "line 1\n\"quoted\"".to_string(),
        ];
        assert_eq!(tool_messages(&backend.requests()[1]), expected);
        assert_eq!(*hook.tool_contents.lock().unwrap(), expected);
        // The UI still receives the raw JSON result.
        assert_eq!(tool_results(&events)[1]["content"], "ignored");
    }

    #[tokio::test]
    async fn retryable_open_errors_back_off_and_client_errors_do_not_retry() {
        let backend = ScriptedBackend::new(vec![
            Script::OpenError("x chat/completions -> 503 Service Unavailable: busy".into()),
            Script::OpenError("x chat/completions -> 429 Too Many Requests: slow".into()),
            text_step("done", "stop"),
        ]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let retries = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ProviderRetry {
                    attempt, reason, ..
                } => Some((*attempt, reason.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retries,
            vec![
                (1, "provider error (503)".to_string()),
                (2, "rate limited (429)".to_string())
            ]
        );
        assert_eq!(backend.requests().len(), 3);
        assert_eq!(
            hook.committed_requests.lock().unwrap().len(),
            1,
            "one step, one commit"
        );
        assert_eq!(hook.model_timings.lock().unwrap()[0].1.attempts, 3);
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));

        let backend = ScriptedBackend::new(vec![Script::OpenError(
            "x chat/completions -> 400 Bad Request: invalid schema".into(),
        )]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 1);
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ProviderRetry { .. })));
        assert!(
            matches!(events.last(), Some(AgentEvent::Error { message }) if message.contains("400"))
        );
    }

    #[tokio::test]
    async fn retries_give_up_after_four_attempts() {
        let backend = ScriptedBackend::new(
            (0..6)
                .map(|_| Script::OpenError("x chat/completions -> 502 Bad Gateway: ".into()))
                .collect(),
        );
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 5);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Error { message }) if message.contains("gave up after 5 attempts")
        ));
    }

    #[tokio::test]
    async fn mid_stream_errors_discard_the_partial_turn_and_retry() {
        let backend = ScriptedBackend::new(vec![
            Script::Events(vec![
                Ok(StreamEvent::Delta(DeltaEvent::text("partial"))),
                Err(Error::Upstream(
                    "error decoding response body: connection closed".into(),
                )),
            ]),
            text_step("done", "stop"),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRetry {
                attempt: 1,
                discarded_content_bytes: 7,
                ..
            }
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::Final { content } if content == "done")));
        assert_eq!(backend.requests().len(), 2);

        // Tool calls only run once their stream completes, so a partial
        // call is discarded and the step retried like partial text.
        let runs = Arc::new(AtomicUsize::new(0));
        let backend = ScriptedBackend::new(vec![
            Script::Events(vec![
                Ok(StreamEvent::Delta(DeltaEvent {
                    content: Some("let me write".into()),
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call-1".into()),
                        kind: Some("function".into()),
                        function: DeltaFunction {
                            name: Some("write".into()),
                            arguments: Some(r#"{"path": "a"#.into()),
                        },
                    }],
                    ..Default::default()
                })),
                Err(Error::Upstream(
                    "error decoding response body: connection closed".into(),
                )),
            ]),
            tool_step(&[("call-2", "write", r#"{"path": "a.rs"}"#)], "tool_calls"),
        ]);
        let events = run_scripted(
            backend.clone(),
            write_registry(&runs),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 3);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRetry {
                attempt: 1,
                discarded_content_bytes: 12,
                ..
            }
        )));
        let calls = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolCall { call_id, .. } => call_id.clone(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(calls, vec!["call-2"], "the partial call is never announced");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        let replayed = &backend.requests()[2].messages;
        assert_eq!(
            replayed[1].tool_calls.as_ref().unwrap()[0].id.as_deref(),
            Some("call-2")
        );
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
    }

    #[tokio::test]
    async fn streams_without_a_completion_event_are_retried() {
        let backend = ScriptedBackend::new(vec![
            Script::Events(vec![Ok(StreamEvent::Delta(DeltaEvent::text("cut")))]),
            text_step("whole answer", "stop"),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 2);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRetry {
                discarded_content_bytes: 3,
                reason,
                ..
            } if reason == "incomplete stream"
        )));
        assert!(events.iter().any(
            |event| matches!(event, AgentEvent::Final { content } if content == "whole answer")
        ));

        // A stream that never completes gives up like any other failure.
        let backend = ScriptedBackend::new(
            (0..6)
                .map(|_| Script::Events(vec![Ok(StreamEvent::Delta(DeltaEvent::text("x")))]))
                .collect(),
        );
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "hi")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 5);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Error { message })
                if message.contains(retry::STREAM_ENDED_EARLY) && message.contains("gave up after 5 attempts")
        ));
    }

    fn long_conversation(turns: usize) -> Vec<ChatMessage> {
        let mut messages = vec![
            ChatMessage::text("system", "be useful"),
            ChatMessage::text("user", "the original task"),
        ];
        for turn in 0..turns {
            let id = format!("old-{turn}");
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                name: None,
                tool_calls: Some(vec![ToolCall {
                    id: Some(id.clone()),
                    kind: "function".into(),
                    function: milim_core::api::openai::FunctionCall {
                        name: "read".into(),
                        arguments: "{}".into(),
                    },
                }]),
                tool_call_id: None,
                reasoning_content: None,
                provider_state: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(Content::Text(format!("{turn}:{}", "x".repeat(600)))),
                name: None,
                tool_calls: None,
                tool_call_id: Some(id),
                reasoning_content: None,
                provider_state: None,
            });
        }
        messages
    }

    fn assert_tool_pairing(messages: &[ChatMessage]) {
        for (index, message) in messages.iter().enumerate() {
            if message.role != "tool" {
                continue;
            }
            let id = message.tool_call_id.as_deref().unwrap();
            let owner = messages[..index]
                .iter()
                .rev()
                .find(|candidate| candidate.role == "assistant")
                .unwrap();
            assert!(
                owner
                    .tool_calls
                    .iter()
                    .flatten()
                    .any(|call| call.id.as_deref() == Some(id)),
                "tool result {id} lost its tool call"
            );
        }
    }

    #[tokio::test]
    async fn context_pressure_elides_old_tool_results_first() {
        // ~2.8k estimated tokens: over 60% of 4k, under 85% once elided.
        let backend = ScriptedBackend::new(vec![text_step("done", "stop")]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig {
                context_window_tokens: Some(4_000),
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 1, "no summarization call was needed");
        let tools = tool_messages(&requests[0]);
        assert_eq!(tools.len(), 16);
        assert_eq!(tools[0], context::elided_stub("read"));
        assert!(tools[10..].iter().all(|text| text.len() > 600));
        assert_tool_pairing(&requests[0].messages);
        assert_eq!(
            hook.committed_requests.lock().unwrap()[0],
            serde_json::to_string(&requests[0].messages).unwrap(),
            "the committed request is exactly what was sent"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextCompacted {
                elided_tool_results: 10,
                summarized_messages: 0,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn severe_context_pressure_summarizes_old_turns_with_the_same_backend() {
        let backend = ScriptedBackend::new(vec![
            text_step("- read sixteen files", "stop"),
            text_step("done", "stop"),
        ]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig {
                context_window_tokens: Some(1_000),
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 2);
        let summary_request = &requests[0];
        assert!(summary_request.tools.is_empty());
        assert_eq!(
            summary_request.messages[0].text_content(),
            context::SUMMARY_INSTRUCTIONS
        );
        assert!(summary_request.messages[1]
            .text_content()
            .contains("[called read with {}]"));

        let sent = &requests[1].messages;
        assert_eq!(sent[0].text_content(), "be useful");
        assert_eq!(sent[1].text_content(), "the original task");
        assert_eq!(sent[2].role, "system");
        assert!(sent[2].text_content().contains("- read sixteen files"));
        // The last four turns stay verbatim and paired.
        assert_eq!(sent.len(), 3 + 8);
        assert_eq!(sent[3].role, "assistant");
        assert_tool_pairing(sent);
        assert!(sent.last().unwrap().text_content().starts_with("15:"));
        assert_eq!(
            hook.committed_requests.lock().unwrap()[0],
            serde_json::to_string(sent).unwrap()
        );
        let compaction = hook.compactions.lock().unwrap()[0].clone();
        assert_eq!(compaction.summarized_messages, 24);
        assert!(compaction.estimated_tokens_after < compaction.estimated_tokens_before);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextCompacted {
                summarized_messages: 24,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn unanswered_approvals_time_out_as_denials() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let broker = Arc::new(ToolApprovalBroker::default());
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]);
        let events = run_scripted(
            backend.clone(),
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                approval_broker: Some(broker.clone()),
                approval_timeout: Some(Duration::from_millis(20)),
                ..Default::default()
            },
        )
        .await;
        let approval_id = events
            .iter()
            .find_map(|event| match event {
                AgentEvent::ToolApprovalResolved {
                    approval_id,
                    decision: "deny",
                    reason: Some(reason),
                    ..
                } if reason == "timed_out" => Some(approval_id.clone()),
                _ => None,
            })
            .expect("the approval resolves as a timed-out denial");
        assert_eq!(
            broker.snapshot(&approval_id).unwrap().state,
            ApprovalState::Failed
        );
        assert_eq!(broker.resolve(&approval_id, true), ApprovalResolve::Failed);
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(tool_results(&events)[0]["error"], APPROVAL_TIMEOUT_MESSAGE);
        assert!(tool_messages(&backend.requests()[1])[0].contains(APPROVAL_TIMEOUT_MESSAGE));
    }

    #[tokio::test]
    async fn approval_uses_the_per_call_effect() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[("call-1", "write", r#"{"safe": true}"#)],
            "tool_calls",
        )]);
        let events = run_scripted(
            backend,
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                approval_broker: Some(Arc::new(ToolApprovalBroker::default())),
                ..Default::default()
            },
        )
        .await;
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolApprovalRequired { .. })));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[derive(Debug, Default)]
    struct FakeInterceptor {
        turn_context: Option<String>,
        turn_block: Option<String>,
        decision: ToolDecision,
        feedback: Option<String>,
        stop_feedback: Option<String>,
        stops: AtomicUsize,
        seen_results: Mutex<Vec<Value>>,
    }

    fn fake_activity(event: &str, outcome: &str) -> HookActivity {
        HookActivity {
            event: event.into(),
            hook: "fake".into(),
            source: "user".into(),
            tool_name: None,
            call_id: None,
            outcome: outcome.into(),
            duration_ms: 0,
            message: None,
            trust: None,
        }
    }

    #[async_trait]
    impl ToolInterceptor for FakeInterceptor {
        async fn before_turn(&self, _messages: &[ChatMessage]) -> TurnInterception {
            TurnInterception {
                context: self.turn_context.clone().into_iter().collect(),
                block: self.turn_block.clone(),
                activity: vec![fake_activity("UserPromptSubmit", "ok")],
            }
        }

        async fn before_tool(&self, call: &InterceptedCall<'_>) -> ToolInterception {
            assert_eq!(call.call_id, Some("call-1"));
            ToolInterception {
                decision: self.decision.clone(),
                activity: vec![fake_activity("PreToolUse", "checked")],
            }
        }

        async fn after_tool(
            &self,
            _call: &InterceptedCall<'_>,
            result: &Value,
        ) -> ResultInterception {
            self.seen_results.lock().unwrap().push(result.clone());
            ResultInterception {
                feedback: self.feedback.clone().into_iter().collect(),
                activity: vec![fake_activity("PostToolUse", "feedback")],
            }
        }

        async fn on_stop(&self, _final_content: &str, continuations: usize) -> StopInterception {
            assert_eq!(
                continuations,
                self.stops.fetch_add(1, Ordering::SeqCst).min(3)
            );
            StopInterception {
                continue_with: self.stop_feedback.clone(),
                activity: vec![fake_activity("Stop", "continue")],
            }
        }
    }

    fn hook_events(events: &[AgentEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::Hook(activity) => Some(activity.event.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn interceptor_denial_blocks_the_call_without_asking_for_approval() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]);
        let interceptor = Arc::new(FakeInterceptor {
            decision: ToolDecision::Deny("PreToolUse hook denied: no writes".into()),
            ..Default::default()
        });
        let events = run_scripted(
            backend.clone(),
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                approval_broker: Some(Arc::new(ToolApprovalBroker::default())),
                interceptor: Some(interceptor.clone()),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolApprovalRequired { .. })));
        let result = &tool_results(&events)[0];
        assert_eq!(result["error"], "PreToolUse hook denied: no writes");
        assert_eq!(result["denied"], true);
        assert!(tool_messages(&backend.requests()[1])[0].contains("no writes"));
        assert!(
            interceptor.seen_results.lock().unwrap().is_empty(),
            "denied calls skip PostToolUse"
        );
        assert_eq!(
            hook_events(&events),
            vec!["UserPromptSubmit", "PreToolUse", "Stop"]
        );
    }

    #[tokio::test]
    async fn interceptor_approval_skips_the_interactive_prompt() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]);
        let events = run_scripted(
            backend,
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                approval_broker: Some(Arc::new(ToolApprovalBroker::default())),
                approval_timeout: Some(Duration::from_millis(20)),
                interceptor: Some(Arc::new(FakeInterceptor {
                    decision: ToolDecision::Approve,
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolApprovalRequired { .. })));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn interceptor_context_and_feedback_reach_the_model() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]);
        let hook = Arc::new(RecordingHook::default());
        let interceptor = Arc::new(FakeInterceptor {
            turn_context: Some("branch is main".into()),
            feedback: Some("formatted 1 file".into()),
            ..Default::default()
        });
        let events = run_scripted(
            backend.clone(),
            registry,
            vec![ChatMessage::text("user", "go")],
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                interceptor: Some(interceptor.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        let first = requests[0].messages.last().unwrap();
        assert_eq!(first.role, "system");
        assert!(first.text_content().contains("branch is main"));
        let expected = "{\"ok\":true}\n\n[hook] formatted 1 file".to_string();
        assert_eq!(tool_messages(&requests[1]), vec![expected.clone()]);
        assert_eq!(*hook.tool_contents.lock().unwrap(), vec![expected]);
        assert_eq!(interceptor.seen_results.lock().unwrap()[0]["ok"], true);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(
            hook_events(&events),
            vec!["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]
        );
    }

    #[tokio::test]
    async fn interceptor_block_stops_the_turn_before_any_model_request() {
        let backend = ScriptedBackend::new(vec![]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "deploy prod")],
            AgentRunConfig {
                interceptor: Some(Arc::new(FakeInterceptor {
                    turn_block: Some("no deploys on Friday".into()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await;
        assert!(backend.requests().is_empty());
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Error { message })
                if message == "UserPromptSubmit hook blocked this turn: no deploys on Friday"
        ));
    }

    #[tokio::test]
    async fn stop_feedback_continues_the_run_at_most_three_times() {
        let backend = ScriptedBackend::new(vec![]);
        let interceptor = Arc::new(FakeInterceptor {
            stop_feedback: Some("tests still fail".into()),
            ..Default::default()
        });
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "fix it")],
            AgentRunConfig {
                interceptor: Some(interceptor.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 4, "one answer plus three continuations");
        let second = &requests[1].messages;
        assert_eq!(second[second.len() - 2].role, "assistant");
        assert_eq!(
            second.last().unwrap().text_content(),
            "[Stop hook feedback]\ntests still fail"
        );
        assert_eq!(interceptor.stops.load(Ordering::SeqCst), 4);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                iterations: 4,
                stopped_at_limit: false,
                ..
            })
        ));

        // The non-streaming loop applies the same cap and denial.
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let backend =
            ScriptedBackend::new(vec![tool_step(&[("call-1", "write", "{}")], "tool_calls")]);
        let outcome = run_agent_with_config(
            backend.as_ref(),
            &registry,
            "scripted",
            vec![ChatMessage::text("user", "go")],
            None,
            AgentRunConfig {
                initial_stream_retry_backoff: Duration::ZERO,
                interceptor: Some(Arc::new(FakeInterceptor {
                    decision: ToolDecision::Deny("blocked".into()),
                    stop_feedback: Some("again".into()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(outcome.steps[0].result["error"], "blocked");
        assert_eq!(outcome.iterations, 5);
    }

    #[tokio::test]
    async fn step_timing_hooks_record_model_and_tool_timing() {
        let hook = Arc::new(RecordingHook::default());
        let events = run_agent_stream_with_config(
            Arc::new(TestBackend::new()),
            Arc::new(ToolRegistry::with_builtins()),
            "test-echo".into(),
            vec![ChatMessage::text("user", "/tool please")],
            None,
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .collect::<Vec<_>>()
        .await;
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
        let timings = hook.model_timings.lock().unwrap().clone();
        assert_eq!(timings.len(), 2);
        assert_eq!(timings[0].0, 1);
        assert_eq!(timings[1].0, 2);
        assert_eq!(timings[0].1.attempts, 1);
        assert_eq!(timings[0].1.finish_reason, "tool_calls");
        assert!(timings[0].1.started_at_ms > 0);
        assert!(timings[0].1.first_token_ms.is_some());
        let tools = hook.tool_timings.lock().unwrap().clone();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, 1);
        assert_eq!(tools[0].2, "echo");
        assert!(!tools[0].3);
    }

    const CONTEXT_OVERFLOW: &str = "x chat/completions -> 400 Bad Request: This model's maximum context length is 1000 tokens. However, your messages resulted in 3000 tokens (context_length_exceeded)";

    fn is_summary_text(message: &ChatMessage) -> bool {
        message.role == "system"
            && message
                .text_content()
                .starts_with("Summary of the earlier conversation")
    }

    #[tokio::test]
    async fn context_length_errors_force_compaction_and_retry_once() {
        let backend = ScriptedBackend::new(vec![
            Script::OpenError(CONTEXT_OVERFLOW.into()),
            text_step("- read sixteen files", "stop"),
            text_step("done", "stop"),
        ]);
        let hook = Arc::new(RecordingHook::default());
        // No configured window: the rejection alone drives compaction.
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].messages.len(), 34);
        assert!(requests[1].tools.is_empty(), "the summary request");
        let sent = &requests[2].messages;
        assert_eq!(sent[1].text_content(), "the original task");
        assert!(is_summary_text(&sent[2]));
        assert!(sent[2].text_content().contains("- read sixteen files"));
        // Forced compaction keeps only the last two turns, and only the
        // latest turn's results verbatim.
        assert_eq!(sent.len(), 3 + 4);
        assert_eq!(sent[4].text_content(), context::elided_stub("read"));
        assert!(sent.last().unwrap().text_content().starts_with("15:"));
        assert_tool_pairing(sent);

        let retry = events
            .iter()
            .position(|event| matches!(event, AgentEvent::ProviderRetry { reason, delay_ms: 0, .. } if reason == "context window exceeded"))
            .expect("the rejected step is retried");
        let compacted = events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    AgentEvent::ContextCompacted {
                        summarized_messages: 28,
                        ..
                    }
                )
            })
            .expect("the compaction is reported");
        assert!(retry < compacted);
        let compactions = hook.compactions.lock().unwrap().clone();
        assert_eq!(compactions.len(), 1);
        assert!(
            compactions[0].context_window_tokens > 0,
            "the rejected prompt size becomes the window"
        );
        let committed = hook.committed_requests.lock().unwrap().clone();
        assert_eq!(
            committed.len(),
            2,
            "the compacted request is committed again"
        );
        assert_eq!(committed[1], serde_json::to_string(sent).unwrap());
        assert_eq!(hook.model_timings.lock().unwrap()[0].1.attempts, 2);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Done {
                iterations: 1,
                stopped_at_limit: false,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn a_context_window_finish_reason_discards_the_step_and_compacts() {
        let backend = ScriptedBackend::new(vec![
            Script::Events(vec![
                Ok(StreamEvent::Delta(DeltaEvent::text("partial"))),
                Ok(StreamEvent::Done {
                    finish_reason: "model_context_window_exceeded".into(),
                    usage: Usage::new(5, 1),
                }),
            ]),
            text_step("- summary", "stop"),
            text_step("done", "stop"),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 3);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ProviderRetry {
                discarded_content_bytes: 7,
                reason,
                ..
            } if reason == "context window exceeded"
        )));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::Final { content } if content == "done")));
    }

    #[tokio::test]
    async fn context_length_errors_fail_clearly_when_compaction_cannot_help() {
        // Still too long after one compaction.
        let backend = ScriptedBackend::new(vec![
            Script::OpenError(CONTEXT_OVERFLOW.into()),
            text_step("- summary", "stop"),
            Script::OpenError(CONTEXT_OVERFLOW.into()),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 3);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Error { message })
                if message.contains("even after compacting older context") && message.contains("context_length_exceeded")
        ));

        // Nothing older to compact.
        let backend = ScriptedBackend::new(vec![Script::OpenError(CONTEXT_OVERFLOW.into())]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            vec![ChatMessage::text("user", "a very long paste")],
            AgentRunConfig::default(),
        )
        .await;
        assert_eq!(backend.requests().len(), 1);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Error { message }) if message.contains("no older context left to compact")
        ));
    }

    #[tokio::test]
    async fn real_prompt_counts_calibrate_the_context_estimate() {
        // By chars/4 the conversation is ~1.3k tokens, far under 60% of the
        // 4k window; the provider reports 3.5k, so the next step compacts.
        let backend = ScriptedBackend::new(vec![Script::Events(vec![
            Ok(StreamEvent::Delta(DeltaEvent {
                tool_calls: vec![DeltaToolCall {
                    index: 0,
                    id: Some("call-new".into()),
                    kind: Some("function".into()),
                    function: DeltaFunction {
                        name: Some("read".into()),
                        arguments: Some("{}".into()),
                    },
                }],
                ..Default::default()
            })),
            Ok(StreamEvent::Done {
                finish_reason: "tool_calls".into(),
                usage: Usage::new(3_500, 1),
            }),
        ])]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(8),
            AgentRunConfig {
                context_window_tokens: Some(4_000),
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 2, "no summary was needed");
        assert!(tool_messages(&requests[0])
            .iter()
            .all(|text| text.len() > 600));
        let compactions = hook.compactions.lock().unwrap().clone();
        assert_eq!(compactions.len(), 1);
        assert!(compactions[0].estimated_tokens_before >= 3_500);
        assert_eq!(compactions[0].elided_tool_results, 3);
        assert_eq!(tool_messages(&requests[1])[0], context::elided_stub("read"));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::ContextCompacted { .. })));
    }

    #[tokio::test]
    async fn summaries_use_their_own_sampling_and_the_retry_policy() {
        let backend = ScriptedBackend::new(vec![
            Script::OpenError("x chat/completions -> 503 Service Unavailable: busy".into()),
            text_step("- read sixteen files", "stop"),
            text_step("done", "stop"),
        ]);
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig {
                context_window_tokens: Some(1_000),
                sampling: SamplingParams {
                    max_tokens: Some(64),
                    stop: vec!["END".into()],
                    temperature: Some(1.5),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 3, "the summary was retried once");
        for summary in &requests[..2] {
            assert!(summary.tools.is_empty());
            assert!(summary.sampling.stop.is_empty());
            assert_eq!(summary.sampling.max_tokens, Some(SUMMARY_MIN_OUTPUT_TOKENS));
            assert_eq!(summary.sampling.temperature, None);
        }
        assert_eq!(requests[2].sampling.stop, vec!["END".to_string()]);
        assert_eq!(requests[2].sampling.max_tokens, Some(64));
        assert!(requests[2].messages.iter().any(is_summary_text));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextCompacted {
                summarized_messages: 24,
                summary_error: None,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn a_failed_summary_falls_back_to_elision_and_is_reported() {
        let backend = ScriptedBackend::new(vec![
            Script::OpenError("x chat/completions -> 400 Bad Request: unsupported".into()),
            text_step("done", "stop"),
        ]);
        let hook = Arc::new(RecordingHook::default());
        let events = run_scripted(
            backend.clone(),
            ToolRegistry::new(),
            long_conversation(16),
            AgentRunConfig {
                context_window_tokens: Some(1_000),
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let requests = backend.requests();
        assert_eq!(requests.len(), 2);
        let tools = tool_messages(&requests[1]);
        assert!(tools[..15]
            .iter()
            .all(|text| *text == context::elided_stub("read")));
        assert!(tools[15].starts_with("15:"));
        let compaction = hook.compactions.lock().unwrap()[0].clone();
        assert_eq!(compaction.elided_tool_results, 15);
        assert_eq!(compaction.summarized_messages, 0);
        assert!(compaction.summary_error.as_deref().unwrap().contains("400"));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ContextCompacted {
                summary_error: Some(error),
                elided_tool_results: 15,
                ..
            } if error.contains("400")
        )));
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
    }

    #[tokio::test]
    async fn later_summaries_fold_earlier_ones_and_keep_the_run_request() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(RecordingTool {
            name: "read",
            runs: runs.clone(),
            result: json!({ "content": "z".repeat(3_000) }),
            model_text: None,
        }));
        // Earlier exchanges in the thread, then this run's request.
        let mut messages = long_conversation(16);
        messages.splice(
            1..2,
            [
                ChatMessage::text("user", "an old question"),
                ChatMessage::text("assistant", "an old answer"),
                ChatMessage::text("user", "the original task"),
            ],
        );
        let backend = ScriptedBackend::new(vec![
            text_step("- first summary", "stop"),
            tool_step(&[("call-new", "read", "{}")], "tool_calls"),
            text_step("- second summary", "stop"),
            text_step("done", "stop"),
        ]);
        let events = run_scripted(
            backend.clone(),
            registry,
            messages,
            AgentRunConfig {
                context_window_tokens: Some(800),
                ..Default::default()
            },
        )
        .await;
        assert!(matches!(events.last(), Some(AgentEvent::Done { .. })));
        let requests = backend.requests();
        assert_eq!(requests.len(), 4);
        let first_step = &requests[1].messages;
        assert!(
            !first_step
                .iter()
                .any(|message| message.text_content().contains("an old question")),
            "stale history is summarized"
        );
        assert_eq!(first_step[1].text_content(), "the original task");
        assert!(is_summary_text(&first_step[2]));
        assert!(requests[2].messages[1]
            .text_content()
            .contains("### earlier summary\n- first summary"));
        let second_step = &requests[3].messages;
        let summaries = second_step
            .iter()
            .filter(|message| is_summary_text(message))
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 1, "summaries never stack");
        assert!(summaries[0].text_content().contains("- second summary"));
        assert!(!summaries[0].text_content().contains("- first summary"));
        assert_eq!(second_step[1].text_content(), "the original task");
        assert_tool_pairing(second_step);
    }

    #[tokio::test]
    async fn approvals_are_announced_together_and_each_is_acknowledged_when_answered() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        registry.register(recording_tool("write", &runs));
        let broker = Arc::new(ToolApprovalBroker::default());
        let events = run_agent_stream_with_config(
            ScriptedBackend::new(vec![tool_step(
                &[("call-1", "write", "{}"), ("call-2", "write", "{}")],
                "tool_calls",
            )]),
            Arc::new(registry),
            "scripted".into(),
            vec![ChatMessage::text("user", "go")],
            None,
            AgentRunConfig {
                approval_broker: Some(broker.clone()),
                approval_timeout: Some(Duration::from_secs(5)),
                ..Default::default()
            },
        );
        futures::pin_mut!(events);
        let mut order = Vec::new();
        let mut requested = Vec::new();
        let mut results = Vec::new();
        while let Some(event) = events.next().await {
            match event {
                AgentEvent::ToolApprovalRequired {
                    approval_id,
                    call_id,
                    ..
                } => {
                    order.push(format!("required:{}", call_id.unwrap()));
                    requested.push(approval_id);
                    if requested.len() == 2 {
                        // Both are pending at once; answer only the second.
                        assert_eq!(
                            broker.resolve(&requested[1], false),
                            ApprovalResolve::Resolved
                        );
                    }
                }
                AgentEvent::ToolApprovalResolved {
                    call_id, decision, ..
                } => {
                    let call_id = call_id.unwrap();
                    order.push(format!("{decision}:{call_id}"));
                    if call_id == "call-2" {
                        // The second answer is acknowledged while the first
                        // is still open, so its resolver never waits on it.
                        assert_eq!(
                            broker.snapshot(&requested[1]).unwrap().state,
                            ApprovalState::Acknowledged
                        );
                        assert_ne!(
                            broker.snapshot(&requested[0]).unwrap().state,
                            ApprovalState::Acknowledged
                        );
                        assert_eq!(
                            broker.resolve(&requested[0], true),
                            ApprovalResolve::Resolved
                        );
                    }
                }
                AgentEvent::ToolResult { result, .. } => results.push(result),
                AgentEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        }
        assert_eq!(
            order,
            vec![
                "required:call-1",
                "required:call-2",
                "deny:call-2",
                "approve:call-1"
            ]
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(results[0]["ok"], true);
        assert_eq!(results[1]["denied"], true);
        for id in &requested {
            assert_eq!(
                broker.snapshot(id).unwrap().state,
                ApprovalState::Acknowledged
            );
        }
    }

    #[tokio::test]
    async fn collected_runs_match_the_streamed_loop() {
        let script = || {
            vec![
                tool_step(&[("call-1", "write", r#"{"path": "a.rs"}"#)], "tool_calls"),
                Script::Events(vec![
                    Ok(StreamEvent::Delta(DeltaEvent {
                        reasoning: Some("checked".into()),
                        content: Some("all done".into()),
                        ..Default::default()
                    })),
                    Ok(StreamEvent::Done {
                        finish_reason: "stop".into(),
                        usage: Usage::new(1, 1),
                    }),
                ]),
            ]
        };
        let runs = Arc::new(AtomicUsize::new(0));
        let streamed = run_scripted(
            ScriptedBackend::new(script()),
            write_registry(&runs),
            vec![ChatMessage::text("user", "write it")],
            AgentRunConfig::default(),
        )
        .await;
        let backend = ScriptedBackend::new(script());
        let outcome = run_agent_with_config(
            backend.as_ref(),
            &write_registry(&runs),
            "scripted",
            vec![ChatMessage::text("user", "write it")],
            None,
            AgentRunConfig {
                initial_stream_retry_backoff: Duration::ZERO,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(outcome.steps.len(), 1);
        assert_eq!(outcome.steps[0].name, "write");
        assert_eq!(outcome.steps[0].arguments, r#"{"path": "a.rs"}"#);
        assert_eq!(outcome.steps[0].result, tool_results(&streamed)[0]);
        assert_eq!(outcome.message.text_content(), "all done");
        assert_eq!(
            outcome.message.reasoning_content.as_deref(),
            Some("checked")
        );
        assert!(streamed
            .iter()
            .any(|event| matches!(event, AgentEvent::Final { content } if content == "all done")));
        assert!(matches!(
            streamed.last(),
            Some(AgentEvent::Done {
                iterations: 2,
                stopped_at_limit: false,
                ..
            })
        ));
        assert_eq!(outcome.iterations, 2);
        assert!(!outcome.stopped_at_limit);
        assert_eq!(backend.requests().len(), 2);

        // Limits end the collected run with the plain reason.
        let outcome = run_agent_with_config(
            ScriptedBackend::new(script()).as_ref(),
            &write_registry(&runs),
            "scripted",
            vec![ChatMessage::text("user", "write it")],
            None,
            AgentRunConfig {
                max_iterations: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(outcome.stopped_at_limit);
        assert_eq!(outcome.message.text_content(), limit_message_text(1));
        assert!(outcome.steps.is_empty());

        // Errors keep their kind.
        let error = run_agent_with_config(
            ScriptedBackend::new(vec![Script::OpenError(
                "x chat/completions -> 400 Bad Request: invalid schema".into(),
            )])
            .as_ref(),
            &ToolRegistry::new(),
            "scripted",
            vec![ChatMessage::text("user", "hi")],
            None,
            AgentRunConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(matches!(error, Error::Upstream(message) if message.contains("400")));
        let error = run_agent_with_config(
            ScriptedBackend::new(vec![]).as_ref(),
            &ToolRegistry::new(),
            "scripted",
            vec![ChatMessage::text("user", "deploy")],
            None,
            AgentRunConfig {
                interceptor: Some(Arc::new(FakeInterceptor {
                    turn_block: Some("not today".into()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, Error::InvalidRequest(message) if message == blocked_turn_message("not today"))
        );
    }

    #[tokio::test]
    async fn one_step_of_large_results_shares_the_replay_budget() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut registry = ToolRegistry::new();
        for name in ["read_a", "read_b", "read_c"] {
            registry.register(Arc::new(RecordingTool {
                name,
                runs: runs.clone(),
                result: json!({}),
                model_text: Some(
                    (0..1_000)
                        .map(|line| format!("{name} line {line:04} {}", "x".repeat(30)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            }));
        }
        registry.register(Arc::new(RecordingTool {
            name: "status",
            runs: runs.clone(),
            result: json!({"clean": true}),
            model_text: None,
        }));
        let backend = ScriptedBackend::new(vec![tool_step(
            &[
                ("call-a", "read_a", "{}"),
                ("call-b", "read_b", "{}"),
                ("call-s", "status", "{}"),
                ("call-c", "read_c", "{}"),
            ],
            "tool_calls",
        )]);
        let hook = Arc::new(RecordingHook::default());
        run_scripted(
            backend.clone(),
            registry,
            vec![ChatMessage::text("user", "read everything")],
            AgentRunConfig {
                step_hook: Some(hook.clone()),
                ..Default::default()
            },
        )
        .await;
        let sent = tool_messages(&backend.requests()[1]);
        assert_eq!(*hook.tool_contents.lock().unwrap(), sent);
        assert_eq!(sent[2], r#"{"clean":true}"#, "small results stay whole");
        let total = sent.iter().map(String::len).sum::<usize>();
        assert!(total <= tool_output::STEP_REPLAY_MAX_BYTES, "{total}");
        for (text, name) in [
            (&sent[0], "read_a"),
            (&sent[1], "read_b"),
            (&sent[3], "read_c"),
        ] {
            assert!(text.starts_with(&format!("{name} line 0000")));
            assert!(text.contains(&format!("{name} line 0999")));
            assert!(text.contains("omitted"));
        }
    }
}
