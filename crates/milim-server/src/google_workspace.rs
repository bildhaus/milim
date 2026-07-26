//! Selected-file Google Workspace integration for the desktop app.
//!
//! One `drive.file` grant backs Drive, Docs, Sheets, and Slides. The connector
//! never accepts an arbitrary Google file id: every operation must target a
//! file explicitly returned by Google Picker or created by Milim.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::extract::{ConnectInfo, Path as AxumPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use milim_core::{Error, Result};
use milim_storage::{create_private_file, EncryptedStore};
use milim_tools::{atomic_write, resolve_workspace_path, Tool, ToolEffect};
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{oneshot, Mutex, RwLock};
use uuid::Uuid;

use crate::auth::authorize;
use crate::error::ApiError;
use crate::state::AppState;

pub const DRIVE_FILE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";
pub const GOOGLE_DOC_MIME: &str = "application/vnd.google-apps.document";
pub const GOOGLE_SHEET_MIME: &str = "application/vnd.google-apps.spreadsheet";
pub const GOOGLE_SLIDE_MIME: &str = "application/vnd.google-apps.presentation";
pub const GOOGLE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const MILIM_DRIVE_FOLDER_NAME: &str = "Milim";

const PICKER_TTL: Duration = Duration::from_secs(5 * 60);
#[cfg(not(test))]
const REVOCATION_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const REVOCATION_TIMEOUT: Duration = Duration::from_millis(100);
const MODEL_TEXT_LIMIT: usize = 100_000;
const PREVIEW_BYTE_LIMIT: usize = 25 * 1024 * 1024;
const TRANSFER_BYTE_LIMIT: usize = 100 * 1024 * 1024;
const SHEET_READ_CELL_LIMIT: usize = 10_000;
const SHEET_WRITE_CELL_LIMIT: usize = 5_000;
const EDIT_OPERATION_LIMIT: usize = 100;
const FOLDER_FILE_LIMIT: usize = 10_000;

#[derive(Clone)]
pub struct GoogleWorkspaceConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub revoke_url: String,
    pub drive_url: String,
    pub upload_url: String,
    pub sheets_url: String,
    pub docs_url: String,
    pub slides_url: String,
}

impl GoogleWorkspaceConfig {
    pub fn desktop(client_id: Option<String>, client_secret: Option<String>) -> Self {
        Self {
            client_id: clean(client_id),
            client_secret: clean(client_secret),
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            revoke_url: "https://oauth2.googleapis.com/revoke".into(),
            drive_url: "https://www.googleapis.com/drive/v3".into(),
            upload_url: "https://www.googleapis.com/upload/drive/v3".into(),
            sheets_url: "https://sheets.googleapis.com/v4".into(),
            docs_url: "https://docs.googleapis.com/v1".into(),
            slides_url: "https://slides.googleapis.com/v1".into(),
        }
    }

    fn available(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoogleFileCapabilities {
    #[serde(default)]
    pub can_edit: bool,
    #[serde(default)]
    pub can_download: bool,
    #[serde(default)]
    pub can_move: bool,
    #[serde(default)]
    pub can_rename: bool,
    #[serde(default)]
    pub can_share: bool,
    #[serde(default)]
    pub can_trash: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GoogleFileSummary {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    #[serde(default)]
    pub web_view_link: Option<String>,
    #[serde(default)]
    pub icon_link: Option<String>,
    #[serde(default)]
    pub modified_time: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub drive_id: Option<String>,
    #[serde(default)]
    pub capabilities: GoogleFileCapabilities,
    #[serde(default)]
    pub created_by_milim: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct StoredWorkspace {
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    managed_folder_id: Option<String>,
    #[serde(default)]
    files: BTreeMap<String, GoogleFileSummary>,
}

struct GoogleWorkspaceStore {
    enc: EncryptedStore,
    path: PathBuf,
}

impl GoogleWorkspaceStore {
    fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let key_path = root.join("google-workspace.key");
        let key = match std::fs::read(&key_path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                key
            }
            Ok(_) => {
                return Err(Error::Other(
                    "Google Workspace encryption key is invalid".into(),
                ))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let key = EncryptedStore::random_key();
                create_private_file(&key_path, &key)?;
                key
            }
            Err(error) => return Err(error.into()),
        };
        Self::open_with_encryption(root, EncryptedStore::from_key(&key))
    }

    fn open_with_encryption(root: &Path, enc: EncryptedStore) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self {
            enc,
            path: root.join("google-workspace.enc"),
        })
    }

    fn load(&self) -> Result<StoredWorkspace> {
        let blob = match std::fs::read(&self.path) {
            Ok(blob) => blob,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoredWorkspace::default())
            }
            Err(error) => return Err(error.into()),
        };
        serde_json::from_slice(&self.enc.decrypt(&blob)?).map_err(Into::into)
    }

    fn save(&self, state: &StoredWorkspace) -> Result<()> {
        let plain = serde_json::to_vec(state)?;
        atomic_write(&self.path, &self.enc.encrypt(&plain)?)
    }

    fn clear(&self) -> Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Clone)]
