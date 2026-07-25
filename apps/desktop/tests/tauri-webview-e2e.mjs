import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import http from "node:http";
import net from "node:net";
import { chromium } from "playwright-core";

const root = fileURLToPath(new URL("..", import.meta.url));
const binary = process.env.MILIM_TAURI_E2E_BINARY || join(root, "src-tauri", "target", "tauri-verify", "debug", "milim-desktop.exe");
const cdpHost = "127.0.0.1";
const cdpPort = Number(process.env.MILIM_TAURI_E2E_CDP_PORT || 9333);
const cdpUrl = `http://${cdpHost}:${cdpPort}`;
const nativePreviewOnly = process.argv.includes("--native-preview-only");
const browserProfileOnly = process.argv.includes("--browser-profile-only");
const zoomOnly = process.argv.includes("--zoom-only");
const microUiOnly = process.argv.includes("--micro-ui-only");
const workersOnly = process.argv.includes("--workers-only");
const mcpAppsOnly = process.argv.includes("--mcp-apps-only");
const sidebarMotionOnly = process.argv.includes("--sidebar-motion-only");
const newChatSplitOnly = process.argv.includes("--new-chat-split-only");
const commandPaletteOnly = process.argv.includes("--command-palette-only");
const settingsOnly = process.argv.includes("--settings-only");
const appMenuOnly = process.argv.includes("--app-menu-only");
const turnChangesOnly = process.argv.includes("--turn-changes-only");
const mcpAppKinds = ["chart", "diagram", "form", "dashboard", "viewer"];
const screenshots = {
  avatars: join(tmpdir(), "milim-tauri-webview-agent-avatars.png"),
  avatarsLight: join(tmpdir(), "milim-tauri-webview-agent-avatars-light.png"),
  profiles: join(tmpdir(), "milim-tauri-webview-personalized-profiles.png"),
  settings: join(tmpdir(), "milim-tauri-webview-provider-settings.png"),
  settingsNarrow: join(tmpdir(), "milim-tauri-webview-settings-narrow.png"),
  chat: join(tmpdir(), "milim-tauri-webview-personalized-chat.png"),
  zoom: join(tmpdir(), "milim-tauri-webview-zoom-chip.png"),
  accountUsage: join(tmpdir(), "milim-tauri-webview-account-usage.png"),
  microUi: join(tmpdir(), "milim-tauri-webview-micro-ui.png"),
  inspectorOverlay: join(tmpdir(), "milim-tauri-webview-inspector-overlay.png"),
  workersPlan: join(tmpdir(), "milim-tauri-webview-workers-plan.png"),
  workersNarrow: join(tmpdir(), "milim-tauri-webview-workers-narrow.png"),
  newChatSplit: join(tmpdir(), "milim-tauri-webview-new-chat-split.png"),
  mcpAppsLight: join(tmpdir(), "milim-tauri-webview-mcp-apps-light.png"),
  mcpAppsDark: join(tmpdir(), "milim-tauri-webview-mcp-apps-dark.png"),
  turnChanges: join(tmpdir(), "milim-tauri-webview-turn-changes.png"),
  failure: join(tmpdir(), "milim-tauri-webview-failure.png"),
};

const profiles = [
  {
    name: "Code Reviewer",
    avatar: "CR",
    mode: "custom",
    tools: ["read_file", "list_dir", "edit_file"],
    prompt: "Review code for correctness, regressions, missing tests, and concise file-level findings.",
  },
  {
    name: "Security Review",
    avatar: "🛡️",
    mode: "custom",
    tools: ["read_file", "list_dir", "http_fetch"],
    prompt: "Find credential leaks, unsafe commands, weak sandboxing, and external action risks.",
  },
  {
    name: "Prompt Enhancer",
    avatar: "PE",
    mode: "none",
    tools: [],
    prompt: "Improve prompts while preserving constraints, intent, and output shape.",
  },
  {
    name: "Media Workflow Planner",
    avatar: "MW",
    mode: "all",
    tools: [],
    prompt: "Plan image, video, and audio generation workflows with provider choice, queue handling, gallery review, and history checks.",
  },
];

if (process.platform !== "win32") {
  console.log("Skipping Tauri WebView2 E2E: this test currently targets Windows WebView2.");
  process.exit(0);
}

if (!existsSync(binary)) {
  throw new Error(`Tauri binary not found. Run npm run verify:tauri first. Missing: ${binary}`);
}

await ensureNoWorkspaceMilimProcesses();

if (await isPortOpen(cdpPort)) {
  throw new Error(`CDP port ${cdpPort} is already in use.`);
}

const milimHome = mkdtempSync(join(tmpdir(), "milim-tauri-e2e-"));
const consoleErrors = [];
let session;
let failure;
let turnChangesRepo;

try {
  session = await launchTauri(milimHome);
  await resetFrontendStorage(session.page);
  if (turnChangesOnly) {
    const errors = collectErrors(session.page);
    turnChangesRepo = createTurnChangesRepo();
    await runTurnChangesCheck(session.page, turnChangesRepo);
    consoleErrors.push(...errors);
  } else if (browserProfileOnly) {
    const errors = collectErrors(session.page);
    await runBrowserProfileCheck(session);
    consoleErrors.push(...errors);
  } else if (nativePreviewOnly) {
    const errors = collectErrors(session.page);
    await runNativePreviewOcclusionCheck(session.page, session.child.pid);
    consoleErrors.push(...errors);
  } else if (sidebarMotionOnly || newChatSplitOnly) {
    const errors = collectErrors(session.page);
    await runSidebarSectionMotionCheck(session.page, newChatSplitOnly);
    consoleErrors.push(...errors);
  } else if (mcpAppsOnly) {
    await session.page.getByTestId("chat-shell").waitFor();
    await dismissOnboardingIfPresent(session.page);
    await runMcpAppsCheck(session.page);
  } else if (workersOnly) {
    const errors = collectErrors(session.page);
    await runWorkersInspectorCheck(session.page, milimHome);
    consoleErrors.push(...errors.filter((message) => !message.includes("/worker-runs/e2e-workers-run/events")));
  } else if (appMenuOnly) {
    const errors = collectErrors(session.page);
    await session.page.getByTestId("chat-shell").waitFor();
    await dismissOnboardingIfPresent(session.page);
    await runAppMenuCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (commandPaletteOnly) {
    const errors = collectErrors(session.page);
    await session.page.getByTestId("chat-shell").waitFor();
    await dismissOnboardingIfPresent(session.page);
    await seedChatSearchFixture(session.page);
    await runCommandPaletteCheck(session.page);
    await runRestartCheck(session);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (settingsOnly) {
    const errors = collectErrors(session.page);
    await runSettingsLayoutCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (zoomOnly || microUiOnly) {
    const errors = collectErrors(session.page);
    await session.page.getByTestId("chat-shell").waitFor();
    await dismissOnboardingIfPresent(session.page);
    await runUiZoomShortcutCheck(session.page);
    await runAccountUsageTitleBarCheck(session.page);
    if (microUiOnly) await runMicroUiCheck(session.page);
    consoleErrors.push(...errors);
  } else {
    consoleErrors.push(...(await runProfileSetup(session.page)));
    await session.page.screenshot({ path: screenshots.profiles, fullPage: false });
    consoleErrors.push(...(await runProviderSetup(session.page)));
    await session.page.screenshot({ path: screenshots.settings, fullPage: false });
    await closeSettings(session.page);
    await closeSession(session);
    session = null;

    session = await launchTauri(milimHome);
    consoleErrors.push(...(await runPersistenceAndChat(session.page, session.child.pid)));
    await session.page.screenshot({ path: screenshots.chat, fullPage: false });
  }

  if (consoleErrors.length) {
    throw new Error(`Console errors during Tauri WebView E2E:\n${consoleErrors.join("\n")}`);
  }
} catch (err) {
  failure = err;
  if (session?.page && !session.page.isClosed()) {
    await session.page.screenshot({ path: screenshots.failure, fullPage: false }).catch((screenshotErr) => {
      console.error(`failureScreenshotError=${screenshotErr.message}`);
    });
  }
} finally {
  const cleanupErrors = [];
  if (session) {
    await closeSession(session).catch((err) => cleanupErrors.push(err));
  }
  await ensureNoWorkspaceMilimProcesses().catch((err) => cleanupErrors.push(err));
  await rmWithRetry(milimHome, { label: "MILIM_HOME" }).catch((err) => cleanupErrors.push(err));
  if (turnChangesRepo) {
    await rmWithRetry(turnChangesRepo.folder, { label: "turn changes repository" }).catch((err) => cleanupErrors.push(err));
  }
  printEvidencePaths(milimHome);
  if (cleanupErrors.length) {
    const cleanupMessage = cleanupErrors.map((err) => err.stack || err.message || String(err)).join("\n\n");
    if (failure) {
      failure = new Error(`${failure.stack || failure.message || String(failure)}\n\nCleanup errors:\n${cleanupMessage}`);
    } else {
      failure = new Error(`Tauri WebView E2E cleanup failed:\n${cleanupMessage}`);
    }
  }
}

if (failure) throw failure;

function createTurnChangesRepo() {
  const folder = mkdtempSync(join(tmpdir(), "milim-turn-changes-e2e-"));
  const checkpoint = "refs/milim/checkpoints/e2e-turn-changes";
  runGit(folder, ["init"]);
  runGit(folder, ["config", "user.email", "milim-e2e@example.test"]);
  runGit(folder, ["config", "user.name", "Milim E2E"]);
  for (let index = 1; index <= 5; index += 1) {
    writeFileSync(join(folder, `file-${index}.txt`), "before\n", "utf8");
  }
  runGit(folder, ["add", "."]);
  runGit(folder, ["commit", "-m", "Initial fixture"]);
  runGit(folder, ["update-ref", checkpoint, "HEAD"]);
  for (let index = 1; index <= 5; index += 1) {
    writeFileSync(join(folder, `file-${index}.txt`), `before\nchange ${index}\n`, "utf8");
  }
  return { folder, checkpoint };
}

function runGit(folder, args) {
  const result = spawnSync("git", args, { cwd: folder, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Git fixture command failed: git ${args.join(" ")}\n${result.stderr}`);
  }
}

async function runTurnChangesCheck(page, fixture) {
  const gitActionRequests = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/workspace/git/action")) {
      gitActionRequests.push(request.postData() ?? "");
    }
  });
  const now = Date.now();
  await page.evaluate(async ({ folder, checkpoint, timestamp }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const settings = {
      model: "",
      instructions: "",
      activeAgentId: null,
      folder,
      sandbox: false,
      computerUse: false,
      memory: false,
      privacy: "off",
      toolApproval: "review",
      delegationPolicy: "off",
      workerModel: "",
      planMode: false,
    };
    const workspaceCheckpoint = { ref: checkpoint, createdAt: timestamp, folder };
    const value = JSON.stringify({
      state: {
        sessions: [{
          id: "e2e-turn-changes",
          title: "Turn changes fixture",
          messages: [
            { id: "old-request", role: "user", content: "Previous request" },
            { id: "old-response", role: "assistant", content: "Previous response", workspaceCheckpoint },
            { id: "latest-request", role: "user", content: "Please update the fixture files" },
            { id: "latest-response", role: "assistant", content: "Updated all fixture files.", workspaceCheckpoint },
          ],
          settings,
          createdAt: timestamp,
          updatedAt: timestamp,
        }],
        activeId: "e2e-turn-changes",
      },
      version: 0,
    });
    await invoke("user_sessions_set", { value });
  }, { ...fixture, timestamp: now });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);

  const card = page.getByTestId("turn-changes-card");
  await card.waitFor();
  if (await page.getByTestId("turn-changes-card").count() !== 1) {
    throw new Error("Only the latest assistant response should show a turn changes card.");
  }
  await assertTextContains(card, "Changed 5 files");
  await assertTextContains(card, "+5");
  await assertTextContains(card, "-0");
  await assertTextContains(card, "file-1.txt");
  await assertHidden(card.getByText("file-4.txt", { exact: true }), "fourth changed path before expansion");
  await page.screenshot({ path: screenshots.turnChanges, fullPage: false });
  await card.getByTestId("turn-changes-toggle").click();
  await card.getByText("file-4.txt", { exact: true }).waitFor();

  const requestsBeforeReview = gitActionRequests.length;
  await card.getByTestId("turn-changes-review").click();
  const gitPanel = page.getByTestId("git-workspace-panel");
  await gitPanel.waitFor();
  await assertTextContains(gitPanel.getByLabel("Diff scope"), "Last turn");
  await assertTextContains(gitPanel.getByLabel("Changed files", { exact: true }), "file-5.txt");
  await page.waitForTimeout(300);
  if (gitActionRequests.length !== requestsBeforeReview) {
    throw new Error("Review changes should use the cached turn diff without another Git action request.");
  }

  await page.getByRole("button", { name: "Close Git panel" }).click();
  await gitPanel.waitFor({ state: "hidden" });
  await page.evaluate(() => {
    window.confirm = () => true;
  });
  await card.getByTestId("turn-changes-undo").click();
  await card.waitFor({ state: "hidden" });
  await page.getByText("Please update the fixture files", { exact: true }).waitFor();
  await assertHidden(page.getByText("Updated all fixture files.", { exact: true }), "removed assistant response");
  for (let index = 1; index <= 5; index += 1) {
    const content = readFileSync(join(fixture.folder, `file-${index}.txt`), "utf8").replaceAll("\r\n", "\n");
    if (content !== "before\n") throw new Error(`Undo did not restore file-${index}.txt.`);
  }
}

async function runProfileSetup(page) {
  const errors = collectErrors(page);
  await page.getByTestId("chat-shell").waitFor();
  await runWindowPinCheck(page);
  await openAgents(page);

  for (const profile of profiles) {
    await createAgent(page, profile);
  }

  for (const profile of profiles) {
    const card = page.getByTestId(`agent-editor-${profile.name}`);
    await card.waitFor();
    await assertAvatarSeed(card.locator("shatz-avatar"), profile.avatar);
  }

  await page.getByTestId("agent-editor-Security Review").click();
  await assertFieldContains(page.getByTestId("agent-system-prompt"), "credential leaks");
  await assertToolMode(page, "custom");
  await assertSelectedTools(page, profiles.find((p) => p.name === "Security Review").tools);
  await page.screenshot({ path: screenshots.avatars, fullPage: false });

  await closeAgents(page);
  await assertAgentAvatarsInLightTheme(page);
  await assertAgentOptions(page);
  await assertScheduleAgentAvatar(page, profiles[0]);
  return errors;
}

async function runPersistenceAndChat(page, pid) {
  const errors = collectErrors(page);
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await runAppMenuCheck(page);
  await runNativePreviewOcclusionCheck(page, pid);
  await assertAgentOptions(page);
  await openSettings(page);
  await assertAppShortcutsPersisted(page);
  await closeSettings(page);
  await runModelPickerSurfaceCheck(page);
  await runAppShortcutCheck(page);

  await runSlashAndAttachmentCheck(page);
  await runContextDrawerCheck(page);
  await runMemoryLibraryCheck(page);
  await runContextMenuChromeCheck(page);

  if (await hasChatModel(page)) {
    await selectAgent(page, "Prompt Enhancer");
    await switchModelWhileAgentActive(page, "Prompt Enhancer");
    await page.getByTestId("composer-input").fill("hello from personalized profile");
    await page.getByTestId("composer-send").click();
    await page.getByTestId("assistant-message").last().waitFor({ timeout: 60_000 });
    await runMessagePopoverLayerCheck(page);
    await runMessageContextMenuCheck(page);
    if (process.env.MILIM_TAURI_E2E_ARTIFACTS === "1") {
      await runArtifactCheck(page);
    } else {
      console.log("artifactGenerationChecks=skipped:set MILIM_TAURI_E2E_ARTIFACTS=1 to run real-model artifact prompts");
    }
  } else {
    console.log("generationChecks=skipped:no chat model configured");
  }

  await openAgentMenu(page);
  await page.getByTestId("manage-agents").click();
  await page.getByTestId("agent-editor-Security Review").click();
  await assertFieldContains(page.getByTestId("agent-system-prompt"), "credential leaks");
  await assertToolMode(page, "custom");
  await assertSelectedTools(page, profiles.find((p) => p.name === "Security Review").tools);
  await closeAgents(page);

  return errors;
}

async function runSidebarSectionMotionCheck(page, splitOnly = false) {
  if (splitOnly) {
    const standaloneNewChat = page.getByRole("button", { name: "New chat", exact: true });
    const standaloneLayout = await standaloneNewChat.evaluate((button) => {
      const style = getComputedStyle(button);
      return {
        wrapped: button.parentElement?.classList.contains("new-chat-actions"),
        border: style.borderTopWidth,
        radius: style.borderTopRightRadius,
      };
    });
    if (standaloneLayout.wrapped || standaloneLayout.border !== "1px" || standaloneLayout.radius !== "8px") {
      throw new Error(`Gitless new chat button has unexpected geometry: ${JSON.stringify(standaloneLayout)}.`);
    }
  }
  const project = await page.evaluate(async ({ folder }) => {
    const key = "milim.sessions";
    const id = `project:${folder}`;
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const raw = await invoke("user_state_get", { key });
    const parsed = raw ? JSON.parse(raw) : { state: {} };
    const state = parsed.state && typeof parsed.state === "object" ? parsed.state : {};
    const now = Date.now();
    state.sessions = [
      ...(Array.isArray(state.sessions) ? state.sessions : []).filter((session) => session?.id !== "e2e-project-chat"),
      { id: "e2e-project-chat", title: "Project chat", messages: [], settings: { folder }, createdAt: now, updatedAt: now },
    ];
    state.projects = [
      ...(Array.isArray(state.projects) ? state.projects : []).filter((item) => item?.id !== id),
      { id, name: "E2E Project", folder, createdAt: now, updatedAt: now },
    ];
    state.sidebar = { ...(state.sidebar ?? {}), projectFolders: [folder] };
    state.activeId = "e2e-project-chat";
    parsed.state = state;
    await invoke("user_state_set", { key, value: JSON.stringify(parsed) });
    await invoke("user_state_set", {
      key: "milim.onboarding",
      value: JSON.stringify({
        state: {
          version: 1,
          status: "completed",
          selectedSetupPath: null,
          completedSteps: ["finish"],
          developerShowOnboarding: false,
          completedAt: now,
        },
        version: 0,
      }),
    });
    return { name: "E2E Project" };
  }, { folder: root });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  const newChatMenu = page.getByRole("button", { name: "Choose new chat workspace", exact: true });
  const splitLayout = await newChatMenu.evaluate((menu) => {
    const style = getComputedStyle(menu);
    const shellStyle = getComputedStyle(menu.parentElement);
    return {
      width: menu.getBoundingClientRect().width,
      leftRadius: style.borderTopLeftRadius,
      rightRadius: style.borderTopRightRadius,
      topBorder: style.borderTopWidth,
      leftBorder: style.borderLeftWidth,
      shellRightRadius: shellStyle.borderTopRightRadius,
    };
  });
  if (
    splitLayout.width !== 24 ||
    splitLayout.leftRadius !== "0px" ||
    splitLayout.rightRadius !== "0px" ||
    splitLayout.topBorder !== "0px" ||
    splitLayout.leftBorder !== "1px" ||
    splitLayout.shellRightRadius !== "8px"
  ) {
    throw new Error(`New chat split control has unexpected geometry: ${JSON.stringify(splitLayout)}.`);
  }
  await newChatMenu.click();
  await page.getByRole("menu", { name: "New chat workspace", exact: true }).waitFor();
  await page.getByRole("menuitem", { name: "Current checkout", exact: true }).waitFor();
  const isolatedWorktree = page.getByRole("menuitem", { name: /Isolated worktree/ });
  await isolatedWorktree.waitFor();
  const isolatedLayout = await isolatedWorktree.evaluate((element) => {
    const label = element.querySelector(".app-context-menu-label").getBoundingClientRect();
    const description = element.querySelector(".app-context-menu-description").getBoundingClientRect();
    return {
      menuWidth: element.closest(".app-context-menu").getBoundingClientRect().width,
      labelBottom: label.bottom,
      descriptionTop: description.top,
    };
  });
  if (isolatedLayout.descriptionTop < isolatedLayout.labelBottom || isolatedLayout.menuWidth > 240) {
    throw new Error(`Worktree note should sit below its title in a compact menu: ${JSON.stringify(isolatedLayout)}.`);
  }
  await page.screenshot({ path: screenshots.newChatSplit, fullPage: false });
  await page.keyboard.press("Escape");
  if (splitOnly) return;
  const sidebarMotion = await page.locator(".sidebar").evaluate((element) => {
    const style = getComputedStyle(element);
    return { duration: style.transitionDuration, property: style.transitionProperty };
  });
  if (!sidebarMotion.property.includes("width") || !sidebarMotion.duration.includes("0.18s")) {
    throw new Error(`Sidebar width should use the shared 180ms transition: ${JSON.stringify(sidebarMotion)}.`);
  }
  const collapse = page.getByRole("button", { name: `Collapse ${project.name}`, exact: true });
  await collapse.waitFor();
  const section = page.locator("[data-sidebar-section-id]", { hasText: project.name });
  const reveal = section.locator(".context-section-reveal");
  const expanded = await reveal.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      height: element.getBoundingClientRect().height,
      transitionDuration: style.transitionDuration,
      transitionProperty: style.transitionProperty,
      visibility: style.visibility,
    };
  });
  if (expanded.height <= 0 || expanded.visibility !== "visible") {
    throw new Error(`Expected the project section to start expanded, got ${JSON.stringify(expanded)}.`);
  }
  if (!expanded.transitionDuration.includes("0.12s") || !expanded.transitionProperty.includes("grid-template-rows")) {
    throw new Error(`Project section did not reuse the app collapse motion: ${JSON.stringify(expanded)}.`);
  }

  await collapse.click();
  await assertAttribute(reveal, "aria-hidden", "true");
  if (await reveal.count() !== 1) throw new Error("Collapsed project content should remain mounted for its exit motion.");
  await delay(160);
  const collapsed = await reveal.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    visibility: getComputedStyle(element).visibility,
  }));
  if (collapsed.height > 0.5 || collapsed.visibility !== "hidden") {
    throw new Error(`Expected the project section collapse motion to finish hidden, got ${JSON.stringify(collapsed)}.`);
  }

  await page.getByRole("button", { name: `Expand ${project.name}`, exact: true }).click();
  await delay(160);
  const reopened = await reveal.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    visibility: getComputedStyle(element).visibility,
  }));
  if (reopened.height <= 0 || reopened.visibility !== "visible") {
    throw new Error(`Expected the project section expand motion to restore its content, got ${JSON.stringify(reopened)}.`);
  }

  await page.emulateMedia({ reducedMotion: "reduce" });
  const reduced = await reveal.evaluate((element) => getComputedStyle(element).transitionDuration);
  const reducedSidebarProperty = await page.locator(".sidebar").evaluate((element) =>
    getComputedStyle(element).transitionProperty);
  const reducedStatus = await page.evaluate(() => {
    const loader = document.createElement("button");
    loader.className = "btn-ghost running";
    const preview = document.createElement("div");
    preview.className = "preview-control-overlay running move";
    const cursor = document.createElement("span");
    cursor.className = "preview-control-cursor";
    preview.append(cursor);
    document.body.append(loader, preview);
    const loaderStyle = getComputedStyle(loader, "::before");
    const result = {
      loaderAnimation: loaderStyle.animationName,
      loaderOpacity: Number.parseFloat(loaderStyle.opacity),
      previewAnimation: getComputedStyle(cursor).animationName,
      previewOpacity: Number.parseFloat(getComputedStyle(preview).opacity),
    };
    loader.remove();
    preview.remove();
    return result;
  });
  if (
    reduced !== "0s" ||
    reducedSidebarProperty.includes("width") ||
    reducedStatus.loaderAnimation !== "none" ||
    reducedStatus.loaderOpacity <= 0 ||
    reducedStatus.previewAnimation !== "none" ||
    reducedStatus.previewOpacity <= 0
  ) {
    throw new Error(`Reduced motion should remove movement but preserve status: ${JSON.stringify({ reduced, reducedSidebarProperty, reducedStatus })}.`);
  }
  await page.emulateMedia({ reducedMotion: "no-preference" });
}

async function runMcpAppsCheck(page) {
  const browserErrors = collectErrors(page);
  const fixture = join(root, "..", "..", "crates", "milim-mcp-client", "tests", "fixtures", "apps_server.js");
  const host = await page.evaluate(async ({ fixturePath }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const [base, token] = await Promise.all([invoke("api_base_url"), invoke("api_token")]);
    const response = await fetch(`${base}/mcp/servers`, {
      method: "POST",
      headers: { "Authorization": `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({
        id: "e2e-mcp-apps",
        name: "E2E MCP Apps",
        command: "node",
        args: [fixturePath],
        enabled: true,
      }),
    });
    if (!response.ok) throw new Error(`MCP fixture setup failed: ${response.status} ${await response.text()}`);
    const now = Date.now();
    const views = [
      { kind: "chart", title: "Usage trend", result: { values: [35, 80, 58], labels: ["Build", "Test", "Ship"], total: "42.8k" } },
      { kind: "diagram", title: "Agent flow", result: { nodes: ["Model", "Tool", "View"] } },
      { kind: "form", title: "Run configuration", result: { model: "GPT-5.5", iterations: 6 } },
      { kind: "dashboard", title: "Provider health", result: { latency: "284 ms", success: "99.7%", queue: 3 } },
      { kind: "viewer", title: "Structured result", result: { status: "ready", files: 12, changed: 3 } },
    ];
    const value = JSON.stringify({
      state: {
        sessions: [{
          id: "e2e-mcp-apps-thread",
          title: "MCP Apps fixture",
          messages: [
            { role: "user", content: "Show an inline chart, diagram, form, dashboard, and viewer." },
            {
              id: "e2e-mcp-apps-turn",
              role: "assistant",
              content: "",
              streamParts: views.map((view, index) => ({
                kind: "event",
                eventType: "tool",
                label: `Used show_${view.kind}`,
                name: `show_${view.kind}`,
                callId: `e2e-call-${index}`,
                icon: "tool",
                status: "done",
                toolArguments: "{}",
                mcpApp: {
                  server_id: "e2e-mcp-apps",
                  resource_uri: `ui://milim.test/${view.kind}`,
                  tool: {
                    name: `show_${view.kind}`,
                    title: view.title,
                    description: `Show a ${view.kind}`,
                    inputSchema: { type: "object" },
                    _meta: { ui: { resourceUri: `ui://milim.test/${view.kind}` } },
                  },
                },
                mcpAppResult: {
                  content: [{ type: "text", text: `${view.title} data` }],
                  structuredContent: view.result,
                  _meta: { refreshCount: 0 },
                },
              })),
            },
          ],
          settings: { model: "", instructions: "", activeAgentId: null, folder: "", sandbox: false, computerUse: false, memory: false, privacy: "off", toolApproval: "review", delegationPolicy: "off", workerModel: "", planMode: false },
          createdAt: now,
          updatedAt: now,
        }],
        activeId: "e2e-mcp-apps-thread",
      },
      version: 0,
    });
    await invoke("user_sessions_set", { value });
    return { base, token };
  }, { fixturePath: fixture });

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  const kinds = mcpAppKinds;
  const apps = page.getByTestId("mcp-app-view");
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="mcp-app-view"]').length === 5);
  for (const kind of kinds) {
    await page
      .frameLocator(`iframe[title="MCP App ui://milim.test/${kind}"]`)
      .locator(`body[data-view="${kind}"][data-ready="true"]`)
      .waitFor({ timeout: 15_000 });
  }

  const lightStyles = await setMcpAppsTheme(page, kinds, "Mono Light", "light");
  await page.getByTestId("assistant-message").last().screenshot({ path: screenshots.mcpAppsLight });
  await captureMcpAppViewScreenshots(apps, kinds, "light");

  const app = apps.first();
  const iframe = app.locator("iframe");
  await iframe.waitFor();
  const frame = page.frameLocator('iframe[title="MCP App ui://milim.test/chart"]');
  await frame.getByRole("button", { name: "Refresh" }).waitFor({ timeout: 15_000 });
  await frame.locator("#security[data-parent-dom][data-storage]").waitFor({ state: "attached", timeout: 15_000 }).catch(async (error) => {
    const diagnostics = {
      appText: await app.innerText().catch(() => ""),
      csp: await frame.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute("content").catch(() => null),
      scripts: await frame.locator("script").count().catch(() => -1),
      scriptText: await frame.locator("script").first().textContent().catch(() => null),
      browserErrors,
    };
    throw new Error(`${error.message}\nMCP App diagnostics: ${JSON.stringify(diagnostics)}`);
  });

  const isolation = {
    parentDom: await frame.locator("#security").getAttribute("data-parent-dom"),
    storage: await frame.locator("#security").getAttribute("data-storage"),
    sandbox: await iframe.getAttribute("sandbox"),
  };
  if (isolation.parentDom !== "blocked") {
    throw new Error(`MCP App could access Milim's parent DOM: ${JSON.stringify(isolation)}`);
  }
  if (isolation.storage !== "blocked") {
    throw new Error(`MCP App received a persistent storage origin: ${JSON.stringify(isolation)}`);
  }
  const viewUrl = await iframe.getAttribute("src");
  if (!viewUrl || viewUrl.includes(host.token)) throw new Error("MCP App view URL is missing or contains Milim's bearer token.");
  const frameHeight = await iframe.evaluate((element) => element.getBoundingClientRect().height);
  if (Math.abs(frameHeight - 180) > 2) throw new Error(`MCP App resize was not applied: ${frameHeight}`);

  const network = await frame.locator("body").evaluate(async (_body, url) => {
    try {
      await fetch(url);
      return "allowed";
    } catch {
      return "blocked";
    }
  }, `${host.base}/health`);
  if (network !== "blocked") throw new Error("MCP App bypassed its default-deny network CSP.");

  await frame.getByRole("button", { name: "Refresh" }).click();
  const approval = app.locator(".mcp-app-approval");
  await approval.waitFor();
  const approvalText = await approval.innerText();
  if (!approvalText.includes("refresh_chart") || !approvalText.includes("{}")) {
    throw new Error(`MCP App Review did not show the exact call: ${approvalText}`);
  }
  await approval.getByRole("button", { name: "Approve once" }).click();
  await frame.locator("body[data-refresh-count='1']").waitFor();

  const form = page.frameLocator('iframe[title="MCP App ui://milim.test/form"]');
  await form.getByLabel("Iterations").fill("9");
  await form.getByRole("button", { name: "Validate" }).click();
  await form.locator("#form-status").filter({ hasText: "Validated" }).waitFor();
  await page.frameLocator('iframe[title="MCP App ui://milim.test/diagram"]').getByLabel("Tool execution diagram").waitFor();
  await page.frameLocator('iframe[title="MCP App ui://milim.test/dashboard"]').getByText("99.7%", { exact: true }).waitFor();
  await page.frameLocator('iframe[title="MCP App ui://milim.test/viewer"]').getByText('"files": 12', { exact: false }).waitFor();

  const darkStyles = await setMcpAppsTheme(page, kinds, "Mono Dark", "dark");
  assertMcpAppsThemeStyles(lightStyles, darkStyles);
  await page.getByTestId("assistant-message").last().screenshot({ path: screenshots.mcpAppsDark });
  await captureMcpAppViewScreenshots(apps, kinds, "dark");
}

