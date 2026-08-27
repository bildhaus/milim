---
id: api
path: api
label: API
title: HTTP API surface
summary: OpenAI-compatible, Anthropic-compatible, Ollama-compatible, providers, media, workspace, MCP, Agents, Worker Runs, memory, privacy, skills, schedules, mobile, and account runtime routes.
group: Reference
order: 90
updated: 2026-08-20
---

The standalone server accepts static bearer keys or `msk-v1` access keys when configured in `~/.milim/config/server.json`. The desktop app uses its own per-launch bearer token and resolves the actual loopback port through Tauri.

## Run the standalone server

```powershell Run the CLI server
cargo build --release
$env:MILIM_REMOTE_BASE_URL = "http://localhost:11434/v1"
cargo run -p milim-cli -- serve
```

In another terminal:

```powershell Probe the CLI server
cargo run -p milim-cli -- status
cargo run -p milim-cli -- models
```

```powershell OpenAI-compatible chat
curl http://127.0.0.1:7377/v1/chat/completions `
  -H "Content-Type: application/json" `
  -d '{"model":"llama3.2","messages":[{"role":"user","content":"hello"}],"stream":true}'
```

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

## Compatible APIs

| API | Endpoint | Use |
|---|---|---|
| OpenAI chat | `POST /v1/chat/completions` | OpenAI-compatible SDKs and tools. |
| OpenAI responses | `POST /v1/responses` | Responses-compatible clients, including `input`, `instructions`, `tools`, streaming, reasoning effort, and `text.format`. |
| OpenAI completions | `POST /v1/completions` | Legacy prompt-completion clients. |
| OpenAI models | `GET /v1/models` | Model discovery. |
| OpenAI embeddings | `POST /v1/embeddings` | Embedding-compatible clients. |
| Anthropic messages | `POST /anthropic/v1/messages` | Claude Messages-compatible clients. |
| Ollama chat | `POST /api/chat` | Ollama clients that speak `/api/chat`, including `think` and streamed or final `message.thinking`. |
| Ollama generate | `POST /api/generate` | Ollama prompt-style generation, including `prompt`, `suffix`, `raw`, `options`, `think`, and `format`; empty-prompt `keep_alive` lifecycle calls are forwarded to native Ollama backends when available. |
| Ollama tags | `GET /api/tags` | Ollama-style model discovery. |

Structured-output controls are passed through to the selected backend: OpenAI `response_format`, Responses `text.format`, and Ollama `format` are normalized onto the internal completion request where supported.

Multimodal compatibility inputs remain provider-native: OpenAI Chat uses `image_url` content parts, Responses accepts `input_image`, Ollama accepts message `images`, and Anthropic Messages accepts base64 or HTTP(S) URL image sources. Malformed or unsupported Anthropic sources return an invalid-request error instead of being discarded. Gemini receives uploaded bytes as `inline_data`; only genuine `generativelanguage.googleapis.com/.../files/...` URIs become `file_data`, and arbitrary web image URLs fail validation without a server-side downloader.

Root aliases are also mounted for OpenAI chat, completions, models, and embeddings: `/chat/completions`, `/completions`, `/models`, and `/embeddings`.

## Route groups

