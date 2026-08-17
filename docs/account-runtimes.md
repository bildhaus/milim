# Account Runtimes

Milim can use signed-in Codex, bring-your-own Claude CLI, OpenCode, and Pi as chat runtimes. These are separate from saved provider records. Each runtime can be enabled or disabled independently in Providers; disabled runtimes remain authenticated but are not queried for the desktop model catalog, shown in model pickers, or allowed to start a desktop turn. The privacy gate scans, redacts, or blocks text before any account runtime receives it. Image pixels cannot be scanned or redacted, so account-runtime images require Privacy Off.

On Windows, Milim refreshes its process search path from the current machine and user environment at startup. Account runtimes and their child tools—including Git, GitHub CLI, Cargo, and ripgrep—therefore see tools installed after sign-in even when Explorer launched Milim with an older environment snapshot.

Account-runtime processes use the `AccountRuntimeInherited` environment policy: they inherit the user's environment plus adapter-specific overrides so their own authentication and developer tooling keep working. Milim journals only the policy name and names of explicit grants, never environment values. Host-shell tools separately use `HostShellInherited`; configured MCP children start from OS essentials plus explicitly configured variables (`ConfiguredIntegrationSanitized`), and container payloads receive no host credentials unless explicitly granted (`SandboxSanitized`). Docker itself may inherit the host values needed to reach the daemon, but Milim does not translate them into container `-e` values.

On macOS and Linux, Milim resolves symlinked CLI launchers to their real executable target before spawning them. This preserves app-bundle-relative companion lookup when a shell command in a directory such as `~/.local/bin` points into an application bundle.

The Providers panel shows each detected CLI version and compares it with the latest stable version published for that CLI. An available update enables and highlights the Update action; a current runtime shows a disabled Up to date action. If the release check is unavailable, the action remains neutral and usable. Updating requires a second confirmation, then invokes that runtime's own updater (`codex update`, `claude update`, `opencode upgrade --pure`, or `pi update self --no-approve`) and rechecks the installed version. Finish active turns first. Milim does not replace these tools' installers, credentials, or standalone configuration.

After a Milim chat has a native Codex thread id, Claude session id, OpenCode session id, or Pi session id, Milim lets that runtime own prior context. Later turns send the current per-turn context plus the latest user message instead of replaying the visible Milim transcript or auto-compacting it first. Manual `/compact` still creates a visible Milim checkpoint, but its summary call is ephemeral and the stored native runtime id is cleared afterward.

Every account-runtime turn must report an explicit completion, error, or intentional terminal notice. If a runtime exits or closes its stream without one, Milim shows a runtime error instead of treating an empty response as success.

Review approvals use a shared lifecycle: requested, user-decided, delivered to the runtime, then acknowledged when the runtime resumes. The decision endpoint is idempotent for identical retries and does not report success until the runtime adapter accepts the response. Delivery failures, disconnects, and post-delivery silence terminalize the turn with an actionable error; only the human decision wait is unbounded. Persisted transcripts retain nonterminal lifecycle states and reconcile them as interrupted after restart.

Each adapter maps the normalized Approve/Deny choice to its own protocol. Codex selects only from the request's advertised `availableDecisions`—including `cancel` rather than assuming `decline`—while Claude, OpenCode, and Pi keep their native response shapes.

Failed or canceled turns clear only the affected native session before the next send. This prevents a prompt that the CLI persisted before the failure from being replayed into divergent native history.

All account-runtime tool events keep their full input text behind the transcript's visual ellipsis. Claude no longer truncates structured tool inputs server-side, while OpenCode and Pi also preserve completion results and error details in the shared run trace.

## Canonical run-event boundary

Rust's canonical `RunManager` accepts desktop and native-mobile turns through `POST /control/v1/commands`. It freezes the selected runtime configuration, invokes a reusable `codex`, `claude`, `opencode`, or `pi` adapter, and persists normalized events in the thread timeline. `POST /harnesses/{id}/run` remains a compatibility facade for direct clients; its common request keeps the existing prompt, images, workspace, reasoning, approval, plan, privacy/Milim context, and model fields, while `native_session_id` and `persist_session` replace bridge-specific names at this facade only.

The response is SSE. Every JSON event has `schema_version: 1`, a per-run `run_id`, increasing `seq`, `at_ms`, `harness_id`, and one canonical application event. The event vocabulary covers native session and turn identity, text and reasoning deltas, tool start/update/finish, approval lifecycle, usage and limits, generated images, native workers, runtime and recovery notices, and exactly one completed, failed, or cancelled terminal event. That terminal event closes the desktop response immediately while the native adapter drains in the background for cleanup. Claude drains through process exit so its session registry is released; other adapters retain a bounded drain. Bridge protocol names, ACP update tags, and JSON-RPC correlation ids do not cross this desktop boundary.

