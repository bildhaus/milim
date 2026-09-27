---
id: models
path: models
label: Models
title: Models and providers
summary: Model-agnostic dev chat routing across provider APIs, local runtimes, Codex, Claude, OpenCode, and Pi bridges.
group: Core
order: 40
updated: 2026-09-27
---

Model routing is provider-agnostic and centered on the active dev thread. The provider registry stores enabled remotes and their model metadata, then the desktop model picker merges local API runtime models, provider models, account runtime models, and media-capable models. Duplicate provider model ids stay provider-scoped in the picker and route back to the selected provider; provider sections with fewer visible models appear first.

On desktop startup, the picker emits the last usable provider catalog immediately while one live refresh checks enabled chat providers. Codex, Claude, OpenCode, and Pi probe independently and merge into the visible catalog as each lane finishes; a CLI that consumes its full startup timeout does not delay cached provider models. A failed or empty provider refresh and a failed account probe preserve already visible rows rather than clearing the picker. Enabled account-runtime probes allow the CLI bridge's full startup window and retry once after a transient failure.

After the startup model lanes settle, opening a thread reconciles its saved provider route with the current catalog. If a provider was recreated and the raw model id has exactly one current route, milim updates the thread lazily without copying or replacing its conversation. Missing or ambiguous provider routes are cleared and require an explicit choice in the model picker. Plain provider ids and account-runtime ids that may belong to a temporarily unavailable catalog lane are preserved. Model and per-thread reasoning changes are written to Rust-owned canonical thread state, and an immediate send waits for that write plus desktop persistence before the turn is accepted. Switching from one non-empty model selection to another appends a canonical `model_changed` timeline item. Desktop and mobile render it in sequence as **Continuing with _new model_** and **Previously _old model_ · thread retained**. Further selections before the next user message update the same pending notice from its original model to the latest choice; returning to the original model removes the notice. A user message freezes the current notice, so later switches start a new one. Choosing the initial model, clearing an unavailable selection, or changing only reasoning effort does not add transcript noise.

The closed model chip keeps the selected provider or account-runtime route visible. The picker also classifies the selected model into one execution lane: plain chat, milim tools, Codex runtime, Claude runtime, OpenCode runtime, Pi runtime, or media. Switching models changes the next turn for the active thread without resetting workspace context, memory, previews, artifacts, approvals, or queued messages.

Worker routing is a separate thread setting. A thread may choose an optional Worker model; otherwise managed Workers inherit the parent model. Account-runtime inheritance starts a fresh Worker-owned Codex, Claude, OpenCode, or Pi session for each managed Worker; Workers never reuse or update the parent's native session binding. Delegated bare model names resolve against the catalog's `provider/model` ids, preferring the selected Worker or parent namespace and rejecting ambiguous matches. Saved Agents remain portable roles: their instructions and resolved skills transfer to the selected Worker runtime, while milim still governs Worker access independently. In Auto, provider and local parents use milim-managed Workers. Read-only Codex and Claude turns may normalize native worker activity into milim Runs; write-capable account-runtime turns use managed read-only Workers so only the parent edits the workspace. If a runtime cannot report reliable worker lineage, milim falls back to managed Workers or ordinary tool activity instead of inventing a parent/child relationship.

## Favorites and reasoning effort

Favorites are the only model shortcut. Favorite model IDs are desktop-host state: the desktop picker and every paired mobile picker read the same canonical list, and a change from either client propagates live to the other. Every picker row keeps its provider or runtime identity visible, including nested routes such as `OpenCode · OpenAI` and `Pi · GitHub Copilot`, so overlapping model names remain distinct in Favorites, search results, and provider groups. Primary model labels omit redundant account-runtime and nested-provider prefixes while keeping creator namespaces. Search matches model names, route IDs, runtimes, and providers. Each model keeps its own persisted reasoning-effort choice as an app-wide default. Choosing an effort inside a chat writes only that chat's override, so other open chats on the same model keep their effort. User-created chats do not copy another chat's overrides and instead inherit the app-wide default; pickers outside the chat continue to read and write that default. Agents do not pin models, so changing the thread model keeps the active Agent enabled and changes the model used by its next interactive run.

Every provider and runtime group can be collapsed. On desktop, the layout is shared by the chat, Hot Swap, and Worker model pickers and persists across restarts. Mobile keeps its own Favorites-only filter and collapsed groups in its host-partitioned cache. Favorites stays expanded, while search and Favorites-only filtering temporarily reveal matches without changing the saved collapsed groups.

