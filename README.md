# milim

**Your local control plane for coding agents.** Use your own models and subscriptions, keep one canonical thread, review the diff, and ship.

Milim embeds a Rust backend with a local HTTP API and brings provider-agnostic dev chat, encrypted provider management, Codex, Claude CLI, OpenCode, and Pi account runtimes, opt-in isolated thread worktrees, review comments, an in-app GitHub PR cockpit, agents/tools, memory/RAG, artifact previews, and outbound privacy controls into one app.

Desktop release artifacts currently target Windows and macOS. Linux packaging is intentionally disabled, but the Rust server and Tauri app remain source-buildable on supported platforms.

## Quickstart

```bash
# build the Rust workspace
cargo build --release

# run the server with an OpenAI-compatible local endpoint, on port 7377
MILIM_REMOTE_BASE_URL=http://localhost:11434/v1 cargo run -p milim-cli -- serve

# in another shell
cargo run -p milim-cli -- status
```

## Usage

Run `milim serve` to expose the local HTTP API, then point OpenAI-compatible clients at `http://127.0.0.1:7377/v1`. Configure the CLI backend with `MILIM_REMOTE_BASE_URL` and optional `MILIM_REMOTE_API_KEY`; for local inference, point it at an external runtime such as Ollama or LM Studio.

For the desktop app:

```bash
corepack enable
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

### CLI Commands

| Command | Description |
|---|---|
| `serve [--port N] [--expose]` | Start the HTTP server; `--expose` binds on the LAN and auto-enables `msk-v1` auth if no auth is configured. |
| `status [--url URL] [--port N] [--token T] [--json]` | Probe a running server's health. |
| `models [--url URL] [--port N] [--token T] [--json]` | List models from a running server. |
| `run [--url URL] [--port N] [--token T] [--system S] [--temperature N] [--max-tokens N] <model> [prompt...]` | Chat through the running server: one-shot if a prompt is given, else an interactive REPL. |
| `keys identity` / `keys mint [...]` | Print this machine's address / mint an `msk-v1` access token. |
| `mcp [--url URL] [--port N] [--token T]` | Run the stdio MCP bridge for Claude Desktop and other MCP clients. |
| `version` | Print the version. |

OpenAI-compatible example:

```bash
curl http://127.0.0.1:7377/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"llama3.2","messages":[{"role":"user","content":"hello"}],"stream":true}'
```

Ollama clients can use `http://127.0.0.1:7377/api/chat` or prompt-style `http://127.0.0.1:7377/api/generate`; `/api/chat` accepts `think` and returns `message.thinking` for reasoning models, and empty-prompt `keep_alive` lifecycle calls are forwarded to native Ollama backends when available.

## Backends

The CLI server chooses a backend from the environment. If none is configured, `/v1/models` returns an empty list and chat requests return a setup error instead of a synthetic response.

| Backend | Trigger | Notes |
|---|---|---|
| **Remote / local provider** | `MILIM_REMOTE_BASE_URL` (+ `MILIM_REMOTE_API_KEY`) | Forwards to any OpenAI-compatible endpoint such as Ollama `/v1`, LM Studio `/v1`, vLLM, OpenAI, or OpenRouter. |

Milim does not ship a GGUF runtime. Local GGUF inference belongs in dedicated runtimes such as Ollama or LM Studio; Milim integrates with them through their API surfaces. Ollama thinking models can use Milim reasoning effort controls through `/v1/chat/completions`; LM Studio models with native reasoning metadata use LM Studio's `/api/v1/chat` reasoning settings when no custom tools are attached and `/v1/responses` for Milim function-tool requests, while `gpt-oss` low/medium/high effort keeps using LM Studio's `/v1/responses` path. Structured-output controls are forwarded through OpenAI `response_format`, Ollama `format`, and Responses `text.format` where the selected backend supports them. The desktop app manages encrypted provider records for OpenAI, OpenRouter, Groq, Anthropic, Gemini, Replicate, fal, local Ollama, local LM Studio, and custom OpenAI-compatible endpoints; duplicate provider model ids stay provider-scoped in the picker and route back to the selected provider. Codex, the installed Claude CLI, OpenCode, and Pi are account runtimes backed by user-installed CLIs; they are separate from saved provider records, can be enabled or disabled independently in Providers, appear in the model picker after authentication when enabled, and use Milim approval modes with visible tool events. Pi exposes its own multi-provider catalog—including subscription-backed providers such as GitHub Copilot—and resumable JSONL RPC sessions. Codex additionally exposes exact permission/MCP elicitation approvals and one-time chat recovery through app-server history APIs.

