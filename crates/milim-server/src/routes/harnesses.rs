use super::*;

use serde::de::DeserializeOwned;
use serde_json::Map;

const HARNESS_EVENT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct HarnessRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<crate::codex_bridge::AccountImage>,
    pub model: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub native_session_id: Option<String>,
    #[serde(default)]
    pub persist_session: Option<bool>,
    #[serde(default)]
    pub tool_approval_policy: Option<String>,
    #[serde(default)]
    pub tool_approval_grant: bool,
    #[serde(default)]
    pub interactive_tool_approval: bool,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub allow_session_recovery: bool,
    #[serde(default)]
    pub milim_context: Option<Value>,
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

/// `POST /harnesses/{id}/run` - run any built-in account harness through one event contract.
pub(crate) async fn harness_run(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    peer: Peer,
    Json(req): Json<HarnessRunRequest>,
) -> Result<Response, ApiError> {
    let kind = HarnessKind::parse(id.trim()).ok_or_else(|| {
        ApiError(Error::InvalidRequest(format!(
            "Unknown harness '{}'",
            id.trim()
        )))
    })?;
    let initial_session_id = req.native_session_id.clone();
    let response = match kind {
        HarnessKind::Codex => {
            codex_run(
                State(st),
                headers,
                peer,
                Json(native_request::<crate::codex_bridge::CodexRunRequest>(
                    &req, kind,
                )?),
            )
            .await?
        }
        HarnessKind::Claude => {
            claude_run(
                State(st),
                headers,
                peer,
                Json(native_request::<crate::claude_bridge::ClaudeRunRequest>(
                    &req, kind,
                )?),
            )
            .await?
        }
        HarnessKind::OpenCode => {
            opencode_run(
                State(st),
                headers,
                peer,
                Json(native_request::<crate::opencode_bridge::OpenCodeRunRequest>(&req, kind)?),
            )
            .await?
        }
        HarnessKind::Pi => {
            pi_run(
                State(st),
                headers,
                peer,
                Json(native_request::<crate::pi_bridge::PiRunRequest>(
                    &req, kind,
                )?),
            )
            .await?
        }
    };

    let body = response.into_body();
    let mapper = HarnessEventMapper::new(
        kind.id(),
        uuid::Uuid::new_v4().to_string(),
        initial_session_id,
    );
    Ok(Sse::new(harness_event_stream(body, mapper))
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
    body: Body,
    mut mapper: HarnessEventMapper,
) -> impl futures::Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let mut source = body.into_data_stream();
        let mut buffer = Vec::new();
        while let Some(chunk) = source.next().await {
            match chunk {
                Ok(chunk) => buffer.extend_from_slice(&chunk),
                Err(error) => {
                    for envelope in mapper.fail(
                        "stream_error",
                        format!("Harness stream failed: {error}"),
                    ) {
                        yield harness_sse(envelope);
                    }
                    return;
                }
            }
            while let Some(frame) = take_sse_frame(&mut buffer) {
                let Some(data) = sse_data(&frame) else {
                    continue;
                };
                if data == "[DONE]" {
                    for envelope in mapper.finish_eof() {
                        yield harness_sse(envelope);
                    }
                    return;
                }
                if mapper.terminal {
                    continue;
                }
                let value = match serde_json::from_str::<Value>(&data) {
                    Ok(value) => value,
                    Err(error) => {
                        for envelope in mapper.fail(
                            "invalid_stream_event",
                            format!("Harness emitted invalid JSON: {error}"),
                        ) {
                            yield harness_sse(envelope);
                        }
                        continue;
                    }
                };
                for envelope in mapper.map_legacy(value) {
                    yield harness_sse(envelope);
                }
            }
        }

        if !buffer.is_empty() && !mapper.terminal {
            if let Some(data) = sse_data(&buffer) {
                if data != "[DONE]" {
                    match serde_json::from_str::<Value>(&data) {
                        Ok(value) => {
                            for envelope in mapper.map_legacy(value) {
                                yield harness_sse(envelope);
                            }
                        }
                        Err(error) => {
                            for envelope in mapper.fail(
                                "invalid_stream_event",
                                format!("Harness emitted invalid JSON: {error}"),
                            ) {
                                yield harness_sse(envelope);
                            }
                            return;
                        }
                    }
                }
            }
        }
        for envelope in mapper.finish_eof() {
            yield harness_sse(envelope);
        }
    }
}

