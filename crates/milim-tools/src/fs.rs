//! Sandboxed filesystem tools (milim's "folder" tools).
//!
//! Each tool is rooted at a working directory and rejects any path that would
//! escape it (absolute paths or `..` components), so an agent can read/list/write
//! within a workspace without touching the rest of the machine.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::{Tool, ToolConcurrency, ToolEffect};

/// Lines `read_file` returns when the caller gives no `limit`.
const DEFAULT_READ_LINES: usize = 2000;
/// Longer lines are cut so one minified line cannot fill the reply.
const MAX_LINE_CHARS: usize = 2000;
/// Upper bound on the text one `read_file` call returns.
const MAX_READ_BYTES: usize = 256 * 1024;
/// Leading bytes inspected to tell text from binary content.
const SNIFF_BYTES: u64 = 8192;
const MAX_LIST_ENTRIES: usize = 1000;

/// Resolve `rel` under `root`, rejecting absolute paths and `..` traversal.
pub fn resolve_workspace_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let canonical_root = std::fs::canonicalize(root)?;
    let mut out = canonical_root.clone();
    let components = Path::new(rel)
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            Component::CurDir => Ok(Default::default()),
            Component::ParentDir => {
                Err(Error::InvalidRequest("'..' is not allowed in paths".into()))
            }
            Component::RootDir | Component::Prefix(_) => Err(Error::InvalidRequest(
                "absolute paths are not allowed".into(),
            )),
        })
        .collect::<Result<Vec<_>>>()?;

    let mut missing = false;
    for component in components {
        if component.is_empty() {
            continue;
        }
        out.push(component);
        if missing {
            continue;
        }
        match std::fs::symlink_metadata(&out) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::InvalidRequest(
                        "workspace paths may not contain symlinks or junctions".into(),
                    ));
                }
                let canonical = std::fs::canonicalize(&out)?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(Error::InvalidRequest(
                        "path resolves outside the workspace".into(),
                    ));
                }
                out = canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(out)
}

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replace a file from a same-directory temporary file so a failed write does
/// not truncate the previous content.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidRequest("file path has no parent directory".into()))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let permissions = std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());

    let (temp_path, mut temp) = loop {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{file_name}.milim-{}-{suffix}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };

    let result = (|| -> std::io::Result<()> {
        temp.write_all(content)?;
        temp.sync_all()?;
        drop(temp);
        if let Some(permissions) = permissions {
            std::fs::set_permissions(&temp_path, permissions)?;
        }
        std::fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result.map_err(Into::into)
}

/// Read a line range of a text file as a `read_file` result. `offset` is the
/// 1-based first line (0 is treated as 1) and `limit` the maximum line count.
/// Lines are returned without their line endings; lines longer than 2000
/// characters are cut, and the reply stops early at 256 KiB. Binary files and
/// images are rejected with an error that names what they are.
pub fn read_text_range(path: &Path, offset: u64, limit: usize) -> Result<Value> {
    let metadata = std::fs::metadata(path)?;
    if metadata.is_dir() {
        return Err(Error::InvalidRequest(format!(
            "{} is a directory; use list_dir or glob",
            path.display()
        )));
    }
    let mut file = std::fs::File::open(path)?;
    let mut head = Vec::new();
    (&mut file).take(SNIFF_BYTES).read_to_end(&mut head)?;
    if let Some(kind) = image_kind(&head) {
        return Err(Error::InvalidRequest(format!(
            "{} is a {kind} image ({} bytes); read_file only returns text",
            path.display(),
            metadata.len()
        )));
    }
    if head.contains(&0) {
        return Err(Error::InvalidRequest(format!(
            "{} is a binary file ({} bytes); read_file only returns text",
            path.display(),
            metadata.len()
        )));
    }
    file.seek(SeekFrom::Start(0))?;

    let start = offset.max(1);
    let limit = limit.max(1);
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut content = String::new();
    let mut total = 0_u64;
    let mut shown = 0_usize;
    let mut cut_lines = 0_usize;
    let mut full = false;
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer)? == 0 {
            break;
        }
        total += 1;
        if total < start || shown >= limit || full {
            continue;
        }
        while matches!(buffer.last(), Some(b'\n' | b'\r')) {
            buffer.pop();
        }
        let text = String::from_utf8_lossy(&buffer);
        let line = match text.char_indices().nth(MAX_LINE_CHARS) {
            Some((cut, _)) => {
                cut_lines += 1;
                format!(
                    "{}... [line cut at {MAX_LINE_CHARS} characters]",
                    &text[..cut]
                )
            }
            None => text.into_owned(),
        };
        if shown > 0 && content.len() + line.len() + 1 > MAX_READ_BYTES {
            full = true;
            continue;
        }
        if shown > 0 {
            content.push('\n');
        }
        content.push_str(&line);
        shown += 1;
    }
    if total > 0 && start > total {
        return Err(Error::InvalidRequest(format!(
            "offset {start} is past the end of the file ({total} lines)"
        )));
    }
    let last = start + shown as u64 - 1;
    let eof = total == 0 || last >= total;
    Ok(json!({
        "content": content,
        "offset": start,
        "lines": shown,
        "total_lines": total,
        "next_offset": (!eof).then_some(last + 1),
        "eof": eof,
        "cut_lines": cut_lines,
    }))
}

