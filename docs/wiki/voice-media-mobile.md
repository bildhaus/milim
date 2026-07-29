---
id: voice-media-mobile
path: voice-media-mobile
label: Media/mobile
title: Media and mobile
summary: Media generation and the mobile companion.
group: Local data
order: 80
updated: 2026-07-20
---

These optional extensions share the same desktop session. Media routes generate images, videos, and prompt-to-music results, and the mobile companion mirrors desktop threads while sending phone prompts through the desktop session. Voice input, dictation, transcription, TTS, audio remix/upload, and voice-chat routes are not part of Milim.

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

The inline model lane remains available in chat. The standalone Media Studio is a focused quick-generation surface with Image, Video, and Music controls in the shared composer surface, a large selected-output stage, and a searchable library sidebar. The library loads its first page in the background but starts closed, and its count reports the saved items loaded so far rather than implying a server-wide total; a `+` suffix means another page is available. If a generation returns multiple visual outputs, the stage keeps one selected output and exposes the returned variants as a keyboard-selectable strip for comparison; multiple music outputs render as separate native audio rows. Generated images and videos open in the existing window-filling preview from both the studio and chat transcript.

Every validated generation submitted through `POST /media/generate`, whether started in chat or the studio, receives a library record before the provider request. The durable index is stored at `~/.milim/media/index.json`, and successfully downloaded outputs live under UUID-named directories at `~/.milim/media/files/<library-id>/`. There is no automatic retention policy. **Delete** requires a second click within three seconds and permanently removes the item and its local files.

Library states are `running`, `saving`, `ready`, and `failed`. Pending provider runs continue through normal client polling while Milim is open and are refreshed when the studio is reopened. If a bounded or validated local download fails, the remote preview remains available; **Refresh** retries the save. Downloads accept allowlisted image, video, and audio MIME types, reject private or local HTTP destinations and unsafe redirects, use temporary `.part` files, and atomically move completed files into place. OpenRouter video content is fetched through its credentialed provider path and is served back to the desktop only through authenticated library content routes.

The original prompt and normalized request settings stay local in the library so **Use settings** can restore a run. The privacy gate is still applied before remote traffic: Block prevents record creation and provider submission when PII is detected, Redact sends the redacted prompt while retaining the original only in the local record, and Off sends the prompt unchanged. OpenRouter image is live-verified, while credentialed OpenRouter video/music and fal/Replicate music smoke verification remains separate from mocked adapter coverage.

## Mobile companion

In the desktop app, open Settings -> Mobile, enable the companion bridge, then use **Set up with Tailscale** to expose the phone-only companion surface through Tailscale Serve. Milim uses the installed `tailscale` CLI, points Serve at a mobile-only loopback listener, fills the phone URL base, and creates the pairing QR. It prefers HTTPS on port `10000` so the installed phone app can request camera access for QR scanning, and falls back to tailnet-only HTTP if HTTPS setup fails. On later launches, if the companion bridge is still enabled, Milim best-effort reapplies Tailscale Serve to the new local relay listener. If no other tailnet devices are visible, Milim warns that the phone must be connected to the same Tailscale tailnet before scanning. If Tailscale is not installed, Milim opens the official Tailscale download page and shows an install shortcut in the panel. If Tailscale is installed but not logged in, the settings panel shows the CLI status and leaves the manual URL field available.

The phone page includes web-app metadata, an icon, and a small service worker so mobile browsers can add it to the home screen. The phone stores its device key locally, and the desktop app persists paired device records under the Milim config directory, so refreshes, home-screen launches, and Milim restarts keep the phone paired unless the bridge is disabled or the device is revoked.
Home-screen apps can have separate browser storage from the tab used to scan the QR. Pairing links stay redeemable for their short pairing window, so launching the installed app from the pairing link can automatically mint its own device key instead of failing on an already-used token. The installed pairing screen can also scan the desktop QR in place or accept a pasted pairing link, which avoids depending on a browser URL bar after the relay has been added to the home screen. Camera scanning requires a secure browser context such as HTTPS or localhost; use the paste field if the phone browser blocks camera access on a plain HTTP relay URL.

Manual setup still works: enter a phone-reachable base URL before pairing. Use a Tailscale Serve URL for a real phone, or `http://10.0.2.2:<port>` for the Android emulator.

```bash Mobile send event
curl http://127.0.0.1:7377/mobile/relay \
  -H "Content-Type: application/json" \
  -d '{"text":"send this from phone","action":"send"}'
```

The companion does not bypass the desktop session. The desktop app persists the phone URL base, then publishes a project-aware thread sidebar, available model IDs, the active desktop theme variables, the active-thread snapshot, and a compact active Worker Run snapshot to the mobile bridge. Paired phones can approve an Ask plan, continue solo, follow worker progress/results, or stop the Run. Detailed logs and worktree diffs remain desktop-only. Phones also receive Markdown-rendered live thread updates, inherit the desktop theme, can switch/create/rename/archive/delete threads, stop or regenerate turns, change the model, attach small files or photos, and send prompts through the same selected model, privacy, memory, and approval settings as the active desktop composer. Desktop relay event polling only runs while the app document is visible and the bridge has at least one paired device; a slower status check wakes relay polling after pairing.
