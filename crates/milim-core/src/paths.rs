//! Cross-platform on-disk layout.
//!
//! Mirrors milim's `~/.milim` root (config, agents, providers, cache,
//! runtime) and a separate models directory. Honors the same environment
//! overrides where it makes sense:
//!   - `MILIM_HOME`     — override the root (default `~/.milim`)
//!   - `MILIM_MODELS_DIR`   — override the model store (default `<root>/models`)

use std::path::{Path, PathBuf};

/// Resolves and lazily creates the directories milim reads and writes.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
    models: PathBuf,
}

impl Paths {
    /// Build the path set from environment overrides and platform home dir.
    pub fn resolve() -> Self {
        let root = non_empty_env_os("MILIM_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".milim"));

        let models = non_empty_env_os("MILIM_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("models"));

        Self { root, models }
    }

    /// The `~/.milim` root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/config` — JSON configuration stores.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    /// `<root>/config/server.json` — the [`crate::config::ServerConfiguration`].
    pub fn server_config_file(&self) -> PathBuf {
        self.config_dir().join("server.json")
    }

    /// `<root>/settings.json` - user settings for hooks and language servers.
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// `<root>/milim.db` - canonical user state database for syncable profile data.
    pub fn user_db_file(&self) -> PathBuf {
        self.root.join("milim.db")
    }

    /// `<root>/runtime/locks` — coordinator lock files (CLI ↔ daemon).
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("runtime").join("locks")
    }

    /// `<root>/cache` — disk caches (KV cache, downloads-in-progress).
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// The local downloaded model asset store, `<root>/models` unless overridden.
    pub fn models_dir(&self) -> &Path {
        &self.models
    }

    /// Create the root, config, runtime, cache and models directories.
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            self.config_dir(),
            self.locks_dir(),
            self.cache_dir(),
            self.models.clone(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

fn non_empty_env_os(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

impl Default for Paths {
    fn default() -> Self {
        Self::resolve()
    }
}

/// Best-effort home directory, cross-platform.
fn home_dir() -> PathBuf {
    if let Some(base) = directories::BaseDirs::new() {
        return base.home_dir().to_path_buf();
    }
    // Last-resort fallbacks if no home is configured.
    if let Some(p) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(p);
    }
    PathBuf::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_dir_defaults_under_root() {
        // With no env overrides the models dir sits under the root.
        let p = Paths {
            root: PathBuf::from("/tmp/osa"),
            models: PathBuf::from("/tmp/osa/models"),
        };
        assert!(p.models_dir().starts_with(p.root()));
        assert_eq!(
            p.server_config_file(),
            PathBuf::from("/tmp/osa/config/server.json")
        );
    }

    #[test]
    fn user_db_file_defaults_under_root() {
        let p = Paths {
            root: PathBuf::from("/tmp/milim"),
            models: PathBuf::from("/tmp/milim/models"),
        };
        assert_eq!(p.user_db_file(), PathBuf::from("/tmp/milim/milim.db"));
    }
}
