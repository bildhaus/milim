//! MCP client transports.
//!
//! - stdio: newline-delimited JSON-RPC over a child's stdin/stdout.
//! - Streamable HTTP (MCP 2025-03-26 / 2025-06-18): JSON-RPC POSTed to one
//!   endpoint, answered with JSON or an SSE stream, plus an optional GET SSE
//!   stream for server-initiated messages. Legacy HTTP+SSE servers (GET the
//!   stream, then POST to its `endpoint` event) are supported as a fallback.
//!
//! Every transport feeds one [`Link`], which demuxes responses by id, answers
//! server requests, forwards notifications to the hub, and records why the
//! connection closed so callers fail fast.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE,
};
use reqwest::{StatusCode, Url};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot, watch, Mutex};

use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};

/// Largest single JSON-RPC message accepted from a server.
pub(crate) const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const LEGACY_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(15);

/// Shared request/response state for one live connection.
pub(crate) struct Link {
    pending: StdMutex<HashMap<i64, oneshot::Sender<Value>>>,
    notifications: StdMutex<Option<mpsc::UnboundedSender<Value>>>,
    closed: watch::Sender<Option<String>>,
}

impl Link {
    pub(crate) fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<Value>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (closed, _) = watch::channel(None);
        (
            Arc::new(Self {
                pending: StdMutex::new(HashMap::new()),
                notifications: StdMutex::new(Some(tx)),
                closed,
            }),
            rx,
        )
    }

    pub(crate) fn register(&self, id: i64) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("mcp link poisoned")
            .insert(id, tx);
        rx
    }

    pub(crate) fn forget(&self, id: i64) {
        self.pending.lock().expect("mcp link poisoned").remove(&id);
    }

    pub(crate) fn closed_reason(&self) -> Option<String> {
        self.closed.borrow().clone()
    }

    /// Resolve once the connection closes, with the recorded reason.
    pub(crate) async fn wait_closed(&self) -> String {
        let mut rx = self.closed.subscribe();
        loop {
            if let Some(reason) = rx.borrow_and_update().clone() {
                return reason;
            }
            if rx.changed().await.is_err() {
                return "MCP connection closed".to_string();
            }
        }
    }

    /// Mark the connection dead: fail every in-flight request and stop
    /// forwarding notifications. Only the first reason is kept.
    pub(crate) fn close(&self, reason: impl Into<String>) {
        let reason = reason.into();
        self.closed.send_if_modified(|current| {
            if current.is_some() {
                return false;
            }
            *current = Some(reason.clone());
            true
        });
        self.notifications.lock().expect("mcp link poisoned").take();
        let error = json!({ "error": { "message": format!("MCP connection closed: {reason}") } });
        let pending = std::mem::take(&mut *self.pending.lock().expect("mcp link poisoned"));
        for tx in pending.into_values() {
            let _ = tx.send(error.clone());
        }
    }

    /// Route one inbound JSON-RPC message. Returns the reply owed to a
    /// server-initiated request, if any.
    pub(crate) fn dispatch(&self, message: Value) -> Option<Value> {
        if message.get("method").is_some() {
            if message.get("id").is_some() {
                return Some(server_request_response(&message));
            }
            if let Some(tx) = self
                .notifications
                .lock()
                .expect("mcp link poisoned")
                .as_ref()
            {
                let _ = tx.send(message);
            }
            return None;
        }
        if let Some(id) = message.get("id").and_then(Value::as_i64) {
            if let Some(tx) = self.pending.lock().expect("mcp link poisoned").remove(&id) {
                let _ = tx.send(message);
            }
        }
        None
    }
}

pub(crate) fn server_request_response(req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    match req.get("method").and_then(Value::as_str) {
        Some("ping") => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": "method not found" }
        }),
    }
}

/// Why a message could not be delivered.
#[derive(Debug)]
pub(crate) enum SendError {
    /// Non-success HTTP status (used to detect legacy HTTP+SSE servers).
    Status(StatusCode, String),
    /// The server requires (re-)authorization.
    Unauthorized(String),
    Other(Error),
}

