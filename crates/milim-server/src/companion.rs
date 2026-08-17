use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use milim_storage::EncryptedStore;
use milim_tools::atomic_write;

const PAIRING_TTL_SECS: u64 = 10 * 60;
const PAIRING_REQUEST_TTL_SECS: u64 = 2 * 60;
const MAX_PENDING_PAIRING_REQUESTS: usize = 5;
const MAX_DEVICE_NAME_CHARS: usize = 60;

#[derive(Clone)]
pub struct MobileCompanionBridge {
    inner: Arc<RwLock<MobileCompanionInner>>,
    persistence_path: Option<Arc<PathBuf>>,
    persistence_encryption: Option<EncryptedStore>,
}

impl Default for MobileCompanionBridge {
    fn default() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MobileCompanionInner::default())),
            persistence_path: None,
            persistence_encryption: None,
        }
    }
}

#[derive(Default)]
struct MobileCompanionInner {
    enabled: bool,
    pairing: Option<MobilePairing>,
    pairing_requests: Vec<MobilePairingRequest>,
    devices: Vec<MobileDevice>,
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

#[derive(Clone, Debug)]
struct MobilePairingRequest {
    id: String,
    key: String,
    device_name: String,
    platform: String,
    created_at: u64,
    expires_at: u64,
    state: MobilePairingRequestState,
}

#[derive(Clone, Debug)]
enum MobilePairingRequestState {
    Pending,
    Approved,
    Denied,
    Claimed(MobilePairResponse),
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
    pub pairing_requests: Vec<MobilePairingRequestInfo>,
    pub devices: Vec<MobileDeviceInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairingInfo {
    pub id: String,
    pub expires_at: u64,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairingRequestInfo {
    pub id: String,
    pub device_name: String,
    pub platform: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MobilePairingRequestCreate {
    pub device_name: Option<String>,
    pub platform: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairingRequestCreated {
    pub request_id: String,
    pub request_key: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MobilePairingRequestView {
    pub request_id: String,
    pub status: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MobilePairingRequestDecision {
    pub approved: bool,
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
        })
    }

    pub fn status(&self, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.expire_pairing(now);
        inner.expire_pairing_requests(now);
        inner.status()
    }

    pub fn set_enabled(&self, enabled: bool, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.enabled = enabled;
        if !enabled {
            inner.pairing = None;
            inner.pairing_requests.clear();
        }
        inner.expire_pairing(now);
        inner.expire_pairing_requests(now);
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

    pub fn start_pairing_request(
        &self,
        req: MobilePairingRequestCreate,
        now: u64,
    ) -> Result<MobilePairingRequestCreated, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        inner.expire_pairing_requests(now);
        if inner
            .pairing_requests
            .iter()
            .filter(|request| matches!(&request.state, MobilePairingRequestState::Pending))
            .count()
            >= MAX_PENDING_PAIRING_REQUESTS
        {
            return Err("too many pairing requests are waiting for approval".to_string());
        }
        let id = format!("request-{}", short_id());
        let key = secret_key("request");
        let expires_at = now.saturating_add(PAIRING_REQUEST_TTL_SECS);
        inner.pairing_requests.push(MobilePairingRequest {
            id: id.clone(),
            key: key.clone(),
            device_name: clean_device_name(req.device_name.as_deref().unwrap_or("Phone")),
            platform: clean_device_platform(req.platform.as_deref()),
            created_at: now,
            expires_at,
            state: MobilePairingRequestState::Pending,
        });
        Ok(MobilePairingRequestCreated {
            request_id: id,
            request_key: key,
            expires_at,
        })
    }

    pub fn pairing_request_status(
        &self,
        id: &str,
        key: &str,
        now: u64,
    ) -> Result<MobilePairingRequestView, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        inner.expire_pairing_requests(now);
        let request = inner
            .pairing_requests
            .iter()
            .find(|request| request.id == id && request.key == key)
            .ok_or_else(|| "pairing request expired or missing".to_string())?;
        Ok(request.view())
    }

    pub fn decide_pairing_request(
        &self,
        id: &str,
        decision: MobilePairingRequestDecision,
        now: u64,
    ) -> Result<MobileCompanionStatus, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        inner.expire_pairing_requests(now);
        let request = inner
            .pairing_requests
            .iter_mut()
            .find(|request| request.id == id)
            .ok_or_else(|| "pairing request expired or missing".to_string())?;
        match request.state.clone() {
            MobilePairingRequestState::Pending => {
                request.state = if decision.approved {
                    MobilePairingRequestState::Approved
                } else {
                    MobilePairingRequestState::Denied
                };
            }
            MobilePairingRequestState::Approved if decision.approved => {}
            MobilePairingRequestState::Denied if !decision.approved => {}
            _ => return Err("pairing request was already resolved".to_string()),
        }
        Ok(inner.status())
    }

    pub fn claim_pairing_request(
        &self,
        id: &str,
        key: &str,
        now: u64,
        user_agent: Option<&str>,
    ) -> Result<MobilePairResponse, String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        if !inner.enabled {
            return Err("mobile companion is disabled".to_string());
        }
        inner.expire_pairing_requests(now);
        let index = inner
            .pairing_requests
            .iter()
            .position(|request| request.id == id && request.key == key)
            .ok_or_else(|| "pairing request expired or missing".to_string())?;
        match inner.pairing_requests[index].state.clone() {
            MobilePairingRequestState::Pending => {
                Err("pairing request is waiting for desktop approval".to_string())
            }
            MobilePairingRequestState::Denied => Err("pairing request was denied".to_string()),
            MobilePairingRequestState::Claimed(response) => Ok(response),
            MobilePairingRequestState::Approved => {
                let name = inner.pairing_requests[index].device_name.clone();
                let response = inner.add_device(name, now, user_agent);
                inner.pairing_requests[index].state =
                    MobilePairingRequestState::Claimed(response.clone());
                self.persist_inner(&inner);
                Ok(response)
            }
        }
    }

