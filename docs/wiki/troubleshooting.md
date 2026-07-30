---
id: troubleshooting
path: troubleshooting
label: Troubleshooting
title: Troubleshooting
summary: Fix missing models, setup errors, workspace tool refusal, sandbox failures, privacy blocks, MCP disconnects, computer use, and busy ports.
group: Reference
order: 120
updated: 2026-07-20
---

Start with the current base URL and selected model. Most local issues are either a missing provider, a workspace folder that was never selected, or an optional runtime that is not running yet.

Desktop diagnostics are local-only under `<MILIM_HOME>/logs` (normally `~/.milim/logs`). Milim retains `desktop.log` and one 5 MiB previous log, never uploads them automatically, and exposes the folder from **Settings → About → Diagnostics** and the recovery screen.

## Diagnostic order

| Question | Next check |
|---|---|
| Can the server answer `/health`? | If no, fix process startup, port, firewall, or desktop embedded-server state first. |
| Does `/v1/models` list the expected model? | If no, fix provider discovery, local runtime startup, or model id before testing chat. |
| Is a workspace folder selected? | If no, file, shell, artifact-save, and workspace tools should refuse to run. |
| Is privacy mode blocking the request? | If yes, switch to `redact`, remove the detected value, or use a local runtime. |
| Is the optional runtime installed? | Media, sandbox, computer use, Codex, and Claude each have separate setup. |

## Common symptoms

| Symptom | Fix |
|---|---|
| Models list is empty | Start Ollama or LM Studio, add a provider, or set `MILIM_REMOTE_BASE_URL` for CLI/server use. |
| Provider returns 401 | Replace the provider key or verify the account has access to the selected model. |
| Workspace tools are missing | Select a folder. Host filesystem, shell, and Git tools are removed until a workspace exists. |
| Guarded approval cannot run shell | Switch to Open approval or use the Docker sandbox when command execution is appropriate. |
| Sandbox run fails | Start Docker, check `MILIM_DOCKER_BIN`, and verify the daemon can run containers. |
| App preview will not Run | Open Preview → App, choose **Review commands**, confirm the folder, exact commands, and source fingerprint, then choose **Run**. Any artifact or project change invalidates the review. |
| App preview is active but unhealthy | The process is still running but the loopback readiness probe failed or the app has a compile error. Keep the URL, inspect the runtime logs, use **Prepare fix** to queue editable context, and wait for recovery or Stop. |
| Preview says disconnected or stale | Status polling failed. Milim keeps the last-known runtime and URL instead of clearing the inspector; confirm the embedded server is reachable, then retry or reopen the inspector. |
| URL preview controls stay disabled | Wait for the native child webview's real page-load-ready event. Only public HTTPS and loopback HTTP URLs are accepted; creation, navigation, and load errors appear in the inspector. |
| Computer use is unavailable | Build with the `computer-use` feature and enable the `/computer` gate. |
| MCP tools disappeared | Check `/mcp/servers` or the MCP Servers sheet. Imported servers stay disabled and secret-looking env values become required placeholders; fill them and use Test connection before enabling. |
| Privacy block error | The server detected PII before a remote send. Use Redact, Off, or a local runtime. |
| Desktop port is busy | The embedded server falls back to a free loopback port and the UI asks Tauri for the actual API base URL. |
| Desktop recovery screen appears | Open the local logs for the recorded failure, then restart Milim from the same screen. Saved chats and settings remain on device. |

## Next reading path

| If you are | Read |
|---|---|
| New user | Quickstart, Models, then Desktop app. |
| Daily operator | Agents, Memory, Privacy, then Media/mobile. |
| Integrator | API, Config, and Overview source links. |
| Release work | Release and verification, then Troubleshooting. |
