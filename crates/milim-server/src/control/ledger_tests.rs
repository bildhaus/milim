//! Run ledger and cross-turn context: exact step rebuilds, precise
//! credential scrubbing, work logs, interrupted runs, and history replay.

use std::sync::Arc;
use std::time::Duration;

use milim_agents::AgentStepHook as _;
use milim_control_contract::{
    ControlAttachmentV1, ControlCommandKindV1, ControlCommandStatusV1, ControlCommandV1,
};
use milim_core::api::openai::{ChatMessage, Content, ContentPart, ToolCall, Usage};
use milim_core::config::ServerConfiguration;
use milim_inference::test_backend::TestBackend;
use milim_inference::{CompletionRequest, SamplingParams};
use milim_storage::{ControlInboxRecord, ControlRunRecord, Database, UserDataStore};
use serde_json::{json, Value};

use super::super::delta::{
    render_work_log, DeltaBuffer, WorkLog, STOPPED_BY_USER_MARKER, WORK_LOG_EARLIER_CHARS,
    WORK_LOG_LATEST_CHARS,
};
use super::super::provider::control_chat_messages;
use super::super::run_config::resolve_frozen_config;
use super::super::{AcceptedTurnV1, RunManager};
use super::{scrub_credential_text, RunJournal};
use crate::privacy::{PrivacyGate, PrivacyMode};
use crate::AppState;

/// A real-looking OpenAI project key; nothing may persist it.
// A fake project key, split so secret scanners do not flag the fixture.
const PROJECT_KEY: &str = concat!("sk-", "proj-Q7pX2mV9kL4sT8wR1yN6bC3dF5gH0jZaW2eR");

fn journal(store: &Arc<UserDataStore>) -> RunJournal {
    RunJournal::new(
        store.clone(),
        Arc::new(PrivacyGate::default()),
        PrivacyMode::Off,
        "thread-1",
        "run-1",
    )
}

fn journal_store() -> Arc<UserDataStore> {
    let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
    store
        .control_create_thread(
            "thread-1",
            r#"{"id":"thread-1","title":"Fixture"}"#,
            "epoch-1",
        )
        .unwrap();
    put_run(&store, "thread-1", "run-1", "running", None);
    store
}

fn put_run(
    store: &UserDataStore,
    thread_id: &str,
    run_id: &str,
    status: &str,
    error: Option<&str>,
) {
    store
        .control_put_run(&ControlRunRecord {
            id: run_id.into(),
            thread_id: thread_id.into(),
            status: status.into(),
            adapter: "provider".into(),
            request_json: r#"{"text":"fixture"}"#.into(),
            agent_snapshot_json: None,
            native_session_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
            completed_at_ms: (status != "running").then_some(2),
            error_json: error.map(|message| json!({ "message": message }).to_string()),
        })
        .unwrap();
}

fn request(messages: Vec<ChatMessage>) -> CompletionRequest {
    CompletionRequest {
        model: "fixture-model".into(),
        messages,
        tools: vec![],
        tool_choice: None,
        response_format: None,
        prompt: None,
        suffix: None,
        sampling: SamplingParams::default(),
        reasoning_effort: None,
    }
}

