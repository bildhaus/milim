import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, resolve } from "node:path";
import { chromium } from "playwright-core";

// Real Windows portable updater smoke. Every mutable path and profile is private.
// The candidate is staged locally: release discovery/download are separate checks.
const oldBinary = process.env.MILIM_UPGRADE_PREVIOUS_BINARY;
const candidate = process.env.MILIM_UPGRADE_CANDIDATE_BINARY;
const fromVersion = process.env.MILIM_UPGRADE_FROM_VERSION || "0.2.64";
const toVersion = process.env.MILIM_UPGRADE_TO_VERSION || "0.2.65";
const prepareOnly = process.argv.includes("--prepare-only");
const cdpPort = Number(process.env.MILIM_UPGRADE_CDP_PORT || 9677);
const cdpUrl = `http://127.0.0.1:${cdpPort}`;
const artifactDir = resolve(process.env.MILIM_UPGRADE_ARTIFACT_DIR || join(tmpdir(), `milim-upgrade-${fromVersion}-to-${toVersion}-${Date.now()}`));
if (process.platform !== "win32") throw new Error("This upgrade smoke targets Windows WebView2.");
if (!oldBinary || !existsSync(oldBinary)) throw new Error("Set MILIM_UPGRADE_PREVIOUS_BINARY to the downloaded public portable EXE.");
if (!prepareOnly && (!candidate || !existsSync(candidate))) throw new Error("Set MILIM_UPGRADE_CANDIDATE_BINARY to the newly built release EXE.");
const checksumPath = `${oldBinary}.sha256`;
if (!existsSync(checksumPath)) throw new Error("The public previous binary must have its downloaded .sha256 sidecar.");
const oldHash = hash(oldBinary);
ensure(readFileSync(checksumPath, "utf8").toLowerCase().includes(oldHash), "Previous public binary SHA-256 mismatch.");
mkdirSync(artifactDir, { recursive: true });
const isolatedHome = mkdtempSync(join(tmpdir(), "milim-upgrade-proof-"));
const installDir = join(isolatedHome, "install");
mkdirSync(installDir);
const installBinary = join(installDir, "milim-desktop.exe");
copyFileSync(oldBinary, installBinary);
const environment = isolatedEnvironment(isolatedHome);
const oldBackupPath = join(artifactDir, `previous-${fromVersion}.milim-backup.json`);
const newBackupPath = join(artifactDir, `upgraded-${toVersion}.milim-backup.json`);
const report = { fromVersion, toVersion, previousSha256: oldHash, candidateSha256: candidate ? hash(candidate) : null, isolatedHome, proof: "public-portable-native-apply-update", downloadGate: "not exercised: local candidate staging", checks: {} };
let session;
let failure;

