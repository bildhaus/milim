---
id: voice-media-mobile
path: voice-media-mobile
label: Media/mobile
title: Media and mobile
summary: Media generation and the mobile companion.
group: Local data
order: 80
updated: 2026-08-15
---

These optional extensions share the same desktop-owned state. Media routes generate images, videos, and prompt-to-music results, while native and compatibility mobile clients control the Rust-owned canonical threads and runs. Voice input, dictation, transcription, TTS, audio remix/upload, and voice-chat routes are not part of Milim.

## Setup paths

| Feature | Setup check |
|---|---|
| Media generation | Add Replicate, fal, or OpenRouter media-capable provider credentials. |
| Mobile companion | Enable the companion bridge, use Tailscale setup or a manual phone URL, pair the phone, then use the phone view to read, switch, and send prompts through desktop threads. |

## Media route

```bash Generate media
curl "http://127.0.0.1:7377/media/generate" \
  -H "Content-Type: application/json" \
  -d '{
    "provider_id": "provider-id",
    "kind": "image",
    "model": "black-forest-labs/flux-schnell",
    "prompt": "A clean product screenshot on a graphite desk"
  }'
```

Media prompts are remote-provider traffic. They pass through the privacy gate in Redact and Block modes before leaving the machine.

The inline model lane remains available in chat. The standalone Media Studio is a focused quick-generation surface with Image, Video, and Music controls in the shared composer surface, a large selected-output stage, and a searchable library sidebar. The composer defaults below Output, can be moved to its side from the stage toolbar, and remembers that placement. Long prompts scroll within the fixed Side prompt region; below Output, the prompt grows to a responsive cap before scrolling. The side composer and Library are resizable, remember their widths, and collapse when dragged past their minimum; the toolbar controls restore them. The library loads its first page in the background, opens initially at wide studio sizes, becomes a drawer at medium sizes, and reports the saved items loaded so far rather than implying a server-wide total; a `+` suffix means another page is available. If a generation returns multiple visual outputs, the stage keeps one selected output and exposes the returned variants as a keyboard-selectable strip for comparison; multiple music outputs render as separate native audio rows. Generated images and videos open in the existing window-filling preview from both the studio and chat transcript.

Every validated generation submitted through `POST /media/generate`, whether started in chat or the studio, receives a library record before the provider request. The durable index is stored at `~/.milim/media/index.json`, and successfully downloaded outputs live under UUID-named directories at `~/.milim/media/files/<library-id>/`. There is no automatic retention policy. **Delete** requires a second click within three seconds and permanently removes the item and its local files.

Library states are `running`, `saving`, `ready`, and `failed`. Pending provider runs continue through normal client polling while Milim is open and are refreshed when the studio is reopened. If a bounded or validated local download fails, the remote preview remains available; **Refresh** retries the save. Downloads accept allowlisted image, video, and audio MIME types, reject private or local HTTP destinations and unsafe redirects, use temporary `.part` files, and atomically move completed files into place. OpenRouter video content is fetched through its credentialed provider path and is served back to the desktop only through authenticated library content routes.

The original prompt and normalized request settings stay local in the library so **Use settings** can restore a run. The privacy gate is still applied before remote traffic: Block prevents record creation and provider submission when PII is detected, Redact sends the redacted prompt while retaining the original only in the local record, and Off sends the prompt unchanged. OpenRouter image is live-verified, while credentialed OpenRouter video/music and fal/Replicate music smoke verification remains separate from mocked adapter coverage.

## Mobile companion

The native companion in `apps/mobile` is a bare React Native application for iOS and Android. It uses TypeScript, Metro, Xcode, and Gradle directly; Milim does not use Expo or EAS. A paired phone can switch among multiple Milim desktop hosts, manage threads, read and send Markdown chat, queue or stop turns, regenerate, choose models and Agents, attach small photos or files, resolve individual Review requests, and control Worker proposals. Git diffs, terminals, desktop settings, and complete Worker logs remain desktop-only.

The active desktop host publishes a resolved appearance snapshot through control v1. Mobile applies its palette, light/dark status-bar mode, glass opacity, border opacity, card/input radii, background fit, and clear/dim/blur/mono treatment immediately and refreshes them after an `appearance.updated` event; switching hosts switches appearance with the rest of that host's replica. Desktop **Cover** (and legacy **Fill**) maps to native edge-to-edge aspect fill, so the image spans the full phone window without distortion; Contain, Center, and Tile retain their desktop meanings. When the active custom theme contains an uploaded JPEG, PNG, GIF, or WebP data image, mobile downloads it from the paired-device-authenticated, size-bounded appearance endpoint and keeps only the current host/revision in its private cache. Arbitrary remote CSS URLs, gradients, and desktop font files are not transferred; those use native color and system-font fallbacks. Native motion honors the OS Reduce Motion setting.

