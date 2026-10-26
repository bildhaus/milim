//! `notebook_edit`: replace, insert, or delete one Jupyter notebook cell.
//!
//! Cells are addressed by the index or id `read_file` shows. The notebook is
//! rewritten as nbformat JSON (sorted keys, the file's own indentation), so
//! a cell edit does not reformat the rest of the file beyond what Jupyter
//! itself would write.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Map, Value};

use milim_core::{Error, Result};
use milim_tools::{atomic_write, Tool, ToolEffect};

use super::{
    arg_str, explain_io, host_tool_scoping, lock_file, missing_path, optional_arg_str, path_schema,
    safe_join, HostCtx,
};

const CELL_TYPES: &[&str] = &["code", "markdown", "raw"];

/// Change one cell of a Jupyter notebook.
pub struct NotebookEditTool {
    pub(super) ctx: HostCtx,
}

/// nbformat stores a cell's source as a list of lines, each but the last
/// keeping its `\n`.
fn source_lines(source: &str) -> Value {
    Value::Array(
        source
            .split_inclusive('\n')
            .map(|line| Value::String(line.to_string()))
            .collect(),
    )
}

/// A fresh cell id, unique within `cells`.
fn new_cell_id(cells: &[Value]) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    loop {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0)
            ^ NEXT.fetch_add(0x9e37_79b9, Ordering::Relaxed);
        let id = format!("{:08x}", seed as u32);
        if !cells.iter().any(|cell| cell["id"].as_str() == Some(&id)) {
            return id;
        }
    }
}

/// A new cell of `kind` holding `source`, keys in Jupyter's sorted order.
fn new_cell(kind: &str, source: &str, id: Option<String>) -> Value {
    let code = kind == "code";
    let mut cell = Map::new();
    cell.insert("cell_type".into(), json!(kind));
    if code {
        cell.insert("execution_count".into(), Value::Null);
    }
    if let Some(id) = id {
        cell.insert("id".into(), json!(id));
    }
    cell.insert("metadata".into(), json!({}));
    if code {
        cell.insert("outputs".into(), json!([]));
    }
    cell.insert("source".into(), source_lines(source));
    Value::Object(cell)
}

/// Give `cell` the type `kind`, adding or dropping the code-only fields.
fn set_cell_type(cell: &mut Value, kind: &str) {
    let Some(cell) = cell.as_object_mut() else {
        return;
    };
    cell.insert("cell_type".into(), json!(kind));
    if kind == "code" {
        cell.entry("execution_count").or_insert(Value::Null);
        cell.entry("outputs").or_insert_with(|| json!([]));
    } else {
        cell.remove("execution_count");
        cell.remove("outputs");
    }
}

