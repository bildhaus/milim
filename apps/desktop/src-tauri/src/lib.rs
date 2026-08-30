//! milim desktop shell.
//!
//! The app hosts the milim server **in-process** (milim's model: the
//! app *is* the server), so launching the desktop app starts the backend on
//! `127.0.0.1:<port>` automatically - the web UI then talks to it over HTTP.
//!
//! Backend priority: a configured remote (`MILIM_REMOTE_BASE_URL`) -> an
//! explicit unavailable backend. The desktop provider registry routes Ollama,
//! LM Studio, and other OpenAI-compatible local runtimes above that fallback.

#[cfg(feature = "computer-use")]
mod computer_tools;
mod diagnostics;
mod host_tools;
mod preview_tools;
mod preview_webview;
mod secret_storage;

use std::collections::{BTreeMap, HashMap};
#[cfg(target_os = "windows")]
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::header::ACCEPT;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::ipc::Channel;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
#[cfg(target_os = "macos")]
use tauri::menu::{MenuBuilder, MenuItemKind, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, WindowEvent};
use tauri_plugin_dialog::DialogExt;
use toml::Value as TomlValue;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use milim_core::config::ServerConfiguration;
use milim_core::paths::Paths;
use milim_core::{Error, Result};
use milim_inference::{remote::RemoteBackend, unavailable::UnavailableBackend, SharedService};
use milim_sandbox::{DockerBackend, RunOpts};
use milim_server::AppState;
use milim_storage::{Database, DatabaseOptions, EncryptedStore, JournalMode, SessionsDelta};
use milim_tools::{
    atomic_write, resolve_workspace_path, ProcessEnvironmentPolicy, Tool, ToolEffect, ToolRegistry,
};

static NATIVE_PERF_STARTED: OnceLock<Instant> = OnceLock::new();
static NATIVE_PERF_STAGES: OnceLock<Mutex<BTreeMap<String, u64>>> = OnceLock::new();
static SUBPROCESS_LAUNCHES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize)]
struct PerformanceReportV2 {
    schema_version: u32,
    build_profile: &'static str,
    perf_diagnostics: bool,
    stages_ms: BTreeMap<String, u64>,
    subprocess_launches: u64,
    storage: milim_storage::StoragePerformanceSnapshot,
}

fn native_perf_mark(stage: &str) {
    let started = NATIVE_PERF_STARTED.get_or_init(Instant::now);
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let stages = NATIVE_PERF_STAGES.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(mut stages) = stages.lock() {
        stages.entry(stage.to_string()).or_insert(elapsed);
    }
}

fn record_subprocess_launch() {
    SUBPROCESS_LAUNCHES.fetch_add(1, Ordering::Relaxed);
}

#[tauri::command]
fn perf_snapshot_v2() -> std::result::Result<PerformanceReportV2, String> {
    if !cfg!(feature = "perf-diagnostics") {
        return Err("perf diagnostics are disabled in this build".into());
    }
    let stages_ms = NATIVE_PERF_STAGES
        .get()
        .and_then(|stages| stages.lock().ok().map(|stages| stages.clone()))
        .unwrap_or_default();
    Ok(PerformanceReportV2 {
        schema_version: 2,
        build_profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        perf_diagnostics: cfg!(feature = "perf-diagnostics"),
        stages_ms,
        subprocess_launches: SUBPROCESS_LAUNCHES.load(Ordering::Relaxed),
        storage: milim_storage::storage_performance_snapshot(),
    })
}

/// Simple Rust/JS bridge example.
#[tauri::command]
fn health() -> String {
    "ok".to_string()
}

#[tauri::command]
fn record_frontend_error(message: String, detail: Option<String>) {
    diagnostics::record_frontend_error(&message, detail.as_deref());
}

#[tauri::command]
fn diagnostics_path() -> std::result::Result<String, String> {
    let path = diagnostics::log_dir();
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn restart_app(app: tauri::AppHandle) {
    restart_after_server_shutdown(app);
}

#[tauri::command]
fn secret_storage_status(
    state: tauri::State<'_, DesktopSecretStorage>,
) -> secret_storage::SecretStorageStatus {
    state.0.clone()
}

struct DesktopApiToken(String);

struct DesktopApiBaseUrl(String);

#[derive(Default)]
struct WorkspaceEditorDirty(AtomicBool);

struct DesktopProviders(Option<Arc<milim_server::providers::ProviderRegistry>>);

struct DesktopSecretStorage(secret_storage::SecretStorageStatus);

struct MobileControlLocalTarget(String);

struct UserDataState(Arc<milim_storage::UserDataStore>);

struct DesktopPreviewRuntime(Arc<milim_server::preview_runtime::PreviewRuntimeManager>);

struct DesktopServerRuntime(Arc<DesktopServerRuntimeState>);

struct DesktopLanActive {
    shutdown: tokio::sync::watch::Sender<bool>,
    port: u16,
    mdns: mdns_sd::ServiceDaemon,
    fullname: String,
}

struct DesktopServerRuntimeState {
    shutdown: tokio::sync::watch::Sender<bool>,
    tasks: Mutex<Vec<tauri::async_runtime::JoinHandle<()>>>,
    finishing: AtomicBool,
    control: Option<Arc<milim_server::control::RunManager>>,
    mobile_state: Option<AppState>,
    lan: Mutex<Option<DesktopLanActive>>,
}

impl DesktopServerRuntimeState {
    fn new(
        control: Option<Arc<milim_server::control::RunManager>>,
        mobile_state: Option<AppState>,
    ) -> Arc<Self> {
        let (shutdown, _) = tokio::sync::watch::channel(false);
        Arc::new(Self {
            shutdown,
            tasks: Mutex::new(Vec::new()),
            finishing: AtomicBool::new(false),
            control,
            mobile_state,
            lan: Mutex::new(None),
        })
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    fn register(&self, task: tauri::async_runtime::JoinHandle<()>) {
        self.tasks
            .lock()
            .expect("desktop server tasks poisoned")
            .push(task);
    }

    fn enable_lan(&self) -> std::result::Result<MobileLanStatus, String> {
        if let Some(active) = self.lan.lock().map_err(|_| "LAN state poisoned")?.as_ref() {
            return Ok(self.lan_status(Some(active)));
        }
        let state = self
            .mobile_state
            .clone()
            .ok_or_else(|| "Mobile control runtime is unavailable.".to_string())?;
        let control = self
            .control
            .as_ref()
            .ok_or_else(|| "Canonical control runtime is unavailable.".to_string())?;
        let listener = TcpListener::bind(("0.0.0.0", MOBILE_LAN_PREFERRED_PORT)).or_else(
            |error| {
                tracing::warn!(
                    "LAN mobile listener could not bind preferred port {MOBILE_LAN_PREFERRED_PORT}: {error}; falling back to an available port"
                );
                TcpListener::bind("0.0.0.0:0")
            },
        )
        .map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let host_id = control.host().host_id;
        let host_label = host_id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(12)
            .collect::<String>()
            .to_ascii_lowercase();
        let hostname = format!("milim-{host_label}.local.");
        let mut properties = HashMap::new();
        properties.insert("host_id".to_string(), host_id);
        properties.insert("protocol_min".to_string(), "1".to_string());
        properties.insert("protocol_max".to_string(), "1".to_string());
        properties.insert("scheme".to_string(), "http".to_string());
        let service = mdns_sd::ServiceInfo::new(
            "_milim._tcp.local.",
            control.host().display_name.as_str(),
            &hostname,
            "",
            port,
            properties,
        )
        .map_err(|error| error.to_string())?
        .enable_addr_auto();
        let fullname = service.get_fullname().to_string();
        let mdns = mdns_sd::ServiceDaemon::new().map_err(|error| error.to_string())?;
        mdns.register(service).map_err(|error| error.to_string())?;
        let (shutdown, local_shutdown) = tokio::sync::watch::channel(false);
        let global_shutdown = self.subscribe();
        let task = tauri::async_runtime::spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(listener) {
                Ok(listener) => listener,
                Err(error) => {
                    tracing::warn!("LAN mobile listener unavailable: {error}");
                    return;
                }
            };
            let shutdown = async move {
                tokio::select! {
                    _ = wait_for_desktop_shutdown(local_shutdown) => {},
                    _ = wait_for_desktop_shutdown(global_shutdown) => {},
                }
            };
            if let Err(error) =
                milim_server::serve_mobile_companion_listener_with_graceful_shutdown(
                    state, listener, shutdown,
                )
                .await
            {
                tracing::warn!("LAN mobile server stopped: {error}");
            }
        });
        self.register(task);
        let active = DesktopLanActive {
            shutdown,
            port,
            mdns,
            fullname,
        };
        let status = self.lan_status(Some(&active));
        *self.lan.lock().map_err(|_| "LAN state poisoned")? = Some(active);
        Ok(status)
    }

    fn disable_lan(&self) -> MobileLanStatus {
        let active = self.lan.lock().ok().and_then(|mut value| value.take());
        if let Some(active) = active {
            let _ = active.shutdown.send(true);
            let _ = active.mdns.unregister(&active.fullname);
            let _ = active.mdns.shutdown();
        }
        self.lan_status(None)
    }

    fn current_lan_status(&self) -> MobileLanStatus {
        let active = self.lan.lock().ok();
        self.lan_status(active.as_deref().and_then(Option::as_ref))
    }

    fn lan_status(&self, active: Option<&DesktopLanActive>) -> MobileLanStatus {
        MobileLanStatus {
            enabled: active.is_some(),
            active: active.is_some(),
            port: active.map(|value| value.port),
            service_type: "_milim._tcp.local.".to_string(),
            host_id: self.control.as_ref().map(|value| value.host().host_id),
            warning: "LAN mode uses plain HTTP. Enable it only on a trusted network.".to_string(),
        }
    }

    async fn shutdown(
        &self,
        preview_runtime: Arc<milim_server::preview_runtime::PreviewRuntimeManager>,
    ) {
        self.disable_lan();
        if let Some(control) = &self.control {
            control.shutdown(Duration::from_secs(2)).await;
        }
        let _ = self.shutdown.send(true);
        let mut tasks =
            std::mem::take(&mut *self.tasks.lock().expect("desktop server tasks poisoned"));
        let cleanup = async {
            for task in &mut tasks {
                if let Err(error) = task.await {
                    tracing::warn!("embedded server shutdown task failed: {error}");
                }
            }
            if let Err(error) = preview_runtime.stop_all().await {
                tracing::warn!("failed to stop preview apps during desktop shutdown: {error}");
            }
        };
        if tokio::time::timeout(DESKTOP_SERVER_SHUTDOWN_TIMEOUT, cleanup)
            .await
            .is_err()
        {
            tracing::warn!("desktop shutdown exceeded its cleanup deadline");
            for task in tasks {
                task.abort();
            }
        }
    }
}

async fn wait_for_desktop_shutdown(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let shutting_down = *shutdown.borrow();
    if !shutting_down {
        let _ = shutdown.changed().await;
    }
}

#[tauri::command]
fn quit_after_user_state_flush(app: tauri::AppHandle) {
    exit_after_server_shutdown(app);
}

#[tauri::command]
fn request_desktop_hide(app: tauri::AppHandle) -> std::result::Result<(), String> {
    request_workspace_editor_leave(&app, WorkspaceEditorLeaveAction::Hide)
}

#[tauri::command]
fn request_desktop_quit(app: tauri::AppHandle) -> std::result::Result<(), String> {
    request_workspace_editor_leave(&app, WorkspaceEditorLeaveAction::Quit)
}

#[tauri::command]
fn set_workspace_editor_dirty(state: tauri::State<'_, WorkspaceEditorDirty>, dirty: bool) {
    state.0.store(dirty, Ordering::Release);
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceEditorLeaveAction {
    Hide,
    Quit,
}

#[tauri::command]
fn complete_workspace_editor_leave(
    app: tauri::AppHandle,
    state: tauri::State<'_, WorkspaceEditorDirty>,
    action: WorkspaceEditorLeaveAction,
) -> std::result::Result<(), String> {
    state.0.store(false, Ordering::Release);
    complete_workspace_editor_leave_action(&app, action)
}

const TAILSCALE_SERVE_PORT: u16 = 10000;
const MOBILE_LAN_PREFERRED_PORT: u16 = 7378;
const TAILSCALE_COMMAND_TIMEOUT_SECS: u64 = 12;
const TAILSCALE_STARTUP_READY_ATTEMPTS: usize = 8;
const TAILSCALE_STARTUP_CONFIG_ATTEMPTS: usize = 3;
const TAILSCALE_STARTUP_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_OPEN_ID: &str = "open";
const TRAY_QUIT_ID: &str = "quit";
const FLUSH_USER_STATE_EVENT: &str = "milim://flush-user-state";
const FLUSH_USER_STATE_AND_EXIT_EVENT: &str = "milim://flush-user-state-and-exit";
const WORKSPACE_EDITOR_LEAVE_EVENT: &str = "milim://workspace-editor-leave-requested";
const USER_STATE_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const DESKTOP_SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const RUNTIME_FAILED_EVENT: &str = "milim://runtime-failed";
const MODEL_FAVORITES_UPDATED_EVENT: &str = "milim://model-favorites-updated";
const APP_MENU_EVENT: &str = "milim://menu-action";
const APP_MENU_NEW_CHAT_ID: &str = "app.new-chat";
const APP_MENU_SETTINGS_ID: &str = "app.settings";
const APP_MENU_TOGGLE_SIDEBAR_ID: &str = "app.toggle-sidebar";
const APP_MENU_DOCUMENTATION_ID: &str = "app.documentation";
const APP_MENU_DIAGNOSTICS_ID: &str = "app.diagnostics";
const APP_MENU_QUIT_ID: &str = "app.quit";
const MAX_ATTACHMENT_BYTES: u64 = 128 * 1024;
const MAX_ATTACHMENT_IMAGE_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ARTIFACT_PREVIEW_BYTES: usize = 256 * 1024;
// ponytail: bounded scan, add an index if very large workspaces need complete matching.
const MAX_WORKSPACE_FILE_SUGGESTION_SCAN: usize = 5_000;
const ARTIFACT_DIFF_CONTEXT_LINES: usize = 2;
const ARTIFACT_DIFF_LCS_CELL_LIMIT: usize = 2_000_000;
const BACKUP_SCHEMA_VERSION: u32 = 1;
const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
const APPEARANCE_STATE_KEY: &str = "milim.appearanceSnapshot";
const BACKUP_STATE_KEYS: &[&str] = &[
    "milim.settings",
    "milim.ui",
    "milim.onboarding",
    "milim.themeId",
    "milim.customThemes",
    APPEARANCE_STATE_KEY,
    "milim.window.alwaysOnTop",
    "milim.sessionDrafts",
    "milim.mobile.lan",
];

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MilimBackup {
    schema_version: u32,
    app_version: String,
    created_at: u64,
    summary: MilimBackupSummary,
    state: MilimBackupState,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MilimBackupSummary {
    chats: usize,
    projects: usize,
    state_keys: usize,
    #[serde(default)]
    control_records: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MilimBackupState {
    sessions: Value,
    entries: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    control: Option<milim_storage::ControlBackupState>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupInspection {
    schema_version: u32,
    app_version: String,
    created_at: u64,
    summary: MilimBackupSummary,
    bytes: u64,
}

fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn strip_backup_exclusions(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in [
                "threadWorkspace",
                "retryWorkspace",
                "pendingHotSwap",
                "pendingWorkerRunIds",
                "previewRuntime",
                "previewRuntimesByKey",
                "workerRuns",
                "generatingSessionIds",
                "accountRuntime",
                "media",
                "mediaRequestId",
            ] {
                map.remove(key);
            }
            for child in map.values_mut() {
                strip_backup_exclusions(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_backup_exclusions),
        _ => {}
    }
}

fn build_backup(store: &milim_storage::UserDataStore) -> std::result::Result<MilimBackup, String> {
    let sessions_json = store
        .get_sessions_snapshot()
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            serde_json::json!({ "state": { "sessions": [] }, "version": 0 }).to_string()
        });
    let mut sessions: Value =
        serde_json::from_str(&sessions_json).map_err(|error| error.to_string())?;
    strip_backup_exclusions(&mut sessions);
    let state = sessions.get("state").and_then(Value::as_object);
    let chats = state
        .and_then(|value| value.get("sessions"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let projects = state
        .and_then(|value| value.get("projects"))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let mut entries = BTreeMap::new();
    for key in BACKUP_STATE_KEYS {
        if let Some(raw) = store.get_json(key).map_err(|error| error.to_string())? {
            entries.insert(
                (*key).to_string(),
                serde_json::from_str(&raw).map_err(|error| error.to_string())?,
            );
        }
    }
    let control = store
        .control_backup_state()
        .map_err(|error| error.to_string())?;
    let control_records = control.threads.len()
        + control.runs.len()
        + control.timeline.len()
        + control.queued_turns.len()
        + control.command_receipts.len()
        + control.approvals.len()
        + usize::from(control.host.is_some());
    Ok(MilimBackup {
        schema_version: BACKUP_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: now_timestamp_ms(),
        summary: MilimBackupSummary {
            chats,
            projects,
            state_keys: entries.len(),
            control_records,
        },
        state: MilimBackupState {
            sessions,
            entries,
            control: Some(control),
        },
    })
}

fn read_backup(path: &Path) -> std::result::Result<(MilimBackup, u64), String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err("Backup is larger than the 64 MiB safety limit.".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let backup: MilimBackup = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Malformed backup JSON: {error}"))?;
    if backup.schema_version != BACKUP_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported backup schema version {}.",
            backup.schema_version
        ));
    }
    if !backup.state.sessions.is_object() {
        return Err("Backup sessions state is invalid.".into());
    }
    if backup
        .state
        .entries
        .keys()
        .any(|key| !BACKUP_STATE_KEYS.contains(&key.as_str()))
    {
        return Err("Backup contains unsupported state keys.".into());
    }
    Ok((backup, metadata.len()))
}

#[tauri::command]
async fn export_milim_backup(
    state: tauri::State<'_, UserDataState>,
    path: String,
) -> std::result::Result<BackupInspection, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || {
        let backup = build_backup(&store)?;
        let bytes = serde_json::to_vec_pretty(&backup).map_err(|error| error.to_string())?;
        atomic_write(Path::new(&path), &bytes).map_err(|error| error.to_string())?;
        Ok(BackupInspection {
            schema_version: backup.schema_version,
            app_version: backup.app_version,
            created_at: backup.created_at,
            summary: backup.summary,
            bytes: bytes.len() as u64,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn inspect_milim_backup(path: String) -> std::result::Result<BackupInspection, String> {
    tokio::task::spawn_blocking(move || {
        let (backup, bytes) = read_backup(Path::new(&path))?;
        Ok(BackupInspection {
            schema_version: backup.schema_version,
            app_version: backup.app_version,
            created_at: backup.created_at,
            summary: backup.summary,
            bytes,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn restore_milim_backup(
    state: tauri::State<'_, UserDataState>,
    runtime: tauri::State<'_, DesktopServerRuntime>,
    path: String,
) -> std::result::Result<String, String> {
    let store = state.0.clone();
    let recovery_path =
        tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let (backup, _) = read_backup(Path::new(&path))?;
            let recovery = build_backup(&store)?;
            let recovery_bytes =
                serde_json::to_vec_pretty(&recovery).map_err(|error| error.to_string())?;
            let data_path = Paths::resolve().user_db_file();
            let recovery_path = data_path.parent().unwrap_or(Path::new(".")).join(format!(
                "pre-restore-{}.milim-backup.json",
                now_timestamp_ms()
            ));
            atomic_write(&recovery_path, &recovery_bytes).map_err(|error| error.to_string())?;
            let MilimBackupState {
                sessions,
                entries,
                control,
            } = backup.state;
            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    serde_json::to_string(&value)
                        .map(|json| (key, json))
                        .map_err(|error| error.to_string())
                })
                .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
            let sessions = serde_json::to_string(&sessions).map_err(|error| error.to_string())?;
            let replace_keys = BACKUP_STATE_KEYS
                .iter()
                .map(|key| (*key).to_string())
                .collect::<Vec<_>>();
            store
                .replace_backup_state(&replace_keys, entries, &sessions)
                .map_err(|error| error.to_string())?;
            if let Some(control) = control {
                store
                    .replace_control_backup_state(&control)
                    .map_err(|error| error.to_string())?;
            } else {
                store
                    .reconcile_control_startup()
                    .map_err(|error| error.to_string())?;
            }
            Ok(recovery_path.to_string_lossy().to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
    if let Some(control) = &runtime.0.control {
        control.refresh_host().map_err(|error| error.to_string())?;
    }
    Ok(recovery_path)
}

#[derive(serde::Serialize)]
struct AttachmentFilePayload {
    name: String,
    path: String,
    size: u64,
    mime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(rename = "dataUrl", skip_serializing_if = "Option::is_none")]
    data_url: Option<String>,
    truncated: bool,
}

#[derive(serde::Serialize, Debug)]
struct WorkspaceFileSuggestion {
    path: String,
    full_path: String,
    name: String,
    size: u64,
}

#[derive(serde::Serialize, Debug)]
struct WorkspaceDirectoryEntry {
    path: String,
    name: String,
    kind: &'static str,
    size: Option<u64>,
}

#[derive(serde::Serialize, Debug)]
struct WorkspaceDirectoryPage {
    entries: Vec<WorkspaceDirectoryEntry>,
    next_cursor: Option<String>,
}

#[derive(serde::Serialize, Debug)]
struct WorkspaceTextFile {
    path: String,
    content: String,
    size: u64,
    revision: String,
}

#[derive(serde::Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorkspaceTextWriteResult {
    Saved {
        path: String,
        size: u64,
        revision: String,
    },
    Conflict {
        revision: String,
    },
}

#[derive(serde::Serialize, Debug)]
struct SavedArtifactFilePayload {
    path: String,
    bytes: usize,
    overwritten: bool,
}

#[derive(serde::Serialize, Debug)]
struct ArtifactFileStatusPayload {
    path: String,
    exists: bool,
    is_file: bool,
    is_dir: bool,
    bytes: Option<u64>,
}

#[derive(serde::Serialize, Debug)]
struct ArtifactWritePreviewPayload {
    path: String,
    exists: bool,
    changed: bool,
    old_content: Option<String>,
    new_content: String,
    old_bytes: Option<usize>,
    new_bytes: usize,
    diff: String,
    truncated: bool,
}

#[derive(serde::Serialize, Debug)]
struct HarnessImportPreview {
    mcps: Vec<HarnessMcpCandidate>,
    skills: Vec<HarnessSkillCandidate>,
}

#[derive(serde::Serialize, Debug)]
struct HarnessMcpCandidate {
    harness: String,
    name: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env: Vec<HarnessMcpEnvCandidate>,
    warnings: Vec<String>,
    source_path: String,
}

#[derive(serde::Serialize, Debug)]
struct HarnessMcpEnvCandidate {
    key: String,
    value: Option<String>,
    secret: bool,
    required: bool,
}

#[derive(serde::Serialize, Debug)]
struct HarnessSkillCandidate {
    harness: String,
    name: String,
    path: String,
    skill_md: String,
}

#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct WorkspaceLauncherPayload {
    id: String,
    label: String,
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommended_reason: Option<String>,
}

#[derive(Clone, Copy, Debug)]
enum ArtifactOpenTarget {
    File,
    Folder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceLauncherId {
    Vscode,
    Zed,
    FileManager,
    Terminal,
    GitBash,
    Wsl,
    AndroidStudio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceLauncherPlatform {
    Windows,
    Macos,
    Linux,
}

#[derive(Debug)]
struct ArtifactOpenCommandSpec {
    program: String,
    args: Vec<String>,
}

type WorkspaceLauncherCommandSpec = ArtifactOpenCommandSpec;

#[derive(serde::Serialize, Debug)]
struct MobileTailscaleStatus {
    installed: bool,
    logged_in: bool,
    serve_configured: bool,
    public_url: Option<String>,
    local_target: String,
    message: Option<String>,
}

#[derive(Clone, serde::Serialize, Debug)]
struct MobileLanStatus {
    enabled: bool,
    active: bool,
    port: Option<u16>,
    service_type: String,
    host_id: Option<String>,
    warning: String,
}

#[tauri::command]
fn api_token(token: tauri::State<'_, DesktopApiToken>) -> String {
    token.0.clone()
}

#[tauri::command]
fn api_base_url(base: tauri::State<'_, DesktopApiBaseUrl>) -> String {
    base.0.clone()
}

#[tauri::command]
async fn refresh_provider_models(
    providers: tauri::State<'_, DesktopProviders>,
    runtime: tauri::State<'_, DesktopServerRuntime>,
) -> std::result::Result<bool, String> {
    let Some(providers) = providers.0.clone() else {
        return Ok(false);
    };
    let refreshed = providers.refresh_all().await.map_err(|e| e.to_string())?;
    if refreshed {
        if let Some(control) = &runtime.0.control {
            control.publish_model_catalog();
        }
    }
    Ok(refreshed)
}

#[tauri::command]
async fn mobile_tailscale_status(
    target: tauri::State<'_, MobileControlLocalTarget>,
) -> std::result::Result<MobileTailscaleStatus, String> {
    let local_target = target.0.clone();
    tokio::task::spawn_blocking(move || mobile_tailscale_status_blocking(local_target))
        .await
        .map_err(|e| format!("tailscale status task failed: {e}"))
}

#[tauri::command]
async fn configure_mobile_tailscale(
    target: tauri::State<'_, MobileControlLocalTarget>,
) -> std::result::Result<MobileTailscaleStatus, String> {
    let local_target = target.0.clone();
    tokio::task::spawn_blocking(move || configure_mobile_tailscale_blocking(local_target))
        .await
        .map_err(|e| format!("tailscale setup task failed: {e}"))
}

#[tauri::command]
async fn disable_mobile_tailscale(
    target: tauri::State<'_, MobileControlLocalTarget>,
) -> std::result::Result<MobileTailscaleStatus, String> {
    let local_target = target.0.clone();
    tokio::task::spawn_blocking(move || disable_mobile_tailscale_blocking(local_target))
        .await
        .map_err(|e| format!("tailscale disable task failed: {e}"))
}

#[tauri::command]
fn mobile_lan_status(runtime: tauri::State<'_, DesktopServerRuntime>) -> MobileLanStatus {
    runtime.0.current_lan_status()
}

#[tauri::command]
fn set_mobile_lan_enabled(
    runtime: tauri::State<'_, DesktopServerRuntime>,
    user_data: tauri::State<'_, UserDataState>,
    enabled: bool,
) -> std::result::Result<MobileLanStatus, String> {
    let status = if enabled {
        runtime.0.enable_lan()?
    } else {
        runtime.0.disable_lan()
    };
    user_data
        .0
        .set_json("milim.mobile.lan", if enabled { "true" } else { "false" })
        .map_err(|error| error.to_string())?;
    Ok(status)
}

#[tauri::command]
async fn user_state_get(
    state: tauri::State<'_, UserDataState>,
    key: String,
) -> std::result::Result<Option<String>, String> {
    if key == "milim.sessions" {
        state.0.get_sessions_snapshot().map_err(|e| e.to_string())
    } else {
        state.0.get_json(&key).map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn user_state_set(
    state: tauri::State<'_, UserDataState>,
    runtime: tauri::State<'_, DesktopServerRuntime>,
    key: String,
    value: String,
) -> std::result::Result<(), String> {
    let result = if key == "milim.sessions" {
        state
            .0
            .set_sessions_snapshot(&value)
            .map_err(|e| e.to_string())
    } else {
        state.0.set_json(&key, &value).map_err(|e| e.to_string())
    };
    if result.is_ok() && key == APPEARANCE_STATE_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_appearance();
        }
    }
    if result.is_ok() && key == milim_server::control::MODEL_FAVORITES_SETTINGS_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_model_favorites();
        }
    }
    if result.is_ok() && key == milim_server::control::MODEL_CATALOG_STATE_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_model_catalog();
        }
    }
    result
}

#[tauri::command]
async fn user_state_delete(
    state: tauri::State<'_, UserDataState>,
    runtime: tauri::State<'_, DesktopServerRuntime>,
    key: String,
) -> std::result::Result<bool, String> {
    let result = if key == "milim.sessions" {
        state
            .0
            .delete_sessions_snapshot()
            .map_err(|e| e.to_string())
    } else {
        state.0.delete_json(&key).map_err(|e| e.to_string())
    };
    if matches!(result, Ok(true)) && key == APPEARANCE_STATE_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_appearance();
        }
    }
    if matches!(result, Ok(true)) && key == milim_server::control::MODEL_FAVORITES_SETTINGS_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_model_favorites();
        }
    }
    if matches!(result, Ok(true)) && key == milim_server::control::MODEL_CATALOG_STATE_KEY {
        if let Some(control) = runtime.0.control.as_ref() {
            control.publish_model_catalog();
        }
    }
    result
}

