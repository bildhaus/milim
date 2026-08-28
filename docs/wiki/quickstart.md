---
id: quickstart
path: quickstart
label: Quickstart
title: Quickstart
summary: Connect any model runtime, optionally pick a workspace, keep one canonical thread, switch models inline, and review the result.
group: Start
order: 20
updated: 2026-08-28
---

Aim for one safe core loop: connect a runtime, optionally select a workspace, approve one small change, inspect the resulting diff, and continue the same thread with another model. Switching models changes the next turn; it does not reset workspace context or conversation history.

## First run

| Step | What to check |
|---|---|
| Install or run | Use a release build for normal use. Use `pnpm -C apps/desktop tauri:dev` only when working on the app. |
| Runtime | The searchable picker detects hosted providers, Ollama, LM Studio, Codex, Claude, OpenCode, and Pi. Cached provider models appear first; slower runtime results join as they arrive. Select any reachable model, then choose **Continue**. **Continue without a model** is available, but chat stays disabled until **Manage models** above the composer is used to connect one. |
| Workspace | A folder is optional for chat and required for repository work. Choose one or skip it, then select **Open Milim**. The composer receives focus and no task is sent automatically. |

Onboarding reaches the composer in two advances: Runtime → optional Workspace → app. Provider, privacy, approval, and workspace choices remain editable from the thread.

## Prove the safe change loop

1. Start an empty chat in a small Git-backed repository. Before the first send, confirm the summary above the composer shows the intended runtime destination, workspace, Privacy mode, and Approval mode. New chats default to Privacy **Off** and Approval **Review**.
2. Send a bounded request, for example: `Read README.md, correct one clear documentation error, and run the smallest relevant check.`
3. In Review, read-only inspection can proceed automatically. A command, file write, or other consequential call pauses before execution and shows its exact arguments. Approve or deny that call. Open mode can auto-approve ordinary eligible tool requests; connector input and authorization remain interactive.
4. After execution, inspect the changed-files card and select **Review changes** to open the resulting Git diff. This is the diff produced by the approved action, not a virtual patch waiting to be applied. If review is unavailable, use **Retry** or **Open Git**; use **Undo** to restore the pre-turn checkpoint.
5. On the latest completed answer, choose the permanently visible **Continue with…** action and select a different runtime. Edit the prepared continuation if useful, then send it in the same thread. **Review with…** and **Retry with…** remain in the adjacent menu.

The model chip shows the selected provider/runtime route, setup status, capabilities, favorite state, and reasoning effort where supported. Provider models use Milim tools when workspace or tool context is active; Codex, Claude, OpenCode, and Pi use their account-runtime bridges.

## Run the desktop app from source

```powershell Run the desktop app
corepack enable
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

The desktop app embeds the server in-process. There is no separate `milim serve` process for normal desktop use.

Memory, sandbox, computer use, imports, Agents, Workers, Skills, MCP, Schedules, Media, and Pull Requests remain available after setup through thread controls, Settings, and the collapsed sidebar **Tools** launcher. Standalone server setup and CLI commands live in the [API reference](api).

## Troubleshooting

| Signal | Meaning |
|---|---|
| Models list is empty | The app is running, but no provider or local API runtime is configured yet. |
| Tools refuse the folder | The thread has no workspace folder. Use the folder control or `/folder C:\path\to\repo`. |
| Remote send is blocked | Privacy is set to `block` and the scanner detected PII or a secret-looking value. |
| Sandbox fails | Docker is not installed, not running, or cannot start the default container. |
| Account runtime is missing | Codex, Claude, OpenCode, or Pi must be installed and authenticated through its own tooling, then refreshed in Milim. |