fn harness_sse(value: Value) -> std::result::Result<Event, Infallible> {
    Ok(Event::default().data(value.to_string()))
}

fn take_sse_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    let (end, delimiter_len) = match (lf, crlf) {
        (Some(lf), Some(crlf)) if lf <= crlf => (lf, 2),
        (Some(_), Some(crlf)) => (crlf, 4),
        (Some(lf), None) => (lf, 2),
        (None, Some(crlf)) => (crlf, 4),
        (None, None) => return None,
    };
    let frame = buffer[..end].to_vec();
    buffer.drain(..end + delimiter_len);
    Some(frame)
}

fn sse_data(frame: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(frame).ok()?;
    let mut data = String::new();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some(value) = line.strip_prefix("data:") else {
            continue;
        };
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(value.strip_prefix(' ').unwrap_or(value));
    }
    (!data.is_empty()).then_some(data)
}

struct HarnessEventMapper {
    harness_id: String,
    run_id: String,
    seq: u64,
    terminal: bool,
    active_tools: HashMap<String, String>,
    native_session_id: Option<String>,
    native_turn_id: Option<String>,
    warning: Option<String>,
    recovery: Option<String>,
}

impl HarnessEventMapper {
    fn new(
        harness_id: impl Into<String>,
        run_id: impl Into<String>,
        native_session_id: Option<String>,
    ) -> Self {
        Self {
            harness_id: harness_id.into(),
            run_id: run_id.into(),
            seq: 0,
            terminal: false,
            active_tools: HashMap::new(),
            native_session_id,
            native_turn_id: None,
            warning: None,
            recovery: None,
        }
    }