fn image_kind(head: &[u8]) -> Option<&'static str> {
    const SIGNATURES: &[(&[u8], &str)] = &[
        (b"\x89PNG\r\n\x1a\n", "PNG"),
        (b"\xff\xd8\xff", "JPEG"),
        (b"GIF87a", "GIF"),
        (b"GIF89a", "GIF"),
        (b"\x00\x00\x01\x00", "ICO"),
        (b"II*\x00", "TIFF"),
        (b"MM\x00*", "TIFF"),
    ];
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some("WebP");
    }
    // "BM" alone also starts ordinary text; require the zeroed reserved field.
    if head.len() >= 14 && head.starts_with(b"BM") && head[6..10] == [0, 0, 0, 0] {
        return Some("BMP");
    }
    SIGNATURES
        .iter()
        .find(|(signature, _)| head.starts_with(signature))
        .map(|(_, kind)| *kind)
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    resolve_workspace_path(root, rel)
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

/// Build the sandboxed filesystem tools rooted at `root`.
pub fn fs_tools(root: impl Into<PathBuf>) -> Vec<Arc<dyn Tool>> {
    let root = Arc::new(root.into());
    vec![
        Arc::new(ReadFileTool { root: root.clone() }),
        Arc::new(ListDirTool { root: root.clone() }),
        Arc::new(WriteFileTool { root }),
    ]
}

/// Read a UTF-8 file within the workspace.
pub struct ReadFileTool {
    root: Arc<PathBuf>,
}

