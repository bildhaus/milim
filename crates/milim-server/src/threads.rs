//! Runtime supervision for Milim child agent threads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::{Stream, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use milim_agents::{
    thread_status_terminal, AgentEvent, AgentThread, ThreadEvent, ThreadStore, WorkerAccess,
    WorkerRun, WorkerRunStatus, WorkerRuntime, THREAD_STATUS_DONE, THREAD_STATUS_ERROR,
    THREAD_STATUS_RUNNING, THREAD_STATUS_STOPPED,
};
use milim_core::api::openai::ChatMessage;
use milim_core::{Error, Result};
use milim_inference::SharedService;
use milim_tools::ToolRegistry;

const TOKEN_EVENT_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

pub type ChildAgentStream = std::pin::Pin<Box<dyn Stream<Item = AgentEvent> + Send + 'static>>;
pub type ChildStreamFactory =
    Arc<dyn Fn(ChildRunSpec) -> Result<ChildAgentStream> + Send + Sync + 'static>;

#[derive(Clone)]
enum ChildRunSource {
    Agent {
        service: SharedService,
        tools: Arc<ToolRegistry>,
    },
    Stream(ChildStreamFactory),
}

#[derive(Clone)]
pub struct ThreadSupervisor {
    store: Arc<ThreadStore>,
    handles: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    events: broadcast::Sender<SupervisorEvent>,
}

