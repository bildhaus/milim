//! `code_navigation`: definitions, references, hover, and symbols from the
//! workspace's language servers (the same ones `diagnostics` uses).
//!
//! Positions are 1-based lines and character columns for the model; the
//! protocol's 0-based lines and UTF-16 offsets are converted using the text
//! of the line. Location lists are sorted, paged, and shown as
//! `path:line:col: snippet`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{Tool, ToolConcurrency, ToolEffect};

use super::lsp::uri_path;
use super::{
    arg_str, host_tool_scoping, missing_path, optional_arg_str, optional_u64, path_schema, root_of,
    safe_join, HostCtx, PathDisplay,
};

const OPERATIONS: &[&str] = &[
    "definition",
    "references",
    "hover",
    "document_symbols",
    "workspace_symbols",
];
const DEFAULT_MAX_RESULTS: u64 = 50;
const MAX_MAX_RESULTS: u64 = 200;
/// Characters of a result's source line shown after its position.
const MAX_SNIPPET_CHARS: usize = 160;
/// Characters of hover text returned.
const MAX_HOVER_CHARS: usize = 4000;
/// Locations one reply can describe at all.
const MAX_LOCATIONS: usize = 5000;

const SYMBOL_KINDS: &[&str] = &[
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum member",
    "struct",
    "event",
    "operator",
    "type parameter",
];

fn symbol_kind(value: &Value) -> &'static str {
    value
        .as_u64()
        .and_then(|kind| SYMBOL_KINDS.get((kind as usize).checked_sub(1)?))
        .copied()
        .unwrap_or("symbol")
}

/// The 0-based UTF-16 offset of 1-based character `column` in `line`.
fn utf16_offset(line: &str, column: u64) -> u64 {
    line.chars()
        .take(column.saturating_sub(1) as usize)
        .map(|c| c.len_utf16() as u64)
        .sum()
}

/// The 1-based character column of 0-based UTF-16 offset `offset` in `line`.
fn char_column(line: &str, offset: u64) -> u64 {
    let mut units = 0;
    let mut column = 1;
    for c in line.chars() {
        if units >= offset {
            break;
        }
        units += c.len_utf16() as u64;
        column += 1;
    }
    column + offset.saturating_sub(units)
}

/// The 1-based column where `symbol` first appears as a whole word in `line`.
fn word_column(line: &str, symbol: &str) -> Option<u64> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    line.match_indices(symbol).find_map(|(start, _)| {
        let before = line[..start].chars().next_back();
        let after = line[start + symbol.len()..].chars().next();
        (!before.is_some_and(is_word) && !after.is_some_and(is_word))
            .then(|| line[..start].chars().count() as u64 + 1)
    })
}

/// One position a server returned.
#[derive(Clone, Debug, PartialEq)]
struct Location {
    path: PathBuf,
    /// 0-based line and UTF-16 offset, as the server sent them.
    line: u64,
    character: u64,
    /// Symbol name, kind, nesting depth, and container, for symbol results.
    name: Option<String>,
    kind: Option<&'static str>,
    depth: usize,
    container: Option<String>,
}

impl Location {
    /// A `Location`, `LocationLink`, or a symbol's location.
    fn from_lsp(value: &Value) -> Option<Self> {
        let uri = value
            .get("targetUri")
            .or_else(|| value.get("uri"))?
            .as_str()?;
        let start = value
            .get("targetSelectionRange")
            .or_else(|| value.get("targetRange"))
            .or_else(|| value.get("range"))
            .and_then(|range| range.get("start"));
        Some(Self {
            path: uri_path(uri)?,
            line: start.and_then(|start| start["line"].as_u64()).unwrap_or(0),
            character: start
                .and_then(|start| start["character"].as_u64())
                .unwrap_or(0),
            name: None,
            kind: None,
            depth: 0,
            container: None,
        })
    }
}