#[tauri::command]
async fn user_sessions_get(
    state: tauri::State<'_, UserDataState>,
) -> std::result::Result<Option<String>, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.get_sessions_snapshot())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_sessions_manifest_get(
    state: tauri::State<'_, UserDataState>,
) -> std::result::Result<Option<String>, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.get_sessions_manifest_snapshot(100))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_session_messages_page(
    state: tauri::State<'_, UserDataState>,
    session_id: String,
    before_index: Option<usize>,
    limit: Option<usize>,
) -> std::result::Result<milim_storage::SessionMessagesPage, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || {
        store.session_messages_page(&session_id, before_index, limit.unwrap_or(100))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_chat_search(
    state: tauri::State<'_, UserDataState>,
    query: String,
    limit: Option<usize>,
) -> std::result::Result<Vec<milim_storage::UserChatSearchResult>, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.user_chat_search(&query, limit.unwrap_or(20)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_sessions_set(
    state: tauri::State<'_, UserDataState>,
    value: String,
) -> std::result::Result<(), String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.set_sessions_snapshot(&value))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_sessions_apply_delta(
    state: tauri::State<'_, UserDataState>,
    delta: SessionsDelta,
) -> std::result::Result<(), String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.apply_sessions_delta(delta))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Typed incremental session mutation path used by the desktop renderer.
/// Keep `user_sessions_apply_delta` for one compatibility release.
#[tauri::command]
async fn user_sessions_apply_ops(
    state: tauri::State<'_, UserDataState>,
    delta: SessionsDelta,
) -> std::result::Result<(), String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || store.apply_sessions_delta(delta))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_sessions_delete(
    state: tauri::State<'_, UserDataState>,
) -> std::result::Result<bool, String> {
    state
        .0
        .delete_sessions_snapshot()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_state_import_legacy(
    state: tauri::State<'_, UserDataState>,
    entries: BTreeMap<String, String>,
) -> std::result::Result<(), String> {
    state
        .0
        .import_json_entries(entries)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn user_data_path() -> std::result::Result<String, String> {
    let paths = Paths::resolve();
    Ok(paths.user_db_file().to_string_lossy().to_string())
}

#[tauri::command]
async fn discover_harness_imports() -> std::result::Result<HarnessImportPreview, String> {
    tokio::task::spawn_blocking(discover_harness_imports_blocking)
        .await
        .map_err(|e| format!("harness import discovery failed: {e}"))?
}

fn discover_harness_imports_blocking() -> std::result::Result<HarnessImportPreview, String> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    let mut preview = HarnessImportPreview {
        mcps: Vec::new(),
        skills: Vec::new(),
    };

    if let Some(home) = home.as_deref() {
        read_codex_mcp_config(&home.join(".codex").join("config.toml"), &mut preview.mcps);
        collect_skill_candidates(
            &home.join(".codex").join("skills"),
            "Codex",
            &mut preview.skills,
        );
        read_json_mcp_config(
            &home.join(".claude").join("settings.json"),
            "Claude",
            &mut preview.mcps,
        );
        collect_skill_candidates(
            &home.join(".claude").join("skills"),
            "Claude",
            &mut preview.skills,
        );
    }
    if let Some(appdata) = appdata.as_deref() {
        read_json_mcp_config(
            &appdata.join("Claude").join("claude_desktop_config.json"),
            "Claude Desktop",
            &mut preview.mcps,
        );
    }

    preview
        .mcps
        .sort_by(|a, b| a.harness.cmp(&b.harness).then_with(|| a.name.cmp(&b.name)));
    preview
        .skills
        .sort_by(|a, b| a.harness.cmp(&b.harness).then_with(|| a.name.cmp(&b.name)));
    preview.mcps.dedup_by(|a, b| {
        a.harness == b.harness && a.name == b.name && a.command == b.command && a.args == b.args
    });
    preview.skills.dedup_by(|a, b| a.path == b.path);

    Ok(preview)
}

fn read_json_mcp_config(path: &Path, harness: &str, out: &mut Vec<HarnessMcpCandidate>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(root) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return;
    };
    for (name, cfg) in servers {
        let Some(command) = cfg.get("command").and_then(Value::as_str) else {
            continue;
        };
        let args = cfg
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !command.trim().is_empty() {
            let (env, warnings) = json_mcp_env(cfg);
            out.push(HarnessMcpCandidate {
                harness: harness.to_string(),
                name: name.to_string(),
                command: command.to_string(),
                args,
                cwd: cfg
                    .get("cwd")
                    .or_else(|| cfg.get("working_directory"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                env,
                warnings,
                source_path: path.to_string_lossy().to_string(),
            });
        }
    }
}

fn json_mcp_env(cfg: &Value) -> (Vec<HarnessMcpEnvCandidate>, Vec<String>) {
    let mut warnings = Vec::new();
    let env = cfg
        .get("env")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| {
                    let key = key.trim();
                    if key.is_empty() {
                        return None;
                    }
                    let Some(value) = value.as_str() else {
                        warnings.push(format!("env.{key} is not a string and was skipped"));
                        return None;
                    };
                    Some(harness_env_candidate(key, value))
                })
                .collect()
        })
        .unwrap_or_default();
    (env, warnings)
}

fn harness_env_candidate(key: &str, value: &str) -> HarnessMcpEnvCandidate {
    let secret = secret_env_key(key);
    HarnessMcpEnvCandidate {
        key: key.to_string(),
        value: (!secret).then(|| value.to_string()),
        secret,
        required: secret,
    }
}

fn secret_env_key(key: &str) -> bool {
    milim_mcp_client::secret_env_key(key)
}

#[cfg(test)]
mod tailscale_tests {
    use super::*;

    fn tailscale_status(
        installed: bool,
        logged_in: bool,
        serve_configured: bool,
        public_url: Option<&str>,
    ) -> MobileTailscaleStatus {
        MobileTailscaleStatus {
            installed,
            logged_in,
            serve_configured,
            public_url: public_url.map(str::to_string),
            local_target: "http://127.0.0.1:12345".to_string(),
            message: None,
        }
    }

    #[test]
    fn tailscale_status_json_reports_dns_url() {
        let raw = r#"{"BackendState":"Running","Self":{"DNSName":"milim-box.tailnet.ts.net."}}"#;
        let (logged_in, dns_name, peer_count) = parse_tailscale_status_json(raw);
        assert!(logged_in);
        assert_eq!(dns_name.as_deref(), Some("milim-box.tailnet.ts.net"));
        assert_eq!(peer_count, 0);
    }

    #[test]
    fn tailscale_status_json_classifies_logged_out() {
        let (logged_in, dns_name, peer_count) =
            parse_tailscale_status_json(r#"{"BackendState":"NeedsLogin"}"#);
        assert!(!logged_in);
        assert!(dns_name.is_none());
        assert_eq!(peer_count, 0);
    }

    #[test]
    fn tailscale_status_json_reports_peer_count() {
        let raw = r#"{"BackendState":"Running","Peer":{"node-a":{},"node-b":{}}}"#;
        let (logged_in, _, peer_count) = parse_tailscale_status_json(raw);
        assert!(logged_in);
        assert_eq!(peer_count, 2);
    }

    #[test]
    fn tailscale_serve_json_reports_http_url() {
        let raw = r#"{
          "TCP": { "10000": { "HTTP": true } },
          "Web": {
            "milim-box.tailnet.ts.net:10000": {
              "Handlers": { "/": { "Proxy": "http://127.0.0.1:12345" } }
            }
          }
        }"#;
        assert_eq!(
            parse_tailscale_serve_url(raw, "http://127.0.0.1:12345", "milim-box.tailnet.ts.net")
                .as_deref(),
            Some("http://milim-box.tailnet.ts.net:10000")
        );
    }

    #[test]
    fn tailscale_serve_json_reports_https_url() {
        let raw = r#"{
          "Web": {
            "milim-box.tailnet.ts.net:10000": {
              "Handlers": { "/": { "Proxy": "http://127.0.0.1:12345" } }
            }
          }
        }"#;
        assert_eq!(
            parse_tailscale_serve_url(raw, "http://127.0.0.1:12345", "milim-box.tailnet.ts.net")
                .as_deref(),
            Some("https://milim-box.tailnet.ts.net:10000")
        );
    }

    #[test]
    fn tailscale_missing_cli_status_is_actionable() {
        let status = tailscale_not_found_status("http://127.0.0.1:12345".to_string());
        assert!(!status.installed);
        assert!(!status.logged_in);
        assert_eq!(status.local_target, "http://127.0.0.1:12345");
        assert!(status.message.unwrap().contains("Tailscale CLI not found"));
    }

    #[test]
    fn macos_tailscale_bundle_is_forced_into_cli_mode() {
        assert_eq!(
            tailscale_cli_environment("macos"),
            Some(("TAILSCALE_BE_CLI", "1"))
        );
        assert_eq!(tailscale_cli_environment("windows"), None);
        assert_eq!(tailscale_cli_environment("linux"), None);
    }

    #[test]
    fn tailscale_startup_retry_is_bounded_by_failure_stage() {
        let starting = tailscale_status(true, false, false, None);
        assert_eq!(
            mobile_tailscale_startup_retry_delay(1, &starting),
            Some(TAILSCALE_STARTUP_RETRY_DELAY)
        );
        assert_eq!(
            mobile_tailscale_startup_retry_delay(TAILSCALE_STARTUP_READY_ATTEMPTS, &starting),
            None
        );

        let configuring = tailscale_status(
            true,
            true,
            false,
            Some("https://milim-box.tailnet.ts.net:10000"),
        );
        assert_eq!(
            mobile_tailscale_startup_retry_delay(2, &configuring),
            Some(TAILSCALE_STARTUP_RETRY_DELAY)
        );
        assert_eq!(
            mobile_tailscale_startup_retry_delay(TAILSCALE_STARTUP_CONFIG_ATTEMPTS, &configuring),
            None
        );

        let ready = tailscale_status(
            true,
            true,
            true,
            Some("https://milim-box.tailnet.ts.net:10000"),
        );
        assert_eq!(mobile_tailscale_startup_retry_delay(1, &ready), None);
        assert_eq!(
            mobile_tailscale_startup_retry_delay(1, &tailscale_status(false, false, false, None)),
            None
        );
    }
}

fn read_codex_mcp_config(path: &Path, out: &mut Vec<HarnessMcpCandidate>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(root) = text.parse::<TomlValue>() else {
        return;
    };
    let Some(servers) = root.get("mcp_servers").and_then(TomlValue::as_table) else {
        return;
    };

    for (name, cfg) in servers {
        let Some(table) = cfg.as_table() else {
            continue;
        };
        let Some(command) = table.get("command").and_then(TomlValue::as_str) else {
            continue;
        };
        if command.trim().is_empty() {
            continue;
        }
        let args = table
            .get("args")
            .and_then(TomlValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(TomlValue::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let (env, warnings) = toml_mcp_env(table);
        out.push(HarnessMcpCandidate {
            harness: "Codex".to_string(),
            name: name.to_string(),
            command: command.to_string(),
            args,
            cwd: table
                .get("cwd")
                .or_else(|| table.get("working_directory"))
                .and_then(TomlValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
            env,
            warnings,
            source_path: path.to_string_lossy().to_string(),
        });
    }
}

fn toml_mcp_env(
    table: &toml::map::Map<String, TomlValue>,
) -> (Vec<HarnessMcpEnvCandidate>, Vec<String>) {
    let mut warnings = Vec::new();
    let env = table
        .get("env")
        .and_then(TomlValue::as_table)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| {
                    let key = key.trim();
                    if key.is_empty() {
                        return None;
                    }
                    let Some(value) = value.as_str() else {
                        warnings.push(format!("env.{key} is not a string and was skipped"));
                        return None;
                    };
                    Some(harness_env_candidate(key, value))
                })
                .collect()
        })
        .unwrap_or_default();
    (env, warnings)
}

fn collect_skill_candidates(root: &Path, harness: &str, out: &mut Vec<HarnessSkillCandidate>) {
    if !root.is_dir() {
        return;
    }
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if name == ".git" || name == "node_modules" || name == ".system" {
                    continue;
                }
                dirs.push(path);
            } else if name.eq_ignore_ascii_case("SKILL.md") {
                if out.len() >= 100 {
                    return;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.len() > 512 * 1024 {
                    continue;
                }
                let Ok(skill_md) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let skill_name = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|value| value.to_str())
                    .unwrap_or("imported-skill")
                    .to_string();
                out.push(HarnessSkillCandidate {
                    harness: harness.to_string(),
                    name: skill_name,
                    path: path.to_string_lossy().to_string(),
                    skill_md,
                });
            }
        }
    }
}

#[tauri::command]
async fn pick_attachment_files(
    app: tauri::AppHandle,
    max_bytes: Option<u64>,
) -> std::result::Result<Vec<AttachmentFilePayload>, String> {
    tokio::task::spawn_blocking(
        move || -> std::result::Result<Vec<AttachmentFilePayload>, String> {
            let Some(paths) = app.dialog().file().blocking_pick_files() else {
                return Ok(Vec::new());
            };
            paths
                .into_iter()
                .map(|path| {
                    let path = path
                        .into_path()
                        .map_err(|_| "picked attachment is not a local file".to_string())?;
                    read_attachment_file_blocking(&path, max_bytes)
                })
                .collect()
        },
    )
    .await
    .map_err(|e| format!("attachment read task failed: {e}"))?
}

#[tauri::command]
async fn read_workspace_attachment_file(
    workspace: String,
    path: String,
    max_bytes: Option<u64>,
) -> std::result::Result<AttachmentFilePayload, String> {
    tokio::task::spawn_blocking(move || {
        let path = resolve_workspace_attachment_path(&workspace, &path)?;
        read_attachment_file_blocking(&path, max_bytes)
    })
    .await
    .map_err(|e| format!("attachment read task failed: {e}"))?
}

