---
id: release
path: release
label: Release
title: Release and verification
summary: Release artifacts, updater behavior, verification commands, and site build checks.
group: Reference
order: 110
updated: 2026-07-31
---

Release work should verify the Rust workspace, desktop app, site docs, and platform artifacts without reintroducing Linux packaging as a release target.

## Release artifacts

| Platform | Artifact |
|---|---|
| Windows | `milim-windows-x64-portable.exe` from the latest GitHub release. |
| macOS | `milim-macos-universal.dmg` and `milim.app.zip` from the latest GitHub release. |
| Linux | Not packaged as a release artifact. The Rust server and Tauri app remain source-buildable. |

Pull requests run full desktop verification in CI. Release tags do not repeat that suite: the Release workflow runs Windows runtime evidence alongside production packaging, Tauri builds the frontend before compiling each platform artifact, and each packaged binary and manifest is verified. macOS release artifacts require the Apple signing secrets and intentionally enable Tauri's macOS private API for transparent preview activity overlay windows. The workflow publishes `manifest.json` plus an aggregate `SHA256SUMS.txt` from the current release run. Updater assets are verified with SHA-256 sidecars and the aggregate checksum file. A rerun may repair an unpublished draft, but every edit and asset upload rechecks draft status; published releases are immutable and require a new version.

## Updater behavior

The desktop app checks GitHub Releases for the latest platform artifact on startup unless it checked within the last 120 minutes, then keeps the existing 12-hour background check cadence. Automatic checks download updates by default, verify their SHA-256 checksum, and stage them in the local update directory; users can disable automatic downloads in Settings. A staged update appears in the top bar as a Restart to update action, and one click flushes pending user state before handing it to the existing Windows portable EXE or macOS app replacement flow without another confirmation. Quit allows up to two seconds for pending state to flush, then quit and restart signal both embedded servers and allow up to three seconds to stop active workers and previews; Milim-owned MCP, agent-runtime, host-shell, sandbox, and preview process trees are contained so process exit closes any remaining descendants. A failed update flush aborts installation and leaves the staged update ready to retry. Updates that have not finished downloading retain the confirmation step. The top-bar dialog and Settings update panel show byte-based download progress, falling back to an indeterminate bar when the server does not provide a total size. Installation and restart never occur without an explicit user action.

Each release entry in `apps/desktop/src/update/releases.json` supplies the Markdown body created for its GitHub Release and may contain up to three bundled in-app update cards. A summary-only maintenance release uses an empty card array and shows no in-app deck. Desktop verification rejects larger decks; published versions `0.2.2` and `0.2.3` predate this limit and remain unchanged as the only exceptions. Verification also fails when `VERSION` has no matching, valid entry. After onboarding is complete, a non-empty deck appears on the first startup of that exact installed version and records dismissal in machine-local storage; Developer → Release UI can replay it without changing that state. The deck uses the installed version already loaded by the updater and does not fetch release notes at startup.

## Checks

```powershell Run release checks
cargo test
cargo clippy --workspace --all-targets
pnpm -C apps/desktop verify
pnpm -C apps/desktop verify:runtime-conformance
pnpm -C apps/desktop perf:canonical
pnpm -C apps/site build
```

## Runtime evidence

`v*` tags and manual Release runs produce a Windows-only `runtime-evidence-windows` artifact. Successful runs contain `runtime-conformance.json`, `canonical-thread.json`, and benchmark screenshots from deterministic, mock-backed scenarios; a benchmark failure adds `failure.json` and `failure.png`. These checks prove Milim's normalized event, session, approval, queue, and desktop interaction contracts; they require no credentials, make no paid or live completion calls, do not change authentication state, and do not establish live third-party compatibility.

The canonical benchmark builds and launches a Windows Tauri/WebView2 binary. Its timing measurements are advisory and have no pass/fail budgets; functional assertions, invalid layout, console errors, and missing evidence still fail the job.

## Docs site

The public docs site imports markdown from `docs/wiki/*.md` using Vite raw imports. The per-section search index is generated from headings and body text, so new sections become searchable without adding keywords in TypeScript. After Vite builds, the site emits route-specific title, description, canonical, Open Graph, and Twitter metadata plus a small Cloudflare Pages Worker that serves the correct static HTML for `docs.milim.ai` while keeping the landing page on `milim.ai`. Matching pull requests run the `Build` job; matching `main` pushes run `Deploy`, which builds once before publishing to Cloudflare Pages. A failed Wrangler publish retries up to five total attempts at one-minute intervals before the deployment fails. Changes to `VERSION` use that production deployment path so release-facing version surfaces stay current.

Use `docs/account-runtimes.md` as the style template for new long-form reference docs: short intro, route table, then behavior notes.
