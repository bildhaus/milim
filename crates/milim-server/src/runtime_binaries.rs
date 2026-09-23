//! Per-runtime executable overrides ("Locate binary...").
//!
//! Automatic discovery cannot see every install: version managers such as
//! nvm or fnm only publish their bin directory from interactive shell
//! startup, and some installs live in arbitrary folders. A user-chosen
//! absolute path for Codex, Claude, OpenCode, or Pi wins over discovery for
//! every spawn of that runtime (turns, status, discovery, and updates).
//!
//! Overrides are persisted in the canonical store beside account profiles and
//! mirrored into a process-wide table so the synchronous command builders can
//! consult them without a store handle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use milim_core::{Error, Result};
use milim_storage::UserDataStore;
use tokio::process::Command;

/// Persisted `{ runtime: absolute path }` map.
pub const RUNTIME_BINARIES_KEY: &str = "milim.account.runtime_binaries";

/// Runtimes whose executable can be overridden.
pub const OVERRIDABLE_RUNTIMES: &[&str] = &["codex", "claude", "opencode", "pi"];

fn table() -> &'static RwLock<BTreeMap<String, PathBuf>> {
    static TABLE: OnceLock<RwLock<BTreeMap<String, PathBuf>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// The configured executable for `runtime`, when one is set.
pub(crate) fn override_for(runtime: &str) -> Option<PathBuf> {
    table()
        .read()
        .ok()
        .and_then(|table| table.get(runtime).cloned())
}

/// Every configured override, for display.
pub(crate) fn overrides() -> BTreeMap<String, String> {
    table()
        .read()
        .map(|table| {
            table
                .iter()
                .map(|(runtime, path)| (runtime.clone(), path.display().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Load persisted overrides into the process table. Entries that no longer
/// point at an executable file are kept on disk but not applied, so discovery
/// takes over until the user fixes or clears them.
pub(crate) fn load(store: &UserDataStore) {
    let persisted = persisted(store);
    if let Ok(mut table) = table().write() {
        table.clear();
        for (runtime, path) in persisted {
            let path = PathBuf::from(path);
            if OVERRIDABLE_RUNTIMES.contains(&runtime.as_str()) && is_executable_file(&path) {
                table.insert(runtime, path);
            }
        }
    }
}

fn persisted(store: &UserDataStore) -> BTreeMap<String, String> {
    store
        .get_json(RUNTIME_BINARIES_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Set (`Some`) or clear (`None`) one runtime's executable, persisting it.
pub(crate) fn set(store: &UserDataStore, runtime: &str, path: Option<&str>) -> Result<()> {
    if !OVERRIDABLE_RUNTIMES.contains(&runtime) {
        return Err(Error::InvalidRequest(format!(
            "{runtime} does not support a binary path override."
        )));
    }
    let path = match path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => Some(validate(path)?),
        None => None,
    };
    let mut persisted = persisted(store);
    match &path {
        Some(path) => {
            persisted.insert(runtime.to_string(), path.display().to_string());
        }
        None => {
            persisted.remove(runtime);
        }
    }
    let json = serde_json::to_string(&persisted)
        .map_err(|error| Error::Other(format!("serialize runtime binaries: {error}")))?;
    store.set_json(RUNTIME_BINARIES_KEY, &json)?;
    if let Ok(mut table) = table().write() {
        match path {
            Some(path) => {
                table.insert(runtime.to_string(), path);
            }
            None => {
                table.remove(runtime);
            }
        }
    }
    Ok(())
}

fn validate(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(Error::InvalidRequest(
            "Choose the executable by its absolute path.".into(),
        ));
    }
    if !is_executable_file(&path) {
        return Err(Error::InvalidRequest(format!(
            "{} is not an executable file.",
            path.display()
        )));
    }
    Ok(path)
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["exe", "cmd", "bat", "com"]
                    .iter()
                    .any(|allowed| extension.eq_ignore_ascii_case(allowed))
            })
}

/// A command for the overridden executable of `runtime`, if one is set.
///
/// The executable's own directory is prepended to the child's `PATH` so a
/// Node launcher from a version manager still finds the `node` that sits
/// beside it.
pub(crate) fn override_command(runtime: &str) -> Option<Command> {
    let path = override_for(runtime)?;
    #[cfg(windows)]
    {
        let script = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            });
        if script {
            let mut command = Command::new("cmd");
            command.arg("/D").arg("/S").arg("/C").arg(&path);
            return Some(command);
        }
        Some(Command::new(path))
    }
    #[cfg(not(windows))]
    {
        let mut dirs: Vec<PathBuf> = path.parent().map(Path::to_path_buf).into_iter().collect();
        for dir in std::env::split_paths(&crate::cli_path::search_path()) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        let executable = path.canonicalize().unwrap_or(path);
        let mut command = Command::new(executable);
        if let Ok(joined) = std::env::join_paths(dirs) {
            command.env("PATH", joined);
        }
        Some(command)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use milim_storage::Database;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn override_persists_validates_and_clears() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("milim-runtime-bin-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("pi");
        std::fs::write(&binary, "#!/bin/sh\n").unwrap();
        let plain = root.join("notes.txt");
        std::fs::write(&plain, "text").unwrap();
        let mut permissions = std::fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&binary, permissions).unwrap();

        assert!(set(&store, "pi", Some("relative/pi")).is_err());
        assert!(set(&store, "pi", Some(plain.to_str().unwrap())).is_err());
        assert!(set(&store, "gh", Some(binary.to_str().unwrap())).is_err());

        set(&store, "pi", Some(binary.to_str().unwrap())).unwrap();
        assert_eq!(override_for("pi"), Some(binary.clone()));
        assert_eq!(
            persisted(&store).get("pi").map(String::as_str),
            binary.to_str()
        );
        let command = override_command("pi").expect("override command");
        let path_env = command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("child PATH");
        assert_eq!(
            std::env::split_paths(path_env).next(),
            Some(root.clone()),
            "the executable's folder leads the child PATH"
        );

        // Reloading from the store restores the table.
        table().write().unwrap().clear();
        load(&store);
        assert_eq!(override_for("pi"), Some(binary.clone()));

        set(&store, "pi", None).unwrap();
        assert_eq!(override_for("pi"), None);
        assert!(!persisted(&store).contains_key("pi"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