fn read_attachment_file_blocking(
    path: &Path,
    max_bytes: Option<u64>,
) -> std::result::Result<AttachmentFilePayload, String> {
    use base64::Engine;

    let metadata =
        std::fs::metadata(path).map_err(|e| format!("failed to read attachment metadata: {e}"))?;
    if !metadata.is_file() {
        return Err("attachment path is not a file".to_string());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();
    let mime = attachment_mime_for_path(path).to_string();
    let mut content = None;
    let mut data_url = None;
    let mut truncated = false;

    if attachment_is_image_mime(&mime) {
        if metadata.len() == 0 || metadata.len() > MAX_ATTACHMENT_IMAGE_PREVIEW_BYTES {
            return Err(format!(
                "image attachments must contain 1 byte to {} bytes",
                MAX_ATTACHMENT_IMAGE_PREVIEW_BYTES
            ));
        }
        let bytes = fs::read(path).map_err(|e| format!("failed to read attachment file: {e}"))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        data_url = Some(format!("data:{mime};base64,{encoded}"));
    } else if attachment_is_text_like_mime(&mime) {
        let limit = max_bytes
            .unwrap_or(MAX_ATTACHMENT_BYTES)
            .clamp(1, MAX_ATTACHMENT_BYTES);
        let mut file =
            File::open(path).map_err(|e| format!("failed to open attachment file: {e}"))?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("failed to read attachment file: {e}"))?;
        truncated = bytes.len() as u64 > limit;
        if truncated {
            bytes.truncate(limit as usize);
        }
        content = Some(String::from_utf8_lossy(&bytes).into_owned());
    } else {
        return Err(format!(
            "{name} is not a supported text or image attachment"
        ));
    }

    Ok(AttachmentFilePayload {
        name,
        path: path.to_string_lossy().to_string(),
        size: metadata.len(),
        mime,
        content,
        data_url,
        truncated,
    })
}

fn attachment_mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "jsx" | "ts" | "tsx" | "rs" | "py" | "go" | "java" | "c" | "cpp" | "h" | "hpp"
        | "toml" | "yaml" | "yml" | "xml" | "txt" => "text/plain",
        _ => "application/octet-stream",
    }
}

fn attachment_is_image_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

fn attachment_is_text_like_mime(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json" | "application/xml" | "application/javascript"
        )
}

fn resolve_workspace_attachment_path(
    workspace: &str,
    path: &str,
) -> std::result::Result<PathBuf, String> {
    let root = workspace_root(workspace)?;
    let target = root.join(safe_artifact_relative_path(path)?);
    let root =
        fs::canonicalize(&root).map_err(|e| format!("failed to resolve working folder: {e}"))?;
    let target =
        fs::canonicalize(&target).map_err(|e| format!("failed to resolve attachment path: {e}"))?;
    if !target.starts_with(&root) {
        return Err("attachment path must stay inside the working folder".to_string());
    }
    Ok(target)
}

#[tauri::command]
async fn list_workspace_files(
    workspace: String,
    query: String,
    limit: Option<usize>,
) -> std::result::Result<Vec<WorkspaceFileSuggestion>, String> {
    tokio::task::spawn_blocking(move || list_workspace_files_blocking(&workspace, &query, limit))
        .await
        .map_err(|e| format!("workspace file list task failed: {e}"))?
}

#[tauri::command]
async fn list_workspace_directory(
    workspace: String,
    path: String,
    cursor: Option<String>,
    limit: Option<usize>,
) -> std::result::Result<WorkspaceDirectoryPage, String> {
    tokio::task::spawn_blocking(move || {
        list_workspace_directory_blocking(&workspace, &path, cursor.as_deref(), limit)
    })
    .await
    .map_err(|error| format!("workspace directory task failed: {error}"))?
}

fn list_workspace_directory_blocking(
    workspace: &str,
    path: &str,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> std::result::Result<WorkspaceDirectoryPage, String> {
    let root = fs::canonicalize(workspace_root(workspace)?)
        .map_err(|error| format!("failed to resolve working folder: {error}"))?;
    let relative = if path.trim().is_empty() {
        PathBuf::new()
    } else {
        safe_artifact_relative_path(path)?
    };
    let lexical = root.join(&relative);
    let directory = fs::canonicalize(&lexical)
        .map_err(|error| format!("failed to resolve workspace directory: {error}"))?;
    if !directory.starts_with(&root) || directory != lexical {
        return Err("workspace directory must not traverse a symlink or junction".to_string());
    }
    if !directory.is_dir() {
        return Err("workspace path is not a directory".to_string());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("failed to list workspace directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to read workspace entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect workspace entry: {error}"))?;
        if file_type.is_symlink() {
            continue;
        }
        let entry_path = entry.path();
        let resolved = fs::canonicalize(&entry_path)
            .map_err(|error| format!("failed to resolve workspace entry: {error}"))?;
        if !resolved.starts_with(&root) || resolved != entry_path {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if file_type.is_dir() && skip_workspace_suggestion_dir(&name) {
            continue;
        }
        let kind = if file_type.is_dir() {
            "directory"
        } else if file_type.is_file() {
            "file"
        } else {
            continue;
        };
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect workspace entry: {error}"))?;
        entries.push(WorkspaceDirectoryEntry {
            path: workspace_relative_path(&root, &entry_path),
            name,
            kind,
            size: file_type.is_file().then_some(metadata.len()),
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != "directory", a.name.to_ascii_lowercase())
            .cmp(&(b.kind != "directory", b.name.to_ascii_lowercase()))
    });
    let offset = cursor
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| "invalid directory cursor".to_string())?;
    let page_size = limit.unwrap_or(200).clamp(1, 200);
    let total = entries.len();
    let end = (offset + page_size).min(total);
    let page = if offset < total {
        entries.drain(offset..end).collect()
    } else {
        Vec::new()
    };
    Ok(WorkspaceDirectoryPage {
        entries: page,
        next_cursor: (end < total).then(|| end.to_string()),
    })
}

#[tauri::command]
async fn read_workspace_text_file(
    workspace: String,
    path: String,
) -> std::result::Result<WorkspaceTextFile, String> {
    tokio::task::spawn_blocking(move || read_workspace_text_file_blocking(&workspace, &path))
        .await
        .map_err(|error| format!("workspace text read task failed: {error}"))?
}

const MAX_WORKSPACE_TEXT_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn workspace_text_file_bytes(
    workspace: &str,
    path: &str,
) -> std::result::Result<(PathBuf, Vec<u8>), String> {
    let root = workspace_root(workspace)?;
    let target = resolve_workspace_path(&root, path).map_err(|error| error.to_string())?;
    let metadata = fs::metadata(&target)
        .map_err(|error| format!("failed to inspect workspace file: {error}"))?;
    if !metadata.is_file() {
        return Err("workspace path is not a file".to_string());
    }
    if metadata.len() > MAX_WORKSPACE_TEXT_FILE_BYTES {
        return Err("workspace file exceeds the 2 MiB limit".to_string());
    }
    let bytes =
        fs::read(&target).map_err(|error| format!("failed to read workspace file: {error}"))?;
    if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        return Err("binary files cannot be edited".to_string());
    }
    Ok((target, bytes))
}

fn read_workspace_text_file_blocking(
    workspace: &str,
    path: &str,
) -> std::result::Result<WorkspaceTextFile, String> {
    let (_, bytes) = workspace_text_file_bytes(workspace, path)?;
    let size = bytes.len() as u64;
    let revision = sha256_hex(&bytes);
    Ok(WorkspaceTextFile {
        path: path.to_string(),
        content: String::from_utf8(bytes).expect("workspace text was validated as UTF-8"),
        size,
        revision,
    })
}

#[tauri::command]
async fn write_workspace_text_file(
    workspace: String,
    path: String,
    content: String,
    expected_revision: String,
    force: Option<bool>,
) -> std::result::Result<WorkspaceTextWriteResult, String> {
    tokio::task::spawn_blocking(move || {
        write_workspace_text_file_blocking(
            &workspace,
            &path,
            &content,
            &expected_revision,
            force.unwrap_or(false),
        )
    })
    .await
    .map_err(|error| format!("workspace text write task failed: {error}"))?
}

fn write_workspace_text_file_blocking(
    workspace: &str,
    path: &str,
    content: &str,
    expected_revision: &str,
    force: bool,
) -> std::result::Result<WorkspaceTextWriteResult, String> {
    if content.len() as u64 > MAX_WORKSPACE_TEXT_FILE_BYTES {
        return Err("workspace file exceeds the 2 MiB limit".to_string());
    }
    let (target, current) = workspace_text_file_bytes(workspace, path)?;
    let current_revision = sha256_hex(&current);
    if !force && current_revision != expected_revision {
        return Ok(WorkspaceTextWriteResult::Conflict {
            revision: current_revision,
        });
    }
    let content = content.as_bytes();
    if current != content {
        atomic_write(&target, content).map_err(|error| error.to_string())?;
    }
    Ok(WorkspaceTextWriteResult::Saved {
        path: path.to_string(),
        size: content.len() as u64,
        revision: sha256_hex(content),
    })
}

fn list_workspace_files_blocking(
    workspace: &str,
    query: &str,
    limit: Option<usize>,
) -> std::result::Result<Vec<WorkspaceFileSuggestion>, String> {
    let root = workspace_root(workspace)?;
    let max = limit.unwrap_or(20).clamp(1, 50);
    let needle = query.trim().to_lowercase();
    let mut out = Vec::new();
    let mut scanned = 0usize;
    let mut dirs = vec![root.clone()];

    while let Some(dir) = dirs.pop() {
        let mut entries = match fs::read_dir(&dir) {
            Ok(entries) => entries.filter_map(|entry| entry.ok()).collect::<Vec<_>>(),
            Err(_) => continue,
        };
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            scanned += 1;
            if scanned > MAX_WORKSPACE_FILE_SUGGESTION_SCAN || out.len() >= max {
                return Ok(out);
            }

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_symlink() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if file_type.is_dir() {
                if !skip_workspace_suggestion_dir(&name) {
                    dirs.push(entry.path());
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let path = entry.path();
            let relative = workspace_relative_path(&root, &path);
            if !needle.is_empty() && !relative.to_lowercase().contains(&needle) {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            out.push(WorkspaceFileSuggestion {
                path: relative,
                full_path: path.to_string_lossy().to_string(),
                name,
                size: metadata.len(),
            });
        }
    }

    Ok(out)
}

fn workspace_root(workspace: &str) -> std::result::Result<PathBuf, String> {
    let root = PathBuf::from(workspace.trim());
    if root.as_os_str().is_empty() {
        return Err("no working folder selected".to_string());
    }
    let metadata =
        fs::metadata(&root).map_err(|e| format!("failed to read working folder metadata: {e}"))?;
    if !metadata.is_dir() {
        return Err("working folder is not a directory".to_string());
    }
    Ok(root)
}

fn skip_workspace_suggestion_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | "dist" | "build" | ".next" | ".turbo"
    )
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[tauri::command]
async fn save_artifact_file(
    workspace: String,
    path: String,
    content: String,
    overwrite: Option<bool>,
) -> std::result::Result<SavedArtifactFilePayload, String> {
    tokio::task::spawn_blocking(move || {
        save_artifact_file_blocking(workspace, path, content, overwrite.unwrap_or(false))
    })
    .await
    .map_err(|e| format!("artifact save task failed: {e}"))?
}

fn save_artifact_file_blocking(
    workspace: String,
    path: String,
    content: String,
    overwrite: bool,
) -> std::result::Result<SavedArtifactFilePayload, String> {
    let target = resolve_artifact_target(&workspace, &path)?;
    let existed = target.exists();
    if existed && !overwrite {
        return Err(format!(
            "artifact file already exists: {}",
            target.display()
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create artifact directory: {e}"))?;
    }
    std::fs::write(&target, content.as_bytes())
        .map_err(|e| format!("failed to write artifact file: {e}"))?;
    Ok(SavedArtifactFilePayload {
        path: target.to_string_lossy().to_string(),
        bytes: content.len(),
        overwritten: existed,
    })
}

fn resolve_artifact_target(workspace: &str, path: &str) -> std::result::Result<PathBuf, String> {
    let workspace = PathBuf::from(workspace.trim());
    if workspace.as_os_str().is_empty() {
        return Err("no working folder selected".to_string());
    }
    let metadata = std::fs::metadata(&workspace)
        .map_err(|e| format!("failed to read working folder metadata: {e}"))?;
    if !metadata.is_dir() {
        return Err("working folder is not a directory".to_string());
    }

    Ok(workspace.join(safe_artifact_relative_path(path)?))
}

fn safe_artifact_relative_path(path: &str) -> std::result::Result<PathBuf, String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err("artifact path is required".to_string());
    }
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(part) => {
                let value = part.to_string_lossy();
                if value.contains(':') {
                    return Err("artifact paths must be relative".to_string());
                }
                out.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err("'..' is not allowed in artifact paths".to_string()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("artifact paths must be relative".to_string())
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err("artifact path is required".to_string());
    }
    Ok(out)
}

#[tauri::command]
async fn preview_artifact_file(
    workspace: String,
    path: String,
    content: String,
) -> std::result::Result<ArtifactWritePreviewPayload, String> {
    tokio::task::spawn_blocking(move || preview_artifact_file_blocking(workspace, path, content))
        .await
        .map_err(|e| format!("artifact preview task failed: {e}"))?
}

fn preview_artifact_file_blocking(
    workspace: String,
    path: String,
    content: String,
) -> std::result::Result<ArtifactWritePreviewPayload, String> {
    let target = resolve_artifact_target(&workspace, &path)?;
    let mut truncated = false;
    let (exists, old_content, old_bytes) = match std::fs::metadata(&target) {
        Ok(metadata) => {
            if !metadata.is_file() {
                return Err("artifact target is not a file".to_string());
            }
            let mut bytes = std::fs::read(&target)
                .map_err(|e| format!("failed to read artifact target: {e}"))?;
            let original_len = bytes.len();
            if bytes.len() > MAX_ARTIFACT_PREVIEW_BYTES {
                bytes.truncate(MAX_ARTIFACT_PREVIEW_BYTES);
                truncated = true;
            }
            (
                true,
                Some(String::from_utf8_lossy(&bytes).into_owned()),
                Some(original_len),
            )
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, None, None),
        Err(e) => return Err(format!("failed to read artifact target metadata: {e}")),
    };
    let diff = artifact_line_diff(old_content.as_deref().unwrap_or(""), &content);
    Ok(ArtifactWritePreviewPayload {
        path: target.to_string_lossy().to_string(),
        exists,
        changed: old_content.as_deref() != Some(content.as_str()),
        old_content,
        new_content: content.clone(),
        old_bytes,
        new_bytes: content.len(),
        diff,
        truncated,
    })
}

#[derive(Clone, Copy)]
enum ArtifactDiffKind {
    Equal,
    Delete,
    Insert,
}

struct ArtifactDiffLine<'a> {
    kind: ArtifactDiffKind,
    text: &'a str,
    old_before: usize,
    new_before: usize,
}

impl ArtifactDiffLine<'_> {
    fn consumes_old(&self) -> bool {
        matches!(
            self.kind,
            ArtifactDiffKind::Equal | ArtifactDiffKind::Delete
        )
    }

    fn consumes_new(&self) -> bool {
        matches!(
            self.kind,
            ArtifactDiffKind::Equal | ArtifactDiffKind::Insert
        )
    }

    fn is_change(&self) -> bool {
        !matches!(self.kind, ArtifactDiffKind::Equal)
    }
}

fn artifact_line_diff(old: &str, new: &str) -> String {
    if old == new {
        return "No changes.".to_string();
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut diff = vec!["--- current".to_string(), "+++ generated".to_string()];

    let lines = artifact_diff_lines(&old_lines, &new_lines);
    let hunks = artifact_diff_hunks(&lines);
    let mut previous_end = 0;
    for (start, end) in hunks {
        let omitted = lines[previous_end..start]
            .iter()
            .filter(|line| matches!(line.kind, ArtifactDiffKind::Equal))
            .count();
        if previous_end > 0 && omitted > 0 {
            diff.push(format!("... {omitted} unchanged lines ..."));
        }
        let hunk = &lines[start..end];
        let old_start = hunk
            .iter()
            .find(|line| line.consumes_old())
            .map(|line| line.old_before + 1)
            .unwrap_or_else(|| hunk.first().map(|line| line.old_before + 1).unwrap_or(1));
        let new_start = hunk
            .iter()
            .find(|line| line.consumes_new())
            .map(|line| line.new_before + 1)
            .unwrap_or_else(|| hunk.first().map(|line| line.new_before + 1).unwrap_or(1));
        let old_count = hunk.iter().filter(|line| line.consumes_old()).count();
        let new_count = hunk.iter().filter(|line| line.consumes_new()).count();
        diff.push(format!(
            "@@ -{},{} +{},{} @@",
            old_start, old_count, new_start, new_count
        ));
        for line in hunk {
            let prefix = match line.kind {
                ArtifactDiffKind::Equal => ' ',
                ArtifactDiffKind::Delete => '-',
                ArtifactDiffKind::Insert => '+',
            };
            diff.push(format!("{prefix}{}", line.text));
        }
        previous_end = end;
    }
    diff.join("\n")
}

fn artifact_diff_lines<'a>(
    old_lines: &[&'a str],
    new_lines: &[&'a str],
) -> Vec<ArtifactDiffLine<'a>> {
    let cells = old_lines
        .len()
        .saturating_add(1)
        .saturating_mul(new_lines.len().saturating_add(1));
    if cells > ARTIFACT_DIFF_LCS_CELL_LIMIT {
        return artifact_positional_diff_lines(old_lines, new_lines);
    }

    let cols = new_lines.len() + 1;
    let mut table = vec![0usize; (old_lines.len() + 1) * cols];
    for i in (0..old_lines.len()).rev() {
        for j in (0..new_lines.len()).rev() {
            let value = if old_lines[i] == new_lines[j] {
                table[(i + 1) * cols + j + 1] + 1
            } else {
                table[(i + 1) * cols + j].max(table[i * cols + j + 1])
            };
            table[i * cols + j] = value;
        }
    }

    let mut lines = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < old_lines.len() && new_index < new_lines.len() {
        if old_lines[old_index] == new_lines[new_index] {
            lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Equal,
                text: old_lines[old_index],
                old_before: old_index,
                new_before: new_index,
            });
            old_index += 1;
            new_index += 1;
        } else if table[(old_index + 1) * cols + new_index]
            >= table[old_index * cols + new_index + 1]
        {
            lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Delete,
                text: old_lines[old_index],
                old_before: old_index,
                new_before: new_index,
            });
            old_index += 1;
        } else {
            lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Insert,
                text: new_lines[new_index],
                old_before: old_index,
                new_before: new_index,
            });
            new_index += 1;
        }
    }
    while old_index < old_lines.len() {
        lines.push(ArtifactDiffLine {
            kind: ArtifactDiffKind::Delete,
            text: old_lines[old_index],
            old_before: old_index,
            new_before: new_index,
        });
        old_index += 1;
    }
    while new_index < new_lines.len() {
        lines.push(ArtifactDiffLine {
            kind: ArtifactDiffKind::Insert,
            text: new_lines[new_index],
            old_before: old_index,
            new_before: new_index,
        });
        new_index += 1;
    }
    lines
}

fn artifact_positional_diff_lines<'a>(
    old_lines: &[&'a str],
    new_lines: &[&'a str],
) -> Vec<ArtifactDiffLine<'a>> {
    let mut lines = Vec::new();
    let len = old_lines.len().max(new_lines.len());
    for index in 0..len {
        match (old_lines.get(index), new_lines.get(index)) {
            (Some(left), Some(right)) if left == right => lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Equal,
                text: left,
                old_before: index,
                new_before: index,
            }),
            (Some(left), Some(right)) => {
                lines.push(ArtifactDiffLine {
                    kind: ArtifactDiffKind::Delete,
                    text: left,
                    old_before: index,
                    new_before: index,
                });
                lines.push(ArtifactDiffLine {
                    kind: ArtifactDiffKind::Insert,
                    text: right,
                    old_before: index + 1,
                    new_before: index,
                });
            }
            (Some(left), None) => lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Delete,
                text: left,
                old_before: index,
                new_before: new_lines.len(),
            }),
            (None, Some(right)) => lines.push(ArtifactDiffLine {
                kind: ArtifactDiffKind::Insert,
                text: right,
                old_before: old_lines.len(),
                new_before: index,
            }),
            (None, None) => {}
        }
    }
    lines
}

fn artifact_diff_hunks(lines: &[ArtifactDiffLine<'_>]) -> Vec<(usize, usize)> {
    let mut hunks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].is_change() {
            index += 1;
            continue;
        }
        let change_start = index;
        let mut change_end = index + 1;
        while change_end < lines.len() && lines[change_end].is_change() {
            change_end += 1;
        }
        let start = change_start.saturating_sub(ARTIFACT_DIFF_CONTEXT_LINES);
        let end = (change_end + ARTIFACT_DIFF_CONTEXT_LINES).min(lines.len());
        if let Some((_, previous_end)) = hunks.last_mut() {
            if start <= *previous_end {
                *previous_end = end;
            } else {
                hunks.push((start, end));
            }
        } else {
            hunks.push((start, end));
        }
        index = change_end;
    }
    hunks
}

