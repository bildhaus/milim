---
id: desktop
path: desktop
label: Desktop app
title: Desktop app
summary: Unified threads, projects, composer controls, artifacts, plan mode, goals, search, rendering, settings, and slash commands.
group: Core
order: 30
updated: 2026-07-23
---

Milim is your local control plane for coding agents: use your own models and subscriptions, keep one canonical thread, review the diff, and ship.

The Providers sheet shows Codex, OpenCode, Claude, and Pi as separate installed account-runtime cards. Pi reports its configured provider and model counts with refresh and `/login` guidance. Cards use a two-column grid: four runtimes render 2×2, while a final odd card spans both columns.

Git-backed New Chat flows keep the current checkout by default and offer a separate isolated worktree. Isolated threads execute from `~/.milim/runtime/threads/<thread-id>` on a retained `milim/thread-<short-id>` branch while continuing to group under the original project. Uncommitted changes in the original checkout are excluded. Archive preserves the worktree; deletion removes a clean worktree or asks before force-discarding dirty state, and never deletes its branch.

The Code inspector can browse the effective workspace on demand, rejects traversal and symlink/junction escapes, and reviews UTF-8 files up to 2 MiB. Workspace and unified-diff lines accept one-line or Shift-selected range comments; comments remain editable in the composer tray and are sent as bounded structured context. The Git inspector opens the current branch PR or creates a GitHub draft after commit and publish prerequisites are met. Ready DOM previews expose an annotation crosshair that captures bounded element metadata without screenshots or complete DOM persistence.

Milim has one model-agnostic workbench. Project, Agent, MCP, media, memory, sandbox, schedule, and computer-use features are always discoverable, with advanced tools collapsed until needed. The sidebar Tools launcher opens MCP servers, Skills, Schedules, and the media manager from one persistent place. The Tauri process starts the embedded backend, connects persisted MCP servers, and runs schedules in the background. The model picker loads cached models while one startup task refreshes enabled chat providers, then reconciles automatically; Codex and Claude discovery remain independent from provider availability. Background schedule completions and mobile relay events use shared app notices so they remain visible outside their settings panels. Closing the desktop window hides it to the system tray so those background services keep running; use the tray menu to reopen or quit.

## Interface modes

| Mode | Visible surface |
|---|---|
| Unified workbench | Chat plus workspace/Git, Agents, skills, MCP, schedules with attached file context and visible result threads, media, sandbox, computer use, and Personal/Project memory. |

## App map

| Area | Role |
|---|---|
| Top bar | Theme, update, provider, and global app controls. |
| Sidebar | Projects, canonical parent threads, pinned groups, archives, unread state, compact thread spacing, animated section collapse, five-at-a-time per-section ellipsis toggles, quick switching, and the Tools launcher for MCP servers, Skills, and Schedules. Thread hover actions are limited to pin and archive; branch and export remain in the right-click menu. Project hover actions are pin and new chat, with archive in the right-click menu. Workers never become sidebar rows. |
| Thread header | The model chip is the sole model selector. The Session chip names only non-default states; its menu uses explicit Privacy and Approval choices instead of cycling. |
| Composer | Prompt text, visually highlighted plain-text skill/MCP/file/link tokens, persisted per-thread unsent drafts, configurable current-chat/across-chat/off prompt recall, slash commands, Automatic/Manual/Off suggestions with independently gated command/file/skill/MCP sources, immediate Plan/Goal activation, file attachments, an interruptible and reorderable queued-send tray, and Smart/Pinned/Hidden quick actions that prefill without submitting. Opt-in local-first ghost completion accepts with Tab and dismisses with Escape. |
| Run timeline | Reasoning, compact live tool activity with expandable details, structured Codex permission/MCP approval cards, nonterminal runtime notices, workspace checkpoint notices, memory notices, one compact Worker Run event, and usage metrics. Built-in tool-agent usage updates after each model request completes; account runtimes remain terminal-only unless their CLIs report more. |
| Error fallback | Root UI render crashes and unexpected embedded-server exits show a restartable recovery screen with direct access to bounded local logs instead of a blank app window. |
| Context card | Persisted per thread and grouped into a compact Workers summary plus independently collapsible Environment, Task, live Activity, Context, and Sources sections. The current prompt estimate opens a collapsed breakdown of non-zero context categories, with repository-rule files nested beneath Repository rules; cumulative provider usage remains separate. Sources shows five filename-only, single-line entries by default with a separate more/show less toggle, and each entry opens its attachment, artifact, or memory target. Workers shows avatars and planned/active/done counts, then opens the full Active/Done history in the Workers inspector. The floating card reserves a 300px layout slot that pushes the transcript while its scrollbar stays at the thread view's right edge, can coexist with the inspector on wide layouts, and yields to the inspector below the dual-panel threshold. Live Codex quota is fetched only while the card is open; the adjacent launcher appears only for folder-backed threads and opens the active folder in installed local tools. |
| Inspector | One Preview / Code / Git / Workers surface for inspecting work products and browsing the parent chat's Active/Done Worker history. Context can remain visible beside it on wide layouts. Horizontal resizing first preserves the transcript minimum; continuing 32px past that limit collapses the sidebar to its rail, and continuing 32px past the expanded docked limit overlays the inspector across the transcript and Context without resetting either. Reversing the same drag restores the prior stage. Overlay is temporary, while the released width and collapsed sidebar preference persist. |

