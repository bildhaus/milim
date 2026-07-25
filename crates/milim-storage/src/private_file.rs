use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use milim_core::{Error, Result};

static PRIVATE_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically create a file containing secret material.
///
/// The destination must not already exist. Unix files are private from their
/// first write; Windows inherits the ACL of Milim's per-user app-data folder.
pub fn create_private_file(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidRequest("private file path has no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("secret");
    let temp_path = parent.join(format!(
        ".{file_name}.milim-{}-{}.tmp",
        std::process::id(),
        PRIVATE_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp_path)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        std::fs::hard_link(&temp_path, path)?;
        std::fs::remove_file(&temp_path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_without_overwriting() {
        let root = std::env::temp_dir().join(format!(
            "milim-private-file-test-{}-{}",
            std::process::id(),
            PRIVATE_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let path = root.join("secret.key");
        create_private_file(&path, b"first").unwrap();
        assert!(create_private_file(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
