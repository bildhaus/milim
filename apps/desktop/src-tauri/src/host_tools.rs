//! Host filesystem + shell tools, using the GUI's selected working folder.
//!
//! Unlike `milim-tools`'s fixed-root fs tools (and the Docker-sandboxed
//! `run_command`), these operate on the **real** machine. Review and Guarded
//! keep them inside the folder selected via the desktop "Folder" chip; Open
//! accepts full host paths while retaining that folder as the working directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::proc::ProcessTreeGuard;
use milim_core::{Error, Result};
use milim_tools::{atomic_write, read_text_range, resolve_workspace_path, Tool, ToolEffect};

/// A cell holding the active working folder (shared with the server state).
pub type Workspace = Arc<RwLock<Option<PathBuf>>>;

#[derive(Clone)]
enum ToolWorkspace {
    Live(Workspace),
    Fixed(Arc<PathBuf>),
    FullAccess(Arc<PathBuf>),
}

/// Max bytes returned by `read_file`.
const MAX_READ: u64 = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 1000;

/// The current workspace root, or an error if the user hasn't picked a folder.
fn root_of(ws: &ToolWorkspace) -> Result<PathBuf> {
    match ws {
        ToolWorkspace::Fixed(root) | ToolWorkspace::FullAccess(root) => {
            Ok(root.as_ref().clone())
        }
        ToolWorkspace::Live(ws) => ws.read().ok().and_then(|g| g.clone()).ok_or_else(|| {
            Error::InvalidRequest(
                "no working folder selected - pick one with the Folder chip first".into(),
            )
        }),
    }
}

/// Resolve a path under the workspace boundary, or from the working directory
/// when the run explicitly has full host access.
fn safe_join(ws: &ToolWorkspace, path: &str) -> Result<PathBuf> {
    if matches!(ws, ToolWorkspace::FullAccess(_)) {
        let path = PathBuf::from(path);
        return Ok(if path.is_absolute() {
            path
        } else {
            root_of(ws)?.join(path)
        });
    }
    resolve_workspace_path(&root_of(ws)?, path)
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRequest(format!("missing string argument: {key}")))
}

fn optional_u64(args: &Value, key: &str, default: u64) -> Result<u64> {
    match args.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| Error::InvalidRequest(format!("{key} must be a non-negative integer"))),
    }
}

/// All host tools bound to the shared workspace cell.
pub fn host_tools(ws: Workspace) -> Vec<Arc<dyn Tool>> {
    let ws = ToolWorkspace::Live(ws);
    vec![
        Arc::new(ReadFileTool { ws: ws.clone() }),
        Arc::new(ReadFileAnchorsTool { ws: ws.clone() }),
        Arc::new(ListDirTool { ws: ws.clone() }),
        Arc::new(WriteFileTool { ws: ws.clone() }),
        Arc::new(EditFileTool { ws: ws.clone() }),
        Arc::new(PatchFileTool { ws: ws.clone() }),
        Arc::new(ShellTool { ws }),
    ]
}

/// Read a UTF-8 file from the working folder.
pub struct ReadFileTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "Read a UTF-8 text file from anywhere on the host. Relative paths use the working folder."
        } else {
            "Read a UTF-8 text file from the working folder (path is relative to it)."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "offset":{"type":"integer","minimum":0,"description":"Byte offset, default 0."},
            "limit":{"type":"integer","minimum":1,"maximum":1048576,"description":"Maximum bytes, default 1 MiB."}
        },"required":["path"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        let offset = optional_u64(&args, "offset", 0)?;
        let limit = usize::try_from(optional_u64(&args, "limit", MAX_READ)?).unwrap_or(usize::MAX);
        let (content, next_offset, eof) = read_text_range(&path, offset, limit)?;
        Ok(json!({ "content": content, "offset": offset, "next_offset": next_offset, "eof": eof }))
    }
}

fn line_hash(line: &str) -> u32 {
    line.as_bytes().iter().fold(0x811c9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    })
}

