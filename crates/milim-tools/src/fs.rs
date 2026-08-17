//! Sandboxed filesystem tools (milim's "folder" tools).
//!
//! Each tool is rooted at a working directory and rejects any path that would
//! escape it (absolute paths or `..` components), so an agent can read/list/write
//! within a workspace without touching the rest of the machine.

use std::fs::OpenOptions;
use std::io::Write;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::{Tool, ToolConcurrency, ToolEffect};

/// Max bytes returned by `read_file`.
const MAX_READ: usize = 1024 * 1024;
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

pub fn read_text_range(path: &Path, offset: u64, limit: usize) -> Result<(String, u64, bool)> {
    let limit = limit.clamp(1, MAX_READ);
    let size = std::fs::metadata(path)?.len();
    if offset > size {
        return Err(Error::InvalidRequest(format!(
            "offset {offset} is past the end of the file ({size} bytes)"
        )));
    }
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; limit];
    let read = file.read(&mut bytes)?;
    bytes.truncate(read);
    let next_offset = offset + read as u64;
    Ok((
        String::from_utf8_lossy(&bytes).into_owned(),
        next_offset,
        next_offset >= size,
    ))
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

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace."
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
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    async fn invoke(&self, args: Value) -> Result<Value> {
        let path = safe_join(&self.root, arg_str(&args, "path")?)?;
        let offset = optional_u64(&args, "offset", 0)?;
        let limit =
            usize::try_from(optional_u64(&args, "limit", MAX_READ as u64)?).unwrap_or(usize::MAX);
        let (content, next_offset, eof) = read_text_range(&path, offset, limit)?;
        Ok(json!({ "content": content, "offset": offset, "next_offset": next_offset, "eof": eof }))
    }
}

/// List directory entries within the workspace.
pub struct ListDirTool {
    root: Arc<PathBuf>,
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
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = match args.get("path") {
            None => "",
            Some(value) => value
                .as_str()
                .ok_or_else(|| Error::InvalidRequest("path must be a string".into()))?,
        };
        let dir = safe_join(&self.root, rel)?;
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

/// Write a UTF-8 file within the workspace (creating parent dirs).
pub struct WriteFileTool {
    root: Arc<PathBuf>,
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
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = arg_str(&args, "path")?;
        let path = safe_join(&self.root, rel)?;
        let content = arg_str(&args, "content")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = safe_join(&self.root, rel)?;
        atomic_write(&path, content.as_bytes())?;
        Ok(json!({ "written": content.len() }))
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

        by("write_file")
            .invoke(json!({"path":"notes/a.txt","content":"hello"}))
            .await
            .unwrap();
        let read = by("read_file")
            .invoke(json!({"path":"notes/a.txt"}))
            .await
            .unwrap();
        assert_eq!(read["content"], "hello");

        let ranged = by("read_file")
            .invoke(json!({"path":"notes/a.txt","offset":1,"limit":2}))
            .await
            .unwrap();
        assert_eq!(ranged["content"], "el");
        assert_eq!(ranged["next_offset"], 3);
        assert_eq!(ranged["eof"], false);

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