#[tauri::command]
async fn artifact_file_status(
    path: String,
) -> std::result::Result<ArtifactFileStatusPayload, String> {
    tokio::task::spawn_blocking(move || artifact_file_status_blocking(path))
        .await
        .map_err(|e| format!("artifact status task failed: {e}"))?
}

fn artifact_file_status_blocking(
    path: String,
) -> std::result::Result<ArtifactFileStatusPayload, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("artifact path is required".to_string());
    }
    let path = PathBuf::from(trimmed);
    match std::fs::metadata(&path) {
        Ok(metadata) => Ok(ArtifactFileStatusPayload {
            path: path.to_string_lossy().to_string(),
            exists: true,
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            bytes: metadata.is_file().then_some(metadata.len()),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ArtifactFileStatusPayload {
            path: path.to_string_lossy().to_string(),
            exists: false,
            is_file: false,
            is_dir: false,
            bytes: None,
        }),
        Err(e) => Err(format!("failed to read artifact file status: {e}")),
    }
}

#[tauri::command]
async fn open_artifact_location(
    path: String,
    target: Option<String>,
) -> std::result::Result<(), String> {
    tokio::task::spawn_blocking(move || {
        open_artifact_location_blocking(path, target.unwrap_or_else(|| "file".to_string()))
    })
    .await
    .map_err(|e| format!("artifact open task failed: {e}"))?
}

fn open_artifact_location_blocking(
    path: String,
    target: String,
) -> std::result::Result<(), String> {
    let path = validate_artifact_open_path(Path::new(path.trim()))?;
    let target = parse_artifact_open_target(&target)?;
    let spec = artifact_open_command(&path, target)?;
    record_subprocess_launch();
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    milim_core::proc::hide_console(&mut cmd)
        .spawn()
        .map_err(|e| format!("failed to open artifact: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn list_workspace_launchers(
    workspace: String,
) -> std::result::Result<Vec<WorkspaceLauncherPayload>, String> {
    tokio::task::spawn_blocking(move || list_workspace_launchers_blocking(&workspace))
        .await
        .map_err(|e| format!("workspace launcher list task failed: {e}"))?
}

fn list_workspace_launchers_blocking(
    workspace: &str,
) -> std::result::Result<Vec<WorkspaceLauncherPayload>, String> {
    let root = validate_workspace_launcher_root(workspace)?;
    Ok(workspace_launcher_ids()
        .iter()
        .map(|id| workspace_launcher_payload(&root, *id))
        .collect())
}

#[tauri::command]
async fn open_workspace_launcher(
    workspace: String,
    launcher_id: String,
) -> std::result::Result<(), String> {
    tokio::task::spawn_blocking(move || open_workspace_launcher_blocking(&workspace, &launcher_id))
        .await
        .map_err(|e| format!("workspace launcher task failed: {e}"))?
}

fn open_workspace_launcher_blocking(
    workspace: &str,
    launcher_id: &str,
) -> std::result::Result<(), String> {
    let root = validate_workspace_launcher_root(workspace)?;
    let id = parse_workspace_launcher_id(launcher_id)?;
    let platform = current_workspace_launcher_platform();
    let spec = workspace_launcher_command(&root, id, platform)?;
    record_subprocess_launch();
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    milim_core::proc::hide_console(&mut cmd)
        .spawn()
        .map_err(|e| format!("failed to open workspace: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn open_external_url(url: String) -> std::result::Result<(), String> {
    tokio::task::spawn_blocking(move || open_external_url_blocking(url))
        .await
        .map_err(|e| format!("URL open task failed: {e}"))?
}

fn open_external_url_blocking(url: String) -> std::result::Result<(), String> {
    let url = validate_external_url(&url)?;
    let spec = external_url_open_command(&url);
    record_subprocess_launch();
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    milim_core::proc::hide_console(&mut cmd)
        .spawn()
        .map_err(|e| format!("failed to open URL: {e}"))?;
    Ok(())
}

fn validate_external_url(url: &str) -> std::result::Result<String, String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return Err("external URL must be http or https".to_string());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("external URL must not contain whitespace".to_string());
    }
    Ok(url.to_string())
}

fn external_url_open_command(url: &str) -> ArtifactOpenCommandSpec {
    if cfg!(windows) {
        return ArtifactOpenCommandSpec {
            program: "rundll32".to_string(),
            args: vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
        };
    }
    if cfg!(target_os = "macos") {
        return ArtifactOpenCommandSpec {
            program: "open".to_string(),
            args: vec![url.to_string()],
        };
    }
    ArtifactOpenCommandSpec {
        program: "xdg-open".to_string(),
        args: vec![url.to_string()],
    }
}

fn mobile_tailscale_status_blocking(local_target: String) -> MobileTailscaleStatus {
    let Some(command) = find_tailscale_command() else {
        return tailscale_not_found_status(local_target);
    };
    let status_output = match run_tailscale(&command, &["status", "--json"]) {
        Ok(output) => output,
        Err(e) => {
            return MobileTailscaleStatus {
                installed: true,
                logged_in: false,
                serve_configured: false,
                public_url: None,
                local_target,
                message: Some(format!("Tailscale status failed: {e}")),
            }
        }
    };
    if !status_output.status.success() {
        return MobileTailscaleStatus {
            installed: true,
            logged_in: false,
            serve_configured: false,
            public_url: None,
            local_target,
            message: Some(first_output_line(
                &status_output,
                "Tailscale is not logged in.",
            )),
        };
    }

    let (logged_in, dns_name, peer_count) =
        parse_tailscale_status_json(&String::from_utf8_lossy(&status_output.stdout));
    let serve_url = if logged_in {
        dns_name
            .as_deref()
            .and_then(|dns_name| tailscale_serve_url(&command, &local_target, dns_name))
    } else {
        None
    };
    let serve_configured = serve_url.is_some();
    let public_url = serve_url.or_else(|| {
        dns_name
            .as_deref()
            .map(|dns_name| format!("https://{dns_name}:{TAILSCALE_SERVE_PORT}"))
    });
    let message = if logged_in {
        if public_url.is_none() {
            Some("Tailscale is running, but no DNS name was reported.".to_string())
        } else if serve_configured && peer_count == 0 {
            Some("Tailscale Serve is ready, but no other tailnet devices are visible. Open Tailscale on your mobile device and make sure it is connected to this same tailnet.".to_string())
        } else {
            None
        }
    } else {
        Some("Tailscale is not logged in.".to_string())
    };
    MobileTailscaleStatus {
        installed: true,
        logged_in,
        serve_configured,
        public_url,
        local_target,
        message,
    }
}

fn configure_mobile_tailscale_blocking(local_target: String) -> MobileTailscaleStatus {
    let Some(command) = find_tailscale_command() else {
        return tailscale_not_found_status(local_target);
    };
    let status = mobile_tailscale_status_blocking(local_target.clone());
    if !status.logged_in || status.public_url.is_none() {
        return status;
    }
    if status
        .public_url
        .as_deref()
        .is_some_and(|url| status.serve_configured && url.starts_with("https://"))
    {
        return status;
    }

    let https = format!("--https={TAILSCALE_SERVE_PORT}");
    let https_result = run_tailscale(
        &command,
        &["serve", "--bg", &https, "--yes", local_target.as_str()],
    );
    if let Ok(output) = &https_result {
        if output.status.success() {
            let mut next = mobile_tailscale_status_blocking(local_target.clone());
            if next.serve_configured {
                if next.message.is_none() {
                    next.message = Some("Tailscale Serve is ready over HTTPS.".to_string());
                }
                return next;
            }
        }
    }

    let http = format!("--http={TAILSCALE_SERVE_PORT}");
    match run_tailscale(
        &command,
        &["serve", "--bg", &http, "--yes", local_target.as_str()],
    ) {
        Ok(output) if output.status.success() => {
            let mut next = mobile_tailscale_status_blocking(local_target);
            if next.message.is_none() {
                next.message = Some("Tailscale Serve is ready over tailnet HTTP.".to_string());
            }
            next
        }
        Ok(output) => MobileTailscaleStatus {
            message: Some(first_output_line(
                &output,
                "Tailscale Serve setup failed after HTTPS and HTTP attempts.",
            )),
            ..status
        },
        Err(e) => MobileTailscaleStatus {
            message: Some(format!(
                "Tailscale Serve setup failed: HTTPS attempt failed; HTTP fallback failed: {e}"
            )),
            ..status
        },
    }
}

fn disable_mobile_tailscale_blocking(local_target: String) -> MobileTailscaleStatus {
    let Some(command) = find_tailscale_command() else {
        return tailscale_not_found_status(local_target);
    };
    let https = format!("--https={TAILSCALE_SERVE_PORT}");
    let http = format!("--http={TAILSCALE_SERVE_PORT}");
    let _ = run_tailscale(&command, &["serve", &https, "--yes", "off"]);
    let _ = run_tailscale(&command, &["serve", &http, "--yes", "off"]);
    mobile_tailscale_status_blocking(local_target)
}

fn mobile_tailscale_startup_retry_delay(
    attempt: usize,
    status: &MobileTailscaleStatus,
) -> Option<Duration> {
    if status.serve_configured || !status.installed {
        return None;
    }
    let attempts = if status.logged_in && status.public_url.is_some() {
        TAILSCALE_STARTUP_CONFIG_ATTEMPTS
    } else {
        TAILSCALE_STARTUP_READY_ATTEMPTS
    };
    (attempt < attempts).then_some(TAILSCALE_STARTUP_RETRY_DELAY)
}

async fn restore_mobile_tailscale_startup(local_target: String) {
    for attempt in 1..=TAILSCALE_STARTUP_READY_ATTEMPTS {
        let target = local_target.clone();
        let status =
            tokio::task::spawn_blocking(move || configure_mobile_tailscale_blocking(target)).await;
        match status {
            Ok(status) if status.serve_configured => {
                tracing::info!(
                    target: "milim_desktop::server",
                    attempt,
                    "mobile control Tailscale restored on startup"
                );
                return;
            }
            Ok(status) => {
                if let Some(delay) = mobile_tailscale_startup_retry_delay(attempt, &status) {
                    tokio::time::sleep(delay).await;
                    continue;
                }
                let message = status
                    .message
                    .as_deref()
                    .unwrap_or("Tailscale Serve did not become ready.");
                tracing::warn!(
                    target: "milim_desktop::server",
                    attempt,
                    "mobile control Tailscale startup unavailable: {message}"
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    target: "milim_desktop::server",
                    attempt,
                    "mobile control Tailscale startup task failed: {error}"
                );
                return;
            }
        }
    }
}

fn find_tailscale_command() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TAILSCALE_CLI").map(PathBuf::from) {
        if path.exists() {
            return Some(path);
        }
    }

    for path in tailscale_command_candidates() {
        if path.exists() {
            return Some(path);
        }
    }

    if run_tailscale(Path::new("tailscale"), &["version"]).is_ok() {
        return Some(PathBuf::from("tailscale"));
    }
    None
}

fn tailscale_command_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(root) = std::env::var_os(key).map(PathBuf::from) {
                paths.push(root.join("Tailscale").join("tailscale.exe"));
            }
        }
        return paths;
    }
    paths.extend([
        PathBuf::from("/usr/local/bin/tailscale"),
        PathBuf::from("/opt/homebrew/bin/tailscale"),
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/tailscale"),
    ]);
    paths
}

fn tailscale_cli_environment(target_os: &str) -> Option<(&'static str, &'static str)> {
    (target_os == "macos").then_some(("TAILSCALE_BE_CLI", "1"))
}

fn run_tailscale(command: &Path, args: &[&str]) -> io::Result<Output> {
    record_subprocess_launch();
    let mut cmd = Command::new(command);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some((key, value)) = tailscale_cli_environment(std::env::consts::OS) {
        cmd.env(key, value);
    }
    let mut child = milim_core::proc::hide_console(&mut cmd).spawn()?;
    let deadline = Instant::now() + Duration::from_secs(TAILSCALE_COMMAND_TIMEOUT_SECS);
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "tailscale {} timed out after {TAILSCALE_COMMAND_TIMEOUT_SECS}s",
                    args.join(" ")
                ),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn parse_tailscale_status_json(raw: &str) -> (bool, Option<String>, usize) {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return (false, None, 0);
    };
    let backend = value
        .get("BackendState")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let dns = value
        .get("Self")
        .and_then(|self_node| self_node.get("DNSName"))
        .and_then(Value::as_str)
        .map(|value| value.trim_end_matches('.').to_string())
        .filter(|value| !value.is_empty());
    let logged_in = dns.is_some()
        || matches!(
            backend,
            "Running" | "Starting" | "NeedsMachineAuth" | "Stopped"
        );
    let peer_count = value
        .get("Peer")
        .and_then(Value::as_object)
        .map(|peers| peers.len())
        .unwrap_or(0);
    (logged_in, dns, peer_count)
}

fn tailscale_serve_url(command: &Path, local_target: &str, dns_name: &str) -> Option<String> {
    run_tailscale(command, &["serve", "status", "--json"])
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            parse_tailscale_serve_url(
                &String::from_utf8_lossy(&output.stdout),
                local_target,
                dns_name,
            )
        })
}

fn parse_tailscale_serve_url(raw: &str, local_target: &str, dns_name: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    let port = TAILSCALE_SERVE_PORT.to_string();
    let host = format!("{dns_name}:{port}");
    let handler = value
        .get("Web")?
        .get(&host)?
        .get("Handlers")?
        .as_object()?
        .values()
        .find(|handler| {
            handler
                .get("Proxy")
                .and_then(Value::as_str)
                .is_some_and(|proxy| proxy.eq_ignore_ascii_case(local_target))
        })?;
    let _ = handler;
    let is_http = value
        .get("TCP")
        .and_then(|tcp| tcp.get(&port))
        .and_then(|config| config.get("HTTP"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(format!(
        "{}://{host}",
        if is_http { "http" } else { "https" }
    ))
}

fn first_output_line(output: &std::process::Output, fallback: &str) -> String {
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn tailscale_not_found_status(local_target: String) -> MobileTailscaleStatus {
    MobileTailscaleStatus {
        installed: false,
        logged_in: false,
        serve_configured: false,
        public_url: None,
        local_target,
        message: Some("Tailscale CLI not found. Install Tailscale and sign in first.".to_string()),
    }
}

fn validate_artifact_open_path(path: &Path) -> std::result::Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("artifact path is required".to_string());
    }
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("artifact path is not available: {e}"))?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("artifact path must be a file or directory".to_string());
    }
    Ok(path.to_path_buf())
}

fn parse_artifact_open_target(target: &str) -> std::result::Result<ArtifactOpenTarget, String> {
    match target {
        "file" => Ok(ArtifactOpenTarget::File),
        "folder" => Ok(ArtifactOpenTarget::Folder),
        _ => Err("artifact open target must be file or folder".to_string()),
    }
}

fn artifact_open_command(
    path: &Path,
    target: ArtifactOpenTarget,
) -> std::result::Result<ArtifactOpenCommandSpec, String> {
    let path_string = path.to_string_lossy().to_string();
    if cfg!(windows) {
        let args = match target {
            ArtifactOpenTarget::File => vec![path_string],
            ArtifactOpenTarget::Folder => {
                if path.is_file() {
                    vec![format!("/select,{path_string}")]
                } else {
                    vec![path_string]
                }
            }
        };
        return Ok(ArtifactOpenCommandSpec {
            program: "explorer".to_string(),
            args,
        });
    }
    if cfg!(target_os = "macos") {
        let args = match target {
            ArtifactOpenTarget::File => vec![path_string],
            ArtifactOpenTarget::Folder if path.is_file() => vec!["-R".to_string(), path_string],
            ArtifactOpenTarget::Folder => vec![path_string],
        };
        return Ok(ArtifactOpenCommandSpec {
            program: "open".to_string(),
            args,
        });
    }
    let target_path = match target {
        ArtifactOpenTarget::File => path.to_path_buf(),
        ArtifactOpenTarget::Folder if path.is_file() => path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf()),
        ArtifactOpenTarget::Folder => path.to_path_buf(),
    };
    Ok(ArtifactOpenCommandSpec {
        program: "xdg-open".to_string(),
        args: vec![target_path.to_string_lossy().to_string()],
    })
}

fn validate_workspace_launcher_root(workspace: &str) -> std::result::Result<PathBuf, String> {
    let workspace = workspace.trim();
    if workspace.is_empty() {
        return Err("workspace folder is required".to_string());
    }
    let root = PathBuf::from(workspace);
    let metadata =
        std::fs::metadata(&root).map_err(|e| format!("workspace folder is not available: {e}"))?;
    if !metadata.is_dir() {
        return Err("workspace launcher requires a directory".to_string());
    }
    Ok(root)
}

fn workspace_launcher_ids() -> [WorkspaceLauncherId; 7] {
    [
        WorkspaceLauncherId::Vscode,
        WorkspaceLauncherId::Zed,
        WorkspaceLauncherId::FileManager,
        WorkspaceLauncherId::Terminal,
        WorkspaceLauncherId::GitBash,
        WorkspaceLauncherId::Wsl,
        WorkspaceLauncherId::AndroidStudio,
    ]
}

fn parse_workspace_launcher_id(id: &str) -> std::result::Result<WorkspaceLauncherId, String> {
    match id.trim() {
        "vscode" => Ok(WorkspaceLauncherId::Vscode),
        "zed" => Ok(WorkspaceLauncherId::Zed),
        "file_manager" => Ok(WorkspaceLauncherId::FileManager),
        "terminal" => Ok(WorkspaceLauncherId::Terminal),
        "git_bash" => Ok(WorkspaceLauncherId::GitBash),
        "wsl" => Ok(WorkspaceLauncherId::Wsl),
        "android_studio" => Ok(WorkspaceLauncherId::AndroidStudio),
        _ => Err("unknown workspace launcher".to_string()),
    }
}

fn workspace_launcher_id(id: WorkspaceLauncherId) -> &'static str {
    match id {
        WorkspaceLauncherId::Vscode => "vscode",
        WorkspaceLauncherId::Zed => "zed",
        WorkspaceLauncherId::FileManager => "file_manager",
        WorkspaceLauncherId::Terminal => "terminal",
        WorkspaceLauncherId::GitBash => "git_bash",
        WorkspaceLauncherId::Wsl => "wsl",
        WorkspaceLauncherId::AndroidStudio => "android_studio",
    }
}

fn workspace_launcher_label(
    id: WorkspaceLauncherId,
    platform: WorkspaceLauncherPlatform,
) -> &'static str {
    match id {
        WorkspaceLauncherId::Vscode => "VS Code",
        WorkspaceLauncherId::Zed => "Zed",
        WorkspaceLauncherId::FileManager => match platform {
            WorkspaceLauncherPlatform::Macos => "Finder",
            _ => "File Explorer",
        },
        WorkspaceLauncherId::Terminal => "Terminal",
        WorkspaceLauncherId::GitBash => "Git Bash",
        WorkspaceLauncherId::Wsl => "WSL",
        WorkspaceLauncherId::AndroidStudio => "Android Studio",
    }
}

fn current_workspace_launcher_platform() -> WorkspaceLauncherPlatform {
    if cfg!(windows) {
        WorkspaceLauncherPlatform::Windows
    } else if cfg!(target_os = "macos") {
        WorkspaceLauncherPlatform::Macos
    } else {
        WorkspaceLauncherPlatform::Linux
    }
}

fn workspace_launcher_payload(root: &Path, id: WorkspaceLauncherId) -> WorkspaceLauncherPayload {
    let platform = current_workspace_launcher_platform();
    match workspace_launcher_command(root, id, platform) {
        Ok(_) => WorkspaceLauncherPayload {
            id: workspace_launcher_id(id).to_string(),
            label: workspace_launcher_label(id, platform).to_string(),
            available: true,
            reason: None,
            recommended_reason: workspace_launcher_marker_reason(root, id),
        },
        Err(reason) => WorkspaceLauncherPayload {
            id: workspace_launcher_id(id).to_string(),
            label: workspace_launcher_label(id, platform).to_string(),
            available: false,
            reason: Some(reason),
            recommended_reason: None,
        },
    }
}

fn workspace_launcher_marker_reason(root: &Path, id: WorkspaceLauncherId) -> Option<String> {
    let marker = match id {
        WorkspaceLauncherId::Zed => ".zed",
        WorkspaceLauncherId::Vscode => ".vscode",
        _ => return None,
    };
    root.join(marker)
        .is_dir()
        .then(|| format!("Workspace has {marker} settings"))
}

