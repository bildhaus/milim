//! Canonical desktop/mobile control protocol and server-owned run lifecycle.
//!
//! `/control/v1` is deliberately separate from the legacy child-thread API.
//! The durable user session tables remain authoritative; this module adds
//! sequencing, command idempotency, queues, and live replication around them.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use milim_core::{Error, Result};
use milim_storage::{ControlHostRecord, UserDataStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, watch, Mutex as AsyncMutex};
use uuid::Uuid;

pub use milim_control_contract::*;

mod agent;
mod approvals;
mod attachments;
mod checkpoints;
mod client_state;
mod commands;
mod delta;
mod events;
mod harness;
mod journal;
mod linked_threads;
mod metrics;
mod native_sessions;
mod preview_runtime;
mod provider;
mod queue;
mod replay;
mod run_config;
mod threads;
mod turns;
mod views;

pub(crate) use replay::{completion_request_from_value, completion_request_value};

const CONTROL_EVENT_CAPACITY: usize = 1_024;
pub(crate) const MAX_LINKED_THREAD_WAIT_MS: u64 = 90_000;
pub const MODEL_FAVORITES_SETTINGS_KEY: &str = "milim.settings";
pub const MODEL_FAVORITES_EVENT_TYPE: &str = "model_favorites.updated";
pub const MODEL_CATALOG_STATE_KEY: &str = "milim.modelCatalog";

// Wire declarations live in `milim-control-contract`. Keeping the former
// declarations compiled out for this cutover makes the ownership move easy to
// audit while all server call sites use the canonical crate above.
#[cfg(any())]
mod legacy_control_contract;

pub(crate) struct AppearanceBackgroundAsset {
    pub revision: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize)]
struct TurnSendPayloadV1 {
    text: String,
    #[serde(default)]
    client_message_id: Option<String>,
    #[serde(default)]
    display_text: Option<String>,
    #[serde(default)]
    attachments: Vec<ControlAttachmentV1>,
    #[serde(default)]
    preview_runtime: Option<ManagedPreviewRuntimeV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ManagedPreviewRuntimeV1 {
    kind: String,
    status: String,
    active: bool,
    ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AcceptedTurnV1 {
    text: String,
    #[serde(default)]
    client_message_id: Option<String>,
    #[serde(default)]
    display_text: Option<String>,
    config: FrozenRunConfigV1,
    #[serde(default = "control_default_true")]
    append_user: bool,
    #[serde(default)]
    mailbox_origin: Option<MailboxOriginV1>,
    #[serde(default)]
    mailbox_context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    preview_runtime: Option<ManagedPreviewRuntimeV1>,
}

struct ActiveRun {
    run_id: String,
    steering: bool,
    stop: watch::Sender<bool>,
}

struct ConfirmationGrant {
    token: String,
    expires_at: Instant,
}

#[derive(Clone)]
struct PendingAttachmentUpload {
    upload_id: String,
    client_attachment_id: String,
    device_id: String,
    name: String,
    mime: String,
    bytes: Vec<u8>,
    expires_at: Instant,
    expires_at_ms: i64,
}

#[derive(Clone)]
pub(crate) struct SocketTicket {
    pub device_key: Option<String>,
    pub expires_at: Instant,
}

pub struct RunManager {
    store: Arc<UserDataStore>,
    restore_admission: Arc<tokio::sync::RwLock<bool>>,
    host: RwLock<ControlHostRecord>,
    active: Mutex<HashMap<String, ActiveRun>>,
    queue_interrupts: Mutex<HashMap<String, String>>,
    command_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    thread_locks: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
    confirmations: Mutex<HashMap<String, ConfirmationGrant>>,
    socket_tickets: Mutex<HashMap<String, SocketTicket>>,
    attachment_uploads: Mutex<HashMap<String, PendingAttachmentUpload>>,
    /// Workspace checkpoints taken before active runs, keyed by run id, so
    /// the final assistant message can carry its undo point.
    turn_checkpoints: Mutex<HashMap<String, Value>>,
    events: broadcast::Sender<ControlEventV1>,
}

/// Exclusive restore admission. A failed restore releases the fence; a successful
/// restore keeps mutation admission closed until the desktop process restarts.
pub struct RestoreGuard {
    guard: tokio::sync::OwnedRwLockWriteGuard<bool>,
    committed: bool,
}

impl RestoreGuard {
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        if !self.committed {
            *self.guard = false;
        }
    }
}

impl RunManager {
    pub fn new(store: Arc<UserDataStore>, display_name: impl AsRef<str>) -> Result<Arc<Self>> {
        let host = store
            .ensure_control_host(&format!("host-{}", Uuid::new_v4()), display_name.as_ref())?;
        store.reconcile_control_startup()?;
        let (events, _) = broadcast::channel(CONTROL_EVENT_CAPACITY);
        let manager = Arc::new(Self {
            store,
            restore_admission: Arc::new(tokio::sync::RwLock::new(false)),
            host: RwLock::new(host),
            active: Mutex::new(HashMap::new()),
            queue_interrupts: Mutex::new(HashMap::new()),
            command_locks: Mutex::new(HashMap::new()),
            thread_locks: Mutex::new(HashMap::new()),
            confirmations: Mutex::new(HashMap::new()),
            socket_tickets: Mutex::new(HashMap::new()),
            attachment_uploads: Mutex::new(HashMap::new()),
            turn_checkpoints: Mutex::new(HashMap::new()),
            events,
        });
        manager.backfill_message_timelines()?;
        manager.reconcile_mailbox_projections()?;
        Ok(manager)
    }

