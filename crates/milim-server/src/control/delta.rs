//! Run output: streamed assistant text and reasoning, buffered and flushed as
//! `assistant_delta` timeline events, and the compact work log of the tools a
//! run used.

use std::time::{Duration, Instant};

use milim_core::Result;
use serde_json::{json, Value};

use super::RunManager;

const DELTA_FLUSH_BYTES: usize = 512;
pub(super) const DELTA_FLUSH_INTERVAL: Duration = Duration::from_millis(40);

/// Accumulates a run's assistant text and reasoning and buffers the part not
/// yet emitted as `assistant_delta` events.
///
/// The first delta is emitted as soon as it arrives; after that, pending
/// output is emitted once it reaches `DELTA_FLUSH_BYTES`. Runs that poll their
/// stream with a `DELTA_FLUSH_INTERVAL` timeout flush idle output themselves;
/// a [`DeltaBuffer::timed`] buffer instead also flushes once that interval has
/// passed since its last due flush.
///
/// Text the harness adds for the user (a notice) is shown like model text but
/// kept out of [`DeltaBuffer::model_content`], which is what later turns
/// replay to the model.
pub(super) struct DeltaBuffer<'a> {
    manager: &'a RunManager,
    thread_id: &'a str,
    run_id: &'a str,
    content: String,
    model_content: String,
    reasoning: String,
    pending_text: String,
    pending_reasoning: String,
    emitted_first_delta: bool,
    last_flush: Option<Instant>,
    step_boundary: bool,
}

impl<'a> DeltaBuffer<'a> {
    /// A buffer whose caller flushes idle output on its own timeout.
    pub(super) fn new(manager: &'a RunManager, thread_id: &'a str, run_id: &'a str) -> Self {
        Self {
            manager,
            thread_id,
            run_id,
            content: String::new(),
            model_content: String::new(),
            reasoning: String::new(),
            pending_text: String::new(),
            pending_reasoning: String::new(),
            emitted_first_delta: false,
            last_flush: None,
            step_boundary: false,
        }
    }

    /// A buffer that also flushes when `DELTA_FLUSH_INTERVAL` has passed since
    /// it was created or last flushed by [`DeltaBuffer::flush_if_due`].
    pub(super) fn timed(manager: &'a RunManager, thread_id: &'a str, run_id: &'a str) -> Self {
        Self {
            last_flush: Some(Instant::now()),
            ..Self::new(manager, thread_id, run_id)
        }
    }

    pub(super) fn push_text(&mut self, text: &str) {
        if !text.is_empty() && std::mem::take(&mut self.step_boundary) {
            let separator = paragraph_separator(&self.content);
            self.content.push_str(separator);
            self.pending_text.push_str(separator);
            self.model_content
                .push_str(paragraph_separator(&self.model_content));
        }
        self.content.push_str(text);
        self.pending_text.push_str(text);
        self.model_content.push_str(text);
    }

    /// Harness-authored text for the user: streamed and shown like model
    /// text, but never replayed to a model.
    pub(super) fn push_notice(&mut self, text: &str) {
        self.content.push_str(text);
        self.pending_text.push_str(text);
    }

    /// A model step ended (it called tools, or a hook continued the run).
    /// The next step's text starts a new paragraph instead of running on
    /// from the previous step's last sentence.
    pub(super) fn mark_step_boundary(&mut self) {
        self.step_boundary = true;
    }

    pub(super) fn push_reasoning(&mut self, text: &str) {
        self.reasoning.push_str(text);
        self.pending_reasoning.push_str(text);
    }

    pub(super) fn has_pending_text(&self) -> bool {
        !self.pending_text.is_empty()
    }

    pub(super) fn has_pending_reasoning(&self) -> bool {
        !self.pending_reasoning.is_empty()
    }

    /// Emits pending output if the first delta is still unsent, if pending
    /// output reached `DELTA_FLUSH_BYTES`, or, for a timed buffer, if the
    /// flush interval has passed.
    pub(super) fn flush_if_due(&mut self) -> Result<()> {
        if !self.emitted_first_delta
            || self.pending_text.len() + self.pending_reasoning.len() >= DELTA_FLUSH_BYTES
            || self
                .last_flush
                .is_some_and(|last_flush| last_flush.elapsed() >= DELTA_FLUSH_INTERVAL)
        {
            self.flush()?;
            self.emitted_first_delta = true;
            if self.last_flush.is_some() {
                self.last_flush = Some(Instant::now());
            }
        }
        Ok(())
    }