Hot Swap assesses the selected target before committing the change. Full-parity swaps stay one-click. Smaller context windows, explicitly unsupported image/tool input, unavailable setup, or stale account-runtime history open a preflight. Unknown image capability allows an attempted send without falsely claiming support; explicit false blocks the capability claim, and explicit provider metadata wins over model-name fallbacks. Codex and Claude native sessions can receive image pixels, so account-runtime targets are no longer degraded solely because they are account runtimes.

OpenCode model rows use the CLI's verbose catalog for context, output, image, tool-use, and reasoning metadata when the installed version exposes it. Each milim thread durably keeps one native binding per account-runtime adapter and resumes it across later turns and app restarts. Only an explicit session-recovery signal compare-and-clears the matching adapter binding; ordinary failures, cancellations, stale recovery events, and switches to another adapter do not erase valid sessions.

## Provider kinds

| Kind | Examples | Implements |
|---|---|---|
| OpenAI-compatible | OpenAI, OpenRouter, Groq, Ollama, LM Studio, vLLM, custom `/v1` servers | Chat, Responses, legacy completions, model list, embeddings, structured output, and reasoning plus vision/tool-use metadata where provided. |
| Anthropic | Claude Messages API through a stored provider key | Chat, streaming, model routing, token usage, and native base64 or URL image blocks. |
| Gemini | Google Generative Language API | Chat, model discovery, model routing, inline image bytes, and genuine Gemini Files API URIs. Arbitrary web image URLs are rejected instead of downloaded server-side. |
| Replicate | Remote image/video/music provider | Media model catalog, schemas, generation status polling, and normalized URL-returning music results. |
| fal | Remote image/video/music provider | Queued generation, status polling, and normalized media results. |
| Brave Search, Tavily | Web search API keys | Backs the agent's `web_search` tool instead of the keyless DuckDuckGo fallback. No chat models. |
| Local API runtimes | Ollama, LM Studio, and vLLM on this machine | Chat, prompt generation, Ollama `keep_alive` lifecycle calls, Responses or completions where the runtime exposes them, model list, embeddings, structured output, native vision/tool-use labels where available, and reasoning effort for supported local reasoning models. |
| Account runtimes | Installed Codex, Claude, OpenCode, and Pi CLIs, not saved provider API keys | Resumable agent-style turns with real image input, visible tool events, active milim browser context, milim-owned tools, and milim approval modes. |

Requests to OpenRouter include its app-attribution headers with `https://milim.ai/` as the identifier and `milim` as the display title.

OpenRouter's provider-reported billed cost is read from the final streamed usage event, accumulated across every completed model request in a tool-agent turn, and persisted with canonical response metrics. Per-response, thread, and app-wide activity totals prefer that exact amount. If a completed call does not report cost, milim falls back to the cached prompt/completion pricing for that model, pricing cached input at the provider's cache-read and cache-write rates when it publishes them, and labels the result as an estimate; run spend limits use the same estimate. Failed calls without usage do not invent token or cost data.

Local detection probes Ollama on `localhost:11434`, LM Studio on `localhost:1234`, and vLLM on `localhost:8000`. It also reads published TCP ports from running Docker containers whose image or name contains `vllm`; it never inspects container arguments or environment. A vLLM candidate is marked reachable only when its unauthenticated `/v1/models` response identifies at least one model with `owned_by: "vllm"`. Containers without a published host port, authenticated vLLM servers, and setups where milim itself runs inside Docker must be added manually with a URL reachable from milim's network.

### Output tokens, finish reasons, and prompt caching

When a request leaves output tokens blank, the Anthropic provider sends a model-aware `max_tokens`: the model's catalog cap (the Models API `max_tokens` field, saved with the provider's model list) capped at 64,000, or, without catalog metadata, the model family's output limit capped at 64,000 (64,000 for Claude 4 and 5 models except Opus 4.0 and 4.1, which get 32,000; 8,192 for Claude 3.5; 4,096 for Claude 3), and 16,000 for a model milim does not recognize. If Anthropic rejects the value because it exceeds the model's output limit, milim retries once with the limit from the error and remembers it for that model until restart. A rejection because the prompt plus `max_tokens` exceeds the context window is retried once with the remaining context, without remembering it. OpenAI-compatible providers and Gemini leave the output limit unset, so the server or model default applies. An explicit output-tokens override is always sent, lowered only to a known model cap.

