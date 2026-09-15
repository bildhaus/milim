//! Multiple signed-in accounts for one account runtime.
//!
//! Claude Code and Codex each keep their credentials, settings, and native
//! session transcripts inside one configuration home: `~/.claude` (overridable
//! with `CLAUDE_CONFIG_DIR`) and `~/.codex` (overridable with `CODEX_HOME`).
//! A profile is a named alternate home. Pointing a runtime at one selects the
//! account signed in there without Milim reading, copying, or storing any
//! credential: the CLI still owns its own tokens, and the only thing Milim
//! contributes is the directory the CLI should use.
//!
//! The implicit default profile is the runtime's own home with no override, so
//! installs that never add a profile behave exactly as before.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use milim_storage::UserDataStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

use milim_core::{Error, Result};

/// Persisted profile records, keyed by runtime.
pub const ACCOUNT_PROFILES_KEY: &str = "milim.account.profiles";
/// Observed per-profile usage and rate-limit cooldowns.
pub const ACCOUNT_PROFILE_STATE_KEY: &str = "milim.account.profiles.state";

/// Reserved id for each runtime's own configuration home.
pub const DEFAULT_PROFILE_ID: &str = "default";
/// Reserved selection meaning "let Milim choose".
pub const AUTO_PROFILE_ID: &str = "auto";

/// Runtimes that keep their whole account state in one relocatable home.
pub const PROFILE_RUNTIMES: &[&str] = &["claude", "codex"];

const MAX_PROFILES_PER_RUNTIME: usize = 16;
const MAX_LABEL_CHARS: usize = 64;

/// A named alternate configuration home for one account runtime.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountProfileRecord {
    pub id: String,
    pub runtime: String,
    pub label: String,
    /// Absolute path used as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`.
    pub config_dir: String,
    /// Eligible for automatic selection. A disabled profile stays usable when
    /// a thread pins it explicitly.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub created_at_ms: u64,
}

fn default_true() -> bool {
    true
}

/// One profile resolved for a turn: what to label it and where to point the CLI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedAccountProfile {
    pub id: String,
    pub runtime: String,
    pub label: String,
    /// `None` keeps the runtime's own default home and sets no override.
    pub home: Option<PathBuf>,
}

impl ResolvedAccountProfile {
    /// The runtime's own configuration home, with no environment override.
    pub fn default_for(runtime: &str) -> Self {
        Self {
            id: DEFAULT_PROFILE_ID.to_string(),
            runtime: runtime.to_string(),
            label: "Default".to_string(),
            home: None,
        }
    }

    /// Environment variable this runtime reads for its configuration home.
    pub fn home_var(&self) -> Option<&'static str> {
        match self.runtime.as_str() {
            "claude" => Some("CLAUDE_CONFIG_DIR"),
            "codex" => Some("CODEX_HOME"),
            _ => None,
        }
    }

    /// Point a spawned CLI at this profile's home.
    ///
    /// Account-runtime children inherit the user's environment, so a profile
    /// must also clear an override the user set for Milim's own process:
    /// leaving it in place would silently route the default profile into
    /// someone else's account home.
    pub fn apply(&self, command: &mut Command) {
        let Some(var) = self.home_var() else {
            return;
        };
        match &self.home {
            Some(home) => {
                command.env(var, home);
            }
            None => {
                command.env_remove(var);
            }
        }
    }

    /// Directories to search for this profile's transcripts and session
    /// registry. The default profile keeps the runtime's historical lookup
    /// across `USERPROFILE` and `HOME`, plus any override on Milim's own
    /// environment, because that is the home its CLI would pick.
    pub fn home_dirs(&self, leaf: &str) -> Vec<PathBuf> {
        if let Some(home) = &self.home {
            return vec![home.clone()];
        }
        let mut dirs = Vec::new();
        let mut push = |dir: PathBuf| {
            if !dirs.iter().any(|existing| existing == &dir) {
                dirs.push(dir);
            }
        };
        if let Some(var) = self.home_var() {
            if let Some(value) = std::env::var_os(var) {
                let value = PathBuf::from(value);
                if !value.as_os_str().is_empty() {
                    push(value);
                }
            }
        }
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            push(PathBuf::from(profile).join(leaf));
        }
        if let Some(home) = std::env::var_os("HOME") {
            push(PathBuf::from(home).join(leaf));
        }
        dirs
    }
}

/// Observed state for one profile: what the runtime last reported about its
/// own limits. Milim records only what a turn or an account read hands back.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct AccountProfileState {
    /// Unix ms until which this profile is known to be rate limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooled_until_ms: Option<u64>,
    /// The limit window that produced the cooldown, as the runtime named it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_kind: Option<String>,
    /// Percent of the short window consumed, when the runtime reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_window_percent: Option<f64>,
    /// Percent of the long window consumed, when the runtime reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_window_percent: Option<f64>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl AccountProfileState {
    fn cooled_at(&self, now_ms: u64) -> bool {
        self.cooled_until_ms.is_some_and(|until| until > now_ms)
    }

    /// Lower is more used. Unknown usage sorts as fully available so a profile
    /// Milim has never measured is not starved behind a measured one.
    fn headroom(&self) -> f64 {
        let used = self
            .short_window_percent
            .unwrap_or(0.0)
            .max(self.long_window_percent.unwrap_or(0.0));
        (100.0 - used).clamp(0.0, 100.0)
    }
}