    fn map_legacy(&mut self, value: Value) -> Vec<Value> {
        if self.terminal {
            return Vec::new();
        }
        let Some(legacy_type) = value.get("type").and_then(Value::as_str) else {
            return vec![self.runtime_notice(
                "warning",
                "Harness emitted an event without a type".to_string(),
                Some("missing_event_type".to_string()),
                None,
            )];
        };
        match legacy_type {
            "thread" | "session" => {
                let mut event = event_fields(&value);
                let session_id = take_string(&mut event, "thread_id")
                    .or_else(|| take_string(&mut event, "session_id"))
                    .or_else(|| self.native_session_id.clone())
                    .unwrap_or_default();
                self.native_session_id = Some(session_id.clone());
                event.insert("native_session_id".to_string(), Value::String(session_id));
                vec![self.envelope("session_established", event)]
            }
            "start" | "started" => {
                let mut event = event_fields(&value);
                let session_id = take_string(&mut event, "thread_id")
                    .or_else(|| take_string(&mut event, "session_id"))
                    .or_else(|| self.native_session_id.clone());
                let turn_id = take_string(&mut event, "turn_id")
                    .or_else(|| take_string(&mut event, "external_turn_id"));
                if let Some(session_id) = session_id {
                    self.native_session_id = Some(session_id.clone());
                    event.insert("native_session_id".to_string(), Value::String(session_id));
                }
                if let Some(turn_id) = turn_id {
                    self.native_turn_id = Some(turn_id.clone());
                    event.insert("native_turn_id".to_string(), Value::String(turn_id));
                }
                vec![self.envelope("turn_started", event)]
            }
            "token" => {
                vec![self.envelope("text_delta", event_fields(&value))]
            }
            "reasoning" => {
                vec![self.envelope("reasoning_delta", event_fields(&value))]
            }
            "tool" => vec![self.map_tool(value)],
            "tool_approval_required" => {
                vec![self.envelope("approval_requested", event_fields(&value))]
            }
            "tool_approval_status" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "decision", "deny");
                vec![self.envelope("approval_status", fields)]
            }
            "tool_approval_resolved" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "decision", "deny");
                vec![self.envelope("approval_resolved", fields)]
            }
            "tool_approval_failed" => {
                vec![self.envelope("approval_failed", event_fields(&value))]
            }
            "rate_limit" => vec![self.envelope("limit_updated", event_fields(&value))],
            "image" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "url", "");
                vec![self.envelope("image_generated", fields)]
            }
            "native_worker" => {
                vec![self.envelope("native_worker_updated", event_fields(&value))]
            }
            "protocol_notice" => {
                let mut fields = event_fields(&value);
                let kind = take_string(&mut fields, "kind")
                    .unwrap_or_else(|| "protocol_notice".to_string());
                let message = take_string(&mut fields, "message")
                    .unwrap_or_else(|| "Harness protocol notice".to_string());
                let detail = take_string(&mut fields, "detail");
                vec![self.runtime_notice("info", message, Some(kind), detail)]
            }
            "warning" => {
                let message = value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Harness runtime warning")
                    .to_string();
                self.warning = Some(message.clone());
                vec![self.runtime_notice(
                    "warning",
                    message,
                    Some("runtime_warning".to_string()),
                    None,
                )]
            }
            "session_recovery_required" => {
                let mut fields = event_fields(&value);
                let message = fields
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Harness session recovery is required")
                    .to_string();
                self.recovery = Some(message);
                self.add_native_ids(&mut fields);
                vec![self.envelope("session_recovery_required", fields)]
            }
            "done" => self.map_done(value),
            "error" => {
                let mut fields = event_fields(&value);
                let message = fields
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Harness turn failed")
                    .to_string();
                fields.insert("message".to_string(), Value::String(message));
                self.add_native_ids(&mut fields);
                self.terminal = true;
                vec![self.envelope("turn_failed", fields)]
            }
            other => {
                let message = format!("Harness emitted unsupported event '{other}'");
                vec![self.runtime_notice(
                    "warning",
                    message,
                    Some("unsupported_event".to_string()),
                    None,
                )]
            }
        }
    }

    fn map_tool(&mut self, value: Value) -> Value {
        let mut fields = event_fields(&value);
        let id = fields
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let raw_status = fields
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("running")
            .to_string();
        let (status, terminal) = canonical_tool_status(&raw_status);
        let prior_name = self.active_tools.get(&id).cloned();
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or(prior_name)
            .unwrap_or_else(|| "tool".to_string());
        fields.insert("id".to_string(), Value::String(id.clone()));
        fields.insert("name".to_string(), Value::String(name.clone()));
        fields.insert("status".to_string(), Value::String(status.to_string()));
        if terminal {
            self.active_tools.remove(&id);
            self.envelope("tool_finished", fields)
        } else if self.active_tools.insert(id, name).is_some() {
            self.envelope("tool_updated", fields)
        } else {
            self.envelope("tool_started", fields)
        }
    }

    fn map_done(&mut self, value: Value) -> Vec<Value> {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        if status == "running" {
            return self.usage_event(&value);
        }
        let mut fields = event_fields(&value);
        if let Some(thread_id) = take_string(&mut fields, "thread_id") {
            self.native_session_id = Some(thread_id);
        }
        if let Some(session_id) = take_string(&mut fields, "session_id") {
            self.native_session_id = Some(session_id);
        }
        if let Some(turn_id) = take_string(&mut fields, "turn_id") {
            self.native_turn_id = Some(turn_id);
        }
        self.add_native_ids(&mut fields);
        self.terminal = true;
        if cancelled_status(status) {
            vec![self.envelope("turn_cancelled", fields)]
        } else if failed_status(status) {
            fields
                .entry("message".to_string())
                .or_insert_with(|| Value::String("Harness turn failed".to_string()));
            vec![self.envelope("turn_failed", fields)]
        } else {
            vec![self.envelope("turn_completed", fields)]
        }
    }

    fn usage_event(&mut self, value: &Value) -> Vec<Value> {
        let mut fields = Map::new();
        if let Some(usage) = value.get("usage").filter(|value| !value.is_null()) {
            fields.insert("usage".to_string(), usage.clone());
        }
        if let Some(cost) = value.get("cost_usd").filter(|value| !value.is_null()) {
            fields.insert("cost_usd".to_string(), cost.clone());
        }
        if fields.is_empty() {
            Vec::new()
        } else {
            vec![self.envelope("usage_updated", fields)]
        }
    }

    fn runtime_notice(
        &mut self,
        level: &str,
        message: String,
        code: Option<String>,
        detail: Option<String>,
    ) -> Value {
        let mut fields = Map::new();
        fields.insert("kind".to_string(), Value::String(level.to_string()));
        fields.insert("level".to_string(), Value::String(level.to_string()));
        fields.insert("message".to_string(), Value::String(message));
        if let Some(code) = code {
            fields.insert("code".to_string(), Value::String(code));
        }
        if let Some(detail) = detail {
            fields.insert("detail".to_string(), Value::String(detail));
        }
        self.envelope("runtime_notice", fields)
    }

    fn finish_eof(&mut self) -> Vec<Value> {
        if self.terminal {
            return Vec::new();
        }
        if let Some(message) = self.recovery.clone() {
            return self.fail("session_recovery_required", message);
        }
        if let Some(message) = self.warning.clone() {
            return self.fail("runtime_warning", message);
        }
        self.fail(
            "missing_stream",
            "Harness stream ended without a terminal event".to_string(),
        )
    }

    fn fail(&mut self, code: &str, message: String) -> Vec<Value> {
        if self.terminal {
            return Vec::new();
        }
        let mut fields = Map::new();
        fields.insert("message".to_string(), Value::String(message));
        fields.insert("code".to_string(), Value::String(code.to_string()));
        self.add_native_ids(&mut fields);
        self.terminal = true;
        vec![self.envelope("turn_failed", fields)]
    }

    fn add_native_ids(&self, fields: &mut Map<String, Value>) {
        if let Some(session_id) = &self.native_session_id {
            fields
                .entry("native_session_id".to_string())
                .or_insert_with(|| Value::String(session_id.clone()));
        }
        if let Some(turn_id) = &self.native_turn_id {
            fields
                .entry("native_turn_id".to_string())
                .or_insert_with(|| Value::String(turn_id.clone()));
        }
    }

    fn envelope(&mut self, event_type: &str, mut fields: Map<String, Value>) -> Value {
        fields.retain(|_, value| !value.is_null());
        fields.insert("type".to_string(), Value::String(event_type.to_string()));
        self.seq += 1;
        json!({
            "schema_version": HARNESS_EVENT_SCHEMA_VERSION,
            "run_id": self.run_id,
            "seq": self.seq,
            "at_ms": now_unix_ms(),
            "harness_id": self.harness_id,
            "event": fields,
        })
    }
}

