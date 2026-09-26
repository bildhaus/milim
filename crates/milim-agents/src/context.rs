//! In-run context management: a cheap prompt-size estimate, eliding older
//! tool outputs, and choosing which span of the conversation to summarize.

use milim_core::api::openai::{ChatMessage, Content, ContentPart, Tool};

/// Elide old tool outputs above this share of the context window.
pub(crate) const PRUNE_THRESHOLD: f64 = 0.60;
/// Summarize older turns when still above this share after pruning.
pub(crate) const SUMMARIZE_THRESHOLD: f64 = 0.85;
/// Most recent tool results kept verbatim when pruning.
const KEEP_RECENT_TOOL_RESULTS: usize = 6;
/// Most recent turns (assistant or user messages, with their tool results)
/// kept verbatim when summarizing.
const KEEP_RECENT_TURNS: usize = 4;
/// Rough token cost charged for one image part.
const IMAGE_TOKENS: usize = 1_000;
/// Per-message framing overhead in tokens.
const MESSAGE_OVERHEAD_TOKENS: usize = 4;
/// Per-message cap when rendering a transcript for summarization.
const TRANSCRIPT_MESSAGE_MAX_CHARS: usize = 4_000;

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

pub(crate) fn elided_stub(tool: &str) -> String {
    format!("[output of {tool} elided to save context; re-run the tool if you need it]")
}

/// Replace the content of tool results older than the most recent
/// [`KEEP_RECENT_TOOL_RESULTS`] with a short stub. Results answering the
/// latest assistant turn are never touched, and messages are only edited in
/// place, so every tool_call keeps its tool_result. Returns how many results
/// were elided.
pub(crate) fn elide_old_tool_results(messages: &mut [ChatMessage]) -> usize {
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
        .saturating_sub(KEEP_RECENT_TOOL_RESULTS.saturating_sub(keep_from));
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

/// The span of messages to replace with a summary: everything after the
/// leading system messages and the original user task, up to the last
/// [`KEEP_RECENT_TURNS`] turns. The span always starts and ends at a turn
/// boundary, so no tool_call is separated from its tool_result. `None` when
/// there is nothing old enough to summarize.
pub(crate) fn summary_span(messages: &[ChatMessage]) -> Option<std::ops::Range<usize>> {
    let task = messages.iter().position(|message| message.role == "user")?;
    let start = task + 1;
    let turn_starts = messages
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, message)| matches!(message.role.as_str(), "assistant" | "user"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if turn_starts.len() <= KEEP_RECENT_TURNS {
        return None;
    }
    let end = turn_starts[turn_starts.len() - KEEP_RECENT_TURNS];
    let summarizable = messages[start..end]
        .iter()
        .any(|message| message.role != "system");
    (end > start && summarizable).then_some(start..end)
}

/// Render the span as a plain transcript for the summarization request, so
/// the request carries no tool_call pairing of its own.
pub(crate) fn transcript(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for message in messages {
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
        if text.chars().count() > TRANSCRIPT_MESSAGE_MAX_CHARS {
            text = text
                .chars()
                .take(TRANSCRIPT_MESSAGE_MAX_CHARS)
                .collect::<String>()
                + " […]";
        }
        out.push_str(&format!("### {}\n{}\n\n", message.role, text.trim_end()));
    }
    out
}

pub(crate) const SUMMARY_INSTRUCTIONS: &str = "You condense the earlier part of an agent's working session so it can continue with less context. Write a concise summary that preserves: the goals and constraints the user stated, decisions made, files and commands touched with their key results, errors hit and how they were resolved, and any open threads. Use short bullet points. Do not invent details and do not address the user.";

pub(crate) fn summary_message(summary: &str) -> ChatMessage {
    ChatMessage::text(
        "system",
        format!(
            "Summary of the earlier conversation (older turns were condensed to save context; re-run tools if you need exact output):\n{}",
            summary.trim()
        ),
    )
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
        let elided = elide_old_tool_results(&mut messages);
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
            elide_old_tool_results(&mut messages),
            0,
            "stubs are not re-elided"
        );
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
        assert_eq!(elide_old_tool_results(&mut messages), 1);
        assert_eq!(messages[2].text_content(), elided_stub("grep"));
        assert!(messages[4..]
            .iter()
            .all(|message| message.text_content() == "z".repeat(500)));
    }

    #[test]
    fn summary_span_keeps_task_and_recent_turns_on_turn_boundaries() {
        let messages = conversation(10);
        let span = summary_span(&messages).unwrap();
        assert_eq!(span.start, 2);
        // The last four assistant turns (with their results) stay verbatim.
        assert_eq!(span.end, messages.len() - 8);
        assert_eq!(messages[span.end].role, "assistant");
        assert!(summary_span(&conversation(4)).is_none());
        let rendered = transcript(&messages[span]);
        assert!(rendered.contains("[called tool_0 with {}]"));
        assert!(rendered.contains("### tool"));
    }
}