    pub fn cancel_pairing_request(&self, id: &str, key: &str, now: u64) -> Result<(), String> {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.expire_pairing_requests(now);
        let index = inner
            .pairing_requests
            .iter()
            .position(|request| request.id == id && request.key == key)
            .ok_or_else(|| "pairing request expired or missing".to_string())?;
        if matches!(
            &inner.pairing_requests[index].state,
            MobilePairingRequestState::Claimed(_)
        ) {
            return Err("paired device requests cannot be cancelled".to_string());
        }
        inner.pairing_requests.remove(index);
        Ok(())
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
            .take()
            .ok_or_else(|| "pairing session expired or missing".to_string())?;
        if pairing.id != req.pair_id || pairing.secret != req.secret {
            inner.pairing = Some(pairing);
            return Err("invalid pairing token".to_string());
        }

        let name = req.device_name.unwrap_or_default();
        let response = inner.add_device(name, now, user_agent);
        self.persist_inner(&inner);
        Ok(response)
    }

    pub fn revoke_device(&self, id: &str, now: u64) -> MobileCompanionStatus {
        let mut inner = self.inner.write().expect("mobile companion lock poisoned");
        inner.devices.retain(|device| device.id != id);
        inner.expire_pairing(now);
        inner.expire_pairing_requests(now);
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
            pairing_requests: self
                .pairing_requests
                .iter()
                .map(MobilePairingRequest::info)
                .collect(),
            devices: self.devices.iter().map(MobileDevice::info).collect(),
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

    fn expire_pairing_requests(&mut self, now: u64) {
        self.pairing_requests
            .retain(|request| request.expires_at > now);
    }

    fn add_device(
        &mut self,
        requested_name: String,
        now: u64,
        user_agent: Option<&str>,
    ) -> MobilePairResponse {
        let fallback = user_agent
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or("Phone");
        let name = clean_device_name(if requested_name.trim().is_empty() {
            fallback
        } else {
            &requested_name
        });
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
        self.devices.push(device);
        response
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

impl MobilePairingRequest {
    fn status(&self) -> &'static str {
        match &self.state {
            MobilePairingRequestState::Pending => "pending",
            MobilePairingRequestState::Approved => "approved",
            MobilePairingRequestState::Denied => "denied",
            MobilePairingRequestState::Claimed(_) => "paired",
        }
    }

    fn info(&self) -> MobilePairingRequestInfo {
        MobilePairingRequestInfo {
            id: self.id.clone(),
            device_name: self.device_name.clone(),
            platform: self.platform.clone(),
            created_at: self.created_at,
            expires_at: self.expires_at,
            status: self.status().to_string(),
        }
    }

    fn view(&self) -> MobilePairingRequestView {
        MobilePairingRequestView {
            request_id: self.id.clone(),
            status: self.status().to_string(),
            expires_at: self.expires_at,
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

fn clean_device_platform(input: Option<&str>) -> String {
    match input
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "android" => "android".to_string(),
        "ios" => "ios".to_string(),
        _ => "mobile".to_string(),
    }
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

    #[test]
    fn mobile_companion_pairing_claims_and_device_credentials_are_isolated_per_host() {
        let host_a = MobileCompanionBridge::default();
        let host_b = MobileCompanionBridge::default();
        host_a.set_enabled(true, 1);
        host_b.set_enabled(true, 1);

        let pairing_a = host_a.start_pairing(2).unwrap();
        let pairing_b = host_b.start_pairing(2).unwrap();
        let secret_a = pairing_a.path.split("secret=").nth(1).unwrap().to_string();
        let secret_b = pairing_b.path.split("secret=").nth(1).unwrap().to_string();

        let cross_pair = host_b.pair_device(
            MobilePairRequest {
                pair_id: pairing_a.id.clone(),
                secret: secret_a.clone(),
                device_name: Some("Wrong host".to_string()),
            },
            3,
            None,
        );
        assert_eq!(cross_pair.unwrap_err(), "invalid pairing token");

        let paired_a = host_a
            .pair_device(
                MobilePairRequest {
                    pair_id: pairing_a.id,
                    secret: secret_a,
                    device_name: Some("Phone A".to_string()),
                },
                4,
                None,
            )
            .unwrap();
        let paired_b = host_b
            .pair_device(
                MobilePairRequest {
                    pair_id: pairing_b.id,
                    secret: secret_b,
                    device_name: Some("Phone B".to_string()),
                },
                4,
                None,
            )
            .unwrap();

        assert!(host_a
            .authenticate_device(&paired_a.device_key, 5)
            .is_some());
        assert!(host_a
            .authenticate_device(&paired_b.device_key, 5)
            .is_none());
        assert!(host_b
            .authenticate_device(&paired_b.device_key, 5)
            .is_some());
        assert!(host_b
            .authenticate_device(&paired_a.device_key, 5)
            .is_none());

        let replay = host_a.pair_device(
            MobilePairRequest {
                pair_id: "consumed".to_string(),
                secret: "consumed".to_string(),
                device_name: None,
            },
            6,
            None,
        );
        assert_eq!(replay.unwrap_err(), "pairing session expired or missing");

        host_a.revoke_device(&paired_a.device_id, 7);
        assert!(host_a
            .authenticate_device(&paired_a.device_key, 8)
            .is_none());
        assert!(host_b
            .authenticate_device(&paired_b.device_key, 8)
            .is_some());
    }

    #[test]
    fn mobile_pairing_requests_require_desktop_approval_and_claim_once() {
        let bridge = MobileCompanionBridge::default();
        bridge.set_enabled(true, 1);
        let request = bridge
            .start_pairing_request(
                MobilePairingRequestCreate {
                    device_name: Some("Android controller".to_string()),
                    platform: Some("android".to_string()),
                },
                2,
            )
            .unwrap();

        assert!(bridge
            .pairing_request_status(&request.request_id, "wrong-key", 3)
            .is_err());
        assert_eq!(
            bridge
                .pairing_request_status(&request.request_id, &request.request_key, 3)
                .unwrap()
                .status,
            "pending"
        );
        let desktop = bridge.status(3);
        assert_eq!(desktop.pairing_requests.len(), 1);
        assert_eq!(
            desktop.pairing_requests[0].device_name,
            "Android controller"
        );
        assert_eq!(desktop.pairing_requests[0].platform, "android");

        bridge
            .decide_pairing_request(
                &request.request_id,
                MobilePairingRequestDecision { approved: true },
                4,
            )
            .unwrap();
        assert_eq!(
            bridge
                .pairing_request_status(&request.request_id, &request.request_key, 4)
                .unwrap()
                .status,
            "approved"
        );

        let paired = bridge
            .claim_pairing_request(&request.request_id, &request.request_key, 5, None)
            .unwrap();
        let replay = bridge
            .claim_pairing_request(&request.request_id, &request.request_key, 6, None)
            .unwrap();
        assert_eq!(replay.device_id, paired.device_id);
        assert_eq!(replay.device_key, paired.device_key);
        assert!(bridge.authenticate_device(&paired.device_key, 7).is_some());
        assert_eq!(bridge.status(7).devices.len(), 1);
    }

    #[test]
    fn denied_cancelled_and_expired_mobile_pairing_requests_cannot_pair() {
        let bridge = MobileCompanionBridge::default();
        bridge.set_enabled(true, 1);
        let denied = bridge
            .start_pairing_request(
                MobilePairingRequestCreate {
                    device_name: Some("Unknown phone".to_string()),
                    platform: Some("unexpected".to_string()),
                },
                2,
            )
            .unwrap();
        bridge
            .decide_pairing_request(
                &denied.request_id,
                MobilePairingRequestDecision { approved: false },
                3,
            )
            .unwrap();
        assert_eq!(
            bridge
                .pairing_request_status(&denied.request_id, &denied.request_key, 3)
                .unwrap()
                .status,
            "denied"
        );
        assert!(bridge
            .claim_pairing_request(&denied.request_id, &denied.request_key, 3, None)
            .is_err());

        let cancelled = bridge
            .start_pairing_request(
                MobilePairingRequestCreate {
                    device_name: None,
                    platform: Some("ios".to_string()),
                },
                4,
            )
            .unwrap();
        bridge
            .cancel_pairing_request(&cancelled.request_id, &cancelled.request_key, 5)
            .unwrap();
        assert!(bridge
            .pairing_request_status(&cancelled.request_id, &cancelled.request_key, 5)
            .is_err());

        let expired = bridge
            .start_pairing_request(
                MobilePairingRequestCreate {
                    device_name: None,
                    platform: None,
                },
                6,
            )
            .unwrap();
        assert!(bridge
            .pairing_request_status(
                &expired.request_id,
                &expired.request_key,
                6 + PAIRING_REQUEST_TTL_SECS,
            )
            .is_err());
    }
}
