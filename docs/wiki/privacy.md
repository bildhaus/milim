---
id: privacy
path: privacy
label: Privacy
title: Privacy and security
summary: Local and remote data boundaries, Google Workspace access, privacy modes, redaction, blocking, bearer auth, and CORS boundaries.
group: Local data
order: 70
updated: 2026-08-28
---

Privacy settings are easiest to reason about as a routing question: what stays local, what goes to a provider, and which gate runs before a remote send.

## Privacy modes

| Mode | Server behavior |
|---|---|
| `off` | No scanning, redaction, or blocking. Remote sends are forwarded as-is. |
| `redact` | Detected PII is replaced with reversible `[KIND_N]` placeholders before the remote call, then restored in streamed replies when possible. |
| `block` | Remote sends containing detected PII fail closed before the provider call. |

The scanner is deterministic regex-based detection for common email, phone, token-like, IP, URL, and secret-looking strings. It does not infer names or sensitive meaning from natural language.

New desktop chats default to `off`. The first-send summary shows the selected runtime destination, workspace, Privacy mode, and Review status before a remote turn leaves the composer.

Because image content cannot be scanned or redacted by the text privacy gate, remote provider and account-runtime requests with image parts are blocked in `redact` and `block` modes. Switch to `off` only when sending those pixels to that provider or account runtime is intended. Local Ollama and LM Studio image inputs stay on the configured local endpoint and remain allowed.

## What is enforced server-side

| Route family | Privacy gate |
|---|---|
| Remote chat providers | Enforced before the provider router sends a completion request. |
| Remote embeddings | Enforced before embedding inputs leave the machine. |
| Remote media providers | Enforced before Replicate, fal, or OpenRouter image, video, or prompt-to-music prompts are sent. |
| Codex runtime | Text is scanned/redacted/blocked before `/codex/run`; image pixels require Privacy Off before any bytes are decoded or written to temporary files. |
| Installed Claude CLI | Text is scanned/redacted/blocked before `/claude/run`; image pixels require Privacy Off before the native multimodal message is built. |
| OpenCode and Pi | Text is scanned/redacted/blocked before `/opencode/run` or `/pi/run`; image pixels require Privacy Off before native ACP/RPC input is built. |
| Local Ollama or LM Studio | Not scanned by Milim because the configured local runtime receives the prompt on the machine. |

Each desktop run snapshots its selected privacy mode and canonical workspace when the request starts. That immutable context is reused for every inference iteration, tool call, delegation, approval, and retry, so changing another thread cannot redirect or reclassify an in-flight run. Legacy API clients may omit the run mode; the server then snapshots the current `POST /privacy/mode` default once at request start. An invalid explicit mode or workspace is rejected instead of falling back.

Schedules persist their own privacy and workspace boundary. New desktop schedules inherit the active thread's values, migrated schedules remain explicitly Privacy Off, and every occurrence freezes that saved context before it enters the canonical run path.

Canonical runs keep a local privacy-processed ledger for their lifetime. The resolved model request is transformed and committed before provider execution; a failed pre-request commit prevents the call, and a failed post-response or tool-result commit prevents another model step. Privacy Block rejects detected text before any request artifact is written. Redact persists the redacted form. Independently of the selected privacy mode, credential-shaped text and fields such as authorization, API keys, bearer tokens, device keys, refresh tokens, and client secrets are replaced before ledger persistence. Attachment bodies are not duplicated: the ledger stores identity, digest, metadata, and a durable reference.

## Data boundary

| Route | Boundary |
|---|---|
| Local Ollama or LM Studio | Prompt, files, and embeddings stay on the machine unless that runtime is configured otherwise. |
| Hosted model provider | Messages, selected context, embedding inputs, and tool-visible text go to the provider after the privacy mode is applied. |
| Account runtime | Prompt text and, only in Privacy Off, attached image pixels go to the selected Codex, Claude, OpenCode, or Pi runtime and its configured model provider. |
| Media provider | Prompt text and model parameters go to Replicate, fal, OpenRouter media, or the selected media backend after the privacy mode is applied. OpenRouter video bytes return through an authenticated Milim proxy; the provider key remains server-side. Generated media URLs or data URLs are stored with the chat result when persistence is enabled. |
| Mobile companion | Paired phone text, files, and photos travel directly to the user's running desktop over Tailscale, opt-in LAN, or a manual endpoint. Rust accepts the turn with the thread's current privacy configuration and applies the same provider/runtime gate. Milim operates no managed relay or hosted transcript store. |
| MCP tools | External MCP servers run as configured local child processes or remotes; treat each configured server as its own trust boundary. |
| MCP Apps | App HTML comes from its configured MCP server, is re-fetched rather than persisted, and runs in an opaque-origin iframe. Validated HTML is held briefly in memory behind a random expiring capability URL; it receives no bearer token. Network access is denied by default and limited to valid `_meta.ui.csp` origins; host calls remain authenticated, same-server, visibility-checked, and approval-gated. |

