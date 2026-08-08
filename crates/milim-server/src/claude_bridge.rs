//! Thin local bridge to the user's installed official `claude` CLI.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader as StdBufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::{Stream, StreamExt};
use milim_agents::ToolApprovalBroker;
use milim_core::api::openai::{ReasoningEffort, Usage};
use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::account_runtime_events::{
    canonicalize_runtime_stream, serialize_runtime_event, HarnessEvent,
};
use crate::codex_bridge::{
    AccountImage, AccountNativeWorkerLifecycle, AccountNativeWorkerState, AccountWorkerEvent,
};
use crate::privacy::Unredactor;

const CLAUDE_STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const CLAUDE_MODEL_ALIASES: &[&str] = &["sonnet", "opus", "haiku", "fable"];
const CLAUDE_PROJECT_DIR_NAME_LIMIT: usize = 200;
const CLAUDE_THREAD_PAGE_SIZE: usize = 25;
const CLAUDE_THREAD_HEAD_BYTES: u64 = 1024 * 1024;
const CLAUDE_THREAD_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeRunRequest {
    pub prompt: String,
    #[serde(default)]
    pub images: Vec<AccountImage>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub tool_approval_policy: Option<String>,
    #[serde(default)]
    pub tool_approval_grant: bool,
    #[serde(default)]
    pub interactive_tool_approval: bool,
    #[serde(default)]
    pub plan_mode: bool,
    #[serde(default)]
    pub allow_session_recovery: bool,
    #[serde(default)]
    pub milim_context: Option<crate::routes::AccountRuntimeMilimContext>,
    #[serde(skip)]
    pub milim_mcp: Option<crate::routes::AccountRuntimeToolEndpoint>,
    #[serde(skip)]
    pub approval_run_id: Option<String>,
    #[serde(skip)]
    pub approval_mcp_url: Option<String>,
    #[serde(skip)]
    pub approval_mcp_authorization: Option<String>,
    #[serde(skip)]
    approval_mcp_config: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClaudeThreadSummary {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub cwd: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub resumable: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeThreadPage {
    pub data: Vec<ClaudeThreadSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClaudeImportedMessage {
    pub role: &'static str,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClaudeImportedThread {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub resumable: bool,
    pub messages: Vec<ClaudeImportedMessage>,
}

#[derive(Clone)]
struct ClaudeTranscriptFile {
    id: String,
    path: PathBuf,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeTranscriptRecord {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    parent_uuid: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    leaf_uuid: Option<String>,
    #[serde(default)]
    custom_title: Option<String>,
    #[serde(default)]
    ai_title: Option<String>,
    #[serde(default)]
    origin: Option<ClaudeTranscriptOrigin>,
    #[serde(default)]
    message: Option<ClaudeTranscriptMessage>,
}

#[derive(Deserialize)]
struct ClaudeTranscriptOrigin {
    #[serde(default)]
    kind: String,
}

#[derive(Deserialize)]
struct ClaudeTranscriptMessage {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: Option<ClaudeTranscriptContent>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ClaudeTranscriptContent {
    Text(String),
    Blocks(Vec<ClaudeTranscriptBlock>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct ClaudeTranscriptBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

struct ClaudeTranscriptNode {
    parent_uuid: Option<String>,
    visible: Option<ClaudeImportedMessage>,
}

pub(crate) async fn threads(
    cursor: Option<String>,
    search: Option<String>,
    all: bool,
) -> Result<ClaudeThreadPage> {
    tokio::task::spawn_blocking(move || claude_threads_sync(cursor, search, all))
        .await
        .map_err(|error| Error::Other(format!("Claude chat listing task failed: {error}")))?
}

pub(crate) async fn import_thread(session_id: &str) -> Result<ClaudeImportedThread> {
    let session_id = session_id.trim().to_string();
    if uuid::Uuid::parse_str(&session_id).is_err() {
        return Err(Error::InvalidRequest(
            "Claude session id must be a UUID".to_string(),
        ));
    }
    tokio::task::spawn_blocking(move || {
        let transcript = claude_transcript_files()
            .into_iter()
            .find(|file| file.id == session_id)
            .ok_or_else(|| Error::ModelNotFound("Claude chat not found".to_string()))?;
        import_claude_transcript(&transcript)
    })
    .await
    .map_err(|error| Error::Other(format!("Claude chat import task failed: {error}")))?
}

fn claude_threads_sync(
    cursor: Option<String>,
    search: Option<String>,
    all: bool,
) -> Result<ClaudeThreadPage> {
    let offset = match clean_optional(cursor.as_deref()) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| Error::InvalidRequest("Claude chat cursor is invalid".to_string()))?,
        None => 0,
    };
    let search = clean_optional(search.as_deref()).map(|value| value.to_lowercase());
    let summaries = claude_transcript_files()
        .iter()
        .map(claude_thread_summary)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    Ok(page_claude_threads(
        summaries,
        offset,
        search.as_deref(),
        all,
    ))
}

fn page_claude_threads(
    mut summaries: Vec<ClaudeThreadSummary>,
    offset: usize,
    search: Option<&str>,
    all: bool,
) -> ClaudeThreadPage {
    if let Some(search) = search {
        summaries.retain(|thread| {
            thread.title.to_lowercase().contains(search)
                || thread.preview.to_lowercase().contains(search)
                || thread
                    .cwd
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(search)
        });
    }
    summaries.sort_by(|left, right| {
        right
            .updated_at_ms
            .cmp(&left.updated_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    let end = if all {
        summaries.len()
    } else {
        offset
            .saturating_add(CLAUDE_THREAD_PAGE_SIZE)
            .min(summaries.len())
    };
    let data = if offset < summaries.len() {
        summaries[offset..end].to_vec()
    } else {
        Vec::new()
    };
    ClaudeThreadPage {
        data,
        next_cursor: (!all && end < summaries.len()).then(|| end.to_string()),
    }
}

fn claude_transcript_files() -> Vec<ClaudeTranscriptFile> {
    let mut by_id = HashMap::<String, ClaudeTranscriptFile>::new();
    for projects_dir in claude_projects_dirs() {
        for file in claude_transcript_files_in(&projects_dir) {
            let replace = by_id
                .get(&file.id)
                .map(|existing| file.updated_at_ms > existing.updated_at_ms)
                .unwrap_or(true);
            if replace {
                by_id.insert(file.id.clone(), file);
            }
        }
    }
    by_id.into_values().collect()
}

fn claude_transcript_files_in(projects_dir: &Path) -> Vec<ClaudeTranscriptFile> {
    let mut files = Vec::new();
    let Ok(projects) = std::fs::read_dir(projects_dir) else {
        return files;
    };
    for project in projects.flatten() {
        if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|value| value.to_str())
                .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let updated_at_ms = system_time_ms(metadata.modified().ok()).unwrap_or_default();
            let created_at_ms = system_time_ms(metadata.created().ok()).unwrap_or(updated_at_ms);
            files.push(ClaudeTranscriptFile {
                id,
                path,
                created_at_ms,
                updated_at_ms,
            });
        }
    }
    files
}

fn system_time_ms(value: Option<SystemTime>) -> Option<u64> {
    value?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

fn claude_thread_summary(transcript: &ClaudeTranscriptFile) -> Result<Option<ClaudeThreadSummary>> {
    let mut cwd = None;
    let mut first_prompt = None;
    let mut custom_title = None;
    let mut ai_title = None;
    let file = File::open(&transcript.path)?;
    let mut reader = StdBufReader::new(file);
    let mut bytes = 0_u64;
    let mut line = String::new();
    while bytes < CLAUDE_THREAD_HEAD_BYTES {
        line.clear();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        if let Ok(record) = serde_json::from_str::<ClaudeTranscriptRecord>(&line) {
            apply_claude_summary_record(
                &record,
                &mut cwd,
                &mut first_prompt,
                &mut custom_title,
                &mut ai_title,
            );
        }
    }
    for line in claude_tail_lines(&transcript.path)? {
        if let Ok(record) = serde_json::from_str::<ClaudeTranscriptRecord>(&line) {
            apply_claude_summary_record(
                &record,
                &mut cwd,
                &mut first_prompt,
                &mut custom_title,
                &mut ai_title,
            );
        }
    }
    let Some(preview) = first_prompt.map(|value| compact_claude_text(&value, 160)) else {
        return Ok(None);
    };
    let title = custom_title
        .or(ai_title)
        .map(|value| compact_claude_text(&value, 100))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| compact_claude_text(&preview, 100));
    let cwd = cwd.filter(|value| Path::new(value).is_absolute());
    let resumable = cwd
        .as_deref()
        .is_some_and(|value| claude_transcript_resumable(transcript, value));
    Ok(Some(ClaudeThreadSummary {
        id: transcript.id.clone(),
        title,
        preview,
        cwd,
        created_at_ms: transcript.created_at_ms,
        updated_at_ms: transcript.updated_at_ms,
        resumable,
    }))
}

fn apply_claude_summary_record(
    record: &ClaudeTranscriptRecord,
    cwd: &mut Option<String>,
    first_prompt: &mut Option<String>,
    custom_title: &mut Option<String>,
    ai_title: &mut Option<String>,
) {
    if cwd.is_none() {
        *cwd = clean_optional(record.cwd.as_deref());
    }
    if first_prompt.is_none() {
        if let Some(message) = visible_claude_message(record).filter(|item| item.role == "user") {
            *first_prompt = Some(message.content);
        }
    }
    if let Some(value) = clean_optional(record.custom_title.as_deref()) {
        *custom_title = Some(value);
    }
    if let Some(value) = clean_optional(record.ai_title.as_deref()) {
        *ai_title = Some(value);
    }
}

fn claude_tail_lines(path: &Path) -> Result<Vec<String>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(CLAUDE_THREAD_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let text = if start == 0 {
        text.as_ref()
    } else {
        text.split_once('\n')
            .map(|(_, tail)| tail)
            .unwrap_or_default()
    };
    Ok(text.lines().map(str::to_string).collect())
}

fn claude_transcript_resumable(transcript: &ClaudeTranscriptFile, cwd: &str) -> bool {
    let cwd = Path::new(cwd);
    if !cwd.is_dir() {
        return false;
    }
    let Some(project_name) = transcript
        .path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
    else {
        return false;
    };
    let expected = claude_project_dir_name(&cwd.to_string_lossy());
    project_name == expected
        || (expected.len() > CLAUDE_PROJECT_DIR_NAME_LIMIT
            && project_name
                .starts_with(&format!("{}-", &expected[..CLAUDE_PROJECT_DIR_NAME_LIMIT])))
}

fn import_claude_transcript(transcript: &ClaudeTranscriptFile) -> Result<ClaudeImportedThread> {
    let file = File::open(&transcript.path)?;
    let reader = StdBufReader::new(file);
    let mut nodes = HashMap::<String, ClaudeTranscriptNode>::new();
    let mut last_uuid = None;
    let mut leaf_uuid = None;
    let mut cwd = None;
    let mut custom_title = None;
    let mut ai_title = None;
    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<ClaudeTranscriptRecord>(&line) else {
            continue;
        };
        if cwd.is_none() {
            cwd = clean_optional(record.cwd.as_deref());
        }
        if let Some(value) = clean_optional(record.custom_title.as_deref()) {
            custom_title = Some(value);
        }
        if let Some(value) = clean_optional(record.ai_title.as_deref()) {
            ai_title = Some(value);
        }
        if record.kind == "last-prompt" {
            if let Some(value) = clean_optional(record.leaf_uuid.as_deref()) {
                leaf_uuid = Some(value);
            }
        }
        let Some(uuid) = clean_optional(record.uuid.as_deref()) else {
            continue;
        };
        last_uuid = Some(uuid.clone());
        let visible = (!record.is_sidechain)
            .then(|| visible_claude_message(&record))
            .flatten();
        nodes.insert(
            uuid,
            ClaudeTranscriptNode {
                parent_uuid: clean_optional(record.parent_uuid.as_deref()),
                visible,
            },
        );
    }
    let mut current = leaf_uuid
        .filter(|value| nodes.contains_key(value))
        .or(last_uuid);
    let mut branch = Vec::<ClaudeImportedMessage>::new();
    let mut visited = HashSet::new();
    while let Some(uuid) = current {
        if !visited.insert(uuid.clone()) {
            break;
        }
        let Some(node) = nodes.get(&uuid) else {
            break;
        };
        if let Some(message) = &node.visible {
            branch.push(message.clone());
        }
        current = node.parent_uuid.clone();
    }
    branch.reverse();
    let mut messages = Vec::<ClaudeImportedMessage>::new();
    for message in branch {
        if message.role == "assistant" {
            if let Some(previous) = messages.last_mut().filter(|item| item.role == "assistant") {
                previous.content.push_str("\n\n");
                previous.content.push_str(&message.content);
                continue;
            }
        }
        messages.push(message);
    }
    if messages.is_empty() {
        return Err(Error::InvalidRequest(
            "This Claude chat has no importable user or assistant messages".to_string(),
        ));
    }
    let first_prompt = messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| compact_claude_text(&message.content, 100));
    let title = custom_title
        .or(ai_title)
        .map(|value| compact_claude_text(&value, 100))
        .filter(|value| !value.is_empty())
        .or(first_prompt)
        .unwrap_or_else(|| "Imported Claude chat".to_string());
    let cwd = cwd.filter(|value| Path::new(value).is_absolute());
    let resumable = cwd
        .as_deref()
        .is_some_and(|value| claude_transcript_resumable(transcript, value));
    Ok(ClaudeImportedThread {
        id: transcript.id.clone(),
        title,
        cwd,
        created_at_ms: transcript.created_at_ms,
        updated_at_ms: transcript.updated_at_ms,
        resumable,
        messages,
    })
}

fn visible_claude_message(record: &ClaudeTranscriptRecord) -> Option<ClaudeImportedMessage> {
    if record
        .origin
        .as_ref()
        .is_some_and(|origin| origin.kind == "task-notification")
    {
        return None;
    }
    let message = record.message.as_ref()?;
    let role = match (record.kind.as_str(), message.role.as_str()) {
        ("user", "user") => "user",
        ("assistant", "assistant") => "assistant",
        _ => return None,
    };
    let (text, omitted_media) = match message.content.as_ref()? {
        ClaudeTranscriptContent::Text(value) => {
            let value = if role == "user" {
                normalize_claude_user_text(value)
            } else {
                clean_optional(Some(value))
            };
            (value.unwrap_or_default(), false)
        }
        ClaudeTranscriptContent::Blocks(blocks) => {
            let text = blocks
                .iter()
                .filter(|block| block.kind == "text")
                .filter_map(|block| clean_optional(block.text.as_deref()))
                .collect::<Vec<_>>()
                .join("\n");
            let omitted_media = role == "user"
                && blocks.iter().any(|block| {
                    matches!(block.kind.as_str(), "image" | "audio" | "document" | "file")
                });
            (text, omitted_media)
        }
        ClaudeTranscriptContent::Other(_) => return None,
    };
    let content = if !text.trim().is_empty() {
        text
    } else if omitted_media {
        "[Media omitted during Claude import]".to_string()
    } else {
        return None;
    };
    Some(ClaudeImportedMessage { role, content })
}

fn normalize_claude_user_text(value: &str) -> Option<String> {
    if let Some(command) = xml_tag_value(value, "command-name") {
        let args = xml_tag_value(value, "command-args");
        return Some(match args {
            Some(args) if !args.is_empty() => format!("{command} {args}"),
            _ => command,
        });
    }
    let value = value.trim();
    if value.starts_with("<local-command") {
        return None;
    }
    clean_optional(Some(value))
}

fn xml_tag_value(value: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{tag}>");
    let end_tag = format!("</{tag}>");
    let start = value.find(&start_tag)? + start_tag.len();
    let end = value[start..].find(&end_tag)? + start;
    clean_optional(Some(&value[start..end]))
}

fn compact_claude_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    format!(
        "{}…",
        compact
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>()
    )
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeStreamEvent {
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        icon: Option<String>,
    },
    RateLimit {
        limit: Value,
    },
    Done {
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Warning {
        message: String,
    },
    ProtocolNotice {
        kind: &'static str,
        message: String,
    },
    SessionRecoveryRequired {
        message: String,
    },
    ToolApprovalRequired {
        approval_id: String,
        call_id: String,
        name: String,
        arguments: String,
        effect: milim_tools::ToolEffect,
    },
    ToolApprovalResolved {
        approval_id: String,
        call_id: String,
        decision: &'static str,
    },
    ToolApprovalStatus {
        approval_id: String,
        call_id: String,
        decision: Option<&'static str>,
        status: &'static str,
    },
    ToolApprovalFailed {
        approval_id: String,
        call_id: String,
        decision: Option<&'static str>,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct ClaudeToolState {
    name: String,
    detail: Option<String>,
}

pub(crate) async fn status() -> Result<Value> {
    let mut command = claude_command();
    command.arg("auth").arg("status");
    #[cfg(windows)]
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    let output =
        match tokio::time::timeout(CLAUDE_STATUS_TIMEOUT, crate::child_process::output(command))
            .await
        {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                let error = claude_spawn_error_message(&e);
                let warning = is_cli_path_warning(&error);
                return Ok(json!({
                    "available": false,
                    "authenticated": false,
                    "models": [],
                    "error": error,
                    "warning": warning
                }));
            }
            Err(_) => {
                return Ok(json!({
                    "available": true,
                    "authenticated": false,
                    "models": [],
                    "error": "`claude auth status` timed out"
                }));
            }
        };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let auth = serde_json::from_str::<Value>(&stdout).unwrap_or_else(|_| json!({ "raw": stdout }));
    let authenticated = output.status.success()
        && auth
            .get("loggedIn")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    Ok(json!({
        "available": true,
        "authenticated": authenticated,
        "auth": auth,
        "models": if authenticated { CLAUDE_MODEL_ALIASES } else { &[] as &[&str] },
        "model_capabilities": if authenticated {
            json!(CLAUDE_MODEL_ALIASES.iter().map(|model| (model.to_string(), json!({ "image_input": true }))).collect::<serde_json::Map<_, _>>())
        } else {
            json!({})
        },
        "error": if output.status.success() { Value::Null } else { Value::String(stderr) },
    }))
}

pub(crate) fn run_stream(
    req: ClaudeRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = HarnessEvent> {
    let initial_session_id = req.session_id.clone();
    canonicalize_runtime_stream(
        native_event_stream(req, redactions, approval_broker),
        initial_session_id,
    )
}

fn native_event_stream(
    mut req: ClaudeRunRequest,
    redactions: BTreeMap<String, String>,
    approval_broker: Option<std::sync::Arc<ToolApprovalBroker>>,
) -> impl Stream<Item = Value> {
    async_stream::stream! {
        let _approval_config = match ClaudeApprovalConfig::materialize(&req) {
            Ok(config) => config,
            Err(error) => {
                yield runtime_event(&ClaudeStreamEvent::Error {
                    message: error.to_string(),
                    usage: None,
                    cost_usd: None,
                });
                return;
            }
        };
        if let Some(config) = &_approval_config {
            req.approval_mcp_config = Some(config.path.clone());
        }
        let approval_run_id = req.approval_run_id.clone();
        let mut approval_rx = approval_broker.as_ref().map(|broker| broker.subscribe());
        let (worker_tx, mut worker_rx) = tokio::sync::mpsc::unbounded_channel();
        let stream = run_stream_with_worker_events(req, redactions, Some(worker_tx));
        futures::pin_mut!(stream);
        loop {
            let (event, notice) = if let Some(receiver) = approval_rx.as_mut() {
                tokio::select! {
                    event = stream.next() => (event, None),
                    notice = receiver.recv() => (None, Some(notice)),
                }
            } else {
                (stream.next().await, None)
            };
            match (event, notice) {
                (Some(event), _) => {
                    if let (Some(broker), Some(run_id)) =
                        (approval_broker.as_ref(), approval_run_id.as_deref())
                    {
                        broker.acknowledge_run(run_id);
                    }
                    yield event
                },
                (None, Some(Ok(notice))) if Some(notice.run_id.as_str()) == approval_run_id.as_deref() => {
                    let call_id = notice.call_id.unwrap_or_else(|| notice.approval_id.clone());
                    match notice.state {
                        milim_agents::ApprovalState::Requested => {
                            yield runtime_event(&ClaudeStreamEvent::ToolApprovalRequired {
                                approval_id: notice.approval_id,
                                call_id,
                                name: notice.name,
                                arguments: notice.arguments,
                                effect: notice.effect,
                            });
                        }
                        milim_agents::ApprovalState::Decided
                        | milim_agents::ApprovalState::Delivered => {
                            yield runtime_event(&ClaudeStreamEvent::ToolApprovalStatus {
                                approval_id: notice.approval_id,
                                call_id,
                                decision: notice.decision,
                                status: if notice.state == milim_agents::ApprovalState::Decided {
                                    "decided"
                                } else {
                                    "delivered"
                                },
                            });
                        }
                        milim_agents::ApprovalState::Acknowledged => {
                            yield runtime_event(&ClaudeStreamEvent::ToolApprovalResolved {
                                approval_id: notice.approval_id,
                                call_id,
                                decision: notice.decision.unwrap_or("deny"),
                            });
                        }
                        milim_agents::ApprovalState::Failed
                        | milim_agents::ApprovalState::Canceled => {
                            yield runtime_event(&ClaudeStreamEvent::ToolApprovalFailed {
                                approval_id: notice.approval_id,
                                call_id,
                                decision: notice.decision,
                                message: notice.error.unwrap_or_else(|| "Approval delivery failed".to_string()),
                            });
                        }
                    }
                }
                (None, Some(Ok(_)))
                | (None, Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(_)))) => {}
                (None, Some(Err(tokio::sync::broadcast::error::RecvError::Closed))) => approval_rx = None,
                (None, None) => break,
            }
            while let Ok(worker) = worker_rx.try_recv() {
                if matches!(worker, AccountWorkerEvent::NativeWorker { .. }) {
                    yield runtime_event(&worker);
                }
            }
        }
    }
}

struct ClaudeApprovalConfig {
    path: PathBuf,
}

impl ClaudeApprovalConfig {
    fn materialize(req: &ClaudeRunRequest) -> Result<Option<Self>> {
        let needs_approval = claude_interactive_tool_approval(req);
        if !needs_approval && req.milim_mcp.is_none() {
            return Ok(None);
        }
        let path = std::env::temp_dir().join(format!(
            "milim-claude-approval-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut servers = serde_json::Map::new();
        if let Some(endpoint) = &req.milim_mcp {
            servers.insert(
                "milim".into(),
                json!({
                    "type": "http",
                    "url": endpoint.url,
                    "headers": { "Authorization": endpoint.authorization }
                }),
            );
        }
        if needs_approval {
            let url = req.approval_mcp_url.as_deref().ok_or_else(|| {
                Error::InvalidRequest(
                    "Claude Review mode requires Milim's interactive approval endpoint".to_string(),
                )
            })?;
            let mut server = json!({ "type": "http", "url": url });
            if let Some(authorization) = clean_optional(req.approval_mcp_authorization.as_deref()) {
                server["headers"] = json!({ "Authorization": authorization });
            }
            servers.insert("milim_approval".into(), server);
        }
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({ "mcpServers": servers }))?,
        )
        .map_err(|error| {
            Error::Other(format!(
                "failed to create Claude approval configuration: {error}"
            ))
        })?;
        Ok(Some(Self { path }))
    }
}

impl Drop for ClaudeApprovalConfig {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn run_stream_with_worker_events(
    req: ClaudeRunRequest,
    redactions: BTreeMap<String, String>,
    worker_events: Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
) -> impl Stream<Item = Value> {
    async_stream::stream! {
        let mut retried_locked_session = false;
        loop {
            let mut command = claude_command();
            for arg in claude_run_args(&req) {
                command.arg(arg);
            }
            if worker_events.is_some() {
                for denied in ["Agent", "Task"] {
                    command.arg("--disallowedTools").arg(denied);
                }
            }
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            if let Some(cwd) = clean_optional(req.cwd.as_deref()) {
                command.current_dir(cwd);
            }
            #[cfg(windows)]
            command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
            #[cfg(unix)]
            command.process_group(0);

            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(e) => {
                    let message = claude_spawn_error_message(&e);
                    if is_cli_path_warning(&message) {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Warning { message }, &worker_events);
                    } else {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                            message,
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    return;
                }
            };
            let _tree = match child.id().map(ProcessTreeGuard::attach) {
                Some(Ok(tree)) => tree,
                Some(Err(e)) => {
                    yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                        message: format!("failed to contain Claude CLI: {e}"),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    return;
                }
                None => {
                    yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                        message: "Claude CLI process id was not available".to_string(),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    return;
                }
            };
            let Some(mut stdin) = child.stdin.take() else {
                yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                    message: "claude stdin was not available".to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                return;
            };
            let input = match claude_stream_input(&req) {
                Ok(input) => input,
                Err(e) => {
                    yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                        message: e.to_string(),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    return;
                }
            };
            if let Err(e) = stdin.write_all(format!("{input}\n").as_bytes()).await {
                yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                    message: format!("failed to send Claude multimodal input: {e}"),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                return;
            }
            let _ = stdin.shutdown().await;
            drop(stdin);
            let Some(stdout) = child.stdout.take() else {
                yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                    message: "claude stdout was not available".to_string(),
                    usage: None,
                    cost_usd: None,
                }, &worker_events);
                return;
            };
            let stderr = child.stderr.take();
            let stderr_task = tokio::spawn(async move {
                let mut text = String::new();
                if let Some(stderr) = stderr {
                    let _ = BufReader::new(stderr).read_to_string(&mut text).await;
                }
                text
            });

            let mut lines = BufReader::new(stdout).lines();
            let mut content = Unredactor::new(redactions.clone());
            let mut reasoning = Unredactor::new(redactions.clone());
            let mut tools = BTreeMap::new();
            let mut saw_terminal_event = false;
            let mut locked_session_error = None;

            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if let Ok(value) = serde_json::from_str::<Value>(&line) {
                            if let Some(lifecycle) = claude_native_worker_event(&value) {
                                publish_worker(&worker_events, AccountWorkerEvent::NativeWorker { lifecycle });
                            }
                        }
                        for event in handle_line(&line, &mut content, &mut reasoning, &mut tools, &mut saw_terminal_event) {
                            if matches!(&event, ClaudeStreamEvent::Error { message, .. } if claude_session_in_use_error(message) && !retried_locked_session)
                            {
                                if let ClaudeStreamEvent::Error { message, .. } = event {
                                    locked_session_error = Some(message);
                                }
                            } else {
                                yield runtime_event_with_worker(&event, &worker_events);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                            message: format!("claude stream failed: {e}"),
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                        return;
                    }
                }

            }

            let status = child.wait().await;
            let stderr = stderr_task.await.unwrap_or_default();
            let tail = content.flush();
            if !tail.is_empty() {
                yield runtime_event_with_worker(&ClaudeStreamEvent::Token { text: tail }, &worker_events);
            }
            let rtail = reasoning.flush();
            if !rtail.is_empty() {
                yield runtime_event_with_worker(&ClaudeStreamEvent::Reasoning { text: rtail }, &worker_events);
            }

            match status {
                Ok(status) if status.success() => {
                    if let Some(message) = locked_session_error {
                        if maybe_recover_locked_session(&req, &mut retried_locked_session).await {
                            yield runtime_event_with_worker(&ClaudeStreamEvent::Warning {
                                message: "Claude session was already in use; Milim stopped the matching local Claude CLI process and retried.".to_string(),
                            }, &worker_events);
                            continue;
                        }
                        yield locked_session_error_event(&req, message);
                    } else if !saw_terminal_event {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                            message: claude_missing_terminal_error(&stderr),
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    break;
                }
                Ok(status) => {
                    let message = locked_session_error
                        .unwrap_or_else(|| first_error(&stderr, &format!("claude exited with {status}")));
                    if claude_session_in_use_error(&message)
                        && maybe_recover_locked_session(&req, &mut retried_locked_session).await
                    {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Warning {
                            message: "Claude session was already in use; Milim stopped the matching local Claude CLI process and retried.".to_string(),
                        }, &worker_events);
                        continue;
                    }
                    if claude_session_in_use_error(&message) {
                        yield locked_session_error_event(&req, message);
                    } else {
                        yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                            message,
                            usage: None,
                            cost_usd: None,
                        }, &worker_events);
                    }
                    break;
                }
                Err(e) => {
                    yield runtime_event_with_worker(&ClaudeStreamEvent::Error {
                        message: format!("claude exit status failed: {e}"),
                        usage: None,
                        cost_usd: None,
                    }, &worker_events);
                    break;
                }
            }
        }
    }
}