Gemini 3 models attach a thought signature to the function calls they stream and reject a follow-up request that omits it. milim saves each turn's signatures with that turn's provider state, so they survive a restart, keeps a bounded in-memory copy for history recorded without them, and sends them back on the next tool-loop step. A function call with no known signature, such as history from another model, is sent with Google's documented `skip_thought_signature_validator` placeholder on the first call of the turn.

Every provider reports one normalized finish reason when a response ends: `stop`, `length` (Anthropic `max_tokens`, Gemini `MAX_TOKENS`), `tool_calls` (Anthropic `tool_use`), `content_filter` (refusals and Gemini safety stops), or `error`.

Anthropic requests use prompt caching by default. Leading system messages become separate `system` blocks, and milim marks up to three `cache_control: {"type": "ephemeral"}` breakpoints: the last system block, the last tool definition, and the last non-empty block of the newest user or tool-result turn. Each agent step can then read the conversation prefix the previous step wrote. A system message later in the conversation is not merged into the top-level prompt. It is sent at its original position as user text wrapped in `<system-reminder>…</system-reminder>`, merged with adjacent user content so turns still alternate, with tool results kept first. Gemini applies the same split: leading system messages become `systemInstruction` parts and later ones become in-place user reminders. OpenRouter requests for `anthropic/*` models get prompt-cache breakpoints too. For OpenAI itself (`api.openai.com` only), requests include a `prompt_cache_key` so every step of one conversation routes to the same cache. Canonical chat runs key it on a hash of the thread id, so turns keep sharing the cache even when the system prompt or per-turn context changes; the key is part of the provider request stored in the run ledger. Other requests derive it from the conversation's leading system messages and first message, and a `prompt_cache_key` sent to milim's `/v1/chat/completions` is used as the source instead. Other OpenAI-compatible servers never receive the field.