This is an ownership cutover, not a replacement of native runtime behavior. The Run Manager owns acceptance, queueing, stop, frozen configuration, durable timeline, and approval state; Codex, Claude, OpenCode, and Pi still use their current native protocols, session stores, approval delivery, and management sidebands. Existing `/codex/run`, `/claude/run`, `/opencode/run`, and `/pi/run` request and SSE contracts remain compatibility facades.

Canonical account-runtime turns also receive a local run ledger, but its visibility is explicitly `harness_boundary`. Milim persists the privacy-processed prompt, images by digest/reference, normalized adapter events, terminal response, usage, approvals, and native-session boundary that it can observe. It does not claim to reconstruct Codex, Claude, OpenCode, or Pi internal system prompts, hidden context, or tool orchestration. These runs therefore advertise inspection but not steering.

Adapters produce typed canonical events inside the server. The compatibility routes serialize those events back to their legacy SSE shapes only at the HTTP boundary; the canonical route no longer calls another route or reparses its SSE/JSON output. Version-pinned native-protocol fixtures cover the installed Codex, Claude Code, OpenCode, and Pi wire shapes. Unsupported protocol updates emit metadata-only notices without copying prompt, tool, or environment payloads. Codex additionally accepts turn notifications only when their native thread and turn ids match the active request, preventing subagent or stale-turn notifications from completing or leaking into the visible turn.

## OpenCode

Milim invokes the user-installed `opencode acp` process once per turn and speaks ACP v1 JSON-RPC over stdio. `GET /opencode/status` and `GET /opencode/models` discover configured models without refreshing OpenCode's network cache; `POST /opencode/run` creates or resumes the native session, applies the exact selected model, streams normalized events, and forwards permission requests to Milim's one-shot approval cards. Plan, Guarded, Review, and Open map to a Milim-owned permission overlay. Guarded and Review refuse to run when `opencode debug config` shows that higher-precedence configuration weakened the promised policy.

OpenCode also supports chats without a workspace folder. Milim supplies a private managed ACP directory for protocol compatibility and disables OpenCode's native filesystem tools; Milim-owned tools remain available.

OpenCode remains responsible for its providers, credentials, instructions, and plugins. Milim does not bundle the CLI or read its credentials. Images use the existing outbound privacy gate and require Privacy Off. When supported by the installed CLI, verbose discovery supplies the picker with exact context, output, image, tool-use, and reasoning metadata.

## Pi

Milim invokes the separately installed `pi` CLI in offline JSONL RPC mode. `GET /pi/status` reports availability, version, authentication/configuration state, provider count, normalized model metadata, and an actionable error; `GET /pi/models` returns the normalized catalog; `POST /pi/run` starts or resumes a Milim-owned Pi session with the exact `provider/model`, reasoning level, prompt, image inputs, workspace, and approval fields. Pi model ids use the `pi:<provider>/<model>` desktop prefix. Milim stores `piSessionId` plus its last-synced Milim message cursor so later turns resume Pi's native JSONL session without replaying already-owned history. Compaction and other side calls are ephemeral and do not reuse that session.

Install Pi separately and authenticate inside Pi with `/login`. Pi owns its credentials and provider configuration; Milim neither reads nor stores them. Catalog discovery confirms configured providers and models, while the first turn verifies the current credential; known expired-token failures show a `/login` recovery message. Pi is useful as a distinct lean agent experience with its own detailed multi-provider catalog and subscription-backed providers such as GitHub Copilot, even where individual models overlap OpenCode or saved Milim providers.

Embedded runs always pass `--offline --no-extensions`. This prevents user or project extensions from executing startup code outside Milim's approval boundary while leaving Pi's context files, prompt templates, and skills on their normal discovery path. Plan and Guarded expose only `read`, `grep`, `find`, and `ls`. Review loads one temporary Milim-owned extension that pauses `bash`, `write`, `edit`, and any unknown tool call and forwards the exact generated arguments to Milim's one-shot approval broker. Open exposes Pi's built-in tools without prompts. The temporary extension is removed after the run; standalone Pi settings are never modified.

Pi also supports chats without a workspace folder. Project context files remain disabled. Open runs start in a private per-session directory under Milim's runtime data and expose Pi's built-in tools without prompts; that directory is a predictable working directory, not a filesystem sandbox. Plan, Guarded, and Review keep Pi's built-in tools disabled until a workspace folder is selected, while eligible Milim-proxied tools remain available.

Images are sent as native RPC image blocks and require Privacy Off. Text uses the same server-side scan/redact/block gate as the other account runtimes. Pi discovery and startup failures are isolated from the other provider and account-runtime lanes.