async fn maybe_recover_locked_session(
    req: &ClaudeRunRequest,
    retried_locked_session: &mut bool,
) -> bool {
    if *retried_locked_session || !claude_session_recovery_allowed(req) {
        return false;
    }
    let Some(session_id) = clean_optional(req.session_id.as_deref()) else {
        return false;
    };
    if terminate_claude_session_processes(&session_id).await {
        *retried_locked_session = true;
        true
    } else {
        false
    }
}

fn locked_session_error_event(req: &ClaudeRunRequest, message: String) -> Value {
    if claude_session_recovery_allowed(req) {
        return runtime_event(&ClaudeStreamEvent::Error {
            message,
            usage: None,
            cost_usd: None,
        });
    }
    runtime_event(&ClaudeStreamEvent::SessionRecoveryRequired {
        message: format!(
            "This Claude session appears to be in use by another Claude CLI process. Milim can try to stop the matching local Claude process and retry, or you can cancel and resume manually. Claude reported: {message}"
        ),
    })
}

fn handle_line(
    line: &str,
    content: &mut Unredactor,
    reasoning: &mut Unredactor,
    tools: &mut BTreeMap<String, ClaudeToolState>,
    saw_terminal_event: &mut bool,
) -> Vec<ClaudeStreamEvent> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return vec![ClaudeStreamEvent::ProtocolNotice {
            kind: "invalid_json",
            message: "Claude emitted invalid stream JSON".to_string(),
        }];
    };
    let mut out = Vec::new();
    if value.get("type").and_then(Value::as_str) == Some("stream_event") {
        let event = value.get("event").unwrap_or(&Value::Null);
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let block = event.get("content_block").unwrap_or(&Value::Null);
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    if let Some(event) = claude_tool_start_event(block, tools) {
                        out.push(event);
                    }
                }
            }
            Some("content_block_delta") => {
                let delta = event.get("delta").unwrap_or(&Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            let text = content.push(text);
                            if !text.is_empty() {
                                out.push(ClaudeStreamEvent::Token { text });
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            let text = reasoning.push(text);
                            if !text.is_empty() {
                                out.push(ClaudeStreamEvent::Reasoning { text });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some(
                "message_start" | "content_block_stop" | "message_delta" | "message_stop" | "ping",
            ) => {}
            Some(kind) => out.push(ClaudeStreamEvent::ProtocolNotice {
                kind: "unsupported_stream_event",
                message: format!("Claude emitted unsupported stream event {kind}"),
            }),
            None => {}
        }
    } else if value.get("type").and_then(Value::as_str) == Some("user") {
        if let Some(blocks) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
        {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let Some(id) = string_field(block, "tool_use_id") else {
                    continue;
                };
                let Some(tool) = tools.remove(&id) else {
                    continue;
                };
                out.push(claude_tool_end_event(
                    id,
                    tool,
                    block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                ));
            }
        }
    } else if value.get("type").and_then(Value::as_str) == Some("result") {
        *saw_terminal_event = true;
        let tail = content.flush();
        if !tail.is_empty() {
            out.push(ClaudeStreamEvent::Token { text: tail });
        }
        let rtail = reasoning.flush();
        if !rtail.is_empty() {
            out.push(ClaudeStreamEvent::Reasoning { text: rtail });
        }
        let status = value
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_string();
        let is_error = value
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.extend(close_claude_tools(tools, is_error));
        let usage = usage_from_claude_result(&value);
        let cost_usd = value.get("total_cost_usd").and_then(Value::as_f64);
        if is_error {
            let message = value
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("claude run failed")
                .to_string();
            out.push(ClaudeStreamEvent::Error {
                message,
                usage,
                cost_usd,
            });
        } else {
            out.push(ClaudeStreamEvent::Done {
                status,
                usage,
                cost_usd,
            });
        }
    } else if value.get("type").and_then(Value::as_str) == Some("rate_limit_event") {
        out.push(ClaudeStreamEvent::RateLimit {
            limit: claude_rate_limit(value.get("rate_limit_info").unwrap_or(&Value::Null)),
        });
    } else if value.get("type").and_then(Value::as_str) == Some("tool_progress") {
        if let Some(event) = claude_tool_progress_event(&value, tools) {
            out.push(event);
        }
    } else if !matches!(
        value.get("type").and_then(Value::as_str),
        Some("system" | "assistant") | None
    ) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("missing");
        out.push(ClaudeStreamEvent::ProtocolNotice {
            kind: "unsupported_event",
            message: format!("Claude emitted unsupported event {kind}"),
        });
    }
    out
}

