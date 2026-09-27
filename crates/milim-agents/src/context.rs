//! In-run context management: a cheap prompt-size estimate calibrated by
//! the provider's real counts, eliding older tool outputs, and choosing
//! which span of the conversation to summarize.

use std::ops::Range;

use milim_core::api::openai::{ChatMessage, Content, ContentPart, Tool};

/// Elide old tool outputs above this share of the context window.
pub(crate) const PRUNE_THRESHOLD: f64 = 0.60;
/// Summarize older turns when still above this share after pruning.
pub(crate) const SUMMARIZE_THRESHOLD: f64 = 0.85;
/// Rough token cost charged for one image part.
const IMAGE_TOKENS: usize = 1_000;
/// Per-message framing overhead in tokens.
const MESSAGE_OVERHEAD_TOKENS: usize = 4;
/// Per-message cap when rendering a transcript for summarization.
const TRANSCRIPT_MESSAGE_MAX_CHARS: usize = 4_000;
/// Smallest per-message share when a transcript must fit the window.
const TRANSCRIPT_MESSAGE_MIN_CHARS: usize = 300;
/// Share of the context window a summarization transcript may fill.
const TRANSCRIPT_WINDOW_SHARE: f64 = 0.5;
const SUMMARY_PREFIX: &str = "Summary of the earlier conversation (older turns were condensed to save context; re-run tools if you need exact output):";

/// How much recent context compaction keeps verbatim.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Retention {
    /// Most recent tool results kept when pruning (results answering the
    /// latest assistant turn are always kept).
    pub tool_results: usize,
    /// Most recent turns (assistant or user messages, with their tool
    /// results) kept when summarizing.
    pub turns: usize,
}

/// Compaction under threshold pressure.
pub(crate) const RETAIN: Retention = Retention {
    tool_results: 6,
    turns: 4,
};
/// Compaction after the provider rejected the prompt as too long.
pub(crate) const RETAIN_FORCED: Retention = Retention {
    tool_results: 0,
    turns: 2,
};

/// Estimate prompt tokens as characters / 4 plus small per-message and
/// per-image allowances. No tokenizer is bundled; this errs toward
/// consistency, not precision.
pub(crate) fn estimate_tokens(messages: &[ChatMessage], tools: &[Tool]) -> usize {
    let mut chars = 0;
    let mut images = 0;
    for message in messages {
        match &message.content {
            Some(Content::Text(text)) => chars += text.chars().count(),
            Some(Content::Parts(parts)) => {
                for part in parts {
                    match part {
                        ContentPart::Text { text } => chars += text.chars().count(),
                        ContentPart::ImageUrl { .. } => images += 1,
                        _ => {}
                    }
                }
            }
            None => {}
        }
        if let Some(reasoning) = &message.reasoning_content {
            chars += reasoning.chars().count();
        }
        for call in message.tool_calls.iter().flatten() {
            chars += call.function.name.len() + call.function.arguments.chars().count();
        }
    }
    let tools_chars = serde_json::to_string(tools).map_or(0, |encoded| encoded.len());
    (chars + tools_chars).div_ceil(4)
        + images * IMAGE_TOKENS
        + messages.len() * MESSAGE_OVERHEAD_TOKENS
}

/// The run's view of its context window: the configured size, lowered once
/// a provider rejects a prompt as too long, and the provider's real prompt
/// count for the last request to calibrate the estimate.
#[derive(Debug, Clone, Default)]
pub(crate) struct ContextWindow {
    tokens: Option<usize>,
    /// (real prompt tokens, raw estimate) of the last answered request.
    calibration: Option<(usize, usize)>,
}

impl ContextWindow {
    pub(crate) fn new(tokens: Option<u32>) -> Self {
        Self {
            tokens: tokens
                .filter(|tokens| *tokens > 0)
                .map(|tokens| tokens as usize),
            calibration: None,
        }
    }

    /// The effective window, when known.
    pub(crate) fn tokens(&self) -> Option<usize> {
        self.tokens
    }

