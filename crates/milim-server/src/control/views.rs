//! Read models: bootstrap, timeline and run-ledger pages, and the record to
//! wire conversions they share.

use std::collections::HashMap;

use milim_control_contract::{
    AgentSummaryV1, ControlBootstrapV1, ControlCapabilitiesV1, ControlProtocolRangeV1,
    EffectiveRunPreviewRequestV1, EffectiveRunPreviewV1, PendingApprovalV1, PendingInputV1,
    QueuedTurnV1, ResolvedRunCompositionV1, RunCapabilitiesV1, RunEventPageV1, RunEventV1,
    RunInspectionV1, RunSnapshotV1, ThreadLinkV1, ThreadSummaryV1, TimelineItemV1, TimelinePageV1,
    CONTROL_PROTOCOL_MAX, CONTROL_PROTOCOL_MIN,
};
use milim_core::{Error, Result};
use milim_storage::{
    ControlApprovalRecord, ControlInboxRecord, ControlQueuedTurnRecord, ControlRunRecord,
    ControlThreadRecord, ControlTimelineRecord,
};
use serde_json::Value;

use super::attachments::validate_control_attachments;
use super::journal::{resolved_run_composition, ModelInputResolver};
use super::native_sessions::{runtime_adapter, runtime_model};
use super::run_config::{parse_reasoning_effort, resolve_frozen_config, thread_agent_id};
use super::{now_ms, parse_value, AcceptedTurnV1, RunManager};
use crate::AppState;

