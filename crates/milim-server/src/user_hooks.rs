//! User-defined hooks around native agent runs.
//!
//! Hooks live under `hooks` in `<milim home>/settings.json` (user) and
//! `<workspace>/.milim/settings.json` (project):
//!
//! ```json
//! {"hooks": {"PreToolUse": [{"matcher": "shell|edit_file", "command": "...", "timeout_secs": 30}]}}
//! ```
//!
//! Each hook is a host shell command run in the workspace with the event JSON
//! on stdin. User hooks are trusted. Project hooks are repository code, so
//! they run only after the user trusts that workspace's exact `hooks` config;
//! trust is stored by workspace path and config hash, so any change needs
//! trust again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use milim_agents::{
    HookActivity, HookTrustRequest, InterceptedCall, ResultInterception, StopInterception,
    ToolDecision, ToolInterception, ToolInterceptor, TurnInterception,
};
use milim_core::api::openai::ChatMessage;
use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};

pub(crate) const PRE_TOOL_USE: &str = "PreToolUse";
pub(crate) const POST_TOOL_USE: &str = "PostToolUse";
pub(crate) const USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
pub(crate) const STOP: &str = "Stop";
const PROJECT_SETTINGS: &str = ".milim/settings.json";
const TRUST_FILE: &str = "hook-trust.json";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 600;
/// Bytes captured per output stream of one hook run.
const MAX_CAPTURE_BYTES: usize = 64 * 1024;
/// Hook output the model sees, per hook run.
const MAX_MODEL_BYTES: usize = 8 * 1024;
/// Hook output shown in the run timeline.
const MAX_TIMELINE_CHARS: usize = 500;
const MAX_HOOK_LABEL_CHARS: usize = 120;
/// How long output readers may drain after the process tree ends.
const READER_DRAIN: Duration = Duration::from_secs(2);

/// The `hooks` object of a settings file.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct HooksConfig {
    #[serde(rename = "PreToolUse", default, skip_serializing_if = "Vec::is_empty")]
    pub pre_tool_use: Vec<HookSpec>,
    #[serde(rename = "PostToolUse", default, skip_serializing_if = "Vec::is_empty")]
    pub post_tool_use: Vec<HookSpec>,
    #[serde(
        rename = "UserPromptSubmit",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub user_prompt_submit: Vec<HookSpec>,
    #[serde(rename = "Stop", default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<HookSpec>,
}