OpenRouter requests identify the app as `milim` using `https://milim.ai/` for OpenRouter analytics and attribution.

At desktop startup, the picker loads the cached catalog while one live refresh checks enabled chat providers, then reconciles automatically when that refresh finishes. Provider, Codex, Claude, OpenCode, and Pi discovery fail independently so one unavailable runtime does not hide the others; failed enabled account-runtime probes retry once after the embedded backend and CLI have had time to start.

Milim does not include Claude Code, does not provide Anthropic credentials, and is not affiliated with or endorsed by Anthropic. This integration only invokes the user's separately installed official Claude CLI on the local machine. Use of Claude and Claude Code is governed by Anthropic's terms. Users must install the Claude CLI separately, authenticate through Claude's own tooling, and Milim does not manage or receive Claude credentials.

Claude CLI integration boundaries:

- Milim invokes the local `claude` executable.
- Milim does not bundle Claude Code, proxy Claude access, or sell Claude access.
- Authentication is handled by the official Claude CLI.
- Milim does not read Claude credentials.
- Claude CLI usage remains subject to Anthropic's terms.
- Open mode for Claude CLI maps to Claude's `bypassPermissions` mode, which may run tools and commands without additional Claude prompts; use it only in trusted workspaces.
- Stale-session recovery asks before stopping a matching local Claude CLI process and does not delete Claude session registry files by default.

Pi is also installed and authenticated separately. Run `/login` inside Pi; Milim never reads or stores Pi credentials. Embedded Pi turns use the documented JSONL RPC protocol with exact provider/model selection and resumable Milim-owned session ids. They disable discovered extensions so arbitrary startup code cannot bypass Milim safety: Plan/Guarded are read-only, Review uses a temporary exact-arguments approval extension, and Open exposes Pi's built-in tools without prompts. Pi's normal context files, prompt templates, and skills remain available for folder-backed chats; no-folder chats run without tools or project context.

OpenCode no-folder chats use a private managed ACP directory for protocol compatibility with native filesystem tools disabled; Milim-owned tools remain available.

## API Surface

