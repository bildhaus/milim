//! Model-facing tool plumbing: argument parsing and light schema validation
//! before a call runs, and the text rendering, head+tail truncation, and
//! on-disk spill of results after it returns.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

pub(crate) const TOOL_REPLAY_MAX_LINES: usize = 2_000;
pub(crate) const TOOL_REPLAY_MAX_BYTES: usize = 50 * 1024;
/// Total model-visible bytes one step's tool results may add, so a step
/// with many parallel calls cannot flood the context.
pub(crate) const STEP_REPLAY_MAX_BYTES: usize = 100 * 1024;
/// Smallest share a result keeps when the step budget cuts it.
const STEP_REPLAY_MIN_BYTES: usize = 2 * 1024;
/// Allowance for the omission marker and spill note a cut result gains.
const TRUNCATION_NOTE_BYTES: usize = 512;

/// Parse streamed tool-call arguments. Empty or whitespace-only arguments are
/// an empty object; anything else must be valid JSON.
pub(crate) fn parse_tool_arguments(arguments: &str) -> Result<Value, serde_json::Error> {
    if arguments.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(arguments)
}

/// Model-visible explanation for arguments that are not valid JSON.
pub(crate) fn invalid_json_message(error: &serde_json::Error, truncated: bool) -> String {
    if truncated {
        format!(
            "Tool call arguments were not valid JSON ({error}). Your previous response was cut off \
             at the output token limit, so the arguments were incomplete. Retry with smaller \
             content, for example split a large write into several smaller edits."
        )
    } else {
        format!(
            "Tool call arguments were not valid JSON ({error}). Send the arguments as a single \
             JSON object that matches the tool's input schema."
        )
    }
}

/// Check the arguments against the parts of a JSON Schema that catch most
/// model mistakes: an object at the top level, required properties present,
/// and top-level property types. Anything the schema does not constrain passes.
pub(crate) fn validate_tool_arguments(schema: &Value, args: &Value) -> Result<(), String> {
    let wants_object = schema.get("type").and_then(Value::as_str) == Some("object")
        || schema.get("properties").is_some();
    if !wants_object {
        return Ok(());
    }
    let Some(object) = args.as_object() else {
        return Err(format!(
            "Tool arguments must be a JSON object, got {}.",
            json_type_name(args)
        ));
    };
    let mut missing = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|name| !object.contains_key(*name))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(format!(
            "Missing required argument{}: {}.",
            if missing.len() == 1 { "" } else { "s" },
            missing.join(", ")
        ));
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (name, value) in object {
        let Some(expected) = properties
            .get(name)
            .and_then(|property| property.get("type"))
        else {
            continue;
        };
        let allowed = match expected {
            Value::String(kind) => vec![kind.as_str()],
            Value::Array(kinds) => kinds.iter().filter_map(Value::as_str).collect(),
            _ => continue,
        };
        if allowed.is_empty() || allowed.iter().any(|kind| json_type_matches(kind, value)) {
            continue;
        }
        return Err(format!(
            "Argument `{name}` must be {}, got {}.",
            allowed.join(" or "),
            json_type_name(value)
        ));
    }
    Ok(())
}

fn json_type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value.as_f64().is_some_and(|number| number.fract() == 0.0)
        }
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        // Unknown type keywords are not ours to enforce.
        _ => true,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Render a JSON tool result as model text.
///
/// A top-level string is sent raw. An object whose top-level string fields
/// contain newlines is split: the remaining fields as compact JSON on the
/// first line, then each multi-line field as a `--- name ---` section holding
/// the raw text, in key order. Everything else is compact JSON.
pub(crate) fn render_tool_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Object(object) => {
            let (multiline, rest): (Vec<_>, Vec<_>) = object
                .iter()
                .partition(|(_, value)| value.as_str().is_some_and(|text| text.contains('\n')));
            if multiline.is_empty() {
                return value.to_string();
            }
            let rest: Map<String, Value> = rest
                .into_iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let mut sections = Vec::new();
            if !rest.is_empty() {
                sections.push(Value::Object(rest).to_string());
            }
            for (key, value) in multiline {
                let text = value.as_str().unwrap_or_default();
                sections.push(format!("--- {key} ---\n{}", text.trim_end_matches('\n')));
            }
            sections.join("\n\n")
        }
        _ => value.to_string(),
    }
}

/// Head+tail truncation result.
pub(crate) struct Truncated {
    pub text: String,
    pub omitted_lines: usize,
    pub omitted_bytes: usize,
}