impl From<SendError> for Error {
    fn from(error: SendError) -> Self {
        match error {
            SendError::Status(status, body) if body.is_empty() => {
                Error::Upstream(format!("MCP server returned HTTP {status}"))
            }
            SendError::Status(status, body) => {
                Error::Upstream(format!("MCP server returned HTTP {status}: {body}"))
            }
            SendError::Unauthorized(message) => Error::Unauthorized(message),
            SendError::Other(error) => error,
        }
    }
}

impl From<Error> for SendError {
    fn from(error: Error) -> Self {
        SendError::Other(error)
    }
}

// ----- stdio -----

pub(crate) struct StdioTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    // Held so the child and its descendants stay alive with the client.
    _tree: ProcessTreeGuard,
    _child: Mutex<Child>,
}

impl StdioTransport {
    pub(crate) fn start(
        mut child: Child,
        tree: ProcessTreeGuard,
        link: &Arc<Link>,
    ) -> Result<Self> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("MCP child has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("MCP child has no stdout".into()))?;
        let stdin = Arc::new(Mutex::new(stdin));
        tokio::spawn(read_stdio(stdout, link.clone(), stdin.clone()));
        Ok(Self {
            stdin,
            _tree: tree,
            _child: Mutex::new(child),
        })
    }

    pub(crate) async fn send(&self, message: &Value) -> Result<()> {
        write_json(&self.stdin, message).await
    }
}

async fn read_stdio(stdout: ChildStdout, link: Arc<Link>, stdin: Arc<Mutex<ChildStdin>>) {
    let mut reader = BufReader::new(stdout);
    let reason = loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(MAX_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut line)
            .await;
        let Ok(read) = read else {
            break "MCP server stdout failed".to_string();
        };
        if read == 0 {
            break "MCP server process exited".to_string();
        }
        if line.len() > MAX_MESSAGE_BYTES || !line.ends_with(b"\n") {
            break "MCP server sent an oversized or truncated message".to_string();
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if let Some(reply) = link.dispatch(message) {
            let _ = write_json(&stdin, &reply).await;
        }
    };
    link.close(reason);
}

async fn write_json(stdin: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<()> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    let mut w = stdin.lock().await;
    w.write_all(line.as_bytes())
        .await
        .map_err(|e| Error::Other(format!("MCP write failed: {e}")))?;
    w.flush()
        .await
        .map_err(|e| Error::Other(format!("MCP flush failed: {e}")))?;
    Ok(())
}

// ----- Streamable HTTP -----

/// Supplies OAuth bearer tokens for an HTTP server.
#[allow(
    clippy::double_must_use,
    reason = "async_trait expansion triggers rust-clippy#17529"
)]
#[async_trait]
pub(crate) trait BearerSource: Send + Sync {
    /// The current access token, refreshing first when `force_refresh` is set
    /// or the token is about to expire. `None` means no sign-in exists.
    async fn bearer(&self, force_refresh: bool) -> Result<Option<String>>;
}

pub(crate) struct HttpTransport {
    http: reqwest::Client,
    url: Url,
    post_url: StdMutex<Url>,
    headers: HeaderMap,
    auth: Option<Arc<dyn BearerSource>>,
    session_id: StdMutex<Option<String>>,
    protocol_version: StdMutex<Option<String>>,
    legacy: AtomicBool,
    link: Weak<Link>,
    tasks: StdMutex<Vec<tokio::task::AbortHandle>>,
}

impl HttpTransport {
    pub(crate) fn new(
        http: reqwest::Client,
        url: Url,
        headers: HeaderMap,
        auth: Option<Arc<dyn BearerSource>>,
        link: &Arc<Link>,
    ) -> Arc<Self> {
        Arc::new(Self {
            http,
            post_url: StdMutex::new(url.clone()),
            url,
            headers,
            auth,
            session_id: StdMutex::new(None),
            protocol_version: StdMutex::new(None),
            legacy: AtomicBool::new(false),
            link: Arc::downgrade(link),
            tasks: StdMutex::new(Vec::new()),
        })
    }

    pub(crate) fn set_protocol_version(&self, version: &str) {
        *self.protocol_version.lock().expect("mcp http poisoned") = Some(version.to_string());
    }

