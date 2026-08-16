---
id: agents
path: agents
label: Agents
title: Agents, tools, skills, and schedules
summary: Reusable Agent profiles, Worker Runs, tool modes, skills, schedules, and approval policies.
group: Core
order: 50
updated: 2026-08-15
---

Agents are for repeatable behavior, tool access, and longer work. Keep one-off questions in plain chat; save an agent when the same instructions or tool policy should survive across threads.

## Agent building blocks

| Block | Behavior |
|---|---|
| Named Agents | Model-agnostic profiles with name, description, deterministic avatar seed, system prompt, tool mode, and skill mode. **Start chat** creates a normal thread bound to the Agent while model choice remains thread-owned. The generated avatar follows the Agent through persona, schedule, and assigned Worker surfaces; unassigned Workers receive deterministic run-local identities. An Agent is a saved role; a Worker is one live instance of that role. |
| Tool modes | `all`, `custom`, or `none`. |
| Skill modes | `auto`, `custom`, or `none`; auto selects enabled skills by keyword, while explicit `@Skill Name` and `/Skill Name` prompt tags inject matching enabled skills for that turn. |
| Run timeline | Start, token, reasoning, tool call, bounded tool result, memory, Worker Run, per-request usage deltas, final usage, and error events render as structured stream parts. Tool results are capped before timeline persistence and again for model replay. Worker events carry monotonic cursors and reload on demand. Runs stop at 100 model turns by default (`stopped_at_limit: true`), and stream-open failures are retried once before surfacing an error. |
| Schedules | Cron schedules capture an explicit model, creation workspace, prompt, files, and optional Agent. Legacy schedules with no model temporarily fall back to their Agent's deprecated saved model; editing persists that fallback. Missing both records a visible error. |
| Tool approval | The UI sends approval policy to the server-side agent loop and resolves exact one-shot Review requests inline. |
| MCP Apps | Negotiated MCP tools may attach a server-authored `ui://` view. The agent sees bounded fallback content while the transcript retains the full structured App result and descriptor. App-only tools stay out of the model catalog. |

## Approval modes

| Mode | Server behavior |
|---|---|
| `review` | Read-only tools run automatically. Every mutating, command, or unknown call pauses before execution and shows its exact arguments inline. Approve or Deny resolves only that invocation; Stop, disconnect, or restart cancels it. This is the default for new chats. |
| `guarded` | Only tools declaring a read-only effect are exposed. Writes, commands, schedules, computer/preview actions, memory writes, and unclassified MCP tools are withheld. |
| `open` | Host filesystem and shell tools run with unrestricted machine access; the selected folder is their working directory, not a sandbox boundary. Supported account runtimes receive their native full-access mode. Enabled-tool, computer-use, MCP, memory, skill, connector-input, and connector-authorization gates still apply. Switching to Open auto-approves ordinary pending and subsequent command, file-change, and permission requests. |

Milim-native uses the registry's effect metadata. Review and Guarded bind host filesystem tools to the selected workspace; Open removes that boundary. The separate **Docker sandbox** setting only enables the bounded `run_command` tool and does not constrain Open host tools. Codex keeps `on-request` approval and relays app-server command, file, and permission requests: Review uses a workspace-write sandbox after approval, while Open uses Codex `danger-full-access` and auto-approves ordinary requests. Claude uses a temporary per-run Streamable HTTP MCP permission tool and deletes its run token/configuration on completion. A runtime that cannot support its approval protocol fails Review instead of silently switching modes. API callers may still set `tool_approval_grant: true` as an explicit whole-run compatibility grant; streamed desktop runs do not.

Each turn also reloads workspace instructions. Milim-native receives both AGENTS and Claude families. Codex relies on its native AGENTS discovery and receives Claude-family additions; Claude relies on native Claude discovery and receives AGENTS-family additions. Conditional Claude rules with `paths:` frontmatter are reported but not globally applied by Milim.

Approval is not just UI decoration. The server rebuilds the effective tool registry per run and removes tools that are not allowed by the current policy.

