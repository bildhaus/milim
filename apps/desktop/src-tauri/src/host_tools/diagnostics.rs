//! `diagnostics`: errors and warnings from the workspace's language servers.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use milim_core::{Error, Result};
use milim_tools::{Tool, ToolConcurrency, ToolEffect};

use super::lsp::{render_line, PUSH_WAIT, READY_WAIT};
use super::{host_tool_scoping, optional_arg_str, root_of, safe_join, HostCtx, PathDisplay};

/// Files checked by one call without a path.
const MAX_FILES: usize = 20;
/// Diagnostic lines rendered for the model; the rest are counted.
const MAX_LINES: usize = 200;

/// Report language-server diagnostics for a file or this run's edits.
pub struct DiagnosticsTool {
    pub(super) ctx: HostCtx,
}

#[async_trait]
impl Tool for DiagnosticsTool {
    fn name(&self) -> &str {
        "diagnostics"
    }
    fn description(&self) -> &str {
        "Report compiler and linter errors and warnings from the workspace's language server (rust-analyzer, typescript-language-server, pyright/pylsp, gopls, or one configured in ~/.milim/settings.json). With a path, checks that file; without one, checks the files written or edited in this run."
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"File to check. Omit to check the files written or edited in this run."}
        }})
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
        let root = root_of(&self.ctx.ws)?;
        let display = PathDisplay::new(&root);
        let requested = optional_arg_str(&args, "path")?;
        let paths = match requested {
            Some(rel) => {
                let path = safe_join(&self.ctx.ws, rel)?;
                if !path.is_file() {
                    return Err(Error::InvalidRequest(format!("{rel} is not a file")));
                }
                vec![path]
            }
            None => self.ctx.run.touched(),
        };
        if paths.is_empty() {
            return Ok(json!({
                "files": [],
                "errors": 0,
                "warnings": 0,
                "message": "No files were written or edited in this run yet; pass a path to check a specific file.",
            }));
        }
        let skipped = paths.len().saturating_sub(MAX_FILES);
        let mut checks = tokio::task::JoinSet::new();
        for (index, path) in paths.into_iter().take(MAX_FILES).enumerate() {
            let lsp = self.ctx.lsp.clone();
            let root = root.clone();
            checks.spawn(async move {
                let report = lsp
                    .file_diagnostics(&root, &path, READY_WAIT, PUSH_WAIT)
                    .await;
                (index, path, report)
            });
        }
        let mut reports = Vec::new();
        while let Some(joined) = checks.join_next().await {
            reports.push(
                joined
                    .map_err(|error| Error::Other(format!("diagnostics task failed: {error}")))?,
            );
        }
        reports.sort_by_key(|(index, _, _)| *index);

        let (mut files, mut unavailable) = (Vec::new(), Vec::new());
        let (mut errors, mut warnings) = (0, 0);
        for (_, path, report) in reports {
            let shown = display.show(&path);
            match report {
                Ok(report) => {
                    let diagnostics = report
                        .diagnostics
                        .into_iter()
                        .filter(|diagnostic| matches!(diagnostic.severity, "error" | "warning"))
                        .collect::<Vec<_>>();
                    errors += diagnostics.iter().filter(|d| d.severity == "error").count();
                    warnings += diagnostics
                        .iter()
                        .filter(|d| d.severity == "warning")
                        .count();
                    files.push(json!({
                        "path": shown,
                        "server": report.server,
                        "partial": report.partial,
                        "diagnostics": diagnostics,
                    }));
                }
                Err(reason) if requested.is_some() => {
                    return Err(Error::InvalidRequest(format!("{shown}: {reason}")));
                }
                Err(reason) => unavailable.push(json!({ "path": shown, "reason": reason })),
            }
        }
        let mut result = json!({
            "files": files,
            "errors": errors,
            "warnings": warnings,
        });
        if !unavailable.is_empty() {
            result["unavailable"] = json!(unavailable);
        }
        if skipped > 0 {
            result["skipped_files"] = json!(skipped);
        }
        Ok(result)
    }
}

fn plural(count: u64, singular: &str) -> String {
    format!("{count} {singular}{}", if count == 1 { "" } else { "s" })
}

fn render_for_model(result: &Value) -> Option<String> {
    if let Some(message) = result.get("message").and_then(Value::as_str) {
        return Some(message.to_string());
    }
    let files = result.get("files")?.as_array()?;
    let errors = result.get("errors")?.as_u64()?;
    let warnings = result.get("warnings")?.as_u64()?;
    let mut out = String::new();
    if errors + warnings == 0 {
        let checked = files.len() as u64;
        if checked > 0 {
            let _ = writeln!(out, "No errors or warnings in {}.", plural(checked, "file"));
        }
    } else {
        let _ = writeln!(
            out,
            "{}, {}.",
            plural(errors, "error"),
            plural(warnings, "warning")
        );
    }
    let mut rendered = 0;
    let mut omitted = 0;
    for file in files {
        let path = file.get("path")?.as_str()?;
        for diagnostic in file.get("diagnostics")?.as_array()? {
            if rendered >= MAX_LINES {
                omitted += 1;
                continue;
            }
            rendered += 1;
            let _ = writeln!(
                out,
                "{}",
                render_line(
                    path,
                    diagnostic.get("line")?.as_u64()?,
                    diagnostic.get("column")?.as_u64()?,
                    diagnostic.get("severity")?.as_str()?,
                    diagnostic.get("message")?.as_str()?,
                )
            );
        }
    }
    if omitted > 0 {
        let _ = writeln!(out, "[{} more not shown]", plural(omitted, "diagnostic"));
    }
    for file in files.iter().filter(|file| file["partial"] == true) {
        let _ = writeln!(
            out,
            "{}: partial, {} had not finished analyzing; try again shortly.",
            file["path"].as_str()?,
            file["server"].as_str().unwrap_or("the language server")
        );
    }
    for entry in result
        .get("unavailable")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let _ = writeln!(
            out,
            "{}: {}",
            entry["path"].as_str()?,
            entry["reason"].as_str()?
        );
    }
    if let Some(skipped) = result.get("skipped_files").and_then(Value::as_u64) {
        let _ = writeln!(
            out,
            "{} not checked; pass a path.",
            plural(skipped, "more file")
        );
    }
    Some(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_lines_partials_and_unavailable_files() {
        let text = render_for_model(&json!({
            "files": [
                {"path": "src/lib.rs", "server": "rust-analyzer", "partial": false, "diagnostics": [
                    {"line": 2, "column": 5, "severity": "error", "message": "expected `;`"},
                    {"line": 1, "column": 1, "severity": "warning", "message": "unused import"}
                ]},
                {"path": "src/app.ts", "server": "typescript-language-server", "partial": true, "diagnostics": []}
            ],
            "errors": 1,
            "warnings": 1,
            "unavailable": [{"path": "notes.xyz", "reason": "no language server found for .xyz"}]
        }))
        .unwrap();
        assert_eq!(
            text,
            "1 error, 1 warning.\n\
             src/lib.rs:2:5 error expected `;`\n\
             src/lib.rs:1:1 warning unused import\n\
             src/app.ts: partial, typescript-language-server had not finished analyzing; try again shortly.\n\
             notes.xyz: no language server found for .xyz"
        );
        let clean = render_for_model(&json!({
            "files": [{"path": "a.rs", "server": "x", "partial": false, "diagnostics": []}],
            "errors": 0,
            "warnings": 0
        }))
        .unwrap();
        assert_eq!(clean, "No errors or warnings in 1 file.");
    }
}