/// A profile plus its observed state, for listing surfaces.
#[derive(Clone, Debug, Serialize)]
pub struct AccountProfileSummary {
    pub id: String,
    pub runtime: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    pub is_default: bool,
    pub enabled: bool,
    pub priority: i32,
    #[serde(flatten)]
    pub state: AccountProfileState,
}

/// The canonical store, when this process owns one. A standalone `milim
/// serve` has no canonical user data, so every runtime there keeps its own
/// default configuration home.
pub fn store_of(state: &crate::state::AppState) -> Option<&UserDataStore> {
    state
        .control
        .as_ref()
        .map(|control| control.store().as_ref())
}

/// Resolve the profile for one runtime from whatever the caller selected.
pub fn resolve_for(
    state: &crate::state::AppState,
    runtime: &str,
    profile_id: Option<&str>,
) -> ResolvedAccountProfile {
    resolve(store_of(state), runtime, profile_id)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

fn is_profile_runtime(runtime: &str) -> bool {
    PROFILE_RUNTIMES.contains(&runtime)
}

fn read_records(store: &UserDataStore) -> Vec<AccountProfileRecord> {
    store
        .get_json(ACCOUNT_PROFILES_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<AccountProfileRecord>>(&raw).ok())
        .unwrap_or_default()
}

fn write_records(store: &UserDataStore, records: &[AccountProfileRecord]) -> Result<()> {
    let json = serde_json::to_string(records)
        .map_err(|error| Error::Other(format!("failed to encode account profiles: {error}")))?;
    store.set_json(ACCOUNT_PROFILES_KEY, &json)
}

fn read_state(store: &UserDataStore) -> BTreeMap<String, AccountProfileState> {
    store
        .get_json(ACCOUNT_PROFILE_STATE_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<BTreeMap<String, AccountProfileState>>(&raw).ok())
        .unwrap_or_default()
}

fn write_state(store: &UserDataStore, state: &BTreeMap<String, AccountProfileState>) -> Result<()> {
    let json = serde_json::to_string(state)
        .map_err(|error| Error::Other(format!("failed to encode profile state: {error}")))?;
    store.set_json(ACCOUNT_PROFILE_STATE_KEY, &json)
}

fn state_key(runtime: &str, profile_id: &str) -> String {
    format!("{runtime}/{profile_id}")
}

/// Every profile for one runtime, default first, with observed state attached.
pub fn list(store: &UserDataStore, runtime: &str) -> Vec<AccountProfileSummary> {
    let state = read_state(store);
    let lookup = |id: &str| {
        state
            .get(&state_key(runtime, id))
            .cloned()
            .unwrap_or_default()
    };
    let mut out = vec![AccountProfileSummary {
        id: DEFAULT_PROFILE_ID.to_string(),
        runtime: runtime.to_string(),
        label: "Default".to_string(),
        config_dir: None,
        is_default: true,
        enabled: true,
        priority: 0,
        state: lookup(DEFAULT_PROFILE_ID),
    }];
    let mut records: Vec<_> = read_records(store)
        .into_iter()
        .filter(|record| record.runtime == runtime)
        .collect();
    records.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
    });
    for record in records {
        let profile_state = lookup(&record.id);
        out.push(AccountProfileSummary {
            id: record.id,
            runtime: record.runtime,
            label: record.label,
            config_dir: Some(record.config_dir),
            is_default: false,
            enabled: record.enabled,
            priority: record.priority,
            state: profile_state,
        });
    }
    out
}

fn record(store: &UserDataStore, runtime: &str, id: &str) -> Option<AccountProfileRecord> {
    read_records(store)
        .into_iter()
        .find(|record| record.runtime == runtime && record.id == id)
}

fn resolve_record(record: &AccountProfileRecord) -> ResolvedAccountProfile {
    ResolvedAccountProfile {
        id: record.id.clone(),
        runtime: record.runtime.clone(),
        label: record.label.clone(),
        home: Some(PathBuf::from(&record.config_dir)),
    }
}

/// Resolve one explicit profile id, falling back to the runtime default when
/// the id is unknown. A thread that pinned a profile the user later removed
/// keeps working on the default rather than failing its next turn.
pub fn resolve(
    store: Option<&UserDataStore>,
    runtime: &str,
    profile_id: Option<&str>,
) -> ResolvedAccountProfile {
    let Some(store) = store else {
        return ResolvedAccountProfile::default_for(runtime);
    };
    let id = profile_id.map(str::trim).filter(|id| !id.is_empty());
    match id {
        None | Some(DEFAULT_PROFILE_ID) => ResolvedAccountProfile::default_for(runtime),
        Some(AUTO_PROFILE_ID) => select(store, runtime),
        Some(id) => record(store, runtime, id)
            .as_ref()
            .map(resolve_record)
            .unwrap_or_else(|| ResolvedAccountProfile::default_for(runtime)),
    }
}