## Google Workspace data

Google Workspace is optional. Milim requests the non-sensitive `drive.file` scope, which limits access to Drive files and folders that you explicitly choose with Google Picker or that Milim creates for you. Milim does not receive general access to your Drive, Gmail, Calendar, Contacts, password, or Google account credentials.

Milim uses authorized Google data only to provide user-facing features that you request: listing authorized files, rendering local previews, reading or editing Docs, Sheets, and Slides, transferring files, organizing Drive items, and applying explicitly approved sharing or trash/restore actions. Milim does not sell Google user data, use it for advertising, or use it to train generalized AI models.

The OAuth refresh token and authorized-file registry are AES-GCM encrypted in Milim's local application data and excluded from backups. The desktop encryption key is stored in Windows Credential Manager or macOS Keychain when available. macOS keeps a matching owner-only local recovery copy so encrypted state remains readable when a local rebuild changes the app's Keychain access identity; Settings > Data & privacy reports this restricted-file mode. File content is fetched directly from Google when needed and may appear in local previews, tool results, or chat history. Before a fresh connection, Milim discloses that deliberately using a remote model or external tool with Google file content sends that content to the selected provider under Milim's visible approval and privacy controls; Milim does not operate an intermediary cloud service for the Google connection.

Milim shares Google data only when necessary to perform an action you request—for example, with Google APIs, with a remote provider you selected for a tool-capable task, or with an exact Drive recipient you approved. Removing a file from Milim deletes its local selected-file record but neither deletes the Drive file nor changes Google's authorization. Disconnecting Google Workspace makes a bounded revocation request and always removes the local token and registry when local deletion succeeds. Milim reports whether Google confirmed revocation; if it could not confirm, you can review or revoke Milim from your Google Account connections.

Milim's use and transfer of information received from Google APIs adheres to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including its Limited Use requirements.

## Auth and CORS

The desktop app disables loopback trust and uses a per-launch bearer token for its embedded server. Standalone server auth supports static bearer tokens or `msk-v1` access keys when configured. Empty CORS allow-list means no browser origins are allowed; configured origins are explicit.

The mobile listener exposes only a host identity probe, pairing and device authentication, and `/control/v1`; it cannot reach the full desktop/provider API and serves no legacy browser relay. Pairing creates a revocable per-device credential stored by the phone in Keychain or Keystore. A pairing secret is consumed by its first successful claim, and the native client verifies that the endpoint's stable host identity matches the scanned claim before submitting it. WebSockets use short-lived, single-use tickets, and revocation invalidates HTTP access, unused tickets, and live sockets. Optional LAN exposure is off by default and advertises only the isolated listener; plain HTTP carries an explicit trusted-network warning.

Every accepted control turn freezes its model, workspace, privacy and approval modes, plan state, Agent snapshot, enabled tools and skills, attachments, native session identity, and each linked chat's revision and maximum timeline sequence. Later settings, links, or Agent edits affect later turns only. Linked reads expose only canonical visible user/assistant content up to that frozen boundary; they omit hidden prompts, reasoning, tool ledgers, and account-runtime history. The consuming chat's privacy policy governs linked transcript reads, while a mailbox destination starts with its own frozen privacy and approval settings. Control backup version 3 includes canonical runs, timelines, the run ledger, content-addressed artifacts, durable inbox, reciprocal links, mailbox exchanges, command receipts, and approvals; paired-device secrets remain in their existing encrypted desktop store. Ordinary transcript exports do not include the ledger. Ledger retention follows thread lifetime.

```bash Scan text before sending
curl http://127.0.0.1:7377/privacy/scan \
  -H "Content-Type: application/json" \
  -d '{"text":"email me at person@example.com"}'
```

```bash Block remote sends with detected PII
curl http://127.0.0.1:7377/privacy/mode \
  -H "Content-Type: application/json" \
  -d '{"mode":"block"}'
```
