---
id: agents
path: agents
label: Agents
title: Agents, tools, skills, and schedules
summary: Reusable Agent profiles, Worker Runs, tool modes, skills, schedules, and approval policies.
group: Core
order: 50
updated: 2026-09-27
---

Agents are for repeatable behavior, tool access, and longer work. Keep one-off questions in plain chat; save an agent when the same instructions or tool policy should survive across threads.

## Agent building blocks

| Block | Behavior |
|---|---|
| Named Agents | Model-agnostic profiles with name, description, deterministic avatar seed, system prompt, tool mode, and skill mode. **Start chat** creates a normal thread bound to the Agent while model choice remains thread-owned. The generated avatar follows the Agent through desktop persona, schedule, and assigned Worker surfaces plus the native mobile composer and Agent sheet; unassigned Workers receive deterministic run-local identities. An Agent is a saved role; a Worker is one live instance of that role. |
| Tool modes | `all`, `custom`, or `none`. `all` holds back a few tools until they are needed: `schedule_*` tools appear once a user message in the thread mentions scheduling or automation (for example "schedule", "cron", "daily", or "every morning"), `mcp_server_*` tools once one mentions MCP, and both then stay for the rest of the thread; `list_agents` appears only with delegation, schedule tools, or Plan mode; `current_time` stays hidden because each turn carries the date. A `custom` Agent that names one of these tools still gets it, and account runtimes always keep the schedule and MCP groups. |
| Skill modes | `auto`, `custom`, or `none`. Auto offers every enabled user and project skill, Custom only the Agent's allowlisted user skills, and None no index or skill tools. Explicit `@Skill Name` and `/Skill Name` prompt tags load a matching enabled skill in full for that turn, limited to the allowlist in Custom. See [Skills](#skills). |
| Run timeline | Start, token, reasoning, tool call, bounded tool result, [hook](#hooks), memory, Worker Run, per-request usage deltas, final usage, and error events render as structured stream parts. Tool results are capped before timeline persistence and again for model replay (see [Agent loop behavior](#agent-loop-behavior)). Worker events carry monotonic cursors and reload on demand. Runs stop at 100 model turns by default (`stopped_at_limit: true`). |
| Schedules | Cron schedules capture an explicit model, creation workspace, prompt, files, and optional Agent. Each occurrence is a normal canonical thread with a durable schedule origin, complete run ledger, and desktop/mobile visibility. Retrying the same occurrence is idempotent. Legacy schedules with no model temporarily fall back to their Agent's deprecated saved model; editing persists that fallback. Missing both records a visible error. |
| Tool approval | The UI sends approval policy to the server-side agent loop and resolves exact one-shot Review requests inline. |
| MCP Apps | Negotiated MCP tools may attach a server-authored `ui://` view. The agent sees bounded fallback content while the transcript retains the full structured App result and descriptor. App-only tools stay out of the model catalog. |

## Agent loop behavior

These rules apply to milim-native provider runs, streamed or not: a non-streaming `/agents/run` collects the same loop, with the same retries, compaction, and cut-off recovery. Account runtimes use their own loops.

| Concern | Behavior |
|---|---|
| Limits | A run stops at its step limit (100 model turns by default), its optional time or spend limit, or the user's Stop. Limits are checked between model and tool boundaries, and time spent waiting for a person to answer approvals does not count against the time limit. The stop reason ("… Send Continue to start another bounded run") arrives as a `notice` event: it is shown to the user and never replayed to the model as assistant text. A paused run can be continued. |
| Provider retries | Rate limits (429), overload (529), server errors (5xx), request timeouts or conflicts (408/409), connection failures, and a stream that closes without its completion event ("provider stream ended before a completion event") are retried up to 4 times with exponential backoff and jitter (at most 30 s per wait). A provider `Retry-After`, Gemini `retryDelay`, or "retry in Ns" is honored up to 60 s. A 429 counts as a rate limit unless its body reports hard billing or credit exhaustion (such as `insufficient_quota`), which fails at once like auth, unknown-model, and bad-request errors; context-length errors are handled under Context overflow. Tool calls are neither announced nor run until their stream completes, so a failed step is discarded and retried even after tool-call arguments started streaming. Each retry emits a `provider_retry` event with the attempt, delay, reason, and the bytes of text and reasoning the failed attempt streamed. The chat drops that partial text at once and shows a quiet "Retrying after rate limit (attempt 2, 4s)..." line while it waits, which folds into the work summary once the retried step streams; the run timeline shows the same line. Mobile shows a compact "Retried after..." row. Backoff never outlasts the run time limit, and Stop cancels it. |
| Tool arguments | Arguments that are not valid JSON, miss required properties, or give a top-level property the wrong type are answered with an error the model can act on, and the tool does not run. Empty arguments count as `{}`. An unknown tool name returns the list of available tools. When the response was cut off at the output token limit, the error says so and asks for smaller calls. |
| Cut-off responses | A response cut off at the output token limit (`length`, including Anthropic `max_tokens`) does not end the run: milim adds a short note asking the model to continue with smaller tool calls and takes another step, at most twice in a row. |
| Tool output | The model receives a tool's plain-text projection verbatim when it has one. Otherwise a string result is sent raw, and an object's multi-line string fields are lifted out of the JSON into `--- field ---` sections with real newlines. Output over 2,000 lines or 50 KiB keeps its head and tail around a `[… N lines / M bytes omitted …]` marker. All tool results of one step together add at most 100 KiB of model-visible text; over that, the largest results are shortened first. The full text is saved under `~/.milim/tool-output/<run id>/<call id>.txt`, and the model is told the path so it can page through it with `read_file` offset/limit or `grep`. Saved output older than seven days is pruned at startup. The transcript keeps the structured result. |
| Context window | When the model's context window is known (see [Models](models#context-windows)), milim estimates the prompt before each step: characters / 4, calibrated by the `prompt_tokens` the provider reported for the previous step and never below the raw estimate. Above 60% of the window, tool results older than the six most recent are replaced with a short stub; results for the latest turn are never touched and every tool call keeps its result. Above 85%, one request to the same model summarizes the oldest messages. The user request that started the run, the latest assistant turn, and the last four turns stay verbatim, and an earlier summary is folded into the new one instead of stacking. The summary request uses the same retry policy and no stop sequences, raises a run output cap to at least 4,096 tokens, and sends a transcript capped at half the window. If it fails, milim elides the older tool results instead and `context_compacted` carries `summary_error`. A `context_compacted` event reports what changed, shown in the chat as a quiet notice such as "Context compacted: 12 older tool outputs elided, 30 messages summarized", and the request committed to the run ledger is exactly the one sent. |
| Context overflow | A context-length error, or a `model_context_window_exceeded` stop, does not fail the run. milim emits a `provider_retry` with reason "context window exceeded", force-compacts (elides every tool result before the latest assistant turn, keeps the last two turns, and summarizes the rest), and retries the step once, even when the model's window is unknown. The rejected prompt's size becomes the run's effective window. If the prompt still does not fit, the run fails with "The conversation no longer fits this model's context window, even after compacting older context". |
| Approvals | All approval requests of one step appear at once and can be answered in any order; the results are applied in call order. An interactive approval waits up to 60 minutes. On expiry the call is denied, the model sees "approval request timed out", the approval card reads "Approval timed out", and the stored approval is marked expired. |
| Timing | Each model step records a `model_timing` run event (start time, time to first token, duration, attempts, finish reason) and each executed tool a `tool_timing` event (duration and whether it failed). |

## Approval modes

| Mode | Server behavior |
|---|---|
| `review` | Read-only tools run automatically. Every mutating, command, or unknown call pauses before execution and shows its exact arguments inline. Approve or Deny resolves only that invocation; Stop, disconnect, restart, or the run ending for any other reason cancels it, and a request left unanswered for 60 minutes is denied with "approval request timed out" and stored as expired. This is the default for new chats. |
| `guarded` | Only tools declaring a read-only effect are exposed. Writes, commands, schedules, computer/preview actions, memory writes, and unclassified MCP tools are withheld. External MCP tools count as read-only only when their server marks them `readOnlyHint` and the user enabled **Trust this server's read-only hints**; otherwise Guarded withholds them. |
| `open` | Host filesystem and shell tools run with unrestricted machine access; the selected folder is their working directory, not a sandbox boundary. Supported account runtimes receive their native full-access mode. Enabled-tool, computer-use, MCP, memory, skill, connector-input, and connector-authorization gates still apply. Switching to Open auto-approves ordinary pending and subsequent command, file-change, and permission requests. |

milim-native uses the registry's effect metadata. Review and Guarded bind host filesystem tools to the selected workspace; Open removes that boundary. The separate **Docker sandbox** setting only enables the bounded `run_command` tool and does not constrain Open host tools. Codex keeps `on-request` approval and relays app-server command, file, and permission requests: Review uses a workspace-write sandbox after approval, while Open uses Codex `danger-full-access` and auto-approves ordinary requests. Claude uses a temporary per-run Streamable HTTP MCP permission tool and deletes its run token/configuration on completion. A runtime that cannot support its approval protocol fails Review instead of silently switching modes. API callers may still set `tool_approval_grant: true` as an explicit whole-run compatibility grant; streamed desktop runs do not.

Each turn also reloads workspace instructions. milim-native receives both AGENTS and Claude families. Codex relies on its native AGENTS discovery and receives Claude-family additions; Claude relies on native Claude discovery and receives AGENTS-family additions. Conditional Claude rules with `paths:` frontmatter are reported but not globally applied by milim.

## Hooks

Hooks run your own shell commands at fixed points of a milim-native run: before a tool call, after it, when a turn starts, and when the run is about to finish. Managed Workers on a milim model run them too, for the folder they work in (their review worktree when they have one). Account runtimes (Codex, Claude, OpenCode, Pi) keep their own hook systems and don't run milim hooks. Hooks run only for runs with a working folder.

Configure them under `hooks` in `~/.milim/settings.json` (user, or `$MILIM_HOME/settings.json`) and `<workspace>/.milim/settings.json` (project). User hooks run first, then project hooks, each in file order.

```json
{
  "hooks": {
    "PreToolUse": [{ "matcher": "shell|edit_file", "command": "./scripts/guard.sh", "timeout_secs": 30 }],
    "PostToolUse": [],
    "UserPromptSubmit": [],
    "Stop": []
  }
}
```

`matcher` is a regex that must match the whole tool name (`shell|edit_file`, or `github_.*__.*` for one [MCP server's tools](desktop#mcp-apps)); empty or missing matches every tool, and it is ignored for `UserPromptSubmit` and `Stop`. A tool also matches under its earlier names, so a matcher written for an older MCP tool name keeps working. `timeout_secs` defaults to 30 (max 600); a hook that runs longer is stopped with its whole process tree and logged as timed out. Commands run with `sh -c` (PowerShell on Windows) in the workspace folder, with the same search path as other helper tools. Milim's own secret `MILIM_*` variables are withheld; `MILIM_HOOK_EVENT` and `MILIM_WORKSPACE` are set. The event arrives as JSON on stdin:

```json
{"event": "PreToolUse", "tool_name": "shell", "tool_input": {"command": "cargo test"}, "call_id": "call_1", "run_id": "…", "thread_id": "…", "workspace": "/path/to/repo"}
```

`PostToolUse` adds `tool_output` (the tool's structured result), `UserPromptSubmit` has `prompt` instead of tool fields, and `Stop` has `last_message` and `stop_continuations`.

| Event | Exit 0 | Exit 2 | Other exit codes |
|---|---|---|---|
| `PreToolUse` | Runs before approval. Stdout `{"decision":"deny","reason":"…"}` denies the call with that reason. `{"decision":"allow"}` skips the Review approval prompt only when user settings set `"allow_hooks_to_approve": true` (never read from project settings). Allow never adds a tool: Guarded and Plan mode still withhold what they withhold. | Denies the call; the model sees stderr as the reason. | Logged as a hook error; the call proceeds. |
| `PostToolUse` | Non-empty stdout (up to 8 KiB) is appended to the model-visible tool result as `[hook] …`. | Stderr is appended the same way, as feedback. | Logged as a hook error. |
| `UserPromptSubmit` | Runs before the turn's first model step. Non-empty stdout is added as extra context in a system reminder. | Blocks the turn; the run ends with stderr as the reason. | Logged as a hook error. |
| `Stop` | The run finishes. | The run continues with stderr as a note to the model, at most 3 times per run. | Logged as a hook error. |

Every hook run appears in the run timeline with its event, command, outcome, and duration. Routine runs fold into the turn's work summary; denials, blocks, errors, and timeouts show as warnings.

**Project hooks need trust.** Project hooks are code from the repository, so they don't run until you trust them. A run that finds untrusted project hooks skips them and shows **Project hooks are not trusted** in the timeline; **Review and trust hooks** lists the commands and records trust for that workspace and that exact `hooks` configuration (a SHA-256 of it) in `~/.milim/config/hook-trust.json`. Any change to the project's hooks needs trust again. User hooks are always trusted. `GET /hooks?workspace=<path>` returns both configurations and the trust state, and `POST /hooks/trust` with `{workspace, config_hash, trusted}` records or removes trust; trusting with a hash that no longer matches the file is refused.

Examples:

```json Block rm -rf
{"hooks": {"PreToolUse": [{
  "matcher": "shell",
  "command": "grep -q 'rm -rf' && { echo 'rm -rf is blocked in this repo' >&2; exit 2; }; exit 0"
}]}}
```

```json Format after edits
{"hooks": {"PostToolUse": [{
  "matcher": "edit_file|write_file",
  "command": "cargo fmt --quiet 2>&1 | tail -n 20"
}]}}
```

```json Append test results after edits
{"hooks": {"PostToolUse": [{
  "matcher": "edit_file|write_file",
  "command": "cargo test --quiet 2>&1 | tail -n 15",
  "timeout_secs": 300
}]}}
```

```json Keep going until tests pass
{"hooks": {"Stop": [{
  "command": "out=$(cargo test --quiet 2>&1) && exit 0; printf 'Tests fail:\\n%s\\n' \"$(printf '%s' \"$out\" | tail -n 40)\" >&2; exit 2",
  "timeout_secs": 300
}]}}
```

## Base prompt and environment

Every milim-native tool-agent run, whether it starts from desktop, mobile, a schedule, or the `/agents/run` API, sends a stable prefix, then the conversation, with one per-turn context message right before the latest user message. Account runtimes (Codex, Claude, OpenCode, Pi) keep their own harness prompts, and plain chat or a run whose policy leaves no tools gets no base prompt or environment.

| Part | Contents |
|---|---|
| Base prompt | First. Identifies milim's coding agent and covers working style (answer questions without editing files and make changes when asked, understand before changing, minimal consistent edits, keep working until the request is done instead of ending with next steps, verify with the project's tests or build, report outcomes honestly), tool use, safety (no destructive commands without clear intent, respect approvals, never exfiltrate secrets, and, when `shell` exists, never commit, push, amend, or rewrite Git history unless asked), and output (brief, `path:line` references). The tool section is built from the run's final registry and mentions only tools and parameters that exist, such as `glob`/`grep`, `read_file` ranges, `edit_file` versus `write_file`, parallel read-only calls, `shell` timeouts and background processes, `todo_write`, `web_search`/`http_fetch`, `delegate_workers`, and `load_skill`. With `read_file` it adds that earlier turns are replayed as text plus a condensed work log, so files should be re-read. In Plan mode it adds the read-only planning rules. It states that the instructions that follow it take precedence. |
| `# Instructions` | One block with every instruction source, ordered from broadest to most specific so the later one wins on conflict: Milim custom instructions, user files (`AGENTS.override.md` or `AGENTS.md` in `$CODEX_HOME` or `~/.codex`, `~/.claude/CLAUDE.md`, and `~/.claude/rules`), the Agent's instructions, repository files from the repository root down to the working folder (`AGENTS.override.md` or `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, and `.claude/rules`), then thread instructions. |
| Caller system messages | System messages an API caller sent, in order. |
| Skill index | See [Skills](#skills). |
| Environment | A stable `<environment>` block: OS and architecture, shell dialect (`sh` or PowerShell), time zone, the absolute workspace root or "none", whether it is a Git repository, and the model id. |
| Per-turn context | "Context for this turn…": today's date, and in a Git repository the current branch, up to 20 `git status --porcelain` lines with the total count, and the five most recent commits; skills mentioned with `@` and relevant skills; preview-runtime and linked-chat context. Anthropic and Gemini receive it as `<system-reminder>` user text. Git commands run with a 3-second timeout and no optional locks. |

The prefix stays byte-identical from step to step and turn to turn (the date, at day granularity, and the Git state travel in the per-turn message), so provider prompt caching covers it. Each turn's context message is stored with its reply and replayed verbatim on later turns (see [Conversation history across turns](#conversation-history-across-turns)).

AGENTS files and Claude files (CLAUDE.md, rules, and their imports) each load within their own 32 KiB budget. A CLAUDE.md `@path` import resolves relative to the importing file, `@~/` from the home folder, or as an absolute path; imports follow at most four hops, skip cycles, and are ignored inside code spans and fenced blocks. Repository files may import only files inside the repository, and imported files load before the file that imports them.

Managed Workers on a milim model get the base prompt and environment too: the tool section follows the Worker's own read-only or worktree-scoped tools, and the environment and turn context describe the folder the Worker works in (its review worktree when it has one). The Worker's run context (with the parent's resolved instructions), Agent instructions, and Worker role sit between them.

## Conversation history across turns

A canonical turn builds its model input from the thread's SQLite timeline. Later steps of the same run are rebuilt from an exact in-memory copy of the request milim last sent, with the run ledger as the fallback; the ledger masks only credential spans (see [Privacy](privacy#what-is-enforced-server-side)).

| History item | Replayed as |
|---|---|
| Assistant turn | Its model-visible text, with step texts separated by a blank line, plus a compact `<work_log>` of the run's tool calls: tool, key arguments, outcome, and files changed. The latest run's log gets about 3,500 characters and earlier runs about 1,000. |
| Stopped or failed run | Its partial reply and work log with a marker: `[interrupted: stopped by user]`, `[failed: <reason>]`, or `[interrupted: run limit reached]`. A run that produced nothing is answered by its marker, so every user message keeps a reply. |
| Per-turn context | Replayed verbatim before the same user message, which keeps the provider's cached prefix valid. |
| `/compact` checkpoint | Its summary replaces the messages it covers. |
| Steering message | Sent with its image attachments. |

Canonical assistant messages carry these as `promptContent` (the model-visible text when it differs from `content`), `workLog`, `interruption`, and `turnContext`. When a thread switches to an account runtime, the synced milim turns include their work log.

## Skills

A skill is a folder with a `SKILL.md` (frontmatter `name` and `description`, then instructions) and optional resources such as scripts, references, and templates. milim imports user skills from `$CODEX_HOME/skills`, `~/.codex/skills`, `~/.agents/skills`, and `~/.claude/skills` into its skill store at startup. Each native run also discovers project skills from `<workspace>/.milim/skills/*/SKILL.md` and `<workspace>/.claude/skills/*/SKILL.md` for that run's workspace only; a project skill replaces a user skill with the same name, and `.milim` wins over `.claude`.

Native runs use progressive disclosure instead of injecting skill bodies every turn:

- A compact index lists each available skill's name and one-line description. With 20 or fewer skills the index lists all of them in name order, so it stays stable across turns; with more it lists the 20 most relevant and points to `milim_skill_search` for the rest.
- `load_skill {name}` returns the full `SKILL.md` instructions plus the list of resource files in the skill folder. `load_skill {name, file}` reads one of those files. Paths must stay inside the skill folder, and files over 256 KiB or that are not UTF-8 text are refused. Both skill tools are read-only, run in parallel, and stay available in Plan mode.
- An explicit `@Skill Name` or `/Skill Name` mention injects that skill's full body up front, within a 12,000-character budget that keeps or omits whole skills.
- Skill scripts are never executed automatically; the model reads them with `load_skill` and runs them through the normal tools and approval policy.

Relevance ranking ignores stopwords and very common words, matches whole words with simple plural folding, scores name matches above description matches above body matches, and requires a minimum score, so a single incidental body match never selects a skill. `POST /skills/select` and `milim_skill_search` use the same ranking.

Approval is not just UI decoration. The server rebuilds the effective tool registry per run and removes tools that are not allowed by the current policy.

Approval controls execution, not a virtual patch queue. After an approved consequential call runs, the latest response's changed-files card inspects the resulting repository diff. Review failures retain **Retry** and **Open Git**, and **Undo** restores the pre-turn checkpoint.

The same policy is rechecked for calls made by an inline MCP App. Review approval is valid only for the exact displayed call; Guarded accepts only a tool whose MCP annotations declare it read-only on a server whose read-only hints are trusted; Open accepts eligible app-visible tools. An App can call only tools from its fixed originating server, so one server's view cannot use another server's private catalog.

## Agents, Workers, and Runs

The parent chat is canonical. Delegated work is stored as a Worker Run attached to one parent turn and never becomes a sidebar chat. A Run contains one to four independent tasks; each task creates a Worker. The model sees one `delegate_workers` operation rather than lifecycle tools for spawning, listing, reading, waiting, and stopping children.

At run acceptance milim resolves a bound Agent exactly once and stores its complete immutable snapshot with the run. Worker proposals also freeze their assigned Agent snapshots before approval, so editing or deleting a profile cannot rewrite running work or an approved plan. Legacy nonterminal proposals without snapshots are stale and must be proposed again. If a thread's Agent is later deleted, history remains readable but new sends are blocked until the binding is cleared or replaced.

The read-only `list_agents` model tool returns Agent IDs, names, descriptions, avatars, and compact tool/skill capability summaries. It deliberately omits system prompts.

Each thread has a delegation policy:

| Policy | Behavior |
|---|---|
| `off` | Delegation is unavailable for that turn. |
| `ask` | The model may freeze an exact task plan. milim pauses for **Run workers** or **Continue solo** before executing it. This is the default for existing and new threads. |
| `auto` | Independent managed workers run in parallel and their results are joined before the parent answers. Read-only account-runtime turns may instead report native worker activity through the same Run contract. |

The Worker model control uses the searchable model catalog and defaults to the parent chat model.

Desktop shows compact Worker avatars plus planned/active/done counts in the thread's Context card. That summary opens the Workers inspector, which groups the canonical parent chat's history into Active and Done, focuses transcript-linked Runs, keeps Active Workers and the selected Worker stable while progress arrives, renders Worker transcripts and result fallbacks as Markdown, and keeps delegation/model settings, Ask approval, live results, stopping, retry, Run deletion, and diff review together. Worker elapsed times use the canonical UTC start timestamp. Failed or stopped Workers can retry with the same model or a model chosen from the existing catalog; each retry is a new Run that preserves the failed attempt and its approved access/context until that terminal Run is explicitly deleted. Deletion requires a second confirmation and removes the Run, Worker transcripts, and events; proposed or running Runs must be stopped first. The Worker Run stream is the sole owner of managed Worker progress. Before resuming the parent, desktop reloads the canonical terminal Run, verifies every returned Worker is terminal, and requires successful or partial Runs to include every planned Worker. Results enter model context exactly once through a hidden synthesis message, and a persisted pending marker lets an approved join finish after desktop reload. If no Worker succeeded, the parent acknowledges the failures and continues the original request with delegation disabled. Running Workers keep their parent thread visibly active in the sidebar. On narrow layouts Worker history stacks above the selected detail, grows only to its content up to half the inspector height, and scrolls beyond that cap. In Ask mode the parent turn stops at the frozen proposal and resumes only after the user chooses **Run workers** or **Continue solo**. On wide layouts Context can remain open beside the inspector; proposed or running Runs reveal Workers automatically.

Delegation is intended for independent work that benefits from parallelism, not short or sequential steps. Managed Workers receive the current request, selected goal and the parent's resolved instructions, workspace and branch, resolved Agent instructions and skills, supported attachments, their assigned task, and the task's `role` when one is given. They do not receive the full transcript. Each Worker has its own run state (checklist, shell working directory, background processes, and file-read records) and runs the user's [hooks](#hooks). The parent's request arrives inside a `<parent_context>` block marked as background, so a Worker carries out only its assigned task rather than the parent's whole goal, and read-only Workers are told to investigate and report instead of trying to change files.

Workers are limited to four per Run and sixteen process-wide. Managed Workers have a five-minute deadline that runs from when they start; a backstop timer stops unfinished work even if the delegating call was cancelled, and milim preserves available results and visible failures. `delegate_workers` itself is allowed five minutes plus two minutes of setup and 30 seconds, instead of the default two-minute tool deadline, so its cleanup always runs; `linked_thread_wait` gets its requested wait plus 30 seconds. Stopping the parent stops its active Run, and restart recovery marks unfinished Runs as errors so stale running states are never shown.

Managed Workers are read-only by default. In Open, their read tools inherit unrestricted host paths so audits can inspect sibling projects; the selected folder remains their working directory. An approved `ask` Run may request write-review access only when the parent uses Review with a grant or Open. Each writer runs against an isolated Git worktree and returns a reviewable diff that is never auto-applied. A non-Git workspace falls back to read-only.

The physical child-thread tables and `/threads/*` routes remain as compatibility storage. Desktop hydration turns legacy child sessions into singleton legacy Runs and hides them from normal navigation.

## Plan mode versus agent mode

| Mode | Use it when |
|---|---|
| Plain chat | You need drafting, comparison, or a short answer without saved behavior. |
| Named agent | The same identity, prompt, tool mode, or skill mode should be reusable across thread models. |
| Plan mode | You want read-only inspection before risky file or shell work. |
| Goal run | You want the thread to continue toward explicit success criteria across turns. |
| Schedule | The same prompt and saved file context should run on a clock without manually opening a thread. |

## Agent API

```bash Run an ad-hoc agent
curl http://127.0.0.1:7377/agents/run \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4.1",
    "stream": true,
    "agent_max_iterations": 100,
    "tool_approval_policy": "guarded",
    "sandbox_enabled": true,
    "messages": [{"role": "user", "content": "Run tests and summarize failures."}]
  }'
```

```bash Create a schedule
curl http://127.0.0.1:7377/schedules \
  -H "Content-Type: application/json" \
  -d '{
    "name": "weekday check",
    "cron": "0 0 9 * * Mon-Fri",
    "model": "gpt-4.1",
    "prompt": "Summarize project status.",
    "agent_id": null,
    "attachments": [
      {
        "id": "notes",
        "name": "notes.md",
        "mime": "text/markdown",
        "size": 18,
        "content": "# Notes\nShip docs."
      }
    ]
  }'
```