/// Choose the profile with the most reported headroom that is not cooling down.
///
/// Every profile is eligible while Milim has measured nothing, so Auto on a
/// fresh install simply keeps using the default. When all candidates are
/// cooling down, the one that recovers first is chosen: its turn may still be
/// rejected, and the caller surfaces that rather than silently stalling.
pub fn select(store: &UserDataStore, runtime: &str) -> ResolvedAccountProfile {
    let now = now_ms();
    let candidates = list(store, runtime)
        .into_iter()
        .filter(|profile| profile.enabled)
        .collect::<Vec<_>>();
    let available = least_used(
        candidates
            .iter()
            .enumerate()
            .filter(|(_, profile)| !profile.state.cooled_at(now)),
    );
    let chosen = available.or_else(|| {
        candidates
            .iter()
            .min_by_key(|profile| profile.state.cooled_until_ms.unwrap_or(u64::MAX))
    });
    chosen
        .map(into_resolved)
        .unwrap_or_else(|| ResolvedAccountProfile::default_for(runtime))
}

/// The least-used candidate, breaking ties toward the earlier listing
/// position. `list` puts the default first, so an install where Milim has
/// measured nothing keeps using the runtime's own account instead of drifting
/// onto whichever profile happened to sort last.
fn least_used<'a>(
    candidates: impl Iterator<Item = (usize, &'a AccountProfileSummary)>,
) -> Option<&'a AccountProfileSummary> {
    candidates
        .min_by(|(a_index, a), (b_index, b)| {
            b.state
                .headroom()
                .total_cmp(&a.state.headroom())
                .then_with(|| b.priority.cmp(&a.priority))
                .then_with(|| a_index.cmp(b_index))
        })
        .map(|(_, profile)| profile)
}

fn into_resolved(profile: &AccountProfileSummary) -> ResolvedAccountProfile {
    match &profile.config_dir {
        Some(dir) => ResolvedAccountProfile {
            id: profile.id.clone(),
            runtime: profile.runtime.clone(),
            label: profile.label.clone(),
            home: Some(PathBuf::from(dir)),
        },
        None => ResolvedAccountProfile::default_for(&profile.runtime),
    }
}

/// The next eligible profile after `exclude` exhausted its limit, if any.
/// Used to retry a turn that a runtime rejected before producing output.
pub fn next_after_limit(
    store: &UserDataStore,
    runtime: &str,
    exclude: &str,
) -> Option<ResolvedAccountProfile> {
    let now = now_ms();
    let candidates = list(store, runtime);
    least_used(candidates.iter().enumerate().filter(|(_, profile)| {
        profile.enabled && profile.id != exclude && !profile.state.cooled_at(now)
    }))
    .map(into_resolved)
}

fn mutate_state<F: FnOnce(&mut AccountProfileState)>(
    store: &UserDataStore,
    runtime: &str,
    profile_id: &str,
    apply: F,
) -> Result<()> {
    let mut state = read_state(store);
    let entry = state.entry(state_key(runtime, profile_id)).or_default();
    apply(entry);
    entry.updated_at_ms = now_ms();
    write_state(store, &state)
}

/// Record a rate limit a runtime reported for the profile that hit it.
pub fn record_rate_limit(
    store: &UserDataStore,
    runtime: &str,
    profile_id: &str,
    kind: Option<&str>,
    resets_at_ms: Option<u64>,
    exhausted: bool,
) -> Result<()> {
    mutate_state(store, runtime, profile_id, |state| {
        if exhausted {
            state.cooled_until_ms = resets_at_ms;
            state.cooldown_kind = kind.map(str::to_string);
        }
        // A warning without exhaustion still marks the window near its cap so
        // Auto prefers another account for the next turn.
        if !exhausted && state.short_window_percent.unwrap_or(0.0) < 90.0 {
            state.short_window_percent = Some(90.0);
        }
        if exhausted {
            state.short_window_percent = Some(100.0);
        }
    })
}

/// Record percentages a runtime published for its own limit windows.
pub fn record_usage(
    store: &UserDataStore,
    runtime: &str,
    profile_id: &str,
    short_percent: Option<f64>,
    long_percent: Option<f64>,
    resets_at_ms: Option<u64>,
) -> Result<()> {
    mutate_state(store, runtime, profile_id, |state| {
        if short_percent.is_some() {
            state.short_window_percent = short_percent;
        }
        if long_percent.is_some() {
            state.long_window_percent = long_percent;
        }
        let exhausted =
            short_percent.unwrap_or(0.0) >= 100.0 || long_percent.unwrap_or(0.0) >= 100.0;
        if exhausted {
            state.cooled_until_ms = resets_at_ms;
        } else if state.cooled_until_ms.is_some_and(|until| until <= now_ms()) {
            state.cooled_until_ms = None;
            state.cooldown_kind = None;
        }
    })
}

/// Clear a cooldown that has expired, so listings do not show a stale timer.
pub fn clear_expired_cooldowns(store: &UserDataStore) -> Result<()> {
    let now = now_ms();
    let mut state = read_state(store);
    let mut changed = false;
    for value in state.values_mut() {
        if value.cooled_until_ms.is_some_and(|until| until <= now) {
            value.cooled_until_ms = None;
            value.cooldown_kind = None;
            changed = true;
        }
    }
    if changed {
        write_state(store, &state)?;
    }
    Ok(())
}

fn validate_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        return Err(Error::InvalidRequest("Profile name is required.".into()));
    }
    if label.chars().count() > MAX_LABEL_CHARS {
        return Err(Error::InvalidRequest(format!(
            "Profile name must be at most {MAX_LABEL_CHARS} characters."
        )));
    }
    Ok(label.to_string())
}

/// Default home for a new profile: a Milim-owned directory per runtime and id.
pub fn default_config_dir(runtime: &str, id: &str) -> PathBuf {
    milim_core::paths::Paths::resolve()
        .root()
        .join("account-profiles")
        .join(runtime)
        .join(id)
}