The native shell adapts the desktop's compact glass hierarchy to a thumb-friendly single column: dense thread rows use overflow actions, user turns keep the restrained accent surface while assistant answers remain dominant, reasoning is disclosed on demand, and scrolling away from a live answer reveals the same explicit **Latest** affordance. Chat uses the canonical sequence to interleave messages with grouped work, merged tool lifecycles, file-change summaries, runtime warnings and failures, and approval context. Running and failed work opens to its icon/label/detail/status rows; completed work collapses to a duration and last meaningful action, while every state remains readable without relying on color. Pending approvals are actionable in chronological context and remain aggregated under **Attention**. The lightweight native bottom bar is limited to **Chat**, **Attention**, and **Hosts**. An explicit button in the global header opens the thread list as a right-side drawer, avoiding Android's back-gesture edge; the drawer groups threads into collapsible project directories plus **Inbox / No Project**, shows group busy and attention state, uses short project names, and keeps full workspace paths inside project details.

The unified composer keeps compact model and Agent controls beside the prompt, attachment pills and an attachment action sheet inside the dock, and desktop-shaped icon actions without spending transcript space on a separate picker bar. The transcript uses the same soft lower-edge fade as desktop so text recedes above the composer instead of ending at a hard boundary; the custom background remains visible through the alpha mask. An empty, unfocused composer is a one-line **Message milim...** pill with a visible running indicator and stop action when needed, whether the reader is at the latest message or farther back. It expands on tap and remains expanded while focused, holding a draft or attachment, or presenting a pending approval; tapping the pill while reading older content also restores follow mode and moves to the authoritative latest turn before focusing. A background state change never pulls a reader away from their current position. A measured 160 ms height and crossfade transition continuously returns the released dock space to the transcript; expansion completes before focus opens the keyboard. Code blocks, tables, commands, and long file paths scroll horizontally inside their own row rather than clipping or widening the transcript. Searchable milim-styled sheets replace platform alert menus, and their dimmed backdrop fades to clear toward the top of the screen instead of applying one flat dark overlay. The right-side thread drawer applies the equivalent horizontal treatment, strongest beside the drawer and fading toward the exposed screen edge. All mobile-owned product copy, native display names, permission copy, and desktop-supplied host labels render `milim` in lowercase; internal React Native target identifiers remain unchanged. Models remain grouped by runtime/provider and show their route, context size, and known vision/tool/reasoning/media capabilities; Agents show their descriptions and compact tool/skill counts. Selection still mutates the canonical desktop-owned thread through control v1.

Milim must be running on the paired computer. Rust owns the canonical thread, active run, queue, approvals, and sequence-numbered timeline, so work continues when the Tauri window is hidden or the renderer reloads. It does not continue after the Milim process exits or the computer sleeps. The mobile app keeps a bounded SQLite replica per `host_id`; drafts are local to that phone and never overwrite an unsent desktop composer.

### Pairing and connectivity

Open Settings -> Mobile and enable the companion bridge. **Set up with Tailscale** remains the recommended path: Milim points Tailscale Serve at its isolated mobile/control listener and prefers HTTPS on the fixed Serve port. Milim operates no relay, account service, or hosted transcript store.

Opt-in LAN access is disabled by default. Enabling it binds only the pairing and `/control/v1` mobile router, selects an available port, and advertises `_milim._tcp.local` with the stable host ID. Plain HTTP should be used only on a trusted network. Manual URLs remain available, including `http://10.0.2.2:<port>` for an Android emulator. The phone tries the last successful endpoint, other saved candidates, matching mDNS discovery, and then a manually entered URL. The Hosts screen leads with friendly connection summaries and compatibility state; exact endpoints, timestamps, protocol bounds, and the destructive device-removal action remain available in host details instead of dominating the main screen.

Scan or open the native `milim://pair` QR/deep link. The short-lived claim contains the one-time pairing secret, stable host identity, protocol version, and selected endpoint. Device credentials are stored in iOS Keychain or Android Keystore and can be revoked individually from desktop or removed by the phone. A revoked key can no longer bootstrap, mint a socket ticket, or keep a WebSocket session alive.

### Foreground synchronization

The phone paints its cached tail immediately, then compares timeline epoch and sequence coverage with the authoritative desktop state. Existing desktop messages are backfilled once into that canonical timeline before clients connect, so opening an older thread shows its transcript instead of beginning at the first mobile-era event. A local discriminated projection merges repeated tool updates by stable call ID, resolves approval lifecycle rows by approval ID, groups user-visible activity by run, and gives future event types a bounded human-readable fallback without exposing raw protocol payloads. The initial view stays bounded and opens at the latest item; **Load earlier messages** pages backward without replacing the portion already on screen. The client fills ordinary gaps by sequence and replaces stale state with a fresh bounded tail after an epoch mismatch or true middle gap. Live events use a short-lived, single-use WebSocket ticket so the durable device key is never placed in a URL.

The WebSocket is maintained only while the operating system permits foreground execution. Backgrounding saves drafts and cursors; foregrounding performs authoritative catch-up and surfaces pending attention. There are no v1 push notifications and no promise that the app remains connected while closed.

Offline prompts remain drafts. Approvals, Worker decisions, destructive commands, and stale prompts are never auto-sent after reconnect. An ambiguous command response may be retried only with the same `command_id`, which returns the original durable result instead of duplicating work.

### PWA compatibility window

The existing `/mobile` phone PWA consumes the same versioned control contract and remains functional during native parity testing. It is a compatibility client, not the execution owner. Native becomes the documented default only after the cross-client acceptance matrix passes; the PWA is then marked deprecated for one compatibility window and removed in a later release.
