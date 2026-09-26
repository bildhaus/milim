//! Account runtime native sessions: which model runs on which adapter, and the
//! per-account session and sync-cursor bindings stored on a thread.

use milim_control_contract::FrozenRunConfigV1;
use milim_core::{Error, Result};
use milim_storage::UserDataStore;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{now_ms, RunManager};

pub(super) const NATIVE_SESSION_FULL_TRANSCRIPT_CURSOR: &str = "__milim_hot_swap_full__";

impl RunManager {
    fn preallocate_claude_session(&self, thread_id: &str, profile_id: &str) -> Result<String> {
        let session_id = Uuid::new_v4().to_string();
        if let Some(updated) = self.store.control_compare_and_set_runtime_session(
            thread_id,
            &runtime_session_field("claude", profile_id)?,
            None,
            Some(&session_id),
            None,
        )? {
            self.emit_thread_changed(&updated, "thread.updated");
            return Ok(session_id);
        }
        current_runtime_session(&self.store, thread_id, "claude", profile_id)?.ok_or_else(|| {
            Error::Other("Claude session binding changed but is no longer available".into())
        })
    }

    pub(super) fn refresh_native_session_for_start(
        &self,
        thread_id: &str,
        config: &mut FrozenRunConfigV1,
    ) -> Result<()> {
        if runtime_session_field_base(&config.adapter).is_err() {
            return Ok(());
        }
        let profile_id = config.account_profile_id.clone();
        config.native_session_id =
            current_runtime_session(&self.store, thread_id, &config.adapter, &profile_id)?;
        config.native_session_cursor =
            current_runtime_cursor(&self.store, thread_id, &config.adapter, &profile_id)?;
        if config.adapter == "claude" && config.native_session_id.is_none() {
            config.native_session_id =
                Some(self.preallocate_claude_session(thread_id, &profile_id)?);
            config.native_session_cursor = Some(NATIVE_SESSION_FULL_TRANSCRIPT_CURSOR.into());
        }
        Ok(())
    }

    pub(super) fn persist_native_session_binding(
        &self,
        thread_id: &str,
        run_id: &str,
        adapter: &str,
        profile_id: &str,
        expected_session_id: Option<&str>,
        native_session_id: &str,
    ) -> Result<Option<String>> {
        let native_session_id = native_session_id.trim();
        if native_session_id.is_empty() {
            return Ok(expected_session_id.map(str::to_string));
        }
        if let Some(mut run) = self.store.control_run(run_id)? {
            run.native_session_json = Some(json!({ "id": native_session_id }).to_string());
            run.updated_at_ms = now_ms();
            self.store.control_put_run(&run)?;
        }
        if expected_session_id == Some(native_session_id) {
            return Ok(Some(native_session_id.to_string()));
        }
        if let Some(updated) = self.store.control_compare_and_set_runtime_session(
            thread_id,
            &runtime_session_field(adapter, profile_id)?,
            expected_session_id,
            Some(native_session_id),
            None,
        )? {
            self.emit_thread_changed(&updated, "thread.updated");
            return Ok(Some(native_session_id.to_string()));
        }
        current_runtime_session(&self.store, thread_id, adapter, profile_id)
    }

    pub(super) fn clear_native_session_binding(
        &self,
        thread_id: &str,
        adapter: &str,
        profile_id: &str,
        expected_session_id: &str,
    ) -> Result<Option<String>> {
        if let Some(updated) = self.store.control_compare_and_set_runtime_session(
            thread_id,
            &runtime_session_field(adapter, profile_id)?,
            Some(expected_session_id),
            None,
            None,
        )? {
            self.emit_thread_changed(&updated, "thread.updated");
            return Ok(None);
        }
        current_runtime_session(&self.store, thread_id, adapter, profile_id)
    }

    pub(super) fn persist_native_session_cursor(
        &self,
        thread_id: &str,
        adapter: &str,
        profile_id: &str,
        native_session_id: &str,
        message_id: &str,
    ) -> Result<()> {
        if let Some(updated) = self.store.control_compare_and_set_runtime_session(
            thread_id,
            &runtime_session_field(adapter, profile_id)?,
            Some(native_session_id),
            Some(native_session_id),
            Some(message_id),
        )? {
            self.emit_thread_changed(&updated, "thread.updated");
        }
        Ok(())
    }
}

