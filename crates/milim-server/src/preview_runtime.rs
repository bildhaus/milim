use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::TcpListener;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use milim_tools::{Tool, ToolEffect};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;

const MAX_LOG_LINES: usize = 500;
const INSTALL_MARKER_FILE: &str = ".milim-install-ok";
const PREVIEW_MANIFEST_FILE: &str = ".milim/preview.json";
#[cfg(not(test))]
const PREVIEW_COMPILE_ERROR_QUIET_MS: u64 = 1_000;
#[cfg(test)]
const PREVIEW_COMPILE_ERROR_QUIET_MS: u64 = 10;
#[cfg(not(test))]
const PREVIEW_READY_PROBE_TIMEOUT_MS: u64 = 10_000;
#[cfg(test)]
const PREVIEW_READY_PROBE_TIMEOUT_MS: u64 = 100;
#[cfg(not(test))]
const PREVIEW_READY_PROBE_INTERVAL_MS: u64 = 250;
#[cfg(test)]
const PREVIEW_READY_PROBE_INTERVAL_MS: u64 = 5;
#[cfg(not(test))]
const PREVIEW_READY_REQUEST_TIMEOUT_MS: u64 = 1_000;
#[cfg(test)]
const PREVIEW_READY_REQUEST_TIMEOUT_MS: u64 = 50;
#[cfg(not(test))]
const PREVIEW_PROCESS_STOP_TIMEOUT_MS: u64 = 10_000;
#[cfg(test)]
const PREVIEW_PROCESS_STOP_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewAppLog {
    pub seq: u64,
    pub ts: u64,
    pub stream: String,
    pub line: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewAppError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewAppPreflight {
    pub thread_id: String,
    pub cwd: String,
    pub managed: bool,
    pub scope: String,
    pub package_manager: String,
    pub configuration: String,
    pub install_required: bool,
    pub install_command: String,
    pub dev_command: String,
    pub source_fingerprint: String,
    pub port: u16,
    pub url: String,
    pub healthcheck_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewAppStatus {
    pub thread_id: String,
    pub kind: String,
    pub status: String,
    pub active: bool,
    pub ready: bool,
    pub managed: bool,
    pub run_id: Option<String>,
    pub updated_at: u64,
    pub error: Option<PreviewAppError>,
    pub preflight: Option<PreviewAppPreflight>,
    pub cwd: String,
    pub url: Option<String>,
    pub pid: Option<u32>,
    pub command: Option<String>,
    pub message: Option<String>,
    pub logs: Vec<PreviewAppLog>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PreviewAppFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreviewAppStageRequest {
    #[serde(default)]
    pub files: Vec<PreviewAppFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PreviewAppPreflightRequest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub files: Vec<PreviewAppFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PreviewAppStartRequest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub files: Vec<PreviewAppFile>,
    #[serde(default)]
    pub source_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreviewStaticStartRequest {
    pub cwd: String,
    pub entry_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewAppLogsResponse {
    pub logs: Vec<PreviewAppLog>,
    pub next_seq: u64,
    pub truncated: bool,
}

#[derive(Default)]
struct PreviewAppEntry {
    cwd: Option<PathBuf>,
    kind: String,
    status: String,
    active: bool,
    ready: bool,
    managed: bool,
    run_id: Option<String>,
    updated_at: u64,
    error: Option<PreviewAppError>,
    preflight: Option<PreviewAppPreflight>,
    url: Option<String>,
    pid: Option<u32>,
    command: Option<String>,
    message: Option<String>,
    logs: VecDeque<PreviewAppLog>,
    next_log_seq: u64,
    cancel: Option<watch::Sender<bool>>,
    task: Option<JoinHandle<()>>,
    compile_error_at: Option<Instant>,
}

pub struct PreviewRuntimeManager {
    root: PathBuf,
    entries: Mutex<HashMap<String, PreviewAppEntry>>,
}

impl PreviewRuntimeManager {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn status(&self, thread_id: &str) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        Ok(self.status_for(&thread_id))
    }

    pub fn logs(&self, thread_id: &str) -> Result<Vec<PreviewAppLog>> {
        Ok(self.logs_after(thread_id, None)?.logs)
    }

    pub fn logs_after(
        &self,
        thread_id: &str,
        after_seq: Option<u64>,
    ) -> Result<PreviewAppLogsResponse> {
        let thread_id = safe_thread_id(thread_id)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let Some(entry) = entries.get(&thread_id) else {
            return Ok(PreviewAppLogsResponse {
                logs: Vec::new(),
                next_seq: after_seq.unwrap_or_default(),
                truncated: false,
            });
        };
        let requested = after_seq.unwrap_or_default();
        let oldest = entry.logs.front().map(|log| log.seq);
        let logs = entry
            .logs
            .iter()
            .filter(|log| after_seq.is_none_or(|seq| log.seq > seq))
            .cloned()
            .collect();
        let next_seq = entry
            .logs
            .back()
            .map(|log| log.seq.max(requested))
            .unwrap_or(requested);
        Ok(PreviewAppLogsResponse {
            logs,
            next_seq,
            truncated: after_seq.is_some()
                && oldest.is_some_and(|seq| seq > requested.saturating_add(1)),
        })
    }

    pub fn stage(&self, thread_id: &str, files: &[PreviewAppFile]) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        if files.is_empty() {
            return Err(Error::InvalidRequest(
                "preview app staging requires at least one file".to_string(),
            ));
        }
        let dir = self.app_dir(&thread_id);
        if self.running_status(&thread_id, &dir)?.is_some() {
            return Err(Error::InvalidRequest(
                "stop the preview app before staging new files".to_string(),
            ));
        }
        self.stage_files_atomically(&thread_id, files)?;
        self.set_entry(&thread_id, |entry| {
            entry.cwd = Some(dir.clone());
            entry.kind = "app".to_string();
            entry.managed = true;
            entry.preflight = None;
            if !entry.active {
                entry.status = "staged".to_string();
            }
            entry.message = Some(format!("Staged {} file(s).", files.len()));
            push_log(entry, "system", &format!("staged {} file(s)", files.len()));
        })?;
        Ok(self.status_for(&thread_id))
    }

    pub async fn start_static(
        self: &Arc<Self>,
        thread_id: &str,
        request: &PreviewStaticStartRequest,
    ) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        let root = canonical_static_root(&request.cwd)?;
        let entry_path = safe_relative_path(&request.entry_path)?;
        let entry_file = canonical_static_entry(&root, &entry_path)?;
        let url_path = static_url_path(&entry_path);

        {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
            if let Some(entry) = entries.get_mut(&thread_id).filter(|entry| entry.active) {
                if entry.kind == "static" && entry.cwd.as_deref() == Some(root.as_path()) {
                    let port = entry
                        .url
                        .as_deref()
                        .and_then(static_preview_port)
                        .ok_or_else(|| {
                            Error::Other("active static preview URL is invalid".to_string())
                        })?;
                    entry.url = Some(format!("http://127.0.0.1:{port}/{url_path}"));
                    entry.message = Some(format!(
                        "Serving {}.",
                        workspace_relative_display(&root, &entry_file)
                    ));
                    entry.updated_at = crate::now_unix();
                    return Ok(status_from_entry(
                        &thread_id,
                        entry,
                        &self.app_dir(&thread_id),
                    ));
                }
                return Err(Error::InvalidRequest(
                    "stop the current preview before starting a static preview".to_string(),
                ));
            }
        }

        let listener = TokioTcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let url = format!("http://127.0.0.1:{port}/{url_path}");
        let run_id = uuid::Uuid::new_v4().simple().to_string();
        let (cancel, cancel_rx) = watch::channel(false);

        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let entry = entries
            .entry(thread_id.clone())
            .or_insert_with(|| PreviewAppEntry {
                status: "idle".to_string(),
                managed: true,
                ..Default::default()
            });
        if entry.active {
            return Err(Error::InvalidRequest(
                "stop the current preview before starting a static preview".to_string(),
            ));
        }
        entry.task.take();
        entry.cwd = Some(root.clone());
        entry.kind = "static".to_string();
        entry.status = "running".to_string();
        entry.active = true;
        entry.ready = true;
        entry.managed = false;
        entry.run_id = Some(run_id.clone());
        entry.error = None;
        entry.preflight = None;
        entry.url = Some(url);
        entry.pid = None;
        entry.command = None;
        entry.message = Some(format!(
            "Serving {}.",
            workspace_relative_display(&root, &entry_file)
        ));
        entry.cancel = Some(cancel);
        entry.compile_error_at = None;
        entry.updated_at = crate::now_unix();
        push_log(entry, "system", "static preview is running");

        let manager = self.clone();
        let run_thread_id = thread_id.clone();
        entry.task = Some(tokio::spawn(async move {
            run_static_preview(manager, run_thread_id, run_id, root, listener, cancel_rx).await;
        }));
        drop(entries);
        Ok(self.status_for(&thread_id))
    }

    pub fn preflight(
        &self,
        thread_id: &str,
        request: &PreviewAppPreflightRequest,
    ) -> Result<PreviewAppPreflight> {
        let thread_id = safe_thread_id(thread_id)?;
        let target = self.start_target(&thread_id, request.cwd.as_deref())?;
        if self.running_status(&thread_id, &target.dir)?.is_some() {
            return Err(Error::InvalidRequest(
                "stop the preview app before running preflight".to_string(),
            ));
        }
        if !target.managed && !request.files.is_empty() {
            return Err(Error::InvalidRequest(
                "selected-folder preview preflight does not accept managed files".to_string(),
            ));
        }
        let inspected = inspect_preview_source(&target, &request.files)?;
        let package = &inspected.package;
        validate_preview_package(&package)?;
        let port = match package.explicit_port {
            Some(port) => port,
            None => free_port()?,
        };
        if package.explicit_port.is_some() && !port_is_available(port) {
            return Err(configured_preview_port_in_use_error(
                port,
                &package.configuration,
            ));
        }
        let cwd = inspected.dir.to_string_lossy().to_string();
        let url = preview_loopback_url(port, &package.url_path);
        let healthcheck_url = preview_loopback_url(port, &package.healthcheck_path);
        let preflight = PreviewAppPreflight {
            thread_id: thread_id.clone(),
            cwd,
            managed: target.managed,
            scope: if target.managed {
                "managed".to_string()
            } else {
                "selected_folder".to_string()
            },
            package_manager: package.launcher_name(),
            configuration: package.configuration.clone(),
            install_required: inspected.install_required,
            install_command: package.install_label(),
            dev_command: package.dev_label(port),
            source_fingerprint: inspected.source_fingerprint,
            port,
            url,
            healthcheck_url,
        };
        self.set_entry(&thread_id, |entry| {
            entry.cwd = Some(inspected.dir.clone());
            entry.kind = "app".to_string();
            entry.managed = target.managed;
            entry.preflight = Some(preflight.clone());
        })?;
        Ok(preflight)
    }

    pub fn start(
        self: &Arc<Self>,
        thread_id: &str,
        request: &PreviewAppStartRequest,
    ) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        let target = self.start_target(&thread_id, request.cwd.as_deref())?;
        if !target.managed && !request.files.is_empty() {
            return Err(Error::InvalidRequest(
                "selected-folder preview start does not accept managed files".to_string(),
            ));
        }
        let supplied_fingerprint = request
            .source_fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::InvalidRequest(
                    "preview app start requires a current preflight fingerprint".to_string(),
                )
            })?;
        let expected = {
            let entries = self
                .entries
                .lock()
                .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
            entries
                .get(&thread_id)
                .and_then(|entry| entry.preflight.clone())
                .ok_or_else(|| {
                    Error::InvalidRequest(
                        "preview app preflight is required before start".to_string(),
                    )
                })?
        };
        let inspected = inspect_preview_source(&target, &request.files)?;
        if let Some(status) = self.running_status(&thread_id, &inspected.dir)? {
            return Ok(status);
        }
        let inspected_package = &inspected.package;
        validate_preview_package(inspected_package)?;
        if expected.managed != target.managed || Path::new(&expected.cwd) != inspected.dir.as_path()
        {
            return Err(stale_preflight_error());
        }
        if supplied_fingerprint != expected.source_fingerprint
            || inspected.source_fingerprint != expected.source_fingerprint
            || inspected.install_required != expected.install_required
            || inspected_package.launcher_name() != expected.package_manager
            || inspected_package.configuration != expected.configuration
            || inspected_package.install_label() != expected.install_command
            || inspected_package.dev_label(expected.port) != expected.dev_command
            || preview_loopback_url(expected.port, &inspected_package.url_path) != expected.url
            || preview_loopback_url(expected.port, &inspected_package.healthcheck_path)
                != expected.healthcheck_url
        {
            return Err(stale_preflight_error());
        }
        if !port_is_available(expected.port) {
            return Err(if inspected_package.explicit_port.is_some() {
                configured_preview_port_in_use_error(
                    expected.port,
                    &inspected_package.configuration,
                )
            } else {
                Error::InvalidRequest(
                    "preview app preflight port is no longer available; run preflight again"
                        .to_string(),
                )
            });
        }

        let dir = inspected.dir;
        let package = inspected.package;
        let install_required = inspected.install_required;
        let files = request.files.clone();
        let stages_files = target.managed && !files.is_empty();
        let run_id = uuid::Uuid::new_v4().simple().to_string();
        let (cancel, cancel_rx) = watch::channel(false);
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let entry = entries
            .entry(thread_id.clone())
            .or_insert_with(|| PreviewAppEntry {
                status: "idle".to_string(),
                managed: true,
                ..Default::default()
            });
        if entry.active {
            return Ok(status_from_entry(
                &thread_id,
                entry,
                &self.app_dir(&thread_id),
            ));
        }
        entry.task.take();
        entry.cwd = Some(dir.clone());
        entry.kind = "app".to_string();
        entry.status = if stages_files {
            "staging".to_string()
        } else if install_required {
            "installing".to_string()
        } else {
            "starting".to_string()
        };
        entry.active = true;
        entry.ready = false;
        entry.managed = target.managed;
        entry.run_id = Some(run_id.clone());
        entry.error = None;
        entry.url = Some(expected.url.clone());
        entry.pid = None;
        entry.command = Some(if install_required {
            expected.install_command.clone()
        } else {
            expected.dev_command.clone()
        });
        entry.message = Some(if stages_files {
            "Staging preview files.".to_string()
        } else if install_required {
            "Installing dependencies.".to_string()
        } else {
            "Starting dev server.".to_string()
        });
        entry.cancel = Some(cancel);
        entry.compile_error_at = None;
        entry.updated_at = crate::now_unix();
        push_log(entry, "system", "starting preview app");
        let manager = self.clone();
        let run_thread_id = thread_id.clone();
        let port = expected.port;
        let healthcheck_url = expected.healthcheck_url.clone();
        let run = PreviewRun {
            thread_id: run_thread_id,
            run_id,
            dir,
            port,
            healthcheck_url,
            package,
            managed: target.managed,
            files,
            install_required,
        };
        let task = tokio::spawn(async move {
            run_preview_app(manager, run, cancel_rx).await;
        });
        entry.task = Some(task);
        drop(entries);
        Ok(self.status_for(&thread_id))
    }

    pub async fn stop(&self, thread_id: &str) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        let (run_id, cancel, task, fallback_pid) = {
            let mut entries = self
                .entries
                .lock()
                .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
            let entry = entries
                .entry(thread_id.clone())
                .or_insert_with(|| PreviewAppEntry {
                    status: "idle".to_string(),
                    managed: true,
                    ..Default::default()
                });
            let run_id = entry.run_id.clone();
            if !entry.active {
                entry.status = "stopped".to_string();
                entry.ready = false;
                entry.pid = None;
                entry.error = None;
                entry.message = Some("Stopped.".to_string());
                entry.updated_at = crate::now_unix();
                return Ok(status_from_entry(
                    &thread_id,
                    entry,
                    &self.app_dir(&thread_id),
                ));
            }
            entry.status = "stopping".to_string();
            entry.ready = false;
            entry.message = Some("Stopping.".to_string());
            entry.updated_at = crate::now_unix();
            (run_id, entry.cancel.take(), entry.task.take(), entry.pid)
        };
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        } else if let Some(pid) = fallback_pid {
            let _ = kill_process_tree(pid).await;
        }
        if let Some(task) = task {
            let _ = task.await;
        }
        self.set_entry(&thread_id, |entry| {
            if entry.run_id == run_id {
                entry.status = "stopped".to_string();
                entry.active = false;
                entry.ready = false;
                entry.pid = None;
                entry.error = None;
                entry.message = Some("Stopped.".to_string());
                entry.cancel = None;
                entry.compile_error_at = None;
                push_log(entry, "system", "stopped preview app");
            }
        })?;
        Ok(self.status_for(&thread_id))
    }

    pub async fn restart(
        self: &Arc<Self>,
        thread_id: &str,
        request: &PreviewAppStartRequest,
    ) -> Result<PreviewAppStatus> {
        let thread_id = safe_thread_id(thread_id)?;
        let _ = self.stop(&thread_id).await?;
        self.start(&thread_id, request)
    }

    pub async fn stop_all(&self) -> Result<()> {
        let thread_ids = {
            let entries = self
                .entries
                .lock()
                .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
            entries
                .iter()
                .filter(|(_, entry)| entry.active)
                .map(|(thread_id, _)| thread_id.clone())
                .collect::<Vec<_>>()
        };
        for thread_id in thread_ids {
            self.stop(&thread_id).await?;
        }
        Ok(())
    }

    fn stage_files_atomically(&self, thread_id: &str, files: &[PreviewAppFile]) -> Result<()> {
        let dir = self.app_dir(thread_id);
        let staging = self.root.join(format!(
            ".{thread_id}.staging-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let backup = self.root.join(format!(
            ".{thread_id}.backup-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let mut seen = HashSet::new();
        let mut paths = Vec::with_capacity(files.len());
        for file in files {
            let rel = safe_relative_path(&file.path)?;
            if !seen.insert(rel.clone()) {
                return Err(Error::InvalidRequest(format!(
                    "duplicate preview app file path: {}",
                    file.path
                )));
            }
            paths.push((rel, file));
        }
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(&staging)?;
        let write_result = (|| -> Result<()> {
            for (rel, file) in &paths {
                let target = staging.join(rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(target, file.content.as_bytes())?;
            }
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let had_previous = dir.exists();
        if had_previous {
            if let Err(error) = std::fs::rename(&dir, &backup) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(error.into());
            }
        }
        if let Err(error) = std::fs::rename(&staging, &dir) {
            if had_previous {
                let _ = std::fs::rename(&backup, &dir);
            }
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        if had_previous {
            let _ = std::fs::remove_dir_all(&backup);
        }
        Ok(())
    }

    fn app_dir(&self, thread_id: &str) -> PathBuf {
        self.root.join(thread_id)
    }

    fn start_target(&self, thread_id: &str, cwd: Option<&str>) -> Result<PreviewAppTarget> {
        let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(PreviewAppTarget {
                dir: self.app_dir(thread_id),
                managed: true,
            });
        };
        let dir = PathBuf::from(cwd);
        if !dir.is_absolute() {
            return Err(Error::InvalidRequest(format!(
                "preview app cwd must be absolute: {cwd}"
            )));
        }
        if !dir.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "preview app cwd is not a directory: {cwd}"
            )));
        }
        Ok(PreviewAppTarget {
            dir,
            managed: false,
        })
    }

    fn status_for(&self, thread_id: &str) -> PreviewAppStatus {
        let entries = self.entries.lock().ok();
        let entry = entries.as_ref().and_then(|items| items.get(thread_id));
        match entry {
            Some(entry) => status_from_entry(thread_id, entry, &self.app_dir(thread_id)),
            None => PreviewAppStatus {
                thread_id: thread_id.to_string(),
                kind: "app".to_string(),
                status: "idle".to_string(),
                active: false,
                ready: false,
                managed: true,
                run_id: None,
                updated_at: 0,
                error: None,
                preflight: None,
                cwd: self.app_dir(thread_id).to_string_lossy().to_string(),
                url: None,
                pid: None,
                command: None,
                message: None,
                logs: Vec::new(),
            },
        }
    }

    fn running_status(&self, thread_id: &str, dir: &Path) -> Result<Option<PreviewAppStatus>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let Some(entry) = entries.get(thread_id) else {
            return Ok(None);
        };
        if entry.active && entry.cwd.as_deref().is_some_and(|current| current != dir) {
            return Err(Error::InvalidRequest(
                "stop the current preview app before starting another folder".to_string(),
            ));
        }
        Ok(entry
            .active
            .then(|| status_from_entry(thread_id, entry, &self.app_dir(thread_id))))
    }

    fn set_entry(&self, thread_id: &str, update: impl FnOnce(&mut PreviewAppEntry)) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let entry = entries
            .entry(thread_id.to_string())
            .or_insert_with(|| PreviewAppEntry {
                status: "idle".to_string(),
                managed: true,
                ..Default::default()
            });
        update(entry);
        entry.updated_at = crate::now_unix();
        Ok(())
    }

    fn with_run_entry(
        &self,
        thread_id: &str,
        run_id: &str,
        update: impl FnOnce(&mut PreviewAppEntry),
    ) -> Result<bool> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| Error::Other("preview runtime state lock poisoned".to_string()))?;
        let Some(entry) = entries.get_mut(thread_id) else {
            return Ok(false);
        };
        if entry.run_id.as_deref() != Some(run_id) {
            return Ok(false);
        }
        update(entry);
        entry.updated_at = crate::now_unix();
        Ok(true)
    }
}