fn line_anchor(line_no: usize, line: &str) -> String {
    format!("{line_no}#{:08x}", line_hash(line))
}

fn newline_separator(content: &str) -> &str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn has_mixed_newlines(content: &str) -> bool {
    let bytes = content.as_bytes();
    let mut saw_lf = false;
    let mut saw_crlf = false;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                saw_crlf = true;
            } else {
                saw_lf = true;
            }
        }
    }
    saw_lf && saw_crlf
}

fn anchored_content(content: &str) -> String {
    let sep = newline_separator(content);
    let mut out = content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{}:{line}", line_anchor(i + 1, line)))
        .collect::<Vec<_>>()
        .join(sep);
    if content.ends_with('\n') && !out.is_empty() {
        out.push_str(sep);
    }
    out
}

fn parse_anchor(anchor: &str) -> Result<(usize, u32)> {
    let (line, hash) = anchor
        .split_once('#')
        .ok_or_else(|| Error::InvalidRequest(format!("invalid anchor: {anchor}")))?;
    let line = line
        .parse::<usize>()
        .map_err(|_| Error::InvalidRequest(format!("invalid anchor line: {anchor}")))?;
    if line == 0 {
        return Err(Error::InvalidRequest(format!(
            "invalid anchor line: {anchor}"
        )));
    }
    let hash = u32::from_str_radix(hash, 16)
        .map_err(|_| Error::InvalidRequest(format!("invalid anchor hash: {anchor}")))?;
    Ok((line, hash))
}

fn nearby_anchors(lines: &[String], index: usize) -> String {
    let start = index.saturating_sub(2);
    let end = lines.len().min(index + 3);
    lines[start..end]
        .iter()
        .enumerate()
        .map(|(offset, line)| {
            let line_no = start + offset + 1;
            format!("{}:{line}", line_anchor(line_no, line))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_anchor(lines: &[String], anchor: &str) -> Result<usize> {
    let (line_no, hash) = parse_anchor(anchor)?;
    let index = line_no - 1;
    let Some(line) = lines.get(index) else {
        return Err(Error::InvalidRequest(format!(
            "anchor out of range: {anchor}; nearby anchors:\n{}",
            nearby_anchors(lines, lines.len().saturating_sub(1))
        )));
    };
    if line_hash(line) != hash {
        return Err(Error::InvalidRequest(format!(
            "stale anchor: {anchor}; nearby anchors:\n{}",
            nearby_anchors(lines, index)
        )));
    }
    Ok(index)
}

fn patch_lines(content: &str) -> Vec<String> {
    content.lines().map(ToString::to_string).collect()
}

struct ResolvedPatch {
    start: usize,
    end: usize,
    lines: Vec<String>,
    order: usize,
}

fn required_obj<'a>(value: &'a Value, name: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| Error::InvalidRequest(format!("{name} must be an object")))
}

fn optional_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key).and_then(Value::as_str)
}

fn required_str<'a>(obj: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str> {
    optional_str(obj, key)
        .ok_or_else(|| Error::InvalidRequest(format!("missing string argument: {key}")))
}

