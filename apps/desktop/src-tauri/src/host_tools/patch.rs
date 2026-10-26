//! `apply_patch`: multi-file patches in the Codex format that OpenAI models
//! are trained to write.
//!
//! ```text
//! *** Begin Patch
//! *** Update File: src/app.py
//! @@ def main():
//! -    print("hi")
//! +    print("hello")
//! *** Add File: src/new.py
//! +print("new")
//! *** Delete File: src/old.py
//! *** End Patch
//! ```
//!
//! Every hunk of every file is located and checked before anything is
//! written; hunks match like `edit_file` (whole lines, tolerating line-ending,
//! trailing-whitespace, and uniform indentation differences). Existing files
//! must pass the same read-before-write and stale-content checks as
//! `edit_file`, and a failed write restores the files already changed.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{atomic_write, Tool, ToolEffect};

use super::edit::{self, Plan, Replacement, MAX_HUNKS};
use super::lsp::{self, Diagnostic};
use super::{explain_io, host_tool_scoping, lock_file, missing_path, root_of, safe_join};
use super::{HostCtx, PathDisplay, RunState};

/// File operations one patch may carry.
const MAX_FILES: usize = 50;

/// One hunk of an `*** Update File` section.
#[derive(Debug, Default, PartialEq)]
struct Chunk {
    /// The `@@` line's text, naming a line (a function or class header) the
    /// hunk follows.
    context: Option<String>,
    /// Context and removed lines, in order.
    old: Vec<String>,
    /// Context and added lines, in order.
    new: Vec<String>,
    /// `*** End of File`: the hunk ends at the end of the file.
    eof: bool,
}

#[derive(Debug, PartialEq)]
enum Op {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

impl Op {
    fn path(&self) -> &str {
        match self {
            Op::Add { path, .. } | Op::Delete { path } | Op::Update { path, .. } => path,
        }
    }
}

fn file_marker(line: &str) -> bool {
    let line = line.trim();
    line == "*** End Patch"
        || ["*** Add File:", "*** Update File:", "*** Delete File:"]
            .iter()
            .any(|marker| line.starts_with(marker))
}

fn marker_path<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    line.trim()
        .strip_prefix(marker)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

/// Parse patch text. Errors name the 1-based patch line.
fn parse(text: &str) -> std::result::Result<Vec<Op>, String> {
    let lines = text.lines().collect::<Vec<_>>();
    let begin = lines
        .iter()
        .position(|line| line.trim() == "*** Begin Patch")
        .ok_or("the patch must start with a `*** Begin Patch` line")?;
    let end = lines
        .iter()
        .rposition(|line| line.trim() == "*** End Patch")
        .filter(|end| *end > begin)
        .ok_or("the patch must end with a `*** End Patch` line")?;
    let mut ops = Vec::new();
    let mut index = begin + 1;
    while index < end {
        let line = lines[index];
        let number = index + 1;
        index += 1;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(path) = marker_path(line, "*** Add File:") {
            let mut content = String::new();
            while index < end && !file_marker(lines[index]) {
                let body = lines[index].strip_prefix('+').ok_or_else(|| {
                    format!(
                        "line {}: every line of an added file must start with `+`",
                        index + 1
                    )
                })?;
                content.push_str(body);
                content.push('\n');
                index += 1;
            }
            ops.push(Op::Add {
                path: path.to_string(),
                content,
            });
        } else if let Some(path) = marker_path(line, "*** Delete File:") {
            ops.push(Op::Delete {
                path: path.to_string(),
            });
        } else if let Some(path) = marker_path(line, "*** Update File:") {
            let move_to = lines
                .get(index)
                .filter(|_| index < end)
                .and_then(|line| marker_path(line, "*** Move to:"))
                .map(str::to_string);
            if move_to.is_some() {
                index += 1;
            }
            let mut chunks: Vec<Chunk> = Vec::new();
            while index < end && !file_marker(lines[index]) {
                let line = lines[index];
                let number = index + 1;
                index += 1;
                let trimmed = line.trim();
                if let Some(header) = trimmed.strip_prefix("@@") {
                    chunks.push(Chunk {
                        context: chunk_context(header),
                        ..Chunk::default()
                    });
                    continue;
                }
                if trimmed == "*** End of File" {
                    if let Some(chunk) = chunks.last_mut() {
                        chunk.eof = true;
                    }
                    continue;
                }
                if chunks.is_empty() {
                    chunks.push(Chunk::default());
                }
                let chunk = chunks.last_mut().expect("a chunk was just ensured");
                match line.chars().next() {
                    None => {
                        chunk.old.push(String::new());
                        chunk.new.push(String::new());
                    }
                    Some(' ') => {
                        chunk.old.push(line[1..].to_string());
                        chunk.new.push(line[1..].to_string());
                    }
                    Some('-') => chunk.old.push(line[1..].to_string()),
                    Some('+') => chunk.new.push(line[1..].to_string()),
                    Some(_) => {
                        return Err(format!(
                            "line {number}: hunk lines must start with ' ' (context), '-' (removed), or '+' (added); found `{}`",
                            cut(line, 80)
                        ))
                    }
                }
            }
            chunks.retain(|chunk| !chunk.old.is_empty() || !chunk.new.is_empty());
            if chunks.is_empty() && move_to.is_none() {
                return Err(format!(
                    "line {number}: `*** Update File: {path}` has no hunks"
                ));
            }
            ops.push(Op::Update {
                path: path.to_string(),
                move_to,
                chunks,
            });
        } else {
            return Err(format!(
                "line {number}: expected `*** Add File:`, `*** Update File:`, or `*** Delete File:`, found `{}`",
                cut(line, 80)
            ));
        }
    }
    if ops.is_empty() {
        return Err("the patch changes no files".into());
    }
    if ops.len() > MAX_FILES {
        return Err(format!(
            "the patch changes {} files; split it into patches of at most {MAX_FILES}",
            ops.len()
        ));
    }
    Ok(ops)
}

/// The text of an `@@` header, or `None` for a bare `@@` or a unified-diff
/// range such as `@@ -12,7 +12,8 @@`, which carries no line to anchor on.
fn chunk_context(header: &str) -> Option<String> {
    let text = header.trim();
    let text = text.strip_suffix("@@").unwrap_or(text).trim();
    let range = text.starts_with('-')
        && text.contains(" +")
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | ',' | ' '));
    (!text.is_empty() && !range).then(|| text.to_string())
}

fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((index, _)) => format!("{}...", &text[..index]),
        None => text.to_string(),
    }
}

/// The byte just past the line ending at or after `at`.
fn next_line_start(content: &str, at: usize) -> usize {
    content[at..]
        .find('\n')
        .map(|offset| at + offset + 1)
        .unwrap_or(content.len())
}

/// The start of the line after the first line at or after `from` whose
/// trimmed text is `context`, or contains it.
fn find_context_line(content: &str, from: usize, context: &str) -> Option<usize> {
    let mut start = from;
    let mut contained = None;
    while start < content.len() {
        let end = next_line_start(content, start);
        let line = content[start..end].trim();
        if line == context {
            return Some(end);
        }
        if contained.is_none() && line.contains(context) {
            contained = Some(end);
        }
        start = end;
    }
    contained
}

fn lines_text(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    text
}

/// Locate every chunk in order, each after the previous one. Errors carry
/// the failing chunk's index.
fn plan_update(content: &str, chunks: &[Chunk]) -> std::result::Result<Plan, (usize, String)> {
    let mut replacements = Vec::new();
    let mut normalized = false;
    let mut cursor = 0;
    for (index, chunk) in chunks.iter().enumerate() {
        // An `@@` line the file lacks only loses its anchoring; the hunk's
        // own lines still have to match.
        let anchored = chunk
            .context
            .as_deref()
            .and_then(|context| find_context_line(content, cursor, context));
        if let Some(after) = anchored {
            cursor = after;
        }
        if chunk.old.is_empty() {
            let at = anchored.unwrap_or(content.len());
            let mut text = lines_text(&chunk.new);
            if at == content.len() && !content.is_empty() && !content.ends_with('\n') {
                text.insert(0, '\n');
                text.pop();
            }
            replacements.push(Replacement {
                start: at,
                end: at,
                text: edit::in_file_style(content, &text),
            });
            continue;
        }
        let old = lines_text(&chunk.old);
        let new = lines_text(&chunk.new);
        let Some((mut found, loose)) = edit::locate_lines(content, cursor, &old, &new, chunk.eof)
        else {
            return Err((
                index,
                edit::not_found(content, &old).replacen(
                    "'old' text was not found",
                    "The hunk's context and removed lines were not found",
                    1,
                ),
            ));
        };
        // A whitespace-tolerant match stops before the line ending, which a
        // hunk that removes whole lines must take too.
        if found.text.is_empty()
            && found.end < content.len()
            && !content[..found.end].ends_with('\n')
        {
            found.end = next_line_start(content, found.end);
        }
        normalized |= loose;
        cursor = if found.end > 0 && content[..found.end].ends_with('\n') {
            found.end
        } else {
            next_line_start(content, found.end)
        };
        replacements.push(found);
    }
    replacements.sort_by_key(|replacement| (replacement.start, replacement.end));
    Ok(Plan {
        replacements,
        normalized,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Add,
    Update,
    Delete,
}

/// One file change, checked and ready to write.
struct Planned {
    action: Action,
    path: PathBuf,
    shown: String,
    /// Destination of an `*** Move to`.
    dest: Option<(PathBuf, String)>,
    /// Content before the patch, when the file existed.
    before: Option<String>,
    /// Content after the patch; `None` deletes the file.
    after: Option<String>,
    diff: String,
    added: usize,
    removed: usize,
    normalized: bool,
}

/// How to put a file back if a later write fails.
enum Undo {
    Restore(PathBuf, String),
    Remove(PathBuf),
}

fn undo_all(undo: Vec<Undo>) {
    for step in undo.into_iter().rev() {
        let _ = match step {
            Undo::Restore(path, content) => atomic_write(&path, content.as_bytes()),
            Undo::Remove(path) => std::fs::remove_file(path).map_err(Into::into),
        };
    }
}

/// Apply a multi-file patch in the Codex `*** Begin Patch` format.
pub struct ApplyPatchTool {
    pub(super) ctx: HostCtx,
}

impl ApplyPatchTool {
    /// Read an existing file the patch changes, enforcing the run's
    /// read-before-write and freshness checks.
    fn read_existing(&self, path: &Path, shown: &str) -> Result<String> {
        if path.is_dir() {
            return Err(Error::InvalidRequest(format!(
                "{shown} is a directory, not a file"
            )));
        }
        if !path.is_file() {
            return Err(missing_path(&self.ctx, path, shown, "apply_patch"));
        }
        self.ctx.run.require_read(path, shown, "apply_patch")?;
        std::fs::read_to_string(path)
            .map_err(|error| explain_io(&self.ctx, error.into(), path, shown, "apply_patch"))
    }

    fn plan(&self, op: &Op, display: &PathDisplay) -> Result<Planned> {
        let path = safe_join(&self.ctx.ws, op.path())?;
        let shown = display.show(&path);
        match op {
            Op::Add { content, .. } => {
                let before = if path.exists() {
                    Some(self.read_existing(&path, &shown)?)
                } else {
                    None
                };
                Ok(Planned {
                    action: Action::Add,
                    added: content.lines().count(),
                    removed: before.as_deref().map_or(0, |before| before.lines().count()),
                    after: Some(content.clone()),
                    before,
                    path,
                    shown,
                    dest: None,
                    diff: String::new(),
                    normalized: false,
                })
            }
            Op::Delete { .. } => {
                let before = self.read_existing(&path, &shown)?;
                Ok(Planned {
                    action: Action::Delete,
                    added: 0,
                    removed: before.lines().count(),
                    before: Some(before),
                    after: None,
                    path,
                    shown,
                    dest: None,
                    diff: String::new(),
                    normalized: false,
                })
            }
            Op::Update {
                move_to, chunks, ..
            } => {
                let before = self.read_existing(&path, &shown)?;
                let dest = match move_to {
                    Some(target) => {
                        let dest = safe_join(&self.ctx.ws, target)?;
                        let dest_shown = display.show(&dest);
                        if dest.exists() && RunState::key(&dest) != RunState::key(&path) {
                            return Err(Error::InvalidRequest(format!(
                                "cannot move {shown} to {dest_shown}: it already exists. Delete it in the same patch first, or pick another name. No file was changed."
                            )));
                        }
                        Some((dest, dest_shown))
                    }
                    None => None,
                };
                let plan = plan_update(&before, chunks).map_err(|(index, message)| {
                    Error::InvalidRequest(format!(
                        "{shown}: hunk {} of {}: {message} No file was changed.",
                        index + 1,
                        chunks.len()
                    ))
                })?;
                let (mut diff, added, removed) = edit::render_diff(&before, &plan, MAX_HUNKS);
                if plan.replacements.len() > MAX_HUNKS {
                    let _ = writeln!(
                        diff,
                        "... {} more hunk(s) not shown",
                        plan.replacements.len() - MAX_HUNKS
                    );
                }
                Ok(Planned {
                    action: Action::Update,
                    after: Some(edit::apply(&before, &plan)),
                    before: Some(before),
                    normalized: plan.normalized,
                    path,
                    shown,
                    dest,
                    diff,
                    added,
                    removed,
                })
            }
        }
    }

    /// Write every planned change, restoring the earlier ones if one fails.
    fn write_all(&self, planned: &[Planned]) -> Result<()> {
        let mut undo = Vec::new();
        for change in planned {
            let written = (|| -> Result<()> {
                let target = change.dest.as_ref().map_or(&change.path, |(dest, _)| dest);
                if let Some(after) = &change.after {
                    atomic_write(target, after.as_bytes())?;
                    undo.push(match (&change.dest, &change.before) {
                        (None, Some(before)) => Undo::Restore(target.clone(), before.clone()),
                        _ => Undo::Remove(target.clone()),
                    });
                }
                if change.after.is_none() || change.dest.is_some() {
                    std::fs::remove_file(&change.path)?;
                    if let Some(before) = &change.before {
                        undo.push(Undo::Restore(change.path.clone(), before.clone()));
                    }
                }
                Ok(())
            })();
            if let Err(error) = written {
                undo_all(undo);
                let message = match explain_io(
                    &self.ctx,
                    error,
                    &change.path,
                    &change.shown,
                    "apply_patch",
                ) {
                    Error::InvalidRequest(message) => message,
                    other => other.to_string(),
                };
                return Err(Error::InvalidRequest(format!(
                    "{message} The files this patch had already changed were restored."
                )));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }
    fn description(&self) -> &str {
        "Apply a patch that adds, updates, deletes, or renames files, all in one call and all or nothing. Use it for changes that span several files or several places in one file; read each existing file with read_file first. For a single small replacement edit_file is simpler; for a brand-new file alone write_file is.\n\
         Format (paths relative to the working folder):\n\
         *** Begin Patch\n\
         *** Update File: src/app.py\n\
         @@ def main():\n\
         -    print(\"hi\")\n\
         +    print(\"hello\")\n\
         *** Add File: src/util.py\n\
         +VALUE = 1\n\
         *** Delete File: src/old.py\n\
         *** End Patch\n\
         Hunk lines start with ' ' (unchanged context, about 3 lines around each change), '-' (removed), or '+' (added); an optional `@@ line` names the function or class the hunk is in. Put `*** Move to: new/path` right after an Update line to rename the file. Hunks match like edit_file, tolerating whitespace differences; if any hunk fails, nothing is written and the error shows the closest region. Returns a diff per file."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "input":{"type":"string","description":"The whole patch, from `*** Begin Patch` to `*** End Patch`."}
        },"required":["input"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        render_for_model(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let text = ["input", "patch"]
            .iter()
            .find_map(|key| args.get(*key).and_then(Value::as_str))
            .ok_or_else(|| Error::InvalidRequest("missing string argument: input".into()))?;
        let ops = parse(text).map_err(|message| {
            Error::InvalidRequest(format!("invalid patch: {message}. No file was changed."))
        })?;
        let root = root_of(&self.ctx.ws)?;
        let display = PathDisplay::new(&root);

        // Lock every path the patch touches, in one order, before checking.
        let mut paths = Vec::new();
        for op in &ops {
            paths.push(RunState::key(&safe_join(&self.ctx.ws, op.path())?));
            if let Op::Update {
                move_to: Some(target),
                ..
            } = op
            {
                paths.push(RunState::key(&safe_join(&self.ctx.ws, target)?));
            }
        }
        let mut unique = paths.clone();
        unique.sort();
        unique.dedup();
        if unique.len() < paths.len() {
            return Err(Error::InvalidRequest(
                "the patch names the same file more than once; put all of a file's hunks under one `*** Update File` section. No file was changed.".into(),
            ));
        }
        let mut locks = Vec::with_capacity(unique.len());
        for path in &unique {
            locks.push(lock_file(path).await);
        }

        let planned = ops
            .iter()
            .map(|op| self.plan(op, &display))
            .collect::<Result<Vec<_>>>()?;
        for change in &planned {
            if let Some(before) = &change.before {
                if std::fs::read_to_string(&change.path).ok().as_ref() != Some(before) {
                    return Err(Error::InvalidRequest(format!(
                        "{} changed while apply_patch was running; read it again. No file was changed.",
                        change.shown
                    )));
                }
            }
        }
        self.write_all(&planned)?;
        drop(locks);

        let mut checks = tokio::task::JoinSet::new();
        for (index, change) in planned.iter().enumerate() {
            let target = change.dest.as_ref().map_or(&change.path, |(dest, _)| dest);
            if let Some(after) = &change.after {
                self.ctx.run.touch(target);
                let (lsp, root, target, after) = (
                    self.ctx.lsp.clone(),
                    root.clone(),
                    target.clone(),
                    after.clone(),
                );
                checks.spawn(async move { (index, lsp.after_edit(&root, &target, &after).await) });
            }
        }
        let mut diagnostics: Vec<Option<Vec<Diagnostic>>> = vec![None; planned.len()];
        while let Some(Ok((index, found))) = checks.join_next().await {
            diagnostics[index] = found.filter(|found| !found.is_empty());
        }

        let files = planned
            .iter()
            .zip(diagnostics)
            .map(|(change, diagnostics)| {
                let mut file = json!({
                    "path": change.shown,
                    "action": match (change.action, &change.dest, &change.before) {
                        (Action::Add, _, Some(_)) => "replace",
                        (Action::Add, _, None) => "add",
                        (Action::Delete, _, _) => "delete",
                        (Action::Update, Some(_), _) => "move",
                        (Action::Update, None, _) => "update",
                    },
                    "added": change.added,
                    "removed": change.removed,
                    "normalized": change.normalized,
                    "diff": change.diff,
                });
                if let Some((_, dest)) = &change.dest {
                    file["moved_to"] = json!(dest);
                }
                if let Some(diagnostics) = diagnostics {
                    file["diagnostics"] = json!(diagnostics);
                }
                file
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "files": files,
            "added": planned.iter().map(|change| change.added).sum::<usize>(),
            "removed": planned.iter().map(|change| change.removed).sum::<usize>(),
        }))
    }
}

fn plural(count: u64, singular: &str) -> String {
    format!("{count} {singular}{}", if count == 1 { "" } else { "s" })
}

fn render_for_model(result: &Value) -> Option<String> {
    let files = result.get("files")?.as_array()?;
    let mut out = format!(
        "Applied patch: {} changed.\n",
        plural(files.len() as u64, "file")
    );
    let mut diagnostics = Vec::new();
    for file in files {
        let path = file.get("path")?.as_str()?;
        let (added, removed) = (file["added"].as_u64()?, file["removed"].as_u64()?);
        let _ = match file["action"].as_str()? {
            "add" => writeln!(out, "Added {path} ({}).", plural(added, "line")),
            "replace" => writeln!(out, "Replaced {path} ({}).", plural(added, "line")),
            "delete" => writeln!(out, "Deleted {path}."),
            "move" => writeln!(
                out,
                "Moved {path} to {} (+{added} -{removed}):",
                file["moved_to"].as_str()?
            ),
            _ => writeln!(out, "Updated {path} (+{added} -{removed}):"),
        };
        out.push_str(file["diff"].as_str().unwrap_or(""));
        if file["normalized"].as_bool() == Some(true) {
            let _ = writeln!(
                out,
                "(Matched {path} after ignoring whitespace differences; the file's indentation and line endings were kept.)"
            );
        }
        if let Some(block) = lsp::render_after_edit(file) {
            diagnostics.push(block.replacen("Diagnostics after edit:\n", "", 1));
        }
    }
    let mut out = out.trim_end().to_string();
    if !diagnostics.is_empty() {
        out.push_str("\n\nDiagnostics after edit:\n");
        out.push_str(&diagnostics.join("\n"));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(context: Option<&str>, old: &[&str], new: &[&str]) -> Chunk {
        Chunk {
            context: context.map(str::to_string),
            old: old.iter().map(|line| line.to_string()).collect(),
            new: new.iter().map(|line| line.to_string()).collect(),
            eof: false,
        }
    }

    #[test]
    fn parses_every_operation_and_reports_bad_lines() {
        let ops = parse(
            "noise before\n*** Begin Patch\n*** Add File: a.txt\n+one\n+two\n*** Update File: src/b.py\n*** Move to: src/c.py\n@@ def main():\n context\n-old\n+new\n\n@@ -10,2 +10,2 @@\n-x\n+y\n*** End of File\n*** Delete File: gone.txt\n*** End Patch\n",
        )
        .unwrap();
        assert_eq!(
            ops,
            vec![
                Op::Add {
                    path: "a.txt".into(),
                    content: "one\ntwo\n".into()
                },
                Op::Update {
                    path: "src/b.py".into(),
                    move_to: Some("src/c.py".into()),
                    chunks: vec![
                        chunk(
                            Some("def main():"),
                            &["context", "old", ""],
                            &["context", "new", ""]
                        ),
                        Chunk {
                            eof: true,
                            ..chunk(None, &["x"], &["y"])
                        },
                    ],
                },
                Op::Delete {
                    path: "gone.txt".into()
                },
            ]
        );
        assert!(parse("*** Update File: a\n-x\n")
            .unwrap_err()
            .contains("Begin Patch"));
        assert!(parse("*** Begin Patch\n*** Add File: a\n+x\n")
            .unwrap_err()
            .contains("End Patch"));
        let bad = parse("*** Begin Patch\n*** Update File: a\n@@\n x\nbad line\n*** End Patch")
            .unwrap_err();
        assert!(
            bad.starts_with("line 5: hunk lines must start with"),
            "{bad}"
        );
        let bad = parse("*** Begin Patch\n*** Add File: a\nno plus\n*** End Patch").unwrap_err();
        assert!(bad.contains("line 3"), "{bad}");
        assert!(parse("*** Begin Patch\n*** End Patch").is_err());
    }

    #[test]
    fn update_hunks_apply_in_order_with_tolerant_matching() {
        let content = "def a():\n    return 1\n\ndef b():\n    return 1\n";
        // The same lines appear twice; the `@@` header picks the second.
        let plan = plan_update(
            content,
            &[chunk(
                Some("def b():"),
                &["    return 1"],
                &["    return 2"],
            )],
        )
        .unwrap();
        assert_eq!(
            edit::apply(content, &plan),
            "def a():\n    return 1\n\ndef b():\n    return 2\n"
        );
        // Without a header, hunks apply to the first match after the previous hunk.
        let plan = plan_update(
            content,
            &[
                chunk(None, &["    return 1"], &["    return 10"]),
                chunk(None, &["    return 1"], &["    return 20"]),
            ],
        )
        .unwrap();
        assert_eq!(
            edit::apply(content, &plan),
            "def a():\n    return 10\n\ndef b():\n    return 20\n"
        );
        // Whitespace differences are tolerated and the file's style kept.
        let crlf = "if x:\r\n    go()   \r\n    stop()\r\n";
        let plan = plan_update(crlf, &[chunk(None, &["go()", "stop()"], &["go()"])]).unwrap();
        assert!(plan.normalized);
        assert_eq!(edit::apply(crlf, &plan), "if x:\r\n    go()\r\n");
        // Removing whole lines takes their line endings too.
        let plan = plan_update(crlf, &[chunk(None, &["  stop()"], &[])]).unwrap();
        assert_eq!(edit::apply(crlf, &plan), "if x:\r\n    go()   \r\n");
        // Pure additions go after the `@@` line, or at the end of the file.
        let plan = plan_update("a\nb", &[chunk(None, &[], &["c"])]).unwrap();
        assert_eq!(edit::apply("a\nb", &plan), "a\nb\nc");
        let plan = plan_update("a\nb\n", &[chunk(Some("a"), &[], &["a2"])]).unwrap();
        assert_eq!(edit::apply("a\nb\n", &plan), "a\na2\nb\n");

        let (index, message) = plan_update(
            content,
            &[
                chunk(None, &["def a():"], &["def a0():"]),
                chunk(None, &["def bee():", "    retrun 1"], &["x"]),
            ],
        )
        .unwrap_err();
        assert_eq!(index, 1);
        assert!(
            message.starts_with("The hunk's context and removed lines were not found. The most similar region is lines 4-5"),
            "{message}"
        );
    }

    #[test]
    fn unified_diff_ranges_are_not_anchors() {
        assert_eq!(chunk_context(" -12,7 +12,8 @@"), None);
        assert_eq!(chunk_context(""), None);
        assert_eq!(chunk_context(" class Foo:"), Some("class Foo:".into()));
    }
}