    /// Emits all pending output as one `assistant_delta` event.
    pub(super) fn flush(&mut self) -> Result<()> {
        if self.pending_text.is_empty() && self.pending_reasoning.is_empty() {
            return Ok(());
        }
        self.manager.persist_and_emit(
            self.thread_id,
            Some(self.run_id),
            "assistant_delta",
            json!({
                "text": std::mem::take(&mut self.pending_text),
                "reasoning": std::mem::take(&mut self.pending_reasoning),
            }),
        )?;
        Ok(())
    }

    /// Drops the tail a failed provider attempt contributed. The failed
    /// attempt's partial text is not part of the answer; the retried step
    /// streams it again.
    pub(super) fn truncate_for_retry(
        &mut self,
        discarded_content_bytes: usize,
        discarded_reasoning_bytes: usize,
    ) {
        self.content
            .truncate(self.content.len().saturating_sub(discarded_content_bytes));
        self.model_content.truncate(
            self.model_content
                .len()
                .saturating_sub(discarded_content_bytes),
        );
        self.reasoning.truncate(
            self.reasoning
                .len()
                .saturating_sub(discarded_reasoning_bytes),
        );
    }

    pub(super) fn content(&self) -> &str {
        &self.content
    }

    pub(super) fn reasoning(&self) -> &str {
        &self.reasoning
    }

    /// The accumulated assistant text without harness notices: what a later
    /// turn replays to the model.
    pub(super) fn model_content(&self) -> &str {
        &self.model_content
    }

    /// The accumulated assistant text and reasoning.
    pub(super) fn into_output(self) -> (String, String) {
        (self.content, self.reasoning)
    }
}

/// The separator that starts a new paragraph after `text`.
fn paragraph_separator(text: &str) -> &'static str {
    if text.is_empty() || text.ends_with("\n\n") {
        ""
    } else if text.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    }
}

/// Tool calls a stored work log keeps from the start and the end of a run;
/// the calls between them are only counted.
const WORK_LOG_HEAD_CALLS: usize = 16;
const WORK_LOG_TAIL_CALLS: usize = 48;
const WORK_LOG_MAX_FILES: usize = 40;
const WORK_LOG_ARGUMENT_CHARS: usize = 120;
const WORK_LOG_OUTCOME_CHARS: usize = 160;
/// Replay budgets for a rendered work log: the latest run keeps the detail
/// the next turn most likely needs, and earlier runs are trimmed harder.
pub(super) const WORK_LOG_LATEST_CHARS: usize = 3_500;
pub(super) const WORK_LOG_EARLIER_CHARS: usize = 1_000;
/// Tools that change the file named by their `path` argument.
const FILE_CHANGING_TOOLS: &[&str] = &["write_file", "edit_file", "patch_file"];
/// Arguments that identify what a call acted on, in preference order.
const WORK_LOG_KEY_ARGUMENTS: &[&str] = &[
    "path",
    "file_path",
    "command",
    "pattern",
    "query",
    "url",
    "process_id",
    "target_thread_id",
    "name",
];

/// A compact record of the tools one run called and how each call ended.
///
/// Built from the same Agent events the timeline records and stored on the
/// run's assistant message as `workLog`. Clients ignore it; later turns
/// replay it to the model with the assistant text (see [`render_work_log`]),
/// so the model remembers the work it did and not only what it said.
#[derive(Debug, Default)]
pub(super) struct WorkLog {
    calls: Vec<WorkLogCall>,
    files_changed: Vec<String>,
}

#[derive(Debug)]
struct WorkLogCall {
    call_id: Option<String>,
    tool: String,
    arguments: String,
    changes_path: Option<String>,
    outcome: Option<String>,
}

impl WorkLog {
    pub(super) fn record_call(&mut self, call_id: Option<&str>, tool: &str, arguments: &str) {
        let arguments = serde_json::from_str::<Value>(arguments).unwrap_or(Value::Null);
        self.calls.push(WorkLogCall {
            call_id: call_id.map(str::to_string),
            tool: tool.to_string(),
            arguments: key_arguments(&arguments),
            changes_path: FILE_CHANGING_TOOLS
                .contains(&tool)
                .then(|| arguments.get("path").and_then(Value::as_str))
                .flatten()
                .map(str::to_string),
            outcome: None,
        });
    }

