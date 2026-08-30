use super::*;

use serde::de::DeserializeOwned;

use crate::account_runtime_events::HarnessEvent;
use crate::routes::account_runtimes::AccountHarnessStream;

const HARNESS_EVENT_SCHEMA_VERSION: u8 = 1;
// ponytail: five seconds bounds post-terminal cleanup; Claude must exit cleanly to release its session lock.
const HARNESS_TERMINAL_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn harness_terminal_drain_timeout(harness_id: &str) -> Option<Duration> {
    (harness_id != "claude").then_some(HARNESS_TERMINAL_DRAIN_TIMEOUT)
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct HarnessRunRequest {
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) developer_instructions: Option<String>,
    #[serde(default)]
    pub(crate) images: Vec<crate::codex_bridge::AccountImage>,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) cwd: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(crate) native_session_id: Option<String>,
    #[serde(default)]
    pub(crate) persist_session: Option<bool>,
    #[serde(default)]
    pub(crate) tool_approval_policy: Option<String>,
    #[serde(default)]
    pub(crate) tool_approval_grant: bool,
    #[serde(default)]
    pub(crate) interactive_tool_approval: bool,
    #[serde(default)]
    pub(crate) plan_mode: bool,
    #[serde(default)]
    pub(crate) allow_session_recovery: bool,
    #[serde(default)]
    pub(crate) milim_context: Option<Value>,
}

#[derive(Clone, Copy)]
enum HarnessKind {
    Codex,
    Claude,
    OpenCode,
    Pi,
}

impl HarnessKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
        }
    }

    fn session_field(self) -> &'static str {
        match self {
            Self::Codex => "thread_id",
            Self::Claude | Self::OpenCode | Self::Pi => "session_id",
        }
    }

    fn persistence_field(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("persist_thread"),
            Self::Pi => Some("persist_session"),
            Self::Claude | Self::OpenCode => None,
        }
    }
}

pub(crate) fn account_harness_stream(
    st: &AppState,
    headers: &HeaderMap,
    id: &str,
    req: HarnessRunRequest,
) -> Result<AccountHarnessStream, ApiError> {
    let kind = HarnessKind::parse(id.trim()).ok_or_else(|| {
        ApiError(Error::InvalidRequest(format!(
            "Unknown harness '{}'",
            id.trim()
        )))
    })?;
    match kind {
        HarnessKind::Codex => codex_harness_stream(
            st,
            headers,
            native_request::<crate::codex_bridge::CodexRunRequest>(&req, kind)?,
        ),
        HarnessKind::Claude => claude_harness_stream(
            st,
            headers,
            native_request::<crate::claude_bridge::ClaudeRunRequest>(&req, kind)?,
        ),
        HarnessKind::OpenCode => opencode_harness_stream(
            st,
            headers,
            native_request::<crate::opencode_bridge::OpenCodeRunRequest>(&req, kind)?,
        ),
        HarnessKind::Pi => pi_harness_stream(
            st,
            headers,
            native_request::<crate::pi_bridge::PiRunRequest>(&req, kind)?,
        ),
    }
}

/// `POST /harnesses/{id}/run` - run any built-in account harness through one event contract.
pub(crate) async fn harness_run(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<HarnessRunRequest>,
) -> Result<Response, ApiError> {
    authorize(&st, &headers, peer_addr(peer))?;
    let kind = HarnessKind::parse(id.trim()).ok_or_else(|| {
        ApiError(Error::InvalidRequest(format!(
            "Unknown harness '{}'",
            id.trim()
        )))
    })?;
    let source = account_harness_stream(&st, &headers, kind.id(), req)?;
    Ok(Sse::new(harness_event_stream(
        kind.id(),
        uuid::Uuid::new_v4().to_string(),
        source,
    ))
    .keep_alive(KeepAlive::default())
    .into_response())
}

fn native_request<T>(request: &HarnessRunRequest, kind: HarnessKind) -> Result<T, ApiError>
where
    T: DeserializeOwned,
{
    let mut value = serde_json::to_value(request).map_err(|error| {
        ApiError(Error::InvalidRequest(format!(
            "Invalid {} harness request: {error}",
            kind.id()
        )))
    })?;
    let object = value
        .as_object_mut()
        .expect("HarnessRunRequest serializes as an object");
    if let Some(session_id) = object.remove("native_session_id") {
        object.insert(kind.session_field().to_string(), session_id);
    }
    if let Some(persist) = object.remove("persist_session") {
        if !persist.is_null() {
            if let Some(field) = kind.persistence_field() {
                object.insert(field.to_string(), persist);
            }
        }
    }
    serde_json::from_value(value).map_err(|error| {
        ApiError(Error::InvalidRequest(format!(
            "Invalid {} harness request: {error}",
            kind.id()
        )))
    })
}

fn harness_event_stream(
    harness_id: &'static str,
    run_id: String,
    mut source: AccountHarnessStream,
) -> impl futures::Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let mut seq = 0;
        while let Some(event) = source.next().await {
            let terminal = event.is_terminal();
            seq += 1;
            yield harness_sse(harness_envelope(harness_id, &run_id, seq, event));
            if terminal {
                let timeout = harness_terminal_drain_timeout(harness_id);
                tokio::spawn(async move {
                    let drain = async move {
                        while source.next().await.is_some() {}
                    };
                    if let Some(timeout) = timeout {
                        let _ = tokio::time::timeout(timeout, drain).await;
                    } else {
                        drain.await;
                    }
                });
                return;
            }
        }
    }
}

fn harness_envelope(harness_id: &str, run_id: &str, seq: u64, event: HarnessEvent) -> Value {
    json!({
        "schema_version": HARNESS_EVENT_SCHEMA_VERSION,
        "run_id": run_id,
        "seq": seq,
        "at_ms": now_unix_ms(),
        "harness_id": harness_id,
        "event": event,
    })
}