struct CachedAccessToken {
    value: String,
    expires_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct GoogleWorkspaceStatus {
    pub available: bool,
    pub connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_folder_id: Option<String>,
    pub files: Vec<GoogleFileSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoogleRevocationStatus {
    Confirmed,
    Unconfirmed,
    NotNeeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDisconnectResult {
    pub local_authorization_removed: bool,
    pub revocation: GoogleRevocationStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct GooglePickerFlow {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub files: Vec<GoogleFileSummary>,
    #[serde(skip)]
    created_at: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default = "default_token_lifetime")]
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
}

fn default_token_lifetime() -> u64 {
    3600
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveCapabilities {
    #[serde(default)]
    can_edit: bool,
    #[serde(default)]
    can_download: bool,
    #[serde(default)]
    can_move_item_within_drive: bool,
    #[serde(default)]
    can_move_item_out_of_drive: bool,
    #[serde(default)]
    can_rename: bool,
    #[serde(default)]
    can_share: bool,
    #[serde(default)]
    can_trash: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    name: String,
    mime_type: String,
    #[serde(default)]
    web_view_link: Option<String>,
    #[serde(default)]
    icon_link: Option<String>,
    #[serde(default)]
    modified_time: Option<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    trashed: bool,
    #[serde(default)]
    parents: Vec<String>,
    #[serde(default)]
    drive_id: Option<String>,
    #[serde(default)]
    capabilities: Option<DriveCapabilities>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    #[serde(default)]
    next_page_token: Option<String>,
}

impl DriveFile {
    fn summary(self, created_by_milim: bool) -> GoogleFileSummary {
        let capabilities = self.capabilities.unwrap_or(DriveCapabilities {
            can_edit: false,
            can_download: false,
            can_move_item_within_drive: false,
            can_move_item_out_of_drive: false,
            can_rename: false,
            can_share: false,
            can_trash: false,
        });
        GoogleFileSummary {
            id: self.id,
            name: self.name,
            mime_type: self.mime_type,
            web_view_link: self.web_view_link,
            icon_link: self.icon_link,
            modified_time: self.modified_time,
            size: self.size.and_then(|value| value.parse().ok()),
            trashed: self.trashed,
            parents: self.parents,
            drive_id: self.drive_id,
            capabilities: GoogleFileCapabilities {
                can_edit: capabilities.can_edit,
                can_download: capabilities.can_download,
                can_move: capabilities.can_move_item_within_drive
                    || capabilities.can_move_item_out_of_drive,
                can_rename: capabilities.can_rename,
                can_share: capabilities.can_share,
                can_trash: capabilities.can_trash,
            },
            created_by_milim,
        }
    }
}

pub struct GoogleWorkspaceConnection {
    config: GoogleWorkspaceConfig,
    client: reqwest::Client,
    store: GoogleWorkspaceStore,
    stored: RwLock<StoredWorkspace>,
    access_token: Mutex<Option<CachedAccessToken>>,
    managed_folder_lock: Mutex<()>,
    flows: RwLock<HashMap<String, GooglePickerFlow>>,
    workspace: Arc<StdRwLock<Option<PathBuf>>>,
    state_error: StdMutex<Option<String>>,
}

impl GoogleWorkspaceConnection {
    pub fn open(
        root: impl AsRef<Path>,
        config: GoogleWorkspaceConfig,
        workspace: Arc<StdRwLock<Option<PathBuf>>>,
    ) -> Result<Self> {
        let store = GoogleWorkspaceStore::open(root.as_ref())?;
        Self::open_with_store(store, config, workspace)
    }

    pub fn open_with_encryption(
        root: impl AsRef<Path>,
        config: GoogleWorkspaceConfig,
        workspace: Arc<StdRwLock<Option<PathBuf>>>,
        encryption: EncryptedStore,
    ) -> Result<Self> {
        let store = GoogleWorkspaceStore::open_with_encryption(root.as_ref(), encryption)?;
        Self::open_with_store(store, config, workspace)
    }

    fn open_with_store(
        store: GoogleWorkspaceStore,
        config: GoogleWorkspaceConfig,
        workspace: Arc<StdRwLock<Option<PathBuf>>>,
    ) -> Result<Self> {
        let (stored, state_error) = match store.load() {
            Ok(stored) => (stored, None),
            Err(error) => {
                tracing::warn!("Google Workspace state rejected: {error}");
                (
                    StoredWorkspace::default(),
                    Some(
                        "Saved Google Workspace authorization is corrupt. Disconnect or reconnect to replace it."
                            .to_string(),
                    ),
                )
            }
        };
        Ok(Self {
            config,
            client: reqwest::Client::builder()
                .user_agent(format!("milim/{}", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| Error::Other(error.to_string()))?,
            store,
            stored: RwLock::new(stored),
            access_token: Mutex::new(None),
            managed_folder_lock: Mutex::new(()),
            flows: RwLock::new(HashMap::new()),
            workspace,
            state_error: StdMutex::new(state_error),
        })
    }

    pub fn available(&self) -> bool {
        self.config.available()
    }

    pub async fn status(&self) -> GoogleWorkspaceStatus {
        let stored = self.stored.read().await;
        GoogleWorkspaceStatus {
            available: self.available(),
            connected: stored.refresh_token.is_some(),
            managed_folder_id: stored.managed_folder_id.clone(),
            files: stored.files.values().cloned().collect(),
            unavailable_reason: (!self.available()).then(|| {
                "This build does not include Milim's Google OAuth credentials.".to_string()
            }),
            error: self.state_error.lock().ok().and_then(|error| error.clone()),
        }
    }

    pub async fn start_picker(self: &Arc<Self>, file_ids: Vec<String>) -> Result<GooglePickerFlow> {
        let client_id = self
            .config
            .client_id
            .as_deref()
            .ok_or_else(unavailable_error)?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let redirect_uri = loopback_redirect_uri(address.port());
        let id = Uuid::new_v4().to_string();
        let oauth_state = random_token();
        let verifier = format!("{}{}", random_token(), random_token());
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let (callback_tx, callback_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let relay = Arc::new(PickerRelay {
            expected_state: oauth_state.clone(),
            callback: StdMutex::new(Some(callback_tx)),
        });
        let callback_app = Router::new()
            .route("/", get(oauth_callback))
            .with_state(relay.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, callback_app)
                .with_graceful_shutdown(async move {
                    let _ = tokio::time::timeout(PICKER_TTL, shutdown_rx).await;
                })
                .await;
        });

        let mut url =
            Url::parse(&self.config.auth_url).map_err(|error| Error::Other(error.to_string()))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("client_id", client_id)
                .append_pair("scope", DRIVE_FILE_SCOPE)
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("response_type", "code")
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent")
                .append_pair("trigger_onepick", "true")
                .append_pair("allow_multiple", "true")
                .append_pair("state", &oauth_state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            if !file_ids.is_empty() {
                query.append_pair("file_ids", &file_ids.join(","));
            }
        }
        let flow = GooglePickerFlow {
            id: id.clone(),
            status: "pending".into(),
            url: Some(url.to_string()),
            error: None,
            files: Vec::new(),
            created_at: now_seconds(),
        };
        let mut flows = self.flows.write().await;
        flows.retain(|_, flow| {
            flow.created_at.saturating_add(PICKER_TTL.as_secs() * 2) > now_seconds()
        });
        flows.insert(id.clone(), flow.clone());
        drop(flows);

        let connection = self.clone();
        tokio::spawn(async move {
            let result = connection
                .run_picker_flow(callback_rx, &redirect_uri, &verifier)
                .await;
            let _ = shutdown_tx.send(());
            let mut flows = connection.flows.write().await;
            if let Some(flow) = flows.get_mut(&id) {
                flow.url = None;
                match result {
                    Ok(files) => {
                        flow.status = "complete".into();
                        flow.files = files;
                    }
                    Err(error) => {
                        flow.status = "error".into();
                        flow.error = Some(error.to_string());
                    }
                }
            }
        });
        Ok(flow)
    }

    pub async fn picker_flow(&self, id: &str) -> Option<GooglePickerFlow> {
        self.flows.read().await.get(id).cloned()
    }

    async fn run_picker_flow(
        &self,
        callback_rx: oneshot::Receiver<OAuthCallbackQuery>,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<Vec<GoogleFileSummary>> {
        let deadline = tokio::time::Instant::now() + PICKER_TTL;
        let callback = tokio::time::timeout_at(deadline, callback_rx)
            .await
            .map_err(|_| Error::Other("Google Picker timed out".into()))?
            .map_err(|_| Error::Other("Google OAuth callback closed".into()))?;
        let token = self
            .exchange_picker_token(&callback, redirect_uri, verifier)
            .await?;
        let file_ids = callback.picked_file_ids()?;
        if file_ids.is_empty() {
            return Err(Error::InvalidRequest(
                "Choose at least one Google Drive file".into(),
            ));
        }
        self.complete_picker(token, file_ids).await
    }

    async fn exchange_picker_token(
        &self,
        callback: &OAuthCallbackQuery,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<TokenResponse> {
        if let Some(error) = callback.error.as_deref() {
            return Err(Error::Other(format!("Google Picker cancelled: {error}")));
        }
        let code = callback
            .code
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| Error::InvalidRequest("Google returned no authorization code".into()))?;
        if callback.scope.as_deref().is_some_and(|scope| {
            scope
                .split_whitespace()
                .any(|value| value != DRIVE_FILE_SCOPE)
        }) {
            return Err(Error::Unauthorized(
                "Google returned an unexpected OAuth scope".into(),
            ));
        }
        let client_secret = self
            .config
            .client_secret
            .as_deref()
            .ok_or_else(unavailable_error)?;
        let form = [
            (
                "client_id",
                self.config.client_id.as_deref().unwrap_or_default(),
            ),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ];
        let token = self
            .client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(upstream)?;
        let token = ensure_google_response(token)
            .await?
            .json::<TokenResponse>()
            .await
            .map_err(upstream)?;
        if token.scope.as_deref().is_some_and(|scope| {
            scope
                .split_whitespace()
                .any(|value| value != DRIVE_FILE_SCOPE)
        }) {
            return Err(Error::Unauthorized(
                "Google returned an unexpected OAuth scope".into(),
            ));
        }
        Ok(token)
    }

    async fn complete_picker(
        &self,
        token: TokenResponse,
        ids: Vec<String>,
    ) -> Result<Vec<GoogleFileSummary>> {
        if ids.len() > 100 {
            return Err(Error::InvalidRequest(
                "Google Picker accepts at most 100 selected files".into(),
            ));
        }
        for id in &ids {
            validate_google_id(id)?;
        }
        let account_id = self
            .fetch_account_id_with_token(&token.access_token)
            .await?;
        if self
            .stored
            .read()
            .await
            .account_id
            .as_deref()
            .is_some_and(|existing| existing != account_id)
        {
            return Err(Error::Unauthorized(
                "Milim supports one Google account; disconnect before choosing another account"
                    .into(),
            ));
        }
        let expires_at = now_seconds().saturating_add(token.expires_in);
        *self.access_token.lock().await = Some(CachedAccessToken {
            value: token.access_token.clone(),
            expires_at,
        });

        let mut selected = Vec::with_capacity(ids.len());
        for id in ids {
            selected.push(self.fetch_file_with_token(&id, &token.access_token).await?);
        }
        let mut files = BTreeMap::new();
        for folder in selected
            .iter()
            .filter(|file| file.mime_type == GOOGLE_FOLDER_MIME)
        {
            for file in self
                .folder_descendants_with_token(&folder.id, &token.access_token)
                .await?
            {
                files.insert(file.id.clone(), file);
            }
        }
        for file in selected {
            files.insert(file.id.clone(), file);
        }
        let mut stored = self.stored.write().await;
        stored.account_id = Some(account_id.to_string());
        if let Some(refresh_token) = token.refresh_token {
            stored.refresh_token = Some(refresh_token);
        }
        if stored.refresh_token.is_none() {
            return Err(Error::Unauthorized(
                "Google did not return a refresh token; reconnect and grant access".into(),
            ));
        }
        for file in files.values_mut() {
            if let Some(existing) = stored.files.get(&file.id) {
                file.created_by_milim = existing.created_by_milim;
            }
            stored.files.insert(file.id.clone(), file.clone());
        }
        self.store.save(&stored)?;
        if let Ok(mut error) = self.state_error.lock() {
            *error = None;
        }
        Ok(files.into_values().collect())
    }

    async fn fetch_account_id_with_token(&self, token: &str) -> Result<String> {
        let mut url = Url::parse(&format!("{}/about", self.config.drive_url))
            .map_err(|error| Error::Other(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("fields", "user(permissionId)");
        let about = ensure_google_response(
            self.client
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .map_err(upstream)?,
        )
        .await?
        .json::<Value>()
        .await
        .map_err(upstream)?;
        about
            .pointer("/user/permissionId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| Error::Upstream("Google returned no account identifier".into()))
    }

    pub async fn remove_file(&self, id: &str) -> Result<bool> {
        let mut stored = self.stored.write().await;
        let removed = stored.files.remove(id).is_some();
        if stored.managed_folder_id.as_deref() == Some(id) {
            stored.managed_folder_id = None;
        }
        self.store.save(&stored)?;
        Ok(removed)
    }

    pub async fn disconnect(&self) -> Result<GoogleDisconnectResult> {
        let refresh = self.stored.read().await.refresh_token.clone();
        let revocation = match refresh {
            Some(token) => match tokio::time::timeout(
                REVOCATION_TIMEOUT,
                self.client
                    .post(&self.config.revoke_url)
                    .form(&[("token", token)])
                    .send(),
            )
            .await
            {
                Ok(Ok(response)) if response.status().is_success() => {
                    GoogleRevocationStatus::Confirmed
                }
                _ => GoogleRevocationStatus::Unconfirmed,
            },
            None => GoogleRevocationStatus::NotNeeded,
        };
        self.store.clear()?;
        *self.stored.write().await = StoredWorkspace::default();
        *self.access_token.lock().await = None;
        if let Ok(mut error) = self.state_error.lock() {
            *error = None;
        }
        Ok(GoogleDisconnectResult {
            local_authorization_removed: true,
            revocation,
        })
    }

    async fn access_token(&self, force_refresh: bool) -> Result<String> {
        if !force_refresh {
            if let Some(token) = self.access_token.lock().await.clone() {
                if token.expires_at > now_seconds().saturating_add(60) {
                    return Ok(token.value);
                }
            }
        }
        let refresh_token = self
            .stored
            .read()
            .await
            .refresh_token
            .clone()
            .ok_or_else(|| Error::Unauthorized("Connect Google Workspace first".into()))?;
        let client_secret = self
            .config
            .client_secret
            .as_deref()
            .ok_or_else(unavailable_error)?;
        let form = [
            (
                "client_id",
                self.config.client_id.as_deref().unwrap_or_default(),
            ),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        let token = self
            .client
            .post(&self.config.token_url)
            .form(&form)
            .send()
            .await
            .map_err(upstream)?;
        if token.status() == StatusCode::BAD_REQUEST || token.status() == StatusCode::UNAUTHORIZED {
            *self.access_token.lock().await = None;
            return Err(Error::Unauthorized(
                "Google authorization expired; reconnect Google Workspace".into(),
            ));
        }
        let token = token
            .error_for_status()
            .map_err(upstream)?
            .json::<TokenResponse>()
            .await
            .map_err(upstream)?;
        *self.access_token.lock().await = Some(CachedAccessToken {
            value: token.access_token.clone(),
            expires_at: now_seconds().saturating_add(token.expires_in),
        });
        Ok(token.access_token)
    }

    async fn request(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        for force in [false, true] {
            let token = self.access_token(force).await?;
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .bearer_auth(token);
            if let Some(body) = body.as_ref() {
                request = request.json(body);
            }
            let response = request.send().await.map_err(upstream)?;
            if response.status() == StatusCode::UNAUTHORIZED && !force {
                *self.access_token.lock().await = None;
                continue;
            }
            return ensure_google_response(response).await;
        }
        Err(Error::Unauthorized(
            "Google authorization expired; reconnect Google Workspace".into(),
        ))
    }

    async fn authorized_file(&self, id: &str) -> Result<GoogleFileSummary> {
        self.stored
            .read()
            .await
            .files
            .get(id)
            .cloned()
            .ok_or_else(|| {
                Error::ModelNotFound(format!("Google Drive file {id} is not authorized"))
            })
    }

    async fn fetch_file_with_token(&self, id: &str, token: &str) -> Result<GoogleFileSummary> {
        validate_google_id(id)?;
        let mut url = Url::parse(&format!("{}/files/{id}", self.config.drive_url))
            .map_err(|error| Error::Other(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("supportsAllDrives", "true")
            .append_pair("fields", DRIVE_FILE_FIELDS);
        let response = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(upstream)?;
        let file = ensure_google_response(response)
            .await?
            .json::<DriveFile>()
            .await
            .map_err(upstream)?;
        Ok(file.summary(false))
    }

    async fn folder_descendants_with_token(
        &self,
        folder_id: &str,
        token: &str,
    ) -> Result<Vec<GoogleFileSummary>> {
        validate_google_id(folder_id)?;
        let mut pending = VecDeque::from([folder_id.to_string()]);
        let mut seen_folders = HashSet::from([folder_id.to_string()]);
        let mut descendants = BTreeMap::new();

        while let Some(parent_id) = pending.pop_front() {
            let mut page_token = None;
            let mut seen_page_tokens = HashSet::new();
            loop {
                let mut url = Url::parse(&format!("{}/files", self.config.drive_url))
                    .map_err(|error| Error::Other(error.to_string()))?;
                {
                    let mut query = url.query_pairs_mut();
                    query
                        .append_pair(
                            "q",
                            &format!("'{parent_id}' in parents and trashed = false"),
                        )
                        .append_pair("pageSize", "1000")
                        .append_pair("supportsAllDrives", "true")
                        .append_pair("includeItemsFromAllDrives", "true")
                        .append_pair(
                            "fields",
                            &format!("nextPageToken,files({DRIVE_FILE_FIELDS})"),
                        );
                    if let Some(value) = page_token.as_deref() {
                        query.append_pair("pageToken", value);
                    }
                }
                let page = ensure_google_response(
                    self.client
                        .get(url)
                        .bearer_auth(token)
                        .send()
                        .await
                        .map_err(upstream)?,
                )
                .await?
                .json::<DriveFileList>()
                .await
                .map_err(upstream)?;
                for file in page.files {
                    let summary = file.summary(false);
                    if descendants.contains_key(&summary.id) {
                        continue;
                    }
                    if descendants.len() >= FOLDER_FILE_LIMIT {
                        return Err(Error::InvalidRequest(format!(
                            "Google Drive folders are limited to {FOLDER_FILE_LIMIT} Milim-authorized descendants"
                        )));
                    }
                    if summary.mime_type == GOOGLE_FOLDER_MIME
                        && seen_folders.insert(summary.id.clone())
                    {
                        pending.push_back(summary.id.clone());
                    }
                    descendants.insert(summary.id.clone(), summary);
                }
                let Some(next) = page.next_page_token.filter(|value| !value.is_empty()) else {
                    break;
                };
                if !seen_page_tokens.insert(next.clone()) {
                    return Err(Error::Upstream(
                        "Google Drive returned a repeated folder page token".into(),
                    ));
                }
                page_token = Some(next);
            }
        }
        Ok(descendants.into_values().collect())
    }

    async fn sync_folder_descendants(
        &self,
        folder: &GoogleFileSummary,
    ) -> Result<Vec<GoogleFileSummary>> {
        if folder.mime_type != GOOGLE_FOLDER_MIME {
            return Err(Error::InvalidRequest(format!(
                "{} is not a Google Drive folder",
                folder.name
            )));
        }
        let descendants = {
            let mut result = None;
            for force in [false, true] {
                let token = self.access_token(force).await?;
                match self.folder_descendants_with_token(&folder.id, &token).await {
                    Err(Error::Unauthorized(_)) if !force => {
                        *self.access_token.lock().await = None;
                    }
                    value => {
                        result = Some(value);
                        break;
                    }
                }
            }
            result.unwrap_or_else(|| {
                Err(Error::Unauthorized(
                    "Google authorization expired; reconnect Google Workspace".into(),
                ))
            })?
        };
        let mut stored = self.stored.write().await;
        for mut file in descendants.iter().cloned() {
            if let Some(existing) = stored.files.get(&file.id) {
                file.created_by_milim = existing.created_by_milim;
            }
            stored.files.insert(file.id.clone(), file);
        }
        self.store.save(&stored)?;
        Ok(descendants)
    }

    async fn refresh_file(&self, id: &str) -> Result<GoogleFileSummary> {
        let existing = self.authorized_file(id).await?;
        let token = self.access_token(false).await?;
        let mut file = self.fetch_file_with_token(id, &token).await?;
        file.created_by_milim = existing.created_by_milim;
        let mut stored = self.stored.write().await;
        stored.files.insert(file.id.clone(), file.clone());
        self.store.save(&stored)?;
        Ok(file)
    }

    async fn save_authorized_file(&self, file: GoogleFileSummary) -> Result<()> {
        let mut stored = self.stored.write().await;
        stored.files.insert(file.id.clone(), file);
        self.store.save(&stored)
    }

    async fn managed_folder(&self) -> Result<GoogleFileSummary> {
        let _guard = self.managed_folder_lock.lock().await;
        if let Some(folder) = {
            let stored = self.stored.read().await;
            stored
                .managed_folder_id
                .as_ref()
                .and_then(|id| stored.files.get(id))
                .filter(|file| file.mime_type == GOOGLE_FOLDER_MIME && !file.trashed)
                .cloned()
        } {
            return Ok(folder);
        }

        let mut url = Url::parse(&format!("{}/files", self.config.drive_url))
            .map_err(|error| Error::Other(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("supportsAllDrives", "true")
            .append_pair("fields", DRIVE_FILE_FIELDS);
        let folder = self
            .request(
                Method::POST,
                url,
                Some(json!({
                    "name": MILIM_DRIVE_FOLDER_NAME,
                    "mimeType": GOOGLE_FOLDER_MIME,
                })),
            )
            .await?
            .json::<DriveFile>()
            .await
            .map_err(upstream)?
            .summary(true);
        let mut stored = self.stored.write().await;
        stored.managed_folder_id = Some(folder.id.clone());
        stored.files.insert(folder.id.clone(), folder.clone());
        self.store.save(&stored)?;
        Ok(folder)
    }

    pub async fn list_files(&self, include_trashed: bool) -> Vec<GoogleFileSummary> {
        self.stored
            .read()
            .await
            .files
            .values()
            .filter(|file| include_trashed || !file.trashed)
            .cloned()
            .collect()
    }

    pub async fn preview(&self, id: &str, range: Option<&str>) -> Result<Value> {
        let file = self.refresh_file(id).await?;
        match file.mime_type.as_str() {
            GOOGLE_SHEET_MIME => self.sheet_preview(file, range).await,
            GOOGLE_DOC_MIME => self.doc_preview(file).await,
            GOOGLE_SLIDE_MIME => self.slide_preview(file).await,
            GOOGLE_FOLDER_MIME => self.folder_preview(file).await,
            mime if is_text_mime(mime) => self.text_preview(file).await,
            mime if mime.starts_with("image/") => Ok(json!({ "kind": "image", "file": file })),
            "application/pdf" => Ok(json!({ "kind": "pdf", "file": file })),
            mime if mime.starts_with("audio/") => Ok(json!({ "kind": "audio", "file": file })),
            mime if mime.starts_with("video/") => Ok(json!({ "kind": "video", "file": file })),
            _ => Ok(json!({ "kind": "unsupported", "file": file })),
        }
    }

    async fn read_for_model(&self, id: &str, range: Option<&str>) -> Result<Value> {
        let preview = self.preview(id, range).await?;
        if preview.get("kind").and_then(Value::as_str) != Some("document") {
            return Ok(preview);
        }
        Ok(json!({
            "kind": "document",
            "file": preview.get("file").cloned().unwrap_or(Value::Null),
            "content": google_doc_model_content(
                preview.get("document").unwrap_or(&Value::Null),
                MODEL_TEXT_LIMIT,
            ),
        }))
    }

    async fn sheet_preview(
        &self,
        file: GoogleFileSummary,
        requested_range: Option<&str>,
    ) -> Result<Value> {
        let spreadsheet_url = Url::parse(&format!(
            "{}/spreadsheets/{}",
            self.config.sheets_url, file.id
        ))
        .map_err(|error| Error::Other(error.to_string()))?;
        let metadata = self.request(Method::GET, spreadsheet_url, None).await?;
        let metadata = response_json_bounded(metadata, PREVIEW_BYTE_LIMIT).await?;
        let first_title = metadata
            .get("sheets")
            .and_then(Value::as_array)
            .and_then(|sheets| sheets.first())
            .and_then(|sheet| sheet.pointer("/properties/title"))
            .and_then(Value::as_str)
            .unwrap_or("Sheet1");
        let range = requested_range
            .filter(|range| !range.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("'{}'!A1:Z200", first_title.replace('\'', "''")));
        validate_sheet_range(&range)?;
        let mut values_url = Url::parse(&format!(
            "{}/spreadsheets/{}/values/{}",
            self.config.sheets_url,
            file.id,
            encode_path_segment(&range)
        ))
        .map_err(|error| Error::Other(error.to_string()))?;
        values_url
            .query_pairs_mut()
            .append_pair("majorDimension", "ROWS");
        let mut formulas_url = values_url.clone();
        values_url
            .query_pairs_mut()
            .append_pair("valueRenderOption", "FORMATTED_VALUE");
        formulas_url
            .query_pairs_mut()
            .append_pair("valueRenderOption", "FORMULA");
        let values = self.request(Method::GET, values_url, None).await?;
        let values = response_json_bounded(values, PREVIEW_BYTE_LIMIT).await?;
        let formulas = self.request(Method::GET, formulas_url, None).await?;
        let formulas = response_json_bounded(formulas, PREVIEW_BYTE_LIMIT).await?;
        enforce_sheet_cell_limit(values.get("values"), SHEET_READ_CELL_LIMIT)?;
        enforce_sheet_cell_limit(formulas.get("values"), SHEET_READ_CELL_LIMIT)?;
        Ok(json!({
            "kind": "sheet",
            "file": file,
            "range": range,
            "sheets": metadata.get("sheets").cloned().unwrap_or_else(|| json!([])),
            "values": values.get("values").cloned().unwrap_or_else(|| json!([])),
            "formulas": formulas.get("values").cloned().unwrap_or_else(|| json!([])),
        }))
    }

    async fn doc_preview(&self, file: GoogleFileSummary) -> Result<Value> {
        let url = Url::parse(&format!("{}/documents/{}", self.config.docs_url, file.id))
            .map_err(|error| Error::Other(error.to_string()))?;
        let document = self.request(Method::GET, url, None).await?;
        let document = response_json_bounded(document, PREVIEW_BYTE_LIMIT).await?;
        let text = truncate_chars(&extract_google_text(&document), MODEL_TEXT_LIMIT);
        Ok(json!({ "kind": "document", "file": file, "document": document, "text": text }))
    }

    async fn slide_preview(&self, file: GoogleFileSummary) -> Result<Value> {
        let url = Url::parse(&format!(
            "{}/presentations/{}",
            self.config.slides_url, file.id
        ))
        .map_err(|error| Error::Other(error.to_string()))?;
        let presentation = self.request(Method::GET, url, None).await?;
        let presentation = response_json_bounded(presentation, PREVIEW_BYTE_LIMIT).await?;
        let page_size = google_slide_page_size(&presentation);
        let mut remaining = MODEL_TEXT_LIMIT;
        let mut slides = Vec::new();
        for slide in presentation
            .get("slides")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if remaining == 0 {
                break;
            }
            slides.push(google_slide_preview_item(slide, &mut remaining, page_size));
        }
        Ok(json!({
            "kind": "presentation",
            "file": file,
            "title": presentation.get("title"),
            "pageAspectRatio": page_size.map(|(width, height)| width / height),
            "slides": slides,
        }))
    }

    async fn folder_preview(&self, file: GoogleFileSummary) -> Result<Value> {
        let descendants = self.sync_folder_descendants(&file).await?;
        let mut children = descendants
            .into_iter()
            .filter(|candidate| candidate.parents.iter().any(|parent| parent == &file.id))
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(json!({ "kind": "folder", "file": file, "children": children }))
    }

    async fn text_preview(&self, file: GoogleFileSummary) -> Result<Value> {
        let bytes = self.download_bytes(&file, None, PREVIEW_BYTE_LIMIT).await?;
        let text = String::from_utf8(bytes)
            .map_err(|_| Error::InvalidRequest("Drive file is not valid UTF-8 text".into()))?;
        Ok(json!({
            "kind": "text",
            "file": file,
            "text": truncate_chars(&text, MODEL_TEXT_LIMIT),
            "truncated": text.chars().count() > MODEL_TEXT_LIMIT,
        }))
    }

    pub async fn content(&self, id: &str, slide_id: Option<&str>) -> Result<(String, Vec<u8>)> {
        let file = self.refresh_file(id).await?;
        if let Some(slide_id) = slide_id {
            validate_google_id(slide_id)?;
            if file.mime_type != GOOGLE_SLIDE_MIME {
                return Err(Error::InvalidRequest(
                    "Slide thumbnails require a Google Slides file".into(),
                ));
            }
            let url = Url::parse(&format!(
                "{}/presentations/{}/pages/{}/thumbnail",
                self.config.slides_url, file.id, slide_id
            ))
            .map_err(|error| Error::Other(error.to_string()))?;
            let thumbnail = self
                .request(Method::GET, url, None)
                .await?
                .json::<Value>()
                .await
                .map_err(upstream)?;
            let content_url = thumbnail
                .get("contentUrl")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Upstream("Google returned no slide thumbnail".into()))?;
            let response = self
                .client
                .get(content_url)
                .send()
                .await
                .map_err(upstream)?;
            let bytes =
                read_response_bounded(ensure_google_response(response).await?, PREVIEW_BYTE_LIMIT)
                    .await?;
            return Ok(("image/png".into(), bytes));
        }
        if !file.capabilities.can_download {
            return Err(Error::InvalidRequest(
                "Google does not allow this file to be downloaded".into(),
            ));
        }
        let mime = file.mime_type.clone();
        let bytes = self.download_bytes(&file, None, PREVIEW_BYTE_LIMIT).await?;
        Ok((mime, bytes))
    }

    async fn download_bytes(
        &self,
        file: &GoogleFileSummary,
        export_mime: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let mut url = if file.mime_type.starts_with("application/vnd.google-apps.")
            && file.mime_type != GOOGLE_FOLDER_MIME
        {
            let export_mime = export_mime.ok_or_else(|| {
                Error::InvalidRequest("Choose an export format for Google Workspace files".into())
            })?;
            let mut url = Url::parse(&format!(
                "{}/files/{}/export",
                self.config.drive_url, file.id
            ))
            .map_err(|error| Error::Other(error.to_string()))?;
            url.query_pairs_mut().append_pair("mimeType", export_mime);
            url
        } else {
            let mut url = Url::parse(&format!("{}/files/{}", self.config.drive_url, file.id))
                .map_err(|error| Error::Other(error.to_string()))?;
            url.query_pairs_mut().append_pair("alt", "media");
            url
        };
        url.query_pairs_mut()
            .append_pair("supportsAllDrives", "true");
        read_response_bounded(self.request(Method::GET, url, None).await?, limit).await
    }

    fn workspace_root(&self) -> Result<PathBuf> {
        self.workspace
            .read()
            .map_err(|_| Error::Other("workspace lock poisoned".into()))?
            .clone()
            .ok_or_else(|| Error::InvalidRequest("Select a Milim workspace folder first".into()))
    }
}

const DRIVE_FILE_FIELDS: &str = "id,name,mimeType,webViewLink,iconLink,modifiedTime,size,trashed,parents,driveId,capabilities(canEdit,canDownload,canMoveItemWithinDrive,canMoveItemOutOfDrive,canRename,canShare,canTrash)";

fn unavailable_error() -> Error {
    Error::InvalidRequest("This build does not include Milim's Google OAuth credentials".into())
}

fn random_token() -> String {
    Uuid::new_v4().simple().to_string()
}

fn loopback_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn upstream(error: impl std::fmt::Display) -> Error {
    Error::Upstream(error.to_string())
}

async fn ensure_google_response(response: reqwest::Response) -> Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let detail = truncate_chars(&body, 2_000);
    if status == StatusCode::UNAUTHORIZED {
        return Err(Error::Unauthorized(
            "Google authorization expired; reconnect Google Workspace".into(),
        ));
    }
    if status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")))
    {
        return Err(Error::Upstream(
            "Google Workspace rate limit reached; retry later".into(),
        ));
    }
    if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
        return Err(Error::InvalidRequest(format!(
            "Google denied access to this selected file ({status}): {detail}"
        )));
    }
    Err(Error::Upstream(format!(
        "Google API returned {status}: {detail}"
    )))
}

async fn read_response_bounded(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::InvalidRequest(format!(
            "Google file exceeds Milim's {} MB limit",
            limit / 1024 / 1024
        )));
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or_default()
            .min(limit as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.map_err(upstream)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(Error::InvalidRequest(format!(
                "Google file exceeds Milim's {} MB limit",
                limit / 1024 / 1024
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn response_json_bounded(response: reqwest::Response, limit: usize) -> Result<Value> {
    serde_json::from_slice(&read_response_bounded(response, limit).await?).map_err(Into::into)
}

fn validate_google_id(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidRequest("Invalid Google resource id".into()))
    }
}

fn validate_sheet_range(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 500 || value.contains(['\r', '\n', '\0']) {
        return Err(Error::InvalidRequest("Invalid Google Sheets range".into()));
    }
    Ok(())
}

fn a1_cell_count(value: &str) -> Result<usize> {
    let cells = value
        .rsplit_once('!')
        .map(|(_, cells)| cells)
        .unwrap_or(value);
    let mut endpoints = cells.split(':');
    let start = parse_a1_cell(endpoints.next().unwrap_or_default())?;
    let end = match endpoints.next() {
        Some(value) => parse_a1_cell(value)?,
        None => start,
    };
    if endpoints.next().is_some() || end.0 < start.0 || end.1 < start.1 {
        return Err(Error::InvalidRequest(
            "Use a bounded A1 cell range such as Sheet1!A1:C20".into(),
        ));
    }
    (end.0 - start.0 + 1)
        .checked_mul(end.1 - start.1 + 1)
        .ok_or_else(|| Error::InvalidRequest("Google Sheets range is too large".into()))
}

fn parse_a1_cell(value: &str) -> Result<(usize, usize)> {
    let value = value.trim().replace('$', "");
    let split = value
        .find(|character: char| character.is_ascii_digit())
        .ok_or_else(|| Error::InvalidRequest("Google Sheets range must use A1 cells".into()))?;
    let (column, row) = value.split_at(split);
    if column.is_empty()
        || row.is_empty()
        || !column.bytes().all(|byte| byte.is_ascii_alphabetic())
        || !row.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Error::InvalidRequest(
            "Google Sheets range must use A1 cells".into(),
        ));
    }
    let column = column.bytes().try_fold(0usize, |value, byte| {
        value
            .checked_mul(26)
            .and_then(|value| value.checked_add((byte.to_ascii_uppercase() - b'A' + 1) as usize))
            .ok_or_else(|| Error::InvalidRequest("Google Sheets column is too large".into()))
    })?;
    let row = row
        .parse::<usize>()
        .ok()
        .filter(|row| *row > 0)
        .ok_or_else(|| Error::InvalidRequest("Google Sheets row must be positive".into()))?;
    Ok((column, row))
}

fn encode_path_segment(value: &str) -> String {
    let mut url = Url::parse("https://milim.invalid/").expect("valid base URL");
    url.path_segments_mut()
        .expect("base URL supports path segments")
        .push(value);
    url.path().trim_start_matches('/').to_string()
}

fn enforce_sheet_cell_limit(values: Option<&Value>, limit: usize) -> Result<()> {
    let cells = values
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_array().map(Vec::len).unwrap_or(0))
                .sum()
        })
        .unwrap_or(0);
    if cells > limit {
        Err(Error::InvalidRequest(format!(
            "Google Sheets request exceeds the {limit}-cell limit"
        )))
    } else {
        Ok(())
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn take_chars_from_budget(value: &str, remaining: &mut usize) -> String {
    let text = truncate_chars(value, *remaining);
    *remaining = remaining.saturating_sub(text.chars().count());
    text
}

fn google_slide_preview_item(
    slide: &Value,
    remaining: &mut usize,
    page_size: Option<(f64, f64)>,
) -> Value {
    let text_elements = google_slide_text_elements(
        slide.get("pageElements").unwrap_or(&Value::Null),
        remaining,
        page_size,
    );
    let notes_element = google_slide_notes_element(
        slide
            .pointer("/slideProperties/notesPage")
            .unwrap_or(&Value::Null),
        remaining,
    );
    json!({
        "objectId": slide.get("objectId"),
        "text": text_elements.iter()
            .filter_map(|element| element.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        "textElements": text_elements,
        "notes": notes_element.as_ref()
            .and_then(|element| element.get("text"))
            .cloned()
            .unwrap_or_else(|| json!("")),
        "notesObjectId": notes_element.as_ref()
            .and_then(|element| element.get("objectId"))
            .cloned(),
    })
}

fn google_slide_text_elements(
    value: &Value,
    remaining: &mut usize,
    page_size: Option<(f64, f64)>,
) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|element| {
            let object_id = element.get("objectId")?.as_str()?;
            let text = element.pointer("/shape/text")?;
            let rect =
                page_size.and_then(|page_size| google_slide_element_rect(element, page_size));
            Some(json!({
                "objectId": object_id,
                "text": take_chars_from_budget(&extract_google_text(text), remaining),
                "styleRuns": google_slide_style_runs(text, "textRun"),
                "paragraphRuns": google_slide_style_runs(text, "paragraphMarker"),
                "x": rect.map(|value| value.0),
                "y": rect.map(|value| value.1),
                "width": rect.map(|value| value.2),
                "height": rect.map(|value| value.3),
            }))
        })
        .take(49)
        .collect()
}

fn google_slide_style_runs(text: &Value, kind: &str) -> Vec<Value> {
    text.get("textElements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|element| {
            let style = element.get(kind)?.get("style")?;
            Some(json!({
                "start": element.get("startIndex").and_then(Value::as_i64).unwrap_or(0),
                "end": element.get("endIndex").and_then(Value::as_i64).unwrap_or(0),
                "style": style,
            }))
        })
        .take(200)
        .collect()
}

fn google_slide_page_size(presentation: &Value) -> Option<(f64, f64)> {
    let width = google_slide_dimension(presentation.pointer("/pageSize/width")?)?;
    let height = google_slide_dimension(presentation.pointer("/pageSize/height")?)?;
    (width > 0.0 && height > 0.0).then_some((width, height))
}

fn google_slide_dimension(value: &Value) -> Option<f64> {
    let magnitude = value.get("magnitude")?.as_f64()?;
    Some(
        match value.get("unit").and_then(Value::as_str).unwrap_or("EMU") {
            "PT" => magnitude * 12_700.0,
            "PX" => magnitude * 9_525.0,
            _ => magnitude,
        },
    )
}

fn google_slide_element_rect(
    element: &Value,
    (page_width, page_height): (f64, f64),
) -> Option<(f64, f64, f64, f64)> {
    let width = google_slide_dimension(element.pointer("/size/width")?)?;
    let height = google_slide_dimension(element.pointer("/size/height")?)?;
    let transform = element.get("transform").unwrap_or(&Value::Null);
    let translate_x = transform
        .get("translateX")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let translate_y = transform
        .get("translateY")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let unit = transform
        .get("unit")
        .and_then(Value::as_str)
        .unwrap_or("EMU");
    let unit_scale = match unit {
        "PT" => 12_700.0,
        "PX" => 9_525.0,
        _ => 1.0,
    };
    if transform
        .get("shearX")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .abs()
        > f64::EPSILON
        || transform
            .get("shearY")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .abs()
            > f64::EPSILON
    {
        return None;
    }
    let scale_x = transform
        .get("scaleX")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .abs();
    let scale_y = transform
        .get("scaleY")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .abs();
    // ponytail: axis-aligned overlays; use affine corners if rotated text editing is needed.
    Some((
        (translate_x * unit_scale / page_width).clamp(0.0, 1.0),
        (translate_y * unit_scale / page_height).clamp(0.0, 1.0),
        (width * scale_x / page_width).clamp(0.01, 1.0),
        (height * scale_y / page_height).clamp(0.01, 1.0),
    ))
}

fn google_slide_notes_element(value: &Value, remaining: &mut usize) -> Option<Value> {
    let elements = value
        .get("pageElements")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let speaker_id = value
        .pointer("/notesProperties/speakerNotesObjectId")
        .and_then(Value::as_str);
    let element = speaker_id
        .and_then(|id| {
            elements
                .iter()
                .find(|element| element.get("objectId").and_then(Value::as_str) == Some(id))
        })
        .or_else(|| {
            elements.iter().find(|element| {
                element
                    .pointer("/shape/placeholder/type")
                    .and_then(Value::as_str)
                    == Some("BODY")
            })
        })
        .or_else(|| {
            elements
                .iter()
                .find(|element| element.pointer("/shape/text").is_some())
        });
    let object_id = speaker_id
        .or_else(|| element.and_then(|element| element.get("objectId").and_then(Value::as_str)))?;
    let text = element
        .and_then(|element| element.pointer("/shape/text"))
        .map(extract_google_text)
        .unwrap_or_default();
    Some(json!({
        "objectId": object_id,
        "text": take_chars_from_budget(&text, remaining),
    }))
}

fn google_doc_model_content(document: &Value, limit: usize) -> Vec<Value> {
    let mut remaining = limit;
    let mut blocks = Vec::new();
    for item in document
        .pointer("/body/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if remaining == 0 {
            break;
        }
        if let Some(paragraph) = item.get("paragraph") {
            let mut runs = Vec::new();
            for element in paragraph
                .get("elements")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if remaining == 0 {
                    break;
                }
                if let Some(run) = element.get("textRun") {
                    let text = run
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|text| take_chars_from_budget(text, &mut remaining))
                        .unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    let style = run.get("textStyle").cloned().unwrap_or_else(|| json!({}));
                    runs.push(json!({
                        "start_index": element.get("startIndex"),
                        "end_index": element.get("endIndex"),
                        "text": text,
                        "style": {
                            "bold": style.get("bold").and_then(Value::as_bool).unwrap_or(false),
                            "italic": style.get("italic").and_then(Value::as_bool).unwrap_or(false),
                            "underline": style.get("underline").and_then(Value::as_bool).unwrap_or(false),
                        }
                    }));
                } else if let Some(id) = element
                    .pointer("/inlineObjectElement/inlineObjectId")
                    .and_then(Value::as_str)
                {
                    runs.push(json!({ "inline_object_id": id }));
                } else if element.get("pageBreak").is_some() {
                    runs.push(json!({ "page_break": true }));
                }
            }
            blocks.push(json!({
                "kind": "paragraph",
                "start_index": item.get("startIndex"),
                "end_index": item.get("endIndex"),
                "named_style_type": paragraph.pointer("/paragraphStyle/namedStyleType"),
                "bullet": paragraph.get("bullet"),
                "runs": runs,
            }));
        } else if let Some(table) = item.get("table") {
            let rows = table
                .get("tableRows")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .map(|row| {
                            row.get("tableCells")
                                .and_then(Value::as_array)
                                .map(|cells| {
                                    cells
                                        .iter()
                                        .map(|cell| {
                                            take_chars_from_budget(
                                                &extract_google_text(cell),
                                                &mut remaining,
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            blocks.push(json!({
                "kind": "table",
                "start_index": item.get("startIndex"),
                "end_index": item.get("endIndex"),
                "rows": rows,
            }));
        }
    }
    blocks
}

fn extract_google_text(value: &Value) -> String {
    fn walk(value: &Value, output: &mut String) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    if key == "content" {
                        if let Some(text) = child.as_str() {
                            output.push_str(text);
                            continue;
                        }
                    }
                    walk(child, output);
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, output);
                }
            }
            _ => {}
        }
    }
    let mut output = String::new();
    walk(value, &mut output);
    output
}

fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/xml"
                | "application/javascript"
                | "application/x-yaml"
        )
}

#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    picked_file_ids: Option<String>,
}

impl OAuthCallbackQuery {
    fn picked_file_ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        for id in self
            .picked_file_ids
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            validate_google_id(id)?;
            if seen.insert(id.to_string()) {
                ids.push(id.to_string());
            }
        }
        if ids.len() > 100 {
            return Err(Error::InvalidRequest(
                "Google Picker accepts at most 100 selected files".into(),
            ));
        }
        Ok(ids)
    }
}