| Area | Endpoint(s) |
|---|---|
| OpenAI chat/responses/completions/models/embeddings | `POST /v1/chat/completions` (`/chat/completions`), `POST /v1/responses`, `POST /v1/completions` (`/completions`), `GET /v1/models` (`/models`), `POST /v1/embeddings` (`/embeddings`) |
| Ollama chat/generate/tags/embeddings | `POST /api/chat`, `POST /api/generate`, `GET /api/tags`, `POST /api/embed`, `POST /api/embeddings` |
| Anthropic messages | `POST /anthropic/v1/messages`, `POST /anthropic/messages`, `POST /v1/messages` |
| Provider registry | `GET/POST /providers`, `GET /providers/discover`, `DELETE /providers/{id}` |
| OpenCode account runtime | `GET /opencode/status`, `GET /opencode/models`, `POST /opencode/run` |
| Account runtimes | Codex, Claude, OpenCode, and Pi status/run endpoints; `GET /account-runtimes/updates`, `POST /account-runtimes/{runtime}/update` |
| Media generation and library | `GET /media/models`, `GET /media/model-schema`, `GET /media/status`, authenticated `GET /media/content`, `POST /media/generate`, `GET /media/library`, `POST /media/library/{id}/refresh`, `GET /media/library/{id}/content/{index}`, `DELETE /media/library/{id}` |
| MCP | `GET /mcp/tools`, `POST /mcp/call`, host-only `POST /mcp/apps/resources/read` and `POST /mcp/apps/tools/call`, ephemeral `GET /mcp/apps/views/{id}`, `GET/POST /mcp/servers`, `POST /mcp/servers/test`, `POST /mcp/servers/{id}/test`, `DELETE /mcp/servers/{id}` |
| Agents and Worker Runs | `POST /agents/run`, `GET/POST /agents`, `GET/PUT/DELETE /agents/{id}`, `POST /agents/{id}/run`, preferred `/worker-runs/*` including task retry, and compatibility `/threads/*` routes |
| Memory / RAG | `POST /memory/ingest`, `/memory/search`, `/memory/register`, scoped hybrid `/memory/graph/search`, plus scope/node routes |
| Privacy filter | `POST /privacy/scan`, `GET/POST /privacy/mode` |
| Skills and schedules | `GET/POST /skills`, `POST /skills/select`, `GET/PUT/DELETE /skills/{id}`, `GET/POST /schedules`, `GET /schedules/events`, `PUT/DELETE /schedules/{id}` |
| Workspace and tools | `GET/POST /workspace`, `GET /workspace/git`, `POST /workspace/git/action`, `GET/POST /computer`, `POST /sandbox/run` |
| Managed preview apps | `GET /preview-apps/{runtime_id}`, read-only `POST /preview-apps/{runtime_id}/preflight`, compatibility `POST /preview-apps/{runtime_id}/stage`, explicit `POST /preview-apps/{runtime_id}/start`, `POST /preview-apps/{runtime_id}/stop`, `POST /preview-apps/{runtime_id}/restart`, and cursor-based `GET /preview-apps/{runtime_id}/logs?after_seq=` |
| Google Workspace (desktop-internal) | Authenticated connection status, system-browser Picker, selected-file registry, honest confirmed/unconfirmed disconnect results, MIME-aware preview, and bounded media proxy routes under `/google-workspace/*` |
| Mobile companion | `GET /mobile`, PWA assets under `/mobile/*`, `GET /mobile/status`, `POST /mobile/enabled`, `POST /mobile/pairing`, `POST /mobile/pair`, `GET /mobile/device/status`, `POST /mobile/relay`, `GET/POST /mobile/thread`, `GET /mobile/thread/events`, `GET /mobile/events`, `DELETE /mobile/devices/{id}` |
| Codex account runtime | `GET /codex/account`, `POST /codex/login/device`, `/codex/login/chatgpt-device`, `/codex/login/api-key`, `/codex/logout`, `GET /codex/rate-limits`, `GET /codex/models`, read-only `GET /codex/threads` and `/codex/threads/{id}`, `POST /codex/run` |
| Installed Claude CLI | `GET /claude/status`, `POST /claude/run` |
| Health | `GET /health` |

Built-in tool safety is enforced server-side. Each run captures its workspace and preview target. Plan is read-only, Guarded exposes only tools declared read-only, Review pauses each mutating, command, or unknown invocation on an inline exact-arguments Approve/Deny card, and Open executes eligible calls automatically. Review approvals are unguessable, one-shot, and canceled when the stream stops; the legacy whole-run grant remains available only for explicit non-interactive API callers. Direct `POST /mcp/call` is read-only even though `GET /mcp/tools` returns the full catalog for agent configuration. The desktop resets tool approval to Guarded whenever a chat's workspace folder changes, so Open permission does not carry across project boundaries. Filesystem tools reject traversal through symlinks/junctions, use atomic replacement for writes, and support bounded ranged reads. `http_fetch` permits public HTTP(S) destinations only. Host and Docker commands have time/output limits; Docker runs also use no-network, memory, CPU, PID, capability, and read-only-root restrictions by default.

Desktop release builds can enable selected-file Google Workspace access with `MILIM_GOOGLE_OAUTH_CLIENT_ID` and `MILIM_GOOGLE_OAUTH_CLIENT_SECRET`. The desktop OAuth client uses PKCE and Google's system-browser desktop Picker; its installed-app client secret is build configuration and is never a user credential. The connector requests only `drive.file` and stores one refresh token plus the selected/Milim-created file registry in AES-GCM encrypted local state. Picker supports selecting many files in one confirmation; selecting a folder itself is intentionally disabled because Google does not extend `drive.file` authorization to its children. Folder URLs open Picker filtered to that folder so its files can be selected together; selected folder entries open in the active Milim tab, middle-click or Ctrl/Cmd-click opens a new Milim tab, and the context menu exposes open/new-tab/copy/external actions. The trailing external-link action alone opens Google directly. Native Sheets, Docs, and Slides previews support inline manual editing with autosave after a short idle period and on blur; Sheets also support pasted ranges and row/column insertion or confirmed deletion, while Slides add a collapsible thumbnail rail and clean fullscreen presentation mode with keyboard navigation. The first parentless create or upload lazily creates a Milim Drive folder; later parentless Milim-created files go there and are automatically added to the registry. Provider models and Codex, Claude, OpenCode, and Pi account runtimes receive the same bounded Drive, Sheets, Docs, and Slides tools through Milim's per-turn gateway. Reads follow Guarded, edits and Drive management are mutating, and workspace transfers are commands. Files removed from Milim remain in Drive, deletion is trash/restore only, sharing is limited to named users/groups, and Google credentials plus the file registry are excluded from backups.