fn workspace_launcher_command(
    root: &Path,
    id: WorkspaceLauncherId,
    platform: WorkspaceLauncherPlatform,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String> {
    workspace_launcher_command_for_platform(root, id, platform, find_program)
}

fn workspace_launcher_command_for_platform<F>(
    root: &Path,
    id: WorkspaceLauncherId,
    platform: WorkspaceLauncherPlatform,
    resolver: F,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    let path = root.to_string_lossy().to_string();
    match platform {
        WorkspaceLauncherPlatform::Windows => {
            workspace_launcher_windows_command(&path, id, resolver)
        }
        WorkspaceLauncherPlatform::Macos => workspace_launcher_macos_command(&path, id, resolver),
        WorkspaceLauncherPlatform::Linux => workspace_launcher_linux_command(&path, id, resolver),
    }
}

fn workspace_launcher_windows_command<F>(
    path: &str,
    id: WorkspaceLauncherId,
    resolver: F,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    match id {
        WorkspaceLauncherId::FileManager => Ok(WorkspaceLauncherCommandSpec {
            program: "explorer".to_string(),
            args: vec![path.to_string()],
        }),
        WorkspaceLauncherId::Terminal => resolver("wt.exe")
            .or_else(|| resolver("wt"))
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec!["-d".to_string(), path.to_string()],
            })
            .ok_or_else(|| "Windows Terminal was not found".to_string()),
        WorkspaceLauncherId::Vscode => resolver("code.cmd")
            .or_else(|| resolver("code.exe"))
            .or_else(|| resolver("code"))
            .or_else(common_windows_vscode_path)
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "VS Code command was not found".to_string()),
        WorkspaceLauncherId::Zed => resolver("zed.exe")
            .or_else(|| resolver("zed.cmd"))
            .or_else(|| resolver("zed"))
            .or_else(common_windows_zed_path)
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "Zed command was not found".to_string()),
        WorkspaceLauncherId::GitBash => resolver("git-bash.exe")
            .or_else(common_windows_git_bash_path)
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![format!("--cd={path}")],
            })
            .ok_or_else(|| "Git Bash was not found".to_string()),
        WorkspaceLauncherId::Wsl => resolver("wsl.exe")
            .or_else(|| resolver("wsl"))
            .or_else(common_windows_wsl_path)
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec!["--cd".to_string(), path.to_string()],
            })
            .ok_or_else(|| "WSL was not found".to_string()),
        WorkspaceLauncherId::AndroidStudio => resolver("studio64.exe")
            .or_else(|| resolver("studio.exe"))
            .or_else(common_windows_android_studio_path)
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "Android Studio was not found".to_string()),
    }
}

fn workspace_launcher_macos_command<F>(
    path: &str,
    id: WorkspaceLauncherId,
    resolver: F,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    match id {
        WorkspaceLauncherId::FileManager => Ok(open_app_spec(path, None)),
        WorkspaceLauncherId::Terminal => Ok(open_app_spec(path, Some("Terminal"))),
        WorkspaceLauncherId::Vscode => resolver("code")
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .or_else(|| {
                macos_app_exists("Visual Studio Code")
                    .then(|| open_app_spec(path, Some("Visual Studio Code")))
            })
            .ok_or_else(|| "VS Code was not found".to_string()),
        WorkspaceLauncherId::Zed => resolver("zed")
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .or_else(|| macos_app_exists("Zed").then(|| open_app_spec(path, Some("Zed"))))
            .ok_or_else(|| "Zed was not found".to_string()),
        WorkspaceLauncherId::AndroidStudio => resolver("studio")
            .or_else(|| resolver("android-studio"))
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .or_else(|| {
                macos_app_exists("Android Studio")
                    .then(|| open_app_spec(path, Some("Android Studio")))
            })
            .ok_or_else(|| "Android Studio was not found".to_string()),
        WorkspaceLauncherId::GitBash => Err("Git Bash is only supported on Windows".to_string()),
        WorkspaceLauncherId::Wsl => Err("WSL is only supported on Windows".to_string()),
    }
}

fn workspace_launcher_linux_command<F>(
    path: &str,
    id: WorkspaceLauncherId,
    resolver: F,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    match id {
        WorkspaceLauncherId::FileManager => resolver("xdg-open")
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "xdg-open was not found".to_string()),
        WorkspaceLauncherId::Terminal => linux_terminal_spec(path, resolver),
        WorkspaceLauncherId::Vscode => resolver("code")
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "VS Code command was not found".to_string()),
        WorkspaceLauncherId::Zed => resolver("zed")
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "Zed command was not found".to_string()),
        WorkspaceLauncherId::AndroidStudio => resolver("android-studio")
            .or_else(|| resolver("studio.sh"))
            .map(|program| WorkspaceLauncherCommandSpec {
                program: program.to_string_lossy().to_string(),
                args: vec![path.to_string()],
            })
            .ok_or_else(|| "Android Studio was not found".to_string()),
        WorkspaceLauncherId::GitBash => Err("Git Bash is only supported on Windows".to_string()),
        WorkspaceLauncherId::Wsl => Err("WSL is only supported on Windows".to_string()),
    }
}

fn open_app_spec(path: &str, app_name: Option<&str>) -> WorkspaceLauncherCommandSpec {
    let mut args = Vec::new();
    if let Some(app_name) = app_name {
        args.push("-a".to_string());
        args.push(app_name.to_string());
    }
    args.push(path.to_string());
    WorkspaceLauncherCommandSpec {
        program: "open".to_string(),
        args,
    }
}

fn linux_terminal_spec<F>(
    path: &str,
    resolver: F,
) -> std::result::Result<WorkspaceLauncherCommandSpec, String>
where
    F: Fn(&str) -> Option<PathBuf>,
{
    if let Some(program) = resolver("x-terminal-emulator") {
        return Ok(WorkspaceLauncherCommandSpec {
            program: program.to_string_lossy().to_string(),
            args: vec!["--working-directory".to_string(), path.to_string()],
        });
    }
    if let Some(program) = resolver("gnome-terminal") {
        return Ok(WorkspaceLauncherCommandSpec {
            program: program.to_string_lossy().to_string(),
            args: vec!["--working-directory".to_string(), path.to_string()],
        });
    }
    if let Some(program) = resolver("konsole") {
        return Ok(WorkspaceLauncherCommandSpec {
            program: program.to_string_lossy().to_string(),
            args: vec!["--workdir".to_string(), path.to_string()],
        });
    }
    if let Some(program) = resolver("xfce4-terminal") {
        return Ok(WorkspaceLauncherCommandSpec {
            program: program.to_string_lossy().to_string(),
            args: vec!["--working-directory".to_string(), path.to_string()],
        });
    }
    Err("terminal command was not found".to_string())
}

