//! Thin bridge to the official `codex app-server` JSON-RPC surface.

use std::collections::{BTreeMap, HashSet};
use std::convert::Infallible;
#[cfg(windows)]
use std::env;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use axum::response::sse::Event;
use base64::Engine;
use futures::{Stream, StreamExt};
use milim_core::api::openai::{ReasoningEffort, Usage};
use milim_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::privacy::Unredactor;

const CODEX_MODEL_FALLBACK: &str = "gpt-5.4";
const CODEX_LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const CODEX_CLIENT_NAME: &str = "milim";
const CODEX_CLIENT_TITLE: &str = "Milim";
pub(crate) const MAX_ACCOUNT_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ACCOUNT_IMAGES: usize = 12;
const CODEX_THREAD_PAGE_SIZE: u64 = 25;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct AccountImage {
    pub media_type: String,
    pub data: String,
}

impl AccountImage {
    fn extension(&self) -> Result<&'static str> {
        match self.media_type.as_str() {
            "image/png" => Ok("png"),
            "image/jpeg" => Ok("jpg"),
            "image/webp" => Ok("webp"),
            "image/gif" => Ok("gif"),
            other => Err(Error::InvalidRequest(format!(
                "unsupported account-runtime image type '{other}'; use PNG, JPEG, WebP, or GIF"
            ))),
        }
    }

    fn decode(&self) -> Result<Vec<u8>> {
        self.extension()?;
        if self.data.is_empty() {
            return Err(Error::InvalidRequest(
                "account-runtime image data is required".to_string(),
            ));
        }
        if self.data.len() > MAX_ACCOUNT_IMAGE_BYTES * 4 / 3 + 8 {
            return Err(Error::InvalidRequest(format!(
                "account-runtime images must be no larger than {} bytes",
                MAX_ACCOUNT_IMAGE_BYTES
            )));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.data)
            .map_err(|_| {
                Error::InvalidRequest("account-runtime image data is not valid base64".to_string())
            })?;
        if bytes.is_empty() {
            return Err(Error::InvalidRequest(
                "account-runtime image data is required".to_string(),
            ));
        }
        if bytes.len() > MAX_ACCOUNT_IMAGE_BYTES {
            return Err(Error::InvalidRequest(format!(
                "account-runtime images must be no larger than {} bytes",
                MAX_ACCOUNT_IMAGE_BYTES
            )));
        }
        if !image_bytes_match_media_type(&self.media_type, &bytes) {
            return Err(Error::InvalidRequest(format!(
                "account-runtime image bytes do not match {}",
                self.media_type
            )));
        }
        Ok(bytes)
    }
}

