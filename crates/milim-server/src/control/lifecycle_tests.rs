//! Run lifecycle: how a finished, stopped, or crashed run releases its
//! thread, approvals, steers, and queue.

use super::*;
use milim_core::config::ServerConfiguration;
use milim_inference::test_backend::TestBackend;
use milim_storage::{ControlApprovalRecord, ControlRunRecord, Database};
use serde_json::json;

use super::turns::RunTaskGuard;
use crate::AppState;

const THREAD: &str = "thread-lifecycle";

fn manager_and_state() -> (Arc<RunManager>, AppState) {
    let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
    let manager = RunManager::new(store, "Fixture desktop").unwrap();
    let state = AppState::new(Arc::new(TestBackend::new()), ServerConfiguration::default())
        .with_control(manager.clone());
    (manager, state)
}

async fn create_thread(manager: &Arc<RunManager>, state: &AppState) {
    let created = manager
        .command(
            state.clone(),
            None,
            ControlCommandV1 {
                command_id: "create-lifecycle".into(),
                kind: ControlCommandKindV1::ThreadCreate,
                thread_id: None,
                expected_revision: None,
                payload: json!({
                    "id": THREAD,
                    "title": "Lifecycle",
                    "settings": { "model": "mock-echo", "privacy": "off", "toolApproval": "review" }
                }),
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.status, ControlCommandStatusV1::Applied);
}

fn command(command_id: &str, kind: ControlCommandKindV1, payload: Value) -> ControlCommandV1 {
    ControlCommandV1 {
        command_id: command_id.into(),
        kind,
        thread_id: Some(THREAD.into()),
        expected_revision: None,
        payload,
        confirmation_token: None,
    }
}

fn send(command_id: &str, text: &str) -> ControlCommandV1 {
    command(
        command_id,
        ControlCommandKindV1::TurnSend,
        json!({ "text": text, "attachments": [] }),
    )
}

/// A mock turn that keeps streaming until it is stopped.
fn long_text() -> String {
    "keep streaming ".repeat(200)
}

fn pending_approval(id: &str, run_id: &str) -> ControlApprovalRecord {
    ControlApprovalRecord {
        id: id.into(),
        run_id: run_id.into(),
        thread_id: THREAD.into(),
        kind: "command".into(),
        request_json: json!({ "name": "shell", "arguments": "{\"command\":\"ls\"}" }).to_string(),
        status: "pending".into(),
        decision_json: None,
        created_at_ms: now_ms(),
        resolved_at_ms: None,
    }
}

fn is_active(manager: &RunManager, run_id: &str) -> bool {
    manager
        .active
        .lock()
        .unwrap()
        .get(THREAD)
        .is_some_and(|run| run.run_id == run_id)
}

async fn wait_until(mut done: impl FnMut() -> bool) {
    for _ in 0..400 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for the run lifecycle");
}

fn assistant_replies(manager: &RunManager) -> Vec<String> {
    manager
        .store
        .control_messages(THREAD)
        .unwrap()
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(|message| message["role"] == "assistant")
        .filter_map(|message| message["content"].as_str().map(str::to_string))
        .collect()
}

fn timeline(manager: &RunManager) -> Vec<TimelineItemV1> {
    manager
        .timeline_page(THREAD, None, None, true, 500)
        .unwrap()
        .unwrap()
        .items
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopped_run_cancels_its_pending_approvals_and_reports_its_end() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let mut events = manager.subscribe();
    let sent = manager
        .command(state.clone(), None, send("send-long", &long_text()))
        .await
        .unwrap();
    let run_id = sent.run_id.unwrap();
    // The approval a runtime requested before the user stopped the run.
    let requested = state.tool_approvals.request();
    manager
        .store
        .control_put_approval(&pending_approval(&requested.id, &run_id))
        .unwrap();
    assert_eq!(
        manager
            .bootstrap(&state)
            .await
            .unwrap()
            .pending_approvals
            .len(),
        1
    );

    manager
        .command(
            state.clone(),
            None,
            command("stop-long", ControlCommandKindV1::TurnStop, Value::Null),
        )
        .await
        .unwrap();
    wait_until(|| !is_active(&manager, &run_id)).await;

    let durable = manager
        .store
        .control_approval(&requested.id)
        .unwrap()
        .unwrap();
    assert_eq!(durable.status, "cancelled");
    assert!(manager
        .bootstrap(&state)
        .await
        .unwrap()
        .pending_approvals
        .is_empty());
    assert_eq!(
        state.tool_approvals.snapshot(&requested.id).unwrap().state,
        milim_agents::ApprovalState::Failed
    );
    let items = timeline(&manager);
    let resolved = items
        .iter()
        .position(|item| {
            item.item_type == "approval_resolved" && item.data["approval_id"] == requested.id
        })
        .expect("the cancelled approval is resolved in the timeline");
    assert_eq!(items[resolved].data["status"], "cancelled");
    assert_eq!(items[resolved].data["reason"], "run_ended");
    let status = items
        .iter()
        .position(|item| item.item_type == "run_status")
        .unwrap();
    assert_eq!(items[status].data["status"], "cancelled");
    assert!(resolved < status, "the run's final item is its status");

    let late = manager
        .command(
            state.clone(),
            None,
            ControlCommandV1 {
                command_id: "approve-late".into(),
                kind: ControlCommandKindV1::ApprovalResolve,
                thread_id: None,
                expected_revision: None,
                payload: json!({ "approval_id": requested.id, "decision": "approve" }),
                confirmation_token: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(late.status, ControlCommandStatusV1::Failed);

    let mut finished = None;
    loop {
        match events.try_recv() {
            Ok(event) if event.event_type == "run.updated" && event.data["status"] != "running" => {
                finished = Some(event);
            }
            Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(_) => break,
        }
    }
    let finished = finished.expect("clients hear that the run ended");
    assert_eq!(finished.data["run_id"], run_id);
    assert_eq!(finished.data["status"], "cancelled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_completion_waits_for_an_in_flight_send_and_drains_what_it_queued() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let first = manager
        .command(state.clone(), None, send("send-first", "first"))
        .await
        .unwrap();
    let first_run = first.run_id.unwrap();

    // Hold the thread lock the way a send does between its busy check and
    // its enqueue, while the run reaches completion.
    let lease = manager.lock_for_thread(THREAD);
    let held = lease.lock().await;
    wait_until(|| assistant_replies(&manager).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        is_active(&manager, &first_run),
        "completion must not release the thread under an in-flight send"
    );
    assert_eq!(
        manager
            .store
            .control_run(&first_run)
            .unwrap()
            .unwrap()
            .status,
        "running"
    );
    let queued = manager
        .accept_turn(state.clone(), &send("send-second", "second"))
        .await
        .unwrap();
    assert_eq!(queued.status, ControlCommandStatusV1::Queued);
    drop(held);
    drop(lease);

    wait_until(|| assistant_replies(&manager).len() == 2).await;
    assert_eq!(assistant_replies(&manager)[1], "Echo: second");
    wait_until(|| manager.active.lock().unwrap().is_empty()).await;
    assert!(manager
        .store
        .control_queued_turns(Some(THREAD))
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steer_accepted_during_completion_becomes_the_next_turn() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let first = manager
        .command(state.clone(), None, send("send-first", "first"))
        .await
        .unwrap();
    let first_run = first.run_id.unwrap();
    // The mock runtime cannot claim steers, like a run that ends before its
    // next step would.
    manager
        .active
        .lock()
        .unwrap()
        .get_mut(THREAD)
        .unwrap()
        .steering = true;
    let steer = command(
        "steer-late",
        ControlCommandKindV1::TurnSteer,
        json!({ "run_id": first_run, "text": "late steer", "attachments": [] }),
    );

    let lease = manager.lock_for_thread(THREAD);
    let held = lease.lock().await;
    wait_until(|| assistant_replies(&manager).len() == 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let steered = manager.steer_turn(&state, &steer).unwrap();
    assert_eq!(steered.status, ControlCommandStatusV1::Accepted);
    drop(held);
    drop(lease);

    wait_until(|| assistant_replies(&manager).len() == 2).await;
    assert_eq!(assistant_replies(&manager)[1], "Echo: late steer");
    wait_until(|| manager.active.lock().unwrap().is_empty()).await;
    assert!(manager
        .store
        .control_pending_inbox(Some(THREAD))
        .unwrap()
        .is_empty());
    let error = manager.steer_turn(&state, &steer).unwrap_err();
    assert!(error.to_string().contains("no active turn"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicking_run_task_fails_the_run_and_releases_its_thread() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let now = now_ms();
    manager
        .store
        .control_put_run(&ControlRunRecord {
            id: "run-panic".into(),
            thread_id: THREAD.into(),
            status: "running".into(),
            adapter: "mock".into(),
            request_json: json!({ "text": "crash" }).to_string(),
            agent_snapshot_json: None,
            native_session_json: None,
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            error_json: None,
        })
        .unwrap();
    manager.active.lock().unwrap().insert(
        THREAD.into(),
        ActiveRun {
            run_id: "run-panic".into(),
            steering: false,
            stop: watch::channel(false).0,
        },
    );
    manager
        .store
        .control_put_approval(&pending_approval("approval-panic", "run-panic"))
        .unwrap();
    let queued = manager
        .command(state.clone(), None, send("send-after", "after the crash"))
        .await
        .unwrap();
    assert_eq!(queued.status, ControlCommandStatusV1::Queued);

    let guard = RunTaskGuard::new(manager.clone(), state.clone(), THREAD, "run-panic");
    let task = tokio::spawn(async move {
        let _guard = guard;
        panic!("run task bug");
    });
    assert!(task.await.unwrap_err().is_panic());

    wait_until(|| assistant_replies(&manager).len() == 1).await;
    assert_eq!(assistant_replies(&manager)[0], "Echo: after the crash");
    let run = manager.store.control_run("run-panic").unwrap().unwrap();
    assert_eq!(run.status, "failed");
    assert!(run.error_json.unwrap().contains("run_task_ended"));
    assert_eq!(
        manager
            .store
            .control_approval("approval-panic")
            .unwrap()
            .unwrap()
            .status,
        "cancelled"
    );
    assert!(timeline(&manager).iter().any(|item| {
        item.item_type == "run_status"
            && item.run_id.as_deref() == Some("run-panic")
            && item.data["status"] == "failed"
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_turn_runs_with_the_threads_settings_when_it_starts() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let first = manager
        .command(state.clone(), None, send("send-long", &long_text()))
        .await
        .unwrap();
    let queued = manager
        .command(state.clone(), None, send("send-queued", "queued"))
        .await
        .unwrap();
    assert_eq!(queued.status, ControlCommandStatusV1::Queued);
    let patched = manager
        .command(
            state.clone(),
            None,
            command(
                "tighten",
                ControlCommandKindV1::ThreadSetExecutionSettings,
                json!({ "tool_approval": "guarded", "plan_mode": true }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(patched.status, ControlCommandStatusV1::Applied);
    manager
        .command(
            state.clone(),
            None,
            command("stop-long", ControlCommandKindV1::TurnStop, Value::Null),
        )
        .await
        .unwrap();
    let first_run = first.run_id.unwrap();
    wait_until(|| !is_active(&manager, &first_run)).await;

    let resumed = manager
        .command(
            state.clone(),
            None,
            command(
                "resume-queued",
                ControlCommandKindV1::TurnQueueResume,
                json!({ "queue_id": queued.queue_id }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status, ControlCommandStatusV1::Accepted);
    let run = manager
        .store
        .control_run(resumed.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let accepted: AcceptedTurnV1 = serde_json::from_str(&run.request_json).unwrap();
    assert_eq!(accepted.text, "queued");
    assert_eq!(accepted.config.approval_mode, "guarded");
    assert!(accepted.config.plan_mode);
}

#[tokio::test]
async fn next_runtime_turn_waits_for_the_previous_runtime_to_exit() {
    let (manager, _) = manager_and_state();
    let release = Arc::new(tokio::sync::Notify::new());
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let drain = tokio::spawn({
        let (release, exited) = (release.clone(), exited.clone());
        async move {
            release.notified().await;
            exited.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    });
    manager
        .harness_drains
        .lock()
        .unwrap()
        .insert(THREAD.into(), drain);
    let (_stop_tx, mut stop) = watch::channel(false);
    let waiting = manager.wait_for_harness_cleanup(THREAD, &mut stop);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut waiting)
            .await
            .is_err(),
        "the next turn must not resume the session while its runtime is exiting"
    );
    release.notify_one();
    assert!(waiting.await);
    assert!(exited.load(std::sync::atomic::Ordering::SeqCst));
    assert!(manager.harness_drains.lock().unwrap().is_empty());

    let stuck = tokio::spawn(std::future::pending::<()>());
    manager
        .harness_drains
        .lock()
        .unwrap()
        .insert(THREAD.into(), stuck);
    let (stop_tx, mut stop) = watch::channel(false);
    stop_tx.send(true).unwrap();
    assert!(
        !manager.wait_for_harness_cleanup(THREAD, &mut stop).await,
        "Stop still works while waiting"
    );
    assert!(
        manager
            .wait_for_harness_cleanup("thread-without-runtime", &mut stop)
            .await
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn regenerate_replaces_only_the_last_reply_and_keeps_a_compaction_checkpoint() {
    let (manager, state) = manager_and_state();
    create_thread(&manager, &state).await;
    let sent = manager
        .command(state.clone(), None, send("send-first", "first question"))
        .await
        .unwrap();
    let first_run = sent.run_id.unwrap();
    wait_until(|| !is_active(&manager, &first_run) && assistant_replies(&manager).len() == 1).await;
    let checkpoint = json!({
        "id": "checkpoint-1",
        "role": "assistant",
        "content": "### Context checkpoint\n\nThe user asked a first question.",
        "compaction": { "kind": "checkpoint", "createdAt": 5 },
    });
    manager
        .store
        .control_append_message(THREAD, &checkpoint.to_string())
        .unwrap();

    let regenerated = manager
        .command(
            state.clone(),
            None,
            command(
                "regenerate-first",
                ControlCommandKindV1::TurnRegenerate,
                json!({}),
            ),
        )
        .await
        .unwrap();
    let second_run = regenerated.run_id.unwrap();
    wait_until(|| !is_active(&manager, &second_run)).await;

    let messages = manager
        .store
        .control_messages(THREAD)
        .unwrap()
        .iter()
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == "checkpoint-1"),
        "the checkpoint row survives regeneration"
    );
    let replies = messages
        .iter()
        .filter(|message| message["role"] == "assistant" && message["compaction"].is_null())
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 1, "the old reply was replaced: {messages:?}");
    assert_eq!(replies[0]["runId"], second_run);
}
