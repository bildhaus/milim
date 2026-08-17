---
id: release
path: release
label: Release
title: Release and verification
summary: Release artifacts, updater behavior, verification commands, and site build checks.
group: Reference
order: 110
updated: 2026-08-17
---

Release work should verify the Rust workspace, desktop app, site docs, and platform artifacts without reintroducing Linux packaging as a release target.

## Release artifacts

| Platform | Artifact |
|---|---|
| Windows | `milim-windows-x64-portable.exe` from the latest GitHub release. |
| macOS | `milim-macos-universal.dmg` and `milim.app.zip` from the latest GitHub release. |
| Linux | Not packaged as a release artifact. The Rust server and Tauri app remain source-buildable. |
| iOS | Protected manual store-delivery job archives a signed IPA and uploads it to TestFlight. |
| Android | Protected manual store-delivery job produces a signed AAB and uploads it to Play internal testing. |

Pull requests run full desktop verification in CI. Release tags do not repeat that suite: the Release workflow runs Windows runtime evidence alongside production packaging, Tauri builds the frontend before compiling each platform artifact, and each packaged binary and manifest is verified. macOS release artifacts require the Apple signing secrets and intentionally enable Tauri's macOS private API for transparent preview activity overlay windows. The workflow publishes `manifest.json` plus an aggregate `SHA256SUMS.txt` from the current release run. Updater assets are verified with SHA-256 sidecars and the aggregate checksum file. A rerun may repair an unpublished draft, but every edit and asset upload rechecks draft status; published releases are immutable and require a new version.

## Updater behavior

The desktop app checks GitHub Releases for the latest platform artifact on startup unless it checked within the last 120 minutes, then keeps the existing 12-hour background check cadence. Automatic checks download updates by default, verify their SHA-256 checksum, and stage them in the local update directory; users can disable automatic downloads in Settings. A staged update appears in the top bar as a Restart to update action, and one click flushes pending user state before handing it to the existing Windows portable EXE or macOS app replacement flow without another confirmation. Quit allows up to two seconds for pending state to flush, then quit and restart signal both embedded servers and allow up to three seconds to stop active workers and previews; Milim-owned MCP, agent-runtime, host-shell, sandbox, and preview process trees are contained so process exit closes any remaining descendants. A failed update flush aborts installation and leaves the staged update ready to retry. Updates that have not finished downloading retain the confirmation step. The top-bar dialog and Settings update panel show byte-based download progress, falling back to an indeterminate bar when the server does not provide a total size. Installation and restart never occur without an explicit user action.

Each release entry in `apps/desktop/src/update/releases.json` supplies the Markdown body created for its GitHub Release and may contain up to three bundled in-app update cards. Cards can keep contributor attribution separate from checked feature details with an optional `credit` line. A summary-only maintenance release uses an empty card array and shows no in-app deck. Desktop verification rejects larger decks; published versions `0.2.2` and `0.2.3` predate this limit and remain unchanged as the only exceptions. Verification also fails when `VERSION` has no matching, valid entry. After onboarding is complete, a non-empty deck appears on the first startup of that exact installed version and records dismissal in machine-local storage; Developer → Release UI can replay it without changing that state. The deck uses the installed version already loaded by the updater and does not fetch release notes at startup.

## Checks

```powershell Run release checks
cargo test
cargo clippy --workspace --all-targets
pnpm -C apps/desktop verify
pnpm -C apps/desktop verify:runtime-conformance
pnpm -C apps/desktop verify:tauri-webview
pnpm -C apps/desktop perf:canonical
pnpm -C apps/mobile verify
pnpm -C apps/site build
```

Pull requests separately verify the mobile protocol fixtures/reducer and build an Android debug APK plus an unsigned iOS simulator app. Simulator success does not replace installation on a real phone.

## Mobile store delivery

`.github/workflows/mobile-store.yml` is manual-only and uses protected GitHub environments. It aligns the mobile marketing version with `VERSION`; callers supply monotonically increasing iOS and Android build numbers so a retried upload remains store-valid. Android signing, Apple distribution credentials, provisioning data, App Store Connect keys, and Play service-account credentials remain protected secrets and are removed with the ephemeral runner.

The iOS job retains the IPA and dSYM evidence before uploading to TestFlight. The Android job retains the signed AAB, mapping files, and native symbols before uploading to the Play internal track. Production promotion remains manual and should happen only after the TestFlight and internal-track builds are installed on real devices and the desktop/mobile compatibility evidence is attached.

Store declarations describe the actual v1 boundary: no Milim account, tracking, managed relay, hosted transcript synchronization, or push gateway. The phone communicates directly with user-paired computers.

## Runtime evidence

`v*` tags and manual Release runs produce a Windows-only `runtime-evidence-windows` artifact. Successful runs contain `runtime-conformance.json`, `canonical-thread.json`, and benchmark screenshots from deterministic, mock-backed scenarios; a benchmark failure adds `failure.json` and `failure.png`. Runtime conformance checks the generated `/control/v1` contract, v5 migration, ledger atomicity and privacy, durable inbox lifecycle, tool execution bounds, runtime adapters, and the quiet desktop run-details rendering. These checks require no credentials, make no paid or live completion calls, do not change authentication state, and do not establish live third-party compatibility.

Two integration tests provide explicit, self-skipping live proof. Set `MILIM_REAL_HARNESS_SMOKE=1` with `MILIM_REAL_HARNESS_BASE_URL`, `MILIM_REAL_HARNESS_API_KEY`, and `MILIM_REAL_HARNESS_MODEL`; optionally set `MILIM_REAL_HARNESS_KIND` to `anthropic` or `gemini` instead of the default `openai_compatible`. The test submits a real turn through `/control/v1`, waits through the authenticated inspection route, and verifies request/response ledger events without exposing the key. Set `MILIM_REAL_ACCOUNT_RUNTIME_SMOKE=1` plus one or more of `MILIM_REAL_CODEX_SMOKE_MODEL`, `MILIM_REAL_CLAUDE_SMOKE_MODEL`, `MILIM_REAL_OPENCODE_SMOKE_MODEL`, and `MILIM_REAL_PI_SMOKE_MODEL` to exercise installed, already-authenticated account CLIs and verify their `harness_boundary` journals. These tests return immediately unless their opt-in flag is set; enabled runs may incur provider usage.

The canonical benchmark builds and launches a Windows Tauri/WebView2 binary. Its timing measurements are advisory and have no pass/fail budgets; functional assertions, invalid layout, console errors, and missing evidence still fail the job.

## Docs site

The public docs site imports markdown from `docs/wiki/*.md` using Vite raw imports. The per-section search index is generated from headings and body text, so new sections become searchable without adding keywords in TypeScript. After Vite builds, the site emits route-specific title, description, canonical, Open Graph, and Twitter metadata plus a small Cloudflare Pages Worker that serves the correct static HTML for `docs.milim.ai` while keeping the landing page on `milim.ai`. Matching pull requests run the `Build` job; matching `main` pushes run `Deploy`, which builds once before publishing to Cloudflare Pages. A failed Wrangler publish retries up to five total attempts at one-minute intervals before the deployment fails. Changes to `VERSION` use that production deployment path so release-facing version surfaces stay current.

Use `docs/account-runtimes.md` as the style template for new long-form reference docs: short intro, route table, then behavior notes.