pub(crate) fn account_runtime_preview_tools(
    manager: Arc<PreviewRuntimeManager>,
    thread_id: String,
    cwd: PathBuf,
) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(PreviewPrepareAppTool {
            manager: manager.clone(),
            thread_id: thread_id.clone(),
            cwd: cwd.clone(),
        }),
        Arc::new(PreviewStartAppTool {
            manager,
            thread_id,
            cwd,
        }),
    ]
}

struct PreviewPrepareAppTool {
    manager: Arc<PreviewRuntimeManager>,
    thread_id: String,
    cwd: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewPrepareAppArgs {}

#[async_trait]
impl Tool for PreviewPrepareAppTool {
    fn name(&self) -> &str {
        "preview_prepare_app"
    }

    fn description(&self) -> &str {
        "Return the active project's current preview status, or inspect its commands and return the fingerprint required by preview_start_app."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        serde_json::from_value::<PreviewPrepareAppArgs>(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid preview_prepare_app arguments: {error}"))
        })?;
        let status = self.manager.status(&self.thread_id)?;
        if status.active {
            if Path::new(&status.cwd) != self.cwd.as_path() {
                return Err(Error::InvalidRequest(
                    "active preview belongs to another workspace".to_string(),
                ));
            }
            return serde_json::to_value(status).map_err(Into::into);
        }
        serde_json::to_value(self.manager.preflight(
            &self.thread_id,
            &PreviewAppPreflightRequest {
                cwd: Some(self.cwd.to_string_lossy().to_string()),
                ..Default::default()
            },
        )?)
        .map_err(Into::into)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewStartAppArgs {
    source_fingerprint: String,
}

struct PreviewStartAppTool {
    manager: Arc<PreviewRuntimeManager>,
    thread_id: String,
    cwd: PathBuf,
}

#[async_trait]
impl Tool for PreviewStartAppTool {
    fn name(&self) -> &str {
        "preview_start_app"
    }

    fn description(&self) -> &str {
        "Start or reuse the active project's dev server as a Milim-owned preview that remains running between agent turns."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source_fingerprint": {
                    "type": "string",
                    "description": "Fingerprint returned by preview_prepare_app."
                }
            },
            "required": ["source_fingerprint"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let args: PreviewStartAppArgs = serde_json::from_value(args).map_err(|error| {
            Error::InvalidRequest(format!("invalid preview_start_app arguments: {error}"))
        })?;
        serde_json::to_value(self.manager.start(
            &self.thread_id,
            &PreviewAppStartRequest {
                cwd: Some(self.cwd.to_string_lossy().to_string()),
                source_fingerprint: Some(args.source_fingerprint),
                ..Default::default()
            },
        )?)
        .map_err(Into::into)
    }
}

fn status_from_entry(
    thread_id: &str,
    entry: &PreviewAppEntry,
    default_dir: &Path,
) -> PreviewAppStatus {
    PreviewAppStatus {
        thread_id: thread_id.to_string(),
        kind: if entry.kind.is_empty() {
            "app".to_string()
        } else {
            entry.kind.clone()
        },
        status: entry.status.clone(),
        active: entry.active,
        ready: entry.ready,
        managed: entry.managed,
        run_id: entry.run_id.clone(),
        updated_at: entry.updated_at,
        error: entry.error.clone(),
        preflight: entry.preflight.clone(),
        cwd: entry
            .cwd
            .clone()
            .unwrap_or_else(|| default_dir.to_path_buf())
            .to_string_lossy()
            .to_string(),
        url: entry.url.clone(),
        pid: entry.pid,
        command: entry.command.clone(),
        message: entry.message.clone(),
        logs: entry.logs.iter().cloned().collect(),
    }
}

#[derive(Clone)]
struct PreviewAppTarget {
    dir: PathBuf,
    managed: bool,
}

fn stale_preflight_error() -> Error {
    Error::InvalidRequest(
        "preview app source changed after preflight; run preflight again".to_string(),
    )
}

struct PreviewRun {
    thread_id: String,
    run_id: String,
    dir: PathBuf,
    port: u16,
    healthcheck_url: String,
    package: PreviewPackage,
    managed: bool,
    files: Vec<PreviewAppFile>,
    install_required: bool,
}

async fn run_static_preview(
    manager: Arc<PreviewRuntimeManager>,
    thread_id: String,
    run_id: String,
    root: PathBuf,
    listener: TokioTcpListener,
    mut cancel: watch::Receiver<bool>,
) {
    let stopped = cancel.clone();
    let app =
        Router::new()
            .fallback_service(ServeDir::new(&root))
            .layer(middleware::from_fn_with_state(
                root,
                validate_static_preview_path,
            ));
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(async move { wait_for_cancel(&mut cancel).await })
        .await;
    if let Err(error) = result {
        fail_run(
            &manager,
            &thread_id,
            &run_id,
            "static_server_failed",
            &format!("static preview server failed: {error}"),
        );
    } else if !*stopped.borrow() {
        fail_run(
            &manager,
            &thread_id,
            &run_id,
            "static_server_stopped",
            "static preview server stopped unexpectedly",
        );
    }
}

async fn validate_static_preview_path(
    State(root): State<PathBuf>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if static_preview_request_target(&root, request.uri().path()).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

fn canonical_static_root(value: &str) -> Result<PathBuf> {
    let path = PathBuf::from(value.trim());
    if !path.is_absolute() {
        return Err(Error::InvalidRequest(
            "static preview cwd must be absolute".to_string(),
        ));
    }
    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        Error::InvalidRequest(format!("static preview folder is invalid: {error}"))
    })?;
    if !canonical.is_dir() {
        return Err(Error::InvalidRequest(
            "static preview cwd must be a directory".to_string(),
        ));
    }
    Ok(canonical)
}