| Area | Routes |
|---|---|
| Provider registry | `GET/POST /providers`, `GET /providers/discover`, `DELETE /providers/{id}` |
| Media | `GET /media/models`, `GET /media/model-schema`, `GET /media/status`, authenticated `GET /media/content`, `POST /media/generate`, `GET /media/library`, `POST /media/library/{id}/refresh`, `GET /media/library/{id}/content/{index}`, `DELETE /media/library/{id}` |
| Workspace | `GET/POST /workspace`, `GET /workspace/git`, `POST /workspace/git/action` (`diff`, sync, commit, checkpoint, restore checkpoint, create/apply/remove Hot Swap retry worktree) |
| Preview apps | `GET /preview-apps/{runtime_id}`, `POST /preview-apps/{runtime_id}/stage`, `POST /preview-apps/{runtime_id}/start`, command-free workspace HTML `POST /preview-apps/{runtime_id}/static`, `POST /preview-apps/{runtime_id}/stop`, `POST /preview-apps/{runtime_id}/restart`, `GET /preview-apps/{runtime_id}/logs` |
| MCP | `GET /mcp/tools`, `POST /mcp/call`, host-only `POST /mcp/apps/resources/read`, host-only `POST /mcp/apps/tools/call`, ephemeral `GET /mcp/apps/views/{id}`, `GET/POST /mcp/servers`, `POST /mcp/servers/test`, `POST /mcp/servers/{id}/test`, `DELETE /mcp/servers/{id}` |
| Agents | `POST /agents/run`, `GET/POST /agents`, `GET/PUT/DELETE /agents/{id}`, `POST /agents/{id}/run` |
| Worker Runs | `GET/POST /worker-runs`, `GET/DELETE /worker-runs/{id}`, cursor-aware `GET /worker-runs/{id}/events?after_seq=N`, `POST /worker-runs/{id}/start`, `POST /worker-runs/{id}/stop`, `POST /worker-runs/{id}/tasks/{task_id}/retry`; writer diff review/apply routes are scoped to a worker in the Run. |
| Threads | `GET /threads/{id}` (`include_events=true&event_limit=N` returns `event_count` and `events_truncated`), `DELETE /threads/{id}`, `GET /threads/{id}/children`, `GET /threads/{id}/events`, `POST /threads/{id}/stop` |
| Memory | `POST /memory/ingest`, `POST /memory/search`, `POST /memory/register`, `POST /memory/graph/search`, `GET /memory/scopes`, `GET /memory/nodes` |
| Workspace context | `GET /workspace/context` |
| Tool approval | `POST /tool-approvals/{approval_id}` |
| Privacy | `POST /privacy/scan`, `GET/POST /privacy/mode` |
| Sandbox and computer | `POST /sandbox/run`, `GET/POST /computer` |
| Skills and schedules | `GET/POST /skills`, `POST /skills/select`, `GET/PUT/DELETE /skills/{id}`, `GET/POST /schedules`, `PUT/DELETE /schedules/{id}` |
| Mobile companion | Identity probe `GET /mobile`; desktop-authenticated status, enablement, manual-pairing, pairing-request decision, and device revocation routes; private-key-protected `POST/GET/DELETE /mobile/pair-requests...` request, status, claim, and cancellation routes; public one-time manual claim `POST /mobile/pair`; paired-device `GET /mobile/device/status` and `DELETE /mobile/device` |
| Canonical control v1 | `GET /control/v1/bootstrap`, `GET /control/v1/threads/{id}/timeline`, read-only `POST /control/v1/threads/{id}/effective-run`, authenticated `GET /control/v1/runs/{run_id}`, paged `GET /control/v1/runs/{run_id}/events`, `POST /control/v1/commands`, `POST /control/v1/socket-ticket`, `GET /control/v1/ws?ticket=...` |
| Account runtimes | Canonical `POST /harnesses/{id}/run`; compatibility and sideband routes: `GET /codex/account`, `POST /codex/login/device`, `POST /codex/login/chatgpt-device`, `POST /codex/login/api-key`, `POST /codex/logout`, `GET /codex/rate-limits`, `GET /codex/models`, read-only `GET /codex/threads` and `/codex/threads/{id}`, `POST /codex/run`, `GET /claude/status`, read-only `GET /claude/threads` and `/claude/threads/{id}`, `POST /claude/run`, `GET /opencode/status`, `GET /opencode/models`, `POST /opencode/run`, `GET /pi/status`, `GET /pi/models`, `POST /pi/run` |

`POST /memory/graph/search` is the compatibility route for scoped hybrid retrieval; it does not perform graph traversal. The request and response shapes are unchanged. Each `MemoryGraphHit.score` is normalized fused relevance in `0..1`, combining exact-term FTS and embedding ranks with equal-weight reciprocal rank fusion. Scope, archive, and `top_k` filters still apply. Desktop turns request 20 candidates and inject no more than five provenance-labeled entries within a 1,024-token memory budget.

`GET /mcp/tools` is the complete agent-configuration catalog and each entry includes an `effect` (`read_only`, `mutating`, `command`, or `unknown`). `GET /mcp/tools?callable=true` returns the read-only subset that `POST /mcp/call` can invoke; mutations must run through an agent with Review approval or Open mode. External MCP tools use stable id-derived namespaces, cannot shadow first-party tools, and default to `unknown` unless their MCP annotations explicitly mark them read-only. Previous display-name-derived MCP names remain accepted as non-advertised aliases for saved custom-agent selections.