fn claude_tool_start_event(
    block: &Value,
    tools: &mut BTreeMap<String, ClaudeToolState>,
) -> Option<ClaudeStreamEvent> {
    let id = string_field(block, "id")?;
    let name = string_field(block, "name").unwrap_or_else(|| "tool".to_string());
    if tools.contains_key(&id) {
        return None;
    }
    let detail = compact_json(block.get("input"));
    tools.insert(
        id.clone(),
        ClaudeToolState {
            name: name.clone(),
            detail: detail.clone(),
        },
    );
    Some(ClaudeStreamEvent::Tool {
        id,
        name: name.clone(),
        status: "running".to_string(),
        label: Some(format!("Using {name}")),
        detail,
        icon: Some(claude_tool_icon(&name).to_string()),
    })
}

fn claude_tool_progress_event(
    value: &Value,
    tools: &mut BTreeMap<String, ClaudeToolState>,
) -> Option<ClaudeStreamEvent> {
    let id = string_field(value, "tool_use_id")?;
    let name = string_field(value, "tool_name").unwrap_or_else(|| "tool".to_string());
    if tools.contains_key(&id) {
        return None;
    }
    tools.insert(
        id.clone(),
        ClaudeToolState {
            name: name.clone(),
            detail: None,
        },
    );
    Some(ClaudeStreamEvent::Tool {
        id,
        name: name.clone(),
        status: "running".to_string(),
        label: Some(format!("Using {name}")),
        detail: None,
        icon: Some(claude_tool_icon(&name).to_string()),
    })
}