fn canonical_static_entry(root: &Path, entry_path: &Path) -> Result<PathBuf> {
    let entry = std::fs::canonicalize(root.join(entry_path)).map_err(|error| {
        Error::InvalidRequest(format!("static preview file is invalid: {error}"))
    })?;
    if !entry.starts_with(root) {
        return Err(Error::InvalidRequest(
            "static preview file must stay inside the workspace".to_string(),
        ));
    }
    if !entry.is_file() {
        return Err(Error::InvalidRequest(
            "static preview entry must be a file".to_string(),
        ));
    }
    let extension = entry
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("html") && !extension.eq_ignore_ascii_case("htm") {
        return Err(Error::InvalidRequest(
            "static preview entry must be an HTML file".to_string(),
        ));
    }
    Ok(entry)
}

fn static_preview_request_target(root: &Path, uri_path: &str) -> Option<PathBuf> {
    let decoded = decode_static_url_path(uri_path)?;
    let relative = if decoded.trim_matches('/').is_empty() {
        PathBuf::from("index.html")
    } else {
        safe_relative_path(decoded.trim_start_matches('/')).ok()?
    };
    let mut target = std::fs::canonicalize(root.join(relative)).ok()?;
    if target.is_dir() {
        target = std::fs::canonicalize(target.join("index.html")).ok()?;
    }
    (target.is_file() && target.starts_with(root)).then_some(target)
}