The authenticated MCP Apps POST routes are for the desktop host, not iframe code. `resources/read` accepts only an advertised `ui://` resource from a server that negotiated Apps, requires `text/html;profile=mcp-app`, and caps decoded HTML at 5 MiB. For rendering, it returns a random, one-use `/mcp/apps/views/{id}` capability path backed only by bounded memory; the GET needs no bearer because its unguessable path is the capability, expires after ten minutes if unused, and returns `no-store`. `tools/call` resolves the app-visible tool in the supplied originating server, applies Review/Guarded/Open server-side, and enforces the 1 MiB result boundary. The iframe never receives the desktop's bearer token. The original read-only `/mcp/call` contract is unchanged.

`http_fetch` accepts public HTTP(S) destinations only, validates every redirect, applies DNS/address checks, and limits transfer time and body size. It rejects loopback, private, link-local, multicast, and metadata-service addresses.

`POST /media/generate` and `GET /media/status` remain backward-compatible and additionally return `library_id` and `save_state` when the local library is enabled. `GET /media/library` accepts optional `query`, `kind`, `provider`, `status`, `cursor`, and `limit` parameters and returns `items` plus `next_cursor`. Refresh resumes a stored pending provider run or retries a failed local save. Content lookup serves only an allowlisted file recorded under that library item's UUID directory, and delete permanently removes the directory before removing the index record. The library index is `~/.milim/media/index.json`; media files are under `~/.milim/media/files/<library-id>/`.

`POST /schedules` and `PUT /schedules/{id}` require a provider/local API `model` for deterministic unattended execution; `codex:*`, `claude:*`, `opencode:*`, and `pi:*` account models are rejected. The optional `attachments` array uses the desktop chat shape (`id`, `name`, `mime`, `size`, `content`, `dataUrl`, `truncated`, `sourcePath`). Text content is appended to the scheduled prompt and stored image data becomes a real image part each time the automation fires. A legacy image without `dataUrl` records a visible error asking for reattachment. Existing schedules may have an empty model for compatibility; the runner falls back to their linked Agent's deprecated model, and records a visible error if neither exists. Each due occurrence creates one canonical control-v1 thread and turn with a durable schedule origin; deterministic command IDs make retries idempotent. Desktop and mobile discover those threads through bootstrap and timeline replication rather than a separate completion queue.

`POST /codex/run`, `POST /claude/run`, and `POST /pi/run` accept an optional `images` array of `{ "media_type": "image/png", "data": "<base64 bytes>" }`. PNG, JPEG, WebP, and GIF are limited to 2 MB each, and either a non-empty `prompt` or at least one valid image is required. Codex materializes validated bytes into temporary per-turn files and sends app-server `localImage` inputs; Claude pipes a native multimodal user message through `--input-format stream-json`; Pi sends native RPC image blocks. Account-runtime images require Privacy Off.

`POST /harnesses/{id}/run` is the desktop-facing facade for `codex`, `claude`, `opencode`, and `pi`. It accepts the existing run fields under one request shape, using `native_session_id` and `persist_session` for native continuity. Its SSE data is `{ "schema_version": 1, "run_id": "...", "seq": 1, "at_ms": 0, "harness_id": "codex", "event": { "type": "text_delta", "text": "..." } }`. Sequence numbers increase within a run, and the stream emits exactly one `turn_completed`, `turn_failed`, or `turn_cancelled` event. Other canonical event types cover session/turn identity, reasoning, tool and approval phases, usage/limits, images, native workers, runtime notices, and session recovery. Unknown v1 event variants are safe to ignore; unsupported schema versions must be rejected. The four legacy run routes retain their original request and event contracts.

`GET /pi/status` reports Pi availability, version, authentication/configuration state, provider count, normalized model metadata, and any actionable error. `GET /pi/models` returns the normalized catalog. `POST /pi/run` accepts the normal account-runtime fields plus a `pi:<provider>/<model>` selection (the HTTP body carries the prefix-stripped `provider/model`), optional `session_id`, and optional `persist_session`; desktop chat turns persist, while compaction and other side calls use `persist_session: false`.

`GET /workspace/context` returns the canonical root, sanitized origin display, stable and legacy Project memory locators, ordered AGENTS/Claude instruction files with contents, byte counts and statuses, plus discovery warnings. AGENTS loading uses override precedence and a 32 KiB aggregate limit; path-conditional Claude rules are returned as conditional with a warning rather than applied globally.