struct PickerRelay {
    expected_state: String,
    callback: StdMutex<Option<oneshot::Sender<OAuthCallbackQuery>>>,
}

async fn oauth_callback(
    State(relay): State<Arc<PickerRelay>>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    if query.state.as_deref() != Some(relay.expected_state.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            Html("<h1>Milim rejected this Google callback.</h1><p>You can close this tab.</p>"),
        )
            .into_response();
    }
    let sender = relay
        .callback
        .lock()
        .ok()
        .and_then(|mut sender| sender.take());
    let Some(sender) = sender else {
        return (
            StatusCode::CONFLICT,
            Html("<h1>This Google callback was already used.</h1><p>You can close this tab.</p>"),
        )
            .into_response();
    };
    let cancelled = query.error.is_some();
    if sender.send(query).is_err() {
        return (
            StatusCode::GONE,
            Html("<h1>This Google Picker flow expired.</h1><p>You can close this tab.</p>"),
        )
            .into_response();
    }
    Html(if cancelled {
        "<h1>Google selection cancelled.</h1><p>You can close this tab and return to Milim.</p>"
    } else {
        "<h1>Files selected for Milim.</h1><p>You can close this tab and return to Milim.</p>"
    })
    .into_response()
}

type ApiResult<T> = std::result::Result<T, ApiError>;