fn resolve_patch_op(lines: &[String], op: &Value) -> Result<ResolvedPatch> {
    let obj = required_obj(op, "patch op")?;
    match required_str(obj, "op")? {
        "replace_range" => {
            let start = validate_anchor(lines, required_str(obj, "start")?)?;
            let end = validate_anchor(lines, required_str(obj, "end")?)? + 1;
            if start >= end {
                return Err(Error::InvalidRequest(
                    "replace_range start must be before end".into(),
                ));
            }
            Ok(ResolvedPatch {
                start,
                end,
                lines: patch_lines(required_str(obj, "content")?),
                order: 0,
            })
        }
        "delete_range" => {
            let start = validate_anchor(lines, required_str(obj, "start")?)?;
            let end = validate_anchor(lines, required_str(obj, "end")?)? + 1;
            if start >= end {
                return Err(Error::InvalidRequest(
                    "delete_range start must be before end".into(),
                ));
            }
            Ok(ResolvedPatch {
                start,
                end,
                lines: Vec::new(),
                order: 0,
            })
        }
        "insert_before" => {
            let start = validate_anchor(lines, required_str(obj, "anchor")?)?;
            Ok(ResolvedPatch {
                start,
                end: start,
                lines: patch_lines(required_str(obj, "content")?),
                order: 0,
            })
        }
        "insert_after" => {
            let start = validate_anchor(lines, required_str(obj, "anchor")?)? + 1;
            Ok(ResolvedPatch {
                start,
                end: start,
                lines: patch_lines(required_str(obj, "content")?),
                order: 0,
            })
        }
        other => Err(Error::InvalidRequest(format!("unknown patch op: {other}"))),
    }
}

/// Read a UTF-8 file with line-numbered content-hash anchors.
pub struct ReadFileAnchorsTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for ReadFileAnchorsTool {
    fn name(&self) -> &str {
        "read_file_anchors"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "Read a UTF-8 text file anywhere on the host with line-numbered hash anchors for patch_file. Relative paths use the working folder."
        } else {
            "Read a UTF-8 text file with line-numbered hash anchors for patch_file."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        let meta = std::fs::metadata(&path)?;
        if meta.len() > MAX_READ {
            return Err(Error::InvalidRequest(format!(
                "file too large ({} bytes, max {MAX_READ})",
                meta.len()
            )));
        }
        let content = std::fs::read_to_string(&path)?;
        if has_mixed_newlines(&content) {
            return Err(Error::InvalidRequest(
                "patch_file does not support mixed line endings; use edit_file".into(),
            ));
        }
        Ok(json!({ "content": anchored_content(&content) }))
    }
}

/// List directory entries within the working folder.
pub struct ListDirTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "List a directory anywhere on the host. Relative paths use the working folder."
        } else {
            "List entries of a directory in the working folder (path defaults to the root)."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = match args.get("path") {
            None => "",
            Some(value) => value
                .as_str()
                .ok_or_else(|| Error::InvalidRequest("path must be a string".into()))?,
        };
        let dir = safe_join(&self.ws, rel)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            entries.push(json!({
                "name": entry.file_name().to_string_lossy(),
                "is_dir": entry.file_type().map(|t| t.is_dir()).unwrap_or(false),
            }));
            if entries.len() > MAX_LIST_ENTRIES {
                break;
            }
        }
        entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let truncated = entries.len() > MAX_LIST_ENTRIES;
        entries.truncate(MAX_LIST_ENTRIES);
        Ok(json!({ "entries": entries, "truncated": truncated }))
    }
}

/// Create or overwrite a UTF-8 file in the working folder.
pub struct WriteFileTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "Create or overwrite a UTF-8 text file anywhere on the host. Relative paths use the working folder."
        } else {
            "Create or overwrite a UTF-8 text file in the working folder."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        let content = arg_str(&args, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        atomic_write(&path, content.as_bytes())?;
        Ok(json!({ "written": content.len() }))
    }
}

/// Exact-string replacement in a file (a surgical code edit). The `old` text
/// must occur exactly once, mirroring an editor's find/replace.
pub struct EditFileTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "Replace an exact text snippet in a file anywhere on the host. Relative paths use the working folder; 'old' must appear exactly once."
        } else {
            "Replace an exact text snippet in a file in the working folder. 'old' must appear exactly once."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "old":{"type":"string","description":"exact text to replace (must be unique in the file)"},
            "new":{"type":"string","description":"replacement text"}
        },"required":["path","old","new"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        let old = arg_str(&args, "old")?;
        let new = arg_str(&args, "new")?;
        let content = std::fs::read_to_string(&path)?;
        let count = content.matches(old).count();
        if count == 0 {
            return Err(Error::InvalidRequest("'old' text not found in file".into()));
        }
        if count > 1 {
            return Err(Error::InvalidRequest(format!(
                "'old' text is not unique ({count} matches) - include more surrounding context"
            )));
        }
        let updated = content.replacen(old, new, 1);
        if std::fs::read_to_string(&path)? != content {
            return Err(Error::InvalidRequest(
                "file changed while edit_file was running; read it again".into(),
            ));
        }
        atomic_write(&path, updated.as_bytes())?;
        Ok(json!({ "replaced": 1, "bytes": updated.len() }))
    }
}