Motion follows interaction frequency. Search, model selection, composer menus, context menus, thread/tab switching, and first-send dock placement are instant. Sidebar, Context, and inspector surfaces use short interruptible transitions; sidebar and queued-message reorder sources follow the pointer directly until the discrete drop. With reduced motion enabled, spatial movement, overshoot, preview gesture illustrations, and spinning loaders stop while static or opacity-only status remains visible.

Messages sent while a response is running enter a compact per-thread queue above the composer. Drag the handle or focus it and press Up/Down to reorder pending messages. **Interrupt** stops the current response, preserves its partial output, and runs the selected row next; **Run** does the same prioritization when the thread is idle. Remaining messages continue in their reordered sequence, while failed or stopped queued runs leave later messages pending. Delete stays inline and Edit is available from the row's overflow menu.

## Context menus

The desktop app replaces the default right-click menu on Milim-owned surfaces with app actions for threads, project sections, chat messages, artifacts, preview panels, Git panels, and empty app chrome. Text inputs, textareas, selected text, and links keep the native browser menu for edit, copy, paste, select-all, and link actions. External pages inside the native preview child webview keep their own native context menu; the Milim preview toolbar and panel chrome use the app menu.

The Milim logo in the existing title bar opens a compact app menu for new chat, composer focus, sidebar visibility, settings, documentation, diagnostics, and a flush-safe quit. It supports first-item focus, arrow/Home/End navigation, Escape dismissal, and configured shortcut labels without adding a second title-bar row. On macOS, the system menu bar also exposes native Milim, File, Edit, View, Window, and Help menus; standard text editing stays native while custom actions route into the same desktop commands.

User and assistant message bodies render Markdown in the transcript, and message bodies plus the composer use automatic text direction so Hebrew and other RTL text read naturally. While an assistant response is still streaming, the live answer tail renders memoized Markdown blocks with expensive final-pass features disabled, falls back to preserved text for long streams, and switches to full Markdown when the turn finalizes. Long transcripts virtualize offscreen rows so old messages stay in history without keeping every row mounted. Message actions, edit/resend, search, model context, and thread export keep using the raw stored source.

The composer keeps prompt storage plain text. Recognized `@Skill Name`, `/Skill Name`, and `/server__tool` tags render as compact pills, while workspace `@file` references and bare HTTP(S) URLs render with link-like highlighting in the editor mirror layer; MCP slash suggestions insert the visible tag only and do not force a tool call or bypass approval/exposure policy.

PNG, JPEG, WebP, and GIF attachments up to 2 MB each are preserved as real image content for provider chat, server-side agent runs, Codex app-server, Claude CLI, Pi RPC, and provider-backed schedules. Browser and native attachment paths use the same validation and reject unsupported, oversized, empty, or unreadable images before attaching them. Each outbound turn keeps the latest images and then complete recent image-bearing turns newest-first within a 20 MiB encoded-image budget. Older pixels stay visible in the local transcript but are replaced by an omission note in model context; a final 30 MiB desktop body check reports removal/compaction guidance before the embedded server can return 413. Codex receives temporary `localImage` inputs; Claude and Pi receive native base64 image blocks; none of these paths uses OCR. Desktop files come from the native picker or workspace-relative `@file` suggestions under the selected folder.

