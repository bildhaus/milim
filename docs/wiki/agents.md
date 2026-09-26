---
id: agents
path: agents
label: Agents
title: Agents, tools, skills, and schedules
summary: Reusable Agent profiles, Worker Runs, tool modes, skills, schedules, and approval policies.
group: Core
order: 50
updated: 2026-08-17
---

Agents are for repeatable behavior, tool access, and longer work. Keep one-off questions in plain chat; save an agent when the same instructions or tool policy should survive across threads.

## Agent building blocks

| Block | Behavior |
|---|---|
| Named Agents | Model-agnostic profiles with name, description, deterministic avatar seed, system prompt, tool mode, and skill mode. **Start chat** creates a normal thread bound to the Agent while model choice remains thread-owned. The generated avatar follows the Agent through desktop persona, schedule, and assigned Worker surfaces plus the native mobile composer and Agent sheet; unassigned Workers receive deterministic run-local identities. An Agent is a saved role; a Worker is one live instance of that role. |
| Tool modes | `all`, `custom`, or `none`. |
| Skill modes | `auto`, `custom`, or `none`; auto selects enabled skills by keyword, while explicit `@Skill Name` and `/Skill Name` prompt tags inject matching enabled skills for that turn. |
| Run timeline | Start, token, reasoning, tool call, bounded tool result, memory, Worker Run, per-request usage deltas, final usage, and error events render as structured stream parts. Tool results are capped before timeline persistence and again for model replay (see [Agent loop behavior](#agent-loop-behavior)). Worker events carry monotonic cursors and reload on demand. Runs stop at 100 model turns by default (`stopped_at_limit: true`). |
| Schedules | Cron schedules capture an explicit model, creation workspace, prompt, files, and optional Agent. Each occurrence is a normal canonical thread with a durable schedule origin, complete run ledger, and desktop/mobile visibility. Retrying the same occurrence is idempotent. Legacy schedules with no model temporarily fall back to their Agent's deprecated saved model; editing persists that fallback. Missing both records a visible error. |
| Tool approval | The UI sends approval policy to the server-side agent loop and resolves exact one-shot Review requests inline. |
| MCP Apps | Negotiated MCP tools may attach a server-authored `ui://` view. The agent sees bounded fallback content while the transcript retains the full structured App result and descriptor. App-only tools stay out of the model catalog. |

## Agent loop behavior

These rules apply to milim-native provider runs; account runtimes use their own loops.

| Concern | Behavior |
|---|---|
| Limits | A run stops at its step limit (100 model turns by default), its optional time or spend limit, or the user's Stop. Limits are checked between model and tool boundaries, and a paused run can be continued. |
| Provider retries | Rate limits (429), overload (529), server errors (5xx), request timeouts or conflicts (408/409), and connection failures are retried up to 4 times with exponential backoff and jitter (at most 30 s per wait); a provider `Retry-After` is honored up to 60 s. Other 4xx errors such as auth, quota, bad requests, and context overflow fail at once. A stream that breaks before any tool call has started is discarded and retried under the same policy. Each retry emits a `provider_retry` event with the attempt, delay, and reason. Backoff never outlasts the run time limit, and Stop cancels it. |
| Tool arguments | Arguments that are not valid JSON, miss required properties, or give a top-level property the wrong type are answered with an error the model can act on, and the tool does not run. Empty arguments count as `{}`. An unknown tool name returns the list of available tools. When the response was cut off at the output token limit, the error says so and asks for smaller calls. |
| Cut-off responses | A response cut off at the output token limit (`length`, including Anthropic `max_tokens`) does not end the run: milim adds a short note asking the model to continue with smaller tool calls and takes another step, at most twice in a row. |
| Tool output | The model receives a tool's plain-text projection verbatim when it has one. Otherwise a string result is sent raw, and an object's multi-line string fields are lifted out of the JSON into `--- field ---` sections with real newlines. Output over 2,000 lines or 50 KiB keeps its head and tail around a `[… N lines / M bytes omitted …]` marker. The full text is saved under `~/.milim/tool-output/<run id>/<call id>.txt`, and the model is told the path so it can page through it with `read_file` offset/limit or `grep`. Saved output older than seven days is pruned at startup. The transcript keeps the structured result. |
| Context window | When the model's context window is known, milim estimates the prompt before each step. Above 60% of the window, tool results older than the six most recent are replaced with a short stub; results for the latest turn are never touched and every tool call keeps its result. Above 85%, one request to the same model summarizes the older conversation, keeping system messages, the original task, and the last four turns verbatim. A `context_compacted` event reports what changed, and the request committed to the run ledger is exactly the one sent. |
| Approvals | An interactive approval waits up to 60 minutes. On expiry the call is denied, the model sees "approval request timed out", and the stored approval is marked expired. |
| Timing | Each model step records a `model_timing` run event (start time, time to first token, duration, attempts, finish reason) and each executed tool a `tool_timing` event (duration and whether it failed). |

## Approval modes

| Mode | Server behavior |
|---|---|
| `review` | Read-only tools run automatically. Every mutating, command, or unknown call pauses before execution and shows its exact arguments inline. Approve or Deny resolves only that invocation; Stop, disconnect, or restart cancels it, and a request left unanswered for 60 minutes is denied with "approval request timed out" and stored as expired. This is the default for new chats. |
| `guarded` | Only tools declaring a read-only effect are exposed. Writes, commands, schedules, computer/preview actions, memory writes, and unclassified MCP tools are withheld. |
| `open` | Host filesystem and shell tools run with unrestricted machine access; the selected folder is their working directory, not a sandbox boundary. Supported account runtimes receive their native full-access mode. Enabled-tool, computer-use, MCP, memory, skill, connector-input, and connector-authorization gates still apply. Switching to Open auto-approves ordinary pending and subsequent command, file-change, and permission requests. |

milim-native uses the registry's effect metadata. Review and Guarded bind host filesystem tools to the selected workspace; Open removes that boundary. The separate **Docker sandbox** setting only enables the bounded `run_command` tool and does not constrain Open host tools. Codex keeps `on-request` approval and relays app-server command, file, and permission requests: Review uses a workspace-write sandbox after approval, while Open uses Codex `danger-full-access` and auto-approves ordinary requests. Claude uses a temporary per-run Streamable HTTP MCP permission tool and deletes its run token/configuration on completion. A runtime that cannot support its approval protocol fails Review instead of silently switching modes. API callers may still set `tool_approval_grant: true` as an explicit whole-run compatibility grant; streamed desktop runs do not.

Each turn also reloads workspace instructions. milim-native receives both AGENTS and Claude families. Codex relies on its native AGENTS discovery and receives Claude-family additions; Claude relies on native Claude discovery and receives AGENTS-family additions. Conditional Claude rules with `paths:` frontmatter are reported but not globally applied by milim.

Approval is not just UI decoration. The server rebuilds the effective tool registry per run and removes tools that are not allowed by the current policy.

Approval controls execution, not a virtual patch queue. After an approved consequential call runs, the latest response's changed-files card inspects the resulting repository diff. Review failures retain **Retry** and **Open Git**, and **Undo** restores the pre-turn checkpoint.

The same policy is rechecked for calls made by an inline MCP App. Review approval is valid only for the exact displayed call; Guarded accepts only a tool whose MCP annotations declare it read-only; Open accepts eligible app-visible tools. An App can call only tools from its fixed originating server, so one server's view cannot use another server's private catalog.

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

Delegation is intended for independent work that benefits from parallelism, not short or sequential steps. Managed Workers receive the current request, selected goal and instructions, workspace and branch, resolved Agent instructions and skills, supported attachments, and their assigned task. They do not receive the full transcript.

Workers are limited to four per Run and sixteen process-wide. Managed Workers have a five-minute deadline; milim stops unfinished work and preserves available results and visible failures. Stopping the parent stops its active Run, and restart recovery marks unfinished Runs as errors so stale running states are never shown.

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
