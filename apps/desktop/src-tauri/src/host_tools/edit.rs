//! `edit_file`: exact string replacement that tolerates whitespace-only
//! differences, explains near misses, and reports a unified diff snippet.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{atomic_write, Tool, ToolEffect};

use super::{
    arg_str, attach_diagnostics, has_mixed_newlines, host_tool_scoping, lock_file, lsp,
    newline_separator, optional_bool, path_schema, safe_join, HostCtx,
};

/// Unchanged lines shown around each change.
const CONTEXT_LINES: usize = 3;
/// Changes rendered in one reply; the rest are counted.
const MAX_HUNKS: usize = 3;
/// Average per-line similarity a region needs to be suggested.
const MIN_SIMILARITY: f64 = 0.5;
/// Bound on line comparisons spent looking for the closest region.
const MAX_SIMILARITY_WORK: usize = 4_000_000;
/// Edits one `edits` array may carry.
const MAX_EDITS: usize = 50;
/// Columns a tab counts as when comparing relative indentation.
const TAB_WIDTH: usize = 4;

/// Replace text in a file (a surgical code edit).
pub struct EditFileTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        if self.ctx.full_access() {
            "Replace text in an existing file anywhere on the host (relative paths use the working folder). Read the file with read_file in this run first. 'old' must match exactly once unless replace_all is set; for several changes to one file pass `edits` instead, applied in order and all-or-nothing. A match that differs only in trailing whitespace, line endings, or a uniform indentation shift is applied and reported; otherwise the error shows the closest region. Returns a diff of the change."
        } else {
            "Replace text in an existing file in the working folder. Read the file with read_file in this run first. 'old' must match exactly once unless replace_all is set; for several changes to one file pass `edits` instead, applied in order and all-or-nothing. A match that differs only in trailing whitespace, line endings, or a uniform indentation shift is applied and reported; otherwise the error shows the closest region. Returns a diff of the change."
        }
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":path_schema(&self.ctx),
            "old":{"type":"string","description":"Exact text to replace, including its indentation. It must be unique in the file unless replace_all is set."},
            "new":{"type":"string","description":"Replacement text."},
            "replace_all":{"type":"boolean","description":"Replace every occurrence of 'old'. Default false."},
            "edits":{"type":"array","maxItems":MAX_EDITS,"description":"Several replacements for this file instead of old/new, applied in order (each sees the result of the previous one). If any fails, the file is left unchanged.","items":{"type":"object","properties":{
                "old":{"type":"string"},
                "new":{"type":"string"},
                "replace_all":{"type":"boolean"}
            },"required":["old","new"],"additionalProperties":false}}
        },"required":["path"]})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        let path = result.get("path")?.as_str()?;
        let replaced = result.get("replaced")?.as_u64()?;
        let unit = if replaced == 1 {
            "replacement"
        } else {
            "replacements"
        };
        let edits = result["edits"].as_u64().unwrap_or(1);
        let mut out = if edits > 1 {
            format!("Edited {path}: {edits} edits, {replaced} {unit}.\n")
        } else {
            format!("Edited {path}: {replaced} {unit}.\n")
        };
        for note in result["notes"].as_array().into_iter().flatten() {
            if let Some(note) = note.as_str() {
                let _ = writeln!(out, "{note}");
            }
        }
        out.push_str(result.get("diff")?.as_str()?);
        lsp::with_after_edit(Some(out.trim_end().to_string()), result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = arg_str(&args, "path")?;
        let path = safe_join(&self.ctx.ws, rel)?;
        let edits = requested_edits(&args)?;
        let _lock = lock_file(&path).await;
        if !path.is_file() {
            return Err(Error::InvalidRequest(format!(
                "{rel} does not exist; use write_file to create a new file"
            )));
        }
        self.ctx.run.require_read(&path, rel, "edit_file")?;
        let content = std::fs::read_to_string(&path)?;
        let mut updated = content.clone();
        let (mut diff, mut added, mut removed, mut replaced) = (String::new(), 0, 0, 0);
        let mut normalized = false;
        for (index, edit) in edits.iter().enumerate() {
            let plan =
                plan_edit(&updated, edit.old, edit.new, edit.replace_all).map_err(|message| {
                    Error::InvalidRequest(if edits.len() > 1 {
                        format!(
                            "{rel}: edit {} of {}: {message} No edit was applied.",
                            index + 1,
                            edits.len()
                        )
                    } else {
                        format!("{rel}: {message}")
                    })
                })?;
            let (hunks, plus, minus) =
                render_diff(&updated, &plan, MAX_HUNKS.saturating_sub(replaced));
            diff.push_str(&hunks);
            added += plus;
            removed += minus;
            replaced += plan.replacements.len();
            normalized |= plan.normalized;
            updated = apply(&updated, &plan);
        }
        if replaced > MAX_HUNKS {
            let _ = writeln!(
                diff,
                "... {} more replacement(s) not shown",
                replaced - MAX_HUNKS
            );
        }
        if std::fs::read_to_string(&path)? != content {
            return Err(Error::InvalidRequest(
                "file changed while edit_file was running; read it again".into(),
            ));
        }
        atomic_write(&path, updated.as_bytes())?;
        self.ctx.run.touch(&path);
        let mut notes = Vec::new();
        if normalized {
            notes.push("Matched after ignoring whitespace differences; the file's indentation and line endings were kept.");
        }
        let mut result = json!({
            "path": rel,
            "replaced": replaced,
            "edits": edits.len(),
            "bytes": updated.len(),
            "added": added,
            "removed": removed,
            "normalized": normalized,
            "diff": diff,
            "notes": notes,
        });
        attach_diagnostics(&self.ctx, &path, &updated, &mut result).await;
        Ok(result)
    }
}