Schedules accept the same stored `dataUrl` pixels and build a multimodal user message when they fire. A legacy scheduled image without stored pixel data records a visible reattachment error. Background schedules list and accept provider/local API models only; account runtimes remain interactive because their approval and session semantics are not safe for unattended runs.

## Session controls

Hot Swap makes the Milim thread canonical rather than any provider session. Selecting a compatible chat model changes the next turn immediately without resetting workspace, memory, artifacts, previews, approvals, goals, or queued messages. A preflight appears only when the target needs context compaction, cannot receive current image/tool context, is unavailable, or has a native Codex/Claude session behind the Milim transcript.

For a stale native session, **Fresh** starts from Milim's complete context while **Resume** keeps the native session and injects turns added since it last completed successfully. The latest assistant response offers **Continue with**, **Review with**, and **Retry with**. Continue and Review prepare editable drafts; Review is read-only for that turn only. Coding retries run from the pre-turn checkpoint in an isolated Git worktree and expose their diff in the Git inspector before it can be applied to the original workspace.

The Codex account card includes **Recover chats**. Search and page through active or archived Codex history, then import one visible user/assistant transcript as a new Milim chat. Recovery attaches the original native thread and sync cursor but does not continuously merge histories; existing imports open their Milim chat instead of duplicating it. Choose a Codex model before continuing a recovered thread.

The Git inspector is a review-first workspace with a compact repository header for branch, status, fetch, commit, sync, and agent review actions. Its edge-to-edge body places a folder-structured changed-file navigator on the left and the local unified diff on the right. The scope selector immediately switches between all changes, unstaged changes, staged changes, the latest turn checkpoint, a recent commit, or another local branch without contacting GitHub. Selecting a file loads the current scope when needed, expands its section, and scrolls it into view; the toolbar can expand or collapse every rendered section, show or hide the navigator, the divider resizes it, and narrow panels stack it above the diff.

The latest code-changing turn ends with a compact changed-files card. **Review changes** opens that checkpoint diff in the Git inspector, while **Undo** restores the checkpoint, removes that assistant response, retains the user request, and clears stale account-runtime state.

| Control | Behavior |
|---|---|
| Model | Pick any discovered chat, account-runtime, or media-capable model. The chip shows provider, runtime lane, setup status, capabilities, favorite state, and reasoning effort where supported. |
| Folder | Sets the host working folder. Each run captures that folder immutably; filesystem tools reject symlink/junction escapes, writes replace files atomically, directory results are sorted and bounded, and `read_file` accepts byte `offset`/`limit` ranges. |
| Sandbox | Enables bounded Docker execution through `run_command`: no network, read-only root, dropped capabilities, no-new-privileges, memory/CPU/PID limits, output caps, timeout, and cancellation cleanup. |
| Computer use | Enables OS-level screen capture plus mouse/keyboard tools when the desktop build includes the feature. |
| Memory | Adds scoped thread/project memory search as cheap turn context. Durable memory writes use `memory_register` only on explicit remember/save requests or already tool-capable turns. |
| Privacy | Sets `off`, `redact`, or `block` for remote-provider and account-runtime traffic. |
| Approval | Sets `review`, `guarded`, or `open` tool execution policy. The session control shows the current mode and explains the selected policy; changing the workspace folder resets approval to `guarded`, so `open` must be enabled explicitly for the new project boundary. |
| Plan | Keeps the turn read-only until you approve execution. |
| Goal | Tracks a thread objective, success criteria, constraints, turn count, and continuation prompts. |

## Artifacts

Named artifacts from later assistant messages become selectable revisions of the same logical file or title. Revisions are immutable snapshots from chat history; preview, copy, download, diff review, apply, and save actions use the selected revision. Threads without a selected folder save/apply named artifacts into a persisted per-thread virtual project; threads with a folder write to disk. Extracted code artifacts collapse to compact source rows in the transcript; open the side panel or review a workspace diff to inspect the full source. Inline artifacts, including anonymous code fences, markdown tables, standalone JSON, and standalone CSV, remain display/export content and do not get workspace target paths, batch apply, or save controls. Saved files record the source app session, message turn, and artifact revision when available.

