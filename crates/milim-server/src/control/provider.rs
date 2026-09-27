//! Direct provider runs (and the mock runtime used by tests and demos), and
//! the model-visible thread history every provider and Agent run starts from.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use milim_agents::AgentStepHook as _;
use milim_core::api::openai::ChatMessage;
use milim_core::{Error, Result};
use milim_inference::{CompletionRequest, StreamEvent};
use milim_storage::UserDataStore;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::delta::{
    failed_run_marker, render_work_log, DeltaBuffer, WorkLog, STOPPED_BY_USER_MARKER,
    WORK_LOG_EARLIER_CHARS, WORK_LOG_LATEST_CHARS,
};
use super::journal::RunJournal;
use super::linked_threads::linked_run_context;
use super::metrics::response_metrics_value;
use super::preview_runtime::managed_preview_runtime_context;
use super::run_config::{parse_reasoning_effort, sampling_from_generation};
use super::{AcceptedTurnV1, RunManager, RunOutcome};
use crate::routes::{service_for_run, RunContext};
use crate::AppState;

impl RunManager {
    pub(super) async fn run_mock(
        &self,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let response = format!("Echo: {}", accepted.text.trim());
        let mut deltas = DeltaBuffer::timed(self, thread_id, run_id);
        for chunk in response.as_bytes().chunks(4) {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    let text = String::from_utf8_lossy(chunk).to_string();
                    deltas.push_text(&text);
                    deltas.flush_if_due()?;
                }
            }
        }
        deltas.flush()?;
        let (content, reasoning) = deltas.into_output();
        self.complete_assistant_message(thread_id, run_id, content, reasoning, None)?;
        Ok(RunOutcome::Completed)
    }

    pub(super) async fn run_provider(
        &self,
        state: &AppState,
        thread_id: &str,
        run_id: &str,
        accepted: &AcceptedTurnV1,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<RunOutcome> {
        let mut messages = control_chat_messages(&self.store, thread_id)?;
        if let Some(context) = managed_preview_runtime_context(&accepted.preview_runtime) {
            messages.insert(0, ChatMessage::text("system", context));
        }
        if let Some(context) = linked_run_context(&accepted.config, &accepted.mailbox_context) {
            messages.insert(0, ChatMessage::text("system", context));
        }
        let context = RunContext::from_control(
            state,
            accepted.config.workspace.as_deref(),
            &accepted.config.privacy,
        )?;
        let service = service_for_run(state, &context);
        let reasoning_effort = accepted
            .config
            .reasoning_effort
            .as_deref()
            .and_then(parse_reasoning_effort);
        let request = CompletionRequest {
            model: accepted.config.model.clone(),
            messages,
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: sampling_from_generation(&accepted.config.generation, thread_id),
            reasoning_effort,
        };
        let journal = RunJournal::new(
            self.store.clone(),
            state.privacy.clone(),
            crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id,
            run_id,
        );
        journal.commit_model_request(1, &request).await?;
        let mut stream = service.stream(request).await?;
        let mut deltas = DeltaBuffer::timed(self, thread_id, run_id);
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        deltas.flush()?;
                        self.persist_interrupted_output(
                            thread_id,
                            run_id,
                            deltas,
                            &WorkLog::default(),
                            STOPPED_BY_USER_MARKER.into(),
                        )?;
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(StreamEvent::Delta(delta))) => {
                            if let Some(text) = delta.content {
                                deltas.push_text(&text);
                            }
                            if let Some(text) = delta.reasoning {
                                deltas.push_reasoning(&text);
                            }
                            deltas.flush_if_due()?;
                        }
                        Some(Ok(StreamEvent::Done { finish_reason, usage })) => {
                            deltas.flush()?;
                            journal
                                .commit_model_response(
                                    1,
                                    deltas.content(),
                                    deltas.reasoning(),
                                    &[],
                                    &finish_reason,
                                    usage,
                                    None,
                                )
                                .await?;
                            let metrics = response_metrics_value(
                                state,
                                &self.store,
                                run_id,
                                &accepted.config.model,
                                Some(usage),
                                None,
                            )
                            .await?;
                            let (content, reasoning) = deltas.into_output();
                            self.complete_assistant_message(
                                thread_id,
                                run_id,
                                content,
                                reasoning,
                                Some(metrics),
                            )?;
                            return Ok(RunOutcome::Completed);
                        }
                        Some(Err(error)) => {
                            // Keep the partial answer visible like the
                            // unterminated-stream case; the stream error wins.
                            let _ = deltas.flush();
                            let _ = self.persist_interrupted_output(
                                thread_id,
                                run_id,
                                deltas,
                                &WorkLog::default(),
                                failed_run_marker(&error.to_string()),
                            );
                            return Err(error);
                        }
                        None => {
                            deltas.flush()?;
                            let error = Error::Other("provider stream ended without a terminal event".into());
                            let _ = self.persist_interrupted_output(
                                thread_id,
                                run_id,
                                deltas,
                                &WorkLog::default(),
                                failed_run_marker(&error.to_string()),
                            );
                            return Err(error);
                        }
                    }
                }
            }
        }
    }
}