    fn backfill_message_timelines(&self) -> Result<usize> {
        let mut seeded = 0;
        for thread in self.store.control_threads_missing_message_timeline()? {
            let session: Value = serde_json::from_str(&thread.session_json).unwrap_or(Value::Null);
            let base_timestamp = session
                .get("createdAt")
                .or_else(|| session.get("created_at_ms"))
                .and_then(Value::as_i64)
                .unwrap_or(thread.updated_at_ms);
            let messages = self
                .store
                .control_messages(&thread.id)?
                .into_iter()
                .enumerate()
                .filter_map(|(index, raw)| {
                    history_timeline_message(&thread.id, index, &raw, base_timestamp)
                })
                .collect::<Vec<_>>();
            seeded += self
                .store
                .control_seed_message_timeline_if_empty(&thread.id, &messages)?;
        }
        Ok(seeded)
    }

    /// Shared admission for commands and desktop replica writes. Never waits
    /// behind a restore, so stale clients cannot resume writing after replacement.
    pub fn mutation_guard(&self) -> Result<tokio::sync::OwnedRwLockReadGuard<bool>> {
        let guard = self
            .restore_admission
            .clone()
            .try_read_owned()
            .map_err(|_| {
                Error::InvalidRequest(
                    "Backup restore is in progress. Try again after milim restarts.".into(),
                )
            })?;
        if *guard {
            return Err(Error::InvalidRequest(
                "Backup restored. Restart milim before making more changes.".into(),
            ));
        }
        Ok(guard)
    }

    /// Which account of `adapter` a thread is set to use, resolved through
    /// Auto. Managed Workers read it so a delegated run bills the same
    /// subscription as the parent turn that asked for it.
    pub fn thread_account_profile(&self, thread_id: &str, adapter: &str) -> String {
        let selected = self
            .store
            .control_thread(thread_id)
            .ok()
            .flatten()
            .and_then(|thread| serde_json::from_str::<Value>(&thread.session_json).ok())
            .and_then(|value| {
                value
                    .get("settings")?
                    .get("accountProfiles")?
                    .get(adapter)?
                    .as_str()
                    .map(str::to_string)
            });
        crate::account_profiles::resolve(Some(&self.store), adapter, selected.as_deref()).id
    }

    /// Canonical user-data store. Surfaces that persist their own settings
    /// beside canonical thread state (account profiles) read it through here.
    pub fn store(&self) -> &Arc<UserDataStore> {
        &self.store
    }

    pub fn begin_restore(&self) -> Result<RestoreGuard> {
        let mut guard = self
            .restore_admission
            .clone()
            .try_write_owned()
            .map_err(|_| {
                Error::InvalidRequest(
                    "milim is processing a change. Wait for it to finish, then restore again."
                        .into(),
                )
            })?;
        if *guard {
            return Err(Error::InvalidRequest(
                "Backup restored. Restart milim before restoring again.".into(),
            ));
        }
        if !self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            || !self.store.control_runs(true)?.is_empty()
            || !self.store.control_queued_turns(None)?.is_empty()
            || !self.store.control_pending_inbox(None)?.is_empty()
        {
            return Err(Error::InvalidRequest(
                "Finish or stop active runs and clear queued messages before restoring a backup."
                    .into(),
            ));
        }
        *guard = true;
        Ok(RestoreGuard {
            guard,
            committed: false,
        })
    }