fn connection(st: &AppState) -> ApiResult<Arc<GoogleWorkspaceConnection>> {
    st.google_workspace
        .clone()
        .ok_or_else(|| ApiError(unavailable_error()))
}

type Peer = ConnectInfo<std::net::SocketAddr>;

pub(crate) async fn status_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> ApiResult<Json<GoogleWorkspaceStatus>> {
    authorize(&st, &headers, Some(peer.0))?;
    Ok(Json(connection(&st)?.status().await))
}

#[derive(Default, Deserialize)]
pub(crate) struct StartPickerRequest {
    #[serde(default)]
    file_ids: Vec<String>,
}

pub(crate) async fn picker_start_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    Json(request): Json<StartPickerRequest>,
) -> ApiResult<Json<GooglePickerFlow>> {
    authorize(&st, &headers, Some(peer.0))?;
    if request.file_ids.len() > 100 {
        return Err(ApiError(Error::InvalidRequest(
            "Google Picker accepts at most 100 requested file ids".into(),
        )));
    }
    for id in &request.file_ids {
        validate_google_id(id).map_err(ApiError)?;
    }
    Ok(Json(
        connection(&st)?
            .start_picker(request.file_ids)
            .await
            .map_err(ApiError)?,
    ))
}

pub(crate) async fn picker_status_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<Json<GooglePickerFlow>> {
    authorize(&st, &headers, Some(peer.0))?;
    connection(&st)?
        .picker_flow(&id)
        .await
        .map(Json)
        .ok_or_else(|| ApiError(Error::ModelNotFound(format!("Google Picker flow {id}"))))
}

pub(crate) async fn file_list_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> ApiResult<Json<Value>> {
    authorize(&st, &headers, Some(peer.0))?;
    Ok(Json(
        json!({ "files": connection(&st)?.list_files(true).await }),
    ))
}

pub(crate) async fn file_remove_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
) -> ApiResult<StatusCode> {
    authorize(&st, &headers, Some(peer.0))?;
    if connection(&st)?.remove_file(&id).await.map_err(ApiError)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError(Error::ModelNotFound(format!(
            "Google Drive file {id}"
        ))))
    }
}

pub(crate) async fn disconnect_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
) -> ApiResult<Json<GoogleDisconnectResult>> {
    authorize(&st, &headers, Some(peer.0))?;
    connection(&st)?
        .disconnect()
        .await
        .map(Json)
        .map_err(ApiError)
}

#[derive(Default, Deserialize)]
pub(crate) struct PreviewQuery {
    range: Option<String>,
}

pub(crate) async fn preview_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PreviewQuery>,
) -> ApiResult<Json<Value>> {
    authorize(&st, &headers, Some(peer.0))?;
    Ok(Json(
        connection(&st)?
            .preview(&id, query.range.as_deref())
            .await
            .map_err(ApiError)?,
    ))
}

#[derive(Default, Deserialize)]
pub(crate) struct ContentQuery {
    slide_id: Option<String>,
}

pub(crate) async fn content_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ContentQuery>,
) -> ApiResult<Response> {
    authorize(&st, &headers, Some(peer.0))?;
    let (mime, bytes) = connection(&st)?
        .content(&id, query.slide_id.as_deref())
        .await
        .map_err(ApiError)?;
    Ok((
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            mime.parse::<axum::http::HeaderValue>().unwrap_or_else(|_| {
                axum::http::HeaderValue::from_static("application/octet-stream")
            }),
        )],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceEditRequest {
    operations: Vec<Value>,
}

pub(crate) async fn sheet_edit_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<WorkspaceEditRequest>,
) -> ApiResult<Json<Value>> {
    authorize(&st, &headers, Some(peer.0))?;
    GoogleSheetsEditTool(connection(&st)?)
        .invoke(json!({ "file_id": id, "operations": request.operations }))
        .await
        .map(Json)
        .map_err(ApiError)
}

pub(crate) async fn doc_edit_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<WorkspaceEditRequest>,
) -> ApiResult<Json<Value>> {
    authorize(&st, &headers, Some(peer.0))?;
    GoogleDocsEditTool(connection(&st)?)
        .invoke(json!({ "file_id": id, "operations": request.operations }))
        .await
        .map(Json)
        .map_err(ApiError)
}

pub(crate) async fn slide_edit_route(
    State(st): State<AppState>,
    headers: HeaderMap,
    peer: Peer,
    AxumPath(id): AxumPath<String>,
    Json(request): Json<WorkspaceEditRequest>,
) -> ApiResult<Json<Value>> {
    authorize(&st, &headers, Some(peer.0))?;
    GoogleSlidesEditTool(connection(&st)?)
        .invoke(json!({ "file_id": id, "operations": request.operations }))
        .await
        .map(Json)
        .map_err(ApiError)
}

pub fn tools(connection: Arc<GoogleWorkspaceConnection>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(GoogleDriveListTool(connection.clone())),
        Arc::new(GoogleDriveReadTool(connection.clone())),
        Arc::new(GoogleDriveManageTool(connection.clone())),
        Arc::new(GoogleDriveTransferTool(connection.clone())),
        Arc::new(GoogleSheetsReadTool(connection.clone())),
        Arc::new(GoogleSheetsEditTool(connection.clone())),
        Arc::new(GoogleDocsReadTool(connection.clone())),
        Arc::new(GoogleDocsEditTool(connection.clone())),
        Arc::new(GoogleSlidesReadTool(connection.clone())),
        Arc::new(GoogleSlidesEditTool(connection)),
    ]
}

struct GoogleDriveListTool(Arc<GoogleWorkspaceConnection>);
struct GoogleDriveReadTool(Arc<GoogleWorkspaceConnection>);
struct GoogleDriveManageTool(Arc<GoogleWorkspaceConnection>);
struct GoogleDriveTransferTool(Arc<GoogleWorkspaceConnection>);
struct GoogleSheetsReadTool(Arc<GoogleWorkspaceConnection>);
struct GoogleSheetsEditTool(Arc<GoogleWorkspaceConnection>);
struct GoogleDocsReadTool(Arc<GoogleWorkspaceConnection>);
struct GoogleDocsEditTool(Arc<GoogleWorkspaceConnection>);
struct GoogleSlidesReadTool(Arc<GoogleWorkspaceConnection>);
struct GoogleSlidesEditTool(Arc<GoogleWorkspaceConnection>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    include_trashed: bool,
}

#[async_trait]
impl Tool for GoogleDriveListTool {
    fn name(&self) -> &str {
        "google_drive_list"
    }

    fn description(&self) -> &str {
        "List Google Drive files explicitly selected for Milim or created by Milim."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "include_trashed": { "type": "boolean", "default": false }
            },
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ListArgs = serde_json::from_value(args)
            .map_err(|error| Error::InvalidRequest(error.to_string()))?;
        Ok(json!({ "files": self.0.list_files(args.include_trashed).await }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    file_id: String,
    #[serde(default)]
    range: Option<String>,
}

#[async_trait]
impl Tool for GoogleDriveReadTool {
    fn name(&self) -> &str {
        "google_drive_read"
    }

    fn description(&self) -> &str {
        "Read bounded metadata or textual content from one Google Drive file authorized for Milim."
    }

    fn input_schema(&self) -> Value {
        read_schema(false)
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ReadArgs = parse_args(args)?;
        self.0
            .read_for_model(&args.file_id, args.range.as_deref())
            .await
    }
}

#[async_trait]
impl Tool for GoogleSheetsReadTool {
    fn name(&self) -> &str {
        "google_sheets_read"
    }

    fn description(&self) -> &str {
        "Read up to 10,000 cells, formulas, and worksheet metadata from an authorized Google Sheet."
    }

    fn input_schema(&self) -> Value {
        read_schema(true)
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ReadArgs = parse_args(args)?;
        let file = self.0.authorized_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_SHEET_MIME)?;
        self.0.preview(&args.file_id, args.range.as_deref()).await
    }
}

#[async_trait]
impl Tool for GoogleDocsReadTool {
    fn name(&self) -> &str {
        "google_docs_read"
    }

    fn description(&self) -> &str {
        "Read bounded structured content and text from an authorized Google Doc."
    }

    fn input_schema(&self) -> Value {
        read_schema(false)
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ReadArgs = parse_args(args)?;
        let file = self.0.authorized_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_DOC_MIME)?;
        self.0.read_for_model(&args.file_id, None).await
    }
}

#[async_trait]
impl Tool for GoogleSlidesReadTool {
    fn name(&self) -> &str {
        "google_slides_read"
    }

    fn description(&self) -> &str {
        "Read slide ids and bounded text from an authorized Google Slides presentation."
    }

    fn input_schema(&self) -> Value {
        read_schema(false)
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: ReadArgs = parse_args(args)?;
        let file = self.0.authorized_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_SLIDE_MIME)?;
        self.0.preview(&args.file_id, None).await
    }
}

fn read_schema(with_range: bool) -> Value {
    let mut properties = json!({
        "file_id": { "type": "string", "description": "Id returned by google_drive_list." }
    });
    if with_range {
        properties["range"] = json!({
            "type": "string",
            "description": "Optional A1 range. Defaults to the first worksheet A1:Z200."
        });
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["file_id"],
        "additionalProperties": false
    })
}

fn parse_args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| Error::InvalidRequest(error.to_string()))
}

fn require_mime(file: &GoogleFileSummary, mime: &str) -> Result<()> {
    if file.mime_type == mime {
        Ok(())
    } else {
        Err(Error::InvalidRequest(format!(
            "{} is not a supported {} file",
            file.name, mime
        )))
    }
}

// Mutating tool implementations are below so all Google API requests continue
// through the same selected-file and token boundary.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriveManageArgs {
    action: String,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    principal_type: Option<String>,
    #[serde(default)]
    role: Option<String>,
}

#[async_trait]
impl Tool for GoogleDriveManageTool {
    fn name(&self) -> &str {
        "google_drive_manage"
    }