Streamed run requests accept optional `interactive_tool_approval`. In Review, a consequential call emits `tool_approval_required { approval_id, call_id, name, arguments, effect }`; resolve it with authenticated `POST /tool-approvals/{approval_id}` and `{ "decision": "approve" | "deny" }`. A successful first resolution returns `204`, an expired/unknown id returns `404`, and a repeated resolution returns `409`. The stream then emits `tool_approval_resolved`. Approvals are ephemeral, exact, and one-shot. `tool_approval_grant: true` remains the explicit whole-run option for headless callers.

`POST /agents/run` accepts optional `workspace` and `privacy_mode` (`off`, `redact`, or `block`). Desktop sends both. Legacy callers may omit them, in which case the server snapshots its current defaults once at request start. An explicit invalid path or mode returns `400`; the server does not silently substitute a different context. New Worker Runs persist the originating workspace and privacy mode. Completed legacy Runs remain readable, but a legacy Run with no captured context cannot be approved or retried.

Worker and child event streams drain stored rows before live broadcast. Pass `after_seq` when reconnecting, deduplicate the monotonic `seq` values client-side, and reconcile terminal state through the canonical Run or thread GET route.

The isolated mobile listener no longer serves a browser PWA, relay queue, thread snapshot, or SSE compatibility routes. Native iOS and Android clients use the identity probe, pairing/device routes, and `/control/v1` directly.

`/control/v1` is separate from legacy Worker `/threads/*`. Bootstrap returns the stable host identity, supported protocol range and additive capabilities, the host's resolved `appearance` snapshot, thread summaries and revisions, model and Agent summaries, active runs, queued turns, `pending_inputs`, and pending approvals. Each thread summary may include reciprocal `linked_threads`; each queued mailbox turn may include `mailbox_origin`. Optional `run_ledger`, `run_inspection`, `steering`, `context_injection`, and `thread_links` capabilities remain inside protocol v1; a missing flag means unsupported. Each queued turn includes its bounded display text and validated attachments so a reconnecting client can restore queue content and controls without maintaining a second queue. Pending steering inputs expose the same bounded display projection while remaining atomically claimable by their exact target run; injected context stays opaque in the ordinary transcript. Run snapshots independently advertise whether that run is inspectable or steerable and whether its ledger visibility is `model_visible` or `harness_boundary`. Appearance is additive within v1 and falls back to Mono Dark for older clients or missing state; `appearance.updated` asks connected clients to refresh it. Successful provider catalog mutations and desktop provider refreshes similarly publish `models.updated`, which asks connected clients to reload bootstrap rather than retain a stale LM Studio or hosted-provider snapshot. `GET /control/v1/appearance/background?revision=...` returns only the active custom theme's bounded JPEG, PNG, GIF, or WebP data image, requires the same paired-device/control authentication, rejects stale revisions, and never resolves arbitrary remote CSS URLs. Before control clients are served, existing `user_session_messages` are idempotently projected into an empty canonical timeline with their stable message IDs, display content, and saved reasoning. Timeline reads accept bounded `tail`, `after_seq`, or `before_seq` queries and return the thread epoch, sequence coverage, pagination cursors, and projected items. A client replaces stale cache state on epoch mismatch and fills monotonic sequence gaps from this authoritative history.

Commands carry a client-generated `command_id`, typed kind, target thread, optional expected revision, and payload. Durable receipts make identical retries return the original `applied`, `accepted`, `queued`, `needs_confirmation`, `conflict`, or `failed` result. Conflicting approval decisions fail while an identical retry succeeds. Destructive commands that need confirmation return a one-use token. The supported kinds cover thread lifecycle and model/Agent binding, message deletion, send/stop/regenerate, queued-turn resume/move/delete, individual approval resolution, Worker start/continue-solo/stop, plus `turn.steer`, `context.inject`, `turn.inbox_delete`, `thread.link.add`, and `thread.link.remove`. Link commands use `thread_id` as the owner and require `payload.target_thread_id`; identical adds/removes are idempotent, while actual changes increment the owner revision and append canonical link events. `turn.queue_move` accepts a queued ID, target ID, and `before` or `after` position and persists that order independently from acceptance time. `turn.queue_resume` normally requires an idle thread; with `payload.interrupt_active: true`, the server validates and reserves that durable queue item, signals the active run to stop, and starts the selected item only after active-run teardown completes. Busy `turn.send` remains a future follow-up. `turn.steer` requires the exact active `run_id`; unsupported or mismatched runs reject it. `context.inject` stays pending for the next request and never starts work. Pending items can be removed only before their atomic claim.