/// Keep both ends of an oversized text: up to half the line and byte budget
/// from the head and half from the tail, joined by an omission marker. Cuts
/// fall on line boundaries unless a single line exceeds its half budget.
pub(crate) fn truncate_head_tail(
    text: &str,
    max_lines: usize,
    max_bytes: usize,
) -> Option<Truncated> {
    let total_lines = text.split('\n').count();
    if total_lines <= max_lines && text.len() <= max_bytes {
        return None;
    }
    let half_lines = (max_lines / 2).max(1);
    let half_bytes = (max_bytes / 2).max(1);

    let mut head_end = 0;
    let mut kept = 0;
    while kept < half_lines {
        let Some(offset) = text[head_end..].find('\n') else {
            break;
        };
        let next = head_end + offset + 1;
        if next > half_bytes {
            break;
        }
        head_end = next;
        kept += 1;
    }
    if kept == 0 {
        head_end = floor_char_boundary(text, half_bytes);
    }

    let mut tail_start = text.len();
    let mut kept = 0;
    while kept < half_lines && tail_start > head_end {
        // The line ending at `tail_start` starts after the previous newline.
        let search_end = if tail_start == text.len() {
            tail_start
        } else {
            tail_start - 1
        };
        let start = text[..search_end].rfind('\n').map_or(0, |index| index + 1);
        if text.len() - start > half_bytes || start < head_end {
            break;
        }
        tail_start = start;
        kept += 1;
    }
    if kept == 0 {
        tail_start = ceil_char_boundary(text, text.len().saturating_sub(half_bytes));
    }
    let tail_start = tail_start.max(head_end);

    let omitted = &text[head_end..tail_start];
    let omitted_lines = omitted.matches('\n').count();
    let omitted_bytes = omitted.len();
    let head = text[..head_end]
        .strip_suffix('\n')
        .unwrap_or(&text[..head_end]);
    let tail = &text[tail_start..];
    Some(Truncated {
        text: format!(
            "{head}\n[… {omitted_lines} lines / {omitted_bytes} bytes omitted …]\n{tail}"
        ),
        omitted_lines,
        omitted_bytes,
    })
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// Build the model-visible content for one step's tool results, in order:
/// each tool's plain-text projection verbatim when it has one, otherwise
/// its rendered JSON, cut to the per-result replay budget and to the step's
/// total [`STEP_REPLAY_MAX_BYTES`]. When an output is cut and a tool output
/// root is registered, the full text is saved there and the model is told
/// where.
pub(crate) fn model_tool_contents<'a>(
    results: impl IntoIterator<Item = (Option<&'a str>, &'a Value, Option<&'a str>)>,
    scope: Option<&str>,
) -> Vec<String> {
    let texts = results
        .into_iter()
        .map(|(model_text, visible, call_id)| {
            let text = model_text
                .map(str::to_string)
                .unwrap_or_else(|| render_tool_value(visible));
            (text, call_id)
        })
        .collect::<Vec<_>>();
    let caps = step_byte_caps(
        &texts.iter().map(|(text, _)| text.len()).collect::<Vec<_>>(),
        STEP_REPLAY_MAX_BYTES,
    );
    let root = milim_tools::tool_output_root();
    texts
        .into_iter()
        .zip(caps)
        .map(|((text, call_id), max_bytes)| {
            replay_text(text, root, scope, call_id, TOOL_REPLAY_MAX_LINES, max_bytes)
        })
        .collect()
}

/// Per-result byte caps that fit one step's results into `budget`. Results
/// no larger than an equal share of what the smaller ones leave keep the
/// normal per-result cap; every larger one is cut to that share, so the
/// largest results give up the most.
fn step_byte_caps(lengths: &[usize], budget: usize) -> Vec<usize> {
    let sizes = lengths
        .iter()
        .map(|length| (*length).min(TOOL_REPLAY_MAX_BYTES))
        .collect::<Vec<_>>();
    if sizes.iter().sum::<usize>() <= budget {
        return vec![TOOL_REPLAY_MAX_BYTES; sizes.len()];
    }
    let mut ascending = sizes.clone();
    ascending.sort_unstable();
    let mut remaining = budget;
    let mut share = 0;
    for (position, size) in ascending.iter().enumerate() {
        share = remaining / (ascending.len() - position);
        if *size > share {
            break;
        }
        remaining -= size;
    }
    // Leave room for the omission marker and the spill note of a cut result.
    let cap = share
        .saturating_sub(TRUNCATION_NOTE_BYTES)
        .max(STEP_REPLAY_MIN_BYTES);
    sizes
        .iter()
        .map(|size| {
            if *size > share {
                cap
            } else {
                TOOL_REPLAY_MAX_BYTES
            }
        })
        .collect()
}

pub(crate) fn replay_text(
    text: String,
    root: Option<&Path>,
    scope: Option<&str>,
    call_id: Option<&str>,
    max_lines: usize,
    max_bytes: usize,
) -> String {
    let Some(truncated) = truncate_head_tail(&text, max_lines, max_bytes) else {
        return text;
    };
    let saved = root.and_then(|root| spill_tool_output(root, scope, call_id, &text).ok());
    match saved {
        Some(path) => format!(
            "{}\n\n[Full output ({} lines, {} bytes) saved to {}. Page through it with read_file \
             using offset/limit, or search it with grep.]",
            truncated.text,
            text.split('\n').count(),
            text.len(),
            path.display()
        ),
        None => format!(
            "{}\n\n[Output truncated for context: {} lines / {} bytes omitted from the middle.]",
            truncated.text, truncated.omitted_lines, truncated.omitted_bytes
        ),
    }
}

/// Save full tool output as `<root>/<scope or "run">/<call_id or uuid>.txt`.
/// Without a run scope the file name is always a fresh uuid so unrelated runs
/// sharing the fallback directory cannot overwrite each other's output.
fn spill_tool_output(
    root: &Path,
    scope: Option<&str>,
    call_id: Option<&str>,
    text: &str,
) -> std::io::Result<PathBuf> {
    let scope = scope
        .map(sanitize_component)
        .filter(|value| !value.is_empty());
    let file = match (&scope, call_id.map(sanitize_component)) {
        (Some(_), Some(call_id)) if !call_id.is_empty() => call_id,
        _ => uuid::Uuid::new_v4().to_string(),
    };
    let dir = root.join(scope.as_deref().unwrap_or("run"));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{file}.txt"));
    std::fs::write(&path, text)?;
    Ok(std::path::absolute(&path).unwrap_or(path))
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_arguments_are_an_empty_object_and_bad_json_explains_itself() {
        assert_eq!(parse_tool_arguments("  \n").unwrap(), json!({}));
        assert_eq!(parse_tool_arguments(r#"{"a":1}"#).unwrap(), json!({"a": 1}));
        let error =
            parse_tool_arguments(r#"{"path": "a.rs", "content": "unterminated"#).unwrap_err();
        let plain = invalid_json_message(&error, false);
        assert!(plain.contains("not valid JSON"));
        assert!(
            plain.contains("EOF"),
            "the parse error is included: {plain}"
        );
        let cut = invalid_json_message(&error, true);
        assert!(cut.contains("cut off at the output token limit"));
        assert!(cut.contains("several smaller edits"));
    }

    #[test]
    fn schema_validation_checks_required_and_top_level_types() {
        let schema = json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "offset": {"type": "integer"},
                "limit": {"type": ["integer", "null"]},
                "anything": {}
            },
            "required": ["path"]
        });
        assert!(validate_tool_arguments(&schema, &json!({"path": "a", "offset": 3.0})).is_ok());
        assert!(validate_tool_arguments(&schema, &json!({"path": "a", "limit": null})).is_ok());
        assert!(validate_tool_arguments(&schema, &json!({"path": "a", "anything": [1]})).is_ok());
        assert_eq!(
            validate_tool_arguments(&schema, &json!({"offset": 1})).unwrap_err(),
            "Missing required argument: path."
        );
        assert_eq!(
            validate_tool_arguments(&schema, &json!({"path": 7})).unwrap_err(),
            "Argument `path` must be string, got number."
        );
        assert_eq!(
            validate_tool_arguments(&schema, &json!({"path": "a", "offset": 1.5})).unwrap_err(),
            "Argument `offset` must be integer, got number."
        );
        assert_eq!(
            validate_tool_arguments(&schema, &json!(["a"])).unwrap_err(),
            "Tool arguments must be a JSON object, got array."
        );
        assert!(validate_tool_arguments(&json!({}), &json!(null)).is_ok());
    }

    #[test]
    fn renders_strings_raw_and_lifts_multiline_fields_out_of_json() {
        assert_eq!(
            render_tool_value(&json!("line 1\nline 2")),
            "line 1\nline 2"
        );
        assert_eq!(
            render_tool_value(&json!({"ok": true, "n": 2})),
            r#"{"n":2,"ok":true}"#
        );
        assert_eq!(
            render_tool_value(&json!({
                "exit_code": 1,
                "stdout": "a\nb\n",
                "stderr": "boom\nfailed",
                "path": "x"
            })),
            "{\"exit_code\":1,\"path\":\"x\"}\n\n--- stderr ---\nboom\nfailed\n\n--- stdout ---\na\nb"
        );
        assert_eq!(
            render_tool_value(&json!({"content": "only\ntext"})),
            "--- content ---\nonly\ntext"
        );
        assert_eq!(render_tool_value(&json!([1, "a\nb"])), r#"[1,"a\nb"]"#);
    }

    #[test]
    fn head_tail_truncation_keeps_both_ends_by_lines() {
        let text = (0..100)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let cut = truncate_head_tail(&text, 10, 1_000_000).unwrap();
        assert!(cut.text.starts_with("0\n1\n2\n3\n4\n[… "));
        assert!(cut.text.ends_with("\n95\n96\n97\n98\n99"));
        assert!(cut.text.contains("[… 90 lines / "));
        assert_eq!(cut.omitted_lines, 90);
        assert!(truncate_head_tail("short", 10, 100).is_none());
    }

    #[test]
    fn head_tail_truncation_cuts_a_single_huge_line_on_char_boundaries() {
        let text = format!("{}{}", "é".repeat(100), "z".repeat(100));
        let cut = truncate_head_tail(&text, 10, 100).unwrap();
        assert!(cut.text.starts_with(&"é".repeat(25)));
        assert!(cut.text.ends_with(&"z".repeat(50)));
        assert_eq!(cut.omitted_bytes, text.len() - 100);
        assert!(cut.text.contains(&format!(
            "[… 0 lines / {} bytes omitted …]",
            text.len() - 100
        )));
    }

    #[test]
    fn step_budget_cuts_the_largest_results_first() {
        let kib = 1024;
        // Under budget: every result keeps the normal per-result cap.
        assert_eq!(
            step_byte_caps(&[10 * kib, 20 * kib], 100 * kib),
            vec![TOOL_REPLAY_MAX_BYTES; 2]
        );
        // Four full-size results and one small one: the small one is
        // untouched and the rest share what is left equally.
        let caps = step_byte_caps(
            &[80 * kib, 4 * kib, 60 * kib, 50 * kib, 45 * kib],
            100 * kib,
        );
        assert_eq!(caps[1], TOOL_REPLAY_MAX_BYTES);
        let share = (100 * kib - 4 * kib) / 4;
        for index in [0, 2, 3, 4] {
            assert_eq!(caps[index], share - TRUNCATION_NOTE_BYTES);
        }
        // Many results never go below the minimum share.
        assert!(step_byte_caps(&vec![50 * kib; 100], 100 * kib)
            .iter()
            .all(|cap| *cap == STEP_REPLAY_MIN_BYTES));
    }

    #[test]
    fn step_contents_fit_the_step_budget() {
        let big = |fill: char| {
            (0..1_000)
                .map(|_| fill.to_string().repeat(45))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let texts = [big('a'), big('b'), big('c'), big('d')];
        let visible = serde_json::Value::Null;
        let small = "small result".to_string();
        let contents = model_tool_contents(
            texts
                .iter()
                .map(|text| (Some(text.as_str()), &visible, None))
                .chain(std::iter::once((Some(small.as_str()), &visible, None))),
            None,
        );
        assert_eq!(contents.len(), 5);
        assert_eq!(contents[4], small);
        let total = contents.iter().map(String::len).sum::<usize>();
        assert!(total <= STEP_REPLAY_MAX_BYTES, "{total}");
        for (content, fill) in contents[..4].iter().zip(['a', 'b', 'c', 'd']) {
            assert!(content.starts_with(fill));
            assert!(content.contains("omitted"), "every large result is cut");
        }
    }

    #[test]
    fn oversized_output_spills_to_the_tool_output_root() {
        let root = std::env::temp_dir().join(format!("milim-spill-{}", uuid::Uuid::new_v4()));
        let text = (0..50)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let replay = replay_text(
            text.clone(),
            Some(&root),
            Some("run/1"),
            Some("call:7"),
            10,
            10_000,
        );
        let saved = root.join("run_1").join("call_7.txt");
        assert_eq!(std::fs::read_to_string(&saved).unwrap(), text);
        assert!(replay.starts_with("line 0\n"));
        assert!(replay.contains("line 49"));
        assert!(replay.contains(&saved.display().to_string()));
        assert!(replay.contains("read_file"));

        let anonymous = replay_text(text.clone(), Some(&root), None, Some("call_0"), 10, 10_000);
        assert!(anonymous.contains(&root.join("run").display().to_string()));
        assert!(!root.join("run").join("call_0.txt").exists());

        let unsaved = replay_text(text, None, None, None, 10, 10_000);
        assert!(unsaved.contains("Output truncated for context"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