pub(crate) fn image_bytes_match_media_type(media_type: &str, bytes: &[u8]) -> bool {
    match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

pub(crate) fn validate_account_images(images: &[AccountImage]) -> Result<()> {
    if images.len() > MAX_ACCOUNT_IMAGES {
        return Err(Error::InvalidRequest(format!(
            "account-runtime turns accept at most {MAX_ACCOUNT_IMAGES} images"
        )));
    }
    for image in images {
        image.decode()?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<AccountImage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub persist_thread: bool,
    #[serde(default)]
    pub tool_approval_policy: Option<String>,
    #[serde(default)]
    pub tool_approval_grant: bool,
    #[serde(default)]
    pub interactive_tool_approval: bool,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub milim_context: Option<crate::routes::AccountRuntimeMilimContext>,
    #[serde(skip)]
    pub milim_mcp: Option<crate::routes::AccountRuntimeToolEndpoint>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CodexThreadSummary {
    pub id: String,
    pub name: Option<String>,
    pub preview: String,
    pub cwd: Option<String>,
    pub model_provider: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub archived: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexThreadPage {
    pub data: Vec<CodexThreadSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexRecoveredMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct CodexRecoveredThread {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<CodexRecoveredMessage>,
}

struct CodexTurnImages {
    dir: PathBuf,
    paths: Vec<PathBuf>,
}

impl CodexTurnImages {
    fn materialize(images: &[AccountImage]) -> Result<Option<Self>> {
        if images.is_empty() {
            return Ok(None);
        }
        let dir = std::env::temp_dir().join(format!("milim-codex-images-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir)
            .map_err(|e| Error::Other(format!("failed to create Codex image directory: {e}")))?;
        let mut materialized = Self {
            dir,
            paths: Vec::new(),
        };
        for (index, image) in images.iter().enumerate() {
            let path = materialized
                .dir
                .join(format!("image-{index}.{}", image.extension()?));
            std::fs::write(&path, image.decode()?)
                .map_err(|e| Error::Other(format!("failed to write Codex image: {e}")))?;
            materialized.paths.push(path);
        }
        Ok(Some(materialized))
    }
}

impl Drop for CodexTurnImages {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn codex_turn_input(req: &CodexRunRequest, images: Option<&CodexTurnImages>) -> Vec<Value> {
    let mut input = Vec::new();
    if !req.prompt.trim().is_empty() {
        input.push(json!({ "type": "text", "text": req.prompt }));
    }
    if let Some(images) = images {
        input.extend(
            images
                .paths
                .iter()
                .map(|path| json!({ "type": "localImage", "path": path.to_string_lossy() })),
        );
    }
    input
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodexApiKeyLoginRequest {
    pub api_key: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CodexStreamEvent {
    Thread {
        thread_id: String,
        model: String,
    },
    Start {
        thread_id: String,
        turn_id: String,
    },
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolApprovalRequired {
        approval_id: String,
        call_id: Option<String>,
        name: String,
        arguments: String,
        effect: &'static str,
        request_kind: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        request: Option<Value>,
    },
    ToolApprovalResolved {
        approval_id: String,
        call_id: Option<String>,
        decision: &'static str,
    },
    ToolApprovalStatus {
        approval_id: String,
        call_id: Option<String>,
        decision: &'static str,
        status: &'static str,
    },
    ToolApprovalFailed {
        approval_id: String,
        call_id: Option<String>,
        decision: Option<&'static str>,
        message: String,
    },
    Tool {
        id: String,
        name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    Done {
        thread_id: String,
        turn_id: Option<String>,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Warning {
        message: String,
    },
    ProtocolNotice {
        kind: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Image {
        id: String,
        status: String,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        revised_prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        saved_path: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AccountWorkerEvent {
    Session {
        runtime: String,
        external_thread_id: String,
        model: String,
    },
    Started {
        runtime: String,
        external_thread_id: Option<String>,
        external_turn_id: Option<String>,
    },
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        status: String,
        label: Option<String>,
        detail: Option<String>,
        icon: Option<String>,
    },
    NativeWorker {
        lifecycle: AccountNativeWorkerLifecycle,
    },
    Done {
        status: String,
        usage: Option<Usage>,
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
        usage: Option<Usage>,
        cost_usd: Option<f64>,
    },
    Warning {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AccountNativeWorkerLifecycle {
    pub runtime: String,
    pub call_id: String,
    pub operation: String,
    pub status: String,
    pub parent_runtime_id: Option<String>,
    pub worker_runtime_ids: Vec<String>,
    pub workers: Vec<AccountNativeWorkerState>,
    pub prompt: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct AccountNativeWorkerState {
    pub runtime_id: String,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CodexLoginEvent {
    Browser {
        login_id: String,
        auth_url: String,
    },
    DeviceCode {
        login_id: String,
        user_code: String,
        verification_url: String,
    },
    Done {
        success: bool,
        error: Option<String>,
    },
    Error {
        message: String,
    },
    Warning {
        message: String,
    },
}

pub(crate) async fn account(refresh_token: bool) -> Result<Value> {
    let mut proc = CodexProcess::start().await?;
    proc.request(
        "account/read",
        Some(json!({ "refreshToken": refresh_token })),
    )
    .await
}

pub(crate) async fn logout() -> Result<Value> {
    let mut proc = CodexProcess::start().await?;
    proc.request("account/logout", None).await
}

pub(crate) async fn rate_limits() -> Result<Value> {
    let mut proc = CodexProcess::start().await?;
    proc.request("account/rateLimits/read", None).await
}

pub(crate) async fn models() -> Result<Value> {
    let mut proc = CodexProcess::start().await?;
    proc.request(
        "model/list",
        Some(json!({ "includeHidden": false, "limit": 100 })),
    )
    .await
}

pub(crate) async fn threads(
    cursor: Option<String>,
    search: Option<String>,
    archived: bool,
) -> Result<CodexThreadPage> {
    let mut proc = CodexProcess::start().await?;
    let mut params = json!({
        "limit": CODEX_THREAD_PAGE_SIZE,
        "sortKey": "updated_at",
        "sortDirection": "desc",
        "archived": archived,
    });
    if let Some(cursor) = clean_optional(cursor.as_deref()) {
        params["cursor"] = Value::String(cursor);
    }
    if let Some(search) = clean_optional(search.as_deref()) {
        params["searchTerm"] = Value::String(search);
    }
    let result = proc.request("thread/list", Some(params)).await?;
    let data = result
        .get("data")
        .and_then(Value::as_array)
        .map(|threads| {
            threads
                .iter()
                .filter_map(|thread| codex_thread_summary(thread, archived))
                .collect()
        })
        .unwrap_or_default();
    Ok(CodexThreadPage {
        data,
        next_cursor: extract_string(&result, &["nextCursor"]),
    })
}

pub(crate) async fn recover_thread(thread_id: &str) -> Result<CodexRecoveredThread> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err(Error::InvalidRequest(
            "Codex thread id is required".to_string(),
        ));
    }
    let mut proc = CodexProcess::start().await?;
    let result = match proc
        .request(
            "thread/read",
            Some(json!({ "threadId": thread_id, "includeTurns": true })),
        )
        .await
    {
        Ok(result) => result,
        Err(error) if is_paginated_history_error(&error.to_string()) => {
            let metadata = proc
                .request(
                    "thread/read",
                    Some(json!({ "threadId": thread_id, "includeTurns": false })),
                )
                .await?;
            let turns = experimental_thread_turns(thread_id).await?;
            let mut thread = metadata.get("thread").cloned().ok_or_else(|| {
                Error::Upstream("Codex did not return thread metadata".to_string())
            })?;
            thread["turns"] = Value::Array(turns);
            json!({ "thread": thread })
        }
        Err(error) => return Err(error),
    };
    recovered_thread_from_result(&result)
}

async fn experimental_thread_turns(thread_id: &str) -> Result<Vec<Value>> {
    let mut proc = CodexProcess::start_experimental().await?;
    let mut cursor: Option<String> = None;
    let mut turns = Vec::new();
    loop {
        let mut params = json!({
            "threadId": thread_id,
            "limit": 100,
            "sortDirection": "asc",
            "itemsView": "full",
        });
        if let Some(value) = cursor.take() {
            params["cursor"] = Value::String(value);
        }
        let result = proc.request("thread/turns/list", Some(params)).await?;
        turns.extend(
            result
                .get("data")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        cursor = extract_string(&result, &["nextCursor"]);
        if cursor.is_none() {
            break;
        }
    }
    Ok(turns)
}

pub(crate) async fn login_api_key(api_key: String) -> Result<Value> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(Error::InvalidRequest(
            "Codex API key is required".to_string(),
        ));
    }
    let mut proc = CodexProcess::start().await?;
    proc.request(
        "account/login/start",
        Some(json!({ "type": "apiKey", "apiKey": api_key })),
    )
    .await
}

pub(crate) fn login_device_stream() -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    login_stream("chatgpt")
}

pub(crate) fn login_chatgpt_device_code_stream(
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    login_stream("chatgptDeviceCode")
}

fn login_stream(
    login_type: &'static str,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let mut proc = match CodexProcess::start().await {
            Ok(proc) => proc,
            Err(e) => {
                let message = e.to_string();
                if is_cli_path_warning(&message) {
                    yield sse_event(&CodexLoginEvent::Warning { message });
                } else {
                    yield sse_event(&CodexLoginEvent::Error { message });
                }
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        let started = match proc
            .request(
                "account/login/start",
                Some(json!({ "type": login_type })),
            )
            .await
        {
            Ok(value) => value,
            Err(e) => {
                yield sse_event(&CodexLoginEvent::Error { message: e.to_string() });
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };

        let login_id = match extract_string(&started, &["loginId"]) {
            Some(id) => id,
            None => {
                yield sse_event(&CodexLoginEvent::Error {
                    message: "codex app-server did not return a login id".to_string(),
                });
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        match started.get("type").and_then(Value::as_str) {
            Some("chatgpt") => match extract_string(&started, &["authUrl"]) {
                Some(auth_url) => yield sse_event(&CodexLoginEvent::Browser {
                    login_id: login_id.clone(),
                    auth_url,
                }),
                None => {
                    yield sse_event(&CodexLoginEvent::Error {
                        message: "codex app-server did not return an auth URL".to_string(),
                    });
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            },
            _ => {
                let user_code = match extract_string(&started, &["userCode"]) {
                    Some(code) => code,
                    None => {
                        yield sse_event(&CodexLoginEvent::Error {
                            message: "codex app-server did not return a device code".to_string(),
                        });
                        yield Ok(Event::default().data("[DONE]"));
                        return;
                    }
                };
                let verification_url = match extract_string(&started, &["verificationUrl"]) {
                    Some(url) => url,
                    None => {
                        yield sse_event(&CodexLoginEvent::Error {
                            message: "codex app-server did not return a verification URL".to_string(),
                        });
                        yield Ok(Event::default().data("[DONE]"));
                        return;
                    }
                };
                yield sse_event(&CodexLoginEvent::DeviceCode {
                    login_id: login_id.clone(),
                    user_code,
                    verification_url,
                });
            }
        }

        loop {
            let msg = match tokio::time::timeout(CODEX_LOGIN_TIMEOUT, proc.read_value()).await {
                Ok(Ok(msg)) => msg,
                Ok(Err(e)) => {
                    yield sse_event(&CodexLoginEvent::Error { message: e.to_string() });
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                Err(_) => {
                    let _ = proc
                        .request("account/login/cancel", Some(json!({ "loginId": login_id })))
                        .await;
                    yield sse_event(&CodexLoginEvent::Error {
                        message: "Codex login timed out before completion.".to_string(),
                    });
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            };
            if let (Some(method), Some(id)) = (
                msg.get("method").and_then(Value::as_str),
                rpc_request_id(&msg),
            ) {
                let response = noninteractive_request_response(method);
                let result = match response {
                    Some(response) => proc.respond(id, response).await,
                    None => proc.respond_error(id, -32601, "Method not found").await,
                };
                if let Err(error) = result {
                    yield sse_event(&CodexLoginEvent::Error { message: error.to_string() });
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                continue;
            }
            if msg.get("method").and_then(Value::as_str) != Some("account/login/completed") {
                continue;
            }
            let params = msg.get("params").unwrap_or(&Value::Null);
            let notification_login_id = params.get("loginId").and_then(Value::as_str);
            if notification_login_id.is_some() && notification_login_id != Some(login_id.as_str()) {
                continue;
            }
            yield sse_event(&CodexLoginEvent::Done {
                success: params
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                error: params
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
            yield Ok(Event::default().data("[DONE]"));
            return;
        }
    }
}

pub(crate) fn run_stream(
    req: CodexRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<milim_agents::ToolApprovalBroker>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let stream = run_stream_with_worker_events(req, redactions, Some(worker_tx), approval_broker);
        futures::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            yield event;
            while let Ok(worker) = worker_rx.try_recv() {
                if matches!(worker, AccountWorkerEvent::NativeWorker { .. }) {
                    yield sse_event(&worker);
                }
            }
        }
    }
}

fn run_stream_with_worker_events(
    req: CodexRunRequest,
    redactions: BTreeMap<String, String>,
    worker_events: Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
    approval_broker: Option<std::sync::Arc<milim_agents::ToolApprovalBroker>>,
) -> impl Stream<Item = std::result::Result<Event, Infallible>> {
    async_stream::stream! {
        let turn_images = match CodexTurnImages::materialize(&req.images) {
            Ok(images) => images,
            Err(e) => {
                yield sse_event_with_worker(&CodexStreamEvent::Error {
                    message: e.to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        let mut proc = match CodexProcess::start().await {
            Ok(proc) => proc,
            Err(e) => {
                let message = e.to_string();
                if is_cli_path_warning(&message) {
                    yield sse_event_with_worker(&CodexStreamEvent::Warning { message }, &worker_events);
                } else {
                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                        message,
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                }
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };

        let model = clean_model(req.model.as_deref());
        let (thread_method, thread_params) = codex_thread_request(&req, model);

        let thread = match proc.request(thread_method, Some(thread_params)).await {
            Ok(value) => value,
            Err(e) => {
                yield sse_event_with_worker(&CodexStreamEvent::Error {
                    message: e.to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        let thread_id = match extract_string(&thread, &["thread", "id"]) {
            Some(id) => id,
            None => {
                yield sse_event_with_worker(&CodexStreamEvent::Error {
                    message: "codex app-server did not return a thread id".to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };
        yield sse_event_with_worker(&CodexStreamEvent::Thread {
            thread_id: thread_id.clone(),
            model: model.to_string(),
        }, &worker_events);

        let cwd = clean_optional(req.cwd.as_deref());
        let mut turn_params = json!({
            "threadId": thread_id,
            "input": codex_turn_input(&req, turn_images.as_ref()),
            "model": model,
            "approvalPolicy": codex_approval_policy(&req),
            "sandboxPolicy": codex_sandbox_policy(&req, cwd.as_deref()),
        });
        if let Some(effort) = req.reasoning_effort.filter(|effort| !effort.is_auto()) {
            turn_params["effort"] = Value::String(effort.as_str().to_string());
        }
        if let Some(cwd) = cwd {
            turn_params["cwd"] = Value::String(cwd);
        }

        let turn_request_id = match proc.send_request("turn/start", Some(turn_params)).await {
            Ok(id) => id,
            Err(e) => {
                yield sse_event_with_worker(&CodexStreamEvent::Error {
                    message: e.to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                yield Ok(Event::default().data("[DONE]"));
                return;
            }
        };

        let mut turn_id: Option<String> = None;
        let mut content = Unredactor::new(redactions.clone());
        let mut reasoning = Unredactor::new(redactions);
        let mut last_agent_message_id: Option<String> = None;
        let mut emitted_agent_text = false;
        let mut approval_ack: Option<(
            milim_agents::PendingApproval,
            Option<String>,
            &'static str,
        )> = None;

        loop {
            let read = if approval_ack.is_some() {
                match tokio::time::timeout(
                    milim_agents::APPROVAL_RUNTIME_ACK_TIMEOUT,
                    proc.read_value(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        if let Some((pending, call_id, decision)) = approval_ack.take() {
                            let message = "Codex did not resume after the approval decision".to_string();
                            pending.fail(message.clone());
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                approval_id: pending.id.clone(),
                                call_id,
                                decision: Some(decision),
                                message: message.clone(),
                            }, &worker_events);
                            yield sse_event_with_worker(&CodexStreamEvent::Error {
                                message,
                                usage: None,
                                cost_usd: None,
                            }, &worker_events);
                        }
                        yield Ok(Event::default().data("[DONE]"));
                        return;
                    }
                }
            } else {
                proc.read_value().await
            };
            let msg = match read {
                Ok(msg) => msg,
                Err(e) => {
                    if let Some((pending, call_id, decision)) = approval_ack.take() {
                        pending.fail(e.to_string());
                        yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                            approval_id: pending.id.clone(),
                            call_id,
                            decision: Some(decision),
                            message: e.to_string(),
                        }, &worker_events);
                    }
                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                        message: e.to_string(),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
            };
            if let Some((pending, call_id, decision)) = approval_ack.take() {
                pending.acknowledge();
                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalResolved {
                    approval_id: pending.id.clone(),
                    call_id,
                    decision,
                }, &worker_events);
            }

            if response_id(&msg) == Some(turn_request_id) {
                if let Some(error) = msg.get("error") {
                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                        message: rpc_error_message(error),
                        usage: usage_from_any(&msg),
                        cost_usd: cost_from_any(&msg),
                    }, &worker_events);
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                turn_id = extract_string(&msg, &["result", "turn", "id"]);
                if let Some(id) = &turn_id {
                    yield sse_event_with_worker(&CodexStreamEvent::Start {
                        thread_id: thread_id.clone(),
                        turn_id: id.clone(),
                    }, &worker_events);
                }
                continue;
            }

            let Some(method) = msg.get("method").and_then(Value::as_str) else {
                continue;
            };
            let params = msg.get("params").unwrap_or(&Value::Null);
            match method {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    if let Some(id) = rpc_request_id(&msg) {
                        let interactive = account_runtime_policy(req.tool_approval_policy.as_deref()) == "review"
                            && req.interactive_tool_approval
                            && !req.tool_approval_grant;
                        let mut pending_delivery = None;
                        let result = if interactive {
                            let Some(broker) = approval_broker.as_ref() else {
                                yield sse_event_with_worker(&CodexStreamEvent::Error {
                                    message: "Codex Review approval broker is unavailable".to_string(),
                                    usage: None,
                                    cost_usd: None,
                                }, &worker_events);
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            };
                            let mut pending = broker.request();
                            let call_id = extract_string(params, &["itemId"])
                                .or_else(|| extract_string(params, &["id"]));
                            let name = if method.contains("commandExecution") { "command" } else { "file_change" };
                            let arguments = serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string());
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalRequired {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                name: name.to_string(),
                                arguments,
                                effect: if name == "command" { "command" } else { "mutating" },
                                request_kind: name,
                                request: None,
                            }, &worker_events);
                            let approved = pending.wait().await.approved;
                            let decision = if approved { "approve" } else { "deny" };
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "decided",
                            }, &worker_events);
                            let result = match codex_decision_response(params, approved, false) {
                                Ok(result) => result,
                                Err(error) => {
                                    pending.fail(error.to_string());
                                    yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                        approval_id: pending.id.clone(),
                                        call_id,
                                        decision: Some(decision),
                                        message: error.to_string(),
                                    }, &worker_events);
                                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                                        message: error.to_string(),
                                        usage: None,
                                        cost_usd: None,
                                    }, &worker_events);
                                    yield Ok(Event::default().data("[DONE]"));
                                    return;
                                }
                            };
                            pending_delivery = Some((pending, call_id, decision));
                            result
                        } else {
                            match codex_approval_response(&req, method, params) {
                                Ok(result) => result,
                                Err(error) => {
                                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                                        message: error.to_string(),
                                        usage: None,
                                        cost_usd: None,
                                    }, &worker_events);
                                    yield Ok(Event::default().data("[DONE]"));
                                    return;
                                }
                            }
                        };
                        if let Err(e) = proc.respond(id, result).await {
                            if let Some((pending, call_id, decision)) = pending_delivery.take() {
                                pending.fail(e.to_string());
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: e.to_string(),
                                }, &worker_events);
                            }
                            yield sse_event_with_worker(&CodexStreamEvent::Error {
                                message: e.to_string(),
                                usage: None,
                                cost_usd: None,
                            }, &worker_events);
                            yield Ok(Event::default().data("[DONE]"));
                            return;
                        }
                        if let Some((pending, call_id, decision)) = pending_delivery {
                            if let Err(message) = pending.deliver() {
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: message.clone(),
                                }, &worker_events);
                                yield sse_event_with_worker(&CodexStreamEvent::Error {
                                    message,
                                    usage: None,
                                    cost_usd: None,
                                }, &worker_events);
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "delivered",
                            }, &worker_events);
                            approval_ack = Some((pending, call_id, decision));
                        }
                    }
                }
                "item/permissions/requestApproval" => {
                    if let Some(id) = rpc_request_id(&msg) {
                        let mut approved = false;
                        let call_id = extract_string(params, &["itemId"]);
                        let mut pending_delivery = None;
                        if let Some(broker) = approval_broker.as_ref() {
                            let mut pending = broker.request();
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalRequired {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                name: "permissions".to_string(),
                                arguments: serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string()),
                                effect: "mutating",
                                request_kind: "permissions",
                                request: Some(permission_request_descriptor(params)),
                            }, &worker_events);
                            approved = pending.wait().await.approved;
                            let decision = if approved { "approve" } else { "deny" };
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "decided",
                            }, &worker_events);
                            pending_delivery = Some((pending, call_id.clone(), decision));
                        }
                        if let Err(e) = proc.respond(id, permission_response(params, approved)).await {
                            if let Some((pending, call_id, decision)) = pending_delivery.take() {
                                pending.fail(e.to_string());
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: e.to_string(),
                                }, &worker_events);
                            }
                            yield sse_event_with_worker(&CodexStreamEvent::Error {
                                message: e.to_string(),
                                usage: None,
                                cost_usd: None,
                            }, &worker_events);
                            yield Ok(Event::default().data("[DONE]"));
                            return;
                        }
                        if let Some((pending, call_id, decision)) = pending_delivery {
                            if let Err(message) = pending.deliver() {
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: message.clone(),
                                }, &worker_events);
                                yield sse_event_with_worker(&CodexStreamEvent::Error {
                                    message,
                                    usage: None,
                                    cost_usd: None,
                                }, &worker_events);
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "delivered",
                            }, &worker_events);
                            approval_ack = Some((pending, call_id, decision));
                        }
                    }
                }
                "mcpServer/elicitation/request" => {
                    if let Some(id) = rpc_request_id(&msg) {
                        let (request_kind, request, supported) = mcp_approval_descriptor(params);
                        let call_id = extract_string(params, &["elicitationId"]);
                        if let Some(broker) = approval_broker.as_ref() {
                            let mut pending = broker.request();
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalRequired {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                name: format!("MCP {}", extract_string(params, &["serverName"]).unwrap_or_else(|| "server".to_string())),
                                arguments: extract_string(params, &["message"])
                                    .unwrap_or_else(|| serde_json::to_string(params).unwrap_or_else(|_| "{}".to_string())),
                                effect: "unknown",
                                request_kind,
                                request: Some(request),
                            }, &worker_events);
                            let decision = pending.wait().await;
                            let (response, accepted, validation_error) =
                                mcp_elicitation_response(params, supported, decision);
                            if let Some(message) = validation_error {
                                yield sse_event_with_worker(&CodexStreamEvent::ProtocolNotice {
                                    kind: "mcp_validation".to_string(),
                                    message: "Codex MCP response was declined".to_string(),
                                    detail: Some(message),
                                }, &worker_events);
                            }
                            let decision = if accepted { "approve" } else { "deny" };
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "decided",
                            }, &worker_events);
                            if let Err(e) = proc.respond(id, response).await {
                                pending.fail(e.to_string());
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: e.to_string(),
                                }, &worker_events);
                                yield sse_event_with_worker(&CodexStreamEvent::Error {
                                    message: e.to_string(),
                                    usage: None,
                                    cost_usd: None,
                                }, &worker_events);
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            if let Err(message) = pending.deliver() {
                                yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalFailed {
                                    approval_id: pending.id.clone(),
                                    call_id,
                                    decision: Some(decision),
                                    message: message.clone(),
                                }, &worker_events);
                                yield sse_event_with_worker(&CodexStreamEvent::Error {
                                    message,
                                    usage: None,
                                    cost_usd: None,
                                }, &worker_events);
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            yield sse_event_with_worker(&CodexStreamEvent::ToolApprovalStatus {
                                approval_id: pending.id.clone(),
                                call_id: call_id.clone(),
                                decision,
                                status: "delivered",
                            }, &worker_events);
                            approval_ack = Some((pending, call_id, decision));
                        } else if let Err(e) = proc
                            .respond(id, noninteractive_request_response(method).expect("known request"))
                            .await
                        {
                            yield sse_event_with_worker(&CodexStreamEvent::Error {
                                message: e.to_string(),
                                usage: None,
                                cost_usd: None,
                            }, &worker_events);
                            yield Ok(Event::default().data("[DONE]"));
                            return;
                        }
                    }
                }
                "configWarning" | "warning" | "model/rerouted" | "model/verification" | "deprecationNotice" => {
                    if let Some(notice) = protocol_notice(method, params) {
                        yield sse_event_with_worker(&notice, &worker_events);
                    }
                }
                "item/started" => {
                    if let Some(event) = codex_native_worker_event(params.get("item").unwrap_or(&Value::Null)) {
                        publish_worker(&worker_events, AccountWorkerEvent::NativeWorker { lifecycle: event });
                    }
                    if let Some(event) = tool_event_from_item(params.get("item").unwrap_or(&Value::Null), true) {
                        yield sse_event_with_worker(&event, &worker_events);
                    }
                }
                "item/agentMessage/delta" => {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        let item_id = extract_string(params, &["itemId"])
                            .or_else(|| extract_string(params, &["item", "id"]));
                        if item_id.is_some() && item_id != last_agent_message_id {
                            if emitted_agent_text {
                                yield sse_event_with_worker(&CodexStreamEvent::Token {
                                    text: "\n\n".to_string(),
                                }, &worker_events);
                            }
                            last_agent_message_id = item_id;
                        }
                        let text = content.push(delta);
                        if !text.is_empty() {
                            emitted_agent_text = true;
                            yield sse_event_with_worker(&CodexStreamEvent::Token { text }, &worker_events);
                        }
                    }
                }
                "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" => {
                    if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                        let text = reasoning.push(delta);
                        if !text.is_empty() {
                            yield sse_event_with_worker(&CodexStreamEvent::Reasoning { text }, &worker_events);
                        }
                    }
                }
                "item/completed" | "rawResponseItem/completed" => {
                    let item = params.get("item").unwrap_or(&Value::Null);
                    if let Some(event) = codex_native_worker_event(item) {
                        publish_worker(&worker_events, AccountWorkerEvent::NativeWorker { lifecycle: event });
                    }
                    if let Some(event) = image_event_from_item(item) {
                        yield sse_event(&event);
                    }
                    if let Some(event) = tool_event_from_item(item, false) {
                        yield sse_event_with_worker(&event, &worker_events);
                    }
                }
                "turn/completed" => {
                    let tail = content.flush();
                    if !tail.is_empty() {
                        yield sse_event_with_worker(&CodexStreamEvent::Token { text: tail }, &worker_events);
                    }
                    let rtail = reasoning.flush();
                    if !rtail.is_empty() {
                        yield sse_event_with_worker(&CodexStreamEvent::Reasoning { text: rtail }, &worker_events);
                    }

                    let status = extract_string(params, &["turn", "status"])
                        .unwrap_or_else(|| "completed".to_string());
                    let completed_turn_id = extract_string(params, &["turn", "id"]).or_else(|| turn_id.clone());
                    let usage = usage_from_any(params);
                    let cost_usd = cost_from_any(params);
                    if status == "failed" {
                        let message = extract_string(params, &["turn", "error", "message"])
                            .unwrap_or_else(|| "codex turn failed".to_string());
                        yield sse_event_with_worker(&CodexStreamEvent::Error {
                            message,
                            usage,
                            cost_usd,
                        }, &worker_events);
                    } else {
                        yield sse_event_with_worker(&CodexStreamEvent::Done {
                            thread_id: thread_id.clone(),
                            turn_id: completed_turn_id,
                            status,
                            usage,
                            cost_usd,
                        }, &worker_events);
                    }
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                "error" => {
                    let message = params
                        .get("error")
                        .map(rpc_error_message)
                        .unwrap_or_else(|| "codex app-server reported an error".to_string());
                    yield sse_event_with_worker(&CodexStreamEvent::Error {
                        message,
                        usage: usage_from_any(params),
                        cost_usd: cost_from_any(params),
                    }, &worker_events);
                    yield Ok(Event::default().data("[DONE]"));
                    return;
                }
                _ => {
                    if let Some(id) = rpc_request_id(&msg) {
                        if let Err(e) = proc.respond_error(id, -32601, "Method not found").await {
                            yield sse_event_with_worker(&CodexStreamEvent::Error {
                                message: e.to_string(),
                                usage: None,
                                cost_usd: None,
                            }, &worker_events);
                            yield Ok(Event::default().data("[DONE]"));
                            return;
                        }
                        yield sse_event_with_worker(&CodexStreamEvent::ProtocolNotice {
                            kind: "unsupported_request".to_string(),
                            message: format!("Codex requested unsupported method {method}"),
                            detail: None,
                        }, &worker_events);
                    }
                }
            }
        }
    }
}