    fn description(&self) -> &str {
        "Create, rename, move, trash, restore, or manage named-recipient access for Milim-authorized Drive files. Creates without a parent use Milim's managed Drive folder."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "rename", "move", "trash", "restore", "share", "unshare"]
                },
                "file_id": { "type": "string" },
                "name": { "type": "string" },
                "mime_type": {
                    "type": "string",
                    "enum": [
                        GOOGLE_FOLDER_MIME,
                        GOOGLE_DOC_MIME,
                        GOOGLE_SHEET_MIME,
                        GOOGLE_SLIDE_MIME
                    ]
                },
                "parent_id": { "type": "string" },
                "email": { "type": "string" },
                "principal_type": { "type": "string", "enum": ["user", "group"] },
                "role": { "type": "string", "enum": ["reader", "commenter", "writer"] }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: DriveManageArgs = parse_args(args)?;
        match args.action.as_str() {
            "create" => {
                let name = required_text(args.name.as_deref(), "name", 240)?;
                let mime = args.mime_type.as_deref().unwrap_or(GOOGLE_FOLDER_MIME);
                if !matches!(
                    mime,
                    GOOGLE_FOLDER_MIME | GOOGLE_DOC_MIME | GOOGLE_SHEET_MIME | GOOGLE_SLIDE_MIME
                ) {
                    return Err(Error::InvalidRequest(
                        "create supports folders, Docs, Sheets, and Slides".into(),
                    ));
                }
                let mut metadata = json!({ "name": name, "mimeType": mime });
                let parent = match args.parent_id.as_deref() {
                    Some(parent_id) => {
                        let parent = self.0.authorized_file(parent_id).await?;
                        require_mime(&parent, GOOGLE_FOLDER_MIME)?;
                        parent
                    }
                    None => self.0.managed_folder().await?,
                };
                metadata["parents"] = json!([parent.id]);
                let mut url = Url::parse(&format!("{}/files", self.0.config.drive_url))
                    .map_err(|error| Error::Other(error.to_string()))?;
                url.query_pairs_mut()
                    .append_pair("supportsAllDrives", "true")
                    .append_pair("fields", DRIVE_FILE_FIELDS);
                let file = self
                    .0
                    .request(Method::POST, url, Some(metadata))
                    .await?
                    .json::<DriveFile>()
                    .await
                    .map_err(upstream)?;
                let file = file.summary(true);
                self.0.save_authorized_file(file.clone()).await?;
                Ok(json!({ "file": file }))
            }
            "rename" => {
                let id = required_id(args.file_id.as_deref())?;
                let name = required_text(args.name.as_deref(), "name", 240)?;
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_rename, "rename", &file)?;
                let file = self
                    .0
                    .patch_file(id, json!({ "name": name }), &[], &[])
                    .await?;
                Ok(json!({ "file": file }))
            }
            "move" => {
                let id = required_id(args.file_id.as_deref())?;
                let parent_id = required_id(args.parent_id.as_deref())?;
                if id == parent_id {
                    return Err(Error::InvalidRequest("A file cannot contain itself".into()));
                }
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_move, "move", &file)?;
                let parent = self.0.refresh_file(parent_id).await?;
                require_mime(&parent, GOOGLE_FOLDER_MIME)?;
                let remove = file.parents.clone();
                let moved = self
                    .0
                    .patch_file(id, json!({}), &[parent_id.to_string()], &remove)
                    .await?;
                Ok(json!({ "file": moved }))
            }
            "trash" | "restore" => {
                let id = required_id(args.file_id.as_deref())?;
                let file = self.0.refresh_file(id).await?;
                if args.action == "trash" {
                    require_capability(file.capabilities.can_trash, "trash", &file)?;
                } else {
                    require_capability(file.capabilities.can_edit, "restore", &file)?;
                }
                let file = self
                    .0
                    .patch_file(id, json!({ "trashed": args.action == "trash" }), &[], &[])
                    .await?;
                Ok(json!({ "file": file }))
            }
            "share" => {
                let id = required_id(args.file_id.as_deref())?;
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_share, "share", &file)?;
                let email = validate_email(args.email.as_deref())?;
                let role = validate_share_role(args.role.as_deref())?;
                let principal_type = args.principal_type.as_deref().unwrap_or("user");
                if !matches!(principal_type, "user" | "group") {
                    return Err(Error::InvalidRequest(
                        "principal_type must be user or group".into(),
                    ));
                }
                let mut url = Url::parse(&format!(
                    "{}/files/{id}/permissions",
                    self.0.config.drive_url
                ))
                .map_err(|error| Error::Other(error.to_string()))?;
                url.query_pairs_mut()
                    .append_pair("supportsAllDrives", "true")
                    .append_pair("sendNotificationEmail", "true")
                    .append_pair("fields", "id,emailAddress,role,type");
                let permission = self
                    .0
                    .request(
                        Method::POST,
                        url,
                        Some(json!({
                            "type": principal_type,
                            "role": role,
                            "emailAddress": email,
                        })),
                    )
                    .await?
                    .json::<Value>()
                    .await
                    .map_err(upstream)?;
                Ok(json!({ "permission": permission }))
            }
            "unshare" => {
                let id = required_id(args.file_id.as_deref())?;
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_share, "unshare", &file)?;
                let email = validate_email(args.email.as_deref())?;
                let mut list_url = Url::parse(&format!(
                    "{}/files/{id}/permissions",
                    self.0.config.drive_url
                ))
                .map_err(|error| Error::Other(error.to_string()))?;
                list_url
                    .query_pairs_mut()
                    .append_pair("supportsAllDrives", "true")
                    .append_pair("fields", "permissions(id,emailAddress,role,type)");
                let permissions = self
                    .0
                    .request(Method::GET, list_url, None)
                    .await?
                    .json::<Value>()
                    .await
                    .map_err(upstream)?;
                let permission = permissions
                    .get("permissions")
                    .and_then(Value::as_array)
                    .and_then(|permissions| {
                        permissions.iter().find(|permission| {
                            permission
                                .get("emailAddress")
                                .and_then(Value::as_str)
                                .is_some_and(|value| value.eq_ignore_ascii_case(email))
                                && permission.get("role").and_then(Value::as_str) != Some("owner")
                                && matches!(
                                    permission.get("type").and_then(Value::as_str),
                                    Some("user" | "group")
                                )
                        })
                    })
                    .ok_or_else(|| {
                        Error::ModelNotFound(format!("No removable named permission for {email}"))
                    })?;
                let permission_id = permission
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::Upstream("Google permission has no id".into()))?;
                validate_google_id(permission_id)?;
                let mut delete_url = Url::parse(&format!(
                    "{}/files/{id}/permissions/{permission_id}",
                    self.0.config.drive_url
                ))
                .map_err(|error| Error::Other(error.to_string()))?;
                delete_url
                    .query_pairs_mut()
                    .append_pair("supportsAllDrives", "true");
                self.0.request(Method::DELETE, delete_url, None).await?;
                Ok(json!({ "removed": true, "email": email }))
            }
            _ => Err(Error::InvalidRequest("Unknown Google Drive action".into())),
        }
    }
}

impl GoogleWorkspaceConnection {
    async fn patch_file(
        &self,
        id: &str,
        body: Value,
        add_parents: &[String],
        remove_parents: &[String],
    ) -> Result<GoogleFileSummary> {
        validate_google_id(id)?;
        let existing = self.authorized_file(id).await?;
        let mut url = Url::parse(&format!("{}/files/{id}", self.config.drive_url))
            .map_err(|error| Error::Other(error.to_string()))?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("supportsAllDrives", "true")
                .append_pair("fields", DRIVE_FILE_FIELDS);
            if !add_parents.is_empty() {
                query.append_pair("addParents", &add_parents.join(","));
            }
            if !remove_parents.is_empty() {
                query.append_pair("removeParents", &remove_parents.join(","));
            }
        }
        let mut file = self
            .request(Method::PATCH, url, Some(body))
            .await?
            .json::<DriveFile>()
            .await
            .map_err(upstream)?
            .summary(existing.created_by_milim);
        file.created_by_milim = existing.created_by_milim;
        self.save_authorized_file(file.clone()).await?;
        Ok(file)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DriveTransferArgs {
    action: String,
    workspace_path: String,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    export_mime_type: Option<String>,
    #[serde(default)]
    overwrite: bool,
}

#[async_trait]
impl Tool for GoogleDriveTransferTool {
    fn name(&self) -> &str {
        "google_drive_transfer"
    }

    fn description(&self) -> &str {
        "Upload, replace, download, or export an authorized Drive file using a path inside the selected Milim workspace. Uploads without a parent use Milim's managed Drive folder."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["upload", "replace", "download", "export"] },
                "workspace_path": { "type": "string", "description": "Workspace-relative source or destination path." },
                "file_id": { "type": "string" },
                "name": { "type": "string" },
                "parent_id": { "type": "string" },
                "mime_type": { "type": "string" },
                "export_mime_type": { "type": "string" },
                "overwrite": { "type": "boolean", "default": false }
            },
            "required": ["action", "workspace_path"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: DriveTransferArgs = parse_args(args)?;
        let root = self.0.workspace_root()?;
        let path = resolve_workspace_path(&root, &args.workspace_path)?;
        match args.action.as_str() {
            "upload" => {
                let bytes = read_bounded_file(&path, TRANSFER_BYTE_LIMIT)?;
                let name = args
                    .name
                    .as_deref()
                    .map(|value| required_text(Some(value), "name", 240))
                    .transpose()?
                    .map(str::to_string)
                    .or_else(|| {
                        path.file_name()
                            .and_then(|value| value.to_str())
                            .map(str::to_string)
                    })
                    .ok_or_else(|| Error::InvalidRequest("Upload requires a file name".into()))?;
                let mime = validate_mime(
                    args.mime_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                )?;
                let mut metadata = json!({ "name": name });
                let parent = match args.parent_id.as_deref() {
                    Some(parent_id) => {
                        let parent = self.0.authorized_file(parent_id).await?;
                        require_mime(&parent, GOOGLE_FOLDER_MIME)?;
                        parent
                    }
                    None => self.0.managed_folder().await?,
                };
                metadata["parents"] = json!([parent.id]);
                let file = self.0.resumable_upload(None, metadata, mime, bytes).await?;
                Ok(json!({ "file": file }))
            }
            "replace" => {
                let id = required_id(args.file_id.as_deref())?;
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_edit, "replace", &file)?;
                if file.mime_type.starts_with("application/vnd.google-apps.") {
                    return Err(Error::InvalidRequest(
                        "Use the Docs, Sheets, or Slides edit tool for Workspace files".into(),
                    ));
                }
                let bytes = read_bounded_file(&path, TRANSFER_BYTE_LIMIT)?;
                let mime =
                    validate_mime(args.mime_type.as_deref().unwrap_or(file.mime_type.as_str()))?;
                let file = self
                    .0
                    .resumable_upload(Some(id), json!({}), mime, bytes)
                    .await?;
                Ok(json!({ "file": file }))
            }
            "download" | "export" => {
                let id = required_id(args.file_id.as_deref())?;
                let file = self.0.refresh_file(id).await?;
                require_capability(file.capabilities.can_download, "download", &file)?;
                if path.exists() && !args.overwrite {
                    return Err(Error::InvalidRequest(format!(
                        "{} already exists; set overwrite to true",
                        args.workspace_path
                    )));
                }
                let export_mime = if args.action == "export" {
                    Some(validate_mime(
                        args.export_mime_type.as_deref().ok_or_else(|| {
                            Error::InvalidRequest("export requires export_mime_type".into())
                        })?,
                    )?)
                } else {
                    None
                };
                let bytes = self
                    .0
                    .download_bytes(&file, export_mime, TRANSFER_BYTE_LIMIT)
                    .await?;
                atomic_write(&path, &bytes)?;
                Ok(json!({
                    "file_id": id,
                    "workspace_path": args.workspace_path,
                    "bytes": bytes.len(),
                }))
            }
            _ => Err(Error::InvalidRequest(
                "Unknown Google Drive transfer action".into(),
            )),
        }
    }
}

impl GoogleWorkspaceConnection {
    async fn resumable_upload(
        &self,
        file_id: Option<&str>,
        metadata: Value,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<GoogleFileSummary> {
        let existing = match file_id {
            Some(id) => Some(self.authorized_file(id).await?),
            None => None,
        };
        let mut start_url = match file_id {
            Some(id) => Url::parse(&format!("{}/files/{id}", self.config.upload_url)),
            None => Url::parse(&format!("{}/files", self.config.upload_url)),
        }
        .map_err(|error| Error::Other(error.to_string()))?;
        start_url
            .query_pairs_mut()
            .append_pair("uploadType", "resumable")
            .append_pair("supportsAllDrives", "true")
            .append_pair("fields", DRIVE_FILE_FIELDS);
        let method = if file_id.is_some() {
            Method::PATCH
        } else {
            Method::POST
        };

        let mut session_url = None;
        for force in [false, true] {
            let token = self.access_token(force).await?;
            let response = self
                .client
                .request(method.clone(), start_url.clone())
                .bearer_auth(token)
                .header("X-Upload-Content-Type", mime)
                .header("X-Upload-Content-Length", bytes.len())
                .json(&metadata)
                .send()
                .await
                .map_err(upstream)?;
            if response.status() == StatusCode::UNAUTHORIZED && !force {
                *self.access_token.lock().await = None;
                continue;
            }
            let response = ensure_google_response(response).await?;
            session_url = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            break;
        }
        let session_url = session_url
            .ok_or_else(|| Error::Upstream("Google returned no resumable upload URL".into()))?;
        let response = self
            .client
            .put(session_url)
            .header("Content-Type", mime)
            .body(bytes)
            .send()
            .await
            .map_err(upstream)?;
        let mut file = ensure_google_response(response)
            .await?
            .json::<DriveFile>()
            .await
            .map_err(upstream)?
            .summary(existing.as_ref().is_none_or(|file| file.created_by_milim));
        file.created_by_milim = existing
            .as_ref()
            .map(|file| file.created_by_milim)
            .unwrap_or(true);
        self.save_authorized_file(file.clone()).await?;
        Ok(file)
    }
}

fn required_id(value: Option<&str>) -> Result<&str> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::InvalidRequest("file_id is required for this action".into()))?;
    validate_google_id(value)?;
    Ok(value)
}

fn required_text<'a>(value: Option<&'a str>, field: &str, max: usize) -> Result<&'a str> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::InvalidRequest(format!("{field} is required")))?;
    if value.chars().count() > max || value.contains(['\0', '\r', '\n']) {
        return Err(Error::InvalidRequest(format!("Invalid {field}")));
    }
    Ok(value)
}

fn require_capability(allowed: bool, action: &str, file: &GoogleFileSummary) -> Result<()> {
    if allowed {
        Ok(())
    } else {
        Err(Error::InvalidRequest(format!(
            "Google does not allow Milim to {action} {}",
            file.name
        )))
    }
}

fn validate_email(value: Option<&str>) -> Result<&str> {
    let value = required_text(value, "email", 320)?;
    let mut parts = value.split('@');
    let valid = parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_some_and(|part| {
            part.contains('.') && !part.starts_with('.') && !part.ends_with('.')
        })
        && parts.next().is_none()
        && !value.contains(char::is_whitespace);
    if valid {
        Ok(value)
    } else {
        Err(Error::InvalidRequest("Invalid recipient email".into()))
    }
}

fn validate_share_role(value: Option<&str>) -> Result<&str> {
    match value {
        Some(value @ ("reader" | "commenter" | "writer")) => Ok(value),
        _ => Err(Error::InvalidRequest(
            "role must be reader, commenter, or writer".into(),
        )),
    }
}

fn validate_mime(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 200
        || value.contains(['\0', '\r', '\n'])
        || !value.contains('/')
    {
        Err(Error::InvalidRequest("Invalid MIME type".into()))
    } else {
        Ok(value)
    }
}