The Sheets inspector uses one compact controls row, an editable autosaving formula bar, contextual row/column actions, and bottom worksheet tabs so the grid keeps most of the inspector height.

Workspace turns reload repository instructions every turn. Milim-native loads Codex-style `AGENTS.md`/`AGENTS.override.md` plus Claude `CLAUDE.md` and unconditional `.claude/rules/**/*.md`; Codex receives only the Claude-family additions and Claude receives only the AGENTS-family additions so native discovery is not duplicated. The authenticated Context API exposes ordered sources and warnings. Git remotes are sanitized into stable project identities, so clones and worktrees share Project memory while legacy exact-folder memories remain readable.

Milim is also an inline [MCP Apps](https://github.com/modelcontextprotocol/ext-apps/blob/main/specification/2026-01-26/apps.mdx) host for negotiated stdio servers. A model-visible MCP tool can attach an advertised `ui://` view, which Milim renders at that tool call and connects back only to app-visible tools on the same server. Apps are server-authored live views; artifacts remain model-authored files/content and keep their existing panel and revision workflow. App descriptors and initial results persist, remote HTML does not, and reopening re-fetches the resource or leaves the textual tool result as fallback. Validated HTML is exposed only through a random, short-lived, memory-only view URL with a generated CSP; the iframe receives no bearer token.

Standalone auth is configured in `~/.milim/config/server.json`: set `authRequired: true` to accept locally minted `msk-v1` keys, add static bearer secrets under `apiKeys`, or trust extra signed-key issuers under `accessKeyIssuers`. `milim serve --expose` persists `authRequired: true` and prints an immediate `MILIM_API_TOKEN` when no auth is already configured. CLI client commands accept `--url`, `--port`, and `--token` (`MILIM_API_TOKEN`) when calling an authenticated server. The desktop app disables loopback trust and uses a per-launch bearer token for its embedded server. Empty CORS allow-list means no browser origins are allowed; set explicit origins at `~/.milim/config/server.json` when needed.

Desktop provider keys, Google authorization, MCP secrets, and mobile-companion device keys are encrypted with one per-data-root master key held in Windows Credential Manager or macOS Keychain. If the OS vault is unavailable, Milim reports and uses a permission-restricted local key. The first launch after this storage upgrade migrates and removes the older adjacent key files and plaintext mobile-device state only after verified re-encryption; that credential migration is intentionally one-way.

## Workspace Layout

```text
crates/
  milim-core/       serde DTOs, config, paths, errors
  milim-inference/  ModelService trait plus remote, Anthropic, Gemini, and test backends
  milim-storage/    SQLite persistence plus AES-GCM encryption
  milim-identity/   secp256k1 identity plus msk-v1 access keys
  milim-tools/      tool registry plus filesystem and built-in tools
  milim-agents/     tool-use Agents and durable Worker Runs
  milim-memory/     embedding vector store, scoped hybrid memory, RAG
  milim-automation/ cron-scheduled agent runs
  milim-skills/     local skill registry
  milim-privacy/    regex PII filter plus reversible redaction gate
  milim-sandbox/    Docker-backed isolated command runner
  milim-mcp-client/ external MCP stdio client bridge
  milim-server/     axum HTTP server
  milim-cli/        the `milim` binary
apps/
  desktop/          Tauri 2 + Vite/React/TypeScript desktop app
  site/             Vite/React Cloudflare Pages site for milim.ai and docs.milim.ai, with Lenis smooth scrolling
```

## Desktop App

The Tauri app embeds the server in-process; the app is the server, so there is no separate `serve` process. Closing the desktop window hides it to the system tray so the embedded backend keeps running; use the tray menu to reopen or quit.

If a desktop UI render crash reaches the app root, Milim shows a reloadable error screen instead of a blank window.

```bash
pnpm -C apps/desktop install
pnpm -C apps/desktop tauri:dev
```

Desktop Tauri commands automatically load an ignored repo-root `.env.local` when it exists. Local Google Workspace builds use the same two `MILIM_GOOGLE_*` variables documented above; GitHub release builds continue to receive them from Actions secrets.

Shipped UI:

- Unified first-run setup follows Model, Defaults, optional Context, and Ready. Advanced features are always available; the collapsed sidebar Tools launcher opens MCP servers, Skills, Schedules, Pull requests, and the media manager.
- Skills use progressive disclosure. Automatic and Custom modes rank at most three enabled candidates and send compact name/description metadata; the model loads a complete body only when needed through read-only `milim_skill_search` and `milim_skill_read` tools. Custom is an allowlist, explicit `@Skill Name` or `/Skill Name` references load that skill immediately, and prompt limits omit later skills whole instead of slicing instructions. Account runtimes keep their native skill catalogs and receive Milim-managed skills through the per-turn gateway without duplicate eager injection.
- The first product loop is dev chat: connect a provider or account runtime, pick a workspace folder, and start from the configured workspace preference or explicitly choose the current checkout or an isolated worktree from the New chat split button. Ask for an edit or test run, switch models inline, and continue the same thread without resetting workspace context, memory, previews, artifacts, approvals, or queued messages.
- Desktop motion is frequency-aware: shortcut-driven search, model, context-menu, and thread changes stay instant; truncated model and thread labels reveal themselves after a one-second hover; occasional panels use short interruptible transitions; reordered rows track the pointer directly; and reduced-motion preferences remove spatial movement and spinning while retaining perceivable status.
- After onboarding, the first startup of a release can show a short, keyboard-accessible stack of update cards sourced from the bundled `apps/desktop/src/update/releases.json`. Dismissing the stack records that exact installed version locally; Developer settings can replay the current cards without changing that state.
- Hot Swap keeps the Milim thread canonical while models change: compatible switches stay immediate, lossy handoffs show a compact preflight, stale Codex/Claude sessions offer Fresh or Resume, and the latest response can be continued, reviewed read-only, or retried with another model. Coding retries use the pre-turn Git checkpoint in an isolated worktree whose reviewed diff can be applied back to the original workspace.
- Empty chats show local, repository-aware starter prompts derived from the active branch, worktree state, and recent commits; selecting one only prefills the composer.
- Desktop profile data uses verified SQLite WAL mode. Session persistence stores only changed session/message rows, while durable Worker Runs remain canonical in `threads.db` instead of the UI snapshot.
- Chat sessions with row-backed desktop persistence, Markdown-rendered user and assistant message bodies with raw source preserved for actions, memoized live Markdown streaming for normal assistant tails with preserved-text fallback for long streams before full Markdown finalize, browser-opening message links, reasoning blocks, virtualized long transcripts, compact live tool activity with expandable details, syntax-highlighted short code snippets, collapsed generated code artifacts, attachments, a compact interruptible and reorderable queued-send tray, persisted thread-local composer drafts and history recall with transient position feedback, immediate Plan/Goal mode pills from slash autocomplete, first-message and optional AI thread titles, copy with transient confirmation, regenerate/edit-and-resend, in-place assistant edits, message deletion, branch-from-message, JSON/Markdown thread export/import, filtered search, compact sidebar thread rows with pin/archive hover actions and branch/export in the right-click menu, project rows with pin/new-chat hover actions and archive/customization in the right-click menu, and persisted project names, built-in icons, and colors that tint the project header and thread titles. Appearance can optionally assign stable contrast-safe project colors from the active theme accent. Custom right-click menus preserve native text/link editing menus; tokenizer-backed context compaction checkpoints reject truncated summaries, keep a bounded recent tail verbatim, cap old attachment/tool bodies in summary prompts, and track lifetime/since-checkpoint usage totals. Persisted sidebar/context/inspector state includes animated project/chat section collapse and five-at-a-time per-section thread reveals, response metrics that update after each built-in tool-agent model request, generated artifact previews with thread-local revision cycling, built-in scoped preview tools for the active DOM-capable preview surface, no-folder virtual project files with review/apply and versioned artifact sources, visual-only preview activity cues for tool events with click-through browser-preview overlays, native URL previews with a shared persistent/private Milim browser profile, a global persistent-mode recent-destinations dropdown, and persisted per-thread tabs for new-window requests, always-private App previews, and native surfaces that yield to overlapping app dialogs and popovers, inline/display-only handling for anonymous code and markdown tables, lightweight anonymous script/TSX preview fallbacks, managed no-folder Node preview apps from named file metadata with package-manager detection, a required `dev` script, Vite entry/style and Tailwind config fallbacks, full named-file staging within the preview size budget, and selected-folder Node preview apps that run that folder directly without staged-file rewrites. Runnable apps never execute on generation: the inspector first shows a read-only command/scope/fingerprint review, then requires an explicit Run; managed files are staged only after that confirmation. Runtime state distinguishes active from ready, survives transient polling failures, reports compile failures without discarding the last URL, and supports a cancellable install/start lifecycle. The unified Preview / Code / Git / Workers inspector keeps artifact revisions, app state, manual URL history, and Worker history independent per thread.
- Negotiated MCP Apps render interactive charts, diagrams, forms, dashboards, and viewers inline at their originating tool call. They use an opaque-origin sandbox, server-declared CSP allow-list, same-server tool/resource bridge, Review/Guarded/Open enforcement, theme and bounded resize updates, and text fallback when their server is unavailable; they never enter the artifact panel.
- Provider/local chats and account runtimes receive the visible Milim browser tab's untrusted URL/title metadata and can use Milim-owned tools through the same approval policy. Account runtimes retain their native filesystem and shell instead of receiving duplicate host tools. They can also list, test, save, enable/disable, and remove Milim-managed MCP servers: Guarded permits read-only calls, Review pauses consequential calls on the existing exact-arguments approval card, and Open executes them directly. Credential-looking environment variables are accepted only as value-free placeholders; enter their values afterward in the encrypted MCP Manager. Newly connected tools become callable on the next chat turn.
- PNG, JPEG, WebP, and GIF attachments up to 2 MB each are forwarded as real image content to provider chat, server-side agent runs, provider-backed schedules, Codex app-server turns, Claude CLI turns, and Pi RPC turns; no OCR or text fallback substitutes for the pixels. Outbound turns keep the latest images plus complete recent image-bearing turns within a 20 MiB encoded-image budget; older pixels remain visible in the transcript but become an omission note in model context. Requests above 30 MiB fail locally with removal/compaction guidance instead of reaching the server's 32 MiB cap. Codex uses temporary per-turn `localImage` files that are deleted when the turn ends, while Claude and Pi receive native base64 image blocks. Oversized, unsupported, unreadable, or legacy scheduled images without stored data fail visibly. Desktop attachment reads remain limited to files picked in the native dialog or workspace-relative files under the selected folder.
- Provider onboarding for OpenAI, OpenRouter, Groq, Ollama, LM Studio, custom OpenAI-compatible remotes, Anthropic, Gemini, and Replicate/fal media with encrypted key storage and model capability metadata, including native local vision/tool-use labels where available; account-runtime onboarding for Codex, the installed Claude CLI, OpenCode, and Pi, with per-chat native session persistence, consent-gated Claude session recovery, Codex transcript recovery, structured approvals, Milim approval modes, and visible tool events. The model chip is the sole visible model selector; branded provider/runtime icons identify its route while every picker row names the provider or nested runtime route, such as `Pi · GitHub Copilot`, and omits those redundant prefixes from the primary model label. The picker also shows setup status, capabilities, Favorites, per-model reasoning effort, and whether the next turn is plain chat, Milim tools, an account runtime, or media. Provider and runtime groups can be collapsed, with one shared layout persisted across every picker surface.
- Per-model reasoning effort controls where the provider, local runtime, or account runtime advertises supported effort levels.
- Model-agnostic Agents store reusable roles with deterministic avatar seeds; the same generated identity follows persona, schedule, and assigned Worker surfaces without image files, while unassigned Workers receive deterministic run-local identities. Workers are live instances grouped into durable Runs on the canonical parent chat. Context shows compact Worker avatars plus planned/active/done counts; the Workers inspector groups the parent chat's full history into Active and Done, opens transcript-linked Runs directly, keeps Active Workers and the selected Worker stable as progress arrives, and owns delegation settings, Ask approval, Markdown-rendered transcripts and results, stopping, retry with the same or another model, Run deletion, and diff review. Running Workers keep their parent thread visibly active in the sidebar, while Workers never become sidebar chats. On narrow layouts the Worker history stacks above the selected detail, grows only to its content up to half the inspector height, and scrolls beyond that cap. Each thread defaults to Ask delegation and can switch Off/Ask/Auto or choose a Worker model from the searchable catalog; bare delegated model names resolve to an unambiguous catalog `provider/model` id. Ask freezes an exact approval plan and pauses the parent until the user runs the workers or continues solo; Auto joins parallel results before the parent answer. The Worker Run stream is the sole owner of managed Worker progress. Before resuming the parent, desktop reloads the canonical terminal Run, verifies every returned Worker is terminal, and requires successful or partial Runs to include every planned Worker. Results join through one hidden context message; pending approved joins survive a desktop reload. If none succeeded, the parent acknowledges the failures and continues the original request without delegating again. Retries create new Runs so failed history remains intact until explicitly deleted. Approved writers use isolated Git worktrees with explicit diff apply; non-Git workspaces fall back to read-only. Schedules remain single-agent.
- Memory/RAG with Personal and Git-stable Project libraries, exact-term plus embedding retrieval fused with reciprocal rank fusion, and at most five provenance-labeled entries inside a hard 1,024-token context budget per enabled turn. Retrieval keeps stable and legacy exact-folder scope reads, archive/recovery controls, legacy thread-memory recovery, explicit tool-capable writes, and provider-routed embeddings behind the same remote privacy gate; lexical recall remains available when embeddings do not.
- Workspace and Git panels with guarded fetch/commit/sync actions, a review-first local diff workspace for all, unstaged, staged, last-turn, commit, and branch comparisons, and a hideable, resizable changed-file tree on the left that navigates the unified diff. Git also supports local branch switching and creation, model-generated commit subjects when the message is left blank, turn-level Git worktree checkpoints with restore actions, a unified Preview / Code / Git / Workers inspector, and a persisted thread-local Context card with a compact Worker summary plus independently collapsible environment, task, live activity, context, and sources sections. Context groups the current prompt estimate, free tokens, fixed-context categories, repository-rule files, and warnings under a collapsed Prompt context disclosure; provider-reported usage remains separately labeled as cumulative usage. Sources show five filename-only, single-line entries at a time and open their attachment, artifact, or memory target. Context's reserved 300px slot pushes the transcript while its scrollbar stays at the thread view's right edge, can coexist with the inspector on wide layouts, and becomes mutually exclusive with it on narrow layouts. A workspace launcher appears only for folder-backed threads and opens the active folder in installed local tools; filesystem/shell tools remain rooted to the selected folder, generated-file diff review/apply/save-to-workspace flows use selected artifact revisions, Docker sandbox runs remain isolated, preview DOM tools require an active ready surface, optional computer-use controls OS input, and Review/Guarded/Open gate tool execution.
- The latest code-changing turn ends with a compact changed-files card showing aggregate and per-file line counts. **Review changes** opens that exact checkpoint diff in the Git inspector; **Undo** restores the pre-turn checkpoint, removes the assistant response, retains the original request, and clears native runtime state that no longer matches the workspace.
- The Git inspector switches between local Changes and the current branch's GitHub PR. The PR cockpit renders Markdown plus sanitized GitHub HTML in descriptions, reviews, and comments, with reviewers, normalized checks, and merge readiness; linked badges and bounded attachment images remain visible without allowing active HTML. It can draft an agent readiness pass, comment, review, mark drafts ready, or merge with an explicit method and head-SHA guard. The resizable, persisted Pull requests tool adds a searchable account-wide authored/review-requested list with GitHub user and App avatars, a persisted draggable list/detail divider, and Summary, Timeline, and Code views. Saved list and detail data render immediately while refreshes upsert changes in the background, and the same guarded comment, review, ready, and merge actions remain available. Visible isolated-worktree threads and the active shared checkout expose live accessible PR-state badges in the sidebar. GitHub actions require the separately installed and authenticated `gh` CLI.
- Privacy gate with Off, Redact, and Block modes enforced server-side before remote-provider and account-runtime calls. Remote providers and account runtimes receive image pixels only in Privacy Off; local Ollama and LM Studio vision remain local and allowed.
- Image, video, and prompt-to-music generation through configured OpenRouter, fal, and Replicate providers, with kind-aware model discovery, schema controls, status polling, native music playback, and window-filling in-app previews for generated images and videos. Chat remains the iterative generation surface, while the standalone Media Studio pairs the shared composer surface with a selected-output stage and collapsible searchable library sidebar. Successful and failed runs from both surfaces are indexed in `~/.milim/media/index.json`; downloaded files live under `~/.milim/media/files/<library-id>/` until explicitly deleted. Failed local saves keep their remote preview and can be retried from the studio. OpenRouter image generation is live-verified; video/music and fal/Replicate music have mocked adapter coverage but still require separate credentialed smoke verification.
- Mobile companion for paired phones, with restart-proof pairing, persisted phone URL base, startup Tailscale Serve repair when the bridge is enabled, in-app QR scanning, active desktop theme mirroring, Markdown-rendered live thread viewing/switching through a project-aware mobile sidebar, compact Worker plan approval/progress/results/stop controls, new-thread/stop/regenerate/rename/archive/delete controls, mobile file/photo attachments, model switching, phone sends through the desktop composer/model path, pairing, enable/disable, queued events, device revocation, and a Tailscale Serve setup helper that opens Tailscale download when missing and creates phone-reachable tailnet HTTPS URLs where available.
- The title-bar logo opens a compact keyboard-accessible app menu for common actions without adding another UI row; macOS builds also expose native Milim, File, Edit, View, Window, and Help menus.
- Searchable settings organized as Preferences, Workflows, Data & devices, and Application, with a dedicated Google Workspace section, direct control results, and actionable-only badges. Workflow preferences cover startup/panel restore, opt-in native notifications, manual-install update policy, inherited or configured new-chat defaults, Smart/Pinned/Hidden quick actions with project overrides, Automatic/Manual/Off composer suggestions, local-only bounded ranking and cross-chat prompt history, live model/agent defaults and safe unavailable-model policies, workspace opener/worktree behavior, JSON/Markdown export defaults, and transactional `.milim-backup.json` backup/restore with a pre-restore recovery snapshot. Opt-in composer ghost completion is local-first, sends only current composer text, excludes account runtimes, and requires explicit privacy/cost confirmation for a remote current-provider model. Appearance can hide the default-on local activity ridgeline above an empty chat's composer; themes retain contrast validation, animated backgrounds, and default-off local sounds. Configurable shortcuts include composer suggestions, `Ctrl/Cmd+K` for the command-and-chat palette, and `Ctrl+Tab` for recent threads. The app also includes resizable panels, frameless controls, close-to-tray background mode, bounded local logs, and checksum-verified Windows/macOS updates that never install or restart automatically.

## Test

```bash
cargo test
cargo clippy --workspace --all-targets
pnpm -C apps/desktop verify
pnpm -C apps/desktop verify:tester-ready
pnpm -C apps/site build
```

## Release Artifacts

The release workflow publishes stable Windows portable EXE and macOS universal DMG/app zip artifacts:

- `milim-windows-x64-portable.exe`
- `milim-macos-universal.dmg`
- `milim.app.zip`

Linux release packaging is intentionally disabled in `apps/desktop/scripts/package-release.mjs`.

Release builds run desktop verification on both macOS and Windows, require Apple signing secrets for macOS artifacts, intentionally enable Tauri's macOS private API for transparent preview activity overlay windows, and publish `manifest.json` plus an aggregate `SHA256SUMS.txt` from the current release run. Updater assets are verified with SHA-256 sidecars and the aggregate checksum file.

## License

MIT.