## MCP Apps

Milim-native provider and local-model chats can manage the same MCP server registry as the MCP Servers sheet through `mcp_server_list`, `mcp_server_test`, `mcp_server_save`, and `mcp_server_delete`. Guarded exposes only the read-only list; Review shows the exact configuration on the normal one-shot approval card before testing, saving, launching, or deleting; Open performs eligible calls immediately. Codex and Claude CLI account runtimes do not receive these Milim-native management tools.

Chat setup accepts ordinary environment values separately from value-free secret placeholders. Credential-looking keys cannot carry values through model tool arguments; add those values afterward in the MCP Servers sheet, where the existing encrypted secret store handles them. Saving with an existing id fully replaces that server configuration while preserving encrypted values for secret placeholders that remain. A successful connection updates the shared hub immediately, and its advertised tools appear when the next chat turn captures a fresh tool registry.

MCP Apps are server-authored live views, not artifacts. When a connected stdio MCP server and Milim both negotiate `io.modelcontextprotocol/ui`, a tool may attach an advertised `ui://` resource. Milim inserts that chart, diagram, form, dashboard, or viewer at the exact tool-call position and keeps it out of completed-tool groups and the artifact panel.

Each view runs inline in a dedicated opaque-origin iframe with scripts and forms but no same-origin, navigation, popup, download, or device privileges. After validation, Milim exposes the HTML through a random, short-lived, memory-only capability URL so the app document can receive its own CSP without weakening the Tauri shell's CSP. Network and static-resource access default to denied and are opened only for valid origins in the resource's `_meta.ui.csp`. The view receives no Milim bearer token or general backend access; it can request supported operations only through the host's exact-window AppBridge handlers. Tool calls remain fixed to the originating MCP server and require `app` visibility; Review shows the exact tool and arguments for one-call approval, Guarded permits annotated read-only calls, and Open permits eligible app-visible calls immediately.

Milim persists the view descriptor and initial tool result, but never persists the remote HTML. Reopening a thread re-reads the resource from its MCP server. If the server is disconnected or the resource fails URI, MIME, CSP, or size validation, the normal textual result remains visible with a retry state. The first release is desktop-inline only for Milim-managed stdio servers in provider/local-model agent runs; mobile and native Codex/Claude events keep text fallback.

HTTP and HTTPS links rendered in chat message bodies open in the system browser. The inspector's native URL source accepts public HTTPS and loopback HTTP (`localhost`, `127.0.0.1`, or `[::1]`) pages. It reuses one incognito child webview for the active App or URL source, reflects real navigations and redirects in the address bar, and enables Back, Forward, reload, and scoped preview tools only after the child webview reports page-load readiness. Generated artifacts preview separately through sandboxed iframe/srcDoc rendering, with anonymous script and TSX artifacts compiled through a lightweight standalone fallback when no filename is provided. Artifact selection, historical revision, App state, and manual URL/history are independent per thread; manual URL history stays memory-only so query strings are not persisted. Runtime updates can make an App source available but do not replace an explicitly selected URL or historical revision.

Runnable generated apps never start automatically. Choose App, select **Review run**, and inspect the scope, working folder, package manager, install requirement, exact install and dev commands, and source fingerprint. **Run** is enabled only for that current preflight. Once healthy, the review collapses into a Ready status control and one-click Stop action; select Ready to reopen the details and Restart action. No-folder projects are atomically staged into Milim's managed runtime directory only after Run; selected-folder previews execute in that folder and warn when dependency installation may modify it. Stop covers installation and the dev server, and restart cannot let an older run overwrite newer state. The runtime probes its loopback URL rather than trusting console text, distinguishes active-but-unhealthy from stopped, preserves the last URL through compile failures and polling disconnects, and returns to ready after recovery. **Prepare fix** adds an editable queued message containing the selected revision and recent failure evidence; it does not send, replace the composer draft, or remove attachments.