impl ReadFileTool {
    /// JSON schema shared by every `read_file` implementation.
    pub fn schema() -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "offset":{"type":"integer","minimum":1,"description":"1-based line number to start from, default 1."},
            "limit":{"type":"integer","minimum":1,"description":"Maximum number of lines, default 2000."}
        },"required":["path"],"additionalProperties":false})
    }

    /// The `(offset, limit)` line window requested by `read_file` arguments.
    pub fn line_window(args: &Value) -> Result<(u64, usize)> {
        let offset = optional_u64(args, "offset", 1)?;
        let limit = usize::try_from(optional_u64(args, "limit", DEFAULT_READ_LINES as u64)?)
            .unwrap_or(usize::MAX);
        Ok((offset, limit))
    }

    /// Resolve a path `read_file` may open under `root`. Absolute paths inside
    /// the registered tool output directory are accepted as well, so saved
    /// oversized tool output stays readable from workspace-scoped runs.
    pub fn resolve_read_path(root: &Path, rel: &str) -> Result<PathBuf> {
        let requested = Path::new(rel);
        if requested.is_absolute() {
            if let Some(output_root) = crate::tool_output_root() {
                if let (Ok(output_root), Ok(path)) = (
                    std::fs::canonicalize(output_root),
                    std::fs::canonicalize(requested),
                ) {
                    if path.starts_with(&output_root) {
                        return Ok(path);
                    }
                }
            }
        }
        resolve_workspace_path(root, rel)
    }

    /// Numbered-line text for the model: `  12\t<line>`, followed by a hint
    /// when the file continues past the returned range.
    pub fn render_for_model(result: &Value) -> Option<String> {
        let content = result.get("content")?.as_str()?;
        let start = result.get("offset")?.as_u64()?;
        let shown = result.get("lines")?.as_u64()?;
        let total = result.get("total_lines")?.as_u64()?;
        if total == 0 {
            return Some("(empty file)".into());
        }
        let mut out = String::with_capacity(content.len() + shown as usize * 8);
        for (index, line) in content.split('\n').enumerate() {
            let _ = writeln!(out, "{:>6}\t{line}", start + index as u64);
        }
        if let Some(next) = result.get("next_offset").and_then(Value::as_u64) {
            let _ = writeln!(
                out,
                "\n(Showing lines {start}-{} of {total}. Continue with offset={next}.)",
                next - 1
            );
        }
        let cut = result.get("cut_lines").and_then(Value::as_u64).unwrap_or(0);
        if cut > 0 {
            let _ = writeln!(
                out,
                "({cut} line(s) longer than {MAX_LINE_CHARS} characters were cut.)"
            );
        }
        Some(out.trim_end().to_string())
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace. Returns numbered lines; use offset/limit (in lines) for large files."
    }
    fn input_schema(&self) -> Value {
        Self::schema()
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        Self::render_for_model(result)
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = Self::resolve_read_path(&self.root, arg_str(&args, "path")?)?;
        let (offset, limit) = Self::line_window(&args)?;
        read_text_range(&path, offset, limit)
    }
}

/// List directory entries within the workspace.
pub struct ListDirTool {
    root: Arc<PathBuf>,
}

impl ListDirTool {
    /// Sorted, bounded entries of `dir` as a `list_dir` result.
    pub fn list(dir: &Path) -> Result<Value> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)? {
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

    /// One entry per line, directories suffixed with `/`.
    pub fn render_for_model(result: &Value) -> Option<String> {
        let entries = result.get("entries")?.as_array()?;
        if entries.is_empty() {
            return Some("(empty directory)".into());
        }
        let mut out = String::new();
        for entry in entries {
            let name = entry.get("name")?.as_str()?;
            let slash = if entry["is_dir"].as_bool().unwrap_or(false) {
                "/"
            } else {
                ""
            };
            let _ = writeln!(out, "{name}{slash}");
        }
        if result["truncated"].as_bool().unwrap_or(false) {
            let _ = writeln!(
                out,
                "(Listing stopped at {MAX_LIST_ENTRIES} entries; use glob to narrow it.)"
            );
        }
        Some(out.trim_end().to_string())
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        "List entries of a directory in the workspace (path defaults to root)."
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
        Self::render_for_model(result)
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = match args.get("path") {
            None => "",
            Some(value) => value
                .as_str()
                .ok_or_else(|| Error::InvalidRequest("path must be a string".into()))?,
        };
        Self::list(&safe_join(&self.root, rel)?)
    }
}

/// Write a UTF-8 file within the workspace (creating parent dirs).
pub struct WriteFileTool {
    root: Arc<PathBuf>,
}

impl WriteFileTool {
    /// The `write_file` result for `content` written to `path`.
    pub fn result(path: &str, content: &str, created: bool) -> Value {
        json!({
            "path": path,
            "written": content.len(),
            "lines": content.lines().count(),
            "created": created,
        })
    }