pub(super) fn runtime_adapter(model: &str) -> &str {
    let model = model.trim();
    if model.eq_ignore_ascii_case("mock-echo") {
        "mock"
    } else if model
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("codex:"))
    {
        "codex"
    } else if model
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("claude:"))
    {
        "claude"
    } else if model
        .get(..9)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("opencode:"))
    {
        "opencode"
    } else if model
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("pi:"))
    {
        "pi"
    } else {
        "provider"
    }
}

fn runtime_session_field_base(adapter: &str) -> Result<&'static str> {
    match adapter {
        "codex" => Ok("codexThreadId"),
        "claude" => Ok("claudeSessionId"),
        "opencode" => Ok("opencodeSessionId"),
        "pi" => Ok("piSessionId"),
        _ => Err(Error::InvalidRequest(format!(
            "runtime adapter {adapter} does not own a native session"
        ))),
    }
}

fn runtime_cursor_field_base(adapter: &str) -> Result<&'static str> {
    match adapter {
        "codex" => Ok("codexLastSyncedMessageId"),
        "claude" => Ok("claudeLastSyncedMessageId"),
        "opencode" => Ok("opencodeLastSyncedMessageId"),
        "pi" => Ok("piLastSyncedMessageId"),
        _ => Err(Error::InvalidRequest(format!(
            "runtime adapter {adapter} does not own a native session cursor"
        ))),
    }
}

/// A native session lives inside one account's configuration home, so a thread
/// holds one binding per account rather than one per runtime. The default
/// account keeps the unsuffixed field, so threads that predate account
/// profiles resume exactly as before.
fn scoped_binding_field(base: &str, profile_id: &str) -> String {
    if profile_id.is_empty() || profile_id == crate::account_profiles::DEFAULT_PROFILE_ID {
        base.to_string()
    } else {
        format!("{base}:{profile_id}")
    }
}

pub(super) fn runtime_session_field(adapter: &str, profile_id: &str) -> Result<String> {
    Ok(scoped_binding_field(
        runtime_session_field_base(adapter)?,
        profile_id,
    ))
}

pub(super) fn runtime_cursor_field(adapter: &str, profile_id: &str) -> Result<String> {
    Ok(scoped_binding_field(
        runtime_cursor_field_base(adapter)?,
        profile_id,
    ))
}

pub(super) fn current_runtime_session(
    store: &UserDataStore,
    thread_id: &str,
    adapter: &str,
    profile_id: &str,
) -> Result<Option<String>> {
    let Some(thread) = store.control_thread(thread_id)? else {
        return Err(Error::NotFound(format!("thread {thread_id}")));
    };
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    Ok(value
        .get("accountRuntime")
        .and_then(Value::as_object)
        .and_then(|runtime| runtime.get(&runtime_session_field(adapter, profile_id).ok()?))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn current_runtime_cursor(
    store: &UserDataStore,
    thread_id: &str,
    adapter: &str,
    profile_id: &str,
) -> Result<Option<String>> {
    let Some(thread) = store.control_thread(thread_id)? else {
        return Err(Error::NotFound(format!("thread {thread_id}")));
    };
    let value: Value = serde_json::from_str(&thread.session_json)
        .map_err(|error| Error::Other(format!("invalid stored thread JSON: {error}")))?;
    Ok(value
        .get("accountRuntime")
        .and_then(Value::as_object)
        .and_then(|runtime| runtime.get(&runtime_cursor_field(adapter, profile_id).ok()?))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

pub(super) fn runtime_model(model: &str) -> &str {
    match runtime_adapter(model) {
        "codex" => model.get(6..).unwrap_or(model).trim(),
        "claude" => model.get(7..).unwrap_or(model).trim(),
        "opencode" => model.get(9..).unwrap_or(model).trim(),
        "pi" => model.get(3..).unwrap_or(model).trim(),
        _ => model.trim(),
    }
}