Milim registers one active preview surface across artifact iframes, native URL previews, and managed runtime previews, and only enables scoped tools when that surface is ready and DOM-capable: `preview_dom_snapshot`, `preview_click`, `preview_type_text`, `preview_key_press`, and `preview_scroll`. A run captures the target surface before execution, so switching threads or previews cannot redirect an in-flight action. Preview callbacks use bounded asynchronous waits. These tools remain separate from opt-in OS-level `/computer` control. Computer-use arguments are range/enum validated and stored screenshots retain only the newest 50 captures.

## Workspace checkpoints

Tool-enabled turns with a selected Git workspace create a Git worktree checkpoint before the assistant can write files or run commands. Assistant messages with a checkpoint expose a restore action that returns the workspace files to their pre-turn content without moving the current branch HEAD.

## Git side panel

The Git side panel appears only for selected folders that are Git repositories. Its branch selector lists local branches, can checkout another local branch, and can create a new branch from the current `HEAD`; checkout failures are reported from Git without changing the worktree.

## Plan mode

Plan mode injects a system instruction that allows read-only inspection and blocks edits, writes, shell commands, computer control, schedule creation, memory registration, and other mutations. The assistant returns a concrete implementation plan. The UI exposes an Execute plan action that sends the approved plan back as a normal run.

## Goals

Goals are thread-level autonomous runs, not saved agent profiles. A goal stores objective, success criteria, constraints, status, last reason, next prompt, turn count, timestamps, and an optional developer max-turn cap. After each visible turn, a controller asks the model whether to continue, complete, or stop as blocked.

## Instructions and compaction

| Feature | Behavior |
|---|---|
| Thread actions | Message rows can copy with temporary confirmation, branch a new thread from that point, delete a visible message, and edit assistant messages in place. Sidebar rows expose pin and archive on hover, with branch and export in the right-click menu. |
| Per-chat instructions | The composer can attach instructions to the active thread without changing global app defaults. |
| Context compaction | Long chats use tokenizer-backed visible checkpoint messages. Milim resolves saved instructions, repository rules, Plan/Goal, skills, memory, artifacts/schedules, attachments, and effective tool definitions first, reserves that fixed context, then compacts the exact post-checkpoint conversation into the remaining model budget. Fixed context that cannot fit blocks with a category breakdown instead of truncating rules. `/compact` creates a checkpoint manually, and auto-compaction uses the same path before long sends. Compaction keeps a bounded recent tail verbatim and caps old attachment/tool bodies in summary prompts. Codex and Claude chats with an existing native thread/session skip Milim auto-compaction and send only per-turn context plus the latest user message. Summary generation rejects truncated or oversized outputs instead of saving incomplete checkpoints. Each run persists its prompt estimate, threshold, free tokens, categories, repository sources, and warnings in Context; provider-reported multi-iteration usage is labeled separately as cumulative usage. |
| Search operators | Chat search accepts plain text plus `from:user`, `from:assistant`, `in:all`, and `is:archived` filters. |
| Auto thread titles | New chats get first-message titles by default. Optional AI names run after the first reply and need a compatible provider chat model when the chat uses Codex, Claude, or media models. |
| Onboarding | First-run setup follows Model, Defaults, optional Context, and Ready. It connects local/hosted/Codex model sources and can import Claude/Codex MCP servers and skills as disabled entries. MCP imports preserve `cwd`, non-secret env, and secret placeholders; use Test connection after filling required secrets. |
| Settings | Preferences contains General, Chat, and Appearance; Workflows contains Models & agents and Workspace; Data & devices contains Data and Mobile; Application contains Shortcuts, About, and Developer. Search replaces the section list with direct control results without changing sections until selection, then jumps, highlights, and focuses the row. Badges are limited to archive counts and actionable update/error states. General owns startup, panel restore, native notifications, window/layout, and update policy. Chat owns new-chat policy, quick actions, autocomplete, history, naming, and opt-in completion. Appearance includes the default-on local activity ridgeline above an empty chat's composer. Models & agents uses live catalogs for configured defaults and never falls back to an arbitrary remote model. Workspace owns remembered/detected launchers and current/ask/worktree project-chat policy. Data owns archives, export defaults, and backup/recovery. |
| Theme editor | Themes and custom style settings are persisted with the desktop state. Custom backgrounds support static images and animated GIFs. Custom palettes must pass core text contrast checks before saving, low-contrast preset/custom themes are marked in the theme grid, and custom theme cards expose an edit button. |
| Keyboard shortcuts | App-window shortcuts are configurable, including Open composer suggestions; `Ctrl/Cmd+K` opens a command palette that searches app commands and chats. Previous thread defaults to `Ctrl+Tab` on Windows and macOS, switches to the last viewed thread immediately, and shows a compact recent-thread switcher for repeated presses. Ctrl/Cmd `+` and `-` scale the UI and reveal transient percentage, step, and reset controls in the title bar. At an inspector limit, repeated `Arrow Left` collapses the sidebar and then enters overlay; `Arrow Right` returns to docked mode at its boundary. `Home` and `Enter` return to the docked minimum and default width. Sidebar and inspector dividers snap closed when dragged 96px past their minimum, reopen when that same drag reverses, and reset on double-click or Enter. |
| Account usage | An enabled-by-default App setting shows a compact quota pill for the active Codex or Claude account runtime in the title bar. Codex refreshes live while selected; Claude shows the latest quota event reported by its CLI. Full quota and reset details remain available in Context and in the title-bar tooltip. |
| Window close | Closing hides Milim to the system tray; minimize keeps normal taskbar behavior. |

