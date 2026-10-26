//! `ask_user` and `exit_plan_mode` end to end: a provider run hands the user
//! a decision, and the next `turn.send` answers, supersedes, or conflicts.

use super::*;
use async_trait::async_trait;
use milim_core::api::openai::{DeltaFunction, DeltaToolCall, Model, Usage};
use milim_core::config::ServerConfiguration;
use milim_inference::{CompletionRequest, DeltaEvent, EventStream, ModelService, StreamEvent};
use milim_storage::Database;
use serde_json::json;

use crate::AppState;

const THREAD: &str = "thread-decisions";

/// Calls `ask_user` when asked to "ask me", `exit_plan_mode` when asked to
/// "plan", and otherwise echoes. Records the tools each request offered.
#[derive(Default)]
struct DecisionBackend {
    offered: Mutex<Vec<Vec<String>>>,
}

impl DecisionBackend {
    fn offered(&self) -> Vec<Vec<String>> {
        self.offered.lock().unwrap().clone()
    }
}

#[async_trait]
impl ModelService for DecisionBackend {
    fn name(&self) -> &str {
        "decisions"
    }

    async fn list_models(&self) -> Result<Vec<Model>> {
        Ok(vec![Model::local("test-echo", 0)])
    }

    async fn stream(&self, req: CompletionRequest) -> Result<EventStream> {
        let tools = req
            .tools
            .iter()
            .map(|tool| tool.function.name.clone())
            .collect::<Vec<_>>();
        self.offered.lock().unwrap().push(tools.clone());
        let last_user = req.last_user_text().to_lowercase();
        let call = if last_user.contains("ask me") && tools.iter().any(|tool| tool == "ask_user") {
            Some((
                "ask_user",
                json!({ "questions": [{
                    "question": "Which store should I use?",
                    "header": "Store",
                    "options": [
                        { "label": "SQLite", "description": "Embedded file" },
                        { "label": "Postgres", "description": "Server database" }
                    ]
                }] }),
            ))
        } else if last_user.contains("plan") && tools.iter().any(|tool| tool == "exit_plan_mode") {
            Some((
                "exit_plan_mode",
                json!({ "plan": "1. Edit `a.rs`\n2. Run tests" }),
            ))
        } else {
            None
        };
        let events = match call {
            Some((name, arguments)) => vec![
                Ok(StreamEvent::Delta(DeltaEvent::text("One moment."))),
                Ok(StreamEvent::Delta(DeltaEvent {
                    tool_calls: vec![DeltaToolCall {
                        index: 0,
                        id: Some("call-decision".into()),
                        kind: Some("function".into()),
                        function: DeltaFunction {
                            name: Some(name.into()),
                            arguments: Some(arguments.to_string()),
                        },
                    }],
                    ..Default::default()
                })),
                Ok(StreamEvent::Done {
                    finish_reason: "tool_calls".into(),
                    usage: Usage::new(10, 5),
                }),
            ],
            None => vec![
                Ok(StreamEvent::Delta(DeltaEvent::text(format!(
                    "Echo: {last_user}"
                )))),
                Ok(StreamEvent::Done {
                    finish_reason: "stop".into(),
                    usage: Usage::new(10, 5),
                }),
            ],
        };
        Ok(Box::pin(futures::stream::iter(events)))
    }

    async fn embed(&self, _model: &str, _inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }
}

fn fixture(plan_mode: bool) -> (Arc<RunManager>, AppState, Arc<DecisionBackend>) {
    let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
    let manager = RunManager::new(store, "Fixture desktop").unwrap();
    let backend = Arc::new(DecisionBackend::default());
    // A run with no tools is plain chat; give it one ordinary tool.
    let mut tools = milim_tools::ToolRegistry::new();
    tools.register(Arc::new(milim_tools::TodoWriteTool::default()));
    let state = AppState::new(backend.clone(), ServerConfiguration::default())
        .with_tools(tools)
        .with_control(manager.clone());
    manager
        .create_thread(&ControlCommandV1 {
            command_id: "create-decisions".into(),
            kind: ControlCommandKindV1::ThreadCreate,
            thread_id: None,
            expected_revision: None,
            payload: json!({
                "id": THREAD,
                "title": "Decisions",
                "settings": {
                    "model": "test-echo",
                    "privacy": "off",
                    "toolApproval": "review",
                    "memory": false,
                    "planMode": plan_mode,
                }
            }),
            confirmation_token: None,
        })
        .unwrap();
    (manager, state, backend)
}