impl RunManager {
    pub async fn bootstrap(&self, state: &AppState) -> Result<ControlBootstrapV1> {
        let store = self.store.clone();
        let (threads, queued, links, runs, inbox, approvals) = crate::blocking::run(move || {
            Ok((
                store.control_threads()?,
                store.control_queued_turns(None)?,
                store.control_thread_links(None)?,
                store.control_runs(true)?,
                store.control_pending_inbox(None)?,
                store.control_pending_approvals()?,
            ))
        })
        .await?;
        let queued_counts = queued
            .iter()
            .fold(HashMap::<String, usize>::new(), |mut map, item| {
                *map.entry(item.thread_id.clone()).or_default() += 1;
                map
            });
        let mut thread_summaries = {
            let active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            threads
                .iter()
                .map(|thread| {
                    thread_summary(
                        thread,
                        active.contains_key(&thread.id),
                        *queued_counts.get(&thread.id).unwrap_or(&0),
                    )
                })
                .collect::<Result<Vec<_>>>()?
        };
        let summary_by_id = thread_summaries
            .iter()
            .map(|thread| (thread.id.clone(), thread.clone()))
            .collect::<HashMap<_, _>>();
        let mut links_by_owner = HashMap::<String, Vec<ThreadLinkV1>>::new();
        for link in links {
            let Some(target) = summary_by_id.get(&link.target_thread_id) else {
                continue;
            };
            let selected_model = target.model.as_deref().unwrap_or_default();
            links_by_owner
                .entry(link.owner_thread_id.clone())
                .or_default()
                .push(ThreadLinkV1 {
                    owner_thread_id: link.owner_thread_id,
                    target_thread_id: target.id.clone(),
                    target_title: target.title.clone(),
                    target_workspace: target.workspace.clone(),
                    target_project: target.workspace.as_deref().and_then(project_label),
                    target_model: target
                        .model
                        .as_deref()
                        .map(runtime_model)
                        .map(str::to_string),
                    target_runtime: runtime_adapter(selected_model).to_string(),
                    target_archived_at_ms: target.archived_at_ms,
                    target_busy: target.busy,
                    target_queued_turns: target.queued_turns,
                    created_at_ms: link.created_at_ms,
                });
        }
        for thread in &mut thread_summaries {
            thread.linked_threads = links_by_owner.remove(&thread.id).unwrap_or_default();
        }
        // A temporarily unavailable provider must not prevent a controller
        // from opening existing threads, stopping work, or resolving an
        // approval. Model discovery can recover on the next bootstrap.
        let models = match self.published_model_catalog() {
            Some(models) => models,
            None => state
                .service
                .list_models()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| Error::Other(format!("serialize models: {error}")))?,
        };
        let agents = state
            .agents
            .as_ref()
            .map(|store| store.list())
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|agent| AgentSummaryV1 {
                id: agent.id,
                name: agent.name,
                description: agent.description,
                avatar: agent.avatar,
                tool_mode: agent.tool_mode,
                enabled_tool_count: agent.enabled_tools.len(),
                skill_mode: agent.skill_mode,
                enabled_skill_count: agent.enabled_skills.len(),
            })
            .collect();
        let active_runs = runs
            .into_iter()
            .map(run_snapshot)
            .collect::<Result<Vec<_>>>()?;
        let queued_turns = queued
            .into_iter()
            .map(queued_turn)
            .collect::<Result<Vec<_>>>()?;
        let pending_inputs = inbox
            .into_iter()
            .filter(|item| item.kind != "followup")
            .map(pending_input)
            .collect::<Result<Vec<_>>>()?;
        let pending_approvals = approvals
            .into_iter()
            .map(pending_approval)
            .collect::<Result<Vec<_>>>()?;
        Ok(ControlBootstrapV1 {
            protocol: ControlProtocolRangeV1 {
                min: CONTROL_PROTOCOL_MIN,
                max: CONTROL_PROTOCOL_MAX,
            },
            host_id: self.host().host_id,
            host_name: self.host().display_name,
            capabilities: ControlCapabilitiesV1::default(),
            appearance: self.appearance_snapshot(),
            threads: thread_summaries,
            models,
            favorite_model_ids: self.model_favorites(),
            agents,
            active_runs,
            queued_turns,
            pending_inputs,
            pending_approvals,
        })
    }

    pub fn timeline_page(
        &self,
        thread_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        tail: bool,
        limit: usize,
    ) -> Result<Option<TimelinePageV1>> {
        // A compatibility import or an incremental renderer write can add a
        // session after startup backfill has run. Ensure its canonical control
        // row exists before the query-only timeline reader looks it up.
        if self.store.control_thread(thread_id)?.is_none() {
            return Ok(None);
        }
        self.store
            .control_timeline_page(thread_id, after_seq, before_seq, tail, limit)?
            .map(|page| {
                let items = page
                    .items
                    .into_iter()
                    .map(timeline_item)
                    .collect::<Result<Vec<_>>>()?;
                Ok(TimelinePageV1 {
                    thread_id: thread_id.to_string(),
                    epoch: page.epoch,
                    first_seq: page.first_seq,
                    last_seq: page.last_seq,
                    has_older: page.has_older,
                    has_newer: page.has_newer,
                    before_seq: page.has_older.then_some(page.first_seq).flatten(),
                    after_seq: page.has_newer.then_some(page.last_seq).flatten(),
                    items,
                })
            })
            .transpose()
    }

    pub fn run_inspection(&self, run_id: &str) -> Result<Option<RunInspectionV1>> {
        let Some(run) = self.store.control_run(run_id)? else {
            return Ok(None);
        };
        let composition = self
            .store
            .control_run_artifacts_by_kind(run_id, "run_composition")?
            .into_iter()
            .next()
            .map(|artifact| {
                serde_json::from_str::<ResolvedRunCompositionV1>(&artifact.data_json).map_err(
                    |error| Error::Other(format!("stored run composition is invalid: {error}")),
                )
            })
            .transpose()?;
        Ok(Some(RunInspectionV1 {
            run: run_snapshot(run)?,
            composition,
        }))
    }

    pub fn effective_run_preview(
        &self,
        state: &AppState,
        thread_id: &str,
        request: EffectiveRunPreviewRequestV1,
    ) -> Result<Option<EffectiveRunPreviewV1>> {
        validate_control_attachments(&request.attachments)?;
        let Some(thread) = self.store.control_thread(thread_id)? else {
            return Ok(None);
        };
        let mut config = resolve_frozen_config(state, &self.store, &thread, request.attachments)?;
        config.linked_thread_grants = self.freeze_linked_thread_grants(thread_id)?;
        if config.agent.is_none() {
            if let Some(agent_id) = thread_agent_id(&thread) {
                return Err(Error::InvalidRequest(format!(
                    "thread is bound to missing Agent {agent_id}; replace or clear the binding before sending"
                )));
            }
        }
        let accepted = AcceptedTurnV1 {
            text: request.text,
            client_message_id: None,
            display_text: None,
            config,
            append_user: true,
            mailbox_origin: None,
            mailbox_context: Vec::new(),
            preview_runtime: None,
        };
        let resolver = ModelInputResolver {
            privacy: &state.privacy,
            privacy_mode: crate::privacy::PrivacyMode::parse(&accepted.config.privacy),
        };
        let mut warnings = Vec::new();
        if accepted.text.trim().is_empty() && accepted.config.attachments.is_empty() {
            warnings.push("No draft or attachment is included in this preview.".to_string());
        }
        if !self
            .store
            .control_pending_inbox(Some(thread_id))?
            .is_empty()
        {
            warnings.push(
                "Pending inbox inputs are claimed atomically only when the turn starts and are not included in this preview."
                    .to_string(),
            );
        }
        if accepted.config.tool_mode != "custom" && accepted.config.enabled_tools.is_empty() {
            warnings.push(
                "The inherited tool registry is resolved when the run starts; this preview shows its frozen policy rather than every runtime tool schema."
                    .to_string(),
            );
        }
        Ok(Some(EffectiveRunPreviewV1 {
            thread_id: thread.id,
            thread_revision: thread.revision,
            resolved_at_ms: now_ms(),
            composition: resolved_run_composition(&accepted, &resolver)?,
            warnings,
        }))
    }

    pub fn run_event_page(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Option<RunEventPageV1>> {
        if self.store.control_run(run_id)?.is_none() {
            return Ok(None);
        }
        let limit = limit.clamp(1, 200);
        let mut records =
            self.store
                .control_run_events(run_id, after_seq, limit.saturating_add(1))?;
        let has_more = records.len() > limit;
        records.truncate(limit);
        let artifact_digests = records
            .iter()
            .filter_map(|record| parse_value(&record.data_json).ok())
            .filter_map(|data| {
                data.get("artifact_digest")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let artifacts = self
            .store
            .control_run_artifacts_by_digests(run_id, &artifact_digests)?
            .into_iter()
            .map(|artifact| (artifact.digest, artifact.data_json))
            .collect::<HashMap<_, _>>();
        let events = records
            .into_iter()
            .map(|record| {
                let mut data = parse_value(&record.data_json)?;
                if let Some(object) = data.as_object_mut() {
                    if let Some(artifact) = object
                        .get("artifact_digest")
                        .and_then(Value::as_str)
                        .and_then(|digest| artifacts.get(digest))
                    {
                        object.insert("artifact".into(), parse_value(artifact)?);
                    }
                }
                Ok(RunEventV1 {
                    id: record.event_id,
                    run_id: record.run_id,
                    seq: record.seq,
                    step_id: record.step_id,
                    event_type: record.event_type,
                    data,
                    created_at_ms: record.created_at_ms,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_seq = has_more
            .then(|| events.last().map(|event| event.seq))
            .flatten();
        Ok(Some(RunEventPageV1 {
            run_id: run_id.to_string(),
            after_seq,
            next_seq,
            has_more,
            events,
        }))
    }
}

pub(super) fn project_label(workspace: &str) -> Option<String> {
    std::path::Path::new(workspace)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
}

pub(super) fn thread_title(thread: &ControlThreadRecord) -> String {
    serde_json::from_str::<Value>(&thread.session_json)
        .ok()
        .and_then(|value| {
            value
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "New chat".into())
}

pub(super) fn thread_summary(
    thread: &ControlThreadRecord,
    busy: bool,
    queued_turns: usize,
) -> Result<ThreadSummaryV1> {
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    let settings = value.get("settings").and_then(Value::as_object);
    Ok(ThreadSummaryV1 {
        id: thread.id.clone(),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("New chat")
            .to_string(),
        revision: thread.revision,
        epoch: thread.epoch.clone(),
        updated_at_ms: thread.updated_at_ms,
        archived_at_ms: value.get("archivedAt").and_then(Value::as_i64),
        model: settings
            .and_then(|settings| settings.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string),
        reasoning_effort_overrides: settings
            .and_then(|settings| settings.get("reasoningEffortOverrides"))
            .and_then(Value::as_object)
            .map(|overrides| {
                overrides
                    .iter()
                    .filter_map(|(model, effort)| {
                        let effort = effort.as_str()?;
                        parse_reasoning_effort(effort)?;
                        Some((model.clone(), effort.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        agent_id: settings
            .and_then(|settings| settings.get("activeAgentId"))
            .and_then(Value::as_str)
            .map(str::to_string),
        workspace: settings
            .and_then(|settings| settings.get("folder"))
            .and_then(Value::as_str)
            .map(str::to_string),
        origin: value
            .get("origin")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| Error::Other(format!("invalid stored thread origin: {error}")))?,
        busy,
        queued_turns,
        linked_threads: Vec::new(),
    })
}

pub(super) fn run_snapshot(run: ControlRunRecord) -> Result<RunSnapshotV1> {
    let accepted: AcceptedTurnV1 = serde_json::from_str(&run.request_json)
        .map_err(|error| Error::Other(format!("invalid stored run snapshot: {error}")))?;
    let visibility = if matches!(run.adapter.as_str(), "codex" | "claude" | "opencode" | "pi") {
        "harness_boundary"
    } else {
        "model_visible"
    };
    let steering = accepted.config.agent.is_some() || accepted.config.adapter == "provider";
    Ok(RunSnapshotV1 {
        id: run.id,
        thread_id: run.thread_id,
        status: run.status,
        adapter: run.adapter,
        config: accepted.config,
        capabilities: RunCapabilitiesV1 {
            ledger: true,
            inspectable: true,
            steering,
            visibility: visibility.into(),
        },
        created_at_ms: run.created_at_ms,
        updated_at_ms: run.updated_at_ms,
        completed_at_ms: run.completed_at_ms,
        error: run.error_json.as_deref().map(parse_value).transpose()?,
    })
}

fn pending_input(item: ControlInboxRecord) -> Result<PendingInputV1> {
    let accepted = (item.kind == "steer")
        .then(|| serde_json::from_str::<AcceptedTurnV1>(&item.payload_json))
        .transpose()
        .map_err(|error| Error::Other(format!("stored steering input is invalid: {error}")))?;
    let display_text = accepted.as_ref().map(|accepted| {
        accepted
            .display_text
            .clone()
            .unwrap_or_else(|| accepted.text.clone())
    });
    let attachments = accepted.map(|accepted| accepted.config.attachments);
    Ok(PendingInputV1 {
        id: item.id,
        thread_id: item.thread_id,
        target_run_id: item.target_run_id,
        kind: item.kind,
        state: item.state,
        display_text,
        attachments,
        created_at_ms: item.created_at_ms,
    })
}

fn queued_turn(turn: ControlQueuedTurnRecord) -> Result<QueuedTurnV1> {
    let accepted = serde_json::from_str::<AcceptedTurnV1>(&turn.request_json)
        .map_err(|error| Error::Other(format!("stored queued turn is invalid: {error}")))?;
    Ok(QueuedTurnV1 {
        id: turn.id,
        thread_id: turn.thread_id,
        command_id: turn.command_id,
        accepted_at_ms: turn.accepted_at_ms,
        display_text: accepted.display_text.unwrap_or(accepted.text),
        attachments: accepted.config.attachments,
        mailbox_origin: accepted.mailbox_origin,
    })
}

fn pending_approval(approval: ControlApprovalRecord) -> Result<PendingApprovalV1> {
    Ok(PendingApprovalV1 {
        id: approval.id,
        run_id: approval.run_id,
        thread_id: approval.thread_id,
        kind: approval.kind,
        request: parse_value(&approval.request_json)?,
        status: approval.status,
        created_at_ms: approval.created_at_ms,
    })
}

pub(super) fn timeline_item(record: ControlTimelineRecord) -> Result<TimelineItemV1> {
    Ok(TimelineItemV1 {
        id: record.item_id,
        thread_id: record.thread_id,
        epoch: record.epoch,
        seq: record.seq,
        run_id: record.run_id,
        item_type: record.item_type,
        data: parse_value(&record.data_json)?,
        created_at_ms: record.created_at_ms,
    })
}