    /// Prompt tokens for `messages`: the provider's real count for the last
    /// request plus the estimated growth since, or the raw estimate before
    /// any request has reported usage. A provider that under-reports (for
    /// example by leaving out prompt tokens it served from a cache) never
    /// lowers the estimate below the raw count.
    pub(crate) fn estimate(&self, messages: &[ChatMessage], tools: &[Tool]) -> usize {
        let raw = estimate_tokens(messages, tools);
        match self.calibration {
            Some((real, estimated)) => raw.max((real + raw).saturating_sub(estimated)),
            None => raw,
        }
    }

    /// Record the real prompt size of a request whose raw estimate was
    /// `raw_estimate`. Providers that report no usage leave the estimate raw.
    pub(crate) fn calibrate(&mut self, raw_estimate: usize, prompt_tokens: u32) {
        if prompt_tokens > 0 {
            self.calibration = Some((prompt_tokens as usize, raw_estimate));
        }
    }

    /// A prompt estimated at `overflowing` tokens did not fit: treat that as
    /// the window for the rest of the run.
    pub(crate) fn lower(&mut self, overflowing: usize) {
        let overflowing = overflowing.max(1);
        self.tokens = Some(
            self.tokens
                .map_or(overflowing, |tokens| tokens.min(overflowing)),
        );
    }
}

pub(crate) fn elided_stub(tool: &str) -> String {
    format!("[output of {tool} elided to save context; re-run the tool if you need it]")
}

/// Replace the content of tool results older than the most recent `keep`
/// with a short stub. Results answering the latest assistant turn are never
/// touched, and messages are only edited in place, so every tool_call keeps
/// its tool_result. Returns how many results were elided.
pub(crate) fn elide_old_tool_results(messages: &mut [ChatMessage], keep: usize) -> usize {
    let latest_turn = messages
        .iter()
        .rposition(|message| message.role == "assistant" && message.tool_calls.is_some())
        .unwrap_or(messages.len());
    let tool_indexes = messages
        .iter()
        .enumerate()
        .filter(|(index, message)| message.role == "tool" && *index < latest_turn)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let keep_from = messages[latest_turn..]
        .iter()
        .filter(|message| message.role == "tool")
        .count();
    let prunable = tool_indexes
        .len()
        .saturating_sub(keep.saturating_sub(keep_from));
    let mut elided = 0;
    for &index in &tool_indexes[..prunable] {
        let name = tool_name_for(messages, index).unwrap_or_else(|| "tool".into());
        let stub = elided_stub(&name);
        let message = &mut messages[index];
        let current = message.text_content();
        if current.starts_with("[output of ") || current.len() <= stub.len() {
            continue;
        }
        message.content = Some(Content::Text(stub));
        elided += 1;
    }
    elided
}

fn tool_name_for(messages: &[ChatMessage], tool_index: usize) -> Option<String> {
    let call_id = messages[tool_index].tool_call_id.as_deref()?;
    messages[..tool_index]
        .iter()
        .rev()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .find(|call| call.id.as_deref() == Some(call_id))
        .map(|call| call.function.name.clone())
}

/// What one summary replaces: stale messages before the user request that
/// started the run, and the run's own older turns after it. The request
/// itself sits between the two and is never summarized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummaryPlan {
    pub before: Range<usize>,
    pub after: Range<usize>,
}

impl SummaryPlan {
    fn messages<'a>(&self, messages: &'a [ChatMessage]) -> impl Iterator<Item = &'a ChatMessage> {
        messages[self.before.clone()]
            .iter()
            .chain(&messages[self.after.clone()])
    }
}

