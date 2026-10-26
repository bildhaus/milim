//! Notes for a model that repeats itself within one run: the same call
//! failing again with the same arguments, a call that was already denied,
//! or a read-only call returning the same output with nothing changed in
//! between.
//!
//! A note is appended to the model-visible tool result, next to any hook
//! feedback, and depends only on the run's own call history, so the request
//! committed to the run ledger is still exactly what the model saw.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use serde_json::Value;

use milim_core::api::openai::ToolCall;
use milim_tools::{ToolEffect, ToolRegistry};

use crate::tool_output;

/// Identical failures, or identical read-only results, before a note.
pub(crate) const REPEAT_THRESHOLD: usize = 3;

/// A call's identity: tool name plus canonical arguments (object keys
/// sorted, whitespace dropped). Arguments that are not JSON keep their raw,
/// trimmed text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallKey {
    name: String,
    arguments: std::result::Result<String, String>,
}

impl CallKey {
    fn new(call: &ToolCall) -> (Self, Option<Value>) {
        let parsed = tool_output::parse_tool_arguments(&call.function.arguments).ok();
        let arguments = match &parsed {
            Some(args) => Ok(canonical_json(args)),
            None => Err(call.function.arguments.trim().to_string()),
        };
        let key = Self {
            name: call.function.name.clone(),
            arguments,
        };
        (key, parsed)
    }
}

/// Per-run history of answered tool calls.
#[derive(Debug, Default)]
pub(crate) struct RepeatTracker {
    /// Consecutive failures of each call since it last succeeded or a
    /// state-changing call succeeded.
    failures: HashMap<CallKey, usize>,
    /// Denial reasons; a denied call is never run or sent for approval again
    /// in this run.
    denials: HashMap<CallKey, String>,
    /// Read-only calls since the last state-changing call: a fingerprint of
    /// the latest output and how many times in a row it came back unchanged.
    reads: HashMap<CallKey, (u64, usize)>,
}

impl RepeatTracker {
    /// The reason an identical call was denied earlier in this run.
    pub(crate) fn prior_denial(&self, call: &ToolCall) -> Option<&str> {
        self.denials.get(&CallKey::new(call).0).map(String::as_str)
    }

    /// Record one answered call and return the note to append to its
    /// model-visible result, if any. Calls skipped by a run limit never ran
    /// and are ignored; an approval that timed out is not remembered as a
    /// denial, since nobody decided.
    pub(crate) fn observe(
        &mut self,
        tools: &ToolRegistry,
        call: &ToolCall,
        visible: &Value,
    ) -> Option<String> {
        if visible.get("skipped").is_some() {
            return None;
        }
        let (key, args) = CallKey::new(call);
        if visible.get("denied").and_then(Value::as_bool) == Some(true) {
            let reason = visible.get("error").and_then(Value::as_str).unwrap_or("");
            if reason == crate::APPROVAL_TIMEOUT_MESSAGE {
                return None;
            }
            let repeated = self.denials.contains_key(&key);
            self.denials
                .entry(key)
                .or_insert_with(|| reason.to_string());
            return repeated.then(|| DENIED_AGAIN_NOTE.to_string());
        }
        if visible.get("error").is_some() {
            let count = self.failures.entry(key).or_insert(0);
            *count += 1;
            return (*count >= REPEAT_THRESHOLD).then(|| failed_again_note(*count));
        }
        self.failures.remove(&key);
        let read_only = args.and_then(|args| tools.effect_for_call(&call.function.name, &args))
            == Some(ToolEffect::ReadOnly);
        if !read_only {
            // The call may have changed what other calls see.
            self.failures.clear();
            self.reads.clear();
            return None;
        }
        let output = fingerprint(visible);
        let seen = self.reads.entry(key).or_insert((output, 0));
        if seen.0 != output {
            *seen = (output, 0);
        }
        seen.1 += 1;
        (seen.1 >= REPEAT_THRESHOLD).then(|| unchanged_read_note(seen.1))
    }
}

const DENIED_AGAIN_NOTE: &str = "This exact call was already denied earlier in this run, so it was not run or sent for approval again. Do not repeat it: change approach, or tell the user what you need and why.";

fn failed_again_note(count: usize) -> String {
    format!(
        "This exact call has now failed {count} times with the same arguments. Stop retrying it unchanged: read the error, inspect the relevant state, and change approach."
    )
}

fn unchanged_read_note(count: usize) -> String {
    format!(
        "This exact read-only call has now returned the same result {count} times with nothing changed in between. Use the output you already have instead of running it again."
    )
}

/// The model-visible tool content with a loop note appended.
pub(crate) fn with_note(mut content: String, note: &str) -> String {
    content.push_str("\n\n[milim] ");
    content.push_str(note);
    content
}

fn fingerprint(value: &Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical_json(value).hash(&mut hasher);
    hasher.finish()
}