    pub(super) fn record_result(&mut self, call_id: Option<&str>, tool: &str, result: &Value) {
        let (outcome, succeeded) = tool_outcome(result);
        let index = self.calls.iter().rposition(|call| {
            call.outcome.is_none()
                && match (call_id, call.call_id.as_deref()) {
                    (Some(id), Some(call_id)) => id == call_id,
                    _ => call.tool == tool,
                }
        });
        let call = match index {
            Some(index) => &mut self.calls[index],
            None => {
                self.calls.push(WorkLogCall {
                    call_id: call_id.map(str::to_string),
                    tool: tool.to_string(),
                    arguments: String::new(),
                    changes_path: None,
                    outcome: None,
                });
                self.calls.last_mut().expect("just pushed")
            }
        };
        call.outcome = Some(outcome);
        if let Some(path) = call.changes_path.as_ref().filter(|_| succeeded) {
            if !self.files_changed.contains(path) {
                self.files_changed.push(path.clone());
            }
        }
    }

    /// The stored form, or `None` when the run called no tools. Entries keep
    /// the first and last calls of a long run, with the calls between them
    /// folded into one `{"omitted": n}` entry.
    pub(super) fn to_value(&self) -> Option<Value> {
        if self.calls.is_empty() {
            return None;
        }
        let head = self.calls.len().min(WORK_LOG_HEAD_CALLS);
        let tail_start = self
            .calls
            .len()
            .saturating_sub(WORK_LOG_TAIL_CALLS)
            .max(head);
        let entry = |call: &WorkLogCall| {
            json!({
                "tool": call.tool,
                "arguments": call.arguments,
                "outcome": call.outcome.as_deref().unwrap_or("not finished"),
            })
        };
        let mut entries = self.calls[..head].iter().map(entry).collect::<Vec<_>>();
        if tail_start > head {
            entries.push(json!({ "omitted": tail_start - head }));
        }
        entries.extend(self.calls[tail_start..].iter().map(entry));
        Some(json!({
            "entries": entries,
            "filesChanged": self.files_changed.iter().take(WORK_LOG_MAX_FILES).collect::<Vec<_>>(),
        }))
    }
}

/// `key=value` for the arguments that identify what a call acted on.
fn key_arguments(arguments: &Value) -> String {
    WORK_LOG_KEY_ARGUMENTS
        .iter()
        .filter_map(|key| {
            let value = one_line(arguments.get(*key)?.as_str()?, WORK_LOG_ARGUMENT_CHARS);
            Some(if value.contains([' ', '"', '=']) {
                format!("{key}={}", Value::String(value))
            } else {
                format!("{key}={value}")
            })
        })
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A one-line outcome for a tool result, and whether the call succeeded.
fn tool_outcome(result: &Value) -> (String, bool) {
    let flag = |key: &str| result.get(key).and_then(Value::as_bool).unwrap_or(false);
    if flag("denied") {
        return ("denied".into(), false);
    }
    if flag("skipped") {
        return ("skipped".into(), false);
    }
    if let Some(error) = result.get("error").filter(|error| !error.is_null()) {
        let message = error
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return (
            format!("error: {}", one_line(&message, WORK_LOG_OUTCOME_CHARS)),
            false,
        );
    }
    if flag("isError") {
        return ("error".into(), false);
    }
    if flag("timed_out") {
        return ("timed out".into(), false);
    }
    match result.get("exit_code").and_then(Value::as_i64) {
        Some(code) => (format!("exit {code}"), code == 0),
        None => ("ok".into(), true),
    }
}

/// `text` on one line, whitespace runs collapsed, cut to `max_chars`.
fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let mut cut = collapsed
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    cut.push('…');
    cut
}

enum WorkLogLine {
    Call(String),
    Omitted(usize),
}

impl WorkLogLine {
    fn text(&self) -> String {
        match self {
            WorkLogLine::Call(line) => line.clone(),
            WorkLogLine::Omitted(count) => format!(
                "- … {count} more tool call{} …",
                if *count == 1 { "" } else { "s" }
            ),
        }
    }

    fn calls(&self) -> usize {
        match self {
            WorkLogLine::Call(_) => 1,
            WorkLogLine::Omitted(count) => *count,
        }
    }
}

