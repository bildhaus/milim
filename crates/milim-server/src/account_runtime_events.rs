use std::collections::HashMap;

use axum::response::sse::Event;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HarnessEventKind {
    SessionEstablished,
    TurnStarted,
    TextDelta,
    ReasoningDelta,
    ToolStarted,
    ToolUpdated,
    ToolFinished,
    ApprovalRequested,
    ApprovalStatus,
    ApprovalResolved,
    ApprovalFailed,
    UsageUpdated,
    LimitUpdated,
    ImageGenerated,
    NativeWorkerUpdated,
    RuntimeNotice,
    SessionRecoveryRequired,
    TurnCompleted,
    TurnFailed,
    TurnCancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HarnessEvent {
    #[serde(rename = "type")]
    kind: HarnessEventKind,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

impl HarnessEvent {
    pub(crate) fn new(kind: HarnessEventKind, mut fields: Map<String, Value>) -> Self {
        fields.retain(|_, value| !value.is_null());
        Self { kind, fields }
    }

    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.kind,
            HarnessEventKind::TurnCompleted
                | HarnessEventKind::TurnFailed
                | HarnessEventKind::TurnCancelled
        )
    }

    pub(crate) fn legacy_value(&self, harness_id: &str) -> Value {
        let mut fields = self.fields.clone();
        let legacy_type = match self.kind {
            HarnessEventKind::SessionEstablished => {
                if let Some(session_id) = fields.remove("native_session_id") {
                    fields.insert(
                        if harness_id == "codex" {
                            "thread_id"
                        } else {
                            "session_id"
                        }
                        .to_string(),
                        session_id,
                    );
                }
                if harness_id == "codex" {
                    "thread"
                } else {
                    "session"
                }
            }
            HarnessEventKind::TurnStarted => {
                if let Some(session_id) = fields.remove("native_session_id") {
                    fields.insert(
                        if harness_id == "codex" {
                            "thread_id"
                        } else {
                            "session_id"
                        }
                        .to_string(),
                        session_id,
                    );
                }
                if let Some(turn_id) = fields.remove("native_turn_id") {
                    fields.insert("turn_id".to_string(), turn_id);
                }
                if harness_id == "codex" {
                    "start"
                } else {
                    "started"
                }
            }
            HarnessEventKind::TextDelta => "token",
            HarnessEventKind::ReasoningDelta => "reasoning",
            HarnessEventKind::ToolStarted
            | HarnessEventKind::ToolUpdated
            | HarnessEventKind::ToolFinished => "tool",
            HarnessEventKind::ApprovalRequested => "tool_approval_required",
            HarnessEventKind::ApprovalStatus => "tool_approval_status",
            HarnessEventKind::ApprovalResolved => "tool_approval_resolved",
            HarnessEventKind::ApprovalFailed => "tool_approval_failed",
            HarnessEventKind::UsageUpdated => {
                fields.insert("status".to_string(), Value::String("running".to_string()));
                "done"
            }
            HarnessEventKind::LimitUpdated => "rate_limit",
            HarnessEventKind::ImageGenerated => "image",
            HarnessEventKind::NativeWorkerUpdated => "native_worker",
            HarnessEventKind::RuntimeNotice => {
                if fields.get("code").and_then(Value::as_str) == Some("runtime_warning") {
                    fields.remove("kind");
                    fields.remove("level");
                    fields.remove("code");
                    "warning"
                } else {
                    if let Some(code) = fields.remove("code") {
                        fields.insert("kind".to_string(), code);
                    }
                    fields.remove("level");
                    "protocol_notice"
                }
            }
            HarnessEventKind::SessionRecoveryRequired => "session_recovery_required",
            HarnessEventKind::TurnCompleted | HarnessEventKind::TurnCancelled => {
                restore_native_ids(&mut fields, harness_id);
                "done"
            }
            HarnessEventKind::TurnFailed => {
                restore_native_ids(&mut fields, harness_id);
                "error"
            }
        };
        fields.insert("type".to_string(), Value::String(legacy_type.to_string()));
        Value::Object(fields)
    }
}

fn restore_native_ids(fields: &mut Map<String, Value>, harness_id: &str) {
    if let Some(session_id) = fields.remove("native_session_id") {
        fields.insert(
            if harness_id == "codex" {
                "thread_id"
            } else {
                "session_id"
            }
            .to_string(),
            session_id,
        );
    }
    if let Some(turn_id) = fields.remove("native_turn_id") {
        fields.insert("turn_id".to_string(), turn_id);
    }
}

pub(crate) struct HarnessEventAdapter {
    active_tools: HashMap<String, String>,
    native_session_id: Option<String>,
    native_turn_id: Option<String>,
    terminal: bool,
    warning: Option<String>,
    recovery: Option<String>,
}