/// One requested replacement.
struct RequestedEdit<'a> {
    old: &'a str,
    new: &'a str,
    replace_all: bool,
}

/// The replacements an `edit_file` call asks for: its `edits` array, or its
/// top-level `old`/`new` pair.
fn requested_edits(args: &Value) -> Result<Vec<RequestedEdit<'_>>> {
    let edits = match args.get("edits") {
        None | Some(Value::Null) => vec![RequestedEdit {
            old: arg_str(args, "old")?,
            new: arg_str(args, "new")?,
            replace_all: optional_bool(args, "replace_all")?,
        }],
        Some(Value::Array(items)) => {
            if args.get("old").is_some() || args.get("new").is_some() {
                return Err(Error::InvalidRequest(
                    "pass either old/new or edits, not both".into(),
                ));
            }
            if items.is_empty() || items.len() > MAX_EDITS {
                return Err(Error::InvalidRequest(format!(
                    "edits must hold 1 to {MAX_EDITS} replacements"
                )));
            }
            items
                .iter()
                .map(|item| {
                    Ok(RequestedEdit {
                        old: arg_str(item, "old")?,
                        new: arg_str(item, "new")?,
                        replace_all: optional_bool(item, "replace_all")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        }
        Some(_) => return Err(Error::InvalidRequest("edits must be an array".into())),
    };
    for edit in &edits {
        if edit.old.is_empty() {
            return Err(Error::InvalidRequest(
                "'old' must not be empty; use write_file to create or replace a whole file".into(),
            ));
        }
        if edit.old == edit.new {
            return Err(Error::InvalidRequest(
                "'old' and 'new' are identical; nothing to change".into(),
            ));
        }
    }
    Ok(edits)
}

/// Bytes `start..end` of the original content become `text`.
#[derive(Debug)]
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug)]
struct Plan {
    /// Ascending and non-overlapping.
    replacements: Vec<Replacement>,
    /// Whether the match needed whitespace normalization.
    normalized: bool,
}

/// `text` with every line ending rewritten to `newline`.
fn with_newlines(text: &str, newline: &str) -> String {
    let unix = text.replace("\r\n", "\n");
    if newline == "\n" {
        unix
    } else {
        unix.replace('\n', newline)
    }
}

/// Find where `old` applies: verbatim, then in the file's line-ending style,
/// then line by line ignoring leading and trailing whitespace.
fn plan_edit(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> std::result::Result<Plan, String> {
    let mixed = has_mixed_newlines(content);
    let newline = newline_separator(content);
    let in_file_style = |text: &str| {
        if mixed {
            text.to_string()
        } else {
            with_newlines(text, newline)
        }
    };
    let text = in_file_style(new);
    let converted = in_file_style(old);
    let candidates = if converted == old {
        vec![old]
    } else {
        vec![old, converted.as_str()]
    };
    for candidate in candidates {
        let starts: Vec<usize> = content
            .match_indices(candidate)
            .map(|(start, _)| start)
            .collect();
        if starts.is_empty() {
            continue;
        }
        if starts.len() > 1 && !replace_all {
            let lines = starts
                .iter()
                .map(|start| line_number_at(content, *start))
                .collect::<Vec<_>>();
            return Err(format!(
                "'old' text is not unique ({} matches, at lines {}) - include more surrounding context or set replace_all",
                starts.len(),
                list_lines(&lines)
            ));
        }
        return Ok(Plan {
            replacements: starts
                .into_iter()
                .map(|start| Replacement {
                    start,
                    end: start + candidate.len(),
                    text: text.clone(),
                })
                .collect(),
            normalized: false,
        });
    }
    if let Some(plan) = normalized_plan(content, old, new, replace_all, &in_file_style)? {
        return Ok(plan);
    }
    Err(not_found(content, old))
}

fn line_number_at(content: &str, byte: usize) -> usize {
    content[..byte].matches('\n').count() + 1
}

fn list_lines(lines: &[usize]) -> String {
    let mut shown = lines
        .iter()
        .take(10)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    if lines.len() > 10 {
        shown.push_str(", ...");
    }
    shown
}

/// Byte span of one line, excluding its line ending.
#[derive(Clone, Copy, Debug)]
struct LineSpan {
    start: usize,
    end: usize,
}

fn line_spans(content: &str) -> Vec<LineSpan> {
    let mut spans = Vec::new();
    let mut start = 0;
    while start < content.len() {
        let next = content[start..]
            .find('\n')
            .map(|offset| start + offset)
            .unwrap_or(content.len());
        let end = if next > start && content.as_bytes()[next - 1] == b'\r' {
            next - 1
        } else {
            next
        };
        spans.push(LineSpan { start, end });
        start = next + 1;
    }
    spans
}

fn span_text(content: &str, span: LineSpan) -> &str {
    &content[span.start..span.end]
}

/// Lines of a search/replacement text without its final line ending, and
/// whether it had one.
fn pattern_lines(text: &str) -> (Vec<String>, bool) {
    let unix = text.replace("\r\n", "\n");
    let trailing = unix.ends_with('\n');
    let body = unix.strip_suffix('\n').unwrap_or(&unix);
    (
        body.split('\n').map(ToString::to_string).collect(),
        trailing,
    )
}

fn indentation(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn normalized_plan(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
    in_file_style: &dyn Fn(&str) -> String,
) -> std::result::Result<Option<Plan>, String> {
    let spans = line_spans(content);
    let (old_lines, trailing) = pattern_lines(old);
    if old_lines.iter().all(|line| line.trim().is_empty()) || old_lines.len() > spans.len() {
        return Ok(None);
    }
    let width = old_lines.len();
    let mut found = Vec::new();
    let mut index = 0;
    while index + width <= spans.len() {
        let window = &spans[index..index + width];
        let matches = old_lines
            .iter()
            .zip(window)
            .all(|(line, span)| span_text(content, *span).trim() == line.trim())
            && same_relative_indentation(&old_lines, content, window);
        if matches {
            found.push(index);
            index += width;
        } else {
            index += 1;
        }
    }
    if found.is_empty() {
        return Ok(None);
    }
    if found.len() > 1 && !replace_all {
        let lines = found.iter().map(|index| index + 1).collect::<Vec<_>>();
        return Err(format!(
            "'old' text only matches when whitespace is ignored, and then it matches {} places (lines {}) - include more surrounding context or set replace_all",
            found.len(),
            list_lines(&lines)
        ));
    }
    let unix_new = new.replace("\r\n", "\n");
    let body = if trailing {
        unix_new.strip_suffix('\n').unwrap_or(&unix_new)
    } else {
        &unix_new
    };
    let replacements = found
        .into_iter()
        .map(|index| {
            let window = &spans[index..index + width];
            Replacement {
                start: window[0].start,
                end: window[width - 1].end,
                text: in_file_style(&reindent(body, &old_lines, content, window)),
            }
        })
        .collect();
    Ok(Some(Plan {
        replacements,
        normalized: true,
    }))
}

/// Indentation width in columns, counting a tab as [`TAB_WIDTH`].
fn indent_width(line: &str) -> usize {
    indentation(line)
        .chars()
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// Whether the file lines are indented like `old_lines` up to one uniform
/// shift, so a whitespace-tolerant match cannot land at another nesting
/// level (which matters for Python and YAML).
fn same_relative_indentation(old_lines: &[String], content: &str, window: &[LineSpan]) -> bool {
    let mut shifts = old_lines
        .iter()
        .zip(window)
        .filter(|(line, _)| !line.trim().is_empty())
        .map(|(line, span)| {
            indent_width(span_text(content, *span)) as isize - indent_width(line) as isize
        });
    let Some(first) = shifts.next() else {
        return true;
    };
    shifts.all(|shift| shift == first)
}

/// Shift `body` from the indentation the caller used in `old_lines` to the
/// indentation the matched file lines actually have.
fn reindent(body: &str, old_lines: &[String], content: &str, window: &[LineSpan]) -> String {
    let Some(first) = old_lines.iter().position(|line| !line.trim().is_empty()) else {
        return body.to_string();
    };
    let assumed = indentation(&old_lines[first]);
    let actual = indentation(span_text(content, window[first]));
    if assumed == actual {
        return body.to_string();
    }
    body.split('\n')
        .map(|line| match line.strip_prefix(assumed) {
            Some(rest) if !line.trim().is_empty() => format!("{actual}{rest}"),
            _ => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A trimmed line and its sorted character bigrams.
struct Grams {
    text: String,
    grams: Vec<(char, char)>,
}

impl Grams {
    fn new(line: &str) -> Self {
        let text = line.trim().to_string();
        let chars = text.chars().collect::<Vec<_>>();
        let mut grams = chars
            .windows(2)
            .map(|pair| (pair[0], pair[1]))
            .collect::<Vec<_>>();
        grams.sort_unstable();
        Self { text, grams }
    }

    /// Sørensen-Dice similarity of the two lines' bigram multisets.
    fn similarity(&self, other: &Self) -> f64 {
        if self.text == other.text {
            return 1.0;
        }
        if self.grams.is_empty() || other.grams.is_empty() {
            return 0.0;
        }
        let (mut left, mut right, mut common) = (0, 0, 0);
        while left < self.grams.len() && right < other.grams.len() {
            match self.grams[left].cmp(&other.grams[right]) {
                std::cmp::Ordering::Less => left += 1,
                std::cmp::Ordering::Greater => right += 1,
                std::cmp::Ordering::Equal => {
                    common += 1;
                    left += 1;
                    right += 1;
                }
            }
        }
        (2 * common) as f64 / (self.grams.len() + other.grams.len()) as f64
    }
}

/// The start and width of the file window most similar to `old_lines`.
fn closest_window(
    content: &str,
    spans: &[LineSpan],
    old_lines: &[String],
) -> Option<(usize, usize)> {
    if spans.is_empty() || old_lines.is_empty() {
        return None;
    }
    let width = old_lines.len().min(spans.len());
    let windows = spans.len() - width + 1;
    if windows.saturating_mul(width) > MAX_SIMILARITY_WORK {
        return None;
    }
    let wanted = old_lines
        .iter()
        .map(|line| Grams::new(line))
        .collect::<Vec<_>>();
    let lines = spans
        .iter()
        .map(|span| Grams::new(span_text(content, *span)))
        .collect::<Vec<_>>();
    let mut best = (0.0, 0);
    for start in 0..windows {
        let score = (0..width)
            .map(|offset| lines[start + offset].similarity(&wanted[offset]))
            .sum::<f64>()
            / old_lines.len() as f64;
        if score > best.0 {
            best = (score, start);
        }
    }
    (best.0 >= MIN_SIMILARITY).then_some((best.1, width))
}

fn not_found(content: &str, old: &str) -> String {
    let spans = line_spans(content);
    let (old_lines, _) = pattern_lines(old);
    match closest_window(content, &spans, &old_lines) {
        Some((start, width)) => {
            let region = (start..start + width)
                .map(|index| format!("{:>6}\t{}", index + 1, span_text(content, spans[index])))
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "'old' text was not found. The most similar region is lines {}-{}:\n{region}\nRetry with the exact current text of that region.",
                start + 1,
                start + width
            )
        }
        None => "'old' text was not found, and no similar region exists. Read the file again before retrying.".into(),
    }
}

fn apply(content: &str, plan: &Plan) -> String {
    let mut out = String::with_capacity(content.len());
    let mut copied = 0;
    for replacement in &plan.replacements {
        out.push_str(&content[copied..replacement.start]);
        out.push_str(&replacement.text);
        copied = replacement.end;
    }
    out.push_str(&content[copied..]);
    out
}

/// Lines of a whole-line block, without the final line ending.
fn block_lines(block: &str) -> Vec<&str> {
    if block.is_empty() {
        return Vec::new();
    }
    let body = block.strip_suffix('\n').unwrap_or(block);
    body.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// A unified-diff snippet of the first `show` replacements, plus the total
/// added and removed line counts.
fn render_diff(content: &str, plan: &Plan, show: usize) -> (String, usize, usize) {
    let spans = line_spans(content);
    let line_of = |byte: usize| {
        spans
            .partition_point(|span| span.start <= byte)
            .saturating_sub(1)
    };
    let block_end = |line: usize| {
        spans
            .get(line + 1)
            .map(|span| span.start)
            .unwrap_or(content.len())
    };
    let (mut diff, mut added, mut removed) = (String::new(), 0, 0);
    let mut delta: i64 = 0;
    for (index, replacement) in plan.replacements.iter().enumerate() {
        let first = line_of(replacement.start);
        let last = line_of(replacement.end.saturating_sub(1).max(replacement.start));
        let start = spans.get(first).map(|span| span.start).unwrap_or(0);
        let end = block_end(last);
        let old_block = &content[start..end];
        let new_block = format!(
            "{}{}{}",
            &content[start..replacement.start],
            replacement.text,
            &content[replacement.end..end]
        );
        let old_lines = block_lines(old_block);
        let new_lines = block_lines(&new_block);
        added += new_lines.len();
        removed += old_lines.len();
        if index < show {
            let before = first.saturating_sub(CONTEXT_LINES)..first;
            let after = (last + 1).min(spans.len())..(last + 1 + CONTEXT_LINES).min(spans.len());
            let context = before.len() + after.len();
            let _ = writeln!(
                diff,
                "@@ -{},{} +{},{} @@",
                before.start + 1,
                context + old_lines.len(),
                before.start as i64 + 1 + delta,
                context + new_lines.len()
            );
            for line in before.clone() {
                let _ = writeln!(diff, " {}", span_text(content, spans[line]));
            }
            for line in &old_lines {
                let _ = writeln!(diff, "-{line}");
            }
            for line in &new_lines {
                let _ = writeln!(diff, "+{line}");
            }
            for line in after {
                let _ = writeln!(diff, " {}", span_text(content, spans[line]));
            }
        }
        delta += new_lines.len() as i64 - old_lines.len() as i64;
    }
    (diff, added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(content: &str, old: &str, new: &str, replace_all: bool) -> String {
        apply(content, &plan_edit(content, old, new, replace_all).unwrap())
    }

    #[test]
    fn replace_all_replaces_every_occurrence_and_uniqueness_is_enforced_otherwise() {
        let content = "let a = 1;\nlet b = a;\nprint(a);\n";
        let error = plan_edit(content, "a", "x", false).unwrap_err();
        assert!(error.contains("not unique"), "{error}");
        assert!(error.contains("at lines 1, 2, 3"), "{error}");
        let plan = plan_edit(content, "a;", "z;", true).unwrap();
        assert_eq!(plan.replacements.len(), 1);
        assert_eq!(
            edit(content, "(a)", "(b)", false),
            "let a = 1;\nlet b = a;\nprint(b);\n"
        );
        let plan = plan_edit(content, " a", " q", true).unwrap();
        assert_eq!(plan.replacements.len(), 2);
        assert_eq!(apply(content, &plan), "let q = 1;\nlet b = q;\nprint(a);\n");
    }

    #[test]
    fn whitespace_normalized_match_keeps_file_indentation() {
        let content = "fn main() {\n        let x = 1;   \n        call(x);\n}\n";
        let plan = plan_edit(
            content,
            "let x = 1;\ncall(x);",
            "let x = 2;\ncall(x);",
            false,
        )
        .unwrap();
        assert!(plan.normalized);
        assert_eq!(
            apply(content, &plan),
            "fn main() {\n        let x = 2;\n        call(x);\n}\n"
        );
        let nested = plan_edit(
            content,
            "    let x = 1;\n    call(x);\n",
            "    if ok {\n        call(x);\n    }\n",
            false,
        )
        .unwrap();
        assert_eq!(
            apply(content, &nested),
            "fn main() {\n        if ok {\n            call(x);\n        }\n}\n"
        );
    }

    #[test]
    fn whitespace_tolerant_matches_keep_relative_indentation() {
        let content = "if ready:\n    run()\nstop()\n";
        // Same text, but `stop()` sits one level deeper than in the file.
        let error = plan_edit(content, "if ready:\n    run()\n    stop()", "x", false).unwrap_err();
        assert!(error.contains("not found"), "{error}");
        let error = plan_edit(content, "if ready:\nrun()", "x", false).unwrap_err();
        assert!(error.contains("not found"), "{error}");
        // A uniform shift, or tabs for four spaces, still matches.
        let shifted = plan_edit(
            content,
            "  if ready:\n      run()",
            "  if go:\n      run()",
            false,
        )
        .unwrap();
        assert_eq!(apply(content, &shifted), "if go:\n    run()\nstop()\n");
        let tabbed = plan_edit(content, "if ready:\n\trun()", "if go:\n\trun()", false).unwrap();
        assert!(tabbed.normalized);
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let content = "one\r\ntwo\r\nthree\r\n";
        assert_eq!(
            edit(content, "one\ntwo\n", "uno\ndos\n", false),
            "uno\r\ndos\r\nthree\r\n"
        );
        assert_eq!(
            edit(content, "two", "2\n2", false),
            "one\r\n2\r\n2\r\nthree\r\n"
        );
    }

    #[test]
    fn missing_text_reports_the_closest_region_with_line_numbers() {
        let content = "alpha\nfn compute(total: u32) -> u32 {\n    total * 2\n}\nomega\n";
        let error = plan_edit(
            content,
            "fn compute(totals: u32) -> u32 {\n    totals * 2\n}",
            "x",
            false,
        )
        .unwrap_err();
        assert!(error.contains("lines 2-4"), "{error}");
        assert!(error.contains("     3\t    total * 2"), "{error}");
        let unrelated = plan_edit(content, "zzzz qqqq", "x", false).unwrap_err();
        assert!(unrelated.contains("no similar region"), "{unrelated}");
    }

    #[test]
    fn diff_snippet_shows_context_and_counts() {
        let content = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let plan = plan_edit(content, "5\n", "five\nFIVE\n", false).unwrap();
        let (diff, added, removed) = render_diff(content, &plan, MAX_HUNKS);
        assert_eq!((added, removed), (2, 1));
        assert_eq!(
            diff,
            "@@ -2,7 +2,8 @@\n 2\n 3\n 4\n-5\n+five\n+FIVE\n 6\n 7\n 8\n"
        );
        let partial = plan_edit(content, "3", "three", false).unwrap();
        let (diff, added, removed) = render_diff(content, &partial, MAX_HUNKS);
        assert_eq!((added, removed), (1, 1));
        assert!(diff.contains("-3\n+three\n"), "{diff}");
    }
}
