//! One builder for every native tool-agent run: prompt layout, instructions,
//! skills, environment, tools, hooks, and approvals.

use super::*;

/// Where a native tool-agent run's request came from. Every kind goes through
/// [`build_native_run`], so the base prompt, instructions, skills,
/// environment, hooks, and approvals cannot drift between them; the matches
/// on this enum are the only places they differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeRunKind {
    /// Canonical desktop, mobile, and scheduled turns from the control plane.
    /// The history comes from the thread's ledger, and its leading system
    /// messages carry this turn's context (the managed preview runtime,
    /// linked-thread mail), not standing instructions.
    Control,
    /// `POST /agents/run` and `POST /agents/{id}/run`. The caller composes
    /// the messages, and its leading system messages are its own
    /// instructions.
    Http,
}

/// Everything one native run is built from.
pub(super) struct NativeRunSpec<'a> {
    pub(super) kind: NativeRunKind,
    pub(super) model: &'a str,
    pub(super) run_id: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) run_context: &'a RunContext,
    pub(super) policy: ToolRunPolicy,
    /// The run streams its events, so an interactive approval can reach the
    /// user.
    pub(super) streamed: bool,
    pub(super) tool_mode: &'a str,
    pub(super) enabled_tools: &'a [String],
    pub(super) skill_mode: &'a str,
    pub(super) enabled_skills: &'a [String],
    /// The caller already put the skills it wants into `messages`.
    pub(super) skills_resolved: bool,
    pub(super) instructions: crate::workspace_context::InstructionLayers,
    pub(super) memory: AgentMemoryContext,
    /// The conversation. For an HTTP run its leading system messages are the
    /// caller's instructions; for a control run it is the replayed thread
    /// history, left in place.
    pub(super) messages: Vec<ChatMessage>,
    /// This turn's own context (linked-thread mail, preview runtime). It
    /// joins the per-turn system message, outside the cached prefix.
    pub(super) turn_context: Vec<ChatMessage>,
}

/// A native run ready for the agent loop.
pub(super) struct NativeRun {
    pub(super) messages: Vec<ChatMessage>,
    pub(super) tools: ToolRegistry,
    pub(super) config: milim_agents::AgentRunConfig,
    /// The per-turn system message as sent, if any. Replaying it verbatim
    /// before the same user message on later turns keeps their prefix
    /// cacheable.
    pub(super) turn_context: Option<String>,
}