fn close_claude_tools(
    tools: &mut BTreeMap<String, ClaudeToolState>,
    is_error: bool,
) -> Vec<ClaudeStreamEvent> {
    std::mem::take(tools)
        .into_iter()
        .map(|(id, tool)| claude_tool_end_event(id, tool, is_error))
        .collect()
}

fn claude_tool_end_event(id: String, tool: ClaudeToolState, is_error: bool) -> ClaudeStreamEvent {
    ClaudeStreamEvent::Tool {
        id,
        name: tool.name.clone(),
        status: if is_error { "error" } else { "done" }.to_string(),
        label: Some(if is_error {
            format!("{} failed", tool.name)
        } else {
            format!("Used {}", tool.name)
        }),
        detail: tool.detail,
        icon: Some(claude_tool_icon(&tool.name).to_string()),
    }
}

fn claude_tool_icon(name: &str) -> &'static str {
    match name {
        "Read" | "Grep" | "Glob" | "Write" | "Edit" | "MultiEdit" => "file",
        "Bash" => "command",
        _ => "tool",
    }
}

fn runtime_event<T: Serialize>(value: &T) -> Value {
    serialize_runtime_event(value)
}

fn runtime_event_with_worker(
    value: &ClaudeStreamEvent,
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
) -> Value {
    if let Some(event) = account_worker_event_from_claude(value) {
        publish_worker(worker_events, event);
    }
    runtime_event(value)
}