fn send(command_id: &str, payload: Value) -> ControlCommandV1 {
    ControlCommandV1 {
        command_id: command_id.into(),
        kind: ControlCommandKindV1::TurnSend,
        thread_id: Some(THREAD.into()),
        expected_revision: None,
        payload,
        confirmation_token: None,
    }
}

async fn run_to_idle(
    manager: &Arc<RunManager>,
    state: &AppState,
    command: ControlCommandV1,
) -> ControlCommandResultV1 {
    let result = manager
        .command(state.clone(), Some("phone".into()), command)
        .await
        .unwrap();
    for _ in 0..400 {
        if manager.store.control_runs(true).unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(manager.store.control_runs(true).unwrap().is_empty());
    result
}

fn items(manager: &RunManager, item_type: &str) -> Vec<TimelineItemV1> {
    manager
        .timeline_page(THREAD, None, None, true, 500)
        .unwrap()
        .unwrap()
        .items
        .into_iter()
        .filter(|item| item.item_type == item_type)
        .collect()
}

fn messages(manager: &RunManager, role: &str) -> Vec<Value> {
    items(manager, "message")
        .into_iter()
        .map(|item| item.data)
        .filter(|message| message["role"] == role)
        .collect()
}

fn answer(decision_id: &str, response: &str) -> Value {
    json!({ "decision_id": decision_id, "response": response })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ask_user_ends_the_run_and_one_answer_wins() {
    let (manager, state, backend) = fixture(false);
    let asked = run_to_idle(
        &manager,
        &state,
        send("ask", json!({ "text": "ask me first" })),
    )
    .await;
    assert_eq!(asked.status, ControlCommandStatusV1::Accepted);
    assert_eq!(backend.offered().len(), 1, "the run ends on the decision");
    assert!(backend.offered()[0].contains(&"ask_user".to_string()));
    assert!(!backend.offered()[0].contains(&"exit_plan_mode".to_string()));

    let requested = items(&manager, USER_DECISION_REQUESTED_ITEM);
    assert_eq!(requested.len(), 1);
    assert_eq!(requested[0].run_id, asked.run_id);
    let decision: UserDecisionV1 =
        serde_json::from_value(requested[0].data["decision"].clone()).unwrap();
    assert_eq!(decision.kind, UserDecisionKindV1::AskUser);
    let assistant = messages(&manager, "assistant").pop().unwrap();
    let content = assistant["content"].as_str().unwrap();
    assert!(content.starts_with("One moment."), "{content}");
    assert!(content.ends_with(&decision.fallback_text), "{content}");
    let run = manager
        .store
        .control_run(asked.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(run.status, "completed");

    // A response of the wrong kind is refused without closing the decision.
    let wrong = run_to_idle(
        &manager,
        &state,
        send(
            "wrong-kind",
            json!({ "text": "ok", "answers_decision": answer(&decision.id, "approve_plan") }),
        ),
    )
    .await;
    assert_eq!(wrong.status, ControlCommandStatusV1::Failed);
    assert!(items(&manager, USER_DECISION_RESOLVED_ITEM).is_empty());

    let reply = send(
        "answer-1",
        json!({
            "text": "Answers:\n- Store: SQLite",
            "answers_decision": answer(&decision.id, "answer"),
        }),
    );
    let answered = run_to_idle(&manager, &state, reply.clone()).await;
    assert_eq!(answered.status, ControlCommandStatusV1::Accepted);
    let retried = run_to_idle(&manager, &state, reply).await;
    assert_eq!(
        retried.run_id, answered.run_id,
        "a retry replays its receipt"
    );
    let second = run_to_idle(
        &manager,
        &state,
        send(
            "answer-2",
            json!({ "text": "Postgres", "answers_decision": answer(&decision.id, "answer") }),
        ),
    )
    .await;
    assert_eq!(second.status, ControlCommandStatusV1::Conflict);

    let resolved = items(&manager, USER_DECISION_RESOLVED_ITEM);
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0].run_id, asked.run_id,
        "it closes the deciding run"
    );
    let resolution: UserDecisionResolvedV1 =
        serde_json::from_value(resolved[0].data.clone()).unwrap();
    assert_eq!(resolution.status, UserDecisionStatusV1::Answered);
    assert_eq!(resolution.response, Some(UserDecisionResponseV1::Answer));
    assert_eq!(resolution.command_id.as_deref(), Some("answer-1"));
    let user = messages(&manager, "user");
    assert_eq!(user.len(), 2);
    assert_eq!(
        user[1]["answersDecision"]["decision_id"],
        decision.id.as_str()
    );
    assert!(user[0].get("answersDecision").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plain_message_supersedes_and_unattended_turns_get_no_decision_tools() {
    let (manager, state, backend) = fixture(false);
    run_to_idle(&manager, &state, send("ask", json!({ "text": "ask me" }))).await;
    let decision_id = items(&manager, USER_DECISION_REQUESTED_ITEM)[0].data["decision"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    run_to_idle(
        &manager,
        &state,
        send("plain", json!({ "text": "never mind" })),
    )
    .await;
    let resolved = items(&manager, USER_DECISION_RESOLVED_ITEM);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].data["status"], "superseded");
    assert_eq!(resolved[0].data["decision_id"], decision_id.as_str());
    let late = run_to_idle(
        &manager,
        &state,
        send(
            "late",
            json!({ "text": "SQLite", "answers_decision": answer(&decision_id, "answer") }),
        ),
    )
    .await;
    assert_eq!(late.status, ControlCommandStatusV1::Conflict);

    run_to_idle(
        &manager,
        &state,
        send(
            "scheduled",
            json!({ "text": "ask me again", "unattended": true }),
        ),
    )
    .await;
    let offered = backend.offered();
    assert!(!offered.last().unwrap().contains(&"ask_user".to_string()));
    assert_eq!(items(&manager, USER_DECISION_REQUESTED_ITEM).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approving_a_plan_turns_plan_mode_off_for_the_answering_turn() {
    let (manager, state, backend) = fixture(true);
    let planned = run_to_idle(&manager, &state, send("plan", json!({ "text": "plan it" }))).await;
    assert_eq!(planned.status, ControlCommandStatusV1::Accepted);
    assert!(backend.offered()[0].contains(&"exit_plan_mode".to_string()));
    let requested = items(&manager, USER_DECISION_REQUESTED_ITEM);
    let decision: UserDecisionV1 =
        serde_json::from_value(requested[0].data["decision"].clone()).unwrap();
    assert_eq!(
        decision.plan.as_deref(),
        Some("1. Edit `a.rs`\n2. Run tests")
    );

    let approved = run_to_idle(
        &manager,
        &state,
        send(
            "approve",
            json!({
                "text": "Plan approved. Implement it.",
                "answers_decision": answer(&decision.id, "approve_plan"),
            }),
        ),
    )
    .await;
    assert_eq!(approved.status, ControlCommandStatusV1::Accepted);
    let run = manager
        .store
        .control_run(approved.run_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let accepted: AcceptedTurnV1 = serde_json::from_str(&run.request_json).unwrap();
    assert!(!accepted.config.plan_mode);
    let thread = manager.store.control_thread(THREAD).unwrap().unwrap();
    let settings: Value = serde_json::from_str(&thread.session_json).unwrap();
    assert_eq!(settings["settings"]["planMode"], false);
    assert!(!backend
        .offered()
        .last()
        .unwrap()
        .contains(&"exit_plan_mode".to_string()));
}