fn decode_static_url_path(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn static_url_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(normalized.len());
    for byte in normalized.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn static_preview_port(url: &str) -> Option<u16> {
    url.strip_prefix("http://127.0.0.1:")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

fn workspace_relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

async fn run_preview_app(
    manager: Arc<PreviewRuntimeManager>,
    run: PreviewRun,
    mut cancel: watch::Receiver<bool>,
) {
    let PreviewRun {
        thread_id,
        run_id,
        dir,
        port,
        healthcheck_url,
        package,
        managed,
        files,
        install_required,
    } = run;
    if managed && !files.is_empty() {
        if let Err(error) = manager.stage_files_atomically(&thread_id, &files) {
            fail_run(
                &manager,
                &thread_id,
                &run_id,
                "stage_failed",
                &format!("failed to stage preview files: {error}"),
            );
            return;
        }
    }
    if !run_is_active(&manager, &thread_id, &run_id) || *cancel.borrow() {
        return;
    }
    if managed {
        match ensure_vite_setup(&dir, &package) {
            Ok(logs) => {
                let _ = manager.with_run_entry(&thread_id, &run_id, |entry| {
                    for log in logs {
                        push_log(entry, "system", &log);
                    }
                });
            }
            Err(error) => {
                fail_run(
                    &manager,
                    &thread_id,
                    &run_id,
                    "stage_failed",
                    &format!("failed to prepare preview files: {error}"),
                );
                return;
            }
        }
    }
    if !run_is_active(&manager, &thread_id, &run_id) || *cancel.borrow() {
        return;
    }
    if install_required {
        match run_install_command(
            manager.clone(),
            &thread_id,
            &run_id,
            &dir,
            &package,
            &mut cancel,
        )
        .await
        {
            Ok(CommandOutcome::Success) => {
                if managed {
                    let _ = std::fs::write(dir.join(INSTALL_MARKER_FILE), b"ok");
                }
            }
            Ok(CommandOutcome::Cancelled) => return,
            Err(error) => {
                fail_run(
                    &manager,
                    &thread_id,
                    &run_id,
                    "install_failed",
                    &error.to_string(),
                );
                return;
            }
        }
    } else {
        let _ = manager.with_run_entry(&thread_id, &run_id, |entry| {
            if entry.active {
                push_log(
                    entry,
                    "system",
                    "dependencies already installed; skipping install",
                );
            }
        });
    }

    if !run_is_active(&manager, &thread_id, &run_id) || *cancel.borrow() {
        return;
    }
    let command = package.dev_label(port);
    let (dev_program, dev_args) = package.dev_invocation(port);
    let mut child = match preview_command(&dev_program)
        .args(&dev_args)
        .current_dir(&dir)
        .env("HOST", "127.0.0.1")
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            fail_run(
                &manager,
                &thread_id,
                &run_id,
                "dev_server_start_failed",
                &format!("failed to start dev server: {error}"),
            );
            return;
        }
    };
    let mut process_tree = match child.id().map(ProcessTreeGuard::attach) {
        Some(Ok(tree)) => tree,
        None => {
            let _ = child.start_kill();
            fail_run(
                &manager,
                &thread_id,
                &run_id,
                "dev_server_start_failed",
                "dev server process id was unavailable",
            );
            return;
        }
        Some(Err(error)) => {
            let _ = child.start_kill();
            fail_run(
                &manager,
                &thread_id,
                &run_id,
                "dev_server_start_failed",
                &format!("failed to contain dev server process tree: {error}"),
            );
            return;
        }
    };
    let pid = child.id();
    let current = manager
        .with_run_entry(&thread_id, &run_id, |entry| {
            if !entry.active {
                return;
            }
            entry.status = "starting".to_string();
            entry.ready = false;
            entry.pid = pid;
            entry.command = Some(command);
            entry.message = Some("Starting dev server.".to_string());
            entry.error = None;
            push_log(entry, "system", "preview app is starting");
        })
        .unwrap_or(false);
    if !current || !run_is_active(&manager, &thread_id, &run_id) {
        terminate_child(&mut child).await;
        return;
    }
    let mut stdout_log = pipe_child_logs(
        manager.clone(),
        thread_id.clone(),
        run_id.clone(),
        child.stdout.take(),
        "stdout",
    );
    let mut stderr_log = pipe_child_logs(
        manager.clone(),
        thread_id.clone(),
        run_id.clone(),
        child.stderr.take(),
        "stderr",
    );

    let readiness_deadline = Instant::now() + Duration::from_millis(PREVIEW_READY_PROBE_TIMEOUT_MS);
    let mut probe_interval =
        tokio::time::interval(Duration::from_millis(PREVIEW_READY_PROBE_INTERVAL_MS));
    probe_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            status = child.wait() => {
                match status {
                    Ok(status) => {
                        let _ = manager.with_run_entry(&thread_id, &run_id, |entry| {
                            entry.pid = None;
                            entry.active = false;
                            entry.ready = false;
                            entry.cancel = None;
                            if status.success() {
                                entry.status = "stopped".to_string();
                                entry.error = None;
                            } else {
                                entry.status = "error".to_string();
                                entry.error = Some(PreviewAppError {
                                    code: "process_exit".to_string(),
                                    message: format!("Process exited with {status}."),
                                });
                            }
                            entry.message = Some(format!("Process exited with {status}."));
                            push_log(entry, "system", &format!("process exited with {status}"));
                        });
                    }
                    Err(error) => fail_run(
                        &manager,
                        &thread_id,
                        &run_id,
                        "process_wait_failed",
                        &format!("process wait failed: {error}"),
                    ),
                }
                return;
            }
            _ = wait_for_cancel(&mut cancel) => {
                process_tree.terminate();
                abort_child_log(&mut stdout_log).await;
                abort_child_log(&mut stderr_log).await;
                terminate_child(&mut child).await;
                let _ = manager.with_run_entry(&thread_id, &run_id, |entry| {
                    entry.pid = None;
                    entry.ready = false;
                });
                return;
            }
            _ = probe_interval.tick() => {
                let probe = probe_preview_url(&healthcheck_url).await;
                apply_probe_result(
                    &manager,
                    &thread_id,
                    &run_id,
                    probe,
                    Instant::now() >= readiness_deadline,
                );
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandOutcome {
    Success,
    Cancelled,
}

async fn run_install_command(
    manager: Arc<PreviewRuntimeManager>,
    thread_id: &str,
    run_id: &str,
    dir: &Path,
    package: &PreviewPackage,
    cancel: &mut watch::Receiver<bool>,
) -> Result<CommandOutcome> {
    let label = package.install_label();
    let (program, args) = package.install_invocation().ok_or_else(|| {
        Error::InvalidRequest("preview install command is unavailable".to_string())
    })?;
    let _ = manager.with_run_entry(thread_id, run_id, |entry| {
        if entry.active {
            entry.status = "installing".to_string();
            entry.ready = false;
            entry.command = Some(label.clone());
            entry.message = Some(label.clone());
            push_log(entry, "system", &label);
        }
    });
    let mut child = preview_command(&program)
        .args(&args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut process_tree = match child.id().map(ProcessTreeGuard::attach) {
        Some(Ok(tree)) => tree,
        None => {
            let _ = child.start_kill();
            return Err(Error::Other("install process id was unavailable".into()));
        }
        Some(Err(error)) => {
            let _ = child.start_kill();
            return Err(error.into());
        }
    };
    let pid = child.id();
    let current = manager.with_run_entry(thread_id, run_id, |entry| {
        if entry.active {
            entry.pid = pid;
        }
    })?;
    if !current || !run_is_active(&manager, thread_id, run_id) {
        terminate_child(&mut child).await;
        return Ok(CommandOutcome::Cancelled);
    }
    let mut stdout_log = pipe_child_logs(
        manager.clone(),
        thread_id.to_string(),
        run_id.to_string(),
        child.stdout.take(),
        "stdout",
    );
    let mut stderr_log = pipe_child_logs(
        manager.clone(),
        thread_id.to_string(),
        run_id.to_string(),
        child.stderr.take(),
        "stderr",
    );
    let status = tokio::select! {
        status = child.wait() => Some(status?),
        _ = wait_for_cancel(cancel) => None,
    };
    if status.is_none() {
        process_tree.terminate();
        abort_child_log(&mut stdout_log).await;
        abort_child_log(&mut stderr_log).await;
        terminate_child(&mut child).await;
        let _ = manager.with_run_entry(thread_id, run_id, |entry| entry.pid = None);
        return Ok(CommandOutcome::Cancelled);
    }
    let status = status.expect("checked above");
    let _ = manager.with_run_entry(thread_id, run_id, |entry| entry.pid = None);
    if status.success() {
        Ok(CommandOutcome::Success)
    } else {
        Err(Error::Other(format!("{label} exited with {status}")))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

#[derive(Clone, Debug)]
struct PreviewPackage {
    manager: PackageManager,
    has_dev_script: bool,
    dev_script: String,
    explicit_port: Option<u16>,
    install_command: Option<Vec<String>>,
    dev_command: Option<Vec<String>>,
    url_path: String,
    healthcheck_path: String,
    configuration: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewManifest {
    version: u8,
    #[serde(default)]
    cwd: Option<String>,
    command: Vec<String>,
    #[serde(default)]
    install: Option<Vec<String>>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    url_path: Option<String>,
    #[serde(default)]
    healthcheck_path: Option<String>,
}

struct InspectedPreviewSource {
    package: PreviewPackage,
    dir: PathBuf,
    install_required: bool,
    source_fingerprint: String,
}

impl PackageManager {
    fn command_name(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        }
    }
}

impl PreviewPackage {
    fn default_install_args(&self) -> Vec<String> {
        match self.manager {
            PackageManager::Npm => vec!["install", "--no-audit", "--no-fund"],
            PackageManager::Pnpm | PackageManager::Yarn | PackageManager::Bun => vec!["install"],
        }
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn default_dev_args(&self, port: u16) -> Vec<String> {
        let port = port.to_string();
        let server_args = if self.is_next_dev() && self.explicit_port.is_some() {
            vec!["--hostname", "127.0.0.1"]
        } else if self.is_next_dev() {
            vec!["--hostname", "127.0.0.1", "--port", &port]
        } else if self.explicit_port.is_some() {
            vec!["--host", "127.0.0.1"]
        } else {
            vec!["--host", "127.0.0.1", "--port", &port]
        };
        let mut args = match self.manager {
            PackageManager::Yarn => vec!["run", "dev"],
            PackageManager::Npm | PackageManager::Pnpm | PackageManager::Bun => {
                vec!["run", "dev", "--"]
            }
        };
        args.extend(server_args);
        args.into_iter().map(str::to_string).collect()
    }

    fn install_label(&self) -> String {
        self.install_invocation()
            .map(|(program, args)| command_label(&program, &args))
            .unwrap_or_default()
    }

    fn dev_label(&self, port: u16) -> String {
        let (program, args) = self.dev_invocation(port);
        command_label(&program, &args)
    }

    fn launcher_name(&self) -> String {
        self.dev_command
            .as_ref()
            .and_then(|command| command.first())
            .cloned()
            .unwrap_or_else(|| self.manager.command_name().to_string())
    }

    fn install_invocation(&self) -> Option<(String, Vec<String>)> {
        if let Some(command) = self.install_command.as_ref() {
            return Some(command_invocation(command, 0));
        }
        (self.configuration == "package_json").then(|| {
            (
                self.manager.command_name().to_string(),
                self.default_install_args(),
            )
        })
    }

    fn dev_invocation(&self, port: u16) -> (String, Vec<String>) {
        self.dev_command
            .as_ref()
            .map(|command| command_invocation(command, port))
            .unwrap_or_else(|| {
                (
                    self.manager.command_name().to_string(),
                    self.default_dev_args(port),
                )
            })
    }

    fn is_next_dev(&self) -> bool {
        self.dev_script
            .split_whitespace()
            .any(|part| part == "next" || part.ends_with("/next"))
    }

    fn is_vite_dev(&self) -> bool {
        self.dev_script
            .split_whitespace()
            .any(|part| part == "vite" || part.ends_with("/vite"))
    }
}

fn command_invocation(command: &[String], port: u16) -> (String, Vec<String>) {
    let replace = |value: &str| value.replace("{port}", &port.to_string());
    (
        replace(command.first().expect("validated preview command")),
        command[1..].iter().map(|value| replace(value)).collect(),
    )
}

fn command_label(command: &str, args: &[String]) -> String {
    args.iter().fold(command.to_string(), |mut out, arg| {
        out.push(' ');
        out.push_str(arg);
        out
    })
}

fn preview_package(dir: &Path) -> Result<PreviewPackage> {
    let package_json = std::fs::read_to_string(dir.join("package.json"))?;
    let package: Value = serde_json::from_str(&package_json)?;
    let dev_script = package
        .get("scripts")
        .and_then(|scripts| scripts.get("dev"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let explicit_port = explicit_dev_script_port(&dev_script)?;
    Ok(PreviewPackage {
        manager: package_manager_for(dir, &package),
        has_dev_script: !dev_script.is_empty(),
        dev_script,
        explicit_port,
        install_command: None,
        dev_command: None,
        url_path: "/".to_string(),
        healthcheck_path: "/".to_string(),
        configuration: "package_json".to_string(),
    })
}

fn preview_package_from_files(files: &[PreviewAppFile]) -> Result<PreviewPackage> {
    let mut package_json = None;
    let mut normalized_paths = Vec::with_capacity(files.len());
    for file in files {
        let path = safe_relative_path(&file.path)?;
        if path == Path::new("package.json") {
            package_json = Some(file.content.as_str());
        }
        normalized_paths.push(path);
    }
    let package_json = package_json
        .ok_or_else(|| Error::InvalidRequest("preview app requires package.json".to_string()))?;
    let package: Value = serde_json::from_str(package_json)?;
    let dev_script = package
        .get("scripts")
        .and_then(|scripts| scripts.get("dev"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let manager = package
        .get("packageManager")
        .and_then(Value::as_str)
        .and_then(package_manager_from_text)
        .or_else(|| {
            normalized_paths
                .iter()
                .any(|path| path == Path::new("pnpm-lock.yaml"))
                .then_some(PackageManager::Pnpm)
        })
        .or_else(|| {
            normalized_paths
                .iter()
                .any(|path| path == Path::new("yarn.lock"))
                .then_some(PackageManager::Yarn)
        })
        .or_else(|| {
            normalized_paths
                .iter()
                .any(|path| path == Path::new("bun.lockb") || path == Path::new("bun.lock"))
                .then_some(PackageManager::Bun)
        })
        .unwrap_or(PackageManager::Npm);
    let explicit_port = explicit_dev_script_port(&dev_script)?;
    Ok(PreviewPackage {
        manager,
        has_dev_script: !dev_script.is_empty(),
        dev_script,
        explicit_port,
        install_command: None,
        dev_command: None,
        url_path: "/".to_string(),
        healthcheck_path: "/".to_string(),
        configuration: "package_json".to_string(),
    })
}

fn explicit_dev_script_port(dev_script: &str) -> Result<Option<u16>> {
    let parts = dev_script.split_whitespace().collect::<Vec<_>>();
    let is_next = parts.iter().any(|part| {
        let part = part.trim_matches(['\'', '"']);
        part == "next" || part.ends_with("/next")
    });
    let mut explicit_port = None;
    let mut index = 0;
    while index < parts.len() {
        let part = parts[index].trim_matches(['\'', '"']);
        let value = if part == "--port" || is_next && part == "-p" {
            index += 1;
            Some(parts.get(index).copied().ok_or_else(|| {
                Error::InvalidRequest(
                    "preview app scripts.dev has a port flag without a value".to_string(),
                )
            })?)
        } else if let Some(value) = part.strip_prefix("--port=") {
            Some(value)
        } else if is_next {
            part.strip_prefix("-p=")
        } else {
            None
        };
        let value = value.or_else(|| part.strip_prefix("PORT="));
        if let Some(value) = value {
            explicit_port = Some(parse_explicit_dev_port(value)?);
        }
        index += 1;
    }
    Ok(explicit_port)
}

fn parse_explicit_dev_port(value: &str) -> Result<u16> {
    value
        .trim_matches(['\'', '"'])
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            Error::InvalidRequest(format!(
                "preview app scripts.dev has invalid explicit port {value}"
            ))
        })
}

fn validate_preview_package(package: &PreviewPackage) -> Result<()> {
    if package.has_dev_script {
        Ok(())
    } else {
        Err(Error::InvalidRequest(
            "preview app package.json requires scripts.dev".to_string(),
        ))
    }
}

fn inspect_preview_source(
    target: &PreviewAppTarget,
    files: &[PreviewAppFile],
) -> Result<InspectedPreviewSource> {
    if target.managed && !files.is_empty() {
        let source_fingerprint = fingerprint_files(files)?;
        if let Some(manifest) = preview_manifest_from_files(files)? {
            let (package, dir) = preview_package_from_manifest(&target.dir, manifest, false)?;
            return Ok(InspectedPreviewSource {
                install_required: package.install_invocation().is_some(),
                package,
                dir,
                source_fingerprint,
            });
        }
        return Ok(InspectedPreviewSource {
            package: preview_package_from_files(files)?,
            dir: target.dir.clone(),
            install_required: true,
            source_fingerprint,
        });
    }
    let source_fingerprint = if target.managed {
        fingerprint_managed_dir(&target.dir)?
    } else {
        fingerprint_selected_dir(&target.dir)?
    };
    if let Some(manifest) = preview_manifest_from_dir(&target.dir)? {
        let (package, dir) = preview_package_from_manifest(&target.dir, manifest, true)?;
        let install_required = package.install_invocation().is_some()
            && needs_dependency_install(&dir, target.managed);
        return Ok(InspectedPreviewSource {
            package,
            dir,
            install_required,
            source_fingerprint,
        });
    }
    if !target.dir.join("package.json").is_file() {
        return Err(Error::InvalidRequest(format!(
            "preview app requires package.json or {PREVIEW_MANIFEST_FILE}"
        )));
    }
    let package = preview_package(&target.dir)?;
    Ok(InspectedPreviewSource {
        package,
        dir: target.dir.clone(),
        install_required: needs_dependency_install(&target.dir, target.managed),
        source_fingerprint,
    })
}

fn preview_manifest_from_dir(dir: &Path) -> Result<Option<PreviewManifest>> {
    let path = dir.join(PREVIEW_MANIFEST_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let source = std::fs::read_to_string(&path)?;
    parse_preview_manifest(&source).map(Some)
}

fn preview_manifest_from_files(files: &[PreviewAppFile]) -> Result<Option<PreviewManifest>> {
    for file in files {
        if safe_relative_path(&file.path)? == Path::new(PREVIEW_MANIFEST_FILE) {
            return parse_preview_manifest(&file.content).map(Some);
        }
    }
    Ok(None)
}

fn parse_preview_manifest(source: &str) -> Result<PreviewManifest> {
    let manifest: PreviewManifest = serde_json::from_str(source).map_err(|error| {
        Error::InvalidRequest(format!("invalid {PREVIEW_MANIFEST_FILE}: {error}"))
    })?;
    if manifest.version != 1 {
        return Err(Error::InvalidRequest(format!(
            "{PREVIEW_MANIFEST_FILE} version must be 1"
        )));
    }
    validate_manifest_command("command", &manifest.command)?;
    if let Some(install) = manifest.install.as_ref() {
        validate_manifest_command("install", install)?;
    }
    if manifest.port == Some(0) {
        return Err(Error::InvalidRequest(format!(
            "{PREVIEW_MANIFEST_FILE} port must be between 1 and 65535"
        )));
    }
    validate_preview_path("url_path", manifest.url_path.as_deref().unwrap_or("/"))?;
    validate_preview_path(
        "healthcheck_path",
        manifest
            .healthcheck_path
            .as_deref()
            .or(manifest.url_path.as_deref())
            .unwrap_or("/"),
    )?;
    Ok(manifest)
}

fn validate_manifest_command(field: &str, command: &[String]) -> Result<()> {
    if command.is_empty() || command[0].trim().is_empty() {
        return Err(Error::InvalidRequest(format!(
            "{PREVIEW_MANIFEST_FILE} {field} must be a non-empty argv array"
        )));
    }
    if command
        .iter()
        .any(|part| part.contains('\0') || part.contains('\r') || part.contains('\n'))
    {
        return Err(Error::InvalidRequest(format!(
            "{PREVIEW_MANIFEST_FILE} {field} contains an invalid control character"
        )));
    }
    Ok(())
}

fn validate_preview_path(field: &str, value: &str) -> Result<()> {
    if !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('\r')
        || value.contains('\n')
        || value.contains('#')
    {
        return Err(Error::InvalidRequest(format!(
            "{PREVIEW_MANIFEST_FILE} {field} must be a loopback URL path beginning with one /"
        )));
    }
    Ok(())
}

fn preview_package_from_manifest(
    root: &Path,
    manifest: PreviewManifest,
    require_existing_cwd: bool,
) -> Result<(PreviewPackage, PathBuf)> {
    let relative_cwd = match manifest.cwd.as_deref().map(str::trim) {
        None | Some("") | Some(".") => None,
        Some(value) => Some(safe_relative_path(value)?),
    };
    let joined = relative_cwd
        .as_deref()
        .map_or_else(|| root.to_path_buf(), |relative| root.join(relative));
    let dir = if require_existing_cwd {
        let canonical_root = std::fs::canonicalize(root).map_err(|error| {
            Error::InvalidRequest(format!("preview workspace is invalid: {error}"))
        })?;
        let canonical_dir = std::fs::canonicalize(&joined).map_err(|error| {
            Error::InvalidRequest(format!(
                "{PREVIEW_MANIFEST_FILE} cwd does not resolve to a directory: {error}"
            ))
        })?;
        if !canonical_dir.is_dir() || !canonical_dir.starts_with(&canonical_root) {
            return Err(Error::InvalidRequest(format!(
                "{PREVIEW_MANIFEST_FILE} cwd must stay inside the workspace"
            )));
        }
        canonical_dir
    } else {
        joined
    };
    let manager = package_manager_from_text(&manifest.command[0])
        .or_else(|| {
            manifest
                .install
                .as_ref()
                .and_then(|command| package_manager_from_text(&command[0]))
        })
        .unwrap_or(PackageManager::Npm);
    let url_path = manifest.url_path.unwrap_or_else(|| "/".to_string());
    let healthcheck_path = manifest
        .healthcheck_path
        .unwrap_or_else(|| url_path.clone());
    let dev_script = manifest.command.join(" ");
    Ok((
        PreviewPackage {
            manager,
            has_dev_script: true,
            dev_script,
            explicit_port: manifest.port,
            install_command: manifest.install,
            dev_command: Some(manifest.command),
            url_path,
            healthcheck_path,
            configuration: "manifest".to_string(),
        },
        dir,
    ))
}

fn preview_loopback_url(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

fn fingerprint_files(files: &[PreviewAppFile]) -> Result<String> {
    let mut normalized = Vec::with_capacity(files.len());
    let mut seen = HashSet::new();
    for file in files {
        let path = safe_relative_path(&file.path)?;
        let path = path.to_string_lossy().replace('\\', "/");
        if !seen.insert(path.clone()) {
            return Err(Error::InvalidRequest(format!(
                "duplicate preview app file path: {}",
                file.path
            )));
        }
        normalized.push((path, file.content.as_bytes().to_vec()));
    }
    normalized.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(fingerprint_parts(&normalized, &[]))
}

fn fingerprint_managed_dir(dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_managed_files(dir, dir, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(fingerprint_parts(&files, &[]))
}

fn collect_managed_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        let first = rel.components().next();
        if first.is_some_and(|part| {
            matches!(part, Component::Normal(name) if name == "node_modules" || name == ".git")
        }) || rel == Path::new(INSTALL_MARKER_FILE)
        {
            continue;
        }
        if path.is_dir() {
            collect_managed_files(root, &path, files)?;
        } else if path.is_file() {
            files.push((
                rel.to_string_lossy().replace('\\', "/"),
                std::fs::read(path)?,
            ));
        }
    }
    Ok(())
}

fn fingerprint_selected_dir(dir: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_selected_files(dir, dir, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hash = 0xcbf29ce484222325_u64;
    for (path, source) in files {
        fingerprint_update(&mut hash, path.as_bytes());
        if source.symlink_metadata()?.file_type().is_symlink() {
            let target = std::fs::read_link(source)?;
            fingerprint_update(&mut hash, target.to_string_lossy().as_bytes());
        } else {
            fingerprint_file(&mut hash, &source)?;
        }
    }
    fingerprint_update(
        &mut hash,
        if dir.join("node_modules").is_dir() {
            "node_modules=present"
        } else {
            "node_modules=missing"
        }
        .as_bytes(),
    );
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn collect_selected_files(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if selected_fingerprint_ignored(&name) {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_selected_files(root, &path, files)?;
            continue;
        }
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path);
        files.push((rel.to_string_lossy().replace('\\', "/"), path));
    }
    Ok(())
}

fn selected_fingerprint_ignored(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | ".next"
            | "dist"
            | "build"
            | "coverage"
            | "target"
            | ".cache"
            | ".turbo"
            | ".vite"
            | INSTALL_MARKER_FILE
    )
}

fn fingerprint_parts(files: &[(String, Vec<u8>)], extra: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for (path, content) in files {
        fingerprint_update(&mut hash, path.as_bytes());
        fingerprint_update(&mut hash, content);
    }
    for value in extra {
        fingerprint_update(&mut hash, value.as_bytes());
    }
    format!("fnv1a64:{hash:016x}")
}

fn fingerprint_update(hash: &mut u64, value: &[u8]) {
    fingerprint_update_raw(hash, &(value.len() as u64).to_le_bytes());
    fingerprint_update_raw(hash, value);
}

fn fingerprint_file(hash: &mut u64, path: &Path) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    fingerprint_update_raw(hash, &file.metadata()?.len().to_le_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        fingerprint_update_raw(hash, &buffer[..read]);
    }
}

fn fingerprint_update_raw(hash: &mut u64, value: &[u8]) {
    for byte in value {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn package_manager_for(dir: &Path, package: &Value) -> PackageManager {
    if let Some(manager) = package
        .get("packageManager")
        .and_then(Value::as_str)
        .and_then(package_manager_from_text)
    {
        return manager;
    }
    if dir.join("pnpm-lock.yaml").is_file() {
        return PackageManager::Pnpm;
    }
    if dir.join("yarn.lock").is_file() {
        return PackageManager::Yarn;
    }
    if dir.join("bun.lockb").is_file() || dir.join("bun.lock").is_file() {
        return PackageManager::Bun;
    }
    PackageManager::Npm
}

fn package_manager_from_text(value: &str) -> Option<PackageManager> {
    let name = value.split('@').next()?.trim().to_ascii_lowercase();
    match name.as_str() {
        "npm" => Some(PackageManager::Npm),
        "pnpm" => Some(PackageManager::Pnpm),
        "yarn" => Some(PackageManager::Yarn),
        "bun" => Some(PackageManager::Bun),
        _ => None,
    }
}

fn pipe_child_logs<T>(
    manager: Arc<PreviewRuntimeManager>,
    thread_id: String,
    run_id: String,
    pipe: Option<T>,
    stream: &'static str,
) -> Option<JoinHandle<()>>
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let pipe = pipe?;
    Some(tokio::spawn(async move {
        let mut lines = BufReader::new(pipe).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = manager.with_run_entry(&thread_id, &run_id, |entry| {
                push_child_log(entry, stream, &line)
            });
        }
    }))
}

async fn abort_child_log(task: &mut Option<JoinHandle<()>>) {
    if let Some(task) = task.take() {
        task.abort();
        let _ = task.await;
    }
}

fn push_child_log(entry: &mut PreviewAppEntry, stream: &str, line: &str) {
    if entry.active && entry.status != "stopping" && is_preview_compile_error(line) {
        let message = "Preview app compile error. Check logs.".to_string();
        entry.status = "error".to_string();
        entry.ready = false;
        entry.message = Some(message.clone());
        entry.error = Some(PreviewAppError {
            code: "compile_error".to_string(),
            message,
        });
        entry.compile_error_at = Some(Instant::now());
    }
    push_log(entry, stream, line);
}

fn apply_probe_result(
    manager: &PreviewRuntimeManager,
    thread_id: &str,
    run_id: &str,
    probe: Option<u16>,
    initial_deadline_elapsed: bool,
) {
    let _ = manager.with_run_entry(thread_id, run_id, |entry| {
        if !entry.active || entry.pid.is_none() || entry.status == "stopping" {
            return;
        }
        match probe {
            Some(code) if (200..400).contains(&code) => {
                if entry.compile_error_at.is_some_and(|at| {
                    at.elapsed() < Duration::from_millis(PREVIEW_COMPILE_ERROR_QUIET_MS)
                }) {
                    return;
                }
                let transitioned = !entry.ready || entry.status != "running";
                entry.status = "running".to_string();
                entry.ready = true;
                entry.message = Some("Running.".to_string());
                entry.error = None;
                entry.compile_error_at = None;
                if transitioned {
                    push_log(entry, "system", "preview app is running");
                }
            }
            Some(code) if code >= 400 => {
                let message = format!("Preview URL returned HTTP {code}. Check logs.");
                let changed = entry
                    .error
                    .as_ref()
                    .is_none_or(|error| error.code != "http_error" || error.message != message);
                entry.status = "error".to_string();
                entry.ready = false;
                entry.message = Some(message.clone());
                entry.error = Some(PreviewAppError {
                    code: "http_error".to_string(),
                    message,
                });
                if changed {
                    push_log(
                        entry,
                        "system",
                        &format!("preview URL returned HTTP {code}"),
                    );
                }
            }
            _ if initial_deadline_elapsed || entry.ready => {
                let message = "Preview URL did not become ready. Check logs.".to_string();
                let changed = entry
                    .error
                    .as_ref()
                    .is_none_or(|error| error.code != "preview_unavailable");
                entry.status = "error".to_string();
                entry.ready = false;
                entry.message = Some(message.clone());
                entry.error = Some(PreviewAppError {
                    code: "preview_unavailable".to_string(),
                    message,
                });
                if changed {
                    push_log(entry, "system", "preview URL did not become ready");
                }
            }
            _ => {}
        }
    });
}

fn fail_run(
    manager: &PreviewRuntimeManager,
    thread_id: &str,
    run_id: &str,
    code: &str,
    message: &str,
) {
    let _ = manager.with_run_entry(thread_id, run_id, |entry| {
        if !entry.active || entry.status == "stopping" {
            return;
        }
        entry.status = "error".to_string();
        entry.active = false;
        entry.ready = false;
        entry.pid = None;
        entry.message = Some(message.to_string());
        entry.error = Some(PreviewAppError {
            code: code.to_string(),
            message: message.to_string(),
        });
        entry.cancel = None;
        push_log(entry, "system", message);
    });
}

fn run_is_active(manager: &PreviewRuntimeManager, thread_id: &str, run_id: &str) -> bool {
    manager
        .entries
        .lock()
        .ok()
        .and_then(|entries| {
            entries.get(thread_id).map(|entry| {
                entry.active
                    && entry.status != "stopping"
                    && entry.run_id.as_deref() == Some(run_id)
            })
        })
        .unwrap_or(false)
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    while cancel.changed().await.is_ok() {
        if *cancel.borrow() {
            return;
        }
    }
    std::future::pending::<()>().await;
}

async fn terminate_child(child: &mut Child) {
    let pid = child.id();
    if let Some(pid) = pid {
        let _ = kill_process_tree(pid).await;
    }
    if tokio::time::timeout(
        Duration::from_millis(PREVIEW_PROCESS_STOP_TIMEOUT_MS),
        child.wait(),
    )
    .await
    .is_err()
    {
        if let Some(pid) = pid {
            let _ = force_kill_process_tree(pid).await;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

async fn probe_preview_url(url: &str) -> Option<u16> {
    let port = preview_url_port(url)?;
    let path = preview_url_request_path(url)?;
    let mut stream = tokio::time::timeout(
        Duration::from_millis(PREVIEW_READY_REQUEST_TIMEOUT_MS),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .ok()?
    .ok()?;
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    tokio::time::timeout(
        Duration::from_millis(PREVIEW_READY_REQUEST_TIMEOUT_MS),
        stream.write_all(request.as_bytes()),
    )
    .await
    .ok()?
    .ok()?;
    let mut response = [0_u8; 256];
    let len = tokio::time::timeout(
        Duration::from_millis(PREVIEW_READY_REQUEST_TIMEOUT_MS),
        stream.read(&mut response),
    )
    .await
    .ok()?
    .ok()?;
    let head = std::str::from_utf8(&response[..len]).ok()?;
    head.split_whitespace().nth(1)?.parse().ok()
}

fn preview_url_port(url: &str) -> Option<u16> {
    url.trim()
        .strip_prefix("http://127.0.0.1:")?
        .split(['/', '?', '#'])
        .next()?
        .parse()
        .ok()
}

fn preview_url_request_path(url: &str) -> Option<&str> {
    let rest = url.trim().strip_prefix("http://127.0.0.1:")?;
    let path_start = rest.find('/');
    Some(path_start.map_or("/", |index| &rest[index..]))
}

fn push_log(entry: &mut PreviewAppEntry, stream: &str, line: &str) {
    let line = strip_ansi_control_sequences(line);
    entry.next_log_seq = entry.next_log_seq.saturating_add(1);
    entry.logs.push_back(PreviewAppLog {
        seq: entry.next_log_seq,
        ts: crate::now_unix(),
        stream: stream.to_string(),
        line: line.into_owned(),
    });
    while entry.logs.len() > MAX_LOG_LINES {
        entry.logs.pop_front();
    }
}

fn is_preview_compile_error(line: &str) -> bool {
    let line = strip_ansi_control_sequences(line);
    line.contains("[vite] Pre-transform error")
        || line.contains("[vite] Internal server error")
        || line.contains("Failed to scan for dependencies from entries:")
        || line.contains("Unexpected closing")
        || line.contains("does not match opening")
        || line.contains("Plugin: vite:")
}

fn strip_ansi_control_sequences(value: &str) -> Cow<'_, str> {
    if !value.as_bytes().contains(&0x1b) {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for code in chars.by_ref() {
                    if ('@'..='~').contains(&code) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                let mut escaped = false;
                for code in chars.by_ref() {
                    if code == '\x07' || escaped && code == '\\' {
                        break;
                    }
                    escaped = code == '\x1b';
                }
            }
            Some('@'..='_') => {
                chars.next();
            }
            _ => {}
        }
    }
    Cow::Owned(out)
}

fn needs_dependency_install(dir: &Path, managed: bool) -> bool {
    !dir.join("node_modules").is_dir() || managed && !dir.join(INSTALL_MARKER_FILE).is_file()
}

fn port_is_available(port: u16) -> bool {
    for _ in 0..5 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn configured_preview_port_in_use_error(port: u16, configuration: &str) -> Error {
    let source = if configuration == "manifest" {
        PREVIEW_MANIFEST_FILE
    } else {
        "package.json scripts.dev"
    };
    Error::InvalidRequest(format!(
        "preview app configured port {port} from {source} is already in use; stop the process using it or change the preview configuration"
    ))
}

fn ensure_vite_setup(dir: &Path, package: &PreviewPackage) -> Result<Vec<String>> {
    if !package.is_vite_dev() {
        return Ok(Vec::new());
    }
    let mut logs = Vec::new();
    if let Some(log) = ensure_vite_entry(dir)? {
        logs.push(log);
    }
    if let Some(log) = ensure_tailwind_config(dir)? {
        logs.push(log);
    }
    Ok(logs)
}

fn ensure_vite_entry(dir: &Path) -> Result<Option<String>> {
    if dir.join("index.html").is_file() {
        return ensure_vite_index_styles(dir);
    }
    if let Some(html) = first_root_html(dir)? {
        std::fs::copy(dir.join(&html), dir.join("index.html"))?;
        return Ok(Some(format!(
            "created Vite index.html from {}",
            vite_path(&html)
        )));
    }
    if let Some(entry) = first_existing(
        dir,
        &["src/main.tsx", "src/main.jsx", "src/main.ts", "src/main.js"],
    ) {
        std::fs::write(
            dir.join("index.html"),
            vite_index_html(&entry, &vite_style_paths(dir)),
        )?;
        return Ok(Some(format!(
            "created Vite index.html for {}",
            vite_path(&entry)
        )));
    }
    if let Some(app) = first_existing(dir, &["src/App.tsx", "src/App.jsx", "App.tsx", "App.jsx"]) {
        let main = app.with_file_name(
            if app.extension().and_then(|ext| ext.to_str()) == Some("jsx") {
                "main.jsx"
            } else {
                "main.tsx"
            },
        );
        if !dir.join(&main).is_file() {
            if let Some(parent) = dir.join(&main).parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(dir.join(&main), react_main_source(&app))?;
        }
        std::fs::write(
            dir.join("index.html"),
            vite_index_html(&main, &vite_style_paths(dir)),
        )?;
        return Ok(Some(format!(
            "created Vite index.html and {}",
            vite_path(&main)
        )));
    }
    Ok(None)
}

fn ensure_tailwind_config(dir: &Path) -> Result<Option<String>> {
    if !vite_style_paths(dir)
        .iter()
        .any(|path| css_uses_tailwind(&dir.join(path)))
    {
        return Ok(None);
    }
    let mut changed = false;
    if !has_any_file(
        dir,
        &[
            "postcss.config.js",
            "postcss.config.cjs",
            "postcss.config.mjs",
            "postcss.config.ts",
        ],
    ) {
        std::fs::write(
            dir.join("postcss.config.cjs"),
            "module.exports = { plugins: { tailwindcss: {}, autoprefixer: {} } };\n",
        )?;
        changed = true;
    }
    if !has_any_file(
        dir,
        &[
            "tailwind.config.js",
            "tailwind.config.cjs",
            "tailwind.config.mjs",
            "tailwind.config.ts",
        ],
    ) {
        std::fs::write(
            dir.join("tailwind.config.cjs"),
            "module.exports = { content: ['./index.html', './src/**/*.{js,jsx,ts,tsx}'], theme: { extend: {} }, plugins: [] };\n",
        )?;
        changed = true;
    }
    Ok(changed.then(|| "created Tailwind preview config".to_string()))
}

fn first_root_html(dir: &Path) -> Result<Option<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("html"))
        {
            if let Some(name) = path.file_name() {
                files.push(PathBuf::from(name));
            }
        }
    }
    files.sort();
    Ok(files.into_iter().next())
}

fn first_existing(dir: &Path, paths: &[&str]) -> Option<PathBuf> {
    paths
        .iter()
        .map(PathBuf::from)
        .find(|path| dir.join(path).is_file())
}

fn has_any_file(dir: &Path, paths: &[&str]) -> bool {
    paths.iter().any(|path| dir.join(path).is_file())
}

fn css_uses_tailwind(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|css| css.contains("@tailwind") || css.contains("@apply"))
        .unwrap_or(false)
}

fn vite_style_paths(dir: &Path) -> Vec<PathBuf> {
    [
        "src/index.css",
        "src/main.css",
        "src/App.css",
        "src/style.css",
        "src/styles.css",
        "index.css",
        "style.css",
        "styles.css",
        "App.css",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| dir.join(path).is_file())
    .collect()
}

fn ensure_vite_index_styles(dir: &Path) -> Result<Option<String>> {
    let styles = vite_style_paths(dir);
    if styles.is_empty() {
        return Ok(None);
    }
    let index_path = dir.join("index.html");
    let index = std::fs::read_to_string(&index_path)?;
    if index.contains("rel=\"stylesheet\"") || index.contains("rel='stylesheet'") {
        return Ok(None);
    }
    let links = vite_style_links(&styles);
    let updated = if let Some(head) = find_ascii_case_insensitive(&index, "<head>") {
        let insert_at = head + "<head>".len();
        format!(
            "{}{}\n{}",
            &index[..insert_at],
            links.trim_end(),
            &index[insert_at..]
        )
    } else {
        format!("{links}{index}")
    };
    std::fs::write(index_path, updated)?;
    Ok(Some("added Vite CSS links to index.html".to_string()))
}

fn vite_index_html(entry: &Path, styles: &[PathBuf]) -> String {
    let links = vite_style_links(styles);
    format!(
        r#"{links}<div id="root"></div>
<script type="module" src="{}"></script>
"#,
        vite_path(entry)
    )
}

fn vite_style_links(styles: &[PathBuf]) -> String {
    let links = styles
        .iter()
        .map(|path| format!(r#"<link rel="stylesheet" href="{}">"#, vite_path(path)))
        .collect::<Vec<_>>()
        .join("\n");
    if links.is_empty() {
        String::new()
    } else {
        format!("{links}\n")
    }
}

fn react_main_source(app: &Path) -> String {
    let import_path = app
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| format!("./{stem}"))
        .unwrap_or_else(|| "./App".to_string());
    format!(
        r#"import React from "react";
import {{ createRoot }} from "react-dom/client";
import * as AppModule from "{}";

const App = AppModule.default ?? AppModule.App;
createRoot(document.getElementById("root")!).render(<App />);
"#,
        import_path
    )
}

fn vite_path(path: &Path) -> String {
    format!("/{}", path.to_string_lossy().replace('\\', "/"))
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

fn safe_thread_id(value: &str) -> Result<String> {
    let id = value.trim();
    if id.is_empty()
        || id == "."
        || id == ".."
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(Error::InvalidRequest(
            "invalid preview app thread id".to_string(),
        ));
    }
    Ok(id.to_string())
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(Error::InvalidRequest(
            "empty preview app file path".to_string(),
        ));
    }
    let path = Path::new(&normalized);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() || part.contains(':') {
                    return Err(Error::InvalidRequest(format!(
                        "unsafe preview app file path: {value}"
                    )));
                }
                out.push(part.as_ref());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidRequest(format!(
                    "unsafe preview app file path: {value}"
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::InvalidRequest(
            "empty preview app file path".to_string(),
        ));
    }
    Ok(out)
}

fn preview_command(name: &str) -> Command {
    #[cfg(windows)]
    let command = {
        let mut command = if name == "npm" {
            windows_npm_command().unwrap_or_else(|| {
                let mut command = Command::new("cmd.exe");
                command.args(["/D", "/S", "/C", "call", "npm.cmd"]);
                command
            })
        } else if name == "bun" {
            Command::new("bun.exe")
        } else {
            let mut command = Command::new("cmd.exe");
            command
                .args(["/D", "/S", "/C", "call"])
                .arg(format!("{name}.cmd"));
            command
        };
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let command = {
        let mut command = crate::cli_path::command(name);
        command.process_group(0);
        command
    };
    command
}

#[cfg(windows)]
fn windows_npm_command() -> Option<Command> {
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        let node = dir.join("node.exe");
        let cli = dir.join("node_modules/npm/bin/npm-cli.js");
        if !node.is_file() || !cli.is_file() {
            return None;
        }
        let mut command = Command::new(node);
        command.arg(cli);
        Some(command)
    })
}

#[cfg(windows)]
async fn kill_process_tree(pid: u32) -> Result<()> {
    let mut command = Command::new("taskkill");
    command
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let status = command.status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!("taskkill failed for pid {pid}")))
    }
}

#[cfg(not(windows))]
async fn kill_process_tree(pid: u32) -> Result<()> {
    let group = format!("-{pid}");
    let status = Command::new("kill")
        .args(["-TERM", "--", &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!("kill failed for pid {pid}")))
    }
}

#[cfg(windows)]
async fn force_kill_process_tree(pid: u32) -> Result<()> {
    kill_process_tree(pid).await
}

#[cfg(not(windows))]
async fn force_kill_process_tree(pid: u32) -> Result<()> {
    let group = format!("-{pid}");
    let status = Command::new("kill")
        .args(["-KILL", "--", &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "force kill failed for process group {pid}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn preview_command_resolves_unix_executable_to_absolute_path() {
        let command = preview_command("sh");
        assert!(Path::new(command.as_std().get_program()).is_absolute());
    }

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "milim-preview-app-test-{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    async fn npm_available() -> bool {
        preview_command("npm")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .is_ok_and(|status| status.success())
    }

    fn managed_node_files(preinstall: Option<&str>, server: &str) -> Vec<PreviewAppFile> {
        let scripts = match preinstall {
            Some(_) => r#"{"preinstall":"node preinstall.js","dev":"node server.js"}"#,
            None => r#"{"dev":"node server.js"}"#,
        };
        let mut files = vec![
            PreviewAppFile {
                path: "package.json".to_string(),
                content: format!(r#"{{"private":true,"scripts":{scripts}}}"#),
            },
            PreviewAppFile {
                path: "server.js".to_string(),
                content: server.to_string(),
            },
        ];
        if let Some(preinstall) = preinstall {
            files.push(PreviewAppFile {
                path: "preinstall.js".to_string(),
                content: preinstall.to_string(),
            });
        }
        files
    }

    #[test]
    fn static_preview_validates_entries_and_request_paths() {
        let root = test_root();
        let outside = test_root();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("index.html"), "<h1>Home</h1>").unwrap();
        std::fs::write(root.join("nested").join("page.htm"), "<h1>Nested</h1>").unwrap();
        std::fs::write(root.join("notes.txt"), "notes").unwrap();
        std::fs::write(outside.join("secret.html"), "secret").unwrap();
        let canonical_root = canonical_static_root(root.to_str().unwrap()).unwrap();

        assert!(canonical_static_entry(&canonical_root, Path::new("index.html")).is_ok());
        assert!(canonical_static_entry(&canonical_root, Path::new("nested/page.htm")).is_ok());
        assert!(canonical_static_entry(&canonical_root, Path::new("missing.html")).is_err());
        assert!(canonical_static_entry(&canonical_root, Path::new("notes.txt")).is_err());
        assert!(static_preview_request_target(&canonical_root, "/").is_some());
        assert!(static_preview_request_target(&canonical_root, "/nested/page.htm").is_some());
        assert!(static_preview_request_target(&canonical_root, "/../secret.html").is_none());
        assert!(static_preview_request_target(&canonical_root, "/%2e%2e/secret.html").is_none());

        let link = root.join("escaped.html");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(outside.join("secret.html"), &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(outside.join("secret.html"), &link).is_ok();
        if linked {
            assert!(canonical_static_entry(&canonical_root, Path::new("escaped.html")).is_err());
            assert!(static_preview_request_target(&canonical_root, "/escaped.html").is_none());
        }

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[tokio::test]
    async fn static_preview_serves_assets_reuses_folder_and_stops() {
        let root = test_root();
        let runtime_root = test_root();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(
            root.join("index.html"),
            r#"<link rel="stylesheet" href="/style.css"><script src="/app.js"></script><img src="/pixel.png">"#,
        )
        .unwrap();
        std::fs::write(root.join("nested").join("page.html"), "<h1>Nested</h1>").unwrap();
        std::fs::write(root.join("style.css"), "body { color: red; }").unwrap();
        std::fs::write(root.join("app.js"), "document.body.dataset.ready = 'yes';").unwrap();
        std::fs::write(root.join("pixel.png"), [137, 80, 78, 71]).unwrap();

        let manager = Arc::new(PreviewRuntimeManager::new(runtime_root.clone()));
        let status = manager
            .start_static(
                "thread-1",
                &PreviewStaticStartRequest {
                    cwd: root.to_string_lossy().to_string(),
                    entry_path: "index.html".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(status.kind, "static");
        assert_eq!(status.status, "running");
        assert!(status.active);
        assert!(status.ready);
        assert!(status.pid.is_none());
        assert!(status.command.is_none());
        assert!(status.preflight.is_none());

        let client = reqwest::Client::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        let html = loop {
            match client.get(status.url.as_ref().unwrap()).send().await {
                Ok(response) => break response,
                Err(error) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    let _ = error;
                }
                Err(error) => panic!("static preview did not start: {error}"),
            }
        };
        assert_eq!(html.status(), reqwest::StatusCode::OK);
        assert!(html
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html"));

        let base = status.url.as_ref().unwrap().rsplit_once('/').unwrap().0;
        for (path, mime) in [
            ("style.css", "text/css"),
            ("app.js", "text/javascript"),
            ("pixel.png", "image/png"),
        ] {
            let response = client.get(format!("{base}/{path}")).send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
            assert!(response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(mime));
        }

        let reused = manager
            .start_static(
                "thread-1",
                &PreviewStaticStartRequest {
                    cwd: root.to_string_lossy().to_string(),
                    entry_path: "nested/page.html".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            static_preview_port(status.url.as_ref().unwrap()),
            static_preview_port(reused.url.as_ref().unwrap())
        );
        assert!(reused.url.as_ref().unwrap().ends_with("/nested/page.html"));

        let stopped_url = reused.url.clone().unwrap();
        let stopped = manager.stop("thread-1").await.unwrap();
        assert_eq!(stopped.status, "stopped");
        assert!(!stopped.active);
        assert!(client.get(stopped_url).send().await.is_err());

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[tokio::test]
    async fn static_preview_rejects_an_active_app_runtime() {
        let root = test_root();
        let runtime_root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<h1>Home</h1>").unwrap();
        let manager = Arc::new(PreviewRuntimeManager::new(runtime_root.clone()));
        manager
            .set_entry("thread-1", |entry| {
                entry.kind = "app".to_string();
                entry.cwd = Some(root.clone());
                entry.status = "running".to_string();
                entry.active = true;
                entry.ready = true;
                entry.run_id = Some("app-run".to_string());
            })
            .unwrap();

        let result = manager
            .start_static(
                "thread-1",
                &PreviewStaticStartRequest {
                    cwd: root.to_string_lossy().to_string(),
                    entry_path: "index.html".to_string(),
                },
            )
            .await;
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        assert_eq!(manager.status("thread-1").unwrap().kind, "app");

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[test]
    fn preview_app_stage_rejects_unsafe_paths() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        let result = manager.stage(
            "thread-1",
            &[PreviewAppFile {
                path: "../package.json".to_string(),
                content: "{}".to_string(),
            }],
        );
        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_stage_writes_safe_relative_files() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        manager
            .stage(
                "thread-1",
                &[PreviewAppFile {
                    path: "src/App.tsx".to_string(),
                    content: "export function App() { return null; }".to_string(),
                }],
            )
            .unwrap();
        assert!(root.join("thread-1").join("src").join("App.tsx").is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_preflight_is_read_only_and_reports_exact_commands() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        let files = managed_node_files(
            Some("require('fs').writeFileSync('sentinel', 'ran')"),
            "setInterval(() => {}, 1000)",
        );
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    files,
                    ..Default::default()
                },
            )
            .unwrap();

        assert!(preflight.managed);
        assert_eq!(preflight.scope, "managed");
        assert_eq!(preflight.package_manager, "npm");
        assert!(preflight.install_required);
        assert_eq!(
            preflight.install_command,
            "npm install --no-audit --no-fund"
        );
        assert!(preflight.dev_command.contains(&preflight.port.to_string()));
        assert!(preflight.source_fingerprint.starts_with("fnv1a64:"));
        assert!(
            !root.exists(),
            "preflight must not stage files or run scripts"
        );
        let status = manager.status("thread-1").unwrap();
        assert_eq!(status.status, "idle");
        assert!(!status.active);
        assert_eq!(status.preflight, Some(preflight));
    }

    #[test]
    fn preview_app_preflight_honors_explicit_dev_script_port() {
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        let port = free_port().unwrap();
        std::fs::write(
            root.join("package.json"),
            format!(r#"{{"private":true,"scripts":{{"dev":"vite --port {port}"}}}}"#),
        )
        .unwrap();
        let runtime_root = test_root();
        let manager = PreviewRuntimeManager::new(runtime_root.clone());
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    cwd: Some(root.to_string_lossy().to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(preflight.port, port);
        assert_eq!(preflight.url, format!("http://127.0.0.1:{port}/"));
        assert!(!preflight.dev_command.contains("--port"));
        assert!(preflight.dev_command.contains("--host 127.0.0.1"));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[test]
    fn preview_app_preflight_prefers_repository_manifest() {
        let root = test_root();
        std::fs::create_dir_all(root.join(".milim")).unwrap();
        std::fs::create_dir_all(root.join("apps").join("site")).unwrap();
        let port = free_port().unwrap();
        std::fs::write(
            root.join(".milim").join("preview.json"),
            format!(
                r#"{{"version":1,"cwd":"apps/site","command":["pnpm","run","preview","--","--port","{{port}}"],"port":{port},"url_path":"/demo","healthcheck_path":"/health"}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"private":true,"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let runtime_root = test_root();
        let manager = PreviewRuntimeManager::new(runtime_root.clone());
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    cwd: Some(root.to_string_lossy().to_string()),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(preflight.configuration, "manifest");
        assert_eq!(
            preflight.cwd,
            std::fs::canonicalize(root.join("apps/site"))
                .unwrap()
                .to_string_lossy()
        );
        assert_eq!(
            preflight.dev_command,
            format!("pnpm run preview -- --port {port}")
        );
        assert!(!preflight.install_required);
        assert_eq!(preflight.install_command, "");
        assert_eq!(preflight.url, format!("http://127.0.0.1:{port}/demo"));
        assert_eq!(
            preflight.healthcheck_url,
            format!("http://127.0.0.1:{port}/health")
        );
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[test]
    fn preview_app_manifest_rejects_workspace_escape_and_shell_text() {
        let escaped =
            parse_preview_manifest(r#"{"version":1,"cwd":"../outside","command":["pnpm","dev"]}"#)
                .unwrap();
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        assert!(preview_package_from_manifest(&root, escaped, false).is_err());

        let shell_text =
            parse_preview_manifest(r#"{"version":1,"command":"pnpm dev && echo unsafe"}"#)
                .unwrap_err()
                .to_string();
        assert!(shell_text.contains("invalid .milim/preview.json"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_preflight_rejects_busy_explicit_dev_script_port() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            format!(r#"{{"private":true,"scripts":{{"dev":"vite --port={port}"}}}}"#),
        )
        .unwrap();
        let runtime_root = test_root();
        let manager = PreviewRuntimeManager::new(runtime_root.clone());
        let error = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    cwd: Some(root.to_string_lossy().to_string()),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains(&format!("configured port {port}")));
        assert!(error.contains("already in use"));
        drop(listener);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[test]
    fn preview_app_start_rejects_stale_managed_files_before_staging() {
        let root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(root.clone()));
        let files = managed_node_files(None, "setInterval(() => {}, 1000)");
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    files: files.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
        let mut changed = files;
        changed[1].content = "console.log('changed'); setInterval(() => {}, 1000)".to_string();
        let result = manager.start(
            "thread-1",
            &PreviewAppStartRequest {
                files: changed,
                source_fingerprint: Some(preflight.source_fingerprint),
                ..Default::default()
            },
        );

        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        assert!(!root.join("thread-1").exists());
        assert!(!manager.status("thread-1").unwrap().active);
    }

    #[test]
    fn preview_app_start_rejects_selected_folder_source_change() {
        let root = test_root();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("server.js"), "setInterval(() => {}, 1000)").unwrap();
        std::fs::write(root.join("src").join("app.js"), "export const value = 1").unwrap();
        let manager = Arc::new(PreviewRuntimeManager::new(test_root()));
        let cwd = root.to_string_lossy().to_string();
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    cwd: Some(cwd.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        std::fs::write(root.join("src").join("app.js"), "export const value = 2").unwrap();

        let result = manager.start(
            "thread-1",
            &PreviewAppStartRequest {
                cwd: Some(cwd),
                source_fingerprint: Some(preflight.source_fingerprint),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        assert!(!manager.status("thread-1").unwrap().active);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_fingerprints_large_selected_folder_without_buffering() {
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        let large = std::fs::File::create(root.join("large.bin")).unwrap();
        large.set_len(64 * 1024 * 1024 + 1).unwrap();

        assert!(fingerprint_selected_dir(&root)
            .unwrap()
            .starts_with("fnv1a64:"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_preflight_requires_package_json() {
        let root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(root.clone()));
        manager
            .stage(
                "thread-1",
                &[PreviewAppFile {
                    path: "src/App.tsx".to_string(),
                    content: "export function App() { return null; }".to_string(),
                }],
            )
            .unwrap();
        let result = manager.preflight("thread-1", &PreviewAppPreflightRequest::default());
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_preflight_requires_dev_script() {
        let root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(root.clone()));
        manager
            .stage(
                "thread-1",
                &[PreviewAppFile {
                    path: "package.json".to_string(),
                    content: r#"{"scripts":{"build":"vite build"}}"#.to_string(),
                }],
            )
            .unwrap();
        let result = manager.preflight("thread-1", &PreviewAppPreflightRequest::default());
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_detects_package_manager() {
        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        let package = serde_json::json!({"packageManager":"pnpm@9.0.0","scripts":{"dev":"vite"}});
        assert_eq!(package_manager_for(&root, &package), PackageManager::Pnpm);

        let package = serde_json::json!({"scripts":{"dev":"vite"}});
        std::fs::write(root.join("yarn.lock"), "").unwrap();
        assert_eq!(package_manager_for(&root, &package), PackageManager::Yarn);
        std::fs::remove_file(root.join("yarn.lock")).unwrap();

        std::fs::write(root.join("bun.lockb"), "").unwrap();
        assert_eq!(package_manager_for(&root, &package), PackageManager::Bun);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_reinstalls_incomplete_node_modules() {
        let root = test_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        assert!(needs_dependency_install(&root, true));
        assert!(!needs_dependency_install(&root, false));

        std::fs::write(root.join(INSTALL_MARKER_FILE), "ok").unwrap();
        assert!(!needs_dependency_install(&root, true));
        assert!(!needs_dependency_install(&root, false));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_project_folder_does_not_need_install_marker() {
        let root = test_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        assert!(!needs_dependency_install(&root, false));
        assert!(needs_dependency_install(&root, true));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_strips_ansi_from_logs() {
        let mut entry = PreviewAppEntry::default();
        push_log(
            &mut entry,
            "stdout",
            "\x1b[32m\u{279c}\x1b[39m  \x1b[1mLocal\x1b[22m: \x1b[36mhttp://127.0.0.1:59993/\x1b[39m",
        );
        assert_eq!(
            entry.logs.back().unwrap().line,
            "\u{279c}  Local: http://127.0.0.1:59993/"
        );
    }

    #[test]
    fn preview_app_logs_use_monotonic_cursor_and_report_truncation() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        manager
            .set_entry("thread-1", |entry| {
                for index in 0..(MAX_LOG_LINES + 3) {
                    push_log(entry, "stdout", &format!("line {index}"));
                }
            })
            .unwrap();

        let response = manager.logs_after("thread-1", Some(1)).unwrap();
        assert_eq!(response.logs.len(), MAX_LOG_LINES);
        assert!(response.truncated);
        assert_eq!(response.logs.first().unwrap().seq, 4);
        assert_eq!(response.next_seq, (MAX_LOG_LINES + 3) as u64);

        let tail = manager
            .logs_after("thread-1", Some(response.next_seq - 1))
            .unwrap();
        assert_eq!(tail.logs.len(), 1);
        assert!(!tail.truncated);
        assert_eq!(tail.logs[0].seq, response.next_seq);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_stale_run_cannot_overwrite_current_state() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        manager
            .set_entry("thread-1", |entry| {
                entry.status = "running".to_string();
                entry.active = true;
                entry.ready = true;
                entry.run_id = Some("new-run".to_string());
            })
            .unwrap();

        let updated = manager
            .with_run_entry("thread-1", "old-run", |entry| {
                entry.status = "error".to_string();
                entry.ready = false;
            })
            .unwrap();
        assert!(!updated);
        let status = manager.status("thread-1").unwrap();
        assert_eq!(status.status, "running");
        assert!(status.ready);
        assert_eq!(status.run_id.as_deref(), Some("new-run"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_marks_vite_compile_errors() {
        let mut entry = PreviewAppEntry {
            status: "running".to_string(),
            active: true,
            ready: true,
            pid: Some(123),
            ..Default::default()
        };
        push_child_log(
            &mut entry,
            "stderr",
            "5:10:14 PM [vite] Pre-transform error: src/App.tsx: Expected corresponding JSX closing tag for <svg>.",
        );
        assert_eq!(entry.status, "error");
        assert_eq!(
            entry.message.as_deref(),
            Some("Preview app compile error. Check logs.")
        );
        assert_eq!(entry.pid, Some(123));
    }

    #[tokio::test]
    async fn preview_app_compile_error_recovers_after_quiet_healthy_probe() {
        let root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(root.clone()));
        manager
            .set_entry("thread-1", |entry| {
                entry.status = "running".to_string();
                entry.active = true;
                entry.ready = true;
                entry.pid = Some(123);
                entry.run_id = Some("run-1".to_string());
            })
            .unwrap();
        manager
            .set_entry("thread-1", |entry| {
                push_child_log(
                    entry,
                    "stderr",
                    "5:24:38 PM [vite] Pre-transform error: src/App.tsx: Expected corresponding JSX closing tag for <svg>.",
                );
            })
            .unwrap();
        apply_probe_result(&manager, "thread-1", "run-1", Some(200), true);
        assert_eq!(manager.status("thread-1").unwrap().status, "error");
        tokio::time::sleep(Duration::from_millis(PREVIEW_COMPILE_ERROR_QUIET_MS + 5)).await;
        apply_probe_result(&manager, "thread-1", "run-1", Some(200), true);
        let status = manager.status("thread-1").unwrap();
        assert_eq!(status.status, "running");
        assert!(status.active);
        assert!(status.ready);
        assert!(status.error.is_none());
        assert!(status
            .logs
            .iter()
            .any(|log| log.line == "preview app is running"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn preview_app_stop_cancels_slow_install_before_dev_server() {
        if !npm_available().await {
            return;
        }
        let root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(root.clone()));
        let files = managed_node_files(
            Some(
                "const fs = require('fs'); fs.writeFileSync('install-started', 'yes'); setInterval(() => {}, 1000);",
            ),
            "require('fs').writeFileSync('dev-started', 'yes'); setInterval(() => {}, 1000);",
        );
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    files: files.clone(),
                    ..Default::default()
                },
            )
            .unwrap();
        manager
            .start(
                "thread-1",
                &PreviewAppStartRequest {
                    files,
                    source_fingerprint: Some(preflight.source_fingerprint),
                    ..Default::default()
                },
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while manager.status("thread-1").unwrap().pid.is_none() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            manager.status("thread-1").unwrap().pid.is_some(),
            "npm install process did not start; logs: {:?}",
            manager.logs("thread-1").unwrap()
        );
        let stop_started = Instant::now();
        let status = manager.stop("thread-1").await.unwrap();
        assert!(stop_started.elapsed() < Duration::from_secs(5));
        assert_eq!(status.status, "stopped");
        assert!(!status.active);
        assert!(!status.ready);
        assert!(status.pid.is_none());
        assert!(!root.join("thread-1").join("dev-started").exists());
        assert!(!root.join("thread-1").join(INSTALL_MARKER_FILE).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn preview_app_cancelled_phase_boundary_never_spawns_dev_server() {
        let root = test_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("server.js"),
            "require('fs').writeFileSync('dev-started', 'yes')",
        )
        .unwrap();
        let manager = Arc::new(PreviewRuntimeManager::new(test_root()));
        manager
            .set_entry("thread-1", |entry| {
                entry.cwd = Some(root.clone());
                entry.status = "installing".to_string();
                entry.active = true;
                entry.run_id = Some("run-1".to_string());
            })
            .unwrap();
        let (cancel, cancel_rx) = watch::channel(false);
        cancel.send(true).unwrap();
        run_preview_app(
            manager.clone(),
            PreviewRun {
                thread_id: "thread-1".to_string(),
                run_id: "run-1".to_string(),
                dir: root.clone(),
                port: free_port().unwrap(),
                healthcheck_url: "http://127.0.0.1:1/".to_string(),
                package: preview_package(&root).unwrap(),
                managed: false,
                files: Vec::new(),
                install_required: false,
            },
            cancel_rx,
        )
        .await;

        assert!(!root.join("dev-started").exists());
        assert!(manager.status("thread-1").unwrap().pid.is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn account_runtime_preview_outlives_its_tool_session() {
        if !npm_available().await {
            return;
        }
        let root = test_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("server.js"),
            r#"const http = require('http');
const index = process.argv.indexOf('--port');
const port = Number(process.argv[index + 1]);
http.createServer((_request, response) => response.end('ok')).listen(port, '127.0.0.1');
"#,
        )
        .unwrap();
        let runtime_root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(runtime_root.clone()));
        let tools = account_runtime_preview_tools(
            manager.clone(),
            "project-preview".to_string(),
            root.clone(),
        );
        let prepared = tools[0].invoke(serde_json::json!({})).await.unwrap();
        let fingerprint = prepared["source_fingerprint"].as_str().unwrap();
        let started = tools[1]
            .invoke(serde_json::json!({ "source_fingerprint": fingerprint }))
            .await
            .unwrap();
        assert_eq!(started["active"], true);
        drop(tools);

        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            let status = manager.status("project-preview").unwrap();
            if status.ready || !status.active || Instant::now() >= deadline {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(
            status.active,
            "server exited with its tool session: {:?}",
            status.logs
        );
        assert!(status.ready, "server never became ready: {:?}", status.logs);

        let next_turn_tools = account_runtime_preview_tools(
            manager.clone(),
            "project-preview".to_string(),
            root.clone(),
        );
        let reused = next_turn_tools[0]
            .invoke(serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(reused["active"], true);
        assert_eq!(reused["ready"], true);
        let wrong_project = account_runtime_preview_tools(
            manager.clone(),
            "project-preview".to_string(),
            runtime_root.clone(),
        );
        assert!(wrong_project[0]
            .invoke(serde_json::json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("another workspace"));

        manager.stop_all().await.unwrap();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[tokio::test]
    async fn preview_app_generic_http_server_becomes_ready_and_stop_all_cleans_up() {
        if !npm_available().await {
            return;
        }
        let root = test_root();
        std::fs::create_dir_all(root.join("node_modules")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"private":true,"scripts":{"dev":"node server.js"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("server.js"),
            r#"const http = require('http');
const index = process.argv.indexOf('--port');
const port = Number(process.argv[index + 1]);
const started = Date.now();
http.createServer((_request, response) => {
  response.statusCode = Date.now() - started < 250 ? 500 : 200;
  response.end('ok');
}).listen(port, '127.0.0.1');
"#,
        )
        .unwrap();
        let runtime_root = test_root();
        let manager = Arc::new(PreviewRuntimeManager::new(runtime_root.clone()));
        let cwd = root.to_string_lossy().to_string();
        let preflight = manager
            .preflight(
                "thread-1",
                &PreviewAppPreflightRequest {
                    cwd: Some(cwd.clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!preflight.managed);
        assert!(!preflight.install_required);
        manager
            .start(
                "thread-1",
                &PreviewAppStartRequest {
                    cwd: Some(cwd),
                    source_fingerprint: Some(preflight.source_fingerprint),
                    ..Default::default()
                },
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_active_error = false;
        let status = loop {
            let status = manager.status("thread-1").unwrap();
            saw_active_error |= status.active && status.status == "error" && status.url.is_some();
            if status.ready || !status.active || Instant::now() >= deadline {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert!(status.active, "server exited; logs: {:?}", status.logs);
        assert!(
            status.ready,
            "server never became ready; logs: {:?}",
            status.logs
        );
        assert_eq!(status.status, "running");
        assert!(status.url.is_some());
        assert!(saw_active_error, "active unhealthy state was not published");
        assert!(
            !status.logs.iter().any(|log| log.line.contains("Local:")),
            "readiness must not depend on Vite console output"
        );

        manager.stop_all().await.unwrap();
        let stopped = manager.status("thread-1").unwrap();
        assert_eq!(stopped.status, "stopped");
        assert!(!stopped.active);
        assert!(stopped.pid.is_none());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(runtime_root);
    }

    #[test]
    fn preview_app_adds_missing_vite_index() {
        let package = PreviewPackage {
            manager: PackageManager::Npm,
            has_dev_script: true,
            dev_script: "vite".to_string(),
            explicit_port: None,
            install_command: None,
            dev_command: None,
            url_path: "/".to_string(),
            healthcheck_path: "/".to_string(),
            configuration: "package_json".to_string(),
        };

        let root = test_root();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src").join("main.tsx"), "render()").unwrap();
        std::fs::write(
            root.join("src").join("index.css"),
            "@tailwind base; .glass { @apply bg-white/5; }",
        )
        .unwrap();
        assert!(!ensure_vite_setup(&root, &package).unwrap().is_empty());
        let index = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(index.contains("/src/main.tsx"));
        assert!(index.contains("/src/index.css"));
        assert!(root.join("postcss.config.cjs").is_file());
        assert!(root.join("tailwind.config.cjs").is_file());
        let _ = std::fs::remove_dir_all(root);

        let root = test_root();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("App.tsx"),
            "export default function App() { return null; }",
        )
        .unwrap();
        std::fs::write(root.join("styles.css"), "body { color: blue; }").unwrap();
        assert!(!ensure_vite_setup(&root, &package).unwrap().is_empty());
        assert!(root.join("src").join("main.tsx").is_file());
        let index = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(index.contains("/src/main.tsx"));
        assert!(index.contains("/styles.css"));
        let _ = std::fs::remove_dir_all(root);

        let root = test_root();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("index.html"),
            r#"<!DOCTYPE html>
<html><head></head><body><div id="root"></div><script type="module" src="/src/main.tsx"></script></body></html>
"#,
        )
        .unwrap();
        std::fs::write(root.join("style.css"), "body { color: green; }").unwrap();
        assert!(!ensure_vite_setup(&root, &package).unwrap().is_empty());
        let index = std::fs::read_to_string(root.join("index.html")).unwrap();
        assert!(index.starts_with("<!DOCTYPE html>"));
        assert!(index.contains("<head><link rel=\"stylesheet\" href=\"/style.css\">"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preview_app_uses_next_dev_flags() {
        let package = PreviewPackage {
            manager: PackageManager::Npm,
            has_dev_script: true,
            dev_script: "next dev".to_string(),
            explicit_port: None,
            install_command: None,
            dev_command: None,
            url_path: "/".to_string(),
            healthcheck_path: "/".to_string(),
            configuration: "package_json".to_string(),
        };
        let (_, args) = package.dev_invocation(3000);
        assert!(args.contains(&"--hostname".to_string()));
        assert!(!args.contains(&"--host".to_string()));
    }

    #[test]
    fn preview_app_detects_explicit_dev_script_ports_without_appending_another() {
        assert_eq!(
            explicit_dev_script_port("vite --port 4173").unwrap(),
            Some(4173)
        );
        assert_eq!(
            explicit_dev_script_port("vite --port=4174").unwrap(),
            Some(4174)
        );
        assert_eq!(
            explicit_dev_script_port("next dev -p 3001").unwrap(),
            Some(3001)
        );
        assert_eq!(
            explicit_dev_script_port("cross-env PORT=3100 vite").unwrap(),
            Some(3100)
        );
        assert_eq!(
            explicit_dev_script_port("node server.js -p 3101").unwrap(),
            None
        );
        assert!(explicit_dev_script_port("vite --port").is_err());
        assert!(explicit_dev_script_port("vite --port 0").is_err());

        let package = PreviewPackage {
            manager: PackageManager::Npm,
            has_dev_script: true,
            dev_script: "vite --port 4173".to_string(),
            explicit_port: Some(4173),
            install_command: None,
            dev_command: None,
            url_path: "/".to_string(),
            healthcheck_path: "/".to_string(),
            configuration: "package_json".to_string(),
        };
        let (_, args) = package.dev_invocation(4173);
        assert!(args.contains(&"--host".to_string()));
        assert!(!args.contains(&"--port".to_string()));
        assert!(!args.contains(&"4173".to_string()));
    }

    #[test]
    fn preview_app_stage_rejects_running_runtime() {
        let root = test_root();
        let manager = PreviewRuntimeManager::new(root.clone());
        manager
            .set_entry("thread-1", |entry| {
                entry.status = "running".to_string();
                entry.active = true;
                entry.pid = Some(123);
            })
            .unwrap();
        let result = manager.stage(
            "thread-1",
            &[PreviewAppFile {
                path: "package.json".to_string(),
                content: "{}".to_string(),
            }],
        );
        assert!(matches!(result, Err(Error::InvalidRequest(_))));

        manager
            .set_entry("thread-2", |entry| {
                entry.status = "error".to_string();
                entry.active = true;
                entry.pid = Some(456);
            })
            .unwrap();
        let result = manager.stage(
            "thread-2",
            &[PreviewAppFile {
                path: "package.json".to_string(),
                content: "{}".to_string(),
            }],
        );
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        let _ = std::fs::remove_dir_all(root);
    }
}
