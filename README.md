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

Use Node 22.13 or newer with the repository-pinned pnpm 9.15.9, then run from the repository root:

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
| Native mobile app | Bare React Native with TypeScript and Metro. Acts as a bounded, multi-host controller and cache for paired desktops; live events and transcript projection are frame-coalesced and incremental, binary attachments use a native streamed upload when supported, and thread height follows the native keyboard so the composer and followed transcript tail remain visible. |
| Embedded server | In-process Axum server and Rust `RunManager`. Owns active turns, queues, approvals, normalized events, and runtime adapters. |
| Local data | Desktop SQLite is authoritative for threads, reciprocal thread links, mailbox exchanges, runs, timelines, queues, approvals, and favorite model IDs. Mobile SQLite is a host-partitioned cache for timelines, drafts, and client-only picker layout. |
| Model sources | Hosted providers, local OpenAI-compatible servers, and separately installed account-runtime CLIs connect through explicit routing and privacy boundaries. |

Desktop and mobile are replicas of Rust-owned canonical state. Model and reasoning changes, message deletion, thread branching, favorites, queues, approvals, and linked-chat exchanges commit there before clients reconcile. Accepted work survives a hidden or reloaded desktop window; quitting or restarting interrupts unfinished runs.

- **Conversation and review.** Long histories load in pages and keep a bounded number of rows mounted while preserving the reader's position. Model switches retain the thread and add a quiet history notice. Attach supported files and images, inspect effective context before sending, steer a compatible active run, or queue a follow-up. Provider run limits can bound further model and tool steps. See [desktop workflows](docs/wiki/desktop.md), [history loading](docs/wiki/desktop.md#desktop-performance-and-history-loading), and [models and providers](docs/wiki/models.md).
- **Workspaces and previews.** Folder-backed chats share one managed App runtime per workspace. Preview actions stay bound to the originating chat; App and URL previews retain their own zoom, mute, and navigation state. An optional `.milim/preview.json` specifies the launch command and readiness checks. See [artifacts and previews](docs/wiki/desktop.md#artifacts) and [Git review](docs/wiki/desktop.md#git-side-panel).
- **Agents and continuity.** Linked chats exchange bounded reads and durable messages. Scheduled occurrences freeze their model, workspace, privacy, time zone, and approval boundary and appear on both clients. Codex, Claude, OpenCode, and Pi retain native session bindings and sync only intervening conversation; Codex receives frozen instructions through its native developer channel. See [linked chats](docs/wiki/desktop.md#linked-chats-and-mailbox), [Agents and schedules](docs/wiki/agents.md), and [account runtimes](docs/account-runtimes.md).
- **Local data and privacy.** Canonical messages retain usage and cost provenance, and the local run ledger supports inspection without loading diagnostics into the ordinary transcript. Privacy gates cover outbound text and structured tool data; backups use atomic file replacement, support files larger than 64 MiB, and require an idle runtime before restore. Memory has explicit review, archive, restore, deletion, and retrieval benchmarks. See [privacy and recovery](docs/wiki/privacy.md) and [memory](docs/wiki/memory.md).
- **Mobile recovery.** Paired phones use the same desktop model catalog and canonical favorites, with local caches and drafts partitioned by desktop. Foreground reconnect catches up from canonical state without sending drafts or decisions. An uncertain command result exposes an explicit retry with the original ID and desktop association. See [pairing and foreground synchronization](docs/wiki/voice-media-mobile.md#mobile-companion).

App-wide Custom instructions apply to newly accepted turns; repository `AGENTS.md` and `CLAUDE.md` remain workspace-scoped. Detailed attachment limits, approval policies, notification counts, preview lifecycles, steering, and shutdown behavior live in the [desktop reference](docs/wiki/desktop.md).

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

Pull-request CI splits complete desktop verification between the Rust matrix, frontend job, and a Windows WebView2 close-to-tray check. Release runs reuse that protected validation and add the canonical Windows runtime benchmark plus packaged-artifact checks.

## Documentation

| Topic | Reference |
|---|---|
| First run and the core desktop loop | [Quickstart](docs/wiki/quickstart.md) |
| Desktop workflows and controls | [Desktop app](docs/wiki/desktop.md) |
| Providers, local models, and runtime lanes | [Models and providers](docs/wiki/models.md) |
| Compatible endpoints and authentication | [API](docs/wiki/api.md) |
| Outbound data controls and backup recovery | [Privacy](docs/wiki/privacy.md) |
| Pairing, mobile control, and reconnection | [Media and mobile](docs/wiki/voice-media-mobile.md) |
| Codex, Claude, OpenCode, and Pi boundaries | [Account runtimes](docs/account-runtimes.md) |
| Builds, artifacts, and verification | [Release](docs/wiki/release.md) |

The public documentation site is generated from `docs/wiki/*.md`; those pages are the canonical source for detailed behavior.

## Third-party account runtimes

Codex, Claude, OpenCode, and Pi integrations invoke separately installed tools and do not bundle their CLIs, credentials, subscriptions, or provider access. Authentication and direct provider communication remain governed by each tool and provider. milim is not affiliated with or endorsed by Anthropic, and Claude usage remains subject to Anthropic's terms.

Review the [account-runtime reference](docs/account-runtimes.md) before enabling permissive tool modes.

## License

[MIT](LICENSE)