fn read_bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(Error::InvalidRequest(
            "Transfer source must be a file".into(),
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(Error::InvalidRequest(format!(
            "Transfer exceeds Milim's {} MB limit",
            limit / 1024 / 1024
        )));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > limit {
        return Err(Error::InvalidRequest(format!(
            "Transfer exceeds Milim's {} MB limit",
            limit / 1024 / 1024
        )));
    }
    Ok(bytes)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    file_id: String,
    operations: Vec<Value>,
}

#[async_trait]
impl Tool for GoogleSheetsEditTool {
    fn name(&self) -> &str {
        "google_sheets_edit"
    }

    fn description(&self) -> &str {
        "Apply bounded value, formula, worksheet, row, column, or basic-formatting operations to an authorized Google Sheet."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string" },
                "operations": {
                    "type": "array",
                    "maxItems": EDIT_OPERATION_LIMIT,
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": [
                                    "set_values", "clear", "append_rows", "add_sheet",
                                    "delete_sheet", "rename_sheet", "insert_rows",
                                    "delete_rows", "insert_columns", "delete_columns",
                                    "format_cells"
                                ]
                            },
                            "range": { "type": "string" },
                            "values": { "type": "array", "items": { "type": "array" } },
                            "input_option": { "type": "string", "enum": ["USER_ENTERED", "RAW"] },
                            "sheet_id": { "type": "integer" },
                            "title": { "type": "string" },
                            "start": { "type": "integer", "minimum": 0 },
                            "end": { "type": "integer", "minimum": 1 },
                            "start_row": { "type": "integer", "minimum": 0 },
                            "end_row": { "type": "integer", "minimum": 1 },
                            "start_column": { "type": "integer", "minimum": 0 },
                            "end_column": { "type": "integer", "minimum": 1 },
                            "bold": { "type": "boolean" },
                            "italic": { "type": "boolean" },
                            "background_color": {
                                "type": "string",
                                "description": "Optional #RRGGBB color."
                            }
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["file_id", "operations"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: EditArgs = parse_args(args)?;
        validate_operations(&args.operations)?;
        let file = self.0.refresh_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_SHEET_MIME)?;
        require_capability(file.capabilities.can_edit, "edit", &file)?;

        let mut results = Vec::new();
        let mut batch_requests = Vec::new();
        let mut written_cells = 0usize;
        for operation in &args.operations {
            let action = operation_action(operation)?;
            match action {
                "set_values" | "append_rows" => {
                    let range = operation
                        .get("range")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::InvalidRequest(format!("{action} requires range")))?;
                    validate_sheet_range(range)?;
                    let values = operation
                        .get("values")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            Error::InvalidRequest(format!("{action} requires values"))
                        })?;
                    written_cells = written_cells.saturating_add(
                        values
                            .iter()
                            .map(|row| row.as_array().map(Vec::len).unwrap_or(0))
                            .sum::<usize>(),
                    );
                    if written_cells > SHEET_WRITE_CELL_LIMIT {
                        return Err(Error::InvalidRequest(format!(
                            "Google Sheets edit exceeds the {SHEET_WRITE_CELL_LIMIT}-cell limit"
                        )));
                    }
                    if values.iter().any(|row| !row.is_array()) {
                        return Err(Error::InvalidRequest(
                            "values must be an array of row arrays".into(),
                        ));
                    }
                    let input_option = operation
                        .get("input_option")
                        .and_then(Value::as_str)
                        .unwrap_or("USER_ENTERED");
                    if !matches!(input_option, "USER_ENTERED" | "RAW") {
                        return Err(Error::InvalidRequest(
                            "input_option must be USER_ENTERED or RAW".into(),
                        ));
                    }
                    let encoded_range = encode_path_segment(range);
                    let mut url = if action == "set_values" {
                        Url::parse(&format!(
                            "{}/spreadsheets/{}/values/{encoded_range}",
                            self.0.config.sheets_url, file.id
                        ))
                    } else {
                        Url::parse(&format!(
                            "{}/spreadsheets/{}/values/{encoded_range}:append",
                            self.0.config.sheets_url, file.id
                        ))
                    }
                    .map_err(|error| Error::Other(error.to_string()))?;
                    url.query_pairs_mut()
                        .append_pair("valueInputOption", input_option);
                    if action == "append_rows" {
                        url.query_pairs_mut()
                            .append_pair("insertDataOption", "INSERT_ROWS");
                    }
                    let result = self
                        .0
                        .request(
                            if action == "set_values" {
                                Method::PUT
                            } else {
                                Method::POST
                            },
                            url,
                            Some(json!({ "range": range, "majorDimension": "ROWS", "values": values })),
                        )
                        .await?
                        .json::<Value>()
                        .await
                        .map_err(upstream)?;
                    results.push(result);
                }
                "clear" => {
                    let range = operation
                        .get("range")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::InvalidRequest("clear requires range".into()))?;
                    validate_sheet_range(range)?;
                    let cells = a1_cell_count(range)?;
                    written_cells = written_cells.saturating_add(cells);
                    if written_cells > SHEET_WRITE_CELL_LIMIT {
                        return Err(Error::InvalidRequest(format!(
                            "Google Sheets edit exceeds the {SHEET_WRITE_CELL_LIMIT}-cell limit"
                        )));
                    }
                    let url = Url::parse(&format!(
                        "{}/spreadsheets/{}/values/{}:clear",
                        self.0.config.sheets_url,
                        file.id,
                        encode_path_segment(range)
                    ))
                    .map_err(|error| Error::Other(error.to_string()))?;
                    let result = self
                        .0
                        .request(Method::POST, url, Some(json!({})))
                        .await?
                        .json::<Value>()
                        .await
                        .map_err(upstream)?;
                    results.push(result);
                }
                "add_sheet" => {
                    let title = required_text(
                        operation.get("title").and_then(Value::as_str),
                        "title",
                        100,
                    )?;
                    batch_requests
                        .push(json!({ "addSheet": { "properties": { "title": title } } }));
                }
                "delete_sheet" => {
                    batch_requests.push(json!({
                        "deleteSheet": { "sheetId": required_i64(operation, "sheet_id")? }
                    }));
                }
                "rename_sheet" => {
                    let title = required_text(
                        operation.get("title").and_then(Value::as_str),
                        "title",
                        100,
                    )?;
                    batch_requests.push(json!({
                        "updateSheetProperties": {
                            "properties": {
                                "sheetId": required_i64(operation, "sheet_id")?,
                                "title": title
                            },
                            "fields": "title"
                        }
                    }));
                }
                "insert_rows" | "delete_rows" | "insert_columns" | "delete_columns" => {
                    let dimension = if action.ends_with("rows") {
                        "ROWS"
                    } else {
                        "COLUMNS"
                    };
                    let range = json!({
                        "sheetId": required_i64(operation, "sheet_id")?,
                        "dimension": dimension,
                        "startIndex": required_index(operation, "start")?,
                        "endIndex": required_index(operation, "end")?
                    });
                    ensure_index_order(&range)?;
                    if range["endIndex"].as_i64().unwrap_or_default()
                        - range["startIndex"].as_i64().unwrap_or_default()
                        > SHEET_WRITE_CELL_LIMIT as i64
                    {
                        return Err(Error::InvalidRequest(format!(
                            "Google Sheets dimension edit exceeds the {SHEET_WRITE_CELL_LIMIT}-item limit"
                        )));
                    }
                    if action.starts_with("insert") {
                        batch_requests.push(json!({
                            "insertDimension": { "range": range, "inheritFromBefore": false }
                        }));
                    } else {
                        batch_requests.push(json!({ "deleteDimension": { "range": range } }));
                    }
                }
                "format_cells" => {
                    let mut grid_range = serde_json::Map::from_iter([(
                        "sheetId".into(),
                        json!(required_i64(operation, "sheet_id")?),
                    )]);
                    for (source, target) in [
                        ("start_row", "startRowIndex"),
                        ("end_row", "endRowIndex"),
                        ("start_column", "startColumnIndex"),
                        ("end_column", "endColumnIndex"),
                    ] {
                        if let Some(index) = optional_index(operation, source)? {
                            grid_range.insert(target.into(), json!(index));
                        }
                    }
                    let start_row = grid_range
                        .get("startRowIndex")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                            Error::InvalidRequest(
                                "format_cells requires start_row and end_row".into(),
                            )
                        })?;
                    let end_row = grid_range
                        .get("endRowIndex")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                            Error::InvalidRequest(
                                "format_cells requires start_row and end_row".into(),
                            )
                        })?;
                    let start_column = grid_range
                        .get("startColumnIndex")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                        Error::InvalidRequest(
                            "format_cells requires start_column and end_column".into(),
                        )
                    })?;
                    let end_column = grid_range
                        .get("endColumnIndex")
                        .and_then(Value::as_i64)
                        .ok_or_else(|| {
                            Error::InvalidRequest(
                                "format_cells requires start_column and end_column".into(),
                            )
                        })?;
                    let cells = end_row
                        .checked_sub(start_row)
                        .filter(|rows| *rows > 0)
                        .and_then(|rows| {
                            end_column
                                .checked_sub(start_column)
                                .filter(|columns| *columns > 0)
                                .and_then(|columns| rows.checked_mul(columns))
                        })
                        .ok_or_else(|| {
                            Error::InvalidRequest(
                                "format_cells end indexes must exceed start indexes".into(),
                            )
                        })?;
                    if cells > SHEET_WRITE_CELL_LIMIT as i64 {
                        return Err(Error::InvalidRequest(format!(
                            "Google Sheets format exceeds the {SHEET_WRITE_CELL_LIMIT}-cell limit"
                        )));
                    }
                    let mut format = json!({});
                    let mut fields = Vec::new();
                    if let Some(bold) = operation.get("bold").and_then(Value::as_bool) {
                        format["textFormat"]["bold"] = json!(bold);
                        fields.push("userEnteredFormat.textFormat.bold");
                    }
                    if let Some(italic) = operation.get("italic").and_then(Value::as_bool) {
                        format["textFormat"]["italic"] = json!(italic);
                        fields.push("userEnteredFormat.textFormat.italic");
                    }
                    if let Some(color) = operation.get("background_color").and_then(Value::as_str) {
                        format["backgroundColorStyle"]["rgbColor"] = parse_hex_color(color)?;
                        fields.push("userEnteredFormat.backgroundColorStyle");
                    }
                    if fields.is_empty() {
                        return Err(Error::InvalidRequest(
                            "format_cells requires bold, italic, or background_color".into(),
                        ));
                    }
                    batch_requests.push(json!({
                        "repeatCell": {
                            "range": Value::Object(grid_range),
                            "cell": { "userEnteredFormat": format },
                            "fields": fields.join(",")
                        }
                    }));
                }
                _ => {
                    return Err(Error::InvalidRequest(format!(
                        "Unsupported Google Sheets operation: {action}"
                    )))
                }
            }
        }
        if !batch_requests.is_empty() {
            let url = Url::parse(&format!(
                "{}/spreadsheets/{}:batchUpdate",
                self.0.config.sheets_url, file.id
            ))
            .map_err(|error| Error::Other(error.to_string()))?;
            results.push(
                self.0
                    .request(
                        Method::POST,
                        url,
                        Some(json!({ "requests": batch_requests })),
                    )
                    .await?
                    .json::<Value>()
                    .await
                    .map_err(upstream)?,
            );
        }
        Ok(json!({ "file_id": file.id, "operations": args.operations.len(), "results": results }))
    }
}

#[async_trait]
impl Tool for GoogleDocsEditTool {
    fn name(&self) -> &str {
        "google_docs_edit"
    }

    fn description(&self) -> &str {
        "Apply bounded text, style, list, table, and page-break operations to an authorized Google Doc."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string" },
                "operations": {
                    "type": "array",
                    "maxItems": EDIT_OPERATION_LIMIT,
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": [
                                    "insert_text", "replace_all_text", "delete_range",
                                    "set_text_style", "clear_text_style", "set_paragraph_style", "create_bullets",
                                    "delete_bullets", "insert_page_break", "insert_table"
                                ]
                            },
                            "index": { "type": "integer", "minimum": 1 },
                            "start": { "type": "integer", "minimum": 1 },
                            "end": { "type": "integer", "minimum": 2 },
                            "text": { "type": "string" },
                            "find": { "type": "string" },
                            "replace": { "type": "string" },
                            "match_case": { "type": "boolean" },
                            "bold": { "type": "boolean" },
                            "italic": { "type": "boolean" },
                            "underline": { "type": "boolean" },
                            "font_size": { "type": "number", "minimum": 6, "maximum": 200 },
                            "foreground_color": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$" },
                            "alignment": {
                                "type": "string",
                                "enum": ["START", "CENTER", "END", "JUSTIFIED"]
                            },
                            "named_style_type": { "type": "string" },
                            "bullet_preset": { "type": "string" },
                            "rows": { "type": "integer", "minimum": 1, "maximum": 50 },
                            "columns": { "type": "integer", "minimum": 1, "maximum": 20 }
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["file_id", "operations"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: EditArgs = parse_args(args)?;
        validate_operations(&args.operations)?;
        let file = self.0.refresh_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_DOC_MIME)?;
        require_capability(file.capabilities.can_edit, "edit", &file)?;
        let mut requests = Vec::with_capacity(args.operations.len());
        let mut inserted_chars = 0usize;
        for operation in &args.operations {
            let action = operation_action(operation)?;
            let request = match action {
                "insert_text" => {
                    let text = operation
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::InvalidRequest("insert_text requires text".into()))?;
                    inserted_chars = inserted_chars.saturating_add(text.chars().count());
                    json!({
                        "insertText": {
                            "location": { "index": required_doc_index(operation, "index")? },
                            "text": text
                        }
                    })
                }
                "replace_all_text" => {
                    let find = required_text(
                        operation.get("find").and_then(Value::as_str),
                        "find",
                        10_000,
                    )?;
                    let replace = operation
                        .get("replace")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            Error::InvalidRequest("replace_all_text requires replace".into())
                        })?;
                    inserted_chars = inserted_chars.saturating_add(replace.chars().count());
                    json!({
                        "replaceAllText": {
                            "containsText": {
                                "text": find,
                                "matchCase": operation.get("match_case").and_then(Value::as_bool).unwrap_or(false)
                            },
                            "replaceText": replace
                        }
                    })
                }
                "delete_range" => json!({
                    "deleteContentRange": { "range": doc_range(operation)? }
                }),
                "set_text_style" => {
                    let mut style = json!({});
                    let mut fields = Vec::new();
                    for field in ["bold", "italic", "underline"] {
                        if let Some(value) = operation.get(field).and_then(Value::as_bool) {
                            style[field] = json!(value);
                            fields.push(field);
                        }
                    }
                    if let Some(value) = optional_font_size(operation)? {
                        style["fontSize"] = json!({ "magnitude": value, "unit": "PT" });
                        fields.push("fontSize");
                    }
                    if let Some(value) = operation.get("foreground_color").and_then(Value::as_str) {
                        style["foregroundColor"] =
                            json!({ "color": { "rgbColor": parse_hex_color(value)? } });
                        fields.push("foregroundColor");
                    }
                    if fields.is_empty() {
                        return Err(Error::InvalidRequest(
                            "set_text_style requires a style field".into(),
                        ));
                    }
                    json!({
                        "updateTextStyle": {
                            "range": doc_range(operation)?,
                            "textStyle": style,
                            "fields": fields.join(",")
                        }
                    })
                }
                "clear_text_style" => json!({
                    "updateTextStyle": {
                        "range": doc_range(operation)?,
                        "textStyle": {},
                        "fields": "*"
                    }
                }),
                "set_paragraph_style" => {
                    let mut style = json!({});
                    let mut fields = Vec::new();
                    if let Some(named_style) =
                        operation.get("named_style_type").and_then(Value::as_str)
                    {
                        if !matches!(
                            named_style,
                            "NORMAL_TEXT"
                                | "TITLE"
                                | "SUBTITLE"
                                | "HEADING_1"
                                | "HEADING_2"
                                | "HEADING_3"
                                | "HEADING_4"
                                | "HEADING_5"
                                | "HEADING_6"
                        ) {
                            return Err(Error::InvalidRequest(
                                "Unsupported Docs named style".into(),
                            ));
                        }
                        style["namedStyleType"] = json!(named_style);
                        fields.push("namedStyleType");
                    }
                    if let Some(alignment) = operation.get("alignment").and_then(Value::as_str) {
                        style["alignment"] = json!(validate_text_alignment(alignment)?);
                        fields.push("alignment");
                    }
                    if fields.is_empty() {
                        return Err(Error::InvalidRequest(
                            "set_paragraph_style requires a style field".into(),
                        ));
                    }
                    json!({
                        "updateParagraphStyle": {
                            "range": doc_range(operation)?,
                            "paragraphStyle": style,
                            "fields": fields.join(",")
                        }
                    })
                }
                "create_bullets" => json!({
                    "createParagraphBullets": {
                        "range": doc_range(operation)?,
                        "bulletPreset": operation
                            .get("bullet_preset")
                            .and_then(Value::as_str)
                            .unwrap_or("BULLET_DISC_CIRCLE_SQUARE")
                    }
                }),
                "delete_bullets" => json!({
                    "deleteParagraphBullets": { "range": doc_range(operation)? }
                }),
                "insert_page_break" => json!({
                    "insertPageBreak": {
                        "location": { "index": required_doc_index(operation, "index")? }
                    }
                }),
                "insert_table" => {
                    let rows = required_positive_i64(operation, "rows", 50)?;
                    let columns = required_positive_i64(operation, "columns", 20)?;
                    let mut request = json!({ "rows": rows, "columns": columns });
                    if operation.get("index").is_some() {
                        request["location"] =
                            json!({ "index": required_doc_index(operation, "index")? });
                    } else {
                        request["endOfSegmentLocation"] = json!({});
                    }
                    json!({ "insertTable": request })
                }
                _ => {
                    return Err(Error::InvalidRequest(format!(
                        "Unsupported Google Docs operation: {action}"
                    )))
                }
            };
            requests.push(request);
        }
        if inserted_chars > MODEL_TEXT_LIMIT {
            return Err(Error::InvalidRequest(format!(
                "Google Docs edit exceeds the {MODEL_TEXT_LIMIT}-character limit"
            )));
        }
        let url = Url::parse(&format!(
            "{}/documents/{}:batchUpdate",
            self.0.config.docs_url, file.id
        ))
        .map_err(|error| Error::Other(error.to_string()))?;
        let result = self
            .0
            .request(Method::POST, url, Some(json!({ "requests": requests })))
            .await?
            .json::<Value>()
            .await
            .map_err(upstream)?;
        Ok(json!({ "file_id": file.id, "operations": args.operations.len(), "result": result }))
    }
}

#[async_trait]
impl Tool for GoogleSlidesEditTool {
    fn name(&self) -> &str {
        "google_slides_edit"
    }