try {
  ensure(!(await cdpReady()), `CDP port ${cdpPort} is already serving a browser.`);
  session = await launch();
  ensure(await invoke("plugin:app|version") === fromVersion, "The previous binary does not match the requested public version.");
  await seedPreviousState();
  await assertData(fromVersion);
  const oldBackup = await invoke("export_milim_backup", { path: oldBackupPath });
  ensure(oldBackup.summary.chats >= 2, "Previous-version backup is missing seeded chats.");
  report.checks.previous = { launchedPublicBinary: true, messages: 350, legacyDraft: true, settings: true, attachments: true, backupReadable: true };
  await session.page.screenshot({ path: join(artifactDir, "previous.png"), fullPage: false });
  if (!prepareOnly) {
    const localData = await invoke("plugin:path|resolve_directory", { directory: 15 });
    ensure(within(isolatedHome, localData), `App local data escaped the private profile: ${localData}`);
    const updateRoot = join(localData, "milim-updates");
    mkdirSync(updateRoot, { recursive: true });
    const staged = join(updateRoot, `milim-${toVersion}.exe`);
    copyFileSync(candidate, staged);
    ensure(hash(staged) === report.candidateSha256, "Locally staged update differs from candidate.");
    console.log(`[upgrade] invoking ${fromVersion} native updater for ${toVersion}`);
    await invoke("apply_update", { updatePath: staged }).catch((error) => {
      if (!/closed|destroyed|Target page|Execution context/i.test(String(error))) throw error;
    });
    await waitFor(() => session.child.exitCode != null, "previous executable exit", 20_000);
    await session.browser.close().catch(() => {});
    session = await reconnectAfterUpgrade();
    ensure(hash(installBinary) === report.candidateSha256, "Native updater did not replace the isolated executable with the candidate.");
    await assertData(toVersion);
    report.checks.upgrade = { nativeReplacement: true, nativeRestart: true, candidateHash: true, messages: 350, legacyDraftMigrated: true, settings: true, attachments: true };

    const linked = await command("upgrade-add-link", "thread.link.add", "upgrade-source", { target_thread_id: "upgrade-peer" });
    ensure(linked.status === "applied", `Could not seed canonical link: ${JSON.stringify(linked)}`);
    const newBackup = await invoke("export_milim_backup", { path: newBackupPath });
    ensure(newBackup.summary.chats >= 2 && newBackup.summary.controlRecords > 0, "New backup did not include canonical control records.");
    await command("upgrade-unlink", "thread.link.remove", "upgrade-source", { target_thread_id: "upgrade-peer" });
    await command("upgrade-rename", "thread.rename", "upgrade-source", { title: "MUTATED AFTER BACKUP" });
    await invoke("restore_milim_backup", { path: newBackupPath });
    await restartNativeAfterRestore();
    await assertData(toVersion);
    const restored = await bootstrap();
    ensure(restored.threads.find((thread) => thread.id === "upgrade-source")?.linked_threads.some((link) => link.target_thread_id === "upgrade-peer"), "New backup restore lost the canonical link.");
    report.checks.newBackupRoundTrip = { completeMessages: true, restoresTitle: true, restoresCanonicalLinks: true };

    await invoke("restore_milim_backup", { path: oldBackupPath });
    await restartNativeAfterRestore();
    await assertData(toVersion);
    report.checks.previousBackupCompatibility = { acceptedPreviousExport: true, messages: 350, legacyDraft: true, settings: true };
    await session.page.screenshot({ path: join(artifactDir, "upgraded-restored.png"), fullPage: false });
  }
  report.preparedOnly = prepareOnly;
  writeFileSync(join(artifactDir, "upgrade-proof.json"), JSON.stringify(report, null, 2));
  console.log(`upgradeProof=${join(artifactDir, "upgrade-proof.json")}`);
} catch (error) {
  failure = error;
  await session?.page?.screenshot({ path: join(artifactDir, "failure.png"), fullPage: false }).catch(() => {});
  writeFileSync(join(artifactDir, "upgrade-failure.json"), JSON.stringify({ ...report, error: String(error), stdout: session?.stdout, stderr: session?.stderr }, null, 2));
} finally {
  await closeIsolatedApp();
  // Retain only the isolated fixture and report for diagnosis; never touch user data.
}
if (failure) throw failure;

async function seedPreviousState() {
  await session.page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const settings = { state: { accountRuntimeEnabled: { codex: false, claude: false, opencode: false, pi: false }, globalInstructions: "UPGRADE_KEEP_GLOBAL_INSTRUCTIONS" }, version: 0 };
    const onboarding = { state: { version: 1, status: "completed", completedSteps: ["model", "context"], completedAt: Date.now() }, version: 0 };
    localStorage.setItem("milim.local.updates", JSON.stringify({ state: { automaticCheck: false, automaticDownload: false }, version: 1 }));
    localStorage.setItem("milim.local.update-cards.seen-version", await invoke("plugin:app|version"));
    await invoke("user_state_set", { key: "milim.settings", value: JSON.stringify(settings) });
    await invoke("user_state_set", { key: "milim.onboarding", value: JSON.stringify(onboarding) });
  });
  await restartRenderer();
  for (const id of ["upgrade-source", "upgrade-peer"]) {
    const result = await command(`${id}-create`, "thread.create", id, { id, title: id === "upgrade-source" ? "Upgrade preserved chat" : "Upgrade peer", settings: { model: "", folder: "", privacy: "off", toolApproval: "review" } });
    ensure(result.status === "applied", `Could not create isolated fixture ${id}.`);
  }
  await session.page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const raw = await invoke("user_state_get", { key: "milim.sessions" });
    const parsed = JSON.parse(raw);
    const state = parsed.state;
    const source = state.sessions.find((item) => item.id === "upgrade-source");
    if (!source) throw new Error("Canonical source did not appear in persisted state.");
    state.activeId = source.id;
    const meta = structuredClone(parsed);
    delete meta.state.sessions;
    const messages = Array.from({ length: 350 }, (_, index) => ({ id: `upgrade-message-${index}`, role: index % 2 ? "assistant" : "user", content: `UPGRADE_MESSAGE_${index}`, ...(index === 0 ? { attachments: [{ id: "upgrade-text", name: "saved.txt", mime: "text/plain", size: 14, content: "UPGRADE_FILE" }] } : {}) }));
    const { messages: _oldMessages, messagesHydrated, messagesLoadedFrom, persistedMessageCount, ...sourceMeta } = source;
    await invoke("user_sessions_apply_ops", { delta: { metaJson: JSON.stringify(meta), sessionOrder: state.sessions.map((item) => item.id), upserts: [{ id: source.id, sessionJson: JSON.stringify(sourceMeta), baseMessageCount: 0, messageCount: messages.length, messages: messages.map((message, index) => ({ index, messageJson: JSON.stringify(message) })) }], deletedSessionIds: [] } });
    await invoke("user_state_set", { key: "milim.sessionDrafts", value: JSON.stringify({ "upgrade-source": "UPGRADE_LEGACY_DRAFT" }) });
  });
  await restartRenderer();
}