    pub(crate) fn is_legacy(&self) -> bool {
        self.legacy.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn session_id(&self) -> Option<String> {
        self.session_id.lock().expect("mcp http poisoned").clone()
    }

    async fn headers_for(&self, force_refresh: bool) -> std::result::Result<HeaderMap, SendError> {
        let mut headers = self.headers.clone();
        if let Some(session) = self.session_id.lock().expect("mcp http poisoned").clone() {
            if let Ok(value) = HeaderValue::from_str(&session) {
                headers.insert(SESSION_HEADER, value);
            }
        }
        if let Some(version) = self
            .protocol_version
            .lock()
            .expect("mcp http poisoned")
            .clone()
        {
            if let Ok(value) = HeaderValue::from_str(&version) {
                headers.insert(PROTOCOL_HEADER, value);
            }
        }
        if let Some(auth) = &self.auth {
            match auth.bearer(force_refresh).await {
                Ok(Some(token)) => {
                    let value =
                        HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                            SendError::Other(Error::Other("invalid OAuth token".into()))
                        })?;
                    headers.insert(AUTHORIZATION, value);
                }
                Ok(None) => {}
                Err(Error::Unauthorized(message)) => return Err(SendError::Unauthorized(message)),
                Err(error) => return Err(SendError::Other(error)),
            }
        }
        Ok(headers)
    }

    fn link(&self) -> Option<Arc<Link>> {
        self.link.upgrade()
    }

    fn close_link(&self, reason: &str) {
        if let Some(link) = self.link() {
            link.close(reason);
        }
    }

    fn track(&self, handle: tokio::task::JoinHandle<()>) {
        let mut tasks = self.tasks.lock().expect("mcp http poisoned");
        tasks.retain(|task| !task.is_finished());
        tasks.push(handle.abort_handle());
    }

    /// POST one JSON-RPC message. Responses arrive through the [`Link`],
    /// either parsed from a JSON body, streamed from an SSE body, or (legacy
    /// and 202 cases) from a long-lived GET stream.
    pub(crate) async fn send(
        self: &Arc<Self>,
        message: &Value,
    ) -> std::result::Result<(), SendError> {
        let mut force_refresh = false;
        loop {
            let url = self.post_url.lock().expect("mcp http poisoned").clone();
            let headers = match self.headers_for(force_refresh).await {
                Err(SendError::Unauthorized(message)) => {
                    self.close_link(&message);
                    return Err(SendError::Unauthorized(message));
                }
                other => other?,
            };
            let response = self
                .http
                .post(url)
                .headers(headers)
                .header(ACCEPT, "application/json, text/event-stream")
                .header(CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(message).map_err(Error::from)?)
                .send()
                .await
                .map_err(|error| self.request_failed(error))?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED {
                if self.auth.is_some() && !force_refresh {
                    force_refresh = true;
                    continue;
                }
                // A revoked sign-in ends the session so the hub reconnects
                // and reports that sign-in is required.
                let message = unauthorized_message(&response);
                self.close_link(&message);
                return Err(SendError::Unauthorized(message));
            }
            let has_session = self.session_id.lock().expect("mcp http poisoned").is_some();
            if status == StatusCode::NOT_FOUND && has_session {
                self.close_link("MCP session expired");
                return Err(SendError::Other(Error::Other("MCP session expired".into())));
            }
            if !status.is_success() {
                let body = body_snippet(response).await;
                return Err(SendError::Status(status, body));
            }
            if let Some(session) = response
                .headers()
                .get(SESSION_HEADER)
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.is_empty())
            {
                *self.session_id.lock().expect("mcp http poisoned") = Some(session.to_string());
            }
            if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
                return Ok(());
            }
            if is_event_stream(response.headers()) {
                let this = self.clone();
                self.track(tokio::spawn(async move {
                    this.read_events(response).await;
                }));
                return Ok(());
            }
            let body = read_bounded(response).await?;
            if body.iter().all(u8::is_ascii_whitespace) {
                return Ok(());
            }
            let value: Value = serde_json::from_slice(&body).map_err(|error| {
                SendError::Other(Error::Upstream(format!(
                    "invalid MCP JSON response: {error}"
                )))
            })?;
            self.deliver(value);
            return Ok(());
        }
    }

    fn request_failed(&self, error: reqwest::Error) -> SendError {
        if error.is_connect() {
            self.close_link(&format!("MCP server unreachable: {error}"));
        }
        SendError::Other(Error::Upstream(format!("MCP HTTP request failed: {error}")))
    }

    /// Dispatch one JSON value (a message or a batch) and send any replies.
    fn deliver(self: &Arc<Self>, value: Value) {
        let Some(link) = self.link() else { return };
        let messages = match value {
            Value::Array(items) => items,
            other => vec![other],
        };
        for message in messages {
            if let Some(reply) = link.dispatch(message) {
                let this = self.clone();
                self.track(tokio::spawn(async move {
                    let _ = this.send(&reply).await;
                }));
            }
        }
    }

    /// Consume an SSE body, dispatching every `message` event.
    async fn read_events(self: &Arc<Self>, response: reqwest::Response) {
        let mut parser = SseParser::default();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else { return };
            let Ok(events) = parser.push(&chunk) else {
                return;
            };
            for event in events {
                self.handle_event(event);
            }
        }
    }

    fn handle_event(self: &Arc<Self>, event: SseEvent) {
        if !(event.event.is_empty() || event.event == "message") {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(&event.data) {
            self.deliver(value);
        }
    }

    /// Open the optional GET stream for server-initiated messages. Servers
    /// that do not offer one answer 405; the stream is re-opened with backoff
    /// while the connection lives.
    pub(crate) fn open_listen_stream(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let handle = tokio::spawn(async move {
            let mut delay = Duration::from_secs(1);
            loop {
                let Some(this) = weak.upgrade() else { return };
                if this
                    .link()
                    .is_none_or(|link| link.closed_reason().is_some())
                {
                    return;
                }
                let Ok(headers) = this.headers_for(false).await else {
                    return;
                };
                match this
                    .http
                    .get(this.url.clone())
                    .headers(headers)
                    .header(ACCEPT, "text/event-stream")
                    .send()
                    .await
                {
                    Ok(response) if response.status() == StatusCode::NOT_FOUND => {
                        if this.session_id.lock().expect("mcp http poisoned").is_some() {
                            this.close_link("MCP session expired");
                        }
                        return;
                    }
                    Ok(response)
                        if response.status().is_success()
                            && is_event_stream(response.headers()) =>
                    {
                        delay = Duration::from_secs(1);
                        this.read_events(response).await;
                    }
                    Ok(_) => return,
                    Err(error) if error.is_connect() => {
                        this.close_link(&format!("MCP server unreachable: {error}"));
                        return;
                    }
                    Err(_) => {}
                }
                drop(this);
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(30));
            }
        });
        self.track(handle);
    }

    /// Switch to the legacy HTTP+SSE transport: GET the stream, wait for its
    /// `endpoint` event, then POST every message there. Responses only arrive
    /// on the stream, so its end closes the connection.
    pub(crate) async fn open_legacy(self: &Arc<Self>) -> Result<()> {
        let headers = self.headers_for(false).await.map_err(Error::from)?;
        let response = self
            .http
            .get(self.url.clone())
            .headers(headers)
            .header(ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|error| Error::Upstream(format!("MCP SSE request failed: {error}")))?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(Error::Unauthorized(unauthorized_message(&response)));
        }
        if !response.status().is_success() || !is_event_stream(response.headers()) {
            return Err(Error::Upstream(format!(
                "MCP server rejected Streamable HTTP and legacy SSE (HTTP {})",
                response.status()
            )));
        }
        let (endpoint_tx, endpoint_rx) = oneshot::channel::<String>();
        let this = self.clone();
        self.track(tokio::spawn(async move {
            let mut endpoint_tx = Some(endpoint_tx);
            let mut parser = SseParser::default();
            let mut stream = response.bytes_stream();
            while let Some(Ok(chunk)) = stream.next().await {
                let Ok(events) = parser.push(&chunk) else {
                    break;
                };
                for event in events {
                    if event.event == "endpoint" {
                        if let Some(tx) = endpoint_tx.take() {
                            let _ = tx.send(event.data);
                        }
                    } else {
                        this.handle_event(event);
                    }
                }
            }
            this.close_link("MCP SSE stream closed");
        }));
        let endpoint = tokio::time::timeout(LEGACY_ENDPOINT_TIMEOUT, endpoint_rx)
            .await
            .map_err(|_| Error::Upstream("MCP SSE server sent no endpoint event".into()))?
            .map_err(|_| {
                Error::Upstream("MCP SSE stream closed before its endpoint event".into())
            })?;
        let endpoint = self
            .url
            .join(endpoint.trim())
            .map_err(|error| Error::Upstream(format!("invalid MCP SSE endpoint: {error}")))?;
        if endpoint.origin() != self.url.origin() {
            return Err(Error::Upstream(
                "MCP SSE endpoint must share the server's origin".into(),
            ));
        }
        *self.post_url.lock().expect("mcp http poisoned") = endpoint;
        self.legacy.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Stop background streams and end the server session (best effort).
    pub(crate) fn shutdown(&self) {
        for task in self.tasks.lock().expect("mcp http poisoned").drain(..) {
            task.abort();
        }
        if self.is_legacy() {
            return;
        }
        let Some(session) = self.session_id.lock().expect("mcp http poisoned").take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let request = self
            .http
            .delete(self.url.clone())
            .headers(self.headers.clone())
            .header(SESSION_HEADER, session)
            .timeout(Duration::from_secs(5));
        runtime.spawn(async move {
            let _ = request.send().await;
        });
    }
}

fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/event-stream"))
}

fn unauthorized_message(response: &reqwest::Response) -> String {
    match response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
    {
        Some(challenge) if !challenge.trim().is_empty() => {
            format!("MCP server requires sign-in ({})", challenge.trim())
        }
        _ => "MCP server requires sign-in".to_string(),
    }
}

async fn body_snippet(response: reqwest::Response) -> String {
    let text = response.text().await.unwrap_or_default();
    text.trim().chars().take(300).collect()
}

async fn read_bounded(mut response: reqwest::Response) -> std::result::Result<Vec<u8>, SendError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        SendError::Other(Error::Upstream(format!("MCP response failed: {error}")))
    })? {
        if body.len() + chunk.len() > MAX_MESSAGE_BYTES {
            return Err(SendError::Other(Error::Upstream(
                "MCP response exceeds 8 MiB".into(),
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

// ----- Server-sent events -----

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Incremental `text/event-stream` parser (LF or CRLF line endings).
#[derive(Default)]
pub(crate) struct SseParser {
    buffer: Vec<u8>,
    event: String,
    data: String,
    has_data: bool,
}

impl SseParser {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=position).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if self.has_data {
                    events.push(SseEvent {
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data),
                    });
                }
                self.event.clear();
                self.data.clear();
                self.has_data = false;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line.as_ref(), ""),
            };
            match field {
                "event" => self.event = value.to_string(),
                "data" => {
                    if self.has_data {
                        self.data.push('\n');
                    }
                    self.data.push_str(value);
                    self.has_data = true;
                }
                _ => {}
            }
            if self.data.len() > MAX_MESSAGE_BYTES {
                return Err(Error::Upstream("MCP SSE event exceeds 8 MiB".into()));
            }
        }
        if self.buffer.len() > MAX_MESSAGE_BYTES {
            return Err(Error::Upstream("MCP SSE line exceeds 8 MiB".into()));
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_handles_split_chunks_multiline_data_and_comments() {
        let mut parser = SseParser::default();
        assert!(parser
            .push(b": keepalive\r\nevent: mess")
            .unwrap()
            .is_empty());
        let events = parser
            .push(b"age\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: two\n\n")
            .unwrap();
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: "message".into(),
                    data: "{\"a\":\n1}".into()
                },
                SseEvent {
                    event: String::new(),
                    data: "two".into()
                },
            ]
        );
    }

    #[test]
    fn link_close_fails_pending_requests_and_keeps_first_reason() {
        let (link, mut notifications) = Link::new();
        let rx = link.register(1);
        link.dispatch(json!({"jsonrpc":"2.0","method":"notifications/message","params":{}}));
        link.close("process exited");
        link.close("second reason");
        let response = rx.blocking_recv().unwrap();
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("process exited"));
        assert_eq!(link.closed_reason().as_deref(), Some("process exited"));
        assert!(notifications.try_recv().is_ok());
        assert!(notifications.try_recv().is_err());
    }
}