async function captureMcpAppViewScreenshots(apps, kinds, theme) {
  for (const [index, kind] of kinds.entries()) {
    await apps.nth(index).screenshot({ path: mcpAppViewScreenshot(kind, theme) });
  }
}

function mcpAppViewScreenshot(kind, theme) {
  return join(tmpdir(), `milim-tauri-webview-mcp-app-${kind}-${theme}.png`);
}

async function setMcpAppsTheme(page, kinds, themeName, expectedTheme) {
  await openSettings(page);
  await page.getByTestId("settings-section-appearance").click();
  await page.locator(".theme-card").filter({ hasText: themeName }).click();
  await closeSettings(page);
  const styles = [];
  for (const kind of kinds) {
    const iframe = page.locator(`iframe[title="MCP App ui://milim.test/${kind}"]`);
    await iframe.scrollIntoViewIfNeeded();
    const body = page.frameLocator(`iframe[title="MCP App ui://milim.test/${kind}"]`).locator("body");
    await body.locator(`:scope[data-theme="${expectedTheme}"]`).waitFor();
    await page.waitForTimeout(180);
    const applied = await body.evaluate((element) => ({
      theme: element.dataset.theme,
      background: getComputedStyle(element).backgroundColor,
      color: getComputedStyle(element).color,
    }));
    if (applied.theme !== expectedTheme) {
      throw new Error(`${kind} did not apply ${expectedTheme} host theme: ${JSON.stringify(applied)}`);
    }
    styles.push({ kind, ...applied });
  }
  return styles;
}

function assertMcpAppsThemeStyles(lightStyles, darkStyles) {
  for (const light of lightStyles) {
    const dark = darkStyles.find(({ kind }) => kind === light.kind);
    if (!dark) throw new Error(`Missing dark-theme proof for ${light.kind}.`);
    const lightBackground = relativeLuminance(light.background);
    const darkBackground = relativeLuminance(dark.background);
    const lightContrast = contrastRatio(light.background, light.color);
    const darkContrast = contrastRatio(dark.background, dark.color);
    if (darkBackground >= lightBackground || lightContrast < 4.5 || darkContrast < 4.5) {
      throw new Error(`Invalid host theme styling for ${light.kind}: ${JSON.stringify({ light, dark, lightContrast, darkContrast })}`);
    }
  }
}

function contrastRatio(first, second) {
  const lighter = Math.max(relativeLuminance(first), relativeLuminance(second));
  const darker = Math.min(relativeLuminance(first), relativeLuminance(second));
  return (lighter + 0.05) / (darker + 0.05);
}

function relativeLuminance(cssColor) {
  const channels = cssColor.match(/[\d.]+/g)?.slice(0, 3).map(Number);
  if (!channels || channels.length !== 3) throw new Error(`Unsupported CSS color: ${cssColor}`);
  const [red, green, blue] = channels.map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

async function runNativePreviewOcclusionCheck(page, pid) {
  await page.locator(".app-notices").waitFor({ state: "hidden", timeout: 8_000 }).catch(() => {});
  const baseline = wryWebviews(pid);
  const baselineHandles = new Set(baseline.map((view) => view.handle));
  const visibleBaselineHandles = new Set(baseline.filter((view) => view.visible).map((view) => view.handle));
  if (!visibleBaselineHandles.size) {
    throw new Error(`Expected a visible main WRY_WEBVIEW before preview test, got ${describeWryWebviews(baseline)}`);
  }

  await page.getByTestId("open-artifact-browser").click();
  const apiBase = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("api_base_url"));
  const input = page.getByTestId("preview-browser-url");
  await input.fill(new URL("/health", apiBase).toString());
  await input.press("Enter");
  await page.getByTestId("preview-native-browser").waitFor();
  await page.locator(".preview-native-browser-status").waitFor({ state: "hidden", timeout: 10_000 });
  const preview = await waitForNewVisibleWryWebview(pid, baselineHandles);

  const previewHandle = page.getByTestId("preview-resize-handle");
  await previewHandle.focus();
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector(".sidebar")?.classList.contains("collapsed"));
  await delay(220);
  await page.keyboard.press("End");
  const nativeHostBefore = await page.getByTestId("preview-native-browser").boundingBox();
  const nativeViewBefore = wryWebviews(pid).find((view) => view.handle === preview.handle);
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await delay(100);
  const nativeHostAfter = await page.getByTestId("preview-native-browser").boundingBox();
  const nativeViewAfter = wryWebviews(pid).find((view) => view.handle === preview.handle);
  if (
    !nativeHostBefore ||
    !nativeHostAfter ||
    !nativeViewBefore ||
    !nativeViewAfter ||
    nativeHostAfter.width - nativeHostBefore.width < 30 ||
    nativeViewAfter.width <= nativeViewBefore.width
  ) {
    throw new Error(`Native preview child webview should follow overlay host bounds: ${JSON.stringify({ nativeHostBefore, nativeHostAfter, nativeViewBefore, nativeViewAfter })}.`);
  }
  await page.keyboard.press("Enter");
  await page.getByTitle("Expand sidebar").click();
  await delay(220);

  await page.getByTestId("open-settings").click();
  await page.getByTestId("settings-section-app").waitFor();
  const blockedViews = await waitForWryVisibility(pid, preview.handle, false);
  if (!blockedViews.some((view) => visibleBaselineHandles.has(view.handle) && view.visible)) {
    throw new Error(`Native preview blocker hid the main webview: ${describeWryWebviews(blockedViews)}`);
  }

  await page.getByTestId("close-settings").click();
  await waitForWryVisibility(pid, preview.handle, true);
  await page.getByLabel("Close inspector", { exact: true }).click();
  await page.getByTestId("open-artifact-browser").waitFor();
}