/// Locations from a definition or references result: one location or link,
/// or a list of them.
fn locations(result: &Value) -> Vec<Location> {
    match result {
        Value::Array(items) => items.iter().filter_map(Location::from_lsp).collect(),
        Value::Object(_) => Location::from_lsp(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Symbols from a document or workspace symbol result: hierarchical
/// `DocumentSymbol`s (flattened depth first) or flat `SymbolInformation`.
fn symbols(result: &Value, document: &Path, out: &mut Vec<Location>, depth: usize) {
    for item in result.as_array().into_iter().flatten() {
        if out.len() >= MAX_LOCATIONS {
            return;
        }
        let mut location = match item.get("location") {
            Some(location) => match Location::from_lsp(location) {
                Some(location) => location,
                None => continue,
            },
            None => {
                let start = item
                    .get("selectionRange")
                    .or_else(|| item.get("range"))
                    .and_then(|range| range.get("start"));
                Location {
                    path: document.to_path_buf(),
                    line: start.and_then(|start| start["line"].as_u64()).unwrap_or(0),
                    character: start
                        .and_then(|start| start["character"].as_u64())
                        .unwrap_or(0),
                    name: None,
                    kind: None,
                    depth: 0,
                    container: None,
                }
            }
        };
        location.name = item["name"].as_str().map(str::to_string);
        location.kind = Some(symbol_kind(&item["kind"]));
        location.depth = depth;
        location.container = item["containerName"]
            .as_str()
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        out.push(location);
        if let Some(children) = item.get("children") {
            symbols(children, document, out, depth + 1);
        }
    }
}

/// Hover contents as plain text: `MarkupContent`, a `MarkedString`, or a
/// list of them.
fn hover_text(result: &Value) -> String {
    fn part(value: &Value) -> String {
        match value {
            Value::String(text) => text.clone(),
            Value::Object(object) => object
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            Value::Array(items) => items
                .iter()
                .map(part)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n"),
            _ => String::new(),
        }
    }
    let text = part(result.get("contents").unwrap_or(&Value::Null));
    let text = text.trim();
    match text.char_indices().nth(MAX_HOVER_CHARS) {
        Some((cut, _)) => format!("{}\n[... hover text cut]", &text[..cut]),
        None => text.to_string(),
    }
}

/// Lines of files read for snippets, loaded once per call.
#[derive(Default)]
struct Sources {
    files: HashMap<PathBuf, Option<Vec<String>>>,
}

impl Sources {
    fn line(&mut self, path: &Path, line: u64) -> Option<&str> {
        self.files
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| text.lines().map(str::to_string).collect())
            })
            .as_ref()?
            .get(line as usize)
            .map(String::as_str)
    }
}

fn snippet(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(MAX_SNIPPET_CHARS) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_string(),
    }
}

/// Look up definitions, references, hover text, and symbols.
pub struct CodeNavigationTool {
    pub(super) ctx: HostCtx,
}

impl CodeNavigationTool {
    /// The 0-based line and UTF-16 character the request is about: `line`
    /// and `column`, or where `symbol` appears on `line` (or first appears
    /// in the file).
    fn position(&self, args: &Value, text: &str, shown: &str) -> Result<(u64, u64)> {
        let lines = text.lines().collect::<Vec<_>>();
        let line = match args.get("line") {
            None | Some(Value::Null) => None,
            Some(_) => Some(optional_u64(args, "line", 1)?.max(1)),
        };
        let column = match args.get("column") {
            None | Some(Value::Null) => None,
            Some(_) => Some(optional_u64(args, "column", 1)?.max(1)),
        };
        let symbol = optional_arg_str(args, "symbol")?.filter(|symbol| !symbol.is_empty());
        let in_range = |line: u64| {
            lines.get(line as usize - 1).copied().ok_or_else(|| {
                Error::InvalidRequest(format!(
                    "line {line} is past the end of {shown} ({} lines)",
                    lines.len()
                ))
            })
        };
        let (line, column) = match (line, column, symbol) {
            (Some(line), Some(column), _) => (line, column),
            (Some(line), None, Some(symbol)) => {
                let column = word_column(in_range(line)?, symbol).ok_or_else(|| {
                    Error::InvalidRequest(format!("`{symbol}` does not appear on line {line} of {shown}"))
                })?;
                (line, column)
            }
            (None, _, Some(symbol)) => lines
                .iter()
                .enumerate()
                .find_map(|(index, text)| {
                    word_column(text, symbol).map(|column| (index as u64 + 1, column))
                })
                .ok_or_else(|| {
                    Error::InvalidRequest(format!(
                        "`{symbol}` does not appear in {shown}; pass line and column, or use workspace_symbols to find where it is defined"
                    ))
                })?,
            _ => {
                return Err(Error::InvalidRequest(
                    "pass line and column, or symbol (optionally with line), to say which identifier to look up".into(),
                ))
            }
        };
        let text = in_range(line)?;
        Ok((line - 1, utf16_offset(text, column)))
    }
}