fn find_program(name: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(name);
    if direct.components().count() > 1 && direct.is_file() {
        return Some(direct);
    }
    let paths = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|item| !item.trim().is_empty())
                    .map(|item| item.trim().to_string())
                    .collect()
            })
            .unwrap_or_else(|| {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            })
    } else {
        vec!["".to_string()]
    };
    for dir in std::env::split_paths(&paths) {
        for ext in &extensions {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn existing_path(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn common_windows_vscode_path() -> Option<PathBuf> {
    env_path("LOCALAPPDATA")
        .map(|root| {
            root.join("Programs")
                .join("Microsoft VS Code")
                .join("Code.exe")
        })
        .and_then(existing_path)
        .or_else(|| {
            env_path("ProgramFiles")
                .map(|root| root.join("Microsoft VS Code").join("Code.exe"))
                .and_then(existing_path)
        })
}

fn common_windows_zed_path() -> Option<PathBuf> {
    env_path("LOCALAPPDATA")
        .map(|root| root.join("Programs").join("Zed").join("Zed.exe"))
        .and_then(existing_path)
}

fn common_windows_git_bash_path() -> Option<PathBuf> {
    env_path("ProgramFiles")
        .map(|root| root.join("Git").join("git-bash.exe"))
        .and_then(existing_path)
        .or_else(|| {
            env_path("ProgramFiles(x86)")
                .map(|root| root.join("Git").join("git-bash.exe"))
                .and_then(existing_path)
        })
}

fn common_windows_wsl_path() -> Option<PathBuf> {
    env_path("SystemRoot")
        .map(|root| root.join("System32").join("wsl.exe"))
        .and_then(existing_path)
}

fn common_windows_android_studio_path() -> Option<PathBuf> {
    env_path("ProgramFiles")
        .map(|root| {
            root.join("Android")
                .join("Android Studio")
                .join("bin")
                .join("studio64.exe")
        })
        .and_then(existing_path)
        .or_else(|| {
            env_path("ProgramFiles")
                .map(|root| {
                    root.join("Android")
                        .join("Android Studio")
                        .join("bin")
                        .join("studio.exe")
                })
                .and_then(existing_path)
        })
}

fn macos_app_exists(name: &str) -> bool {
    let app_name = format!("{name}.app");
    [
        PathBuf::from("/Applications").join(&app_name),
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Applications")
            .join(&app_name),
    ]
    .iter()
    .any(|path| path.is_dir())
}

const UPDATE_DIR_NAME: &str = "milim-updates";
const UPDATE_RECOVERY_ERROR_NAME: &str = "install-error.txt";
const MAX_UPDATE_PACKAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_UPDATE_CHECKSUM_BYTES: usize = 1024 * 1024;
const UPDATE_PROGRESS_UNKNOWN_STEP_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateDownloadProgress {
    phase: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

fn send_update_progress(
    channel: &Channel<UpdateDownloadProgress>,
    phase: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) {
    let _ = channel.send(UpdateDownloadProgress {
        phase,
        downloaded_bytes,
        total_bytes,
    });
}

fn update_progress_percent(downloaded_bytes: u64, total_bytes: Option<u64>) -> Option<u8> {
    total_bytes.filter(|total| *total > 0).map(|total| {
        downloaded_bytes
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u8
    })
}

fn should_report_update_progress(
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    last_reported_bytes: u64,
    last_reported_percent: Option<u8>,
) -> bool {
    match update_progress_percent(downloaded_bytes, total_bytes) {
        Some(percent) => Some(percent) != last_reported_percent,
        None => {
            downloaded_bytes.saturating_sub(last_reported_bytes)
                >= UPDATE_PROGRESS_UNKNOWN_STEP_BYTES
        }
    }
}

fn update_dir(app: &tauri::AppHandle) -> std::result::Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("could not resolve app local data directory: {e}"))?;
    Ok(data_dir.join(UPDATE_DIR_NAME))
}

fn update_recovery_error_file(app: &tauri::AppHandle) -> std::result::Result<PathBuf, String> {
    Ok(update_dir(app)?.join(UPDATE_RECOVERY_ERROR_NAME))
}

#[tauri::command]
fn take_update_recovery_error(
    app: tauri::AppHandle,
) -> std::result::Result<Option<String>, String> {
    let marker = update_recovery_error_file(&app)?;
    match fs::read_to_string(&marker) {
        Ok(contents) => {
            let _ = fs::remove_file(marker);
            let message = contents.trim();
            Ok((!message.is_empty()).then(|| message.to_string()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn validate_update_archive_name(file_name: &str) -> std::result::Result<(), String> {
    if file_name.trim().is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.starts_with('.')
    {
        return Err("Invalid update file name.".to_string());
    }
    let lower = file_name.to_ascii_lowercase();
    if !(lower.ends_with(".exe") || lower.ends_with(".app.zip")) {
        return Err("Unsupported update file type.".to_string());
    }
    Ok(())
}

#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
fn validate_install_source_name(file_name: &str) -> std::result::Result<(), String> {
    if file_name.trim().is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.starts_with('.')
    {
        return Err("Invalid update source name.".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".exe") {
            return Ok(());
        }
    }

    #[cfg(target_os = "macos")]
    {
        let lower = file_name.to_ascii_lowercase();
        if lower.ends_with(".app") {
            return Ok(());
        }
    }

    Err("Unsupported update source type.".to_string())
}

#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
fn canonical_update_source(
    app: &tauri::AppHandle,
    update_path: &str,
) -> std::result::Result<PathBuf, String> {
    let update_root = update_dir(app)?;
    let canonical_root = fs::canonicalize(&update_root).map_err(|e| e.to_string())?;
    let canonical_source = fs::canonicalize(Path::new(update_path)).map_err(|e| e.to_string())?;
    if !canonical_source.starts_with(&canonical_root) {
        return Err("Update source must be inside the app update directory.".to_string());
    }
    let file_name = canonical_source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Invalid update source path")?;
    validate_install_source_name(file_name)?;
    Ok(canonical_source)
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn canonical_update_archive(
    app: &tauri::AppHandle,
    update_path: &str,
) -> std::result::Result<PathBuf, String> {
    let update_root = update_dir(app)?;
    let canonical_root = fs::canonicalize(&update_root).map_err(|e| e.to_string())?;
    let canonical_archive = fs::canonicalize(Path::new(update_path)).map_err(|e| e.to_string())?;
    if !canonical_archive.starts_with(&canonical_root) {
        return Err("Update archive must be inside the app update directory.".to_string());
    }
    let file_name = canonical_archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("Invalid update archive path")?;
    validate_update_archive_name(file_name)?;
    Ok(canonical_archive)
}

#[tauri::command]
fn get_update_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    }
}

fn staged_update_file_path(
    update_root: &Path,
    file_name: &str,
) -> std::result::Result<PathBuf, String> {
    validate_update_archive_name(file_name)?;
    Ok(update_root.join(file_name))
}

fn validate_update_download_url(url: &str, label: &str) -> std::result::Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("{label} is invalid: {e}"))?;
    if parsed.scheme() != "https" {
        return Err(format!("{label} must use https."));
    }
    match parsed.host_str() {
        Some("github.com" | "api.github.com") => Ok(parsed.to_string()),
        _ => Err(format!("{label} must be a GitHub release URL.")),
    }
}

fn first_sha256_hex(line: &str) -> Option<String> {
    line.split(|ch: char| !ch.is_ascii_hexdigit())
        .find(|part| part.len() == 64 && part.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|part| part.to_ascii_lowercase())
}

fn parse_expected_sha256(checksum_text: &str, asset_name: &str) -> Option<String> {
    let lines: Vec<&str> = checksum_text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let asset_base_name = asset_name.rsplit(['/', '\\']).next().unwrap_or(asset_name);
    let matching_line = lines
        .iter()
        .copied()
        .find(|line| line.contains(asset_name) && first_sha256_hex(line).is_some())
        .or_else(|| {
            lines
                .iter()
                .copied()
                .find(|line| line.contains(asset_base_name) && first_sha256_hex(line).is_some())
        })
        .or_else(|| {
            (lines.len() == 1)
                .then(|| lines[0])
                .filter(|line| first_sha256_hex(line).is_some())
        });
    matching_line.and_then(first_sha256_hex)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn verify_update_checksum(
    bytes: &[u8],
    checksum_text: &str,
    asset_name: &str,
) -> std::result::Result<(), String> {
    let expected = parse_expected_sha256(checksum_text, asset_name).ok_or_else(|| {
        format!("Checksum file does not contain a SHA-256 hash for {asset_name}.")
    })?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(format!("Update checksum mismatch for {asset_name}."));
    }
    Ok(())
}

fn append_update_chunk(
    bytes: &mut Vec<u8>,
    chunk: &[u8],
    label: &str,
    max_bytes: usize,
) -> std::result::Result<(), String> {
    if bytes.len().saturating_add(chunk.len()) > max_bytes {
        return Err(format!("{label} is too large."));
    }
    bytes.extend_from_slice(chunk);
    Ok(())
}

async fn fetch_update_bytes(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
    label: &str,
    max_bytes: usize,
    on_progress: Option<&Channel<UpdateDownloadProgress>>,
) -> std::result::Result<Vec<u8>, String> {
    let response = client
        .get(url)
        .header(ACCEPT, accept)
        .send()
        .await
        .map_err(|e| format!("{label} download failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{label} download failed ({status})."));
    }
    let total_bytes = response.content_length();
    if total_bytes.is_some_and(|length| length > max_bytes as u64) {
        return Err(format!("{label} is too large."));
    }
    if let Some(channel) = on_progress {
        send_update_progress(channel, "downloading", 0, total_bytes);
    }
    let mut response = response;
    let mut bytes = Vec::new();
    let mut last_reported_bytes = 0;
    let mut last_reported_percent = update_progress_percent(0, total_bytes);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("{label} download failed: {e}"))?
    {
        append_update_chunk(&mut bytes, &chunk, label, max_bytes)?;
        let downloaded_bytes = bytes.len() as u64;
        if let Some(channel) = on_progress {
            if should_report_update_progress(
                downloaded_bytes,
                total_bytes,
                last_reported_bytes,
                last_reported_percent,
            ) {
                send_update_progress(channel, "downloading", downloaded_bytes, total_bytes);
                last_reported_bytes = downloaded_bytes;
                last_reported_percent = update_progress_percent(downloaded_bytes, total_bytes);
            }
        }
    }
    if bytes.is_empty() {
        return Err(format!("{label} returned no bytes."));
    }
    if let Some(channel) = on_progress {
        let downloaded_bytes = bytes.len() as u64;
        if downloaded_bytes != last_reported_bytes {
            send_update_progress(channel, "downloading", downloaded_bytes, total_bytes);
        }
    }
    Ok(bytes)
}

#[tauri::command]
async fn download_update_file(
    app: tauri::AppHandle,
    download_url: String,
    checksum_url: String,
    asset_name: String,
    file_name: String,
    on_progress: Channel<UpdateDownloadProgress>,
) -> std::result::Result<String, String> {
    validate_update_archive_name(&asset_name)?;
    let download_url = validate_update_download_url(&download_url, "Update download URL")?;
    let checksum_url = validate_update_download_url(&checksum_url, "Checksum download URL")?;
    let update_root = update_dir(&app)?;
    fs::create_dir_all(&update_root).map_err(|e| e.to_string())?;
    let update_file = staged_update_file_path(&update_root, &file_name)?;

    let client = reqwest::Client::builder()
        .user_agent("milim-updater")
        .build()
        .map_err(|e| e.to_string())?;
    let package = fetch_update_bytes(
        &client,
        &download_url,
        "application/octet-stream",
        "Update package",
        MAX_UPDATE_PACKAGE_BYTES,
        Some(&on_progress),
    )
    .await?;
    send_update_progress(
        &on_progress,
        "verifying",
        package.len() as u64,
        Some(package.len() as u64),
    );
    let checksum = fetch_update_bytes(
        &client,
        &checksum_url,
        "text/plain, application/octet-stream",
        "Update checksum",
        MAX_UPDATE_CHECKSUM_BYTES,
        None,
    )
    .await?;
    let checksum_text =
        String::from_utf8(checksum).map_err(|_| "Checksum file is not valid UTF-8.".to_string())?;
    verify_update_checksum(&package, &checksum_text, &asset_name)?;
    fs::write(&update_file, package).map_err(|e| e.to_string())?;
    Ok(update_file.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn escape_powershell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn windows_update_replacement_path(current_exe: &Path) -> PathBuf {
    let file_name = current_exe
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("milim.exe");
    current_exe.with_file_name(format!("{file_name}.update"))
}

#[cfg(target_os = "windows")]
fn windows_update_backup_path(current_exe: &Path) -> PathBuf {
    let file_name = current_exe
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("milim.exe");
    current_exe.with_file_name(format!("{file_name}.previous"))
}

#[cfg(target_os = "windows")]
struct WindowsUpdateScriptPaths<'a> {
    source: &'a Path,
    replacement: &'a Path,
    target: &'a Path,
    backup: &'a Path,
    log: &'a Path,
    error_marker: &'a Path,
    script: &'a Path,
}

#[cfg(target_os = "windows")]
fn build_windows_update_script(pid: u32, paths: WindowsUpdateScriptPaths<'_>) -> String {
    let template = r#"
param([switch]$Elevated)
$ErrorActionPreference = 'Stop'
$procId = __PID__
$source = '__SOURCE__'
$replacement = '__REPLACEMENT__'
$target = '__TARGET__'
$backup = '__BACKUP__'
$log = '__LOG__'
$errorMarker = '__ERROR_MARKER__'
$script = '__SCRIPT__'

function Write-UpdateLog([string]$message) {
  try {
    $timestamp = (Get-Date).ToString('s')
    Add-Content -LiteralPath $log -Value "$timestamp $message"
  } catch {}
}

Write-UpdateLog "Waiting for process $procId to exit."
while (Get-Process -Id $procId -ErrorAction SilentlyContinue) {
  Start-Sleep -Milliseconds 200
}

for ($attempt = 1; $attempt -le 120; $attempt++) {
  try {
    if (Test-Path -LiteralPath $backup) {
      Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
    if (-not (Test-Path -LiteralPath $source)) {
      throw "Downloaded update is missing: $source"
    }

    Copy-Item -LiteralPath $source -Destination $replacement -Force
    if ((Get-Item -LiteralPath $source).Length -ne (Get-Item -LiteralPath $replacement).Length) {
      throw "Staged update size did not match the downloaded update."
    }

    Move-Item -LiteralPath $target -Destination $backup -Force
    Move-Item -LiteralPath $replacement -Destination $target -Force
    Write-UpdateLog "Installed update on attempt $attempt."
    Start-Process -FilePath $target
    Start-Sleep -Seconds 2

    if (Test-Path -LiteralPath $backup) {
      Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
    exit 0
  } catch {
    Write-UpdateLog "Attempt $attempt failed: $($_.Exception.Message)"
    if ((-not (Test-Path -LiteralPath $target)) -and (Test-Path -LiteralPath $backup)) {
      try {
        Move-Item -LiteralPath $backup -Destination $target -Force
        Write-UpdateLog "Restored previous executable after failed attempt $attempt."
      } catch {
        Write-UpdateLog "Failed to restore previous executable: $($_.Exception.Message)"
      }
    }
    Start-Sleep -Milliseconds 500
  }
}

if (-not $Elevated) {
  try {
    Write-UpdateLog "Retrying update with elevation."
    Start-Process -FilePath "powershell.exe" -Verb RunAs -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $script, "-Elevated")
    exit 0
  } catch {
    Write-UpdateLog "Elevation failed: $($_.Exception.Message)"
  }
}

Write-UpdateLog "Update failed after retries."
try {
  Set-Content -LiteralPath $errorMarker -Value "The last update failed after milim closed. The previous version was restored. See $log for details."
} catch {}
try {
  if ((-not (Test-Path -LiteralPath $target)) -and (Test-Path -LiteralPath $backup)) {
    Move-Item -LiteralPath $backup -Destination $target -Force
  }
  if (Test-Path -LiteralPath $target) {
    Start-Process -FilePath $target
  }
} catch {
  Write-UpdateLog "Failed to relaunch after update failure: $($_.Exception.Message)"
}
exit 1
"#;

    template
        .replace("__PID__", &pid.to_string())
        .replace(
            "__SOURCE__",
            &escape_powershell_literal(&paths.source.to_string_lossy()),
        )
        .replace(
            "__REPLACEMENT__",
            &escape_powershell_literal(&paths.replacement.to_string_lossy()),
        )
        .replace(
            "__TARGET__",
            &escape_powershell_literal(&paths.target.to_string_lossy()),
        )
        .replace(
            "__BACKUP__",
            &escape_powershell_literal(&paths.backup.to_string_lossy()),
        )
        .replace(
            "__LOG__",
            &escape_powershell_literal(&paths.log.to_string_lossy()),
        )
        .replace(
            "__ERROR_MARKER__",
            &escape_powershell_literal(&paths.error_marker.to_string_lossy()),
        )
        .replace(
            "__SCRIPT__",
            &escape_powershell_literal(&paths.script.to_string_lossy()),
        )
}

#[cfg(target_os = "macos")]
fn escape_bash_literal(value: &str) -> String {
    value.replace('\'', "'\\''")
}

#[tauri::command]
fn apply_update(app: tauri::AppHandle, update_path: String) -> std::result::Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Auto-update is disabled in dev builds.".to_string());
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (app, update_path);
        Err("Auto-update is not supported on this platform.".to_string())
    }

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let update_file = canonical_update_source(&app, &update_path)?;
        let current_exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let pid = std::process::id();

        #[cfg(target_os = "windows")]
        {
            let update_root = update_dir(&app)?;
            fs::create_dir_all(&update_root).map_err(|e| e.to_string())?;
            let replacement = windows_update_replacement_path(&current_exe);
            let backup = windows_update_backup_path(&current_exe);
            let log = update_root.join("install.log");
            let error_marker = update_root.join(UPDATE_RECOVERY_ERROR_NAME);
            let script_path = update_root.join("apply-update.ps1");
            let script = build_windows_update_script(
                pid,
                WindowsUpdateScriptPaths {
                    source: &update_file,
                    replacement: &replacement,
                    target: &current_exe,
                    backup: &backup,
                    log: &log,
                    error_marker: &error_marker,
                    script: &script_path,
                },
            );
            fs::write(&script_path, script).map_err(|e| e.to_string())?;
            record_subprocess_launch();
            Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-WindowStyle",
                    "Hidden",
                    "-File",
                    &script_path.to_string_lossy(),
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| e.to_string())?;
        }

        #[cfg(target_os = "macos")]
        {
            let update_root = update_dir(&app)?;
            fs::create_dir_all(&update_root).map_err(|e| e.to_string())?;
            let error_marker = update_root.join(UPDATE_RECOVERY_ERROR_NAME);
            let app_bundle = current_exe
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .ok_or("Could not determine app bundle path")?;
            let backup = app_bundle.with_extension("app.previous");
            let script = format!(
                r#"set -e
pid={}
source='{}'
target='{}'
backup='{}'
error_marker='{}'
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
trap 'echo "The last update failed after milim closed. The previous app was restored." > "$error_marker"; if [ ! -e "$target" ] && [ -e "$backup" ]; then mv "$backup" "$target"; open "$target"; fi' ERR
rm -rf "$backup"
mv "$target" "$backup"
mv "$source" "$target"
open "$target"
rm -rf "$backup"
"#,
                pid,
                escape_bash_literal(&update_file.to_string_lossy()),
                escape_bash_literal(&app_bundle.to_string_lossy()),
                escape_bash_literal(&backup.to_string_lossy()),
                escape_bash_literal(&error_marker.to_string_lossy()),
            );
            record_subprocess_launch();
            Command::new("bash")
                .args(["-c", &script])
                .spawn()
                .map_err(|e| e.to_string())?;
        }

        exit_after_server_shutdown(app);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn extract_app_zip(app: tauri::AppHandle, zip_path: String) -> std::result::Result<String, String> {
    let zip_file = canonical_update_archive(&app, &zip_path)?;
    let parent = zip_file.parent().ok_or("Invalid zip path")?;
    let app_path = parent.join("milim.app");
    if app_path.exists() {
        fs::remove_dir_all(&app_path).map_err(|e| e.to_string())?;
    }
    record_subprocess_launch();
    let status = Command::new("ditto")
        .arg("-xk")
        .arg(zip_file.as_os_str())
        .arg(parent.as_os_str())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Failed to extract update".to_string());
    }
    if !app_path.exists() {
        return Err("Extracted app not found".to_string());
    }
    let _ = fs::remove_file(zip_file);
    Ok(app_path.to_string_lossy().to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn extract_app_zip(_zip_path: String) -> std::result::Result<String, String> {
    Err("This command is only available on macOS".to_string())
}

fn desktop_config() -> ServerConfiguration {
    ServerConfiguration {
        allowed_origins: vec![
            "http://localhost:5180".to_string(),
            "http://127.0.0.1:5180".to_string(),
            "http://tauri.localhost".to_string(),
            "https://tauri.localhost".to_string(),
            "tauri://localhost".to_string(),
        ],
        ..ServerConfiguration::default()
    }
}

const LEGACY_DESKTOP_CONTROL_NAME: &str = "milim desktop";

fn normalize_desktop_control_name(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    let without_local_suffix = trimmed
        .get(..trimmed.len().saturating_sub(".local".len()))
        .filter(|_| trimmed.to_ascii_lowercase().ends_with(".local"))
        .unwrap_or(trimmed)
        .trim();

    if without_local_suffix.is_empty() {
        LEGACY_DESKTOP_CONTROL_NAME.to_string()
    } else {
        without_local_suffix.to_string()
    }
}

fn desktop_control_display_name() -> String {
    normalize_desktop_control_name(&gethostname::gethostname().to_string_lossy())
}

fn has_legacy_desktop_control_name(display_name: &str) -> bool {
    matches!(
        display_name.trim().to_ascii_lowercase().as_str(),
        LEGACY_DESKTOP_CONTROL_NAME | "milim-desktop"
    )
}

fn migrate_desktop_control_name(
    user_data: &milim_storage::UserDataStore,
    control: &milim_server::control::RunManager,
    desktop_name: &str,
) {
    let current = control.host();
    if !has_legacy_desktop_control_name(&current.display_name)
        || current.display_name == desktop_name
    {
        return;
    }

    if let Err(error) = user_data.update_control_host_name(desktop_name) {
        tracing::warn!("failed to update the mobile desktop name: {error}");
        return;
    }
    if let Err(error) = control.refresh_host() {
        tracing::warn!("failed to refresh the mobile desktop name: {error}");
    }
}

fn open_user_data_store_at(path: &Path) -> Result<Arc<milim_storage::UserDataStore>> {
    let db = Database::open_with_options(
        path,
        DatabaseOptions {
            journal_mode: JournalMode::Wal,
        },
    )?;
    if db.schema_version_scoped("user_data")? == 4 {
        let snapshot = path.with_extension("pre-v5.sqlite3");
        db.vacuum_snapshot_into(&snapshot)?;
    }
    milim_storage::UserDataStore::new(db).map(Arc::new)
}

fn open_user_data_store() -> Result<Arc<milim_storage::UserDataStore>> {
    let paths = Paths::resolve();
    open_user_data_store_at(&paths.user_db_file())
}

fn desktop_google_oauth_value(name: &str, compiled: Option<&'static str>) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| compiled.map(str::to_string))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Build the embedded server state (backend + tools). Returns the MCP hub too
/// so the caller can connect persisted servers on the async runtime (connecting
/// spawns child processes).
fn build_state(
    api_key: String,
    preview_tools_state: preview_tools::SharedPreviewToolState,
    encryption: Option<EncryptedStore>,
    user_data: Arc<milim_storage::UserDataStore>,
) -> (
    AppState,
    SocketAddr,
    Arc<milim_mcp_client::McpHub>,
    Option<Arc<milim_server::providers::ProviderRegistry>>,
) {
    let config = desktop_config();
    let addr: SocketAddr = format!("127.0.0.1:{}", config.port)
        .parse()
        .expect("valid loopback address");

    let paths = Paths::resolve();

    let mut tools = ToolRegistry::with_builtins();
    tools.register(Arc::new(RunCommandTool::default()));
    for tool in preview_tools::preview_tools(preview_tools_state) {
        tools.register(tool);
    }

    // Host filesystem + shell tools, rooted to the GUI's working folder. The
    // same cell is shared with AppState so `POST /workspace` (the Folder chip)
    // updates the root the tools operate within.
    let workspace: host_tools::Workspace = Arc::new(RwLock::new(None));
    for tool in host_tools::host_tools(workspace.clone()) {
        tools.register(tool);
    }

    let google_workspace = encryption.as_ref().and_then(|encryption| {
        match milim_server::google_workspace::GoogleWorkspaceConnection::open_with_encryption(
            paths.root(),
            milim_server::google_workspace::GoogleWorkspaceConfig::desktop(
                desktop_google_oauth_value(
                    "MILIM_GOOGLE_OAUTH_CLIENT_ID",
                    option_env!("MILIM_GOOGLE_OAUTH_CLIENT_ID"),
                ),
                desktop_google_oauth_value(
                    "MILIM_GOOGLE_OAUTH_CLIENT_SECRET",
                    option_env!("MILIM_GOOGLE_OAUTH_CLIENT_SECRET"),
                ),
            ),
            workspace.clone(),
            encryption.clone(),
        ) {
            Ok(connection) => {
                let connection = Arc::new(connection);
                if connection.available() {
                    for tool in milim_server::google_workspace::tools(connection.clone()) {
                        tools.register(tool);
                    }
                }
                Some(connection)
            }
            Err(error) => {
                tracing::warn!("Google Workspace connector unavailable: {error}");
                None
            }
        }
    });

    // Computer-use tools (screen capture + mouse/keyboard). Registered when
    // built with the feature; dormant until the runtime gate is enabled.
    #[cfg(feature = "computer-use")]
    let computer_gate = {
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        for tool in computer_tools::computer_tools(gate.clone(), paths.root().join("captures")) {
            tools.register(tool);
        }
        gate
    };

    // The default backend (configured remote / explicit unavailable) is the
    // fallback; configured providers route by model on top of it.
    let local = pick_backend();
    let registry = encryption.as_ref().and_then(|encryption| {
        match milim_server::providers::ProviderRegistry::open_with_encryption(
            paths.root(),
            local.clone(),
            encryption.clone(),
        ) {
            Ok(registry) => Some(Arc::new(registry)),
            Err(e) => {
                tracing::warn!("provider registry unavailable: {e}");
                None
            }
        }
    });

    // External MCP servers (their tools merge into the agent registry per-run).
    let mcp = Arc::new(match encryption.as_ref() {
        Some(encryption) => {
            milim_mcp_client::McpHub::open_with_encryption(paths.root(), encryption.clone())
        }
        None => milim_mcp_client::McpHub::open_without_secrets(
            paths.root(),
            "desktop secret storage is unavailable",
        ),
    });

    // Outbound privacy gate: shared between the provider router (enforcement)
    // and the `/privacy/mode` endpoint (the desktop's active thread setting).
    let privacy = Arc::new(milim_server::privacy::PrivacyGate::from_env());
    let mobile_companion = Arc::new(
        encryption
            .as_ref()
            .and_then(|encryption| {
                milim_server::companion::MobileCompanionBridge::persistent_encrypted(
                    paths.config_dir().join("mobile-companion.enc"),
                    encryption.clone(),
                )
                .map_err(|error| {
                    tracing::warn!("mobile companion persistence unavailable: {error}")
                })
                .ok()
            })
            .unwrap_or_default(),
    );
    let desktop_name = desktop_control_display_name();
    let control = milim_server::control::RunManager::new(user_data.clone(), &desktop_name)
        .expect("initialize canonical control runtime");
    migrate_desktop_control_name(&user_data, &control, &desktop_name);
    let service: SharedService = registry
        .as_ref()
        .map(|registry| Arc::new(registry.router(privacy.clone())) as SharedService)
        .unwrap_or_else(|| local.clone());

    let mut state = AppState::new(service.clone(), config)
        .with_tools(tools)
        .with_workspace(workspace)
        .with_mcp(mcp.clone())
        .with_privacy(privacy)
        .with_mobile_companion(mobile_companion)
        .with_control(control)
        .with_api_keys([api_key])
        .with_loopback_trust(false);
    match milim_server::media_library::MediaLibrary::open(paths.root().join("media")) {
        Ok(library) => state = state.with_media_library(library),
        Err(e) => tracing::warn!("media library unavailable: {e}"),
    }
    if let Some(registry) = &registry {
        state = state.with_providers(registry.clone());
    }
    if let Some(google_workspace) = google_workspace {
        state = state.with_google_workspace(google_workspace);
    }
    #[cfg(feature = "computer-use")]
    {
        state = state.with_computer_use(computer_gate);
    }

    // Persisted named-agent store (best-effort; chat works without it).
    let agents_db = paths.root().join("agents.db");
    match milim_storage::Database::open(&agents_db).and_then(milim_agents::AgentStore::new) {
        Ok(agent_store) => state = state.with_agents(agent_store),
        Err(e) => tracing::warn!("agent store unavailable: {e}"),
    }

    // Persisted child-thread supervisor used by parent agent runs.
    let threads_db = paths.root().join("threads.db");
    match milim_storage::Database::open(&threads_db).and_then(milim_agents::ThreadStore::new) {
        Ok(thread_store) => {
            state = state.with_threads(milim_server::threads::ThreadSupervisor::new(thread_store))
        }
        Err(e) => tracing::warn!("child threads unavailable: {e}"),
    }

    // Persisted SKILL.md instructions used by both simple chat and workbench.
    let skills_db = paths.root().join("skills.db");
    match milim_storage::Database::open(&skills_db).and_then(milim_skills::SkillStore::new) {
        Ok(skill_store) => {
            if let Err(e) = skill_store.import_global_skills() {
                tracing::warn!("global skills import failed: {e}");
            }
            state = state.with_skills(skill_store)
        }
        Err(e) => tracing::warn!("skills unavailable: {e}"),
    }

    // Embedding memory/RAG store (best-effort; embeds through the active router).
    let memory_db = paths.user_db_file();
    match milim_storage::Database::open_with_options(
        &memory_db,
        DatabaseOptions {
            journal_mode: JournalMode::Wal,
        },
    )
    .and_then(|db| milim_memory::MemoryStore::new(db, service.clone()))
    {
        Ok(memory) => state = state.with_memory(memory),
        Err(e) => tracing::warn!("memory store unavailable: {e}"),
    }

    // Persisted cron schedules, shared by the Schedules sheet and the agent
    // schedule tools used for chat-created automations.
    let schedules_db = paths.root().join("schedules.db");
    match milim_storage::Database::open(&schedules_db)
        .and_then(milim_automation::ScheduleStore::new)
    {
        Ok(schedule_store) => state = state.with_schedules(schedule_store),
        Err(e) => tracing::warn!("schedules unavailable: {e}"),
    }

    (state, addr, mcp, registry)
}

fn bind_desktop_server_listener(
    preferred_addr: SocketAddr,
) -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = match TcpListener::bind(preferred_addr) {
        Ok(listener) => listener,
        Err(e) if preferred_addr.port() != 0 => {
            tracing::warn!(
                "embedded milim server could not bind {preferred_addr}: {e}; falling back to a free loopback port"
            );
            TcpListener::bind(SocketAddr::new(preferred_addr.ip(), 0))?
        }
        Err(e) => return Err(e),
    };
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    Ok((listener, addr))
}

/// Choose the inference backend: a configured remote, else an explicit
/// unavailable backend.
fn pick_backend() -> SharedService {
    if let Some(base) = non_empty_env("MILIM_REMOTE_BASE_URL") {
        let key = non_empty_env("MILIM_REMOTE_API_KEY");
        return Arc::new(RemoteBackend::new("remote", base, key));
    }
    Arc::new(UnavailableBackend::new())
}

/// A tool that runs a shell command inside an isolated Docker sandbox
/// (`milim-sandbox`). Exposed to the agent loop so models can execute code/CLI
/// safely - this is how the GUI's Sandbox toggle is honored. Requires Docker.
struct RunCommandTool {
    backend: DockerBackend,
    image: String,
}

impl Default for RunCommandTool {
    fn default() -> Self {
        Self {
            backend: DockerBackend::new(),
            image: non_empty_env("MILIM_SANDBOX_IMAGE").unwrap_or_else(|| "alpine".to_string()),
        }
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }
    fn description(&self) -> &str {
        "Run a shell command inside an isolated Docker sandbox (no network). Use to execute code or CLI tools safely."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "command": { "type": "string", "description": "Shell command to run" } },
            "required": ["command"]
        })
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }
    fn environment_policy(&self) -> ProcessEnvironmentPolicy {
        ProcessEnvironmentPolicy::SandboxSanitized
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("missing string argument: command".into()))?;
        let out = self
            .backend
            .run(
                &self.image,
                &["sh".to_string(), "-c".to_string(), command.to_string()],
                &RunOpts::default(),
            )
            .await?;
        Ok(json!({
            "stdout": out.stdout,
            "stderr": out.stderr,
            "stdout_truncated": out.stdout_truncated,
            "stderr_truncated": out.stderr_truncated,
            "exit_code": out.exit_code
        }))
    }
}

fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[derive(Clone, Copy)]
enum DesktopFinishAction {
    Exit,
    Restart,
}

fn finish_after_server_shutdown<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    action: DesktopFinishAction,
) {
    let server_runtime = app.state::<DesktopServerRuntime>().0.clone();
    if server_runtime.finishing.swap(true, Ordering::AcqRel) {
        return;
    }
    let preview_runtime = app.state::<DesktopPreviewRuntime>().0.clone();
    tauri::async_runtime::spawn(async move {
        server_runtime.shutdown(preview_runtime).await;
        arm_desktop_finish_watchdog(action);
        match action {
            DesktopFinishAction::Exit => app.exit(0),
            DesktopFinishAction::Restart => app.restart(),
        }
    });
}

#[cfg(windows)]
fn arm_desktop_finish_watchdog(action: DesktopFinishAction) {
    let executable = std::env::current_exe().ok();
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        if matches!(action, DesktopFinishAction::Restart) {
            if let Some(executable) = executable {
                let _ = std::process::Command::new(executable)
                    .args(arguments)
                    .spawn();
            }
        }
        std::process::exit(0);
    });
}

#[cfg(not(windows))]
fn arm_desktop_finish_watchdog(_action: DesktopFinishAction) {}

fn exit_after_server_shutdown<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    finish_after_server_shutdown(app, DesktopFinishAction::Exit);
}

fn restart_after_server_shutdown<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    finish_after_server_shutdown(app, DesktopFinishAction::Restart);
}

fn request_user_state_flush_then_exit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let _ = app.emit(FLUSH_USER_STATE_AND_EXIT_EVENT, ());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(USER_STATE_FLUSH_TIMEOUT).await;
        exit_after_server_shutdown(app);
    });
}

fn complete_workspace_editor_leave_action<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: WorkspaceEditorLeaveAction,
) -> std::result::Result<(), String> {
    match action {
        WorkspaceEditorLeaveAction::Hide => {
            if let Err(error) = app.emit(FLUSH_USER_STATE_EVENT, ()) {
                tracing::warn!("failed to request user-state flush before hiding: {error}");
            }
            let window = app
                .get_webview_window(MAIN_WINDOW_LABEL)
                .ok_or_else(|| "The main Milim window is unavailable.".to_string())?;
            window
                .hide()
                .map_err(|error| format!("Failed to hide Milim: {error}"))
        }
        WorkspaceEditorLeaveAction::Quit => {
            request_user_state_flush_then_exit(app);
            Ok(())
        }
    }
}

fn request_workspace_editor_leave<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: WorkspaceEditorLeaveAction,
) -> std::result::Result<(), String> {
    if app
        .state::<WorkspaceEditorDirty>()
        .0
        .load(Ordering::Acquire)
    {
        let action = match action {
            WorkspaceEditorLeaveAction::Hide => "hide",
            WorkspaceEditorLeaveAction::Quit => "quit",
        };
        app.emit(WORKSPACE_EDITOR_LEAVE_EVENT, action)
            .map_err(|error| format!("Failed to request workspace editor confirmation: {error}"))
    } else {
        complete_workspace_editor_leave_action(app, action)
    }
}

fn handle_app_menu_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: MenuEvent) {
    let action = match event.id().as_ref() {
        APP_MENU_NEW_CHAT_ID => "new-chat",
        APP_MENU_SETTINGS_ID => "settings",
        APP_MENU_TOGGLE_SIDEBAR_ID => "toggle-sidebar",
        APP_MENU_DOCUMENTATION_ID => "documentation",
        APP_MENU_DIAGNOSTICS_ID => "diagnostics",
        APP_MENU_QUIT_ID => {
            if let Err(error) =
                request_workspace_editor_leave(app, WorkspaceEditorLeaveAction::Quit)
            {
                tracing::warn!("app-menu quit request failed: {error}");
            }
            return;
        }
        _ => return,
    };
    let _ = app.emit(APP_MENU_EVENT, action);
}