fn sse_event<T: Serialize>(value: &T) -> std::result::Result<Event, Infallible> {
    Ok(Event::default().data(serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())))
}

fn sse_event_with_worker(
    value: &CodexStreamEvent,
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
) -> std::result::Result<Event, Infallible> {
    if let Some(event) = account_worker_event_from_codex(value) {
        publish_worker(worker_events, event);
    }
    sse_event(value)
}

fn publish_worker(
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
    event: AccountWorkerEvent,
) {
    if let Some(worker_events) = worker_events {
        let _ = worker_events.send(event);
    }
}

fn account_worker_event_from_codex(value: &CodexStreamEvent) -> Option<AccountWorkerEvent> {
    match value {
        CodexStreamEvent::Thread { thread_id, model } => Some(AccountWorkerEvent::Session {
            runtime: "codex".to_string(),
            external_thread_id: thread_id.clone(),
            model: model.clone(),
        }),
        CodexStreamEvent::Start { thread_id, turn_id } => Some(AccountWorkerEvent::Started {
            runtime: "codex".to_string(),
            external_thread_id: Some(thread_id.clone()),
            external_turn_id: Some(turn_id.clone()),
        }),
        CodexStreamEvent::Token { text } => Some(AccountWorkerEvent::Token { text: text.clone() }),
        CodexStreamEvent::Reasoning { text } => {
            Some(AccountWorkerEvent::Reasoning { text: text.clone() })
        }
        CodexStreamEvent::ToolApprovalRequired { .. }
        | CodexStreamEvent::ToolApprovalResolved { .. }
        | CodexStreamEvent::ToolApprovalStatus { .. }
        | CodexStreamEvent::ToolApprovalFailed { .. } => None,
        CodexStreamEvent::Tool {
            id,
            name,
            status,
            label,
            detail,
            icon,
        } => Some(AccountWorkerEvent::Tool {
            id: id.clone(),
            name: name.clone(),
            status: status.clone(),
            label: label.clone(),
            detail: detail.clone(),
            icon: icon.clone(),
        }),
        CodexStreamEvent::Done {
            status,
            usage,
            cost_usd,
            ..
        } => Some(AccountWorkerEvent::Done {
            status: status.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        CodexStreamEvent::Error {
            message,
            usage,
            cost_usd,
        } => Some(AccountWorkerEvent::Error {
            message: message.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        CodexStreamEvent::Warning { message } => Some(AccountWorkerEvent::Warning {
            message: message.clone(),
        }),
        CodexStreamEvent::ProtocolNotice {
            message, detail, ..
        } => Some(AccountWorkerEvent::Warning {
            message: detail
                .as_deref()
                .map(|detail| format!("{message}: {detail}"))
                .unwrap_or_else(|| message.clone()),
        }),
        CodexStreamEvent::Image { .. } => None,
    }
}

struct CodexProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

impl CodexProcess {
    async fn start() -> Result<Self> {
        Self::start_with_experimental(false).await
    }

    async fn start_experimental() -> Result<Self> {
        Self::start_with_experimental(true).await
    }

    async fn start_with_experimental(experimental: bool) -> Result<Self> {
        let mut command = codex_command();
        command
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // Don't flash a console window when spawning codex on Windows.
        #[cfg(windows)]
        command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);

        let mut child = command
            .spawn()
            .map_err(|e| Error::Upstream(codex_spawn_error_message(&e)))?;
        let stdin = child.stdin.take().ok_or_else(|| {
            Error::Upstream("codex app-server stdin was not available".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            Error::Upstream("codex app-server stdout was not available".to_string())
        })?;
        let mut proc = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        };
        proc.initialize(experimental).await?;
        Ok(proc)
    }

    async fn initialize(&mut self, experimental: bool) -> Result<()> {
        self.request("initialize", Some(initialize_params(experimental)))
            .await?;
        self.notify("initialized", json!({})).await
    }

    async fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.send_request(method, params).await?;
        loop {
            let msg = self.read_value().await?;
            if response_id(&msg) == Some(id) {
                if let Some(error) = msg.get("error") {
                    return Err(Error::Upstream(rpc_error_message(error)));
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
            if let (Some(method), Some(request_id)) = (
                msg.get("method").and_then(Value::as_str),
                rpc_request_id(&msg),
            ) {
                if let Some(result) = noninteractive_request_response(method) {
                    self.respond(request_id, result).await?;
                } else {
                    self.respond_error(request_id, -32601, "Method not found")
                        .await?;
                }
                continue;
            }
        }
    }

    async fn send_request(&mut self, method: &str, params: Option<Value>) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let mut msg = json!({ "method": method, "id": id });
        if let Some(params) = params {
            msg["params"] = params;
        }
        self.write_value(&msg).await?;
        Ok(id)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_value(&json!({ "method": method, "params": params }))
            .await
    }

    async fn respond(&mut self, id: Value, result: Value) -> Result<()> {
        self.write_value(&json!({ "id": id, "result": result }))
            .await
    }

    async fn respond_error(&mut self, id: Value, code: i64, message: &str) -> Result<()> {
        self.write_value(&json!({
            "id": id,
            "error": { "code": code, "message": message },
        }))
        .await
    }

    async fn write_value(&mut self, value: &Value) -> Result<()> {
        let line = serde_json::to_string(value)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_value(&mut self) -> Result<Value> {
        let line = self.stdout.next_line().await?.ok_or_else(|| {
            let status = self.child.try_wait().ok().flatten();
            match status {
                Some(status) => Error::Upstream(format!("codex app-server exited with {status}")),
                None => Error::Upstream("codex app-server closed stdout".to_string()),
            }
        })?;
        serde_json::from_str(&line).map_err(|e| {
            Error::Upstream(format!(
                "codex app-server emitted invalid JSON: {e}; line={line}"
            ))
        })
    }
}

#[cfg(windows)]
fn codex_command() -> Command {
    if let Some(path) = find_on_path("codex.cmd") {
        if let Some(command) = codex_npm_command(&path) {
            return command;
        }
        let mut fallback = Command::new("cmd");
        fallback.arg("/C").arg(path);
        return fallback;
    }
    if let Some(path) = find_on_path("codex.exe") {
        return Command::new(path);
    }
    Command::new("codex.exe")
}

#[cfg(not(windows))]
fn codex_command() -> Command {
    crate::cli_path::command("codex")
}

#[cfg(windows)]
fn find_on_path(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

#[cfg(windows)]
fn codex_npm_command(cmd_path: &std::path::Path) -> Option<Command> {
    let basedir = cmd_path.parent()?;
    let script = basedir
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("bin")
        .join("codex.js");
    if !script.is_file() {
        return None;
    }
    let node = find_on_path("node.exe").unwrap_or_else(|| PathBuf::from("node.exe"));
    let mut command = Command::new(node);
    command.arg(script);
    Some(command)
}

fn clean_model(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(CODEX_MODEL_FALLBACK)
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn existing_absolute_directory(value: Option<&str>) -> Option<String> {
    let value = clean_optional(value)?;
    let path = PathBuf::from(&value);
    (path.is_absolute() && path.is_dir()).then_some(value)
}

fn codex_thread_summary(thread: &Value, archived: bool) -> Option<CodexThreadSummary> {
    Some(CodexThreadSummary {
        id: extract_string(thread, &["id"])?,
        name: extract_string(thread, &["name"]),
        preview: extract_string(thread, &["preview"]).unwrap_or_default(),
        cwd: existing_absolute_directory(extract_string(thread, &["cwd"]).as_deref()),
        model_provider: extract_string(thread, &["modelProvider"])
            .unwrap_or_else(|| "openai".to_string()),
        created_at_ms: thread
            .get("createdAt")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            .saturating_mul(1000),
        updated_at_ms: thread
            .get("updatedAt")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            .saturating_mul(1000),
        archived,
    })
}

fn recovered_thread_from_result(result: &Value) -> Result<CodexRecoveredThread> {
    let thread = result
        .get("thread")
        .ok_or_else(|| Error::Upstream("Codex did not return thread data".to_string()))?;
    let summary = codex_thread_summary(thread, false)
        .ok_or_else(|| Error::Upstream("Codex did not return a thread id".to_string()))?;
    let messages = recovered_messages(thread.get("turns").and_then(Value::as_array));
    if messages.is_empty() {
        return Err(Error::InvalidRequest(
            "This Codex thread has no recoverable user or assistant messages".to_string(),
        ));
    }
    let title = summary
        .name
        .as_deref()
        .and_then(clean_optional_value)
        .or_else(|| clean_optional_value(&summary.preview))
        .or_else(|| {
            messages
                .iter()
                .find(|message| message.role == "user")
                .map(|message| compact_recovery_title(&message.content))
        })
        .unwrap_or_else(|| "Recovered Codex chat".to_string());
    Ok(CodexRecoveredThread {
        id: summary.id,
        title,
        cwd: summary.cwd,
        created_at_ms: summary.created_at_ms,
        updated_at_ms: summary.updated_at_ms,
        messages,
    })
}

fn clean_optional_value(value: &str) -> Option<String> {
    clean_optional(Some(value))
}

fn compact_recovery_title(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 100 {
        return compact;
    }
    format!("{}…", compact.chars().take(99).collect::<String>())
}

fn recovered_messages(turns: Option<&Vec<Value>>) -> Vec<CodexRecoveredMessage> {
    let mut messages = Vec::new();
    for item in turns
        .into_iter()
        .flatten()
        .filter_map(|turn| turn.get("items").and_then(Value::as_array))
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("userMessage") => {
                let content = item.get("content").and_then(Value::as_array);
                let text = content
                    .into_iter()
                    .flatten()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|part| extract_string(part, &["text"]))
                    .filter(|text| !text.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let has_omitted_media = content.into_iter().flatten().any(|part| {
                    matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("image" | "localImage" | "audio")
                    )
                });
                let content = if !text.is_empty() {
                    text
                } else if has_omitted_media {
                    "[Media omitted during Codex recovery]".to_string()
                } else {
                    continue;
                };
                messages.push(CodexRecoveredMessage {
                    role: "user",
                    content,
                });
            }
            Some("agentMessage") => {
                if let Some(content) =
                    extract_string(item, &["text"]).filter(|text| !text.trim().is_empty())
                {
                    messages.push(CodexRecoveredMessage {
                        role: "assistant",
                        content,
                    });
                }
            }
            _ => {}
        }
    }
    messages
}

fn is_paginated_history_error(message: &str) -> bool {
    message.to_ascii_lowercase().contains("paginated")
}

fn codex_thread_request(req: &CodexRunRequest, model: &str) -> (&'static str, Value) {
    let cwd = clean_optional(req.cwd.as_deref());
    let mut params = json!({
        "model": model,
        "approvalPolicy": codex_approval_policy(req),
        "sandbox": codex_sandbox_mode(req, cwd.as_deref()),
        "developerInstructions": codex_developer_instructions(),
    });
    if let Some(cwd) = cwd {
        params["cwd"] = Value::String(cwd);
    }
    if let Some(endpoint) = &req.milim_mcp {
        params["config"] = json!({
            "mcp_servers": {
                "milim": {
                    "url": endpoint.url,
                    "http_headers": { "Authorization": endpoint.authorization },
                    "required": true
                }
            }
        });
    }
    if let Some(thread_id) = clean_optional(req.thread_id.as_deref()) {
        params["threadId"] = Value::String(thread_id);
        ("thread/resume", params)
    } else {
        if !req.persist_thread {
            params["ephemeral"] = Value::Bool(true);
        }
        ("thread/start", params)
    }
}

fn account_runtime_policy(value: Option<&str>) -> &str {
    match value.map(str::trim) {
        Some("review") => "review",
        Some("open") => "open",
        _ => "guarded",
    }
}

fn codex_tools_allowed(req: &CodexRunRequest) -> bool {
    !req.plan_mode
        && match account_runtime_policy(req.tool_approval_policy.as_deref()) {
            "review" => req.tool_approval_grant || req.interactive_tool_approval,
            _ => true,
        }
}

fn codex_approval_policy(req: &CodexRunRequest) -> &'static str {
    if (account_runtime_policy(req.tool_approval_policy.as_deref()) == "review"
        && req.interactive_tool_approval
        && !req.tool_approval_grant)
        || !codex_tools_allowed(req)
        || account_runtime_policy(req.tool_approval_policy.as_deref()) == "guarded"
    {
        "on-request"
    } else {
        // Matches Milim's Open mode: no per-tool prompt after the user selected Open or approved a Review run.
        "never"
    }
}

fn codex_sandbox_policy(req: &CodexRunRequest, cwd: Option<&str>) -> Value {
    if codex_sandbox_mode(req, cwd) == "workspace-write" {
        return json!({
            "type": "workspaceWrite",
            "writableRoots": [cwd.unwrap_or_default()],
            "networkAccess": true,
        });
    }
    json!({ "type": "readOnly", "access": { "type": "fullAccess" } })
}

fn codex_sandbox_mode(req: &CodexRunRequest, cwd: Option<&str>) -> &'static str {
    let policy = account_runtime_policy(req.tool_approval_policy.as_deref());
    let mutations_allowed = policy == "open"
        || (policy == "review" && (req.tool_approval_grant || req.interactive_tool_approval));
    if !req.plan_mode && mutations_allowed && cwd.is_some() {
        "workspace-write"
    } else {
        "read-only"
    }
}

fn codex_approval_response(req: &CodexRunRequest, method: &str, params: &Value) -> Result<Value> {
    let policy = account_runtime_policy(req.tool_approval_policy.as_deref());
    let approved = codex_tools_allowed(req)
        && !(policy == "guarded" && method == "item/commandExecution/requestApproval");
    codex_decision_response(params, approved, approved && policy != "guarded")
}

fn codex_decision_response(params: &Value, approved: bool, prefer_session: bool) -> Result<Value> {
    let available = params.get("availableDecisions").and_then(Value::as_array);
    let has = |name: &str| {
        available.is_some_and(|decisions| {
            decisions.iter().any(|decision| {
                decision.as_str() == Some(name)
                    || decision
                        .as_object()
                        .is_some_and(|object| object.contains_key(name))
            })
        })
    };
    let candidates: &[&str] = if approved && prefer_session {
        &["acceptForSession", "accept"]
    } else if approved {
        &["accept", "acceptForSession"]
    } else {
        &["cancel", "decline"]
    };
    let decision = if available.is_some() {
        candidates.iter().copied().find(|candidate| has(candidate))
    } else if approved && prefer_session {
        Some("acceptForSession")
    } else if approved {
        Some("accept")
    } else {
        // Older Codex app-server releases did not advertise their decision enum.
        Some("decline")
    };
    decision
        .map(|decision| json!({ "decision": decision }))
        .ok_or_else(|| {
            Error::Other(format!(
                "Codex did not advertise a supported {} decision",
                if approved { "approval" } else { "denial" }
            ))
        })
}

fn permission_request_descriptor(params: &Value) -> Value {
    json!({
        "reason": params.get("reason").cloned().unwrap_or(Value::Null),
        "cwd": params.get("cwd").cloned().unwrap_or(Value::Null),
        "permissions": params.get("permissions").cloned().unwrap_or_else(|| json!({})),
    })
}

fn granted_permissions(params: &Value) -> Value {
    let Some(requested) = params.get("permissions").and_then(Value::as_object) else {
        return json!({});
    };
    let mut granted = serde_json::Map::new();
    for key in ["network", "fileSystem"] {
        if let Some(value) = requested.get(key).filter(|value| !value.is_null()) {
            granted.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(granted)
}

fn permission_response(params: &Value, approved: bool) -> Value {
    json!({
        "permissions": if approved { granted_permissions(params) } else { json!({}) },
        "scope": "turn",
    })
}

fn mcp_approval_descriptor(params: &Value) -> (&'static str, Value, bool) {
    let server_name =
        extract_string(params, &["serverName"]).unwrap_or_else(|| "MCP server".to_string());
    let message = extract_string(params, &["message"])
        .unwrap_or_else(|| "The MCP server requested input.".to_string());
    match params.get("mode").and_then(Value::as_str) {
        Some("form") => {
            match normalize_mcp_form_schema(params.get("requestedSchema").unwrap_or(&Value::Null)) {
                Ok(fields) => (
                    "mcp_form",
                    json!({ "server_name": server_name, "message": message, "fields": fields }),
                    true,
                ),
                Err(reason) => (
                    "mcp_unsupported",
                    json!({ "server_name": server_name, "message": message, "reason": reason }),
                    false,
                ),
            }
        }
        Some("url") => {
            let url = extract_string(params, &["url"]).unwrap_or_default();
            if safe_elicitation_url(&url) {
                (
                    "mcp_url",
                    json!({ "server_name": server_name, "message": message, "url": url }),
                    true,
                )
            } else {
                (
                    "mcp_unsupported",
                    json!({ "server_name": server_name, "message": message, "reason": "Only HTTP and HTTPS URLs can be opened." }),
                    false,
                )
            }
        }
        Some("openai/form") => (
            "mcp_unsupported",
            json!({ "server_name": server_name, "message": message, "reason": "OpenAI-specific MCP forms are not supported." }),
            false,
        ),
        _ => (
            "mcp_unsupported",
            json!({ "server_name": server_name, "message": message, "reason": "Unsupported MCP elicitation mode." }),
            false,
        ),
    }
}

fn normalize_mcp_form_schema(schema: &Value) -> std::result::Result<Vec<Value>, String> {
    let object = schema
        .as_object()
        .ok_or_else(|| "The form schema must be an object.".to_string())?;
    let allowed_root = ["$schema", "type", "properties", "required"];
    if object
        .keys()
        .any(|key| !allowed_root.contains(&key.as_str()))
    {
        return Err("The form uses unsupported schema composition or constraints.".to_string());
    }
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err("The form schema must have type object.".to_string());
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| "The form schema must define properties.".to_string())?;
    let required: HashSet<&str> = match object.get("required") {
        None => HashSet::new(),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "Required field names must be strings.".to_string())
            })
            .collect::<std::result::Result<_, _>>()?,
        Some(_) => return Err("The required field list must be an array.".to_string()),
    };
    if required.iter().any(|name| !properties.contains_key(*name)) {
        return Err("The form requires an unknown field.".to_string());
    }

    properties
        .iter()
        .map(|(name, schema)| {
            normalize_mcp_form_field(name, schema, required.contains(name.as_str()))
        })
        .collect()
}