#[async_trait]
impl Tool for CodeNavigationTool {
    fn name(&self) -> &str {
        "code_navigation"
    }
    fn description(&self) -> &str {
        "Navigate code with the workspace's language server (rust-analyzer, typescript-language-server, pyright/pylsp, gopls, or one configured in ~/.milim/settings.json). Operations: definition, references, hover (type and docs), document_symbols (outline of `path`), workspace_symbols (search symbol names; `symbol` is the query and `path` picks the language).\n\
         Use it to jump to where something is defined or find every real use of it, more precisely than grep. Use grep instead for plain text, comments, or languages without a server.\n\
         Point at an identifier with 1-based line and column, or name it with `symbol` (optionally with line). Results are `path:line:col: source line`, paged with offset.\n\
         Example: {\"operation\": \"references\", \"path\": \"src/server.ts\", \"line\": 42, \"symbol\": \"handleRequest\"}"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "operation":{"type":"string","enum":OPERATIONS},
            "path":path_schema(&self.ctx),
            "line":{"type":"integer","minimum":1,"description":"1-based line of the identifier."},
            "column":{"type":"integer","minimum":1,"description":"1-based column of the identifier on that line."},
            "symbol":{"type":"string","description":"The identifier to look up (its column is found on `line`, or its first use in the file), or the query for workspace_symbols."},
            "offset":{"type":"integer","minimum":0,"description":"Skip this many results to page through long lists. Default 0."},
            "max_results":{"type":"integer","minimum":1,"maximum":MAX_MAX_RESULTS,"description":"Results per call. Default 50."}
        },"required":["operation","path"],"additionalProperties":false})
    }
    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }
    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }
    fn model_text(&self, result: &Value) -> Option<String> {
        render_for_model(result)
    }
    host_tool_scoping!();
    async fn invoke(&self, args: Value) -> Result<Value> {
        let operation = arg_str(&args, "operation")?;
        if !OPERATIONS.contains(&operation) {
            return Err(Error::InvalidRequest(format!(
                "unknown operation: {operation} (use {})",
                OPERATIONS.join(", ")
            )));
        }
        let rel = arg_str(&args, "path")?;
        let root = root_of(&self.ctx.ws)?;
        let path = safe_join(&self.ctx.ws, rel)?;
        if !path.is_file() {
            return Err(missing_path(&self.ctx, &path, rel, "code_navigation"));
        }
        let offset = usize::try_from(optional_u64(&args, "offset", 0)?).unwrap_or(usize::MAX);
        let limit =
            optional_u64(&args, "max_results", DEFAULT_MAX_RESULTS)?.clamp(1, MAX_MAX_RESULTS);
        let (method, params, document) = match operation {
            "document_symbols" => ("textDocument/documentSymbol", json!({}), true),
            "workspace_symbols" => {
                let query = optional_arg_str(&args, "symbol")?
                    .or(optional_arg_str(&args, "query")?)
                    .ok_or_else(|| {
                        Error::InvalidRequest(
                            "workspace_symbols needs `symbol`, the name (or part of it) to search for".into(),
                        )
                    })?;
                ("workspace/symbol", json!({ "query": query }), false)
            }
            _ => {
                let text = std::fs::read_to_string(&path).map_err(|error| {
                    super::explain_io(&self.ctx, error.into(), &path, rel, "code_navigation")
                })?;
                let (line, character) = self.position(&args, &text, rel)?;
                let position = json!({ "line": line, "character": character });
                match operation {
                    "definition" => (
                        "textDocument/definition",
                        json!({ "position": position }),
                        true,
                    ),
                    "references" => (
                        "textDocument/references",
                        json!({ "position": position, "context": { "includeDeclaration": true } }),
                        true,
                    ),
                    _ => ("textDocument/hover", json!({ "position": position }), true),
                }
            }
        };
        let (server, answer) = self
            .ctx
            .lsp
            .navigate(&root, &path, method, params, document)
            .await
            .map_err(|reason| {
                Error::InvalidRequest(format!(
                    "{rel}: {reason}. Use grep to search for the identifier instead."
                ))
            })?;
        let display = PathDisplay::new(&root);
        if operation == "hover" {
            return Ok(json!({
                "operation": operation,
                "server": server,
                "hover": hover_text(&answer),
            }));
        }
        let mut found = Vec::new();
        if operation.ends_with("_symbols") {
            let document = std::fs::canonicalize(&path).unwrap_or(path.clone());
            symbols(&answer, &document, &mut found, 0);
        } else {
            found = locations(&answer);
            found.truncate(MAX_LOCATIONS);
            found.sort_by(|a, b| {
                (&a.path, a.line, a.character).cmp(&(&b.path, b.line, b.character))
            });
            found.dedup();
        }
        let total = found.len();
        let mut sources = Sources::default();
        let results = found
            .iter()
            .skip(offset)
            .take(limit as usize)
            .map(|location| {
                let line = sources
                    .line(&location.path, location.line)
                    .map(str::to_string);
                let column = line
                    .as_deref()
                    .map(|text| char_column(text, location.character))
                    .unwrap_or(location.character + 1);
                let mut entry = json!({
                    "path": display.show(&location.path),
                    "line": location.line + 1,
                    "column": column,
                    "text": line.as_deref().map(snippet).unwrap_or_default(),
                });
                if let Some(name) = &location.name {
                    entry["name"] = json!(name);
                    entry["kind"] = json!(location.kind);
                    entry["depth"] = json!(location.depth);
                    if let Some(container) = &location.container {
                        entry["container"] = json!(container);
                    }
                }
                entry
            })
            .collect::<Vec<_>>();
        let next = offset.saturating_add(results.len());
        Ok(json!({
            "operation": operation,
            "server": server,
            "results": results,
            "total": total,
            "offset": offset,
            "next_offset": (next < total).then_some(next),
        }))
    }
}