async function runBrowserProfileCheck(session) {
  const testServer = await startBrowserProfileServer();
  try {
    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-persistent-1",
      `${testServer.origin}/?phase=persistent-write&write=persistent`,
      "persistent",
    );
    assertBrowserReport(
      await testServer.waitForReport("persistent-write").then((report) => {
        assertBrowserCapabilities(report);
        return report;
      }),
      "persistent",
      "Persistent browser write",
    );
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-persistent-1");

    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-persistent-2",
      `${testServer.origin}/?phase=persistent-reopen`,
      "persistent",
    );
    assertBrowserReport(
      await testServer.waitForReport("persistent-reopen"),
      "persistent",
      "Persistent browser reopen",
    );
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-persistent-2");
    await runRestartCheck(session);
    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-persistent-3",
      `${testServer.origin}/?phase=persistent-restart`,
      "persistent",
    );
    assertBrowserReport(
      await testServer.waitForReport("persistent-restart"),
      "persistent",
      "Persistent browser restart",
    );

    await navigateE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-persistent-3",
      `${testServer.origin}/?phase=blocked-navigation&blocked=1`,
    );
    await testServer.waitForReport("blocked-navigation");
    await testServer.waitForReport("blocked-navigation-survived");
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-persistent-3");

    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-private-1",
      `${testServer.origin}/?phase=private-write&write=private`,
      "private",
    );
    assertBrowserReport(
      await testServer.waitForReport("private-write"),
      "private",
      "Private browser write",
    );
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-private-1");
    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-private-2",
      `${testServer.origin}/?phase=private-reopen`,
      "private",
    );
    assertBrowserReport(
      await testServer.waitForReport("private-reopen"),
      null,
      "Private browser reopen",
    );
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-private-2");

    await session.page.evaluate(async () => {
      await window.__TAURI_INTERNALS__.invoke("preview_webview_clear_data");
    });
    await createE2ePreviewWebview(
      session.page,
      "artifact-browser-e2e-persistent-cleared",
      `${testServer.origin}/?phase=persistent-cleared`,
      "persistent",
    );
    assertBrowserReport(
      await testServer.waitForReport("persistent-cleared"),
      null,
      "Cleared persistent browser",
    );
    await closeE2ePreviewWebview(session.page, "artifact-browser-e2e-persistent-cleared");
  } finally {
    await testServer.close();
  }
}

async function createE2ePreviewWebview(page, label, url, storageMode) {
  await page.evaluate(async ({ label, url, storageMode }) => {
    await window.__TAURI_INTERNALS__.invoke("preview_webview_create", {
      label,
      url,
      bounds: { x: 1180, y: 160, width: 480, height: 720 },
      storageMode,
      profileId: label,
    });
  }, { label, url, storageMode });
  await delay(500);
}

async function navigateE2ePreviewWebview(page, label, url) {
  await page.evaluate(async ({ label, url }) => {
    await window.__TAURI_INTERNALS__.invoke("preview_webview_navigate", { label, url });
  }, { label, url });
}

async function closeE2ePreviewWebview(page, label) {
  await page.evaluate(async (label) => {
    await window.__TAURI_INTERNALS__.invoke("preview_webview_close", { label });
  }, label);
  await delay(1_000);
}

function assertBrowserReport(report, expectedValue, label) {
  const cookieValue = /(?:^|;\s*)milim_e2e=([^;]+)/.exec(report.cookie)?.[1] ?? null;
  const storageValue = report.storage || null;
  if (cookieValue !== expectedValue || storageValue !== expectedValue) {
    throw new Error(
      `${label} expected cookie and local storage ${JSON.stringify(expectedValue)}, got ${JSON.stringify(report)}.`,
    );
  }
}

function assertBrowserCapabilities(report) {
  const expected = {
    cookieEnabled: true,
    dynamicCode: true,
    indexedDb: true,
    serviceWorker: true,
    webAssembly: true,
  };
  if (JSON.stringify(report.capabilities) !== JSON.stringify(expected)) {
    throw new Error(`Native browser lacks required web capabilities: ${JSON.stringify(report.capabilities)}.`);
  }
}

async function startBrowserProfileServer() {
  const reports = [];
  const server = http.createServer((request, response) => {
    const url = new URL(request.url || "/", "http://127.0.0.1");
    if (url.pathname === "/report") {
      reports.push({
        phase: url.searchParams.get("phase"),
        kind: url.searchParams.get("kind") || "main",
        cookie: url.searchParams.get("cookie") || "",
        storage: url.searchParams.get("storage") || "",
        capabilities: JSON.parse(url.searchParams.get("capabilities") || "null"),
      });
      response.writeHead(204);
      response.end();
      return;
    }

    const kind = url.pathname === "/popup" ? "popup" : "main";
    const phase = url.searchParams.get("phase") || "unknown";
    if (kind === "popup") {
      reports.push({
        phase,
        kind,
        cookie: request.headers.cookie || "",
        storage: "",
      });
    }
    const write = url.searchParams.get("write");
    const popup = url.searchParams.get("popup");
    const blocked = url.searchParams.get("blocked") === "1";
    response.writeHead(200, {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
    });
    response.end(`<!doctype html>
<meta charset="utf-8">
<title>Milim browser profile test</title>
<style>html,body,#popup{box-sizing:border-box;width:100%;height:100%;margin:0}#popup{font:24px sans-serif}</style>
<button id="popup" type="button">Open popup</button>
<script>
  const phase = ${JSON.stringify(phase)};
  const kind = ${JSON.stringify(kind)};
  const write = ${JSON.stringify(write)};
  const popup = ${JSON.stringify(popup)};
  const blocked = ${JSON.stringify(blocked)};
  if (write) {
    document.cookie = "milim_e2e=" + encodeURIComponent(write) + "; path=/; Max-Age=3600; SameSite=Lax";
    localStorage.setItem("milim_e2e", write);
  }
  const report = (reportPhase, reportKind) => fetch(
    "/report?phase=" + encodeURIComponent(reportPhase)
      + "&kind=" + encodeURIComponent(reportKind)
      + "&cookie=" + encodeURIComponent(document.cookie)
      + "&storage=" + encodeURIComponent(localStorage.getItem("milim_e2e") || "")
      + "&capabilities=" + encodeURIComponent(JSON.stringify({
        cookieEnabled: navigator.cookieEnabled,
        dynamicCode: Function("return true")(),
        indexedDb: "indexedDB" in window,
        serviceWorker: "serviceWorker" in navigator,
        webAssembly: WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0])),
      })),
    { cache: "no-store" },
  );
  report(phase, kind).then(() => {
    if (blocked) {
      setTimeout(() => {
        location.assign("http://example.com/blocked");
        setTimeout(() => report(phase + "-survived", kind), 500);
      }, 100);
    }
  });
  document.getElementById("popup").addEventListener("click", () => {
    if (popup) {
      report(popup + "-click", "main");
      window.open("/popup?phase=" + encodeURIComponent(popup), "_blank", "width=420,height=320");
    }
  });
</script>`);
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Browser profile test server did not bind.");
  return {
    origin: `http://127.0.0.1:${address.port}`,
    async waitForReport(phase, kind = "main", timeoutMs = 10_000) {
      const started = Date.now();
      while (Date.now() - started < timeoutMs) {
        const report = reports.find((candidate) => candidate.phase === phase && candidate.kind === kind);
        if (report) return report;
        await delay(50);
      }
      throw new Error(`Timed out waiting for browser report ${kind}:${phase}. reports=${JSON.stringify(reports)}`);
    },
    close: () => new Promise((resolve, reject) => {
      server.closeAllConnections?.();
      server.close((error) => error ? reject(error) : resolve());
    }),
  };
}

async function waitForNewVisibleWryWebview(pid, baselineHandles, timeoutMs = 10_000) {
  const started = Date.now();
  let views = [];
  while (Date.now() - started < timeoutMs) {
    views = wryWebviews(pid);
    const preview = views.find((view) => !baselineHandles.has(view.handle) && view.visible);
    if (preview) return preview;
    await delay(100);
  }
  throw new Error(`Timed out waiting for native preview HWND. views=${describeWryWebviews(views)}`);
}

async function waitForWryVisibility(pid, handle, visible, timeoutMs = 10_000) {
  const started = Date.now();
  let views = [];
  while (Date.now() - started < timeoutMs) {
    views = wryWebviews(pid);
    const target = views.find((view) => view.handle === handle);
    if (target?.visible === visible) return views;
    await delay(100);
  }
  throw new Error(`Timed out waiting for WRY_WEBVIEW ${handle} visible=${visible}. views=${describeWryWebviews(views)}`);
}

function wryWebviews(pid) {
  if (!Number.isInteger(pid) || pid <= 0) throw new Error(`Invalid Tauri PID: ${pid}`);
  const script = String.raw`
$source = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class MilimWryWebviewProbe {
  private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lparam);
  [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr lparam);
  [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr hwnd);
  [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, StringBuilder name, int maxCount);

  private struct Rect { public int Left; public int Top; public int Right; public int Bottom; }

  public static string[] Find(uint pid) {
    var results = new List<string>();
    EnumWindows((window, _) => {
      uint windowPid;
      GetWindowThreadProcessId(window, out windowPid);
      if (windowPid != pid || ClassName(window) != "Tauri Window") return true;
      EnumChildWindows(window, (child, __) => {
        if (GetParent(child) == window && ClassName(child) == "WRY_WEBVIEW") {
          Rect rect;
          GetWindowRect(child, out rect);
          results.Add(child.ToInt64() + "|" + (IsWindowVisible(child) ? "1" : "0") + "|" + rect.Left + "|" + rect.Top + "|" + (rect.Right - rect.Left) + "|" + (rect.Bottom - rect.Top));
        }
        return true;
      }, IntPtr.Zero);
      return true;
    }, IntPtr.Zero);
    return results.ToArray();
  }

  private static string ClassName(IntPtr hwnd) {
    var name = new StringBuilder(256);
    GetClassName(hwnd, name, name.Capacity);
    return name.ToString();
  }
}
'@
Add-Type -TypeDefinition $source
[MilimWryWebviewProbe]::Find(${pid})
`;
  const result = spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8", timeout: 5_000 });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Native webview probe failed (${result.status}): ${result.stderr || result.stdout}`);
  }
  return result.stdout.trim()
    ? result.stdout.trim().split(/\r?\n/).map((line) => {
      const [handle, visible, x, y, width, height] = line.split("|");
      return { handle, visible: visible === "1", x: Number(x), y: Number(y), width: Number(width), height: Number(height) };
    })
    : [];
}

function describeWryWebviews(views) {
  return views.length
    ? views.map((view) => `${view.handle}:${view.visible ? "visible" : "hidden"}:${view.width}x${view.height}@${view.x},${view.y}`).join(" | ")
    : "none";
}

async function runModelPickerSurfaceCheck(page) {
  const trigger = page.getByTestId("model-picker-trigger");
  await trigger.click();
  const picker = page.locator(".mp");
  await picker.waitFor();
  await picker.locator(".mp-search input").waitFor();
  await picker.locator(".mp-foot").waitFor();

  const rows = picker.locator(".mp-item");
  const rowCount = await rows.count();
  if (rowCount > 0) {
    const compactControls = await picker.evaluate((root) => ({
      capabilityRows: root.querySelectorAll(".mp-caps").length,
      effortButtons: root.querySelectorAll(".mp-effort-btn").length,
    }));
    const first = rows.first();
    await first.locator(".mp-star").waitFor();
    await first.locator(".mp-pick").waitFor();

    const audit = await first.evaluate((row) => {
      const pick = row.querySelector(".mp-pick");
      const pickChildren = Array.from(pick?.children ?? []).map((child) => child.className || child.tagName);
      return {
        height: row.getBoundingClientRect().height,
        heavyMetadataCount: row.querySelectorAll(".mp-meta, .mp-status, .mp-provider, .mp-runtime, .mp-lane").length,
        routeCount: row.querySelectorAll(".mp-route").length,
        starCount: row.querySelectorAll(".mp-star").length,
        capsCount: row.querySelectorAll(".mp-caps").length,
        effortCount: row.querySelectorAll(".mp-effort-btn").length,
        pickTitle: pick?.getAttribute("title") ?? "",
        pickAria: pick?.getAttribute("aria-label") ?? "",
        pickChildren,
      };
    });

    if (audit.height > 38) {
      throw new Error(`Expected compact model picker row height, got ${audit.height}px.`);
    }
    if (audit.heavyMetadataCount !== 0) {
      throw new Error("Model picker row should not render visible provider/runtime/status metadata elements.");
    }
    if (audit.routeCount !== 1) {
      throw new Error("Model picker row should render one visible route label.");
    }
    if (audit.starCount !== 1) {
      throw new Error("Model picker row should include one favorite control.");
    }
    if (!audit.pickTitle || !audit.pickAria) {
      throw new Error("Model picker route/setup metadata should remain available through title and aria labels.");
    }
    if (!audit.pickChildren.includes("mp-title")) {
      throw new Error(`Model picker row should keep a one-line title structure, got children: ${audit.pickChildren.join(", ")}.`);
    }
    if (compactControls.capabilityRows === 0 && compactControls.effortButtons === 0) {
      throw new Error("Model picker should expose compact capability or reasoning controls when models exist.");
    }

    const collapsibleGroup = picker.locator(".mp-group:has(.mp-group-toggle)").first();
    if (await collapsibleGroup.count()) {
      const toggle = collapsibleGroup.locator(".mp-group-toggle");
      const groupLabel = (await toggle.locator(".mp-group-label > span").last().textContent())?.trim();
      const modelName = (await collapsibleGroup.locator(".mp-name").first().textContent())?.trim();
      if (groupLabel && modelName) {
        await toggle.click();
        if (await toggle.getAttribute("aria-expanded") !== "false" || await collapsibleGroup.locator(".mp-item").count() !== 0) {
          throw new Error(`Expected ${groupLabel} to collapse its model rows.`);
        }

        await trigger.click();
        await picker.waitFor({ state: "hidden" }).catch(() => {});
        await trigger.click();
        await picker.waitFor();
        const reopenedToggle = picker.getByRole("button", { name: `Expand ${groupLabel} models` });
        await reopenedToggle.waitFor();

        const search = picker.locator(".mp-search input");
        await search.fill(modelName);
        await picker.locator(".mp-item").filter({ hasText: modelName }).waitFor();
        await search.fill("");
        await picker.locator(".mp-item").filter({ hasText: modelName }).waitFor({ state: "hidden" });
        await picker.getByRole("button", { name: `Expand ${groupLabel} models` }).click();
      }
    }
  } else {
    await picker.locator(".mp-empty").waitFor();
  }

  await trigger.click();
  await picker.waitFor({ state: "hidden" }).catch(() => {});
}

async function runArtifactCheck(page) {
  const workspace = mkdtempSync(join(tmpdir(), "milim-artifact-workspace-"));
  const prompt = [
    "Return this generated file:",
    "",
    "```ts file=src/e2e-artifact.ts",
    "export const e2eArtifact = true;",
    "```",
  ].join("\n");
  try {
    const savedPath = join(workspace, "src", "e2e-artifact.ts");
    await page.getByTestId("composer-input").fill(`/folder ${workspace}`);
    await page.getByTestId("composer-send").click();
    await page.getByTestId("composer-input").fill(prompt);
    await page.getByTestId("composer-send").click();
    const card = page.getByTestId("artifact-card").last();
    await card.waitFor();
    await card.getByText("src/e2e-artifact.ts").waitFor();
    await card.getByText("export const e2eArtifact = true;").waitFor();
    await runArtifactContextMenuCheck(page, card);
    await card.getByTestId("artifact-copy").waitFor();
    await card.getByTestId("artifact-download").waitFor();
    await card.getByTestId("artifact-review-workspace").click();
    await card.getByTestId("artifact-preview-diff").waitFor();
    await card.getByTestId("artifact-reviewed-time").waitFor();
    await card.getByText("New file preview").waitFor();
    await card.getByText("+export const e2eArtifact = true;").waitFor();
    await card.locator(".artifact-diff-line.added", { hasText: "+export const e2eArtifact = true;" }).waitFor();
    await card.getByTestId("artifact-save-workspace").click();
    await waitForFileText(savedPath, "export const e2eArtifact = true;");
    await card.getByTestId("artifact-saved-path").waitFor();
    await card.getByTestId("artifact-saved-session").getByText("Saved in this app session").waitFor();
    await card.getByTestId("artifact-saved-time").waitFor();
    await card.getByTestId("artifact-open-file").waitFor();
    await card.getByTestId("artifact-open-folder").waitFor();
    writeFileSync(savedPath, "export const e2eArtifact = false;\n", "utf8");
    await card.getByTestId("artifact-save-workspace").click();
    await card.getByTestId("artifact-conflict").waitFor();
    await card.getByTestId("artifact-preview-changes").click();
    await card.getByTestId("artifact-preview-diff").waitFor();
    await card.getByText("-export const e2eArtifact = false;").waitFor();
    await card.getByText("+export const e2eArtifact = true;").waitFor();
    await card.locator(".artifact-diff-line.removed", { hasText: "-export const e2eArtifact = false;" }).waitFor();
    await card.locator(".artifact-diff-line.added", { hasText: "+export const e2eArtifact = true;" }).waitFor();
    await card.locator(".artifact-diff-line.hunk").first().waitFor();
    await card.getByTestId("artifact-preview-apply").click();
    await waitForFileText(savedPath, "export const e2eArtifact = true;");
    await card.getByTestId("artifact-conflict").waitFor({ state: "hidden" });
    await waitForPersistedUserStateText(page, "milim.sessions", savedPath);
    await page.reload();
    await page.getByTestId("chat-shell").waitFor();
    const persistedCard = await waitForArtifactCardWithText(page, savedPath);
    await persistedCard.getByTestId("artifact-saved-path").waitFor();
    await persistedCard.getByText(savedPath).waitFor();
    await persistedCard.getByTestId("artifact-saved-session").getByText("Saved in a previous app session").waitFor();
    await persistedCard.getByTestId("artifact-saved-time").waitFor();
    await persistedCard.getByTestId("artifact-open-file").waitFor();
    await persistedCard.getByTestId("artifact-open-folder").waitFor();
    rmSync(savedPath, { force: true });
    await page.reload();
    await page.getByTestId("chat-shell").waitFor();
    const missingCard = await waitForArtifactCardWithText(page, savedPath);
    await missingCard.getByTestId("artifact-file-missing").waitFor();
    await assertHidden(missingCard.getByTestId("artifact-open-file"), "open file button for missing artifact");
    await assertHidden(missingCard.getByTestId("artifact-open-folder"), "open folder button for missing artifact");
    await missingCard.getByTestId("artifact-save-workspace").click();
    await waitForFileText(savedPath, "export const e2eArtifact = true;");
    await missingCard.getByTestId("artifact-file-missing").waitFor({ state: "hidden" });
    await missingCard.getByTestId("artifact-open-file").waitFor();
    await missingCard.getByTestId("artifact-open-folder").waitFor();
    await runPerArtifactUnchangedCheck(missingCard);
    await runBatchArtifactCheck(page, workspace);
    await runBatchArtifactSelectionCheck(page, workspace);
    await runBatchArtifactUnchangedCheck(page, workspace);
    await runBatchArtifactFailureCheck(page, workspace);
    await runArtifactTargetPathCheck(page, workspace);
    await runLargeArtifactDiffCheck(page, workspace);
    await runArtifactPreviewPanelCheck(page);
  } finally {
    rmWithRetry(workspace);
  }
}

async function runArtifactPreviewPanelCheck(page) {
  const prompt = [
    "Return exactly this two-file artifact:",
    "",
    "```html file=index.html",
    '<!doctype html><html><body><div id="app"></div><script type="module" src="./src/main.js"></script></body></html>',
    "```",
    "```js file=src/main.js",
    'console.log("artifact log ready");',
    'throw new Error("artifact boom");',
    "```",
  ].join("\n");
  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const indexCard = page.getByTestId("artifact-card").filter({ hasText: "index.html" }).last();
  await indexCard.waitFor({ timeout: 60_000 });
  await indexCard.getByTestId("artifact-open-preview").click();
  const preview = page.getByTestId("chat-preview-split");
  await preview.waitFor();
  await assertHidden(preview.getByTestId("preview-code-file-list"), "code file list in preview mode");
  await preview.getByTestId("preview-log-drawer").waitFor();
  await preview.getByTestId("preview-log-list").getByText("artifact boom").waitFor({ timeout: 20_000 });
  await preview.getByTestId("preview-tab-code").click();
  await preview.getByTestId("preview-code-file-list").waitFor();
  await preview.getByTestId("preview-code-line-number").filter({ hasText: "1" }).first().waitFor();
  const before = await preview.getByTestId("preview-code-file-list").boundingBox();
  const handle = await preview.getByTestId("preview-code-resize-handle").boundingBox();
  if (!before || !handle) throw new Error("Preview code splitter should have measurable bounds.");
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2);
  await page.mouse.down();
  await page.mouse.move(handle.x + handle.width / 2 + 40, handle.y + handle.height / 2);
  await page.mouse.up();
  const after = await preview.getByTestId("preview-code-file-list").boundingBox();
  if (!after || after.width <= before.width) {
    throw new Error(`Preview file list should resize wider, before=${before?.width}, after=${after?.width}`);
  }
  await preview.getByTestId("preview-tab-preview").click();
  await preview.getByTestId("preview-quick-fix").click();
  await page.getByTestId("user-message").last().getByText("Please fix the current artifact preview errors.").waitFor({ timeout: 10_000 });
  await page.getByRole("button", { name: "Stop generating" }).click().catch(() => {});
}