/// Opening of the system note that replays a compaction checkpoint; the
/// desktop renderer words its own checkpoint replay the same way.
const CHECKPOINT_NOTE: &str = "Previous thread context checkpoint. Treat this as the durable state for earlier messages that remain visible in the UI but are not replayed below.";

/// The model-visible transcript of a thread, rebuilt from its canonical
/// timeline.
///
/// - The latest `/compact` checkpoint replaces the messages before it with a
///   system note carrying its summary.
/// - User turns replay their prompt text with their image attachments, each
///   after the per-turn context system message it was sent with (stored on
///   the run's assistant message), so the provider's cached prefix from that
///   turn still matches. The latest user turn gets fresh context instead.
/// - Assistant turns replay their model-visible text (without harness
///   notices), then the run's work log, then why the run ended early. The
///   latest turn's work log keeps more detail than earlier ones; only that
///   one message changes when a new turn starts, so the cached prefix before
///   it stays stable.
/// - A user turn whose run was stopped or failed before producing anything
///   is answered by a marker instead of running into the next user turn.
pub(super) fn control_chat_messages(
    store: &UserDataStore,
    thread_id: &str,
) -> Result<Vec<ChatMessage>> {
    let mut stored = store
        .control_projected_messages(thread_id)?
        .into_iter()
        .map(|raw| {
            serde_json::from_str::<Value>(&raw)
                .map_err(|error| Error::Other(format!("invalid stored message: {error}")))
        })
        .collect::<Result<Vec<_>>>()?;
    // A checkpoint message is transcript UI; its summary replays as a note.
    stored.retain(|value| !is_compaction_checkpoint(value));
    let mut messages = Vec::with_capacity(stored.len() + 1);
    if let Some((summary, replaced_ids)) = store.control_compaction_checkpoint(thread_id)? {
        let replaced_ids = replaced_ids.into_iter().collect::<HashSet<_>>();
        stored.retain(|value| {
            !value
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| replaced_ids.contains(id))
        });
        messages.push(ChatMessage::text(
            "system",
            format!("{CHECKPOINT_NOTE}\n\n{}", checkpoint_summary(&summary)),
        ));
    }
    let latest_assistant = stored
        .iter()
        .rposition(|value| value.get("role").and_then(Value::as_str) == Some("assistant"));
    let is_turn = |value: &Value| {
        value.get("role").and_then(Value::as_str) == Some("user")
            && value.get("steering").and_then(Value::as_bool) != Some(true)
    };
    let latest_user = stored.iter().rposition(is_turn);
    let turn_contexts = stored
        .iter()
        .filter(|value| value.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|value| {
            Some((
                value.get("runId")?.as_str()?.to_string(),
                value.get("turnContext")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let mut unanswered_run: Option<String> = None;
    for (index, mut value) in stored.into_iter().enumerate() {
        match value.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                unanswered_run = None;
                let budget = if Some(index) == latest_assistant {
                    WORK_LOG_LATEST_CHARS
                } else {
                    WORK_LOG_EARLIER_CHARS
                };
                value["content"] = Value::String(assistant_replay_text(&value, budget));
            }
            Some("user") if value.get("steering").and_then(Value::as_bool) != Some(true) => {
                if let Some(run_id) = unanswered_run.take() {
                    messages.extend(unanswered_run_marker(store, &run_id)?);
                }
                unanswered_run = value
                    .get("runId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if Some(index) != latest_user {
                    if let Some(context) = unanswered_run
                        .as_ref()
                        .and_then(|run_id| turn_contexts.get(run_id))
                    {
                        messages.push(ChatMessage::text("system", context.clone()));
                    }
                }
                replay_prompt_content(&mut value);
            }
            _ => replay_prompt_content(&mut value),
        }
        with_image_attachments(&mut value);
        messages.push(
            serde_json::from_value(value)
                .map_err(|error| Error::Other(format!("invalid control chat message: {error}")))?,
        );
    }
    Ok(messages)
}

fn replay_prompt_content(message: &mut Value) {
    if let Some(prompt) = message
        .get("promptContent")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        message["content"] = Value::String(prompt);
    }
}

