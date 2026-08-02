# milim

**Your local control plane for coding agents.** Use your own models and subscriptions, keep one canonical thread, review the diff, and ship.

Milim is a model-agnostic software development app built around a Tauri desktop client and an embedded Rust server. Connect hosted providers, local runtimes, or installed coding-agent CLIs; work in one persistent thread; and keep model choice, tools, approvals, memory, previews, and Git review under one roof.

[Website](https://milim.ai/) · [Documentation](https://docs.milim.ai/) · [Latest release](https://github.com/oshtz/milim/releases/latest) · [Source](https://github.com/oshtz/milim)

Release artifacts target Windows and macOS. Linux packaging is intentionally disabled, but the Rust server and Tauri app remain source-buildable on supported platforms.

## What Milim does

- **Keep one canonical thread.** Workspace context, conversation, and review history remain together instead of fragmenting by provider.
- **Work from a thread inbox.** A General preference can show active threads without section pagination in a flat recent-activity view, keep quiet project context beneath each thread, and fold completed threads into a Settled footer without archiving them.
- **Hot-swap models.** Switch the next turn between hosted providers, local Ollama or LM Studio models, and separately installed Codex, Claude, OpenCode, or Pi runtimes.
- **Keep local control.** Project selection, model routing, tool approvals, local persistence, and outbound privacy boundaries stay explicit.
- **Approve execution, then review the result.** Review is the new-chat default for consequential tool calls; configured defaults may opt into Open, while changing workspace folders resets approval to Review. Built-in Git views keep resulting diffs, checkpoints, and recovery beside the thread.

### Power tools

Agents, Workers, skills, schedules, MCP servers and Apps, media generation, Google Workspace, previews, and the mobile companion remain available from the app's collapsed **Tools** section. They extend the core thread without competing with the default workflow.

Milim keeps provider-backed chat and installed account runtimes distinct. Provider models use Milim's tool-agent loop, while account runtimes retain their native sessions and tools behind the same visible approval policy. Open gives host tools and supported account runtimes unrestricted filesystem and command access, keeps the selected folder only as the working directory, starts eligible worker plans immediately, and clears ordinary pending tool approvals; connector input and authorization remain interactive. Managed read-only Workers inherit unrestricted host reads in Open, while write-review Workers still use isolated Git worktrees. Changing the selected model affects the next turn without turning each model into a separate project history.

The desktop consumes one versioned `HarnessEvent` stream for Codex, Claude, OpenCode, and Pi. A terminal event finishes the visible turn immediately while native cleanup drains in the background; runtime-specific account, model, quota, import, update, worker, and recovery behavior remains intact behind that boundary.

Connected Codex and Claude histories can be imported by project, as selected individual chats, or as the complete no-project group; existing imports are opened instead of duplicated.

Review decisions are tracked through runtime delivery and acknowledgement. A runtime that rejects, drops, or fails to resume after a decision ends the turn with a visible recovery error instead of leaving it Running indefinitely.

Failed or canceled native-runtime turns discard that runtime's native session before the next send, preventing a partially persisted prompt from being replayed into divergent history.

Completed assistant responses retain their provider or account runtime and model in the transcript footer. Tool inputs remain fully copyable from collapsed activity even when the row is visually clipped.

Git and preview review comments are sent as structured context with the next user turn for provider models and account runtimes. Live work drawers start collapsed, and approval lifecycle rows settle when the turn ends instead of remaining animated.

## Get started

### Install the desktop app

Download the Windows portable EXE or macOS universal DMG from the [latest GitHub release](https://github.com/oshtz/milim/releases/latest). The desktop app embeds the server, so normal desktop use does not require a separate `milim serve` process.

On first run, choose a runtime, optionally choose a workspace folder, and open Milim. The two advances focus the composer without sending a task. Preferences and power tools remain available after setup. See the [full quickstart](https://docs.milim.ai/quickstart) for the safe-change, resulting-diff, and same-thread runtime-switch loop.

### Run the desktop app from source

From the repository root:

```powershell
corepack enable
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

Desktop Tauri commands load an ignored repository-root `.env.local` when it exists. Provider credentials can also be added through the app and are stored in encrypted local state.

### Run the CLI server

The standalone `milim` server exposes OpenAI-, Ollama-, and Anthropic-compatible routes over a local HTTP API. Point `MILIM_REMOTE_BASE_URL` at an OpenAI-compatible backend such as Ollama, LM Studio, vLLM, OpenAI, or OpenRouter:

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

OpenAI-compatible clients can use `http://127.0.0.1:7377/v1`. Milim does not ship a GGUF inference runtime; local inference remains the responsibility of the configured Ollama, LM Studio, or other compatible server. See the [API reference](https://docs.milim.ai/api) for supported route groups, authentication, and examples.

## Architecture

| Part | Responsibility |
|---|---|
| Desktop app | Tauri 2 with Vite, React, and TypeScript; provides chat, model and provider management, approvals, previews, Git review, settings, and local persistence. |
| Embedded server | The Axum server runs in-process with the desktop app and is also exposed by the Rust CLI for standalone API use. |
| Local data | Threads, settings, memories, schedules, provider records, and runtime state live under the Milim data directory; updates flush pending thread state before replacing the app, and secrets use OS-backed or encrypted local storage. |
| Model sources | Hosted providers, local OpenAI-compatible runtimes, and separately installed account-runtime CLIs connect through explicit routing and privacy boundaries. |

The [docs overview](https://docs.milim.ai/) includes the source map for the server router, desktop API client, Tauri host, session state, and account-runtime reference.

## Development

Use the smallest relevant check for the area changed:

```powershell
# Rust workspace
cargo test
cargo clippy --workspace --all-targets

# Desktop
pnpm -C apps/desktop verify
pnpm -C apps/desktop verify:tester-ready

# Focused runtime contract and Windows canonical-thread benchmark
pnpm -C apps/desktop verify:runtime-conformance
pnpm -C apps/desktop perf:canonical

# Site and documentation
pnpm -C apps/site build
```

`verify:tester-ready` includes the broader release and Tauri smoke path. Routine desktop changes normally use `pnpm -C apps/desktop verify`; platform release work should also follow the [release guide](https://docs.milim.ai/release).

`verify:runtime-conformance` exercises deterministic local runtime-contract fixtures. On Windows, `perf:canonical` builds a debug Tauri binary and records the mock-backed canonical-thread benchmark.

## Documentation

| Topic | Reference |
|---|---|
| First run and the core desktop loop | [Quickstart](docs/wiki/quickstart.md) |
| Desktop workflows and controls | [Desktop app](docs/wiki/desktop.md) |
| Providers, local models, and runtime lanes | [Models and providers](docs/wiki/models.md) |
| Compatible endpoints and authentication | [API](docs/wiki/api.md) |
| Outbound data controls | [Privacy](docs/wiki/privacy.md) |
| Codex, Claude, OpenCode, and Pi integration boundaries | [Account runtimes](docs/account-runtimes.md) |
| Builds, artifacts, and verification | [Release](docs/wiki/release.md) |

The public documentation site is generated from `docs/wiki/*.md`; those pages are the canonical source for detailed behavior.

## Third-party account runtimes

Codex, Claude, OpenCode, and Pi integrations invoke separately installed tools and do not bundle their CLIs, credentials, subscriptions, or provider access. Authentication and direct provider communication remain governed by each tool and provider. Milim is not affiliated with or endorsed by Anthropic, and Claude usage remains subject to Anthropic's terms. Review the [account-runtime reference](docs/account-runtimes.md) before enabling permissive tool modes.

On Windows, Milim refreshes the current system and user executable search path at startup, so account runtimes and host tools installed after sign-in are available without launching Milim from a terminal.

## License

[MIT](LICENSE)
