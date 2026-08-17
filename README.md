# milim

**Your local control plane for coding agents.** Use your own models and subscriptions, keep one canonical thread, review the diff, and ship.

Milim is a model-agnostic software development app built around a Rust-owned agent runtime with interchangeable desktop and mobile clients. Connect hosted providers, local runtimes, or installed coding-agent CLIs; work in one persistent thread; and keep model choice, tools, approvals, memory, previews, and Git review under one roof.

[Website](https://milim.ai/) · [Documentation](https://docs.milim.ai/) · [Latest release](https://github.com/bildhaus/milim/releases/latest) · [Source](https://github.com/bildhaus/milim)

Release artifacts target Windows and macOS. Linux packaging is intentionally disabled, but the Rust server and Tauri app remain source-buildable on supported platforms.

## What Milim does

- **Keep one canonical thread.** Workspace context, conversation, attached files, and review history remain together instead of fragmenting by provider. Add files from the picker, clipboard, or by dropping them onto the composer. Scrolling back pauses live following and exposes a Latest control; live assistant answers retain Markdown formatting as they grow, and completed assistant links show their source identity and destination on hover or focus.
- **Put thread navigation where it fits.** General settings can keep the inset left sidebar, merge navigation into the window's top bar, or place one always-visible floating rail below the chat. Horizontal project and inbox groups open into scrollable thread lists without changing thread actions or ordering.
- **Work from a thread inbox.** A General preference can show active threads without section pagination in a flat recent-activity view, keep quiet project context beneath each thread, and fold completed threads into a Settled footer before they can be archived. The native taskbar or Dock badge counts distinct chats with unread updates or unresolved approvals.
- **Hot-swap models.** Switch the next turn between hosted providers, local Ollama or LM Studio models, and separately installed Codex, Claude, OpenCode, or Pi runtimes. Reasoning effort chosen inside a chat stays with that chat; new chats inherit app-wide model defaults instead of copying another chat's overrides.
- **Keep local control.** Project selection, model routing, tool approvals, local persistence, and outbound privacy boundaries stay explicit.
- **Control work from mobile.** The bare React Native iOS/Android companion pairs directly with one or more running desktops over Tailscale, opt-in LAN discovery, or a manual URL. Its pairing page automatically lists desktops by their OS computer names when they are advertised on the trusted local network; tap one, approve the named device on desktop, and mobile completes credential setup without copying a link. Desktop Settings proactively reports whether Tailscale is missing, disconnected, needs remote-access configuration, or is ready. Configuring Tailscale only prepares the private network path; it does not pair a device. QR/deep links remain the explicit fallback when discovery is blocked or the desktop is remote, and unreachable Tailscale connections prompt users to check both devices and retry. Development builds also detect the host desktop from an Android emulator or iOS simulator through the preferred local LAN port. The paged canonical timeline projects messages, grouped work, live and completed tool rows, file-change summaries, failures, and approval context into one ordered transcript; actionable approvals also remain aggregated under Attention. Each host publishes its active milim appearance, authenticated custom background, fit/treatment effects, glass surfaces, borders, radii, and typography; mobile preserves that visual system in a thumb-friendly single column and maps desktop font stacks to their available native equivalents. Mobile-owned product copy and desktop-supplied host labels consistently render the `milim` brand in lowercase. A header-opened project drawer mirrors desktop thread grouping without consuming a navigation tab, while the composer stays expanded at the latest turn or while active and collapses into a translucent message pill only when reading earlier content. Expanding that pill preserves the reader's position; selecting Latest keeps the pill compact during the return, then expands it once while keeping the final transcript lines clear above it. Sending goes directly to the bottom. Native searchable model and Agent sheets preserve the desktop picker hierarchy and capability context. milim operates no relay, account service, cloud transcript store, or v1 push service.
- **Choose models consistently on mobile.** The native model sheet reuses desktop provider/runtime brands, compact capability glyphs, favorites, collapsible groups, and thread-scoped reasoning effort controls. Local catalog entries whose upstream owner is unavailable are labeled as local models with their publisher namespace instead of appearing as a Milim provider. Connected phones refresh automatically when desktop provider catalogs change, including LM Studio models appearing after a connection test or provider refresh.
- **Approve execution, then review the result.** Review is the new-chat default for consequential tool calls; configured defaults may opt into Open, while changing workspace folders resets approval to Review. Built-in Git views keep resulting diffs, checkpoints, and recovery beside the thread.
- **Inspect and edit the workspace.** Code provides a searchable file rail and one focused editor with explicit, conflict-safe saves; generated artifacts remain available as read-only sources. Threads using the same effective directory share whether the Inspector is open and its selected tab, while the Inspector's contents remain thread-specific. Resizable desktop panes share one seam-centered handle and pointer target, including horizontal drawers and deliberately spaced side-by-side surfaces.
- **Render structured output inline.** The built-in `render_chart` tool places responsive, interactive native bar, line, pie, and scatter charts, including horizontal bars, directly in the transcript, while completed fenced `mermaid` blocks become theme-aware diagrams with source, image-copy, SVG, and PNG actions. Neither path requires an MCP App server.

### Power tools

Agents, Workers, skills, schedules, MCP servers and Apps, media generation, Google Workspace, previews, and the mobile companion remain available from the app's **Tools** launcher. Account runtimes can start project previews through Milim's managed runtime and open HTTPS or loopback URLs directly in the in-app Preview inspector; managed dev servers remain available between turns, and loopback reloads bypass the browser cache so local CSS and JavaScript changes stay current while an agent works. These tools extend the core thread without competing with the default workflow.

Milim keeps provider-backed chat and installed account runtimes distinct. Provider models use Milim's tool-agent loop, while account runtimes retain their native sessions and tools behind the same visible approval policy. Open gives host tools and supported account runtimes unrestricted filesystem and command access, keeps the selected folder only as the working directory, starts eligible worker plans immediately, and clears ordinary pending tool approvals; connector input and authorization remain interactive. Managed read-only Workers inherit unrestricted host reads in Open, while write-review Workers still use isolated Git worktrees. Changing the selected model affects the next turn without turning each model into a separate project history.

Rust's `RunManager` owns accepted turns, per-thread queues, approvals, cancellation, frozen run configuration, and the durable normalized timeline. The desktop and mobile apps are replicas of that state, so hiding the Tauri window or reloading its renderer does not pause work. Different threads may run concurrently; a single thread runs one turn at a time. Explicit Quit and restart cancel active runs and close Milim-owned MCP, agent-runtime, host-shell, sandbox, and preview process trees. A restart marks unfinished runs and approvals interrupted instead of replaying them.

Codex, Claude, OpenCode, and Pi continue through one versioned `HarnessEvent` vocabulary behind reusable runtime adapters. Existing harness and runtime-specific routes remain compatibility facades while `/control/v1` is the authoritative desktop/native-mobile control contract.

Every canonical run now has a local, privacy-processed run ledger. Milim commits the exact provider-visible request before a remote model call, then commits responses, usage, tool calls, approvals, model-visible tool results, and claimed steering/context before another model step can begin. The ordinary transcript stays unchanged: ledger data is fetched only when **Run details** is selected inside an already-open work drawer, where Composition, Model steps, Tools, Inbox, and failures remain nested and raw bodies stay collapsed. Account runtimes are labeled `harness_boundary` because Milim records only what crosses its adapter boundary, not runtime-internal prompts.

Busy Send still creates a durable follow-up. A steer-capable Milim provider-agent run additionally exposes **Steer next step** in the existing secondary composer actions; native account runtimes and plain compatibility calls do not advertise steering. Context injection is durable but never wakes or extends an idle run.

Connected Codex and Claude histories can be imported by project, as selected individual chats, or as the complete no-project group; existing imports are opened instead of duplicated.

Review decisions are tracked through runtime delivery and acknowledgement. A runtime that rejects, drops, or fails to resume after a decision ends the turn with a visible recovery error instead of leaving it Running indefinitely.

Failed or canceled native-runtime turns discard that runtime's native session before the next send, preventing a partially persisted prompt from being replayed into divergent history.

Completed assistant responses retain their provider or account runtime and model in the transcript footer. Command rows visually replace the current workspace with `.` and unwrap recognized shell launchers, while their tooltip and copy action preserve the exact original input.

Git and preview review comments are sent as structured context with the next user turn for provider models and account runtimes. Completed turns fold intermediate assistant updates, reasoning, and ordinary tool activity into one duration-labeled work drawer whose summary retains the last meaningful action, primary detail, step counts, and compact diff stats while keeping the final response and structured outputs visible. Live work drawers open automatically only while their newest activity is a failure, collapse after recovery or turn completion, and remain manually expandable with failure details intact. Approval lifecycle rows settle when the turn ends instead of remaining animated.

Tool calls enter one fixed execution pipeline. Mutating, command, unknown, and external MCP calls are exclusive; only explicitly parallel-safe reads may overlap, with four calls per run and sixteen process-wide. The default deadline is 120 seconds. Host shell and account-runtime processes inherit the user environment, while configured MCP processes and sandbox payloads receive sanitized environments. Review approval copy warns when an inherited host environment may expose developer credentials.

## Get started

### Install the desktop app

Download the Windows portable EXE or macOS universal DMG from the [latest GitHub release](https://github.com/bildhaus/milim/releases/latest). The desktop app embeds the server, so normal desktop use does not require a separate `milim serve` process.

Milim runs as a single desktop instance. Launching it again restores and focuses the existing window, including when that window is hidden in the system tray.

On first run, choose a runtime, optionally choose a workspace folder, and open Milim. The two advances focus the composer without sending a task. Preferences and power tools remain available after setup. See the [full quickstart](https://docs.milim.ai/quickstart) for the safe-change, resulting-diff, and same-thread runtime-switch loop.

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

Run iOS from macOS with CocoaPods and the `apps/mobile/ios/MilimMobile.xcworkspace` workspace. Pairing and the control API require the Milim desktop process to remain alive.

Mobile transcripts render Markdown links and pasted web URLs as tappable links in both user and assistant messages.

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
| Desktop app | Tauri 2 with Vite, React, and TypeScript; presents a local replica of canonical thread/run state plus desktop-only previews, Git review, and settings. |
| Native mobile app | Bare React Native with TypeScript and Metro for iOS and Android; a bounded, multi-host controller/cache that connects directly to paired desktops. |
| Embedded server | The in-process Axum server and Rust `RunManager` own active turns, queues, approvals, normalized events, and compatibility adapters. No independent OS daemon is installed. |
| Local data | Desktop SQLite is authoritative for threads, frozen runs, timelines, queues, command receipts, and approvals. Mobile SQLite is a host-partitioned bounded cache; secrets use OS-backed or encrypted storage. |
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

# Native mobile protocol, reducer, lint, and types
pnpm -C apps/mobile verify

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

On macOS and Linux, Milim resolves symlinked CLI launchers to their installed executable before startup so app-bundled runtimes can find companion executables beside the real binary.

## License

[MIT](LICENSE)