async function runPerArtifactUnchangedCheck(card) {
  await card.getByTestId("artifact-review-workspace").click();
  await card.getByTestId("artifact-unchanged").waitFor();
  await card.getByText("No changes").waitFor();
  await assertHidden(card.getByTestId("artifact-preview-apply"), "apply button for unchanged artifact");
}

async function runBatchArtifactCheck(page, workspace) {
  const existingPath = join(workspace, "src", "batch-one.ts");
  const newPath = join(workspace, "src", "batch-two.ts");
  writeFileSync(existingPath, "export const batchOne = false;\n", "utf8");
  rmSync(newPath, { force: true });

  const prompt = [
    "Return this two-file change set:",
    "",
    "```ts file=src/batch-one.ts",
    "export const batchOne = true;",
    "```",
    "",
    "```ts file=src/batch-two.ts",
    "export const batchTwo = true;",
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const list = page.getByTestId("artifact-list").last();
  await list.getByTestId("artifact-batch-review").waitFor();
  await list.getByTestId("artifact-card").filter({ hasText: "src/batch-one.ts" }).waitFor();
  await list.getByTestId("artifact-card").filter({ hasText: "src/batch-two.ts" }).waitFor();
  await list.getByTestId("artifact-batch-review").click();
  await list.getByText("2 artifacts reviewed").waitFor();
  await list.getByText("-export const batchOne = false;").waitFor();
  await list.getByText("+export const batchOne = true;").waitFor();
  await list.getByText("+export const batchTwo = true;").waitFor();
  await list.getByTestId("artifact-batch-apply").click();
  await list.getByText("2 artifacts applied.").waitFor();
  await waitForFileText(existingPath, "export const batchOne = true;");
  await waitForFileText(newPath, "export const batchTwo = true;");
  const firstResultRow = list.getByTestId("artifact-batch-result").filter({ hasText: "batch-one.ts" });
  await firstResultRow.getByTestId("artifact-batch-open-file").waitFor();
  await firstResultRow.getByTestId("artifact-batch-open-folder").waitFor();
  const secondResultRow = list.getByTestId("artifact-batch-result").filter({ hasText: "batch-two.ts" });
  await secondResultRow.getByTestId("artifact-batch-open-file").waitFor();
  await secondResultRow.getByTestId("artifact-batch-open-folder").waitFor();
}

async function runBatchArtifactSelectionCheck(page, workspace) {
  const selectedPath = join(workspace, "src", "batch-selected.ts");
  const skippedPath = join(workspace, "src", "batch-skipped.ts");
  rmSync(selectedPath, { force: true });
  rmSync(skippedPath, { force: true });

  const prompt = [
    "Return this two-file change set:",
    "",
    "```ts file=src/batch-selected.ts",
    "export const batchSelected = true;",
    "```",
    "",
    "```ts file=src/batch-skipped.ts",
    "export const batchSkipped = true;",
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const list = page.getByTestId("artifact-list").last();
  await list.getByTestId("artifact-batch-apply").waitFor();
  const selectedCard = list.getByTestId("artifact-card").filter({ hasText: "src/batch-selected.ts" });
  const skippedCard = list.getByTestId("artifact-card").filter({ hasText: "src/batch-skipped.ts" });
  await selectedCard.waitFor();
  await skippedCard.waitFor();
  await skippedCard.getByTestId("artifact-select-toggle").click();
  await assertAttribute(skippedCard.getByTestId("artifact-select-toggle"), "aria-checked", "false");
  await assertAttribute(selectedCard.getByTestId("artifact-select-toggle"), "aria-checked", "true");
  await list.getByTestId("artifact-batch-selection-count").getByText("1 of 2 selected").waitFor();
  await list.getByTestId("artifact-batch-review").click();
  await list.getByText("1 artifact reviewed; 1 changed.").waitFor();
  await selectedCard.getByTestId("artifact-preview-diff").waitFor();
  await assertHidden(skippedCard.getByTestId("artifact-preview-diff"), "deselected artifact preview diff");
  await list.getByTestId("artifact-batch-apply").click();
  await list.getByText("1 artifact applied.").waitFor();
  await waitForFileText(selectedPath, "export const batchSelected = true;");
  if (existsSync(skippedPath)) {
    throw new Error("Deselected batch artifact should not be written.");
  }
  await list.getByTestId("artifact-batch-result").filter({ hasText: "batch-selected.ts" }).getByText("Applied").waitFor();
  if (await list.getByTestId("artifact-batch-result").filter({ hasText: "batch-skipped.ts" }).isVisible().catch(() => false)) {
    throw new Error("Deselected batch artifact should not appear in batch results.");
  }
}

async function runBatchArtifactUnchangedCheck(page, workspace) {
  const changedPath = join(workspace, "src", "batch-change-needed.ts");
  const unchangedPath = join(workspace, "src", "batch-unchanged.ts");
  writeFileSync(changedPath, "export const batchChangeNeeded = false;\n", "utf8");
  writeFileSync(unchangedPath, "export const batchUnchanged = true;", "utf8");
  const unchangedMtimeBefore = statSync(unchangedPath).mtimeMs;
  await delay(1200);

  const prompt = [
    "Return this two-file change set:",
    "",
    "```ts file=src/batch-change-needed.ts",
    "export const batchChangeNeeded = true;",
    "```",
    "",
    "```ts file=src/batch-unchanged.ts",
    "export const batchUnchanged = true;",
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const list = page.getByTestId("artifact-list").last();
  await list.getByTestId("artifact-batch-review").waitFor();
  await list.getByTestId("artifact-card").filter({ hasText: "src/batch-change-needed.ts" }).waitFor();
  await list.getByTestId("artifact-card").filter({ hasText: "src/batch-unchanged.ts" }).waitFor();
  await list.getByTestId("artifact-batch-review").click();
  await list.getByText("2 artifacts reviewed; 1 changed, 1 unchanged.").waitFor();
  await list.getByTestId("artifact-batch-apply").click();
  await list.getByText("1 applied; 1 unchanged.").waitFor();
  await waitForFileText(changedPath, "export const batchChangeNeeded = true;");
  const unchangedMtimeAfter = statSync(unchangedPath).mtimeMs;
  if (unchangedMtimeAfter !== unchangedMtimeBefore) {
    throw new Error("Unchanged batch artifact should not be rewritten.");
  }
  await list.getByTestId("artifact-batch-result").filter({ hasText: "batch-change-needed.ts" }).getByText("Applied").waitFor();
  const unchangedRow = list.getByTestId("artifact-batch-result").filter({ hasText: "batch-unchanged.ts" });
  const unchangedRowText = await unchangedRow.innerText();
  if (!unchangedRowText.includes("Unchanged")) {
    throw new Error(`Expected unchanged batch result row, got: ${unchangedRowText}`);
  }
  await unchangedRow.getByTestId("artifact-batch-open-file").waitFor();
  await unchangedRow.getByTestId("artifact-batch-open-folder").waitFor();
}

async function runBatchArtifactFailureCheck(page, workspace) {
  const okPath = join(workspace, "src", "batch-ok.ts");
  const blockedOriginalPath = join(workspace, "src", "batch-blocked.ts");
  rmSync(okPath, { force: true });
  rmSync(blockedOriginalPath, { force: true });

  const prompt = [
    "Return this two-file change set:",
    "",
    "```ts file=src/batch-ok.ts",
    "export const batchOk = true;",
    "```",
    "",
    "```ts file=src/batch-blocked.ts",
    "export const batchBlocked = true;",
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const list = page.getByTestId("artifact-list").last();
  await list.getByTestId("artifact-batch-apply").waitFor();
  await list.getByTestId("artifact-card").filter({ hasText: "src/batch-ok.ts" }).waitFor();
  const blockedCard = list.getByTestId("artifact-card").filter({ hasText: "src/batch-blocked.ts" });
  await blockedCard.getByTestId("artifact-target-path").fill("../blocked-batch.ts");
  await list.getByTestId("artifact-batch-apply").click();
  await waitForFileText(okPath, "export const batchOk = true;");
  if (existsSync(blockedOriginalPath)) {
    throw new Error("Failed batch artifact should not write its original generated path.");
  }
  await list.getByText("1 applied; 1 failed.").waitFor();
  await list.getByTestId("artifact-batch-results").waitFor();
  await list.getByTestId("artifact-batch-result").filter({ hasText: "batch-ok.ts" }).getByText("Applied").waitFor();
  const failedRow = list.getByTestId("artifact-batch-result").filter({ hasText: "blocked-batch.ts" });
  await failedRow.getByText("Failed").waitFor();
  await assertHidden(failedRow.getByTestId("artifact-batch-open-file"), "open file action for failed batch result");
  await assertHidden(failedRow.getByTestId("artifact-batch-open-folder"), "open folder action for failed batch result");
  await blockedCard.getByTestId("artifact-error").waitFor();
}

async function runArtifactTargetPathCheck(page, workspace) {
  const originalPath = join(workspace, "src", "target-original.ts");
  const renamedPath = join(workspace, "src", "target-renamed.ts");
  rmSync(originalPath, { force: true });
  rmSync(renamedPath, { force: true });

  const prompt = [
    "Return this generated file:",
    "",
    "```ts file=src/target-original.ts",
    "export const targetPath = true;",
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const card = page.getByTestId("artifact-card").last();
  await card.getByText("src/target-original.ts").waitFor();
  await card.getByTestId("artifact-target-path").fill("src/target-renamed.ts");
  await card.getByTestId("artifact-save-workspace").click();
  await waitForFileText(renamedPath, "export const targetPath = true;");
  if (existsSync(originalPath)) {
    throw new Error("Target override should not write the original artifact path.");
  }
  await card.getByTestId("artifact-saved-path").waitFor();
  await card.getByText(renamedPath).waitFor();
  await card.getByTestId("artifact-target-path").fill("../blocked.ts");
  await card.getByTestId("artifact-save-workspace").click();
  await card.getByTestId("artifact-error").waitFor();
  await card.getByText("..").waitFor();
  await card.getByText("artifact paths").waitFor();
}

async function runLargeArtifactDiffCheck(page, workspace) {
  const largePath = join(workspace, "src", "large-diff.ts");
  const oldLines = numberedLargeLines(false);
  const newLines = numberedLargeLines(true);
  writeFileSync(largePath, `${oldLines.join("\n")}\n`, "utf8");

  const prompt = [
    "Return this generated file:",
    "",
    "```ts file=src/large-diff.ts",
    ...newLines,
    "```",
  ].join("\n");

  await page.getByTestId("composer-input").fill(prompt);
  await page.getByTestId("composer-send").click();
  const card = page.getByTestId("artifact-card").last();
  await card.getByText("src/large-diff.ts").waitFor();
  await card.getByTestId("artifact-review-workspace").click();
  await card.getByTestId("artifact-preview-diff").waitFor();
  await card.getByTestId("artifact-diff-summary").waitFor();
  const summary = await card.getByTestId("artifact-diff-summary").innerText();
  if (!/\d+ added, \d+ removed/.test(summary)) {
    throw new Error(`Expected large diff summary to include added/removed counts, got: ${summary}`);
  }
  await card.getByTestId("artifact-diff-toggle").waitFor();
  const diffLines = card.locator(".artifact-diff-line");
  const collapsedLineCount = await diffLines.count();
  if (collapsedLineCount > 85) {
    throw new Error(`Expected collapsed large diff to render a bounded line count, got ${collapsedLineCount}.`);
  }
  await card.getByTestId("artifact-diff-toggle").click();
  await waitForLocatorCountGreaterThan(diffLines, collapsedLineCount);
  await card.getByTestId("artifact-diff-toggle").click();
  await waitForLocatorCountAtMost(diffLines, collapsedLineCount);
}

function numberedLargeLines(value) {
  return Array.from({ length: 120 }, (_, index) => {
    const n = String(index + 1).padStart(3, "0");
    return `export const largeLine${n} = ${value};`;
  });
}

async function runSlashAndAttachmentCheck(page) {
  await page.getByTestId("composer-input").fill("/privacy redact");
  await page.getByTestId("composer-send").click();
  await page.getByTestId("context-menu-trigger").waitFor();
  await page.getByTestId("context-menu-trigger").click();
  const privacyRedact = page.locator('[role="radiogroup"][aria-label="Privacy"] [role="radio"]', { hasText: "Redact" });
  await assertAttribute(privacyRedact, "aria-checked", "true");
  await page.getByTestId("composer-input").click();

  await page.getByTestId("composer-input").fill("/privacy");
  await page.getByTestId("composer-send").click();
  await page.getByTestId("context-menu-trigger").click();
  await assertAttribute(privacyRedact, "aria-checked", "true");
  await page.getByTestId("composer-input").click();

  await page.getByTestId("composer-input").fill("/approval open");
  await page.getByTestId("composer-send").click();
  const approvalTrigger = page.getByTestId("context-menu-trigger");
  await approvalTrigger.getByText("Open", { exact: true }).waitFor();
  await approvalTrigger.click();
  const approvalGroup = page.locator('[role="radiogroup"][aria-label="Tool approval"]');
  await assertAttribute(approvalGroup, "aria-describedby", "tool-approval-description");
  const approvalDescription = page.locator("#tool-approval-description");
  await assertTextContains(approvalDescription, "Run without approval in trusted workspaces.");
  const approvalOpen = approvalGroup.getByRole("radio", { name: "Open" });
  await assertAttribute(approvalOpen, "aria-checked", "true");
  const approvalReview = approvalGroup.getByRole("radio", { name: "Review" });
  await approvalReview.click();
  await approvalTrigger.getByText("Review", { exact: true }).waitFor();
  await assertTextContains(approvalDescription, "Ask before each tool action.");
  const approvalGuarded = approvalGroup.getByRole("radio", { name: "Guarded" });
  await approvalGuarded.click();
  await approvalTrigger.getByText("Guarded", { exact: true }).waitFor();
  await assertTextContains(approvalDescription, "Run safe tools; ask before consequential actions.");
  await approvalOpen.click();
  await approvalTrigger.getByText("Open", { exact: true }).waitFor();
  await page.getByTestId("composer-input").click();

  await page.getByTestId("composer-input").fill("/approval nope");
  await page.getByTestId("composer-send").click();
  await page.getByTestId("context-menu-trigger").click();
  await assertAttribute(approvalOpen, "aria-checked", "true");
  await page.getByTestId("composer-input").click();

  const attachmentPath = join(tmpdir(), `milim-e2e-attachment-${Date.now()}.txt`);
  writeFileSync(attachmentPath, "attached context from webview e2e\n", "utf8");
  try {
    await page.getByTestId("composer-file-input").setInputFiles(attachmentPath);
    const tray = page.getByTestId("attachment-tray");
    await tray.waitFor();
    await tray.getByText("milim-e2e-attachment").waitFor();
    await pasteFileIntoComposer(page, "pasted-screenshot.png", "image/png", "fake-png-bytes");
    await tray.getByText("pasted-screenshot.png").waitFor();
    await tray.locator(".attachment-thumb").waitFor();
    if (!(await hasChatModel(page))) {
      console.log("attachmentSendCheck=skipped:no chat model configured");
      await clearAttachments(page);
      return;
    }
    await page.getByTestId("composer-input").fill("read the attached note");
    await page.getByTestId("composer-send").click();
    const sentMessage = page.getByTestId("user-message").last();
    await waitForLocatorCountGreaterThan(sentMessage.locator('[data-testid^="message-attachment-"]'), 1);
    await sentMessage.getByText("milim-e2e-attachment").waitFor();
    await sentMessage.getByText("pasted-screenshot.png").waitFor();
    await sentMessage.getByText("read the attached note").waitFor();
  } finally {
    rmSync(attachmentPath, { force: true });
  }
}

async function runMemoryLibraryCheck(page) {
  await page.getByTestId("context-menu-trigger").click();
  const memoryToggle = page.getByTestId("memory-toggle");
  const memoryBefore = await memoryToggle.getAttribute("aria-pressed");
  await page.getByRole("button", { name: "Manage memory" }).click();
  await page.getByRole("heading", { name: "Memory" }).waitFor();
  await page.getByRole("tab", { name: "Personal" }).waitFor();
  await page.getByRole("tab", { name: "Project" }).waitFor();
  await page.getByLabel("Search memories").waitFor();
  await page.getByText("Show archived", { exact: true }).waitFor();
  await page.getByLabel("Close memory manager").click();
  await page.getByTestId("context-menu-trigger").click();
  await assertAttribute(memoryToggle, "aria-pressed", memoryBefore);
  await page.getByTestId("context-menu-trigger").click();
}

async function runContextDrawerCheck(page) {
  await page.getByLabel("Open context").click();
  const context = page.getByTestId("quick-summary-panel");
  await context.waitFor();
  await context.getByText("Model", { exact: true }).waitFor();
  await page.getByLabel("Close context").click();
}

async function switchModelWhileAgentActive(page, agentName) {
  await page.getByTestId("model-picker-trigger").click();
  const candidates = page.locator(".mp-item:not(.active) .mp-pick");
  for (let index = 0; index < await candidates.count(); index += 1) {
    const candidate = candidates.nth(index);
    const label = await candidate.getAttribute("aria-label");
    if (!label || label.includes("Media")) continue;
    await candidate.click();
    await assertAttribute(page.getByTestId("agent-switcher"), "aria-label", `Persona, current ${agentName}`);
    return;
  }
  await page.keyboard.press("Escape");
  console.log("agentModelSwitchCheck=skipped:no alternate chat model");
}

async function runContextMenuChromeCheck(page) {
  await page.keyboard.press("Escape").catch(() => {});
  await page.getByTestId("composer-input").click({ button: "right" });
  await assertHidden(page.getByTestId("app-context-menu"), "app context menu on composer textarea");

  const sessionRow = page.locator(".session-item").first();
  await sessionRow.waitFor();
  await sessionRow.click({ button: "right" });
  const menu = page.getByTestId("app-context-menu");
  await menu.waitFor();
  await menu.getByText(/Open chat|Current chat/).waitFor();
  await menu.getByText("Branch chat").waitFor();
  await menu.getByText("Export chat").waitFor();
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
}

async function runMessageContextMenuCheck(page) {
  const message = page.getByTestId("user-message").last();
  await message.waitFor();
  await message.click({ button: "right" });
  const menu = page.getByTestId("app-context-menu");
  await menu.waitFor();
  await menu.getByText("Copy").waitFor();
  await menu.getByText("Edit and resend").waitFor();
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
}

async function runMessagePopoverLayerCheck(page) {
  const trigger = page.getByTestId("baton-menu-trigger").last();
  await trigger.waitFor({ timeout: 60_000 });
  await trigger.click();
  const popover = page.getByRole("menu", { name: "Model handoff actions" });
  await popover.waitFor();
  const layers = await page.evaluate(() => {
    const popoverElement = document.querySelector(".baton-menu-popover");
    const sidebar = document.querySelector(".sidebar");
    if (!(popoverElement instanceof HTMLElement) || !(sidebar instanceof HTMLElement)) return null;
    return {
      parentIsBody: popoverElement.parentElement === document.body,
      popover: Number.parseInt(getComputedStyle(popoverElement).zIndex, 10),
      sidebar: Number.parseInt(getComputedStyle(sidebar).zIndex, 10),
    };
  });
  if (!layers?.parentIsBody) throw new Error("Expected message popover to render directly under document.body");
  if (!Number.isFinite(layers.popover) || !Number.isFinite(layers.sidebar) || layers.popover <= layers.sidebar) {
    throw new Error(`Expected message popover above sidebar, got popover=${layers.popover} sidebar=${layers.sidebar}`);
  }
  await page.keyboard.press("Escape");
  await popover.waitFor({ state: "hidden" });
}

async function runArtifactContextMenuCheck(page, card) {
  await card.click({ button: "right" });
  const menu = page.getByTestId("app-context-menu");
  await menu.waitFor();
  await menu.getByText("Copy artifact").waitFor();
  await menu.getByText("Download artifact").waitFor();
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
}

async function pasteFileIntoComposer(page, name, type, content) {
  await page.getByTestId("composer-input").evaluate(
    (el, payload) => {
      const file = new File([payload.content], payload.name, { type: payload.type });
      const data = new DataTransfer();
      data.items.add(file);
      const event = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: data });
      el.dispatchEvent(event);
    },
    { name, type, content },
  );
}

async function hasChatModel(page) {
  const label = await page.getByTestId("model-picker-trigger").locator(".chip-label").innerText().catch(() => "");
  const normalized = label.trim().toLowerCase();
  return Boolean(normalized && normalized !== "choose model" && normalized !== "no model" && normalized !== "model");
}

async function clearAttachments(page) {
  const buttons = page.locator(".attachment-remove");
  while ((await buttons.count()) > 0) {
    await buttons.first().click();
  }
  await page.getByTestId("composer-input").fill("");
}

async function runProviderSetup(page) {
  const errors = collectErrors(page);
  await page.getByTestId("chat-shell").waitFor();

  await openProviders(page);
  await page.getByText("Account runtimes", { exact: true }).waitFor();
  await page.getByText("Add providers", { exact: true }).waitFor();
  await page.getByTestId("detect-local-providers").click();
  await page.getByText("Ollama (local)").waitFor({ timeout: 20_000 });
  await page.getByText("LM Studio (local)").waitFor({ timeout: 20_000 });
  await page.getByTestId("new-provider").click();
  await page.getByTestId("provider-name-input").fill("E2E Local Provider");
  await page.getByTestId("provider-kind-select").click();
  await page.locator(".providers-sheet .ui-select-menu .ui-select-item", { hasText: "OpenAI-compatible" }).click();
  await page.getByTestId("provider-base-url-input").fill("http://127.0.0.1:9/v1");
  await page.getByTestId("provider-api-key-input").fill("e2e-key");
  await assertFieldContains(page.getByTestId("provider-name-input"), "E2E Local Provider");
  await assertFieldContains(page.getByTestId("provider-base-url-input"), "127.0.0.1:9");

  await page.getByTestId("new-provider").click();
  await page.getByTestId("provider-preset-select").click();
  await page.locator(".providers-sheet .ui-select-menu .ui-select-item", { hasText: "fal" }).click();
  await page.getByTestId("provider-api-key-input").fill("fal-e2e-key");
  await page.getByTestId("save-provider").click();
  await page.getByText("Media provider saved").waitFor();
  await closeProviders(page);

  await page.getByTestId("model-picker-trigger").click();
  const falModel = page.locator(".mp-item", { hasText: "fal-ai/flux/schnell" }).locator(".mp-pick");
  if (await falModel.waitFor({ timeout: 2_000 }).then(() => true).catch(() => false)) {
    await falModel.click();
    await page.getByTestId("inline-media-generator").waitFor();
    await assertAttributeContains(page.getByTestId("inline-media-generator"), "title", "fal-ai/flux/schnell");
    await page.getByTestId("composer-input").fill("studio product photo");
  } else {
    console.log("mediaPickerSelection=skipped:no fal media model in picker");
    await page.getByTestId("model-picker-trigger").click();
    await page.locator(".mp").waitFor({ state: "hidden" }).catch(() => {});
  }

  await openSettings(page);
  await runAppShortcutSettingsCheck(page);

  return errors;
}

async function resetFrontendStorage(page) {
  await page.getByTestId("chat-shell").waitFor();
  await page.evaluate(() => {
    for (const key of [
      "milim.sessions",
      "milim.settings",
      "milim.ui",
      "milim.window.alwaysOnTop",
    ]) {
      window.localStorage.removeItem(key);
    }
  });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
}

async function runWorkersInspectorCheck(page, milimHome) {
  await seedWorkerFixture(page, milimHome, "proposed");
  const inspector = page.getByTestId("workers-inspector");
  await inspector.waitFor();
  await assertHidden(page.getByRole("tab", { name: "Workers" }), "Workers inspector tab");
  await page.getByTestId("workers-plan").waitFor();
  await page.screenshot({ path: screenshots.workersPlan, fullPage: false });

  await seedWorkerFixture(page, milimHome, "running");
  await page.setViewportSize({ width: 760, height: 720 });
  await inspector.waitFor();
  await inspector.locator(".workers-body").waitFor();
  await assertTextContains(inspector.locator(".workers-status"), "Running");
  const fits = await inspector.evaluate((element) => element.scrollWidth <= element.clientWidth + 1);
  if (!fits) throw new Error("Workers Context panel overflows at narrow width.");
  const workersSectionToggle = page.getByTestId("workers-section-toggle");
  const workersSettingsToggle = page.getByTestId("workers-settings-toggle");
  const workersChevronToggle = page.getByTestId("workers-chevron-toggle");
  await workersChevronToggle.click();
  await assertAttribute(workersSectionToggle, "aria-expanded", "false");
  await inspector.locator(".workers-body").waitFor({ state: "hidden" });
  await workersSettingsToggle.click();
  await assertAttribute(workersSectionToggle, "aria-expanded", "true");
  await assertAttribute(workersSettingsToggle, "aria-expanded", "true");
  await inspector.locator(".workers-controls").waitFor();
  await workersSettingsToggle.click();
  const sourceToggle = page.locator(".quick-summary-more");
  await assertTextContains(sourceToggle, "2 more");
  await sourceToggle.click();
  await assertTextContains(sourceToggle, "Show less");
  const sourceSeven = page.getByTestId("quick-summary-panel").getByText("source-7.txt", { exact: true });
  await sourceSeven.waitFor();
  await sourceToggle.click();
  await assertHidden(sourceSeven, "collapsed source");
  const sourcesSectionToggle = page.getByTestId("quick-summary-section-sources");
  const sourcesReveal = page.locator("#quick-summary-sources-content");
  const transitionDuration = await sourcesReveal.evaluate((element) =>
    getComputedStyle(element).transitionDuration,
  );
  if (!transitionDuration.split(",").some((duration) => Number.parseFloat(duration) > 0)) {
    throw new Error("Context section reveal should animate.");
  }
  await sourcesSectionToggle.click();
  await assertAttribute(sourcesSectionToggle, "aria-expanded", "false");
  await sourceToggle.waitFor({ state: "hidden" });
  await waitForPersistedUserStateText(page, "milim.sessions", "sources");
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await assertAttribute(page.getByTestId("quick-summary-section-sources"), "aria-expanded", "false");
  await assertHidden(page.locator(".quick-summary-more"), "persisted collapsed Sources content");
  await page.getByTestId("quick-summary-section-sources").click();
  await page.locator(".quick-summary-more").waitFor();
  await page.screenshot({ path: screenshots.workersNarrow, fullPage: false });
}

async function seedWorkerFixture(page, milimHome, status) {
  const fixture = await page.evaluate(async () => {
    const key = "milim.sessions";
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    const now = Date.now();
    const timestamp = new Date(now).toISOString();
    const sessionId = "e2e-workers-parent";
    const runId = "e2e-workers-run";
    const tasks = [
      { id: "task-a", title: "Inspect API contract", prompt: "Review the Worker Run API contract.", role: "Reviewer", agent_id: null, model: "test-model", access: "read_only" },
      { id: "task-b", title: "Check desktop states", prompt: "Check normal and narrow inspector states.", role: "UI audit", agent_id: null, model: "test-model", access: "read_only" },
    ];
    const value = JSON.stringify({
      state: {
        sessions: [{
          id: sessionId,
          title: "Worker Context fixture",
          messages: [
            {
              role: "user",
              content: "Use workers to inspect this change.",
              attachments: Array.from({ length: 7 }, (_, index) => ({ id: `source-${index + 1}`, name: `source-${index + 1}.txt`, mime: "text/plain", size: 1 })),
            },
            { id: "turn-a", role: "assistant", content: "", workerRunId: runId },
          ],
          settings: { model: "", instructions: "", activeAgentId: null, folder: "", sandbox: false, computerUse: false, memory: false, privacy: "off", toolApproval: "guarded", delegationPolicy: "ask", workerModel: "", planMode: false },
          contextPanelOpen: true,
          createdAt: now,
          updatedAt: now,
        }],
        activeId: sessionId,
      },
      version: 0,
    });
    if (invoke) await invoke("user_sessions_set", { value });
    else window.localStorage.setItem(key, value);
    return { sessionId, runId, tasks, timestamp };
  });
  await seedWorkerRunDatabase(milimHome, fixture, status);
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
}

async function seedWorkerRunDatabase(milimHome, fixture, status) {
  const { DatabaseSync } = await import("node:sqlite");
  const db = new DatabaseSync(join(milimHome, "threads.db"));
  db.exec("PRAGMA busy_timeout = 5000; BEGIN IMMEDIATE");
  try {
    db.prepare("DELETE FROM threads WHERE run_id = ?").run(fixture.runId);
    db.prepare("DELETE FROM worker_runs WHERE id = ?").run(fixture.runId);
    db.prepare(`
      INSERT INTO worker_runs
        (id, parent_thread_id, parent_turn_id, policy, runtime, status, tasks, created_at, updated_at)
      VALUES (?, ?, 'turn-a', 'ask', 'managed', ?, ?, ?, ?)
    `).run(
      fixture.runId,
      fixture.sessionId,
      status,
      JSON.stringify(fixture.tasks),
      fixture.timestamp,
      fixture.timestamp,
    );
    if (status === "running") {
      const insertWorker = db.prepare(`
        INSERT INTO threads
          (id, parent_id, root_id, title, status, model, prompt, created_at, updated_at, run_id, runtime, access)
        VALUES (?, ?, ?, ?, 'running', ?, ?, ?, ?, ?, 'managed', 'read_only')
      `);
      fixture.tasks.forEach((task, index) => insertWorker.run(
        `worker-${index}`,
        fixture.sessionId,
        fixture.sessionId,
        task.title,
        task.model,
        task.prompt,
        fixture.timestamp,
        fixture.timestamp,
        fixture.runId,
      ));
    }
    db.exec("COMMIT");
  } catch (error) {
    db.exec("ROLLBACK");
    throw error;
  } finally {
    db.close();
  }
}

async function dismissOnboardingIfPresent(page) {
  await page.getByTestId("onboarding-preflight").waitFor({ state: "hidden", timeout: 10_000 }).catch(() => {});
  const flow = page.getByTestId("onboarding-flow");
  if (await flow.isVisible().catch(() => false)) {
    await page.getByLabel("Close onboarding").click();
    await flow.waitFor({ state: "hidden", timeout: 10_000 });
    await page.getByTestId("update-cards").waitFor({ state: "visible", timeout: 10_000 });
  }
  const updates = page.getByTestId("update-cards");
  if (await updates.isVisible().catch(() => false)) {
    await page.getByLabel("Dismiss updates").click();
    await updates.waitFor({ state: "hidden", timeout: 10_000 });
  }
}

async function runWindowPinCheck(page) {
  const pin = page.getByTestId("pin-window");
  await pin.waitFor();
  if ((await pin.getAttribute("aria-pressed")) !== "false") {
    await pin.click();
  }
  await pin.click();
  await page.waitForFunction(() => document.querySelector('[data-testid="pin-window"]')?.getAttribute("aria-pressed") === "true");
  await pin.click();
  await page.waitForFunction(() => document.querySelector('[data-testid="pin-window"]')?.getAttribute("aria-pressed") === "false");
}

async function runAppMenuCheck(page) {
  const trigger = page.getByTestId("app-menu-trigger");
  const menu = page.getByRole("menu", { name: "Milim menu" });
  await trigger.click();
  await menu.waitFor();
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("New chat"));
  await page.keyboard.press("End");
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("Quit Milim"));
  await page.keyboard.press("Home");
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("New chat"));
  await page.keyboard.press("ArrowUp");
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("Quit Milim"));
  await page.keyboard.press("ArrowDown");
  await page.waitForFunction(() => document.activeElement?.textContent?.includes("New chat"));
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "hidden" });
  await expectFocusedTestId(page, "app-menu-trigger");

  await trigger.click();
  await menu.getByText("Hide sidebar").click();
  await page.getByTestId("sidebar-search").waitFor({ state: "hidden" });
  await trigger.click();
  await menu.getByText("Show sidebar").click();
  await page.getByTestId("sidebar-search").waitFor();

  await trigger.click();
  await menu.getByText("Settings", { exact: true }).click();
  await page.getByTestId("settings-dialog").waitFor();
  await closeSettings(page);
}

