//! Host filesystem, search, and shell tools, using the GUI's selected working folder.
//!
//! Unlike `milim-tools`'s fixed-root fs tools (and the Docker-sandboxed
//! `run_command`), these operate on the **real** machine. Review and Guarded
//! keep them inside the folder selected via the desktop "Folder" chip; Open
//! accepts full host paths while retaining that folder as the working directory.
//!
//! Tools rebound for one run share a [`RunState`]: the files read in that run
//! (so edits can detect stale content), the shell's working directory, and the
//! run's background processes, which are killed when the run's tools drop.

mod edit;
mod search;
mod shell;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{
    atomic_write, read_text_range, resolve_workspace_path, Tool, ToolConcurrency, ToolEffect,
};

use edit::EditFileTool;
use search::{GlobTool, GrepTool};
use shell::{ProcessKillTool, ProcessOutputTool, ShellTool};

/// Max bytes `read_file_anchors` reads.
const MAX_ANCHORED_READ: u64 = 1024 * 1024;

/// A cell holding the active working folder (shared with the server state).
pub type Workspace = Arc<RwLock<Option<PathBuf>>>;

#[derive(Clone)]
enum ToolWorkspace {
    Live(Workspace),
    Fixed(Arc<PathBuf>),
    FullAccess(Arc<PathBuf>),
}

/// Size and modification time of a file when a run last read or wrote it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// Whether a file may be edited given what this run has seen of it.
#[derive(Debug, Eq, PartialEq)]
enum Freshness {
    /// Unchanged since this run last read or wrote it.
    Current,
    /// Never read in this run; the edit proceeds with a note.
    Unread,
}

/// State shared by one run's host tools.
#[derive(Default)]
struct RunState {
    stamps: Mutex<HashMap<PathBuf, FileStamp>>,
    shell: shell::ShellRunState,
}

impl RunState {
    fn key(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// Remember the file's current stamp after the run read or wrote it.
    fn record(&self, path: &Path) {
        if let (Some(stamp), Ok(mut stamps)) = (FileStamp::of(path), self.stamps.lock()) {
            stamps.insert(Self::key(path), stamp);
        }
    }

    /// Refuse edits to files that changed on disk after this run read them.
    fn freshness(&self, path: &Path) -> Result<Freshness> {
        let recorded = self
            .stamps
            .lock()
            .ok()
            .and_then(|stamps| stamps.get(&Self::key(path)).copied());
        match recorded {
            None => Ok(Freshness::Unread),
            Some(stamp) if FileStamp::of(path) == Some(stamp) => Ok(Freshness::Current),
            Some(_) => Err(Error::InvalidRequest(format!(
                "{} changed on disk since this run last read it; read it again before editing",
                path.display()
            ))),
        }
    }
}

thread_local! {
    /// The run state most recently created on this thread, and the tools
    /// already bound to it.
    static RUN_BINDING: RefCell<Option<RunBinding>> = const { RefCell::new(None) };
}

struct RunBinding {
    family: u64,
    run: Weak<RunState>,
    bound: Vec<String>,
}

static NEXT_FAMILY: AtomicU64 = AtomicU64::new(1);

/// A host tool's workspace binding plus the run state it shares with the
/// other host tools of the same run.
#[derive(Clone)]
struct HostCtx {
    ws: ToolWorkspace,
    run: Arc<RunState>,
    /// Identifies the tools created by one [`host_tools`] call.
    family: u64,
}

impl HostCtx {
    fn new(ws: ToolWorkspace) -> Self {
        Self {
            ws,
            run: Arc::new(RunState::default()),
            family: NEXT_FAMILY.fetch_add(1, Ordering::Relaxed),
        }
    }

    fn full_access(&self) -> bool {
        matches!(self.ws, ToolWorkspace::FullAccess(_))
    }

    fn fixed(&self, root: &Path) -> Self {
        Self {
            ws: ToolWorkspace::Fixed(Arc::new(root.to_path_buf())),
            ..self.clone()
        }
    }

    fn with_full_access(&self, cwd: &Path) -> Self {
        Self {
            ws: ToolWorkspace::FullAccess(Arc::new(cwd.to_path_buf())),
            ..self.clone()
        }
    }