#[tauri::command]
fn set_sidebar_toggle_enabled(
    app: tauri::AppHandle,
    enabled: bool,
) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    if let Some(MenuItemKind::MenuItem(item)) = app
        .menu()
        .and_then(|menu| menu.get(APP_MENU_TOGGLE_SIDEBAR_ID))
    {
        item.set_enabled(enabled)
            .map_err(|error| error.to_string())?;
    }

    #[cfg(not(target_os = "macos"))]
    let _ = (app, enabled);

    Ok(())
}

#[cfg(target_os = "macos")]
fn setup_native_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, APP_MENU_SETTINGS_ID, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, APP_MENU_QUIT_ID, "Quit Milim", true, Some("Cmd+Q"))?;
    let app_menu = SubmenuBuilder::new(app, "Milim")
        .about(None)
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&quit)
        .build()?;
    let file_menu = SubmenuBuilder::new(app, "File")
        .text(APP_MENU_NEW_CHAT_ID, "New Chat")
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view_menu = SubmenuBuilder::new(app, "View")
        .text(APP_MENU_TOGGLE_SIDEBAR_ID, "Show/Hide Sidebar")
        .separator()
        .fullscreen()
        .build()?;
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize_with_text("Zoom")
        .build()?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .text(APP_MENU_DOCUMENTATION_ID, "Milim Documentation")
        .text(APP_MENU_DIAGNOSTICS_ID, "Open Diagnostics")
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ])
        .build()?;
    let _ = app.set_menu(menu)?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn setup_native_menu<R: tauri::Runtime>(_app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    Ok(())
}