    pub fn host(&self) -> ControlHostRecord {
        self.host
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn refresh_host(&self) -> Result<ControlHostRecord> {
        let current = self.host();
        let restored = self
            .store
            .ensure_control_host(&current.host_id, &current.display_name)?;
        *self
            .host
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = restored.clone();
        Ok(restored)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ControlEventV1> {
        self.events.subscribe()
    }

    pub fn issue_socket_ticket(&self, device_key: Option<String>) -> (String, u64) {
        let ticket = Uuid::new_v4().to_string();
        let ttl = Duration::from_secs(30);
        let mut tickets = self
            .socket_tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tickets.retain(|_, value| value.expires_at > Instant::now());
        tickets.insert(
            ticket.clone(),
            SocketTicket {
                device_key,
                expires_at: Instant::now() + ttl,
            },
        );
        (ticket, ttl.as_secs())
    }

    pub(crate) fn take_socket_ticket(&self, ticket: &str) -> Option<SocketTicket> {
        let mut tickets = self
            .socket_tickets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tickets.retain(|_, value| value.expires_at > Instant::now());
        tickets.remove(ticket)
    }

    pub async fn shutdown(&self, timeout: Duration) {
        self.queue_interrupts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        {
            let active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for run in active.values() {
                let _ = run.stop.send(true);
            }
        }
        let wait = async {
            loop {
                if self
                    .active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        if tokio::time::timeout(timeout, wait).await.is_err() {
            let _ = self.store.reconcile_control_startup();
        }
    }
}

#[derive(Clone, Copy)]
enum ThreadPatch {
    Rename,
    Archive,
    Model,
    Agent,
    Execution,
    AccountProfile,
}

enum RunOutcome {
    Completed,
    Limited,
    Cancelled,
}

fn history_timeline_message(
    thread_id: &str,
    index: usize,
    raw: &str,
    base_timestamp: i64,
) -> Option<(String, String, i64)> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    if value.get("modelChange").is_some() {
        return None;
    }
    let role = value.get("role")?.as_str()?.to_string();
    if !matches!(role.as_str(), "user" | "assistant" | "system") {
        return None;
    }
    let message_id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("legacy-{thread_id}-{index}"));
    let stream_parts = value
        .get("streamParts")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .filter(|content| !content.is_empty())
        .or_else(|| value.get("promptContent").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| history_stream_text(stream_parts, "text"));
    let reasoning = value
        .get("reasoning")
        .or_else(|| value.get("reasoningContent"))
        .or_else(|| value.get("reasoning_content"))
        .and_then(Value::as_str)
        .filter(|reasoning| !reasoning.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| history_stream_text(stream_parts, "thinking"));
    let created_at_ms = value
        .get("createdAt")
        .or_else(|| value.get("created_at_ms"))
        .or_else(|| value.get("timestamp"))
        .and_then(Value::as_i64)
        .or_else(|| {
            value
                .get("metrics")
                .and_then(|metrics| metrics.get("startedAt"))
                .and_then(Value::as_i64)
        })
        .or_else(|| {
            value
                .get("run")
                .and_then(|run| run.get("startedAt"))
                .and_then(Value::as_i64)
        })
        .unwrap_or_else(|| base_timestamp.saturating_add(index as i64));
    let object = value.as_object_mut()?;
    object.insert("id".into(), Value::String(message_id));
    object.insert("role".into(), Value::String(role));
    object.insert("content".into(), Value::String(content));
    object.insert("reasoning".into(), Value::String(reasoning));
    Some((
        format!("history:{thread_id}:{index}"),
        Value::Object(object.clone()).to_string(),
        created_at_ms,
    ))
}

fn history_stream_text(parts: Option<&[Value]>, kind: &str) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some(kind))
        .filter_map(|part| part.get("content").and_then(Value::as_str))
        .collect::<String>()
}

fn parse_value(value: &str) -> Result<Value> {
    serde_json::from_str(value)
        .map_err(|error| Error::Other(format!("invalid stored control JSON: {error}")))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

fn control_default_true() -> bool {
    true
}

#[cfg(test)]
mod tests;