Native notifications are off by default. Enabling Run finished or Needs attention requests OS permission at that moment; denial leaves the toggle off. Notifications stay generic unless Include thread title is enabled and are suppressed while Milim is focused by default. Windows acceptance must be verified from an installed build because development notifications use the development host identity.

Milim backups are versioned `.milim-backup.json` files. Export flushes deferred session writes and atomically writes chats, projects, drafts, archive state, settings, UI preferences, custom themes, quick actions, and bounded personalization metadata. Provider credentials, MCP secrets, paired-device tokens, memory databases, generated media, update packages, worktrees, and running jobs are excluded. Restore validates size/schema/shape, creates an atomic recovery snapshot in app data, replaces canonical keys and normalized session tables in one SQLite transaction, rolls back on failure, and restarts only after commit; restore never merges or duplicates imported state.

Composer completion defaults off. Local providers requires a loopback provider endpoint; Current provider model requires explicit privacy/cost confirmation when enabled and never runs through Codex, Claude, or OpenCode account runtimes. Requests begin after about 600 ms idle and 20 non-whitespace characters, send only current composer text with a small output cap, and cancel on typing, thread/model changes, submission, or unmount. Completion is suppressed during IME composition, media mode, active runs, and command/file menus, and ghost text is never persisted.

## Slash commands

| Command | Behavior |
|---|---|
| `/plan build feature X` | Turn on read-only planning and send the remaining text as the prompt. Selecting `/plan` from autocomplete activates its pill immediately and leaves the composer ready for the prompt. |
| `/goal build feature X` | Start a goal with the remaining text as its objective. Selecting `/goal` from autocomplete activates a removable Goal pill, and the next prompt becomes the objective. |
| `/model llama3.2` | Set the thread model. |
| `/folder C:\project` | Set the thread working folder. |
| `/sandbox` and `/nosandbox` | Enable or disable sandbox tools. |
| `/computer` and `/nocomputer` | Enable or disable computer-use tools. |
| `/memory` and `/nomemory` | Enable or disable scoped memory. |
| `/privacy redact` | Set the outbound privacy gate to `off`, `redact`, or `block`. |
| `/approval guarded` | Set tool approval to `review`, `guarded`, or `open`. |

Calling `/privacy` or `/approval` without one of those valid arguments shows usage help and leaves the current state unchanged.
| `/agent none` | Switch to a named agent by id/name, or clear the active agent. |
| `/compact` | Summarize prior context into a visible checkpoint and start future model calls from that summary. |
| `/export` | Download the active thread using the configured JSON/Markdown default. `/export json` and `/export md` override it. |
| `/import` | Pick a Milim JSON or Markdown thread export and import it as a new local thread. |
| `/clear` | Start a fresh chat through the shared inherited/configured new-chat policy. Computer Use, Plan Mode, goals, temporary instructions, and running state always reset. |