async function openAgents(page) {
  await page.getByTestId("agent-switcher").click();
  await page.getByTestId("manage-agents").click();
}

async function closeAgents(page) {
  await page.getByTestId("close-agents").click();
}

async function openProviders(page) {
  await page.getByTestId("model-picker-trigger").click();
  await page.getByTestId("manage-providers").click();
  await page.getByTestId("provider-overview").waitFor();
}

async function closeProviders(page) {
  await page.getByTestId("close-providers").click();
}

async function openSettings(page) {
  await page.getByTestId("open-settings").click();
  await page.getByTestId("settings-section-app").waitFor();
  const sheetMotion = await page.locator(".sheet-overlay .sheet").evaluate((element) => {
    const style = getComputedStyle(element);
    const origin = style.transformOrigin.split(" ").map(Number.parseFloat);
    return {
      transitionDuration: style.transitionDuration,
      transitionProperty: style.transitionProperty,
      centeredOrigin: Math.abs(origin[0] - element.offsetWidth / 2) < 1 && Math.abs(origin[1] - element.offsetHeight / 2) < 1,
    };
  });
  if (
    !sheetMotion.transitionDuration.includes("0.18s") ||
    !sheetMotion.transitionProperty.includes("scale") ||
    !sheetMotion.transitionProperty.includes("translate") ||
    !sheetMotion.centeredOrigin
  ) {
    throw new Error(`Settings sheet should use centered 180ms entry motion: ${JSON.stringify(sheetMotion)}.`);
  }
  const usageToggle = page.getByTestId("general-titlebar-account-usage-toggle");
  if (await usageToggle.isVisible() && await usageToggle.getAttribute("aria-checked") !== "true") {
    throw new Error("Title-bar account usage should default on.");
  }
}