fn normalize_mcp_form_field(
    name: &str,
    schema: &Value,
    required: bool,
) -> std::result::Result<Value, String> {
    let object = schema
        .as_object()
        .ok_or_else(|| format!("Field {name} must use a primitive schema."))?;
    let allowed = [
        "type",
        "title",
        "description",
        "default",
        "enum",
        "enumNames",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Field {name} uses an unsupported schema feature."));
    }
    let value_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Field {name} is missing a primitive type."))?;
    if !matches!(value_type, "string" | "number" | "integer" | "boolean") {
        return Err(format!("Field {name} has unsupported type {value_type}."));
    }
    for key in ["title", "description"] {
        if object.get(key).is_some_and(|value| !value.is_string()) {
            return Err(format!("Field {name} has an invalid {key}."));
        }
    }
    for key in ["minimum", "maximum"] {
        if object.get(key).is_some_and(|value| !value.is_number())
            || (!matches!(value_type, "number" | "integer") && object.contains_key(key))
        {
            return Err(format!("Field {name} has an invalid {key}."));
        }
    }
    for key in ["minLength", "maxLength"] {
        if object
            .get(key)
            .is_some_and(|value| value.as_u64().is_none())
            || (value_type != "string" && object.contains_key(key))
        {
            return Err(format!("Field {name} has an invalid {key}."));
        }
    }
    if object
        .get("minimum")
        .and_then(Value::as_f64)
        .is_some_and(|min| {
            object
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|max| min > max)
        })
        || object
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|min| {
                object
                    .get("maxLength")
                    .and_then(Value::as_u64)
                    .is_some_and(|max| min > max)
            })
    {
        return Err(format!("Field {name} has an invalid constraint range."));
    }
    let mut field = json!({
        "name": name,
        "label": object.get("title").and_then(Value::as_str).unwrap_or(name),
        "description": object.get("description").and_then(Value::as_str),
        "kind": value_type,
        "required": required,
    });
    for (source, target) in [
        ("default", "default"),
        ("minimum", "minimum"),
        ("maximum", "maximum"),
        ("minLength", "min_length"),
        ("maxLength", "max_length"),
    ] {
        if let Some(value) = object.get(source) {
            field[target] = value.clone();
        }
    }
    let enum_values = match object.get("enum") {
        None => None,
        Some(Value::Array(values)) => Some(values),
        Some(_) => return Err(format!("Field {name} has an invalid enum.")),
    };
    let enum_names = match object.get("enumNames") {
        None => None,
        Some(Value::Array(values)) => Some(values),
        Some(_) => return Err(format!("Field {name} has invalid enum labels.")),
    };
    if enum_names.is_some() && enum_values.is_none() {
        return Err(format!("Field {name} has enum labels without values."));
    }
    if let Some(values) = enum_values {
        if values.is_empty()
            || values
                .iter()
                .any(|value| !mcp_value_matches_type(value, value_type))
        {
            return Err(format!("Field {name} has an invalid enum."));
        }
        let names = enum_names;
        if names.is_some_and(|names| {
            names.len() != values.len() || names.iter().any(|value| !value.is_string())
        }) {
            return Err(format!("Field {name} has invalid enum labels."));
        }
        field["kind"] = Value::String("enum".to_string());
        field["value_type"] = Value::String(value_type.to_string());
        field["options"] = Value::Array(
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    json!({
                        "value": value,
                        "label": names.and_then(|names| names.get(index)).and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| scalar_label(value)),
                    })
                })
                .collect(),
        );
    }
    if let Some(default) = object.get("default") {
        validate_mcp_field_value(name, object, default)?;
    }
    Ok(field)
}

