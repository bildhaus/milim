//! Local backup format, validation and restore orchestration.
use super::{DesktopServerRuntime, UserDataState, APPEARANCE_STATE_KEY};
use milim_core::paths::Paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const BACKUP_SCHEMA_VERSION: u32 = 1;
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
pub(crate) struct BackupInspection {
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
    let (sessions_json, raw_entries, control) = store
        .complete_backup_snapshot(BACKUP_STATE_KEYS)
        .map_err(|error| error.to_string())?;
    let sessions_json = sessions_json.unwrap_or_else(|| {
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
    let entries = raw_entries
        .into_iter()
        .map(|(key, raw)| {
            serde_json::from_str(&raw)
                .map(|value| (key, value))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<BTreeMap<String, Value>, String>>()?;
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
    let file = File::open(path).map_err(|error| error.to_string())?;
    let backup: MilimBackup = serde_json::from_reader(BufReader::new(file))
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
    let entries = serialized_entries(&backup.state.entries)?;
    milim_storage::UserDataStore::validate_backup_state(
        &entries,
        &backup.state.sessions.to_string(),
        backup.state.control.as_ref(),
    )
    .map_err(|error| error.to_string())?;
    Ok((backup, metadata.len()))
}

#[tauri::command]
pub(crate) async fn export_milim_backup(
    state: tauri::State<'_, UserDataState>,
    path: String,
) -> std::result::Result<BackupInspection, String> {
    let store = state.0.clone();
    tokio::task::spawn_blocking(move || {
        let backup = build_backup(&store)?;
        let bytes = write_backup(Path::new(&path), &backup)?;
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
pub(crate) async fn inspect_milim_backup(
    path: String,
) -> std::result::Result<BackupInspection, String> {
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
pub(crate) async fn restore_milim_backup(
    state: tauri::State<'_, UserDataState>,
    runtime: tauri::State<'_, DesktopServerRuntime>,
    path: String,
) -> std::result::Result<String, String> {
    let control = runtime
        .0
        .control
        .as_ref()
        .ok_or("Canonical control runtime is unavailable.")?
        .clone();
    let admission = control.begin_restore().map_err(|error| error.to_string())?;
    let store = state.0.clone();
    let recovery_path =
        tokio::task::spawn_blocking(move || -> std::result::Result<String, String> {
            let (backup, _) = read_backup(Path::new(&path))?;
            let recovery = build_backup(&store)?;
            let data_path = Paths::resolve().user_db_file();
            let recovery_path = data_path.parent().unwrap_or(Path::new(".")).join(format!(
                "pre-restore-{}.milim-backup.json",
                now_timestamp_ms()
            ));
            write_backup(&recovery_path, &recovery)?;
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
                .replace_complete_backup_state(&replace_keys, entries, &sessions, control.as_ref())
                .map_err(|error| error.to_string())?;
            admission.commit();
            Ok(recovery_path.to_string_lossy().to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
    Ok(recovery_path)
}

fn serialized_entries(
    entries: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    entries
        .iter()
        .map(|(key, value)| {
            serde_json::to_string(value)
                .map(|value| (key.clone(), value))
                .map_err(|error| error.to_string())
        })
        .collect()
}

/// Stream to a same-directory temporary file; both exports and recovery snapshots
/// use the exact JSON format the streaming reader accepts, regardless of size.
fn write_backup(path: &Path, backup: &MilimBackup) -> Result<u64, String> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let (temp_path, file) = loop {
        let candidate = parent.join(format!(
            ".milim-backup-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    };
    let result = (|| {
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, backup).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| error.to_string())?;
        let bytes = writer
            .get_ref()
            .metadata()
            .map_err(|error| error.to_string())?
            .len();
        drop(writer);
        fs::rename(&temp_path, path).map_err(|error| error.to_string())?;
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_backup() -> MilimBackup {
        MilimBackup {
            schema_version: 1,
            app_version: "test".into(),
            created_at: 0,
            summary: MilimBackupSummary {
                chats: 0,
                projects: 0,
                state_keys: 0,
                control_records: 0,
            },
            state: MilimBackupState {
                sessions: serde_json::json!({"state":{"sessions":[]},"version":0}),
                entries: BTreeMap::new(),
                control: None,
            },
        }
    }

    #[test]
    fn large_export_and_recovery_json_remain_readable_and_restorable() {
        let path = std::env::temp_dir().join(format!(
            "milim-large-backup-test-{}-{}.json",
            std::process::id(),
            now_timestamp_ms()
        ));
        let mut backup = empty_backup();
        backup.state.entries.insert(
            "milim.settings".into(),
            Value::String("x".repeat(65 * 1024 * 1024)),
        );
        let bytes = write_backup(&path, &backup).unwrap();
        assert!(bytes > 64 * 1024 * 1024);
        drop(backup);
        let (restored, read_bytes) = read_backup(&path).unwrap();
        assert_eq!(read_bytes, bytes);
        let store =
            milim_storage::UserDataStore::new(milim_storage::Database::open_in_memory().unwrap())
                .unwrap();
        store
            .replace_complete_backup_state(
                &["milim.settings".into()],
                serialized_entries(&restored.state.entries).unwrap(),
                &restored.state.sessions.to_string(),
                None,
            )
            .unwrap();
        assert_eq!(
            store.get_json("milim.settings").unwrap().unwrap().len(),
            65 * 1024 * 1024 + 2
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn legacy_json_is_supported_and_nested_control_is_validated_before_restore() {
        let path = std::env::temp_dir().join(format!(
            "milim-backup-test-{}-{}.json",
            std::process::id(),
            now_timestamp_ms()
        ));
        let mut backup = empty_backup();
        // Legacy files used pretty-printed JSON and omitted the control field.
        fs::write(&path, serde_json::to_vec_pretty(&backup).unwrap()).unwrap();
        assert!(read_backup(&path).is_ok());
        backup.state.control = Some(milim_storage::ControlBackupState {
            schema_version: 999,
            ..Default::default()
        });
        write_backup(&path, &backup).unwrap();
        assert!(read_backup(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