fn profile_id_from_label(label: &str) -> String {
    let slug: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').replace("--", "-");
    let slug: String = slug.chars().take(32).collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() || slug == DEFAULT_PROFILE_ID || slug == AUTO_PROFILE_ID {
        format!("profile-{}", now_ms())
    } else {
        slug
    }
}

/// Create a profile. The configuration home is created empty; the account is
/// signed in by the runtime's own login inside that home, so Milim never sees
/// a credential.
pub fn create(
    store: &UserDataStore,
    runtime: &str,
    label: &str,
    config_dir: Option<&str>,
) -> Result<AccountProfileRecord> {
    if !is_profile_runtime(runtime) {
        return Err(Error::InvalidRequest(format!(
            "{runtime} does not support account profiles."
        )));
    }
    let label = validate_label(label)?;
    let mut records = read_records(store);
    if records.iter().filter(|r| r.runtime == runtime).count() >= MAX_PROFILES_PER_RUNTIME {
        return Err(Error::InvalidRequest(format!(
            "At most {MAX_PROFILES_PER_RUNTIME} {runtime} profiles are supported."
        )));
    }
    if records
        .iter()
        .any(|r| r.runtime == runtime && r.label.eq_ignore_ascii_case(&label))
    {
        return Err(Error::InvalidRequest(format!(
            "A {runtime} profile named \"{label}\" already exists."
        )));
    }
    let base = profile_id_from_label(&label);
    let mut id = base.clone();
    let mut suffix = 2;
    while records.iter().any(|r| r.runtime == runtime && r.id == id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    let dir = match config_dir.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(Error::InvalidRequest(
                    "A profile folder must be an absolute path.".into(),
                ));
            }
            path
        }
        None => default_config_dir(runtime, &id),
    };
    if records
        .iter()
        .any(|r| same_dir(Path::new(&r.config_dir), &dir))
    {
        return Err(Error::InvalidRequest(
            "Another profile already uses that folder.".into(),
        ));
    }
    std::fs::create_dir_all(&dir).map_err(|error| {
        Error::Other(format!(
            "failed to create the profile folder {}: {error}",
            dir.display()
        ))
    })?;
    let record = AccountProfileRecord {
        id,
        runtime: runtime.to_string(),
        label,
        config_dir: dir.to_string_lossy().to_string(),
        enabled: true,
        priority: 0,
        created_at_ms: now_ms(),
    };
    records.push(record.clone());
    write_records(store, &records)?;
    Ok(record)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let canonical =
        |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical(a) == canonical(b)
}

/// Rename a profile or change its automatic-selection eligibility.
pub fn update(
    store: &UserDataStore,
    runtime: &str,
    id: &str,
    label: Option<&str>,
    enabled: Option<bool>,
    priority: Option<i32>,
) -> Result<AccountProfileRecord> {
    let mut records = read_records(store);
    let Some(found) = records
        .iter_mut()
        .find(|record| record.runtime == runtime && record.id == id)
    else {
        return Err(Error::InvalidRequest("Unknown account profile.".into()));
    };
    if let Some(label) = label {
        found.label = validate_label(label)?;
    }
    if let Some(enabled) = enabled {
        found.enabled = enabled;
    }
    if let Some(priority) = priority {
        found.priority = priority;
    }
    let updated = found.clone();
    write_records(store, &records)?;
    Ok(updated)
}

/// Forget a profile. The configuration home is left on disk: it holds the
/// runtime's own credentials and transcripts, which are not Milim's to delete.
pub fn remove(store: &UserDataStore, runtime: &str, id: &str) -> Result<Option<String>> {
    let mut records = read_records(store);
    let Some(index) = records
        .iter()
        .position(|record| record.runtime == runtime && record.id == id)
    else {
        return Ok(None);
    };
    let removed = records.remove(index);
    write_records(store, &records)?;
    let mut state = read_state(store);
    if state.remove(&state_key(runtime, id)).is_some() {
        write_state(store, &state)?;
    }
    Ok(Some(removed.config_dir))
}

/// Pass a runtime's events through while recording what they say about the
/// selected profile's limits.
///
/// Claude publishes a limit only when it is already near or past a cap, so
/// this is the only signal Milim has for a Claude account without reading its
/// credentials. Codex additionally reports percentages up front, recorded by
/// `record_usage` when the account is read.
pub fn observe_limits<S>(
    stream: S,
    store: Option<std::sync::Arc<UserDataStore>>,
    runtime: &str,
    profile_id: &str,
) -> impl futures::Stream<Item = crate::account_runtime_events::HarnessEvent>
where
    S: futures::Stream<Item = crate::account_runtime_events::HarnessEvent>,
{
    use crate::account_runtime_events::HarnessEventKind;
    use futures::StreamExt;

    let runtime = runtime.to_string();
    let profile_id = profile_id.to_string();
    async_stream::stream! {
        futures::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            let mut notice = None;
            if event.kind() == HarnessEventKind::LimitUpdated {
                if let Some(store) = store.as_deref() {
                    let limit = event.field("limit");
                    let status = limit
                        .and_then(|limit| limit.get("status"))
                        .and_then(Value::as_str);
                    let kind = limit.and_then(|limit| limit.get("kind")).and_then(Value::as_str);
                    let resets_at_ms = limit
                        .and_then(|limit| limit.get("reset_at"))
                        .and_then(Value::as_i64)
                        .map(unix_seconds_to_ms);
                    // Claude reports `rejected` once the window is spent and
                    // `allowed_warning` while it is close; anything else is a
                    // routine update that should not move Auto.
                    let exhausted = matches!(status, Some("rejected"));
                    let warned = matches!(status, Some("allowed_warning"));
                    if exhausted || warned {
                        let _ = record_rate_limit(
                            store,
                            &runtime,
                            &profile_id,
                            kind,
                            resets_at_ms,
                            exhausted,
                        );
                        // A cross-account retry cannot happen inside this turn:
                        // the prompt was built for a session that belongs to the
                        // exhausted account, and the next account has none of
                        // that history. The next turn resolves Auto again and
                        // rebuilds the prompt for whichever account it picks, so
                        // the useful thing to do here is say which one that is.
                        notice = switch_notice(store, &runtime, &profile_id, exhausted, kind);
                    }
                }
            }
            yield event;
            if let Some(notice) = notice.take() {
                yield notice;
            }
        }
    }
}