    /// Bind `tool` to a run's shared state. `ToolRegistry::scoped_for_run`
    /// rebinds every tool of a registry synchronously, one after another, so
    /// host tools rebound back to back on one thread belong to the same run.
    /// A tool that is already bound to the latest state starts the next run.
    fn for_run(&self, tool: &str) -> Self {
        let run = RUN_BINDING.with(|cell| {
            let mut binding = cell.borrow_mut();
            if let Some(current) = binding.as_mut().filter(|current| {
                current.family == self.family && !current.bound.iter().any(|name| name == tool)
            }) {
                if let Some(run) = current.run.upgrade() {
                    current.bound.push(tool.to_string());
                    return run;
                }
            }
            let run = Arc::new(RunState::default());
            *binding = Some(RunBinding {
                family: self.family,
                run: Arc::downgrade(&run),
                bound: vec![tool.to_string()],
            });
            run
        });
        Self {
            run,
            ..self.clone()
        }
    }
}

/// Implements the workspace, full-access, and per-run rebinding hooks for a
/// host tool whose only field is `ctx: HostCtx`.
macro_rules! host_tool_scoping {
    () => {
        fn scoped_to_workspace(&self, root: &Path) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(Self {
                ctx: self.ctx.fixed(root),
            }))
        }
        fn with_full_access(&self, cwd: &Path) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(Self {
                ctx: self.ctx.with_full_access(cwd),
            }))
        }
        fn scoped_for_run(&self) -> Option<Arc<dyn Tool>> {
            Some(Arc::new(Self {
                ctx: self.ctx.for_run(self.name()),
            }))
        }
    };
}
use host_tool_scoping;

/// The current workspace root, or an error if the user hasn't picked a folder.
fn root_of(ws: &ToolWorkspace) -> Result<PathBuf> {
    match ws {
        ToolWorkspace::Fixed(root) | ToolWorkspace::FullAccess(root) => Ok(root.as_ref().clone()),
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

/// Like [`safe_join`], but also admits saved oversized tool output.
fn read_path(ws: &ToolWorkspace, path: &str) -> Result<PathBuf> {
    if matches!(ws, ToolWorkspace::FullAccess(_)) {
        return safe_join(ws, path);
    }
    milim_tools::ReadFileTool::resolve_read_path(&root_of(ws)?, path)
}

/// Whether `dir` may serve as a working directory for this binding.
fn inside_workspace(ws: &ToolWorkspace, dir: &Path) -> bool {
    if matches!(ws, ToolWorkspace::FullAccess(_)) {
        return true;
    }
    let (Ok(root), Ok(dir)) = (
        root_of(ws).and_then(|root| std::fs::canonicalize(root).map_err(Into::into)),
        std::fs::canonicalize(dir),
    ) else {
        return false;
    };
    dir.starts_with(root)
}

/// Formats paths relative to the workspace root with `/` separators, or in
/// full when they lie outside it.
struct PathDisplay {
    root: PathBuf,
    canonical_root: PathBuf,
}

impl PathDisplay {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            canonical_root: std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
        }
    }

    fn show(&self, path: &Path) -> String {
        let relative = path
            .strip_prefix(&self.canonical_root)
            .or_else(|_| path.strip_prefix(&self.root));
        match relative {
            Ok(relative) if relative.as_os_str().is_empty() => ".".into(),
            Ok(relative) => relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            Err(_) => path.display().to_string(),
        }
    }
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidRequest(format!("missing string argument: {key}")))
}

fn optional_arg_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| Error::InvalidRequest(format!("{key} must be a string"))),
    }
}

fn optional_u64(args: &Value, key: &str, default: u64) -> Result<u64> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| Error::InvalidRequest(format!("{key} must be a non-negative integer"))),
    }
}

fn optional_bool(args: &Value, key: &str) -> Result<bool> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| Error::InvalidRequest(format!("{key} must be a boolean"))),
    }
}