CLI calls to an authenticated standalone server should pass `--token` or set `MILIM_API_TOKEN`; desktop account-runtime calls use the desktop app's per-launch bearer token internally.

## Codex

Codex uses the installed Codex CLI app-server.

| Surface | Behavior |
|---|---|
| `GET /codex/account` | Reads the current Codex account state. |
| `POST /codex/login/device` | Starts the ChatGPT browser login flow. This is what the desktop Providers UI uses. |
| `POST /codex/login/chatgpt-device` | Starts the ChatGPT device-code login flow. |
| `POST /codex/login/api-key` | Passes `{ "api_key": "..." }` to Codex app-server login. Milim does not store this key. |
| `POST /codex/logout` | Logs out through Codex app-server. |
| `GET /codex/models` | Lists Codex models and forwards Codex model metadata to the picker. |
| `GET /codex/rate-limits` | Reads Codex account rate-limit state. |
| `GET /codex/threads` | Lists active or archived interactive Codex threads in cursor-based pages of 25, with optional `search`; `all=true` drains the matching catalog into one response. |
| `GET /codex/threads/{id}` | Reads one importable user/assistant transcript without changing or deleting the Codex thread. |
| `POST /codex/run` | Starts or resumes a Codex app-server thread with Milim's selected tool approval and sandbox policy. |

`/codex/run` accepts `model`, `prompt`, optional `images`, optional `cwd`, optional `reasoning_effort`, optional `thread_id`, optional `persist_thread`, and Milim tool approval fields. Guarded and unapproved Review turns use Codex read-only mode, approved Review turns use workspace-write, and Open uses Codex `danger-full-access`; Plan stays read-only regardless of approval mode. A request is valid when the prompt is non-empty or at least one image is present. Each image is `{ "media_type": "image/png", "data": "<base64>" }`; PNG, JPEG, WebP, and GIF are accepted up to 2 MB each. Milim validates the bytes after the privacy check, writes only those bytes into a private temporary per-turn directory, sends Codex app-server `localImage` inputs, and deletes the directory when the turn ends. Caller-supplied filesystem paths are never accepted as image inputs.

Milim desktop persists the returned Codex thread id on the Milim chat and sends it back on later turns, so reopening a chat resumes the same Codex app-server thread. One-off side calls omit `persist_thread` and remain ephemeral. Any effort except `auto` is forwarded to Codex as the app-server `effort` field.

The desktop model picker reads Codex `supportedReasoningEfforts`, `defaultReasoningEffort`, and `inputModalities`. Image-capable Codex models show Vision when Codex advertises image input; missing modality metadata remains unknown and does not block an attempted send. Text attachments remain bounded prompt context, while image attachments are sent as real multimodal inputs.

Codex image-generation results are streamed back as:

```json
{ "type": "image", "id": "...", "status": "completed", "url": "data:image/png;base64,..." }
```

Milim renders those as generated image previews in chat. This covers Codex models that emit image-generation items when the installed Codex runtime exposes them.

Normal Codex processes initialize against the stable app-server API. Milim starts one process per account operation or turn and safely declines interactive requests during non-interactive operations. During turns, command and file-change approvals keep their existing Review behavior; additional permission requests always show the exact requested profile and grant it for that turn only. Standard MCP form elicitation supports top-level string, number, integer, boolean, and scalar-enum fields, with validation in both desktop and Rust. HTTP(S) URL elicitation requires explicit Open link and Continue actions. Unsupported schemas, OpenAI-specific forms, and unsafe URL schemes are decline-only. Entered form values are sent through the one-shot approval response and are not stored in the transcript, run trace, or resolved event.

Milim's in-app Preview inspector is separate from the OpenAI Browser plugin. Codex, Claude, OpenCode, and Pi turns receive the same injected preview instruction and use the `preview_prepare_app` and `preview_start_app` Milim MCP tools to inspect and start the selected project's dev command under Milim's managed preview runtime, then use `preview_open_url` to open its loopback URL in the visible task. The managed process tree remains alive between account-runtime turns and is still covered by the Preview Stop action and Milim shutdown cleanup. Once the page is ready, the existing scoped preview tools provide DOM inspection and interaction.

Codex warnings, configuration warnings, model reroutes/verifications, and deprecation notices appear as nonterminal run events. Unknown server requests receive JSON-RPC method-not-found responses instead of blocking the app-server stream.

The Providers screen exposes **Import chats** after Codex is connected. The picker loads the complete matching history, groups it by recorded project folder, and supports All, Active, and Archived scopes. Select a whole project, the no-project group, or individual chats; project selection is not narrowed by search, and existing imports are excluded. Missing folders remain visible under their recorded project but are display-only and never become the imported Milim workspace. Selected chats import sequentially, continue past individual failures, and remain in the dialog for review or retry.

