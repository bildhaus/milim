//! Server configuration, ported from milim's `ServerConfiguration` +
//! `ServerConfigurationStore`. Persisted as pretty JSON at
//! `<root>/config/server.json` with atomic writes.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

/// How aggressively idle models are unloaded from memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ModelIdleResidencyPolicy {
    /// Unload as soon as the last in-flight request drains.
    Immediately,
    /// Keep warm for a short window (milim's `defaultWarm`).
    #[default]
    DefaultWarm,
    /// Never auto-unload.
    Never,
}

/// Appearance mode (kept for GUI-phase parity; ignored headless).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

/// The milim default port.
pub const DEFAULT_PORT: u16 = 7377;

/// Server-wide configuration. Field names match milim's JSON so a config
/// authored by either app is interchangeable. `#[serde(default)]` makes loads
/// tolerant of older/partial documents (absent fields take their default).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfiguration {
    /// TCP port to bind (default 7377).
    pub port: u16,
    /// Bind to `0.0.0.0` (LAN) instead of loopback only.
    pub expose_to_network: bool,
    /// Launch the app at login (GUI phase).
    pub start_at_login: bool,
    /// Hide the dock icon / run as a menu-bar app (macOS GUI phase).
    pub hide_dock_icon: bool,
    /// UI appearance preference (GUI phase).
    pub appearance_mode: AppearanceMode,
    /// Default top-p applied to local generation when the request omits it.
    pub gen_top_p: f32,
    /// CORS allow-list. Empty means "no browser origins allowed".
    pub allowed_origins: Vec<String>,
    /// Optional outbound proxy for remote providers.
    pub global_proxy_url: Option<String>,
    /// Idle-residency policy for loaded models.
    pub model_idle_residency_policy: ModelIdleResidencyPolicy,
    /// Hard cap on request body size, in bytes.
    pub max_request_body_bytes: usize,
    /// Require bearer auth for the standalone server. When true, the local
    /// machine identity is accepted as an msk-v1 issuer.
    pub auth_required: bool,
    /// Static bearer tokens accepted by the standalone server.
    pub api_keys: Vec<String>,
    /// Extra msk-v1 issuer addresses accepted by the standalone server.
    pub access_key_issuers: Vec<String>,
}

impl Default for ServerConfiguration {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            expose_to_network: false,
            start_at_login: false,
            hide_dock_icon: true,
            appearance_mode: AppearanceMode::default(),
            gen_top_p: 1.0,
            allowed_origins: Vec::new(),
            global_proxy_url: None,
            model_idle_residency_policy: ModelIdleResidencyPolicy::default(),
            max_request_body_bytes: 32 * 1024 * 1024, // 32 MiB
            auth_required: false,
            api_keys: Vec::new(),
            access_key_issuers: Vec::new(),
        }
    }
}

impl ServerConfiguration {
    /// Load from `path`, or return defaults if the file does not exist.
    /// Unknown/missing fields fall back to defaults rather than failing.
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing_warn(&format!("server.json parse failed ({e}); using defaults"));
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Atomically persist to `path` (write temp + rename).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&json)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// The host string to bind, based on `expose_to_network`.
    pub fn bind_host(&self) -> &'static str {
        if self.expose_to_network {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        }
    }
}

// milim-core stays runtime-light; emit warnings via eprintln rather than pulling
// in `tracing` here. `milim-server`/`milim-cli` install a tracing subscriber.
fn tracing_warn(msg: &str) {
    eprintln!("[milim-core] WARN {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_through_json() {
        let cfg = ServerConfiguration::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ServerConfiguration = serde_json::from_str(&json).unwrap();
        assert_eq!(back.port, DEFAULT_PORT);
        assert_eq!(back.bind_host(), "127.0.0.1");
        assert!(!back.auth_required);
        assert!(back.api_keys.is_empty());
        assert!(back.access_key_issuers.is_empty());
    }

    #[test]
    fn missing_fields_use_defaults() {
        // A minimal/older config loads: present fields win, absent fields
        // take their default (container-level `#[serde(default)]`).
        let partial = r#"{"port": 9000, "exposeToNetwork": true}"#;
        let cfg: ServerConfiguration = serde_json::from_str(partial).unwrap();
        assert_eq!(cfg.port, 9000);
        assert!(cfg.expose_to_network);
        assert_eq!(cfg.bind_host(), "0.0.0.0");
        // untouched field falls back to its default
        assert_eq!(cfg.gen_top_p, 1.0);
        assert_eq!(cfg.max_request_body_bytes, 32 * 1024 * 1024);
        assert!(!cfg.auth_required);
        assert!(cfg.api_keys.is_empty());
        assert!(cfg.access_key_issuers.is_empty());
    }

    #[test]
    fn auth_fields_roundtrip_through_json() {
        let json = r#"{
            "authRequired": true,
            "apiKeys": ["static-secret"],
            "accessKeyIssuers": ["0xabc"]
        }"#;
        let cfg: ServerConfiguration = serde_json::from_str(json).unwrap();
        assert!(cfg.auth_required);
        assert_eq!(cfg.api_keys, vec!["static-secret"]);
        assert_eq!(cfg.access_key_issuers, vec!["0xabc"]);
    }
}