/// Apply line-anchored edits produced from `read_file_anchors`.
pub struct PatchFileTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for PatchFileTool {
    fn name(&self) -> &str {
        "patch_file"
    }
    fn description(&self) -> &str {
        if matches!(self.ws, ToolWorkspace::FullAccess(_)) {
            "Patch a UTF-8 text file anywhere on the host using LINE#HASH anchors from read_file_anchors. Relative paths use the working folder."
        } else {
            "Patch a UTF-8 text file using LINE#HASH anchors from read_file_anchors."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "ops":{"type":"array","items":{"type":"object","properties":{
                "op":{"type":"string","enum":["replace_range","insert_before","insert_after","delete_range"]},
                "anchor":{"type":"string","description":"LINE#HASH anchor for insert ops"},
                "start":{"type":"string","description":"LINE#HASH anchor for range start"},
                "end":{"type":"string","description":"LINE#HASH anchor for range end"},
                "content":{"type":"string","description":"replacement or inserted text"}
            },"required":["op"]}}
        },"required":["path","ops"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ws, arg_str(&args, "path")?)?;
        let ops = args
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidRequest("missing array argument: ops".into()))?;
        if ops.is_empty() {
            return Err(Error::InvalidRequest("ops must not be empty".into()));
        }

        let content = std::fs::read_to_string(&path)?;
        if has_mixed_newlines(&content) {
            return Err(Error::InvalidRequest(
                "patch_file does not support mixed line endings; use edit_file".into(),
            ));
        }
        let sep = newline_separator(&content);
        let keep_trailing_newline = content.ends_with('\n');
        let mut lines = content.lines().map(ToString::to_string).collect::<Vec<_>>();
        let mut patches = ops
            .iter()
            .enumerate()
            .map(|(order, op)| {
                resolve_patch_op(&lines, op).map(|mut patch| {
                    patch.order = order;
                    patch
                })
            })
            .collect::<Result<Vec<_>>>()?;
        patches.sort_by(|a, b| {
            b.start
                .cmp(&a.start)
                .then_with(|| b.end.cmp(&a.end))
                .then_with(|| b.order.cmp(&a.order))
        });

        let mut next_lower_start = usize::MAX;
        for patch in &patches {
            if patch.end > next_lower_start {
                return Err(Error::InvalidRequest(
                    "patch ranges must not overlap".into(),
                ));
            }
            next_lower_start = patch.start;
        }

        let added: usize = patches.iter().map(|patch| patch.lines.len()).sum();
        let removed: usize = patches.iter().map(|patch| patch.end - patch.start).sum();
        for patch in patches {
            lines.splice(patch.start..patch.end, patch.lines);
        }

        // ponytail: preserve the existing newline style; byte-exact hunk control can wait.
        let mut updated = lines.join(sep);
        if keep_trailing_newline && !updated.is_empty() {
            updated.push_str(sep);
        }
        if std::fs::read_to_string(&path)? != content {
            return Err(Error::InvalidRequest(
                "file changed while patch_file was running; read it again".into(),
            ));
        }
        atomic_write(&path, updated.as_bytes())?;
        Ok(
            json!({ "patched": ops.len(), "added": added, "removed": removed, "bytes": updated.len() }),
        )
    }
}