/// JSON with object keys sorted at every level, independent of how the
/// map preserves insertion order.
fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(value, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::FunctionCall;
    use serde_json::json;

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            id: Some("id".into()),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    #[test]
    fn keys_ignore_key_order_and_whitespace() {
        let a = CallKey::new(&call(
            "grep",
            r#"{"pattern": "x", "path": {"b": 1, "a": 2}}"#,
        ))
        .0;
        let b = CallKey::new(&call("grep", r#"{"path":{"a":2,"b":1},"pattern":"x"}"#)).0;
        assert_eq!(a, b);
        assert_ne!(a, CallKey::new(&call("grep", r#"{"pattern": "y"}"#)).0);
        assert_ne!(
            a,
            CallKey::new(&call(
                "read",
                r#"{"pattern": "x", "path": {"b": 1, "a": 2}}"#
            ))
            .0
        );
        // Empty arguments count as `{}`; unparseable ones keep their text.
        assert_eq!(
            CallKey::new(&call("t", "")).0,
            CallKey::new(&call("t", " {} ")).0
        );
        assert_eq!(
            CallKey::new(&call("t", "{bad")).0.arguments,
            Err("{bad".to_string())
        );
    }

    #[test]
    fn notes_follow_the_third_identical_failure_until_it_succeeds() {
        let tools = ToolRegistry::with_builtins();
        let mut tracker = RepeatTracker::default();
        let failing = call("http_fetch", r#"{"url": "http://x"}"#);
        let error = json!({ "error": "connection refused" });
        assert_eq!(tracker.observe(&tools, &failing, &error), None);
        assert_eq!(tracker.observe(&tools, &failing, &error), None);
        assert_eq!(
            tracker.observe(&tools, &failing, &error),
            Some(failed_again_note(3))
        );
        assert_eq!(
            tracker.observe(&tools, &failing, &error),
            Some(failed_again_note(4))
        );
        // A different call failing does not count toward this one.
        let other = call("http_fetch", r#"{"url": "http://y"}"#);
        assert_eq!(tracker.observe(&tools, &other, &error), None);
        // Success resets it.
        assert_eq!(tracker.observe(&tools, &failing, &json!({"ok": 1})), None);
        assert_eq!(tracker.observe(&tools, &failing, &error), None);
    }

    #[test]
    fn a_state_change_resets_failures_and_unchanged_reads() {
        let tools = ToolRegistry::with_builtins();
        let mut tracker = RepeatTracker::default();
        let failing = call("http_fetch", r#"{"url": "http://x"}"#);
        let error = json!({ "error": "boom" });
        tracker.observe(&tools, &failing, &error);
        tracker.observe(&tools, &failing, &error);
        let read = call("current_time", "{}");
        tracker.observe(&tools, &read, &json!({"unix": 1}));
        tracker.observe(&tools, &read, &json!({"unix": 1}));
        // An unknown-effect call that succeeded may have changed state.
        tracker.observe(&tools, &call("mystery", "{}"), &json!({"ok": true}));
        assert_eq!(tracker.observe(&tools, &failing, &error), None);
        assert_eq!(tracker.observe(&tools, &read, &json!({"unix": 1})), None);
    }

    #[test]
    fn unchanged_read_only_results_get_a_lighter_note() {
        let tools = ToolRegistry::with_builtins();
        let mut tracker = RepeatTracker::default();
        let read = call("current_time", "{}");
        assert_eq!(tracker.observe(&tools, &read, &json!({"unix": 1})), None);
        assert_eq!(tracker.observe(&tools, &read, &json!({"unix": 1})), None);
        assert_eq!(
            tracker.observe(&tools, &read, &json!({"unix": 1})),
            Some(unchanged_read_note(3))
        );
        // Output that changed is not a wasted call.
        assert_eq!(tracker.observe(&tools, &read, &json!({"unix": 2})), None);
        assert_eq!(tracker.observe(&tools, &read, &json!({"unix": 2})), None);
    }

    #[test]
    fn denials_are_remembered_but_timeouts_and_skips_are_not() {
        let tools = ToolRegistry::with_builtins();
        let mut tracker = RepeatTracker::default();
        let write = call("write", r#"{"path": "a"}"#);
        let denied = json!({ "error": "Tool call denied by user", "denied": true });
        assert_eq!(tracker.observe(&tools, &write, &denied), None);
        assert_eq!(
            tracker.prior_denial(&call("write", r#"{ "path" : "a" }"#)),
            Some("Tool call denied by user")
        );
        assert_eq!(
            tracker.observe(&tools, &write, &denied),
            Some(DENIED_AGAIN_NOTE.to_string())
        );
        // Denials are not failures.
        assert_eq!(
            tracker.observe(&tools, &write, &denied),
            Some(DENIED_AGAIN_NOTE.to_string())
        );
        assert!(tracker.failures.is_empty());

        let other = call("write", r#"{"path": "b"}"#);
        let timed_out = json!({ "error": crate::APPROVAL_TIMEOUT_MESSAGE, "denied": true });
        assert_eq!(tracker.observe(&tools, &other, &timed_out), None);
        assert_eq!(tracker.prior_denial(&other), None);
        let skipped = json!({ "skipped": true, "error": "time limit" });
        for _ in 0..4 {
            assert_eq!(tracker.observe(&tools, &other, &skipped), None);
        }
    }

    #[test]
    fn notes_are_appended_like_hook_feedback() {
        assert_eq!(with_note("out".into(), "stop"), "out\n\n[milim] stop");
    }
}