async function runSettingsLayoutCheck(page) {
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.getByTestId("empty-usage-ridgeline").waitFor();
  await openSettings(page);
  await page.getByTestId("settings-section-appearance").click();
  const ridgelineToggle = page.getByTestId("empty-chat-ridgeline-toggle");
  await ridgelineToggle.scrollIntoViewIfNeeded();
  if (await ridgelineToggle.getAttribute("aria-checked") !== "true") {
    throw new Error("The empty-chat ridgeline should default on.");
  }
  await page.screenshot({ path: screenshots.settings, fullPage: false });

  await page.setViewportSize({ width: 740, height: 720 });
  await page.waitForTimeout(200);
  const navigation = await page.locator(".settings-nav-list").evaluate((element) => ({
    display: getComputedStyle(element).display,
    direction: getComputedStyle(element).flexDirection,
  }));
  if (navigation.display !== "flex" || navigation.direction !== "row") {
    throw new Error(`Settings navigation should become horizontal below 760px: ${JSON.stringify(navigation)}.`);
  }
  await ridgelineToggle.scrollIntoViewIfNeeded();
  await page.screenshot({ path: screenshots.settingsNarrow, fullPage: false });

  await ridgelineToggle.click();
  await closeSettings(page);
  await page.getByTestId("empty-usage-ridgeline").waitFor({ state: "hidden" });
}

async function closeSettings(page) {
  await page.getByTestId("close-settings").click();
}

async function runAppShortcutSettingsCheck(page) {
  await page.getByTestId("settings-section-system").click();
  await page.getByTestId("app-shortcut-stopGeneration").click();
  await page.keyboard.press("F2");
  await shortcutRow(page, "stopGeneration").getByText("F2").waitFor();
}

async function assertAppShortcutsPersisted(page) {
  await page.getByTestId("settings-section-system").click();
  await shortcutRow(page, "newChat").getByText("Ctrl+N").waitFor();
  await shortcutRow(page, "focusSearch").getByText("Ctrl+K").waitFor();
  await shortcutRow(page, "focusComposer").getByText("Ctrl+L").waitFor();
  await shortcutRow(page, "stopGeneration").getByText("F2").waitFor();
  await shortcutRow(page, "toggleSidebar").getByText("Ctrl+B").waitFor();
  await shortcutRow(page, "previousThread").getByText("Ctrl+Tab").waitFor();
}

function shortcutRow(page, action) {
  return page.locator(".shortcut-recorder-row", { has: page.getByTestId(`app-shortcut-${action}`) });
}

async function runAppShortcutCheck(page) {
  await runUiZoomShortcutCheck(page);
  await seedChatSearchFixture(page);
  await page.getByTestId("composer-input").fill("shortcut draft");
  await page.keyboard.press("Control+B");
  await page.getByTestId("sidebar-search").waitFor({ state: "hidden" });
  await page.keyboard.press("Control+B");
  await page.getByTestId("sidebar-search").waitFor();

  await runCommandPaletteCheck(page);

  await page.keyboard.press("Control+L");
  await expectFocusedTestId(page, "composer-input");
  await page.keyboard.press("Control+N");
  await expectFocusedTestId(page, "composer-input");
  const value = await page.getByTestId("composer-input").inputValue();
  if (value !== "") throw new Error(`Expected Ctrl+N to clear composer, got "${value}".`);
}

async function runCommandPaletteCheck(page) {
  await page.keyboard.press("Control+K");
  await page.getByTestId("command-palette-input").waitFor();
  const searchMotion = await page.locator(".chat-search-overlay").evaluate((element) => {
    const style = getComputedStyle(element);
    return { animationName: style.animationName, transitionProperty: style.transitionProperty };
  });
  if (searchMotion.animationName !== "none" || /opacity|transform|scale|translate/.test(searchMotion.transitionProperty)) {
    throw new Error(`Keyboard command palette should open instantly: ${JSON.stringify(searchMotion)}.`);
  }
  await expectFocusedTestId(page, "command-palette-input");
  await page.getByTestId("command-palette-input").fill("open settings");
  await page.getByTestId("command-palette-command").filter({ hasText: "Open settings" }).waitFor();
  await page.keyboard.press("Enter");
  await page.getByTestId("settings-dialog").waitFor();
  await closeSettings(page);

  await page.keyboard.press("Control+K");
  await page.getByTestId("command-palette-input").fill("volcano ledger");
  await page.getByTestId("command-palette-chat").filter({ hasText: "Older Search Fixture" }).waitFor();
  await page.keyboard.press("Enter");
  await page.getByTestId("user-message").filter({ hasText: "volcano ledger phrase" }).waitFor();
  if (await page.getByTestId("command-palette-input").isVisible().catch(() => false)) {
    await closeChatSearch(page);
  }
  await page.keyboard.press("Control+Tab");
  await page.getByTestId("user-message").filter({ hasText: "volcano ledger phrase" }).waitFor({ state: "hidden" });
  await page.keyboard.press("Control+Tab");
  await page.getByTestId("user-message").filter({ hasText: "volcano ledger phrase" }).waitFor();
  await page.keyboard.press("Control+K");
  await page.getByTestId("command-palette-input").waitFor();
  await closeChatSearch(page);

  const diagnosticsMarker = `tauri-diagnostics-${Date.now()}`;
  const diagnosticsDir = await page.evaluate(async (marker) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    await invoke("record_frontend_error", { message: marker });
    return await invoke("diagnostics_path");
  }, diagnosticsMarker);
  await waitForFileText(join(diagnosticsDir, "desktop.log"), diagnosticsMarker);
}

async function runRestartCheck(session) {
  await session.page.evaluate(async () => {
    await window.__TAURI_INTERNALS__.invoke("restart_app");
  }).catch(() => {});
  await waitForExit(session.child, 20_000);
  await session.browser?.close().catch(() => {});
  session.browser = null;
  session.page = null;

  const started = Date.now();
  let lastError;
  while (Date.now() - started < 20_000) {
    let browser;
    try {
      browser = await chromium.connectOverCDP(cdpUrl);
      const context = browser.contexts()[0] ?? await browser.newContext();
      const page = await firstPage(context);
      page.setDefaultTimeout(10_000);
      await page.getByTestId("chat-shell").waitFor({ timeout: 2_000 });
      session.browser = browser;
      session.page = page;
      session.restarted = true;
      return;
    } catch (error) {
      lastError = error;
      await browser?.close().catch(() => {});
      await delay(250);
    }
  }
  throw new Error(`Restarted Tauri app did not return through CDP: ${lastError?.message || "unknown error"}`);
}

async function runUiZoomShortcutCheck(page) {
  const chip = page.getByTestId("ui-zoom-chip");
  const value = page.getByTestId("ui-zoom-value");
  const composer = page.getByTestId("composer-input");

  await page.keyboard.press("Control+=");
  await chip.waitFor();
  await value.filter({ hasText: "110%" }).waitFor();
  await page.screenshot({ path: screenshots.zoom, fullPage: false });

  const increase = page.getByTestId("ui-zoom-increase");
  for (let step = 0; step < 3; step += 1) await increase.click();
  await value.filter({ hasText: "140%" }).waitFor();
  if (!(await increase.isDisabled())) {
    throw new Error("Zoom in should be disabled at 140%.");
  }

  await composer.hover();
  await composer.focus();
  await delay(3000);
  await page.keyboard.press("Control+=");
  await delay(1500);
  await chip.waitFor();

  const reset = page.getByTestId("ui-zoom-reset");
  await reset.click();
  await value.filter({ hasText: "100%" }).waitFor();
  await page.getByTestId("ui-zoom-decrease").click();
  await value.filter({ hasText: "90%" }).waitFor();
  await increase.click();
  await value.filter({ hasText: "100%" }).waitFor();
  if (!(await reset.isDisabled())) throw new Error("Zoom reset should be disabled at 100%.");

  await chip.hover();
  await increase.focus();
  await delay(3200);
  await chip.waitFor({ state: "hidden" });
}

async function runAccountUsageTitleBarCheck(page) {
  const modelLabel = await page.getByTestId("model-picker-trigger").locator(".chip-label").innerText();
  if (!modelLabel.toLowerCase().includes("codex")) {
    console.log("accountUsageCheck=skipped:active model is not Codex");
    return;
  }

  await page.route("**/codex/rate-limits", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      rateLimits: {
        primary: { usedPercent: 48, windowDurationMins: 300, resetsAt: 1_782_660_000 },
        secondary: { usedPercent: 60, windowDurationMins: 10_080, resetsAt: 1_782_900_000 },
      },
    }),
  }));

  await openSettings(page);
  const toggle = page.getByTestId("general-titlebar-account-usage-toggle");
  await toggle.click();
  await page.getByTestId("account-usage-pill").waitFor({ state: "hidden" });
  await toggle.click();
  await closeSettings(page);

  const pill = page.getByTestId("account-usage-pill");
  await pill.filter({ hasText: "Codex · 5h 52% left · weekly 40% left" }).waitFor();
  const pillBox = await pill.boundingBox();
  const controlsBox = await page.locator(".topbar-right").boundingBox();
  if (pillBox && controlsBox && pillBox.x + pillBox.width > controlsBox.x) {
    throw new Error("Account usage pill should not overlap title-bar controls.");
  }
  await page.screenshot({ path: screenshots.accountUsage, fullPage: false });

  await page.unroute("**/codex/rate-limits");
  await page.route("**/codex/rate-limits", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: "{",
  }));
  await openSettings(page);
  await toggle.click();
  await toggle.click();
  await closeSettings(page);
  await pill.waitFor({ state: "hidden" });
}

async function runMicroUiCheck(page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: async () => undefined },
    });
  });
  await page.evaluate(() => window.localStorage.setItem("milim.perf", "1"));
  await seedChatSearchFixture(page, true);
  await dismissOnboardingIfPresent(page);
  await page.keyboard.press("Control+K");
  await page.getByTestId("command-palette-input").fill("volcano ledger");
  await page.getByTestId("command-palette-chat").filter({ hasText: "Older Search Fixture" }).click();

  const message = page.getByTestId("user-message").last();
  await message.hover();
  const copy = message.getByTestId("message-copy");
  await copy.click();
  await assertAttribute(copy, "title", "Copied");

  await runHoverScrollTextCheck(page);
  await assertPointerReorderFollowsSource(page, {
    rowSelector: ".queued-item[data-queued-message-id]",
    handleSelector: ".queued-drag-handle",
    idAttribute: "data-queued-message-id",
    label: "Queued message",
  });
  await assertPointerReorderFollowsSource(page, {
    rowSelector: '.session-item[data-sidebar-session-id="e2e-motion-fixture"], .session-item[data-sidebar-session-id="e2e-search-fixture"]',
    idAttribute: "data-sidebar-session-id",
    label: "Sidebar thread",
  });
  await page.getByTestId("project-menu-trigger").click();
  const projectMenu = page.locator(".session-menu.project-menu");
  await projectMenu.waitFor();
  const popoverMotion = await projectMenu.evaluate((element) => {
    const style = getComputedStyle(element);
    const origin = style.transformOrigin.split(" ").map(Number.parseFloat);
    return {
      duration: style.transitionDuration,
      property: style.transitionProperty,
      topLeftOrigin: origin[0] < 1 && origin[1] < 1,
    };
  });
  if (
    !popoverMotion.duration.includes("0.12s") ||
    !popoverMotion.property.includes("scale") ||
    !popoverMotion.topLeftOrigin
  ) {
    throw new Error(`Occasional project menu should use origin-aware 120ms entry motion: ${JSON.stringify(popoverMotion)}.`);
  }
  await page.getByTestId("project-menu-trigger").click();

  const composer = page.getByTestId("composer-input");
  await composer.fill("");
  await composer.focus();
  await page.keyboard.press("ArrowUp");
  const history = page.getByTestId("composer-history-status");
  await history.filter({ hasText: "History 1 / 1" }).waitFor();
  if (!(await composer.inputValue()).includes("volcano ledger phrase")) {
    throw new Error("Composer history should recall the latest sent message.");
  }
  await page.screenshot({ path: screenshots.microUi, fullPage: false });
  await history.waitFor({ state: "hidden", timeout: 3000 });

  const sidebarHandle = page.getByTestId("sidebar-resize-handle");
  await sidebarHandle.focus();
  await page.keyboard.press("ArrowRight");
  if ((await sidebarHandle.getAttribute("aria-valuenow")) === "248") {
    throw new Error("Sidebar keyboard resize should change its width.");
  }
  await page.keyboard.press("Enter");
  await assertAttribute(sidebarHandle, "aria-valuenow", "248");
  await page.keyboard.press("ArrowRight");
  await sidebarHandle.dblclick();
  await assertAttribute(sidebarHandle, "aria-valuenow", "248");
  await delay(220);

  await resetUiPersistenceWrites(page);
  const sidebarDragBox = await sidebarHandle.boundingBox();
  if (!sidebarDragBox) throw new Error("Sidebar resize handle should have measurable bounds.");
  const sidebarDragX = sidebarDragBox.x + 4;
  const sidebarDragY = sidebarDragBox.y + sidebarDragBox.height / 2;
  await page.mouse.move(sidebarDragX, sidebarDragY);
  await page.mouse.down();
  for (let step = 1; step <= 24; step += 1) {
    await page.mouse.move(sidebarDragX + step * 4, sidebarDragY);
    await delay(8);
  }
  await assertAttribute(sidebarHandle, "aria-valuenow", "344");
  await assertUiPersistenceWrites(page, 0, "Sidebar drag before pointer-up");
  await page.mouse.up();
  await assertAttribute(sidebarHandle, "aria-valuenow", "344");
  await assertUiPersistenceWrites(page, 1, "Completed sidebar drag");
  await sidebarHandle.focus();
  await page.keyboard.press("Enter");
  await assertAttribute(sidebarHandle, "aria-valuenow", "248");
  await delay(220);

  const sidebarHandleBox = await sidebarHandle.boundingBox();
  if (!sidebarHandleBox) throw new Error("Sidebar resize handle should have measurable bounds.");
  await page.mouse.move(sidebarHandleBox.x + 4, sidebarHandleBox.y + sidebarHandleBox.height / 2);
  await page.mouse.down();
  await delay(50);
  await page.mouse.move(sidebarHandleBox.x - 112, sidebarHandleBox.y + sidebarHandleBox.height / 2, { steps: 4 });
  await assertAttribute(sidebarHandle, "aria-valuenow", "220");
  await page.mouse.move(sidebarHandleBox.x - 128, sidebarHandleBox.y + sidebarHandleBox.height / 2);
  await sidebarHandle.waitFor({ state: "hidden" });
  await delay(150);
  await page.mouse.move(sidebarHandleBox.x - 112, sidebarHandleBox.y + sidebarHandleBox.height / 2);
  await sidebarHandle.waitFor();
  await assertAttribute(sidebarHandle, "aria-valuenow", "220");
  await page.mouse.move(sidebarHandleBox.x - 128, sidebarHandleBox.y + sidebarHandleBox.height / 2);
  await sidebarHandle.waitFor({ state: "hidden" });
  await page.mouse.up();
  await page.getByTitle("Expand sidebar").click();
  await sidebarHandle.waitFor();
  await sidebarHandle.focus();
  await page.keyboard.press("ArrowRight");
  if ((await sidebarHandle.getAttribute("aria-valuenow")) === "220") {
    throw new Error("Reopened sidebar should remain resizable.");
  }
  await page.keyboard.press("Enter");

  await page.getByTestId("open-artifact-browser").click();
  const previewHandle = page.getByTestId("preview-resize-handle");
  await previewHandle.waitFor();
  await previewHandle.focus();
  await page.keyboard.press("ArrowLeft");
  if ((await previewHandle.getAttribute("aria-valuenow")) === "420") {
    throw new Error("Inspector keyboard resize should change its width.");
  }
  await page.keyboard.press("Enter");
  await assertAttribute(previewHandle, "aria-valuenow", "420");
  await page.keyboard.press("ArrowLeft");
  await previewHandle.dblclick();
  await assertAttribute(previewHandle, "aria-valuenow", "420");

  await resetUiPersistenceWrites(page);
  const previewDragBox = await previewHandle.boundingBox();
  if (!previewDragBox) throw new Error("Inspector resize handle should have measurable bounds.");
  const previewDragX = previewDragBox.x + 4;
  const previewDragY = previewDragBox.y + previewDragBox.height / 2;
  await page.mouse.move(previewDragX, previewDragY);
  await page.mouse.down();
  for (let step = 1; step <= 24; step += 1) {
    await page.mouse.move(previewDragX - step * 4, previewDragY);
    await delay(8);
  }
  await assertAttribute(previewHandle, "aria-valuenow", "516");
  await assertUiPersistenceWrites(page, 0, "Inspector drag before pointer-up");
  await page.mouse.up();
  await assertAttribute(previewHandle, "aria-valuenow", "516");
  await assertUiPersistenceWrites(page, 1, "Completed inspector drag");
  await previewHandle.focus();
  await page.keyboard.press("Enter");
  await assertAttribute(previewHandle, "aria-valuenow", "420");

  await runProgressiveInspectorResizeCheck(page, previewHandle);

  const previewHandleBox = await previewHandle.boundingBox();
  if (!previewHandleBox) throw new Error("Inspector resize handle should have measurable bounds.");
  await page.mouse.move(previewHandleBox.x + 4, previewHandleBox.y + previewHandleBox.height / 2);
  await page.mouse.down();
  await delay(50);
  await page.mouse.move(previewHandleBox.x + 152, previewHandleBox.y + previewHandleBox.height / 2, { steps: 4 });
  await assertAttribute(previewHandle, "aria-valuenow", "360");
  await page.mouse.move(previewHandleBox.x + 168, previewHandleBox.y + previewHandleBox.height / 2);
  const closingPreviewPanel = page.locator(".preview-panel.closing");
  await closingPreviewPanel.waitFor();
  const closeMotion = await closingPreviewPanel.evaluate((element) => {
    const style = getComputedStyle(element);
    element.dataset.motionProbe = "close-reversal";
    return {
      property: style.transitionProperty,
      duration: style.transitionDuration,
      opacity: style.opacity,
    };
  });
  if (!closeMotion.property.includes("flex-basis") || !closeMotion.duration.includes("0.18s")) {
    throw new Error(`Inspector close should use the shared 180ms transition: ${JSON.stringify(closeMotion)}.`);
  }
  await page.mouse.move(previewHandleBox.x + 152, previewHandleBox.y + previewHandleBox.height / 2);
  const reversingPanel = page.locator('[data-motion-probe="close-reversal"]');
  await reversingPanel.waitFor();
  await page.locator(".preview-panel:not(.closing)").waitFor();
  await previewHandle.waitFor();
  const openMotion = await reversingPanel.evaluate((element) => {
    const style = getComputedStyle(element);
    return {
      className: element.className,
      property: style.transitionProperty,
      duration: style.transitionDuration,
    };
  });
  if (openMotion.className.includes("closing") || !openMotion.duration.includes("0.18s")) {
    throw new Error(`Inspector close reversal should retarget the mounted panel: ${JSON.stringify(openMotion)}.`);
  }
  await page.mouse.up();
  await page.emulateMedia({ reducedMotion: "reduce" });
  const reducedMotion = await page.locator(".preview-panel").evaluate((element) => ({
    transitionDuration: getComputedStyle(element).transitionDuration,
  }));
  if (reducedMotion.transitionDuration !== "0s") {
    throw new Error(`Reduced motion should remove inspector movement: ${JSON.stringify(reducedMotion)}.`);
  }
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await previewHandle.focus();
  await page.keyboard.press("ArrowLeft");
  if ((await previewHandle.getAttribute("aria-valuenow")) === "360") {
    throw new Error("Reopened inspector should remain resizable.");
  }
  await page.keyboard.press("Enter");
}