/// All host tools bound to the shared workspace cell.
pub fn host_tools(ws: Workspace) -> Vec<Arc<dyn Tool>> {
    let ctx = HostCtx::new(ToolWorkspace::Live(ws));
    vec![
        Arc::new(ReadFileTool { ctx: ctx.clone() }),
        Arc::new(ReadFileAnchorsTool { ctx: ctx.clone() }),
        Arc::new(ListDirTool { ctx: ctx.clone() }),
        Arc::new(GlobTool { ctx: ctx.clone() }),
        Arc::new(GrepTool { ctx: ctx.clone() }),
        Arc::new(WriteFileTool { ctx: ctx.clone() }),
        Arc::new(EditFileTool { ctx: ctx.clone() }),
        Arc::new(PatchFileTool { ctx: ctx.clone() }),
        Arc::new(ShellTool { ctx: ctx.clone() }),
        Arc::new(ProcessOutputTool { ctx: ctx.clone() }),
        Arc::new(ProcessKillTool { ctx }),
    ]
}

/// Read a UTF-8 file from the working folder.
pub struct ReadFileTool {
    ctx: HostCtx,
}
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
            "Read a UTF-8 text file from anywhere on the host. Relative paths use the working folder. Returns numbered lines; offset/limit select a line range (default: the first 2000 lines)."
        } else {
            "Read a UTF-8 text file from the working folder (path is relative to it). Returns numbered lines; offset/limit select a line range (default: the first 2000 lines)."
        }
    }
    fn input_schema(&self) -> Value {
        milim_tools::ReadFileTool::schema()
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        milim_tools::ReadFileTool::render_for_model(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = read_path(&self.ctx.ws, arg_str(&args, "path")?)?;
        let (offset, limit) = milim_tools::ReadFileTool::line_window(&args)?;
        let result = read_text_range(&path, offset, limit)?;
        self.ctx.run.record(&path);
        Ok(result)
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
    ctx: HostCtx,
}
#[async_trait]
impl Tool for ReadFileAnchorsTool {
    fn name(&self) -> &str {
        "read_file_anchors"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
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
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ctx.ws, arg_str(&args, "path")?)?;
        let meta = std::fs::metadata(&path)?;
        if meta.len() > MAX_ANCHORED_READ {
            return Err(Error::InvalidRequest(format!(
                "file too large ({} bytes, max {MAX_ANCHORED_READ})",
                meta.len()
            )));
        }
        let content = std::fs::read_to_string(&path)?;
        if has_mixed_newlines(&content) {
            return Err(Error::InvalidRequest(
                "patch_file does not support mixed line endings; use edit_file".into(),
            ));
        }
        self.ctx.run.record(&path);
        Ok(json!({ "content": anchored_content(&content) }))
    }
}

/// List directory entries within the working folder.
pub struct ListDirTool {
    ctx: HostCtx,
}
#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
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
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        milim_tools::ListDirTool::render_for_model(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = optional_arg_str(&args, "path")?.unwrap_or("");
        milim_tools::ListDirTool::list(&safe_join(&self.ctx.ws, rel)?)
    }
}