/// Choose what to summarize. Everything after the leading system prompt is
/// a candidate, oldest first, up to the last `keep_turns` turns and never
/// past the latest assistant turn, so the current step stays coherent.
/// `anchor` is the user request that started the run (the first user
/// message when unknown); it is always kept. Spans start and end at turn
/// boundaries, so no tool_call is separated from its tool_result, and when
/// only older history is summarized the kept messages open with a user turn.
/// `None` when there is nothing old enough to summarize.
pub(crate) fn summary_span(
    messages: &[ChatMessage],
    anchor: Option<usize>,
    keep_turns: usize,
) -> Option<SummaryPlan> {
    let head = messages
        .iter()
        .position(|message| message.role != "system" || is_summary(message))?;
    let anchor = anchor
        .filter(|anchor| *anchor >= head && messages.get(*anchor).is_some_and(|m| m.role == "user"))
        .or_else(|| {
            messages[head..]
                .iter()
                .position(|message| message.role == "user")
                .map(|offset| head + offset)
        });
    let turn_starts = (head..messages.len())
        .filter(|index| matches!(messages[*index].role.as_str(), "assistant" | "user"))
        .collect::<Vec<_>>();
    let keep_turns = keep_turns.max(1);
    if turn_starts.len() <= keep_turns {
        return None;
    }
    let latest_assistant = messages
        .iter()
        .rposition(|message| message.role == "assistant")
        .unwrap_or(messages.len());
    let recent = turn_starts[turn_starts.len() - keep_turns].min(latest_assistant);
    let plan = match anchor {
        Some(anchor) if anchor < recent => SummaryPlan {
            before: head..anchor,
            after: anchor + 1..recent,
        },
        _ => {
            let end = (recent..messages.len())
                .find(|index| messages[*index].role == "user")
                .unwrap_or(recent);
            SummaryPlan {
                before: head..end,
                after: end..end,
            }
        }
    };
    plan.messages(messages)
        .any(|message| message.role != "system")
        .then_some(plan)
}

/// Replace the plan's messages with one summary message and return how many
/// were folded into it. Earlier summaries are folded, never kept; mid-run
/// system context (steering, injected notes) stays verbatim. The summary
/// takes the place of the run's own older turns, or of the older history
/// when only history was summarized. `anchor` is shifted to match.
pub(crate) fn apply_summary(
    messages: &mut Vec<ChatMessage>,
    plan: &SummaryPlan,
    anchor: &mut Option<usize>,
    summary: &str,
) -> usize {
    let kept = |range: Range<usize>| {
        messages[range]
            .iter()
            .filter(|message| message.role == "system" && !is_summary(message))
            .cloned()
            .collect::<Vec<_>>()
    };
    let kept_before = kept(plan.before.clone());
    let kept_after = kept(plan.after.clone());
    let folded = plan.before.len() - kept_before.len() + plan.after.len() - kept_after.len();
    let summary = summary_message(summary);
    let before_replacement = if plan.after.is_empty() {
        let replacement = kept_before.len() + 1;
        messages.splice(
            plan.before.clone(),
            std::iter::once(summary).chain(kept_before),
        );
        replacement
    } else {
        messages.splice(
            plan.after.clone(),
            std::iter::once(summary).chain(kept_after),
        );
        let replacement = kept_before.len();
        messages.splice(plan.before.clone(), kept_before);
        replacement
    };
    if let Some(index) = anchor.as_mut().filter(|index| **index >= plan.before.end) {
        *index = *index + before_replacement - plan.before.len();
    }
    folded
}

/// Render the plan's messages as a plain transcript for the summarization
/// request, so the request carries no tool_call pairing of its own. An
/// earlier summary is included in full so the new one can fold it in; other
/// messages are cut to share half of `window` tokens when it is known.
pub(crate) fn summary_transcript(
    messages: &[ChatMessage],
    plan: &SummaryPlan,
    window: Option<usize>,
) -> String {
    let count = plan.messages(messages).count().max(1);
    let per_message = window.map_or(TRANSCRIPT_MESSAGE_MAX_CHARS, |tokens| {
        let chars = (tokens as f64 * TRANSCRIPT_WINDOW_SHARE) as usize * 4;
        (chars / count).clamp(TRANSCRIPT_MESSAGE_MIN_CHARS, TRANSCRIPT_MESSAGE_MAX_CHARS)
    });
    transcript(plan.messages(messages), per_message)
}

