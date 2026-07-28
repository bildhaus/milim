---
id: quickstart
path: quickstart
label: Quickstart
title: Quickstart
summary: Connect any model runtime, optionally pick a workspace, keep one canonical thread, switch models inline, and review the result.
group: Start
order: 20
updated: 2026-07-28
---

Aim for the core loop first: connect a model source, optionally select a workspace, continue one canonical thread across models, and review proposed changes before accepting them. Switching models changes the next turn; it does not reset workspace context or conversation history.

## First run checklist

| Step | What to check |
|---|---|
| Install or run | Use a release build for normal use. Use `pnpm -C apps/desktop tauri:dev` only when working on the app. |
| Connect a runtime | The first panel detects Codex, Claude, OpenCode, Pi, Ollama, and LM Studio, and can connect a hosted provider. Select any reachable model. |
| Select a workspace | The folder is optional for chat. Pick one before asking for host file reads, shell commands, Git actions, or folder-backed previews. |
| Review setup | Confirm the selected model and optional workspace, then open Milim. Onboarding does not run or prefill a task. |
| Check the model chip | The chip shows provider, runtime lane, setup status, capabilities, favorite state, and reasoning effort where supported. |
| Send a useful prompt | Ask for a repo map, failing-test diagnosis, small docs edit, or test command. A generic hello only proves chat works. |
| Switch and continue | Pick another model from the same chip. Provider models use Milim tools when workspace/tool context is active; Codex, Claude, OpenCode, and Pi use account-runtime bridges. |
| Review the result | Check the selected model, tool timeline, workspace, and Git diff before accepting changes. |

## Desktop app

```powershell Run the desktop app
corepack enable
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

The desktop app embeds the server in-process. There is no separate `milim serve` process for normal desktop use.

On first run, follow Runtime, optional Workspace, and Ready. Memory, privacy, sandbox, computer use, imports, and other power tools remain available after setup through thread controls, Settings, and the collapsed sidebar Tools launcher.

## CLI server

```powershell Run the CLI server
cargo build --release
$env:MILIM_REMOTE_BASE_URL = "http://localhost:11434/v1"
cargo run -p milim-cli -- serve
cargo run -p milim-cli -- status
cargo run -p milim-cli -- models
```

```powershell OpenAI-compatible chat
curl http://127.0.0.1:7377/v1/chat/completions `
  -H "Content-Type: application/json" `
  -d '{"model":"llama3.2","messages":[{"role":"user","content":"hello"}],"stream":true}'
```

## CLI commands

| Command | Use |
|---|---|
| `serve [--port N] [--expose]` | Start the HTTP server. |
| `status [--url URL] [--port N] [--token T] [--json]` | Probe a running server. |
| `models [--url URL] [--port N] [--token T] [--json]` | List server models. |
| `run [--url URL] [--port N] [--token T] <model> [prompt...]` | One-shot chat or interactive REPL through a running server. |
| `keys identity` | Print this machine identity address. |
| `keys mint [--audience A] [--label L] [--expires-secs N]` | Mint an `msk-v1` access token. |
| `mcp [--url URL] [--port N] [--token T]` | Run a stdio MCP bridge to the local server. |
| `version` | Print the binary version. |

## Before a longer run

| Signal | Meaning |
|---|---|
| Models list is empty | The app is running, but no provider or local API runtime is configured yet. |
| Tools refuse the folder | The thread has no workspace folder. Use the folder control or `/folder C:\path\to\repo`. |
| Remote send is blocked | Privacy is set to `block` and the scanner detected PII or a secret-looking value. |
| Sandbox fails | Docker is not installed, not running, or cannot start the default container. |
| Account runtime is missing | Codex, Claude, OpenCode, or Pi must be installed and authenticated through its own tooling, then refreshed in Milim. |