fn publish_worker(
    worker_events: &Option<tokio::sync::mpsc::UnboundedSender<AccountWorkerEvent>>,
    event: AccountWorkerEvent,
) {
    if let Some(worker_events) = worker_events {
        let _ = worker_events.send(event);
    }
}

fn account_worker_event_from_claude(value: &ClaudeStreamEvent) -> Option<AccountWorkerEvent> {
    match value {
        ClaudeStreamEvent::Token { text } => Some(AccountWorkerEvent::Token { text: text.clone() }),
        ClaudeStreamEvent::Reasoning { text } => {
            Some(AccountWorkerEvent::Reasoning { text: text.clone() })
        }
        ClaudeStreamEvent::Tool {
            id,
            name,
            status,
            label,
            detail,
            icon,
        } => Some(AccountWorkerEvent::Tool {
            id: id.clone(),
            name: name.clone(),
            status: status.clone(),
            label: label.clone(),
            detail: detail.clone(),
            icon: icon.clone(),
        }),
        ClaudeStreamEvent::Done {
            status,
            usage,
            cost_usd,
        } => Some(AccountWorkerEvent::Done {
            status: status.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        ClaudeStreamEvent::Error {
            message,
            usage,
            cost_usd,
        } => Some(AccountWorkerEvent::Error {
            message: message.clone(),
            usage: *usage,
            cost_usd: *cost_usd,
        }),
        ClaudeStreamEvent::Warning { message }
        | ClaudeStreamEvent::SessionRecoveryRequired { message } => {
            Some(AccountWorkerEvent::Warning {
                message: message.clone(),
            })
        }
        ClaudeStreamEvent::RateLimit { .. }
        | ClaudeStreamEvent::ProtocolNotice { .. }
        | ClaudeStreamEvent::ToolApprovalRequired { .. }
        | ClaudeStreamEvent::ToolApprovalResolved { .. }
        | ClaudeStreamEvent::ToolApprovalStatus { .. }
        | ClaudeStreamEvent::ToolApprovalFailed { .. } => None,
    }
}

fn first_error(stderr: &str, fallback: &str) -> String {
    stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn claude_missing_terminal_error(stderr: &str) -> String {
    first_error(stderr, "Claude CLI ended without a terminal result.")
}

fn claude_session_in_use_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("session id") && lower.contains("already in use")
}

fn safe_session_id_for_process_match(session_id: &str) -> Option<&str> {
    let session_id = session_id.trim();
    if session_id.len() < 8 || session_id.len() > 128 {
        return None;
    }
    if session_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        Some(session_id)
    } else {
        None
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn process_matches_claude_session(name: &str, command_line: &str, session_id: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    matches!(name.as_str(), "claude" | "claude.exe" | "node" | "node.exe")
        && command_line.contains(session_id)
}

async fn terminate_claude_session_processes(session_id: &str) -> bool {
    let Some(session_id) = safe_session_id_for_process_match(session_id) else {
        return false;
    };
    let removed_stale_registry = remove_stale_claude_session_registry(session_id);
    let killed_from_command_line = terminate_claude_session_processes_impl(session_id).await;
    if killed_from_command_line {
        if let Some(entry) = find_claude_session_registry_entry(session_id) {
            let _ = wait_for_process_exit(entry.pid).await;
        }
        remove_stale_claude_session_registry(session_id);
    }
    removed_stale_registry || find_claude_session_registry_entry(session_id).is_none()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClaudeSessionRegistryEntry {
    pid: u32,
    path: PathBuf,
}

fn remove_stale_claude_session_registry(session_id: &str) -> bool {
    let Some(entry) = find_claude_session_registry_entry(session_id) else {
        return false;
    };
    remove_stale_claude_session_registry_entry(&entry)
}

fn remove_stale_claude_session_registry_entry(entry: &ClaudeSessionRegistryEntry) -> bool {
    if process_id_is_running(entry.pid) {
        return false;
    }
    match std::fs::remove_file(&entry.path) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn find_claude_session_registry_entry(session_id: &str) -> Option<ClaudeSessionRegistryEntry> {
    for dir in claude_session_registry_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let Some(entry) = claude_session_registry_entry_from_value(&value, path) else {
                continue;
            };
            if value.get("sessionId").and_then(Value::as_str) == Some(session_id) {
                return Some(entry);
            }
        }
    }
    None
}

fn claude_session_registry_entry_from_value(
    value: &Value,
    path: PathBuf,
) -> Option<ClaudeSessionRegistryEntry> {
    let pid = value
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())?;
    let session_id = value.get("sessionId").and_then(Value::as_str)?.trim();
    safe_session_id_for_process_match(session_id)?;
    Some(ClaudeSessionRegistryEntry { pid, path })
}

fn claude_session_registry_dirs() -> Vec<PathBuf> {
    claude_home_dirs()
        .into_iter()
        .map(|dir| dir.join("sessions"))
        .collect()
}

#[cfg(windows)]
fn process_id_is_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let running =
            GetExitCodeProcess(handle, &mut exit_code) != 0 && exit_code == STILL_ACTIVE as u32;
        CloseHandle(handle);
        running
    }
}

async fn wait_for_process_exit(pid: u32) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if !process_id_is_running(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(not(windows))]
fn process_id_is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
async fn terminate_claude_session_processes_impl(session_id: &str) -> bool {
    let current_pid = std::process::id();
    let script = format!(
        "$sid = '{session_id}'; $self = {current_pid}; Get-CimInstance Win32_Process | Where-Object {{ $_.ProcessId -ne $self -and $_.CommandLine -like \"*$sid*\" -and ($_.Name -ieq 'claude.exe' -or $_.Name -ieq 'claude' -or $_.Name -ieq 'node.exe' -or $_.Name -ieq 'node') }} | ForEach-Object {{ Write-Output $_.ProcessId; Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}"
    );
    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(script);
    command.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
    match crate::child_process::output(command).await {
        Ok(output) if output.status.success() => {
            !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        }
        _ => false,
    }
}

#[cfg(not(windows))]
async fn terminate_claude_session_processes_impl(session_id: &str) -> bool {
    let output = match Command::new("ps")
        .arg("-axo")
        .arg("pid=,comm=,args=")
        .output()
        .await
    {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };
    let current_pid = std::process::id();
    let mut killed = false;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim_start();
        let Some((pid_text, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = pid_text.trim().parse::<u32>() else {
            continue;
        };
        if pid == current_pid {
            continue;
        }
        let rest = rest.trim_start();
        let Some((name, command_line)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        if !process_matches_claude_session(name, command_line, session_id) {
            continue;
        }
        if Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
        {
            killed = true;
        }
    }
    killed
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn claude_native_worker_event(value: &Value) -> Option<AccountNativeWorkerLifecycle> {
    if value.get("type").and_then(Value::as_str) != Some("system") {
        return None;
    }
    let subtype = value.get("subtype").and_then(Value::as_str)?;
    let status = match subtype {
        "agent_started" => "running",
        "agent_completed" => "completed",
        "agent_failed" => "error",
        "agent_stopped" => "stopped",
        _ => return None,
    };
    let agent_id = string_field(value, "agent_id")?;
    Some(AccountNativeWorkerLifecycle {
        runtime: "claude".to_string(),
        call_id: string_field(value, "tool_use_id").unwrap_or_else(|| agent_id.clone()),
        operation: "native_agent".to_string(),
        status: status.to_string(),
        parent_runtime_id: string_field(value, "parent_agent_id"),
        worker_runtime_ids: vec![agent_id.clone()],
        workers: vec![AccountNativeWorkerState {
            runtime_id: agent_id,
            status: status.to_string(),
            message: string_field(value, "message"),
        }],
        prompt: string_field(value, "prompt").or_else(|| string_field(value, "description")),
        model: string_field(value, "model"),
    })
}

fn claude_run_args(req: &ClaudeRunRequest) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        "--input-format".to_string(),
        "stream-json".to_string(),
        "--output-format".to_string(),
        "stream-json".to_string(),
        "--verbose".to_string(),
        "--include-partial-messages".to_string(),
    ];
    if let Some(session_id) = clean_optional(req.session_id.as_deref()) {
        if claude_project_session_exists(req, &session_id) {
            args.extend(["--resume".to_string(), session_id]);
        } else {
            args.extend(["--session-id".to_string(), session_id]);
        }
    } else {
        args.push("--no-session-persistence".to_string());
    }
    args.extend([
        "--permission-mode".to_string(),
        claude_permission_mode(req).to_string(),
    ]);
    if let Some(path) = &req.approval_mcp_config {
        args.extend([
            "--mcp-config".to_string(),
            path.to_string_lossy().into_owned(),
        ]);
        if req.milim_mcp.is_some() {
            args.extend(["--allowedTools".to_string(), "mcp__milim__*".to_string()]);
        }
        if req.approval_mcp_url.is_some() {
            args.extend([
                "--permission-prompt-tool".to_string(),
                "mcp__milim_approval__request_tool_approval".to_string(),
            ]);
        }
    }
    for denied in claude_denied_tools(req) {
        args.extend(["--disallowedTools".to_string(), denied.to_string()]);
    }
    if let Some(model) = clean_optional(req.model.as_deref()) {
        args.extend(["--model".to_string(), model]);
    }
    if let Some(effort) = claude_effort(req.reasoning_effort) {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    args
}

fn claude_stream_input(req: &ClaudeRunRequest) -> Result<Value> {
    let mut content = Vec::new();
    if !req.prompt.trim().is_empty() {
        content.push(json!({ "type": "text", "text": req.prompt }));
    }
    for image in &req.images {
        if !matches!(
            image.media_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) || image.data.is_empty()
        {
            return Err(Error::InvalidRequest(
                "Claude images must contain PNG, JPEG, WebP, or GIF base64 data".to_string(),
            ));
        }
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": image.media_type,
                "data": image.data,
            }
        }));
    }
    Ok(json!({
        "type": "user",
        "session_id": "",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": Value::Null,
    }))
}