    fn description(&self) -> &str {
        "Apply bounded slide, text, shape, table, ordering, and HTTPS-image operations to an authorized presentation."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_id": { "type": "string" },
                "operations": {
                    "type": "array",
                    "maxItems": EDIT_OPERATION_LIMIT,
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": [
                                    "create_slide", "duplicate_slide", "delete_slide",
                                    "reorder_slides", "replace_all_text", "insert_text",
                                    "delete_text", "set_text_style", "set_paragraph_style",
                                    "create_shape", "create_table", "insert_image"
                                ]
                            },
                            "object_id": { "type": "string" },
                            "page_id": { "type": "string" },
                            "layout": { "type": "string" },
                            "slide_object_ids": { "type": "array", "items": { "type": "string" } },
                            "insertion_index": { "type": "integer", "minimum": 0 },
                            "find": { "type": "string" },
                            "replace": { "type": "string" },
                            "match_case": { "type": "boolean" },
                            "text": { "type": "string" },
                            "offset": { "type": "integer", "minimum": 0 },
                            "start": { "type": "integer", "minimum": 0 },
                            "end": { "type": "integer", "minimum": 0 },
                            "bold": { "type": "boolean" },
                            "italic": { "type": "boolean" },
                            "underline": { "type": "boolean" },
                            "font_size": { "type": "number", "minimum": 6, "maximum": 200 },
                            "foreground_color": { "type": "string", "pattern": "^#?[0-9A-Fa-f]{6}$" },
                            "alignment": {
                                "type": "string",
                                "enum": ["START", "CENTER", "END", "JUSTIFIED"]
                            },
                            "shape_type": { "type": "string" },
                            "rows": { "type": "integer", "minimum": 1, "maximum": 50 },
                            "columns": { "type": "integer", "minimum": 1, "maximum": 20 },
                            "x": { "type": "number" },
                            "y": { "type": "number" },
                            "width": { "type": "number", "exclusiveMinimum": 0 },
                            "height": { "type": "number", "exclusiveMinimum": 0 },
                            "url": { "type": "string" }
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["file_id", "operations"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: EditArgs = parse_args(args)?;
        validate_operations(&args.operations)?;
        let file = self.0.refresh_file(&args.file_id).await?;
        require_mime(&file, GOOGLE_SLIDE_MIME)?;
        require_capability(file.capabilities.can_edit, "edit", &file)?;
        let mut requests = Vec::with_capacity(args.operations.len());
        let mut inserted_chars = 0usize;
        for operation in &args.operations {
            let action = operation_action(operation)?;
            let request = match action {
                "create_slide" => {
                    let mut create = json!({});
                    if let Some(id) = operation.get("object_id").and_then(Value::as_str) {
                        validate_google_id(id)?;
                        create["objectId"] = json!(id);
                    }
                    if let Some(layout) = operation.get("layout").and_then(Value::as_str) {
                        create["slideLayoutReference"] =
                            json!({ "predefinedLayout": validate_slide_layout(layout)? });
                    }
                    json!({ "createSlide": create })
                }
                "duplicate_slide" => json!({
                    "duplicateObject": { "objectId": operation_id(operation, "object_id")? }
                }),
                "delete_slide" => json!({
                    "deleteObject": { "objectId": operation_id(operation, "object_id")? }
                }),
                "reorder_slides" => {
                    let ids = operation
                        .get("slide_object_ids")
                        .and_then(Value::as_array)
                        .ok_or_else(|| {
                            Error::InvalidRequest("reorder_slides requires slide_object_ids".into())
                        })?
                        .iter()
                        .map(|id| {
                            let id = id.as_str().ok_or_else(|| {
                                Error::InvalidRequest("slide ids must be strings".into())
                            })?;
                            validate_google_id(id)?;
                            Ok(id)
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if ids.is_empty() {
                        return Err(Error::InvalidRequest(
                            "reorder_slides requires at least one slide".into(),
                        ));
                    }
                    json!({
                        "updateSlidesPosition": {
                            "slideObjectIds": ids,
                            "insertionIndex": optional_index(operation, "insertion_index")?.unwrap_or(0)
                        }
                    })
                }
                "replace_all_text" => {
                    let find = required_text(
                        operation.get("find").and_then(Value::as_str),
                        "find",
                        10_000,
                    )?;
                    let replace = operation
                        .get("replace")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            Error::InvalidRequest("replace_all_text requires replace".into())
                        })?;
                    inserted_chars = inserted_chars.saturating_add(replace.chars().count());
                    json!({
                        "replaceAllText": {
                            "containsText": {
                                "text": find,
                                "matchCase": operation.get("match_case").and_then(Value::as_bool).unwrap_or(false)
                            },
                            "replaceText": replace
                        }
                    })
                }
                "insert_text" => {
                    let text = operation
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::InvalidRequest("insert_text requires text".into()))?;
                    inserted_chars = inserted_chars.saturating_add(text.chars().count());
                    json!({
                        "insertText": {
                            "objectId": operation_id(operation, "object_id")?,
                            "insertionIndex": optional_index(operation, "offset")?.unwrap_or(0),
                            "text": text
                        }
                    })
                }
                "delete_text" => {
                    json!({ "deleteText": {
                        "objectId": operation_id(operation, "object_id")?,
                        "textRange": slide_text_range(operation)?
                    }})
                }
                "set_text_style" => {
                    let mut style = json!({});
                    let mut fields = Vec::new();
                    for field in ["bold", "italic", "underline"] {
                        if let Some(value) = operation.get(field).and_then(Value::as_bool) {
                            style[field] = json!(value);
                            fields.push(field);
                        }
                    }
                    if let Some(value) = optional_font_size(operation)? {
                        style["fontSize"] = json!({ "magnitude": value, "unit": "PT" });
                        fields.push("fontSize");
                    }
                    if let Some(value) = operation.get("foreground_color").and_then(Value::as_str) {
                        style["foregroundColor"] = json!({
                            "opaqueColor": { "rgbColor": parse_hex_color(value)? }
                        });
                        fields.push("foregroundColor");
                    }
                    if fields.is_empty() {
                        return Err(Error::InvalidRequest(
                            "set_text_style requires a style field".into(),
                        ));
                    }
                    json!({ "updateTextStyle": {
                        "objectId": operation_id(operation, "object_id")?,
                        "textRange": slide_text_range(operation)?,
                        "style": style,
                        "fields": fields.join(",")
                    }})
                }
                "set_paragraph_style" => {
                    let alignment = operation
                        .get("alignment")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            Error::InvalidRequest("set_paragraph_style requires alignment".into())
                        })?;
                    json!({ "updateParagraphStyle": {
                        "objectId": operation_id(operation, "object_id")?,
                        "textRange": slide_text_range(operation)?,
                        "style": { "alignment": validate_text_alignment(alignment)? },
                        "fields": "alignment"
                    }})
                }
                "create_shape" => {
                    let page_id = operation_id(operation, "page_id")?;
                    let mut shape = json!({
                        "shapeType": operation
                            .get("shape_type")
                            .and_then(Value::as_str)
                            .unwrap_or("RECTANGLE"),
                        "elementProperties": slide_element_properties(operation, page_id)?
                    });
                    if let Some(id) = operation.get("object_id").and_then(Value::as_str) {
                        validate_google_id(id)?;
                        shape["objectId"] = json!(id);
                    }
                    json!({ "createShape": shape })
                }
                "create_table" => {
                    let page_id = operation_id(operation, "page_id")?;
                    let mut table = json!({
                        "rows": required_positive_i64(operation, "rows", 50)?,
                        "columns": required_positive_i64(operation, "columns", 20)?,
                        "elementProperties": slide_element_properties(operation, page_id)?
                    });
                    if let Some(id) = operation.get("object_id").and_then(Value::as_str) {
                        validate_google_id(id)?;
                        table["objectId"] = json!(id);
                    }
                    json!({ "createTable": table })
                }
                "insert_image" => {
                    let url = operation
                        .get("url")
                        .and_then(Value::as_str)
                        .ok_or_else(|| Error::InvalidRequest("insert_image requires url".into()))?;
                    let parsed = Url::parse(url)
                        .map_err(|_| Error::InvalidRequest("Invalid image URL".into()))?;
                    if parsed.scheme() != "https" || parsed.host_str().is_none() {
                        return Err(Error::InvalidRequest(
                            "Slides images require a public HTTPS URL".into(),
                        ));
                    }
                    json!({
                        "createImage": {
                            "url": url,
                            "elementProperties": slide_element_properties(
                                operation,
                                operation_id(operation, "page_id")?
                            )?
                        }
                    })
                }
                _ => {
                    return Err(Error::InvalidRequest(format!(
                        "Unsupported Google Slides operation: {action}"
                    )))
                }
            };
            requests.push(request);
        }
        if inserted_chars > MODEL_TEXT_LIMIT {
            return Err(Error::InvalidRequest(format!(
                "Google Slides edit exceeds the {MODEL_TEXT_LIMIT}-character limit"
            )));
        }
        let url = Url::parse(&format!(
            "{}/presentations/{}:batchUpdate",
            self.0.config.slides_url, file.id
        ))
        .map_err(|error| Error::Other(error.to_string()))?;
        let result = self
            .0
            .request(Method::POST, url, Some(json!({ "requests": requests })))
            .await?
            .json::<Value>()
            .await
            .map_err(upstream)?;
        Ok(json!({ "file_id": file.id, "operations": args.operations.len(), "result": result }))
    }
}

fn validate_operations(operations: &[Value]) -> Result<()> {
    if operations.is_empty() {
        return Err(Error::InvalidRequest(
            "At least one operation is required".into(),
        ));
    }
    if operations.len() > EDIT_OPERATION_LIMIT {
        return Err(Error::InvalidRequest(format!(
            "Google edit exceeds the {EDIT_OPERATION_LIMIT}-operation limit"
        )));
    }
    if operations.iter().any(|operation| !operation.is_object()) {
        return Err(Error::InvalidRequest(
            "Each Google edit operation must be an object".into(),
        ));
    }
    Ok(())
}

fn operation_action(operation: &Value) -> Result<&str> {
    operation
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRequest("Each operation requires action".into()))
}

fn operation_id<'a>(operation: &'a Value, field: &str) -> Result<&'a str> {
    let id = operation
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRequest(format!("{field} is required")))?;
    validate_google_id(id)?;
    Ok(id)
}

fn required_i64(operation: &Value, field: &str) -> Result<i64> {
    operation
        .get(field)
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| Error::InvalidRequest(format!("{field} must be a non-negative integer")))
}

fn required_positive_i64(operation: &Value, field: &str, max: i64) -> Result<i64> {
    operation
        .get(field)
        .and_then(Value::as_i64)
        .filter(|value| *value > 0 && *value <= max)
        .ok_or_else(|| Error::InvalidRequest(format!("{field} must be between 1 and {max}")))
}

fn required_index(operation: &Value, field: &str) -> Result<i64> {
    required_i64(operation, field)
}

fn required_doc_index(operation: &Value, field: &str) -> Result<i64> {
    operation
        .get(field)
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| Error::InvalidRequest(format!("{field} must be a positive integer")))
}

fn optional_index(operation: &Value, field: &str) -> Result<Option<i64>> {
    operation
        .get(field)
        .map(|value| {
            value.as_i64().filter(|value| *value >= 0).ok_or_else(|| {
                Error::InvalidRequest(format!("{field} must be a non-negative integer"))
            })
        })
        .transpose()
}

fn ensure_index_order(range: &Value) -> Result<()> {
    let start = range
        .get("startIndex")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let end = range
        .get("endIndex")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if end > start {
        Ok(())
    } else {
        Err(Error::InvalidRequest(
            "Dimension end must be greater than start".into(),
        ))
    }
}

fn parse_hex_color(value: &str) -> Result<Value> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidRequest("Color must be #RRGGBB".into()));
    }
    let channel = |offset| {
        u8::from_str_radix(&hex[offset..offset + 2], 16)
            .map(|value| value as f64 / 255.0)
            .map_err(|_| Error::InvalidRequest("Invalid color".into()))
    };
    Ok(json!({
        "red": channel(0)?,
        "green": channel(2)?,
        "blue": channel(4)?,
    }))
}

fn optional_font_size(operation: &Value) -> Result<Option<f64>> {
    operation
        .get("font_size")
        .map(|value| {
            value
                .as_f64()
                .filter(|value| (6.0..=200.0).contains(value))
                .ok_or_else(|| Error::InvalidRequest("font_size must be between 6 and 200".into()))
        })
        .transpose()
}

fn validate_text_alignment(value: &str) -> Result<&str> {
    match value {
        "START" | "CENTER" | "END" | "JUSTIFIED" => Ok(value),
        _ => Err(Error::InvalidRequest("Unsupported text alignment".into())),
    }
}

fn doc_range(operation: &Value) -> Result<Value> {
    let start = required_doc_index(operation, "start")?;
    let end = required_doc_index(operation, "end")?;
    if end <= start {
        return Err(Error::InvalidRequest(
            "Docs range end must be greater than start".into(),
        ));
    }
    Ok(json!({ "startIndex": start, "endIndex": end }))
}

fn slide_text_range(operation: &Value) -> Result<Value> {
    if operation.get("start").is_none() && operation.get("end").is_none() {
        return Ok(json!({ "type": "ALL" }));
    }
    let start = required_index(operation, "start")?;
    let end = required_index(operation, "end")?;
    if end <= start {
        return Err(Error::InvalidRequest(
            "Slides text range end must be greater than start".into(),
        ));
    }
    Ok(json!({
        "type": "FIXED_RANGE",
        "startIndex": start,
        "endIndex": end
    }))
}

fn validate_slide_layout(value: &str) -> Result<&str> {
    match value {
        "BLANK"
        | "CAPTION_ONLY"
        | "MAIN_POINT"
        | "SECTION_HEADER"
        | "SECTION_TITLE_AND_DESCRIPTION"
        | "TITLE"
        | "TITLE_AND_BODY"
        | "TITLE_AND_TWO_COLUMNS"
        | "TITLE_ONLY"
        | "ONE_COLUMN_TEXT"
        | "BIG_NUMBER" => Ok(value),
        _ => Err(Error::InvalidRequest(
            "Unsupported predefined slide layout".into(),
        )),
    }
}