/// An assistant turn as the model replays it: its model-visible text, its
/// run's work log, and why the run ended early.
pub(super) fn assistant_replay_text(message: &Value, work_log_budget: usize) -> String {
    let mut text = message
        .get("promptContent")
        .or_else(|| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let sections = [
        message
            .get("workLog")
            .and_then(|log| render_work_log(log, work_log_budget)),
        message
            .get("interruption")
            .and_then(Value::as_str)
            .map(str::to_string),
    ];
    for section in sections.into_iter().flatten() {
        text.truncate(text.trim_end().len());
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&section);
    }
    text
}

/// The marker for a user turn left without a reply because its run was
/// stopped or failed before producing any output.
fn unanswered_run_marker(store: &UserDataStore, run_id: &str) -> Result<Option<ChatMessage>> {
    let Some(run) = store.control_run(run_id)? else {
        return Ok(None);
    };
    let marker = match run.status.as_str() {
        "cancelled" => STOPPED_BY_USER_MARKER.to_string(),
        "failed" => failed_run_marker(
            run.error_json
                .as_deref()
                .and_then(|error| serde_json::from_str::<Value>(error).ok())
                .as_ref()
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the run failed"),
        ),
        _ => return Ok(None),
    };
    Ok(Some(ChatMessage::text("assistant", marker)))
}

fn is_compaction_checkpoint(message: &Value) -> bool {
    message
        .get("compaction")
        .and_then(|compaction| compaction.get("kind"))
        .and_then(Value::as_str)
        == Some("checkpoint")
}

/// A checkpoint's summary without its `### Context checkpoint` heading.
fn checkpoint_summary(content: &str) -> &str {
    const HEADING: &str = "context checkpoint";
    let content = content.trim();
    let unmarked = content.trim_start_matches('#').trim_start();
    if unmarked.len() < content.len()
        && unmarked
            .get(..HEADING.len())
            .is_some_and(|heading| heading.eq_ignore_ascii_case(HEADING))
    {
        unmarked[HEADING.len()..].trim_start()
    } else {
        content
    }
}

/// Replace a stored message's text with text and `image_url` parts when it
/// carries image attachments, the shape a user turn replays to the model.
pub(super) fn with_image_attachments(message: &mut Value) {
    let image_parts = message
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|attachment| {
            let url = attachment.get("data_url")?.as_str()?;
            attachment
                .get("mime")?
                .as_str()?
                .starts_with("image/")
                .then(|| json!({ "type": "image_url", "image_url": { "url": url } }))
        })
        .collect::<Vec<_>>();
    if image_parts.is_empty() {
        return;
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut parts = vec![json!({ "type": "text", "text": text })];
    parts.extend(image_parts);
    message["content"] = Value::Array(parts);
}
