//! Direct provider runs (and the mock runtime used by tests and demos).

use std::time::{Duration, Instant};

use futures::StreamExt;
use milim_agents::AgentStepHook as _;
use milim_core::api::openai::ChatMessage;
use milim_core::{Error, Result};
use milim_inference::{CompletionRequest, StreamEvent};
use milim_storage::UserDataStore;
use serde_json::{json, Value};
use tokio::sync::watch;

use super::delta::{flush_deltas, DELTA_FLUSH_BYTES, DELTA_FLUSH_INTERVAL};
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
        let mut content = String::new();
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        let mut emitted_first_delta = false;
        let mut last_flush = Instant::now();
        for chunk in response.as_bytes().chunks(4) {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {
                    let text = String::from_utf8_lossy(chunk).to_string();
                    content.push_str(&text);
                    pending_text.push_str(&text);
                    if !emitted_first_delta
                        || pending_text.len() >= DELTA_FLUSH_BYTES
                        || last_flush.elapsed() >= DELTA_FLUSH_INTERVAL
                    {
                        flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                        emitted_first_delta = true;
                        last_flush = Instant::now();
                    }
                }
            }
        }
        flush_deltas(
            self,
            thread_id,
            run_id,
            &mut pending_text,
            &mut pending_reasoning,
        )?;
        self.complete_assistant_message(thread_id, run_id, content, String::new(), None)?;
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
        let journal = RunJournal {
            store: self.store.clone(),
            privacy: state.privacy.clone(),
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
            thread_id: thread_id.to_string(),
            run_id: run_id.to_string(),
        };
        journal.commit_model_request(1, &request).await?;
        let mut stream = service.stream(request).await?;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut pending_text = String::new();
        let mut pending_reasoning = String::new();
        let mut emitted_first_delta = false;
        let mut last_flush = Instant::now();
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_ok() && *stop.borrow() {
                        flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                        return Ok(RunOutcome::Cancelled);
                    }
                }
                event = stream.next() => {
                    match event {
                        Some(Ok(StreamEvent::Delta(delta))) => {
                            if let Some(text) = delta.content {
                                content.push_str(&text);
                                pending_text.push_str(&text);
                            }
                            if let Some(text) = delta.reasoning {
                                reasoning.push_str(&text);
                                pending_reasoning.push_str(&text);
                            }
                            if !emitted_first_delta
                                || pending_text.len() + pending_reasoning.len() >= DELTA_FLUSH_BYTES
                                || last_flush.elapsed() >= DELTA_FLUSH_INTERVAL
                            {
                                flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                                emitted_first_delta = true;
                                last_flush = Instant::now();
                            }
                        }
                        Some(Ok(StreamEvent::Done { finish_reason, usage })) => {
                            flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                            journal
                                .commit_model_response(
                                    1,
                                    &content,
                                    &reasoning,
                                    &[],
                                    &finish_reason,
                                    usage,
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
                            self.complete_assistant_message(
                                thread_id,
                                run_id,
                                content,
                                reasoning,
                                Some(metrics),
                            )?;
                            return Ok(RunOutcome::Completed);
                        }
                        Some(Err(error)) => return Err(error),
                        None => {
                            flush_deltas(self, thread_id, run_id, &mut pending_text, &mut pending_reasoning)?;
                            return Err(Error::Other("provider stream ended without a terminal event".into()));
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn control_chat_messages(
    store: &UserDataStore,
    thread_id: &str,
) -> Result<Vec<ChatMessage>> {
    store
        .control_projected_messages(thread_id)?
        .into_iter()
        .map(|raw| {
            let mut value: Value = serde_json::from_str(&raw)
                .map_err(|error| Error::Other(format!("invalid stored message: {error}")))?;
            if let Some(prompt) = value
                .get("promptContent")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                value["content"] = Value::String(prompt);
            }
            let image_parts = value
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
            if !image_parts.is_empty() {
                let text = value
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut parts = vec![json!({ "type": "text", "text": text })];
                parts.extend(image_parts);
                value["content"] = Value::Array(parts);
            }
            serde_json::from_value(value)
                .map_err(|error| Error::Other(format!("invalid control chat message: {error}")))
        })
        .collect()
}