fn claude_project_session_exists(req: &ClaudeRunRequest, session_id: &str) -> bool {
    let Some(session_id) = safe_session_id_for_process_match(session_id) else {
        return false;
    };
    let Some(cwd) = claude_session_cwd(req) else {
        return false;
    };
    for projects_dir in claude_projects_dirs() {
        let project_name = claude_project_dir_name(cwd.to_string_lossy().as_ref());
        let session_file = projects_dir
            .join(&project_name)
            .join(format!("{session_id}.jsonl"));
        if session_file.is_file() {
            return true;
        }
        if project_name.len() <= CLAUDE_PROJECT_DIR_NAME_LIMIT {
            continue;
        }
        let prefix = format!("{}-", &project_name[..CLAUDE_PROJECT_DIR_NAME_LIMIT]);
        let Ok(entries) = std::fs::read_dir(&projects_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix)
                && entry.path().join(format!("{session_id}.jsonl")).is_file()
            {
                return true;
            }
        }
    }
    false
}

fn claude_session_cwd(req: &ClaudeRunRequest) -> Option<PathBuf> {
    if let Some(cwd) = clean_optional(req.cwd.as_deref()) {
        return Some(normalize_claude_cwd(Path::new(&cwd)));
    }
    std::env::current_dir()
        .ok()
        .map(|path| normalize_claude_cwd(&path))
}

fn normalize_claude_cwd(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn claude_project_dir_name(cwd: &str) -> String {
    let unc_cwd;
    let cwd = if let Some(path) = cwd.strip_prefix(r"\\?\UNC\") {
        unc_cwd = format!(r"\\{path}");
        unc_cwd.as_str()
    } else {
        cwd.strip_prefix(r"\\?\").unwrap_or(cwd)
    };
    let normalized: String = cwd
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    if normalized.len() <= CLAUDE_PROJECT_DIR_NAME_LIMIT {
        return normalized;
    }
    normalized
}

fn claude_projects_dirs() -> Vec<PathBuf> {
    claude_home_dirs()
        .into_iter()
        .map(|dir| dir.join("projects"))
        .collect()
}

fn claude_home_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(profile).join(".claude"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let dir = PathBuf::from(home).join(".claude");
        if !dirs.iter().any(|existing| existing == &dir) {
            dirs.push(dir);
        }
    }
    dirs
}

fn account_runtime_policy(value: Option<&str>) -> &str {
    match value.map(str::trim) {
        Some("review") => "review",
        Some("open") => "open",
        _ => "guarded",
    }
}

fn claude_session_recovery_allowed(req: &ClaudeRunRequest) -> bool {
    req.allow_session_recovery
        || (!req.plan_mode && account_runtime_policy(req.tool_approval_policy.as_deref()) == "open")
}

fn claude_tools_allowed(req: &ClaudeRunRequest) -> bool {
    !req.plan_mode
        && match account_runtime_policy(req.tool_approval_policy.as_deref()) {
            "review" => req.tool_approval_grant || req.interactive_tool_approval,
            _ => true,
        }
}

pub(crate) fn claude_interactive_tool_approval(req: &ClaudeRunRequest) -> bool {
    req.interactive_tool_approval
        && !req.plan_mode
        && !req.tool_approval_grant
        && account_runtime_policy(req.tool_approval_policy.as_deref()) == "review"
}

fn claude_permission_mode(req: &ClaudeRunRequest) -> &'static str {
    if req.plan_mode {
        "plan"
    } else if claude_interactive_tool_approval(req) {
        "manual"
    } else if !claude_tools_allowed(req)
        || account_runtime_policy(req.tool_approval_policy.as_deref()) == "guarded"
    {
        "dontAsk"
    } else {
        "bypassPermissions"
    }
}

fn claude_denied_tools(req: &ClaudeRunRequest) -> Vec<&'static str> {
    if req.plan_mode {
        Vec::new()
    } else if !claude_tools_allowed(req) {
        vec!["*"]
    } else if account_runtime_policy(req.tool_approval_policy.as_deref()) == "guarded" {
        vec!["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"]
    } else {
        Vec::new()
    }
}

fn claude_effort(effort: Option<ReasoningEffort>) -> Option<&'static str> {
    match effort? {
        ReasoningEffort::Low => Some("low"),
        ReasoningEffort::Medium => Some("medium"),
        ReasoningEffort::High => Some("high"),
        ReasoningEffort::Xhigh => Some("xhigh"),
        ReasoningEffort::Max => Some("max"),
        ReasoningEffort::Auto
        | ReasoningEffort::None
        | ReasoningEffort::Minimal
        | ReasoningEffort::On => None,
    }
}

fn usage_from_claude_result(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let prompt = opt_u32(usage, "input_tokens").unwrap_or(0)
        + opt_u32(usage, "cache_creation_input_tokens").unwrap_or(0)
        + opt_u32(usage, "cache_read_input_tokens").unwrap_or(0);
    let completion = opt_u32(usage, "output_tokens").unwrap_or(0);
    if prompt == 0 && completion == 0 {
        return None;
    }
    Some(Usage::new(prompt, completion))
}

fn claude_rate_limit(info: &Value) -> Value {
    json!({
        "provider": "Local Claude CLI",
        "status": info.get("status").and_then(Value::as_str),
        "kind": info.get("rateLimitType").and_then(Value::as_str),
        "reset_at": info.get("resetsAt").and_then(Value::as_i64),
        "raw": info,
    })
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn compact_json(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(value.to_string())
}

fn claude_spawn_error_message(error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        return cli_path_warning("Claude CLI", "claude");
    }
    format!("failed to start `claude`: {error}. Install Anthropic's official Claude CLI and sign in with `claude auth login`.")
}

fn cli_path_warning(label: &str, command: &str) -> String {
    format!("{label} CLI was not found on PATH. Apps launched from the Dock or Finder do not inherit your shell PATH, so on macOS and Linux Milim also looks in the usual install directories (`~/.local/bin`, Homebrew, `~/.bun/bin`, and asdf/mise/volta shims). Install `{command}` into one of those, or launch Milim from a terminal.")
}

fn is_cli_path_warning(message: &str) -> bool {
    message.contains("CLI was not found on PATH")
}

