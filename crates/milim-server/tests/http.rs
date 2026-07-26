//! End-to-end HTTP tests: boot the real server over a loopback socket with the
//! deterministic test backend and assert the OpenAI/Ollama wire contract. This
//! is the Phase 1 verification gate.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use milim_core::api::openai::{DeltaFunction, DeltaToolCall, Model, ReasoningEffort, Usage};
use milim_core::config::ServerConfiguration;
use milim_inference::test_backend::TestBackend;
use milim_inference::{CompletionRequest, DeltaEvent, EventStream, ModelService, StreamEvent};
use milim_server::AppState;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Spawn the server on an ephemeral port; return its base URL.
async fn spawn(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        milim_server::serve_listener(state, listener).await.unwrap();
    });
    format!("http://{addr}")
}

async fn spawn_mobile(state: AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        milim_server::serve_mobile_companion_listener(state, listener)
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

fn test_state() -> AppState {
    AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
}

fn mobile_test_state() -> AppState {
    test_state().with_mobile_companion(Arc::new(
        milim_server::companion::MobileCompanionBridge::default(),
    ))
}

fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

struct NamedTestTool {
    name: &'static str,
}

#[async_trait]
impl milim_tools::Tool for NamedTestTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }

    fn effect(&self) -> milim_tools::ToolEffect {
        match self.name {
            "current_time"
            | "echo"
            | "http_fetch"
            | "read_file"
            | "read_file_anchors"
            | "list_dir"
            | "screenshot"
            | "preview_dom_snapshot"
            | "schedule_list"
            | "child_thread_list"
            | "child_thread_read"
            | "child_thread_wait" => milim_tools::ToolEffect::ReadOnly,
            "delegate_workers" => milim_tools::ToolEffect::Mutating,
            "shell" | "run_command" => milim_tools::ToolEffect::Command,
            _ => milim_tools::ToolEffect::Mutating,
        }
    }

    async fn invoke(&self, _args: Value) -> milim_core::Result<Value> {
        Ok(json!({"ok": true}))
    }
}

#[derive(Debug, Default)]
struct ToolListingBackend;