async function runHoverScrollTextCheck(page) {
  const label = page.locator('[data-sidebar-session-id="e2e-motion-fixture"] [data-hover-scroll-text]').first();
  const composer = page.getByTestId("composer-input");
  await label.waitFor();
  const overflowing = await label.evaluate((outer) => {
    const inner = outer.querySelector("[data-hover-scroll-inner]");
    return Boolean(inner && inner.scrollWidth > outer.clientWidth + 1);
  });
  if (!overflowing) throw new Error("Hover-scroll fixture should overflow its sidebar row.");

  await label.hover();
  await delay(800);
  const waiting = await label.evaluate((outer) => {
    const inner = outer.querySelector("[data-hover-scroll-inner]");
    return {
      active: inner?.classList.contains("hover-scroll-text-active") ?? false,
      animations: inner?.getAnimations().length ?? 0,
      overflow: inner ? getComputedStyle(inner).textOverflow : "",
      title: outer.getAttribute("title"),
    };
  });
  if (waiting.active || waiting.animations || waiting.overflow !== "ellipsis" || waiting.title !== "") {
    throw new Error(`Hover scroll should retain its ellipsis during the delay: ${JSON.stringify(waiting)}.`);
  }

  await delay(350);
  const running = await label.evaluate((outer) => {
    const inner = outer.querySelector("[data-hover-scroll-inner]");
    return {
      active: inner?.classList.contains("hover-scroll-text-active") ?? false,
      animations: inner?.getAnimations().length ?? 0,
      transform: inner ? getComputedStyle(inner).transform : "none",
    };
  });
  if (!running.active || running.animations !== 1 || running.transform === "none") {
    throw new Error(`Hover scroll should move overflowing text after one second: ${JSON.stringify(running)}.`);
  }

  await composer.hover();
  const reset = await label.evaluate((outer) => {
    const inner = outer.querySelector("[data-hover-scroll-inner]");
    return {
      active: inner?.classList.contains("hover-scroll-text-active") ?? false,
      animations: inner?.getAnimations().length ?? 0,
      transform: inner ? getComputedStyle(inner).transform : "none",
      title: outer.getAttribute("title"),
    };
  });
  if (reset.active || reset.animations || reset.transform !== "none" || !reset.title) {
    throw new Error(`Hover scroll should reset and restore its title on exit: ${JSON.stringify(reset)}.`);
  }

  await page.emulateMedia({ reducedMotion: "reduce" });
  try {
    await label.hover();
    await delay(1_100);
    const reduced = await label.evaluate((outer) => {
      const inner = outer.querySelector("[data-hover-scroll-inner]");
      return {
        active: inner?.classList.contains("hover-scroll-text-active") ?? false,
        animations: inner?.getAnimations().length ?? 0,
        title: outer.getAttribute("title"),
      };
    });
    if (reduced.active || reduced.animations || !reduced.title) {
      throw new Error(`Reduced motion should retain the static label and native title: ${JSON.stringify(reduced)}.`);
    }
  } finally {
    await composer.hover();
    await page.emulateMedia({ reducedMotion: "no-preference" });
  }
}

