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
| Skill modes | `auto`, `custom`, or `none`. Auto offers every enabled user and project skill, Custom only the Agent's allowlisted user skills, and None no index or skill tools. Explicit `@Skill Name` and `/Skill Name` prompt tags load a matching enabled skill in full for that turn, limited to the allowlist in Custom. See [Skills](#skills). |
| Run timeline | Start, token, reasoning, tool call, bounded tool result, memory, Worker Run, per-request usage deltas, final usage, and error events render as structured stream parts. Tool results are capped before timeline persistence and again for model replay. Worker events carry monotonic cursors and reload on demand. Runs stop at 100 model turns by default (`stopped_at_limit: true`), and stream-open failures are retried once before surfacing an error. |
| Schedules | Cron schedules capture an explicit model, creation workspace, prompt, files, and optional Agent. Each occurrence is a normal canonical thread with a durable schedule origin, complete run ledger, and desktop/mobile visibility. Retrying the same occurrence is idempotent. Legacy schedules with no model temporarily fall back to their Agent's deprecated saved model; editing persists that fallback. Missing both records a visible error. |
| Tool approval | The UI sends approval policy to the server-side agent loop and resolves exact one-shot Review requests inline. |
| MCP Apps | Negotiated MCP tools may attach a server-authored `ui://` view. The agent sees bounded fallback content while the transcript retains the full structured App result and descriptor. App-only tools stay out of the model catalog. |

## Approval modes

| Mode | Server behavior |
|---|---|
| `review` | Read-only tools run automatically. Every mutating, command, or unknown call pauses before execution and shows its exact arguments inline. Approve or Deny resolves only that invocation; Stop, disconnect, or restart cancels it. This is the default for new chats. |
| `guarded` | Only tools declaring a read-only effect are exposed. Writes, commands, schedules, computer/preview actions, memory writes, and unclassified MCP tools are withheld. |
| `open` | Host filesystem and shell tools run with unrestricted machine access; the selected folder is their working directory, not a sandbox boundary. Supported account runtimes receive their native full-access mode. Enabled-tool, computer-use, MCP, memory, skill, connector-input, and connector-authorization gates still apply. Switching to Open auto-approves ordinary pending and subsequent command, file-change, and permission requests. |

milim-native uses the registry's effect metadata. Review and Guarded bind host filesystem tools to the selected workspace; Open removes that boundary. The separate **Docker sandbox** setting only enables the bounded `run_command` tool and does not constrain Open host tools. Codex keeps `on-request` approval and relays app-server command, file, and permission requests: Review uses a workspace-write sandbox after approval, while Open uses Codex `danger-full-access` and auto-approves ordinary requests. Claude uses a temporary per-run Streamable HTTP MCP permission tool and deletes its run token/configuration on completion. A runtime that cannot support its approval protocol fails Review instead of silently switching modes. API callers may still set `tool_approval_grant: true` as an explicit whole-run compatibility grant; streamed desktop runs do not.

Each turn also reloads workspace instructions. milim-native receives both AGENTS and Claude families. Codex relies on its native AGENTS discovery and receives Claude-family additions; Claude relies on native Claude discovery and receives AGENTS-family additions. Conditional Claude rules with `paths:` frontmatter are reported but not globally applied by milim.

## Base prompt and environment

Every milim-native tool-agent run gets two server-built system messages, whether it starts from desktop, mobile, a schedule, or the `/agents/run` API. Account runtimes (Codex, Claude, OpenCode, Pi) keep their own harness prompts, and plain chat or a run whose policy leaves no tools gets neither.

| Message | Contents |
|---|---|
| Base prompt | Placed first. Identifies milim's coding agent and covers working style (understand before changing, minimal consistent edits, verify with the project's tests or build, report outcomes honestly), tool use, safety (no destructive commands without clear intent, respect approvals, never exfiltrate secrets), and output (brief, `path:line` references). The tool section is built from the run's final registry and mentions only tools and parameters that exist, such as `glob`/`grep`, `read_file` ranges, `edit_file` versus `write_file`, parallel read-only calls, `shell` timeouts and background processes, `todo_write`, `web_search`/`http_fetch`, `delegate_workers`, and `load_skill`. In Plan mode it adds the read-only planning rules. It states that custom, Agent, and repository instructions, which follow it, take precedence. |
| Environment | Placed at the end of the leading system messages. OS and architecture, shell dialect (`sh` or PowerShell), today's date with the local timezone, the absolute workspace root or "none", whether it is a Git repository with its current branch, up to 20 `git status --porcelain` lines with the total count, the five most recent commits, and the model id. Git commands run with a 3-second timeout and no optional locks. |

Both are computed once when the run starts and stay byte-identical across its steps, so provider prompt caching keeps working within a run.

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

Workers are limited to four per Run and sixteen process-wide. Managed Workers have a five-minute deadline; milim stops unfinished work and preserves available results and visible failures. `delegate_workers` itself is allowed 30 seconds past that deadline, instead of the default two-minute tool deadline, so its cleanup always runs; `linked_thread_wait` likewise gets its requested wait plus 30 seconds. Stopping the parent stops its active Run, and restart recovery marks unfinished Runs as errors so stale running states are never shown.

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
