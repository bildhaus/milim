//! Direct provider runs (and the mock runtime used by tests and demos), and
//! the model-visible thread history every provider and Agent run starts from.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use futures::StreamExt;
use milim_agents::AgentStepHook as _;
use milim_core::api::openai::{ChatMessage, Content, ContentPart};
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
use crate::privacy::PrivacyMode;
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
        // A request without tools cannot carry earlier runs' tool calls.
        let target = ReplayTarget {
            model: &accepted.config.model,
            privacy: PrivacyMode::parse(&accepted.config.privacy),
            tools: false,
        };
        let mut messages = control_chat_messages(&self.store, thread_id, &target)?;
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

/// What the next request can take from earlier runs' recorded messages.
#[derive(Debug, Clone, Copy)]
pub(super) struct ReplayTarget<'a> {
    /// The model the next request goes to.
    pub(super) model: &'a str,
    /// The privacy mode of the next request.
    pub(super) privacy: PrivacyMode,
    /// Whether the next request offers tools. Providers reject tool calls in
    /// a history sent without tools, so then every earlier run replays as
    /// its text and work log.
    pub(super) tools: bool,
}

/// The model-visible transcript of a thread, rebuilt from its canonical
/// timeline.
///
/// - The latest `/compact` checkpoint replaces the messages before it with a
///   system note carrying its summary.
/// - User turns replay their prompt text with their image attachments, each
///   after the per-turn context system message it was sent with (stored on
///   the run's assistant message), so the provider's cached prefix from that
///   turn still matches. The latest user turn gets fresh context instead.
/// - A tool-agent run that recorded its own messages (see
///   [`TurnMessages`](super::journal::TurnMessages)) replays them as its
///   model last saw them: tool calls, model-visible tool results, and
///   intermediate text, then why the run ended early. That replay is the
///   same on every later turn, so the next turn's prefix matches the
///   previous turn's last request unless something in it was scrubbed.
/// - Other assistant turns (account runtimes, plain chat, runs from before
///   messages were recorded or under Redact privacy) replay their
///   model-visible text (without harness notices), then the run's work log,
///   then why the run ended early. The latest turn's work log keeps more
///   detail than earlier ones; only that one message changes when a new turn
///   starts, so the cached prefix before it stays stable.
/// - A user turn whose run was stopped or failed before producing anything
///   is answered by a marker instead of running into the next user turn.
pub(super) fn control_chat_messages(
    store: &UserDataStore,
    thread_id: &str,
    target: &ReplayTarget<'_>,
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
    let run_of = |value: &Value| {
        value
            .get("runId")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let assistants = stored
        .iter()
        .filter(|value| value.get("role").and_then(Value::as_str) == Some("assistant"))
        .filter_map(|value| Some((run_of(value)?, value)))
        .collect::<Vec<_>>();
    let turn_contexts = assistants
        .iter()
        .filter_map(|(run_id, value)| {
            Some((
                run_id.clone(),
                value.get("turnContext")?.as_str()?.to_string(),
            ))
        })
        .collect::<HashMap<_, _>>();
    let mut recorded_turns = assistants
        .iter()
        .filter(|_| target.tools)
        .filter_map(|(run_id, value)| {
            Some((run_id.clone(), recorded_turn(store, run_id, value, target)?))
        })
        .collect::<HashMap<_, _>>();
    // Steering that a recorded run never consumed (it ended first) replays
    // after the run's messages rather than before them.
    let mut unconsumed_steering = Vec::new();
    let mut unanswered_run: Option<String> = None;
    for (index, mut value) in stored.into_iter().enumerate() {
        match value.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                unanswered_run = None;
                if let Some(turn) = run_of(&value).and_then(|run| recorded_turns.remove(&run)) {
                    messages.extend(turn);
                    messages.append(&mut unconsumed_steering);
                    continue;
                }
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
                unanswered_run = run_of(&value);
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
            Some("user") => {
                replay_prompt_content(&mut value);
                if let Some(turn) = run_of(&value).and_then(|run| recorded_turns.get(&run)) {
                    let text = value
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let consumed = turn
                        .iter()
                        .any(|message| message.role == "user" && message.text_content() == text);
                    if !consumed {
                        with_image_attachments(&mut value);
                        unconsumed_steering.push(chat_message(value)?);
                    }
                    continue;
                }
            }
            _ => replay_prompt_content(&mut value),
        }
        with_image_attachments(&mut value);
        messages.push(chat_message(value)?);
    }
    Ok(messages)
}