The authenticated run inspection routes are deliberately absent from bootstrap and transcript reads. `GET /control/v1/runs/{run_id}` returns the frozen run and resolved composition, including additive `linked_thread_grants` and `claimed_mailbox_ids`. `GET /control/v1/runs/{run_id}/events?after_seq=N&limit=N` returns bounded ordered events and expands content-addressed artifact references only in that explicit response. Link, unlink, mailbox queued/running/replied/failed, and reply-consumed transitions are canonical timeline events. An agent may explicitly wait on an exchange created by its current run; successful waits record reply consumption in that run, while timeout or cancellation leaves the exchange claimable later. Ordinary transcript exports omit the ledger; control backup version 3 includes it, durable thread links, and mailbox state. Versions 1 and 2 remain restorable, and version 1 queued turns restore as follow-up inbox items.

`POST /control/v1/threads/{id}/effective-run` accepts the current draft text and control-v1 attachments, resolves the same frozen configuration and privacy boundary used at acceptance, and returns a composition plus the source thread revision. It does not append a message, claim inbox work, create a run, or invoke a model. Pending inbox inputs and runtime-inherited tool schemas are reported as warnings because they finalize only when execution starts.

All registry invocation paths use the fixed tool-execution pipeline, including canonical agents, Worker Runs, schedules, `/agents/run`, MCP, and MCP Apps. Consequential and unknown tools are exclusive; only tools declaring both ReadOnly and parallel safety may overlap, up to four calls per run and sixteen process-wide. External MCP tools remain exclusive. Results append in model-call order, sibling failures remain independent, and the default deadline is 120 seconds unless a tool has a justified override.

Thread summaries include the validated per-model `reasoning_effort_overrides` map. `thread.set_model` requires `payload.model` and accepts an optional validated `payload.reasoning_effort`; when present, both values are persisted in the same thread revision so a mobile client does not need a second mutation after reconnect. Explicit `auto` is retained as a thread override so the chat can opt out of a non-auto app default.

Paired-device credentials are accepted only on the isolated mobile/control router. A credential exchanges for a short-lived, single-use socket ticket; only that ticket appears in the WebSocket URL. Device revocation invalidates subsequent HTTP and ticket access, and live sockets recheck authorization.

The built-in `memory_register` tool accepts `content`, optional `title`, and optional `scope` (`personal` or `project`). The lower-level `/memory/*` HTTP routes remain compatible with scoped node records.

## msk-v1 keys

`msk-v1` keys are signed local access tokens. The token layout is `msk-v1.<base64url(payload)>.<hex(sig65)>`. The payload is canonical JSON with alphabetical keys, and the signature is a secp256k1 recoverable signature over the `"Milim Signed Access"` domain-separated digest.

| Payload field | Meaning |
|---|---|
| `aud` | Audience address the key authorizes against. |
| `cnt` | Monotonic counter used with revocation. |
| `exp` | Optional Unix-seconds expiry. |
| `iat` | Issued-at Unix timestamp. |
| `iss` | Issuer address, which must match the recovered signer. |
| `lbl` | Optional human label. |
| `nonce` | Random nonce used with revocation. |

```powershell Mint an msk-v1 key
cargo run -p milim-cli -- keys identity
cargo run -p milim-cli -- keys mint --label local-client --expires-secs 86400
```

Set `authRequired: true` in `server.json` to make `milim serve` accept keys minted by this machine. `milim serve --expose` saves that setting and prints a one-time token when no auth is already configured. Use `--audience` when minting a key for a different Milim identity. Omitting it mints for this machine's own address.

## Common failures

| Status | Usually means |
|---|---|
| 400 | An explicit workspace, privacy mode, cursor, or other request value is invalid. |
| 401 | Missing or invalid bearer token or access key. |
| 404 | Route group is not mounted in this build or the id does not exist. |
| 409 | Local state rejected the requested mutation. |
| 422 | JSON shape or enum value is invalid. |
| 500 | Provider, runtime, database, Docker, or external process failed. |