async function runProgressiveInspectorResizeCheck(page, previewHandle) {
  const sidebar = page.locator(".sidebar");
  const sidebarHandle = page.getByTestId("sidebar-resize-handle");
  const chatBody = page.locator(".chat-body");
  const previewPanel = page.locator(".chat-body > .preview-panel");
  const startBox = await previewHandle.boundingBox();
  if (!startBox) throw new Error("Progressive inspector resize requires measurable handle bounds.");
  const startX = startBox.x + startBox.width / 2;
  const startY = startBox.y + startBox.height / 2;
  const startWidth = Number(await previewHandle.getAttribute("aria-valuenow"));
  const initial = await page.evaluate(() => {
    const body = document.querySelector(".chat-body");
    const rail = document.querySelector(".sidebar");
    return {
      bodyWidth: body?.getBoundingClientRect().width ?? 0,
      sidebarWidth: rail?.getBoundingClientRect().width ?? 0,
    };
  });
  const dockedLimit = Math.round(initial.bodyWidth - 420 - 8);
  const sidebarGain = Math.round(initial.sidebarWidth - 48);
  const dockedDelta = dockedLimit - startWidth;

  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX - dockedDelta - 31, startY);
  if ((await sidebar.getAttribute("class"))?.includes("collapsed")) {
    throw new Error("Inspector should not collapse the sidebar before the 32px overshoot.");
  }
  await page.mouse.move(startX - dockedDelta - 33, startY);
  await page.waitForFunction(() => document.querySelector(".sidebar")?.classList.contains("collapsed"));
  await page.mouse.move(startX - dockedDelta, startY);
  await page.waitForFunction(() => !document.querySelector(".sidebar")?.classList.contains("collapsed"));
  await page.mouse.up();
  await previewHandle.focus();
  await page.keyboard.press("Enter");
  await delay(220);

  await resetUiPersistenceWrites(page);
  const stickyBox = await previewHandle.boundingBox();
  if (!stickyBox) throw new Error("Inspector handle should remain measurable after reversal.");
  const stickyX = stickyBox.x + stickyBox.width / 2;
  const stickyY = stickyBox.y + stickyBox.height / 2;
  await page.mouse.move(stickyX, stickyY);
  await page.mouse.down();
  await page.mouse.move(stickyX - dockedDelta - 33, stickyY);
  await page.waitForFunction(() => document.querySelector(".sidebar")?.classList.contains("collapsed"));
  await delay(220);
  const overlayDelta = dockedLimit + sidebarGain - startWidth;
  await page.mouse.move(stickyX - overlayDelta - 31, stickyY);
  if ((await chatBody.getAttribute("class"))?.includes("inspector-overlay")) {
    throw new Error("Inspector should not enter overlay before the second 32px overshoot.");
  }
  const transcriptBeforeOverlay = await page.locator(".chat-main").boundingBox();
  await page.mouse.move(stickyX - overlayDelta - 33, stickyY);
  await page.waitForFunction(() => document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  const overlayGeometry = await page.evaluate(() => {
    const body = document.querySelector(".chat-body");
    const transcript = document.querySelector(".chat-main");
    const panel = body?.querySelector(":scope > .preview-panel, :scope > .inspector-git-panel");
    const bodyRect = body?.getBoundingClientRect();
    const transcriptRect = transcript?.getBoundingClientRect();
    const panelRect = panel?.getBoundingClientRect();
    return {
      transcriptWidth: transcriptRect?.width ?? 0,
      transcriptRight: transcriptRect?.right ?? 0,
      panelLeft: panelRect?.left ?? 0,
      panelRightGap: bodyRect && panelRect ? Math.abs(bodyRect.right - panelRect.right) : Infinity,
    };
  });
  if (!transcriptBeforeOverlay || Math.abs(overlayGeometry.transcriptWidth - transcriptBeforeOverlay.width) > 1) {
    throw new Error(`Overlay should not reflow the transcript: ${JSON.stringify({ transcriptBeforeOverlay, overlayGeometry })}.`);
  }
  if (overlayGeometry.panelLeft >= overlayGeometry.transcriptRight || overlayGeometry.panelRightGap > 1) {
    throw new Error(`Overlay should right-anchor across the transcript: ${JSON.stringify(overlayGeometry)}.`);
  }
  if (!(await previewHandle.getAttribute("aria-valuetext"))?.includes("overlay")) {
    throw new Error("Inspector separator should expose overlay state through aria-valuetext.");
  }
  await page.screenshot({ path: screenshots.inspectorOverlay, fullPage: false });
  await page.mouse.move(stickyX - overlayDelta, stickyY);
  await page.waitForFunction(() => !document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await page.mouse.move(stickyX - overlayDelta - 33, stickyY);
  await page.waitForFunction(() => document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await assertUiPersistenceWrites(page, 1, "Progressive inspector drag before pointer-up");
  await page.mouse.up();
  await assertUiPersistenceWrites(page, 2, "Completed progressive inspector drag");
  if (!(await sidebar.getAttribute("class"))?.includes("collapsed")) {
    throw new Error("Auto-collapsed sidebar should remain collapsed after pointer-up.");
  }

  const overlayBox = await previewHandle.boundingBox();
  if (!overlayBox) throw new Error("Overlay inspector handle should remain measurable.");
  const overlayWidth = Number(await previewHandle.getAttribute("aria-valuenow"));
  const collapsedDockedLimit = Math.round((await chatBody.boundingBox()).width - 420 - 8);
  await page.mouse.move(overlayBox.x + overlayBox.width / 2, overlayBox.y + overlayBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(
    overlayBox.x + overlayBox.width / 2 + overlayWidth - collapsedDockedLimit,
    overlayBox.y + overlayBox.height / 2,
  );
  await page.waitForFunction(() => !document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await page.mouse.up();

  const contextLauncher = page.getByTestId("open-context-panel");
  await contextLauncher.click();
  const context = page.getByTestId("quick-summary-panel");
  await context.waitFor();
  await delay(220);
  await previewHandle.focus();
  await page.keyboard.press("End");
  const contextHandleBox = await previewHandle.boundingBox();
  if (!contextHandleBox) throw new Error("Inspector handle should be measurable beside Context.");
  const dockedGeometry = await page.evaluate(() => ({
    transcript: document.querySelector(".chat-main")?.getBoundingClientRect().toJSON(),
    context: document.querySelector('[data-testid="quick-summary-panel"]')?.getBoundingClientRect().toJSON(),
  }));
  const contextX = contextHandleBox.x + contextHandleBox.width / 2;
  const contextY = contextHandleBox.y + contextHandleBox.height / 2;
  await page.mouse.move(contextX, contextY);
  await page.mouse.down();
  await page.mouse.move(contextX - 31, contextY);
  if ((await chatBody.getAttribute("class"))?.includes("inspector-overlay")) {
    throw new Error("Collapsed-sidebar overlay should still require a 32px overshoot.");
  }
  await page.mouse.move(contextX - 33, contextY);
  await page.waitForFunction(() => document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  const contextOverlayGeometry = await page.evaluate(() => ({
    transcript: document.querySelector(".chat-main")?.getBoundingClientRect().toJSON(),
    context: document.querySelector('[data-testid="quick-summary-panel"]')?.getBoundingClientRect().toJSON(),
    panel: document.querySelector(".chat-body > .preview-panel")?.getBoundingClientRect().toJSON(),
  }));
  if (
    Math.abs(contextOverlayGeometry.transcript.width - dockedGeometry.transcript.width) > 1 ||
    Math.abs(contextOverlayGeometry.context.width - dockedGeometry.context.width) > 1 ||
    contextOverlayGeometry.panel.left >= contextOverlayGeometry.context.right
  ) {
    throw new Error(`Overlay should cover Context without reflowing it: ${JSON.stringify({ dockedGeometry, contextOverlayGeometry })}.`);
  }
  await page.mouse.move(contextX, contextY);
  await page.waitForFunction(() => !document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await page.mouse.up();
  await page.getByLabel("Close context", { exact: true }).click();

  await page.getByTitle("Expand sidebar").click();
  await sidebarHandle.waitFor();
  await delay(220);
  await previewHandle.focus();
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector(".sidebar")?.classList.contains("collapsed"));
  await delay(220);
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await page.waitForFunction(() => document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await page.keyboard.press("ArrowRight");
  await page.waitForFunction(() => !document.querySelector(".chat-body")?.classList.contains("inspector-overlay"));
  await page.keyboard.press("Home");
  await assertAttribute(previewHandle, "aria-valuenow", "360");
  await page.keyboard.press("Enter");
  await assertAttribute(previewHandle, "aria-valuenow", "420");
  await page.getByTitle("Expand sidebar").click();
  await sidebarHandle.waitFor();
  await delay(220);
}

async function resetUiPersistenceWrites(page) {
  const ready = await page.evaluate(() => {
    if (!window.__MILIM_PERF__) return false;
    window.__MILIM_PERF__.reset();
    return true;
  });
  if (!ready) throw new Error("UI persistence performance counters should be enabled.");
}

async function assertUiPersistenceWrites(page, expected, label) {
  const actual = await page.evaluate(() =>
    window.__MILIM_PERF__?.snapshot().counters["persist.milim.ui.write"] ?? 0,
  );
  if (actual !== expected) {
    throw new Error(`${label} should persist milim.ui ${expected} time(s), got ${actual}.`);
  }
}

async function assertPointerReorderFollowsSource(page, {
  rowSelector,
  handleSelector,
  idAttribute,
  label,
}) {
  const rows = page.locator(rowSelector);
  if (await rows.count() < 2) throw new Error(`${label} drag check requires two rows.`);
  const source = rows.first();
  const target = rows.nth(1);
  const sourceId = await source.getAttribute(idAttribute);
  const before = await rows.evaluateAll((elements, attribute) =>
    elements.map((element) => element.getAttribute(attribute)), idAttribute);
  const sourceBox = await (handleSelector ? source.locator(handleSelector) : source).boundingBox();
  const targetBox = await target.boundingBox();
  if (!sourceId || !sourceBox || !targetBox) throw new Error(`${label} rows should have measurable bounds.`);

  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(targetBox.x + targetBox.width / 2, targetBox.y + targetBox.height - 2, { steps: 4 });
  const direct = await source.evaluate((element) => ({
    pointerEvents: element.style.pointerEvents,
    translate: element.style.translate,
  }));
  if (!direct.translate || direct.translate === "0px" || direct.pointerEvents !== "none") {
    throw new Error(`${label} source should follow the pointer directly: ${JSON.stringify(direct)}.`);
  }
  const beforePointerUp = await rows.evaluateAll((elements, attribute) =>
    elements.map((element) => element.getAttribute(attribute)), idAttribute);
  if (JSON.stringify(beforePointerUp) !== JSON.stringify(before)) {
    throw new Error(`${label} order should not persist before pointer-up.`);
  }

  await page.mouse.up();
  await page.waitForFunction(({ selector, attribute, id }) => {
    const elements = Array.from(document.querySelectorAll(selector));
    return elements[1]?.getAttribute(attribute) === id;
  }, { selector: rowSelector, attribute: idAttribute, id: sourceId });
  const cleared = await page.evaluate(({ selector, attribute, id }) => {
    const element = Array.from(document.querySelectorAll(selector))
      .find((candidate) => candidate.getAttribute(attribute) === id);
    return element instanceof HTMLElement
      ? { pointerEvents: element.style.pointerEvents, translate: element.style.translate }
      : null;
  }, { selector: rowSelector, attribute: idAttribute, id: sourceId });
  if (!cleared || cleared.pointerEvents || cleared.translate) {
    throw new Error(`${label} direct drag state should clear after drop: ${JSON.stringify(cleared)}.`);
  }
}

async function closeChatSearch(page) {
  await page.keyboard.press("Escape");
  if (await page.getByTestId("command-palette-input").isVisible().catch(() => false)) {
    await page.getByLabel("Close command palette").click();
  }
  await page.getByTestId("command-palette-input").waitFor({ state: "hidden" });
}

async function seedChatSearchFixture(page, withQueuedMessages = false) {
  await page.evaluate(async ({ withQueuedMessages }) => {
    const key = "milim.sessions";
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    const raw = invoke ? await invoke("user_state_get", { key }) : window.localStorage.getItem(key);
    const parsed = raw ? JSON.parse(raw) : { state: {} };
    const state = parsed.state && typeof parsed.state === "object" ? parsed.state : {};
    const now = Date.now();
    const current = {
      id: "e2e-current-chat",
      title: "New chat",
      messages: [],
      createdAt: now,
      updatedAt: now,
    };
    const existingSessions = Array.isArray(state.sessions) ? state.sessions : [];
    const sessions = (existingSessions.length ? existingSessions : [current]).filter((session) =>
      session && session.id !== "e2e-search-fixture" && session.id !== "e2e-motion-fixture");
    sessions.push({
      id: "e2e-motion-fixture",
      title: "Motion Fixture With A Deliberately Long Sidebar Thread Title",
      messages: [],
      createdAt: now - 6 * 24 * 60 * 60 * 1000,
      updatedAt: now - 1,
    });
    sessions.push({
      id: "e2e-search-fixture",
      title: "Older Search Fixture",
      messages: [
        { role: "user", content: "The volcano ledger phrase lives in this older message." },
      ],
      createdAt: now - 7 * 24 * 60 * 60 * 1000,
      updatedAt: now - 7 * 24 * 60 * 60 * 1000,
    });
    state.sessions = sessions;
    const queuedMessagesBySession =
      state.queuedMessagesBySession && typeof state.queuedMessagesBySession === "object"
        ? { ...state.queuedMessagesBySession }
        : {};
    if (withQueuedMessages) {
      queuedMessagesBySession["e2e-search-fixture"] = [
        { id: "e2e-queued-first", content: "First queued fixture", createdAt: now - 2 },
        { id: "e2e-queued-second", content: "Second queued fixture", createdAt: now - 1 },
      ];
    } else {
      delete queuedMessagesBySession["e2e-search-fixture"];
    }
    state.queuedMessagesBySession = queuedMessagesBySession;
    if (!sessions.some((session) => session.id === state.activeId)) state.activeId = sessions[0].id;
    parsed.state = state;
    const value = JSON.stringify(parsed);
    if (invoke) await invoke("user_state_set", { key, value });
    else window.localStorage.setItem(key, value);
  }, { withQueuedMessages });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
}

async function assertAgentOptions(page) {
  await openAgentMenu(page);
  for (const profile of profiles) {
    const option = page.getByTestId(`agent-option-${profile.name}`);
    await option.waitFor();
    await assertAvatarSeed(option.locator("shatz-avatar"), profile.avatar);
  }
  await closeAgentMenu(page);
}

async function assertAvatarSeed(locator, seed) {
  await locator.waitFor();
  const actual = await locator.getAttribute("seed");
  if (actual !== seed) throw new Error(`Expected avatar seed ${JSON.stringify(seed)}, got ${JSON.stringify(actual)}.`);
  const visual = await locator.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      hasSvg: Boolean(element.shadowRoot?.querySelector("svg")),
      width: rect.width,
      height: rect.height,
    };
  });
  if (!visual.hasSvg || visual.width < 16 || visual.width > 40 || Math.abs(visual.width - visual.height) > 1) {
    throw new Error(`Avatar did not render as a square thumbnail: ${JSON.stringify(visual)}.`);
  }
}

async function assertScheduleAgentAvatar(page, profile) {
  const tools = page.getByRole("button", { name: "Tools", exact: true }).last();
  if ((await tools.getAttribute("aria-expanded")) !== "true") await tools.click();
  await page.getByRole("button", { name: "Schedules", exact: true }).last().click();
  await page.getByRole("button", { name: "New schedule", exact: true }).click();
  const select = page.getByTestId("schedule-agent-select");
  await select.waitFor();
  await select.click();
  const option = page.locator(".ui-select-item").filter({ hasText: profile.name });
  await assertAvatarSeed(option.locator("shatz-avatar"), profile.avatar);
  await option.click();
  await assertAvatarSeed(select.locator("shatz-avatar"), profile.avatar);
  await page.getByRole("button", { name: "Close schedules" }).click();
}

async function assertAgentAvatarsInLightTheme(page) {
  await page.getByTestId("open-settings").click();
  await page.getByTestId("settings-section-appearance").waitFor();
  await page.getByTestId("settings-section-appearance").click();
  await page.locator(".theme-card").filter({ hasText: "Mono Light" }).click();
  await closeSettings(page);
  await openAgents(page);
  const card = page.getByTestId("agent-editor-Security Review");
  await card.click();
  await assertAvatarSeed(card.locator("shatz-avatar"), profiles[1].avatar);
  await page.screenshot({ path: screenshots.avatarsLight, fullPage: false });
  await closeAgents(page);
  await page.getByTestId("open-settings").click();
  await page.getByTestId("settings-section-appearance").waitFor();
  await page.getByTestId("settings-section-appearance").click();
  await page.locator(".theme-card").filter({ hasText: "Mono Dark" }).click();
  await page.getByTestId("settings-section-app").click();
  await closeSettings(page);
}

async function selectAgent(page, name) {
  await openAgentMenu(page);
  await page.getByTestId(`agent-option-${name}`).click();
}

async function openAgentMenu(page) {
  const firstOption = page.getByTestId(`agent-option-${profiles[0].name}`);
  if (!(await firstOption.isVisible().catch(() => false))) {
    await page.getByTestId("agent-switcher").click();
  }
  await firstOption.waitFor();
}

async function closeAgentMenu(page) {
  if (await page.getByTestId(`agent-option-${profiles[0].name}`).isVisible().catch(() => false)) {
    await page.keyboard.press("Escape");
    await page.getByTestId(`agent-option-${profiles[0].name}`).waitFor({ state: "hidden" });
  }
}

async function createAgent(page, profile) {
  await page.getByTestId("new-agent").click();
  await page.getByTestId("agent-name-input").fill(profile.name);

  if (profile.avatar) {
    await page.getByTestId("agent-avatar-input").fill(profile.avatar);
  }

  await page.getByTestId("agent-system-prompt").fill(profile.prompt);

  if (profile.mode === "custom") {
    await setCustomTools(page, profile.tools);
  } else {
    await page.getByTestId(`tool-mode-${profile.mode}`).click();
  }

  await page.getByTestId("save-agent").click();
  await page.getByTestId(`agent-editor-${profile.name}`).waitFor();
}

async function setCustomTools(page, wantedTools) {
  const wanted = new Set(wantedTools);
  await page.getByTestId("tool-mode-custom").click();
  await page.getByTestId("tool-search").fill("");
  const rows = await page.locator(".tool-row").all();
  if (rows.length === 0) throw new Error("Expected custom tool rows to be visible.");

  for (const row of rows) {
    const name = (await row.locator(".tool-name").innerText()).trim();
    const checkbox = row.getByRole("checkbox");
    const checked = (await checkbox.getAttribute("aria-checked")) === "true";
    const shouldBeChecked = wanted.has(name);
    if (checked !== shouldBeChecked) {
      await checkbox.click();
    }
  }

  await assertSelectedTools(page, wantedTools);
}

async function assertSelectedTools(page, wantedTools) {
  const wanted = new Set(wantedTools);
  await page.getByTestId("tool-search").fill("");
  for (const tool of wanted) {
    await page.getByTestId(`tool-row-${tool}`).waitFor();
  }
  const rows = await page.locator(".tool-row").all();
  const seen = new Map();

  for (const row of rows) {
    const name = (await row.locator(".tool-name").innerText()).trim();
    const checkbox = row.getByRole("checkbox");
    const checked = (await checkbox.getAttribute("aria-checked")) === "true";
    seen.set(name, checked);
  }

  for (const tool of wanted) {
    if (seen.get(tool) !== true) throw new Error(`Expected tool ${tool} to be selected.`);
  }

  for (const [tool, checked] of seen) {
    if (!wanted.has(tool) && checked) throw new Error(`Expected tool ${tool} to be deselected.`);
  }
}

async function assertToolMode(page, mode) {
  const classes = await page.getByTestId(`tool-mode-${mode}`).getAttribute("class");
  if (!classes?.includes("active")) throw new Error(`Expected tool mode ${mode} to be active.`);
}

async function assertFieldContains(locator, text) {
  const value = await locator.inputValue();
  if (!value.includes(text)) throw new Error(`Expected field to contain "${text}".`);
}

async function assertTextContains(locator, text) {
  const value = await locator.innerText();
  if (!value.includes(text)) throw new Error(`Expected text to contain "${text}".`);
}

async function assertAttributeContains(locator, attribute, text) {
  const value = await locator.getAttribute(attribute);
  if (!value?.includes(text)) throw new Error(`Expected ${attribute} to contain "${text}".`);
}

async function expectFocusedTestId(page, testId) {
  await page.waitForFunction((expected) => document.activeElement?.getAttribute("data-testid") === expected, testId);
}

async function assertTextContainsIgnoreCase(locator, text) {
  const value = await locator.innerText();
  if (!value.toLowerCase().includes(text.toLowerCase())) {
    throw new Error(`Expected text to contain "${text}" ignoring case, got "${value}".`);
  }
}

async function waitForArtifactCardWithText(page, text) {
  const card = page.getByTestId("artifact-card").filter({ hasText: text });
  await card.waitFor({ timeout: 30_000 });
  return card;
}

async function waitForPersistedUserStateText(page, key, text, timeoutMs = 10_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const found = await page.evaluate(async ({ key, text }) => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      const value = invoke ? await invoke("user_state_get", { key }) : window.localStorage.getItem(key);
      if (typeof value !== "string") return false;
      try {
        const parsed = JSON.parse(value);
        const containsText = (item) => {
          if (typeof item === "string") return item.includes(text);
          if (Array.isArray(item)) return item.some(containsText);
          if (item && typeof item === "object") return Object.values(item).some(containsText);
          return false;
        };
        return containsText(parsed);
      } catch {
        return value.includes(text) || value.includes(text.replaceAll("\\", "\\\\"));
      }
    }, { key, text });
    if (found) return;
    await delay(100);
  }
  throw new Error(`Timed out waiting for persisted ${key} to include ${text}.`);
}

async function assertHidden(locator, label) {
  if (await locator.isVisible().catch(() => false)) {
    throw new Error(`Expected ${label} to be hidden.`);
  }
}

async function assertAttribute(locator, name, expected) {
  const value = await locator.getAttribute(name);
  if (value !== expected) throw new Error(`Expected ${name} to be "${expected}", got "${value}".`);
}

async function waitForTestIdTextContainsIgnoreCase(page, testId, text) {
  await page.waitForFunction(
    ([testId, expected]) => {
      const el = document.querySelector(`[data-testid="${testId}"]`);
      return el?.textContent?.toLowerCase().includes(expected.toLowerCase());
    },
    [testId, text],
  );
}

async function waitForFileText(path, expected, timeoutMs = 5_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (existsSync(path) && readFileSync(path, "utf8").includes(expected)) return;
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${path} to contain ${expected}`);
}

async function waitForLocatorCountGreaterThan(locator, minCount, timeoutMs = 5_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const count = await locator.count();
    if (count > minCount) return count;
    await delay(100);
  }
  throw new Error(`Timed out waiting for locator count to exceed ${minCount}.`);
}

async function waitForLocatorCountAtMost(locator, maxCount, timeoutMs = 5_000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const count = await locator.count();
    if (count <= maxCount) return count;
    await delay(100);
  }
  throw new Error(`Timed out waiting for locator count to return to ${maxCount} or less.`);
}

async function setSwitch(locator, checked, label) {
  const current = (await locator.getAttribute("aria-checked")) === "true";
  if (current !== checked) await locator.click();
  await assertSwitch(locator, checked, label);
}

async function assertSwitch(locator, checked, label) {
  const current = (await locator.getAttribute("aria-checked")) === "true";
  if (current !== checked) throw new Error(`Expected ${label} switch to be ${checked ? "on" : "off"}.`);
}

function collectErrors(page) {
  const errors = [];
  const expectedFailedResources = [];
  page.on("response", (response) => {
    const url = response.url();
    if (
      ([400, 502].includes(response.status()) && /\/media\/(?:models|model-schema)\b/.test(url)) ||
      (response.status() === 400 && /\/(?:codex|claude|opencode|pi)\/run\b/.test(url))
    ) {
      expectedFailedResources.push(url);
    }
  });
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    const url = msg.location().url;
    if (
      text.includes("Content Security Policy directive 'frame-src'") &&
      text.includes("http://[::1]:*")
    ) {
      return;
    }
    if (text.includes("favicon.ico") && text.includes("Content Security Policy directive")) return;
    if (/^Failed to load resource: the server responded with a status of (?:400 \(Bad Request\)|502 \(Bad Gateway\))$/.test(text)) {
      const responseIndex = expectedFailedResources.findIndex((resourceUrl) => !url || resourceUrl === url);
      if (/\/media\/(?:models|model-schema)\b/.test(url) || responseIndex >= 0) {
        if (responseIndex >= 0) expectedFailedResources.splice(responseIndex, 1);
        return;
      }
    }
    errors.push(url ? `${text} (${url})` : text);
  });
  page.on("pageerror", (err) => errors.push(err.message));
  return errors;
}

async function launchTauri(milimHome) {
  const child = spawn(binary, [], {
    cwd: root,
    env: {
      ...process.env,
      MILIM_HOME: milimHome,
      WEBVIEW2_USER_DATA_FOLDER: join(milimHome, "webview2"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: appendWebViewArg(
        process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS,
        `--remote-debugging-port=${cdpPort}`,
      ),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  const session = { child, stdout: "", stderr: "", browser: null, page: null };
  child.stdout?.on("data", (chunk) => {
    session.stdout += chunk.toString();
  });
  child.stderr?.on("data", (chunk) => {
    session.stderr += chunk.toString();
  });

  await waitForCdp(session, cdpUrl, 20_000);
  session.browser = await chromium.connectOverCDP(cdpUrl);
  const context = session.browser.contexts()[0] ?? await session.browser.newContext();
  session.page = await firstPage(context);
  session.page.setDefaultTimeout(10_000);
  return session;
}

async function closeSession(session) {
  await session.browser?.close().catch(() => {});
  session.browser = null;
  session.page = null;
  if (session.restarted) {
    await ensureNoWorkspaceMilimProcesses();
    await waitForPortClosed(cdpPort, 10_000);
    return;
  }
  if (session.child.exitCode == null) {
    const killResult = killTree(session.child.pid);
    await waitForExit(session.child, 10_000).catch((err) => {
      throw new Error(
        `Timed out waiting for Tauri process ${session.child.pid} to exit after taskkill.\n` +
          `taskkillStatus=${killResult?.status ?? "unknown"}\n` +
          `taskkillStdout=${killResult?.stdout ?? ""}\n` +
          `taskkillStderr=${killResult?.stderr ?? ""}\n` +
          `workspaceProcesses=${describeWorkspaceMilimProcesses()}\n` +
          `stdout:\n${session.stdout}\nstderr:\n${session.stderr}\n` +
          `waitError=${err.message}`,
      );
    });
  }
  await waitForPortClosed(cdpPort, 10_000);
}

function appendWebViewArg(existing, arg) {
  const trimmed = existing?.trim();
  return trimmed ? `${trimmed} ${arg}` : arg;
}

function isPortOpen(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => resolve(false));
    socket.setTimeout(500, () => {
      socket.destroy();
      resolve(false);
    });
  });
}

async function waitForPortClosed(port, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (!(await isPortOpen(port))) return;
    await delay(250);
  }
  throw new Error(`Timed out waiting for port ${port} to close.`);
}

async function waitForCdp(session, url, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (session.child.exitCode != null) {
      throw new Error(`Tauri exited before CDP was ready.\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
    }
    try {
      const resp = await fetch(`${url}/json/version`);
      if (resp.ok) return;
    } catch {
      await delay(250);
    }
  }
  throw new Error(`Timed out waiting for ${url}/json/version.\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
}

async function firstPage(context) {
  for (const page of context.pages()) {
    if (!page.isClosed()) return page;
  }
  return await context.waitForEvent("page", { timeout: 10_000 });
}

function killTree(pid) {
  if (!pid) return null;
  return spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { encoding: "utf8" });
}

function waitForExit(proc, timeoutMs) {
  if (proc.exitCode != null) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Timed out waiting for Tauri process exit")), timeoutMs);
    proc.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

async function ensureNoWorkspaceMilimProcesses() {
  const processes = workspaceMilimProcesses();
  for (const proc of processes) {
    killTree(proc.ProcessId);
  }

  const started = Date.now();
  while (Date.now() - started < 10_000) {
    if (workspaceMilimProcesses().length === 0) return;
    await delay(250);
  }

  throw new Error(`Workspace milim-desktop.exe process still running: ${describeWorkspaceMilimProcesses()}`);
}

function workspaceMilimProcesses() {
  const script = "Get-CimInstance Win32_Process -Filter \"Name = 'milim-desktop.exe'\" | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress";
  const result = spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout.trim()) return [];
  try {
    const parsed = JSON.parse(result.stdout);
    const rows = Array.isArray(parsed) ? parsed : [parsed];
    const binaryLower = binary.toLowerCase();
    const rootLower = root.toLowerCase();
    return rows.filter((row) => {
      const executable = String(row.ExecutablePath ?? "").toLowerCase();
      const command = String(row.CommandLine ?? "").toLowerCase();
      return (
        executable === binaryLower ||
        command.includes(binaryLower) ||
        executable.startsWith(rootLower) ||
        command.includes(rootLower)
      );
    });
  } catch {
    return [];
  }
}

function describeWorkspaceMilimProcesses() {
  const processes = workspaceMilimProcesses();
  if (!processes.length) return "none";
  return processes
    .map((proc) => `pid=${proc.ProcessId}; exe=${proc.ExecutablePath ?? ""}; cmd=${proc.CommandLine ?? ""}`)
    .join(" | ");
}

async function rmWithRetry(path, options = {}) {
  const attempts = options.attempts ?? 48;
  const delayMs = options.delayMs ?? 250;
  const label = options.label ?? path;
  for (let i = 0; i < attempts; i += 1) {
    try {
      rmSync(path, { recursive: true, force: true });
      return;
    } catch (err) {
      if (i === attempts - 1) {
        throw new Error(
          `Failed to remove ${label} after ${attempts} attempts.\n` +
            `path=${path}\n` +
            `lockedPath=${err.path ?? "unknown"}\n` +
            `code=${err.code ?? "unknown"}\n` +
            `message=${err.message}\n` +
            `workspaceProcesses=${describeWorkspaceMilimProcesses()}`,
        );
      }
      await delay(delayMs);
    }
  }
}

function printEvidencePaths(milimHome) {
  console.log(`milimHome=${milimHome}`);
  console.log(`avatarsScreenshot=${screenshots.avatars}`);
  console.log(`avatarsLightScreenshot=${screenshots.avatarsLight}`);
  console.log(`profilesScreenshot=${screenshots.profiles}`);
  console.log(`settingsScreenshot=${screenshots.settings}`);
  console.log(`settingsNarrowScreenshot=${screenshots.settingsNarrow}`);
  console.log(`chatScreenshot=${screenshots.chat}`);
  console.log(`zoomScreenshot=${screenshots.zoom}`);
  console.log(`accountUsageScreenshot=${screenshots.accountUsage}`);
  console.log(`microUiScreenshot=${screenshots.microUi}`);
  console.log(`inspectorOverlayScreenshot=${screenshots.inspectorOverlay}`);
  console.log(`workersPlanScreenshot=${screenshots.workersPlan}`);
  console.log(`workersNarrowScreenshot=${screenshots.workersNarrow}`);
  console.log(`newChatSplitScreenshot=${screenshots.newChatSplit}`);
  console.log(`mcpAppsLightScreenshot=${screenshots.mcpAppsLight}`);
  console.log(`mcpAppsDarkScreenshot=${screenshots.mcpAppsDark}`);
  console.log(`turnChangesScreenshot=${screenshots.turnChanges}`);
  for (const theme of ["light", "dark"]) {
    for (const kind of mcpAppKinds) console.log(`mcpApp${kind}Screenshot(${theme})=${mcpAppViewScreenshot(kind, theme)}`);
  }
  console.log(`failureScreenshot=${screenshots.failure}`);
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