fn scalar_label(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn mcp_value_matches_type(value: &Value, value_type: &str) -> bool {
    match value_type {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        _ => false,
    }
}

fn validate_mcp_field_value(
    name: &str,
    schema: &serde_json::Map<String, Value>,
    value: &Value,
) -> std::result::Result<(), String> {
    let value_type = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !mcp_value_matches_type(value, value_type) {
        return Err(format!("Field {name} must be {value_type}."));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(format!("Field {name} must match an allowed value."));
        }
    }
    if let Some(text) = value.as_str() {
        let length = text.chars().count() as u64;
        if schema
            .get("minLength")
            .and_then(Value::as_u64)
            .is_some_and(|min| length < min)
            || schema
                .get("maxLength")
                .and_then(Value::as_u64)
                .is_some_and(|max| length > max)
        {
            return Err(format!("Field {name} has an invalid length."));
        }
    }
    if let Some(number) = value.as_f64() {
        if schema
            .get("minimum")
            .and_then(Value::as_f64)
            .is_some_and(|min| number < min)
            || schema
                .get("maximum")
                .and_then(Value::as_f64)
                .is_some_and(|max| number > max)
        {
            return Err(format!("Field {name} is outside the allowed range."));
        }
    }
    Ok(())
}

fn validate_mcp_form_response(
    schema: &Value,
    response: &Value,
) -> std::result::Result<Value, String> {
    let schema = schema
        .as_object()
        .ok_or_else(|| "Invalid form schema.".to_string())?;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| "Invalid form properties.".to_string())?;
    let required: HashSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let response = response
        .as_object()
        .ok_or_else(|| "Form response must be an object.".to_string())?;
    if response.keys().any(|name| !properties.contains_key(name)) {
        return Err("Form response contains an unknown field.".to_string());
    }
    for name in required {
        if !response.contains_key(name) {
            return Err(format!("Field {name} is required."));
        }
    }
    for (name, value) in response {
        let field = properties
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("Field {name} has an invalid schema."))?;
        validate_mcp_field_value(name, field, value)?;
    }
    Ok(Value::Object(response.clone()))
}