Approval controls execution, not a virtual patch queue. After an approved consequential call runs, the latest response's changed-files card inspects the resulting repository diff. Review failures retain **Retry** and **Open Git**, and **Undo** restores the pre-turn checkpoint.

The same policy is rechecked for calls made by an inline MCP App. Review approval is valid only for the exact displayed call; Guarded accepts only a tool whose MCP annotations declare it read-only; Open accepts eligible app-visible tools. An App can call only tools from its fixed originating server, so one server's view cannot use another server's private catalog.

## Agents, Workers, and Runs

The parent chat is canonical. Delegated work is stored as a Worker Run attached to one parent turn and never becomes a sidebar chat. A Run contains one to four independent tasks; each task creates a Worker. The model sees one `delegate_workers` operation rather than lifecycle tools for spawning, listing, reading, waiting, and stopping children.

At run acceptance Milim resolves a bound Agent exactly once and stores its complete immutable snapshot with the run. Worker proposals also freeze their assigned Agent snapshots before approval, so editing or deleting a profile cannot rewrite running work or an approved plan. Legacy nonterminal proposals without snapshots are stale and must be proposed again. If a thread's Agent is later deleted, history remains readable but new sends are blocked until the binding is cleared or replaced.

The read-only `list_agents` model tool returns Agent IDs, names, descriptions, avatars, and compact tool/skill capability summaries. It deliberately omits system prompts.

Each thread has a delegation policy:

| Policy | Behavior |
|---|---|
| `off` | Delegation is unavailable for that turn. |
| `ask` | The model may freeze an exact task plan. Milim pauses for **Run workers** or **Continue solo** before executing it. This is the default for existing and new threads. |
| `auto` | Independent managed workers run in parallel and their results are joined before the parent answers. Read-only account-runtime turns may instead report native worker activity through the same Run contract. |

The Worker model control uses the searchable model catalog and defaults to the parent chat model.

Desktop shows compact Worker avatars plus planned/active/done counts in the thread's Context card. That summary opens the Workers inspector, which groups the canonical parent chat's history into Active and Done, focuses transcript-linked Runs, keeps Active Workers and the selected Worker stable while progress arrives, renders Worker transcripts and result fallbacks as Markdown, and keeps delegation/model settings, Ask approval, live results, stopping, retry, Run deletion, and diff review together. Worker elapsed times use the canonical UTC start timestamp. Failed or stopped Workers can retry with the same model or a model chosen from the existing catalog; each retry is a new Run that preserves the failed attempt and its approved access/context until that terminal Run is explicitly deleted. Deletion requires a second confirmation and removes the Run, Worker transcripts, and events; proposed or running Runs must be stopped first. The Worker Run stream is the sole owner of managed Worker progress. Before resuming the parent, desktop reloads the canonical terminal Run, verifies every returned Worker is terminal, and requires successful or partial Runs to include every planned Worker. Results enter model context exactly once through a hidden synthesis message, and a persisted pending marker lets an approved join finish after desktop reload. If no Worker succeeded, the parent acknowledges the failures and continues the original request with delegation disabled. Running Workers keep their parent thread visibly active in the sidebar. On narrow layouts Worker history stacks above the selected detail, grows only to its content up to half the inspector height, and scrolls beyond that cap. In Ask mode the parent turn stops at the frozen proposal and resumes only after the user chooses **Run workers** or **Continue solo**. On wide layouts Context can remain open beside the inspector; proposed or running Runs reveal Workers automatically.

Delegation is intended for independent work that benefits from parallelism, not short or sequential steps. Managed Workers receive the current request, selected goal and instructions, workspace and branch, resolved Agent instructions and skills, supported attachments, and their assigned task. They do not receive the full transcript.

Workers are limited to four per Run and sixteen process-wide. Managed Workers have a five-minute deadline; Milim stops unfinished work and preserves available results and visible failures. Stopping the parent stops its active Run, and restart recovery marks unfinished Runs as errors so stale running states are never shown.

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