/// Render a stored work log for model replay in about `budget` bytes.
///
/// It is appended to the assistant text as a `<work_log>` section rather than
/// sent as a separate system message: the section stays attached to the turn
/// it describes on every adapter, where mid-conversation system messages are
/// converted to user reminders by some (Anthropic, Gemini), may be hoisted
/// into the system prompt by OpenAI-compatible gateways (moving the log and
/// breaking the cached prefix), and are rejected by some local chat
/// templates. The header marks it as recorded by milim so the model reads it
/// as a record, not as something to write itself. Over budget, the earliest
/// calls keep a third of it and the latest calls the rest.
pub(super) fn render_work_log(log: &Value, budget: usize) -> Option<String> {
    let mut lines = log
        .get("entries")?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            if let Some(count) = entry.get("omitted").and_then(Value::as_u64) {
                return Some(WorkLogLine::Omitted(usize::try_from(count).ok()?));
            }
            let tool = entry.get("tool")?.as_str()?;
            let arguments = entry
                .get("arguments")
                .and_then(Value::as_str)
                .filter(|arguments| !arguments.is_empty());
            let outcome = entry.get("outcome").and_then(Value::as_str).unwrap_or("ok");
            Some(WorkLogLine::Call(match arguments {
                Some(arguments) => format!("- {tool} {arguments} -> {outcome}"),
                None => format!("- {tool} -> {outcome}"),
            }))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    let files = log
        .get("filesChanged")
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let files_line = (!files.is_empty())
        .then(|| one_line(&format!("Files changed: {}", files.join(", ")), budget / 4));
    let remaining = budget.saturating_sub(files_line.as_ref().map_or(0, |line| line.len() + 1));
    let line_len = |line: &WorkLogLine| line.text().len() + 1;
    if lines.iter().map(line_len).sum::<usize>() > remaining {
        const MARKER_RESERVE: usize = 40;
        let mut used = 0;
        let mut head_end = 0;
        while let Some(line @ WorkLogLine::Call(_)) = lines.get(head_end) {
            if used + line_len(line) > remaining / 3 {
                break;
            }
            used += line_len(line);
            head_end += 1;
        }
        let mut tail_start = lines.len();
        while tail_start > head_end {
            let line = &lines[tail_start - 1];
            if matches!(line, WorkLogLine::Omitted(_))
                || used + line_len(line) + MARKER_RESERVE > remaining
            {
                break;
            }
            used += line_len(line);
            tail_start -= 1;
        }
        let omitted = lines[head_end..tail_start]
            .iter()
            .map(WorkLogLine::calls)
            .sum();
        let tail = lines.split_off(tail_start);
        lines.truncate(head_end);
        lines.push(WorkLogLine::Omitted(omitted));
        lines.extend(tail);
    }
    let mut rendered = String::from(
        "<work_log>\nTool calls from this turn, recorded by milim (not part of the visible reply):\n",
    );
    for line in &lines {
        rendered.push_str(&line.text());
        rendered.push('\n');
    }
    if let Some(files_line) = files_line {
        rendered.push_str(&files_line);
        rendered.push('\n');
    }
    rendered.push_str("</work_log>");
    Some(rendered)
}

/// Replay markers for a run that ended before its answer was complete.
pub(super) const STOPPED_BY_USER_MARKER: &str = "[interrupted: stopped by user]";
pub(super) const RUN_LIMIT_MARKER: &str = "[interrupted: run limit reached]";
const FAILURE_REASON_CHARS: usize = 300;

pub(super) fn failed_run_marker(reason: &str) -> String {
    format!("[failed: {}]", one_line(reason, FAILURE_REASON_CHARS))
}

/// Model-replay fields of an assistant message. Clients ignore them;
/// `provider::control_chat_messages` replays them with the message.
#[derive(Debug, Default)]
pub(super) struct AssistantReplay {
    /// The text a model sees (`promptContent`) when it differs from the
    /// displayed content, which also shows harness notices.
    pub(super) prompt_content: Option<String>,
    /// The run's stored [`WorkLog`] (`workLog`).
    pub(super) work_log: Option<Value>,
    /// Why the run ended early (`interruption`), replayed after its output.
    pub(super) interruption: Option<String>,
}

impl AssistantReplay {
    pub(super) fn from_run(
        deltas: &DeltaBuffer<'_>,
        work_log: &WorkLog,
        interruption: Option<String>,
    ) -> Self {
        Self {
            prompt_content: (deltas.model_content() != deltas.content())
                .then(|| deltas.model_content().to_string()),
            work_log: work_log.to_value(),
            interruption,
        }
    }
}