#[derive(Debug, Clone)]
pub struct ChildRunSpec {
    pub parent_id: String,
    pub title: String,
    pub model: String,
    pub agent_id: Option<String>,
    pub system_prompt: Option<String>,
    pub prompt: String,
    pub run_id: Option<String>,
    pub runtime: WorkerRuntime,
    pub access: WorkerAccess,
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SupervisorEvent {
    ChildThreadStarted {
        thread: AgentThread,
    },
    ChildThreadDone {
        thread: AgentThread,
    },
    ChildThreadError {
        thread: AgentThread,
        message: String,
    },
    ChildThreadStopped {
        thread: AgentThread,
        message: String,
    },
    ChildThreadEvent {
        thread: AgentThread,
        event: ThreadEvent,
    },
}

impl SupervisorEvent {
    pub fn thread(&self) -> &AgentThread {
        match self {
            SupervisorEvent::ChildThreadStarted { thread }
            | SupervisorEvent::ChildThreadDone { thread }
            | SupervisorEvent::ChildThreadError { thread, .. }
            | SupervisorEvent::ChildThreadStopped { thread, .. }
            | SupervisorEvent::ChildThreadEvent { thread, .. } => thread,
        }
    }
}

impl ThreadSupervisor {
    pub fn new(store: ThreadStore) -> Self {
        if let Err(err) =
            store.update_non_terminal_status(THREAD_STATUS_ERROR, Some("interrupted by restart"))
        {
            tracing::warn!("failed to sweep interrupted child threads: {err}");
        }
        if let Err(err) = store.update_non_terminal_worker_runs("interrupted by restart") {
            tracing::warn!("failed to sweep interrupted worker runs: {err}");
        }
        let (events, _) = broadcast::channel(512);
        Self {
            store: Arc::new(store),
            handles: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    pub fn store(&self) -> Arc<ThreadStore> {
        self.store.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SupervisorEvent> {
        self.events.subscribe()
    }

    pub fn spawn(
        &self,
        service: SharedService,
        tools: ToolRegistry,
        spec: ChildRunSpec,
    ) -> Result<AgentThread> {
        self.spawn_batch(service, tools, vec![spec])?
            .pop()
            .ok_or_else(|| Error::Other("worker batch created no worker".to_string()))
    }

    pub fn spawn_batch(
        &self,
        service: SharedService,
        tools: ToolRegistry,
        specs: Vec<ChildRunSpec>,
    ) -> Result<Vec<AgentThread>> {
        self.spawn_batch_from(
            ChildRunSource::Agent {
                service,
                tools: Arc::new(tools),
            },
            specs,
        )
    }

    pub fn spawn_stream(
        &self,
        factory: ChildStreamFactory,
        spec: ChildRunSpec,
    ) -> Result<AgentThread> {
        self.spawn_batch_from(ChildRunSource::Stream(factory), vec![spec])?
            .pop()
            .ok_or_else(|| Error::Other("worker batch created no worker".to_string()))
    }

    fn spawn_batch_from(
        &self,
        source: ChildRunSource,
        specs: Vec<ChildRunSpec>,
    ) -> Result<Vec<AgentThread>> {
        const MAX_ACTIVE_CHILDREN: usize = 16;
        const MAX_ACTIVE_CHILDREN_PER_PARENT: usize = 4;
        if specs.is_empty() || specs.len() > MAX_ACTIVE_CHILDREN_PER_PARENT {
            return Err(Error::InvalidRequest(
                "worker runs require 1 to 4 tasks".to_string(),
            ));
        }
        let parent_id = specs[0].parent_id.clone();
        if specs.iter().any(|spec| spec.parent_id != parent_id) {
            return Err(Error::InvalidRequest(
                "all workers in a run must share one parent thread".to_string(),
            ));
        }
        let mut active = self.handles.lock().expect("thread handles poisoned");
        if active.len() + specs.len() > MAX_ACTIVE_CHILDREN {
            return Err(Error::InvalidRequest(format!(
                "at most {MAX_ACTIVE_CHILDREN} child threads may run at once"
            )));
        }
        let parent_active = active
            .keys()
            .filter_map(|id| self.store.get(id).ok().flatten())
            .filter(|thread| thread.parent_id == parent_id)
            .count();
        if parent_active + specs.len() > MAX_ACTIVE_CHILDREN_PER_PARENT {
            return Err(Error::InvalidRequest(format!(
                "at most {MAX_ACTIVE_CHILDREN_PER_PARENT} child threads may run for one parent"
            )));
        }
        let mut workers = Vec::with_capacity(specs.len());
        for spec in specs {
            let thread = self.store.create_worker(
                &spec.parent_id,
                &spec.title,
                &spec.model,
                spec.agent_id.as_deref(),
                &spec.prompt,
                spec.run_id.as_deref(),
                spec.runtime,
                spec.access,
            )?;
            let thread = if let Some(path) = spec.worktree_path.as_deref() {
                self.store
                    .update_worker_worktree(&thread.id, path)?
                    .unwrap_or(thread)
            } else {
                thread
            };
            let thread = self
                .store
                .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)?
                .unwrap_or(thread);
            if let Ok(event) = self.store.append_event(&thread.id, "started", json!({})) {
                let _ = self.events.send(SupervisorEvent::ChildThreadEvent {
                    thread: thread.clone(),
                    event,
                });
            }
            let _ = self.events.send(SupervisorEvent::ChildThreadStarted {
                thread: thread.clone(),
            });
            let id = thread.id.clone();
            let task_id = id.clone();
            let task_thread = thread.clone();
            let store = self.store.clone();
            let handles = self.handles.clone();
            let events = self.events.clone();
            let source = source.clone();
            let handle = tokio::spawn(async move {
                let stream = match &source {
                    ChildRunSource::Agent { service, tools } => {
                        agent_child_stream(service.clone(), tools.clone(), spec.clone())
                    }
                    ChildRunSource::Stream(factory) => match factory(spec.clone()) {
                        Ok(stream) => stream,
                        Err(error) => Box::pin(futures::stream::once(async move {
                            AgentEvent::Error {
                                message: error.to_string(),
                            }
                        })),
                    },
                };
                run_child_stream(store, events, stream, task_thread, spec).await;
                let _ = handles.lock().map(|mut h| h.remove(&task_id));
            });
            active.insert(id, handle);
            workers.push(thread);
        }
        Ok(workers)
    }

    pub fn get(&self, id: &str) -> Result<Option<AgentThread>> {
        self.store.get(id)
    }

    pub fn children(
        &self,
        parent_id: &str,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentThread>> {
        self.store.list_children(parent_id, status, limit)
    }

    pub fn events(&self, thread_id: &str, limit: usize) -> Result<Vec<ThreadEvent>> {
        self.store.events(thread_id, limit)
    }

    pub fn events_after(
        &self,
        thread_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<ThreadEvent>> {
        self.store.events_after(thread_id, after_seq, limit)
    }

    pub fn child_events_after(
        &self,
        parent_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(AgentThread, ThreadEvent)>> {
        self.store.child_events_after(parent_id, after_seq, limit)
    }

    pub fn event_count(&self, thread_id: &str) -> Result<usize> {
        self.store.event_count(thread_id)
    }

    pub fn worker_run(&self, id: &str) -> Result<Option<WorkerRun>> {
        self.store.get_worker_run(id)
    }

    pub fn worker_runs(&self, parent_id: &str, limit: usize) -> Result<Vec<WorkerRun>> {
        self.store.list_worker_runs(parent_id, limit)
    }

    pub fn workers_for_run(&self, run_id: &str) -> Result<Vec<AgentThread>> {
        self.store.workers_for_run(run_id)
    }

    pub fn worker_events_after(
        &self,
        run_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(AgentThread, ThreadEvent)>> {
        self.store.worker_events_after(run_id, after_seq, limit)
    }

    pub async fn wait_run(&self, run_id: &str, timeout_ms: u64) -> Result<Option<WorkerRun>> {
        let mut events = self.subscribe();
        let timeout = tokio::time::sleep(Duration::from_millis(timeout_ms));
        tokio::pin!(timeout);
        loop {
            let run = self.store.refresh_worker_run_status(run_id)?;
            if run
                .as_ref()
                .map(|run| {
                    !matches!(
                        run.status,
                        WorkerRunStatus::Proposed | WorkerRunStatus::Running
                    )
                })
                .unwrap_or(true)
            {
                return Ok(run);
            }
            tokio::select! {
                _ = &mut timeout => return self.store.get_worker_run(run_id),
                received = events.recv() => match received {
                    Ok(event) if event.thread().run_id.as_deref() == Some(run_id) => {},
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return self.store.get_worker_run(run_id),
                }
            }
        }
    }

    pub fn stop_run(&self, run_id: &str, message: &str) -> Result<Option<WorkerRun>> {
        for worker in self.store.workers_for_run(run_id)? {
            if !thread_status_terminal(&worker.status) {
                self.stop(&worker.id)?;
            }
        }
        self.store
            .update_worker_run_status(run_id, WorkerRunStatus::Stopped, Some(message))
    }

    pub async fn wait(&self, id: &str, timeout_ms: u64) -> Result<Option<AgentThread>> {
        let mut events = self.subscribe();
        let current = self.store.get(id)?;
        if current
            .as_ref()
            .map(|t| thread_status_terminal(&t.status))
            .unwrap_or(true)
        {
            return Ok(current);
        }

        let timeout = tokio::time::sleep(Duration::from_millis(timeout_ms));
        tokio::pin!(timeout);
        loop {
            tokio::select! {
                _ = &mut timeout => return self.store.get(id),
                received = events.recv() => {
                    match received {
                        Ok(event) => {
                            if event.thread().id == id && thread_status_terminal(&event.thread().status) {
                                return Ok(Some(event.thread().clone()));
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let current = self.store.get(id)?;
                            if current
                                .as_ref()
                                .map(|t| thread_status_terminal(&t.status))
                                .unwrap_or(true)
                            {
                                return Ok(current);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return self.store.get(id),
                    }
                }
            }
        }
    }

    pub fn stop(&self, id: &str) -> Result<Option<AgentThread>> {
        if let Some(handle) = self
            .handles
            .lock()
            .expect("thread handles poisoned")
            .remove(id)
        {
            handle.abort();
        }
        let updated =
            self.store
                .update_status(id, THREAD_STATUS_STOPPED, None, Some("stopped by parent"))?;
        if let Some(thread) = updated.as_ref() {
            let _ = self.events.send(SupervisorEvent::ChildThreadStopped {
                thread: thread.clone(),
                message: thread
                    .error
                    .clone()
                    .unwrap_or_else(|| "stopped by parent".to_string()),
            });
            if let Some(run_id) = thread.run_id.as_deref() {
                let _ = self.store.refresh_worker_run_status(run_id);
            }
            return Ok(updated);
        }
        self.store.get(id)
    }

    pub fn stop_running_children(&self, message: &str) -> Result<usize> {
        let mut handles = self.handles.lock().expect("thread handles poisoned");
        for (_, handle) in handles.drain() {
            handle.abort();
        }
        drop(handles);
        self.store
            .update_running_status(THREAD_STATUS_STOPPED, Some(message))
    }

    pub fn delete_tree(&self, id: &str) -> Result<Vec<String>> {
        let ids = self.store.tree_ids(id)?;
        if ids.is_empty() {
            return Ok(ids);
        }
        let mut handles = self.handles.lock().expect("thread handles poisoned");
        for thread_id in &ids {
            if let Some(handle) = handles.remove(thread_id) {
                handle.abort();
            }
        }
        drop(handles);
        self.store.delete_tree(id)
    }
}

fn emit_thread_event(
    store: &ThreadStore,
    events: &broadcast::Sender<SupervisorEvent>,
    thread: &AgentThread,
    kind: &str,
    payload: Value,
) {
    if let Ok(event) = store.append_event(&thread.id, kind, payload) {
        let _ = events.send(SupervisorEvent::ChildThreadEvent {
            thread: thread.clone(),
            event,
        });
    }
}

fn flush_token_event(
    store: &ThreadStore,
    events: &broadcast::Sender<SupervisorEvent>,
    thread: &AgentThread,
    token_buffer: &mut String,
) {
    if token_buffer.is_empty() {
        return;
    }
    let text = std::mem::take(token_buffer);
    emit_thread_event(store, events, thread, "token", json!({ "text": text }));
}

fn agent_child_stream(
    service: SharedService,
    tools: Arc<ToolRegistry>,
    spec: ChildRunSpec,
) -> ChildAgentStream {
    let mut messages = Vec::new();
    if let Some(system_prompt) = spec
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.trim().is_empty())
    {
        messages.push(ChatMessage::text("system", system_prompt));
    }
    messages.push(ChatMessage::text(
        "system",
        "You are a Milim Worker. Complete only the delegated task and return a concise final report. Do not delegate more work.",
    ));
    messages.push(ChatMessage::text("user", &spec.prompt));
    Box::pin(milim_agents::run_agent_stream(
        service, tools, spec.model, messages, None,
    ))
}

async fn run_child_stream(
    store: Arc<ThreadStore>,
    events: broadcast::Sender<SupervisorEvent>,
    mut stream: ChildAgentStream,
    mut thread: AgentThread,
    spec: ChildRunSpec,
) {
    let mut text = String::new();
    let mut final_text = None;
    let mut token_buffer = String::new();
    let mut last_token_flush = Instant::now();

    while let Some(event) = stream.next().await {
        match event {
            AgentEvent::Token { text: chunk } => {
                text.push_str(&chunk);
                token_buffer.push_str(&chunk);
                if last_token_flush.elapsed() >= TOKEN_EVENT_FLUSH_INTERVAL {
                    flush_token_event(&store, &events, &thread, &mut token_buffer);
                    last_token_flush = Instant::now();
                }
            }
            AgentEvent::Reasoning { text } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                emit_thread_event(
                    &store,
                    &events,
                    &thread,
                    "reasoning",
                    json!({ "text": text }),
                );
            }
            AgentEvent::UsageDelta { usage } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                emit_thread_event(
                    &store,
                    &events,
                    &thread,
                    "usage_delta",
                    json!({ "usage": usage }),
                );
            }
            AgentEvent::ToolCall {
                call_id,
                name,
                arguments,
                mcp_app,
            } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                emit_thread_event(
                    &store,
                    &events,
                    &thread,
                    "tool_call",
                    json!({
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                        "mcp_app": mcp_app,
                    }),
                );
            }
            AgentEvent::ToolResult {
                call_id,
                name,
                result,
                mcp_app,
                mcp_app_result,
            } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                emit_thread_event(
                    &store,
                    &events,
                    &thread,
                    "tool_result",
                    json!({
                        "call_id": call_id,
                        "name": name,
                        "result": result,
                        "mcp_app": mcp_app,
                        "mcp_app_result": mcp_app_result,
                    }),
                );
            }
            AgentEvent::Final { content } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                final_text = Some(content.clone());
                emit_thread_event(
                    &store,
                    &events,
                    &thread,
                    "final",
                    json!({ "content": content }),
                );
            }
            AgentEvent::Done {
                iterations,
                stopped_at_limit,
                usage,
            } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                let summary = final_text
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| text.trim());
                match store.update_status(&thread.id, THREAD_STATUS_DONE, Some(summary), None) {
                    Ok(Some(done)) => thread = done,
                    Ok(None) => return,
                    Err(_) => return,
                }
                if let Ok(event) = store.append_event(
                    &thread.id,
                    "done",
                    json!({
                        "iterations": iterations,
                        "stopped_at_limit": stopped_at_limit,
                        "usage": usage
                    }),
                ) {
                    let _ = events.send(SupervisorEvent::ChildThreadEvent {
                        thread: thread.clone(),
                        event,
                    });
                }
                let _ = events.send(SupervisorEvent::ChildThreadDone { thread });
                if let Some(run_id) = spec.run_id.as_deref() {
                    let _ = store.refresh_worker_run_status(run_id);
                }
                return;
            }
            AgentEvent::Error { message } => {
                flush_token_event(&store, &events, &thread, &mut token_buffer);
                match store.update_status(&thread.id, THREAD_STATUS_ERROR, None, Some(&message)) {
                    Ok(Some(error_thread)) => thread = error_thread,
                    Ok(None) => return,
                    Err(_) => return,
                }
                if let Ok(event) =
                    store.append_event(&thread.id, "error", json!({ "message": message }))
                {
                    let _ = events.send(SupervisorEvent::ChildThreadEvent {
                        thread: thread.clone(),
                        event,
                    });
                }
                let _ = events.send(SupervisorEvent::ChildThreadError { thread, message });
                if let Some(run_id) = spec.run_id.as_deref() {
                    let _ = store.refresh_worker_run_status(run_id);
                }
                return;
            }
            AgentEvent::Start { .. }
            | AgentEvent::ToolApprovalRequired { .. }
            | AgentEvent::ToolApprovalResolved { .. }
            | AgentEvent::MemoryRegistered { .. }
            | AgentEvent::ChildThreadStarted { .. }
            | AgentEvent::ChildThreadDone { .. }
            | AgentEvent::ChildThreadError { .. }
            | AgentEvent::WorkerRunProposed { .. }
            | AgentEvent::WorkerRunStarted { .. }
            | AgentEvent::WorkerRunDone { .. }
            | AgentEvent::WorkerRunError { .. } => {}
        }
    }

    flush_token_event(&store, &events, &thread, &mut token_buffer);
    let message = "child thread ended without a terminal event";
    if let Ok(Some(error_thread)) =
        store.update_status(&thread.id, THREAD_STATUS_ERROR, None, Some(message))
    {
        let _ = events.send(SupervisorEvent::ChildThreadError {
            thread: error_thread,
            message: message.to_string(),
        });
    }
    if let Some(run_id) = spec.run_id.as_deref() {
        let _ = store.refresh_worker_run_status(run_id);
    }
}

pub fn missing_threads_error() -> Error {
    Error::InvalidRequest("child thread storage is not configured".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_storage::Database;

    fn supervisor() -> ThreadSupervisor {
        ThreadSupervisor::new(ThreadStore::new(Database::open_in_memory().unwrap()).unwrap())
    }

    #[tokio::test]
    async fn wait_returns_latest_thread_on_timeout() {
        let supervisor = supervisor();
        let thread = supervisor
            .store()
            .create("parent-1", "Worker", "test-echo", None, "work")
            .unwrap();
        supervisor
            .store()
            .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)
            .unwrap();

        let waited = supervisor.wait(&thread.id, 5).await.unwrap().unwrap();

        assert_eq!(waited.id, thread.id);
        assert_eq!(waited.status, THREAD_STATUS_RUNNING);
    }

    #[tokio::test]
    async fn wait_wakes_from_stopped_event() {
        let supervisor = supervisor();
        let thread = supervisor
            .store()
            .create("parent-1", "Worker", "test-echo", None, "work")
            .unwrap();
        supervisor
            .store()
            .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)
            .unwrap();
        let waiter = {
            let supervisor = supervisor.clone();
            let id = thread.id.clone();
            tokio::spawn(async move { supervisor.wait(&id, 10_000).await.unwrap().unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;

        supervisor.stop(&thread.id).unwrap();
        let stopped = waiter.await.unwrap();

        assert_eq!(stopped.status, THREAD_STATUS_STOPPED);
    }

    #[tokio::test]
    async fn custom_worker_stream_uses_the_same_supervisor_lifecycle() {
        let supervisor = supervisor();
        let factory: ChildStreamFactory = Arc::new(|_| {
            Ok(Box::pin(futures::stream::iter(vec![
                AgentEvent::Token {
                    text: "account result".to_string(),
                },
                AgentEvent::Final {
                    content: "account result".to_string(),
                },
                AgentEvent::Done {
                    iterations: 1,
                    stopped_at_limit: false,
                    usage: milim_core::api::openai::Usage::new(1, 2),
                },
            ])))
        });
        let worker = supervisor
            .spawn_stream(
                factory,
                ChildRunSpec {
                    parent_id: "parent-1".to_string(),
                    title: "Account Worker".to_string(),
                    model: "codex:gpt-5.6".to_string(),
                    agent_id: None,
                    system_prompt: None,
                    prompt: "work".to_string(),
                    run_id: None,
                    runtime: WorkerRuntime::Managed,
                    access: WorkerAccess::ReadOnly,
                    worktree_path: None,
                },
            )
            .unwrap();

        let done = supervisor.wait(&worker.id, 10_000).await.unwrap().unwrap();
        assert_eq!(done.status, THREAD_STATUS_DONE);
        assert_eq!(done.summary.as_deref(), Some("account result"));
        assert!(supervisor
            .events(&worker.id, 10)
            .unwrap()
            .iter()
            .any(|event| event.kind == "done"));
    }

    #[test]
    fn stop_emits_stopped_not_error_and_preserves_terminal_state() {
        let supervisor = supervisor();
        let mut events = supervisor.subscribe();
        let thread = supervisor
            .store()
            .create("parent-1", "Worker", "test-echo", None, "work")
            .unwrap();
        supervisor
            .store()
            .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)
            .unwrap();

        let stopped = supervisor.stop(&thread.id).unwrap().unwrap();
        let done_after_stop = supervisor
            .store()
            .update_status(&thread.id, THREAD_STATUS_DONE, Some("done"), None)
            .unwrap();
        let event = events.try_recv().unwrap();

        assert_eq!(stopped.status, THREAD_STATUS_STOPPED);
        assert!(done_after_stop.is_none());
        assert!(matches!(event, SupervisorEvent::ChildThreadStopped { .. }));
        assert_eq!(
            supervisor.store().get(&thread.id).unwrap().unwrap().status,
            THREAD_STATUS_STOPPED
        );
    }

    #[tokio::test]
    async fn lagged_worker_events_replay_all_stored_sequences() {
        let supervisor = supervisor();
        let store = supervisor.store();
        let run = store
            .create_worker_run(
                "parent-1",
                Some("turn-1"),
                milim_agents::DelegationPolicy::Ask,
                WorkerRuntime::Managed,
                vec![],
                None,
            )
            .unwrap();
        let worker = store
            .create_worker(
                "parent-1",
                "Worker",
                "test-echo",
                None,
                "work",
                Some(&run.id),
                WorkerRuntime::Managed,
                WorkerAccess::ReadOnly,
            )
            .unwrap();
        let mut live = supervisor.subscribe();
        for index in 0..2_505 {
            let event = store
                .append_event(&worker.id, "token", json!({ "index": index }))
                .unwrap();
            supervisor
                .events
                .send(SupervisorEvent::ChildThreadEvent {
                    thread: worker.clone(),
                    event,
                })
                .unwrap();
        }
        assert!(matches!(
            live.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));

        let mut cursor = 0;
        let mut sequences = Vec::with_capacity(2_505);
        loop {
            let page = supervisor
                .worker_events_after(&run.id, cursor, 1_000)
                .unwrap();
            if page.is_empty() {
                break;
            }
            cursor = page.last().unwrap().1.seq;
            sequences.extend(page.into_iter().map(|(_, event)| event.seq));
        }
        assert_eq!(sequences, (1..=2_505).collect::<Vec<_>>());
    }
}