/// A calm nonterminal notice naming the account the next turn will use.
fn switch_notice(
    store: &UserDataStore,
    runtime: &str,
    profile_id: &str,
    exhausted: bool,
    kind: Option<&str>,
) -> Option<crate::account_runtime_events::HarnessEvent> {
    use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};

    let current = list(store, runtime)
        .into_iter()
        .find(|profile| profile.id == profile_id)?;
    let next = next_after_limit(store, runtime, profile_id)?;
    let window = match kind {
        Some(kind) if !kind.trim().is_empty() => format!(" ({kind})"),
        _ => String::new(),
    };
    let state = if exhausted {
        "is rate limited"
    } else {
        "is close to its limit"
    };
    let mut fields = serde_json::Map::new();
    fields.insert("kind".into(), Value::String("info".into()));
    fields.insert("level".into(), Value::String("info".into()));
    fields.insert(
        "code".into(),
        Value::String("account_profile_switch".into()),
    );
    fields.insert(
        "message".into(),
        Value::String(format!(
            "The {} account{window} {state}. Auto will use {} for the next turn in this chat.",
            current.label, next.label
        )),
    );
    fields.insert("profile_id".into(), Value::String(profile_id.to_string()));
    fields.insert("next_profile_id".into(), Value::String(next.id));
    fields.insert("next_profile_label".into(), Value::String(next.label));
    Some(HarnessEvent::new(HarnessEventKind::RuntimeNotice, fields))
}

/// Percentages Codex reports for its own limit windows, as
/// `(short_window, long_window, soonest_reset_ms)`.
///
/// Codex has moved these fields around across releases, so this reads the
/// shapes it has used — a `rateLimits` wrapper, `primary`/`secondary` buckets,
/// and either snake or camel case — and otherwise walks for any object
/// carrying a used-percent field. Anything it cannot recognize simply yields
/// no measurement, leaving Auto on its existing preference.
pub fn codex_usage(payload: &Value) -> (Option<f64>, Option<f64>, Option<u64>) {
    let root = ["rateLimits", "rate_limits", "limits"]
        .iter()
        .find_map(|key| payload.get(*key))
        .unwrap_or(payload);
    let mut buckets = Vec::new();
    collect_usage_buckets(root, 0, &mut buckets);
    let short = buckets
        .iter()
        .find(|(key, _, _)| is_short_window(key))
        .or_else(|| buckets.first());
    let long = buckets
        .iter()
        .find(|(key, _, _)| is_long_window(key))
        .or_else(|| buckets.iter().find(|bucket| Some(*bucket) != short));
    let resets = buckets.iter().filter_map(|(_, _, resets)| *resets).min();
    (
        short.map(|(_, percent, _)| *percent),
        long.map(|(_, percent, _)| *percent),
        resets,
    )
}

fn is_short_window(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("primary")
        || key.contains("5h")
        || key.contains("five")
        || key.contains("hour")
        || key.contains("short")
}

fn is_long_window(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("secondary")
        || key.contains("7d")
        || key.contains("seven")
        || key.contains("week")
        || key.contains("day")
        || key.contains("long")
}

/// `(key hint, used percent, reset instant in unix ms)` for each window found.
fn collect_usage_buckets(value: &Value, depth: usize, out: &mut Vec<(String, f64, Option<u64>)>) {
    const MAX_DEPTH: usize = 4;
    let Some(object) = value.as_object() else {
        return;
    };
    if depth > MAX_DEPTH {
        return;
    }
    for (key, child) in object {
        let Some(child_object) = child.as_object() else {
            continue;
        };
        let percent = ["used_percent", "usedPercent", "percent_used", "percentUsed"]
            .iter()
            .find_map(|field| child_object.get(*field))
            .and_then(Value::as_f64);
        if let Some(percent) = percent {
            let resets = ["resets_at", "resetsAt", "reset_at", "resetAt"]
                .iter()
                .find_map(|field| child_object.get(*field))
                .and_then(Value::as_i64)
                .map(unix_seconds_to_ms)
                .or_else(|| {
                    ["resets_in_seconds", "resetsInSeconds"]
                        .iter()
                        .find_map(|field| child_object.get(*field))
                        .and_then(Value::as_i64)
                        .map(|seconds| now_ms() + (seconds.max(0) as u64) * 1000)
                });
            out.push((key.clone(), percent.clamp(0.0, 100.0), resets));
        } else {
            collect_usage_buckets(child, depth + 1, out);
        }
    }
}