async fn manager_with_thread(model: &str) -> (Arc<RunManager>, AppState) {
    let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
    let manager = RunManager::new(store, "Fixture desktop").unwrap();
    let mut tools = milim_tools::ToolRegistry::new();
    tools.register(Arc::new(milim_tools::EchoTool));
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_tools(tools)
        .with_control(manager.clone());
    let created = manager
        .command(
            state.clone(),
            None,
            ControlCommandV1 {
                command_id: "create".into(),
                kind: ControlCommandKindV1::ThreadCreate,
                thread_id: None,
                expected_revision: None,
                payload: json!({
                    "id": "thread-fixture",
                    "title": "Fixture",
                    "settings": { "model": model, "privacy": "off", "toolApproval": "review" }
                }),
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.status, ControlCommandStatusV1::Applied);
    (manager, state)
}

fn project(manager: &RunManager, run_id: &str, message: Value) {
    manager
        .persist_and_emit("thread-fixture", Some(run_id), "message", message)
        .unwrap();
}

#[test]
fn credential_scrubbing_replaces_only_the_secret_span() {
    for clean in [
        "Run the task-list, then check disk-usage, flask-app, and the risk-register.",
        r#"-Headers @{ Authorization = "Bearer $deviceKey" }"#,
        r#"curl -H "Authorization: Bearer $TOKEN" "$URL""#,
        "export OPENAI_API_KEY=$OPENAI_API_KEY",
        "Bearer authentication needs a password: required below.",
        "sk-learn-is-a-python-library-for-machine-learning",
    ] {
        assert_eq!(scrub_credential_text(clean), clean);
    }
    assert_eq!(
        scrub_credential_text(&format!("use {PROJECT_KEY} for the task-list")),
        "use [REDACTED_CREDENTIAL] for the task-list"
    );
    assert_eq!(
        scrub_credential_text(&format!("OPENAI_API_KEY={PROJECT_KEY}\nnext")),
        "OPENAI_API_KEY=[REDACTED_CREDENTIAL]\nnext"
    );
    assert_eq!(
        scrub_credential_text("Authorization: Bearer abcdEFGH1234ijkl5678"),
        "Authorization: Bearer [REDACTED_CREDENTIAL]"
    );
    assert_eq!(
        scrub_credential_text("Authorization: Basic dXNlcjpwYXNz"),
        "Authorization: Basic [REDACTED_CREDENTIAL]"
    );
    assert_eq!(
        scrub_credential_text("?client_secret=s3cr3t-value-2024&page=2"),
        "?client_secret=[REDACTED_CREDENTIAL]&page=2"
    );

    // Tool-call arguments are JSON documents inside a string; scrubbing a
    // span keeps them parseable, escaped quotes and newlines included.
    let arguments = json!({
        "command": format!(
            "curl -H 'Authorization: Bearer abcd1234efgh5678ijkl' -d '{{\"api_key\": \"{PROJECT_KEY}\"}}' https://example.com"
        ),
        "content": format!("first line\n{PROJECT_KEY}\nsecond line"),
        "path": "task-list.md",
    })
    .to_string();
    let scrubbed = scrub_credential_text(&arguments);
    let parsed: Value =
        serde_json::from_str(&scrubbed).expect("scrubbed tool arguments stay valid JSON");
    assert!(!scrubbed.contains(PROJECT_KEY));
    assert!(!scrubbed.contains("abcd1234efgh5678ijkl"));
    assert_eq!(parsed["path"], "task-list.md");
    assert_eq!(
        parsed["content"],
        "first line\n[REDACTED_CREDENTIAL]\nsecond line"
    );
}

#[tokio::test]
async fn later_steps_rebuild_exactly_what_was_sent_while_the_ledger_scrubs_spans() {
    let store = journal_store();
    let journal = journal(&store);
    let sent = request(vec![
        ChatMessage::text(
            "system",
            "Pair with -H \"Authorization: Bearer $deviceKey\".",
        ),
        ChatMessage::text(
            "user",
            format!("Finish the task-list. The deploy key is {PROJECT_KEY}."),
        ),
    ]);
    let arguments = json!({
        "command": format!("curl -H 'Authorization: Bearer {PROJECT_KEY}' https://example.com/task-list")
    })
    .to_string();
    let tool_calls: Vec<ToolCall> = serde_json::from_value(json!([{
        "id": "call-1",
        "type": "function",
        "function": {"name": "shell", "arguments": arguments}
    }]))
    .unwrap();
    // Opaque continuation data that merely looks like a key.
    let provider_state =
        json!({"anthropic": [{"signature": "sk-opaque-signature-0123456789abcdef"}]});
    let tool_output = format!("{{\"stdout\":\"deployed with {PROJECT_KEY}\",\"exit_code\":0}}");
    journal.commit_model_request(1, &sent).await.unwrap();
    journal
        .commit_model_response(
            1,
            "Running the task-list.",
            "check disk-usage first",
            &tool_calls,
            "tool_calls",
            Usage::default(),
            Some(&provider_state),
        )
        .await
        .unwrap();
    journal
        .commit_tool_result(
            1,
            Some("call-1"),
            "shell",
            &json!({"stdout": format!("deployed with {PROJECT_KEY}"), "exit_code": 0}),
            &tool_output,
        )
        .await
        .unwrap();

    let mut messages = vec![ChatMessage::text("user", "poisoned memory cache")];
    journal.prepare_model_step(2, &mut messages).await.unwrap();
    let mut expected = sent.messages.clone();
    expected.push(ChatMessage {
        role: "assistant".into(),
        content: Some(Content::Text("Running the task-list.".into())),
        name: None,
        tool_calls: Some(tool_calls.clone()),
        tool_call_id: None,
        reasoning_content: Some("check disk-usage first".into()),
        provider_state: Some(provider_state.clone()),
    });
    expected.push(ChatMessage {
        role: "tool".into(),
        content: Some(Content::Text(tool_output.clone())),
        name: None,
        tool_calls: None,
        tool_call_id: Some("call-1".into()),
        reasoning_content: None,
        provider_state: None,
    });
    assert_eq!(
        serde_json::to_value(&messages).unwrap(),
        serde_json::to_value(&expected).unwrap(),
        "the model sees exactly the bytes it was sent and answered"
    );

    // The ledger scrubs only the key, keeps ordinary text, and never touches
    // provider state.
    let artifacts = store.control_run_artifacts("run-1").unwrap();
    let stored = serde_json::to_string(&artifacts).unwrap();
    assert!(!stored.contains(PROJECT_KEY));
    for survivor in [
        "Finish the task-list",
        "Bearer $deviceKey",
        "check disk-usage first",
        "sk-opaque-signature-0123456789abcdef",
    ] {
        assert!(stored.contains(survivor), "{survivor} must survive storage");
    }
    let response = artifacts
        .iter()
        .find(|artifact| artifact.kind == "provider_response")
        .unwrap();
    let response: Value = serde_json::from_str(&response.data_json).unwrap();
    let stored_arguments = response["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    let stored_arguments: Value =
        serde_json::from_str(stored_arguments).expect("stored tool-call arguments stay valid JSON");
    assert_eq!(
        stored_arguments["command"],
        "curl -H 'Authorization: Bearer [REDACTED_CREDENTIAL]' https://example.com/task-list"
    );

    // A journal without the exact copy (another instance) rebuilds from the
    // ledger: scrubbed spans only, valid tool-call JSON, provider state intact.
    let restarted = RunJournal::new(
        store.clone(),
        Arc::new(PrivacyGate::default()),
        PrivacyMode::Off,
        "thread-1",
        "run-1",
    );
    let mut rebuilt = Vec::new();
    restarted.prepare_model_step(2, &mut rebuilt).await.unwrap();
    assert_eq!(rebuilt.len(), 4);
    assert_eq!(
        rebuilt[0].text_content(),
        "Pair with -H \"Authorization: Bearer $deviceKey\"."
    );
    assert_eq!(
        rebuilt[1].text_content(),
        "Finish the task-list. The deploy key is [REDACTED_CREDENTIAL]."
    );
    let rebuilt_call = &rebuilt[2].tool_calls.as_ref().unwrap()[0];
    serde_json::from_str::<Value>(&rebuilt_call.function.arguments).unwrap();
    assert_eq!(rebuilt[2].provider_state.as_ref(), Some(&provider_state));
    assert_eq!(rebuilt[3].tool_call_id.as_deref(), Some("call-1"));
}

#[tokio::test]
async fn redact_mode_rebuilds_raw_text_for_the_outbound_gate_and_stores_it_redacted() {
    let store = journal_store();
    let journal = RunJournal::new(
        store.clone(),
        Arc::new(PrivacyGate::default()),
        PrivacyMode::Redact,
        "thread-1",
        "run-1",
    );
    let sent = request(vec![ChatMessage::text(
        "user",
        "Email private@example.com the task-list.",
    )]);
    journal.commit_model_request(1, &sent).await.unwrap();
    journal
        .commit_model_response(
            1,
            "I will write to private@example.com.",
            "",
            &[],
            "length",
            Usage::default(),
            None,
        )
        .await
        .unwrap();
    let mut messages = Vec::new();
    journal.prepare_model_step(2, &mut messages).await.unwrap();
    // The outbound privacy gate redacts the whole request with one map on
    // every step, so the rebuilt step keeps the text the loop sent.
    assert_eq!(
        messages[0].text_content(),
        "Email private@example.com the task-list."
    );
    assert_eq!(
        messages[1].text_content(),
        "I will write to private@example.com."
    );
    let stored = serde_json::to_string(&store.control_run_artifacts("run-1").unwrap()).unwrap();
    assert!(!stored.contains("private@example.com"));
    assert!(stored.contains("the task-list"));
}

#[tokio::test]
async fn steering_input_carries_its_image_attachments_to_the_model() {
    let (manager, state) = manager_with_thread("mock-echo").await;
    let thread = manager
        .store
        .control_thread("thread-fixture")
        .unwrap()
        .unwrap();
    let image = "data:image/png;base64,iVBORw0KGgo=";
    let config = resolve_frozen_config(
        &state,
        &manager.store,
        &thread,
        vec![ControlAttachmentV1 {
            id: "image-1".into(),
            name: "screen.png".into(),
            mime: "image/png".into(),
            size: 8,
            content: None,
            data_url: Some(image.into()),
            upload_id: None,
            truncated: false,
        }],
    )
    .unwrap();
    put_run(
        &manager.store,
        "thread-fixture",
        "run-active",
        "running",
        None,
    );
    manager
        .store
        .control_put_inbox(&ControlInboxRecord {
            id: "inbox-1".into(),
            thread_id: "thread-fixture".into(),
            target_run_id: Some("run-active".into()),
            command_id: None,
            kind: "steer".into(),
            state: "pending".into(),
            payload_json: serde_json::to_string(&AcceptedTurnV1 {
                text: "Match this layout".into(),
                client_message_id: None,
                display_text: None,
                config,
                append_user: true,
                mailbox_origin: None,
                mailbox_context: Vec::new(),
                preview_runtime: None,
            })
            .unwrap(),
            created_at_ms: 1,
            claimed_at_ms: None,
            resolved_at_ms: None,
        })
        .unwrap();
    let journal = RunJournal::new(
        manager.store.clone(),
        state.privacy.clone(),
        PrivacyMode::Off,
        "thread-fixture",
        "run-active",
    );
    let mut messages = vec![ChatMessage::text("user", "Build the page")];
    journal.prepare_model_step(1, &mut messages).await.unwrap();

    let steer = messages.last().unwrap();
    assert_eq!(steer.role, "user");
    let Some(Content::Parts(parts)) = steer.content.as_ref() else {
        panic!("steering with an image replays as content parts");
    };
    assert!(matches!(&parts[0], ContentPart::Text { text } if text == "Match this layout"));
    assert!(matches!(&parts[1], ContentPart::ImageUrl { image_url } if image_url.url == image));
}

#[tokio::test]
async fn step_texts_are_separated_and_notices_stay_out_of_model_text() {
    let (manager, _state) = manager_with_thread("mock-echo").await;
    for run_id in ["run-deltas", "run-retry"] {
        put_run(&manager.store, "thread-fixture", run_id, "running", None);
    }
    let mut deltas = DeltaBuffer::new(&manager, "thread-fixture", "run-deltas");
    deltas.push_text("Let me check.");
    deltas.mark_step_boundary();
    deltas.push_text("");
    deltas.push_text("Found it");
    deltas.push_text(" in lib.rs.");
    deltas.mark_step_boundary();
    deltas.push_text("Fixed.");
    deltas.push_notice("\n\nRun limit reached. Send Continue to start another bounded run.");
    deltas.flush().unwrap();
    assert_eq!(
        deltas.content(),
        "Let me check.\n\nFound it in lib.rs.\n\nFixed.\n\nRun limit reached. Send Continue to start another bounded run."
    );
    assert_eq!(
        deltas.model_content(),
        "Let me check.\n\nFound it in lib.rs.\n\nFixed."
    );
    // The streamed deltas carry the separators too, so the renderer's
    // streamed text matches the final message.
    let streamed = manager
        .timeline_page("thread-fixture", None, None, true, 50)
        .unwrap()
        .unwrap()
        .items
        .into_iter()
        .filter(|item| item.item_type == "assistant_delta")
        .map(|item| item.data["text"].as_str().unwrap().to_string())
        .collect::<String>();
    assert_eq!(streamed, deltas.content());

    // A retried attempt's discarded text leaves the separator in place once.
    let mut retried = DeltaBuffer::new(&manager, "thread-fixture", "run-retry");
    retried.push_text("Reading.");
    retried.mark_step_boundary();
    retried.push_text("partial");
    retried.truncate_for_retry("partial".len(), 0);
    retried.push_text("Done.");
    assert_eq!(retried.content(), "Reading.\n\nDone.");
    assert_eq!(retried.model_content(), "Reading.\n\nDone.");
}

#[test]
fn work_log_records_calls_outcomes_and_changed_files() {
    let mut log = WorkLog::default();
    log.record_call(Some("c1"), "read_file", r#"{"path":"src/lib.rs"}"#);
    log.record_call(
        Some("c2"),
        "edit_file",
        r#"{"path":"src/lib.rs","old":"a","new":"b"}"#,
    );
    log.record_result(
        Some("c2"),
        "edit_file",
        &json!({"path": "src/lib.rs", "bytes": 10}),
    );
    log.record_result(Some("c1"), "read_file", &json!({"content": "fn main() {}"}));
    log.record_call(
        Some("c3"),
        "shell",
        r#"{"command":"cargo test --workspace"}"#,
    );
    log.record_result(
        Some("c3"),
        "shell",
        &json!({"stdout": "", "exit_code": 101}),
    );
    log.record_call(
        Some("c4"),
        "write_file",
        r#"{"path":"blocked.rs","content":"x"}"#,
    );
    log.record_result(
        Some("c4"),
        "write_file",
        &json!({"error": "Tool call denied by user", "denied": true}),
    );
    log.record_call(Some("c5"), "http_fetch", r#"{"url":"https://example.com"}"#);
    log.record_result(
        Some("c5"),
        "http_fetch",
        &json!({"error": "connection\nrefused"}),
    );
    log.record_call(Some("c6"), "grep", r#"{"pattern":"fn main","path":"src"}"#);

    let stored = log.to_value().unwrap();
    assert_eq!(
        render_work_log(&stored, WORK_LOG_LATEST_CHARS).unwrap(),
        "<work_log>\n\
         Tool calls from this turn, recorded by milim (not part of the visible reply):\n\
         - read_file path=src/lib.rs -> ok\n\
         - edit_file path=src/lib.rs -> ok\n\
         - shell command=\"cargo test --workspace\" -> exit 101\n\
         - write_file path=blocked.rs -> denied\n\
         - http_fetch url=https://example.com -> error: connection refused\n\
         - grep path=src pattern=\"fn main\" -> not finished\n\
         Files changed: src/lib.rs\n\
         </work_log>"
    );
    assert!(WorkLog::default().to_value().is_none());
}

#[test]
fn long_work_logs_keep_their_first_and_last_calls_within_budget() {
    let mut log = WorkLog::default();
    for index in 0..200 {
        let id = format!("call-{index}");
        log.record_call(
            Some(&id),
            "edit_file",
            &json!({"path": format!("src/module_{index}.rs")}).to_string(),
        );
        log.record_result(Some(&id), "edit_file", &json!({"bytes": 1}));
    }
    let stored = log.to_value().unwrap();
    let entries = stored["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 16 + 1 + 48);
    assert_eq!(entries[16], json!({"omitted": 136}));
    assert_eq!(stored["filesChanged"].as_array().unwrap().len(), 40);

    let latest = render_work_log(&stored, WORK_LOG_LATEST_CHARS).unwrap();
    let earlier = render_work_log(&stored, WORK_LOG_EARLIER_CHARS).unwrap();
    let header =
        "<work_log>\nTool calls from this turn, recorded by milim (not part of the visible reply):\n</work_log>"
            .len();
    assert!(latest.len() <= WORK_LOG_LATEST_CHARS + header);
    assert!(earlier.len() <= WORK_LOG_EARLIER_CHARS + header);
    assert!(earlier.len() < latest.len());
    for rendered in [&latest, &earlier] {
        assert!(rendered.contains("- edit_file path=src/module_0.rs -> ok"));
        assert!(rendered.contains("- edit_file path=src/module_199.rs -> ok"));
        assert!(rendered.contains("more tool calls …"));
        let omitted = rendered
            .lines()
            .filter(|line| line.starts_with("- … "))
            .collect::<Vec<_>>();
        assert_eq!(omitted.len(), 1, "one omission line: {rendered}");
        let calls = rendered
            .lines()
            .filter(|line| line.starts_with("- edit_file"))
            .count();
        let folded: usize = omitted[0]
            .trim_start_matches("- … ")
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(calls + folded, 200);
    }
}

#[tokio::test]
async fn history_replays_work_logs_and_hides_notices_from_the_model() {
    let (manager, _state) = manager_with_thread("mock-echo").await;
    for run_id in ["run-1", "run-2", "run-3"] {
        put_run(&manager.store, "thread-fixture", run_id, "completed", None);
    }
    let mut first_log = WorkLog::default();
    for index in 0..40 {
        let id = format!("first-{index}");
        first_log.record_call(
            Some(&id),
            "read_file",
            &json!({"path": format!("docs/chapter_{index}.md")}).to_string(),
        );
        first_log.record_result(Some(&id), "read_file", &json!({"content": "text"}));
    }
    let mut second_log = WorkLog::default();
    second_log.record_call(Some("edit"), "edit_file", r#"{"path":"src/lib.rs"}"#);
    second_log.record_result(Some("edit"), "edit_file", &json!({"path": "src/lib.rs"}));
    project(
        &manager,
        "run-1",
        json!({"id": "user-1", "role": "user", "content": "Read the docs", "runId": "run-1"}),
    );
    project(
        &manager,
        "run-1",
        json!({
            "id": "assistant-1",
            "role": "assistant",
            "content": "Read them.\n\nRun limit reached. Send Continue to start another bounded run.",
            "promptContent": "Read them.",
            "workLog": first_log.to_value(),
            "interruption": "[interrupted: run limit reached]",
            "runId": "run-1",
        }),
    );
    project(
        &manager,
        "run-2",
        json!({"id": "user-2", "role": "user", "content": "Fix lib.rs", "runId": "run-2"}),
    );
    project(
        &manager,
        "run-2",
        json!({
            "id": "assistant-2",
            "role": "assistant",
            "content": "Fixed.",
            "workLog": second_log.to_value(),
            "runId": "run-2",
        }),
    );
    project(
        &manager,
        "run-3",
        json!({"id": "user-3", "role": "user", "content": "Thanks", "runId": "run-3"}),
    );

    let messages = control_chat_messages(&manager.store, "thread-fixture").unwrap();
    let texts = messages
        .iter()
        .map(|message| (message.role.as_str(), message.text_content()))
        .collect::<Vec<_>>();
    assert_eq!(texts.len(), 5);
    let first = &texts[1].1;
    assert!(first.starts_with("Read them.\n\n<work_log>\n"), "{first}");
    assert!(
        first.ends_with("</work_log>\n\n[interrupted: run limit reached]"),
        "{first}"
    );
    assert!(
        !first.contains("Send Continue"),
        "notices are never replayed"
    );
    assert!(
        first.contains("more tool calls"),
        "an earlier run's log is trimmed to the smaller budget: {first}"
    );
    assert_eq!(
        texts[3].1,
        "Fixed.\n\n<work_log>\n\
         Tool calls from this turn, recorded by milim (not part of the visible reply):\n\
         - edit_file path=src/lib.rs -> ok\n\
         Files changed: src/lib.rs\n\
         </work_log>"
    );
    assert_eq!(texts[4], ("user", "Thanks".to_string()));
}

#[tokio::test]
async fn interrupted_runs_keep_partial_output_and_mark_silent_gaps() {
    let (manager, _state) = manager_with_thread("mock-echo").await;
    for (run_id, status, error) in [
        ("run-stopped", "cancelled", None),
        ("run-silent", "cancelled", None),
        (
            "run-failed",
            "failed",
            Some("upstream 400: context too long"),
        ),
        ("run-current", "running", None),
    ] {
        put_run(&manager.store, "thread-fixture", run_id, status, error);
    }
    project(
        &manager,
        "run-stopped",
        json!({"id": "user-1", "role": "user", "content": "Update the config", "runId": "run-stopped"}),
    );
    // The run edited a file and was stopped mid-answer.
    let mut deltas = DeltaBuffer::new(&manager, "thread-fixture", "run-stopped");
    let mut log = WorkLog::default();
    deltas.push_text("I updated");
    log.record_call(Some("c1"), "edit_file", r#"{"path":"config.toml"}"#);
    log.record_result(Some("c1"), "edit_file", &json!({"path": "config.toml"}));
    deltas.flush().unwrap();
    manager
        .persist_interrupted_output(
            "thread-fixture",
            "run-stopped",
            deltas,
            &log,
            STOPPED_BY_USER_MARKER.into(),
        )
        .unwrap();
    // Runs that produced nothing keep nothing.
    project(
        &manager,
        "run-silent",
        json!({"id": "user-2", "role": "user", "content": "Try again", "runId": "run-silent"}),
    );
    manager
        .persist_interrupted_output(
            "thread-fixture",
            "run-silent",
            DeltaBuffer::new(&manager, "thread-fixture", "run-silent"),
            &WorkLog::default(),
            STOPPED_BY_USER_MARKER.into(),
        )
        .unwrap();
    project(
        &manager,
        "run-failed",
        json!({"id": "user-3", "role": "user", "content": "Once more", "runId": "run-failed"}),
    );
    project(
        &manager,
        "run-failed",
        json!({"id": "steer-1", "role": "user", "content": "use TOML", "runId": "run-failed", "steering": true}),
    );
    project(
        &manager,
        "run-current",
        json!({"id": "user-4", "role": "user", "content": "Status?", "runId": "run-current"}),
    );

    let stored = manager
        .store
        .control_projected_messages("thread-fixture")
        .unwrap()
        .into_iter()
        .map(|raw| serde_json::from_str::<Value>(&raw).unwrap())
        .filter(|message| message["role"] == "assistant")
        .collect::<Vec<_>>();
    assert_eq!(stored.len(), 1, "only the run with output keeps a message");
    assert_eq!(stored[0]["content"], "I updated");
    assert_eq!(stored[0]["interruption"], STOPPED_BY_USER_MARKER);

    let messages = control_chat_messages(&manager.store, "thread-fixture").unwrap();
    let texts = messages
        .iter()
        .map(|message| (message.role.clone(), message.text_content()))
        .collect::<Vec<_>>();
    let roles = texts
        .iter()
        .map(|(role, _)| role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        [
            "user",
            "assistant",
            "user",
            "assistant",
            "user",
            "user",
            "assistant",
            "user"
        ]
    );
    assert!(texts[1].1.starts_with("I updated\n\n<work_log>\n"));
    assert!(texts[1].1.contains("Files changed: config.toml"));
    assert!(texts[1].1.ends_with("\n\n[interrupted: stopped by user]"));
    assert_eq!(texts[3].1, "[interrupted: stopped by user]");
    assert_eq!(texts[5].1, "use TOML");
    assert_eq!(texts[6].1, "[failed: upstream 400: context too long]");
    assert_eq!(texts[7].1, "Status?");
}

#[tokio::test]
async fn history_replay_honors_the_latest_compaction_checkpoint() {
    let (manager, _state) = manager_with_thread("mock-echo").await;
    for run_id in ["run-1", "run-2"] {
        put_run(&manager.store, "thread-fixture", run_id, "completed", None);
    }
    let rows = [
        json!({"id": "user-1", "role": "user", "content": "Plan the migration", "runId": "run-1"}),
        json!({"id": "assistant-1", "role": "assistant", "content": "Here is the plan.", "runId": "run-1"}),
        json!({"id": "user-2", "role": "user", "content": "Start step one", "runId": "run-2"}),
        json!({"id": "assistant-2", "role": "assistant", "content": "Step one is done.", "runId": "run-2"}),
    ];
    for row in &rows {
        project(&manager, row["runId"].as_str().unwrap(), row.clone());
        manager
            .store
            .control_append_message("thread-fixture", &row.to_string())
            .unwrap();
    }
    assert_eq!(
        control_chat_messages(&manager.store, "thread-fixture")
            .unwrap()
            .len(),
        4
    );
    // `/compact` summarized the first turn and kept the second as its tail.
    manager
        .store
        .control_delete_message("thread-fixture", "user-2")
        .unwrap();
    manager
        .store
        .control_delete_message("thread-fixture", "assistant-2")
        .unwrap();
    for row in [
        json!({
            "id": "checkpoint-1",
            "role": "assistant",
            "content": "### Context checkpoint\n\nThe user approved a three-step migration plan.",
            "compaction": {"kind": "checkpoint", "createdAt": 5},
        }),
        rows[2].clone(),
        rows[3].clone(),
    ] {
        manager
            .store
            .control_append_message("thread-fixture", &row.to_string())
            .unwrap();
    }

    let messages = control_chat_messages(&manager.store, "thread-fixture").unwrap();
    let texts = messages
        .iter()
        .map(|message| (message.role.as_str(), message.text_content()))
        .collect::<Vec<_>>();
    assert_eq!(
        texts,
        [
            (
                "system",
                "Previous thread context checkpoint. Treat this as the durable state for earlier messages that remain visible in the UI but are not replayed below.\n\nThe user approved a three-step migration plan.".to_string()
            ),
            ("user", "Start step one".to_string()),
            ("assistant", "Step one is done.".to_string()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_runs_store_a_work_log_the_next_turn_replays() {
    let (manager, state) = manager_with_thread("test-echo").await;
    let sent = manager
        .command(
            state.clone(),
            None,
            ControlCommandV1 {
                command_id: "send-tool".into(),
                kind: ControlCommandKindV1::TurnSend,
                thread_id: Some("thread-fixture".into()),
                expected_revision: None,
                payload: json!({ "text": "please call tool", "attachments": [] }),
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(sent.status, ControlCommandStatusV1::Accepted, "{sent:?}");
    for _ in 0..400 {
        if manager.store.control_runs(true).unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let run = manager
        .store
        .control_run(sent.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "completed", "{:?}", run.error_json);
    let assistant = manager
        .store
        .control_projected_messages("thread-fixture")
        .unwrap()
        .into_iter()
        .map(|raw| serde_json::from_str::<Value>(&raw).unwrap())
        .find(|message| message["role"] == "assistant")
        .unwrap();
    assert_eq!(
        assistant["workLog"]["entries"],
        json!([{"tool": "echo", "arguments": "", "outcome": "ok"}]),
        "{assistant}"
    );
    let replayed = control_chat_messages(&manager.store, "thread-fixture").unwrap();
    assert!(replayed[1]
        .text_content()
        .contains("<work_log>\nTool calls from this turn, recorded by milim (not part of the visible reply):\n- echo -> ok\n</work_log>"));

    // The turn's context message is kept with its reply and, once a later
    // turn exists, replays verbatim before the user message it was sent with.
    let turn_context = assistant["turnContext"].as_str().unwrap().to_string();
    assert!(
        turn_context.starts_with("Context for this turn"),
        "{turn_context}"
    );
    assert_eq!(
        replayed.len(),
        2,
        "the latest turn gets fresh context instead"
    );
    put_run(
        &manager.store,
        "thread-fixture",
        "run-next",
        "running",
        None,
    );
    project(
        &manager,
        "run-next",
        json!({"id": "user-next", "role": "user", "content": "and now?", "runId": "run-next"}),
    );
    let replayed = control_chat_messages(&manager.store, "thread-fixture").unwrap();
    let roles = replayed
        .iter()
        .map(|message| message.role.as_str())
        .collect::<Vec<_>>();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(replayed[0].text_content(), turn_context);
    assert_eq!(replayed[1].text_content(), "please call tool");
}