    /// A one-line confirmation naming the file, its line count, and whether
    /// it was created or overwritten.
    pub fn render_for_model(result: &Value) -> Option<String> {
        let path = result.get("path")?.as_str()?;
        let lines = result.get("lines")?.as_u64()?;
        let verb = if result["created"].as_bool()? {
            "Created"
        } else {
            "Overwrote"
        };
        let unit = if lines == 1 { "line" } else { "lines" };
        Some(format!("{verb} {path} ({lines} {unit})."))
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write a UTF-8 text file into the workspace (overwrites)."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        Self::render_for_model(result)
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = arg_str(&args, "path")?;
        let path = safe_join(&self.root, rel)?;
        let content = arg_str(&args, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = safe_join(&self.root, rel)?;
        let created = !path.exists();
        atomic_write(&path, content.as_bytes())?;
        Ok(Self::result(rel, content, created))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        // Unique per call (process id + atomic counter) so concurrently-running
        // tests don't share a dir and wipe each other's files.
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "milim-fs-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn write_read_list_round_trip() {
        let root = tmp();
        let tools = fs_tools(root.clone());
        let by = |n: &str| tools.iter().find(|t| t.name() == n).unwrap().clone();

        let written = by("write_file")
            .invoke(json!({"path":"notes/a.txt","content":"hello\nworld\nagain\n"}))
            .await
            .unwrap();
        assert_eq!(
            by("write_file").model_text(&written).unwrap(),
            "Created notes/a.txt (3 lines)."
        );
        let read = by("read_file")
            .invoke(json!({"path":"notes/a.txt"}))
            .await
            .unwrap();
        assert_eq!(read["content"], "hello\nworld\nagain");
        assert_eq!(read["total_lines"], 3);

        let ranged = by("read_file")
            .invoke(json!({"path":"notes/a.txt","offset":2,"limit":1}))
            .await
            .unwrap();
        assert_eq!(ranged["content"], "world");
        assert_eq!(ranged["next_offset"], 3);
        assert_eq!(ranged["eof"], false);
        let text = by("read_file").model_text(&ranged).unwrap();
        assert!(text.starts_with("     2\tworld\n"), "{text}");
        assert!(text.contains("Continue with offset=3"), "{text}");

        let list = by("list_dir")
            .invoke(json!({"path":"notes"}))
            .await
            .unwrap();
        let names: Vec<&str> = list["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert!(names.contains(&"a.txt"));
        assert_eq!(by("list_dir").model_text(&list).unwrap(), "a.txt");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_text_range_rejects_binary_and_images_and_cuts_long_lines() {
        let root = tmp();
        std::fs::write(root.join("blob.bin"), b"abc\0def").unwrap();
        std::fs::write(root.join("pixel.png"), b"\x89PNG\r\n\x1a\nrest").unwrap();
        std::fs::write(
            root.join("long.txt"),
            format!("{}\nshort\n", "x".repeat(2500)),
        )
        .unwrap();
        let binary = read_text_range(&root.join("blob.bin"), 1, 10)
            .unwrap_err()
            .to_string();
        assert!(binary.contains("binary file"), "{binary}");
        let image = read_text_range(&root.join("pixel.png"), 1, 10)
            .unwrap_err()
            .to_string();
        assert!(image.contains("PNG image"), "{image}");
        let long = read_text_range(&root.join("long.txt"), 1, 10).unwrap();
        assert_eq!(long["cut_lines"], 1);
        assert!(long["content"]
            .as_str()
            .unwrap()
            .ends_with("[line cut at 2000 characters]\nshort"));
        assert!(read_text_range(&root.join("long.txt"), 5, 10).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let root = tmp();
        let tools = fs_tools(root.clone());
        let read = tools.iter().find(|t| t.name() == "read_file").unwrap();
        assert!(read.invoke(json!({"path":"../secret"})).await.is_err());
        assert!(read.invoke(json!({"path":"/etc/passwd"})).await.is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn rejects_workspace_link_escape() {
        let root = tmp();
        let outside = tmp();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        let link = root.join("outside");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .status()
                .unwrap();
            if !status.success() {
                let _ = std::fs::remove_dir_all(&root);
                let _ = std::fs::remove_dir_all(&outside);
                return;
            }
        }

        let tools = fs_tools(root.clone());
        let read = tools
            .iter()
            .find(|tool| tool.name() == "read_file")
            .unwrap();
        let write = tools
            .iter()
            .find(|tool| tool.name() == "write_file")
            .unwrap();
        assert!(read
            .invoke(json!({"path":"outside/secret.txt"}))
            .await
            .is_err());
        assert!(write
            .invoke(json!({"path":"outside/new.txt","content":"no"}))
            .await
            .is_err());
        assert!(!outside.join("new.txt").exists());

        #[cfg(windows)]
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }
}