/// Runtimes report reset instants in unix seconds; some send milliseconds.
fn unix_seconds_to_ms(value: i64) -> u64 {
    let value = value.max(0) as u64;
    if value > 100_000_000_000 {
        value
    } else {
        value.saturating_mul(1000)
    }
}

/// The command a user runs to sign a profile in, shown beside the profile.
pub fn login_hint(profile: &ResolvedAccountProfile) -> Option<Value> {
    let home = profile.home.as_ref()?;
    let var = profile.home_var()?;
    let dir = home.to_string_lossy().to_string();
    Some(json!({
        "variable": var,
        "directory": dir,
        "posix": match profile.runtime.as_str() {
            "codex" => format!("{var}=\"{dir}\" codex login"),
            _ => format!("{var}=\"{dir}\" claude auth login"),
        },
        "powershell": match profile.runtime.as_str() {
            "codex" => format!("$env:{var}=\"{dir}\"; codex login"),
            _ => format!("$env:{var}=\"{dir}\"; claude auth login"),
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_storage::Database;

    /// A scratch root plus an in-memory store. Profile homes are real
    /// directories because `create` makes them, so the root is removed after.
    struct Fixture {
        root: PathBuf,
        store: UserDataStore,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("milim-account-profiles-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
            Self { root, store }
        }

        fn home(&self, name: &str) -> String {
            self.root.join(name).to_string_lossy().to_string()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn default_profile_is_listed_before_any_profile_exists() {
        let fixture = Fixture::new();
        let profiles = list(&fixture.store, "claude");
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].is_default);
        assert_eq!(profiles[0].id, DEFAULT_PROFILE_ID);
        assert!(profiles[0].config_dir.is_none());
    }

    #[test]
    fn resolving_without_a_store_keeps_the_runtime_default() {
        let profile = resolve(None, "claude", Some("work"));
        assert!(profile.home.is_none());
        assert_eq!(profile.home_var(), Some("CLAUDE_CONFIG_DIR"));
    }

    #[test]
    fn an_unknown_profile_falls_back_to_the_default() {
        let fixture = Fixture::new();
        let profile = resolve(Some(&fixture.store), "codex", Some("removed"));
        assert!(profile.home.is_none());
        assert_eq!(profile.home_var(), Some("CODEX_HOME"));
    }

    #[test]
    fn creating_a_profile_makes_its_home_and_resolves_to_it() {
        let fixture = Fixture::new();
        let home = fixture.home("work-home");
        let record = create(&fixture.store, "claude", "Work Max", Some(&home)).unwrap();
        assert_eq!(record.id, "work-max");
        assert!(Path::new(&home).is_dir());
        let resolved = resolve(Some(&fixture.store), "claude", Some("work-max"));
        assert_eq!(resolved.home.as_deref(), Some(Path::new(&home)));
        assert_eq!(resolved.home_dirs(".claude"), vec![PathBuf::from(&home)]);
    }

    #[test]
    fn profiles_without_a_folder_land_under_milim_runtime_data() {
        let fixture = Fixture::new();
        let record = create(&fixture.store, "codex", "Personal", None).unwrap();
        assert_eq!(
            Path::new(&record.config_dir),
            default_config_dir("codex", "personal")
        );
        let _ = std::fs::remove_dir_all(&record.config_dir);
    }

    #[test]
    fn duplicate_names_and_folders_are_refused() {
        let fixture = Fixture::new();
        let home = fixture.home("one");
        create(&fixture.store, "claude", "Work", Some(&home)).unwrap();
        assert!(create(&fixture.store, "claude", "work", None).is_err());
        assert!(create(&fixture.store, "claude", "Second", Some(&home)).is_err());
    }

    #[test]
    fn the_same_name_is_allowed_on_a_different_runtime() {
        let fixture = Fixture::new();
        create(&fixture.store, "claude", "Work", Some(&fixture.home("c"))).unwrap();
        create(&fixture.store, "codex", "Work", Some(&fixture.home("x"))).unwrap();
        assert_eq!(list(&fixture.store, "claude").len(), 2);
        assert_eq!(list(&fixture.store, "codex").len(), 2);
    }

    #[test]
    fn relative_folders_and_unsupported_runtimes_are_refused() {
        let fixture = Fixture::new();
        assert!(create(&fixture.store, "codex", "Relative", Some("./somewhere")).is_err());
        assert!(create(&fixture.store, "opencode", "Any", None).is_err());
    }

    #[test]
    fn reserved_ids_are_never_produced() {
        let fixture = Fixture::new();
        let record = create(&fixture.store, "claude", "Auto", Some(&fixture.home("a"))).unwrap();
        assert_ne!(record.id, AUTO_PROFILE_ID);
        let record = create(
            &fixture.store,
            "claude",
            "Default",
            Some(&fixture.home("d")),
        )
        .unwrap();
        assert_ne!(record.id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn auto_prefers_the_profile_with_the_most_headroom() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "codex",
            "Second",
            Some(&fixture.home("second")),
        )
        .unwrap();
        record_usage(
            &fixture.store,
            "codex",
            DEFAULT_PROFILE_ID,
            Some(80.0),
            None,
            None,
        )
        .unwrap();
        record_usage(&fixture.store, "codex", "second", Some(10.0), None, None).unwrap();
        assert_eq!(select(&fixture.store, "codex").id, "second");
        record_usage(&fixture.store, "codex", "second", Some(95.0), None, None).unwrap();
        assert_eq!(select(&fixture.store, "codex").id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn auto_stays_on_the_default_until_a_limit_is_observed() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        assert_eq!(select(&fixture.store, "claude").id, DEFAULT_PROFILE_ID);
    }

    #[test]
    fn auto_skips_a_cooling_profile() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        record_rate_limit(
            &fixture.store,
            "claude",
            DEFAULT_PROFILE_ID,
            Some("five_hour"),
            Some(now_ms() + 60_000),
            true,
        )
        .unwrap();
        assert_eq!(select(&fixture.store, "claude").id, "backup");
    }

    #[test]
    fn an_expired_cooldown_is_cleared_and_the_profile_returns() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        record_rate_limit(
            &fixture.store,
            "claude",
            DEFAULT_PROFILE_ID,
            Some("five_hour"),
            Some(now_ms() - 1),
            true,
        )
        .unwrap();
        clear_expired_cooldowns(&fixture.store).unwrap();
        assert!(list(&fixture.store, "claude")
            .iter()
            .all(|profile| profile.state.cooled_until_ms.is_none()));
    }

    #[test]
    fn every_profile_cooling_still_yields_the_one_that_recovers_first() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        let now = now_ms();
        record_rate_limit(
            &fixture.store,
            "claude",
            DEFAULT_PROFILE_ID,
            Some("seven_day"),
            Some(now + 600_000),
            true,
        )
        .unwrap();
        record_rate_limit(
            &fixture.store,
            "claude",
            "backup",
            Some("five_hour"),
            Some(now + 60_000),
            true,
        )
        .unwrap();
        assert_eq!(select(&fixture.store, "claude").id, "backup");
    }

    #[test]
    fn a_warning_moves_auto_off_the_warned_profile() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        record_rate_limit(
            &fixture.store,
            "claude",
            DEFAULT_PROFILE_ID,
            Some("five_hour"),
            None,
            false,
        )
        .unwrap();
        assert_eq!(select(&fixture.store, "claude").id, "backup");
        assert!(list(&fixture.store, "claude")
            .iter()
            .all(|profile| profile.state.cooled_until_ms.is_none()));
    }

    #[test]
    fn a_disabled_profile_is_skipped_by_auto_but_still_pinnable() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Paused",
            Some(&fixture.home("paused")),
        )
        .unwrap();
        update(&fixture.store, "claude", "paused", None, Some(false), None).unwrap();
        record_usage(
            &fixture.store,
            "claude",
            DEFAULT_PROFILE_ID,
            Some(99.0),
            None,
            None,
        )
        .unwrap();
        assert_eq!(select(&fixture.store, "claude").id, DEFAULT_PROFILE_ID);
        assert_eq!(
            resolve(Some(&fixture.store), "claude", Some("paused")).id,
            "paused"
        );
    }

    #[test]
    fn retry_selection_excludes_the_exhausted_profile() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        let next = next_after_limit(&fixture.store, "claude", DEFAULT_PROFILE_ID).unwrap();
        assert_eq!(next.id, "backup");
        assert!(next_after_limit(&fixture.store, "claude", "backup").is_some());
        remove(&fixture.store, "claude", "backup").unwrap();
        assert!(next_after_limit(&fixture.store, "claude", DEFAULT_PROFILE_ID).is_none());
    }

    #[test]
    fn retry_selection_skips_a_cooling_alternative() {
        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        record_rate_limit(
            &fixture.store,
            "claude",
            "backup",
            Some("five_hour"),
            Some(now_ms() + 60_000),
            true,
        )
        .unwrap();
        assert!(next_after_limit(&fixture.store, "claude", DEFAULT_PROFILE_ID).is_none());
    }

    #[test]
    fn removing_a_profile_keeps_its_credentials_on_disk() {
        let fixture = Fixture::new();
        let home = fixture.home("gone");
        create(&fixture.store, "codex", "Gone", Some(&home)).unwrap();
        let removed = remove(&fixture.store, "codex", "gone").unwrap();
        assert_eq!(removed.as_deref(), Some(home.as_str()));
        assert!(Path::new(&home).is_dir());
        assert_eq!(list(&fixture.store, "codex").len(), 1);
    }

    #[test]
    fn login_hints_name_the_runtime_variable_and_folder() {
        let fixture = Fixture::new();
        let home = fixture.home("hinted");
        create(&fixture.store, "claude", "Hinted", Some(&home)).unwrap();
        let profile = resolve(Some(&fixture.store), "claude", Some("hinted"));
        let hint = login_hint(&profile).unwrap();
        assert_eq!(hint["variable"], "CLAUDE_CONFIG_DIR");
        assert!(hint["posix"]
            .as_str()
            .unwrap()
            .contains("claude auth login"));
        assert!(hint["posix"].as_str().unwrap().contains(&home));
        assert!(login_hint(&ResolvedAccountProfile::default_for("claude")).is_none());
    }

    #[test]
    fn codex_usage_reads_the_primary_and_secondary_windows() {
        let payload = serde_json::json!({
            "rateLimits": {
                "primary": { "used_percent": 42.5, "resets_in_seconds": 600 },
                "secondary": { "used_percent": 7.0 }
            }
        });
        let (short, long, resets) = codex_usage(&payload);
        assert_eq!(short, Some(42.5));
        assert_eq!(long, Some(7.0));
        assert!(resets.is_some_and(|value| value > now_ms()));
    }

    #[test]
    fn codex_usage_accepts_camel_case_and_absolute_resets() {
        let payload = serde_json::json!({
            "fiveHour": { "usedPercent": 90, "resetsAt": 1_800_000_000i64 },
            "sevenDay": { "usedPercent": 30 }
        });
        let (short, long, resets) = codex_usage(&payload);
        assert_eq!(short, Some(90.0));
        assert_eq!(long, Some(30.0));
        assert_eq!(resets, Some(1_800_000_000_000));
    }

    #[test]
    fn unrecognized_usage_payloads_measure_nothing() {
        let (short, long, resets) = codex_usage(&serde_json::json!({ "plan": "pro" }));
        assert!(short.is_none() && long.is_none() && resets.is_none());
    }

    #[tokio::test]
    async fn a_rejected_limit_records_a_cooldown_and_names_the_next_account() {
        use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};
        use futures::StreamExt;

        let fixture = Fixture::new();
        create(
            &fixture.store,
            "claude",
            "Backup",
            Some(&fixture.home("backup")),
        )
        .unwrap();
        let store =
            std::sync::Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        // Reuse the fixture's records in the shared store the stream holds.
        store
            .set_json(
                ACCOUNT_PROFILES_KEY,
                &fixture
                    .store
                    .get_json(ACCOUNT_PROFILES_KEY)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();

        let mut fields = serde_json::Map::new();
        fields.insert(
            "limit".into(),
            serde_json::json!({
                "status": "rejected",
                "kind": "five_hour",
                "reset_at": 1_800_000_000i64,
            }),
        );
        let source = futures::stream::iter(vec![HarnessEvent::new(
            HarnessEventKind::LimitUpdated,
            fields,
        )]);
        let events: Vec<_> = observe_limits(source, Some(store.clone()), "claude", "default")
            .collect()
            .await;

        assert_eq!(
            events.len(),
            2,
            "the limit passes through and a notice follows"
        );
        let notice = &events[1];
        assert_eq!(notice.kind(), HarnessEventKind::RuntimeNotice);
        assert_eq!(
            notice.field("code").and_then(Value::as_str),
            Some("account_profile_switch")
        );
        assert_eq!(
            notice.field("next_profile_id").and_then(Value::as_str),
            Some("backup")
        );
        assert_eq!(
            list(&store, "claude")
                .into_iter()
                .find(|profile| profile.id == DEFAULT_PROFILE_ID)
                .and_then(|profile| profile.state.cooled_until_ms),
            Some(1_800_000_000_000)
        );
    }

    /// With nothing to switch to, the limit still passes through untouched and
    /// no notice promises an account that does not exist.
    #[tokio::test]
    async fn a_limit_without_an_alternative_adds_no_notice() {
        use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};
        use futures::StreamExt;

        let store =
            std::sync::Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        let mut fields = serde_json::Map::new();
        fields.insert(
            "limit".into(),
            serde_json::json!({ "status": "rejected", "kind": "seven_day" }),
        );
        let source = futures::stream::iter(vec![HarnessEvent::new(
            HarnessEventKind::LimitUpdated,
            fields,
        )]);
        let events: Vec<_> = observe_limits(source, Some(store), "claude", "default")
            .collect()
            .await;
        assert_eq!(events.len(), 1);
    }

    /// A routine limit update is not a cap, and must not cool an account down.
    #[tokio::test]
    async fn an_allowed_limit_update_changes_nothing() {
        use crate::account_runtime_events::{HarnessEvent, HarnessEventKind};
        use futures::StreamExt;

        let store =
            std::sync::Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        let mut fields = serde_json::Map::new();
        fields.insert(
            "limit".into(),
            serde_json::json!({ "status": "allowed", "kind": "five_hour" }),
        );
        let source = futures::stream::iter(vec![HarnessEvent::new(
            HarnessEventKind::LimitUpdated,
            fields,
        )]);
        let events: Vec<_> = observe_limits(source, Some(store.clone()), "claude", "default")
            .collect()
            .await;
        assert_eq!(events.len(), 1);
        assert!(list(&store, "claude")[0]
            .state
            .short_window_percent
            .is_none());
    }

    #[tokio::test]
    async fn applying_a_profile_overrides_the_home_variable() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf '%s' \"$CLAUDE_CONFIG_DIR\""]);
        let profile = ResolvedAccountProfile {
            id: "work".into(),
            runtime: "claude".into(),
            label: "Work".into(),
            home: Some(PathBuf::from("/tmp/milim-profile-proof")),
        };
        profile.apply(&mut command);
        let output = command.output().await.unwrap();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "/tmp/milim-profile-proof"
        );
    }

    /// Account-runtime children inherit Milim's own environment, so a stray
    /// override on the Milim process must not leak into the default profile.
    #[tokio::test]
    async fn the_default_profile_clears_an_inherited_override() {
        let mut command = Command::new("sh");
        command.args(["-c", "printf '%s' \"${CODEX_HOME-unset}\""]);
        command.env("CODEX_HOME", "/tmp/milim-inherited");
        ResolvedAccountProfile::default_for("codex").apply(&mut command);
        let output = command.output().await.unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "unset");
    }
}