fn transcript<'a>(messages: impl Iterator<Item = &'a ChatMessage>, max_chars: usize) -> String {
    let mut out = String::new();
    for message in messages {
        if is_summary(message) {
            let text = message.text_content();
            let text = text.strip_prefix(SUMMARY_PREFIX).unwrap_or(&text);
            out.push_str(&format!("### earlier summary\n{}\n\n", text.trim()));
            continue;
        }
        let mut text = message.text_content();
        for call in message.tool_calls.iter().flatten() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!(
                "[called {} with {}]",
                call.function.name, call.function.arguments
            ));
        }
        if text.chars().count() > max_chars {
            text = text.chars().take(max_chars).collect::<String>() + " […]";
        }
        out.push_str(&format!("### {}\n{}\n\n", message.role, text.trim_end()));
    }
    out
}

pub(crate) const SUMMARY_INSTRUCTIONS: &str = "You condense the earlier part of an agent's working session so it can continue with less context. Write a concise summary that preserves: the goals and constraints the user stated, decisions made, files and commands touched with their key results, errors hit and how they were resolved, and any open threads. When the transcript starts with an earlier summary, fold its content into yours. Use short bullet points. Do not invent details and do not address the user.";

pub(crate) fn summary_message(summary: &str) -> ChatMessage {
    ChatMessage::text("system", format!("{SUMMARY_PREFIX}\n{}", summary.trim()))
}