fn slide_element_properties(operation: &Value, page_id: &str) -> Result<Value> {
    let number = |field: &str| {
        operation
            .get(field)
            .and_then(Value::as_f64)
            .ok_or_else(|| Error::InvalidRequest(format!("{field} is required")))
    };
    let x = number("x")?;
    let y = number("y")?;
    let width = number("width")?;
    let height = number("height")?;
    if width <= 0.0 || height <= 0.0 || ![x, y, width, height].iter().all(|v| v.is_finite()) {
        return Err(Error::InvalidRequest(
            "Slide element geometry must be finite with positive width and height".into(),
        ));
    }
    Ok(json!({
        "pageObjectId": page_id,
        "size": {
            "width": { "magnitude": width, "unit": "PT" },
            "height": { "magnitude": height, "unit": "PT" }
        },
        "transform": {
            "scaleX": 1,
            "scaleY": 1,
            "translateX": x,
            "translateY": y,
            "unit": "PT"
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GoogleWorkspaceConfig {
        GoogleWorkspaceConfig::desktop(
            Some("123-test.apps.googleusercontent.com".into()),
            Some("secret".into()),
        )
    }

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!("milim-google-workspace-{}", Uuid::new_v4()))
    }

    #[test]
    fn validates_google_ids_and_ranges() {
        assert!(validate_google_id("abc_123-XYZ").is_ok());
        assert!(validate_google_id("../secret").is_err());
        assert!(validate_sheet_range("'Sheet 1'!A1:Z200").is_ok());
        assert!(validate_sheet_range("A1\nB2").is_err());
        assert_eq!(a1_cell_count("'Sheet 1'!A1:Z200").unwrap(), 5_200);
        assert_eq!(a1_cell_count("$B$2:$C$4").unwrap(), 6);
        assert!(a1_cell_count("A:Z").is_err());
    }

    #[test]
    fn extracts_workspace_text_without_serializing_metadata() {
        let value = json!({
            "body": { "content": [
                { "paragraph": { "elements": [
                    { "textRun": { "content": "Hello " } },
                    { "textRun": { "content": "Milim" } }
                ]}}
            ]}
        });
        assert_eq!(extract_google_text(&value), "Hello Milim");
        let content = google_doc_model_content(&value, 5);
        assert_eq!(content[0]["runs"][0]["text"], "Hello");
        assert!(!content[0].to_string().contains("Milim"));
    }

    #[test]
    fn separates_slide_content_from_speaker_notes() {
        let slide = json!({
            "objectId": "slide_1",
            "pageElements": [{ "objectId": "title_1", "shape": { "text": { "textElements": [
                { "startIndex": 0, "endIndex": 14, "paragraphMarker": {
                    "style": { "alignment": "CENTER" }
                }},
                { "startIndex": 0, "endIndex": 14, "textRun": {
                    "content": "Visible title",
                    "style": { "bold": true, "fontSize": { "magnitude": 24, "unit": "PT" } }
                }}
            ]}}}],
            "slideProperties": { "notesPage": {
                "notesProperties": { "speakerNotesObjectId": "notes_1" },
                "pageElements": [
                { "objectId": "notes_1", "shape": {
                    "placeholder": { "type": "BODY" },
                    "text": { "textElements": [
                    { "textRun": { "content": "Private speaker note" } }
                ]}}}
            ]}}
        });
        let mut remaining = 100;
        let preview =
            google_slide_preview_item(&slide, &mut remaining, Some((9_144_000.0, 5_143_500.0)));
        assert_eq!(preview["text"], "Visible title");
        assert_eq!(preview["notes"], "Private speaker note");
        assert_eq!(preview["textElements"][0]["objectId"], "title_1");
        assert_eq!(
            preview["textElements"][0]["styleRuns"][0]["style"]["bold"],
            true
        );
        assert_eq!(
            preview["textElements"][0]["paragraphRuns"][0]["style"]["alignment"],
            "CENTER"
        );
        assert_eq!(preview["notesObjectId"], "notes_1");

        let mut remaining = 100;
        let empty_notes = google_slide_preview_item(
            &json!({
                "objectId": "slide_2",
                "slideProperties": { "notesPage": {
                    "notesProperties": { "speakerNotesObjectId": "notes_2" }
                }}
            }),
            &mut remaining,
            None,
        );
        assert_eq!(empty_notes["notesObjectId"], "notes_2");
        assert_eq!(empty_notes["notes"], "");
    }

    #[test]
    fn normalizes_slide_text_box_geometry() {
        let presentation = json!({
            "pageSize": {
                "width": { "magnitude": 720.0, "unit": "PT" },
                "height": { "magnitude": 405.0, "unit": "PT" }
            }
        });
        let element = json!({
            "size": {
                "width": { "magnitude": 360.0, "unit": "PT" },
                "height": { "magnitude": 81.0, "unit": "PT" }
            },
            "transform": {
                "translateX": 180.0,
                "translateY": 40.5,
                "unit": "PT"
            }
        });
        let page_size = google_slide_page_size(&presentation).unwrap();
        assert_eq!(
            google_slide_element_rect(&element, page_size),
            Some((0.25, 0.1, 0.5, 0.2))
        );
        let mut rotated = element;
        rotated["transform"]["shearX"] = json!(0.25);
        assert_eq!(google_slide_element_rect(&rotated, page_size), None);
    }

    #[test]
    fn enforces_sheet_cell_budget() {
        assert!(enforce_sheet_cell_limit(Some(&json!([[1, 2], [3]])), 3).is_ok());
        assert!(enforce_sheet_cell_limit(Some(&json!([[1, 2], [3]])), 2).is_err());
    }

    #[test]
    fn auth_url_scope_is_narrow() {
        assert_eq!(
            DRIVE_FILE_SCOPE,
            "https://www.googleapis.com/auth/drive.file"
        );
        assert!(test_config().available());
        assert!(!GoogleWorkspaceConfig::desktop(
            Some("123-test.apps.googleusercontent.com".into()),
            None,
        )
        .available());
        assert!(!GoogleWorkspaceConfig::desktop(None, None).available());
    }

    #[test]
    fn encrypted_store_round_trips_and_fails_closed() {
        let root = test_root();
        let store = GoogleWorkspaceStore::open(&root).unwrap();
        let mut state = StoredWorkspace {
            refresh_token: Some("refresh-token-that-must-not-be-plain".into()),
            managed_folder_id: Some("folder_1".into()),
            ..StoredWorkspace::default()
        };
        state.files.insert(
            "file_1".into(),
            GoogleFileSummary {
                id: "file_1".into(),
                name: "Selected sheet".into(),
                mime_type: GOOGLE_SHEET_MIME.into(),
                ..GoogleFileSummary::default()
            },
        );
        store.save(&state).unwrap();
        let encrypted = std::fs::read(root.join("google-workspace.enc")).unwrap();
        assert!(!encrypted
            .windows(b"refresh-token-that-must-not-be-plain".len())
            .any(|window| window == b"refresh-token-that-must-not-be-plain"));
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.refresh_token.as_deref(),
            Some("refresh-token-that-must-not-be-plain")
        );
        assert!(loaded.files.contains_key("file_1"));
        assert_eq!(loaded.managed_folder_id.as_deref(), Some("folder_1"));
        atomic_write(&root.join("google-workspace.enc"), b"corrupt").unwrap();
        assert!(store.load().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn oauth_callback_validates_state_and_rejects_replay() {
        let (callback, mut received) = oneshot::channel();
        let relay = Arc::new(PickerRelay {
            expected_state: "expected".into(),
            callback: StdMutex::new(Some(callback)),
        });
        let wrong = oauth_callback(
            State(relay.clone()),
            Query(OAuthCallbackQuery {
                state: Some("wrong".into()),
                code: Some("code".into()),
                scope: None,
                error: None,
                picked_file_ids: None,
            }),
        )
        .await;
        assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
        assert!(received.try_recv().is_err());

        let valid = oauth_callback(
            State(relay.clone()),
            Query(OAuthCallbackQuery {
                state: Some("expected".into()),
                code: Some("code".into()),
                scope: Some(DRIVE_FILE_SCOPE.into()),
                error: None,
                picked_file_ids: Some("folder_1,file_2".into()),
            }),
        )
        .await;
        assert_eq!(valid.status(), StatusCode::OK);
        let received = received.await.unwrap();
        assert_eq!(received.code.as_deref(), Some("code"));
        assert_eq!(received.picked_file_ids().unwrap(), ["folder_1", "file_2"]);

        let replay = oauth_callback(
            State(relay),
            Query(OAuthCallbackQuery {
                state: Some("expected".into()),
                code: Some("other".into()),
                scope: None,
                error: None,
                picked_file_ids: Some("file_3".into()),
            }),
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn reuses_the_managed_folder_from_encrypted_state() {
        let root = test_root();
        let connection =
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap();
        let folder = GoogleFileSummary {
            id: "folder_1".into(),
            name: MILIM_DRIVE_FOLDER_NAME.into(),
            mime_type: GOOGLE_FOLDER_MIME.into(),
            created_by_milim: true,
            ..GoogleFileSummary::default()
        };
        {
            let mut stored = connection.stored.write().await;
            stored.managed_folder_id = Some(folder.id.clone());
            stored.files.insert(folder.id.clone(), folder.clone());
            connection.store.save(&stored).unwrap();
        }
        assert_eq!(connection.managed_folder().await.unwrap().id, folder.id);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn authorized_folder_refreshes_paginated_descendants() {
        async fn files(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
            let parent = query.get("q").map(String::as_str).unwrap_or_default();
            let page = query.get("pageToken").map(String::as_str);
            Json(if parent.contains("folder_1") && page.is_none() {
                json!({
                    "nextPageToken": "next",
                    "files": [
                        { "id": "file_1", "name": "Sheet", "mimeType": GOOGLE_SHEET_MIME, "parents": ["folder_1"] },
                        { "id": "folder_2", "name": "Nested", "mimeType": GOOGLE_FOLDER_MIME, "parents": ["folder_1"] }
                    ]
                })
            } else if parent.contains("folder_1") {
                json!({
                    "files": [
                        { "id": "file_2", "name": "Document", "mimeType": GOOGLE_DOC_MIME, "parents": ["folder_1"] }
                    ]
                })
            } else {
                json!({
                    "files": [
                        { "id": "file_3", "name": "Slides", "mimeType": GOOGLE_SLIDE_MIME, "parents": ["folder_2"] }
                    ]
                })
            })
        }

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, Router::new().route("/files", get(files)))
                .await
                .unwrap();
        });
        let root = test_root();
        let mut config = test_config();
        config.drive_url = format!("http://{address}");
        let connection =
            GoogleWorkspaceConnection::open(&root, config, Arc::new(StdRwLock::new(None))).unwrap();
        let descendants = connection
            .folder_descendants_with_token("folder_1", "token")
            .await
            .unwrap();
        assert_eq!(
            descendants
                .iter()
                .map(|file| file.id.as_str())
                .collect::<Vec<_>>(),
            ["file_1", "file_2", "file_3", "folder_2"]
        );
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_picker_callback_validates_and_deduplicates_ids() {
        let query = OAuthCallbackQuery {
            state: None,
            code: None,
            scope: None,
            error: None,
            picked_file_ids: Some("file_1,file_1,folder-2".into()),
        };
        assert_eq!(query.picked_file_ids().unwrap(), ["file_1", "folder-2"]);
        let invalid = OAuthCallbackQuery {
            picked_file_ids: Some("../secret".into()),
            ..query
        };
        assert!(invalid.picked_file_ids().is_err());
    }

    #[test]
    fn desktop_oauth_uses_google_loopback_redirect_shape() {
        assert_eq!(loopback_redirect_uri(64484), "http://127.0.0.1:64484");
    }

    #[tokio::test]
    async fn desktop_picker_allows_multiple_files_but_not_folder_selection() {
        let root = test_root();
        let connection = Arc::new(
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap(),
        );
        let flow = connection
            .start_picker(vec!["folder_1".into()])
            .await
            .unwrap();
        let url = Url::parse(flow.url.as_deref().unwrap()).unwrap();
        let query = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(
            query.get("allow_multiple").map(|value| value.as_ref()),
            Some("true")
        );
        assert_eq!(
            query.get("file_ids").map(|value| value.as_ref()),
            Some("folder_1")
        );
        assert!(!query.contains_key("allow_folder_selection"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_reports_confirmed_revocation_and_clears_local_state() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/revoke", axum::routing::post(|| async { StatusCode::OK })),
            )
            .await
            .unwrap();
        });
        let root = test_root();
        let mut config = test_config();
        config.revoke_url = format!("http://{address}/revoke");
        let connection =
            GoogleWorkspaceConnection::open(&root, config, Arc::new(StdRwLock::new(None))).unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.refresh_token = Some("refresh".into());
            connection.store.save(&stored).unwrap();
        }
        *connection.access_token.lock().await = Some(CachedAccessToken {
            value: "access".into(),
            expires_at: u64::MAX,
        });

        let result = connection.disconnect().await.unwrap();
        assert_eq!(result.revocation, GoogleRevocationStatus::Confirmed);
        assert!(result.local_authorization_removed);
        assert!(!root.join("google-workspace.enc").exists());
        assert!(connection.stored.read().await.refresh_token.is_none());
        assert!(connection.access_token.lock().await.is_none());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_reports_unconfirmed_when_google_is_unreachable() {
        let root = test_root();
        let mut config = test_config();
        config.revoke_url = "http://127.0.0.1:9/revoke".into();
        let connection =
            GoogleWorkspaceConnection::open(&root, config, Arc::new(StdRwLock::new(None))).unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.refresh_token = Some("refresh".into());
            connection.store.save(&stored).unwrap();
        }

        let result = connection.disconnect().await.unwrap();
        assert_eq!(result.revocation, GoogleRevocationStatus::Unconfirmed);
        assert!(!root.join("google-workspace.enc").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_reports_unconfirmed_when_google_times_out() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/revoke",
                    axum::routing::post(|| async {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        StatusCode::OK
                    }),
                ),
            )
            .await
            .unwrap();
        });
        let root = test_root();
        let mut config = test_config();
        config.revoke_url = format!("http://{address}/revoke");
        let connection =
            GoogleWorkspaceConnection::open(&root, config, Arc::new(StdRwLock::new(None))).unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.refresh_token = Some("refresh".into());
            connection.store.save(&stored).unwrap();
        }

        let result = connection.disconnect().await.unwrap();
        assert_eq!(result.revocation, GoogleRevocationStatus::Unconfirmed);
        assert!(!root.join("google-workspace.enc").exists());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_reports_unconfirmed_for_google_error_status() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/revoke",
                    axum::routing::post(|| async { StatusCode::BAD_REQUEST }),
                ),
            )
            .await
            .unwrap();
        });
        let root = test_root();
        let mut config = test_config();
        config.revoke_url = format!("http://{address}/revoke");
        let connection =
            GoogleWorkspaceConnection::open(&root, config, Arc::new(StdRwLock::new(None))).unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.refresh_token = Some("refresh".into());
            connection.store.save(&stored).unwrap();
        }

        let result = connection.disconnect().await.unwrap();
        assert_eq!(result.revocation, GoogleRevocationStatus::Unconfirmed);
        assert!(!root.join("google-workspace.enc").exists());
        server.abort();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_without_refresh_token_needs_no_revocation() {
        let root = test_root();
        let connection =
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap();
        let result = connection.disconnect().await.unwrap();
        assert_eq!(result.revocation, GoogleRevocationStatus::NotNeeded);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn disconnect_does_not_claim_local_removal_when_clear_fails() {
        let root = test_root();
        let connection =
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap();
        let data_path = root.join("google-workspace.enc");
        std::fs::create_dir_all(&data_path).unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.account_id = Some("account".into());
        }

        assert!(connection.disconnect().await.is_err());
        assert_eq!(
            connection.stored.read().await.account_id.as_deref(),
            Some("account")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn removing_file_only_changes_the_local_registry() {
        let root = test_root();
        let connection =
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap();
        {
            let mut stored = connection.stored.write().await;
            stored.files.insert(
                "file_1".into(),
                GoogleFileSummary {
                    id: "file_1".into(),
                    name: "Sheet".into(),
                    mime_type: GOOGLE_SHEET_MIME.into(),
                    ..GoogleFileSummary::default()
                },
            );
            connection.store.save(&stored).unwrap();
        }
        assert!(connection.remove_file("file_1").await.unwrap());
        assert!(!connection.stored.read().await.files.contains_key("file_1"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn google_tools_have_bounded_effects_and_sharing_schema() {
        let root = test_root();
        let connection = Arc::new(
            GoogleWorkspaceConnection::open(&root, test_config(), Arc::new(StdRwLock::new(None)))
                .unwrap(),
        );
        let tools = tools(connection);
        let effects = tools
            .iter()
            .map(|tool| (tool.name(), tool.effect()))
            .collect::<HashMap<_, _>>();
        assert_eq!(effects["google_drive_list"], ToolEffect::ReadOnly);
        assert_eq!(effects["google_sheets_edit"], ToolEffect::Mutating);
        assert_eq!(effects["google_drive_transfer"], ToolEffect::Command);
        let manage = tools
            .iter()
            .find(|tool| tool.name() == "google_drive_manage")
            .unwrap()
            .input_schema()
            .to_string();
        assert!(manage.contains("\"user\""));
        assert!(manage.contains("\"group\""));
        assert!(!manage.contains("\"anyone\""));
        assert!(!manage.contains("\"owner\""));
        let docs = tools
            .iter()
            .find(|tool| tool.name() == "google_docs_edit")
            .unwrap()
            .input_schema()
            .to_string();
        assert!(docs.contains("\"clear_text_style\""));
        drop(tools);
        std::fs::remove_dir_all(root).unwrap();
    }
}