#[async_trait]
impl ModelService for ToolListingBackend {
    fn name(&self) -> &str {
        "tool-listing"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("tool-listing", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        let tools = req
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let has_workspace_notice = req.messages.iter().any(|message| {
            message
                .text_content()
                .contains("No working folder is selected")
        });
        let reply = format!("tools={tools};workspace_notice={has_workspace_notice}");
        let stream = async_stream::stream! {
            yield Ok(StreamEvent::Delta(DeltaEvent::text(reply.clone())));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[derive(Debug, Default, Clone)]
struct FormatCaptureBackend {
    seen: Arc<RwLock<Vec<Option<Value>>>>,
}

impl FormatCaptureBackend {
    fn seen(&self) -> Arc<RwLock<Vec<Option<Value>>>> {
        self.seen.clone()
    }
}

#[async_trait]
impl ModelService for FormatCaptureBackend {
    fn name(&self) -> &str {
        "format-capture"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("format-capture", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        self.seen.write().unwrap().push(req.response_format.clone());
        let stream = async_stream::stream! {
            yield Ok(StreamEvent::Delta(DeltaEvent::text("ok")));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[derive(Debug, Default)]
struct ReasoningStreamBackend;

#[async_trait]
impl ModelService for ReasoningStreamBackend {
    fn name(&self) -> &str {
        "reasoning-stream"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("reasoning-stream", 0)])
    }

    async fn stream(&self, _req: CompletionRequest) -> milim_core::Result<EventStream> {
        let stream = async_stream::stream! {
            yield Ok(StreamEvent::Delta(DeltaEvent {
                reasoning: Some("checking".to_string()),
                ..Default::default()
            }));
            yield Ok(StreamEvent::Delta(DeltaEvent::text("done")));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[derive(Debug, Default, Clone)]
struct MessageCaptureBackend {
    seen: Arc<RwLock<Vec<Vec<String>>>>,
}

impl MessageCaptureBackend {
    fn seen(&self) -> Arc<RwLock<Vec<Vec<String>>>> {
        self.seen.clone()
    }
}

#[async_trait]
impl ModelService for MessageCaptureBackend {
    fn name(&self) -> &str {
        "message-capture"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("message-capture", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        self.seen.write().unwrap().push(
            req.messages
                .iter()
                .map(|message| message.text_content())
                .collect(),
        );
        let stream = async_stream::stream! {
            yield Ok(StreamEvent::Delta(DeltaEvent::text("ok")));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[derive(Debug, Default, Clone)]
struct KeepAliveBackend {
    calls: KeepAliveCalls,
    stream_calls: Arc<AtomicUsize>,
}

type KeepAliveCalls = Arc<RwLock<Vec<(String, Option<Value>)>>>;

impl KeepAliveBackend {
    fn calls(&self) -> KeepAliveCalls {
        self.calls.clone()
    }

    fn stream_calls(&self) -> Arc<AtomicUsize> {
        self.stream_calls.clone()
    }
}

#[async_trait]
impl ModelService for KeepAliveBackend {
    fn name(&self) -> &str {
        "keep-alive"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("keep-alive", 0)])
    }

    async fn stream(&self, _req: CompletionRequest) -> milim_core::Result<EventStream> {
        self.stream_calls.fetch_add(1, Ordering::Relaxed);
        let stream = async_stream::stream! {
            yield Ok(StreamEvent::Delta(DeltaEvent::text("unexpected generation")));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn ollama_keep_alive(
        &self,
        model: &str,
        keep_alive: Option<Value>,
    ) -> milim_core::Result<bool> {
        self.calls
            .write()
            .unwrap()
            .push((model.to_string(), keep_alive));
        Ok(true)
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[derive(Debug, Default)]
struct ChildThreadToolBackend;

#[async_trait]
impl ModelService for ChildThreadToolBackend {
    fn name(&self) -> &str {
        "child-thread-tool"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("child-thread-tool", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        let has_tool_reply = req.messages.iter().any(|message| message.role == "tool");
        let last_user = req
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.text_content())
            .unwrap_or_default();
        let has_child_spawn = req
            .tools
            .iter()
            .any(|tool| tool.function.name == "delegate_workers");
        let stream = async_stream::stream! {
            if has_tool_reply {
                yield Ok(StreamEvent::Delta(DeltaEvent::text("parent saw child")));
                yield Ok(StreamEvent::Done {
                    finish_reason: "stop".to_string(),
                    usage: Usage::new(1, 1),
                });
                return;
            }
            if last_user.contains("child task") {
                let report = if last_user.contains("list child tools") {
                    let tools = req
                        .tools
                        .iter()
                        .map(|tool| tool.function.name.as_str())
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("child tools={tools}")
                } else {
                    "child report".to_string()
                };
                yield Ok(StreamEvent::Delta(DeltaEvent::text(report)));
                yield Ok(StreamEvent::Done {
                    finish_reason: "stop".to_string(),
                    usage: Usage::new(1, 1),
                });
                return;
            }
            if has_child_spawn {
                let arguments = if last_user.contains("missing child model") {
                    "{\"tasks\":[{\"prompt\":\"child task\",\"title\":\"Child worker\",\"model\":\"missing-child-model\"}]}"
                } else if last_user.contains("list child tools") {
                    "{\"tasks\":[{\"prompt\":\"child task list child tools\",\"title\":\"Child worker\"}]}"
                } else {
                    "{\"tasks\":[{\"prompt\":\"child task\",\"title\":\"Child worker\"}]}"
                };
                yield Ok(StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call_child".to_string()),
                        kind: Some("function".to_string()),
                        function: DeltaFunction {
                            name: Some("delegate_workers".to_string()),
                            arguments: Some(arguments.to_string()),
                        },
                    }],
                    ..Default::default()
                }));
                yield Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".to_string(),
                    usage: Usage::new(1, 1),
                });
                return;
            }
            yield Ok(StreamEvent::Delta(DeltaEvent::text("no child tool")));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(1, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|_| vec![0.0]).collect())
    }
}

#[tokio::test]
async fn mobile_companion_pairs_relays_and_revokes_device() {
    let base = spawn(mobile_test_state()).await;
    let client = reqwest::Client::new();

    let status: Value = client
        .post(format!("{base}/mobile/enabled"))
        .json(&json!({ "enabled": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["enabled"], true);

    let pairing: Value = client
        .post(format!("{base}/mobile/pairing"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pair_id = pairing["id"].as_str().unwrap();
    let secret = pairing["path"]
        .as_str()
        .unwrap()
        .split("secret=")
        .nth(1)
        .unwrap();

    let paired: Value = client
        .post(format!("{base}/mobile/pair"))
        .json(&json!({
            "pair_id": pair_id,
            "secret": secret,
            "device_name": "Pixel QA"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let device_id = paired["device_id"].as_str().unwrap();
    let device_key = paired["device_key"].as_str().unwrap();
    assert_eq!(paired["device_name"], "Pixel QA");

    let paired_pwa: Value = client
        .post(format!("{base}/mobile/pair"))
        .json(&json!({
            "pair_id": pair_id,
            "secret": secret,
            "device_name": "Pixel QA PWA"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let pwa_device_id = paired_pwa["device_id"].as_str().unwrap();
    let pwa_device_key = paired_pwa["device_key"].as_str().unwrap();
    assert_ne!(pwa_device_key, device_key);

    let no_thread = client
        .get(format!("{base}/mobile/thread"))
        .send()
        .await
        .unwrap();
    assert_eq!(no_thread.status(), reqwest::StatusCode::UNAUTHORIZED);

    let published: Value = client
        .post(format!("{base}/mobile/thread"))
        .json(&json!({
            "session_id": "session-1",
            "title": "Mobile QA",
            "model": "test-model",
            "busy": true,
            "messages": [
                { "role": "user", "content": "hello desktop" },
                { "role": "assistant", "content": "hello phone" }
            ],
            "threads": [
                { "id": "session-1", "title": "Mobile QA", "model": "test-model", "updated_at": 1, "busy": true, "project_label": "Milim", "project_path": "C:\\Dev\\milim" },
                { "id": "session-2", "title": "Second thread", "model": "test-model", "updated_at": 2 }
            ],
            "groups": [
                {
                    "id": "project:C:\\Dev\\milim",
                    "label": "Milim",
                    "subtitle": "C:\\Dev\\milim",
                    "project_id": "project:C:\\Dev\\milim",
                    "threads": [
                        { "id": "session-1", "title": "Mobile QA", "model": "test-model", "updated_at": 1, "busy": true, "project_label": "Milim", "project_path": "C:\\Dev\\milim" }
                    ]
                },
                {
                    "id": "chats",
                    "label": "Chats",
                    "threads": [
                        { "id": "session-2", "title": "Second thread", "model": "test-model", "updated_at": 2 }
                    ]
                }
            ],
            "models": [
                { "id": "test-model", "provider": "Test" },
                { "id": "other-model", "provider": "Test" }
            ],
            "theme": {
                "is_dark": false,
                "css_vars": {
                    "--bg-primary": "#fafafa",
                    "--primary-text": "#111111",
                    "--bg-image": "url(data:image/png;base64,abc)",
                    "bad-key": "ignored"
                },
                "background_fit": "contain",
                "background_treatment": "mono"
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(published["thread"]["version"], 1);

    let thread: Value = client
        .get(format!("{base}/mobile/thread"))
        .bearer_auth(device_key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(thread["thread"]["title"], "Mobile QA");
    assert_eq!(thread["thread"]["busy"], true);
    assert_eq!(thread["thread"]["messages"][0]["content"], "hello desktop");
    assert_eq!(thread["thread"]["threads"][1]["title"], "Second thread");
    assert_eq!(thread["thread"]["threads"][0]["project_label"], "Milim");
    assert_eq!(thread["thread"]["groups"][0]["label"], "Milim");
    assert_eq!(
        thread["thread"]["groups"][0]["threads"][0]["title"],
        "Mobile QA"
    );
    assert_eq!(thread["thread"]["groups"][1]["id"], "chats");
    assert_eq!(thread["thread"]["models"][1]["id"], "other-model");
    assert_eq!(thread["thread"]["theme"]["is_dark"], false);
    assert_eq!(
        thread["thread"]["theme"]["css_vars"]["--bg-primary"],
        "#fafafa"
    );
    assert_eq!(
        thread["thread"]["theme"]["css_vars"]["--bg-image"],
        "url(data:image/png;base64,abc)"
    );
    assert!(thread["thread"]["theme"]["css_vars"]["bad-key"].is_null());
    assert_eq!(thread["thread"]["theme"]["background_fit"], "contain");
    assert_eq!(thread["thread"]["theme"]["background_treatment"], "mono");

    let switch: Value = client
        .post(format!("{base}/mobile/relay"))
        .bearer_auth(device_key)
        .json(&json!({ "text": "session-2", "action": "switch_thread" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(switch["event"]["action"], "switch_thread");

    let switch_events: Value = client
        .get(format!("{base}/mobile/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(switch_events["events"][0]["action"], "switch_thread");

    let stop: Value = client
        .post(format!("{base}/mobile/relay"))
        .bearer_auth(device_key)
        .json(&json!({ "action": "stop" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stop["event"]["action"], "stop");

    let stop_events: Value = client
        .get(format!("{base}/mobile/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(stop_events["events"][0]["action"], "stop");

    let unauth = client
        .post(format!("{base}/mobile/relay"))
        .json(&json!({ "text": "should not relay", "action": "append" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let relay: Value = client
        .post(format!("{base}/mobile/relay"))
        .bearer_auth(device_key)
        .json(&json!({
            "text": "hello from phone",
            "action": "send",
            "attachments": [
                {
                    "id": "att-1",
                    "name": "note.txt",
                    "mime": "text/plain",
                    "size": 5,
                    "content": "hello"
                }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(relay["ok"], true);
    assert_eq!(relay["event"]["device_name"], "Pixel QA");

    let events: Value = client
        .get(format!("{base}/mobile/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events["events"].as_array().unwrap().len(), 1);
    assert_eq!(events["events"][0]["text"], "hello from phone");
    assert_eq!(events["events"][0]["action"], "send");
    assert_eq!(events["events"][0]["attachments"][0]["name"], "note.txt");

    let drained: Value = client
        .get(format!("{base}/mobile/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(drained["events"].as_array().unwrap().len(), 0);

    let revoked: Value = client
        .delete(format!("{base}/mobile/devices/{device_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revoked["devices"].as_array().unwrap().len(), 1);
    assert_eq!(revoked["devices"][0]["id"], pwa_device_id);

    let rejected = client
        .post(format!("{base}/mobile/relay"))
        .bearer_auth(device_key)
        .json(&json!({ "text": "after revoke", "action": "append" }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mobile_companion_phone_router_exposes_only_phone_routes() {
    let base = spawn_mobile(mobile_test_state()).await;
    let client = reqwest::Client::new();

    let page = client.get(format!("{base}/mobile")).send().await.unwrap();
    assert_eq!(page.status(), reqwest::StatusCode::OK);
    let page_text = page.text().await.unwrap();
    assert!(page_text.contains("Milim Relay"));
    assert!(page_text.contains("maximum-scale=1"));
    assert!(page_text.contains("/mobile/manifest.webmanifest"));
    assert!(page_text.contains("/mobile/icon.png"));
    assert!(page_text.contains("/mobile/wordmark.svg"));
    assert!(page_text.contains("--app-height"));
    assert!(page_text.contains("--composer-height"));
    assert!(page_text.contains("--message-actions-inset"));
    assert!(page_text.contains("visualViewport"));
    assert!(page_text.contains("syncComposerInset"));
    assert!(page_text.contains("scroll-padding-bottom"));
    assert!(page_text.contains("overscroll-behavior: contain"));
    assert!(page_text.contains("renderMarkdown"));
    assert!(page_text.contains("safeHref"));
    assert!(page_text.contains("Scan desktop QR"));
    assert!(page_text.contains("pairScanner"));
    assert!(page_text.contains("applyThemeSnapshot"));
    assert!(page_text.contains("bg-fit-cover"));
    assert!(page_text.contains("url.origin !== location.origin"));
    assert!(page_text.contains("thread-drawer"));
    assert!(page_text.contains("thread?.groups"));

    let manifest: Value = client
        .get(format!("{base}/mobile/manifest.webmanifest"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(manifest["start_url"], "/mobile");
    assert!(manifest["icons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|icon| icon["src"] == "/mobile/icon.png"));

    let service_worker = client
        .get(format!("{base}/mobile/sw.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(service_worker.status(), reqwest::StatusCode::OK);

    let icon = client
        .get(format!("{base}/mobile/icon.svg"))
        .send()
        .await
        .unwrap();
    assert_eq!(icon.status(), reqwest::StatusCode::OK);

    let icon_png = client
        .get(format!("{base}/mobile/icon.png"))
        .send()
        .await
        .unwrap();
    assert_eq!(icon_png.status(), reqwest::StatusCode::OK);

    let wordmark = client
        .get(format!("{base}/mobile/wordmark.svg"))
        .send()
        .await
        .unwrap();
    assert_eq!(wordmark.status(), reqwest::StatusCode::OK);

    let models = client
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(models.status(), reqwest::StatusCode::NOT_FOUND);

    let status = client
        .get(format!("{base}/mobile/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), reqwest::StatusCode::NOT_FOUND);

    let pairing = client
        .post(format!("{base}/mobile/pairing"))
        .send()
        .await
        .unwrap();
    assert_eq!(pairing.status(), reqwest::StatusCode::NOT_FOUND);

    let thread_publish = client
        .post(format!("{base}/mobile/thread"))
        .json(&json!({ "session_id": "x", "title": "x", "messages": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        thread_publish.status(),
        reqwest::StatusCode::METHOD_NOT_ALLOWED
    );

    let thread_events = client
        .get(format!("{base}/mobile/thread/events"))
        .send()
        .await
        .unwrap();
    assert_eq!(thread_events.status(), reqwest::StatusCode::UNAUTHORIZED);

    let relay = client
        .post(format!("{base}/mobile/relay"))
        .json(&json!({ "text": "phone only", "action": "append" }))
        .send()
        .await
        .unwrap();
    assert_eq!(relay.status(), reqwest::StatusCode::UNAUTHORIZED);
}

struct MemoryToolBackend;

#[async_trait]
impl ModelService for MemoryToolBackend {
    fn name(&self) -> &str {
        "memory-tool"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("test-memory", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        let has_tool_result = req.messages.iter().any(|m| m.role == "tool");
        let stream = async_stream::stream! {
            if !has_tool_result {
                yield Ok(StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call_memory".to_string()),
                        kind: Some("function".to_string()),
                        function: DeltaFunction {
                            name: Some("memory_register".to_string()),
                            arguments: None,
                        },
                    }],
                    ..Default::default()
                }));
                for frag in [
                    "{\"scope\":\"personal\",",
                    "\"title\":\"Remember memory breadcrumbs\",",
                    "\"content\":\"Show a breadcrumb when an agent registers memory.\"}"
                ] {
                    yield Ok(StreamEvent::Delta(DeltaEvent {
                        tool_calls: vec![DeltaToolCall {
                            index: 0,
                            id: None,
                            kind: None,
                            function: DeltaFunction {
                                name: None,
                                arguments: Some(frag.to_string()),
                            },
                        }],
                        ..Default::default()
                    }));
                }
                yield Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".to_string(),
                    usage: Usage::new(4, 4),
                });
                return;
            }

            yield Ok(StreamEvent::Delta(DeltaEvent::text("Done".to_string())));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(8, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs
            .into_iter()
            .map(|value| {
                let len = value.len().max(1) as f32;
                vec![len, 1.0]
            })
            .collect())
    }
}

struct ScheduleToolBackend;

#[async_trait]
impl ModelService for ScheduleToolBackend {
    fn name(&self) -> &str {
        "schedule-tool"
    }

    async fn list_models(&self) -> milim_core::Result<Vec<Model>> {
        Ok(vec![Model::local("test-schedule", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> milim_core::Result<EventStream> {
        let has_tool_result = req.messages.iter().any(|m| m.role == "tool");
        let stream = async_stream::stream! {
            if !has_tool_result {
                yield Ok(StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call_schedule".to_string()),
                        kind: Some("function".to_string()),
                        function: DeltaFunction {
                            name: Some("schedule_create".to_string()),
                            arguments: None,
                        },
                    }],
                    ..Default::default()
                }));
                for frag in [
                    "{\"name\":\"OSS Maintainer Orchestrator\",",
                    "\"cron\":\"0 */5 * * * *\",",
                    "\"model\":\"test-schedule\",",
                    "\"prompt\":\"Check the maintainer queue and report actionable changes.\"}"
                ] {
                    yield Ok(StreamEvent::Delta(DeltaEvent {
                        tool_calls: vec![DeltaToolCall {
                            index: 0,
                            id: None,
                            kind: None,
                            function: DeltaFunction {
                                name: None,
                                arguments: Some(frag.to_string()),
                            },
                        }],
                        ..Default::default()
                    }));
                }
                yield Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".to_string(),
                    usage: Usage::new(4, 4),
                });
                return;
            }

            yield Ok(StreamEvent::Delta(DeltaEvent::text("Scheduled.".to_string())));
            yield Ok(StreamEvent::Done {
                finish_reason: "stop".to_string(),
                usage: Usage::new(8, 1),
            });
        };
        Ok(Box::pin(stream))
    }

    async fn embed(&self, _model: &str, inputs: Vec<String>) -> milim_core::Result<Vec<Vec<f32>>> {
        Ok(inputs
            .into_iter()
            .map(|value| vec![value.len().max(1) as f32])
            .collect())
    }
}

#[derive(Debug)]
struct CapturedUpstreamRequest {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
    body: Value,
}

fn assert_openrouter_attribution(headers: &std::collections::HashMap<String, String>) {
    assert_eq!(
        headers.get("http-referer").map(String::as_str),
        Some("https://milim.ai/")
    );
    assert_eq!(
        headers.get("x-openrouter-title").map(String::as_str),
        Some("milim")
    );
}

async fn spawn_two_request_anthropic_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let (content_type, body) = if idx == 0 {
                (
                    "application/json",
                    r#"{"data":[{"id":"claude-sonnet-4-20250514","display_name":"Claude Sonnet 4"}],"has_more":false}"#,
                )
            } else {
                (
                    "text/event-stream",
                    concat!(
                        "event: message_start\n",
                        r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":5,"output_tokens":1}}}"#,
                        "\n\n",
                        "event: content_block_start\n",
                        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                        "\n\n",
                        "event: content_block_delta\n",
                        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"via anthropic"}}"#,
                        "\n\n",
                        "event: message_delta\n",
                        r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}"#,
                        "\n\n",
                        "event: message_stop\n",
                        r#"data: {"type":"message_stop"}"#,
                        "\n\n"
                    ),
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/v1"), handle)
}

async fn spawn_two_request_gemini_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let (content_type, body) = if idx == 0 {
                (
                    "application/json",
                    r#"{"models":[{"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash","inputTokenLimit":1048576,"outputTokenLimit":8192}]}"#,
                )
            } else {
                (
                    "text/event-stream",
                    concat!(
                        r#"data: {"candidates":[{"content":{"parts":[{"text":"via gemini"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":2,"totalTokenCount":7}}"#,
                        "\n\n"
                    ),
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/v1beta"), handle)
}

async fn spawn_two_request_openrouter_media_upstream(
    media_response_body: &'static str,
) -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let body = if idx == 0 {
                r#"{"data":[{"id":"google/gemini-2.5-flash-image","owned_by":"openrouter"}]}"#
            } else {
                media_response_body
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/api/v1"), handle)
}

async fn spawn_openrouter_metadata_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let body = match idx {
                0 => r#"{"data":[{"id":"google/gemini-2.5-flash-image","owned_by":"google"}]}"#,
                1 => {
                    r#"{"data":[{"id":"google/gemini-2.5-flash-image","name":"Gemini Flash Image","description":"Fast image generator","architecture":{"input_modalities":["text"],"output_modalities":["image"],"modality":"text->image"},"pricing":{"prompt":"0.0000001","completion":"0","image_output":"0.02"},"supported_parameters":["temperature","top_p","seed"],"default_parameters":{"temperature":0.7,"top_p":0.9}}]}"#
                }
                _ => {
                    r#"{"id":"google/gemini-2.5-flash-image","name":"Gemini Flash Image","description":"Fast image generator","architecture":{"input_modalities":["text"],"output_modalities":["image"],"modality":"text->image"},"endpoints":[{"name":"Google: Gemini Flash Image","provider_name":"Google","tag":"google","model_id":"google/gemini-2.5-flash-image","model_name":"Gemini Flash Image","context_length":32768,"pricing":{"prompt":"0.0000001","completion":"0","image_output":"0.02"},"quantization":null,"max_completion_tokens":8192,"max_prompt_tokens":32768,"supported_parameters":["temperature","top_p","seed"],"uptime_last_30m":100,"uptime_last_5m":100,"uptime_last_1d":100,"supports_implicit_caching":false,"latency_last_30m":null,"throughput_last_30m":null}]}"#
                }
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/api/v1"), handle)
}

async fn spawn_replicate_metadata_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let body = if idx == 0 {
                r#"{"query":"image","models":[{"model":{"owner":"black-forest-labs","name":"flux-schnell","description":"Fast text to image","latest_version":{"openapi_schema":{"components":{"schemas":{"Input":{"type":"object","properties":{"prompt":{"type":"string","title":"Prompt"},"aspect_ratio":{"type":"string","title":"Aspect Ratio","enum":["1:1","16:9"],"default":"1:1","description":"Output aspect ratio"},"num_outputs":{"type":"integer","title":"Num Outputs","minimum":1,"maximum":4,"default":1},"go_fast":{"type":"boolean","title":"Go Fast","default":true},"image_url":{"type":"string","title":"Image URL","format":"uri","description":"Optional input image URL"},"reference_images":{"type":"array","title":"Reference Images","description":"Reference image URLs","items":{"type":"string","format":"uri"}}}},"Output":{"type":"array","items":{"type":"string","format":"uri"}}}}}}},"metadata":{"generated_description":"Fast text to image","tags":["image"]}},{"model":{"owner":"audio-labs","name":"voice","description":"Audio only","latest_version":{"openapi_schema":{"components":{"schemas":{"Output":{"type":"string"}}}}}},"metadata":{"tags":["audio"]}}],"collections":[],"pages":[]}"#
            } else {
                r#"{"owner":"black-forest-labs","name":"flux-schnell","description":"Fast text to image","latest_version":{"openapi_schema":{"components":{"schemas":{"Input":{"type":"object","properties":{"prompt":{"type":"string","title":"Prompt"},"aspect_ratio":{"type":"string","title":"Aspect Ratio","enum":["1:1","16:9"],"default":"1:1","description":"Output aspect ratio"},"num_outputs":{"type":"integer","title":"Num Outputs","minimum":1,"maximum":4,"default":1},"go_fast":{"type":"boolean","title":"Go Fast","default":true},"image_url":{"type":"string","title":"Image URL","format":"uri","description":"Optional input image URL"},"reference_images":{"type":"array","title":"Reference Images","description":"Reference image URLs","items":{"type":"string","format":"uri"}}}},"Output":{"type":"array","items":{"type":"string","format":"uri"}}}}}}}"#
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/v1"), handle)
}

async fn spawn_fal_metadata_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let body = if idx == 0 {
                r#"{"models":[{"endpoint_id":"fal-ai/flux/schnell","metadata":{"display_name":"FLUX.1 [schnell]","category":"text-to-image","description":"Fast text-to-image generation","status":"active","tags":["image"],"model_url":"https://fal.run/fal-ai/flux/schnell"}},{"endpoint_id":"fal-ai/wan/v2.2-a14b/text-to-video","metadata":{"display_name":"WAN video","category":"text-to-video","description":"Video generation","status":"active","tags":["video"]}}],"next_cursor":null,"has_more":false}"#
            } else {
                r##"{"models":[{"endpoint_id":"fal-ai/flux/schnell","metadata":{"display_name":"FLUX.1 [schnell]","category":"text-to-image","description":"Fast text-to-image generation","status":"active","tags":["image"],"model_url":"https://fal.run/fal-ai/flux/schnell"},"openapi":{"openapi":"3.0.4","components":{"schemas":{"FluxSchnellInput":{"type":"object","required":["prompt"],"x-fal-order-properties":["prompt","image_size","num_images","sync_mode"],"properties":{"prompt":{"type":"string","title":"Prompt"},"image_size":{"type":"string","title":"Image Size","enum":["square_hd","landscape_4_3"],"default":"landscape_4_3"},"num_images":{"type":"integer","title":"Num Images","minimum":1,"maximum":4,"default":1},"sync_mode":{"type":"boolean","title":"Sync Mode","default":false}}}}},"paths":{"/fal-ai/flux/schnell":{"post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/FluxSchnellInput"}}}}}}}}}]}"##
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/v1"), handle)
}

async fn spawn_replicate_refresh_upstream() -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut out = Vec::new();
        for idx in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 1024];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            out.push(parse_upstream_request(&bytes));

            let name = if idx == 0 { "flux-schnell" } else { "flux-dev" };
            let body = format!(
                r#"{{"query":"flux","models":[{{"model":{{"owner":"black-forest-labs","name":"{name}","description":"Cached text to image","latest_version":{{"openapi_schema":{{"components":{{"schemas":{{"Input":{{"type":"object","properties":{{"prompt":{{"type":"string","title":"Prompt"}}}}}},"Output":{{"type":"array","items":{{"type":"string","format":"uri"}}}}}}}}}}}}}},"metadata":{{"generated_description":"Cached text to image","tags":["image"]}}}}],"collections":[],"pages":[]}}"#
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        out
    });

    (format!("http://{addr}/v1"), handle)
}

async fn spawn_one_request_json_upstream(
    response_body: &'static str,
) -> (String, tokio::task::JoinHandle<CapturedUpstreamRequest>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buf[..n]);
            if upstream_request_complete(&bytes) {
                break;
            }
        }
        let request = parse_upstream_request(&bytes);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_media_sequence_upstream(
    responses: Vec<(u16, &'static str, Vec<u8>)>,
) -> (
    String,
    tokio::task::JoinHandle<Vec<CapturedUpstreamRequest>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, content_type, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 2048];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if upstream_request_complete(&bytes) {
                    break;
                }
            }
            requests.push(parse_upstream_request(&bytes));
            let reason = match status {
                200 => "OK",
                202 => "Accepted",
                _ => "Response",
            };
            let headers = format!(
                "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(&body).await.unwrap();
        }
        requests
    });
    (format!("http://{addr}/api/v1"), handle)
}

fn upstream_request_complete(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let len = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("Content-Length:")
                .or_else(|| l.strip_prefix("content-length:"))
        })
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    body.len() >= len
}

fn parse_upstream_request(bytes: &[u8]) -> CapturedUpstreamRequest {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let first = lines.next().unwrap();
    let mut first_parts = first.split_whitespace();
    let method = first_parts.next().unwrap().to_string();
    let path = first_parts.next().unwrap().to_string();
    let headers = lines
        .filter_map(|line| {
            let (k, v) = line.split_once(':')?;
            Some((k.to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let body = if body.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(body).unwrap()
    };
    CapturedUpstreamRequest {
        method,
        path,
        headers,
        body,
    }
}

/// Reconstruct assistant text from an OpenAI SSE transcript.
fn reconstruct_sse(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(data) {
            if let Some(c) = v["choices"][0]["delta"]["content"].as_str() {
                out.push_str(c);
            }
        }
    }
    out
}

#[tokio::test]
async fn mcp_stdio_bridge_lists_and_calls() {
    use milim_server::mcp_bridge::handle_request;
    use milim_tools::ToolRegistry;

    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(ToolRegistry::with_builtins());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let init = handle_request(
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        &base,
        None,
        &client,
    )
    .await
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "milim");

    let list = handle_request(
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        &base,
        None,
        &client,
    )
    .await
    .unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"echo"));
    // MCP uses camelCase `inputSchema`.
    assert!(list["result"]["tools"][0]["inputSchema"].is_object());

    let call = handle_request(
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}),
        &base,
        None,
        &client,
    )
    .await
    .unwrap();
    assert!(call["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("hi"));

    // Notifications get no response.
    let none = handle_request(
        &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        &base,
        None,
        &client,
    )
    .await;
    assert!(none.is_none());
}

#[tokio::test]
async fn mcp_stdio_bridge_forwards_bearer_token() {
    use milim_server::mcp_bridge::handle_request;
    use milim_tools::ToolRegistry;

    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(ToolRegistry::with_builtins())
        .with_api_keys(["secret".to_string()])
        .with_loopback_trust(false);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let list = handle_request(
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        &base,
        Some("secret"),
        &client,
    )
    .await
    .unwrap();
    assert!(!list["result"]["tools"].as_array().unwrap().is_empty());

    let denied = handle_request(
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        &base,
        Some("wrong"),
        &client,
    )
    .await
    .unwrap();
    assert_eq!(denied["error"]["code"], -32603);
}

#[tokio::test]
async fn privacy_scan_redacts_pii() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/privacy/scan"))
        .json(&json!({"text":"Email a@b.com and SSN 123-45-6789"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["clean"], false);
    let redacted = v["redacted"].as_str().unwrap();
    assert!(redacted.contains("[EMAIL_1]"));
    assert!(redacted.contains("[SSN_1]"));
    assert!(!redacted.contains("a@b.com"));
    assert_eq!(v["map"]["[EMAIL_1]"], "a@b.com");
}

#[tokio::test]
async fn health_ok() {
    let base = spawn(test_state()).await;
    let r = reqwest::get(format!("{base}/health")).await.unwrap();
    assert!(r.status().is_success());
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["status"], "ok");
}

#[tokio::test]
async fn empty_cors_allowlist_does_not_allow_browser_origins() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();

    let r = client
        .get(format!("{base}/v1/models"))
        .header(reqwest::header::ORIGIN, "http://evil.example")
        .send()
        .await
        .unwrap();

    assert!(r
        .headers()
        .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[tokio::test]
async fn configured_cors_allowlist_allows_exact_origin() {
    let cfg = ServerConfiguration {
        allowed_origins: vec!["http://localhost:5180".to_string()],
        ..Default::default()
    };
    let state = AppState::new(Arc::new(TestBackend::new()), cfg);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let r = client
        .get(format!("{base}/v1/models"))
        .header(reqwest::header::ORIGIN, "http://localhost:5180")
        .send()
        .await
        .unwrap();

    assert_eq!(
        r.headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:5180")
    );
}

#[tokio::test]
async fn lists_models_openai_and_ollama() {
    let base = spawn(test_state()).await;
    let v: Value = reqwest::get(format!("{base}/v1/models"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["object"], "list");
    assert_eq!(v["data"][0]["id"], "test-echo");

    let v: Value = reqwest::get(format!("{base}/api/tags"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["models"][0]["name"], "test-echo");
}

#[tokio::test]
async fn audio_routes_are_absent() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    for route in [
        "/audio/transcriptions",
        "/audio/vad",
        "/audio/speech",
        "/audio/setup/check",
        "/audio/piper/presets/download",
        "/audio/kokoro/presets/download",
        "/audio/vad/presets/download",
        "/audio/piper/executable/install",
    ] {
        let response = client.post(format!("{base}{route}")).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND, "{route}");
    }
}

#[tokio::test]
async fn providers_discover_lists_local_candidates() {
    let base = spawn(test_state()).await;
    let body: Value = reqwest::Client::new()
        .get(format!("{base}/providers/discover"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let providers = body["providers"].as_array().expect("providers array");
    assert_eq!(providers.len(), 2);
    assert_eq!(providers[0]["name"], "Ollama (local)");
    assert_eq!(providers[0]["kind"], "openai_compatible");
    assert_eq!(providers[0]["base_url"], "http://localhost:11434/v1");
    assert_eq!(providers[0]["configured"], false);
    assert_eq!(providers[1]["name"], "LM Studio (local)");
    assert_eq!(providers[1]["kind"], "openai_compatible");
    assert_eq!(providers[1]["base_url"], "http://localhost:1234/v1");
    assert_eq!(providers[1]["configured"], false);
}

#[tokio::test]
async fn providers_accept_openai_compatible_wire_kind() {
    let tmp = std::env::temp_dir().join(format!(
        "milim-openai-provider-kind-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;

    let saved: Value = reqwest::Client::new()
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Local test",
            "kind": "openai_compatible",
            "base_url": "http://127.0.0.1:9/v1",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["kind"], "openai_compatible");
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn providers_accept_media_provider_kinds_without_chat_model_fetch() {
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-provider-kind-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    for (name, kind, base_url) in [
        ("Replicate", "replicate", "https://api.replicate.com/v1"),
        ("fal", "fal", "https://queue.fal.run"),
    ] {
        let saved: Value = client
            .post(format!("{base}/providers"))
            .json(&json!({
                "name": name,
                "kind": kind,
                "base_url": base_url,
                "api_key": "media-secret",
                "enabled": true
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(saved["kind"], kind);
        assert_eq!(saved["base_url"], base_url);
        assert_eq!(saved["has_key"], true);
        assert_eq!(saved["models"].as_array().unwrap().len(), 0);
        assert_eq!(saved["error"], Value::Null);
    }

    let providers: Value = client
        .get(format!("{base}/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let kinds = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|provider| provider["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"replicate"));
    assert!(kinds.contains(&"fal"));

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_replicate_uses_saved_provider_and_redacts_prompt() {
    let (upstream_base, upstream_request) = spawn_one_request_json_upstream(
        r#"{"id":"pred_123","status":"successful","output":["https://cdn.example.test/out.png"],"urls":{"web":"https://replicate.com/p/pred_123","get":"https://api.replicate.com/v1/predictions/pred_123"}}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-replicate-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::default());
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry)
        .with_privacy(privacy);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": format!("{upstream_base}/v1"),
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();
    client
        .post(format!("{base}/privacy/mode"))
        .json(&json!({ "mode": "redact" }))
        .send()
        .await
        .unwrap();

    let body: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "black-forest-labs/flux-schnell",
            "kind": "image",
            "prompt": "Render a poster for ada@example.com",
            "input": { "num_outputs": 1 }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["provider_kind"], "replicate");
    assert_eq!(body["id"], "pred_123");
    assert_eq!(body["status"], "successful");
    assert_eq!(body["media"][0]["url"], "https://cdn.example.test/out.png");
    assert_eq!(body["privacy"]["mode"], "redact");
    assert_eq!(body["privacy"]["redacted"], true);

    let request = upstream_request.await.unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/predictions");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer replicate-secret")
    );
    assert_eq!(
        request.headers.get("prefer").map(String::as_str),
        Some("wait")
    );
    assert_eq!(request.body["model"], "black-forest-labs/flux-schnell");
    assert_eq!(
        request.body["input"]["prompt"],
        "Render a poster for [EMAIL_1]"
    );
    assert_eq!(request.body["input"]["num_outputs"], 1);

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_fal_uses_queue_endpoint_and_key_auth() {
    let (upstream_base, upstream_request) = spawn_one_request_json_upstream(
        r#"{"request_id":"fal_req_123","status":"IN_QUEUE","response_url":"https://queue.fal.run/fal-ai/fast-sdxl/requests/fal_req_123"}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!("milim-media-fal-test-{}", uuid::Uuid::new_v4()));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "fal",
            "kind": "fal",
            "base_url": upstream_base,
            "api_key": "fal-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let body: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "fal-ai/fast-sdxl",
            "kind": "image",
            "prompt": "a small product photo",
            "input": { "image_size": "square_hd" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["provider_kind"], "fal");
    assert_eq!(body["id"], "fal_req_123");
    assert_eq!(body["status"], "IN_QUEUE");
    assert_eq!(
        body["urls"]["response"],
        "https://queue.fal.run/fal-ai/fast-sdxl/requests/fal_req_123"
    );

    let request = upstream_request.await.unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/fal-ai/fast-sdxl");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Key fal-secret")
    );
    assert_eq!(request.body["prompt"], "a small product photo");
    assert_eq!(request.body["image_size"], "square_hd");

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_openrouter_normalizes_to_image_only_output_modality() {
    let (upstream_base, upstream_requests) = spawn_two_request_openrouter_media_upstream(
        r#"{"id":"gen_123","choices":[{"message":{"role":"assistant","content":"done","images":[{"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}}]}}]}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-openrouter-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "OpenRouter",
            "kind": "openai_compatible",
            "base_url": upstream_base,
            "api_key": "openrouter-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let body: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "black-forest-labs/flux.2-klein-4b",
            "kind": "image",
            "prompt": "a compact product render",
            "input": {
                "modalities": ["image", "text"],
                "image_config": { "aspect_ratio": "16:9" }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["provider_kind"], "openai_compatible");
    assert_eq!(body["id"], "gen_123");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["media"][0]["url"], "data:image/png;base64,AAAA");
    assert_eq!(body["media"][0]["mime"], "image/png");

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/api/v1/models");
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer openrouter-secret")
    );
    assert_openrouter_attribution(&requests[0].headers);
    let request = &requests[1];
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/v1/chat/completions");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer openrouter-secret")
    );
    assert_openrouter_attribution(&request.headers);
    assert_eq!(request.body["model"], "black-forest-labs/flux.2-klein-4b");
    assert_eq!(request.body["messages"][0]["role"], "user");
    assert_eq!(
        request.body["messages"][0]["content"],
        "a compact product render"
    );
    assert_eq!(request.body["modalities"][0], "image");
    assert_eq!(request.body["modalities"].as_array().unwrap().len(), 1);
    assert_eq!(request.body["image_config"]["aspect_ratio"], "16:9");
    assert_eq!(request.body["stream"], false);

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_openrouter_video_uses_async_status_and_content_proxy() {
    let video_models = br#"{"data":[{"id":"google/veo-3","name":"Veo 3","description":"Video generation with optional audio","supported_parameters":["duration","resolution","aspect_ratio","generate_audio","seed"],"supported_durations":[4,8],"supported_resolutions":["720p","1080p"],"supported_aspect_ratios":["16:9","9:16"]}]}"#.to_vec();
    let (upstream_base, upstream_requests) = spawn_media_sequence_upstream(vec![
        (200, "application/json", br#"{"data":[]}"#.to_vec()),
        (200, "application/json", video_models.clone()),
        (200, "application/json", video_models),
        (202, "application/json", br#"{"id":"vid_123","status":"queued","polling_url":"/videos/vid_123"}"#.to_vec()),
        (200, "application/json", br#"{"id":"vid_123","status":"completed","unsigned_urls":["https://cdn.openrouter.ai/video.mp4"]}"#.to_vec()),
        (200, "video/mp4", b"video-bytes".to_vec()),
    ])
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-openrouter-video-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "OpenRouter",
            "kind": "openai_compatible",
            "base_url": upstream_base,
            "api_key": "openrouter-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=video"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"][0]["id"], "google/veo-3");
    assert_eq!(models["models"][0]["output_modalities"][0], "video");

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=google%2Fveo-3&kind=video"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "duration" && control["options"][0]["value"] == 4));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "generate_audio" && control["kind"] == "checkbox"));

    let submitted: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "kind": "video",
            "model": "google/veo-3",
            "prompt": "waves at sunset",
            "input": {"duration": 8, "resolution": "1080p", "generate_audio": true}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(submitted["id"], "vid_123");
    assert_eq!(submitted["status"], "queued");
    assert_eq!(submitted["kind"], "video");

    let completed: Value = client
        .get(format!(
            "{base}/media/status?provider_id={provider_id}&id=vid_123&model=google%2Fveo-3&kind=video"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["media"][0]["kind"], "video");
    assert_eq!(completed["media"][0]["requires_auth"], true);
    assert!(completed["media"][0]["url"]
        .as_str()
        .unwrap()
        .starts_with("/media/content?"));

    let content = client
        .get(format!(
            "{base}/media/content?provider_id={provider_id}&id=vid_123&index=0"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(content.headers()["content-type"], "video/mp4");
    assert_eq!(content.bytes().await.unwrap().as_ref(), b"video-bytes");

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[1].path, "/api/v1/videos/models");
    assert_eq!(requests[2].path, "/api/v1/videos/models");
    assert_eq!(requests[3].method, "POST");
    assert_eq!(requests[3].path, "/api/v1/videos");
    assert_eq!(requests[3].body["prompt"], "waves at sunset");
    assert_eq!(requests[3].body["generate_audio"], true);
    assert_eq!(requests[4].path, "/api/v1/videos/vid_123");
    assert_eq!(requests[5].path, "/api/v1/videos/vid_123/content?index=0");
    for request in &requests {
        assert_openrouter_attribution(&request.headers);
    }
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_openrouter_music_discovers_only_music_and_assembles_sse_audio() {
    let music_models = br#"{"data":[{"id":"google/lyria-3-pro-preview","name":"Lyria 3 Pro","description":"Prompt-to-music generation","architecture":{"input_modalities":["text"],"output_modalities":["text","audio"]},"supported_parameters":["seed","temperature"]},{"id":"openai/gpt-audio","name":"GPT Audio","description":"Voice and speech conversations","architecture":{"input_modalities":["text","audio"],"output_modalities":["text","audio"]}}]}"#.to_vec();
    let sse = concat!(
        "data: {\"id\":\"music_123\",\"choices\":[{\"delta\":{\"content\":\"Verse \",\"audio\":{\"data\":\"QUJD\",\"transcript\":\"la \"}}}]}\n\n",
        "data: {\"id\":\"music_123\",\"choices\":[{\"delta\":{\"content\":\"Chorus\",\"audio\":{\"data\":\"RA==\",\"transcript\":\"la\"}}}]}\n\n",
        "data: [DONE]\n\n"
    )
    .as_bytes()
    .to_vec();
    let (upstream_base, upstream_requests) = spawn_media_sequence_upstream(vec![
        (200, "application/json", br#"{"data":[]}"#.to_vec()),
        (200, "application/json", music_models),
        (
            200,
            "application/json",
            br#"{"endpoints":[{"supported_parameters":["seed","temperature"]}]}"#.to_vec(),
        ),
        (200, "text/event-stream", sse),
    ])
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-openrouter-music-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "OpenRouter",
            "kind": "openai_compatible",
            "base_url": upstream_base,
            "api_key": "openrouter-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=music"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
    assert_eq!(models["models"][0]["id"], "google/lyria-3-pro-preview");
    assert_eq!(models["models"][0]["output_modalities"][0], "music");

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=google%2Flyria-3-pro-preview&kind=music"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "temperature"));

    let generated: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "kind": "music",
            "model": "google/lyria-3-pro-preview",
            "prompt": "warm synthwave instrumental",
            "input": {"temperature": 0.5}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(generated["id"], "music_123");
    assert_eq!(generated["kind"], "music");
    assert_eq!(generated["media"][0]["kind"], "music");
    assert_eq!(generated["media"][0]["mime"], "audio/mpeg");
    assert_eq!(
        generated["media"][0]["url"],
        "data:audio/mpeg;base64,QUJDRA=="
    );
    assert_eq!(generated["output"]["text"], "Verse Chorus");
    assert_eq!(generated["output"]["transcript"], "la la");

    let invalid = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "kind": "speech",
            "model": "google/lyria-3-pro-preview",
            "prompt": "hello"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[1].path, "/api/v1/models?output_modalities=audio");
    assert_eq!(requests[3].path, "/api/v1/chat/completions");
    assert_eq!(requests[3].body["modalities"], json!(["text", "audio"]));
    assert_eq!(requests[3].body["stream"], true);
    for request in &requests {
        assert_openrouter_attribution(&request.headers);
    }
    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_openrouter_metadata_lists_image_models_and_schema_controls() {
    let (upstream_base, upstream_requests) = spawn_openrouter_metadata_upstream().await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-openrouter-metadata-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "OpenRouter",
            "kind": "openai_compatible",
            "base_url": upstream_base,
            "api_key": "openrouter-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"][0]["id"], "google/gemini-2.5-flash-image");
    assert_eq!(models["models"][0]["name"], "Gemini Flash Image");
    assert_eq!(models["models"][0]["output_modalities"][0], "image");
    assert_eq!(
        models["models"][0]["supported_parameters"][0],
        "temperature"
    );

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=google%2Fgemini-2.5-flash-image"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(schema["model"], "google/gemini-2.5-flash-image");
    assert_eq!(schema["supported_parameters"][0], "temperature");
    assert_eq!(
        schema["controls"][0]["path"].as_array().unwrap(),
        &vec![json!("image_config"), json!("aspect_ratio")]
    );
    assert_eq!(schema["controls"][0]["kind"], "select");
    assert_eq!(schema["controls"][0]["options"][0]["value"], "1:1");
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "seed" && control["kind"] == "number"));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "temperature" && control["default"] == 0.7));

    let requests = upstream_requests.await.unwrap();
    for request in &requests {
        assert_openrouter_attribution(&request.headers);
    }
    assert_eq!(requests[0].path, "/api/v1/models");
    assert_eq!(requests[1].path, "/api/v1/models?output_modalities=image");
    assert_eq!(
        requests[1].headers.get("authorization").map(String::as_str),
        Some("Bearer openrouter-secret")
    );
    assert_eq!(
        requests[2].path,
        "/api/v1/models/google/gemini-2.5-flash-image/endpoints"
    );
    assert_eq!(
        requests[2].headers.get("authorization").map(String::as_str),
        Some("Bearer openrouter-secret")
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_replicate_metadata_lists_image_models_and_schema_controls() {
    let (upstream_base, upstream_requests) = spawn_replicate_metadata_upstream().await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-replicate-metadata-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": upstream_base,
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
    assert_eq!(models["models"][0]["id"], "black-forest-labs/flux-schnell");
    assert_eq!(models["models"][0]["output_modalities"][0], "image");
    assert_eq!(
        models["models"][0]["supported_parameters"][0],
        "aspect_ratio"
    );

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=black-forest-labs%2Fflux-schnell"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(schema["model"], "black-forest-labs/flux-schnell");
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| { control["key"] == "aspect_ratio" && control["kind"] == "select" }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "num_outputs" && control["kind"] == "number" && control["max"] == 4.0
        }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "go_fast"
                && control["kind"] == "checkbox"
                && control["default"] == true
        }));

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1/search?query=image&limit=50");
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer replicate-secret")
    );
    assert_eq!(
        requests[1].path,
        "/v1/models/black-forest-labs/flux-schnell"
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_fal_metadata_lists_image_models_and_schema_controls() {
    let (upstream_base, upstream_requests) = spawn_fal_metadata_upstream().await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-fal-metadata-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "fal",
            "kind": "fal",
            "base_url": upstream_base,
            "api_key": "fal-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
    assert_eq!(models["models"][0]["id"], "fal-ai/flux/schnell");
    assert_eq!(models["models"][0]["name"], "FLUX.1 [schnell]");
    assert_eq!(models["models"][0]["output_modalities"][0], "image");

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=fal-ai%2Fflux%2Fschnell"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(schema["model"], "fal-ai/flux/schnell");
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| { control["key"] == "image_size" && control["kind"] == "select" }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "num_images" && control["kind"] == "number" && control["min"] == 1.0
        }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| { control["key"] == "sync_mode" && control["kind"] == "checkbox" }));

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        "/v1/models?limit=50&category=text-to-image&status=active"
    );
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Key fal-secret")
    );
    assert_eq!(
        requests[1].path,
        "/v1/models?endpoint_id=fal-ai%2Fflux%2Fschnell&expand=openapi-3.0"
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_music_catalogs_include_generators_and_exclude_voice_models() {
    let replicate_body = br#"{"models":[{"model":{"owner":"meta","name":"musicgen","description":"Generate original music from a prompt","latest_version":{"openapi_schema":{"components":{"schemas":{"Input":{"type":"object","properties":{"prompt":{"type":"string"}}},"AudioOutput":{"type":"string","format":"uri"}}}}}},"metadata":{"tags":["music","text-to-music"]}},{"model":{"owner":"voice-labs","name":"tts","description":"Text-to-speech voice generation","latest_version":{"openapi_schema":{"components":{"schemas":{"AudioOutput":{"type":"string","format":"uri"}}}}}},"metadata":{"tags":["audio","voice"]}}]}"#.to_vec();
    let (replicate_base, replicate_requests) =
        spawn_media_sequence_upstream(vec![(200, "application/json", replicate_body)]).await;
    let replicate_tmp = std::env::temp_dir().join(format!(
        "milim-media-replicate-music-test-{}",
        uuid::Uuid::new_v4()
    ));
    let replicate_registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(
            &replicate_tmp,
            Arc::new(TestBackend::new()),
        )
        .unwrap(),
    );
    let replicate_state =
        AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
            .with_providers(replicate_registry);
    let replicate_server = spawn(replicate_state).await;
    let client = reqwest::Client::new();
    let saved: Value = client
        .post(format!("{replicate_server}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": replicate_base,
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();
    let models: Value = client
        .get(format!(
            "{replicate_server}/media/models?provider_id={provider_id}&kind=music"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
    assert_eq!(models["models"][0]["id"], "meta/musicgen");
    assert_eq!(models["models"][0]["output_modalities"][0], "music");
    let requests = replicate_requests.await.unwrap();
    assert_eq!(requests[0].path, "/api/v1/search?query=music&limit=50");

    let fal_body = br#"{"models":[{"endpoint_id":"fal-ai/elevenlabs/music","metadata":{"display_name":"Eleven Music","category":"text-to-audio","description":"Generate original music and songs","tags":["music"]}},{"endpoint_id":"fal-ai/tts","metadata":{"display_name":"TTS","category":"text-to-audio","description":"Text-to-speech voice synthesis","tags":["voice","tts"]}},{"endpoint_id":"fal-ai/transcribe","metadata":{"display_name":"Transcribe","category":"text-to-audio","description":"Speech transcription","tags":["speech"]}}]}"#.to_vec();
    let (fal_base, fal_requests) =
        spawn_media_sequence_upstream(vec![(200, "application/json", fal_body)]).await;
    let fal_tmp = std::env::temp_dir().join(format!(
        "milim-media-fal-music-test-{}",
        uuid::Uuid::new_v4()
    ));
    let fal_registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&fal_tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let fal_state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(fal_registry);
    let fal_server = spawn(fal_state).await;
    let saved: Value = client
        .post(format!("{fal_server}/providers"))
        .json(&json!({
            "name": "fal",
            "kind": "fal",
            "base_url": fal_base,
            "api_key": "fal-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();
    let models: Value = client
        .get(format!(
            "{fal_server}/media/models?provider_id={provider_id}&kind=music"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["models"].as_array().unwrap().len(), 1);
    assert_eq!(models["models"][0]["id"], "fal-ai/elevenlabs/music");
    assert_eq!(models["models"][0]["output_modalities"][0], "music");
    let requests = fal_requests.await.unwrap();
    assert_eq!(
        requests[0].path,
        "/api/v1/models?limit=50&category=text-to-audio&status=active&q=music"
    );

    let _ = std::fs::remove_dir_all(replicate_tmp);
    let _ = std::fs::remove_dir_all(fal_tmp);
}

#[tokio::test]
async fn media_models_cache_uses_cached_list_until_refresh_requested() {
    let (upstream_base, upstream_requests) = spawn_replicate_refresh_upstream().await;
    let tmp = std::env::temp_dir().join(format!("milim-media-cache-test-{}", uuid::Uuid::new_v4()));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": upstream_base,
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let first: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image&q=flux"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first["models"][0]["id"], "black-forest-labs/flux-schnell");
    assert_eq!(first["cached"], false);

    let cached: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image&q=flux"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cached["models"][0]["id"], "black-forest-labs/flux-schnell");
    assert_eq!(cached["cached"], true);

    let refreshed: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image&q=flux&refresh=true"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refreshed["models"][0]["id"], "black-forest-labs/flux-dev");
    assert_eq!(refreshed["cached"], false);

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/v1/search?query=flux&limit=50");
    assert_eq!(requests[1].path, "/v1/search?query=flux&limit=50");

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_schema_controls_include_descriptions_urls_and_arrays() {
    let (upstream_base, upstream_request) = spawn_one_request_json_upstream(
        r#"{"owner":"black-forest-labs","name":"flux-schnell","description":"Fast text to image","latest_version":{"openapi_schema":{"components":{"schemas":{"Input":{"type":"object","properties":{"prompt":{"type":"string","title":"Prompt"},"aspect_ratio":{"type":"string","title":"Aspect Ratio","enum":["1:1","16:9"],"default":"1:1","description":"Output aspect ratio"},"num_outputs":{"type":"integer","title":"Num Outputs","minimum":1,"maximum":4,"default":1},"go_fast":{"type":"boolean","title":"Go Fast","default":true},"image_url":{"type":"string","title":"Image URL","format":"uri","description":"Optional input image URL"},"reference_images":{"type":"array","title":"Reference Images","description":"Reference image URLs","items":{"type":"string","format":"uri"}}}},"Output":{"type":"array","items":{"type":"string","format":"uri"}}}}}}}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-schema-rich-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": format!("{upstream_base}/v1"),
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=black-forest-labs%2Fflux-schnell"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "aspect_ratio"
                && control["description"] == "Output aspect ratio"
                && control["kind"] == "select"
        }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "image_url"
                && control["kind"] == "url"
                && control["description"] == "Optional input image URL"
        }));
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["key"] == "reference_images"
                && control["kind"] == "array"
                && control["item_kind"] == "url"
        }));

    let request = upstream_request.await.unwrap();
    assert_eq!(request.path, "/v1/models/black-forest-labs/flux-schnell");

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_status_replicate_fetches_prediction_and_extracts_media() {
    let (upstream_base, upstream_request) = spawn_one_request_json_upstream(
        r#"{"id":"pred_123","status":"succeeded","output":["https://cdn.example.test/out.png"],"urls":{"get":"https://api.replicate.com/v1/predictions/pred_123"}}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-replicate-status-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": format!("{upstream_base}/v1"),
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let status: Value = client
        .get(format!(
            "{base}/media/status?provider_id={provider_id}&id=pred_123&model=black-forest-labs%2Fflux-schnell"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["object"], "media.status");
    assert_eq!(status["status"], "succeeded");
    assert_eq!(
        status["media"][0]["url"],
        "https://cdn.example.test/out.png"
    );

    let request = upstream_request.await.unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/v1/predictions/pred_123");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer replicate-secret")
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_status_fal_fetches_result_url_and_extracts_images() {
    let (upstream_base, upstream_request) = spawn_one_request_json_upstream(
        r#"{"request_id":"fal_req_123","status":"COMPLETED","images":[{"url":"https://fal.media/files/out.png"}]}"#,
    )
    .await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-fal-status-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "fal",
            "kind": "fal",
            "base_url": upstream_base,
            "api_key": "fal-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();
    let url = reqwest::Url::parse_with_params(
        &format!("{base}/media/status"),
        &[
            ("provider_id", provider_id),
            ("id", "fal_req_123"),
            ("model", "fal-ai/flux/schnell"),
            (
                "response_url",
                &format!("{upstream_base}/fal-ai/flux/schnell/requests/fal_req_123"),
            ),
        ],
    )
    .unwrap();

    let status: Value = client.get(url).send().await.unwrap().json().await.unwrap();
    assert_eq!(status["object"], "media.status");
    assert_eq!(status["status"], "COMPLETED");
    assert_eq!(status["media"][0]["url"], "https://fal.media/files/out.png");

    let request = upstream_request.await.unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/fal-ai/flux/schnell/requests/fal_req_123");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Key fal-secret")
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_library_endpoints_list_serve_and_delete_saved_content() {
    use milim_server::media_library::{
        MediaDownloadSource, MediaLibrary, MediaLibraryMediaItem, MediaLibraryUpdate,
        NewMediaLibraryItem,
    };

    let root = unique_temp_path("milim-media-library-http");
    let library = MediaLibrary::open(root.clone()).unwrap();
    let item = library
        .create(NewMediaLibraryItem {
            provider_id: "provider-1".to_string(),
            provider: "Provider One".to_string(),
            provider_kind: "replicate".to_string(),
            kind: "image".to_string(),
            model: "owner/model".to_string(),
            prompt: "A blue chair".to_string(),
            input: json!({"aspect_ratio":"1:1"}),
            privacy: json!({"mode":"off","redacted":false}),
        })
        .unwrap();
    let data_url = "data:image/png;base64,iVBORw0KGgo=";
    library
        .update(
            &item.id,
            MediaLibraryUpdate {
                provider_run_id: "run-1".to_string(),
                status: "completed".to_string(),
                urls: Default::default(),
                media: vec![MediaLibraryMediaItem {
                    url: data_url.to_string(),
                    source_url: data_url.to_string(),
                    kind: "image".to_string(),
                    mime: Some("image/png".to_string()),
                    requires_auth: false,
                    file_name: None,
                    local_path: None,
                    size_bytes: None,
                }],
            },
        )
        .unwrap();
    library
        .save(
            &item.id,
            vec![MediaDownloadSource {
                url: data_url.to_string(),
                kind: "image".to_string(),
                mime: Some("image/png".to_string()),
                authorization: None,
            }],
        )
        .await
        .unwrap();

    let base = spawn(test_state().with_media_library(library)).await;
    let client = reqwest::Client::new();
    let page: Value = client
        .get(format!(
            "{base}/media/library?query=chair&status=ready&limit=1"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["items"][0]["id"], item.id);
    assert_eq!(page["items"][0]["save_state"], "ready");

    let content = client
        .get(format!("{base}/media/library/{}/content/0", item.id))
        .send()
        .await
        .unwrap();
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.headers()["content-type"], "image/png");
    assert_eq!(
        content.bytes().await.unwrap().as_ref(),
        b"\x89PNG\r\n\x1a\n"
    );

    let deleted: Value = client
        .delete(format!("{base}/media/library/{}", item.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
    assert!(!root.join("files").join(&item.id).exists());
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn media_real_fal_flux_schnell_smoke_when_env_configured() {
    if std::env::var("MILIM_REAL_MEDIA_SMOKE").ok().as_deref() != Some("1") {
        return;
    }
    let Ok(fal_key) = std::env::var("FAL_KEY").or_else(|_| std::env::var("FAL_API_KEY")) else {
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-real-fal-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "fal",
            "kind": "fal",
            "base_url": "https://queue.fal.run",
            "api_key": fal_key,
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image&q=flux+schnell&refresh=true"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(models["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["id"] == "fal-ai/flux/schnell"));

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=fal-ai%2Fflux%2Fschnell&refresh=true"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "image_size"));

    let generated: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "fal-ai/flux/schnell",
            "kind": "image",
            "prompt": "a tiny blue square icon on a white background",
            "input": {
                "image_size": "square",
                "num_images": 1,
                "num_inference_steps": 1,
                "sync_mode": false,
                "enable_safety_checker": true,
                "output_format": "jpeg"
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(generated["id"].as_str().unwrap_or_default().len() > 4);

    let mut latest = generated;
    for _ in 0..20 {
        if latest["media"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let status_url = reqwest::Url::parse_with_params(
            &format!("{base}/media/status"),
            &[
                ("provider_id", provider_id),
                ("id", latest["id"].as_str().unwrap_or_default()),
                ("model", "fal-ai/flux/schnell"),
                (
                    "response_url",
                    latest["urls"]["response"].as_str().unwrap_or_default(),
                ),
                (
                    "status_url",
                    latest["urls"]["status"].as_str().unwrap_or_default(),
                ),
            ],
        )
        .unwrap();
        latest = client
            .get(status_url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }
    assert!(
        latest["media"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "expected generated media from fal smoke response: {latest}"
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_real_replicate_flux_schnell_smoke_when_env_configured() {
    if std::env::var("MILIM_REAL_MEDIA_SMOKE").ok().as_deref() != Some("1") {
        return;
    }
    let Ok(replicate_key) =
        std::env::var("REPLICATE_API_TOKEN").or_else(|_| std::env::var("REPLICATE_API_KEY"))
    else {
        return;
    };
    let tmp = std::env::temp_dir().join(format!(
        "milim-media-real-replicate-test-{}",
        uuid::Uuid::new_v4()
    ));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": "https://api.replicate.com/v1",
            "api_key": replicate_key,
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();

    let models: Value = client
        .get(format!(
            "{base}/media/models?provider_id={provider_id}&kind=image&q=flux+schnell&refresh=true"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(models["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["id"] == "black-forest-labs/flux-schnell"));

    let schema: Value = client
        .get(format!(
            "{base}/media/model-schema?provider_id={provider_id}&model=black-forest-labs%2Fflux-schnell&refresh=true"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(schema["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| control["key"] == "num_outputs"));

    let generated: Value = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "black-forest-labs/flux-schnell",
            "kind": "image",
            "prompt": "a tiny blue square icon on a white background",
            "input": {
                "go_fast": true,
                "num_outputs": 1,
                "output_format": "jpg",
                "output_quality": 80,
                "num_inference_steps": 1
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(generated["id"].as_str().unwrap_or_default().len() > 4);

    let mut latest = generated;
    for _ in 0..20 {
        if latest["media"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let status_url = reqwest::Url::parse_with_params(
            &format!("{base}/media/status"),
            &[
                ("provider_id", provider_id),
                ("id", latest["id"].as_str().unwrap_or_default()),
                ("model", "black-forest-labs/flux-schnell"),
            ],
        )
        .unwrap();
        latest = client
            .get(status_url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }
    assert!(
        latest["media"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false),
        "expected generated media from Replicate smoke response: {latest}"
    );

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn media_generate_blocks_remote_prompt_when_privacy_gate_blocks_pii() {
    let tmp = std::env::temp_dir().join(format!("milim-media-block-test-{}", uuid::Uuid::new_v4()));
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::default());
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_providers(registry)
        .with_privacy(privacy);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Replicate",
            "kind": "replicate",
            "base_url": "http://127.0.0.1:9/v1",
            "api_key": "replicate-secret",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let provider_id = saved["id"].as_str().unwrap();
    client
        .post(format!("{base}/privacy/mode"))
        .json(&json!({ "mode": "block" }))
        .send()
        .await
        .unwrap();

    let response = client
        .post(format!("{base}/media/generate"))
        .json(&json!({
            "provider_id": provider_id,
            "model": "black-forest-labs/flux-schnell",
            "kind": "image",
            "prompt": "use ada@example.com in the image"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("blocked by the privacy gate"));

    let _ = std::fs::remove_dir_all(tmp);
}

#[tokio::test]
async fn account_runtime_images_require_privacy_off() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/privacy/mode"))
        .json(&json!({ "mode": "redact" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let response = client
        .post(format!("{base}/codex/run"))
        .json(&json!({
            "prompt": "",
            "images": [{ "media_type": "image/png", "data": "AAAA" }]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("image pixels can only be sent in Privacy Off"));
}

#[tokio::test]
async fn anthropic_provider_kind_routes_via_messages_api() {
    let (upstream_base, upstream_requests) = spawn_two_request_anthropic_upstream().await;
    let tmp = std::env::temp_dir().join(format!("milim-provider-kind-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::default());
    let state = AppState::new(
        Arc::new(registry.router(privacy.clone())),
        ServerConfiguration::default(),
    )
    .with_providers(registry)
    .with_privacy(privacy);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Anthropic",
            "kind": "anthropic",
            "base_url": upstream_base,
            "api_key": "sk-ant-test",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["kind"], "anthropic");
    assert_eq!(saved["models"][0], "claude-sonnet-4-20250514");

    let chat: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "claude-sonnet-4-20250514",
            "messages": [
                {"role": "system", "content": "Be direct."},
                {"role": "user", "content": "Ping"}
            ],
            "max_tokens": 32
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(chat["choices"][0]["message"]["content"], "via anthropic");

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1/models");
    assert_eq!(
        requests[0].headers.get("x-api-key").map(String::as_str),
        Some("sk-ant-test")
    );
    assert_eq!(
        requests[0]
            .headers
            .get("anthropic-version")
            .map(String::as_str),
        Some("2023-06-01")
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, "/v1/messages");
    assert_eq!(requests[1].body["system"], "Be direct.");
    assert_eq!(requests[1].body["messages"][0]["content"], "Ping");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn gemini_provider_kind_routes_via_generate_content_api() {
    let (upstream_base, upstream_requests) = spawn_two_request_gemini_upstream().await;
    let tmp = std::env::temp_dir().join(format!(
        "milim-gemini-provider-kind-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    let registry = Arc::new(
        milim_server::providers::ProviderRegistry::open(&tmp, Arc::new(TestBackend::new()))
            .unwrap(),
    );
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::default());
    let state = AppState::new(
        Arc::new(registry.router(privacy.clone())),
        ServerConfiguration::default(),
    )
    .with_providers(registry)
    .with_privacy(privacy);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let saved: Value = client
        .post(format!("{base}/providers"))
        .json(&json!({
            "name": "Gemini",
            "kind": "gemini",
            "base_url": upstream_base,
            "api_key": "AIza-test",
            "enabled": true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["kind"], "gemini");
    assert_eq!(saved["models"][0], "gemini-2.5-flash");

    let listed: Value = client
        .get(format!("{base}/v1/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let gemini_model = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "gemini-2.5-flash")
        .unwrap();
    assert_eq!(gemini_model["context_length"], 1_048_576);
    assert_eq!(gemini_model["max_prompt_tokens"], 1_048_576);
    assert_eq!(gemini_model["max_completion_tokens"], 8192);

    let chat: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "gemini-2.5-flash",
            "messages": [
                {"role": "system", "content": "Be direct."},
                {"role": "user", "content": "Ping"}
            ],
            "max_tokens": 32
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(chat["choices"][0]["message"]["content"], "via gemini");

    let requests = upstream_requests.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/v1beta/models");
    assert_eq!(
        requests[0]
            .headers
            .get("x-goog-api-key")
            .map(String::as_str),
        Some("AIza-test")
    );
    assert_eq!(requests[1].method, "POST");
    assert_eq!(
        requests[1].path,
        "/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
    );
    assert_eq!(
        requests[1].body["systemInstruction"]["parts"][0]["text"],
        "Be direct."
    );
    assert_eq!(requests[1].body["contents"][0]["parts"][0]["text"], "Ping");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn openai_chat_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"ping"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["object"], "chat.completion");
    assert_eq!(v["choices"][0]["message"]["role"], "assistant");
    assert_eq!(v["choices"][0]["message"]["content"], "Echo: ping");
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    assert!(v["usage"]["total_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn openai_chat_streaming_sse() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let text = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model":"test-echo",
            "messages":[{"role":"user","content":"ping"}],
            "stream":true,
            "stream_options":{"include_usage":true}
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // Wire shape: first chunk carries role; a finish chunk; a usage chunk; DONE.
    assert!(text.contains("\"role\":\"assistant\""));
    assert!(text.contains("\"finish_reason\":\"stop\""));
    assert!(text.contains("\"usage\""));
    assert!(text.trim_end().ends_with("data: [DONE]"));
    // Reassembled content matches the non-streamed answer.
    assert_eq!(reconstruct_sse(&text), "Echo: ping");
}

#[tokio::test]
async fn openai_chat_streams_tool_calls() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model":"test-echo",
            "messages":[{"role":"user","content":"/tool please"}],
            "tools":[{"type":"function","function":{"name":"echo","parameters":{}}}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    let call = &v["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "echo");
    assert_eq!(call["function"]["arguments"], "{\"text\":\"test\"}");
}

#[tokio::test]
async fn openai_legacy_completions_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/v1/completions"))
        .json(&json!({"model":"test-echo","prompt":"finish this","stream":false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["object"], "text_completion");
    assert_eq!(v["choices"][0]["text"], "Echo: finish this");
    assert_eq!(v["choices"][0]["finish_reason"], "stop");
    assert!(v["usage"]["total_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn openai_responses_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/v1/responses"))
        .json(&json!({"model":"test-echo","input":"hello responses"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["object"], "response");
    assert_eq!(v["status"], "completed");
    assert_eq!(v["output"][0]["type"], "message");
    assert_eq!(v["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(
        v["output"][0]["content"][0]["text"],
        "Echo: hello responses"
    );
    assert!(v["usage"]["total_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn compatibility_routes_accept_multimodal_image_inputs() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC";
    let data_url = format!("data:image/png;base64,{png}");

    let chat: Value = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model": "test-echo",
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "chat image" },
                { "type": "image_url", "image_url": { "url": data_url.clone() } }
            ] }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(chat["choices"][0]["message"]["content"], "Echo: chat image");

    let responses: Value = client
        .post(format!("{base}/v1/responses"))
        .json(&json!({
            "model": "test-echo",
            "input": [{ "role": "user", "content": [
                { "type": "input_text", "text": "responses image" },
                { "type": "input_image", "image_url": data_url }
            ] }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        responses["output"][0]["content"][0]["text"],
        "Echo: responses image"
    );

    let ollama: Value = client
        .post(format!("{base}/api/chat"))
        .json(&json!({
            "model": "test-echo",
            "stream": false,
            "messages": [{ "role": "user", "content": "ollama image", "images": [png] }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ollama["message"]["content"], "Echo: ollama image");

    let anthropic: Value = client
        .post(format!("{base}/anthropic/v1/messages"))
        .json(&json!({
            "model": "test-echo",
            "max_tokens": 100,
            "messages": [{ "role": "user", "content": [
                { "type": "text", "text": "anthropic images" },
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": png } },
                { "type": "image", "source": { "type": "url", "url": "https://example.com/shape.png" } }
            ] }]
        }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(anthropic["content"][0]["text"], "Echo: anthropic images");
}

#[tokio::test]
async fn anthropic_compatibility_rejects_malformed_image_sources() {
    let base = spawn(test_state()).await;
    let response = reqwest::Client::new()
        .post(format!("{base}/anthropic/v1/messages"))
        .json(&json!({
            "model": "test-echo",
            "max_tokens": 100,
            "messages": [{ "role": "user", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/svg+xml", "data": "AAAA" } }
            ] }]
        }))
        .send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn openai_responses_rejects_malformed_function_tool() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/v1/responses"))
        .json(&json!({
            "model": "test-echo",
            "input": "hello responses",
            "tools": [{
                "type": "function",
                "function": {
                    "description": "missing name"
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn ollama_chat_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/api/chat"))
        .json(
            &json!({"model":"test-echo","messages":[{"role":"user","content":"x"}],"stream":false}),
        )
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["done"], true);
    assert_eq!(v["message"]["content"], "Echo: x");
    assert_eq!(v["done_reason"], "stop");
}

#[tokio::test]
async fn ollama_chat_maps_think_to_reasoning_effort() {
    let backend = TestBackend::new();
    let base = spawn(AppState::new(
        Arc::new(backend.clone()),
        ServerConfiguration::default(),
    ))
    .await;
    let client = reqwest::Client::new();

    client
        .post(format!("{base}/api/chat"))
        .json(&json!({
            "model":"test-echo",
            "messages":[{"role":"user","content":"x"}],
            "stream":false,
            "think":"high"
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(backend.last_reasoning_effort(), Some(ReasoningEffort::High));

    client
        .post(format!("{base}/api/chat"))
        .json(&json!({
            "model":"test-echo",
            "messages":[{"role":"user","content":"x"}],
            "stream":false,
            "think":false
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    assert_eq!(backend.last_reasoning_effort(), Some(ReasoningEffort::None));
}

#[tokio::test]
async fn ollama_chat_streams_thinking() {
    let state = AppState::new(
        Arc::new(ReasoningStreamBackend),
        ServerConfiguration::default(),
    );
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let text = client
        .post(format!("{base}/api/chat"))
        .json(&json!({"model":"reasoning-stream","messages":[{"role":"user","content":"x"}]}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let lines: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(lines.iter().any(|v| v["message"]["thinking"] == "checking"));

    let content: String = lines
        .iter()
        .filter_map(|v| v["message"]["content"].as_str().map(str::to_string))
        .collect();
    assert_eq!(content, "done");
}

#[tokio::test]
async fn ollama_generate_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/api/generate"))
        .json(&json!({
            "model":"test-echo",
            "prompt":"write one line",
            "stream":false,
            "raw":true,
            "suffix":" done",
            "keep_alive":"5m"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["done"], true);
    assert_eq!(v["response"], "Echo: write one line");
    assert_eq!(v["done_reason"], "stop");
}

#[tokio::test]
async fn ollama_generate_empty_prompt_keep_alive_uses_lifecycle_hook() {
    let backend = KeepAliveBackend::default();
    let calls = backend.calls();
    let stream_calls = backend.stream_calls();
    let state = AppState::new(Arc::new(backend), ServerConfiguration::default());
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/api/generate"))
        .json(&json!({
            "model":"keep-alive",
            "prompt":"",
            "stream":false,
            "keep_alive":0
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["done"], true);
    assert_eq!(v["done_reason"], "unload");
    assert_eq!(v["response"], "");
    assert_eq!(stream_calls.load(Ordering::Relaxed), 0);
    let calls = calls.read().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "keep-alive");
    assert_eq!(calls[0].1, Some(json!(0)));
}

#[tokio::test]
async fn ollama_chat_streams_ndjson() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let text = client
        .post(format!("{base}/api/chat"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"x y"}]}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    // Every line is a standalone JSON object; the last has done=true.
    let last: Value = serde_json::from_str(lines.last().unwrap()).unwrap();
    assert_eq!(last["done"], true);
    assert_eq!(last["eval_count"].as_u64().unwrap(), 3); // "Echo: x y" -> 3 words

    let content: String = lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["message"]["content"].as_str().map(str::to_string))
        .collect();
    assert_eq!(content, "Echo: x y");
}

#[tokio::test]
async fn structured_output_formats_reach_backend() {
    let backend = FormatCaptureBackend::default();
    let seen = backend.seen();
    let state = AppState::new(Arc::new(backend), ServerConfiguration::default());
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let openai_format = json!({
        "type":"json_schema",
        "json_schema":{
            "name":"answer",
            "schema":{"type":"object","properties":{"answer":{"type":"string"}}}
        }
    });
    client
        .post(format!("{base}/v1/chat/completions"))
        .json(&json!({
            "model":"format-capture",
            "messages":[{"role":"user","content":"json please"}],
            "response_format": openai_format
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    client
        .post(format!("{base}/api/chat"))
        .json(&json!({
            "model":"format-capture",
            "messages":[{"role":"user","content":"json please"}],
            "stream":false,
            "format":{"type":"object","properties":{"answer":{"type":"string"}}}
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let formats = seen.read().unwrap().clone();
    assert_eq!(formats.len(), 2);
    assert_eq!(formats[0], Some(openai_format));
    let ollama_format = formats[1].as_ref().unwrap();
    assert_eq!(ollama_format["type"], "json_schema");
    assert_eq!(ollama_format["json_schema"]["name"], "ollama_schema");
    assert_eq!(
        ollama_format["json_schema"]["schema"]["properties"]["answer"]["type"],
        "string"
    );
}

#[tokio::test]
async fn openai_embeddings() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/v1/embeddings"))
        .json(&json!({"model":"test-echo","input":"hello world"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["object"], "list");
    let emb = v["data"][0]["embedding"].as_array().unwrap();
    assert_eq!(emb.len(), 16);
}

#[tokio::test]
async fn workspace_git_status_without_folder() {
    let base = spawn(test_state()).await;
    let v: Value = reqwest::get(format!("{base}/workspace/git"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["state"], "no_folder");
    assert_eq!(v["is_repo"], false);
}

#[tokio::test]
async fn workspace_git_status_reports_selected_repo() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-status");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    for index in 0..25 {
        fs::write(root.join(format!("note-{index:02}.txt")), "hello\n").unwrap();
    }

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let set: Value = client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(set["folder"].as_str().unwrap().contains("milim-git-status"));

    let v: Value = client
        .get(format!("{base}/workspace/git"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["state"], "ready");
    assert_eq!(v["is_repo"], true);
    assert_eq!(v["untracked"], 25);
    assert_eq!(v["has_changes"], true);
    assert_eq!(v["changed_file_count"], 25);
    assert_eq!(v["changed_files"].as_array().unwrap().len(), 25);
    assert_eq!(v["changed_files"][0]["status"], "??");
    assert_eq!(v["changed_files"][0]["path"], "note-00.txt");
    assert_eq!(v["recent_commits"].as_array().unwrap().len(), 0);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_status_reports_recent_commits_newest_first() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-recent-commits");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(init.status.success());
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Milim Test"].as_slice(),
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    for (file, contents, subject) in [
        ("one.txt", "one\n", "first project commit"),
        ("two.txt", "two\n", "second project commit"),
    ] {
        fs::write(root.join(file), contents).unwrap();
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["add", file])
            .output()
            .unwrap()
            .status
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["commit", "-m", subject])
            .output()
            .unwrap()
            .status
            .success());
    }

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();
    let status: Value = client
        .get(format!("{base}/workspace/git"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let commits = status["recent_commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0]["subject"], "second project commit");
    assert_eq!(commits[1]["subject"], "first project commit");
    assert!(!commits[0]["hash"].as_str().unwrap().is_empty());

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_branch_actions_checkout_and_create() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-branches");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(root.join("note.txt"), "hello\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());
    let initial = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["branch", "--show-current"])
        .output()
        .unwrap();
    let initial_branch = String::from_utf8_lossy(&initial.stdout).trim().to_string();
    assert!(!initial_branch.is_empty());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["branch", "feature/one"])
        .output()
        .unwrap()
        .status
        .success());

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();

    let status: Value = client
        .get(format!("{base}/workspace/git"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["branch"].as_str().unwrap(), initial_branch);
    assert!(status["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| { branch["name"] == initial_branch && branch["current"] == true }));
    assert!(status["branches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|branch| { branch["name"] == "feature/one" }));

    let checkout: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "checkout_branch", "branch": "feature/one" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(checkout["ok"], true);

    let create: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "create_branch", "branch": "codex/new-branch" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(create["ok"], true);

    let next: Value = client
        .get(format!("{base}/workspace/git"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(next["branch"], "codex/new-branch");

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_action_diff_reports_patch() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-action");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(root.join("note.txt"), "hello\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["branch", "comparison-base"])
        .output()
        .unwrap()
        .status
        .success());
    fs::write(root.join("note.txt"), "committed\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "second"])
        .output()
        .unwrap()
        .status
        .success());
    let second_commit = String::from_utf8_lossy(
        &Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    fs::write(root.join("note.txt"), "hello there\n").unwrap();
    fs::write(root.join("Code block 1"), "new code\n").unwrap();
    fs::write(
        root.join("large.txt"),
        format!("{}diff-tail-marker\n", "expanded line\n".repeat(2_000)),
    )
    .unwrap();

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();

    let v: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "diff" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["action"], "diff");
    assert!(v["stdout"].as_str().unwrap().contains("note.txt"));
    assert!(v["stdout"].as_str().unwrap().contains("hello there"));
    assert!(v["stdout"].as_str().unwrap().contains("Code block 1"));
    assert!(v["stdout"].as_str().unwrap().contains("new code"));
    assert!(v["stdout"].as_str().unwrap().len() > 24_000);
    assert!(v["stdout"].as_str().unwrap().contains("diff-tail-marker"));
    assert_eq!(v["truncated"], false);

    let commit: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "diff",
            "diff_scope": "commit",
            "diff_base": second_commit,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(commit["stdout"].as_str().unwrap().contains("committed"));

    let branch: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "diff",
            "diff_scope": "branch",
            "diff_base": "comparison-base",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(branch["stdout"].as_str().unwrap().contains("committed"));

    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    let staged: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "diff", "diff_scope": "staged" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(staged["stdout"].as_str().unwrap().contains("note.txt"));
    assert!(!staged["stdout"].as_str().unwrap().contains("Code block 1"));
    assert!(!staged["stdout"]
        .as_str()
        .unwrap()
        .contains("diff-tail-marker"));

    let unstaged: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "diff", "diff_scope": "unstaged" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!unstaged["stdout"].as_str().unwrap().contains("note.txt"));
    assert!(unstaged["stdout"]
        .as_str()
        .unwrap()
        .contains("Code block 1"));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_action_checkpoint_restores_worktree() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-checkpoint");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(root.join("note.txt"), "base\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());
    let head_before = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;

    fs::write(root.join("note.txt"), "before turn\n").unwrap();
    fs::write(root.join("user-note.txt"), "user scratch\n").unwrap();

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();

    let checkpoint: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "checkpoint", "message": "test-turn" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(checkpoint["ok"], true);
    let checkpoint_ref = checkpoint["checkpoint"].as_str().unwrap();
    assert!(checkpoint_ref.starts_with("refs/milim/checkpoints/"));

    fs::write(root.join("note.txt"), "after turn\n").unwrap();
    fs::write(root.join("user-note.txt"), "changed by turn\n").unwrap();
    fs::write(root.join("agent-new.txt"), "new by turn\n").unwrap();

    std::thread::sleep(Duration::from_secs(1));
    let newer_checkpoint: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "checkpoint", "message": "newer-turn" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(newer_checkpoint["ok"], true);
    fs::write(root.join("note.txt"), "after newer turn\n").unwrap();

    let last_turn: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "diff", "diff_scope": "last_turn" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(last_turn["ok"], true, "{last_turn}");
    assert!(last_turn["stdout"]
        .as_str()
        .unwrap()
        .contains("after newer turn"));
    assert!(!last_turn["stdout"]
        .as_str()
        .unwrap()
        .contains("changed by turn"));

    let selected_turn: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "diff",
            "diff_scope": "last_turn",
            "diff_base": checkpoint_ref,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(selected_turn["ok"], true, "{selected_turn}");
    assert!(selected_turn["stdout"]
        .as_str()
        .unwrap()
        .contains("changed by turn"));
    assert!(selected_turn["stdout"]
        .as_str()
        .unwrap()
        .contains("agent-new.txt"));

    let invalid_turn: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "diff",
            "diff_scope": "last_turn",
            "diff_base": "HEAD",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invalid_turn["ok"], false, "{invalid_turn}");
    assert!(invalid_turn["message"]
        .as_str()
        .unwrap()
        .contains("valid Milim turn checkpoint"));

    let missing_turn: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "diff",
            "diff_scope": "last_turn",
            "diff_base": "refs/milim/checkpoints/missing",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(missing_turn["ok"], false, "{missing_turn}");
    assert!(missing_turn["message"]
        .as_str()
        .unwrap()
        .contains("valid Milim turn checkpoint"));

    let restored: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "restore_checkpoint", "checkpoint": checkpoint_ref }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(restored["ok"], true);
    assert_eq!(
        fs::read_to_string(root.join("note.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "before turn\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("user-note.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "user scratch\n"
    );
    assert!(!root.join("agent-new.txt").exists());
    let head_after = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap()
        .stdout;
    assert_eq!(head_before, head_after);

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_hot_swap_retry_isolated_and_applied() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-hot-swap");
    fs::create_dir_all(&root).unwrap();
    assert!(Command::new("git")
        .arg("init")
        .arg(&root)
        .output()
        .unwrap()
        .status
        .success());
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    fs::write(root.join("note.txt"), "base\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();
    let checkpoint: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "checkpoint", "message": "hot-swap-base" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let checkpoint_ref = checkpoint["checkpoint"].as_str().unwrap();
    let created: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "create_retry_worktree", "checkpoint": checkpoint_ref }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["ok"], true, "{created}");
    let worktree = PathBuf::from(created["worktree"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("note.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "base\n"
    );
    fs::write(worktree.join("note.txt"), "retry\n").unwrap();
    fs::write(worktree.join("new.txt"), "new\n").unwrap();

    let applied: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({
            "action": "apply_retry_worktree",
            "checkpoint": checkpoint_ref,
            "worktree": worktree,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(applied["ok"], true, "{applied}");
    assert!(applied["undo_checkpoint"].as_str().is_some());
    assert_eq!(
        fs::read_to_string(root.join("note.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "retry\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("new.txt"))
            .unwrap()
            .replace("\r\n", "\n"),
        "new\n"
    );

    let removed: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "remove_retry_worktree", "worktree": worktree }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(removed["ok"], true, "{removed}");
    assert!(!worktree.exists());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn workspace_git_action_commit_stages_and_commits() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }

    let root = unique_temp_path("milim-git-commit");
    fs::create_dir_all(&root).unwrap();
    let init = Command::new("git").arg("init").arg(&root).output().unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    for args in [
        ["config", "user.email", "test@example.com"].as_slice(),
        ["config", "user.name", "Test User"].as_slice(),
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    fs::write(root.join("note.txt"), "hello\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["add", "note.txt"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());
    fs::write(root.join("note.txt"), "hello there\n").unwrap();
    fs::write(root.join("extra.txt"), "extra\n").unwrap();

    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/workspace"))
        .json(&json!({ "folder": root }))
        .send()
        .await
        .unwrap();

    let v: Value = client
        .post(format!("{base}/workspace/git/action"))
        .json(&json!({ "action": "commit", "message": "panel commit", "stage_all": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["action"], "commit");
    assert!(v["command"].as_str().unwrap().contains("git add -A"));

    let status = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["status", "--short"])
        .output()
        .unwrap();
    assert!(status.status.success());
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");

    let subject = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "-1", "--pretty=%s"])
        .output()
        .unwrap();
    assert!(subject.status.success());
    assert_eq!(
        String::from_utf8_lossy(&subject.stdout).trim(),
        "panel commit"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn auth_required_when_keys_configured() {
    let state = test_state()
        .with_api_keys(["secret".to_string()])
        .with_loopback_trust(false);
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = json!({"model":"test-echo","messages":[{"role":"user","content":"hi"}]});

    // Missing key -> 401.
    let r = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // Valid key -> 200.
    let r = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth("secret")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
}

#[tokio::test]
async fn mcp_tools_list_and_call() {
    use milim_tools::ToolRegistry;
    let mut tools = ToolRegistry::with_builtins();
    for name in ["read_file_anchors", "patch_file", "write_file"] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(tools);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    // List tools.
    let v: Value = reqwest::get(format!("{base}/mcp/tools"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = v["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"echo"));
    assert!(names.contains(&"current_time"));
    assert!(!names.contains(&"read_file_anchors"));
    assert!(!names.contains(&"patch_file"));

    // Call echo.
    let v: Value = client
        .post(format!("{base}/mcp/call"))
        .json(&json!({"name":"echo","arguments":{"text":"hi"}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["result"]["echoed"]["text"], "hi");

    let denied = client
        .post(format!("{base}/mcp/call"))
        .json(&json!({"name":"write_file","arguments":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::BAD_REQUEST);

    // Unknown tool -> 400.
    let r = client
        .post(format!("{base}/mcp/call"))
        .json(&json!({"name":"nope","arguments":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mcp_server_tools_follow_chat_approval_policy() {
    let root = unique_temp_path("milim-mcp-server-tools-policy");
    let hub = Arc::new(milim_mcp_client::McpHub::open(&root));
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::new())
        .with_mcp(hub);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let catalog: Value = client
        .get(format!("{base}/mcp/tools"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let effect = |name: &str| {
        catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()["effect"]
            .as_str()
            .unwrap()
    };
    assert_eq!(effect("mcp_server_list"), "read_only");
    assert_eq!(effect("mcp_server_test"), "command");
    assert_eq!(effect("mcp_server_save"), "command");
    assert_eq!(effect("mcp_server_delete"), "mutating");

    let offered = |policy: &'static str, interactive: bool| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}/agents/run"))
                .json(&json!({
                    "model": "tool-listing",
                    "messages": [{ "role": "user", "content": "list tools" }],
                    "tool_approval_policy": policy,
                    "interactive_tool_approval": interactive
                }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["message"]["content"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };

    let guarded = offered("guarded", false).await;
    assert!(guarded.contains("mcp_server_list"), "{guarded}");
    assert!(!guarded.contains("mcp_server_save"), "{guarded}");
    assert!(!guarded.contains("mcp_server_test"), "{guarded}");
    assert!(!guarded.contains("mcp_server_delete"), "{guarded}");

    for content in [offered("review", true).await, offered("open", false).await] {
        for name in [
            "mcp_server_list",
            "mcp_server_test",
            "mcp_server_save",
            "mcp_server_delete",
        ] {
            assert!(content.contains(name), "missing {name}: {content}");
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn mcp_apps_http_bridge_auth_validation_and_isolation() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let root = unique_temp_path("milim-mcp-apps-http");
    fs::create_dir_all(&root).unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("milim-mcp-client")
        .join("tests")
        .join("fixtures")
        .join("apps_server.js");
    let hub = Arc::new(milim_mcp_client::McpHub::open(&root));
    hub.upsert(milim_mcp_client::McpServerConfig {
        id: "apps-a".into(),
        name: "Apps fixture".into(),
        command: "node".into(),
        args: vec![fixture.to_string_lossy().into_owned()],
        cwd: None,
        env: Vec::new(),
        enabled: true,
    })
    .await
    .unwrap();
    let state = test_state()
        .with_mcp(hub)
        .with_api_keys(["secret".to_string()])
        .with_loopback_trust(false);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let unauthenticated = client
        .post(format!("{base}/mcp/apps/resources/read"))
        .json(&json!({"server_id":"apps-a","uri":"ui://milim.test/chart"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let resource: Value = client
        .post(format!("{base}/mcp/apps/resources/read"))
        .bearer_auth("secret")
        .json(&json!({"server_id":"apps-a","uri":"ui://milim.test/chart"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        resource["result"]["contents"][0]["mimeType"],
        "text/html;profile=mcp-app"
    );
    let rendered: Value = client
        .post(format!("{base}/mcp/apps/resources/read"))
        .bearer_auth("secret")
        .json(&json!({"server_id":"apps-a","uri":"ui://milim.test/chart","render":true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(rendered["result"].is_null());
    let view_path = rendered["view_path"].as_str().unwrap();
    let view = client
        .get(format!("{base}{view_path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(view.status(), reqwest::StatusCode::OK);
    assert_eq!(view.headers()["cache-control"], "no-store");
    assert!(view
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("default-src 'none'")));
    assert!(view.text().await.unwrap().contains("Interactive chart"));
    let consumed_view = client
        .get(format!("{base}{view_path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(consumed_view.status(), reqwest::StatusCode::BAD_REQUEST);

    let missing_view = client
        .get(format!("{base}/mcp/apps/views/not-a-capability"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_view.status(), reqwest::StatusCode::BAD_REQUEST);

    for uri in [
        "ui://milim.test/not-advertised",
        "ui://milim.test/bad",
        "ui://milim.test/large",
    ] {
        let response = client
            .post(format!("{base}/mcp/apps/resources/read"))
            .bearer_auth("secret")
            .json(&json!({"server_id":"apps-a","uri":uri}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST, "{uri}");
    }

    let review_denied = client
        .post(format!("{base}/mcp/apps/tools/call"))
        .bearer_auth("secret")
        .json(&json!({
            "server_id":"apps-a",
            "name":"refresh_chart",
            "arguments":{},
            "approval":"review"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(review_denied.status(), reqwest::StatusCode::BAD_REQUEST);

    let approved: Value = client
        .post(format!("{base}/mcp/apps/tools/call"))
        .bearer_auth("secret")
        .json(&json!({
            "server_id":"apps-a",
            "name":"refresh_chart",
            "arguments":{},
            "approval":"review",
            "approval_granted":true
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(approved["result"].get("structuredContent").is_some());

    let oversized_result = client
        .post(format!("{base}/mcp/apps/tools/call"))
        .bearer_auth("secret")
        .json(&json!({
            "server_id":"apps-a",
            "name":"large_result",
            "arguments":{},
            "approval":"guarded"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized_result.status(), reqwest::StatusCode::BAD_REQUEST);

    for body in [
        json!({"server_id":"other-server","name":"refresh_chart","arguments":{},"approval":"open"}),
        json!({"server_id":"apps-a","name":"model_only","arguments":{},"approval":"open"}),
    ] {
        let response = client
            .post(format!("{base}/mcp/apps/tools/call"))
            .bearer_auth("secret")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn memory_ingest_and_search() {
    use milim_memory::MemoryStore;
    use milim_storage::Database;

    let mem = MemoryStore::new(
        Database::open_in_memory().unwrap(),
        Arc::new(TestBackend::new()),
    )
    .unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_memory(mem);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    for t in ["the sky is blue", "rust is fast", "cats nap often"] {
        let r = client
            .post(format!("{base}/memory/ingest"))
            .json(&json!({ "text": t }))
            .send()
            .await
            .unwrap();
        assert!(r.status().is_success());
    }

    let v: Value = client
        .post(format!("{base}/memory/search"))
        .json(&json!({"query":"rust is fast","top_k":2}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // Exact match ranks first (test backend embedding is deterministic).
    assert_eq!(v["hits"][0]["text"], "rust is fast");
    assert_eq!(v["hits"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn memory_graph_register_search_update_and_delete() {
    use milim_memory::MemoryStore;
    use milim_storage::Database;

    let mem = MemoryStore::new(
        Database::open_in_memory().unwrap(),
        Arc::new(TestBackend::new()),
    )
    .unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_memory(mem);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let registered: Value = client
        .post(format!("{base}/memory/register"))
        .json(&json!({
            "model": "test-echo",
            "scope": { "kind": "thread", "label": "Memory test", "locator": "thread-http" },
            "node": {
                "kind": "decision",
                "title": "Use graph memory",
                "body": "Milim stores durable memories as scoped graph nodes.",
                "confidence": 0.95,
                "source": "test"
            },
            "event": { "thread_id": "thread-http", "summary": "Remembered graph memory decision" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(registered["notice"]["scope_kind"], "thread");
    let node_id = registered["node"]["id"].as_str().unwrap().to_string();

    let scopes: Value = client
        .get(format!("{base}/memory/scopes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(scopes["scopes"].as_array().unwrap().len(), 1);

    let nodes: Value = client
        .get(format!(
            "{base}/memory/nodes?scope_kind=thread&scope_locator=thread-http"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes["nodes"][0]["title"], "Use graph memory");

    let hits: Value = client
        .post(format!("{base}/memory/graph/search"))
        .json(&json!({
            "model": "test-echo",
            "query": "graph memory nodes",
            "scopes": [{ "kind": "thread", "locator": "thread-http" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits["hits"][0]["node"]["id"], node_id);
    let score = hits["hits"][0]["score"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&score));
    assert!(hits["hits"][0].get("node").is_some());

    let repeated_hits: Value = client
        .post(format!("{base}/memory/graph/search"))
        .json(&json!({
            "model": "test-echo",
            "query": "graph memory nodes",
            "scopes": [{ "kind": "thread", "locator": "thread-http" }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(hits["hits"], repeated_hits["hits"]);

    let updated: Value = client
        .put(format!("{base}/memory/nodes/{node_id}"))
        .json(&json!({
            "model": "test-echo",
            "title": "Use scoped graph memory",
            "body": "Milim stores durable memories per thread and project."
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["title"], "Use scoped graph memory");

    let deleted: Value = client
        .delete(format!("{base}/memory/nodes/{node_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
}

#[tokio::test]
async fn agent_memory_register_streams_breadcrumb_event() {
    use milim_memory::MemoryStore;
    use milim_storage::Database;

    let mem = MemoryStore::new(
        Database::open_in_memory().unwrap(),
        Arc::new(TestBackend::new()),
    )
    .unwrap();
    let state = AppState::new(Arc::new(MemoryToolBackend), ServerConfiguration::default())
        .with_memory(mem)
        .with_tools(milim_tools::ToolRegistry::new());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let text = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model": "test-memory",
            "messages": [{ "role": "user", "content": "remember the plan" }],
            "stream": true,
            "memory_enabled": true,
            "tool_approval_policy": "open",
            "thread_id": "thread-memory-event",
            "thread_label": "Memory event test"
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        text.contains("\"type\":\"memory_registered\""),
        "missing memory_registered event: {text}"
    );
    assert!(text.contains("Remember memory breadcrumbs"));
    assert!(text.trim_end().ends_with("data: [DONE]"));

    let nodes: Value = client
        .get(format!(
            "{base}/memory/nodes?scope_kind=global&scope_locator=personal"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes["nodes"][0]["title"], "Remember memory breadcrumbs");
}

#[tokio::test]
async fn agent_run_streaming_sse() {
    use milim_tools::ToolRegistry;
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(ToolRegistry::with_builtins());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let text = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"test-echo",
            "messages":[{"role":"user","content":"/tool please"}],
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // Tagged AgentEvent sequence: tool_call -> tool_result -> token/final -> done -> DONE.
    assert!(
        text.contains("\"type\":\"tool_call\""),
        "missing tool_call: {text}"
    );
    assert!(text.contains("\"name\":\"echo\""));
    assert!(text.contains("\"type\":\"tool_result\""));
    assert!(text.contains("\"type\":\"final\"") || text.contains("\"type\":\"token\""));
    assert!(text.contains("\"type\":\"done\""));
    assert!(text.trim_end().ends_with("data: [DONE]"));
}

#[tokio::test]
async fn agent_run_executes_tool_loop() {
    use milim_tools::ToolRegistry;
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(ToolRegistry::with_builtins());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"/tool please"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["object"], "agent.run");
    // The test backend calls `echo` once, the loop runs it, then answers in text.
    assert_eq!(v["iterations"].as_u64().unwrap(), 2);
    assert_eq!(v["steps"][0]["name"], "echo");
    assert_eq!(v["steps"][0]["result"]["echoed"]["text"], "test");
    assert!(v["message"]["content"].as_str().unwrap().contains("Echo:"));
}

#[tokio::test]
async fn thread_supervisor_sweeps_interrupted_threads_on_restart() {
    use milim_agents::{THREAD_STATUS_DONE, THREAD_STATUS_ERROR, THREAD_STATUS_RUNNING};
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let store = milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap();
    let queued = store
        .create("parent-1", "Queued", "test-echo", None, "queued")
        .unwrap();
    let running = store
        .create("parent-1", "Running", "test-echo", None, "running")
        .unwrap();
    store
        .update_status(&running.id, THREAD_STATUS_RUNNING, None, None)
        .unwrap();
    let done = store
        .create("parent-1", "Done", "test-echo", None, "done")
        .unwrap();
    store
        .update_status(&done.id, THREAD_STATUS_DONE, Some("finished"), None)
        .unwrap();

    let supervisor = ThreadSupervisor::new(store);
    let store = supervisor.store();

    for id in [&queued.id, &running.id] {
        let thread = store.get(id).unwrap().unwrap();
        assert_eq!(thread.status, THREAD_STATUS_ERROR);
        assert_eq!(thread.error.as_deref(), Some("interrupted by restart"));
        assert!(thread.finished_at.is_some());
    }
    assert_eq!(
        store.get(&done.id).unwrap().unwrap().status,
        THREAD_STATUS_DONE
    );
}

#[tokio::test]
async fn graceful_shutdown_marks_running_threads_stopped() {
    use milim_agents::{THREAD_STATUS_RUNNING, THREAD_STATUS_STOPPED};
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let thread = store
        .create("parent-1", "Running", "test-echo", None, "running")
        .unwrap();
    store
        .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)
        .unwrap();
    let state = test_state().with_threads(supervisor.clone());

    milim_server::with_graceful_shutdown(state, async {}).await;

    let stopped = store.get(&thread.id).unwrap().unwrap();
    assert_eq!(stopped.status, THREAD_STATUS_STOPPED);
    assert_eq!(stopped.error.as_deref(), Some("stopped by server shutdown"));
    assert!(stopped.finished_at.is_some());
}

#[tokio::test]
async fn thread_supervisor_runs_child_with_test_backend() {
    use milim_server::threads::{ChildRunSpec, ThreadSupervisor};
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let thread = supervisor
        .spawn(
            Arc::new(TestBackend::new()),
            milim_tools::ToolRegistry::with_builtins(),
            ChildRunSpec {
                parent_id: "parent-1".to_string(),
                title: "Child".to_string(),
                model: "test-echo".to_string(),
                agent_id: None,
                system_prompt: None,
                prompt: "hello child".to_string(),
                run_id: None,
                runtime: milim_agents::WorkerRuntime::Legacy,
                access: milim_agents::WorkerAccess::ReadOnly,
                worktree_path: None,
            },
        )
        .unwrap();

    let done = supervisor.wait(&thread.id, 5_000).await.unwrap().unwrap();
    assert_eq!(done.status, milim_agents::THREAD_STATUS_DONE);
    assert!(done.summary.unwrap().contains("Echo: hello child"));
    let events = supervisor.events(&thread.id, 20).unwrap();
    assert!(events.iter().any(|event| event.kind == "token"));
}

#[tokio::test]
async fn thread_events_stream_supervisor_updates() {
    use milim_server::threads::{ChildRunSpec, ThreadSupervisor};
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let supervisor_handle = supervisor.clone();
    let state = test_state().with_threads(supervisor);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/threads/parent-1/events"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let mut bytes = resp.bytes_stream();

    supervisor_handle
        .spawn(
            Arc::new(TestBackend::new()),
            milim_tools::ToolRegistry::with_builtins(),
            ChildRunSpec {
                parent_id: "parent-1".to_string(),
                title: "Child".to_string(),
                model: "test-echo".to_string(),
                agent_id: None,
                system_prompt: None,
                prompt: "hello child".to_string(),
                run_id: None,
                runtime: milim_agents::WorkerRuntime::Legacy,
                access: milim_agents::WorkerAccess::ReadOnly,
                worktree_path: None,
            },
        )
        .unwrap();

    let mut text = String::new();
    for _ in 0..20 {
        let chunk = tokio::time::timeout(std::time::Duration::from_millis(500), bytes.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
        if text.contains("\"type\":\"child_thread_done\"") {
            break;
        }
    }

    assert!(text.contains("\"type\":\"child_thread_started\""), "{text}");
    assert!(text.contains("\"type\":\"child_thread_event\""), "{text}");
    assert!(text.contains("\"type\":\"child_thread_done\""), "{text}");
    assert!(text.contains("Echo: hello child"), "{text}");
}

#[tokio::test]
async fn child_thread_routes_list_read_and_stop() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let child = store
        .create("parent-1", "Child", "test-echo", None, "inspect")
        .unwrap();
    for i in 0..250 {
        store
            .append_event(
                &child.id,
                "token",
                serde_json::json!({ "text": format!("chunk-{i}") }),
            )
            .unwrap();
    }
    let state = test_state().with_threads(supervisor);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let children: Value = client
        .get(format!("{base}/threads/parent-1/children"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(children["threads"][0]["id"], child.id);

    let stopped: Value = client
        .post(format!("{base}/threads/{}/stop", child.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        stopped["thread"]["status"],
        milim_agents::THREAD_STATUS_STOPPED
    );

    let read: Value = client
        .get(format!("{base}/threads/{}?include_events=true", child.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(read["thread"]["id"], child.id);
    assert!(read["events"].is_array());
    assert_eq!(read["events"].as_array().unwrap().len(), 250);
    assert_eq!(read["event_count"], 250);
    assert_eq!(read["events_truncated"], false);

    let truncated: Value = client
        .get(format!(
            "{base}/threads/{}?include_events=true&event_limit=20",
            child.id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(truncated["events"].as_array().unwrap().len(), 20);
    assert_eq!(truncated["event_count"], 250);
    assert_eq!(truncated["events_truncated"], true);

    let deleted: Value = client
        .delete(format!("{base}/threads/parent-1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], 1);
    assert!(store
        .list_children("parent-1", None, 20)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn agent_run_can_spawn_waiting_child_thread() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(milim_tools::ToolRegistry::with_builtins())
    .with_threads(supervisor);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let text = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"child-thread-tool",
            "messages":[{"role":"user","content":"delegate this"}],
            "thread_id":"parent-1",
            "delegation_policy":"auto",
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        text.contains("\"type\":\"worker_run_done\""),
        "missing child done event: {text}"
    );
    assert!(text.contains("parent saw child"));
    let children = store
        .list_children("parent-1", Some(milim_agents::THREAD_STATUS_DONE), 10)
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].model, "child-thread-tool");
    assert_eq!(children[0].summary.as_deref(), Some("child report"));
}

#[tokio::test]
async fn agent_run_ask_waits_for_worker_approval() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(milim_tools::ToolRegistry::with_builtins())
    .with_threads(supervisor);
    let base = spawn(state).await;

    let text = reqwest::Client::new()
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"child-thread-tool",
            "messages":[{"role":"user","content":"delegate this"}],
            "thread_id":"parent-1",
            "delegation_policy":"ask",
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(text.contains("\"type\":\"worker_run_proposed\""), "{text}");
    assert!(!text.contains("parent saw child"), "{text}");
}

#[tokio::test]
async fn worker_run_retry_preserves_task_context_and_terminal_stream_state() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let source = store
        .create_worker_run(
            "parent-1",
            Some("turn-1"),
            milim_agents::DelegationPolicy::Ask,
            milim_agents::WorkerRuntime::Managed,
            vec![milim_agents::WorkerPlanTask {
                id: "failed-task".to_string(),
                title: "Failed worker".to_string(),
                prompt: "child task".to_string(),
                role: Some("reviewer".to_string()),
                agent_id: None,
                model: "child-thread-tool".to_string(),
                access: milim_agents::WorkerAccess::ReadOnly,
            }],
            Some("preserved retry context"),
        )
        .unwrap();
    let proposed = store
        .create_worker_run(
            "parent-1",
            Some("turn-1"),
            milim_agents::DelegationPolicy::Ask,
            milim_agents::WorkerRuntime::Managed,
            vec![],
            None,
        )
        .unwrap();
    store
        .update_worker_run_status(
            &source.id,
            milim_agents::WorkerRunStatus::Error,
            Some("worker failed"),
        )
        .unwrap();
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(milim_tools::ToolRegistry::with_builtins())
    .with_threads(supervisor);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("{base}/worker-runs/{}/events", source.id))
        .send()
        .await
        .unwrap();
    let chunk = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        response.bytes_stream().next(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let event = String::from_utf8_lossy(&chunk);
    assert!(event.contains("\"type\":\"worker_run_error\""), "{event}");
    assert!(event.contains("\"status\":\"error\""), "{event}");

    let retried: Value = client
        .post(format!(
            "{base}/worker-runs/{}/tasks/failed-task/retry",
            source.id
        ))
        .json(&json!({"model":"child-thread-tool"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retried["run"]["parent_thread_id"], "parent-1");
    assert_eq!(retried["run"]["parent_turn_id"], "turn-1");
    assert_eq!(retried["run"]["context"], "preserved retry context");
    assert_eq!(retried["run"]["tasks"][0]["prompt"], "child task");
    assert_eq!(retried["run"]["tasks"][0]["model"], "child-thread-tool");
    assert_ne!(retried["run"]["tasks"][0]["id"], "failed-task");
    assert_eq!(retried["workers"].as_array().unwrap().len(), 1);

    let active_delete = client
        .delete(format!("{base}/worker-runs/{}", proposed.id))
        .send()
        .await
        .unwrap();
    assert_eq!(active_delete.status(), reqwest::StatusCode::BAD_REQUEST);

    let deleted: Value = client
        .delete(format!("{base}/worker-runs/{}", source.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
    assert_eq!(
        client
            .get(format!("{base}/worker-runs/{}", source.id))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn agent_run_rejects_unavailable_child_thread_model() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(milim_tools::ToolRegistry::with_builtins())
    .with_threads(supervisor);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let run: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"child-thread-tool",
            "messages":[{"role":"user","content":"delegate this with missing child model"}],
            "thread_id":"parent-1",
            "delegation_policy":"auto"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(run["steps"][0]["result"]["error"]
        .as_str()
        .unwrap()
        .contains("worker model 'missing-child-model' is not available"));
    assert!(store
        .list_children("parent-1", None, 10)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn agent_run_open_mode_child_inherits_parent_tools_without_child_spawn() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;
    use milim_tools::ToolRegistry;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    for name in ["read_file", "list_dir", "write_file", "shell"] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(tools)
    .with_threads(supervisor)
    .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let text = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"child-thread-tool",
            "messages":[{"role":"user","content":"delegate this and list child tools"}],
            "thread_id":"parent-1",
            "delegation_policy":"auto",
            "tool_approval_policy":"open",
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(text.contains("\"type\":\"worker_run_done\""), "{text}");
    let children = store
        .list_children("parent-1", Some(milim_agents::THREAD_STATUS_DONE), 10)
        .unwrap();
    let summary = children[0].summary.as_deref().unwrap();
    assert!(!summary.contains("shell"), "{summary}");
    assert!(!summary.contains("write_file"), "{summary}");
    assert!(!summary.contains("delegate_workers"), "{summary}");
}

#[tokio::test]
async fn agent_run_guarded_mode_child_stays_read_only() {
    use milim_server::threads::ThreadSupervisor;
    use milim_storage::Database;
    use milim_tools::ToolRegistry;

    let supervisor = ThreadSupervisor::new(
        milim_agents::ThreadStore::new(Database::open_in_memory().unwrap()).unwrap(),
    );
    let store = supervisor.store();
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    for name in ["read_file", "list_dir", "write_file", "shell"] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(
        Arc::new(ChildThreadToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(tools)
    .with_threads(supervisor)
    .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let text = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"child-thread-tool",
            "messages":[{"role":"user","content":"delegate this and list child tools"}],
            "thread_id":"parent-1",
            "delegation_policy":"auto",
            "tool_approval_policy":"guarded",
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(text.contains("\"type\":\"worker_run_done\""), "{text}");
    let children = store
        .list_children("parent-1", Some(milim_agents::THREAD_STATUS_DONE), 10)
        .unwrap();
    let summary = children[0].summary.as_deref().unwrap();
    assert!(summary.contains("read_file"), "{summary}");
    assert!(summary.contains("list_dir"), "{summary}");
    assert!(!summary.contains("write_file"), "{summary}");
    assert!(!summary.contains("shell"), "{summary}");
}

#[tokio::test]
async fn agent_run_hides_desktop_host_tools_without_workspace() {
    use milim_tools::ToolRegistry;
    let mut tools = ToolRegistry::with_builtins();
    for name in [
        "run_command",
        "read_file",
        "read_file_anchors",
        "list_dir",
        "write_file",
        "edit_file",
        "patch_file",
        "shell",
    ] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "sandbox_enabled": true,
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    assert!(content.contains("run_command"));
    assert!(content.contains("workspace_notice=true"));
    for unavailable in [
        "read_file",
        "read_file_anchors",
        "list_dir",
        "write_file",
        "edit_file",
        "patch_file",
        "shell",
    ] {
        assert!(
            !content.contains(unavailable),
            "unexpected unavailable tool {unavailable}: {content}"
        );
    }
}

#[tokio::test]
async fn agent_run_hides_hashline_tools_unless_experimental_flag_set() {
    use milim_tools::ToolRegistry;
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    for name in ["read_file_anchors", "patch_file"] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools)
        .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let without_flag: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = without_flag["message"]["content"].as_str().unwrap();
    assert!(
        !content.contains("read_file_anchors"),
        "ungated anchored read exposed: {content}"
    );
    assert!(
        !content.contains("patch_file"),
        "ungated patch exposed: {content}"
    );

    let with_flag: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "experimental_hashline_patch": true,
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = with_flag["message"]["content"].as_str().unwrap();
    assert!(
        content.contains("read_file_anchors"),
        "anchored read hidden despite flag: {content}"
    );
    assert!(
        content.contains("patch_file"),
        "patch tool hidden despite flag: {content}"
    );
}

#[tokio::test]
async fn agent_run_hides_sandbox_tool_when_sandbox_is_off() {
    use milim_tools::ToolRegistry;
    let mut tools = ToolRegistry::with_builtins();
    tools.register(Arc::new(NamedTestTool {
        name: "run_command",
    }));
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    assert!(
        !content.contains("run_command"),
        "unexpected sandbox tool: {content}"
    );
}

#[tokio::test]
async fn agent_run_hides_preview_tools_unless_preview_is_active() {
    use milim_tools::ToolRegistry;
    let mut tools = ToolRegistry::with_builtins();
    for name in [
        "preview_dom_snapshot",
        "preview_click",
        "preview_type_text",
        "preview_key_press",
        "preview_scroll",
    ] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let without_preview: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = without_preview["message"]["content"].as_str().unwrap();
    assert!(
        !content.contains("preview_click"),
        "inactive preview exposed preview tools: {content}"
    );

    let with_preview: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "preview_tools_enabled": true,
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let content = with_preview["message"]["content"].as_str().unwrap();
    assert!(
        content.contains("preview_dom_snapshot"),
        "active preview hid inspection tool: {content}"
    );
    assert!(
        content.contains("preview_click"),
        "active preview hid click tool: {content}"
    );
}

#[tokio::test]
async fn agent_run_review_mode_without_grant_hides_all_tools() {
    use milim_tools::ToolRegistry;
    let mut tools = ToolRegistry::with_builtins();
    tools.register(Arc::new(NamedTestTool { name: "shell" }));
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "review",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    assert!(
        content.starts_with("tools=;"),
        "review mode exposed tools: {content}"
    );
}

#[tokio::test]
async fn agent_run_guarded_mode_hides_host_shell_with_workspace() {
    use milim_tools::ToolRegistry;
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    for name in ["read_file", "write_file", "edit_file", "shell"] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools)
        .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "guarded",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    assert!(content.contains("read_file"));
    assert!(!content.contains("write_file"));
    assert!(!content.contains("edit_file"));
    assert!(
        !content.contains("shell"),
        "guarded mode exposed shell: {content}"
    );
}

#[tokio::test]
async fn agent_run_plan_mode_exposes_only_read_only_workspace_tools() {
    use milim_tools::ToolRegistry;
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    for name in [
        "run_command",
        "read_file",
        "read_file_anchors",
        "list_dir",
        "write_file",
        "edit_file",
        "patch_file",
        "shell",
        "screenshot",
        "mouse_click",
    ] {
        tools.register(Arc::new(NamedTestTool { name }));
    }
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools)
        .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "open",
            "sandbox_enabled": true,
            "computer_use_enabled": true,
            "experimental_hashline_patch": true,
            "plan_mode": true,
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    let tool_csv = content
        .strip_prefix("tools=")
        .and_then(|text| text.split(';').next())
        .unwrap_or_default();
    let names: Vec<&str> = tool_csv
        .split(',')
        .filter(|name| !name.is_empty())
        .collect();
    assert_eq!(names, vec!["list_dir", "read_file", "read_file_anchors"]);
}

#[tokio::test]
async fn agent_run_open_mode_allows_host_shell_with_workspace() {
    use milim_tools::ToolRegistry;
    let workspace = Arc::new(RwLock::new(Some(unique_temp_path("milim-workspace"))));
    let mut tools = ToolRegistry::with_builtins();
    tools.register(Arc::new(NamedTestTool { name: "shell" }));
    let state = AppState::new(Arc::new(ToolListingBackend), ServerConfiguration::default())
        .with_tools(tools)
        .with_workspace(workspace);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let v: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model":"tool-listing",
            "tool_approval_policy": "open",
            "messages":[{"role":"user","content":"what tools are available?"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let content = v["message"]["content"].as_str().unwrap();
    assert!(
        content.contains("shell"),
        "open mode should expose shell: {content}"
    );
}

#[tokio::test]
async fn http_fetch_tool_rejects_private_and_non_http_targets() {
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins());
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let private = client
        .post(format!("{base}/mcp/call"))
        .json(&json!({"name":"http_fetch","arguments":{"url": format!("{base}/health")}}))
        .send()
        .await
        .unwrap();
    assert_eq!(private.status(), reqwest::StatusCode::BAD_REQUEST);

    // Non-http(s) schemes are rejected.
    let bad = client
        .post(format!("{base}/mcp/call"))
        .json(&json!({"name":"http_fetch","arguments":{"url":"file:///etc/passwd"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn schedules_crud_and_fire_due() {
    use milim_automation::{ScheduleAttachment, ScheduleStore};
    use milim_storage::Database;

    let store = ScheduleStore::new(Database::open_in_memory().unwrap()).unwrap();
    // A schedule that is already "due" (next fire after epoch is well before now).
    let mut schedule = store
        .create_with_attachments(
            "hourly",
            "0 0 * * * *",
            None,
            "summarize the attachment",
            vec![ScheduleAttachment {
                id: "att-1".to_string(),
                name: "notes.md".to_string(),
                mime: "text/markdown".to_string(),
                size: 13,
                content: Some("alpha\nbeta".to_string()),
                data_url: None,
                truncated: false,
                source_path: Some("C:\\tmp\\notes.md".to_string()),
            }],
        )
        .unwrap();
    schedule.created_unix = 0;
    schedule.model = "test-model".to_string();
    store.upsert(&schedule).unwrap();
    let backend = MessageCaptureBackend::default();
    let seen_messages = backend.seen();
    let state = AppState::new(Arc::new(backend), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins())
        .with_schedules(store);

    // fire_due runs the due schedule and marks it ran (deterministic).
    let fired = milim_server::fire_due(&state, 10_000).await.unwrap();
    assert_eq!(fired, 1);
    {
        let captured = seen_messages.read().unwrap();
        let prompt = captured[0].last().unwrap();
        assert!(prompt.contains("summarize the attachment"));
        assert!(prompt.contains("[Attached files]"));
        assert!(prompt.contains("name=notes.md"));
        assert!(prompt.contains("path=C:\\tmp\\notes.md"));
        assert!(prompt.contains("alpha\nbeta"));
    }
    assert!(state
        .schedules
        .as_ref()
        .unwrap()
        .due(10_050)
        .unwrap()
        .is_empty());

    // HTTP CRUD.
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let events: Value = client
        .get(format!("{base}/schedules/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events["events"][0]["schedule_name"], "hourly");
    assert_eq!(events["events"][0]["response"], "ok");
    let drained: Value = client
        .get(format!("{base}/schedules/events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(drained["events"].as_array().unwrap().is_empty());

    let created: Value = client
        .post(format!("{base}/schedules"))
        .json(&json!({
            "name":"nightly",
            "cron":"0 0 0 * * *",
            "model":"test-model",
            "prompt":"hi",
            "attachments":[
                {
                    "id":"att-http",
                    "name":"brief.txt",
                    "mime":"text/plain",
                    "size":5,
                    "content":"hello",
                    "sourcePath":"C:\\tmp\\brief.txt"
                },
                {
                    "id":"att-image",
                    "name":"shape.png",
                    "mime":"image/png",
                    "size":75,
                    "dataUrl":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC"
                }
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["attachments"][0]["name"], "brief.txt");
    assert_eq!(
        created["attachments"][1]["dataUrl"],
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC"
    );
    assert_eq!(
        created["attachments"][0]["sourcePath"],
        "C:\\tmp\\brief.txt"
    );

    let list: Value = reqwest::get(format!("{base}/schedules"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list["schedules"].as_array().unwrap().len() >= 2);

    let updated: Value = client
        .put(format!("{base}/schedules/{id}"))
        .json(&json!({
            "name": "weekday digest",
            "cron": "0 30 9 * * Mon-Fri",
            "agent_id": "agent-1",
            "model": "test-model",
            "prompt": "Summarize today's queue",
            "attachments": [{
                "id": "att-update",
                "name": "queue.md",
                "mime": "text/markdown",
                "size": 7,
                "content": "- item"
            }],
            "enabled": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["id"], id);
    assert_eq!(updated["name"], "weekday digest");
    assert_eq!(updated["agent_id"], "agent-1");
    assert_eq!(updated["enabled"], false);
    assert_eq!(updated["attachments"][0]["name"], "queue.md");

    let del = client
        .delete(format!("{base}/schedules/{id}"))
        .send()
        .await
        .unwrap();
    assert!(del.status().is_success());

    // Invalid cron -> 400.
    let bad = client
        .post(format!("{base}/schedules"))
        .json(&json!({"name":"x","cron":"not-a-cron","model":"test-model","prompt":""}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);

    let account_model = client
        .post(format!("{base}/schedules"))
        .json(&json!({
            "name":"interactive-only",
            "cron":"0 0 * * * *",
            "model":"codex:gpt-5",
            "prompt":"hi"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(account_model.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn schedule_mark_failure_prevents_agent_run() {
    use milim_automation::ScheduleStore;
    use milim_storage::Database;

    let db_path = unique_temp_path("milim-schedule-mark-fail.db");
    let store = ScheduleStore::new(Database::open(&db_path).unwrap()).unwrap();
    let mut schedule = store
        .create("hourly", "0 0 * * * *", None, "must not run")
        .unwrap();
    schedule.created_unix = 0;
    schedule.model = "test-model".to_string();
    store.upsert(&schedule).unwrap();
    let blocker = Database::open(&db_path).unwrap();
    blocker
        .conn()
        .execute_batch(
            "CREATE TRIGGER fail_schedule_mark
             BEFORE UPDATE OF last_run ON schedules
             BEGIN
               SELECT RAISE(FAIL, 'mark failed');
             END;",
        )
        .unwrap();

    let backend = MessageCaptureBackend::default();
    let seen_messages = backend.seen();
    let state = AppState::new(Arc::new(backend), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins())
        .with_schedules(store);

    let err = milim_server::fire_due(&state, 10_000)
        .await
        .expect_err("mark failure should stop the run");
    assert!(err.to_string().contains("mark failed"));
    assert!(seen_messages.read().unwrap().is_empty());

    fs::remove_file(&db_path).ok();
    fs::remove_file(db_path.with_extension("db-wal")).ok();
    fs::remove_file(db_path.with_extension("db-shm")).ok();
}

#[tokio::test]
async fn agent_run_can_create_schedule_from_chat_tool() {
    use milim_automation::ScheduleStore;
    use milim_storage::Database;

    let store = ScheduleStore::new(Database::open_in_memory().unwrap()).unwrap();
    let state = AppState::new(
        Arc::new(ScheduleToolBackend),
        ServerConfiguration::default(),
    )
    .with_tools(milim_tools::ToolRegistry::new())
    .with_schedules(store);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let run: Value = client
        .post(format!("{base}/agents/run"))
        .json(&json!({
            "model": "test-schedule",
            "tool_approval_policy": "open",
            "messages": [{ "role": "user", "content": "Create an OSS maintainer automation every 5 minutes." }]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(run["steps"][0]["name"], "schedule_create");
    assert_eq!(
        run["steps"][0]["result"]["schedule"]["name"],
        "OSS Maintainer Orchestrator"
    );
    assert_eq!(
        run["steps"][0]["result"]["schedule"]["cron"],
        "0 */5 * * * *"
    );

    let list: Value = reqwest::get(format!("{base}/schedules"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["schedules"].as_array().unwrap().len(), 1);
    assert_eq!(
        list["schedules"][0]["prompt"],
        "Check the maintainer queue and report actionable changes."
    );
}

#[tokio::test]
async fn skills_create_list_get() {
    use milim_skills::SkillStore;
    use milim_storage::Database;

    let store = SkillStore::new(Database::open_in_memory().unwrap()).unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_skills(store);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("{base}/skills"))
        .json(&json!({"skill_md":"---\nname: Git Helper\ndescription: version control\n---\nUse git carefully."}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Git Helper");
    assert_eq!(created["enabled"], true);

    let list: Value = reqwest::get(format!("{base}/skills"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["skills"].as_array().unwrap().len(), 1);

    let got: Value = reqwest::get(format!("{base}/skills/{id}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(got["instructions"].as_str().unwrap().contains("Use git"));

    let selected: Value = client
        .post(format!("{base}/skills/select"))
        .json(&json!({"query":"git version control", "limit": 3}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(selected["skills"].as_array().unwrap().len(), 1);

    let selected_outside_allowlist: Value = client
        .post(format!("{base}/skills/select"))
        .json(&json!({"query":"git version control", "limit": 3, "ids": ["other"]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(selected_outside_allowlist["skills"]
        .as_array()
        .unwrap()
        .is_empty());

    let updated: Value = client
        .put(format!("{base}/skills/{id}"))
        .json(&json!({
            "name": "Git Helper",
            "description": "version control",
            "instructions": "Use git carefully.",
            "enabled": false,
            "source_kind": "manual"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["enabled"], false);

    let selected_after_disable: Value = client
        .post(format!("{base}/skills/select"))
        .json(&json!({"query":"git version control", "limit": 3}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        selected_after_disable["skills"].as_array().unwrap().len(),
        0
    );

    let deleted: Value = client
        .delete(format!("{base}/skills/{id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
}

#[tokio::test]
async fn named_agent_create_get_run() {
    use milim_agents::AgentStore;
    use milim_storage::Database;

    let store = AgentStore::new(Database::open_in_memory().unwrap()).unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins())
        .with_agents(store);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    // Create.
    let created: Value = client
        .post(format!("{base}/agents"))
        .json(&json!({"name":"Helper","model":"legacy-agent-model","system_prompt":"You help."}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["name"], "Helper");

    // List + get.
    let list: Value = reqwest::get(format!("{base}/agents"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["agents"].as_array().unwrap().len(), 1);
    let got: Value = reqwest::get(format!("{base}/agents/{id}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["model"], "legacy-agent-model");

    // Run the named agent through the tool loop.
    let run: Value = client
        .post(format!("{base}/agents/{id}/run"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"/tool please"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(run["object"], "agent.run");
    assert_eq!(run["model"], "test-echo");
    assert_eq!(run["steps"][0]["name"], "echo");

    // Unknown agent -> 404.
    let r = client
        .post(format!("{base}/agents/nope/run"))
        .json(&json!({"model":"m","messages":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn named_agent_run_does_not_execute_unselected_tools() {
    use milim_agents::AgentStore;
    use milim_storage::Database;

    let store = AgentStore::new(Database::open_in_memory().unwrap()).unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins())
        .with_agents(store);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("{base}/agents"))
        .json(&json!({
            "name":"Restricted",
            "model":"test-echo",
            "enabled_tools":["current_time"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();

    let run: Value = client
        .post(format!("{base}/agents/{id}/run"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"/tool please"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(run["steps"][0]["name"], "echo");
    assert!(run["steps"][0]["result"]["error"]
        .as_str()
        .unwrap()
        .contains("unknown tool: echo"));
    assert!(run["steps"][0]["result"].get("echoed").is_none());
}

#[tokio::test]
async fn named_agent_tool_mode_none_offers_no_tools() {
    use milim_agents::AgentStore;
    use milim_storage::Database;

    let store = AgentStore::new(Database::open_in_memory().unwrap()).unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(milim_tools::ToolRegistry::with_builtins())
        .with_agents(store);
    let base = spawn(state).await;
    let client = reqwest::Client::new();

    let created: Value = client
        .post(format!("{base}/agents"))
        .json(&json!({
            "name":"No Tools",
            "model":"test-echo",
            "tool_mode":"none",
            "enabled_tools":[]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["tool_mode"], "none");

    let run: Value = client
        .post(format!("{base}/agents/{id}/run"))
        .json(&json!({"model":"test-echo","messages":[{"role":"user","content":"/tool please"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(run["steps"].as_array().unwrap().len(), 0);
    assert!(run["message"]["content"]
        .as_str()
        .unwrap()
        .contains("/tool please"));
}

#[tokio::test]
async fn accepts_valid_msk_v1_access_key() {
    use milim_identity::access_key::{mint_access_key, AccessKeyPayload, AccessKeyValidator};
    use milim_identity::crypto::{derive_address, generate_secret};

    let secret = generate_secret();
    let issuer = derive_address(&secret).unwrap();
    let audience = derive_address(&generate_secret()).unwrap();
    let token = mint_access_key(
        &secret,
        &AccessKeyPayload {
            aud: audience.clone(),
            cnt: 1,
            exp: None,
            iat: 1_700_000_000,
            iss: issuer.clone(),
            lbl: None,
            nonce: "n1".into(),
        },
    )
    .unwrap();

    let validator = AccessKeyValidator::new(&audience, "0xmaster").with_issuer(&issuer);
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_loopback_trust(false)
        .with_access_validator(validator);
    let base = spawn(state).await;
    let client = reqwest::Client::new();
    let body = json!({"model":"test-echo","messages":[{"role":"user","content":"hi"}]});

    // No token -> 401.
    let r = client
        .post(format!("{base}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A bogus msk-v1 token -> 401.
    let r = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth("msk-v1.bogus.deadbeef")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The real signed token -> 200.
    let r = client
        .post(format!("{base}/v1/chat/completions"))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
}

/// Reconstruct assistant text from an Anthropic SSE transcript.
fn reconstruct_anthropic(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
            if v["delta"]["type"] == "text_delta" {
                if let Some(t) = v["delta"]["text"].as_str() {
                    out.push_str(t);
                }
            }
        }
    }
    out
}

#[tokio::test]
async fn anthropic_messages_non_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/anthropic/v1/messages"))
        .json(&json!({
            "model":"test-echo",
            "max_tokens":100,
            "messages":[{"role":"user","content":"ping"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["type"], "message");
    assert_eq!(v["role"], "assistant");
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][0]["text"], "Echo: ping");
    assert_eq!(v["stop_reason"], "end_turn");
    assert!(v["usage"]["output_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn anthropic_messages_streaming() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let text = client
        .post(format!("{base}/anthropic/v1/messages"))
        .json(&json!({
            "model":"test-echo",
            "max_tokens":100,
            "messages":[{"role":"user","content":"ping"}],
            "stream":true
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    // Typed event sequence.
    assert!(text.contains("event: message_start"));
    assert!(text.contains("event: content_block_start"));
    assert!(text.contains("\"type\":\"text_delta\""));
    assert!(text.contains("event: message_delta"));
    assert!(text.contains("event: message_stop"));
    assert_eq!(reconstruct_anthropic(&text), "Echo: ping");
}

#[tokio::test]
async fn anthropic_tool_use_block() {
    let base = spawn(test_state()).await;
    let client = reqwest::Client::new();
    let v: Value = client
        .post(format!("{base}/anthropic/v1/messages"))
        .json(&json!({
            "model":"test-echo",
            "max_tokens":100,
            "messages":[{"role":"user","content":"/tool please"}],
            "tools":[{"name":"echo","input_schema":{"type":"object"}}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(v["stop_reason"], "tool_use");
    let block = &v["content"][0];
    assert_eq!(block["type"], "tool_use");
    assert_eq!(block["name"], "echo");
    assert_eq!(block["input"]["text"], "test");
}
