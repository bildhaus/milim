---
id: overview
path:
label: Overview
title: milim docs wiki
summary: Start here for one canonical development thread, model hot-swapping, local control, and diff review.
group: Start
order: 10
updated: 2026-07-28
---

milim is a model-agnostic software development desktop app with an embedded Rust backend and local HTTP API. Its default workflow keeps one canonical thread, lets the next turn hot-swap between provider, local, and account runtimes, keeps workspace execution under explicit local control, and puts Git diff review beside the conversation.

## Start here

| Use case | Read |
|---|---|
| First run | Quickstart, then Models, then Desktop app. Stop when you can connect a runtime, optionally pick a folder, and open the main thread. |
| Daily app | Desktop app, Models, and Privacy explain the canonical thread, model switching, local controls, and review surfaces. |
| API integration | API, Models, Config, and Troubleshooting cover compatibility routes and stored state. |
| Release or support | Release, Config, and Troubleshooting cover build checks, local state, and failure triage. |

## Core workflow

1. Keep the work in one canonical thread.
2. Hot-swap the next turn between connected models and account runtimes.
3. Choose the local workspace and execution boundaries explicitly.
4. Inspect tool output and Git diffs before accepting changes.

Agents, Workers, skills, schedules, MCP, media, Google Workspace, previews, and the mobile companion are optional power modules available from **Tools**. They extend the core workflow without changing it.

## App model

| Part | Boundary |
|---|---|
| Desktop app | Tauri 2, Vite, React, TypeScript, one canonical thread, persisted UI state, and per-launch bearer auth. |
| Embedded server | Axum HTTP server with OpenAI, Anthropic, Ollama, provider, workspace, agent, memory, MCP, media, mobile, and privacy routes. |
| Local data | Provider records, settings, threads, memories, schedules, and runtime state live under the Milim home directory. |
| Remote traffic | Hosted chat, embeddings, media, Codex, and installed Claude CLI calls pass through explicit routing and the privacy gate. |

## Source map

| Source | Path |
|---|---|
| Server router | [crates/milim-server/src/lib.rs](https://github.com/oshtz/milim/blob/main/crates/milim-server/src/lib.rs) |
| Desktop API client | [apps/desktop/src/api.ts](https://github.com/oshtz/milim/blob/main/apps/desktop/src/api.ts) |
| Embedded Tauri server | [apps/desktop/src-tauri/src/lib.rs](https://github.com/oshtz/milim/blob/main/apps/desktop/src-tauri/src/lib.rs) |
| Thread/session state | [apps/desktop/src/sessions/store.ts](https://github.com/oshtz/milim/blob/main/apps/desktop/src/sessions/store.ts) |
| Account runtimes | [docs/account-runtimes.md](https://github.com/oshtz/milim/blob/main/docs/account-runtimes.md) |

## Local-first line

Local-first does not mean local-only. milim can talk to OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio, Replicate, fal, Codex, and the installed Claude CLI. The important boundary is explicit routing: local API runtimes stay on the machine, provider models use Milim's tool-agent loop when workspace or tool context is active, Codex and Claude use their account-runtime bridges, and remote sends can pass through the server-side privacy gate before leaving it.