fn safe_elicitation_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn mcp_elicitation_response(
    params: &Value,
    supported: bool,
    decision: milim_agents::ApprovalDecision,
) -> (Value, bool, Option<String>) {
    let declined = || json!({ "action": "decline", "content": null, "_meta": null });
    if !supported || !decision.approved {
        return (declined(), false, None);
    }
    match params.get("mode").and_then(Value::as_str) {
        Some("form") => match decision
            .response
            .as_ref()
            .ok_or_else(|| "Form values are required.".to_string())
            .and_then(|response| {
                validate_mcp_form_response(
                    params.get("requestedSchema").unwrap_or(&Value::Null),
                    response,
                )
            }) {
            Ok(content) => (
                json!({ "action": "accept", "content": content, "_meta": null }),
                true,
                None,
            ),
            Err(error) => (declined(), false, Some(error)),
        },
        Some("url") => (
            json!({ "action": "accept", "content": null, "_meta": null }),
            true,
            None,
        ),
        _ => (declined(), false, None),
    }
}

fn protocol_notice(method: &str, params: &Value) -> Option<CodexStreamEvent> {
    let (message, detail) = match method {
        "configWarning" | "deprecationNotice" => (
            extract_string(params, &["summary"])
                .unwrap_or_else(|| "Codex reported a warning".to_string()),
            extract_string(params, &["details"]),
        ),
        "warning" => (
            extract_string(params, &["message"])
                .unwrap_or_else(|| "Codex reported a warning".to_string()),
            None,
        ),
        "model/rerouted" => {
            let from = extract_string(params, &["fromModel"])
                .unwrap_or_else(|| "requested model".to_string());
            let to =
                extract_string(params, &["toModel"]).unwrap_or_else(|| "another model".to_string());
            (
                format!("Codex rerouted {from} to {to}"),
                params.get("reason").map(scalar_label),
            )
        }
        "model/verification" => (
            "Codex model verification updated".to_string(),
            params.get("verifications").map(|value| value.to_string()),
        ),
        _ => return None,
    };
    Some(CodexStreamEvent::ProtocolNotice {
        kind: method.to_string(),
        message,
        detail,
    })
}

fn image_event_from_item(item: &Value) -> Option<CodexStreamEvent> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    if item_type != "imageGeneration" && item_type != "image_generation_call" {
        return None;
    }
    let result = item.get("result").and_then(Value::as_str)?;
    let url = image_result_url(result)?;
    Some(CodexStreamEvent::Image {
        id: extract_string(item, &["id"]).unwrap_or_else(|| "codex-image".to_string()),
        status: extract_string(item, &["status"]).unwrap_or_else(|| "completed".to_string()),
        url,
        revised_prompt: extract_string(item, &["revisedPrompt"])
            .or_else(|| extract_string(item, &["revised_prompt"])),
        saved_path: extract_string(item, &["savedPath"]),
    })
}

fn image_result_url(result: &str) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("data:image/")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return Some(trimmed.to_string());
    }
    // ponytail: base64 over SSE is enough for Codex image previews; save-to-file if payload size hurts.
    Some(format!("data:image/png;base64,{trimmed}"))
}

fn tool_event_from_item(item: &Value, running: bool) -> Option<CodexStreamEvent> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    let id = extract_string(item, &["id"]).unwrap_or_else(|| item_type.to_string());
    let status = if running {
        "running".to_string()
    } else {
        match item
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_ascii_lowercase()
            .as_str()
        {
            "failed" | "declined" | "cancelled" | "canceled" | "error" => "error".to_string(),
            _ => "done".to_string(),
        }
    };

    let (name, label, detail, icon) = match item_type {
        "commandExecution" => (
            "shell".to_string(),
            match status.as_str() {
                "running" => "Running command",
                "error" => "Command failed",
                _ => "Ran command",
            }
            .to_string(),
            command_detail(item),
            "command".to_string(),
        ),
        "fileChange" => (
            "patch_file".to_string(),
            match status.as_str() {
                "running" => "Editing files",
                "error" => "File edit failed",
                _ => "Edited files",
            }
            .to_string(),
            file_change_detail(item),
            "file".to_string(),
        ),
        "dynamicToolCall" | "mcpToolCall" | "collabToolCall" => {
            let tool = extract_string(item, &["tool"]).unwrap_or_else(|| item_type.to_string());
            (
                tool.clone(),
                match status.as_str() {
                    "running" => format!("Using {tool}"),
                    "error" => format!("{tool} failed"),
                    _ => format!("Used {tool}"),
                },
                compact_json(item.get("arguments").or_else(|| item.get("input"))),
                "tool".to_string(),
            )
        }
        "webSearch" => (
            "web_search".to_string(),
            match status.as_str() {
                "running" => "Searching web",
                "error" => "Web search failed",
                _ => "Searched web",
            }
            .to_string(),
            web_search_detail(item),
            "tool".to_string(),
        ),
        "imageView" => (
            "image_view".to_string(),
            match status.as_str() {
                "running" => "Viewing image",
                "error" => "Image view failed",
                _ => "Viewed image",
            }
            .to_string(),
            extract_string(item, &["path"]).map(|path| compact(&path, 110)),
            "screen".to_string(),
        ),
        "contextCompaction" => (
            "context_compaction".to_string(),
            match status.as_str() {
                "running" => "Compacting context",
                "error" => "Context compaction failed",
                _ => "Compacted context",
            }
            .to_string(),
            None,
            "tool".to_string(),
        ),
        _ => return None,
    };

    Some(CodexStreamEvent::Tool {
        id,
        name,
        status,
        label: Some(label),
        detail,
        icon: Some(icon),
    })
}

