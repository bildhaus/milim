---
id: release
path: release
label: Release
title: Release and verification
summary: Release artifacts, updater behavior, verification commands, and site build checks.
group: Reference
order: 110
updated: 2026-07-28
---

Release work should verify the Rust workspace, desktop app, site docs, and platform artifacts without reintroducing Linux packaging as a release target.

## Release artifacts

| Platform | Artifact |
|---|---|
| Windows | `milim-windows-x64-portable.exe` from the latest GitHub release. |
| macOS | `milim-macos-universal.dmg` and `milim.app.zip` from the latest GitHub release. |
| Linux | Not packaged as a release artifact. The Rust server and Tauri app remain source-buildable. |

Release builds run desktop verification on both macOS and Windows. macOS release artifacts require the Apple signing secrets and intentionally enable Tauri's macOS private API for transparent preview activity overlay windows. The workflow publishes `manifest.json` plus an aggregate `SHA256SUMS.txt` from the current release run. Updater assets are verified with SHA-256 sidecars and the aggregate checksum file.

## Updater behavior

The desktop app checks GitHub Releases for the latest platform artifact on startup unless it checked within the last 120 minutes, then keeps the existing 12-hour background check cadence. Automatic checks download updates by default, verify their SHA-256 checksum, and stage them in the local update directory; users can disable automatic downloads in Settings. A staged update appears in the top bar as a Restart to update action, and one click hands it to the existing Windows portable EXE or macOS app replacement flow without another confirmation. Updates that have not finished downloading retain the confirmation step. The top-bar dialog and Settings update panel show byte-based download progress, falling back to an indeterminate bar when the server does not provide a total size. Installation and restart never occur without an explicit user action.

Each release entry in `apps/desktop/src/update/releases.json` supplies both the bundled in-app update-card deck and the Markdown body created for its GitHub Release. New release entries contain one to three cards; desktop verification rejects larger decks. Published versions `0.2.2` and `0.2.3` predate this limit and remain unchanged as the only exceptions. Verification also fails when `VERSION` has no matching, valid entry. After onboarding is complete, the deck appears on the first startup of that exact installed version and records dismissal in machine-local storage; Developer → Release UI can replay it without changing that state. The deck uses the installed version already loaded by the updater and does not fetch release notes at startup.

## Checks

```powershell Run release checks
cargo test
cargo clippy --workspace --all-targets
pnpm -C apps/desktop verify
pnpm -C apps/site build
```

## Docs site

The public docs site imports markdown from `docs/wiki/*.md` using Vite raw imports. The per-section search index is generated from headings and body text, so new sections become searchable without adding keywords in TypeScript. After Vite builds, the site emits route-specific title, description, canonical, Open Graph, and Twitter metadata plus a small Cloudflare Pages Worker that serves the correct static HTML for `docs.milim.ai` while keeping the landing page on `milim.ai`.

Use `docs/account-runtimes.md` as the style template for new long-form reference docs: short intro, route table, then behavior notes.