Import keeps only visible user/assistant text, replaces media-only messages with an omission marker, and omits reasoning, tool records, and local file references. Milim attaches the original Codex thread id and the final imported message as its sync cursor, so the next Codex turn resumes without replaying the imported transcript. New Milim chats have current local timestamps and no selected model; choose a Codex model before continuing. Milim remains authoritative after this one-time import and does not continuously merge Codex history.

Legacy histories use stable `thread/read`. When Codex explicitly reports paginated history, chat import alone starts a narrowly experimental process and pages `thread/turns/list`; normal account operations and turns remain stable-only.

## Installed Claude CLI

Milim does not include Claude Code, does not provide Anthropic credentials, and is not affiliated with or endorsed by Anthropic. This integration only invokes the user's separately installed official Claude CLI on the local machine. Use of Claude and Claude Code is governed by Anthropic's terms.

Claude CLI integration boundaries:

- Milim invokes the local `claude` executable.
- Milim uses Claude's documented `claude -p` stream-JSON interface and matches each structured `tool_result` to its `tool_use_id`.
- Milim does not bundle Claude Code, proxy Claude access, or sell Claude access.
- Authentication and direct Anthropic communication are handled by the official Claude CLI; Milim does not offer Claude.ai login.
- Milim does not manage, store, or receive Claude credentials.
- Claude CLI usage remains subject to Anthropic's terms.
- Anthropic documents API-key or supported cloud-provider authentication as the unambiguous path for third-party and commercial integrations; Milim does not represent subscription compatibility as an Anthropic partnership or entitlement.
- Some permission modes may allow Claude to run local tools and commands.
- Stale-session recovery asks before stopping a matching local Claude CLI process unless the turn is already in Open mode.

| Surface | Behavior |
|---|---|
| `GET /claude/status` | Checks installed CLI availability, auth state, account metadata, model aliases, and optional per-alias image capability metadata. |
| `GET /claude/threads` | Lists locally retained top-level Claude chats in cursor-based pages of 25, with optional `search`; `all=true` returns the complete matching catalog. |
| `GET /claude/threads/{id}` | Reads the selected chat's active importable user/assistant branch without changing its Claude transcript. |
| `POST /claude/run` | Runs `claude -p --input-format stream-json --output-format stream-json` with Milim's selected tool approval mode. |

`/claude/run` accepts `model`, `prompt`, the same optional base64 `images` array as Codex, optional `cwd`, optional `reasoning_effort`, optional `session_id`, optional `allow_session_recovery`, and Milim tool approval fields. A request may be image-only. Milim pipes a native user message containing text and Anthropic base64 image blocks into the CLI; no OCR or prompt-only image note is used. Milim desktop stores one Claude session id per Milim chat. New native sessions pass it as `--session-id`; existing Claude project transcripts pass it as `--resume`, so reopening a chat restores the same installed Claude CLI session instead of colliding with the existing transcript file. One-off side calls omit `session_id` and use `--no-session-persistence`.

The Claude account card also exposes **Import chats**. Milim searches locally retained top-level UUID transcripts under Claude Code's project history and groups the complete newest-first catalog by recorded project folder. Select a whole project, the no-project group, or individual chats; search does not reduce project-wide selection, and existing imports open instead of duplicating. Selected chats import sequentially and failed transcripts remain selected for retry.

Import keeps human prompts and assistant text, joins assistant fragments around omitted tool activity, replaces media-only prompts with an omission marker, and omits reasoning, tools, task notifications, attachments, and internal command scaffolding. A retained project folder attaches the native session and sync cursor for `--resume`; a missing project remains grouped under its recorded path but imports as transcript-only and leaves the session stale so Milim's existing Fresh/Resume choice appears before continuation. This is a one-time local import, not continuous synchronization or Claude.ai cloud history access.

If Claude reports that a persisted session id is already in use, Review and Guarded emit a recovery-required event and ask before Milim tries to stop a matching local `claude`/`node` process for that exact session id and retry once. Open authorizes that recovery immediately. After stopping a match, Milim waits for the recorded owner process to exit before removing only that session's registry file and retrying; an unrelated or still-live registry owner is preserved.

Milim maps `low`, `medium`, `high`, `xhigh`, and `max` to Claude CLI `--effort`; `auto`, `none`, and `minimal` are omitted. Runs map Milim approval modes onto Claude permission modes and do not set a max-turn cap. Open mode maps to Claude's `bypassPermissions` mode and authorizes exact-session recovery without another prompt; it may run tools and commands without additional Claude prompts, so use it only in trusted workspaces.

Claude CLI models in the picker advertise image input plus `low`, `medium`, `high`, `xhigh`, and `max` reasoning efforts. The built-in aliases include `sonnet`, `opus`, `haiku`, and `fable`.