impl HooksConfig {
    fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty()
            && self.post_tool_use.is_empty()
            && self.user_prompt_submit.is_empty()
            && self.stop.is_empty()
    }

    fn events(&self) -> [(&'static str, &[HookSpec]); 4] {
        [
            (PRE_TOOL_USE, &self.pre_tool_use),
            (POST_TOOL_USE, &self.post_tool_use),
            (USER_PROMPT_SUBMIT, &self.user_prompt_submit),
            (STOP, &self.stop),
        ]
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct HookSpec {
    /// Regex that must match the whole tool name; empty or missing matches
    /// every tool. Ignored for `UserPromptSubmit` and `Stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// One settings file's hook-related content.
#[derive(Debug, Clone, Default)]
pub(crate) struct SettingsHooks {
    pub hooks: HooksConfig,
    /// The raw `hooks` value, hashed for project trust.
    pub raw: Option<Value>,
    pub allow_hooks_to_approve: bool,
}

/// Read the hook settings of one file. A missing file has no hooks; an
/// unreadable or invalid one is logged and treated as having none.
pub(crate) fn read_settings(path: &Path) -> SettingsHooks {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("cannot read {}: {error}", path.display());
            }
            return SettingsHooks::default();
        }
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!("ignoring invalid settings file {}: {error}", path.display());
            return SettingsHooks::default();
        }
    };
    let raw = value.get("hooks").filter(|hooks| !hooks.is_null()).cloned();
    let hooks = match raw.clone().map(serde_json::from_value::<HooksConfig>) {
        None => HooksConfig::default(),
        Some(Ok(hooks)) => hooks,
        Some(Err(error)) => {
            tracing::warn!("ignoring invalid hooks in {}: {error}", path.display());
            HooksConfig::default()
        }
    };
    SettingsHooks {
        hooks,
        raw,
        allow_hooks_to_approve: value
            .get("allow_hooks_to_approve")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

/// Stable hash of a `hooks` value; object keys are hashed in sorted order.
pub(crate) fn config_hash(raw: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    let encoded = serde_json::to_vec(&canonical(raw)).unwrap_or_default();
    Sha256::digest(&encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Canonical workspace key for trust records.
fn workspace_key(workspace: &Path) -> String {
    std::fs::canonicalize(workspace)
        .unwrap_or_else(|_| workspace.to_path_buf())
        .display()
        .to_string()
}

/// Trusted project hook configs, keyed by workspace path, in
/// `<milim home>/config/hook-trust.json`.
#[derive(Debug, Clone)]
pub(crate) struct HookTrustStore {
    path: PathBuf,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct TrustFile {
    #[serde(default)]
    workspaces: BTreeMap<String, String>,
}

static TRUST_WRITE: Mutex<()> = Mutex::new(());

impl HookTrustStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn resolve() -> Self {
        Self::new(
            milim_core::paths::Paths::resolve()
                .config_dir()
                .join(TRUST_FILE),
        )
    }

    fn read(&self) -> TrustFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub(crate) fn is_trusted(&self, workspace: &Path, hash: &str) -> bool {
        self.read()
            .workspaces
            .get(&workspace_key(workspace))
            .map(String::as_str)
            == Some(hash)
    }

    /// Trust `hash` for `workspace`, or forget the workspace's trust.
    pub(crate) fn set(&self, workspace: &Path, hash: Option<&str>) -> Result<()> {
        let _guard = TRUST_WRITE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut file = self.read();
        let key = workspace_key(workspace);
        match hash {
            Some(hash) => {
                file.workspaces.insert(key, hash.to_string());
            }
            None => {
                file.workspaces.remove(&key);
            }
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let encoded = serde_json::to_vec_pretty(&file)
            .map_err(|error| Error::Other(format!("encode hook trust: {error}")))?;
        milim_tools::atomic_write(&self.path, &encoded)
    }
}

/// The project settings file of a workspace.
pub(crate) fn project_settings_path(workspace: &Path) -> PathBuf {
    workspace.join(PROJECT_SETTINGS)
}

pub(crate) fn user_settings_path() -> PathBuf {
    milim_core::paths::Paths::resolve().settings_file()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookSource {
    User,
    Project,
}

impl HookSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

#[derive(Debug)]
struct LoadedHook {
    event: &'static str,
    matcher: Option<Regex>,
    command: String,
    timeout: Duration,
    source: HookSource,
}

impl LoadedHook {
    fn matches(&self, event: &str, tool: Option<&str>) -> bool {
        self.event == event
            && match (&self.matcher, tool) {
                (Some(matcher), Some(tool)) => matcher.is_match(tool),
                _ => true,
            }
    }
}

/// Compile a matcher to a whole-name regex. `None` means every tool.
fn compile_matcher(matcher: Option<&str>) -> std::result::Result<Option<Regex>, regex::Error> {
    match matcher.map(str::trim).filter(|matcher| !matcher.is_empty()) {
        None => Ok(None),
        Some(pattern) => Regex::new(&format!("^(?:{pattern})$")).map(Some),
    }
}

fn load_hooks(config: &HooksConfig, source: HookSource, out: &mut Vec<LoadedHook>) {
    for (event, specs) in config.events() {
        for spec in specs {
            let command = spec.command.trim();
            if command.is_empty() {
                continue;
            }
            let matcher = match compile_matcher(spec.matcher.as_deref()) {
                Ok(matcher) => matcher,
                Err(error) => {
                    tracing::warn!("skipping {event} hook with invalid matcher: {error}");
                    continue;
                }
            };
            out.push(LoadedHook {
                event,
                matcher,
                command: command.to_string(),
                timeout: Duration::from_secs(
                    spec.timeout_secs
                        .unwrap_or(DEFAULT_TIMEOUT_SECS)
                        .clamp(1, MAX_TIMEOUT_SECS),
                ),
                source,
            });
        }
    }
}

/// The hooks that apply to one native run, as a loop interceptor.
#[derive(Debug)]
pub(crate) struct UserHooks {
    hooks: Vec<LoadedHook>,
    allow_hooks_to_approve: bool,
    workspace: PathBuf,
    run_id: String,
    thread_id: Option<String>,
    /// Notice that project hooks were skipped until trusted.
    untrusted: Option<HookActivity>,
}

/// The interceptor for a native run in `workspace`, or `None` when no hook
/// applies. Runs without a working folder have no hooks.
pub(crate) fn interceptor(
    workspace: Option<&Path>,
    run_id: &str,
    thread_id: Option<&str>,
) -> Option<Arc<dyn ToolInterceptor>> {
    let hooks = UserHooks::load(
        workspace?,
        &user_settings_path(),
        &HookTrustStore::resolve(),
        run_id,
        thread_id,
    )?;
    Some(Arc::new(hooks))
}

impl UserHooks {
    fn load(
        workspace: &Path,
        user_settings: &Path,
        trust: &HookTrustStore,
        run_id: &str,
        thread_id: Option<&str>,
    ) -> Option<Self> {
        let user = read_settings(user_settings);
        let project = read_settings(&project_settings_path(workspace));
        let mut hooks = Vec::new();
        load_hooks(&user.hooks, HookSource::User, &mut hooks);
        let mut untrusted = None;
        if let Some(raw) = project.raw.as_ref().filter(|_| !project.hooks.is_empty()) {
            let hash = config_hash(raw);
            if trust.is_trusted(workspace, &hash) {
                load_hooks(&project.hooks, HookSource::Project, &mut hooks);
            } else {
                untrusted = Some(HookActivity {
                    event: "*".into(),
                    hook: PROJECT_SETTINGS.into(),
                    source: HookSource::Project.as_str().into(),
                    tool_name: None,
                    call_id: None,
                    outcome: "skipped".into(),
                    duration_ms: 0,
                    message: Some(
                        "Project hooks in .milim/settings.json did not run because this workspace's hooks are not trusted. Review them and trust the workspace to enable them."
                            .into(),
                    ),
                    trust: Some(HookTrustRequest {
                        workspace: workspace_key(workspace),
                        config_hash: hash,
                    }),
                });
            }
        }
        if hooks.is_empty() && untrusted.is_none() {
            return None;
        }
        Some(Self {
            hooks,
            allow_hooks_to_approve: user.allow_hooks_to_approve,
            workspace: workspace.to_path_buf(),
            run_id: run_id.to_string(),
            thread_id: thread_id.map(str::to_string),
            untrusted,
        })
    }

    fn matching<'a>(
        &'a self,
        event: &'a str,
        tool: Option<&'a str>,
    ) -> impl Iterator<Item = &'a LoadedHook> + 'a {
        self.hooks
            .iter()
            .filter(move |hook| hook.matches(event, tool))
    }

    fn payload(&self, event: &str, fields: Value) -> Value {
        let mut payload = json!({
            "event": event,
            "run_id": self.run_id,
            "thread_id": self.thread_id,
            "workspace": self.workspace.display().to_string(),
        });
        if let (Some(payload), Value::Object(fields)) = (payload.as_object_mut(), fields) {
            payload.extend(fields);
        }
        payload
    }

    async fn run(&self, hook: &LoadedHook, payload: &Value) -> HookRun {
        run_hook(
            &hook.command,
            hook.event,
            &self.workspace,
            payload,
            hook.timeout,
        )
        .await
    }
}

fn activity(
    hook: &LoadedHook,
    call: Option<&InterceptedCall<'_>>,
    run: &HookRun,
    outcome: &str,
    message: Option<String>,
) -> HookActivity {
    HookActivity {
        event: hook.event.to_string(),
        hook: truncate_chars(&hook.command, MAX_HOOK_LABEL_CHARS),
        source: hook.source.as_str().to_string(),
        tool_name: call.map(|call| call.name.to_string()),
        call_id: call.and_then(|call| call.call_id.map(str::to_string)),
        outcome: outcome.to_string(),
        duration_ms: run.duration_ms,
        message: message
            .map(|message| truncate_chars(message.trim(), MAX_TIMELINE_CHARS))
            .filter(|message| !message.is_empty()),
        trust: None,
    }
}

/// Timeline outcome for a run that neither exited 0 nor 2.
fn failure_outcome(run: &HookRun) -> Option<(&'static str, String)> {
    match &run.exit {
        HookExit::Code(0) | HookExit::Code(2) => None,
        HookExit::Code(code) => Some(("error", format!("exit code {code}: {}", run.stderr.trim()))),
        HookExit::Signal => Some(("error", "terminated by a signal".to_string())),
        HookExit::Timeout => Some((
            "timeout",
            "timed out; the hook's process tree was killed".to_string(),
        )),
        HookExit::SpawnFailed(error) => Some(("error", error.clone())),
    }
}

/// A PreToolUse hook's stdout decision on exit 0.
#[derive(Debug, PartialEq, Eq)]
enum StdoutDecision {
    None,
    Allow,
    Deny(String),
}

fn parse_decision(stdout: &str) -> StdoutDecision {
    let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) else {
        return StdoutDecision::None;
    };
    match value.get("decision").and_then(Value::as_str) {
        Some("allow") => StdoutDecision::Allow,
        Some("deny") => StdoutDecision::Deny(
            value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        _ => StdoutDecision::None,
    }
}

fn deny_reason(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "Denied by PreToolUse hook.".to_string()
    } else {
        format!(
            "Denied by PreToolUse hook: {}",
            truncate_bytes(reason, MAX_MODEL_BYTES)
        )
    }
}

#[async_trait]
impl ToolInterceptor for UserHooks {
    async fn before_turn(&self, messages: &[ChatMessage]) -> TurnInterception {
        let mut result = TurnInterception {
            activity: self.untrusted.clone().into_iter().collect(),
            ..Default::default()
        };
        let prompt = messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(ChatMessage::text_content)
            .unwrap_or_default();
        let payload = self.payload(USER_PROMPT_SUBMIT, json!({ "prompt": prompt }));
        for hook in self.matching(USER_PROMPT_SUBMIT, None) {
            let run = self.run(hook, &payload).await;
            if let Some((outcome, message)) = failure_outcome(&run) {
                result
                    .activity
                    .push(activity(hook, None, &run, outcome, Some(message)));
            } else if run.exit == HookExit::Code(2) {
                let reason = non_empty_or(&run.stderr, "blocked by a UserPromptSubmit hook");
                result
                    .activity
                    .push(activity(hook, None, &run, "block", Some(reason.clone())));
                result.block = Some(truncate_bytes(&reason, MAX_MODEL_BYTES));
                return result;
            } else if run.stdout.trim().is_empty() {
                result.activity.push(activity(hook, None, &run, "ok", None));
            } else {
                result.activity.push(activity(
                    hook,
                    None,
                    &run,
                    "context",
                    Some(run.stdout.clone()),
                ));
                result
                    .context
                    .push(truncate_bytes(run.stdout.trim(), MAX_MODEL_BYTES));
            }
        }
        result
    }

    async fn before_tool(&self, call: &InterceptedCall<'_>) -> ToolInterception {
        let mut result = ToolInterception::default();
        let mut allowed = false;
        let payload = self.payload(
            PRE_TOOL_USE,
            json!({
                "tool_name": call.name,
                "tool_input": call.arguments,
                "call_id": call.call_id,
            }),
        );
        for hook in self.matching(PRE_TOOL_USE, Some(call.name)) {
            let run = self.run(hook, &payload).await;
            if let Some((outcome, message)) = failure_outcome(&run) {
                result
                    .activity
                    .push(activity(hook, Some(call), &run, outcome, Some(message)));
                continue;
            }
            let decision = if run.exit == HookExit::Code(2) {
                StdoutDecision::Deny(run.stderr.trim().to_string())
            } else {
                parse_decision(&run.stdout)
            };
            match decision {
                StdoutDecision::Deny(reason) => {
                    result.activity.push(activity(
                        hook,
                        Some(call),
                        &run,
                        "deny",
                        Some(reason.clone()),
                    ));
                    result.decision = ToolDecision::Deny(deny_reason(&reason));
                    return result;
                }
                StdoutDecision::Allow => {
                    allowed = true;
                    result.activity.push(activity(
                        hook,
                        Some(call),
                        &run,
                        "allow",
                        (!self.allow_hooks_to_approve).then(|| {
                            "allow_hooks_to_approve is off; the approval policy still applies"
                                .to_string()
                        }),
                    ));
                }
                StdoutDecision::None => {
                    result
                        .activity
                        .push(activity(hook, Some(call), &run, "ok", None));
                }
            }
        }
        if allowed && self.allow_hooks_to_approve {
            result.decision = ToolDecision::Approve;
        }
        result
    }

    async fn after_tool(&self, call: &InterceptedCall<'_>, output: &Value) -> ResultInterception {
        let mut result = ResultInterception::default();
        let mut hooks = self.matching(POST_TOOL_USE, Some(call.name)).peekable();
        if hooks.peek().is_none() {
            return result;
        }
        let payload = self.payload(
            POST_TOOL_USE,
            json!({
                "tool_name": call.name,
                "tool_input": call.arguments,
                "tool_output": output,
                "call_id": call.call_id,
            }),
        );
        for hook in hooks {
            let run = self.run(hook, &payload).await;
            if let Some((outcome, message)) = failure_outcome(&run) {
                result
                    .activity
                    .push(activity(hook, Some(call), &run, outcome, Some(message)));
                continue;
            }
            let text = if run.exit == HookExit::Code(2) {
                &run.stderr
            } else {
                &run.stdout
            };
            if text.trim().is_empty() {
                result
                    .activity
                    .push(activity(hook, Some(call), &run, "ok", None));
            } else {
                result.activity.push(activity(
                    hook,
                    Some(call),
                    &run,
                    "feedback",
                    Some(text.clone()),
                ));
                result
                    .feedback
                    .push(truncate_bytes(text.trim(), MAX_MODEL_BYTES));
            }
        }
        result
    }

    async fn on_stop(&self, final_content: &str, continuations: usize) -> StopInterception {
        let mut result = StopInterception::default();
        let payload = self.payload(
            STOP,
            json!({
                "last_message": final_content,
                "stop_continuations": continuations,
            }),
        );
        for hook in self.matching(STOP, None) {
            let run = self.run(hook, &payload).await;
            if let Some((outcome, message)) = failure_outcome(&run) {
                result
                    .activity
                    .push(activity(hook, None, &run, outcome, Some(message)));
            } else if run.exit == HookExit::Code(2) && result.continue_with.is_none() {
                let feedback = non_empty_or(&run.stderr, "A Stop hook asked you to continue.");
                result.activity.push(activity(
                    hook,
                    None,
                    &run,
                    "continue",
                    Some(feedback.clone()),
                ));
                result.continue_with = Some(truncate_bytes(&feedback, MAX_MODEL_BYTES));
            } else {
                result.activity.push(activity(hook, None, &run, "ok", None));
            }
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HookExit {
    Code(i32),
    Signal,
    Timeout,
    SpawnFailed(String),
}

#[derive(Debug)]
struct HookRun {
    exit: HookExit,
    stdout: String,
    stderr: String,
    duration_ms: u64,
}

/// Milim's own secrets never reach hook processes; the user's unrelated
/// environment is inherited.
fn milim_secret_env_keys() -> Vec<std::ffi::OsString> {
    std::env::vars_os()
        .filter_map(|(key, _)| {
            let name = key.to_str()?;
            let milim_owned = name
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("MILIM_"));
            (milim_owned && milim_mcp_client::secret_env_key(name)).then_some(key)
        })
        .collect()
}

/// PowerShell on Windows, `sh -c` elsewhere, in its own process group.
fn hook_command(command: &str, event: &str, workspace: &Path) -> tokio::process::Command {
    let mut cmd = if cfg!(windows) {
        let mut cmd = tokio::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.args(["-c", command]);
        cmd
    };
    for key in milim_secret_env_keys() {
        cmd.env_remove(key);
    }
    #[cfg(not(windows))]
    cmd.env("PATH", crate::cli_path::search_path());
    cmd.current_dir(workspace)
        .env("MILIM_HOOK_EVENT", event)
        .env("MILIM_WORKSPACE", workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd
}

async fn capture(mut stream: impl AsyncRead + Unpin) -> String {
    let mut kept = Vec::new();
    let mut buffer = [0_u8; 8192];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        let room = MAX_CAPTURE_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..count.min(room)]);
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// Run one hook command with `payload` on stdin, killing its process tree
/// when it outlives `timeout`.
async fn run_hook(
    command: &str,
    event: &str,
    workspace: &Path,
    payload: &Value,
    timeout: Duration,
) -> HookRun {
    let started = Instant::now();
    let finish = |exit: HookExit, stdout: String, stderr: String| HookRun {
        exit,
        stdout,
        stderr,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    let mut child = match hook_command(command, event, workspace).spawn() {
        Ok(child) => child,
        Err(error) => {
            return finish(
                HookExit::SpawnFailed(format!("hook failed to start: {error}")),
                String::new(),
                String::new(),
            )
        }
    };
    let Some(pid) = child.id() else {
        return finish(
            HookExit::SpawnFailed("hook process id unavailable".into()),
            String::new(),
            String::new(),
        );
    };
    let mut guard = match ProcessTreeGuard::attach(pid) {
        Ok(guard) => guard,
        Err(error) => {
            let _ = child.kill().await;
            return finish(
                HookExit::SpawnFailed(format!("failed to contain hook process: {error}")),
                String::new(),
                String::new(),
            );
        }
    };
    let input = serde_json::to_vec(payload).unwrap_or_default();
    let writer = child.stdin.take().map(|mut stdin| {
        tokio::spawn(async move {
            // A hook may exit without reading its input.
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        })
    });
    let stdout = child.stdout.take().map(|out| tokio::spawn(capture(out)));
    let stderr = child.stderr.take().map(|err| tokio::spawn(capture(err)));
    let exit = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status.code().map_or(HookExit::Signal, HookExit::Code),
        Ok(Err(error)) => HookExit::SpawnFailed(format!("hook wait failed: {error}")),
        Err(_) => {
            guard.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
            HookExit::Timeout
        }
    };
    // Background descendants must not outlive the hook or hold its pipes.
    guard.terminate();
    if let Some(writer) = writer {
        writer.abort();
    }
    let collect = |reader: Option<tokio::task::JoinHandle<String>>| async move {
        let Some(reader) = reader else {
            return String::new();
        };
        let abort = reader.abort_handle();
        match tokio::time::timeout(READER_DRAIN, reader).await {
            Ok(Ok(text)) => text,
            _ => {
                abort.abort();
                String::new()
            }
        }
    };
    let (stdout, stderr) = futures::join!(collect(stdout), collect(stderr));
    finish(exit, stdout, stderr)
}

fn non_empty_or(text: &str, fallback: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        fallback.to_string()
    } else {
        text.to_string()
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn truncate_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[… {} more bytes of hook output omitted]",
        &text[..end],
        text.len() - end
    )
}

/// Hook settings for a workspace, as the desktop shows them.
pub(crate) fn status(workspace: Option<&Path>) -> Value {
    let user_path = user_settings_path();
    let user = read_settings(&user_path);
    let trust = HookTrustStore::resolve();
    let project = workspace.map(|workspace| {
        let path = project_settings_path(workspace);
        let settings = read_settings(&path);
        let hash = settings.raw.as_ref().map(config_hash);
        json!({
            "workspace": workspace_key(workspace),
            "path": path.display().to_string(),
            "hooks": settings.hooks,
            "has_hooks": !settings.hooks.is_empty(),
            "config_hash": hash,
            "trusted": hash.as_deref().is_some_and(|hash| trust.is_trusted(workspace, hash)),
        })
    });
    json!({
        "user": {
            "path": user_path.display().to_string(),
            "hooks": user.hooks,
            "allow_hooks_to_approve": user.allow_hooks_to_approve,
        },
        "project": project,
    })
}

/// Trust (or forget) a workspace's project hooks. `reviewed_hash` must match
/// the current config, so a stale review cannot trust changed hooks.
pub(crate) fn set_trust(workspace: &Path, reviewed_hash: &str, trusted: bool) -> Result<Value> {
    let trust = HookTrustStore::resolve();
    if trusted {
        let settings = read_settings(&project_settings_path(workspace));
        let current = settings.raw.as_ref().map(config_hash);
        if current.as_deref() != Some(reviewed_hash) {
            return Err(Error::InvalidRequest(
                "the workspace's hooks changed since they were reviewed; review them again".into(),
            ));
        }
        trust.set(workspace, Some(reviewed_hash))?;
    } else {
        trust.set(workspace, None)?;
    }
    Ok(status(Some(workspace)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "milim-user-hooks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Fixture {
        root: PathBuf,
        workspace: PathBuf,
        user_settings: PathBuf,
        trust: HookTrustStore,
    }

    impl Fixture {
        fn new() -> Self {
            let root = temp_dir();
            let workspace = root.join("workspace");
            std::fs::create_dir_all(workspace.join(".milim")).unwrap();
            Self {
                user_settings: root.join("settings.json"),
                trust: HookTrustStore::new(root.join("config").join(TRUST_FILE)),
                workspace,
                root,
            }
        }

        fn user(&self, settings: Value) -> &Self {
            std::fs::write(&self.user_settings, settings.to_string()).unwrap();
            self
        }

        fn project(&self, settings: Value) -> &Self {
            std::fs::write(project_settings_path(&self.workspace), settings.to_string()).unwrap();
            self
        }

        fn hooks(&self) -> Option<UserHooks> {
            UserHooks::load(
                &self.workspace,
                &self.user_settings,
                &self.trust,
                "run-1",
                Some("thread-1"),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn call<'a>(name: &'a str, arguments: &'a Value) -> InterceptedCall<'a> {
        InterceptedCall {
            call_id: Some("call-1"),
            name,
            arguments,
        }
    }

    #[test]
    fn matchers_match_whole_tool_names() {
        let matcher = compile_matcher(Some("shell|edit_file")).unwrap().unwrap();
        assert!(matcher.is_match("shell"));
        assert!(matcher.is_match("edit_file"));
        assert!(!matcher.is_match("shell_extra"));
        assert!(!matcher.is_match("write_file"));
        let prefix = compile_matcher(Some("mcp__.*")).unwrap().unwrap();
        assert!(prefix.is_match("mcp__github__search"));
        assert!(compile_matcher(Some("  ")).unwrap().is_none());
        assert!(compile_matcher(None).unwrap().is_none());
        assert!(compile_matcher(Some("(")).is_err());

        let fixture = Fixture::new();
        fixture.user(json!({"hooks": {"PreToolUse": [
            {"matcher": "shell", "command": "true"},
            {"command": "true"},
            {"matcher": "(", "command": "true"}
        ]}}));
        let hooks = fixture.hooks().unwrap();
        assert_eq!(hooks.hooks.len(), 2, "the invalid matcher is skipped");
        assert_eq!(hooks.matching(PRE_TOOL_USE, Some("shell")).count(), 2);
        assert_eq!(hooks.matching(PRE_TOOL_USE, Some("read_file")).count(), 1);
        assert_eq!(hooks.matching(POST_TOOL_USE, Some("shell")).count(), 0);
    }

    #[test]
    fn decisions_parse_from_stdout_json() {
        assert_eq!(
            parse_decision(r#"{"decision":"allow"}"#),
            StdoutDecision::Allow
        );
        assert_eq!(
            parse_decision(" {\"decision\":\"deny\",\"reason\":\"no rm\"}\n"),
            StdoutDecision::Deny("no rm".into())
        );
        assert_eq!(parse_decision("formatted"), StdoutDecision::None);
        assert_eq!(
            parse_decision(r#"{"decision":"ask"}"#),
            StdoutDecision::None
        );
        assert_eq!(parse_decision(""), StdoutDecision::None);
    }

    #[test]
    fn trust_hash_is_stable_across_key_order_and_changes_with_content() {
        let a = json!({"PreToolUse": [{"command": "x", "matcher": "shell"}]});
        let b: Value =
            serde_json::from_str(r#"{"PreToolUse": [{"matcher": "shell", "command": "x"}]}"#)
                .unwrap();
        let c = json!({"PreToolUse": [{"command": "y", "matcher": "shell"}]});
        assert_eq!(config_hash(&a), config_hash(&b));
        assert_ne!(config_hash(&a), config_hash(&c));
        assert_eq!(config_hash(&a).len(), 64);
    }

    #[test]
    fn project_hooks_need_trust_for_their_exact_config() {
        let fixture = Fixture::new();
        let settings = json!({"hooks": {"Stop": [{"command": "true"}]}});
        fixture.project(settings.clone());
        let hooks = fixture.hooks().expect("the untrusted notice still applies");
        assert!(hooks.hooks.is_empty());
        let notice = hooks.untrusted.clone().unwrap();
        let trust = notice.trust.unwrap();
        assert_eq!(trust.config_hash, config_hash(&settings["hooks"]));

        fixture
            .trust
            .set(&fixture.workspace, Some(&trust.config_hash))
            .unwrap();
        let hooks = fixture.hooks().unwrap();
        assert!(hooks.untrusted.is_none());
        assert_eq!(hooks.hooks.len(), 1);
        assert_eq!(hooks.hooks[0].source, HookSource::Project);

        fixture.project(json!({"hooks": {"Stop": [{"command": "echo changed"}]}}));
        let hooks = fixture.hooks().unwrap();
        assert!(hooks.hooks.is_empty(), "a changed config needs trust again");
        assert!(hooks.untrusted.is_some());

        fixture.trust.set(&fixture.workspace, None).unwrap();
        assert!(!fixture
            .trust
            .is_trusted(&fixture.workspace, &trust.config_hash));
    }

    #[test]
    fn approval_opt_in_is_read_from_user_settings_only() {
        let fixture = Fixture::new();
        fixture.project(json!({
            "allow_hooks_to_approve": true,
            "hooks": {"Stop": [{"command": "true"}]}
        }));
        fixture.user(json!({"hooks": {"Stop": [{"command": "true"}]}}));
        assert!(!fixture.hooks().unwrap().allow_hooks_to_approve);
        fixture.user(json!({
            "allow_hooks_to_approve": true,
            "hooks": {"Stop": [{"command": "true"}]}
        }));
        assert!(fixture.hooks().unwrap().allow_hooks_to_approve);
        assert!(Fixture::new().hooks().is_none(), "no settings, no hooks");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pre_tool_use_exit_codes_and_decisions() {
        let fixture = Fixture::new();
        fixture.user(json!({"hooks": {"PreToolUse": [
            {"matcher": "shell", "command": "grep -q 'rm -rf' && { echo 'no rm -rf' >&2; exit 2; }; exit 0"},
            {"matcher": "read_file", "command": "echo '{\"decision\":\"allow\"}'"},
            {"matcher": "write_file", "command": "echo '{\"decision\":\"deny\",\"reason\":\"read-only repo\"}'"},
            {"matcher": "edit_file", "command": "echo boom >&2; exit 3"}
        ]}}));
        let hooks = fixture.hooks().unwrap();

        let args = json!({"command": "rm -rf /"});
        let denied = hooks.before_tool(&call("shell", &args)).await;
        assert_eq!(
            denied.decision,
            ToolDecision::Deny("Denied by PreToolUse hook: no rm -rf".into())
        );
        assert_eq!(denied.activity[0].outcome, "deny");
        assert_eq!(denied.activity[0].tool_name.as_deref(), Some("shell"));

        let args = json!({"command": "ls"});
        let allowed = hooks.before_tool(&call("shell", &args)).await;
        assert_eq!(allowed.decision, ToolDecision::Continue);
        assert_eq!(allowed.activity[0].outcome, "ok");

        let args = json!({"path": "a"});
        let allow = hooks.before_tool(&call("read_file", &args)).await;
        assert_eq!(
            allow.decision,
            ToolDecision::Continue,
            "allow approves only with the user opt-in"
        );
        assert_eq!(allow.activity[0].outcome, "allow");

        let deny = hooks.before_tool(&call("write_file", &args)).await;
        assert_eq!(
            deny.decision,
            ToolDecision::Deny("Denied by PreToolUse hook: read-only repo".into())
        );

        let failed = hooks.before_tool(&call("edit_file", &args)).await;
        assert_eq!(
            failed.decision,
            ToolDecision::Continue,
            "errors don't block"
        );
        assert_eq!(failed.activity[0].outcome, "error");
        assert_eq!(
            failed.activity[0].message.as_deref(),
            Some("exit code 3: boom")
        );

        fixture.user(json!({
            "allow_hooks_to_approve": true,
            "hooks": {"PreToolUse": [{"command": "echo '{\"decision\":\"allow\"}'"}]}
        }));
        let hooks = fixture.hooks().unwrap();
        let approve = hooks.before_tool(&call("shell", &args)).await;
        assert_eq!(approve.decision, ToolDecision::Approve);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hooks_receive_the_event_on_stdin_and_environment() {
        let fixture = Fixture::new();
        fixture.user(json!({"hooks": {"PostToolUse": [{
            "command": "cat > payload.json; printf '%s %s' \"$MILIM_HOOK_EVENT\" \"$(basename \"$MILIM_WORKSPACE\")\""
        }]}}));
        let hooks = fixture.hooks().unwrap();
        let args = json!({"path": "a.rs"});
        let result = hooks
            .after_tool(&call("edit_file", &args), &json!({"ok": true}))
            .await;
        assert_eq!(result.feedback, vec!["PostToolUse workspace".to_string()]);
        assert_eq!(result.activity[0].outcome, "feedback");
        let payload: Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.workspace.join("payload.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(payload["event"], "PostToolUse");
        assert_eq!(payload["tool_name"], "edit_file");
        assert_eq!(payload["tool_input"]["path"], "a.rs");
        assert_eq!(payload["tool_output"]["ok"], true);
        assert_eq!(payload["run_id"], "run-1");
        assert_eq!(payload["thread_id"], "thread-1");
        assert_eq!(payload["call_id"], "call-1");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn post_tool_use_feedback_is_capped_and_exit_two_uses_stderr() {
        let fixture = Fixture::new();
        fixture.user(json!({"hooks": {"PostToolUse": [
            {"command": "head -c 20000 /dev/zero | tr '\\0' 'a'"},
            {"command": "echo 'tests failed' >&2; exit 2"},
            {"command": "true"}
        ]}}));
        let hooks = fixture.hooks().unwrap();
        let args = json!({});
        let result = hooks.after_tool(&call("shell", &args), &json!({})).await;
        assert_eq!(result.feedback.len(), 2);
        assert!(result.feedback[0].starts_with(&"a".repeat(MAX_MODEL_BYTES)));
        assert!(result.feedback[0].contains("more bytes of hook output omitted"));
        assert_eq!(result.feedback[1], "tests failed");
        let outcomes = result
            .activity
            .iter()
            .map(|activity| activity.outcome.as_str())
            .collect::<Vec<_>>();
        assert_eq!(outcomes, vec!["feedback", "feedback", "ok"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeouts_kill_the_process_tree() {
        let fixture = Fixture::new();
        let marker = fixture.workspace.join("survived");
        fixture.user(json!({"hooks": {"Stop": [{
            "command": format!("(sleep 3; touch '{}') & sleep 30", marker.display()),
            "timeout_secs": 1
        }]}}));
        let hooks = fixture.hooks().unwrap();
        let started = Instant::now();
        let result = hooks.on_stop("done", 0).await;
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(result.continue_with.is_none());
        assert_eq!(result.activity[0].outcome, "timeout");
        tokio::time::sleep(Duration::from_millis(3_500)).await;
        assert!(!marker.exists(), "the background child was killed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prompt_and_stop_hooks_add_context_block_and_continue() {
        let fixture = Fixture::new();
        fixture.user(json!({"hooks": {
            "UserPromptSubmit": [{"command": "echo 'on branch main'"}],
            "Stop": [{"command": "echo 'run the tests first' >&2; exit 2"}]
        }}));
        let hooks = fixture.hooks().unwrap();
        let turn = hooks
            .before_turn(&[ChatMessage::text("user", "hello")])
            .await;
        assert_eq!(turn.context, vec!["on branch main".to_string()]);
        assert!(turn.block.is_none());
        let stop = hooks.on_stop("done", 0).await;
        assert_eq!(stop.continue_with.as_deref(), Some("run the tests first"));
        assert_eq!(stop.activity[0].outcome, "continue");

        fixture.user(json!({"hooks": {"UserPromptSubmit": [
            {"command": "grep -q deploy && { echo 'no deploys today' >&2; exit 2; }; exit 0"}
        ]}}));
        let hooks = fixture.hooks().unwrap();
        let turn = hooks
            .before_turn(&[ChatMessage::text("user", "deploy it")])
            .await;
        assert_eq!(turn.block.as_deref(), Some("no deploys today"));
        assert_eq!(turn.activity[0].outcome, "block");
    }

    #[tokio::test]
    async fn untrusted_project_hooks_surface_a_notice_and_do_not_run() {
        let fixture = Fixture::new();
        let marker = fixture.workspace.join("ran");
        fixture.project(json!({"hooks": {"UserPromptSubmit": [
            {"command": format!("touch '{}'", marker.display())}
        ]}}));
        let hooks = fixture.hooks().unwrap();
        let turn = hooks.before_turn(&[ChatMessage::text("user", "hi")]).await;
        assert!(!marker.exists());
        assert_eq!(turn.activity.len(), 1);
        assert_eq!(turn.activity[0].outcome, "skipped");
        assert!(turn.activity[0].trust.is_some());
    }
}
