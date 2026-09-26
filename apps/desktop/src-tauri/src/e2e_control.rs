//! Debug-only end-to-end test hook.
//!
//! The desktop API token is random per launch and handed only to the webview,
//! so an external smoke test cannot reach the embedded server's `/control/v1`
//! routes. When a debug build starts with `MILIM_E2E_CONTROL_FILE=<path>`, it
//! writes `{"api_url": ..., "token": ...}` to that path, readable only by the
//! current user, and deletes it on exit. Release builds do not compile this
//! module, and the token is never logged.

use std::io::Write;
use std::path::{Path, PathBuf};

const CONTROL_FILE_ENV: &str = "MILIM_E2E_CONTROL_FILE";

/// The control file requested through the environment, if any.
pub(crate) fn control_file() -> Option<PathBuf> {
    std::env::var_os(CONTROL_FILE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Publish the API URL and token for an end-to-end test. The file is written
/// beside its final path and renamed into place, so a reader never sees a
/// partial file.
pub(crate) fn publish(path: &Path, api_url: &str, token: &str) -> std::io::Result<()> {
    let body = serde_json::to_vec(&serde_json::json!({ "api_url": api_url, "token": token }))
        .map_err(std::io::Error::other)?;
    let staging = path.with_extension("tmp");
    let _ = std::fs::remove_file(&staging);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&staging)?;
    file.write_all(&body)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&staging, path)
}

/// Remove the control file when the app exits.
pub(crate) fn withdraw(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Publish the control file when the environment asks for one.
pub(crate) fn publish_from_env(api_url: &str, token: &str) {
    let Some(path) = control_file() else {
        return;
    };
    match publish(&path, api_url, token) {
        Ok(()) => tracing::info!("wrote the end-to-end control file {}", path.display()),
        Err(error) => tracing::warn!(
            "could not write the end-to-end control file {}: {error}",
            path.display()
        ),
    }
}

/// Remove the control file named by the environment, if any.
pub(crate) fn withdraw_from_env() {
    if let Some(path) = control_file() {
        withdraw(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_a_private_control_file_and_withdraws_it() {
        let dir = std::env::temp_dir().join(milim_server::gen_id("milim-e2e"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("control.json");

        publish(&path, "http://127.0.0.1:7377", "desktop-secret").unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["api_url"], "http://127.0.0.1:7377");
        assert_eq!(value["token"], "desktop-secret");
        assert!(!path.with_extension("tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // A later launch replaces the file from an earlier one.
        publish(&path, "http://127.0.0.1:50000", "next-secret").unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["token"], "next-secret");

        withdraw(&path);
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