fn render_for_model(result: &Value) -> Option<String> {
    let operation = result.get("operation")?.as_str()?;
    if operation == "hover" {
        let text = result.get("hover")?.as_str()?;
        return Some(if text.is_empty() {
            "No hover information at that position.".into()
        } else {
            text.to_string()
        });
    }
    let results = result.get("results")?.as_array()?;
    let total = result.get("total")?.as_u64()?;
    if total == 0 {
        let what = match operation {
            "definition" => "No definition found",
            "references" => "No references found",
            _ => "No symbols found",
        };
        return Some(format!(
            "{what}. The language server may still be indexing the project; retry shortly, or use grep."
        ));
    }
    let mut out = String::new();
    for entry in results {
        let _ = write!(
            out,
            "{}:{}:{}: ",
            entry.get("path")?.as_str()?,
            entry["line"],
            entry["column"]
        );
        match entry.get("name").and_then(Value::as_str) {
            Some(name) => {
                let depth = entry["depth"].as_u64().unwrap_or(0) as usize;
                let _ = write!(
                    out,
                    "{}{} {name}",
                    "  ".repeat(depth),
                    entry["kind"].as_str().unwrap_or("symbol")
                );
                if let Some(container) = entry["container"].as_str() {
                    let _ = write!(out, " (in {container})");
                }
            }
            None => out.push_str(entry["text"].as_str().unwrap_or("")),
        }
        out.push('\n');
    }
    if let Some(next) = result["next_offset"].as_u64() {
        let _ = writeln!(
            out,
            "(Showing {} of {total} results. Continue with offset={next}.)",
            results.len()
        );
    }
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_convert_between_characters_and_utf16() {
        let line = "let \u{e9}\u{1f600} = x;";
        // `x` is the 10th character, after one 2-unit emoji.
        assert_eq!(utf16_offset(line, 10), 10);
        assert_eq!(char_column(line, 10), 10);
        assert_eq!(utf16_offset(line, 7), 7);
        assert_eq!(char_column(line, 7), 7);
        assert_eq!(utf16_offset(line, 6), 5);
        assert_eq!(char_column(line, 5), 6);
        assert_eq!(char_column("ab", 5), 6);
        assert_eq!(word_column("  let foo = foobar(foo);", "foo"), Some(7));
        assert_eq!(word_column("foobar", "foo"), None);
    }

    #[test]
    fn results_parse_locations_links_symbols_and_hover() {
        let link = json!([{"targetUri": "file:///tmp/a%20b.rs", "targetSelectionRange": {"start": {"line": 4, "character": 2}}}]);
        let parsed = locations(&link);
        assert_eq!(parsed[0].path, PathBuf::from("/tmp/a b.rs"));
        assert_eq!((parsed[0].line, parsed[0].character), (4, 2));
        let single =
            json!({"uri": "file:///tmp/x.rs", "range": {"start": {"line": 1, "character": 0}}});
        assert_eq!(locations(&single).len(), 1);
        assert!(locations(&Value::Null).is_empty());

        let mut found = Vec::new();
        symbols(
            &json!([{"name": "Server", "kind": 5, "selectionRange": {"start": {"line": 0, "character": 6}},
                     "children": [{"name": "run", "kind": 6, "range": {"start": {"line": 3, "character": 4}}}]}]),
            Path::new("/tmp/s.ts"),
            &mut found,
            0,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(
            (found[1].name.as_deref(), found[1].kind, found[1].depth),
            (Some("run"), Some("method"), 1)
        );

        assert_eq!(
            hover_text(&json!({"contents": {"kind": "markdown", "value": " `fn x()` "}})),
            "`fn x()`"
        );
        assert_eq!(
            hover_text(&json!({"contents": ["a", {"language": "rust", "value": "b"}]})),
            "a\n\nb"
        );
    }
}