fn setup_tray<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, TRAY_OPEN_ID, "Open milim", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "Quit milim", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &separator, &quit])?;
    let mut tray = TrayIconBuilder::with_id(MAIN_WINDOW_LABEL)
        .tooltip("milim")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_OPEN_ID => show_main_window(app),
            TRAY_QUIT_ID => {
                if let Err(error) =
                    request_workspace_editor_leave(app, WorkspaceEditorLeaveAction::Quit)
                {
                    tracing::warn!("tray quit request failed: {error}");
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn merged_windows_path(inherited: Option<&OsStr>, persisted: &OsStr) -> io::Result<OsString> {
    let mut paths = Vec::new();
    for source in [inherited.unwrap_or_default(), persisted] {
        for path in std::env::split_paths(source) {
            if !path.as_os_str().is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    std::env::join_paths(paths).map_err(io::Error::other)
}

#[cfg(target_os = "windows")]
fn refresh_windows_path() -> io::Result<()> {
    let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
    let powershell = PathBuf::from(windows)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    record_subprocess_launch();
    let mut command = Command::new(powershell);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "$path = @([Environment]::GetEnvironmentVariable('Path', 'Machine'), [Environment]::GetEnvironmentVariable('Path', 'User')) -join ';'; [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); [Console]::Write($path)",
    ]);
    milim_core::proc::hide_console(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "PowerShell exited with {}",
            output.status
        )));
    }
    let persisted = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let inherited = std::env::var_os("PATH");
    let merged = merged_windows_path(inherited.as_deref(), OsStr::new(&persisted))?;
    std::env::set_var("PATH", merged);
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod windows_path_tests {
    use super::*;

    #[test]
    fn refreshed_windows_path_keeps_session_entries_and_adds_current_entries() {
        let merged = merged_windows_path(
            Some(OsStr::new(r"C:\inherited;C:\shared")),
            OsStr::new(r"C:\shared;C:\current"),
        )
        .unwrap();

        assert_eq!(
            std::env::split_paths(&merged).collect::<Vec<_>>(),
            [
                PathBuf::from(r"C:\inherited"),
                PathBuf::from(r"C:\shared"),
                PathBuf::from(r"C:\current"),
            ]
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    native_perf_mark("process_entry");
    if let Err(error) = diagnostics::init() {
        eprintln!("desktop diagnostics unavailable: {error}");
    }
    #[cfg(target_os = "windows")]
    std::thread::spawn(|| {
        native_perf_mark("path_setup_started");
        if let Err(error) = refresh_windows_path() {
            tracing::warn!("could not refresh the Windows executable search path: {error}");
        }
        native_perf_mark("path_setup_completed");
    });
    native_perf_mark("path_setup_scheduled");
    let api_key = milim_server::gen_id("desktop");
    let preview_tools_state = Arc::new(preview_tools::PreviewToolState::default());
    let secret_storage = secret_storage::initialize(Paths::resolve().root());
    let storage_status = secret_storage.status.clone();
    let user_data = open_user_data_store().expect("initialize user data store");
    native_perf_mark("database_open_migrations_completed");
    let (mut state, preferred_addr, mcp, providers) = build_state(
        api_key.clone(),
        preview_tools_state.clone(),
        secret_storage.encryption,
        user_data.clone(),
    );
    native_perf_mark("control_reconciliation_completed");
    let (server_listener, addr) =
        bind_desktop_server_listener(preferred_addr).expect("bind embedded milim server");
    // Account-runtime tool endpoints must point at the actual listener. The
    // preferred port may already be occupied, in which case binding falls
    // back to an ephemeral loopback port.
    let mut bound_config = (*state.config).clone();
    bound_config.port = addr.port();
    state.config = Arc::new(bound_config);
    let (mobile_listener, mobile_addr) = bind_desktop_server_listener(
        "127.0.0.1:0"
            .parse()
            .expect("valid mobile control loopback address"),
    )
    .expect("bind embedded mobile control server");
    native_perf_mark("servers_listening");
    let api_base = format!("http://{addr}");
    let mobile_local_target = format!("http://{mobile_addr}");
    let preview_runtime = state.preview_runtime.clone();
    let mobile_state = state.clone();
    let mobile_startup_companion = state.mobile_companion.clone();
    let mobile_startup_target = mobile_local_target.clone();
    let mobile_lan_enabled = user_data
        .get_json("milim.mobile.lan")
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str::<bool>(&value).ok())
        .unwrap_or(false);
    let server_runtime =
        DesktopServerRuntimeState::new(state.control.clone(), Some(mobile_state.clone()));
    let model_favorites_control = state.control.clone();
    let artifact_migration_store = user_data.clone();

    let perf_runtime = std::env::var_os("MILIM_PERF").is_some();
    let builder = tauri::Builder::default();
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    let builder = if perf_runtime {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
    };

    builder
        .manage(DesktopApiToken(api_key))
        .manage(DesktopApiBaseUrl(api_base))
        .manage(DesktopProviders(providers))
        .manage(DesktopSecretStorage(storage_status))
        .manage(MobileControlLocalTarget(mobile_local_target))
        .manage(UserDataState(user_data))
        .manage(DesktopPreviewRuntime(preview_runtime))
        .manage(DesktopServerRuntime(server_runtime.clone()))
        .manage(WorkspaceEditorDirty::default())
        .manage(preview_tools_state.clone())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Persist + restore window size/position/maximized across restarts.
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(preview_webview::init())
        .on_page_load(|webview, payload| {
            native_perf_mark("window_created");
            preview_webview::handle_page_load(webview, payload);
        })
        .on_menu_event(handle_app_menu_event)
        .setup(move |app| {
            native_perf_mark("tauri_ready");
            setup_native_menu(app.handle())?;
            setup_tray(app.handle())?;
            preview_tools_state.set_app(app.handle().clone());
            // Connect any persisted MCP servers in the background (best-effort).
            tauri::async_runtime::spawn(async move {
                mcp.connect_all().await;
            });
            tauri::async_runtime::spawn(async move {
                loop {
                    let store = artifact_migration_store.clone();
                    let progress = tauri::async_runtime::spawn_blocking(move || -> milim_core::Result<
                        Option<(milim_storage::RunArtifactMigrationProgress, usize)>,
                    > {
                        if store.control_has_active_runs()? {
                            return Ok(None);
                        }
                        let artifacts = store.migrate_run_artifacts_batch(4 * 1024 * 1024)?;
                        let indexed_messages = store.index_session_messages_batch(500)?;
                        Ok(Some((artifacts, indexed_messages)))
                    })
                    .await;
                    match progress {
                        Ok(Ok(Some((progress, indexed))))
                            if progress.remaining == 0 && indexed == 0 =>
                        {
                            break;
                        }
                        Ok(Ok(Some(_))) | Ok(Ok(None)) => {
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                        Ok(Err(error)) => {
                            tracing::warn!("run artifact migration paused: {error}");
                            break;
                        }
                        Err(error) => {
                            tracing::warn!("run artifact migration task failed: {error}");
                            break;
                        }
                    }
                }
            });
            if state.schedules.is_some() {
                let scheduler_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    milim_server::scheduler_loop(scheduler_state).await;
                });
            }
            if let Some(control) = model_favorites_control {
                let mut events = control.subscribe();
                let favorites_app = app.handle().clone();
                let favorites_control = control.clone();
                let favorites_task = tauri::async_runtime::spawn(async move {
                    loop {
                        match events.recv().await {
                            Ok(event)
                                if event.event_type
                                    == milim_server::control::MODEL_FAVORITES_EVENT_TYPE =>
                            {
                                let favorites = event
                                    .data
                                    .get("favorite_model_ids")
                                    .cloned()
                                    .unwrap_or_else(|| json!([]));
                                let _ = favorites_app.emit(MODEL_FAVORITES_UPDATED_EVENT, favorites);
                            }
                            Ok(_) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                let _ = favorites_app.emit(
                                    MODEL_FAVORITES_UPDATED_EVENT,
                                    favorites_control.model_favorites(),
                                );
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
                server_runtime.register(favorites_task);
            }
            // Run both embedded servers until desktop shutdown signals them.
            let server_app = app.handle().clone();
            let main_shutdown = server_runtime.subscribe();
            let mobile_shutdown = server_runtime.subscribe();
            let server_task = tauri::async_runtime::spawn(async move {
                let main_server = async move {
                    match tokio::net::TcpListener::from_std(server_listener) {
                        Ok(listener) => {
                            if let Err(e) = milim_server::serve_listener_with_graceful_shutdown(
                                state,
                                listener,
                                wait_for_desktop_shutdown(main_shutdown),
                            )
                            .await
                            {
                                tracing::error!("embedded milim server error: {e}");
                                let _ = server_app.emit(RUNTIME_FAILED_EVENT, ());
                            }
                        }
                        Err(e) => {
                            tracing::error!("embedded milim server listener error: {e}");
                            let _ = server_app.emit(RUNTIME_FAILED_EVENT, ());
                        }
                    }
                };
                let mobile_server = async move {
                    match tokio::net::TcpListener::from_std(mobile_listener) {
                        Ok(listener) => {
                            if let Err(e) =
                                milim_server::serve_mobile_companion_listener_with_graceful_shutdown(
                                    mobile_state,
                                    listener,
                                    wait_for_desktop_shutdown(mobile_shutdown),
                                )
                                .await
                            {
                                tracing::warn!("embedded mobile control server error: {e}");
                            }
                        }
                        Err(e) => tracing::warn!("embedded mobile control listener error: {e}"),
                    }
                };
                tokio::join!(main_server, mobile_server);
            });
            server_runtime.register(server_task);
            if mobile_lan_enabled {
                if let Err(error) = server_runtime.enable_lan() {
                    tracing::warn!("saved LAN mobile mode could not start: {error}");
                }
            }
            if let Some(companion) = mobile_startup_companion {
                tauri::async_runtime::spawn(async move {
                    if companion.status(milim_server::now_unix()).enabled {
                        restore_mobile_tailscale_startup(mobile_startup_target).await;
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == MAIN_WINDOW_LABEL {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Err(error) = request_workspace_editor_leave(
                        window.app_handle(),
                        WorkspaceEditorLeaveAction::Hide,
                    ) {
                        tracing::warn!("window close-to-tray request failed: {error}");
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            health,
            perf_snapshot_v2,
            record_frontend_error,
            diagnostics_path,
            restart_app,
            secret_storage_status,
            request_desktop_hide,
            request_desktop_quit,
            set_sidebar_toggle_enabled,
            set_workspace_editor_dirty,
            complete_workspace_editor_leave,
            api_base_url,
            api_token,
            refresh_provider_models,
            mobile_tailscale_status,
            configure_mobile_tailscale,
            disable_mobile_tailscale,
            mobile_lan_status,
            set_mobile_lan_enabled,
            user_state_get,
            user_state_set,
            user_state_delete,
            user_sessions_get,
            user_sessions_manifest_get,
            user_session_messages_page,
            user_chat_search,
            user_sessions_set,
            user_sessions_apply_delta,
            user_sessions_apply_ops,
            user_sessions_delete,
            user_state_import_legacy,
            export_milim_backup,
            inspect_milim_backup,
            restore_milim_backup,
            quit_after_user_state_flush,
            user_data_path,
            pick_attachment_files,
            read_workspace_attachment_file,
            list_workspace_files,
            list_workspace_directory,
            read_workspace_text_file,
            write_workspace_text_file,
            save_artifact_file,
            preview_artifact_file,
            artifact_file_status,
            open_artifact_location,
            list_workspace_launchers,
            open_workspace_launcher,
            open_external_url,
            discover_harness_imports,
            get_update_platform,
            take_update_recovery_error,
            download_update_file,
            extract_app_zip,
            apply_update,
            preview_tools::set_active_preview_target,
            preview_tools::preview_picker_start,
            preview_tools::preview_picker_take,
            preview_tools::preview_picker_cancel,
            preview_webview::preview_webview_navigate,
            preview_webview::preview_webview_reload,
            preview_webview::preview_webview_set_muted,
            preview_webview::preview_webview_set_zoom,
            preview_webview::preview_webview_set_bounds,
            preview_webview::preview_webview_create,
            preview_webview::preview_webview_clear_data,
            preview_webview::preview_webview_set_visibility,
            preview_webview::preview_webview_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("error while running milim desktop");
}

#[cfg(test)]
mod desktop_control_name_tests {
    use super::*;

    #[test]
    fn computer_name_is_prepared_for_mobile_display() {
        assert_eq!(
            normalize_desktop_control_name("  Omer-MacBook-Pro.local  "),
            "Omer-MacBook-Pro"
        );
        assert_eq!(normalize_desktop_control_name("STUDIO-PC"), "STUDIO-PC");
        assert_eq!(normalize_desktop_control_name(".local"), "milim desktop");
    }

    #[test]
    fn only_old_generic_names_are_migrated() {
        assert!(has_legacy_desktop_control_name("Milim desktop"));
        assert!(has_legacy_desktop_control_name("milim-desktop"));
        assert!(!has_legacy_desktop_control_name("Omer-MacBook-Pro"));
    }
}

#[cfg(test)]
mod desktop_shutdown_tests {
    use super::*;

    #[test]
    fn desktop_shutdown_deadline_is_at_most_five_seconds() {
        assert!(
            USER_STATE_FLUSH_TIMEOUT + DESKTOP_SERVER_SHUTDOWN_TIMEOUT <= Duration::from_secs(5)
        );
    }

    #[tokio::test]
    async fn desktop_shutdown_signal_releases_server_waiters() {
        let runtime = DesktopServerRuntimeState::new(None, None);
        let waiter = tokio::spawn(wait_for_desktop_shutdown(runtime.subscribe()));

        runtime.shutdown.send(true).unwrap();

        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("shutdown waiter timed out")
            .unwrap();
    }
}

#[cfg(test)]
mod artifact_save_tests {
    use super::*;

    #[test]
    fn workspace_file_suggestions_match_relative_paths() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-files-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("Composer.tsx"), "composer").unwrap();
        fs::write(root.join("README.md"), "readme").unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules").join("skip.js"), "skip").unwrap();

        let matches =
            list_workspace_files_blocking(root.to_str().unwrap(), "comp", Some(10)).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "src/Composer.tsx");

        let all = list_workspace_files_blocking(root.to_str().unwrap(), "", Some(10)).unwrap();
        assert!(!all.iter().any(|file| file.path.contains("node_modules")));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_directory_pages_directories_first_and_rejects_traversal() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-directory-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("z.txt"), "z").unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        let first =
            list_workspace_directory_blocking(root.to_str().unwrap(), "", None, Some(2)).unwrap();
        assert_eq!(first.entries[0].kind, "directory");
        assert_eq!(first.entries[1].name, "a.txt");
        assert_eq!(first.next_cursor.as_deref(), Some("2"));
        let second = list_workspace_directory_blocking(
            root.to_str().unwrap(),
            "",
            first.next_cursor.as_deref(),
            Some(2),
        )
        .unwrap();
        assert_eq!(second.entries[0].name, "z.txt");
        assert!(!first
            .entries
            .iter()
            .chain(second.entries.iter())
            .any(|entry| entry.name == "node_modules"));
        assert!(
            list_workspace_directory_blocking(root.to_str().unwrap(), "../", None, None,).is_err()
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_text_files_save_atomically_and_detect_conflicts() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-text-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("notes.txt"), "first\r\n").unwrap();

        let original =
            read_workspace_text_file_blocking(root.to_str().unwrap(), "notes.txt").unwrap();
        assert_eq!(original.content, "first\r\n");
        let saved = write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "notes.txt",
            "second\r\n",
            &original.revision,
            false,
        )
        .unwrap();
        let WorkspaceTextWriteResult::Saved { revision, .. } = saved else {
            panic!("expected workspace text save");
        };
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "second\r\n"
        );
        assert!(matches!(
            write_workspace_text_file_blocking(
                root.to_str().unwrap(),
                "notes.txt",
                "second\r\n",
                &revision,
                false,
            )
            .unwrap(),
            WorkspaceTextWriteResult::Saved { .. }
        ));

        fs::write(root.join("notes.txt"), "external\r\n").unwrap();
        let conflict = write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "notes.txt",
            "draft\r\n",
            &revision,
            false,
        )
        .unwrap();
        assert!(matches!(
            conflict,
            WorkspaceTextWriteResult::Conflict { .. }
        ));
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "external\r\n"
        );

        let forced = write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "notes.txt",
            "draft\r\n",
            &revision,
            true,
        )
        .unwrap();
        assert!(matches!(forced, WorkspaceTextWriteResult::Saved { .. }));
        assert_eq!(
            fs::read_to_string(root.join("notes.txt")).unwrap(),
            "draft\r\n"
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_text_files_reject_unsafe_or_unsupported_inputs() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-text-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("binary.bin"), [0_u8, 1, 2]).unwrap();
        fs::write(
            root.join("large.txt"),
            vec![b'x'; MAX_WORKSPACE_TEXT_FILE_BYTES as usize + 1],
        )
        .unwrap();

        assert!(
            read_workspace_text_file_blocking(root.to_str().unwrap(), "../outside.txt").is_err()
        );
        assert!(read_workspace_text_file_blocking(root.to_str().unwrap(), "missing.txt").is_err());
        assert!(read_workspace_text_file_blocking(root.to_str().unwrap(), "binary.bin").is_err());
        assert!(read_workspace_text_file_blocking(root.to_str().unwrap(), "large.txt").is_err());
        assert!(write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "missing.txt",
            "new",
            "missing",
            true,
        )
        .is_err());
        assert!(write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "binary.bin",
            "text",
            "binary",
            true,
        )
        .is_err());
        assert!(write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "large.txt",
            &"x".repeat(MAX_WORKSPACE_TEXT_FILE_BYTES as usize + 1),
            "large",
            true,
        )
        .is_err());
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_text_files_reject_symlinks_and_permission_failures() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let root = std::env::temp_dir().join(format!(
            "milim-workspace-text-unix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("locked")).unwrap();
        fs::write(root.join("target.txt"), "target").unwrap();
        fs::write(root.join("locked").join("notes.txt"), "notes").unwrap();
        symlink(root.join("target.txt"), root.join("link.txt")).unwrap();
        assert!(read_workspace_text_file_blocking(root.to_str().unwrap(), "link.txt").is_err());

        let original =
            read_workspace_text_file_blocking(root.to_str().unwrap(), "locked/notes.txt").unwrap();
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o555)).unwrap();
        assert!(write_workspace_text_file_blocking(
            root.to_str().unwrap(),
            "locked/notes.txt",
            "changed",
            &original.revision,
            false,
        )
        .is_err());
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_attachment_reads_safe_relative_file() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-attachment-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("notes")).unwrap();
        fs::write(root.join("notes").join("brief.md"), "hello").unwrap();

        let path =
            resolve_workspace_attachment_path(root.to_str().unwrap(), "notes/brief.md").unwrap();
        let payload = read_attachment_file_blocking(&path, Some(MAX_ATTACHMENT_BYTES)).unwrap();
        assert_eq!(payload.name, "brief.md");
        assert_eq!(payload.mime, "text/markdown");
        assert_eq!(payload.content.as_deref(), Some("hello"));
        assert!(payload.data_url.is_none());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn attachment_reader_returns_image_data_url_without_text() {
        let root = std::env::temp_dir().join(format!(
            "milim-image-attachment-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("screen.png");
        fs::write(&image, [0_u8, 1, 2]).unwrap();

        let payload = read_attachment_file_blocking(&image, Some(MAX_ATTACHMENT_BYTES)).unwrap();
        assert_eq!(payload.name, "screen.png");
        assert_eq!(payload.mime, "image/png");
        assert!(payload.content.is_none());
        assert_eq!(
            payload.data_url.as_deref(),
            Some("data:image/png;base64,AAEC")
        );
        assert!(!payload.truncated);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn attachment_reader_rejects_images_over_two_megabytes() {
        let root = std::env::temp_dir().join(format!(
            "milim-large-image-attachment-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("large.png");
        fs::write(
            &image,
            vec![0_u8; MAX_ATTACHMENT_IMAGE_PREVIEW_BYTES as usize + 1],
        )
        .unwrap();

        let error = match read_attachment_file_blocking(&image, Some(MAX_ATTACHMENT_BYTES)) {
            Err(error) => error,
            Ok(_) => panic!("expected oversized image rejection"),
        };
        assert!(error.contains("image attachments must contain 1 byte"));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn workspace_attachment_rejects_absolute_and_parent_paths() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-attachment-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let outside = std::env::temp_dir().join("milim-outside-attachment.txt");
        fs::write(&outside, "secret").unwrap();

        assert!(resolve_workspace_attachment_path(root.to_str().unwrap(), "../x").is_err());
        assert!(resolve_workspace_attachment_path(
            root.to_str().unwrap(),
            outside.to_str().unwrap()
        )
        .is_err());

        fs::remove_dir_all(root).ok();
        fs::remove_file(outside).ok();
    }

    #[test]
    fn updater_rejects_nested_or_unexpected_archive_names() {
        assert!(validate_update_archive_name("milim-0.1.30.exe").is_ok());
        assert!(validate_update_archive_name("milim-0.1.30.app.zip").is_ok());
        assert!(validate_update_archive_name("../milim.exe").is_err());
        assert!(validate_update_archive_name("updates/milim.exe").is_err());
        assert!(validate_update_archive_name("milim.zip").is_err());
        assert!(validate_update_archive_name(".milim.exe").is_err());
    }

    #[test]
    fn updater_validates_download_urls() {
        assert!(validate_update_download_url(
            "https://github.com/bildhaus/milim/releases/download/v0.1.1/milim-windows-x64-portable.exe",
            "Update download URL"
        )
        .is_ok());
        assert!(validate_update_download_url(
            "https://api.github.com/repos/bildhaus/milim/releases/assets/1",
            "Update download URL"
        )
        .is_ok());
        assert!(validate_update_download_url(
            "http://github.com/bildhaus/milim/releases/download/v0.1.1/milim.exe",
            "Update download URL"
        )
        .is_err());
        assert!(validate_update_download_url(
            "https://example.com/milim.exe",
            "Update download URL"
        )
        .is_err());
    }

    #[test]
    fn updater_stages_only_safe_update_file_names() {
        let root = Path::new(r"C:\Users\USER\AppData\Local\milim\milim-updates");
        assert_eq!(
            staged_update_file_path(root, "milim-0.1.30.exe").unwrap(),
            root.join("milim-0.1.30.exe")
        );
        assert!(staged_update_file_path(root, "../milim.exe").is_err());
        assert!(staged_update_file_path(root, "milim.txt").is_err());
    }

    #[test]
    fn updater_verifies_sha256_sidecars_and_aggregate_sums() {
        let package = b"milim-update";
        let hash = sha256_hex(package);

        assert!(verify_update_checksum(
            package,
            &format!("{hash}  milim-windows-x64-portable.exe"),
            "milim-windows-x64-portable.exe"
        )
        .is_ok());
        assert!(verify_update_checksum(
            package,
            &format!(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  other.zip\n{hash}  bundle/portable/milim-windows-x64-portable.exe"
            ),
            "milim-windows-x64-portable.exe"
        )
        .is_ok());
        assert!(verify_update_checksum(
            package,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  milim-windows-x64-portable.exe",
            "milim-windows-x64-portable.exe"
        )
        .is_err());
    }

    #[test]
    fn updater_rejects_chunks_that_would_exceed_limit() {
        let mut bytes = vec![1, 2, 3];
        let err = append_update_chunk(&mut bytes, &[4, 5], "Update package", 4).unwrap_err();
        assert!(err.contains("too large"));
        assert_eq!(bytes, vec![1, 2, 3]);
    }

    #[test]
    fn updater_coalesces_known_and_unknown_download_progress() {
        assert_eq!(update_progress_percent(500, Some(1_000)), Some(50));
        assert!(!should_report_update_progress(
            509,
            Some(1_000),
            500,
            Some(50)
        ));
        assert!(should_report_update_progress(
            510,
            Some(1_000),
            500,
            Some(50)
        ));
        assert!(!should_report_update_progress(
            UPDATE_PROGRESS_UNKNOWN_STEP_BYTES - 1,
            None,
            0,
            None,
        ));
        assert!(should_report_update_progress(
            UPDATE_PROGRESS_UNKNOWN_STEP_BYTES,
            None,
            0,
            None,
        ));
    }

    #[test]
    fn updater_rejects_install_sources_for_other_platforms() {
        #[cfg(target_os = "windows")]
        {
            assert!(validate_install_source_name("milim.exe").is_ok());
            assert!(validate_install_source_name("milim.app").is_err());
        }

        #[cfg(target_os = "macos")]
        {
            assert!(validate_install_source_name("milim.app").is_ok());
            assert!(validate_install_source_name("milim.exe").is_err());
        }

        assert!(validate_install_source_name("nested/milim.exe").is_err());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_updater_uses_staged_and_backup_paths_next_to_target() {
        let current_exe = PathBuf::from(r"C:\Apps\milim\milim_0.1.29_x64-portable.exe");

        assert_eq!(
            windows_update_replacement_path(&current_exe),
            PathBuf::from(r"C:\Apps\milim\milim_0.1.29_x64-portable.exe.update")
        );
        assert_eq!(
            windows_update_backup_path(&current_exe),
            PathBuf::from(r"C:\Apps\milim\milim_0.1.29_x64-portable.exe.previous")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_update_script_retries_replaces_elevates_then_relaunches() {
        let script = build_windows_update_script(
            1234,
            WindowsUpdateScriptPaths {
                source: Path::new(
                    r"C:\Users\USER\AppData\Local\milim\milim-updates\milim-0.1.30.exe",
                ),
                replacement: Path::new(r"C:\Apps\milim\milim.exe.update"),
                target: Path::new(r"C:\Apps\milim\milim.exe"),
                backup: Path::new(r"C:\Apps\milim\milim.exe.previous"),
                log: Path::new(r"C:\Users\O'Brien\AppData\Local\milim\install.log"),
                error_marker: Path::new(r"C:\Users\O'Brien\AppData\Local\milim\install-error.txt"),
                script: Path::new(r"C:\Users\O'Brien\AppData\Local\milim\apply-update.ps1"),
            },
        );

        assert!(script.contains("$ErrorActionPreference = 'Stop'"));
        assert!(script.contains("for ($attempt = 1; $attempt -le 120; $attempt++)"));
        assert!(script.contains("Copy-Item -LiteralPath $source -Destination $replacement -Force"));
        assert!(script.contains("Move-Item -LiteralPath $target -Destination $backup -Force"));
        assert!(script.contains("Move-Item -LiteralPath $replacement -Destination $target -Force"));
        assert!(script.contains("Start-Process -FilePath $target"));
        assert!(script.contains("-Verb RunAs"));
        assert!(script.contains(r"C:\Users\O''Brien\AppData\Local\milim\install.log"));
        assert!(script.contains("Set-Content -LiteralPath $errorMarker"));
        assert!(script.contains(r"C:\Users\O''Brien\AppData\Local\milim\install-error.txt"));
    }

    #[test]
    fn harness_import_reads_codex_mcp_servers() {
        let root = std::env::temp_dir().join(format!(
            "milim-harness-import-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("config.toml");
        std::fs::write(
            &config,
            r#"
[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
cwd = "C:\\Users\\USER\\Documents\\DEV\\milim"

[mcp_servers.filesystem.env]
SECRET = "ignored"
CONFIG_PATH = "config.json"

[mcp_servers.filesystem.tools.read_file]
approval_mode = "approve"

[mcp_servers.docs]
url = "https://example.com/mcp"

[mcp_servers.local]
command='uvx'
args=['mcp-obsidian']
"#,
        )
        .unwrap();

        let mut out = Vec::new();
        read_codex_mcp_config(&config, &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "filesystem");
        assert_eq!(out[0].command, "npx");
        assert_eq!(out[0].args[1], "@modelcontextprotocol/server-filesystem");
        assert_eq!(
            out[0].cwd.as_deref(),
            Some(r"C:\Users\USER\Documents\DEV\milim")
        );
        assert_eq!(out[0].env.len(), 2);
        let secret = out[0].env.iter().find(|item| item.key == "SECRET").unwrap();
        assert!(secret.secret);
        assert!(secret.required);
        assert!(secret.value.is_none());
        let config_path = out[0]
            .env
            .iter()
            .find(|item| item.key == "CONFIG_PATH")
            .unwrap();
        assert!(!config_path.secret);
        assert_eq!(config_path.value.as_deref(), Some("config.json"));
        assert_eq!(out[1].name, "local");
        assert_eq!(out[1].command, "uvx");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_save_rejects_absolute_paths() {
        assert!(safe_artifact_relative_path("/tmp/out.txt").is_err());
        assert!(safe_artifact_relative_path("C:\\tmp\\out.txt").is_err());
    }

    #[test]
    fn artifact_save_rejects_parent_traversal() {
        assert!(safe_artifact_relative_path("../secret.txt").is_err());
        assert!(safe_artifact_relative_path("src/../../secret.txt").is_err());
    }

    #[test]
    fn artifact_save_writes_nested_file_and_blocks_overwrite() {
        let root = std::env::temp_dir().join(format!(
            "milim-artifact-save-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let first = save_artifact_file_blocking(
            root.to_string_lossy().to_string(),
            "src/generated.txt".to_string(),
            "hello".to_string(),
            false,
        )
        .unwrap();
        assert_eq!(first.bytes, 5);
        assert!(!first.overwritten);
        assert_eq!(
            std::fs::read_to_string(root.join("src").join("generated.txt")).unwrap(),
            "hello"
        );

        let blocked = save_artifact_file_blocking(
            root.to_string_lossy().to_string(),
            "src/generated.txt".to_string(),
            "updated".to_string(),
            false,
        )
        .unwrap_err();
        assert!(blocked.contains("already exists"));

        let overwritten = save_artifact_file_blocking(
            root.to_string_lossy().to_string(),
            "src/generated.txt".to_string(),
            "updated".to_string(),
            true,
        )
        .unwrap();
        assert!(overwritten.overwritten);
        assert_eq!(
            std::fs::read_to_string(root.join("src").join("generated.txt")).unwrap(),
            "updated"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_preview_reports_existing_file_diff() {
        let root = std::env::temp_dir().join(format!(
            "milim-artifact-preview-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("generated.ts"),
            "export const value = false;\n",
        )
        .unwrap();

        let preview = preview_artifact_file_blocking(
            root.to_string_lossy().to_string(),
            "src/generated.ts".to_string(),
            "export const value = true;\n".to_string(),
        )
        .unwrap();

        assert!(preview.exists);
        assert!(preview.changed);
        assert!(preview.diff.contains("-export const value = false;"));
        assert!(preview.diff.contains("+export const value = true;"));
        assert_eq!(preview.old_bytes, Some(28));
        assert_eq!(preview.new_bytes, 27);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_preview_diff_uses_unified_hunks_and_elides_middle_context() {
        let old = [
            "line 01", "line 02", "line 03", "line 04", "line 05", "line 06", "line 07", "line 08",
            "line 09", "line 10", "line 11", "line 12", "line 13",
        ]
        .join("\n");
        let new = old
            .replace("line 02", "line 02 changed")
            .replace("line 12", "line 12 changed");

        let diff = artifact_line_diff(&old, &new);

        assert!(diff.contains("@@ -1,4 +1,4 @@"));
        assert!(diff.contains("@@ -10,4 +10,4 @@"));
        assert!(diff.contains("-line 02"));
        assert!(diff.contains("+line 02 changed"));
        assert!(diff.contains("-line 12"));
        assert!(diff.contains("+line 12 changed"));
        assert!(diff.contains("... 5 unchanged lines ..."));
        assert!(!diff.contains(" line 07\n"));
    }

    #[test]
    fn artifact_open_rejects_missing_path() {
        let missing = std::env::temp_dir().join(format!(
            "milim-missing-artifact-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(validate_artifact_open_path(&missing).is_err());
    }

    #[test]
    fn artifact_status_reports_missing_and_existing_file() {
        let root = std::env::temp_dir().join(format!(
            "milim-artifact-status-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("status-target.txt");

        let missing = artifact_file_status_blocking(path.to_string_lossy().to_string()).unwrap();
        assert!(!missing.exists);
        assert!(!missing.is_file);
        assert!(!missing.is_dir);
        assert_eq!(missing.bytes, None);

        std::fs::write(&path, "hello").unwrap();
        let existing = artifact_file_status_blocking(path.to_string_lossy().to_string()).unwrap();
        assert!(existing.exists);
        assert!(existing.is_file);
        assert!(!existing.is_dir);
        assert_eq!(existing.bytes, Some(5));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn artifact_open_command_uses_existing_file_as_arg() {
        let root = std::env::temp_dir().join(format!(
            "milim-open-artifact-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("open-target.txt");
        std::fs::write(&path, "hello").unwrap();
        let spec = artifact_open_command(&path, ArtifactOpenTarget::File).unwrap();
        assert!(!spec.program.is_empty());
        assert!(
            spec.args.iter().any(|arg| arg.contains("open-target.txt")),
            "open command must pass the file path as a process argument"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_launcher_rejects_invalid_workspace_roots() {
        assert!(validate_workspace_launcher_root("").is_err());

        let missing = std::env::temp_dir().join(format!(
            "milim-missing-workspace-launcher-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(validate_workspace_launcher_root(missing.to_str().unwrap()).is_err());

        let file = std::env::temp_dir().join(format!(
            "milim-workspace-launcher-file-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&file, "not a dir").unwrap();
        assert!(validate_workspace_launcher_root(file.to_str().unwrap()).is_err());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn workspace_launcher_rejects_unknown_launcher_id() {
        assert!(parse_workspace_launcher_id("unknown").is_err());
        assert_eq!(
            parse_workspace_launcher_id("vscode").unwrap(),
            WorkspaceLauncherId::Vscode
        );
    }

    #[test]
    fn workspace_launcher_file_manager_command_uses_workspace_arg() {
        let root = Path::new(r"C:\workspaces\milim");
        let spec = workspace_launcher_command_for_platform(
            root,
            WorkspaceLauncherId::FileManager,
            WorkspaceLauncherPlatform::Windows,
            |_| None,
        )
        .unwrap();
        assert_eq!(spec.program, "explorer");
        assert_eq!(spec.args, vec![r"C:\workspaces\milim".to_string()]);
    }

    #[test]
    fn workspace_launcher_editor_command_passes_path_as_arg() {
        let root = Path::new(r"C:\workspaces\milim");
        let spec = workspace_launcher_command_for_platform(
            root,
            WorkspaceLauncherId::Vscode,
            WorkspaceLauncherPlatform::Windows,
            |name| (name == "code.cmd").then(|| PathBuf::from(r"C:\Tools\Code\code.cmd")),
        )
        .unwrap();
        assert_eq!(spec.program, r"C:\Tools\Code\code.cmd");
        assert_eq!(spec.args, vec![r"C:\workspaces\milim".to_string()]);
    }

    #[test]
    fn workspace_launcher_windows_specs_cover_terminal_git_bash_and_wsl() {
        let root = Path::new(r"C:\workspaces\milim");
        let resolver = |name: &str| match name {
            "wt.exe" => Some(PathBuf::from(r"C:\Windows\System32\wt.exe")),
            "git-bash.exe" => Some(PathBuf::from(r"C:\Program Files\Git\git-bash.exe")),
            "wsl.exe" => Some(PathBuf::from(r"C:\Windows\System32\wsl.exe")),
            _ => None,
        };

        let terminal = workspace_launcher_command_for_platform(
            root,
            WorkspaceLauncherId::Terminal,
            WorkspaceLauncherPlatform::Windows,
            resolver,
        )
        .unwrap();
        assert_eq!(terminal.program, r"C:\Windows\System32\wt.exe");
        assert_eq!(
            terminal.args,
            vec!["-d".to_string(), r"C:\workspaces\milim".to_string()]
        );

        let git_bash = workspace_launcher_command_for_platform(
            root,
            WorkspaceLauncherId::GitBash,
            WorkspaceLauncherPlatform::Windows,
            resolver,
        )
        .unwrap();
        assert_eq!(git_bash.program, r"C:\Program Files\Git\git-bash.exe");
        assert_eq!(git_bash.args, vec![r"--cd=C:\workspaces\milim".to_string()]);

        let wsl = workspace_launcher_command_for_platform(
            root,
            WorkspaceLauncherId::Wsl,
            WorkspaceLauncherPlatform::Windows,
            resolver,
        )
        .unwrap();
        assert_eq!(wsl.program, r"C:\Windows\System32\wsl.exe");
        assert_eq!(
            wsl.args,
            vec!["--cd".to_string(), r"C:\workspaces\milim".to_string()]
        );
    }

    #[test]
    fn workspace_launcher_marker_reasons_use_workspace_folders() {
        let root = std::env::temp_dir().join(format!(
            "milim-workspace-launcher-markers-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join(".zed")).unwrap();
        assert_eq!(
            workspace_launcher_marker_reason(&root, WorkspaceLauncherId::Zed).as_deref(),
            Some("Workspace has .zed settings")
        );
        assert!(workspace_launcher_marker_reason(&root, WorkspaceLauncherId::Vscode).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_url_open_rejects_non_web_urls() {
        assert!(validate_external_url("file:///C:/secret.txt").is_err());
        assert!(validate_external_url("https://example.com/login").is_ok());
    }

    #[test]
    fn external_url_open_command_passes_url_as_arg() {
        let spec = external_url_open_command("https://example.com/login");
        assert!(spec
            .args
            .iter()
            .any(|arg| arg == "https://example.com/login"));
    }

    #[test]
    fn user_data_store_opens_wal_database() {
        let root = std::env::temp_dir().join(format!(
            "milim-storage-user-data-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("milim.db");

        let store = open_user_data_store_at(&db_path).unwrap();
        store
            .set_json("milim.sessions", r#"{"state":{"sessions":[]},"version":0}"#)
            .unwrap();
        assert_eq!(
            store.get_json("milim.sessions").unwrap().as_deref(),
            Some(r#"{"state":{"sessions":[]},"version":0}"#)
        );
        assert!(db_path.with_extension("db-wal").exists());

        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod database_concurrency_tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn wal_profile_handles_workers_sessions_and_memory_concurrently() {
        let root = std::env::temp_dir().join(format!(
            "milim-database-concurrency-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let profile_path = root.join("milim.db");

        let sessions = open_user_data_store_at(&profile_path).unwrap();
        sessions
            .set_sessions_snapshot(
                r#"{"state":{"sessions":[{"id":"session-1","title":"Test","messages":[]}],"activeId":"session-1"},"version":0}"#,
            )
            .unwrap();

        let embedder: SharedService = Arc::new(UnavailableBackend::new());
        let memory = Arc::new(
            milim_memory::MemoryStore::new(
                Database::open_with_options(
                    &profile_path,
                    DatabaseOptions {
                        journal_mode: JournalMode::Wal,
                    },
                )
                .unwrap(),
                embedder,
            )
            .unwrap(),
        );
        let threads = Arc::new(
            milim_agents::ThreadStore::new(Database::open(&root.join("threads.db")).unwrap())
                .unwrap(),
        );
        let worker_ids = (0..4)
            .map(|index| {
                threads
                    .create(
                        "parent-1",
                        &format!("Worker {index}"),
                        "test-echo",
                        None,
                        "stress",
                    )
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>();

        let barrier = Arc::new(Barrier::new(7));
        let mut handles = worker_ids
            .into_iter()
            .map(|worker_id| {
                let threads = threads.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for index in 0..25 {
                        threads
                            .append_event(
                                &worker_id,
                                "token",
                                serde_json::json!({ "index": index }),
                            )
                            .unwrap();
                        std::thread::yield_now();
                    }
                })
            })
            .collect::<Vec<_>>();

        {
            let threads = threads.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    threads.child_events_after("parent-1", 0, 500).unwrap();
                    std::thread::yield_now();
                }
            }));
        }
        {
            let sessions = sessions.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for revision in 0..50 {
                    sessions
                        .apply_sessions_delta(SessionsDelta {
                            meta_json: format!(
                                r#"{{"state":{{"activeId":"session-1","revision":{revision}}},"version":0}}"#
                            ),
                            session_order: vec!["session-1".into()],
                            upserts: Vec::new(),
                            deleted_session_ids: Vec::new(),
                        })
                        .unwrap();
                    std::thread::yield_now();
                }
            }));
        }
        {
            let memory = memory.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    memory.count().unwrap();
                    memory.list_scopes().unwrap();
                    std::thread::yield_now();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
        let events = threads.child_events_after("parent-1", 0, 500).unwrap();
        assert_eq!(events.len(), 100);
        assert!(events.windows(2).all(|pair| pair[0].1.seq < pair[1].1.seq));
        assert_eq!(memory.count().unwrap(), 0);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &sessions.get_sessions_snapshot().unwrap().unwrap()
            )
            .unwrap()["state"]["revision"],
            49
        );

        drop(memory);
        drop(sessions);
        drop(threads);
        let _ = std::fs::remove_dir_all(root);
    }
}