/// The notebook serialized like Jupyter: `indent` spaces, keys as stored,
/// non-ASCII text unescaped, and a final newline when the file had one.
fn serialize(notebook: &Value, indent: usize, trailing_newline: bool) -> Result<String> {
    let spaces = " ".repeat(indent.max(1));
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(spaces.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    notebook.serialize(&mut serializer)?;
    let mut text = String::from_utf8(out).map_err(|error| Error::Other(error.to_string()))?;
    if trailing_newline {
        text.push('\n');
    }
    Ok(text)
}

/// The indentation width of the notebook's second line (Jupyter uses 1).
fn indent_of(text: &str) -> usize {
    text.lines()
        .nth(1)
        .map(|line| line.len() - line.trim_start_matches(' ').len())
        .filter(|width| *width > 0)
        .unwrap_or(1)
}

/// Resolve the requested cell to its index, by `cell_id` or `cell_index`.
fn find_cell(cells: &[Value], index: Option<u64>, id: Option<&str>) -> Result<usize> {
    match (index, id) {
        (_, Some(id)) => cells
            .iter()
            .position(|cell| cell["id"].as_str() == Some(id))
            .ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "no cell has id {id}; read the notebook with read_file to see the cell ids"
                ))
            }),
        (Some(index), None) => usize::try_from(index)
            .ok()
            .filter(|index| *index < cells.len())
            .ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "cell_index {index} is out of range; the notebook has {} cells (0-{})",
                    cells.len(),
                    cells.len().saturating_sub(1)
                ))
            }),
        (None, None) => Err(Error::InvalidRequest(
            "pass cell_index or cell_id to choose the cell".into(),
        )),
    }
}

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "notebook_edit"
    }
    fn description(&self) -> &str {
        "Change one cell of a Jupyter notebook (.ipynb): replace its source (and optionally its type), insert a new cell, or delete one. Address cells by the 0-based index or the id that read_file shows; read the notebook with read_file first (a read in an earlier turn of the chat counts while the file is unchanged).\n\
         Use it instead of edit_file for notebooks, so the JSON stays valid. Replacing a code cell's source clears its outputs and execution count.\n\
         Insert puts the new cell at cell_index (the cell count appends), or right after the cell named by cell_id.\n\
         Example: {\"path\": \"analysis.ipynb\", \"operation\": \"replace\", \"cell_id\": \"3f2a\", \"source\": \"df.describe()\"}"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":path_schema(&self.ctx),
            "operation":{"type":"string","enum":["replace","insert","delete"],"description":"Default replace."},
            "cell_index":{"type":"integer","minimum":0,"description":"0-based cell index."},
            "cell_id":{"type":"string","description":"Cell id, as read_file shows it. Takes precedence over cell_index."},
            "source":{"type":"string","description":"The cell's complete new source (replace and insert)."},
            "cell_type":{"type":"string","enum":CELL_TYPES,"description":"The cell's type. Insert defaults to code; replace keeps the current type unless this is set."}
        },"required":["path"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::Mutating
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        let path = result.get("path")?.as_str()?;
        let index = result.get("cell_index")?.as_u64()?;
        let cells = result.get("cells")?.as_u64()?;
        let verb = match result.get("operation")?.as_str()? {
            "insert" => "Inserted",
            "delete" => "Deleted",
            _ => "Replaced",
        };
        let mut cell = format!("cell {index}");
        if let Some(kind) = result["cell_type"].as_str() {
            cell.push_str(&format!(" ({kind}"));
            if let Some(id) = result["cell_id"].as_str() {
                cell.push_str(&format!(", id {id}"));
            }
            cell.push(')');
        }
        Some(format!(
            "{verb} {cell} in {path}; the notebook now has {cells} cell{}.",
            if cells == 1 { "" } else { "s" }
        ))
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let rel = arg_str(&args, "path")?;
        let path = safe_join(&self.ctx.ws, rel)?;
        let operation = optional_arg_str(&args, "operation")?.unwrap_or("replace");
        if !["replace", "insert", "delete"].contains(&operation) {
            return Err(Error::InvalidRequest(format!(
                "unknown operation: {operation} (use replace, insert, or delete)"
            )));
        }
        let kind = optional_arg_str(&args, "cell_type")?;
        if let Some(kind) = kind.filter(|kind| !CELL_TYPES.contains(kind)) {
            return Err(Error::InvalidRequest(format!(
                "unknown cell_type: {kind} (use code, markdown, or raw)"
            )));
        }
        let source = optional_arg_str(&args, "source")?;
        if operation != "delete" && source.is_none() {
            return Err(Error::InvalidRequest(format!(
                "{operation} needs the cell's complete `source`"
            )));
        }
        let index = match args.get("cell_index") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                Error::InvalidRequest("cell_index must be a non-negative integer".into())
            })?),
        };
        let id = optional_arg_str(&args, "cell_id")?.filter(|id| !id.is_empty());

        let _lock = lock_file(&path).await;
        if !path.is_file() {
            return Err(missing_path(&self.ctx, &path, rel, "notebook_edit"));
        }
        self.ctx.run.require_read(&path, rel, "notebook_edit")?;
        let text = std::fs::read_to_string(&path)
            .map_err(|error| explain_io(&self.ctx, error.into(), &path, rel, "notebook_edit"))?;
        let mut notebook: Value = serde_json::from_str(&text).map_err(|error| {
            Error::InvalidRequest(format!(
                "{rel} is not valid notebook JSON ({error}); fix it with edit_file"
            ))
        })?;
        let uses_ids = notebook["nbformat"].as_u64().unwrap_or(4) > 4
            || (notebook["nbformat"].as_u64() == Some(4)
                && notebook["nbformat_minor"].as_u64().unwrap_or(0) >= 5);
        let cells = notebook
            .get_mut("cells")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                Error::InvalidRequest(format!("{rel} has no `cells` list; is it a notebook?"))
            })?;
        let (changed, described) = match operation {
            "insert" => {
                let at = match (id, index) {
                    (Some(_), _) => find_cell(cells, None, id)? + 1,
                    (None, Some(index)) if index as usize <= cells.len() => index as usize,
                    (None, Some(index)) => {
                        return Err(Error::InvalidRequest(format!(
                            "cell_index {index} is past the end; the notebook has {} cells, so insert at {} or less",
                            cells.len(),
                            cells.len()
                        )))
                    }
                    (None, None) => cells.len(),
                };
                let new_id = (uses_ids || cells.iter().any(|cell| cell.get("id").is_some()))
                    .then(|| new_cell_id(cells));
                let cell = new_cell(kind.unwrap_or("code"), source.unwrap_or(""), new_id);
                cells.insert(at, cell);
                (at, cells[at].clone())
            }
            "delete" => {
                let at = find_cell(cells, index, id)?;
                (at, cells.remove(at))
            }
            _ => {
                let at = find_cell(cells, index, id)?;
                let cell = &mut cells[at];
                if let Some(kind) = kind {
                    set_cell_type(cell, kind);
                }
                cell["source"] = source_lines(source.unwrap_or(""));
                if cell["cell_type"] == "code" {
                    cell["outputs"] = json!([]);
                    cell["execution_count"] = Value::Null;
                }
                (at, cell.clone())
            }
        };
        let total = cells.len();
        let updated = serialize(&notebook, indent_of(&text), text.ends_with('\n'))?;
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            return Err(Error::InvalidRequest(
                "file changed while notebook_edit was running; read it again".into(),
            ));
        }
        atomic_write(&path, updated.as_bytes())
            .map_err(|error| explain_io(&self.ctx, error, &path, rel, "notebook_edit"))?;
        self.ctx.run.touch(&path);
        Ok(json!({
            "path": rel,
            "operation": operation,
            "cell_index": changed,
            "cell_type": described.get("cell_type"),
            "cell_id": described.get("id"),
            "cells": total,
        }))
    }
}