impl HarnessEventAdapter {
    pub(crate) fn new(native_session_id: Option<String>) -> Self {
        Self {
            active_tools: HashMap::new(),
            native_session_id,
            native_turn_id: None,
            terminal: false,
            warning: None,
            recovery: None,
        }
    }

    pub(crate) fn map_legacy(&mut self, value: Value) -> Vec<HarnessEvent> {
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
                let mut fields = event_fields(&value);
                let session_id = take_string(&mut fields, "thread_id")
                    .or_else(|| take_string(&mut fields, "session_id"))
                    .or_else(|| self.native_session_id.clone())
                    .unwrap_or_default();
                self.native_session_id = Some(session_id.clone());
                fields.insert("native_session_id".to_string(), Value::String(session_id));
                vec![HarnessEvent::new(
                    HarnessEventKind::SessionEstablished,
                    fields,
                )]
            }
            "start" | "started" => {
                let mut fields = event_fields(&value);
                let session_id = take_string(&mut fields, "thread_id")
                    .or_else(|| take_string(&mut fields, "session_id"))
                    .or_else(|| self.native_session_id.clone());
                let turn_id = take_string(&mut fields, "turn_id")
                    .or_else(|| take_string(&mut fields, "external_turn_id"));
                if let Some(session_id) = session_id {
                    self.native_session_id = Some(session_id.clone());
                    fields.insert("native_session_id".to_string(), Value::String(session_id));
                }
                if let Some(turn_id) = turn_id {
                    self.native_turn_id = Some(turn_id.clone());
                    fields.insert("native_turn_id".to_string(), Value::String(turn_id));
                }
                vec![HarnessEvent::new(HarnessEventKind::TurnStarted, fields)]
            }
            "token" => vec![HarnessEvent::new(
                HarnessEventKind::TextDelta,
                event_fields(&value),
            )],
            "reasoning" => vec![HarnessEvent::new(
                HarnessEventKind::ReasoningDelta,
                event_fields(&value),
            )],
            "tool" => vec![self.map_tool(value)],
            "tool_approval_required" => vec![HarnessEvent::new(
                HarnessEventKind::ApprovalRequested,
                event_fields(&value),
            )],
            "tool_approval_status" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "decision", "deny");
                vec![HarnessEvent::new(HarnessEventKind::ApprovalStatus, fields)]
            }
            "tool_approval_resolved" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "decision", "deny");
                vec![HarnessEvent::new(
                    HarnessEventKind::ApprovalResolved,
                    fields,
                )]
            }
            "tool_approval_failed" => vec![HarnessEvent::new(
                HarnessEventKind::ApprovalFailed,
                event_fields(&value),
            )],
            "rate_limit" => vec![HarnessEvent::new(
                HarnessEventKind::LimitUpdated,
                event_fields(&value),
            )],
            "image" => {
                let mut fields = event_fields(&value);
                ensure_string(&mut fields, "url", "");
                vec![HarnessEvent::new(HarnessEventKind::ImageGenerated, fields)]
            }
            "native_worker" => vec![HarnessEvent::new(
                HarnessEventKind::NativeWorkerUpdated,
                event_fields(&value),
            )],
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
                vec![HarnessEvent::new(
                    HarnessEventKind::SessionRecoveryRequired,
                    fields,
                )]
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
                vec![HarnessEvent::new(HarnessEventKind::TurnFailed, fields)]
            }
            other => vec![self.runtime_notice(
                "warning",
                format!("Harness emitted unsupported event '{other}'"),
                Some("unsupported_event".to_string()),
                None,
            )],
        }
    }

    fn map_tool(&mut self, value: Value) -> HarnessEvent {
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
        let kind = if terminal {
            self.active_tools.remove(&id);
            HarnessEventKind::ToolFinished
        } else if self.active_tools.insert(id, name).is_some() {
            HarnessEventKind::ToolUpdated
        } else {
            HarnessEventKind::ToolStarted
        };
        HarnessEvent::new(kind, fields)
    }

    fn map_done(&mut self, value: Value) -> Vec<HarnessEvent> {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed");
        if status == "running" {
            let mut fields = Map::new();
            if let Some(usage) = value.get("usage").filter(|value| !value.is_null()) {
                fields.insert("usage".to_string(), usage.clone());
            }
            if let Some(cost) = value.get("cost_usd").filter(|value| !value.is_null()) {
                fields.insert("cost_usd".to_string(), cost.clone());
            }
            return (!fields.is_empty())
                .then(|| HarnessEvent::new(HarnessEventKind::UsageUpdated, fields))
                .into_iter()
                .collect();
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
        let kind = if cancelled_status(status) {
            HarnessEventKind::TurnCancelled
        } else if failed_status(status) {
            fields
                .entry("message".to_string())
                .or_insert_with(|| Value::String("Harness turn failed".to_string()));
            HarnessEventKind::TurnFailed
        } else {
            HarnessEventKind::TurnCompleted
        };
        vec![HarnessEvent::new(kind, fields)]
    }

    fn runtime_notice(
        &self,
        level: &str,
        message: String,
        code: Option<String>,
        detail: Option<String>,
    ) -> HarnessEvent {
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
        HarnessEvent::new(HarnessEventKind::RuntimeNotice, fields)
    }

    pub(crate) fn finish_eof(&mut self) -> Vec<HarnessEvent> {
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

    pub(crate) fn fail(&mut self, code: &str, message: String) -> Vec<HarnessEvent> {
        if self.terminal {
            return Vec::new();
        }
        let mut fields = Map::new();
        fields.insert("message".to_string(), Value::String(message));
        fields.insert("code".to_string(), Value::String(code.to_string()));
        self.add_native_ids(&mut fields);
        self.terminal = true;
        vec![HarnessEvent::new(HarnessEventKind::TurnFailed, fields)]
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
}

pub(crate) fn canonicalize_runtime_stream<S>(
    stream: S,
    initial_session_id: Option<String>,
) -> impl Stream<Item = HarnessEvent>
where
    S: Stream<Item = Value>,
{
    async_stream::stream! {
        let mut adapter = HarnessEventAdapter::new(initial_session_id);
        futures::pin_mut!(stream);
        while let Some(value) = stream.next().await {
            for event in adapter.map_legacy(value) {
                yield event;
            }
        }
        for event in adapter.finish_eof() {
            yield event;
        }
    }
}

pub(crate) fn legacy_sse_stream<S>(
    harness_id: &'static str,
    stream: S,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>>
where
    S: Stream<Item = HarnessEvent>,
{
    async_stream::stream! {
        futures::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            yield Ok(Event::default().data(event.legacy_value(harness_id).to_string()));
        }
        yield Ok(Event::default().data("[DONE]"));
    }
}

pub(crate) fn serialize_runtime_event<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|error| {
        json!({
            "type": "error",
            "message": format!("Harness event serialization failed: {error}")
        })
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_round_trip_preserves_runtime_event_shapes() {
        let fixtures = [
            (
                "codex",
                json!({"type":"thread","thread_id":"thread-1","model":"gpt"}),
                "thread",
            ),
            (
                "claude",
                json!({"type":"session","session_id":"session-1","model":"sonnet"}),
                "session",
            ),
            (
                "pi",
                json!({"type":"tool","id":"call-1","name":"shell","status":"completed"}),
                "tool",
            ),
        ];
        for (harness_id, input, expected_type) in fixtures {
            let mut adapter = HarnessEventAdapter::new(None);
            let event = adapter.map_legacy(input).remove(0);
            assert_eq!(event.legacy_value(harness_id)["type"], expected_type);
        }
    }

    #[test]
    fn tool_phases_follow_active_call_ids() {
        let mut adapter = HarnessEventAdapter::new(None);
        let kinds = [
            json!({"type":"tool","id":"call-1","name":"shell","status":"running"}),
            json!({"type":"tool","id":"call-1","status":"in_progress"}),
            json!({"type":"tool","id":"call-1","status":"completed"}),
            json!({"type":"tool","id":"call-1","name":"shell","status":"pending"}),
        ]
        .into_iter()
        .map(|value| adapter.map_legacy(value).remove(0).kind)
        .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                HarnessEventKind::ToolStarted,
                HarnessEventKind::ToolUpdated,
                HarnessEventKind::ToolFinished,
                HarnessEventKind::ToolStarted,
            ]
        );
    }

    #[test]
    fn terminalization_is_exactly_once_and_fail_closed() {
        let mut completed = HarnessEventAdapter::new(Some("session-1".into()));
        let terminal = completed.map_legacy(json!({"type":"done","status":"completed"}));
        assert_eq!(terminal[0].kind, HarnessEventKind::TurnCompleted);
        assert!(completed
            .map_legacy(json!({"type":"error","message":"late"}))
            .is_empty());
        assert!(completed.finish_eof().is_empty());

        let mut missing = HarnessEventAdapter::new(Some("session-2".into()));
        let terminal = missing.finish_eof();
        assert_eq!(terminal[0].kind, HarnessEventKind::TurnFailed);
        assert_eq!(
            terminal[0].fields.get("code"),
            Some(&Value::String("missing_stream".into()))
        );
        assert_eq!(
            terminal[0].fields.get("native_session_id"),
            Some(&Value::String("session-2".into()))
        );
    }
}