/// Build a native tool-agent run on top of the caller's loop `config`.
///
/// The prompt is laid out for provider prompt caching. A leading system
/// prefix stays byte-identical from turn to turn of a thread: the base prompt
/// for the run's tools, one block of instructions from broadest to most
/// specific, the caller's own instructions, the skill index, and the stable
/// environment. Everything that changes per turn (today's date, git state,
/// skills mentioned in or relevant to the request, and per-turn context)
/// goes in one system message right before the latest user message; adapters
/// with a separate system prompt (Anthropic, Gemini) send it there as
/// `<system-reminder>` user text in that user turn, outside the cached
/// prefix (on a thread's first turn nothing separates it from the prefix
/// yet, so it rides at the end of the system prompt). Everything is captured
/// once, so it also stays byte-identical across the run's steps. Runs
/// without tools are plain chat and get no base prompt or environment.
pub(super) fn build_native_run(
    st: &AppState,
    spec: NativeRunSpec<'_>,
    mut config: milim_agents::AgentRunConfig,
) -> NativeRun {
    let NativeRunSpec {
        kind,
        model,
        run_id,
        thread_id,
        run_context,
        policy,
        streamed,
        tool_mode,
        enabled_tools,
        skill_mode,
        enabled_skills,
        skills_resolved,
        instructions,
        mut memory,
        mut messages,
        turn_context,
    } = spec;
    // A control run's history can open with system notes of its own (a
    // compaction checkpoint, earlier turns' context), which stay in place.
    let caller_instructions: Vec<ChatMessage> = match kind {
        NativeRunKind::Control => Vec::new(),
        NativeRunKind::Http => {
            let leading = messages
                .iter()
                .take_while(|message| message.role == "system")
                .count();
            messages.drain(..leading).collect()
        }
    };
    let requests: Vec<String> = messages
        .iter()
        .filter(|message| message.role == "user")
        .map(ChatMessage::text_content)
        .collect();
    let query = requests.last().cloned().unwrap_or_default();
    let workspace = run_context.workspace();
    let skills = if skills_resolved {
        crate::AgentSkillContext::default()
    } else {
        crate::agent_skill_context(st, skill_mode, enabled_skills, &query, workspace)
    };
    memory.worker_context = worker_context(
        &query,
        &instructions,
        &caller_instructions,
        skills.explicit.as_deref(),
    );
    let policy = policy.with_management_tools_for(requests.iter().map(String::as_str));
    let mut tools = agent_registry_for_mode_with_context(
        st,
        tool_mode,
        enabled_tools,
        Some(memory),
        &policy,
        run_context,
    );
    register_skill_tools(&mut tools, st, skill_mode, enabled_skills, workspace);
    let can_load_skills = tools.contains("load_skill");
    let environment = (!tools.is_empty())
        .then(|| crate::workspace_context::RunEnvironment::capture(workspace, model));

    let system = |text: String| ChatMessage::text("system", text);
    let mut prompt = Vec::new();
    if !tools.is_empty() {
        prompt.push(system(crate::agent_prompt::base_system_prompt(
            &tools,
            policy.plan_mode,
        )));
    }
    let files = crate::workspace_context::resolve(workspace);
    prompt.extend(crate::workspace_context::instruction_block(&files, &instructions).map(system));
    prompt.extend(caller_instructions);
    if can_load_skills {
        prompt.extend(skills.index.map(system));
    }
    if desktop_workspace_unavailable_for(st, workspace) {
        prompt.push(system(WORKSPACE_UNAVAILABLE_SYSTEM_PROMPT.to_string()));
    }
    prompt.extend(environment.as_ref().map(|env| system(env.render_stable())));

    let mut turn = Vec::new();
    turn.extend(environment.as_ref().map(turn_environment));
    if can_load_skills {
        turn.extend(skills.relevant);
    }
    turn.extend(skills.explicit);
    turn.extend(turn_context.iter().map(ChatMessage::text_content));
    let turn_context = add_turn_context(&mut messages, turn);
    prompt.extend(messages);

    if streamed
        && policy.approval == ToolApprovalPolicy::Review
        && policy.interactive_approval
        && !policy.approval_granted
    {
        config.approval_broker = Some(st.tool_approvals.clone());
    }
    config.interceptor = crate::user_hooks::interceptor(workspace, run_id, thread_id);
    NativeRun {
        messages: prompt,
        tools,
        config,
        turn_context,
    }
}

/// The date and git state a turn starts from.
pub(super) fn turn_environment(environment: &crate::workspace_context::RunEnvironment) -> String {
    format!(
        "Context for this turn, captured when it started (re-check git state before relying on it):\n{}",
        environment.render_turn()
    )
}

/// Put this turn's context in one system message right before the latest
/// user message, and return its text. Everything earlier stays untouched,
/// and the conversation still ends with the user's message, which some
/// OpenAI-compatible APIs require.
pub(super) fn add_turn_context(
    messages: &mut Vec<ChatMessage>,
    sections: Vec<String>,
) -> Option<String> {
    let text = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        return None;
    }
    let at = messages
        .iter()
        .rposition(|message| message.role == "user")
        .unwrap_or(messages.len());
    messages.insert(at, ChatMessage::text("system", text.clone()));
    Some(text)
}

/// What a delegated Worker learns about the run that delegates to it: the
/// request, plus the instructions and explicitly mentioned skills the run
/// follows. Workers cannot call `load_skill`, so they get those skill bodies
/// but not the index. Repository instruction files are added for the
/// Worker's own folder when the Worker Run is created.
pub(super) fn worker_context(
    query: &str,
    instructions: &crate::workspace_context::InstructionLayers,
    caller_instructions: &[ChatMessage],
    explicit_skills: Option<&str>,
) -> Option<String> {
    let labeled = [
        ("Custom instructions", instructions.milim.as_str()),
        ("Agent instructions", instructions.agent.as_str()),
        ("Thread instructions", instructions.thread.as_str()),
    ]
    .into_iter()
    .filter(|(_, text)| !text.trim().is_empty())
    .map(|(label, text)| format!("{label}:\n{}", text.trim()));
    let resolved = labeled
        .chain(caller_instructions.iter().map(ChatMessage::text_content))
        .chain(explicit_skills.map(str::to_string))
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let context = [
        (!query.trim().is_empty()).then(|| format!("Current request:\n{query}")),
        (!resolved.is_empty()).then(|| format!("Resolved instructions and skills:\n{resolved}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n");
    (!context.is_empty()).then(|| context.chars().take(32_000).collect())
}