fn event_fields(value: &Value) -> Map<String, Value> {
    let mut fields = value.as_object().cloned().unwrap_or_default();
    fields.remove("type");
    fields.retain(|_, value| !value.is_null());
    fields
}

fn take_string(fields: &mut Map<String, Value>, key: &str) -> Option<String> {
    fields
        .remove(key)
        .and_then(|value| value.as_str().map(str::to_string))
}

fn ensure_string(fields: &mut Map<String, Value>, key: &str, fallback: &str) {
    if !fields.get(key).is_some_and(Value::is_string) {
        fields.insert(key.to_string(), Value::String(fallback.to_string()));
    }
}

fn canonical_tool_status(status: &str) -> (&'static str, bool) {
    match status.to_ascii_lowercase().as_str() {
        "completed" | "complete" | "done" | "succeeded" | "success" => ("completed", true),
        "failed" | "error" => ("failed", true),
        "cancelled" | "canceled" => ("cancelled", true),
        "denied" => ("denied", true),
        "pending" => ("pending", false),
        _ => ("running", false),
    }
}

fn cancelled_status(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "cancelled" | "canceled" | "aborted" | "stopped"
    )
}

fn failed_status(status: &str) -> bool {
    matches!(
        status.to_ascii_lowercase().as_str(),
        "failed" | "error" | "errored"
    )
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

    fn event_type(value: &Value) -> &str {
        value["event"]["type"].as_str().unwrap()
    }

    #[test]
    fn common_request_maps_native_session_and_persistence_fields() {
        let request: HarnessRunRequest = serde_json::from_value(json!({
            "prompt": "hello",
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

        let request_without_persistence: HarnessRunRequest = serde_json::from_value(json!({
            "prompt": "hello",
            "model": "model"
        }))
        .unwrap();
        let codex = native_request::<crate::codex_bridge::CodexRunRequest>(
            &request_without_persistence,
            HarnessKind::Codex,
        )
        .ok()
        .unwrap();
        assert!(!codex.persist_thread);
    }

    #[test]
    fn canonicalizes_legacy_event_families_without_legacy_tags() {
        let mut mapper = HarnessEventMapper::new("codex", "run-1", None);
        let inputs = [
            json!({"type":"thread","thread_id":"thread-1","model":"gpt"}),
            json!({"type":"start","thread_id":"thread-1","turn_id":"turn-1"}),
            json!({"type":"token","text":"hello"}),
            json!({"type":"reasoning","text":"thinking"}),
            json!({"type":"tool_approval_required","approval_id":"a","name":"shell","arguments":"{}","effect":"command"}),
            json!({"type":"tool_approval_status","approval_id":"a","decision":"approve","status":"delivered"}),
            json!({"type":"tool_approval_resolved","approval_id":"a","decision":"approve"}),
            json!({"type":"tool_approval_failed","approval_id":"b","message":"failed"}),
            json!({"type":"rate_limit","limit":{"provider":"claude"}}),
            json!({"type":"image","id":"image-1","status":"completed","url":"data:image/png;base64,AAAA"}),
            json!({"type":"native_worker","lifecycle":{"runtime":"codex","call_id":"worker","operation":"spawn","status":"running","worker_runtime_ids":[],"workers":[]}}),
            json!({"type":"protocol_notice","kind":"rerouted","message":"Model rerouted"}),
            json!({"type":"session_recovery_required","message":"resume required"}),
        ];
        let expected = [
            "session_established",
            "turn_started",
            "text_delta",
            "reasoning_delta",
            "approval_requested",
            "approval_status",
            "approval_resolved",
            "approval_failed",
            "limit_updated",
            "image_generated",
            "native_worker_updated",
            "runtime_notice",
            "session_recovery_required",
        ];

        let events = inputs
            .into_iter()
            .flat_map(|input| mapper.map_legacy(input))
            .collect::<Vec<_>>();
        assert_eq!(events.iter().map(event_type).collect::<Vec<_>>(), expected);
        assert_eq!(events[0]["event"]["native_session_id"], "thread-1");
        assert_eq!(events[1]["event"]["native_turn_id"], "turn-1");
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["seq"], u64::try_from(index + 1).unwrap());
        }
        let serialized = serde_json::to_string(&events).unwrap();
        for legacy in [
            "thread",
            "start",
            "token",
            "reasoning",
            "tool_approval_required",
            "rate_limit",
            "image",
            "native_worker",
            "protocol_notice",
        ] {
            assert!(!serialized.contains(&format!("\"type\":\"{legacy}\"")));
        }
    }

    #[test]
    fn tool_phases_follow_active_call_ids() {
        let mut mapper = HarnessEventMapper::new("pi", "run-1", None);
        let started = mapper
            .map_legacy(json!({"type":"tool","id":"call-1","name":"shell","status":"running"}));
        let updated = mapper
            .map_legacy(json!({"type":"tool","id":"call-1","name":"shell","status":"in_progress"}));
        let finished = mapper
            .map_legacy(json!({"type":"tool","id":"call-1","name":"shell","status":"completed"}));
        let restarted = mapper
            .map_legacy(json!({"type":"tool","id":"call-1","name":"shell","status":"pending"}));

        assert_eq!(event_type(&started[0]), "tool_started");
        assert_eq!(event_type(&updated[0]), "tool_updated");
        assert_eq!(event_type(&finished[0]), "tool_finished");
        assert_eq!(finished[0]["event"]["status"], "completed");
        assert_eq!(event_type(&restarted[0]), "tool_started");
        assert_eq!(restarted[0]["event"]["status"], "pending");

        for (raw, expected) in [
            ("success", "completed"),
            ("failed", "failed"),
            ("canceled", "cancelled"),
            ("denied", "denied"),
        ] {
            let mut mapper = HarnessEventMapper::new("codex", raw, None);
            let event =
                mapper.map_legacy(json!({"type":"tool","id":"call-1","name":"shell","status":raw}));
            assert_eq!(event_type(&event[0]), "tool_finished");
            assert_eq!(event[0]["event"]["status"], expected);
        }
    }

    #[test]
    fn emits_only_one_terminal_and_synthesizes_eof_outcomes() {
        let mut mapper = HarnessEventMapper::new("codex", "run-1", None);
        let completed = mapper
            .map_legacy(json!({"type":"done","status":"completed","usage":{"total_tokens":3}}));
        assert_eq!(completed.len(), 1);
        assert_eq!(event_type(&completed[0]), "turn_completed");
        assert_eq!(completed[0]["event"]["usage"]["total_tokens"], 3);
        assert!(mapper
            .map_legacy(json!({"type":"error","message":"late"}))
            .is_empty());
        assert!(mapper.finish_eof().is_empty());

        let mut cancelled = HarnessEventMapper::new("pi", "run-cancelled", None);
        let cancelled = cancelled.map_legacy(json!({"type":"done","status":"cancelled"}));
        assert_eq!(event_type(&cancelled[0]), "turn_cancelled");

        let mut usage_only = HarnessEventMapper::new("opencode", "run-2", None);
        let usage = usage_only
            .map_legacy(json!({"type":"done","status":"running","usage":{"total_tokens":2}}));
        assert_eq!(event_type(&usage[0]), "usage_updated");
        let failed = usage_only.finish_eof();
        assert_eq!(event_type(&failed[0]), "turn_failed");
        assert_eq!(failed[0]["event"]["code"], "missing_stream");

        let mut warning = HarnessEventMapper::new("pi", "run-3", None);
        warning.map_legacy(json!({"type":"warning","message":"runtime missing"}));
        let failed = warning.finish_eof();
        assert_eq!(event_type(&failed[0]), "turn_failed");
        assert_eq!(failed[0]["event"]["code"], "runtime_warning");

        let mut recovery = HarnessEventMapper::new("claude", "run-4", Some("session-1".into()));
        recovery
            .map_legacy(json!({"type":"session_recovery_required","message":"session is locked"}));
        let failed = recovery.finish_eof();
        assert_eq!(event_type(&failed[0]), "turn_failed");
        assert_eq!(failed[0]["event"]["code"], "session_recovery_required");

        let mut missing = HarnessEventMapper::new("opencode", "run-5", None);
        let failed = missing.finish_eof();
        assert_eq!(event_type(&failed[0]), "turn_failed");
        assert_eq!(failed[0]["event"]["code"], "missing_stream");
    }

    #[tokio::test]
    async fn transformer_drains_inner_stream_after_terminal() {
        let drained = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resumed = drained.clone();
        let source = async_stream::stream! {
            yield Ok::<Bytes, Infallible>(Bytes::from_static(
                b"data: {\"type\":\"done\",\"status\":\"completed\"}\n\n",
            ));
            resumed.store(true, std::sync::atomic::Ordering::SeqCst);
            yield Ok(Bytes::from_static(
                b"data: {\"type\":\"token\",\"text\":\"late\"}\n\n",
            ));
            yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
        };
        let body = Body::from_stream(source);
        let response = Sse::new(harness_event_stream(
            body,
            HarnessEventMapper::new("codex", "run-1", None),
        ))
        .into_response();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let output = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(drained.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(output.matches("\"type\":\"turn_completed\"").count(), 1);
        assert!(!output.contains("late"));
    }

    #[tokio::test]
    async fn transformer_drains_inner_stream_after_invalid_json() {
        let drained = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let resumed = drained.clone();
        let source = async_stream::stream! {
            yield Ok::<Bytes, Infallible>(Bytes::from_static(b"data: {\n\n"));
            resumed.store(true, std::sync::atomic::Ordering::SeqCst);
            yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
        };
        let response = Sse::new(harness_event_stream(
            Body::from_stream(source),
            HarnessEventMapper::new("codex", "run-1", None),
        ))
        .into_response();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let output = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(drained.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(output.matches("\"type\":\"turn_failed\"").count(), 1);
    }
}