async function assertData(version) {
  ensure(await invoke("plugin:app|version") === version, `Expected app version ${version}.`);
  const raw = await invoke("user_state_get", { key: "milim.sessions" });
  const source = JSON.parse(raw).state.sessions.find((item) => item.id === "upgrade-source");
  ensure(source?.title === "Upgrade preserved chat", "Source title was lost.");
  ensure(source.messages.length === 350 && source.messages[0].content === "UPGRADE_MESSAGE_0" && source.messages[349].content === "UPGRADE_MESSAGE_349", "Complete 350-message history was lost or reordered.");
  ensure(source.messages[0].attachments[0].content === "UPGRADE_FILE", "Persisted attachment was lost.");
  const settings = JSON.parse(await invoke("user_state_get", { key: "milim.settings" }));
  ensure(settings.state.globalInstructions === "UPGRADE_KEEP_GLOBAL_INSTRUCTIONS", "Global instructions were lost.");
  await waitFor(async () => await session.page.getByTestId("composer-input").inputValue().catch(() => "") === "UPGRADE_LEGACY_DRAFT", "restored legacy text draft", 15_000);
}

async function command(commandId, kind, threadId, payload) {
  return control("/control/v1/commands", { method: "POST", body: JSON.stringify({ command_id: commandId, kind, thread_id: threadId, payload }) });
}
async function bootstrap() { return control("/control/v1/bootstrap"); }
async function control(path, init = {}) {
  const credentials = await session.page.evaluate(async () => ({ base: await window.__TAURI_INTERNALS__.invoke("api_base_url"), token: await window.__TAURI_INTERNALS__.invoke("api_token") }));
  const response = await fetch(`${credentials.base}${path}`, { ...init, headers: { "Content-Type": "application/json", ...(credentials.token ? { Authorization: `Bearer ${credentials.token}` } : {}) }, signal: AbortSignal.timeout(10_000) });
  ensure(response.ok, `Control command failed: HTTP ${response.status}`);
  return response.json();
}
async function invoke(command, args) { return session.page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }); }
async function restartRenderer() {
  await session.page.reload({ waitUntil: "domcontentloaded" });
  await session.page.getByTestId("chat-shell").waitFor({ timeout: 30_000 });
}
async function restartNativeAfterRestore() {
  const previous = session;
  const disconnected = new Promise((resolve) => previous.browser.once("disconnected", resolve));
  await invoke("restart_app").catch((error) => {
    if (!/closed|destroyed|Target page|Execution context/i.test(String(error))) throw error;
  });
  let timeout;
  try {
    await Promise.race([disconnected, new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error("Native restore restart did not disconnect the old WebView.")), 20_000); })]);
  } finally {
    clearTimeout(timeout);
  }
  await previous.browser.close().catch(() => {});
  session = await reconnectAfterUpgrade();
  await session.page.getByTestId("chat-shell").waitFor({ timeout: 30_000 });
}
async function launch() {
  const child = spawn(installBinary, [], { cwd: installDir, env: environment, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  const result = { child, stdout: "", stderr: "", browser: null, page: null };
  child.stdout.on("data", (chunk) => { result.stdout += chunk; });
  child.stderr.on("data", (chunk) => { result.stderr += chunk; });
  session = result;
  await waitFor(() => { if (child.exitCode != null) throw new Error(`Previous binary exited: ${result.stderr}`); return cdpReady(); }, "isolated WebView2 CDP", 60_000);
  return attach(result);
}
async function reconnectAfterUpgrade() {
  await waitFor(async () => hash(installBinary) === report.candidateSha256 && await cdpReady(), "native updater replacement and relaunch", 60_000);
  return attach({ child: null, stdout: "", stderr: "", browser: null, page: null });
}
async function attach(result) {
  result.browser = await chromium.connectOverCDP(cdpUrl);
  await waitFor(async () => {
    for (const page of result.browser.contexts().flatMap((context) => context.pages())) {
      if (await page.evaluate(() => Boolean(window.__TAURI_INTERNALS__?.invoke)).catch(() => false)) { result.page = page; return true; }
    }
    return false;
  }, "native Tauri IPC bridge", 20_000);
  result.page.setDefaultTimeout(15_000);
  return result;
}
async function cdpReady() {
  try { const response = await fetch(`${cdpUrl}/json/list`, { signal: AbortSignal.timeout(1000) }); return response.ok && (await response.json()).length > 0; } catch { return false; }
}
async function closeIsolatedApp() {
  await session?.page?.locator(".win-close").click({ timeout: 1000 }).catch(() => {});
  await session?.browser?.close().catch(() => {});
  const escaped = installBinary.replaceAll("'", "''");
  // Exact executable path under our verified private install directory only.
  ensure(within(isolatedHome, installBinary), "Refusing process cleanup outside isolated home.");
  spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `Get-CimInstance Win32_Process -Filter \"Name = 'milim-desktop.exe'\" | Where-Object { $_.ExecutablePath -eq '${escaped}' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`], { windowsHide: true, stdio: "ignore" });
}
function isolatedEnvironment(home) {
  const systemRoot = process.env.SystemRoot || "C:\\Windows";
  const profile = join(home, "profile");
  const bin = join(home, "empty-path");
  for (const path of [profile, bin, join(profile, "AppData", "Roaming"), join(profile, "AppData", "Local")]) mkdirSync(path, { recursive: true });
  const stub = join(bin, "codex-stub.mjs");
  writeFileSync(stub, `import readline from 'node:readline'; readline.createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);if(m.id!=null)process.stdout.write(JSON.stringify({id:m.id,result:m.method==='account/read'?{requiresOpenaiAuth:true,account:null}:m.method==='model/list'?{data:[]}: {}})+'\\n');});`);
  for (const name of ["codex", "claude", "opencode", "pi"]) {
    copyFileSync(join(systemRoot, "System32", "where.exe"), join(bin, `${name}.exe`));
    writeFileSync(join(bin, `${name}.cmd`), name === "codex" ? `@"${process.execPath}" "${stub}" %*\r\n` : "@exit /b 1\r\n");
  }
  return { SystemRoot: systemRoot, WINDIR: systemRoot, COMSPEC: join(systemRoot, "System32", "cmd.exe"), TEMP: home, TMP: home, PATH: `${bin};${join(systemRoot, "System32")};${join(systemRoot, "System32", "WindowsPowerShell", "v1.0")}`, USERPROFILE: profile, HOME: profile, APPDATA: join(profile, "AppData", "Roaming"), LOCALAPPDATA: join(profile, "AppData", "Local"), CODEX_HOME: join(profile, ".codex"), CLAUDE_CONFIG_DIR: join(profile, ".claude"), XDG_CONFIG_HOME: join(profile, ".config"), XDG_DATA_HOME: join(profile, ".local", "share"), MILIM_HOME: home, WEBVIEW2_USER_DATA_FOLDER: join(home, "webview2"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}` };
}
function within(parent, child) { const path = relative(resolve(parent).toLowerCase(), resolve(child).toLowerCase()); return path === "" || (!path.startsWith("..") && !isAbsolute(path)); }
function hash(path) { return createHash("sha256").update(readFileSync(path)).digest("hex"); }
function ensure(condition, message) { if (!condition) throw new Error(message); }
async function waitFor(check, label, timeoutMs) { const until = Date.now() + timeoutMs; while (Date.now() < until) { if (await check()) return; await new Promise((resolve) => setTimeout(resolve, 200)); } throw new Error(`Timed out waiting for ${label}.`); }