Token usage keeps cached input inside `prompt_tokens` and adds optional `cache_read_tokens` and `cache_write_tokens`. They are read from Anthropic `cache_read_input_tokens` and `cache_creation_input_tokens`, OpenAI and OpenRouter `prompt_tokens_details.cached_tokens` (plus OpenRouter's `cache_write_tokens`), Responses API `input_tokens_details.cached_tokens`, and Gemini `cachedContentTokenCount`. Anthropic reports uncached input separately, so milim adds the cached portions to `prompt_tokens` to keep totals comparable across providers. The fields are omitted when zero, and tool-agent turns sum them across model requests. milim's own compatible endpoints pass them on: `/v1/chat/completions` adds `usage.prompt_tokens_details.cached_tokens` (streamed in the `include_usage` chunk too), and `/anthropic/v1/messages` reports `cache_read_input_tokens` and `cache_creation_input_tokens` with `input_tokens` as the uncached remainder, in the response and in the streamed `message_delta` usage.

### Reasoning and thinking

| Provider | Behavior |
|---|---|
| Anthropic | Thinking follows the model family, with summarized display so the reasoning panel shows text. Opus 5.5 and later and Fable always think adaptively; an effort of None asks for the lowest effort. Opus 5 and Sonnet 5 think adaptively by default and Off disables it. Under Auto, Opus and Sonnet 4.6 and Opus 4.7 and 4.8 turn adaptive thinking on. Budget-thinking models (Haiku 4.5, Sonnet 3.7, and Claude 4 models through 4.5) think only when an effort level is chosen (a 1,024 to 32,000-token budget, at most half the output budget) or `thinking_token_budget` is set. The effort picker offers what each model accepts: Off wherever thinking can be turned off, and no xhigh on 4.6 models. Signed thinking blocks are replayed to the same model within a run; if Anthropic refuses the replay, the step is retried once without them. Sampling parameters are dropped for models that reject them, and a forced `tool_choice` becomes `auto` where it is unsupported. |
| OpenAI | Reasoning models on `api.openai.com` (o1, o3, o4, gpt-5 and later, `codex-*`) use the Responses API with `store: false`: encrypted reasoning is carried between tool steps, reasoning summaries stream to the reasoning panel, and summaries are dropped automatically for organizations OpenAI has not verified. They get no temperature or Top P. Other OpenAI-compatible hosts keep Chat Completions; OpenAI reasoning models there get `max_completion_tokens` (except through OpenRouter) and no temperature or Top P. |
| Gemini | Thinking tokens count as completion tokens and cost. Tool schemas that use `$ref`, `oneOf`, or `allOf` are inlined or merged. |
| OpenRouter | `reasoning_details` are kept across tool steps. |
| Local models | Tool calls written as `<tool_call>{…}</tool_call>` text are recovered conservatively, and streamed tool-call chunks without `index` are handled. |

A generation stream may stay silent for 60 seconds before it counts as stalled, 5 minutes when the model is expected to reason, and 15 minutes at high, xhigh, or max effort and for `-pro` and deep-research models.

### Context windows

The agent loop's compaction uses the model's context window (see [Agent loop behavior](agents#agent-loop-behavior)): the provider-reported prompt or context limit when there is one, otherwise a built-in table of documented limits. It covers GPT-6 and GPT-5.x (for example `gpt-5`: 400K, of which 272K input), GPT-4.1 and GPT-4o, the o-series, gpt-oss, Gemini API models (1,048,576), and Claude by model family. Other hosted models and local servers that report nothing get 32,768 (200,000 for unrecognized Anthropic models). A per-model `context_window` capability override in the provider record's `model_overrides` replaces both; it has no Provider sheet control yet.

### vLLM capabilities and controls

vLLM is an OpenAI-compatible server, but vision, reasoning, and tool use are model- and server-configuration capabilities rather than universal vLLM features. milim reads `max_model_len` as the model context window, recognizes the vLLM owner marker, and reads the installed server's `reasoning_effort` enum from `/openapi.json`. Standard multimodal `image_url` content, streamed `reasoning`/`reasoning_content`, and function tools pass through the normal provider route. The matching model must support the modality, and vLLM may still require launch-time chat templates, reasoning parsers, tool-call parsers, or automatic tool-choice flags.

The Provider sheet exposes per-model Vision, Reasoning, and Tools choices. **Auto** keeps discovery; **Yes** or **No** is an encrypted explicit override that survives later model refreshes. **Verify vision, reasoning, and tools** runs three small live requests against the selected model and reports each result without changing metadata. The probes send no sampling parameters or forced tool choice, and reasoning counts when the stream carries reasoning deltas or reasoning continuation data. Applying reliable results creates unsaved overrides, which take effect after **Save changes**. These probes execute real inference and may consume provider resources.

The composer session menu exposes per-model generation overrides. Common controls are output tokens, temperature, Top P, seed, stop sequences, frequency penalty, and presence penalty. vLLM models additionally expose Top K, Min P, repetition penalty, and thinking-token budget. Blank values use the server/model default. milim validates the values, freezes them when a turn is accepted, records them in run details, and reuses the same values for every model request in a tool-agent loop. Reasoning effort remains the model picker's separate per-model control and is forwarded to recognized vLLM reasoning models.

## Runtime lanes

| Lane | When it appears | What happens next |
|---|---|---|
| Plain chat | No workspace, tool, preview, schedule, agent, or memory-write context is active. | The provider/local model answers directly. |
| milim tools | A provider/local model is selected while workspace, sandbox, computer-use, preview tools, schedule intent, active agent, or memory-write intent is active. | The model runs through milim's tool-agent loop with visible tool events and approval policy. |
| Codex runtime | A Codex account model is selected. | milim sends the turn through the Codex account-runtime bridge. |
| Claude runtime | An installed Claude CLI model is selected. | milim sends the turn through the Claude CLI bridge. |
| OpenCode runtime | An installed OpenCode model is selected. | milim sends the turn through the OpenCode ACP bridge. |
| Pi runtime | A `pi:<provider>/<model>` entry is selected. | milim sends the turn through Pi's JSONL RPC bridge with the exact provider/model. |
| Media | An image, video, or prompt-to-music model is selected. | milim uses the media generation flow and keeps the model out of chat, naming, schedule, and Worker lists. |

## Choose a backend

| Goal | Route | Why |
|---|---|---|
| Best local privacy | Ollama, LM Studio, or vLLM | Prompts stay on your machine unless that runtime is configured otherwise. |
| General reasoning | OpenAI, Anthropic, Gemini, or OpenRouter | Use hosted providers when quality, context length, or latency matters more than staying fully local. |
| Local reasoning control | Ollama thinking models, LM Studio models with reasoning metadata, or vLLM reasoning models | Ollama uses `/v1/chat/completions`; LM Studio uses `/api/v1/chat` for advertised native reasoning options without custom tools and `/v1/responses` when milim function tools are attached. `gpt-oss` still uses `/v1/responses` for `low`, `medium`, and `high` effort. vLLM uses `/v1/chat/completions` with its advertised `reasoning_effort` values and configured reasoning parser. |
| Media workflow | Replicate, fal, or OpenRouter media models | Use image, video, or prompt-to-music generation from the same milim surface. |

OpenRouter video uses its asynchronous `/videos` submission and polling workflow; completed bytes are fetched through milim's authenticated content proxy so provider credentials never enter the webview. OpenRouter music buffers streamed audio chunks into a completed MP3 result. fal and Replicate keep their provider job URLs and polling behavior. Discovery includes text-to-music generators only, excluding TTS, transcription, generic voice, and conversational audio models.

Verification is recorded per provider and modality. OpenRouter image is live-verified. OpenRouter video/music and fal/Replicate music are covered by mocked adapter tests but are not described as live-verified until separate credentialed, potentially billable smoke tests pass.
| Development coding loop | Any capable provider model or account runtime | Every lane receives the visible milim browser URL/title and milim-owned tools; account runtimes additionally keep their native resumable bridge, filesystem, and shell. |

## Account runtimes

Codex, the installed Claude CLI, OpenCode, and Pi are separate from saved provider records. They are backed by user-installed CLIs, appear in the model picker after authentication/configuration, and reuse the active milim chat session when the runtime exposes a native session id. milim does not read or store their credentials.

Providers checks installed account runtimes for newer versions. You can update an enabled runtime from its page or use **Update all** in the Overview attention bar to apply every detected update in sequence; both paths ask you to finish active runtime turns and confirm before changing an installed CLI.

### Several accounts for one runtime

Codex and Claude can hold more than one signed-in account. Each account is a named profile backed by its own configuration folder, which milim points the CLI at through `CODEX_HOME` or `CLAUDE_CONFIG_DIR`. milim never reads, copies, or stores a credential to do this; the CLI keeps owning whatever it writes into that folder. OpenCode and Pi do not relocate their configuration this way and keep a single account.

Add accounts from the runtime's page in Providers, which lists every account (including **Default**, the CLI's own folder) with the same layout for Codex and Claude. milim creates an empty folder and shows the command that signs it in; run it in a terminal, then refresh. Each added account has a **Use in Auto** switch (Default shows **Always in Auto**), an **Auto picks** marker on the account Auto would choose now, and a menu with **Rename...**, **Open folder**, **Copy path**, and **Remove account...**. Removing an account forgets it in milim and leaves the folder, and its credentials, on disk; Default cannot be removed or excluded from Auto.

Chats pick an account with the account chip, which appears once a second account exists. **Auto** re-picks before each turn, preferring the enabled account with the most room left and skipping any that is rate limited; a named account pins the chat to it. The choice is frozen per turn, and managed Workers inherit their parent chat's account.

Switching a chat to another account starts a fresh native session there, because a session lives inside one account's folder. milim replays the conversation so far, and the previous account keeps its own session untouched.

Codex reports its 5-hour and weekly usage up front, so Auto can prefer the account with the most room before a turn. Claude reports a limit only once a turn reaches one, so its accounts show usage after a capped turn and a rejected turn ends with a notice naming the account Auto will use next.

Each account runtime keeps its native skill catalog. milim does not copy all enabled skill bodies into every turn: it supplies compact ranked candidates and read-only lazy search/read tools through the authenticated per-turn gateway. Explicitly tagged milim skills are resolved immediately, while a saved Agent's Custom skill selection acts as an allowlist.

### Sign in to an account runtime

Each CLI signs in with its own tooling; milim never handles those credentials. In onboarding, the **Coding CLIs** path offers **Connect** for Codex and **Sign-in help** for Claude, OpenCode, and Pi, which opens this section. After signing in, choose **Refresh CLIs** in onboarding or **Refresh status** on the runtime's Providers page.

| Runtime | Setup | Session behavior |
|---|---|---|
| Codex | Use `/codex/login/device`, `/codex/login/chatgpt-device`, or `/codex/login/api-key`. | milim stores the returned Codex thread id on the milim chat when persistence is enabled, per signed-in account. |
| Installed Claude CLI | Install Anthropic's official `claude` CLI separately and run `claude auth login` outside milim, once per account. | milim stores one Claude session id per milim chat and account, uses `--session-id` for new native sessions and `--resume` for existing project transcripts, and can stop only a matching local Claude CLI process if Claude reports the session is already in use. Review and Guarded ask first; Open authorizes recovery immediately. milim waits for the recorded owner to exit before removing only the matching registry entry and retrying once. |
| OpenCode | Install OpenCode separately and configure a provider, for example with `opencode auth login`. | milim stores the native ACP session id and applies its approval overlay; no-folder chats use a private managed ACP directory without native filesystem tools. |
| Pi | Install Pi separately and authenticate with Pi's `/login`; catalog discovery confirms configuration, while the first turn verifies the current credential. | milim stores one Pi session id and sync cursor per chat; side calls use `--no-session`. Embedded runs disable discovered extensions, while normal Pi context, prompt, and skill discovery remains active. |

Codex model metadata is authoritative when `inputModalities` is present. Claude aliases advertise image input. For OpenAI, Anthropic, Gemini, and Groq families without explicit metadata, the picker uses conservative current-family Vision labels; custom compatible servers with unknown metadata are allowed to attempt standard `image_url` parts but cannot be guaranteed.

The repo-level account runtime reference lives at `docs/account-runtimes.md`.

## Provider setup failures

| Failure | Likely cause |
|---|---|
| No models | Provider discovery failed, the local runtime is stopped, or the base URL points at the wrong API shape. |
| 401 or 403 | The key is missing, expired, or attached to an account that cannot use the selected model. |
| 404 model | The provider works, but the selected model id does not exist for that provider. |
| Connection refused | The local runtime is not listening on the configured host/port. |
| Streaming stalls | Check proxy buffering, provider rate limits, and whether the selected model supports streaming. |

## CLI backend selection

| Environment | Use |
|---|---|
| `MILIM_REMOTE_BASE_URL` | OpenAI-compatible base URL used by CLI/server fallback. |
| `MILIM_REMOTE_API_KEY` | Optional bearer key for `MILIM_REMOTE_BASE_URL`. |

If no CLI backend is configured, `/v1/models` returns an empty list and chat requests return a setup error instead of a synthetic response.

## Provider errors

Rust classifies a failed canonical run at the point it enters run state. The run's `error` object keeps its `code` and raw `message` and adds an optional `provider_error`:

| `kind` | Typical cause | Desktop action |
|---|---|---|
| `auth` | HTTP 401/403, invalid or rejected API key | **Update key** opens Providers |
| `rate_limited` | HTTP 429 or a rate/usage-limit message; `retry_after_secs` when the provider sent `Retry-After`, "try again in", or Gemini's `retryDelay` or "retry in Ns" | **Retry**, counting down first |
| `context_length` | The prompt exceeds the model's context window, even after the agent loop compacted and retried once | **Switch model** |
| `model_not_found` | The provider does not offer the selected model | **Switch model** |
| `provider_unavailable` | HTTP 5xx, overload, timeouts, or network failures | **Retry** |
| `quota` | Exhausted quota, credit, or billing problems, including a 429 that reports hard exhaustion (OpenAI's `insufficient_quota`, Anthropic's spend limit, or a Gemini quota with `limit: 0`) | **Switch model** |

Unrecognized failures omit `provider_error` and show their original text. Provider HTTP failures carry `"{label} {operation} -> {status}: {body}"` plus ` (retry after Ns)` when a `retry-after-ms` or delta-seconds `Retry-After` header was present. Errors a provider sends inside an open stream (an Anthropic `event: error` such as `overloaded_error`, an OpenAI or OpenRouter error object, or a Gemini error payload) use `"{label} {operation} stream -> {status} {type}: {message}"`, with the status taken from the payload or mapped from its error type (for example `overloaded_error` is 529 and `rate_limit_error` is 429), so they classify exactly like the matching HTTP failure. A dropped connection or read timeout mid-stream is reported as `stream interrupted` with its cause and classifies as `provider_unavailable`. Older servers and non-canonical paths send text only; the desktop applies the same rules to that text as a fallback. The raw message always remains available under **Technical details**.