/// Run a command in the host terminal, in the working folder. PowerShell on
/// Windows, `sh -c` elsewhere. Executes on the real machine - the agentic
/// counterpart to the Docker-sandboxed `run_command`.
pub struct ShellTool {
    ws: ToolWorkspace,
}
#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        if cfg!(windows) {
            "Run a PowerShell command on the host, in the working folder. Returns stdout/stderr/exit_code."
        } else {
            "Run a shell command (sh -c) on the host, in the working folder. Returns stdout/stderr/exit_code."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Command
    }
    fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
        }))
    }
    fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
        Some(Arc::new(Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
        }))
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let cwd = root_of(&self.ws)?;
        let command = arg_str(&args, "command")?.to_string();
        run_shell(&cwd, &command).await
    }
}

async fn run_shell(cwd: &Path, command: &str) -> Result<Value> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;

    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
    const MAX_OUTPUT: usize = 1024 * 1024;
    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", command])
            .current_dir(cwd);
        #[cfg(windows)]
        cmd.creation_flags(milim_core::proc::CREATE_NO_WINDOW);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]).current_dir(cwd);
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd
        .spawn()
        .map_err(|error| Error::Other(format!("shell failed to start: {error}")))?;
    let mut guard = ProcessTreeGuard::attach(
        child
            .id()
            .ok_or_else(|| Error::Other("shell process id unavailable".into()))?,
    )
    .map_err(|error| Error::Other(format!("failed to contain shell process: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Other("shell stdout unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Other("shell stderr unavailable".into()))?;
    let read = |mut stream: tokio::process::ChildStdout| async move {
        let mut kept = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            let count = stream.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            let remaining = MAX_OUTPUT.saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..count.min(remaining)]);
            truncated |= count > remaining;
        }
        Ok::<_, std::io::Error>((kept, truncated))
    };
    let stdout_task = tokio::spawn(read(stdout));
    let stderr_task = tokio::spawn(async move {
        let mut stream = stderr;
        let mut kept = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut truncated = false;
        loop {
            let count = stream.read(&mut buffer).await?;
            if count == 0 {
                break;
            }
            let remaining = MAX_OUTPUT.saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..count.min(remaining)]);
            truncated |= count > remaining;
        }
        Ok::<_, std::io::Error>((kept, truncated))
    });
    let status = match tokio::time::timeout(TIMEOUT, child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            guard.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(Error::Other("shell timed out after 120 seconds".into()));
        }
    };
    guard.terminate();
    let (stdout, stdout_truncated) = stdout_task
        .await
        .map_err(|error| Error::Other(format!("shell stdout task failed: {error}")))??;
    let (stderr, stderr_truncated) = stderr_task
        .await
        .map_err(|error| Error::Other(format!("shell stderr task failed: {error}")))??;
    Ok(json!({
        "stdout": String::from_utf8_lossy(&stdout),
        "stderr": String::from_utf8_lossy(&stderr),
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated,
        "exit_code": status.code(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn temp_workspace() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "milim-host-tools-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn tool(tools: &[Arc<dyn Tool>], name: &str) -> Arc<dyn Tool> {
        tools
            .iter()
            .find(|tool| tool.name() == name)
            .unwrap_or_else(|| panic!("missing tool: {name}"))
            .clone()
    }

    #[test]
    fn read_file_anchors_and_patch_file_round_trip() {
        let root = temp_workspace();
        let path = root.join("notes.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let ws = Arc::new(RwLock::new(Some(root.clone())));
        let tools = host_tools(ws);

        let anchored =
            block_on(tool(&tools, "read_file_anchors").invoke(json!({"path":"notes.txt"})))
                .unwrap();
        let anchored = anchored["content"].as_str().unwrap();
        assert!(anchored.contains(&format!("{}:one", line_anchor(1, "one"))));
        assert!(anchored.contains(&format!("{}:two", line_anchor(2, "two"))));

        block_on(tool(&tools, "patch_file").invoke(json!({
            "path": "notes.txt",
            "ops": [
                {"op":"insert_after","anchor":line_anchor(1, "one"),"content":"one point five"},
                {"op":"replace_range","start":line_anchor(2, "two"),"end":line_anchor(2, "two"),"content":"TWO"},
                {"op":"delete_range","start":line_anchor(3, "three"),"end":line_anchor(3, "three")}
            ]
        })))
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "one\none point five\nTWO\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn patch_file_rejects_stale_anchor() {
        let root = temp_workspace();
        std::fs::write(root.join("notes.txt"), "one\ntwo\n").unwrap();
        let ws = Arc::new(RwLock::new(Some(root.clone())));
        let tools = host_tools(ws);

        let err = block_on(tool(&tools, "patch_file").invoke(json!({
            "path": "notes.txt",
            "ops": [
                {"op":"replace_range","start":"2#00000000","end":"2#00000000","content":"TWO"}
            ]
        })))
        .unwrap_err()
        .to_string();

        assert!(err.contains("stale anchor"), "unexpected error: {err}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn registry_workspace_is_immutable_for_the_run() {
        let first = temp_workspace();
        let second = temp_workspace();
        let workspace = Arc::new(RwLock::new(Some(first.clone())));
        let mut registry = milim_tools::ToolRegistry::new();
        for item in host_tools(workspace.clone()) {
            registry.register(item);
        }
        let run_registry = registry.scoped_to_workspace(&first);
        *workspace.write().unwrap() = Some(second.clone());

        block_on(run_registry.call("write_file", json!({"path":"bound.txt","content":"first"})))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(first.join("bound.txt")).unwrap(),
            "first"
        );
        assert!(!second.join("bound.txt").exists());

        let _ = std::fs::remove_dir_all(first);
        let _ = std::fs::remove_dir_all(second);
    }

    #[test]
    fn full_access_registry_reads_outside_the_working_folder() {
        let root = temp_workspace();
        let working = root.join("working");
        let sibling = root.join("sibling");
        std::fs::create_dir_all(&working).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let outside = sibling.join("outside.txt");
        std::fs::write(&outside, "visible in Open").unwrap();
        let workspace = Arc::new(RwLock::new(Some(working.clone())));
        let mut registry = milim_tools::ToolRegistry::new();
        for item in host_tools(workspace) {
            registry.register(item);
        }

        assert!(block_on(
            registry
                .scoped_to_workspace(&working)
                .call("read_file", json!({"path": outside}))
        )
        .is_err());
        let full_access = registry.with_full_access(&working);
        let result = block_on(
            full_access
                .read_only()
                .call("read_file", json!({"path": outside})),
        )
        .unwrap();
        assert_eq!(result["content"], "visible in Open");
        let written = sibling.join("written.txt");
        block_on(full_access.call(
            "write_file",
            json!({"path": written, "content": "written in Open"}),
        ))
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(written).unwrap(),
            "written in Open"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn patch_preserves_same_anchor_insert_order() {
        let root = temp_workspace();
        let path = root.join("notes.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let tools = host_tools(Arc::new(RwLock::new(Some(root.clone()))));

        block_on(tool(&tools, "patch_file").invoke(json!({
            "path": "notes.txt",
            "ops": [
                {"op":"insert_after","anchor":line_anchor(1, "one"),"content":"first"},
                {"op":"insert_after","anchor":line_anchor(1, "one"),"content":"second"}
            ]
        })))
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "one\nfirst\nsecond\ntwo\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn hashline_tools_reject_mixed_line_endings() {
        let root = temp_workspace();
        std::fs::write(root.join("mixed.txt"), "one\r\ntwo\n").unwrap();
        let tools = host_tools(Arc::new(RwLock::new(Some(root.clone()))));
        let error = block_on(tool(&tools, "read_file_anchors").invoke(json!({
            "path": "mixed.txt"
        })))
        .unwrap_err()
        .to_string();
        assert!(error.contains("mixed line endings"));
        let _ = std::fs::remove_dir_all(root);
    }
}