fn opt_u32(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

#[cfg(windows)]
fn claude_command() -> Command {
    Command::new("claude")
}

#[cfg(not(windows))]
fn claude_command() -> Command {
    crate::cli_path::command("claude")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_transcript(contents: &str) -> (PathBuf, ClaudeTranscriptFile) {
        let root =
            std::env::temp_dir().join(format!("milim-claude-import-test-{}", uuid::Uuid::new_v4()));
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = project.join(format!("{id}.jsonl"));
        std::fs::write(&path, contents).unwrap();
        (
            root,
            ClaudeTranscriptFile {
                id,
                path,
                created_at_ms: 1,
                updated_at_ms: 2,
            },
        )
    }

    #[test]
    fn claude_import_uses_active_branch_and_visible_text_only() {
        let (root, transcript) = test_transcript(
            r#"{"type":"user","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"hello"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"thinking","thinking":"secret"},{"type":"text","text":"first"}]}}
{"type":"user","uuid":"old-u","parentUuid":"a1","message":{"role":"user","content":"old branch"}}
{"type":"assistant","uuid":"old-a","parentUuid":"old-u","message":{"role":"assistant","content":[{"type":"text","text":"old answer"}]}}
not json
{"type":"user","uuid":"u2","parentUuid":"a1","message":{"role":"user","content":"new branch"}}
{"type":"assistant","uuid":"tool","parentUuid":"u2","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"path":"secret"}}]}}
{"type":"user","uuid":"result","parentUuid":"tool","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"large private result"}]}}
{"type":"assistant","uuid":"a2","parentUuid":"result","message":{"role":"assistant","content":[{"type":"text","text":"new answer"}]}}
{"type":"user","uuid":"notice","parentUuid":"a2","origin":{"kind":"task-notification"},"message":{"role":"user","content":"worker detail"}}
{"type":"assistant","uuid":"a3","parentUuid":"notice","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
{"type":"last-prompt","leafUuid":"a3"}
{"type":"custom-title","customTitle":"Chosen title"}
"#,
        );
        let imported = import_claude_transcript(&transcript).unwrap();
        assert_eq!(imported.title, "Chosen title");
        assert_eq!(
            imported
                .messages
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("user", "hello"),
                ("assistant", "first"),
                ("user", "new branch"),
                ("assistant", "new answer\n\ndone"),
            ]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_import_normalizes_commands_and_marks_media_only_prompts() {
        let (root, transcript) = test_transcript(
            r#"{"type":"user","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"<command-name>/review</command-name><command-args>123</command-args><local-command-stdout>hidden</local-command-stdout>"}}
{"type":"assistant","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"text","text":"reviewed"}]}}
{"type":"user","uuid":"u2","parentUuid":"a1","message":{"role":"user","content":[{"type":"image","source":{"data":"pixels"}}]}}
{"type":"last-prompt","leafUuid":"u2"}
"#,
        );
        let imported = import_claude_transcript(&transcript).unwrap();
        assert_eq!(imported.messages[0].content, "/review 123");
        assert_eq!(
            imported.messages[2].content,
            "[Media omitted during Claude import]"
        );
        assert!(!imported.resumable);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_discovery_excludes_nested_subagent_transcripts() {
        let root = std::env::temp_dir().join(format!(
            "milim-claude-discovery-test-{}",
            uuid::Uuid::new_v4()
        ));
        let project = root.join("project");
        let nested = project.join("session").join("subagents");
        std::fs::create_dir_all(&nested).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::write(project.join(format!("{id}.jsonl")), "{}\n").unwrap();
        std::fs::write(project.join("agent-main.jsonl"), "{}\n").unwrap();
        std::fs::write(nested.join("agent-child.jsonl"), "{}\n").unwrap();
        let files = claude_transcript_files_in(&root);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, id);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn claude_import_listing_filters_sorts_and_pages() {
        let summaries: Vec<ClaudeThreadSummary> = (0..30)
            .map(|index| ClaudeThreadSummary {
                id: format!("{index:02}"),
                title: if index == 7 {
                    "Needle chat".to_string()
                } else {
                    format!("Chat {index}")
                },
                preview: String::new(),
                cwd: Some(format!("C:\\project-{index}")),
                created_at_ms: index,
                updated_at_ms: index,
                resumable: true,
            })
            .collect();
        let first = page_claude_threads(summaries.clone(), 0, None, false);
        assert_eq!(first.data.len(), CLAUDE_THREAD_PAGE_SIZE);
        assert_eq!(first.data[0].updated_at_ms, 29);
        assert_eq!(first.next_cursor.as_deref(), Some("25"));
        let second = page_claude_threads(summaries.clone(), 25, None, false);
        assert_eq!(second.data.len(), 5);
        assert!(second.next_cursor.is_none());
        let filtered = page_claude_threads(summaries.clone(), 0, Some("needle"), false);
        assert_eq!(filtered.data.len(), 1);
        assert_eq!(filtered.data[0].id, "07");
        let complete = page_claude_threads(summaries, 0, None, true);
        assert_eq!(complete.data.len(), 30);
        assert!(complete.next_cursor.is_none());
    }

    #[test]
    fn parses_text_delta() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"ok"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn text_delta_does_not_close_open_tool() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let started = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"README.md"}}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(
            matches!(started.first(), Some(ClaudeStreamEvent::Tool { status, label: Some(label), .. }) if status == "running" && label == "Using Read")
        );

        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"I'll continue"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(
            matches!(events.first(), Some(ClaudeStreamEvent::Token { text }) if text == "I'll continue")
        );
        assert!(tools.contains_key("toolu_1"));

        let events = handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":"still working"}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Reasoning { text }) if text == "still working"
        ));
        assert!(tools.contains_key("toolu_1"));
    }

    #[test]
    fn tool_inputs_keep_full_copy_text() {
        let command = format!("powershell -Command \"{}\"", "x".repeat(140));
        let input = json!({ "command": command });
        let expected = input.to_string();
        let mut tools = BTreeMap::new();
        let event = claude_tool_start_event(
            &json!({
                "type": "tool_use",
                "id": "toolu-long",
                "name": "Bash",
                "input": input,
            }),
            &mut tools,
        );
        assert!(matches!(
            event,
            Some(ClaudeStreamEvent::Tool { detail: Some(detail), .. })
                if detail == expected
        ));
    }

    #[test]
    fn tool_results_close_only_the_matching_call() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::from([
            (
                "toolu_1".to_string(),
                ClaudeToolState {
                    name: "Read".to_string(),
                    detail: None,
                },
            ),
            (
                "toolu_2".to_string(),
                ClaudeToolState {
                    name: "Edit".to_string(),
                    detail: None,
                },
            ),
        ]);
        let mut done = false;
        let success = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#;
        let failure = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":"failed","is_error":true}]}}"#;

        let events = handle_line(success, &mut content, &mut reasoning, &mut tools, &mut done);
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_1" && status == "done"
        ));
        assert_eq!(events.len(), 1);
        assert!(tools.contains_key("toolu_2"));

        let events = handle_line(failure, &mut content, &mut reasoning, &mut tools, &mut done);
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_2" && status == "error"
        ));
        assert!(tools.is_empty());
        assert!(
            handle_line(failure, &mut content, &mut reasoning, &mut tools, &mut done,).is_empty()
        );
    }

    #[test]
    fn final_result_closes_tools_without_structured_results() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::from([(
            "toolu_1".to_string(),
            ClaudeToolState {
                name: "Read".to_string(),
                detail: None,
            },
        )]);
        let mut done = false;
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );

        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { id, status, .. })
                if id == "toolu_1" && status == "done"
        ));
        assert!(matches!(
            events.last(),
            Some(ClaudeStreamEvent::Done { .. })
        ));
        assert!(tools.is_empty());
    }

    #[test]
    fn flushes_before_result_done() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        assert!(handle_line(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"["}}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        )
        .is_empty());
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn"}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(events.first(), Some(ClaudeStreamEvent::Token { text }) if text == "["));
        assert!(
            matches!(events.last(), Some(ClaudeStreamEvent::Done { status, .. }) if status == "end_turn")
        );
    }

    #[test]
    fn parses_result_usage_and_limit_events() {
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","total_cost_usd":0.01,"usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":3,"output_tokens":4}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.last(),
            Some(ClaudeStreamEvent::Done {
                usage: Some(Usage { prompt_tokens: 15, completion_tokens: 4, total_tokens: 19 }),
                cost_usd: Some(cost),
                ..
            }) if (*cost - 0.01).abs() < f64::EPSILON
        ));

        let events = handle_line(
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1782660000,"rateLimitType":"five_hour"}}"#,
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::RateLimit { .. })
        ));
    }

    #[test]
    fn replays_versioned_stream_json_fixture_without_payload_diagnostics() {
        let fixture = include_str!(
            "../tests/fixtures/account-runtimes/claude-code-2.1.222-stream-json.jsonl"
        );
        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = fixture
            .lines()
            .flat_map(|line| handle_line(line, &mut content, &mut reasoning, &mut tools, &mut done))
            .collect::<Vec<_>>();
        let types = events
            .iter()
            .map(runtime_event)
            .filter_map(|event| {
                event
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            types,
            [
                "token",
                "tool",
                "tool",
                "rate_limit",
                "protocol_notice",
                "done"
            ]
        );
        assert!(!runtime_event(&events[4])
            .to_string()
            .contains("private_payload"));
        assert!(done);
        assert!(tools.is_empty());
    }

    #[test]
    fn status_models_require_authentication() {
        assert!(CLAUDE_MODEL_ALIASES.contains(&"sonnet"));
        assert!(CLAUDE_MODEL_ALIASES.contains(&"haiku"));
    }

    #[test]
    fn maps_supported_effort_flags() {
        assert_eq!(claude_effort(Some(ReasoningEffort::Low)), Some("low"));
        assert_eq!(claude_effort(Some(ReasoningEffort::Xhigh)), Some("xhigh"));
        assert_eq!(claude_effort(Some(ReasoningEffort::Minimal)), None);
        assert_eq!(claude_effort(Some(ReasoningEffort::Auto)), None);
    }

    #[test]
    fn persistent_run_args_use_session_id_without_turn_cap() {
        let args = claude_run_args(&ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: Some("sonnet".into()),
            cwd: None,
            reasoning_effort: Some(ReasoningEffort::High),
            session_id: Some("11111111-1111-4111-8111-111111111111".into()),
            tool_approval_policy: Some("open".into()),
            tool_approval_grant: true,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        });
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--session-id"
                && pair[1] == "11111111-1111-4111-8111-111111111111"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--effort" && pair[1] == "high"));
        assert!(!args.iter().any(|arg| arg == "--max-turns"));
        assert!(!args.iter().any(|arg| arg == "--no-session-persistence"));

        let args = claude_run_args(&ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: None,
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: None,
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        });
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(!args.iter().any(|arg| arg == "--max-turns"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--input-format", "stream-json"]));
    }

    #[test]
    fn claude_stream_input_preserves_image_only_turns() {
        let req = ClaudeRunRequest {
            prompt: String::new(),
            images: vec![AccountImage {
                media_type: "image/png".to_string(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC".to_string(),
            }],
            model: Some("sonnet".to_string()),
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: None,
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        };
        let input = claude_stream_input(&req).unwrap();
        assert_eq!(input["type"], "user");
        assert_eq!(input["message"]["content"].as_array().unwrap().len(), 1);
        assert_eq!(input["message"]["content"][0]["type"], "image");
        assert_eq!(
            input["message"]["content"][0]["source"]["media_type"],
            "image/png"
        );
    }

    #[test]
    fn claude_project_dir_names_match_cli_sanitizer() {
        assert_eq!(
            claude_project_dir_name("C:\\Users\\USER\\Documents\\DEV\\screenmeister"),
            "C--Users-USER-Documents-DEV-screenmeister"
        );
        assert_eq!(
            claude_project_dir_name(r"\\?\C:\Users\USER\Documents\DEV\screenmeister"),
            "C--Users-USER-Documents-DEV-screenmeister"
        );
        assert_eq!(
            claude_project_dir_name(r"\\?\UNC\server\share\screenmeister"),
            "--server-share-screenmeister"
        );
        assert_eq!(
            claude_project_dir_name("/Users/omer/Documents/DEV/milim"),
            "-Users-omer-Documents-DEV-milim"
        );
    }

    #[test]
    fn maps_milim_tool_modes_to_claude_permissions() {
        let mut req = ClaudeRunRequest {
            prompt: "hello".into(),
            images: Vec::new(),
            model: None,
            cwd: None,
            reasoning_effort: None,
            session_id: None,
            tool_approval_policy: Some("guarded".into()),
            tool_approval_grant: false,
            interactive_tool_approval: false,
            plan_mode: false,
            allow_session_recovery: false,
            milim_context: None,
            milim_mcp: None,
            approval_run_id: None,
            approval_mcp_url: None,
            approval_mcp_authorization: None,
            approval_mcp_config: None,
        };
        assert_eq!(claude_permission_mode(&req), "dontAsk");
        assert_eq!(
            claude_denied_tools(&req),
            vec!["Bash", "PowerShell", "Edit", "Write", "NotebookEdit"]
        );

        req.tool_approval_policy = Some("open".into());
        req.interactive_tool_approval = true;
        assert!(!claude_interactive_tool_approval(&req));
        assert!(claude_session_recovery_allowed(&req));
        assert_eq!(claude_permission_mode(&req), "bypassPermissions");
        assert!(claude_denied_tools(&req).is_empty());

        req.tool_approval_policy = Some("review".into());
        assert!(claude_interactive_tool_approval(&req));
        assert!(!claude_session_recovery_allowed(&req));
        assert_eq!(claude_permission_mode(&req), "manual");
        assert!(claude_denied_tools(&req).is_empty());

        req.plan_mode = true;
        assert!(!claude_session_recovery_allowed(&req));
        assert_eq!(claude_permission_mode(&req), "plan");
    }

    #[test]
    fn maps_only_explicit_claude_agent_lineage() {
        let event = claude_native_worker_event(&json!({
            "type": "system",
            "subtype": "agent_started",
            "agent_id": "agent-2",
            "parent_agent_id": "agent-1",
            "tool_use_id": "toolu-1",
            "description": "review the parser",
            "model": "sonnet"
        }))
        .expect("native worker event");
        assert_eq!(event.call_id, "toolu-1");
        assert_eq!(event.status, "running");
        assert_eq!(event.parent_runtime_id.as_deref(), Some("agent-1"));
        assert_eq!(event.worker_runtime_ids, vec!["agent-2"]);

        let tool_use = json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_start",
                "content_block": {
                    "type": "tool_use",
                    "id": "toolu-2",
                    "name": "Agent",
                    "input": { "description": "review" }
                }
            }
        });
        assert!(claude_native_worker_event(&tool_use).is_none());

        let mut content = Unredactor::new(BTreeMap::new());
        let mut reasoning = Unredactor::new(BTreeMap::new());
        let mut tools = BTreeMap::new();
        let mut done = false;
        let events = handle_line(
            &tool_use.to_string(),
            &mut content,
            &mut reasoning,
            &mut tools,
            &mut done,
        );
        assert!(matches!(
            events.first(),
            Some(ClaudeStreamEvent::Tool { name, .. }) if name == "Agent"
        ));
    }

    #[test]
    fn spawn_not_found_is_path_warning() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let message = claude_spawn_error_message(&error);
        assert!(is_cli_path_warning(&message));
        assert!(message.contains("claude"));
        assert!(message.contains("macOS"));
    }

    #[test]
    fn missing_terminal_result_is_an_error() {
        assert_eq!(
            claude_missing_terminal_error(""),
            "Claude CLI ended without a terminal result."
        );
        assert_eq!(
            claude_missing_terminal_error("\nprovider authentication failed\nignored"),
            "provider authentication failed"
        );
    }

    #[test]
    fn detects_locked_claude_session_errors() {
        assert!(claude_session_in_use_error(
            "Error: Session ID 374c9003-5446-4eba-841a-78bf02c93b95 is already in use."
        ));
        assert!(!claude_session_in_use_error(
            "claude exited with exit status: 1"
        ));
    }

    #[test]
    fn process_kill_matching_is_narrow() {
        let sid = "374c9003-5446-4eba-841a-78bf02c93b95";
        assert_eq!(safe_session_id_for_process_match(sid), Some(sid));
        assert_eq!(safe_session_id_for_process_match("bad'id"), None);
        assert!(process_matches_claude_session(
            "node.exe",
            &format!("node claude.js --session-id {sid}"),
            sid,
        ));
        assert!(process_matches_claude_session(
            "claude",
            &format!("claude -p hi --session-id {sid}"),
            sid,
        ));
        assert!(!process_matches_claude_session(
            "powershell.exe",
            &format!("powershell {sid}"),
            sid,
        ));
    }

    #[test]
    fn process_exit_probe() {
        if std::env::var_os("MILIM_CLAUDE_EXIT_PROBE").is_some() {
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    #[tokio::test]
    async fn waits_for_claude_session_owner_to_exit() {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("claude_bridge::tests::process_exit_probe")
            .env("MILIM_CLAUDE_EXIT_PROBE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let waiter = tokio::spawn(async move { child.wait().await.unwrap() });

        assert!(wait_for_process_exit(pid).await);
        assert!(waiter.await.unwrap().success());
    }

    #[test]
    fn parses_claude_session_registry_entries() {
        let path = PathBuf::from("44856.json");
        let value = json!({
            "pid": 44856,
            "sessionId": "baee7712-0151-4f0c-a6d3-9b77207b6575",
            "cwd": "C:\\Users\\USER\\Documents\\DEV\\screenmeister",
        });
        assert_eq!(
            claude_session_registry_entry_from_value(&value, path.clone()),
            Some(ClaudeSessionRegistryEntry { pid: 44856, path }),
        );

        let bad = json!({ "pid": 44856, "sessionId": "bad'id" });
        assert_eq!(
            claude_session_registry_entry_from_value(&bad, PathBuf::new()),
            None
        );
    }

    #[test]
    fn stale_registry_cleanup_preserves_live_entries() {
        let root =
            std::env::temp_dir().join(format!("milim-claude-registry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        let live_path = root.join("live.json");
        std::fs::write(&live_path, "{}").unwrap();
        assert!(!remove_stale_claude_session_registry_entry(
            &ClaudeSessionRegistryEntry {
                pid: std::process::id(),
                path: live_path.clone(),
            }
        ));
        assert!(live_path.is_file());

        let mut exited = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("milim_exited_pid_probe")
            .spawn()
            .unwrap();
        let exited_pid = exited.id();
        assert!(exited.wait().unwrap().success());
        let stale_path = root.join("stale.json");
        std::fs::write(&stale_path, "{}").unwrap();
        assert!(remove_stale_claude_session_registry_entry(
            &ClaudeSessionRegistryEntry {
                pid: exited_pid,
                path: stale_path.clone(),
            }
        ));
        assert!(!stale_path.exists());

        std::fs::remove_file(live_path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