fn harness_sse(value: Value) -> std::result::Result<Event, Infallible> {
    Ok(Event::default().data(value.to_string()))
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_runtime_events::HarnessEventKind;

    fn event(kind: HarnessEventKind, fields: Value) -> HarnessEvent {
        HarnessEvent::new(kind, fields.as_object().cloned().unwrap_or_default())
    }

    #[test]
    fn common_request_maps_native_session_and_persistence_fields() {
        let request: HarnessRunRequest = serde_json::from_value(json!({
            "prompt": "hello",
            "developer_instructions": "Follow the Milim thread rules.",
            "model": "model",
            "native_session_id": "native-1",
            "persist_session": true,
            "reasoning_effort": "high",
            "tool_approval_policy": "review",
            "tool_approval_grant": true,
            "interactive_tool_approval": true,
            "plan_mode": true,
            "allow_session_recovery": true
        }))
        .unwrap();

        let codex =
            native_request::<crate::codex_bridge::CodexRunRequest>(&request, HarnessKind::Codex)
                .ok()
                .unwrap();
        assert_eq!(codex.thread_id.as_deref(), Some("native-1"));
        assert_eq!(
            codex.developer_instructions.as_deref(),
            Some("Follow the Milim thread rules.")
        );
        assert!(codex.persist_thread);
        assert!(codex.tool_approval_grant);
        assert!(codex.plan_mode);

        let pi = native_request::<crate::pi_bridge::PiRunRequest>(&request, HarnessKind::Pi)
            .ok()
            .unwrap();
        assert_eq!(pi.session_id.as_deref(), Some("native-1"));
        assert_eq!(pi.persist_session, Some(true));
        assert_eq!(pi.reasoning_effort.as_deref(), Some("high"));

        let claude =
            native_request::<crate::claude_bridge::ClaudeRunRequest>(&request, HarnessKind::Claude)
                .ok()
                .unwrap();
        assert_eq!(claude.session_id.as_deref(), Some("native-1"));
        assert!(claude.allow_session_recovery);

        let opencode = native_request::<crate::opencode_bridge::OpenCodeRunRequest>(
            &request,
            HarnessKind::OpenCode,
        )
        .ok()
        .unwrap();
        assert_eq!(opencode.session_id.as_deref(), Some("native-1"));
    }

    #[tokio::test]
    async fn typed_transformer_emits_one_terminal_and_drains_inner_stream() {
        let drained = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resumed = drained.clone();
        let release = Arc::new(tokio::sync::Notify::new());
        let resume = release.clone();
        let source: AccountHarnessStream = Box::pin(async_stream::stream! {
            yield event(HarnessEventKind::TurnCompleted, json!({"status":"completed"}));
            resume.notified().await;
            resumed.store(true, std::sync::atomic::Ordering::SeqCst);
            yield event(HarnessEventKind::TextDelta, json!({"text":"late"}));
        });
        let response =
            Sse::new(harness_event_stream("codex", "run-1".into(), source)).into_response();
        let bytes = tokio::time::timeout(
            Duration::from_secs(1),
            axum::body::to_bytes(response.into_body(), 1024 * 1024),
        )
        .await
        .unwrap()
        .unwrap();
        let output = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(!drained.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(output.matches("\"type\":\"turn_completed\"").count(), 1);
        assert!(!output.contains("late"));
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !drained.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[test]
    fn claude_terminal_cleanup_is_not_timed_out() {
        assert_eq!(harness_terminal_drain_timeout("claude"), None);
        assert_eq!(
            harness_terminal_drain_timeout("codex"),
            Some(HARNESS_TERMINAL_DRAIN_TIMEOUT)
        );
    }

    #[test]
    #[ignore = "manual before/after bridge benchmark"]
    fn benchmark_harness_event_conversion() {
        const ITERATIONS: usize = 20_000;
        let events = [
            event(
                HarnessEventKind::SessionEstablished,
                json!({"native_session_id":"thread-1","model":"gpt"}),
            ),
            event(
                HarnessEventKind::TurnStarted,
                json!({"native_session_id":"thread-1","native_turn_id":"turn-1"}),
            ),
            event(HarnessEventKind::TextDelta, json!({"text":"hello"})),
            event(HarnessEventKind::ReasoningDelta, json!({"text":"thinking"})),
            event(
                HarnessEventKind::ToolStarted,
                json!({"id":"call-1","name":"shell","status":"running"}),
            ),
            event(
                HarnessEventKind::ToolFinished,
                json!({"id":"call-1","name":"shell","status":"completed","result":"ok"}),
            ),
            event(
                HarnessEventKind::UsageUpdated,
                json!({"usage":{"total_tokens":3}}),
            ),
            event(
                HarnessEventKind::TurnCompleted,
                json!({"status":"completed","usage":{"total_tokens":3}}),
            ),
        ];

        let started = std::time::Instant::now();
        let mut output_events = 0usize;
        let mut output_bytes = 0usize;
        for iteration in 0..ITERATIONS {
            let run_id = format!("run-{iteration}");
            for (index, event) in events.iter().cloned().enumerate() {
                let envelope = harness_envelope("codex", &run_id, index as u64 + 1, event);
                output_events += 1;
                output_bytes += serde_json::to_vec(&envelope).unwrap().len();
            }
        }
        let elapsed = started.elapsed();
        eprintln!(
            "bridge_benchmark iterations={ITERATIONS} events={output_events} bytes={output_bytes} elapsed_ns={} ns_per_event={:.2}",
            elapsed.as_nanos(),
            elapsed.as_nanos() as f64 / output_events as f64,
        );
        assert_eq!(output_events, ITERATIONS * events.len());
    }
}
