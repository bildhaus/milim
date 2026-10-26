//! Readable text for Jupyter notebooks (`.ipynb`).
//!
//! `read_file` shows a notebook as its cells instead of raw JSON: each cell's
//! index, id, type, and source, followed by its text outputs cut to a few
//! lines. Images and other rich outputs are named, not inlined.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

/// Characters of one cell's combined output shown in the rendered view.
const MAX_OUTPUT_CHARS: usize = 2000;
/// Lines of one cell's combined output shown in the rendered view.
const MAX_OUTPUT_LINES: usize = 40;

/// Whether `path` names a Jupyter notebook.
pub fn is_notebook(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ipynb"))
}

/// A cell's `source` (a string, or a list of line strings) as one string.
pub fn cell_source(cell: &Value) -> String {
    multiline_text(cell.get("source"))
}

fn multiline_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// The notebook's language from its kernel or language metadata.
fn language(notebook: &Value) -> Option<&str> {
    notebook
        .pointer("/metadata/kernelspec/language")
        .or_else(|| notebook.pointer("/metadata/language_info/name"))
        .and_then(Value::as_str)
}

/// Drop ANSI color sequences, which tracebacks are full of.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The text of one output: stream text, a result's `text/plain`, an error's
/// name, value, and traceback, or the names of rich-only data.
fn output_text(output: &Value) -> String {
    match output.get("output_type").and_then(Value::as_str) {
        Some("stream") => multiline_text(output.get("text")),
        Some("error") => {
            let mut text = format!(
                "{}: {}",
                output["ename"].as_str().unwrap_or("Error"),
                output["evalue"].as_str().unwrap_or("")
            );
            for line in output["traceback"].as_array().into_iter().flatten() {
                if let Some(line) = line.as_str() {
                    text.push('\n');
                    text.push_str(&strip_ansi(line));
                }
            }
            text
        }
        Some("execute_result" | "display_data") => {
            let data = output.get("data");
            match data.and_then(|data| data.get("text/plain")) {
                Some(plain) => multiline_text(Some(plain)),
                None => {
                    let kinds = data
                        .and_then(Value::as_object)
                        .map(|data| data.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default();
                    format!(
                        "[{} output]",
                        if kinds.is_empty() { "rich" } else { &kinds }
                    )
                }
            }
        }
        _ => String::new(),
    }
}

/// `text` cut to the output budget, with a note of what was dropped.
fn cut_output(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let mut out = lines
        .iter()
        .take(MAX_OUTPUT_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let mut cut = lines.len() > MAX_OUTPUT_LINES;
    if let Some((index, _)) = out.char_indices().nth(MAX_OUTPUT_CHARS) {
        out.truncate(index);
        cut = true;
    }
    if cut {
        out.push_str("\n[... output truncated]");
    }
    out
}

/// The notebook as readable text, or `None` when `notebook` does not look
/// like one (then `read_file` shows the file as plain text).
pub fn render_notebook(notebook: &Value) -> Option<(String, usize)> {
    let cells = notebook.get("cells")?.as_array()?;
    let mut out = format!(
        "Jupyter notebook: {} cell{}",
        cells.len(),
        if cells.len() == 1 { "" } else { "s" }
    );
    if let Some(language) = language(notebook) {
        let _ = write!(out, ", {language}");
    }
    out.push_str(".\n");
    for (index, cell) in cells.iter().enumerate() {
        let kind = cell["cell_type"].as_str().unwrap_or("unknown");
        let _ = write!(out, "\n=== cell {index} ({kind}");
        if let Some(id) = cell.get("id").and_then(Value::as_str) {
            let _ = write!(out, ", id {id}");
        }
        if let Some(count) = cell.get("execution_count").and_then(Value::as_u64) {
            let _ = write!(out, ", execution_count {count}");
        }
        out.push_str(") ===\n");
        let source = cell_source(cell);
        out.push_str(source.trim_end_matches('\n'));
        out.push('\n');
        let outputs = cell["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(output_text)
            .filter(|text| !text.trim().is_empty())
            .map(|text| text.trim_end_matches('\n').to_string())
            .collect::<Vec<_>>();
        if !outputs.is_empty() {
            out.push_str("--- output ---\n");
            out.push_str(&cut_output(&outputs.join("\n")));
            out.push('\n');
        }
    }
    Some((out, cells.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notebooks_render_cells_ids_and_cut_outputs() {
        let notebook = json!({
            "metadata": {"kernelspec": {"language": "python"}},
            "nbformat": 4, "nbformat_minor": 5,
            "cells": [
                {"cell_type": "markdown", "id": "intro", "source": ["# Title\n", "Some text"]},
                {"cell_type": "code", "id": "load", "execution_count": 3,
                 "source": "import pandas as pd\nprint(1)",
                 "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["1\n"]},
                    {"output_type": "display_data", "data": {"image/png": "AAAA"}},
                    {"output_type": "execute_result", "data": {"text/plain": ["   a\n", "0  1"]}}
                 ]},
                {"cell_type": "code", "source": "boom()", "outputs": [
                    {"output_type": "error", "ename": "NameError", "evalue": "boom",
                     "traceback": ["\u{1b}[0;31mNameError\u{1b}[0m: boom"]}
                ]},
                {"cell_type": "code", "source": "", "outputs": [
                    {"output_type": "stream", "text": "x\n".repeat(100)}
                ]}
            ]
        });
        let (text, cells) = render_notebook(&notebook).unwrap();
        assert_eq!(cells, 4);
        assert!(
            text.starts_with("Jupyter notebook: 4 cells, python.\n"),
            "{text}"
        );
        assert!(text.contains("=== cell 0 (markdown, id intro) ===\n# Title\nSome text\n"));
        assert!(text.contains(
            "=== cell 1 (code, id load, execution_count 3) ===\nimport pandas as pd\nprint(1)\n--- output ---\n1\n[image/png output]\n   a\n0  1\n"
        ), "{text}");
        assert!(text.contains("NameError: boom\nNameError: boom"), "{text}");
        assert!(text.ends_with("[... output truncated]\n"), "{text}");
        assert!(render_notebook(&json!({"not": "a notebook"})).is_none());
        assert!(is_notebook(Path::new("a/B.IPYNB")));
        assert!(!is_notebook(Path::new("a/b.json")));
    }
}
