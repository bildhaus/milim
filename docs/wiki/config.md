---
id: config
path: config
label: Config
title: Config, storage, and build flags
summary: Milim home, runtime asset directory, server config, persisted databases, provider records, desktop state, and native build variants.
group: Reference
order: 100
updated: 2026-07-13
---

Configuration is intentionally local. The desktop app embeds the server, encrypts provider, Google, MCP, and mobile-companion credentials with a master key held in the OS credential vault, and keeps optional native runtimes behind explicit build flags. macOS also retains a matching owner-only recovery key so encrypted state survives local app rebuilds whose Keychain access identity changes. Settings > Data & privacy reports whenever this permission-restricted local key is present.

## Default locations

| Item | Default |
|---|---|
| Milim home | OS app-data location resolved by `milim-core` paths. |
| Server config | `~/.milim/config/server.json` for standalone CLI/server use. |
| Identity key | `~/.milim/identity/master.key`. |
| Desktop credential key | Windows Credential Manager, or macOS Keychain with a matching owner-only `desktop-storage.key` recovery copy for rebuild continuity. |
| Provider records | AES-GCM encrypted provider records under the Milim data root. |
| Runtime assets | Milim runtime directory for downloaded model and media-related assets. Previously downloaded voice assets are left untouched but no longer used. |
| Schedules | `schedules.db` under the Milim root. |
| Agents and Worker Runs | `agents.db` and `threads.db` under the Milim root. `threads.db` retains legacy child rows and stores Run batches in `worker_runs`. |

## Desktop session state

The desktop UI hydrates through the canonical `milim.sessions` user-state key, but the Tauri store persists each chat session as a `user_sessions` SQLite row and each transcript message as a `user_session_messages` row keyed by session id and message index. Normal saves send a transactional delta containing only changed session metadata and message indices; full snapshots remain for hydration, legacy import, and recovery. Non-session metadata such as the active id, queued messages, sidebar organization, and archive retention stays in a small `milim.sessions.meta` JSON entry. Worker Runs are not duplicated there: `threads.db` remains their durable authority and the UI reloads them through the server API. Legacy `milim.sessions` blobs are migrated into rows on first session read. During active generation, desktop session persistence defers writes and flushes the final state when the turn ends; unsent composer drafts use the separate tiny `milim.sessionDrafts` user-state key.

The shared `milim.db` profile database uses verified SQLite WAL mode with foreign keys and a five-second busy timeout. WAL lets session and memory readers overlap with a writer; writes to one SQLite file are still serialized. Live backup or future sync must use SQLite's backup/checkpoint mechanisms rather than copying only `milim.db` while the app is running.

The remaining storage work is:

| Phase | Behavior |
|---|---|
| Queue/sidebar rows | Move queued messages and sidebar state out of metadata JSON when their write volume justifies it. |
| Usage reads | Store checkpoint and response metrics separately from raw message text for cheaper usage summaries. |

## Server config

| Setting | Behavior |
|---|---|
| Port | Standalone server defaults to `7377`; desktop discovers its embedded loopback port through Tauri. |
| Expose | `milim serve --expose` binds beyond loopback and auto-enables `msk-v1` auth when no auth is configured. |
| CORS | Empty allow-list means no browser origins are allowed. |
| Auth | `authRequired: true` accepts locally minted `msk-v1` keys; `apiKeys` accepts static bearer secrets; `accessKeyIssuers` trusts additional signed-key issuers. |

Standalone CLI/server identity and configuration remain file-based for headless compatibility. Milim creates key-bearing files with owner-only permissions on Unix. Desktop upgrades re-encrypt legacy provider, Google, MCP, and mobile-companion credentials with the OS-backed master key and remove the old key/plaintext files only after verification. This migration is one-way; an older desktop build requires those integrations to be reconnected or re-entered.

## Build variants

```powershell Native feature builds
$env:MILIM_WHISPER_MODEL = "C:\models\ggml-base.bin"
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --features whisper
cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml --features native-vad
```

## Narrow resets

| Problem | Smallest reset |
|---|---|
| Bad provider key | Delete or update that provider record. |
| Broken MCP server | Use the MCP Servers sheet Test connection action, fill any required env placeholders, or remove the server through `/mcp/servers/{id}` or the desktop UI. Imported secret-looking env values are placeholders only; Milim never copies secret values from Claude/Codex configs. |
| Bad theme | Reset desktop theme settings, not the whole app state. |
| Stale memory | Archive or delete the specific memory node. |
| Stuck schedule | Disable or delete the schedule row. |
