use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use uuid::Uuid;

use milim_storage::EncryptedStore;
use milim_tools::atomic_write;

const PAIRING_TTL_SECS: u64 = 10 * 60;
const MAX_DEVICE_NAME_CHARS: usize = 60;
const MAX_RELAY_TEXT_BYTES: usize = 20 * 1024;
const MAX_RELAY_ATTACHMENTS: usize = 6;
const MAX_RELAY_ATTACHMENT_NAME_CHARS: usize = 140;
const MAX_RELAY_ATTACHMENT_MIME_CHARS: usize = 120;
const MAX_RELAY_ATTACHMENT_CONTENT_CHARS: usize = 128 * 1024;
const MAX_RELAY_ATTACHMENT_DATA_URL_CHARS: usize = 3 * 1024 * 1024;
const MAX_EVENTS: usize = 200;
const MAX_THREAD_MESSAGES: usize = 160;
const MAX_THREAD_MESSAGE_CHARS: usize = 40_000;
const MAX_THREAD_SUMMARIES: usize = 80;
const MAX_THREAD_GROUPS: usize = 80;
const MAX_THREAD_MODELS: usize = 120;
const MAX_THREAD_TITLE_CHARS: usize = 120;
const MAX_THREAD_MODEL_CHARS: usize = 120;
const MAX_THREAD_PROJECT_CHARS: usize = 240;
const MAX_THEME_CSS_VARS: usize = 80;
const MAX_THEME_CSS_KEY_CHARS: usize = 80;
const MAX_THEME_CSS_VALUE_CHARS: usize = 3 * 1024 * 1024;
const MAX_THEME_BACKGROUND_MODE_CHARS: usize = 20;

#[derive(Clone)]
pub struct MobileCompanionBridge {
    inner: Arc<RwLock<MobileCompanionInner>>,
    persistence_path: Option<Arc<PathBuf>>,
    persistence_encryption: Option<EncryptedStore>,
    thread_updates: watch::Sender<u64>,
}

impl Default for MobileCompanionBridge {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MobileCompanionInner::default())),
            persistence_path: None,
            persistence_encryption: None,
            thread_updates: watch::channel(0).0,
        }
    }
}

#[derive(Default)]
struct MobileCompanionInner {
    enabled: bool,
    pairing: Option<MobilePairing>,
    devices: Vec<MobileDevice>,
    events: VecDeque<MobileRelayEvent>,
    next_event_id: u64,
    thread: Option<MobileThreadSnapshot>,
    next_thread_version: u64,
}

#[derive(Default, Deserialize, Serialize)]
struct MobileCompanionPersisted {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    devices: Vec<MobileDevice>,
}