/// Create or overwrite a UTF-8 file in the working folder.
pub struct WriteFileTool {
    ctx: HostCtx,
}
#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
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
    fn model_text(&self, result: &Value) -> Option<String> {
        milim_tools::WriteFileTool::render_for_model(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = arg_str(&args, "path")?;
        let path = safe_join(&self.ctx.ws, rel)?;
        let content = arg_str(&args, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = safe_join(&self.ctx.ws, rel)?;
        let created = !path.exists();
        atomic_write(&path, content.as_bytes())?;
        self.ctx.run.record(&path);
        Ok(milim_tools::WriteFileTool::result(rel, content, created))
    }
}

/// Apply line-anchored edits produced from `read_file_anchors`.
pub struct PatchFileTool {
    ctx: HostCtx,
}
#[async_trait]
impl Tool for PatchFileTool {
    fn name(&self) -> &str {
        "patch_file"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
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
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.ctx.ws, arg_str(&args, "path")?)?;
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
        self.ctx.run.record(&path);
        Ok(
            json!({ "patched": ops.len(), "added": added, "removed": removed, "bytes": updated.len() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::time::SystemTime;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
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
        assert_eq!(std::fs::read_to_string(written).unwrap(), "written in Open");

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

    #[test]
    fn host_shell_inherits_user_environment_and_declares_the_policy() {
        let root = temp_workspace();
        let tools = host_tools(Arc::new(RwLock::new(Some(root.clone()))));
        let shell = tool(&tools, "shell");
        assert_eq!(
            shell.environment_policy(),
            milim_tools::ProcessEnvironmentPolicy::HostShellInherited
        );
        let key = format!("MILIM_HOST_ENV_PROOF_{}", std::process::id());
        let value = "host-environment-visible";
        std::env::set_var(&key, value);
        let command = if cfg!(windows) {
            format!("[Console]::Write($env:{key})")
        } else {
            format!("printf '%s' \"${key}\"")
        };
        let result = block_on(shell.invoke(json!({"command": command}))).unwrap();
        std::env::remove_var(&key);
        assert_eq!(result["stdout"], value);
        assert_eq!(result["exit_code"], 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn host_shell_withholds_milim_secrets() {
        let root = temp_workspace();
        let tools = host_tools(Arc::new(RwLock::new(Some(root.clone()))));
        let shell = tool(&tools, "shell");
        let secret_key = format!("MILIM_TEST_SHELL_API_KEY_{}", std::process::id());
        let user_key = format!("USER_TEST_SHELL_API_KEY_{}", std::process::id());
        std::env::set_var(&secret_key, "milim-secret");
        std::env::set_var(&user_key, "user-owned");
        let command = if cfg!(windows) {
            format!("[Console]::Write(\"$env:{secret_key}|$env:{user_key}\")")
        } else {
            format!("printf '%s|%s' \"${secret_key}\" \"${user_key}\"")
        };
        let result = block_on(shell.invoke(json!({"command": command}))).unwrap();
        std::env::remove_var(&secret_key);
        std::env::remove_var(&user_key);
        assert_eq!(result["stdout"], "|user-owned");
        let _ = std::fs::remove_dir_all(root);
    }

    fn run_registry(root: &Path) -> milim_tools::ToolRegistry {
        let mut registry = milim_tools::ToolRegistry::new();
        for item in host_tools(Arc::new(RwLock::new(Some(root.to_path_buf())))) {
            registry.register(item);
        }
        registry.scoped_to_workspace(root).scoped_for_run()
    }

    fn model_text(registry: &milim_tools::ToolRegistry, name: &str, args: Value) -> String {
        block_on(registry.call_for_agent(name, args))
            .unwrap()
            .model_text
            .unwrap_or_else(|| panic!("{name} returned no model text"))
    }

    #[test]
    fn read_file_returns_numbered_line_ranges() {
        let root = temp_workspace();
        let lines = (1..=30)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>();
        std::fs::write(root.join("long.txt"), lines.join("\n")).unwrap();
        let registry = run_registry(&root);

        let raw = block_on(registry.call(
            "read_file",
            json!({"path":"long.txt","offset":10,"limit":3}),
        ))
        .unwrap();
        assert_eq!(raw["content"], "line 10\nline 11\nline 12");
        assert_eq!(raw["total_lines"], 30);
        assert_eq!(raw["next_offset"], 13);
        let text = model_text(
            &registry,
            "read_file",
            json!({"path":"long.txt","offset":10,"limit":3}),
        );
        assert_eq!(
            text,
            "    10\tline 10\n    11\tline 11\n    12\tline 12\n\n(Showing lines 10-12 of 30. Continue with offset=13.)"
        );
        let end = model_text(
            &registry,
            "read_file",
            json!({"path":"long.txt","offset":29}),
        );
        assert_eq!(end, "    29\tline 29\n    30\tline 30");
        assert_eq!(
            registry
                .execution_specs()
                .iter()
                .find(|spec| spec.name == "read_file")
                .unwrap()
                .concurrency,
            ToolConcurrency::Parallel
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scoped_read_file_accepts_saved_tool_output_only() {
        let output_root = std::env::temp_dir().join("milim-host-tools-test-output");
        std::fs::create_dir_all(&output_root).unwrap();
        milim_tools::set_tool_output_root(output_root.clone());
        let output_root = milim_tools::tool_output_root().unwrap().to_path_buf();
        let saved = output_root.join(format!("saved-{}.txt", std::process::id()));
        std::fs::write(&saved, "saved output").unwrap();
        let root = temp_workspace();
        let outside = temp_workspace().join("other.txt");
        std::fs::write(&outside, "private").unwrap();
        let registry = run_registry(&root);

        let read = block_on(registry.call("read_file", json!({"path": saved}))).unwrap();
        assert_eq!(read["content"], "saved output");
        assert!(block_on(registry.call("read_file", json!({"path": outside}))).is_err());
        assert!(
            block_on(registry.call("write_file", json!({"path": saved, "content": "no"}))).is_err()
        );
        let _ = std::fs::remove_file(saved);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside.parent().unwrap());
    }

    #[test]
    fn edit_file_replaces_all_and_reports_a_diff() {
        let root = temp_workspace();
        std::fs::write(root.join("app.js"), "let a = 1;\nuse(a);\nuse(a);\n").unwrap();
        let registry = run_registry(&root);
        block_on(registry.call("read_file", json!({"path":"app.js"}))).unwrap();

        let error = block_on(registry.call(
            "edit_file",
            json!({"path":"app.js","old":"use(a)","new":"use(b)"}),
        ))
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("not unique (2 matches, at lines 2, 3)"),
            "{error}"
        );

        let raw = block_on(registry.call(
            "edit_file",
            json!({"path":"app.js","old":"use(a)","new":"use(b)","replace_all":true}),
        ))
        .unwrap();
        assert_eq!(raw["replaced"], 2);
        assert_eq!(
            (raw["added"].as_u64(), raw["removed"].as_u64()),
            (Some(2), Some(2))
        );
        assert_eq!(
            std::fs::read_to_string(root.join("app.js")).unwrap(),
            "let a = 1;\nuse(b);\nuse(b);\n"
        );
        let text = model_text(
            &registry,
            "edit_file",
            json!({"path":"app.js","old":"let a = 1;","new":"let b = 1;"}),
        );
        assert_eq!(
            text,
            "Edited app.js: 1 replacement.\n@@ -1,3 +1,3 @@\n-let a = 1;\n+let b = 1;\n use(b);\n use(b);"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn edit_file_refuses_files_changed_since_they_were_read() {
        let root = temp_workspace();
        let path = root.join("notes.txt");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let registry = run_registry(&root);
        block_on(registry.call("read_file", json!({"path":"notes.txt"}))).unwrap();
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

        let error = block_on(registry.call(
            "edit_file",
            json!({"path":"notes.txt","old":"beta","new":"BETA"}),
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("changed on disk"), "{error}");

        block_on(registry.call("read_file", json!({"path":"notes.txt"}))).unwrap();
        let first = block_on(registry.call(
            "edit_file",
            json!({"path":"notes.txt","old":"beta","new":"BETA"}),
        ))
        .unwrap();
        assert_eq!(first["notes"], json!([]));
        block_on(registry.call(
            "edit_file",
            json!({"path":"notes.txt","old":"gamma","new":"GAMMA"}),
        ))
        .unwrap();

        let other_run = run_registry(&root);
        let unread = block_on(other_run.call(
            "edit_file",
            json!({"path":"notes.txt","old":"alpha","new":"ALPHA"}),
        ))
        .unwrap();
        assert!(unread["notes"][0]
            .as_str()
            .unwrap()
            .contains("not read earlier in this run"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "ALPHA\nBETA\nGAMMA\n"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn edit_file_applies_whitespace_normalized_matches_and_explains_misses() {
        let root = temp_workspace();
        let path = root.join("main.py");
        std::fs::write(&path, "def run():\r\n    value = 1\r\n    return value\r\n").unwrap();
        let registry = run_registry(&root);

        let text = model_text(
            &registry,
            "edit_file",
            json!({"path":"main.py","old":"value = 1\nreturn value","new":"value = 2\nreturn value"}),
        );
        assert!(text.contains("ignoring whitespace differences"), "{text}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "def run():\r\n    value = 2\r\n    return value\r\n"
        );
        let error = block_on(registry.call(
            "edit_file",
            json!({"path":"main.py","old":"    valeu = 2\n    return valeu","new":"x"}),
        ))
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("most similar region is lines 2-3"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn write_and_list_tools_confirm_in_plain_text() {
        let root = temp_workspace();
        let registry = run_registry(&root);
        assert_eq!(
            model_text(
                &registry,
                "write_file",
                json!({"path":"src/a.txt","content":"one\ntwo\n"})
            ),
            "Created src/a.txt (2 lines)."
        );
        assert_eq!(
            model_text(
                &registry,
                "write_file",
                json!({"path":"src/a.txt","content":"one\n"})
            ),
            "Overwrote src/a.txt (1 line)."
        );
        assert_eq!(model_text(&registry, "list_dir", json!({})), "src/");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn glob_and_grep_tools_search_the_workspace_newest_first() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/old.rs"), "fn old() {}\n").unwrap();
        std::fs::write(root.join("src/nested/new.rs"), "fn new() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "fn in docs\n").unwrap();
        let old = std::fs::File::options()
            .write(true)
            .open(root.join("src/old.rs"))
            .unwrap();
        old.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))
            .unwrap();
        let registry = run_registry(&root);

        let found = block_on(registry.call("glob", json!({"pattern":"**/*.rs"}))).unwrap();
        assert_eq!(found["files"], json!(["src/nested/new.rs", "src/old.rs"]));
        let scoped =
            block_on(registry.call("glob", json!({"pattern":"*.rs","path":"src/nested"}))).unwrap();
        assert_eq!(scoped["files"], json!(["src/nested/new.rs"]));
        assert_eq!(
            model_text(&registry, "glob", json!({"pattern":"*.go"})),
            "No files matched *.go."
        );

        let matches =
            block_on(registry.call("grep", json!({"pattern":"^fn \\w+\\(","glob":"*.rs"})))
                .unwrap();
        assert_eq!(matches["files"], json!(["src/nested/new.rs", "src/old.rs"]));
        let content = model_text(
            &registry,
            "grep",
            json!({"pattern":"old","output_mode":"content"}),
        );
        assert_eq!(content, "src/old.rs:1:fn old() {}");
        assert!(block_on(registry.call("grep", json!({"pattern":"x","path":"../"}))).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn search_tools_reject_symlink_escapes() {
        let root = temp_workspace();
        let outside = temp_workspace();
        std::fs::write(outside.join("secret.rs"), "fn secret() {}\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.rs"), root.join("alias.rs")).unwrap();
        let registry = run_registry(&root);

        assert!(block_on(registry.call("glob", json!({"pattern":"*","path":"link"}))).is_err());
        assert!(
            block_on(registry.call("grep", json!({"pattern":"secret","path":"link"}))).is_err()
        );
        let all = block_on(registry.call("glob", json!({"pattern":"**"}))).unwrap();
        assert_eq!(all["files"], json!([]));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn shell_effect_and_deadline_depend_on_the_call() {
        let registry = run_registry(&temp_workspace());
        let effect = |args: Value| registry.effect_for_call("shell", &args).unwrap();
        let read_only = if cfg!(windows) {
            "Get-ChildItem"
        } else {
            "git status"
        };
        assert_eq!(effect(json!({"command": read_only})), ToolEffect::ReadOnly);
        assert_eq!(
            effect(json!({"command": "rm -rf build"})),
            ToolEffect::Command
        );
        assert_eq!(
            effect(json!({"command": read_only, "run_in_background": true})),
            ToolEffect::Command
        );
        assert_eq!(
            registry.effect_for_call("process_output", &json!({})),
            Some(ToolEffect::ReadOnly)
        );
        assert_eq!(
            registry.effect_for_call("process_kill", &json!({})),
            Some(ToolEffect::Command)
        );
        let tools = host_tools(Arc::new(RwLock::new(None)));
        let shell = tool(&tools, "shell");
        assert_eq!(
            shell.deadline_for_call(&json!({"command":"x","timeout_secs":300})),
            Some(std::time::Duration::from_secs(315))
        );
        assert_eq!(
            shell.deadline_for_call(&json!({"command":"x"})),
            Some(std::time::Duration::from_secs(135))
        );
        assert_eq!(
            shell.deadline_for_call(&json!({"command":"x","run_in_background":true})),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_timeout_kills_the_process_tree_and_keeps_partial_output() {
        let root = temp_workspace();
        let registry = run_registry(&root);
        let started = std::time::Instant::now();
        let result = block_on(registry.call(
            "shell",
            json!({"command":"echo started; sleep 30","timeout_secs":1}),
        ))
        .unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert_eq!(result["timed_out"], true);
        assert_eq!(result["stdout"], "started\n");
        let text = model_text(
            &registry,
            "shell",
            json!({"command":"sleep 30","timeout_secs":1}),
        );
        assert!(
            text.starts_with("exit code: none (timed out after 1 seconds"),
            "{text}"
        );
        assert!(
            block_on(registry.call("shell", json!({"command":"true","timeout_secs":601}))).is_err()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn shell_keeps_the_working_directory_between_commands() {
        let root = temp_workspace();
        std::fs::create_dir_all(root.join("sub/inner")).unwrap();
        let canonical = std::fs::canonicalize(&root).unwrap();
        let registry = run_registry(&root);

        let moved =
            block_on(registry.call("shell", json!({"command":"cd sub && echo moved"}))).unwrap();
        assert_eq!(moved["stdout"], "moved\n");
        assert_eq!(moved["cwd"], "sub");
        let here = block_on(registry.call("shell", json!({"command":"pwd -P"}))).unwrap();
        assert_eq!(
            here["stdout"].as_str().unwrap().trim(),
            canonical.join("sub").to_str().unwrap()
        );
        let text = model_text(&registry, "shell", json!({"command":"cd inner; exit 3"}));
        assert_eq!(text, "exit code: 3\n[cwd: sub]");

        let left = model_text(&registry, "shell", json!({"command":"cd /"}));
        assert!(left.contains("reset to the workspace root"), "{left}");
        let back = block_on(registry.call("shell", json!({"command":"pwd -P"}))).unwrap();
        assert_eq!(
            back["stdout"].as_str().unwrap().trim(),
            canonical.to_str().unwrap()
        );

        let other_run = run_registry(&root);
        block_on(other_run.call("shell", json!({"command":"cd sub"}))).unwrap();
        let fresh = block_on(registry.call("shell", json!({"command":"pwd -P"}))).unwrap();
        assert_eq!(
            fresh["stdout"].as_str().unwrap().trim(),
            canonical.to_str().unwrap()
        );

        let mut base = milim_tools::ToolRegistry::new();
        for item in host_tools(Arc::new(RwLock::new(Some(root.clone())))) {
            base.register(item);
        }
        let full = base.with_full_access(&root).scoped_for_run();
        block_on(full.call("shell", json!({"command":"cd /"}))).unwrap();
        let outside = block_on(full.call("shell", json!({"command":"pwd -P"}))).unwrap();
        assert_eq!(outside["stdout"], "/\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn background_processes_stream_output_and_die_with_their_run() {
        let root = temp_workspace();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let registry = run_registry(&root);
            let started = registry
                .call_for_agent(
                    "shell",
                    json!({"command":"echo ready; sleep 30","run_in_background":true}),
                )
                .await
                .unwrap();
            let id = started.result["process_id"].as_str().unwrap().to_string();
            assert!(started
                .model_text
                .unwrap()
                .starts_with(&format!("Started background process {id}")));

            let first = registry
                .call_for_agent("process_output", json!({"process_id": id, "wait_secs": 1}))
                .await
                .unwrap();
            assert_eq!(first.result["running"], true);
            assert_eq!(first.model_text.unwrap(), format!("{id}: running\nready"));
            let empty = registry
                .call("process_output", json!({"process_id": id}))
                .await
                .unwrap();
            assert_eq!(empty["output"], "");

            let killed = registry
                .call("process_kill", json!({"process_id": id}))
                .await
                .unwrap();
            assert_eq!(killed["killed"], true);
            assert_eq!(killed["running"], false);
            assert!(registry
                .call("process_output", json!({"process_id": id}))
                .await
                .is_err());

            let finished = registry
                .call(
                    "shell",
                    json!({"command":"echo done; exit 4","run_in_background":true}),
                )
                .await
                .unwrap();
            let finished_id = finished["process_id"].as_str().unwrap();
            let status = registry
                .call(
                    "process_output",
                    json!({"process_id": finished_id, "wait_secs": 10}),
                )
                .await
                .unwrap();
            assert_eq!(status["exit_code"], 4);
            assert_eq!(status["output"], "done\n");

            let lingering = registry
                .call(
                    "shell",
                    json!({"command":"sleep 30","run_in_background":true}),
                )
                .await
                .unwrap();
            let pid = lingering["pid"].as_u64().unwrap().to_string();
            drop(registry);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let alive = std::process::Command::new("kill")
                    .args(["-0", &pid])
                    .status()
                    .unwrap()
                    .success();
                if !alive {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "background process outlived its run"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });
        let _ = std::fs::remove_dir_all(root);
    }
}
