//! Thread commands: create, patch, delete, link, and message deletion.

use milim_control_contract::{
    ControlCommandResultV1, ControlCommandStatusV1, ControlCommandV1, ThreadOriginV1,
};
use milim_core::{Error, Result};
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::commands::{required_payload_string, required_thread_id};
use super::{now_ms, parse_reasoning_effort, thread_title, timeline_item, RunManager, ThreadPatch};

impl RunManager {
    pub(super) fn create_thread(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let id = command
            .payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if let Some(thread) = self.store.control_thread(&id)? {
            return Ok(ControlCommandResultV1 {
                command_id: command.command_id.clone(),
                status: ControlCommandStatusV1::Applied,
                thread_id: Some(thread.id),
                revision: Some(thread.revision),
                run_id: None,
                queue_id: None,
                confirmation_token: None,
                message: None,
                data: Value::Null,
            });
        }
        let title = command
            .payload
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("New chat");
        let now = now_ms();
        let mut session = json!({
            "id": id,
            "title": title,
            "createdAt": now,
            "updatedAt": now,
            "settings": command.payload.get("settings").cloned().unwrap_or_else(|| json!({}))
        });
        let cloning = command.payload.get("source_thread_id").is_some();
        if cloning {
            let source_id = command
                .payload
                .get("source_thread_id")
                .and_then(Value::as_str)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    Error::InvalidRequest("source_thread_id must identify a thread".into())
                })?;
            let source = self.store.session_snapshot(source_id)?;
            let mut messages = source
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| Error::Other("invalid source messages".into()))?;
            if let Some(count) = command.payload.get("source_message_count") {
                if count.as_u64() != Some(0) || command.payload.get("through_message_id").is_some()
                {
                    return Err(Error::InvalidRequest("Only source_message_count: 0 is supported; use through_message_id for a branch boundary.".into()));
                }
                messages.clear();
            } else if let Some(boundary) = command.payload.get("through_message_id") {
                let boundary = boundary.as_str().ok_or_else(|| {
                    Error::InvalidRequest("through_message_id must be a message ID".into())
                })?;
                let index = messages
                    .iter()
                    .position(|message| {
                        message.get("id").and_then(Value::as_str) == Some(boundary)
                            || message.get("canonicalId").and_then(Value::as_str) == Some(boundary)
                    })
                    .ok_or_else(|| {
                        Error::InvalidRequest(
                            "The branch message no longer exists. Refresh the source chat.".into(),
                        )
                    })?;
                messages.truncate(index + 1);
            }
            for message in &mut messages {
                let object = message
                    .as_object_mut()
                    .ok_or_else(|| Error::Other("invalid source message".into()))?;
                object.insert("id".into(), json!(Uuid::new_v4().to_string()));
                for field in [
                    "canonicalId",
                    "runId",
                    "isStreaming",
                    "streamRenderId",
                    "workerRunId",
                    "mailboxOrigin",
                ] {
                    object.remove(field);
                }
            }
            if command.payload.get("title").is_none() {
                session["title"] = json!(format!(
                    "{} branch",
                    source
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Chat")
                ));
            }
            let mut settings = source
                .get("settings")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(overrides) = command.payload.get("settings") {
                let overrides = overrides
                    .as_object()
                    .ok_or_else(|| Error::InvalidRequest("settings must be an object".into()))?;
                let changed_folder = overrides
                    .get("folder")
                    .is_some_and(|folder| Some(folder) != settings.get("folder"));
                settings.extend(overrides.clone());
                if changed_folder {
                    settings.insert("toolApproval".into(), json!("review"));
                }
            }
            if let Some(goal) = settings.get_mut("goal").and_then(Value::as_object_mut) {
                if goal
                    .get("status")
                    .and_then(Value::as_str)
                    .is_some_and(|status| {
                        matches!(status, "running" | "waiting_for_worker_approval")
                    })
                {
                    goal.insert("status".into(), json!("paused"));
                }
            }
            session["settings"] = Value::Object(settings);
            session["messages"] = Value::Array(messages);
            session["parentId"] = json!(source_id);
            // A branch shares the effective folder, but does not own the
            // source thread's managed worktree lifecycle.
            for field in ["virtualFiles", "project"] {
                if let Some(value) = source.get(field) {
                    session[field] = value.clone();
                }
            }
        }
        if let Some(project) = command.payload.get("project") {
            session["project"] = project.clone();
        }
        if let Some(origin) = command.payload.get("origin") {
            let _: ThreadOriginV1 = serde_json::from_value(origin.clone()).map_err(|error| {
                Error::InvalidRequest(format!("invalid thread origin: {error}"))
            })?;
            session["origin"] = origin.clone();
        }
        let thread = self.store.control_create_thread(
            &id,
            &session.to_string(),
            &Uuid::new_v4().to_string(),
        )?;
        self.emit_thread_changed(&thread, "thread.created");
        let data = if cloning {
            json!({ "session": session })
        } else {
            Value::Null
        };
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread.id),
            revision: Some(thread.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data,
        })
    }

    pub(super) fn patch_thread(
        &self,
        command: &ControlCommandV1,
        patch: ThreadPatch,
    ) -> Result<ControlCommandResultV1> {
        let id = required_thread_id(command)?;
        let current = self
            .store
            .control_thread(id)?
            .ok_or_else(|| Error::NotFound(format!("thread {id}")))?;
        if let Some(expected) = command.expected_revision {
            if expected != current.revision {
                return Err(Error::InvalidRequest(format!(
                    "thread revision conflict: expected {expected}, current {}",
                    current.revision
                )));
            }
        }
        let mut value: Value = serde_json::from_str(&current.session_json)
            .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| Error::Other("stored thread is not an object".into()))?;
        let mut model_change: Option<(String, String)> = None;
        match patch {
            ThreadPatch::Rename => {
                let title = required_payload_string(&command.payload, "title")?;
                object.insert("title".into(), Value::String(title));
            }
            ThreadPatch::Archive => {
                let archived = command
                    .payload
                    .get("archived")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if archived {
                    object.insert("archivedAt".into(), Value::from(now_ms()));
                } else {
                    object.remove("archivedAt");
                }
            }
            ThreadPatch::Model => {
                let previous_model = object
                    .get("settings")
                    .and_then(Value::as_object)
                    .and_then(|settings| settings.get("model"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let model = command
                    .payload
                    .get("model")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::InvalidRequest("payload.model must be a string".into()))?
                    .trim()
                    .to_string();
                let reasoning_effort = match command.payload.get("reasoning_effort") {
                    Some(value) => {
                        let raw = value.as_str().ok_or_else(|| {
                            Error::InvalidRequest(
                                "payload.reasoning_effort must be a supported string".into(),
                            )
                        })?;
                        Some(parse_reasoning_effort(raw).ok_or_else(|| {
                            Error::InvalidRequest(format!(
                                "unsupported payload.reasoning_effort: {raw}"
                            ))
                        })?)
                    }
                    None => None,
                };
                if !previous_model.is_empty() && !model.is_empty() && previous_model != model {
                    model_change = Some((previous_model, model.clone()));
                }
                let settings = settings_object(object)?;
                settings.insert("model".into(), Value::String(model.clone()));
                if !model.is_empty() {
                    if let Some(reasoning_effort) = reasoning_effort {
                        let overrides = settings
                            .entry("reasoningEffortOverrides")
                            .or_insert_with(|| json!({}))
                            .as_object_mut()
                            .ok_or_else(|| {
                                Error::Other(
                                    "stored reasoning effort overrides are not an object".into(),
                                )
                            })?;
                        overrides
                            .insert(model, Value::String(reasoning_effort.as_str().to_string()));
                    }
                }
            }
            ThreadPatch::Agent => {
                let agent = command
                    .payload
                    .get("agent_id")
                    .cloned()
                    .unwrap_or(Value::Null);
                if !agent.is_null() && !agent.is_string() {
                    return Err(Error::InvalidRequest(
                        "agent_id must be a string or null".into(),
                    ));
                }
                settings_object(object)?.insert("activeAgentId".into(), agent);
            }
            // Which signed-in account of one runtime this thread uses. The
            // value is stored, not resolved: `auto` stays `auto` so each turn
            // re-picks with current usage, and an id names one account.
            ThreadPatch::AccountProfile => {
                let payload = command.payload.as_object().ok_or_else(|| {
                    Error::InvalidRequest("account profile payload must be an object".into())
                })?;
                let runtime = payload
                    .get("runtime")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| Error::InvalidRequest("runtime is required".into()))?;
                if !crate::account_profiles::PROFILE_RUNTIMES.contains(&runtime) {
                    return Err(Error::InvalidRequest(format!(
                        "{runtime} does not support account profiles"
                    )));
                }
                let profile = match payload.get("profile_id") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(value)) => {
                        let value = value.trim();
                        if value.is_empty() {
                            return Err(Error::InvalidRequest(
                                "profile_id must not be empty".into(),
                            ));
                        }
                        Some(value.to_string())
                    }
                    Some(_) => {
                        return Err(Error::InvalidRequest(
                            "profile_id must be a string or null".into(),
                        ))
                    }
                };
                let settings = settings_object(object)?;
                let profiles = settings
                    .entry("accountProfiles")
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .ok_or_else(|| {
                        Error::InvalidRequest("accountProfiles must be an object".into())
                    })?;
                match profile {
                    // Clearing returns the thread to the runtime's own account.
                    None => {
                        profiles.remove(runtime);
                    }
                    Some(profile) => {
                        profiles.insert(runtime.to_string(), Value::String(profile));
                    }
                }
            }
            ThreadPatch::Execution => {
                const ALLOWED: &[&str] = &[
                    "workspace",
                    "privacy",
                    "tool_approval",
                    "memory",
                    "sandbox",
                    "computer_use",
                    "plan_mode",
                    "delegation_policy",
                    "worker_model",
                ];
                let payload = command.payload.as_object().ok_or_else(|| {
                    Error::InvalidRequest("execution settings payload must be an object".into())
                })?;
                if let Some(key) = payload.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
                    return Err(Error::InvalidRequest(format!(
                        "unsupported execution setting: {key}"
                    )));
                }
                let previous_workspace = object
                    .get("settings")
                    .and_then(Value::as_object)
                    .and_then(|settings| settings.get("folder"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let settings = settings_object(object)?;
                let mut workspace_changed = false;
                for (wire_key, stored_key) in [
                    ("memory", "memory"),
                    ("sandbox", "sandbox"),
                    ("computer_use", "computerUse"),
                    ("plan_mode", "planMode"),
                ] {
                    if let Some(value) = payload.get(wire_key) {
                        let value = value.as_bool().ok_or_else(|| {
                            Error::InvalidRequest(format!("{wire_key} must be a boolean"))
                        })?;
                        settings.insert(stored_key.into(), Value::Bool(value));
                    }
                }
                if let Some(value) = payload.get("workspace") {
                    if !value.is_null() && !value.is_string() {
                        return Err(Error::InvalidRequest(
                            "workspace must be a string or null".into(),
                        ));
                    }
                    let workspace = value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| Value::String(value.to_string()))
                        .unwrap_or(Value::Null);
                    let next_workspace = workspace.as_str().unwrap_or_default();
                    workspace_changed = previous_workspace != next_workspace;
                    settings.insert("folder".into(), workspace);
                }
                if let Some(value) = payload.get("privacy") {
                    let value = value.as_str().ok_or_else(|| {
                        Error::InvalidRequest("privacy must be a supported string".into())
                    })?;
                    if !matches!(value, "off" | "redact" | "block") {
                        return Err(Error::InvalidRequest(format!(
                            "unsupported privacy mode: {value}"
                        )));
                    }
                    settings.insert("privacy".into(), Value::String(value.to_string()));
                }
                if let Some(value) = payload.get("tool_approval") {
                    let value = value.as_str().ok_or_else(|| {
                        Error::InvalidRequest("tool_approval must be a supported string".into())
                    })?;
                    if !matches!(value, "review" | "guarded" | "open") {
                        return Err(Error::InvalidRequest(format!(
                            "unsupported tool approval mode: {value}"
                        )));
                    }
                    settings.insert("toolApproval".into(), Value::String(value.to_string()));
                }
                if let Some(value) = payload.get("delegation_policy") {
                    let value = value.as_str().ok_or_else(|| {
                        Error::InvalidRequest("delegation_policy must be a supported string".into())
                    })?;
                    if !matches!(value, "off" | "ask" | "auto") {
                        return Err(Error::InvalidRequest(format!(
                            "unsupported delegation policy: {value}"
                        )));
                    }
                    settings.insert("delegationPolicy".into(), Value::String(value.to_string()));
                }
                if let Some(value) = payload.get("worker_model") {
                    let value = value.as_str().ok_or_else(|| {
                        Error::InvalidRequest("worker_model must be a string".into())
                    })?;
                    settings.insert("workerModel".into(), Value::String(value.to_string()));
                }
                if workspace_changed {
                    settings.insert("toolApproval".into(), Value::String("review".to_string()));
                    // "Allow for this chat" rules were granted for the old
                    // project boundary; a new folder starts asking again.
                    self.revoke_approval_allowances(id, None)?;
                }
            }
        }
        object.insert("updatedAt".into(), Value::from(now_ms()));
        let timeline_item_id = model_change
            .as_ref()
            .map(|_| format!("model-change-{}", Uuid::new_v4()));
        let timeline_data = model_change.as_ref().map(|(previous_model, model)| {
            json!({
                "previous_model": previous_model,
                "model": model,
            })
            .to_string()
        });
        let timeline = timeline_item_id
            .as_deref()
            .zip(timeline_data.as_deref())
            .map(|(item_id, data_json)| (item_id, "model_changed", data_json));
        let (updated, timeline) = self
            .store
            .control_update_thread(id, &value.to_string(), command.expected_revision, timeline)?
            .ok_or_else(|| Error::NotFound(format!("thread {id}")))?;
        if let Some(timeline) = timeline {
            let epoch = timeline.epoch.clone();
            let seq = timeline.seq;
            let item = timeline_item(timeline)?;
            self.emit(
                "timeline.appended",
                Some(id),
                Some(&epoch),
                Some(seq),
                json!({ "item": item }),
            );
        }
        self.emit_thread_changed(&updated, "thread.updated");
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(updated.id),
            revision: Some(updated.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    pub(super) fn delete_thread(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let id = required_thread_id(command)?;
        if self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before deleting this thread".into(),
            ));
        }
        if !self.store.control_delete_thread(id)? {
            return Err(Error::NotFound(format!("thread {id}")));
        }
        let _ = crate::approval_allowances::revoke(&self.store, id, None);
        self.emit(
            "thread.deleted",
            Some(id),
            None,
            None,
            json!({ "thread_id": id }),
        );
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(id.to_string()),
            revision: None,
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }

    pub(super) fn link_thread(
        &self,
        command: &ControlCommandV1,
        add: bool,
    ) -> Result<ControlCommandResultV1> {
        let owner_thread_id = required_thread_id(command)?;
        let target_thread_id = required_payload_string(&command.payload, "target_thread_id")?;
        if owner_thread_id == target_thread_id {
            return Err(Error::InvalidRequest(
                "a thread cannot link to itself".into(),
            ));
        }
        let owner = self
            .store
            .control_thread(owner_thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {owner_thread_id}")))?;
        if let Some(expected) = command.expected_revision {
            if expected != owner.revision {
                return Err(Error::InvalidRequest(format!(
                    "thread revision conflict: expected {expected}, current {}",
                    owner.revision
                )));
            }
        }
        let target = self.store.control_thread(&target_thread_id)?;
        if add && target.is_none() {
            return Err(Error::NotFound(format!("thread {target_thread_id}")));
        }
        let timelines = if add {
            self.store.control_add_thread_link(
                owner_thread_id,
                &target_thread_id,
                &Uuid::new_v4().to_string(),
            )?
        } else {
            self.store.control_remove_thread_link(
                owner_thread_id,
                &target_thread_id,
                &Uuid::new_v4().to_string(),
            )?
        };
        let current = self
            .store
            .control_thread(owner_thread_id)?
            .ok_or_else(|| Error::NotFound(format!("thread {owner_thread_id}")))?;
        let event_type = if add {
            "thread.link.added"
        } else {
            "thread.link.removed"
        };
        let owner_title = thread_title(&owner);
        let target_title = target
            .as_ref()
            .map(thread_title)
            .unwrap_or_else(|| "Unavailable chat".into());
        for timeline in timelines {
            let (event_target_id, event_target_title) = if timeline.thread_id == owner_thread_id {
                (target_thread_id.as_str(), target_title.as_str())
            } else {
                (owner_thread_id, owner_title.as_str())
            };
            self.emit(
                event_type,
                Some(&timeline.thread_id),
                Some(&timeline.epoch),
                Some(timeline.seq),
                json!({
                    "owner_thread_id": timeline.thread_id,
                    "target_thread_id": event_target_id,
                    "target_title": event_target_title,
                }),
            );
        }
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(owner_thread_id.to_string()),
            revision: Some(current.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: json!({
                "target_thread_id": target_thread_id,
                "target_title": target.as_ref().map(thread_title).unwrap_or_else(|| "Unavailable chat".into()),
                "linked": add,
            }),
        })
    }

    pub(super) fn delete_message(
        &self,
        command: &ControlCommandV1,
    ) -> Result<ControlCommandResultV1> {
        let thread_id = required_thread_id(command)?;
        if self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(thread_id)
        {
            return Err(Error::InvalidRequest(
                "stop the active turn before deleting a message".into(),
            ));
        }
        let message_id = required_payload_string(&command.payload, "message_id")?;
        if !self.store.control_delete_message(thread_id, &message_id)?
            && !self.is_stream_placeholder_for_thread(thread_id, &message_id)?
        {
            return Err(Error::NotFound(format!("message {message_id}")));
        }
        self.persist_and_emit(
            thread_id,
            None,
            "message_deleted",
            json!({ "message_id": message_id }),
        )?;
        Ok(ControlCommandResultV1 {
            command_id: command.command_id.clone(),
            status: ControlCommandStatusV1::Applied,
            thread_id: Some(thread_id.to_string()),
            revision: self
                .store
                .control_thread(thread_id)?
                .map(|thread| thread.revision),
            run_id: None,
            queue_id: None,
            confirmation_token: None,
            message: None,
            data: Value::Null,
        })
    }
}

fn settings_object(object: &mut Map<String, Value>) -> Result<&mut Map<String, Value>> {
    if !object.contains_key("settings") {
        object.insert("settings".into(), json!({}));
    }
    object
        .get_mut("settings")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::Other("stored thread settings are not an object".into()))
}
