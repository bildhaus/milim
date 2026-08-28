<h1>
  <img src="./apps/desktop/public/milim-single-m.svg" alt="" width="38" />
  milim
</h1>

**Your local control plane for coding agents.** Use your own models and subscriptions, keep one canonical thread, review the diff, and ship.

milim is a model-agnostic software development app built around a Rust-owned agent runtime with interchangeable desktop and mobile clients. Connect hosted providers, local model servers, or installed coding-agent CLIs without rebuilding your workflow around one vendor.

[Website](https://milim.ai/) · [Documentation](https://docs.milim.ai/) · [Latest release](https://github.com/bildhaus/milim/releases/latest) · [Source](https://github.com/bildhaus/milim)

Release artifacts target Windows and macOS. Linux packaging is not a primary release target, but the Rust server and Tauri app remain source-buildable.

## Why milim

- **One canonical thread.** Conversation, workspace context, attachments, tool activity, approvals, and review history stay together when you change models or runtimes.
- **Threads can collaborate.** Drag one root chat into another to create a reciprocal link with bounded transcript reads, durable mailbox sends, and explicit reply waits, without merging either history.
- **Your runtime choice.** Switch the next turn between hosted providers, Ollama, LM Studio, or vLLM, and separately installed Codex, Claude, OpenCode, or Pi runtimes. Thread history stays put; provider turns can keep per-model generation and capability overrides.
- **Explicit local boundaries.** Workspace selection, model routing, outbound privacy, and approval mode remain visible and under your control.
- **Execution you can inspect.** Consequential tool calls can pause for review. Git diffs, checkpoints, previews, run details, and recovery actions stay beside the conversation.
- **A workbench when you need it.** Agents, Workers, skills, schedules, MCP servers and Apps, memory, media generation, Google Workspace, and previews extend the core thread through the **Tools** launcher.
- **Direct mobile control.** The native iOS and Android companion connects to paired desktops over Tailscale, trusted LAN discovery, or a manual URL. Enabled desktop transports restore automatically after milim restarts. milim operates no relay, account service, or cloud transcript store for this path.

Provider-backed chat and installed account runtimes remain distinct. Provider models use milim's tool-agent loop; account runtimes retain their own sessions and tools behind the same visible approval policy.

## Core workflow

1. Connect a hosted provider, local model server, or installed account runtime.
2. Optionally choose a workspace and confirm the privacy and approval boundaries above the composer.
3. Send a bounded task, approve consequential actions when required, and inspect the resulting diff or preview.
4. Continue the same thread with another model when a second implementation or review pass is useful.

The [quickstart](https://docs.milim.ai/quickstart) walks through this safe-change loop end to end.

## Get started

### Install the desktop app

Download the Windows portable EXE or macOS universal DMG from the [latest GitHub release](https://github.com/bildhaus/milim/releases/latest).

The desktop app embeds the server and runs as a single instance, so normal use does not require a separate `milim serve` process. On first run, choose a runtime, optionally choose a workspace folder, and open milim.

### Run the desktop app from source

From the repository root:

```powershell
corepack enable
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

Desktop Tauri commands load an ignored repository-root `.env.local` when it exists. Provider credentials can also be added through the app and are stored in encrypted local state.

### Run the native mobile app from source

The companion is a bare React Native project; it does not include Expo or EAS.

```powershell
pnpm -C apps/mobile install --frozen-lockfile
pnpm -C apps/mobile verify
pnpm -C apps/mobile android
```

Run iOS from macOS with CocoaPods and `apps/mobile/ios/MilimMobile.xcworkspace`. Pairing and control require the milim desktop process to remain alive. Signed beta builds are delivered from immutable release tags through the protected, manual mobile-store workflow to TestFlight and Play internal testing; public promotion remains a separate manual store action.

### Run the standalone server

The `milim` CLI exposes OpenAI-, Ollama-, and Anthropic-compatible routes over a local HTTP API. Point `MILIM_REMOTE_BASE_URL` at an OpenAI-compatible backend such as Ollama, LM Studio, vLLM, OpenAI, or OpenRouter:

```powershell
cargo build --release
$env:MILIM_REMOTE_BASE_URL = "http://localhost:11434/v1"
cargo run -p milim-cli -- serve
```

In another terminal:

```powershell
cargo run -p milim-cli -- status
cargo run -p milim-cli -- models
```

OpenAI-compatible clients can use `http://127.0.0.1:7377/v1`. milim does not ship a GGUF inference runtime; local inference remains the responsibility of the configured model server. See the [API reference](https://docs.milim.ai/api) for supported routes, authentication, and examples.

## Architecture

| Part | Responsibility |
|---|---|
| Desktop app | Tauri 2 with Vite, React, and TypeScript. Presents canonical thread state plus desktop-only workspace, preview, Git review, and a dedicated theme-aware full-window Settings surface that leaves the active workspace mounted. |
| Native mobile app | Bare React Native with TypeScript and Metro. Acts as a bounded, multi-host controller and cache for paired desktops; thread height follows the native keyboard so the composer and followed transcript tail remain visible. |
| Embedded server | In-process Axum server and Rust `RunManager`. Owns active turns, queues, approvals, normalized events, and runtime adapters. |
| Local data | Desktop SQLite is authoritative for threads, reciprocal thread links, mailbox exchanges, runs, timelines, queues, approvals, and favorite model IDs. Mobile SQLite is a host-partitioned cache for timelines, drafts, and client-only picker layout. |
| Model sources | Hosted providers, local OpenAI-compatible servers, and separately installed account-runtime CLIs connect through explicit routing and privacy boundaries. |

Desktop publishes its resolved provider and account-runtime model catalog for control v1, so mobile receives the same hosted, local, Codex, Claude, OpenCode, and Pi choices.

Scheduled occurrences enter that same canonical control path: each one freezes its model, workspace, privacy mode, local-or-UTC cron zone, and Guarded approval boundary; it has a durable origin, timeline, and run ledger, appears on desktop and mobile, and remains idempotent when dispatch is retried.

The desktop Context panel can explicitly resolve an effective-run preview before sending, showing the privacy-processed prompt layers, runtime boundary, model, workspace, policies, tool/skill selection, and attachment provenance without creating a run.

Memory retrieval has an explicit quality and lifecycle boundary: labeled cases can benchmark the production hybrid retriever with recall@k and mean reciprocal rank, while the Memory library distinguishes reviewed entries, revocation by archive, restoration, and confirmed permanent deletion.

The desktop and mobile clients are replicas of Rust-owned canonical state. Favorite model changes made in either picker update the desktop-owned list and propagate live to the other client. New desktop chats are created in canonical Rust state before their local replica becomes active, but stay out of desktop thread navigation until their first message is sent; desktop model and reasoning changes are committed there before the next turn starts. Inbox navigation freezes each working thread's recency key until that work ends, so concurrent stream and tool events cannot make active rows repeatedly leapfrog each other. Settings > Model & agent defaults includes app-wide Custom instructions that are frozen into every accepted turn, while repository `AGENTS.md` and `CLAUDE.md` files remain workspace-scoped context. Deleting a message commits the canonical deletion before removing its row, so reloads and future model context cannot restore it. Renderer session-order deltas preserve chats added canonically after the replica snapshot. Renderer message deltas identify their acknowledged message-count base; if canonical Rust state changed first, SQLite keeps the authoritative transcript while still accepting safe replica metadata. Canonical assistant messages retain token usage and cost provenance: provider-reported billed cost is preferred and cached catalog pricing supplies a labeled estimate only when exact cost is unavailable. Codex, Claude, OpenCode, and Pi each keep one native session binding and sync cursor per Milim thread; Rust persists them with the run, resumes them after restart, and sends only intervening messages when returning to an adapter. Native App and URL previews remember their zoom levels independently across child-webview recreation and app restarts, and muted preview tabs stay muted for the lifetime of that persisted tab. On Windows, routine preview panel and tab teardown parks one of three bounded native WebView slots instead of synchronously closing WebView2 controllers; the desktop process reclaims them when it exits. Folder-backed chats share one managed App runtime per normalized workspace, while preview tool targets and URL-open requests stay bound to their originating chat across background work and navigation. Desktop turns accepted while a managed App or static preview is active freeze a sanitized runtime snapshot for model awareness even when the Inspector is hidden; preview tools remain gated by a visible, ready, DOM-capable surface. App preview prefers an optional `.milim/preview.json` repository manifest for explicit argv, working directory, port, browser path, and readiness path, then falls back to `package.json` dev-script detection; a configured-port conflict blocks launch with an actionable error. A successful switch between two selected models adds a quiet, durable transcript notice showing the previous model and that the same thread continues. Repeated selections before the next user message update that pending notice from the original model to the latest choice, and returning to the original removes it; initial selection and reasoning-only changes stay silent. When an older thread contains a provider route that no longer exists, opening it repairs an unambiguous replacement or asks for a new model selection. Hiding or reloading the desktop renderer does not pause accepted work; explicit quit or restart cancels active runs and records unfinished work as interrupted, with a bounded Windows shutdown fallback if native WebView teardown stalls.

External text files and supported images can be dropped anywhere in the desktop window and attach once to the active chat's composer, up to twelve items. Large text files keep their original size metadata but send a bounded 128 KiB preview; unsupported binary files fail at attachment time. Models that explicitly report no image-input support reject image paste, drop, picker, and send attempts with an actionable composer notice; unknown capability metadata stays permissive. Dragging a canonical root chat from Sidebar, Top, or Bottom navigation over another active chat creates a durable reciprocal link, shown in both composers. Linked chats receive bounded read/list/send/wait tools according to the selected Agent and approval mode; mailbox sends steer a compatible active destination and otherwise start or queue durable work. Replies return visibly without starting a new origin run, while a run that depends on one can explicitly wait for its exchange and continue with that reply.

The taskbar or Dock badge counts distinct visible chats with unread terminal updates or unresolved attention. Opening a chat clears its unread portion, resolving its pending action clears attention, and archived chats or projects never keep the badge active.

Desktop history is loaded lazily: startup hydrates thread summaries and the latest 100 messages of the active or running chats, then pages older messages without moving the reader's scroll position. Long transcripts keep at most 200 message rows mounted; measured window shifts and asynchronously resizing streamed content preserve the visible row when the reader is detached and the true bottom while auto-follow is active. An accepted turn shows a transient shimmering `thinking...` cue at the transcript tail until assistant activity arrives, without persisting a blank message. Claude CLI thinking deltas remain available after completion as clearly labeled reasoning summaries; an explicit high, xhigh, or max turn that receives no summary shows a non-error notice. Live Worker Run activity stays in a stable slot at the transcript tail until the run finishes. Stable run-scoped render identity keeps the live canonical assistant row mounted when its final message ID arrives, while canonical IDs continue to own persistence and mutations. The lossless run ledger deduplicates artifacts by SHA-256, compresses large JSON, and stores repeated provider requests as verified deltas with periodic checkpoints; legacy rows migrate in small idle-only batches.

While a steer-capable provider run is active, Ctrl/Cmd+Enter sends the composer input to the next model step and immediately adds a separate `Steer · Pending` user bubble to the transcript. Claiming it at the next model boundary removes the pending state while preserving the steer as its own user message without replacing the original prompt; reconnecting clients restore the same pending text from canonical state. The primary busy Send action remains Queue; runtimes without steering keep modifier-Enter as queue submission and never substitute cancellation. Interrupting with a queued message is one server-owned handoff: the current response stops, then the selected durable queue item starts without a client-side stop/resume race.

## Development

Use the smallest relevant check for the area changed:

```powershell
# Rust workspace
cargo test
cargo clippy --workspace --all-targets

# Desktop
pnpm -C apps/desktop verify

# Release-runtime performance proof (Windows WebView2)
pnpm -C apps/desktop perf:canonical

# Seven fresh canonical processes with median/p95 reporting
pnpm -C apps/desktop perf:suite

# Native mobile
pnpm -C apps/mobile verify

# Site and documentation
pnpm -C apps/site build
```

Platform release work should also follow the [release guide](https://docs.milim.ai/release). The broader packaged-app and Tauri smoke path is available through `pnpm -C apps/desktop verify:tester-ready`.

Pull-request CI splits complete desktop verification between the Rust matrix and the frontend job. Release runs reuse that protected validation and add the canonical Windows runtime benchmark plus packaged-artifact checks.

## Documentation

| Topic | Reference |
|---|---|
| First run and the core desktop loop | [Quickstart](docs/wiki/quickstart.md) |
| Desktop workflows and controls | [Desktop app](docs/wiki/desktop.md) |
| Providers, local models, and runtime lanes | [Models and providers](docs/wiki/models.md) |
| Compatible endpoints and authentication | [API](docs/wiki/api.md) |
| Outbound data controls | [Privacy](docs/wiki/privacy.md) |
| Codex, Claude, OpenCode, and Pi boundaries | [Account runtimes](docs/account-runtimes.md) |
| Builds, artifacts, and verification | [Release](docs/wiki/release.md) |

The public documentation site is generated from `docs/wiki/*.md`; those pages are the canonical source for detailed behavior.

## Third-party account runtimes

Codex, Claude, OpenCode, and Pi integrations invoke separately installed tools and do not bundle their CLIs, credentials, subscriptions, or provider access. Authentication and direct provider communication remain governed by each tool and provider. milim is not affiliated with or endorsed by Anthropic, and Claude usage remains subject to Anthropic's terms.

Review the [account-runtime reference](docs/account-runtimes.md) before enabling permissive tool modes.

## License

[MIT](LICENSE)