#[derive(Clone, Debug)]
struct MobilePairing {
    id: String,
    secret: String,
    expires_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct MobileDevice {
    id: String,
    name: String,
    key: String,
    paired_at: u64,
    last_seen_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileCompanionStatus {
    pub enabled: bool,
    pub pairing: Option<MobilePairingInfo>,
    pub devices: Vec<MobileDeviceInfo>,
    pub queued_events: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairingInfo {
    pub id: String,
    pub expires_at: u64,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileDeviceInfo {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub paired_at: u64,
    pub last_seen_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MobilePairRequest {
    pub pair_id: String,
    pub secret: String,
    pub device_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairResponse {
    pub device_id: String,
    pub device_key: String,
    pub device_name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MobileRelayAction {
    Append,
    Replace,
    Send,
    SwitchThread,
    NewThread,
    Stop,
    Regenerate,
    DeleteMessage,
    RenameThread,
    ArchiveThread,
    DeleteThread,
    SetModel,
    Attach,
    WorkerRunStart,
    WorkerRunContinueSolo,
    WorkerRunStop,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MobileRelayRequest {
    #[serde(default)]
    pub text: String,
    #[serde(default = "default_relay_action")]
    pub action: MobileRelayAction,
    #[serde(default)]
    pub attachments: Vec<MobileRelayAttachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileRelayAttachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default, rename = "dataUrl")]
    pub data_url: Option<String>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileRelayEvent {
    pub id: u64,
    pub device_id: String,
    pub device_name: String,
    pub text: String,
    pub action: MobileRelayAction,
    pub attachments: Vec<MobileRelayAttachment>,
    pub received_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MobileThreadUpdateRequest {
    pub session_id: String,
    pub title: String,
    pub model: Option<String>,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub messages: Vec<MobileThreadMessage>,
    #[serde(default)]
    pub threads: Vec<MobileThreadSummary>,
    #[serde(default)]
    pub groups: Vec<MobileThreadGroup>,
    #[serde(default)]
    pub models: Vec<MobileModelSummary>,
    #[serde(default)]
    pub theme: Option<MobileThemeSnapshot>,
    #[serde(default)]
    pub worker_run: Option<MobileWorkerRunSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileThreadMessage {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobileThreadSnapshot {
    pub version: u64,
    pub session_id: String,
    pub title: String,
    pub model: Option<String>,
    pub busy: bool,
    pub updated_at: u64,
    pub messages: Vec<MobileThreadMessage>,
    pub threads: Vec<MobileThreadSummary>,
    pub groups: Vec<MobileThreadGroup>,
    pub models: Vec<MobileModelSummary>,
    pub theme: Option<MobileThemeSnapshot>,
    pub worker_run: Option<MobileWorkerRunSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileWorkerRunSnapshot {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub tasks: Vec<MobileWorkerTaskSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileWorkerTaskSnapshot {
    pub title: String,
    pub model: String,
    pub access: String,
    pub status: String,
    #[serde(default)]
    pub result: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileThemeSnapshot {
    pub is_dark: bool,
    #[serde(default)]
    pub css_vars: BTreeMap<String, String>,
    #[serde(default)]
    pub background_fit: Option<String>,
    #[serde(default)]
    pub background_treatment: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileThreadSummary {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub updated_at: u64,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub project_label: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileThreadGroup {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub threads: Vec<MobileThreadSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MobileModelSummary {
    pub id: String,
    #[serde(default)]
    pub provider: Option<String>,
}

fn default_relay_action() -> MobileRelayAction {
    MobileRelayAction::Append
}

fn relay_requires_content(action: MobileRelayAction) -> bool {
    matches!(
        action,
        MobileRelayAction::Append
            | MobileRelayAction::Replace
            | MobileRelayAction::Send
            | MobileRelayAction::SwitchThread
            | MobileRelayAction::DeleteMessage
            | MobileRelayAction::RenameThread
            | MobileRelayAction::SetModel
            | MobileRelayAction::Attach
    )
}

impl MobileCompanionBridge {
    pub fn persistent(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let inner = match MobileCompanionInner::load_persisted(&path) {
            Ok(inner) => inner,
            Err(err) => {
                tracing::warn!(
                    target: "milim_desktop::server",
                    "mobile companion persistence unavailable: {err}"
                );
                MobileCompanionInner::default()
            }
        };
        Self {
            inner: Arc::new(RwLock::new(inner)),
            persistence_path: Some(Arc::new(path)),
            persistence_encryption: None,
            thread_updates: watch::channel(0).0,
        }
    }

    pub fn persistent_encrypted(
        path: impl Into<PathBuf>,
        encryption: EncryptedStore,
    ) -> Result<Self, String> {
        let path = path.into();
        let inner = MobileCompanionInner::load_encrypted(&path, &encryption)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            persistence_path: Some(Arc::new(path)),
            persistence_encryption: Some(encryption),
            thread_updates: watch::channel(0).0,
        })
    }

    pub fn status(&self, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.expire_pairing(now);
        inner.status()
    }

    pub fn set_enabled(&self, enabled: bool, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.enabled = enabled;
        if !enabled {
            inner.pairing = None;
            inner.events.clear();
            inner.thread = None;
            inner.next_thread_version = inner.next_thread_version.saturating_add(1).max(1);
            let _ = self.thread_updates.send(inner.next_thread_version);
        }
        inner.expire_pairing(now);
        self.persist_inner(&inner);
        inner.status()
    }

    pub fn start_pairing(&self, now: u64) -> Result<MobilePairingInfo, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        let pairing = MobilePairing {
            id: format!("pair-{}", short_id()),
            secret: secret_key("pair"),
            expires_at: now.saturating_add(PAIRING_TTL_SECS),
        };
        let info = pairing.info();
        inner.pairing = Some(pairing);
        Ok(info)
    }

    pub fn pair_device(
        &self,
        req: MobilePairRequest,
        now: u64,
        user_agent: Option<&str>,
    ) -> Result<MobilePairResponse, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        inner.expire_pairing(now);
        let pairing = inner
            .pairing
            .as_ref()
            .ok_or_else(|| "pairing session expired or missing".to_string())?;
        if pairing.id != req.pair_id || pairing.secret != req.secret {
            return Err("invalid pairing token".to_string());
        }

        let fallback = user_agent
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or("Phone");
        let name = clean_device_name(req.device_name.as_deref().unwrap_or(fallback));
        let device = MobileDevice {
            id: format!("device-{}", short_id()),
            name,
            key: secret_key("mobile"),
            paired_at: now,
            last_seen_at: Some(now),
        };
        let response = MobilePairResponse {
            device_id: device.id.clone(),
            device_key: device.key.clone(),
            device_name: device.name.clone(),
        };
        inner.devices.push(device);
        self.persist_inner(&inner);
        Ok(response)
    }

    pub fn revoke_device(&self, id: &str, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.devices.retain(|device| device.id != id);
        inner.events.retain(|event| event.device_id != id);
        inner.expire_pairing(now);
        self.persist_inner(&inner);
        inner.status()
    }

    pub fn authenticate_device(&self, key: &str, now: u64) -> Option<MobileDeviceInfo> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return None;
        }
        let device = inner.devices.iter_mut().find(|device| device.key == key)?;
        device.last_seen_at = Some(now);
        Some(device.info())
    }

    pub fn submit_relay(
        &self,
        device_key: &str,
        request: MobileRelayRequest,
        now: u64,
    ) -> Result<MobileRelayEvent, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        let text = request.text.trim().to_string();
        if text.len() > MAX_RELAY_TEXT_BYTES {
            return Err(format!(
                "relay text is too large; limit is {MAX_RELAY_TEXT_BYTES} bytes"
            ));
        }
        let attachments = request
            .attachments
            .into_iter()
            .take(MAX_RELAY_ATTACHMENTS)
            .filter_map(clean_relay_attachment)
            .collect::<Vec<_>>();
        if relay_requires_content(request.action) && text.is_empty() && attachments.is_empty() {
            return Err("relay text or attachment is required".to_string());
        }
        let device_index = inner
            .devices
            .iter()
            .position(|device| device.key == device_key)
            .ok_or_else(|| "invalid device key".to_string())?;
        inner.devices[device_index].last_seen_at = Some(now);
        let device_id = inner.devices[device_index].id.clone();
        let device_name = inner.devices[device_index].name.clone();
        inner.next_event_id = inner.next_event_id.saturating_add(1).max(1);
        let event = MobileRelayEvent {
            id: inner.next_event_id,
            device_id,
            device_name,
            text,
            action: request.action,
            attachments,
            received_at: now,
        };
        inner.events.push_back(event.clone());
        while inner.events.len() > MAX_EVENTS {
            inner.events.pop_front();
        }
        self.persist_inner(&inner);
        Ok(event)
    }

    pub fn take_events(&self) -> Vec<MobileRelayEvent> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.events.drain(..).collect()
    }

    pub fn update_thread(
        &self,
        request: MobileThreadUpdateRequest,
        now: u64,
    ) -> Option<MobileThreadSnapshot> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            inner.thread = None;
            return None;
        }
        inner.next_thread_version = inner.next_thread_version.saturating_add(1).max(1);
        let snapshot = MobileThreadSnapshot {
            version: inner.next_thread_version,
            session_id: clean_limited(&request.session_id, 160),
            title: clean_limited(&request.title, MAX_THREAD_TITLE_CHARS),
            model: request
                .model
                .as_deref()
                .map(|value| clean_limited(value, MAX_THREAD_MODEL_CHARS))
                .filter(|value| !value.is_empty()),
            busy: request.busy,
            updated_at: now,
            messages: request
                .messages
                .into_iter()
                .rev()
                .take(MAX_THREAD_MESSAGES)
                .filter_map(clean_thread_message)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            threads: request
                .threads
                .into_iter()
                .take(MAX_THREAD_SUMMARIES)
                .filter_map(clean_thread_summary)
                .collect(),
            groups: request
                .groups
                .into_iter()
                .take(MAX_THREAD_GROUPS)
                .filter_map(clean_thread_group)
                .collect(),
            models: request
                .models
                .into_iter()
                .take(MAX_THREAD_MODELS)
                .filter_map(clean_model_summary)
                .collect(),
            theme: request.theme.map(clean_mobile_theme),
            worker_run: request.worker_run.map(clean_mobile_worker_run),
        };
        inner.thread = Some(snapshot.clone());
        let _ = self.thread_updates.send(snapshot.version);
        Some(snapshot)
    }

    pub fn thread_for_device(
        &self,
        device_key: &str,
        now: u64,
    ) -> Result<Option<MobileThreadSnapshot>, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        let device = inner
            .devices
            .iter_mut()
            .find(|device| device.key == device_key)
            .ok_or_else(|| "invalid device key".to_string())?;
        device.last_seen_at = Some(now);
        Ok(inner.thread.clone())
    }

    pub fn subscribe_thread(&self) -> watch::Receiver<u64> {
        self.thread_updates.subscribe()
    }

    fn persist_inner(&self, inner: &MobileCompanionInner) {
        let Some(path) = self.persistence_path.as_deref() else {
            return;
        };
        if let Err(err) = write_mobile_companion_persistence(
            path,
            &inner.persisted(),
            self.persistence_encryption.as_ref(),
        ) {
            tracing::warn!(
                target: "milim_desktop::server",
                "mobile companion persistence write failed: {err}"
            );
        }
    }
}

impl MobileCompanionInner {
    fn load_persisted(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let data =
            fs::read(path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let persisted: MobileCompanionPersisted = serde_json::from_slice(&data)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
        Ok(Self {
            enabled: persisted.enabled,
            devices: persisted.devices,
            ..Self::default()
        })
    }

    fn load_encrypted(path: &Path, encryption: &EncryptedStore) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let encrypted =
            fs::read(path).map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let data = encryption
            .decrypt(&encrypted)
            .map_err(|err| format!("failed to decrypt {}: {err}", path.display()))?;
        let persisted = parse_mobile_companion_persistence(&data)
            .map_err(|err| format!("failed to parse {}: {err}", path.display()))?;
        Ok(Self {
            enabled: persisted.enabled,
            devices: persisted.devices,
            ..Self::default()
        })
    }

    fn persisted(&self) -> MobileCompanionPersisted {
        MobileCompanionPersisted {
            enabled: self.enabled,
            devices: self.devices.clone(),
        }
    }

    fn status(&self) -> MobileCompanionStatus {
        MobileCompanionStatus {
            enabled: self.enabled,
            pairing: self.pairing.as_ref().map(MobilePairing::info),
            devices: self.devices.iter().map(MobileDevice::info).collect(),
            queued_events: self.events.len(),
        }
    }

    fn expire_pairing(&mut self, now: u64) {
        if self
            .pairing
            .as_ref()
            .is_some_and(|pairing| pairing.expires_at <= now)
        {
            self.pairing = None;
        }
    }
}

impl MobilePairing {
    fn info(&self) -> MobilePairingInfo {
        MobilePairingInfo {
            id: self.id.clone(),
            expires_at: self.expires_at,
            path: format!("/mobile?pair_id={}&secret={}", self.id, self.secret),
        }
    }
}

impl MobileDevice {
    fn info(&self) -> MobileDeviceInfo {
        MobileDeviceInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            key_prefix: self.key.chars().take(14).collect(),
            paired_at: self.paired_at,
            last_seen_at: self.last_seen_at,
        }
    }
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn secret_key(prefix: &str) -> String {
    format!(
        "{prefix}-{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn clean_device_name(input: &str) -> String {
    let trimmed = clean_limited(input, MAX_DEVICE_NAME_CHARS);
    if trimmed.is_empty() {
        return "Phone".to_string();
    }
    trimmed
}

fn clean_thread_message(message: MobileThreadMessage) -> Option<MobileThreadMessage> {
    let role = match message.role.as_str() {
        "user" | "assistant" | "system" => message.role,
        _ => return None,
    };
    let content = trim_limited(&message.content, MAX_THREAD_MESSAGE_CHARS);
    if content.is_empty() {
        return None;
    }
    Some(MobileThreadMessage { role, content })
}

fn clean_relay_attachment(attachment: MobileRelayAttachment) -> Option<MobileRelayAttachment> {
    let name = clean_limited(&attachment.name, MAX_RELAY_ATTACHMENT_NAME_CHARS);
    if name.is_empty() {
        return None;
    }
    let mime = clean_limited(&attachment.mime, MAX_RELAY_ATTACHMENT_MIME_CHARS);
    let content = attachment
        .content
        .as_deref()
        .map(|value| trim_limited(value, MAX_RELAY_ATTACHMENT_CONTENT_CHARS))
        .filter(|value| !value.is_empty());
    let data_url = attachment
        .data_url
        .as_deref()
        .map(|value| trim_limited(value, MAX_RELAY_ATTACHMENT_DATA_URL_CHARS))
        .filter(|value| value.starts_with("data:image/"));
    Some(MobileRelayAttachment {
        id: clean_limited(&attachment.id, 80),
        name,
        mime,
        size: attachment.size,
        content,
        data_url,
        truncated: attachment.truncated,
    })
}

fn clean_thread_summary(summary: MobileThreadSummary) -> Option<MobileThreadSummary> {
    let id = clean_limited(&summary.id, 160);
    if id.is_empty() {
        return None;
    }
    Some(MobileThreadSummary {
        id,
        title: clean_limited(&summary.title, MAX_THREAD_TITLE_CHARS),
        model: summary
            .model
            .as_deref()
            .map(|value| clean_limited(value, MAX_THREAD_MODEL_CHARS))
            .filter(|value| !value.is_empty()),
        updated_at: summary.updated_at,
        busy: summary.busy,
        parent_id: summary
            .parent_id
            .as_deref()
            .map(|value| clean_limited(value, 160))
            .filter(|value| !value.is_empty()),
        project_label: summary
            .project_label
            .as_deref()
            .map(|value| clean_limited(value, MAX_THREAD_TITLE_CHARS))
            .filter(|value| !value.is_empty()),
        project_path: summary
            .project_path
            .as_deref()
            .map(|value| clean_limited(value, MAX_THREAD_PROJECT_CHARS))
            .filter(|value| !value.is_empty()),
    })
}

fn clean_thread_group(group: MobileThreadGroup) -> Option<MobileThreadGroup> {
    let id = clean_limited(&group.id, 180);
    let label = clean_limited(&group.label, MAX_THREAD_TITLE_CHARS);
    if id.is_empty() || label.is_empty() {
        return None;
    }
    let threads = group
        .threads
        .into_iter()
        .take(MAX_THREAD_SUMMARIES)
        .filter_map(clean_thread_summary)
        .collect::<Vec<_>>();
    if threads.is_empty() {
        return None;
    }
    Some(MobileThreadGroup {
        id,
        label,
        subtitle: group
            .subtitle
            .as_deref()
            .map(|value| clean_limited(value, MAX_THREAD_PROJECT_CHARS))
            .filter(|value| !value.is_empty()),
        project_id: group
            .project_id
            .as_deref()
            .map(|value| clean_limited(value, 180))
            .filter(|value| !value.is_empty()),
        threads,
    })
}

fn clean_model_summary(model: MobileModelSummary) -> Option<MobileModelSummary> {
    let id = clean_limited(&model.id, MAX_THREAD_MODEL_CHARS);
    if id.is_empty() {
        return None;
    }
    Some(MobileModelSummary {
        id,
        provider: model
            .provider
            .as_deref()
            .map(|value| clean_limited(value, MAX_THREAD_MODEL_CHARS))
            .filter(|value| !value.is_empty()),
    })
}

fn clean_mobile_worker_run(mut run: MobileWorkerRunSnapshot) -> MobileWorkerRunSnapshot {
    run.id = clean_limited(&run.id, MAX_THREAD_MODEL_CHARS);
    run.status = clean_limited(&run.status, 40);
    run.tasks = run
        .tasks
        .into_iter()
        .take(4)
        .map(|mut task| {
            task.title = clean_limited(&task.title, MAX_THREAD_TITLE_CHARS);
            task.model = clean_limited(&task.model, MAX_THREAD_MODEL_CHARS);
            task.access = clean_limited(&task.access, 40);
            task.status = clean_limited(&task.status, 40);
            task.result = task
                .result
                .map(|value| trim_limited(&value, MAX_THREAD_MESSAGE_CHARS));
            task
        })
        .collect();
    run
}

fn clean_mobile_theme(theme: MobileThemeSnapshot) -> MobileThemeSnapshot {
    let css_vars = theme
        .css_vars
        .into_iter()
        .filter_map(|(key, value)| {
            let key = clean_limited(&key, MAX_THEME_CSS_KEY_CHARS);
            if !is_safe_theme_css_var_key(&key) {
                return None;
            }
            Some((key, trim_limited(&value, MAX_THEME_CSS_VALUE_CHARS)))
        })
        .take(MAX_THEME_CSS_VARS)
        .collect();
    MobileThemeSnapshot {
        is_dark: theme.is_dark,
        css_vars,
        background_fit: theme
            .background_fit
            .as_deref()
            .map(|value| clean_limited(value, MAX_THEME_BACKGROUND_MODE_CHARS))
            .filter(|value| matches!(value.as_str(), "cover" | "contain" | "tile" | "center")),
        background_treatment: theme
            .background_treatment
            .as_deref()
            .map(|value| clean_limited(value, MAX_THEME_BACKGROUND_MODE_CHARS))
            .filter(|value| matches!(value.as_str(), "clear" | "dim" | "blur" | "mono")),
    }
}

fn is_safe_theme_css_var_key(value: &str) -> bool {
    value.starts_with("--")
        && value.len() > 2
        && value[2..]
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

fn clean_limited(input: &str, max_chars: usize) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn trim_limited(input: &str, max_chars: usize) -> String {
    input.trim().chars().take(max_chars).collect()
}

pub fn validate_mobile_companion_persistence(data: &[u8]) -> Result<(), String> {
    parse_mobile_companion_persistence(data).map(|_| ())
}

fn parse_mobile_companion_persistence(data: &[u8]) -> Result<MobileCompanionPersisted, String> {
    let value: serde_json::Value =
        serde_json::from_slice(data).map_err(|error| error.to_string())?;
    if !value.is_object() {
        return Err("expected a JSON object".into());
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn write_mobile_companion_persistence(
    path: &Path,
    persisted: &MobileCompanionPersisted,
    encryption: Option<&EncryptedStore>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    let data = serde_json::to_vec_pretty(persisted)
        .map_err(|err| format!("failed to encode mobile companion state: {err}"))?;
    if let Some(encryption) = encryption {
        let data = encryption
            .encrypt(&data)
            .map_err(|err| format!("failed to encrypt mobile companion state: {err}"))?;
        return atomic_write(path, &data)
            .map_err(|err| format!("failed to write {}: {err}", path.display()));
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, data).map_err(|err| format!("failed to write {}: {err}", temp.display()))?;
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(err) if path.exists() && err.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)
                .map_err(|err| format!("failed to replace {}: {err}", path.display()))?;
            fs::rename(&temp, path).map_err(|err| {
                format!(
                    "failed to move {} to {} after replace: {err}",
                    temp.display(),
                    path.display()
                )
            })
        }
        Err(err) => Err(format!(
            "failed to move {} to {}: {err}",
            temp.display(),
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_persistence_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}.json", std::process::id()))
    }

    #[test]
    fn mobile_companion_persists_enabled_devices_and_revokes() {
        let path = unique_persistence_path("milim-mobile-companion");
        let bridge = MobileCompanionBridge::persistent(path.clone());
        assert!(!bridge.status(1).enabled);

        bridge.set_enabled(true, 2);
        let pairing = bridge.start_pairing(3).unwrap();
        let secret = pairing.path.split("secret=").nth(1).unwrap().to_string();
        let paired = bridge
            .pair_device(
                MobilePairRequest {
                    pair_id: pairing.id,
                    secret,
                    device_name: Some("Pixel QA".to_string()),
                },
                4,
                None,
            )
            .unwrap();

        let reloaded = MobileCompanionBridge::persistent(path.clone());
        let status = reloaded.status(5);
        assert!(status.enabled);
        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].id, paired.device_id);
        assert!(reloaded
            .authenticate_device(&paired.device_key, 6)
            .is_some());

        reloaded.set_enabled(false, 7);
        let disabled = MobileCompanionBridge::persistent(path.clone());
        assert!(!disabled.status(8).enabled);
        assert!(disabled
            .authenticate_device(&paired.device_key, 9)
            .is_none());

        disabled.set_enabled(true, 10);
        assert!(disabled
            .authenticate_device(&paired.device_key, 11)
            .is_some());
        disabled.revoke_device(&paired.device_id, 12);

        let revoked = MobileCompanionBridge::persistent(path.clone());
        assert!(revoked.status(13).devices.is_empty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn encrypted_mobile_companion_reloads_device_credentials() {
        let path = unique_persistence_path("milim-mobile-companion-encrypted");
        let encryption = EncryptedStore::from_key(&[8_u8; 32]);
        let bridge =
            MobileCompanionBridge::persistent_encrypted(path.clone(), encryption.clone()).unwrap();
        bridge.set_enabled(true, 2);
        let pairing = bridge.start_pairing(3).unwrap();
        let secret = pairing.path.split("secret=").nth(1).unwrap().to_string();
        let paired = bridge
            .pair_device(
                MobilePairRequest {
                    pair_id: pairing.id,
                    secret,
                    device_name: Some("Encrypted phone".to_string()),
                },
                4,
                None,
            )
            .unwrap();
        let persisted = fs::read(&path).unwrap();
        assert!(!persisted
            .windows(paired.device_key.len())
            .any(|window| window == paired.device_key.as_bytes()));

        let reloaded =
            MobileCompanionBridge::persistent_encrypted(path.clone(), encryption).unwrap();
        assert!(reloaded
            .authenticate_device(&paired.device_key, 5)
            .is_some());
        let _ = fs::remove_file(path);
    }
}