fn is_summary(message: &ChatMessage) -> bool {
    message.role == "system"
        && matches!(&message.content, Some(Content::Text(text)) if text.starts_with(SUMMARY_PREFIX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use milim_core::api::openai::{FunctionCall, ToolCall};

    fn assistant_call(id: &str, name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".into(),
            content: None,
            name: None,
            tool_calls: Some(vec![ToolCall {
                id: Some(id.into()),
                kind: "function".into(),
                function: FunctionCall {
                    name: name.into(),
                    arguments: "{}".into(),
                },
            }]),
            tool_call_id: None,
            reasoning_content: None,
            provider_state: None,
        }
    }

    fn tool_result(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            content: Some(Content::Text(text.into())),
            name: None,
            tool_calls: None,
            tool_call_id: Some(id.into()),
            reasoning_content: None,
            provider_state: None,
        }
    }

    fn conversation(turns: usize) -> Vec<ChatMessage> {
        let mut messages = vec![
            ChatMessage::text("system", "be useful"),
            ChatMessage::text("user", "do the task"),
        ];
        for turn in 0..turns {
            let id = format!("call-{turn}");
            messages.push(assistant_call(&id, &format!("tool_{turn}")));
            messages.push(tool_result(&id, &"x".repeat(500)));
        }
        messages
    }

    #[test]
    fn estimate_grows_with_content() {
        let small = estimate_tokens(&conversation(1), &[]);
        let large = estimate_tokens(&conversation(10), &[]);
        assert!(large > small + 1_000);
    }

    #[test]
    fn pruning_keeps_recent_results_and_pairing() {
        let mut messages = conversation(10);
        let before = messages.clone();
        let elided = elide_old_tool_results(&mut messages, RETAIN.tool_results);
        assert_eq!(elided, 4);
        assert_eq!(messages.len(), before.len());
        for (index, (after, before)) in messages.iter().zip(&before).enumerate() {
            assert_eq!(after.role, before.role);
            assert_eq!(after.tool_call_id, before.tool_call_id);
            assert_eq!(after.tool_calls.is_some(), before.tool_calls.is_some());
            if after.role == "tool" {
                // Tool results sit at 3, 5, ..., 21; the first four are elided.
                if index <= 9 {
                    let turn = (index - 3) / 2;
                    assert_eq!(after.text_content(), elided_stub(&format!("tool_{turn}")));
                } else {
                    assert_eq!(after.text_content(), before.text_content());
                }
            }
        }
        assert_eq!(
            elide_old_tool_results(&mut messages, RETAIN.tool_results),
            0,
            "stubs are not re-elided"
        );
        // Forced pruning keeps only the latest turn's results.
        assert_eq!(elide_old_tool_results(&mut messages, 0), 5);
        assert_eq!(messages[21].text_content(), "x".repeat(500));
    }

    #[test]
    fn pruning_never_touches_the_latest_turn_even_when_it_has_many_results() {
        let mut messages = vec![ChatMessage::text("user", "task")];
        let mut latest = assistant_call("a", "read");
        let calls = (0..8)
            .map(|n| ToolCall {
                id: Some(format!("latest-{n}")),
                kind: "function".into(),
                function: FunctionCall {
                    name: "read".into(),
                    arguments: "{}".into(),
                },
            })
            .collect();
        messages.push(assistant_call("old", "grep"));
        messages.push(tool_result("old", &"y".repeat(500)));
        latest.tool_calls = Some(calls);
        messages.push(latest);
        for n in 0..8 {
            messages.push(tool_result(&format!("latest-{n}"), &"z".repeat(500)));
        }
        assert_eq!(
            elide_old_tool_results(&mut messages, RETAIN.tool_results),
            1
        );
        assert_eq!(messages[2].text_content(), elided_stub("grep"));
        assert!(messages[4..]
            .iter()
            .all(|message| message.text_content() == "z".repeat(500)));
    }

    #[test]
    fn summary_span_keeps_task_and_recent_turns_on_turn_boundaries() {
        let messages = conversation(10);
        let plan = summary_span(&messages, Some(1), RETAIN.turns).unwrap();
        assert!(plan.before.is_empty());
        assert_eq!(plan.after.start, 2);
        // The last four assistant turns (with their results) stay verbatim.
        assert_eq!(plan.after.end, messages.len() - 8);
        assert_eq!(messages[plan.after.end].role, "assistant");
        assert!(summary_span(&conversation(3), Some(1), RETAIN.turns).is_none());
        let rendered = summary_transcript(&messages, &plan, None);
        assert!(rendered.contains("[called tool_0 with {}]"));
        assert!(rendered.contains("### tool"));
    }

    /// A thread with two earlier exchanges, then this run's request and
    /// `turns` tool steps.
    fn thread(turns: usize) -> Vec<ChatMessage> {
        let mut messages = vec![
            ChatMessage::text("system", "be useful"),
            ChatMessage::text("user", "first question"),
            ChatMessage::text("assistant", "first answer"),
            ChatMessage::text("user", "second question"),
            ChatMessage::text("assistant", "second answer"),
            ChatMessage::text("user", "current request"),
        ];
        for turn in 0..turns {
            let id = format!("call-{turn}");
            messages.push(assistant_call(&id, &format!("tool_{turn}")));
            messages.push(tool_result(&id, &"x".repeat(500)));
        }
        messages
    }

    #[test]
    fn summary_span_summarizes_stale_history_first_and_never_the_run_request() {
        let messages = thread(8);
        let anchor = 5;
        let plan = summary_span(&messages, Some(anchor), RETAIN.turns).unwrap();
        // Older exchanges and the run's own older steps; the request stays.
        assert_eq!(plan.before, 1..anchor);
        assert_eq!(plan.after, anchor + 1..messages.len() - 8);
        assert!(!plan.before.contains(&anchor) && !plan.after.contains(&anchor));

        // Early in the run the request is among the recent turns: only the
        // history is summarized, and what is kept opens with a user turn.
        let early = thread(1);
        let plan = summary_span(&early, Some(anchor), RETAIN.turns).unwrap();
        assert_eq!(plan.after.len(), 0);
        assert_eq!(plan.before, 1..3);
        assert_eq!(early[plan.before.end].role, "user");

        // Forced compaction still keeps the latest assistant turn.
        let messages = thread(3);
        let plan = summary_span(&messages, Some(anchor), RETAIN_FORCED.turns).unwrap();
        let latest = messages.len() - 2;
        assert!(plan.after.end <= latest);
    }

    #[test]
    fn applying_a_summary_folds_earlier_summaries_and_keeps_mid_run_context() {
        let mut messages = thread(8);
        messages.insert(
            8,
            ChatMessage::text("system", "Injected context:\nbranch main"),
        );
        let mut anchor = Some(5);
        let plan = summary_span(&messages, anchor, RETAIN.turns).unwrap();
        let folded = apply_summary(&mut messages, &plan, &mut anchor, "- did early work");
        assert_eq!(
            anchor,
            Some(1),
            "the request moved up with the history folded"
        );
        assert_eq!(messages[1].text_content(), "current request");
        assert!(is_summary(&messages[2]));
        assert_eq!(messages[3].text_content(), "Injected context:\nbranch main");
        assert_eq!(folded, 4 + (plan.after.len() - 1));
        assert_eq!(
            messages
                .iter()
                .filter(|message| is_summary(message))
                .count(),
            1
        );

        // More steps later: the earlier summary is folded into the next one,
        // not kept next to it.
        for turn in 8..14 {
            let id = format!("call-{turn}");
            messages.push(assistant_call(&id, &format!("tool_{turn}")));
            messages.push(tool_result(&id, &"x".repeat(500)));
        }
        let plan = summary_span(&messages, anchor, RETAIN.turns).unwrap();
        assert!(
            plan.after.contains(&2),
            "the old summary is part of the span"
        );
        let rendered = summary_transcript(&messages, &plan, None);
        assert!(rendered.starts_with("### earlier summary\n- did early work"));
        apply_summary(&mut messages, &plan, &mut anchor, "- did all the work");
        let summaries = messages
            .iter()
            .filter(|message| is_summary(message))
            .collect::<Vec<_>>();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].text_content().ends_with("- did all the work"));
        assert_eq!(anchor, Some(1));
        assert_eq!(messages[1].text_content(), "current request");
    }

    #[test]
    fn transcript_fits_a_share_of_a_known_window() {
        let messages = conversation(40);
        let plan = summary_span(&messages, Some(1), RETAIN.turns).unwrap();
        let unbounded = summary_transcript(&messages, &plan, None);
        let bounded = summary_transcript(&messages, &plan, Some(2_000));
        assert!(bounded.len() < unbounded.len());
        assert!(bounded.contains(" […]"));
    }

    #[test]
    fn calibrated_estimate_follows_the_real_prompt_count() {
        let messages = conversation(4);
        let raw = estimate_tokens(&messages, &[]);
        let mut window = ContextWindow::new(Some(10_000));
        assert_eq!(window.estimate(&messages, &[]), raw);
        window.calibrate(raw, 0);
        assert_eq!(
            window.estimate(&messages, &[]),
            raw,
            "no usage, no calibration"
        );
        window.calibrate(raw, 3_000);
        assert_eq!(window.estimate(&messages, &[]), 3_000);
        let mut grown = messages.clone();
        grown.push(tool_result("call-9", &"y".repeat(400)));
        let growth = estimate_tokens(&grown, &[]) - raw;
        assert_eq!(window.estimate(&grown, &[]), 3_000 + growth);

        window.calibrate(raw, 10);
        assert_eq!(
            window.estimate(&messages, &[]),
            raw,
            "under-reports never lower it"
        );

        window.lower(2_500);
        assert_eq!(window.tokens(), Some(2_500));
        window.lower(9_000);
        assert_eq!(window.tokens(), Some(2_500), "the window only shrinks");
        let mut unknown = ContextWindow::new(None);
        assert_eq!(unknown.tokens(), None);
        unknown.lower(4_000);
        assert_eq!(unknown.tokens(), Some(4_000));
    }
}