fn codex_native_worker_event(item: &Value) -> Option<AccountNativeWorkerLifecycle> {
    match item.get("type").and_then(Value::as_str)? {
        "collabAgentToolCall" | "collabToolCall" => {}
        _ => return None,
    }
    let call_id = extract_string(item, &["id"])?;
    let operation = extract_string(item, &["tool"])?;
    let parent_runtime_id = extract_string(item, &["senderThreadId"]);
    let worker_runtime_ids = item
        .get("receiverThreadIds")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if parent_runtime_id.is_none() && worker_runtime_ids.is_empty() {
        return None;
    }
    let workers = item
        .get("agentsStates")
        .and_then(Value::as_object)
        .map(|states| {
            states
                .iter()
                .filter_map(|(runtime_id, value)| {
                    Some(AccountNativeWorkerState {
                        runtime_id: runtime_id.clone(),
                        status: normalized_native_status(value.get("status")?.as_str()?),
                        message: value
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(AccountNativeWorkerLifecycle {
        runtime: "codex".to_string(),
        call_id,
        operation,
        status: normalized_native_status(
            item.get("status")
                .and_then(Value::as_str)
                .unwrap_or("inProgress"),
        ),
        parent_runtime_id,
        worker_runtime_ids,
        workers,
        prompt: extract_string(item, &["prompt"]),
        model: extract_string(item, &["model"]),
    })
}

fn normalized_native_status(status: &str) -> String {
    match status {
        "pendingInit" | "inProgress" => "running",
        "completed" | "shutdown" => "completed",
        "failed" | "errored" | "notFound" => "error",
        "interrupted" => "stopped",
        other => other,
    }
    .to_string()
}

fn command_detail(item: &Value) -> Option<String> {
    if let Some(command) = item.get("command").and_then(Value::as_str) {
        return Some(command.to_string());
    }
    item.get("command")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|command| !command.trim().is_empty())
}

fn file_change_detail(item: &Value) -> Option<String> {
    let changes = item.get("changes").and_then(Value::as_array)?;
    let paths = changes
        .iter()
        .filter_map(|change| {
            extract_string(change, &["path"])
                .or_else(|| extract_string(change, &["file"]))
                .or_else(|| extract_string(change, &["filePath"]))
        })
        .collect::<Vec<_>>();
    if paths.is_empty() {
        Some(format!("{} changes", changes.len()))
    } else {
        Some(compact(&paths.join(", "), 110))
    }
}

fn web_search_detail(item: &Value) -> Option<String> {
    extract_string(item, &["query"])
        .or_else(|| extract_string(item, &["action", "query"]))
        .or_else(|| {
            item.get("action")
                .and_then(|action| action.get("queries"))
                .and_then(Value::as_array)
                .map(|queries| {
                    queries
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
        })
        .or_else(|| extract_string(item, &["action", "url"]))
        .map(|detail| compact(&detail, 110))
}

fn compact_json(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(value.to_string())
}

fn compact(value: &str, limit: usize) -> String {
    let text = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > limit {
        let prefix = text
            .chars()
            .take(limit.saturating_sub(3))
            .collect::<String>();
        format!("{prefix}...")
    } else {
        text
    }
}

fn codex_developer_instructions() -> &'static str {
    "You are running inside Milim as a Codex-powered chat runtime. Answer in chat. Respect the tool approval and sandbox settings Milim supplies for each turn."
}

fn response_id(value: &Value) -> Option<u64> {
    value.get("id").and_then(Value::as_u64)
}

fn rpc_request_id(value: &Value) -> Option<Value> {
    value
        .get("id")
        .filter(|id| id.is_number() || id.is_string())
        .cloned()
}

fn noninteractive_request_response(method: &str) -> Option<Value> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            Some(json!({ "decision": "decline" }))
        }
        "item/permissions/requestApproval" => Some(json!({
            "permissions": {},
            "scope": "turn",
        })),
        "mcpServer/elicitation/request" => Some(json!({
            "action": "decline",
            "content": null,
            "_meta": null,
        })),
        _ => None,
    }
}

fn initialize_params(experimental: bool) -> Value {
    let mut params = json!({
        "clientInfo": {
            "name": CODEX_CLIENT_NAME,
            "title": CODEX_CLIENT_TITLE,
            "version": env!("CARGO_PKG_VERSION"),
        },
    });
    if experimental {
        params["capabilities"] = json!({ "experimentalApi": true });
    }
    params
}

fn extract_string(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str().map(str::to_string)
}

fn rpc_error_message(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn codex_spawn_error_message(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        return cli_path_warning("Codex", "codex");
    }
    format!(
        "failed to start `codex app-server`: {error}. Install or update the Codex CLI and make sure `codex` is on PATH."
    )
}

fn cli_path_warning(label: &str, command: &str) -> String {
    format!("{label} CLI was not found on PATH. Apps launched from the Dock or Finder do not inherit your shell PATH, so on macOS and Linux Milim also looks in the usual install directories (`~/.local/bin`, Homebrew, `~/.bun/bin`, and asdf/mise/volta shims). Install `{command}` into one of those, or launch Milim from a terminal.")
}

fn is_cli_path_warning(message: &str) -> bool {
    message.contains("CLI was not found on PATH")
}

fn usage_from_any(value: &Value) -> Option<Usage> {
    if let Some(usage) = usage_from_object(value) {
        return Some(usage);
    }
    match value {
        Value::Object(map) => {
            for key in [
                "usage",
                "tokenUsage",
                "token_usage",
                "response",
                "turn",
                "message",
                "result",
            ] {
                if let Some(usage) = map.get(key).and_then(usage_from_any) {
                    return Some(usage);
                }
            }
            map.values().find_map(usage_from_any)
        }
        Value::Array(items) => items.iter().find_map(usage_from_any),
        _ => None,
    }
}

fn usage_from_object(value: &Value) -> Option<Usage> {
    let prompt = opt_u32_any(value, &["prompt_tokens", "input_tokens"]);
    let completion = opt_u32_any(value, &["completion_tokens", "output_tokens"]);
    let total = opt_u32_any(value, &["total_tokens"]);
    if prompt.is_none() && completion.is_none() && total.is_none() {
        return None;
    }
    let prompt_tokens =
        prompt.unwrap_or_else(|| total.unwrap_or(0).saturating_sub(completion.unwrap_or(0)));
    let completion_tokens =
        completion.unwrap_or_else(|| total.unwrap_or(0).saturating_sub(prompt_tokens));
    Some(Usage {
        prompt_tokens,
        completion_tokens,
        total_tokens: total.unwrap_or(prompt_tokens + completion_tokens),
    })
}

fn cost_from_any(value: &Value) -> Option<f64> {
    match value {
        Value::Object(map) => {
            for key in ["total_cost_usd", "totalCostUsd", "cost_usd", "costUsd"] {
                if let Some(cost) = map.get(key).and_then(Value::as_f64) {
                    return Some(cost);
                }
            }
            map.values().find_map(cost_from_any)
        }
        Value::Array(items) => items.iter().find_map(cost_from_any),
        _ => None,
    }
}

fn opt_u32_any(value: &Value, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_nested_string() {
        let value = json!({ "thread": { "id": "thr_123" } });
        assert_eq!(
            extract_string(&value, &["thread", "id"]).as_deref(),
            Some("thr_123")
        );
    }

    #[test]
    fn rpc_error_prefers_message() {
        let value = json!({ "code": -1, "message": "not initialized" });
        assert_eq!(rpc_error_message(&value), "not initialized");
    }

    #[test]
    fn clean_model_uses_fallback_for_empty_values() {
        assert_eq!(clean_model(None), CODEX_MODEL_FALLBACK);
        assert_eq!(clean_model(Some("  ")), CODEX_MODEL_FALLBACK);
        assert_eq!(clean_model(Some(" gpt-5.4 ")), "gpt-5.4");
    }

    #[test]
    fn codex_turn_materializes_local_images_and_cleans_them_up() {
        let req = CodexRunRequest {
            prompt: String::new(),
            images: vec![AccountImage {
                media_type: "image/png".to_string(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC".to_string(),
            }],
            model: None,
            cwd: None,
            reasoning_effort: None,
            thread_id: None,
            persist_thread: false,
            tool_approval_policy: None,
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            milim_context: None,
            milim_mcp: None,
        };
        let materialized = CodexTurnImages::materialize(&req.images).unwrap().unwrap();
        let path = materialized.paths[0].clone();
        assert!(path.is_file());
        let input = codex_turn_input(&req, Some(&materialized));
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "localImage");
        assert_eq!(input[0]["path"], path.to_string_lossy().as_ref());
        drop(materialized);
        assert!(!path.exists());
    }

    #[test]
    fn account_images_reject_unsupported_mime_and_oversize_data() {
        let unsupported = AccountImage {
            media_type: "image/svg+xml".to_string(),
            data: "AAAA".to_string(),
        };
        assert!(validate_account_images(&[unsupported]).is_err());

        let oversized = AccountImage {
            media_type: "image/png".to_string(),
            data: base64::engine::general_purpose::STANDARD
                .encode(vec![0; MAX_ACCOUNT_IMAGE_BYTES + 1]),
        };
        assert!(validate_account_images(&[oversized]).is_err());
    }

    #[test]
    fn thread_request_resumes_or_starts_with_expected_persistence() {
        let req = CodexRunRequest {
            prompt: "hi".into(),
            images: Vec::new(),
            model: None,
            cwd: Some("C:\\repo".into()),
            reasoning_effort: None,
            thread_id: Some("thread-1".into()),
            persist_thread: true,
            tool_approval_policy: Some("open".into()),
            tool_approval_grant: true,
            interactive_tool_approval: false,
            plan_mode: false,
            milim_context: None,
            milim_mcp: None,
        };
        let (method, params) = codex_thread_request(&req, "gpt-5.4");
        assert_eq!(method, "thread/resume");
        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["sandbox"], "workspace-write");
        assert!(params.get("sandboxPolicy").is_none());
        assert!(params.get("ephemeral").is_none());

        let req = CodexRunRequest {
            thread_id: None,
            persist_thread: false,
            ..req
        };
        let (method, params) = codex_thread_request(&req, "gpt-5.4");
        assert_eq!(method, "thread/start");
        assert_eq!(params["ephemeral"], true);

        let req = CodexRunRequest {
            persist_thread: true,
            ..req
        };
        let (_, params) = codex_thread_request(&req, "gpt-5.4");
        assert!(params.get("ephemeral").is_none());
    }

    #[test]
    fn maps_milim_tool_modes_to_codex_permissions() {
        let mut req = CodexRunRequest {
            prompt: "hi".into(),
            images: Vec::new(),
            model: None,
            cwd: Some("C:\\repo".into()),
            reasoning_effort: None,
            thread_id: None,
            persist_thread: true,
            tool_approval_policy: Some("guarded".into()),
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            milim_context: None,
            milim_mcp: None,
        };
        assert_eq!(codex_approval_policy(&req), "on-request");
        assert_eq!(codex_sandbox_mode(&req, req.cwd.as_deref()), "read-only");
        assert_eq!(
            codex_sandbox_policy(&req, req.cwd.as_deref())["type"],
            "readOnly"
        );

        req.tool_approval_policy = Some("review".into());
        assert_eq!(codex_approval_policy(&req), "on-request");
        assert_eq!(codex_sandbox_mode(&req, req.cwd.as_deref()), "read-only");
        assert_eq!(
            codex_sandbox_policy(&req, req.cwd.as_deref())["type"],
            "readOnly"
        );

        req.interactive_tool_approval = true;
        assert_eq!(codex_approval_policy(&req), "on-request");
        assert_eq!(
            codex_sandbox_mode(&req, req.cwd.as_deref()),
            "workspace-write"
        );
        req.interactive_tool_approval = false;

        req.tool_approval_grant = true;
        assert_eq!(codex_approval_policy(&req), "never");
        assert_eq!(
            codex_sandbox_mode(&req, req.cwd.as_deref()),
            "workspace-write"
        );
        assert_eq!(
            codex_sandbox_policy(&req, req.cwd.as_deref())["type"],
            "workspaceWrite"
        );

        req.plan_mode = true;
        assert_eq!(codex_sandbox_mode(&req, req.cwd.as_deref()), "read-only");
        assert_eq!(
            codex_sandbox_policy(&req, req.cwd.as_deref())["type"],
            "readOnly"
        );
    }

    #[test]
    fn extracts_usage_from_nested_turn_payload() {
        let value = json!({
            "turn": {
                "status": "completed",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 5,
                    "total_tokens": 17
                }
            },
            "total_cost_usd": 0.002
        });
        let usage = usage_from_any(&value).expect("usage");
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 17);
        assert_eq!(cost_from_any(&value), Some(0.002));
    }

    #[test]
    fn parses_image_generation_items() {
        let event = image_event_from_item(&json!({
            "type": "imageGeneration",
            "id": "img_1",
            "status": "completed",
            "result": "abc123",
            "revisedPrompt": "a small diagram"
        }));
        assert!(matches!(
            event,
            Some(CodexStreamEvent::Image {
                id,
                status,
                url,
                revised_prompt: Some(prompt),
                ..
            }) if id == "img_1"
                && status == "completed"
                && url == "data:image/png;base64,abc123"
                && prompt == "a small diagram"
        ));
    }

    #[test]
    fn command_details_keep_full_copy_text() {
        let command = format!("powershell -Command \"{}\"", "x".repeat(140));
        assert_eq!(
            command_detail(&json!({ "command": command })).as_deref(),
            Some(command.as_str())
        );
        assert_eq!(
            compact_json(Some(&json!({ "command": command }))),
            Some(json!({ "command": command }).to_string())
        );
    }

    #[test]
    fn codex_decisions_follow_the_runtime_advertisement() {
        let params = json!({
            "availableDecisions": [
                "accept",
                { "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["git", "status"] } },
                "cancel"
            ]
        });
        assert_eq!(
            codex_decision_response(&params, true, false).unwrap(),
            json!({ "decision": "accept" })
        );
        assert_eq!(
            codex_decision_response(&params, false, false).unwrap(),
            json!({ "decision": "cancel" })
        );
        assert!(codex_decision_response(
            &json!({ "availableDecisions": ["accept"] }),
            false,
            false,
        )
        .is_err());
    }

    #[test]
    fn maps_codex_collab_lineage_to_native_worker_lifecycle() {
        let event = codex_native_worker_event(&json!({
            "type": "collabAgentToolCall",
            "id": "call-1",
            "tool": "spawnAgent",
            "status": "completed",
            "senderThreadId": "parent-thread",
            "receiverThreadIds": ["worker-thread"],
            "prompt": "review the parser",
            "model": "gpt-5.4-mini",
            "agentsStates": {
                "worker-thread": { "status": "completed", "message": "done" }
            }
        }))
        .expect("native worker event");
        assert_eq!(event.call_id, "call-1");
        assert_eq!(event.operation, "spawnAgent");
        assert_eq!(event.status, "completed");
        assert_eq!(event.parent_runtime_id.as_deref(), Some("parent-thread"));
        assert_eq!(event.worker_runtime_ids, vec!["worker-thread"]);
        assert_eq!(event.workers[0].runtime_id, "worker-thread");
        assert_eq!(event.workers[0].status, "completed");
    }

    #[test]
    fn ignores_codex_collab_items_without_lineage() {
        assert!(codex_native_worker_event(&json!({
            "type": "collabAgentToolCall",
            "id": "call-1",
            "tool": "spawnAgent",
            "status": "inProgress"
        }))
        .is_none());
    }

    #[test]
    fn stable_initialization_omits_experimental_capability() {
        assert!(initialize_params(false).get("capabilities").is_none());
        assert_eq!(
            initialize_params(true)["capabilities"]["experimentalApi"],
            true
        );
    }

    #[test]
    fn noninteractive_requests_decline_and_unknown_requests_are_unhandled() {
        assert_eq!(
            noninteractive_request_response("item/permissions/requestApproval").unwrap(),
            json!({ "permissions": {}, "scope": "turn" })
        );
        assert_eq!(
            noninteractive_request_response("mcpServer/elicitation/request").unwrap()["action"],
            "decline"
        );
        assert!(noninteractive_request_response("future/request").is_none());
        assert_eq!(
            rpc_request_id(&json!({ "id": "request-1" })),
            Some(json!("request-1"))
        );
    }

    #[test]
    fn permission_approval_grants_only_the_requested_profile_for_one_turn() {
        let params = json!({
            "permissions": {
                "network": { "enabled": true, "domains": ["example.com"] },
                "fileSystem": { "read": ["C:\\repo"], "write": ["C:\\repo\\out"] },
                "unexpected": { "admin": true }
            }
        });
        let approved = permission_response(&params, true);
        assert_eq!(approved["scope"], "turn");
        assert_eq!(
            approved["permissions"]["network"]["domains"][0],
            "example.com"
        );
        assert!(approved["permissions"].get("unexpected").is_none());
        assert_eq!(
            permission_response(&params, false)["permissions"],
            json!({})
        );
    }

    #[test]
    fn validates_bounded_mcp_forms_and_rejects_unsupported_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "title": "Name", "minLength": 1 },
                "count": { "type": "integer", "minimum": 1 },
                "enabled": { "type": "boolean", "default": true },
                "tone": { "type": "string", "enum": ["calm", "direct"] }
            },
            "required": ["name", "tone"]
        });
        let fields = normalize_mcp_form_schema(&schema).expect("supported form");
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[3]["kind"], "enum");
        assert!(validate_mcp_form_response(
            &schema,
            &json!({ "name": "Milim", "count": 2, "enabled": false, "tone": "direct" })
        )
        .is_ok());
        assert!(
            validate_mcp_form_response(&schema, &json!({ "name": "", "tone": "loud" })).is_err()
        );
        assert!(normalize_mcp_form_schema(&json!({
            "type": "object",
            "properties": { "nested": { "type": "object" } }
        }))
        .is_err());
        assert!(normalize_mcp_form_schema(&json!({
            "type": "object",
            "properties": {},
            "allOf": []
        }))
        .is_err());
        assert!(normalize_mcp_form_schema(&json!({
            "type": "object",
            "properties": {},
            "required": "name"
        }))
        .is_err());
        assert!(normalize_mcp_form_schema(&json!({
            "type": "object",
            "properties": { "tone": { "type": "string", "enum": "calm" } }
        }))
        .is_err());
    }

    #[test]
    fn mcp_urls_and_notices_are_normalized_safely() {
        assert!(safe_elicitation_url("https://example.com/continue"));
        assert!(!safe_elicitation_url("file:///C:/secret"));
        assert!(matches!(
            protocol_notice(
                "model/rerouted",
                &json!({ "fromModel": "gpt-a", "toModel": "gpt-b", "reason": "capacity" })
            ),
            Some(CodexStreamEvent::ProtocolNotice { message, .. }) if message.contains("gpt-a") && message.contains("gpt-b")
        ));
    }

    #[test]
    fn recovery_keeps_only_visible_messages_and_omits_media_paths() {
        let result = json!({ "thread": {
            "id": "thread-1",
            "name": "Recovered work",
            "preview": "preview",
            "cwd": "relative/path",
            "modelProvider": "openai",
            "createdAt": 10,
            "updatedAt": 20,
            "turns": [{ "items": [
                { "type": "userMessage", "content": [{ "type": "localImage", "path": "C:\\secret.png" }] },
                { "type": "reasoning", "content": ["hidden"] },
                { "type": "agentMessage", "text": "Visible answer" },
                { "type": "commandExecution", "command": "secret" }
            ] }]
        }});
        let recovered = recovered_thread_from_result(&result).expect("recoverable thread");
        assert_eq!(recovered.messages.len(), 2);
        assert_eq!(
            recovered.messages[0].content,
            "[Media omitted during Codex recovery]"
        );
        assert_eq!(recovered.messages[1].content, "Visible answer");
        assert!(recovered.cwd.is_none());
        assert_eq!(recovered.created_at_ms, 10_000);
    }

    #[test]
    fn paginated_fallback_is_narrow() {
        assert!(is_paginated_history_error("thread uses paginated history"));
        assert!(!is_paginated_history_error("thread not found"));
    }

    #[tokio::test]
    #[ignore = "requires an installed and authenticated Codex CLI"]
    async fn codex_app_server_smoke() {
        let mut proc = CodexProcess::start()
            .await
            .expect("start stable app-server");
        let models = proc
            .request(
                "model/list",
                Some(json!({ "includeHidden": false, "limit": 1 })),
            )
            .await
            .expect("model/list");
        assert!(models.get("data").is_some());
        let threads = proc
            .request("thread/list", Some(json!({ "limit": 1 })))
            .await
            .expect("thread/list");
        assert!(threads.get("data").is_some());
    }

    #[test]
    fn spawn_not_found_is_path_warning() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = codex_spawn_error_message(&error);
        assert!(is_cli_path_warning(&message));
        assert!(message.contains("codex"));
        assert!(message.contains("macOS"));
    }
}