fn chat_message(value: Value) -> Result<ChatMessage> {
    serde_json::from_value(value)
        .map_err(|error| Error::Other(format!("invalid control chat message: {error}")))
}

/// A run's recorded messages ready for `target`, or `None` when the run
/// replays as its text and work log instead: it recorded none, recorded them
/// under Redact privacy (their placeholders would collide with the next
/// request's own), or they hold something a Block request would refuse.
///
/// A turn another model produced drops the continuation data and reasoning
/// only that model can use, and its tool call ids are made acceptable to
/// every adapter. Without Off privacy, images become a short note, since the
/// outbound gate refuses image data.
fn recorded_turn(
    store: &UserDataStore,
    run_id: &str,
    message: &Value,
    target: &ReplayTarget<'_>,
) -> Option<Vec<ChatMessage>> {
    let recorded = match super::journal::turn_messages(store, run_id) {
        Ok(recorded) => recorded?,
        Err(error) => {
            tracing::warn!(%run_id, "replaying the run's work log instead of its messages: {error}");
            return None;
        }
    };
    if PrivacyMode::parse(&recorded.privacy) == PrivacyMode::Redact
        || !recorded
            .messages
            .iter()
            .any(|message| message.role == "assistant")
    {
        return None;
    }
    let mut turn = recorded.messages;
    if recorded.model != target.model {
        for message in &mut turn {
            message.provider_state = None;
            message.reasoning_content = None;
            for call in message.tool_calls.iter_mut().flatten() {
                call.id = call.id.as_deref().map(portable_call_id);
            }
            message.tool_call_id = message.tool_call_id.as_deref().map(portable_call_id);
        }
    }
    if target.privacy != PrivacyMode::Off {
        for part in turn
            .iter_mut()
            .flat_map(|message| match &mut message.content {
                Some(Content::Parts(parts)) => parts.as_mut_slice(),
                _ => &mut [],
            })
        {
            if matches!(part, ContentPart::ImageUrl { .. }) {
                *part = ContentPart::Text {
                    text: IMAGE_NOT_REPLAYED.to_string(),
                };
            }
        }
    }
    if target.privacy == PrivacyMode::Block
        && !crate::privacy::scan_request(&CompletionRequest {
            model: String::new(),
            messages: turn.clone(),
            tools: Vec::new(),
            tool_choice: None,
            response_format: None,
            prompt: None,
            suffix: None,
            sampling: Default::default(),
            reasoning_effort: None,
        })
        .is_empty()
    {
        return None;
    }
    if let Some(marker) = message.get("interruption").and_then(Value::as_str) {
        match turn.last_mut() {
            Some(last)
                if last.role == "assistant"
                    && last.tool_calls.is_none()
                    && matches!(&last.content, Some(Content::Text(text)) if !text.trim().is_empty()) =>
            {
                let text = last.text_content();
                last.content = Some(Content::Text(format!("{}\n\n{marker}", text.trim_end())));
            }
            _ => turn.push(ChatMessage::text("assistant", marker)),
        }
    }
    Some(turn)
}

/// What replaces an earlier turn's image when privacy is not Off.
const IMAGE_NOT_REPLAYED: &str =
    "[image from an earlier turn not replayed: the privacy gate does not send images]";

/// A tool call id in the characters every adapter accepts (Anthropic allows
/// only letters, digits, `_`, and `-`), mapped the same way for a call and
/// its result.
fn portable_call_id(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect()
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
