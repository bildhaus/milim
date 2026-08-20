import { spawn, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
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
const staticPreviewOnly = process.argv.includes("--static-preview-only");
const browserProfileOnly = process.argv.includes("--browser-profile-only");
const zoomOnly = process.argv.includes("--zoom-only");
const microUiOnly = process.argv.includes("--micro-ui-only");
const resizeHandlesOnly = process.argv.includes("--resize-handles-only");
const workersOnly = process.argv.includes("--workers-only");
const mcpAppsOnly = process.argv.includes("--mcp-apps-only");
const sidebarMotionOnly = process.argv.includes("--sidebar-motion-only");
const inboxSidebarOnly = process.argv.includes("--inbox-sidebar-only");
const newChatSplitOnly = process.argv.includes("--new-chat-split-only");
const commandPaletteOnly = process.argv.includes("--command-palette-only");
const settingsOnly = process.argv.includes("--settings-only");
const appMenuOnly = process.argv.includes("--app-menu-only");
const turnChangesOnly = process.argv.includes("--turn-changes-only");
const chatAffordancesOnly = process.argv.includes("--chat-affordances-only");
const reasoningEffortOnly = process.argv.includes("--reasoning-effort-only");
const generationControlsOnly = process.argv.includes("--generation-controls-only");
const mediaOnly = process.argv.includes("--media-only");
const linkedThreadDropOnly = process.argv.includes("--linked-thread-drop-only");
const mcpAppKinds = ["chart", "diagram", "form", "dashboard", "viewer"];
const screenshots = {
  avatars: join(tmpdir(), "milim-tauri-webview-agent-avatars.png"),
  avatarsLight: join(tmpdir(), "milim-tauri-webview-agent-avatars-light.png"),
  profiles: join(tmpdir(), "milim-tauri-webview-personalized-profiles.png"),
  settings: join(tmpdir(), "milim-tauri-webview-provider-settings.png"),
  settingsNarrow: join(tmpdir(), "milim-tauri-webview-settings-narrow.png"),
  settingsMinimum: join(tmpdir(), "milim-tauri-webview-settings-minimum.png"),
  settingsThemeEditor: join(tmpdir(), "milim-tauri-webview-settings-theme-editor.png"),
  chat: join(tmpdir(), "milim-tauri-webview-personalized-chat.png"),
  zoom: join(tmpdir(), "milim-tauri-webview-zoom-chip.png"),
  accountUsage: join(tmpdir(), "milim-tauri-webview-account-usage.png"),
  microUi: join(tmpdir(), "milim-tauri-webview-micro-ui.png"),
  resizeHandles: join(tmpdir(), "milim-tauri-webview-resize-handles.png"),
  inspectorOverlay: join(tmpdir(), "milim-tauri-webview-inspector-overlay.png"),
  workersPlan: join(tmpdir(), "milim-tauri-webview-workers-plan.png"),
  workersNarrow: join(tmpdir(), "milim-tauri-webview-workers-narrow.png"),
  newChatSplit: join(tmpdir(), "milim-tauri-webview-new-chat-split.png"),
  mcpAppsLight: join(tmpdir(), "milim-tauri-webview-mcp-apps-light.png"),
  mcpAppsDark: join(tmpdir(), "milim-tauri-webview-mcp-apps-dark.png"),
  nativeChartLight: join(tmpdir(), "milim-tauri-webview-native-chart-light.png"),
  nativeChartDark: join(tmpdir(), "milim-tauri-webview-native-chart-dark.png"),
  nativeChartNarrow: join(tmpdir(), "milim-tauri-webview-native-chart-narrow.png"),
  turnChanges: join(tmpdir(), "milim-tauri-webview-turn-changes.png"),
  inboxProjects: join(tmpdir(), "milim-tauri-webview-inbox-projects.png"),
  inboxSettings: join(tmpdir(), "milim-tauri-webview-inbox-settings.png"),
  threadBarTop: join(tmpdir(), "milim-tauri-webview-thread-bar-top.png"),
  threadBarTopNarrow: join(tmpdir(), "milim-tauri-webview-thread-bar-top-narrow.png"),
  threadBarBottom: join(tmpdir(), "milim-tauri-webview-thread-bar-bottom.png"),
  chatSources: join(tmpdir(), "milim-tauri-webview-chat-sources.png"),
  chatLatest: join(tmpdir(), "milim-tauri-webview-chat-latest.png"),
  inboxActive: join(tmpdir(), "milim-tauri-webview-inbox-active.png"),
  inboxSettled: join(tmpdir(), "milim-tauri-webview-inbox-settled.png"),
  workspaceCode: join(tmpdir(), "milim-tauri-webview-workspace-code.png"),
  reasoningEffort: join(tmpdir(), "milim-tauri-webview-reasoning-effort.png"),
  failure: join(tmpdir(), "milim-tauri-webview-failure.png"),
  linkedThreadDrop: join(tmpdir(), "milim-tauri-webview-linked-thread-drop.png"),
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
  if (mediaOnly) {
    const errors = collectErrors(session.page);
    await session.page.getByTestId("chat-shell").waitFor();
    await dismissOnboardingIfPresent(session.page);
    await runMediaStudioCheck(session.page);
    consoleErrors.push(
      ...errors.filter(
        (message) =>
          !(
            message.includes("/media/library") &&
            message.includes("500")
          ) &&
          !message.includes("/codex/models"),
      ),
    );
  } else if (linkedThreadDropOnly) {
    const errors = collectErrors(session.page);
    await runLinkedThreadDropCheck(session.page);
    await session.page.screenshot({ path: screenshots.linkedThreadDrop, fullPage: false });
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (turnChangesOnly) {
    const errors = collectErrors(session.page);
    turnChangesRepo = createTurnChangesRepo();
    await runTurnChangesCheck(session.page, turnChangesRepo);
    consoleErrors.push(...errors);
  } else if (chatAffordancesOnly) {
    const errors = collectErrors(session.page);
    await runChatAffordancesCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (reasoningEffortOnly) {
    const errors = collectErrors(session.page);
    await runReasoningEffortIsolationCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (generationControlsOnly) {
    const errors = collectErrors(session.page);
    await runGenerationControlsCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (browserProfileOnly) {
    const errors = collectErrors(session.page);
    await runBrowserProfileCheck(session);
    consoleErrors.push(...errors);
  } else if (nativePreviewOnly) {
    const errors = collectErrors(session.page);
    await runNativePreviewOcclusionCheck(session.page, session.child.pid);
    consoleErrors.push(...errors);
  } else if (staticPreviewOnly) {
    const errors = collectErrors(session.page);
    await runStaticWorkspacePreviewCheck(session.page, session.child.pid);
    consoleErrors.push(...errors.filter((message) => !message.includes("(about:srcdoc)")));
  } else if (sidebarMotionOnly || newChatSplitOnly) {
    const errors = collectErrors(session.page);
    await runSidebarSectionMotionCheck(session.page, newChatSplitOnly);
    consoleErrors.push(...errors);
  } else if (inboxSidebarOnly) {
    const errors = collectErrors(session.page);
    await runInboxSidebarCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
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
    consoleErrors.push(
      ...errors.filter(
        (message) =>
          !message.includes("/codex/models") &&
          !message.includes("net::ERR_CONNECTION_REFUSED"),
      ),
    );
  } else if (settingsOnly) {
    const errors = collectErrors(session.page);
    await runSettingsLayoutCheck(session.page);
    consoleErrors.push(...errors.filter((message) => !message.includes("/codex/models")));
  } else if (resizeHandlesOnly) {
    const errors = collectErrors(session.page);
    await runResizeHandleCheck(session.page);
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
    await runChatAffordancesCheck(session.page);
    await runHarnessHardeningUiCheck(session.page);
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

async function runMediaStudioCheck(page) {
  const fixtureProvider = {
    id: "e2e-media-provider",
    name: "E2E Media",
    kind: "fal",
    base_url: "https://queue.fal.run",
    enabled: true,
    has_key: true,
    models: [],
  };
  let providerAvailable = false;
  let generationIndex = 0;
  let deleteRequests = 0;
  const generationBodies = [];
  const svgData = (label, color) =>
    `data:image/svg+xml,${encodeURIComponent(
      `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 320 200"><rect width="320" height="200" fill="${color}"/><text x="160" y="105" text-anchor="middle" font-family="sans-serif" font-size="22" fill="white">${label}</text></svg>`,
    )}`;
  const makeLibraryItem = (id, label, color) => {
    const url = svgData(label, color);
    return {
      id,
      provider_run_id: `run-${id}`,
      created_at_ms: 1_700_000_000_000 + Number(id.split("-").pop() ?? 0),
      updated_at_ms: 1_700_000_000_000 + Number(id.split("-").pop() ?? 0),
      provider_id: fixtureProvider.id,
      provider: fixtureProvider.name,
      provider_kind: fixtureProvider.kind,
      kind: "image",
      model: "fixture/image-model",
      prompt: `${label} prompt`,
      input: { seed: id, aspect_ratio: "16:9" },
      status: "succeeded",
      save_state: "ready",
      privacy: { mode: "off", redacted: false, detections: 0, kinds: "" },
      urls: {},
      media: [{
        url,
        source_url: url,
        kind: "image",
        mime: "image/svg+xml",
        local_path: null,
      }],
    };
  };
  let libraryItems = [
    makeLibraryItem("library-1", "Fixture one", "#7c3aed"),
    makeLibraryItem("library-2", "Fixture two", "#0f766e"),
    makeLibraryItem("library-3", "Fixture three", "#c2410c"),
  ];
  let releaseFirstGeneration;
  const firstGenerationGate = new Promise((resolve) => {
    releaseFirstGeneration = resolve;
  });
  let resolveFirstImageSchemaSeen;
  const firstImageSchemaSeen = new Promise((resolve) => {
    resolveFirstImageSchemaSeen = resolve;
  });
  let releaseFirstImageSchema;
  const firstImageSchemaGate = new Promise((resolve) => {
    releaseFirstImageSchema = resolve;
  });
  let imageSchemaRequests = 0;

  await page.route("**/privacy/mode", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({ mode: "off" }),
  }));
  await page.route("**/providers", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({ providers: providerAvailable ? [fixtureProvider] : [] }),
  }));
  await page.route("**/providers/discover", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({ providers: [] }),
  }));
  await page.route("**/media/models?*", (route) => {
    const url = new URL(route.request().url());
    const kind = url.searchParams.get("kind") || "image";
    const label = kind[0].toUpperCase() + kind.slice(1);
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        models: [{
          id: `fixture/${kind}-model`,
          name: `Fixture ${label}`,
          description: `${label} fixture model`,
          output_modalities: [kind],
          supported_parameters: kind === "image" ? ["aspect_ratio"] : kind === "video" ? ["duration"] : ["tempo"],
          default_parameters: null,
          pricing: null,
        }],
      }),
    });
  });
  await page.route("**/media/model-schema?*", async (route) => {
    const url = new URL(route.request().url());
    const kind = url.searchParams.get("kind") || "image";
    if (kind === "image") {
      imageSchemaRequests += 1;
      if (imageSchemaRequests === 1) {
        resolveFirstImageSchemaSeen();
        await firstImageSchemaGate;
      }
    }
    const controls = kind === "image"
      ? [{
          key: "aspect_ratio",
          label: "Image aspect ratio",
          kind: "select",
          path: ["aspect_ratio"],
          options: [
            { label: "Square", value: "1:1" },
            { label: "Wide", value: "16:9" },
          ],
          default: "1:1",
        }]
      : kind === "video"
        ? [{
            key: "duration",
            label: "Video duration",
            kind: "number",
            path: ["duration"],
            min: 1,
            max: 12,
            default: 4,
          }]
        : [{
            key: "tempo",
            label: "Music tempo",
            kind: "number",
            path: ["tempo"],
            min: 40,
            max: 220,
            default: 120,
          }];
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        model: url.searchParams.get("model"),
        provider_id: fixtureProvider.id,
        provider: fixtureProvider.name,
        supported_parameters: controls.map((control) => control.key),
        controls,
      }),
    });
  });
  await page.route("**/media/generate", async (route) => {
    generationIndex += 1;
    const body = JSON.parse(route.request().postData() ?? "{}");
    generationBodies.push(body);
    if (generationIndex === 1) await firstGenerationGate;
    const media = ["#2563eb", "#7c3aed", "#db2777"].map((color, index) => ({
      url: svgData(`Run ${generationIndex} · ${index + 1}`, color),
      kind: "image",
      mime: "image/svg+xml",
    }));
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: `generation-${generationIndex}`,
        object: "media.generation",
        provider_id: fixtureProvider.id,
        provider: fixtureProvider.name,
        provider_kind: fixtureProvider.kind,
        kind: body.kind,
        model: body.model,
        status: "succeeded",
        output: null,
        media,
        urls: {},
        library_id: `generated-${generationIndex}`,
        save_state: "ready",
        privacy: { mode: "off", redacted: false, detections: 0, kinds: "" },
      }),
    });
  });
  await page.route("**/media/library?*", async (route) => {
    const url = new URL(route.request().url());
    const query = (url.searchParams.get("query") ?? "").trim().toLowerCase();
    if (query === "fixture library failure") {
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: "Fixture library unavailable" }),
      });
      return;
    }
    const kind = url.searchParams.get("kind");
    const provider = url.searchParams.get("provider");
    const status = url.searchParams.get("status");
    const filtered = libraryItems.filter((item) =>
      (!query || `${item.prompt} ${item.model}`.toLowerCase().includes(query)) &&
      (!kind || item.kind === kind) &&
      (!provider || item.provider_id === provider) &&
      (!status || item.save_state === status)
    );
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        items: filtered,
        next_cursor: query || kind || provider || status ? null : "more-fixtures",
      }),
    });
  });
  await page.route("**/media/library/*", async (route) => {
    const request = route.request();
    if (request.method() !== "DELETE") {
      await route.fulfill({
        status: 405,
        contentType: "application/json",
        body: JSON.stringify({ error: "Fixture supports delete only" }),
      });
      return;
    }
    deleteRequests += 1;
    const id = decodeURIComponent(new URL(request.url()).pathname.split("/").pop() ?? "");
    libraryItems = libraryItems.filter((item) => item.id !== id);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ deleted: true }),
    });
  });

  const tools = page.getByTestId("open-tools");
  if ((await tools.getAttribute("aria-expanded")) !== "true") await tools.click();
  await page.getByRole("button", { name: "Media", exact: true }).click();

  const studio = page.getByTestId("media-generator");
  await studio.waitFor();
  const libraryToggle = studio.getByRole("button", { name: /local library/i });
  await assertAttribute(libraryToggle, "aria-expanded", "true");
  const wideLayout = await studio.evaluate((element) => {
    const grid = element.querySelector(".media-grid");
    const library = element.querySelector(".media-library");
    const preview = element.querySelector(".media-stage-preview");
    const composer = element.querySelector(".media-create-pane");
    const previewBounds = preview?.getBoundingClientRect();
    const composerBounds = composer?.getBoundingClientRect();
    return {
      columns: grid ? getComputedStyle(grid).gridTemplateColumns : "",
      libraryPosition: library ? getComputedStyle(library).position : "",
      composerCoversPreview: Boolean(previewBounds && composerBounds && previewBounds.bottom > composerBounds.top),
      composerSideGap: previewBounds && composerBounds ? Math.abs(previewBounds.width - composerBounds.width) : null,
    };
  });
  if (
    wideLayout.columns.split(" ").length !== 2
    || wideLayout.libraryPosition !== "static"
    || wideLayout.composerCoversPreview
    || wideLayout.composerSideGap === null
    || wideLayout.composerSideGap > 1
  ) {
    throw new Error(`Wide Media Studio should render flexible Output and Library columns: ${JSON.stringify(wideLayout)}.`);
  }
  const placementControl = page.getByTestId("media-composer-placement");
  const sidePlacement = placementControl.getByRole("button", { name: "Side", exact: true });
  const bottomPlacement = placementControl.getByRole("button", { name: "Bottom", exact: true });
  await assertAttribute(bottomPlacement, "aria-pressed", "true");
  await sidePlacement.click();
  await page.waitForFunction(() => document.querySelector(".media-output-body")?.getAttribute("data-composer-placement") === "side");
  const sideLayout = await studio.evaluate((element) => {
    const preview = element.querySelector(".media-stage-preview")?.getBoundingClientRect();
    const composer = element.querySelector(".media-create-pane")?.getBoundingClientRect();
    return preview && composer
      ? { composerRight: composer.right, previewLeft: preview.left, composerTop: composer.top, previewTop: preview.top }
      : null;
  });
  if (!sideLayout || sideLayout.composerRight > sideLayout.previewLeft || Math.abs(sideLayout.composerTop - sideLayout.previewTop) < 1) {
    throw new Error(`Side composer placement should use a separate column and remain bottom-aligned: ${JSON.stringify(sideLayout)}.`);
  }
  await bottomPlacement.click();
  await page.waitForFunction(() => document.querySelector(".media-output-body")?.getAttribute("data-composer-placement") === "bottom");
  const initialWidth = await studio.evaluate((element) => element.style.width);
  await studio.evaluate((element) => { element.style.width = "800px"; });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('[data-testid="media-generator"] .media-library')).position === "absolute");
  await studio.evaluate((element) => { element.style.width = "680px"; });
  await page.waitForFunction(() => {
    const grid = document.querySelector('[data-testid="media-generator"] .media-grid');
    const stage = document.querySelector('[data-testid="media-generator"] .media-stage');
    const create = document.querySelector('[data-testid="media-generator"] .media-create-pane');
    return grid && stage && create && getComputedStyle(grid).gridTemplateColumns.split(" ").length === 1
      && getComputedStyle(stage).gridRowStart === "1"
      && getComputedStyle(create).position === "relative"
      && create.parentElement?.classList.contains("media-output-body")
      && create.previousElementSibling?.classList.contains("media-stage-preview");
  });
  await studio.evaluate((element, width) => { element.style.width = width; }, initialWidth);
  const prompt = page.getByTestId("media-prompt-input");
  const preservedPrompt = [
    "Preserve this provider setup draft",
    "with enough lines to grow",
    "across provider setup",
    "without collapsing",
  ].join("\n");
  await prompt.fill(preservedPrompt);
  await page.waitForFunction(() => {
    const input = document.querySelector('[data-testid="media-prompt-input"]');
    return input instanceof HTMLTextAreaElement
      && getComputedStyle(input).overflowY === "hidden"
      && input.scrollHeight <= input.clientHeight + 2;
  });

  const mediaSettingsSummary = page.getByTestId("inline-media-settings-summary");
  await mediaSettingsSummary.click();
  const mediaSettingsGeometry = await studio.evaluate((element) => {
    const preview = element.querySelector(".media-stage-preview")?.getBoundingClientRect();
    const composer = element.querySelector(".media-create-pane")?.getBoundingClientRect();
    const settings = element.querySelector(".media-inline-settings")?.getBoundingClientRect();
    return {
      settingsInsideComposer: Boolean(settings && composer && settings.top >= composer.top && settings.bottom <= composer.bottom),
      compactSettings: Boolean(settings && settings.height <= 64),
      composerCoversPreview: Boolean(preview && composer && preview.bottom > composer.top),
    };
  });
  if (!mediaSettingsGeometry.settingsInsideComposer || !mediaSettingsGeometry.compactSettings || mediaSettingsGeometry.composerCoversPreview) {
    throw new Error(`Media settings should expand inside the composer without covering Output: ${JSON.stringify(mediaSettingsGeometry)}.`);
  }
  const kindSelect = page.getByTestId("inline-media-kind-select");
  await kindSelect.waitFor();
  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.getByRole("listbox", { name: "Media type" }).waitFor();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await assertTextContains(kindSelect, "Video");

  const advanced = page.getByLabel("Advanced media input JSON");
  const advancedDraft = '{\n  "seed": 42\n}';
  await advanced.evaluate((element) => element.closest("details")?.setAttribute("open", ""));
  await advanced.fill(advancedDraft);

  const noProviderTrigger = page.getByTestId("media-model-picker-trigger");
  await assertAttribute(noProviderTrigger, "aria-label", "Add media provider");
  await assertTextContains(noProviderTrigger, "Add media provider");
  await noProviderTrigger.click();
  await page.getByTestId("provider-overview").waitFor();
  providerAvailable = true;
  await page.getByTestId("close-providers").click();

  await prompt.waitFor();
  if ((await prompt.inputValue()) !== preservedPrompt) {
    throw new Error("Media prompt draft should survive the in-place provider setup round trip.");
  }
  if (!await prompt.evaluate((element) => Number.parseFloat(element.style.height) > 58)) {
    throw new Error("Media prompt height should be restored after the provider setup round trip.");
  }
  await mediaSettingsSummary.click();
  await kindSelect.waitFor();
  await assertTextContains(kindSelect, "Video");
  await advanced.evaluate((element) => element.closest("details")?.setAttribute("open", ""));
  const advancedAfterProviderSetup = await advanced.inputValue();
  if (advancedAfterProviderSetup !== advancedDraft) {
    throw new Error(
      `Advanced media input should survive the in-place provider setup round trip. ` +
      `Expected ${JSON.stringify(advancedDraft)}, got ${JSON.stringify(advancedAfterProviderSetup)}.`,
    );
  }
  await advanced.evaluate((element) => element.closest("details")?.removeAttribute("open"));
  await page.getByLabel("Video duration").waitFor();
  await page.getByTestId("media-generate").waitFor();
  if (await page.getByTestId("media-generate").isDisabled()) {
    throw new Error("The refreshed provider and model should be usable after returning to Media Studio.");
  }
  const configuredModelTrigger = page.getByTestId("media-model-picker-trigger");
  await configuredModelTrigger.click();
  const mediaModelPicker = page.getByRole("dialog", { name: "Choose a video model", exact: true });
  await mediaModelPicker.waitFor();
  await mediaModelPicker.getByRole("button", { name: "Favorites only", exact: true }).focus();
  await page.keyboard.press("Tab");
  await page.waitForFunction(() =>
    Boolean(document.activeElement?.closest('[data-testid="media-generator"]'))
  );
  await mediaModelPicker.getByLabel("Search models").focus();
  await page.keyboard.press("Escape");
  await mediaModelPicker.waitFor({ state: "hidden" });
  await page.waitForFunction(() =>
    document.activeElement?.getAttribute("data-testid") === "media-model-picker-trigger"
  );

  if (await mediaSettingsSummary.getAttribute("aria-expanded") !== "true") await mediaSettingsSummary.click();
  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.getByRole("listbox", { name: "Media type" }).waitFor();
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  await Promise.race([
    firstImageSchemaSeen,
    delay(3_000).then(() => {
      throw new Error("Timed out waiting for the delayed image schema fixture.");
    }),
  ]);
  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.getByRole("listbox", { name: "Media type" }).waitFor();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await page.getByLabel("Video duration").waitFor();
  releaseFirstImageSchema();
  await delay(50);
  await assertHidden(page.getByLabel("Image aspect ratio"), "stale image schema control");

  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.getByRole("listbox", { name: "Media type" }).waitFor();
  await page.keyboard.press("Escape");
  await page.getByRole("listbox", { name: "Media type" }).waitFor({ state: "hidden" });
  await page.waitForFunction(() =>
    document.activeElement?.getAttribute("aria-label") === "Media type"
  );
  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Home");
  await page.keyboard.press("Enter");
  await page.getByLabel("Image aspect ratio").waitFor();

  const firstGenerate = page.getByTestId("media-generate");
  await prompt.fill("Generate three fixture variants");
  await firstGenerate.click();
  const generatingStatus = page.getByTestId("media-generation-progress");
  await generatingStatus.filter({ hasText: "Generating image" }).waitFor();
  await assertTextContains(generatingStatus, "Generate three fixture variants");
  await prompt.fill("Editing should not hide generation progress");
  if (!(await firstGenerate.isDisabled())) {
    throw new Error("Generation should stay disabled while the active request is in flight.");
  }
  await kindSelect.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await generatingStatus.filter({ hasText: "Generating image" }).waitFor();
  await assertTextContains(generatingStatus, "Generate three fixture variants");
  if ((await generatingStatus.innerText()).includes("Editing should not hide generation progress")) {
    throw new Error("The generating canvas should describe the submitted prompt, not the edited composer draft.");
  }
  releaseFirstGeneration();

  const variants = page.getByRole("listbox", { name: "Output variants" });
  await variants.waitFor();
  await assertHidden(generatingStatus, "completed generation placeholder");
  const variantOptions = variants.getByRole("option");
  if ((await variantOptions.count()) !== 3) {
    throw new Error(`Expected 3 output variants, got ${await variantOptions.count()}.`);
  }
  await assertAttribute(variantOptions.first(), "aria-selected", "true");
  await variantOptions.first().focus();
  await page.keyboard.press("End");
  await assertAttribute(variantOptions.last(), "aria-selected", "true");

  const secondResponse = page.waitForResponse((response) =>
    response.request().method() === "POST" &&
    new URL(response.url()).pathname === "/media/generate" &&
    response.status() === 200
  );
  await prompt.fill("Generate a fresh variant set");
  await firstGenerate.click();
  await secondResponse;
  await page.waitForFunction(() =>
    document.querySelector('[data-testid="media-variant"]')?.getAttribute("aria-selected") === "true"
  );

  const libraryCountText = await libraryToggle.innerText();
  const libraryCountLabel = await libraryToggle.getAttribute("aria-label");
  if (!/\d+\+/.test(libraryCountText) || !/\d+ loaded, more available/.test(libraryCountLabel ?? "")) {
    throw new Error(
      `Expected a plus-suffixed loaded count with an accurate accessible description, ` +
      `got text=${JSON.stringify(libraryCountText)} aria-label=${JSON.stringify(libraryCountLabel)}.`,
    );
  }
  if ((await libraryToggle.getAttribute("aria-expanded")) !== "true") await libraryToggle.click();
  const library = page.getByRole("complementary", { name: "Local library", exact: true });
  await library.waitFor();

  const cards = page.getByTestId("media-library-item");
  if ((await cards.count()) !== 3) {
    throw new Error(`Expected 3 loaded library cards, got ${await cards.count()}.`);
  }
  const firstCardContract = await cards.first().evaluate((element) => ({
    tag: element.tagName,
    nestedInteractive: element.querySelectorAll("button, a, input, textarea, select, audio, [tabindex]").length,
  }));
  if (firstCardContract.tag !== "BUTTON" || firstCardContract.nestedInteractive !== 0) {
    throw new Error(`Each library card should be one button with no nested focus targets: ${JSON.stringify(firstCardContract)}.`);
  }
  await cards.first().focus();
  await page.keyboard.press("Enter");
  await assertAttribute(cards.first(), "aria-current", "true");
  await page.getByRole("button", { name: "Use settings", exact: true }).click();
  await assertTextContains(page.getByLabel("Image aspect ratio"), "Wide");
  await advanced.evaluate((element) => element.closest("details")?.setAttribute("open", ""));
  const reusedAdvanced = JSON.parse(await advanced.inputValue());
  if (reusedAdvanced.aspect_ratio !== undefined || reusedAdvanced.seed !== "library-1") {
    throw new Error(`Use settings should split schema controls from Advanced input: ${JSON.stringify(reusedAdvanced)}.`);
  }

  const librarySearch = page.getByLabel("Search media library");
  await librarySearch.fill("Fixture one");
  const clearFilters = page.getByRole("button", { name: "Clear filters" });
  await clearFilters.waitFor();
  await clearFilters.click();
  if ((await librarySearch.inputValue()) !== "") {
    throw new Error("Clear filters should reset the library search query.");
  }
  await librarySearch.fill("fixture library failure");
  const libraryAlert = library.getByRole("alert");
  await libraryAlert.filter({ hasText: "Fixture library unavailable" }).waitFor();
  await assertHidden(page.getByTestId("media-generation-error"), "generation error during a library-list failure");
  await clearFilters.click();
  await cards.first().waitFor();

  const beforeNonPromptShortcuts = generationBodies.length;
  await librarySearch.focus();
  await page.keyboard.press("Control+Enter");
  await delay(75);
  await advanced.evaluate((element) => element.closest("details")?.setAttribute("open", ""));
  await advanced.focus();
  await page.keyboard.press("Control+Enter");
  await delay(75);
  await advanced.evaluate((element) => element.closest("details")?.removeAttribute("open"));
  if (generationBodies.length !== beforeNonPromptShortcuts) {
    throw new Error("Ctrl+Enter outside the media prompt should not submit a generation.");
  }

  await prompt.fill("IME should not submit");
  await prompt.dispatchEvent("keydown", {
    key: "Enter",
    code: "Enter",
    ctrlKey: true,
    isComposing: true,
  });
  await delay(75);
  if (generationBodies.length !== beforeNonPromptShortcuts) {
    throw new Error("Ctrl+Enter while composing text should not submit a generation.");
  }
  if ((await prompt.getAttribute("dir")) !== "auto") {
    throw new Error("The media prompt should infer text direction.");
  }
  await prompt.fill("line one\nline two\nline three\nline four\nline five\nline six");
  await page.waitForFunction(() => {
    const input = document.querySelector('[data-testid="media-prompt-input"]');
    return input instanceof HTMLTextAreaElement && Number.parseFloat(input.style.height) > 58;
  });

  const promptShortcutResponse = page.waitForResponse((response) =>
    response.request().method() === "POST" &&
    new URL(response.url()).pathname === "/media/generate" &&
    response.status() === 200
  );
  await prompt.press("Control+Enter");
  await promptShortcutResponse;
  if (generationBodies.length !== beforeNonPromptShortcuts + 1) {
    throw new Error("Ctrl+Enter in the media prompt should submit exactly one generation.");
  }

  await cards.first().click();
  const deleteButton = page.getByRole("button", { name: "Delete", exact: true });
  await deleteButton.click();
  await page.getByRole("button", { name: "Confirm delete", exact: true }).waitFor();
  await page.getByRole("status").filter({ hasText: "Delete again within 3 seconds" }).waitFor();
  if (deleteRequests !== 0) throw new Error("The first Delete click must not issue a request.");
  await deleteButton.waitFor({ timeout: 4_000 });
  if (deleteRequests !== 0) throw new Error("An expired delete confirmation must not issue a request.");

  await deleteButton.click();
  const deleteResponse = page.waitForResponse((response) =>
    response.request().method() === "DELETE" &&
    /\/media\/library\/library-\d+$/.test(new URL(response.url()).pathname) &&
    response.status() === 200
  );
  await page.getByRole("button", { name: "Confirm delete", exact: true }).click();
  await deleteResponse;
  if (deleteRequests !== 1) throw new Error(`Expected one permanent-delete request, got ${deleteRequests}.`);
  await page.getByRole("status").filter({ hasText: "Deleted from local library" }).waitFor();
  await page.waitForFunction(() =>
    document.activeElement?.matches('[data-testid="media-library-item"][aria-current="true"]')
  );

  for (const expectedDeleteCount of [2, 3]) {
    await page.getByRole("button", { name: "Delete", exact: true }).click();
    const nextDeleteResponse = page.waitForResponse((response) =>
      response.request().method() === "DELETE" &&
      /\/media\/library\/library-\d+$/.test(new URL(response.url()).pathname) &&
      response.status() === 200
    );
    await page.getByRole("button", { name: "Confirm delete", exact: true }).click();
    await nextDeleteResponse;
    if (deleteRequests !== expectedDeleteCount) {
      throw new Error(`Expected ${expectedDeleteCount} permanent-delete requests, got ${deleteRequests}.`);
    }
  }
  await page.waitForFunction(() =>
    document.activeElement?.getAttribute("aria-controls") === "media-library-sidebar"
  );
}

function runGit(folder, args) {
  const result = spawnSync("git", args, { cwd: folder, encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`Git fixture command failed: git ${args.join(" ")}\n${result.stderr}`);
  }
}

async function runTurnChangesCheck(page, fixture) {
  const diffActionRequests = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().includes("/workspace/git/action")) {
      const body = JSON.parse(request.postData() ?? "{}");
      if (body.action === "diff") diffActionRequests.push(body);
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
  let failNextDiff = true;
  await page.route("**/workspace/git/action", async (route) => {
    const body = route.request().postDataJSON();
    if (failNextDiff && body?.action === "diff") {
      failNextDiff = false;
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          message: "simulated unavailable diff",
          stdout: "",
          stderr: "",
        }),
      });
      return;
    }
    await route.continue();
  });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);

  const card = page.getByTestId("turn-changes-card");
  await card.waitFor();
  if (await page.getByTestId("turn-changes-card").count() !== 1) {
    throw new Error("Only the latest assistant response should show a turn changes card.");
  }
  await card.getByText("Change review unavailable", { exact: true }).waitFor();
  await card.getByTestId("turn-changes-open-git").click();
  const unavailableGitPanel = page.getByTestId("git-workspace-panel");
  await unavailableGitPanel.waitFor();
  await page.getByRole("button", { name: "Close Git panel" }).click();
  await unavailableGitPanel.waitFor({ state: "hidden" });
  await card.getByTestId("turn-changes-retry").click();
  await card.getByText("Changed 5 files", { exact: true }).waitFor();
  await assertTextContains(card, "Changed 5 files");
  await assertTextContains(card, "+5");
  await assertTextContains(card, "-0");
  await assertTextContains(card, "file-1.txt");
  await assertHidden(card.getByText("file-4.txt", { exact: true }), "fourth changed path before expansion");
  await page.screenshot({ path: screenshots.turnChanges, fullPage: false });
  await card.getByTestId("turn-changes-toggle").click();
  await card.getByText("file-4.txt", { exact: true }).waitFor();

  const requestsBeforeReview = diffActionRequests.length;
  await card.getByTestId("turn-changes-review").click();
  const gitPanel = page.getByTestId("git-workspace-panel");
  await gitPanel.waitFor();
  await assertTextContains(gitPanel.getByLabel("Diff scope"), "Last turn");
  await assertTextContains(gitPanel.getByLabel("Changed files", { exact: true }), "file-5.txt");
  await page.waitForTimeout(300);
  if (diffActionRequests.length !== requestsBeforeReview) {
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
  await dismissOnboardingIfPresent(page);
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
  await runTextSelectionPolicyCheck(page);
  await runAppMenuCheck(page);
  await runNativePreviewOcclusionCheck(page, pid);
  await runStaticWorkspacePreviewCheck(page, pid);
  await assertAgentOptions(page);
  await openSettings(page);
  await assertAppShortcutsPersisted(page);
  await closeSettings(page);
  await runModelPickerSurfaceCheck(page);
  await runAppShortcutCheck(page);

  await dismissOnboardingIfPresent(page);
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

async function runInboxSidebarCheck(page) {
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
  const fixture = await page.evaluate(async ({ projectA, projectB, projectC }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const now = Date.now();
    const sessions = [
      {
        id: "inbox-active-new",
        title: "Cross-project newest",
        messages: [{
          id: "topbar-usage-fixture",
          role: "assistant",
          content: "Ready",
          usage: { prompt_tokens: 1_200, completion_tokens: 300, total_tokens: 1_500 },
        }],
        settings: { folder: projectA, model: "codex:gpt-5.4" },
        createdAt: now,
        updatedAt: now,
      },
      {
        id: "inbox-branch",
        title: "Flattened branch",
        messages: [],
        settings: { folder: projectA },
        parentId: "inbox-active-new",
        threadWorkspace: {
          mode: "worktree",
          projectFolder: projectA,
          branch: "e2e-branch",
        },
        createdAt: now,
        updatedAt: now - 10_000,
      },
      {
        id: "inbox-active-old",
        title: "Cross-project older",
        messages: [],
        settings: { folder: projectB },
        createdAt: now,
        updatedAt: now - 60_000,
      },
      {
        id: "inbox-settled-a",
        title: "Settled Alpha",
        messages: [],
        settings: { folder: projectB },
        settledAt: now - 1_000,
        createdAt: now,
        updatedAt: now - 120_000,
      },
      {
        id: "inbox-settled-b",
        title: "Settled Beta",
        messages: [],
        settings: { folder: "" },
        settledAt: now - 2_000,
        createdAt: now,
        updatedAt: now - 180_000,
      },
      {
        id: "thread-bar-drag-one",
        title: "Drag thread one",
        messages: [],
        settings: { folder: projectC },
        createdAt: now,
        updatedAt: now - 240_000,
      },
      {
        id: "thread-bar-drag-two",
        title: "Drag thread two",
        messages: [],
        settings: { folder: projectC },
        createdAt: now,
        updatedAt: now - 250_000,
      },
    ];
    await invoke("user_state_set", {
      key: "milim.sessions",
      value: JSON.stringify({
        state: {
          sessions,
          projects: [
            {
              id: `project:${projectA}`,
              name: "Workspace A",
              folder: projectA,
              icon: "terminal",
              color: "#22aa88",
              createdAt: now,
              updatedAt: now,
            },
            {
              id: `project:${projectB}`,
              name: "Workspace B",
              folder: projectB,
              icon: "code",
              color: "#aa6622",
              createdAt: now,
              updatedAt: now,
            },
            {
              id: `project:${projectC}`,
              name: "Workspace C",
              folder: projectC,
              icon: "folder",
              color: "#7c6be8",
              createdAt: now,
              updatedAt: now,
            },
          ],
          activeId: "inbox-active-new",
          unreadSessionIds: [],
          sidebar: {
            collapsedSectionIds: [],
            pinnedSessionIds: ["inbox-active-old"],
            pinnedSectionIds: [],
            sessionOrder: sessions.map((session) => session.id),
            sectionOrder: [],
            projectFolders: [projectA, projectB, projectC],
          },
        },
        version: 0,
      }),
    });
    await invoke("user_state_set", {
      key: "milim.ui",
      value: JSON.stringify({
        state: { settledThreadsEnabled: false, sidebarOpen: true, sidebarWidth: 236, threadNavigationPlacement: "sidebar", showAccountUsageInTitleBar: true },
        version: 0,
      }),
    });
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
    return { sessionIds: sessions.map((session) => session.id) };
  }, { projectA: root, projectB: join(root, "src"), projectC: join(root, "docs") });

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.getByRole("complementary", { name: "Chats" }).waitFor();
  await page.getByRole("button", { name: "Collapse Workspace A", exact: true }).waitFor();
  await page.getByRole("button", { name: "Collapse Workspace B", exact: true }).waitFor();
  await page.screenshot({ path: screenshots.inboxProjects, fullPage: false });

  await openSettings(page);
  await page.getByTestId("settings-section-app").click();
  const projectsChoice = page.getByTestId("sidebar-organization-projects");
  const inboxChoice = page.getByTestId("sidebar-organization-inbox");
  await assertAttribute(projectsChoice, "aria-checked", "true");

  await page.getByTestId("thread-navigation-placement-top").click();
  if (await page.getByTestId("general-sidebar-open-toggle").count()) {
    throw new Error("Horizontal placement should hide sidebar-only settings.");
  }
  await closeSettings(page);

  const topBar = page.getByTestId("thread-bar");
  await topBar.waitFor();
  if (await page.locator(".sidebar").count()) {
    throw new Error("Top placement should replace the sidebar instead of rendering both rails.");
  }
  const topProject = topBar.getByRole("button", { name: /Workspace A/ }).first();
  await topProject.waitFor();
  const topFlow = await page.evaluate(() => {
    const titlebar = document.querySelector(".topbar")?.getBoundingClientRect();
    const bar = document.querySelector(".thread-bar")?.getBoundingClientRect();
    const project = document.querySelector(".thread-bar-project");
    const projectStyle = project ? getComputedStyle(project) : null;
    return {
      nested: Boolean(document.querySelector(".topbar > .topbar-thread-navigation > .thread-bar-top")),
      titlebarTop: titlebar?.top,
      titlebarBottom: titlebar?.bottom,
      barTop: bar?.top,
      barBottom: bar?.bottom,
      leftWidth: document.querySelector(".topbar-left")?.getBoundingClientRect().width,
      threadWidth: document.querySelector(".topbar-thread")?.getBoundingClientRect().width,
      accountUsageClipped: (() => {
        const usage = document.querySelector(".topbar-account-usage");
        return usage instanceof HTMLElement ? usage.clientWidth < usage.scrollWidth : false;
      })(),
      projectPaddingLeft: projectStyle ? Number.parseFloat(projectStyle.paddingLeft) : 0,
      projectPaddingRight: projectStyle ? Number.parseFloat(projectStyle.paddingRight) : 0,
    };
  });
  if (
    !topFlow.nested ||
    topFlow.titlebarTop == null ||
    topFlow.titlebarBottom == null ||
    topFlow.barTop == null ||
    topFlow.barBottom == null ||
    topFlow.barTop < topFlow.titlebarTop ||
    topFlow.barBottom > topFlow.titlebarBottom ||
    topFlow.leftWidth == null ||
    topFlow.leftWidth < 420 ||
    topFlow.threadWidth == null ||
    topFlow.threadWidth < 100 ||
    topFlow.accountUsageClipped ||
    topFlow.projectPaddingLeft < 14 ||
    topFlow.projectPaddingRight < 14
  ) {
    throw new Error(`Top thread navigation should share the native title bar: ${JSON.stringify(topFlow)}.`);
  }
  const topViewport = await page.evaluate(() => ({ width: window.innerWidth, height: window.innerHeight }));
  await page.setViewportSize({ width: 800, height: 720 });
  const narrowTopFlow = await page.evaluate(() => {
    const title = document.querySelector(".topbar-thread");
    const accountUsage = document.querySelector(".topbar-account-usage");
    const threadUsage = document.querySelector(".topbar-usage");
    return {
      titleWidth: title?.getBoundingClientRect().width,
      accountUsageClipped: accountUsage instanceof HTMLElement
        ? accountUsage.clientWidth < accountUsage.scrollWidth
        : true,
      threadUsageForcedHidden: threadUsage
        ? getComputedStyle(threadUsage).display === "none"
        : false,
    };
  });
  if (
    narrowTopFlow.titleWidth == null ||
    narrowTopFlow.titleWidth < 60 ||
    narrowTopFlow.accountUsageClipped ||
    narrowTopFlow.threadUsageForcedHidden
  ) {
    throw new Error(`Narrow top thread navigation should preserve the title, visible thread spend, and account quota: ${JSON.stringify(narrowTopFlow)}.`);
  }
  await page.screenshot({ path: screenshots.threadBarTopNarrow, fullPage: false });
  await page.setViewportSize(topViewport);
  await topProject.click();
  const topDropdown = page.getByRole("dialog", { name: "Workspace A threads" });
  await topDropdown.waitFor();
  const topDropdownContract = await topDropdown.evaluate((element) => {
    const trigger = document.querySelector('.thread-bar-project[aria-expanded="true"]')?.getBoundingClientRect();
    const dropdown = element.getBoundingClientRect();
    const list = element.querySelector(".thread-bar-popover-list");
    return {
      below: Boolean(trigger && dropdown.top >= trigger.bottom),
      overflowY: list ? getComputedStyle(list).overflowY : "",
      rows: element.querySelectorAll("[data-sidebar-session-id]").length,
    };
  });
  if (!topDropdownContract.below || topDropdownContract.overflowY !== "auto" || topDropdownContract.rows !== 2) {
    throw new Error(`Top project dropdown contract failed: ${JSON.stringify(topDropdownContract)}.`);
  }
  await page.keyboard.press("Escape");

  const topProjectC = topBar.getByRole("button", { name: /Workspace C/ }).first();
  await dragLocator(page, topProjectC, topProject, 0.08, 0.5);
  await page.waitForFunction(() => {
    const labels = [...document.querySelectorAll(".thread-bar-project-label")].map((element) => element.textContent);
    return labels.indexOf("Workspace C") < labels.indexOf("Workspace A");
  });
  await page.waitForTimeout(140);
  await topProjectC.click();
  const projectCDropdown = page.getByRole("dialog", { name: "Workspace C threads" });
  const dragOne = projectCDropdown.locator('[data-sidebar-session-id="thread-bar-drag-one"]');
  const dragTwo = projectCDropdown.locator('[data-sidebar-session-id="thread-bar-drag-two"]');
  await dragLocator(page, dragTwo, dragOne, 0.5, 0.08);
  await page.waitForFunction(() => {
    const ids = [...document.querySelectorAll('[aria-label="Workspace C threads"] [data-sidebar-session-id]')]
      .map((element) => element.getAttribute("data-sidebar-session-id"));
    return ids.indexOf("thread-bar-drag-two") < ids.indexOf("thread-bar-drag-one");
  });
  await page.waitForTimeout(250);
  const persistedDragOrder = await page.evaluate(async () => {
    const raw = await window.__TAURI_INTERNALS__.invoke("user_state_get", { key: "milim.sessions" });
    const state = raw ? JSON.parse(raw).state : {};
    return {
      sectionOrder: state.sidebar?.sectionOrder ?? [],
      sessionOrder: state.sidebar?.sessionOrder ?? [],
    };
  });
  const projectAId = `project:${root}`;
  const projectCId = `project:${join(root, "docs")}`;
  if (
    persistedDragOrder.sectionOrder.indexOf(projectCId) > persistedDragOrder.sectionOrder.indexOf(projectAId) ||
    persistedDragOrder.sessionOrder.indexOf("thread-bar-drag-two") > persistedDragOrder.sessionOrder.indexOf("thread-bar-drag-one")
  ) {
    throw new Error(`Horizontal drag order did not persist: ${JSON.stringify(persistedDragOrder)}.`);
  }
  await dragLocator(page, dragOne, topProject, 0.5, 0.5);
  await dragOne.waitFor({ state: "detached" });
  await page.waitForTimeout(250);
  const movedFolder = await page.evaluate(async (sessionId) => {
    const raw = await window.__TAURI_INTERNALS__.invoke("user_state_get", { key: "milim.sessions" });
    return raw ? JSON.parse(raw).state?.sessions?.find((session) => session.id === sessionId)?.settings?.folder : undefined;
  }, "thread-bar-drag-one");
  if (movedFolder !== root) {
    throw new Error(`Cross-project thread drop should use the existing folder move action, got ${String(movedFolder)}.`);
  }
  await page.evaluate(async ({ projectC, disposableIds }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const raw = await invoke("user_state_get", { key: "milim.sessions" });
    const parsed = raw ? JSON.parse(raw) : { state: {} };
    const state = parsed.state ?? {};
    state.sessions = (state.sessions ?? []).filter((session) => !disposableIds.includes(session.id));
    state.projects = (state.projects ?? []).filter((project) => project.folder !== projectC);
    if (state.sidebar) {
      state.sidebar.sessionOrder = (state.sidebar.sessionOrder ?? []).filter((id) => !disposableIds.includes(id));
      state.sidebar.sectionOrder = (state.sidebar.sectionOrder ?? []).filter((id) => id !== `project:${projectC}`);
      state.sidebar.projectFolders = (state.sidebar.projectFolders ?? []).filter((folder) => folder !== projectC);
    }
    await invoke("user_state_set", { key: "milim.sessions", value: JSON.stringify(parsed) });
  }, { projectC: join(root, "docs"), disposableIds: ["thread-bar-drag-one", "thread-bar-drag-two"] });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.locator(".thread-bar-top").waitFor();
  await topBar.getByRole("button", { name: "Search chats" }).click();
  await page.getByTestId("command-palette-input").waitFor();
  await closeChatSearch(page);
  await topBar.getByTestId("open-tools").click();
  await page.getByRole("menu", { name: "Tools" }).getByText("MCP Servers", { exact: true }).waitFor();
  await page.keyboard.press("Escape");
  await page.getByTestId("app-menu-trigger").click();
  const horizontalAppMenu = page.getByRole("menu", { name: "Milim menu" });
  if (await horizontalAppMenu.getByText(/sidebar/i).count()) {
    throw new Error("Horizontal placement should omit the app-menu sidebar toggle.");
  }
  await page.keyboard.press("Escape");
  await page.keyboard.press("Control+B");
  await topBar.waitFor();
  await page.screenshot({ path: screenshots.threadBarTop, fullPage: false });

  await openSettings(page);
  await page.getByTestId("settings-section-app").click();
  await page.getByTestId("thread-navigation-placement-bottom").click();
  await closeSettings(page);
  const bottomBar = page.getByTestId("thread-bar");
  await bottomBar.waitFor();
  const bottomFlow = await page.evaluate(() => {
    const bar = document.querySelector(".thread-bar")?.getBoundingClientRect();
    const chat = document.querySelector('[data-testid="chat-shell"]')?.getBoundingClientRect();
    return { barTop: bar?.top, chatBottom: chat?.bottom };
  });
  if (bottomFlow.barTop == null || bottomFlow.chatBottom == null || bottomFlow.barTop + 1 < bottomFlow.chatBottom) {
    throw new Error(`Bottom thread bar should sit below chat in document flow: ${JSON.stringify(bottomFlow)}.`);
  }
  const bottomProject = bottomBar.getByRole("button", { name: /Workspace A/ }).first();
  await bottomProject.click();
  const bottomDropdown = page.getByRole("dialog", { name: "Workspace A threads" });
  await bottomDropdown.waitFor();
  const bottomDropdownAbove = await bottomDropdown.evaluate((element) => {
    const trigger = document.querySelector('.thread-bar-project[aria-expanded="true"]')?.getBoundingClientRect();
    return Boolean(trigger && element.getBoundingClientRect().bottom <= trigger.top);
  });
  if (!bottomDropdownAbove) throw new Error("Bottom project dropdown should open upward.");
  await page.keyboard.press("Escape");
  const bottomToolsTrigger = bottomBar.getByTestId("open-tools");
  await bottomToolsTrigger.click();
  const bottomToolsMenu = page.getByRole("menu", { name: "Tools" });
  await bottomToolsMenu.waitFor();
  const bottomToolsGap = await bottomToolsMenu.evaluate((element) => {
    const trigger = document.querySelector('.thread-bar-bottom [data-testid="open-tools"]')?.getBoundingClientRect();
    return trigger ? trigger.top - element.getBoundingClientRect().bottom : null;
  });
  if (bottomToolsGap == null || bottomToolsGap < 0 || bottomToolsGap > 8) {
    throw new Error(`Bottom Tools menu should hug the rail: ${String(bottomToolsGap)}px gap.`);
  }
  await page.screenshot({ path: screenshots.threadBarBottom, fullPage: false });
  await page.keyboard.press("Escape");

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.locator(".thread-bar-bottom").waitFor();
  await openSettings(page);
  await page.getByTestId("settings-section-app").click();
  await assertAttribute(page.getByTestId("thread-navigation-placement-bottom"), "aria-checked", "true");
  await page.getByTestId("thread-navigation-placement-sidebar").click();
  await inboxChoice.click();
  await assertAttribute(inboxChoice, "aria-checked", "true");
  await page.screenshot({ path: screenshots.inboxSettings, fullPage: false });
  await closeSettings(page);

  await page.getByRole("complementary", { name: "Thread inbox" }).waitFor();
  const restoredSidebarWidth = await page.locator(".sidebar").evaluate((element) => Math.round(element.getBoundingClientRect().width));
  if (restoredSidebarWidth !== 236) {
    throw new Error(`Returning to Sidebar should restore its saved width, got ${restoredSidebarWidth}.`);
  }
  if (await page.locator(".sidebar .session-section-title").count()) {
    throw new Error("Inbox mode should not render project section headers.");
  }
  const pinnedIds = await page
    .locator('[data-sidebar-section-id="pinned"] [data-sidebar-session-id]')
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-sidebar-session-id")));
  if (JSON.stringify(pinnedIds) !== JSON.stringify(["inbox-active-old"])) {
    throw new Error(`Inbox pinned placement is wrong: ${JSON.stringify(pinnedIds)}.`);
  }
  const activeIds = await page
    .locator('[data-sidebar-section-id="inbox"] [data-sidebar-session-id]')
    .evaluateAll((rows) => rows.map((row) => row.getAttribute("data-sidebar-session-id")));
  if (JSON.stringify(activeIds) !== JSON.stringify(["inbox-active-new", "inbox-branch"])) {
    throw new Error(`Inbox activity order is wrong: ${JSON.stringify(activeIds)}.`);
  }
  await page
    .locator('[data-sidebar-session-id="inbox-branch"] .inbox-session-branch')
    .getByText("e2e-branch", { exact: true })
    .waitFor();
  await page
    .locator('[data-sidebar-session-id="inbox-active-new"] .inbox-session-project')
    .getByText("Workspace A", { exact: true })
    .waitFor();
  const projectMetadataColor = await page
    .locator('[data-sidebar-session-id="inbox-active-new"] .inbox-session-project')
    .evaluate((element) => getComputedStyle(element).color);
  if (projectMetadataColor !== "rgb(34, 170, 136)") {
    throw new Error(`Inbox project metadata should retain the custom project color, got ${projectMetadataColor}.`);
  }
  if (await page.locator(".inbox-project-badge").count()) {
    throw new Error("Inbox project metadata should not use badge chrome.");
  }

  const settledToggle = page.getByRole("button", { name: /^Settled/ });
  await assertAttribute(settledToggle, "aria-expanded", "false");
  if (await page.locator("#sidebar-settled-list").count()) {
    throw new Error("Settled should start collapsed in Inbox mode.");
  }
  const search = page.getByTestId("sidebar-search");
  await search.fill("Settled Alpha");
  await assertAttribute(settledToggle, "aria-expanded", "true");
  await page.getByText("Settled Alpha", { exact: true }).waitFor();
  await search.fill("");
  await page.locator("#sidebar-settled-list").waitFor({ state: "detached" });
  await page.screenshot({ path: screenshots.inboxActive, fullPage: false });

  const newestActive = page.locator('[data-sidebar-session-id="inbox-active-new"]');
  await newestActive.locator(".session-side").hover();
  await page.waitForTimeout(150);
  if (await newestActive.getByRole("button", { name: "Archive Cross-project newest", exact: true }).count()) {
    throw new Error("Active Inbox threads should offer Settle instead of Archive.");
  }
  await newestActive.click({ button: "right" });
  const activeMenu = page.getByTestId("app-context-menu");
  await activeMenu.getByText("Settle chat", { exact: true }).waitFor();
  if (await activeMenu.getByText("Archive chat", { exact: true }).count()) {
    throw new Error("Active Inbox thread menus should omit Archive.");
  }
  await page.keyboard.press("Escape");

  for (const [id, title, nextId] of [
    ["inbox-active-new", "Cross-project newest", "inbox-branch"],
    ["inbox-branch", "Flattened branch", "inbox-active-old"],
  ]) {
    const row = page.locator(`[data-sidebar-session-id="${id}"]`);
    await row.locator(".session-side").hover();
    await page.waitForTimeout(150);
    await row.getByRole("button", { name: `Settle ${title}`, exact: true }).click();
    await page.locator(`[data-sidebar-session-id="${nextId}"].active`).waitFor();
  }

  const finalActive = page.locator('[data-sidebar-session-id="inbox-active-old"]');
  await finalActive.locator(".session-side").hover();
  await page.waitForTimeout(150);
  await finalActive
    .getByRole("button", { name: "Settle Cross-project older", exact: true })
    .click();
  await page
    .locator('[data-sidebar-section-id="settled"] [data-sidebar-session-id="inbox-active-old"].active')
    .waitFor();
  await assertAttribute(settledToggle, "aria-expanded", "true");
  if (
    await page
      .locator('[data-sidebar-session-id="inbox-settled-b"] .inbox-session-project')
      .count()
  ) {
    throw new Error("Loose Inbox threads should omit synthetic project metadata.");
  }
  await page.screenshot({ path: screenshots.inboxSettled, fullPage: false });

  await finalActive.locator(".session-side").hover();
  await page.waitForTimeout(150);
  await finalActive
    .getByRole("button", { name: "Archive Cross-project older", exact: true })
    .waitFor();
  await finalActive
    .getByRole("button", { name: "Unsettle Cross-project older", exact: true })
    .click();
  await page
    .locator('[data-sidebar-section-id="pinned"] [data-sidebar-session-id="inbox-active-old"].active')
    .waitFor();

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.getByRole("complementary", { name: "Thread inbox" }).waitFor();

  await openSettings(page);
  await page.getByTestId("settings-section-app").click();
  await assertAttribute(page.getByTestId("sidebar-organization-inbox"), "aria-checked", "true");
  await page.getByTestId("sidebar-organization-projects").click();
  await closeSettings(page);
  await page.getByRole("complementary", { name: "Chats" }).waitFor();
  await page.getByRole("button", { name: "Collapse Workspace A", exact: true }).waitFor();
  await page.locator(".sidebar").getByText("Settled Alpha", { exact: true }).waitFor();
  if (await page.locator(".inbox-session-section").count()) {
    throw new Error("Turning Inbox off should restore Projects rendering.");
  }
  if (fixture.sessionIds.length !== 7) {
    throw new Error("Inbox fixture was not seeded completely.");
  }
}

async function dragLocator(page, source, target, targetXRatio = 0.5, targetYRatio = 0.5) {
  const sourceBox = await source.boundingBox();
  const targetBox = await target.boundingBox();
  if (!sourceBox || !targetBox) throw new Error("Drag source or target was not visible.");
  const startX = sourceBox.x + sourceBox.width / 2;
  const startY = sourceBox.y + sourceBox.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  const targetX = targetBox.x + targetBox.width * targetXRatio;
  const targetY = targetBox.y + targetBox.height * targetYRatio;
  await page.mouse.move(
    targetX,
    targetY,
    { steps: 4 },
  );
  await page.waitForTimeout(50);
  const direct = await source.evaluate((element) => ({
    className: element.className,
    pointerEvents: element.closest("[data-sidebar-section-id], [data-sidebar-session-id]")?.style.pointerEvents,
    translate: element.closest("[data-sidebar-section-id], [data-sidebar-session-id]")?.style.translate,
  }));
  const targetClass = await target.evaluate((element) =>
    element.closest("[data-sidebar-section-id], [data-sidebar-session-id]")?.className,
  );
  const hit = await page.evaluate(({ x, y }) => {
    const element = document.elementFromPoint(x, y);
    const owner = element?.closest("[data-sidebar-section-id], [data-sidebar-session-id]");
    return {
      className: element?.className?.baseVal ?? element?.className,
      ownerClass: owner?.className,
      sectionId: owner?.getAttribute("data-sidebar-section-id"),
      sessionId: owner?.getAttribute("data-sidebar-session-id"),
    };
  }, { x: targetX, y: targetY });
  if (!direct.translate || direct.pointerEvents !== "none" || !String(targetClass).includes("drag-over")) {
    await page.mouse.up();
    throw new Error(`Drag source or target did not activate: ${JSON.stringify({ direct, targetClass, hit, sourceBox, targetBox })}.`);
  }
  await page.mouse.up();
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
    const createResponse = page.waitForResponse((response) =>
      response.url().includes("/control/v1/commands") &&
      response.request().method() === "POST",
    );
    await standaloneNewChat.click();
    const response = await createResponse;
    const command = response.request().postDataJSON();
    const result = await response.json();
    if (!response.ok() || command.kind !== "thread.create" || result.status !== "applied") {
      throw new Error(`New chat did not provision canonical state first: ${JSON.stringify({ command, result })}.`);
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
    const nativeChart = {
      title: "Weekly change",
      subtitle: "Percentage-point movement by week",
      type: "bar",
      orientation: "horizontal",
      x_label: "Week",
      y_label: "Change",
      y_format: { style: "percent", precision: 1, sign_display: "always" },
      series: [
        { name: "Current", points: [{ x: "Jul 6–10", y: 8.2 }, { x: "Jul 13–17", y: 3.7 }, { x: "Jul 20–24", y: -12.4 }] },
        { name: "Previous", points: [{ x: "Jul 6–10", y: 5.4 }, { x: "Jul 13–17", y: 1.8 }, { x: "Jul 20–24", y: -8.1 }] },
      ],
    };
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
              streamParts: [{
                kind: "event",
                eventType: "tool",
                label: "Rendered chart",
                name: "render_chart",
                callId: "e2e-native-chart",
                icon: "tool",
                status: "done",
                toolArguments: JSON.stringify(nativeChart),
                mcpApp: { kind: "native_chart" },
                mcpAppResult: nativeChart,
              }, ...views.map((view, index) => ({
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
              }))],
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
  const nativeChart = page.getByTestId("native-chart-view");
  await nativeChart.waitFor();
  await page.waitForFunction(() => document.querySelectorAll('[data-testid="mcp-app-view"]').length === 5);
  for (const kind of kinds) {
    await page
      .frameLocator(`iframe[title="MCP App ui://milim.test/${kind}"]`)
      .locator(`body[data-view="${kind}"][data-ready="true"]`)
      .waitFor({ timeout: 15_000 });
  }

  const lightStyles = await setMcpAppsTheme(page, kinds, "Mono Light", "light");
  await nativeChart.evaluate((element) => { element.style.setProperty("--chart-series-1", "#a6edf2"); });
  const gradientStops = await nativeChart.locator("linearGradient").first().locator("stop").evaluateAll((stops) => stops.map((stop) => getComputedStyle(stop).stopColor));
  if (gradientStops.length !== 2 || gradientStops[0] === gradientStops[1]) throw new Error(`Native bar gradient stops should resolve to distinct tones, got ${gradientStops.join(", ")}.`);
  await nativeChart.screenshot({ path: screenshots.nativeChartLight });
  const marks = nativeChart.locator('[data-chart-mark="true"]');
  if ((await marks.count()) !== 6) throw new Error(`Expected six native chart marks, got ${await marks.count()}.`);
  await nativeChart.getByText("horizontal bar", { exact: true }).waitFor();
  if (!(await marks.first().getAttribute("fill"))?.startsWith("url(#")) throw new Error("Native bars should use their series gradient.");
  await marks.first().hover();
  const chartTooltip = nativeChart.getByTestId("native-chart-tooltip");
  await chartTooltip.waitFor();
  await chartTooltip.getByText("Current", { exact: true }).waitFor();
  await chartTooltip.getByText("Previous", { exact: true }).waitFor();
  await nativeChart.locator(".native-chart-tick.category.active").waitFor();
  const tooltipPointer = await chartTooltip.evaluate((element) => getComputedStyle(element, "::after").content);
  if (tooltipPointer === "none" || tooltipPointer === "normal") throw new Error("Native chart tooltip should render a directional pointer.");
  await marks.first().focus();
  await page.keyboard.press("ArrowDown");
  await page.waitForFunction(() => document.activeElement?.getAttribute("data-point-index") === "1");
  await page.keyboard.press("ArrowRight");
  await page.waitForFunction(() =>
    document.activeElement?.getAttribute("data-series-index") === "1" &&
    document.activeElement?.getAttribute("data-category-index") === "1"
  );
  await marks.first().dispatchEvent("pointerdown", { pointerType: "touch", bubbles: true });
  await marks.first().dispatchEvent("pointerleave", { pointerType: "touch", bubbles: true });
  await chartTooltip.waitFor();
  await nativeChart.locator(".native-chart-header").click();
  await chartTooltip.waitFor({ state: "hidden" });
  const previousLegend = nativeChart.locator(".native-chart-legend button").filter({ hasText: "Previous" });
  await assertAttribute(previousLegend, "aria-pressed", "true");
  await previousLegend.click();
  await assertAttribute(previousLegend, "aria-pressed", "false");
  if ((await marks.count()) !== 3) throw new Error(`Hiding a native chart series should leave three marks, got ${await marks.count()}.`);
  await previousLegend.click();
  await nativeChart.evaluate((element) => { element.style.width = "320px"; });
  await page.waitForFunction(() => {
    const svg = document.querySelector('[data-testid="native-chart-view"] svg');
    const width = Number(svg?.getAttribute("viewBox")?.split(" ")[2]);
    return width >= 280 && width <= 320;
  });
  await nativeChart.screenshot({ path: screenshots.nativeChartNarrow });
  await nativeChart.evaluate((element) => { element.style.removeProperty("width"); });
  const barAnimation = await marks.first().evaluate((element) => getComputedStyle(element).animationName);
  if (!barAnimation.includes("native-chart-bar-reveal-horizontal")) throw new Error(`Horizontal bars should reveal from zero, got ${barAnimation}.`);
  await page.emulateMedia({ reducedMotion: "reduce" });
  const chartAnimation = await marks.first().evaluate((element) => getComputedStyle(element).animationName);
  if (chartAnimation !== "none") throw new Error(`Reduced motion should disable native chart reveal animation, got ${chartAnimation}.`);
  await page.emulateMedia({ reducedMotion: "no-preference" });
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
  await nativeChart.screenshot({ path: screenshots.nativeChartDark });
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

  const apiBase = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke("api_base_url"));
  const previewUrl = new URL("/health", apiBase).toString();
  await page.evaluate(
    async (url) => window.__TAURI_INTERNALS__.invoke("plugin:event|emit", {
      event: "milim://preview-open-url",
      payload: { url },
    }),
    previewUrl,
  );
  const input = page.getByTestId("preview-browser-url");
  await input.waitFor();
  if ((await input.inputValue()) !== previewUrl) {
    throw new Error(`Preview-open event did not select ${previewUrl}.`);
  }
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
  const wideHeader = await page.locator(".preview-header").evaluate((header) => ({
    width: header.getBoundingClientRect().width,
    paddingTop: Number.parseFloat(getComputedStyle(header).paddingTop),
  }));
  if (wideHeader.width < 640 || Math.abs(wideHeader.paddingTop - 10) > 1) {
    throw new Error(`Wide inspector header should not reserve an extra title-bar row: ${JSON.stringify(wideHeader)}.`);
  }
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

async function runStaticWorkspacePreviewCheck(page, pid) {
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.locator(".app-notices").waitFor({ state: "hidden", timeout: 8_000 }).catch(() => {});
  const workspace = mkdtempSync(join(tmpdir(), "milim-static-preview-e2e-"));
  const indexPath = join(workspace, "index.html");
  writeFileSync(
    indexPath,
    '<!doctype html><title>Static Fixture</title><link rel="stylesheet" href="style.css"><h1>Static preview ready</h1>',
    "utf8",
  );
  writeFileSync(join(workspace, "style.css"), "body { color: rgb(12, 34, 56); }", "utf8");
  writeFileSync(join(workspace, "package.json"), '{"private":true,"scripts":{"dev":"node server.js"}}', "utf8");
  writeFileSync(join(workspace, "server.js"), "setInterval(() => {}, 1000);", "utf8");
  mkdirSync(join(workspace, "src"));
  writeFileSync(join(workspace, "src", "nested.txt"), "nested fixture", "utf8");
  mkdirSync(join(workspace, "many"));
  for (let index = 0; index < 205; index += 1) writeFileSync(join(workspace, "many", `file-${String(index).padStart(3, "0")}.txt`), `${index}`, "utf8");
  mkdirSync(join(workspace, "node_modules"));
  writeFileSync(join(workspace, "node_modules", "hidden.js"), "hidden", "utf8");

  try {
    await page.getByTestId("composer-input").fill(`/folder ${workspace}`);
    await page.getByTestId("composer-send").click();
    await page.getByTestId("open-artifact-browser").click();
    await page.getByRole("combobox", { name: "Preview source" }).selectOption("app");
    const reviewCommands = page.getByTestId("preview-runtime-preflight");
    await reviewCommands.getByText("Review commands", { exact: true }).waitFor();
    await reviewCommands.click();
    await reviewCommands.getByText("Refresh commands", { exact: true }).waitFor();
    await page.getByRole("tab", { name: "Code", exact: true }).click();

    const workspaceSearch = page.getByRole("textbox", { name: "Search workspace files", exact: true });
    await workspaceSearch.fill("nested");
    await page.getByRole("button", { name: "src/nested.txt", exact: true }).waitFor();
    await page.getByRole("button", { name: "src/nested.txt", exact: true }).click();
    await page.locator(".workspace-code-toolbar strong", { hasText: "src/nested.txt" }).waitFor();
    await workspaceSearch.fill("");
    await page.getByRole("button", { name: "src", exact: true }).click();
    await page.getByRole("button", { name: "nested.txt", exact: true }).waitFor();
    if (await page.getByRole("button", { name: "node_modules", exact: true }).count()) throw new Error("Excluded dependency directories should not appear in Code.");
    await page.getByRole("button", { name: "many", exact: true }).click();
    await page.getByRole("button", { name: "Load more", exact: true }).waitFor();
    await page.getByRole("button", { name: "Load more", exact: true }).click();
    await page.getByRole("button", { name: "file-204.txt", exact: true }).waitFor();
    await page.getByRole("button", { name: "many", exact: true }).click();
    await page.getByRole("button", { name: "index.html", exact: true }).click();
    const editor = page.locator(".workspace-code-editor .cm-content");
    await editor.waitFor();

    const replaceEditor = async (content) => {
      await editor.click();
      await page.keyboard.press("Control+A");
      await page.keyboard.insertText(content);
    };
    const savedHtml = '<!doctype html><title>Static Fixture</title><link rel="stylesheet" href="style.css"><h1>Saved in Code</h1>';
    await replaceEditor(savedHtml);
    await page.keyboard.press("Control+S");
    await waitForFileText(indexPath, savedHtml);
    if (readFileSync(indexPath, "utf8") !== savedHtml) throw new Error("Ctrl+S did not persist the workspace editor draft.");

    writeFileSync(indexPath, "<!doctype html><title>External version</title><h1>External</h1>", "utf8");
    await replaceEditor("<!doctype html><title>Draft version</title><h1>Draft</h1>");
    await page.keyboard.press("Control+S");
    await page.getByRole("alert").getByText("File changed on disk", { exact: true }).waitFor();
    await page.getByRole("button", { name: "Reload", exact: true }).click();
    await page.getByRole("alert").getByText("File changed on disk", { exact: true }).waitFor({ state: "hidden" });
    if (!(await editor.innerText()).includes("External version")) throw new Error("Conflict Reload did not replace the editor with the disk version.");

    const overwrittenHtml = '<!doctype html><title>Static Fixture</title><link rel="stylesheet" href="style.css"><h1>Overwrite won</h1>';
    await replaceEditor(overwrittenHtml);
    writeFileSync(indexPath, "<!doctype html><title>Second external version</title>", "utf8");
    await page.keyboard.press("Control+S");
    await page.getByRole("alert").getByText("File changed on disk", { exact: true }).waitFor();
    await page.getByRole("button", { name: "Overwrite", exact: true }).click();
    await waitForFileText(indexPath, overwrittenHtml);
    if (readFileSync(indexPath, "utf8") !== overwrittenHtml) throw new Error("Explicit conflict overwrite did not persist the draft.");

    const previewHtml = '<!doctype html><title>Static Fixture</title><link rel="stylesheet" href="style.css"><h1>Static preview ready</h1>';
    await replaceEditor(previewHtml);
    await page.locator(".workspace-code-dirty").waitFor({ state: "attached" });
    await page.waitForTimeout(100);
    await page.evaluate(async () => window.__TAURI_INTERNALS__.invoke("request_desktop_quit"));
    const quitDialog = page.getByRole("dialog", { name: "Save changes?", exact: true });
    await quitDialog.waitFor();
    await quitDialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.getByLabel("Close inspector", { exact: true }).click();
    const leaveDialog = page.getByRole("dialog", { name: "Save changes?", exact: true });
    await leaveDialog.waitFor();
    await leaveDialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.getByTestId("workspace-code-editor").waitFor();
    await editor.click();
    await page.keyboard.press("Control+S");
    await waitForFileText(indexPath, previewHtml);
    await page.keyboard.press("Control+Home");
    await page.getByTestId("preview-resize-handle").press("End");
    await page.getByTestId("workspace-code-rail-resizer").press("Home");
    await page.screenshot({ path: screenshots.workspaceCode, fullPage: false });

    const previewButton = page.getByTestId("workspace-html-preview");
    await previewButton.waitFor();

    const baselineHandles = new Set(wryWebviews(pid).map((view) => view.handle));
    const responsePromise = page.waitForResponse(
      (response) => response.request().method() === "POST" && response.url().includes("/preview-apps/") && response.url().endsWith("/static"),
    );
    await previewButton.click();
    const response = await responsePromise;
    if (!response.ok()) throw new Error(`Static preview start failed: ${response.status()} ${await response.text()}`);
    const status = await response.json();
    if (
      status.kind !== "static" ||
      status.status !== "running" ||
      status.command != null ||
      status.pid != null ||
      status.preflight != null
    ) {
      throw new Error(`Static preview should run without commands: ${JSON.stringify(status)}`);
    }

    await page.getByTestId("preview-native-browser").waitFor();
    await page.locator(".preview-native-browser-status").waitFor({ state: "hidden", timeout: 10_000 });
    await page.getByTestId("preview-runtime-status").getByText("Static preview", { exact: true }).waitFor();
    await page.getByTestId("preview-runtime-quick-stop").getByText("Stop", { exact: true }).waitFor();
    if (!(await page.getByTestId("preview-managed-runtime").evaluate((element) => element.classList.contains("compact")))) throw new Error("Healthy static preview should use the compact runtime toolbar.");
    await waitForNewVisibleWryWebview(pid, baselineHandles);
    const html = await (await fetch(status.url)).text();
    const css = await (await fetch(new URL("style.css", status.url))).text();
    if (!html.includes("Static preview ready") || !css.includes("rgb(12, 34, 56)")) {
      throw new Error("Static preview did not serve the workspace HTML and relative stylesheet.");
    }

    writeFileSync(
      indexPath,
      '<!doctype html><title>Static Fixture Updated</title><link rel="stylesheet" href="style.css"><h1>Updated</h1>',
      "utf8",
    );
    await page.getByRole("button", { name: "Reload page", exact: true }).click();
    await page.locator(".preview-browser-tab.active > button").getByText("Static Fixture Updated", { exact: true }).waitFor({ timeout: 10_000 });

    await page.getByRole("tab", { name: "Code", exact: true }).click();
    await page.getByRole("button", { name: "index.html", exact: true }).click();
    await page.getByTestId("workspace-html-preview").waitFor();
    await page.getByTestId("workspace-html-preview").click();
    await page.getByRole("tab", { name: "Preview", exact: true, selected: true }).waitFor();
    const reusedUrl = await page.getByTestId("preview-browser-url").inputValue();
    if (new URL(reusedUrl).port !== new URL(status.url).port) throw new Error(`Static preview should reuse its loopback server: ${status.url} -> ${reusedUrl}`);

    await page.getByTestId("preview-runtime-quick-stop").click();
    const deadline = Date.now() + 5_000;
    while (Date.now() < deadline) {
      try {
        await fetch(status.url);
        await delay(50);
      } catch {
        await page.getByLabel("Close inspector", { exact: true }).click();
        await page.getByTestId("composer-input").fill(`/folder ${root}`);
        await page.getByTestId("composer-send").click();
        return;
      }
    }
    throw new Error("Static preview server remained reachable after Stop.");
  } finally {
    await rmWithRetry(workspace, { label: "static preview workspace" });
  }
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
      `${testServer.origin}/?phase=cache-reload`,
    );
    await testServer.waitForReport("cache-reload", "main", 10_000, (report) => report.version === "rgb(12, 34, 56)");
    testServer.setCacheColor("rgb(65, 43, 21)");
    await session.page.evaluate(async (label) => {
      await window.__TAURI_INTERNALS__.invoke("preview_webview_reload", { label });
    }, "artifact-browser-e2e-persistent-3");
    assertBrowserReport(
      await testServer.waitForReport("cache-reload", "main", 10_000, (report) => report.version === "rgb(65, 43, 21)"),
      "persistent",
      "Cache-bypassing loopback reload",
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
  let cacheColor = "rgb(12, 34, 56)";
  const server = http.createServer((request, response) => {
    const url = new URL(request.url || "/", "http://127.0.0.1");
    if (url.pathname === "/cache.css") {
      response.writeHead(200, {
        "content-type": "text/css; charset=utf-8",
        "cache-control": "public, max-age=3600",
      });
      response.end(`#popup { color: ${cacheColor}; }`);
      return;
    }
    if (url.pathname === "/report") {
      reports.push({
        phase: url.searchParams.get("phase"),
        kind: url.searchParams.get("kind") || "main",
        cookie: url.searchParams.get("cookie") || "",
        storage: url.searchParams.get("storage") || "",
        version: url.searchParams.get("version") || "",
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
<link rel="stylesheet" href="/cache.css">
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
      + "&version=" + encodeURIComponent(getComputedStyle(document.getElementById("popup")).color)
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
    setCacheColor(value) {
      cacheColor = value;
    },
    async waitForReport(phase, kind = "main", timeoutMs = 10_000, matches = () => true) {
      const started = Date.now();
      while (Date.now() - started < timeoutMs) {
        const report = reports.find((candidate) => candidate.phase === phase && candidate.kind === kind && matches(candidate));
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

async function runReasoningEffortIsolationCheck(page) {
  const model = "e2e-reasoning";
  const originalId = "reasoning-medium";
  const changedId = "reasoning-high";
  const now = Date.now();
  const settings = {
    model,
    instructions: "",
    activeAgentId: null,
    folder: "",
    sandbox: false,
    computerUse: false,
    memory: true,
    privacy: "off",
    toolApproval: "review",
    delegationPolicy: "off",
    workerModel: "",
    planMode: false,
  };

  await page.route("**/v1/models", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      data: [{
        id: model,
        owned_by: "OpenAI",
        reasoning: {
          supported_efforts: ["low", "medium", "high"],
          default_effort: "medium",
          default_enabled: true,
          mandatory: true,
        },
      }],
    }),
  }));
  await page.evaluate(async ({ modelId, firstId, secondId, timestamp, threadSettings }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    await invoke("user_state_set", {
      key: "milim.settings",
      value: JSON.stringify({
        state: {
          accountRuntimeEnabled: { codex: false, claude: false, opencode: false, pi: false },
          reasoningEffortByModel: { [modelId]: "medium" },
          newThreadBehavior: "inherit",
        },
        version: 0,
      }),
    });
    await invoke("user_sessions_set", {
      value: JSON.stringify({
        state: {
          sessions: [
            {
              id: secondId,
              title: "High effort thread",
              messages: [{ id: "high-message", role: "user", content: "Keep this chat separate" }],
              settings: threadSettings,
              createdAt: timestamp + 1,
              updatedAt: timestamp + 1,
            },
            {
              id: firstId,
              title: "Medium effort thread",
              messages: [{ id: "medium-message", role: "user", content: "Keep this chat on medium" }],
              settings: threadSettings,
              createdAt: timestamp,
              updatedAt: timestamp,
            },
          ],
          activeId: secondId,
        },
        version: 0,
      }),
    });
  }, { modelId: model, firstId: originalId, secondId: changedId, timestamp: now, threadSettings: settings });

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  const trigger = page.getByTestId("model-picker-trigger");
  await assertTextContains(trigger.locator(".chip-detail"), "Medium");

  await trigger.click();
  const picker = page.locator(".mp");
  await picker.waitFor();
  await picker.getByRole("button", { name: `Reasoning effort for ${model}: Medium` }).click();
  const effortMenu = page.getByRole("menu", { name: `Reasoning effort for ${model}` });
  await effortMenu.getByRole("menuitemradio").filter({ hasText: "High" }).click();
  await assertTextContains(trigger.locator(".chip-detail"), "High");
  await trigger.click();
  await picker.waitFor({ state: "hidden" }).catch(() => {});

  await page.waitForFunction(async ({ modelId, sessionId }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const sessionsRaw = await invoke("user_state_get", { key: "milim.sessions" });
    const settingsRaw = await invoke("user_state_get", { key: "milim.settings" });
    const sessions = sessionsRaw ? JSON.parse(sessionsRaw).state?.sessions ?? [] : [];
    const appSettings = settingsRaw ? JSON.parse(settingsRaw).state ?? {} : {};
    return (
      sessions.find((session) => session.id === sessionId)?.settings?.reasoningEffortOverrides?.[modelId] === "high" &&
      appSettings.reasoningEffortByModel?.[modelId] === "medium"
    );
  }, { modelId: model, sessionId: changedId });

  await page.locator(`[data-sidebar-session-id="${originalId}"]:visible`).first().click();
  await assertTextContains(trigger.locator(".chip-detail"), "Medium");
  await page.locator(`[data-sidebar-session-id="${changedId}"]:visible`).first().click();
  await assertTextContains(trigger.locator(".chip-detail"), "High");

  await page.locator("button.new-chat:not(.new-chat-menu):visible").first().click();
  await assertTextContains(trigger.locator(".chip-detail"), "Medium");
  await page.waitForFunction(async ({ modelId, previousId }) => {
    const raw = await window.__TAURI_INTERNALS__.invoke("user_state_get", { key: "milim.sessions" });
    const state = raw ? JSON.parse(raw).state ?? {} : {};
    const active = (state.sessions ?? []).find((session) => session.id === state.activeId);
    return state.activeId !== previousId && active?.settings?.reasoningEffortOverrides?.[modelId] === undefined;
  }, { modelId: model, previousId: changedId });
  await page.screenshot({ path: screenshots.reasoningEffort, fullPage: false });
}

async function runGenerationControlsCheck(page) {
  const model = "vllm-e2e-model";
  const sessionId = "generation-controls";
  await page.route("**/v1/models", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({ data: [{ id: model, owned_by: "vLLM (local)" }] }),
  }));
  await page.evaluate(async ({ modelId, threadId }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    await invoke("user_state_set", {
      key: "milim.settings",
      value: JSON.stringify({
        state: {
          accountRuntimeEnabled: { codex: false, claude: false, opencode: false, pi: false },
          newThreadBehavior: "inherit",
        },
        version: 0,
      }),
    });
    await invoke("user_sessions_set", {
      value: JSON.stringify({
        state: {
          sessions: [{
            id: threadId,
            title: "Generation controls",
            messages: [],
            settings: {
              model: modelId,
              instructions: "",
              activeAgentId: null,
              folder: "",
              sandbox: false,
              computerUse: false,
              memory: true,
              privacy: "off",
              toolApproval: "review",
              delegationPolicy: "off",
              workerModel: "",
              planMode: false,
            },
            createdAt: Date.now(),
            updatedAt: Date.now(),
          }],
          activeId: threadId,
        },
        version: 0,
      }),
    });
  }, { modelId: model, threadId: sessionId });

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.getByTestId("context-menu-trigger").click();
  const controls = page.locator(".generation-controls");
  await controls.getByText("Generation", { exact: true }).click();
  await controls.getByText("Model defaults", { exact: true }).waitFor();
  await controls.getByLabel("Output tokens").fill("512");
  await controls.getByLabel("Temperature").fill("0.4");
  await controls.getByLabel("Top K").fill("40");
  await controls.getByLabel("Thinking budget").fill("2048");
  await controls.getByLabel(/Stop sequences/).fill("END\nSTOP");
  await controls.getByText(/Overrides for vllm-e2e-model/).click();

  await page.waitForFunction(async ({ modelId, threadId }) => {
    const raw = await window.__TAURI_INTERNALS__.invoke("user_state_get", { key: "milim.sessions" });
    const session = raw ? JSON.parse(raw).state?.sessions?.find((item) => item.id === threadId) : null;
    const generation = session?.settings?.generationOverrides?.[modelId];
    return generation?.maxTokens === 512
      && generation?.temperature === 0.4
      && generation?.topK === 40
      && generation?.thinkingTokenBudget === 2048
      && generation?.stop?.join(",") === "END,STOP";
  }, { modelId: model, threadId: sessionId });

  await controls.getByRole("button", { name: "Reset generation overrides" }).click();
  await page.waitForFunction(async ({ modelId, threadId }) => {
    const raw = await window.__TAURI_INTERNALS__.invoke("user_state_get", { key: "milim.sessions" });
    const session = raw ? JSON.parse(raw).state?.sessions?.find((item) => item.id === threadId) : null;
    return session?.settings?.generationOverrides?.[modelId] == null;
  }, { modelId: model, threadId: sessionId });
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
  await assertTextContains(
    approvalDescription,
    "Run read-only tools; ask before consequential actions.",
  );
  const approvalGuarded = approvalGroup.getByRole("radio", { name: "Guarded" });
  await approvalGuarded.click();
  await approvalTrigger.getByText("Guarded", { exact: true }).waitFor();
  await assertTextContains(
    approvalDescription,
    "Read-only tools only; consequential actions are unavailable.",
  );
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

async function runTextSelectionPolicyCheck(page) {
  const styles = await page.evaluate(() => {
    const app = document.querySelector(".app");
    const titlebar = document.querySelector(".topbar-thread");
    const composer = document.querySelector('[data-testid="composer-input"]');
    if (!(app instanceof HTMLElement) || !(titlebar instanceof HTMLElement) || !(composer instanceof HTMLElement)) {
      throw new Error("Text selection smoke fixtures require the app shell, titlebar, and composer.");
    }

    const fixture = document.createElement("div");
    const copyable = document.createElement("div");
    copyable.className = "md";
    copyable.textContent = "Copyable message text";
    const nestedControl = document.createElement("button");
    nestedControl.textContent = "Message action";
    copyable.append(nestedControl);
    const input = document.createElement("input");
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    fixture.append(copyable, input, editable);
    app.append(fixture);

    const result = {
      chrome: getComputedStyle(titlebar).userSelect,
      nestedControl: getComputedStyle(nestedControl).userSelect,
      copyable: getComputedStyle(copyable).userSelect,
      composer: getComputedStyle(composer).userSelect,
      input: getComputedStyle(input).userSelect,
      editable: getComputedStyle(editable).userSelect,
    };
    fixture.remove();
    return result;
  });

  const expected = {
    chrome: "none",
    nestedControl: "none",
    copyable: "text",
    composer: "text",
    input: "text",
    editable: "text",
  };
  for (const [surface, value] of Object.entries(expected)) {
    if (styles[surface] !== value) {
      throw new Error(`Expected ${surface} user-select ${value}, got ${styles[surface]}.`);
    }
  }
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
  const continueButton = page.getByTestId("baton-continue").last();
  await continueButton.waitFor({ timeout: 60_000 });
  if (!(await continueButton.isVisible())) {
    throw new Error("Continue with should remain visible on the latest completed response.");
  }
  await continueButton.focus();
  if (!(await continueButton.evaluate((element) => element === document.activeElement))) {
    throw new Error("Continue with should accept keyboard focus.");
  }
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
  await page.getByText("vLLM (local)").waitFor({ timeout: 20_000 });
  await page
    .locator(".provider-discovery-row", { hasText: "vLLM (local)" })
    .locator('[data-provider-brand="vllm"]')
    .waitFor();
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
  await page.waitForTimeout(350);
}

async function runLinkedThreadDropCheck(page) {
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);

  const targetId = "e2e-linked-target";
  const originId = "e2e-linked-origin";
  await page.evaluate(async ({ targetId: target, originId: origin }) => {
    const invoke = window.__TAURI_INTERNALS__.invoke;
    const [base, token] = await Promise.all([invoke("api_base_url"), invoke("api_token")]);
    for (const [id, title] of [[target, "Linked target"], [origin, "Linked origin"]]) {
      const response = await fetch(`${base}/control/v1/commands`, {
        method: "POST",
        headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
        body: JSON.stringify({
          command_id: `e2e-create-${id}`,
          kind: "thread.create",
          thread_id: id,
          payload: { id, title, settings: {} },
        }),
      });
      const result = await response.json();
      if (!response.ok || result.status !== "applied") {
        throw new Error(`Canonical fixture create failed: ${JSON.stringify({ id, result })}`);
      }
    }
  }, { targetId, originId });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.locator(`[data-sidebar-session-id="${originId}"]:visible`).first().click();
  await page.locator(`[data-sidebar-session-id="${originId}"].active`).waitFor();

  const fileTargets = [
    ["sidebar", ".sidebar"],
    ["top bar", ".topbar"],
    ["transcript", ".chat-scroll"],
    ["composer", '[data-testid="composer"]'],
  ];
  for (let index = 0; index < fileTargets.length; index += 1) {
    const [label, selector] = fileTargets[index];
    const before = await page.locator(".attachment-pill").count();
    const result = await page.evaluate(({ selector: targetSelector, index: targetIndex }) => {
      const target = document.querySelector(targetSelector);
      if (!(target instanceof HTMLElement)) return { found: false };
      const binary = atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=");
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([bytes], `native-drop-${targetIndex}.png`, { type: "image/png" }));
      target.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: transfer }));
      return { found: true };
    }, { selector, index });
    if (!result.found) throw new Error(`Native file-drop target was missing: ${label}.`);
    await page.getByTestId("window-file-drop-overlay").waitFor();
    await page.evaluate(({ selector: targetSelector, index: targetIndex }) => {
      const target = document.querySelector(targetSelector);
      const binary = atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=");
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([bytes], `native-drop-${targetIndex}.png`, { type: "image/png" }));
      target?.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    }, { selector, index });
    await page.waitForFunction((expected) => document.querySelectorAll(".attachment-pill").length === expected, before + 1);
  }

  await page.getByTestId("open-artifact-browser").click();
  const inspector = page.getByTestId("inspector-shell");
  await inspector.waitFor();
  const beforeInspector = await page.locator(".attachment-pill").count();
  await inspector.evaluate((target, targetIndex) => {
    const binary = atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=");
    const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
    const transfer = new DataTransfer();
    transfer.items.add(new File([bytes], `native-drop-${targetIndex}.png`, { type: "image/png" }));
    target.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
  }, fileTargets.length);
  await page.waitForFunction((expected) => document.querySelectorAll(".attachment-pill").length === expected, beforeInspector + 1);

  const beforeTextDrop = await page.locator(".attachment-pill").count();
  await page.locator(".chat-scroll").evaluate((target) => {
    const transfer = new DataTransfer();
    transfer.setData("text/plain", "not a file");
    target.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
  });
  if (await page.getByTestId("window-file-drop-overlay").count()) {
    throw new Error("A non-file drag activated the native full-window attachment affordance.");
  }
  if (await page.locator(".attachment-pill").count() !== beforeTextDrop) {
    throw new Error("A non-file drag changed native composer attachments.");
  }

  const source = page.locator(`[data-sidebar-session-id="${targetId}"]:visible`).first();
  const destination = page.locator(`[data-thread-link-drop-target="${originId}"]`);
  const sourceBox = await source.boundingBox();
  const destinationBox = await destination.boundingBox();
  if (!sourceBox || !destinationBox) throw new Error("Native linked-chat drag endpoints were not visible.");
  await page.mouse.move(sourceBox.x + sourceBox.width / 2, sourceBox.y + sourceBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(destinationBox.x + destinationBox.width / 2, destinationBox.y + destinationBox.height / 2, { steps: 6 });
  await page.waitForFunction(() => document.body.classList.contains("thread-link-target-active"));
  const prompt = await destination.getAttribute("data-thread-link-prompt");
  if (!prompt?.startsWith("Link ")) {
    await page.mouse.up();
    throw new Error(`Native linked-chat affordance was not populated: ${String(prompt)}.`);
  }
  await page.mouse.up();
  const pill = page.getByTestId(`linked-thread-pill-${targetId}`);
  await pill.waitFor();
  await pill.getByRole("button", { name: /^Unlink / }).click();
  await pill.waitFor({ state: "detached" });
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
  await page.getByTestId("onboarding-preflight").waitFor({ state: "hidden", timeout: 30_000 });
  const flow = page.getByTestId("onboarding-flow");
  if (await flow.isVisible().catch(() => false)) {
    await page.getByLabel("Close onboarding").click();
    await flow.waitFor({ state: "hidden", timeout: 10_000 });
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
  await page.getByTestId("settings-page").waitFor();
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
  const settingsPage = page.getByTestId("settings-page");
  await settingsPage.waitFor();
  await page.getByTestId("settings-section-app").waitFor();
  const surface = await settingsPage.evaluate((element) => {
    const style = getComputedStyle(element);
    const rect = element.getBoundingClientRect();
    return {
      transitionDuration: style.transitionDuration,
      transitionProperty: style.transitionProperty,
      position: style.position,
      bounds: { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
      viewport: { width: innerWidth, height: innerHeight },
    };
  });
  if (
    !surface.transitionDuration.includes("0.18s") ||
    !surface.transitionProperty.includes("opacity") ||
    surface.transitionProperty.includes("scale") ||
    surface.transitionProperty.includes("translate") ||
    surface.position !== "fixed" ||
    Math.abs(surface.bounds.left) > 1 ||
    Math.abs(surface.bounds.top) > 1 ||
    Math.abs(surface.bounds.width - surface.viewport.width) > 1 ||
    Math.abs(surface.bounds.height - surface.viewport.height) > 1
  ) {
    throw new Error(`Settings should fill the viewport with opacity-only 180ms entry motion: ${JSON.stringify(surface)}.`);
  }
  if (await page.locator(".settings-page .sheet-overlay").count()) {
    throw new Error("Settings should not render modal sheet chrome or a backdrop.");
  }
  await settingsPage.locator(".settings-page-titlebar").getByText("Back to app", { exact: true }).waitFor();
  await settingsPage.getByRole("button", { name: "Close window" }).waitFor();
  const workspace = await page.locator(".main").evaluate((element) => ({
    inert: element.inert,
    ariaHidden: element.getAttribute("aria-hidden"),
    chatMounted: Boolean(element.querySelector('[data-testid="chat-shell"]')),
    opacity: getComputedStyle(element).opacity,
  }));
  if (!workspace.inert || workspace.ariaHidden !== "true" || !workspace.chatMounted || workspace.opacity !== "0") {
    throw new Error(`Settings should inert but preserve the mounted workspace: ${JSON.stringify(workspace)}.`);
  }
  const usageToggle = page.getByTestId("general-titlebar-account-usage-toggle");
  if (await usageToggle.isVisible() && await usageToggle.getAttribute("aria-checked") !== "true") {
    throw new Error("Title-bar account usage should default on.");
  }
}

async function runSettingsLayoutCheck(page) {
  await page.setViewportSize({ width: 1000, height: 720 });
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  const ridgeline = page.getByTestId("empty-usage-ridgeline");
  if (await ridgeline.count()) {
    await ridgeline.waitFor();
  }
  await openSettings(page);
  await page.setViewportSize({ width: 1440, height: 720 });
  await page.waitForTimeout(120);
  const wideSettingsLayout = await page.getByTestId("settings-page").evaluate((element) => {
    const detail = element.querySelector(".settings-detail");
    const content = element.querySelector(".settings-content");
    const inner = element.querySelector(".settings-content-inner");
    const detailRect = detail?.getBoundingClientRect();
    const contentRect = content?.getBoundingClientRect();
    const innerRect = inner?.getBoundingClientRect();
    return {
      detailRight: detailRect?.right,
      contentRight: contentRect?.right,
      innerLeft: innerRect?.left,
      innerRight: innerRect?.right,
      innerWidth: innerRect?.width,
    };
  });
  if (
    wideSettingsLayout.detailRight === undefined ||
    wideSettingsLayout.contentRight === undefined ||
    wideSettingsLayout.innerLeft === undefined ||
    wideSettingsLayout.innerRight === undefined ||
    wideSettingsLayout.innerWidth === undefined ||
    Math.abs(wideSettingsLayout.contentRight - wideSettingsLayout.detailRight) > 1 ||
    wideSettingsLayout.innerWidth > 901 ||
    wideSettingsLayout.contentRight - wideSettingsLayout.innerRight < 100
  ) {
    throw new Error(`Settings scrollbar should stay at the pane edge while content remains centered: ${JSON.stringify(wideSettingsLayout)}.`);
  }
  await page.setViewportSize({ width: 1000, height: 720 });
  await page.waitForTimeout(120);
  await page.getByTestId("settings-section-chat").click();
  await page.getByTestId("new-thread-behavior-configured").click();
  const openApprovalDefault = page.getByTestId("default-approval-open");
  await openApprovalDefault.scrollIntoViewIfNeeded();
  await openApprovalDefault.click();
  if (await openApprovalDefault.getAttribute("aria-checked") !== "true") {
    throw new Error("Open should be selectable as the configured new-chat approval default.");
  }
  await page.getByTestId("settings-section-app").click();
  const projectsChoice = page.getByTestId("sidebar-organization-projects");
  const inboxChoice = page.getByTestId("sidebar-organization-inbox");
  await assertAttribute(projectsChoice, "aria-checked", "true");
  await inboxChoice.click();
  await assertAttribute(inboxChoice, "aria-checked", "true");
  await page.getByTestId("settings-section-appearance").click();
  const ridgelineToggle = page.getByTestId("empty-chat-ridgeline-toggle");
  await ridgelineToggle.scrollIntoViewIfNeeded();
  if (await ridgelineToggle.getAttribute("aria-checked") !== "true") {
    throw new Error("The empty-chat ridgeline should default on.");
  }
  await page.locator(".settings-content").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: screenshots.settings, fullPage: false });

  const initialSurfaceColor = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--bg-primary").trim(),
  );
  await page.getByTestId("theme-customize").click();
  const themeEditor = page.getByTestId("theme-editor");
  await themeEditor.waitFor();
  const editorBounds = await themeEditor.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { left: rect.left, top: rect.top, width: rect.width, height: rect.height, viewportWidth: innerWidth, viewportHeight: innerHeight };
  });
  if (
    Math.abs(editorBounds.left) > 1 ||
    Math.abs(editorBounds.top) > 1 ||
    Math.abs(editorBounds.width - editorBounds.viewportWidth) > 1 ||
    Math.abs(editorBounds.height - editorBounds.viewportHeight) > 1
  ) {
    throw new Error(`Theme editor should replace Settings at full-window bounds: ${JSON.stringify(editorBounds)}.`);
  }
  await themeEditor.getByText("Back to Appearance", { exact: true }).waitFor();
  await page.waitForTimeout(220);
  const backgroundCss = "linear-gradient(135deg, #3b1d75 0%, #0b4a67 52%, #07111e 100%)";
  await page.getByRole("textbox", { name: "Image or gradient CSS" }).fill(backgroundCss);
  const editorThemeSurface = await themeEditor.evaluate((element) => ({
    hasBackgroundClass: element.classList.contains("has-theme-background"),
    surfaceBackground: getComputedStyle(element).backgroundColor,
    workspaceOpacity: getComputedStyle(document.querySelector(".main")).opacity,
  }));
  if (!editorThemeSurface.hasBackgroundClass || editorThemeSurface.surfaceBackground !== "rgba(0, 0, 0, 0)" || editorThemeSurface.workspaceOpacity !== "0") {
    throw new Error(`Theme editor should expose the live custom background without workspace bleed-through: ${JSON.stringify(editorThemeSurface)}.`);
  }
  await page.screenshot({ path: screenshots.settingsThemeEditor, fullPage: false });
  await setThemeEditorColor(page, "Background", "#ff00ff");
  const previewSurfaceColor = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--bg-primary").trim(),
  );
  if (previewSurfaceColor === initialSurfaceColor) {
    throw new Error("Theme editor color changes should preview live.");
  }
  await page.keyboard.press("Escape");
  await themeEditor.waitFor({ state: "detached" });
  await page.getByTestId("settings-page").waitFor();
  await assertThemeSurfaceColor(page, initialSurfaceColor, "Escape should revert theme preview");

  await page.getByTestId("theme-customize").click();
  await page.getByTestId("theme-editor").waitFor();
  await setThemeEditorColor(page, "Background", "#00ffff");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByTestId("settings-page").waitFor();
  await assertThemeSurfaceColor(page, initialSurfaceColor, "Cancel should revert theme preview");

  await page.getByTestId("theme-customize").click();
  await page.getByTestId("theme-editor").waitFor();
  const originalPrimary = await themeEditorColorValue(page, "Primary");
  await setThemeEditorColor(page, "Background", originalPrimary);
  const saveBlocked = page.getByRole("button", { name: "Save", exact: true });
  if (!(await saveBlocked.isDisabled())) {
    throw new Error("Theme editor should block saving a low-contrast theme.");
  }
  await page.getByTestId("theme-editor-back").click();
  await page.getByTestId("settings-page").waitFor();
  await assertThemeSurfaceColor(page, initialSurfaceColor, "Back to Appearance should revert theme preview");

  const customThemeName = `Settings E2E ${Date.now()}`;
  await page.getByTestId("theme-customize").click();
  await page.getByTestId("theme-editor").waitFor();
  await page.getByRole("textbox", { name: "Theme name" }).fill(customThemeName);
  await page.getByRole("textbox", { name: "Image or gradient CSS" }).fill(backgroundCss);
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.getByTestId("settings-page").waitFor();
  const settingsThemeSurface = await page.getByTestId("settings-page").evaluate((element) => ({
    hasBackgroundClass: element.classList.contains("has-theme-background"),
    surfaceBackground: getComputedStyle(element).backgroundColor,
  }));
  if (!settingsThemeSurface.hasBackgroundClass || settingsThemeSurface.surfaceBackground !== "rgba(0, 0, 0, 0)") {
    throw new Error(`Settings should retain a saved custom background: ${JSON.stringify(settingsThemeSurface)}.`);
  }
  const savedTheme = page.locator(".theme-card").filter({ hasText: customThemeName });
  await savedTheme.waitFor();
  await page.getByRole("button", { name: `Edit ${customThemeName}` }).click();
  await page.getByTestId("theme-editor").waitFor();
  await page.getByRole("button", { name: `Delete ${customThemeName}` }).click();
  await page.getByRole("button", { name: `Confirm delete ${customThemeName}` }).click();
  await page.getByTestId("settings-page").waitFor();
  await savedTheme.waitFor({ state: "detached" });

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
  await page.locator(".settings-content").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: screenshots.settingsNarrow, fullPage: false });

  await page.setViewportSize({ width: 640, height: 480 });
  await page.waitForTimeout(200);
  const minimumLayout = await page.getByTestId("settings-page").evaluate((element) => {
    const closeButton = element.querySelector('[aria-label="Close window"]');
    const content = element.querySelector(".settings-content");
    const closeRect = closeButton?.getBoundingClientRect();
    return {
      closeVisible: Boolean(closeRect && closeRect.width > 0 && closeRect.right <= innerWidth + 1 && closeRect.top >= -1),
      contentOverflow: content ? getComputedStyle(content).overflowY : "missing",
      contentScrollable: Boolean(content && content.scrollHeight > content.clientHeight),
    };
  });
  if (!minimumLayout.closeVisible || minimumLayout.contentOverflow !== "auto" || !minimumLayout.contentScrollable) {
    throw new Error(`Settings should remain controllable and scrollable at 640x480: ${JSON.stringify(minimumLayout)}.`);
  }
  await page.locator(".settings-content").evaluate((element) => { element.scrollTop = 0; });
  await page.screenshot({ path: screenshots.settingsMinimum, fullPage: false });

  await ridgelineToggle.click();
  await closeSettings(page);
  await expectFocusedTestId(page, "open-settings");
  await ridgeline.waitFor({ state: "hidden" });

  await openSettings(page);
  await page.keyboard.press("Escape");
  await page.getByTestId("settings-page").waitFor({ state: "detached" });
  await expectFocusedTestId(page, "open-settings");
}

async function closeSettings(page) {
  await page.getByTestId("close-settings").click();
  await page.getByTestId("settings-page").waitFor({ state: "detached" });
  const workspace = await page.locator(".main").evaluate((element) => ({
    inert: element.inert,
    ariaHidden: element.getAttribute("aria-hidden"),
  }));
  if (workspace.inert || workspace.ariaHidden !== null) {
    throw new Error(`Closing Settings should reactivate the workspace: ${JSON.stringify(workspace)}.`);
  }
}

function themeEditorColorField(page, label) {
  return page.locator(".ui-color").filter({ hasText: label }).first();
}

async function themeEditorColorValue(page, label) {
  return (await themeEditorColorField(page, label).locator("code").innerText()).trim();
}

async function setThemeEditorColor(page, label, value) {
  const field = themeEditorColorField(page, label);
  await field.locator(".ui-color-swatch").click();
  await field.locator(".ui-hex").fill(value);
  await page.locator(".editor-header").click();
}

async function assertThemeSurfaceColor(page, expected, message) {
  const actual = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue("--bg-primary").trim(),
  );
  if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}.`);
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
  await page.getByTestId("settings-page").waitFor();
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
  const shutdownStarted = Date.now();
  await session.page.evaluate(async () => {
    await window.__TAURI_INTERNALS__.invoke("restart_app");
  }).catch(() => {});
  await waitForExit(session.child, 5_000);
  const shutdownElapsed = Date.now() - shutdownStarted;
  if (shutdownElapsed > 5_000) {
    throw new Error(`Graceful desktop shutdown took ${shutdownElapsed}ms; expected at most 5000ms.`);
  }
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
  const initialViewportWidth = await page.evaluate(() => document.documentElement.clientWidth);
  await composer.fill("W".repeat(100));

  await page.keyboard.press("Control+=");
  await chip.waitFor();
  await value.filter({ hasText: "110%" }).waitFor();
  await page.screenshot({ path: screenshots.zoom, fullPage: false });

  const increase = page.getByTestId("ui-zoom-increase");
  for (let step = 0; step < 3; step += 1) await increase.click();
  await value.filter({ hasText: "140%" }).waitFor();
  await page.waitForFunction(
    (width) => document.documentElement.clientWidth < width * 0.85,
    initialViewportWidth,
  );
  const composerOverflow = await composer.evaluate((element) => ({
    horizontal: element.scrollWidth - element.clientWidth,
    vertical: element.scrollHeight - element.clientHeight,
    overflowX: getComputedStyle(element).overflowX,
    overflowY: getComputedStyle(element).overflowY,
  }));
  if (
    composerOverflow.horizontal > 1 ||
    composerOverflow.vertical > 1 ||
    composerOverflow.overflowX !== "hidden" ||
    composerOverflow.overflowY !== "hidden"
  ) {
    throw new Error(`Zoomed composer should grow without scrollbars: ${JSON.stringify(composerOverflow)}.`);
  }
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
  await composer.fill("");
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

async function assertSidebarResizeHandleAlignment(page) {
  const alignment = await page.evaluate(() => {
    const sidebar = document.querySelector(".sidebar");
    const handle = document.querySelector('[data-testid="sidebar-resize-handle"]');
    if (!(sidebar instanceof HTMLElement) || !(handle instanceof HTMLElement)) return null;
    const sidebarRect = sidebar.getBoundingClientRect();
    const handleRect = handle.getBoundingClientRect();
    const visibleInset = Number.parseFloat(getComputedStyle(sidebar, "::before").right);
    return {
      handleCenter: handleRect.left + handleRect.width / 2,
      handleWidth: handleRect.width,
      seam: sidebarRect.right - visibleInset,
    };
  });
  if (!alignment || Math.abs(alignment.handleCenter - alignment.seam) > 1 || Math.abs(alignment.handleWidth - 12) > 1) {
    throw new Error(`Sidebar resize handle should center its 12px target on the visible seam: ${JSON.stringify(alignment)}.`);
  }
}

async function assertInspectorResizeHandleAlignment(page) {
  const alignment = await page.evaluate(() => {
    const shell = document.querySelector(".inspector-shell");
    const handle = document.querySelector('[data-testid="preview-resize-handle"]');
    if (!(shell instanceof HTMLElement) || !(handle instanceof HTMLElement)) return null;
    const shellRect = shell.getBoundingClientRect();
    const handleRect = handle.getBoundingClientRect();
    return {
      handleCenter: handleRect.left + handleRect.width / 2,
      handleWidth: handleRect.width,
      seam: shellRect.left,
    };
  });
  if (!alignment || Math.abs(alignment.handleCenter - alignment.seam) > 1 || Math.abs(alignment.handleWidth - 12) > 1) {
    throw new Error(`Inspector resize handle should center its 12px target on the panel seam: ${JSON.stringify(alignment)}.`);
  }
}

async function runResizeHandleCheck(page) {
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);

  const sidebarHandle = page.getByTestId("sidebar-resize-handle");
  await assertSidebarResizeHandleAlignment(page);
  const sidebarWidth = await sidebarHandle.getAttribute("aria-valuenow");
  await sidebarHandle.focus();
  await page.keyboard.press("ArrowRight");
  if ((await sidebarHandle.getAttribute("aria-valuenow")) === sidebarWidth) {
    throw new Error("Sidebar resize handle should remain keyboard operable.");
  }
  await page.keyboard.press("Enter");

  await page.getByTestId("open-artifact-browser").click();
  const previewHandle = page.getByTestId("preview-resize-handle");
  await previewHandle.waitFor();
  await delay(220);
  await assertInspectorResizeHandleAlignment(page);
  const previewWidth = await previewHandle.getAttribute("aria-valuenow");
  await previewHandle.focus();
  await page.keyboard.press("ArrowLeft");
  if ((await previewHandle.getAttribute("aria-valuenow")) === previewWidth) {
    throw new Error("Inspector resize handle should remain keyboard operable.");
  }
  await page.keyboard.press("Enter");
  await delay(220);
  await page.screenshot({ path: screenshots.resizeHandles, fullPage: false });
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

  await runComposerAutocompleteDismissalCheck(page, composer);

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
  await assertSidebarResizeHandleAlignment(page);
  const sidebarDragX = sidebarDragBox.x + sidebarDragBox.width / 2;
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
  await page.mouse.move(sidebarHandleBox.x + sidebarHandleBox.width / 2, sidebarHandleBox.y + sidebarHandleBox.height / 2);
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
  await delay(220);
  await assertInspectorResizeHandleAlignment(page);
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
  const previewDragX = previewDragBox.x + previewDragBox.width / 2;
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
  await page.mouse.move(previewHandleBox.x + previewHandleBox.width / 2, previewHandleBox.y + previewHandleBox.height / 2);
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

async function runComposerAutocompleteDismissalCheck(page, composer) {
  await composer.fill("/");
  const slashMenu = page.getByTestId("slash-menu");
  await slashMenu.waitFor();
  await page.keyboard.press("Escape");
  await slashMenu.waitFor({ state: "hidden" });
  await composer.fill("");
  await composer.fill("/");
  await slashMenu.waitFor();
  await composer.fill("");
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
  const inspectorShell = page.getByTestId("inspector-shell");
  const startBox = await previewHandle.boundingBox();
  if (!startBox) throw new Error("Progressive inspector resize requires measurable handle bounds.");
  const startX = startBox.x + startBox.width / 2;
  const startY = startBox.y + startBox.height / 2;
  const startWidth = Number(await previewHandle.getAttribute("aria-valuenow"));
  const floatingGeometry = await inspectorShell.evaluate((shell) => {
    const panel = shell.querySelector(":scope > .preview-panel");
    const shellRect = shell.getBoundingClientRect();
    const panelRect = panel?.getBoundingClientRect();
    const style = panel ? getComputedStyle(panel) : null;
    return {
      shellWidth: shellRect.width,
      titlebarHeight: Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue("--titlebar-height"),
      ),
      top: panelRect ? panelRect.top - shellRect.top : -1,
      right: panelRect ? shellRect.right - panelRect.right : -1,
      bottom: panelRect ? shellRect.bottom - panelRect.bottom : -1,
      left: panelRect ? panelRect.left - shellRect.left : -1,
      border: style?.borderTopWidth,
      radius: Number.parseFloat(style?.borderTopLeftRadius ?? "0"),
    };
  });
  if (
    Math.abs(floatingGeometry.shellWidth - startWidth) > 1 ||
    Math.abs(floatingGeometry.top - floatingGeometry.titlebarHeight - 6) > 1 ||
    [floatingGeometry.right, floatingGeometry.bottom, floatingGeometry.left]
      .some((gap) => Math.abs(gap - 6) > 1) ||
    floatingGeometry.border !== "1px" ||
    floatingGeometry.radius <= 0
  ) {
    throw new Error(`Inspector should retain its width around one inset rounded surface below the title bar: ${JSON.stringify(floatingGeometry)}.`);
  }
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
    const panel = body?.querySelector(":scope > .inspector-shell");
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
    panel: document.querySelector(".chat-body > .inspector-shell")?.getBoundingClientRect().toJSON(),
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

async function runChatAffordancesCheck(page) {
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke unavailable for chat affordance fixture.");
    const now = Date.now();
    const messages = Array.from({ length: 36 }, (_, index) => ({
      id: `chat-affordance-${index}`,
      role: index % 2 ? "assistant" : "user",
      content: index === 35
        ? "Read [Prompt Kit docs](https://www.prompt-kit.com/docs?ref=e2e) for the source pattern."
        : `Transcript fixture message ${index + 1}.`,
    }));
    await invoke("user_sessions_set", {
      value: JSON.stringify({
        state: {
          sessions: [{
            id: "e2e-chat-affordances",
            title: "Chat affordances fixture",
            messages,
            createdAt: now,
            updatedAt: now,
          }],
          activeId: "e2e-chat-affordances",
        },
        version: 0,
      }),
    });
  });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);

  const source = page.locator(".md-source-link").last();
  await source.waitFor();
  if (!(await source.getAttribute("aria-label"))?.includes("opens in browser")) {
    throw new Error("Assistant source chip did not expose its external destination.");
  }
  const scroll = page.locator(".chat-scroll");
  const scrollBeforeHover = await scroll.evaluate((element) => element.scrollTop);
  await source.hover();
  await page.waitForFunction(() => {
    const preview = document.querySelector(".md-source-preview");
    return preview instanceof HTMLElement && getComputedStyle(preview).position === "fixed";
  });
  const scrollAfterHover = await scroll.evaluate((element) => element.scrollTop);
  if (Math.abs(scrollAfterHover - scrollBeforeHover) > 1) {
    throw new Error(`Source hover moved the transcript from ${scrollBeforeHover} to ${scrollAfterHover}.`);
  }
  const sourceDetail = await page.locator(".md-source-preview").last().innerText();
  if (!sourceDetail.includes("prompt-kit.com/docs") || sourceDetail.includes("ref=e2e")) {
    throw new Error(`Source hover detail was not compact and query-free: ${sourceDetail}`);
  }
  const sourcePreviewStyle = await page.locator(".md-source-preview").last().evaluate((element) => {
    const style = getComputedStyle(element);
    const reference = document.createElement("div");
    reference.className = "run-body";
    document.body.append(reference);
    const referenceStyle = getComputedStyle(reference);
    const result = {
      parentIsBody: element.parentElement === document.body,
      background: style.backgroundColor,
      blur: style.backdropFilter,
      expectedBackground: referenceStyle.backgroundColor,
      expectedBlur: referenceStyle.backdropFilter,
    };
    reference.remove();
    return result;
  });
  if (!sourcePreviewStyle.parentIsBody) {
    throw new Error("Source tooltip should render outside the scrolling transcript.");
  }
  if (sourcePreviewStyle.background !== sourcePreviewStyle.expectedBackground) {
    throw new Error(`Source tooltip did not use the shared popover background: ${JSON.stringify(sourcePreviewStyle)}.`);
  }
  if (!sourcePreviewStyle.blur.includes(sourcePreviewStyle.expectedBlur)) {
    throw new Error(`Source tooltip did not use the shared popover blur: ${JSON.stringify(sourcePreviewStyle)}.`);
  }
  await page.screenshot({ path: screenshots.chatSources, fullPage: false });

  const canScroll = await scroll.evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event("scroll"));
    return element.scrollHeight > element.clientHeight;
  });
  if (!canScroll) throw new Error("Chat affordance fixture did not create a scrollable transcript.");
  const latest = page.getByTestId("chat-jump-latest");
  await latest.waitFor();
  await page.screenshot({ path: screenshots.chatLatest, fullPage: false });
  await latest.click();
  await page.waitForFunction(() => {
    const element = document.querySelector(".chat-scroll");
    return element instanceof HTMLElement &&
      element.scrollHeight - element.scrollTop - element.clientHeight <= 32;
  });
  await latest.waitFor({ state: "hidden" });
}

async function runHarnessHardeningUiCheck(page) {
  const requests = [];
  // The preceding chat-affordance interaction can leave a debounced session
  // write pending. Let it commit before replacing the native session fixture so
  // pagehide cannot restore the previous transcript over this one.
  await page.waitForTimeout(3_100);
  await page.route("**/control/v1/runs/e2e-ledger-run**", async (route) => {
    const url = new URL(route.request().url());
    requests.push(url.pathname + url.search);
    if (url.pathname.endsWith("/events")) {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          run_id: "e2e-ledger-run",
          after_seq: null,
          next_seq: null,
          has_more: false,
          events: [{
            id: "e2e-ledger-event",
            run_id: "e2e-ledger-run",
            seq: 1,
            step_id: "step-1",
            type: "model_request_resolved",
            data: { artifact_digest: "sha256:e2e" },
            created_at_ms: 2,
          }],
        }),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        run: {
          id: "e2e-ledger-run",
          thread_id: "e2e-ledger-thread",
          status: "completed",
          adapter: "provider",
          config: { model: "e2e-ledger-model" },
          capabilities: {
            ledger: true,
            inspectable: true,
            steering: true,
            visibility: "model_visible",
          },
          created_at_ms: 1,
          updated_at_ms: 2,
          completed_at_ms: 2,
          error: null,
        },
        composition: {
          visibility: "model_visible",
          adapter: "provider",
          model: "e2e-ledger-model",
          reasoning_effort: null,
          native_session_boundary: null,
          prompt_sections: [],
          tools: [],
          policies: {},
          environment_policy: "HostShellInherited",
          explicit_environment_grants: [],
        },
      }),
    });
  });

  await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke unavailable for run-ledger fixture.");
    const now = Date.now();
    await invoke("user_sessions_set", {
      value: JSON.stringify({
        state: {
          sessions: [{
            id: "e2e-ledger-thread",
            title: "Quiet run details fixture",
            messages: [
              { id: "e2e-user", role: "user", content: "Inspect quietly." },
              {
                id: "e2e-legacy-work",
                role: "assistant",
                content: "Legacy complete.",
                metrics: { model: "legacy-fixture-model", durationMs: 900 },
                streamParts: [{
                  kind: "event",
                  eventType: "tool",
                  label: "Read legacy file",
                  status: "done",
                }, { kind: "text", content: "Legacy complete." }],
              },
              { id: "e2e-ledger-user", role: "user", content: "Inspect the ledger quietly." },
              {
                id: "e2e-ledger-work",
                role: "assistant",
                content: "Ledger complete.",
                runId: "e2e-ledger-run",
                ledgerVersion: 1,
                metrics: { model: "e2e-ledger-model", durationMs: 1200 },
                streamParts: [{
                  kind: "event",
                  eventType: "tool",
                  label: "Read ledger file",
                  status: "done",
                }, { kind: "text", content: "Ledger complete." }],
              },
            ],
            settings: {
              model: "mock-echo",
              instructions: "",
              activeAgentId: null,
              folder: "",
              sandbox: false,
              computerUse: false,
              memory: false,
              privacy: "off",
              toolApproval: "review",
              delegationPolicy: "off",
              workerModel: "",
              planMode: false,
            },
            createdAt: now,
            updatedAt: now,
          }],
          activeId: "e2e-ledger-thread",
        },
        version: 0,
      }),
    });
  });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await dismissOnboardingIfPresent(page);
  await page.waitForFunction(() =>
    document.body.textContent?.includes("Ledger complete.") ||
    document.body.textContent?.includes("Milim needs a quick restart."),
  );
  if (await page.getByText("Milim needs a quick restart.", { exact: true }).isVisible().catch(() => false)) {
    await page.getByText("Technical details", { exact: true }).click();
    const detail = await page.locator(".app-error-details code").innerText();
    throw new Error(`Run-ledger fixture crashed the UI: ${detail}`);
  }

  if (requests.length !== 0) {
    throw new Error(`Closed transcript requested run details: ${JSON.stringify(requests)}.`);
  }
  const workGroups = page.getByTestId("assistant-stream-work-group");
  const workGroupCount = await workGroups.count();
  if (workGroupCount !== 2) {
    const assistantText = await page.getByTestId("assistant-message").allInnerTexts();
    const persisted = await page.evaluate(async () => {
      const raw = await window.__TAURI_INTERNALS__?.invoke("user_sessions_get");
      if (typeof raw !== "string") return null;
      const parsed = JSON.parse(raw);
      return {
        activeId: parsed.state?.activeId,
        sessions: parsed.state?.sessions?.map((session) => ({
          id: session.id,
          roles: session.messages?.map((message) => message.role),
        })),
      };
    });
    throw new Error(`Run-ledger fixture should render two existing work drawers; found ${workGroupCount}: ${JSON.stringify({ assistantText, persisted })}.`);
  }
  const ledgerGroup = workGroups.filter({ hasText: "Read ledger file" });
  const summary = await ledgerGroup.locator("summary").innerText();
  if (!summary.includes("Worked for") || summary.includes("e2e-ledger-model")) {
    throw new Error(`Closed run summary gained diagnostic chrome: ${summary}.`);
  }
  if (await page.getByRole("button", { name: "Run details", exact: true }).isVisible().catch(() => false)) {
    throw new Error("Run details should stay hidden until the existing work drawer is opened.");
  }

  await ledgerGroup.locator("summary").click();
  const action = ledgerGroup.getByRole("button", { name: "Run details", exact: true });
  await action.waitFor();
  if ((await page.locator(".stream-run-details-action").count()) !== 1) {
    throw new Error("Legacy work should omit the Run details action.");
  }
  if (requests.length !== 0) {
    throw new Error("Opening the work drawer should not fetch the ledger.");
  }
  await action.click();
  const details = page.getByTestId("assistant-run-details");
  await details.waitFor();
  await details.getByText("Composition", { exact: true }).waitFor();
  await details.getByText("Model steps", { exact: true }).waitFor();
  const inspectionRequests = requests.filter((request) => !request.includes("/events"));
  const eventRequests = requests.filter((request) => request.includes("/events?limit=100"));
  if (inspectionRequests.length !== 1 || eventRequests.length !== 1 || requests.length !== 2) {
    throw new Error(`Run details should make one inspection and one bounded event request: ${JSON.stringify(requests)}.`);
  }

  const commandBodies = [];
  const bootstrap = JSON.parse(readFileSync(join(root, "..", "..", "contracts", "control-v1", "bootstrap.json"), "utf8"));
  bootstrap.capabilities = {
    ...bootstrap.capabilities,
    run_ledger: true,
    run_inspection: true,
    steering: true,
    context_injection: true,
  };
  bootstrap.threads = [{
    id: "e2e-ledger-thread",
    title: "Quiet run details fixture",
    revision: 7,
    epoch: "e2e-ledger-epoch",
    updated_at_ms: Date.now(),
    archived_at_ms: null,
    model: "mock-echo",
    reasoning_effort_overrides: {},
    agent_id: null,
    workspace: null,
    busy: true,
    queued_turns: 0,
  }];
  bootstrap.models = [{ id: "mock-echo", object: "model", created: 0, owned_by: "milim" }];
  bootstrap.active_runs = [{
    id: "e2e-active-provider-run",
    thread_id: "e2e-ledger-thread",
    status: "running",
    adapter: "provider",
    config: {
      model: "mock-echo",
      instructions: "",
      workspace: null,
      privacy: "off",
      approval_mode: "review",
      plan_mode: false,
      sandbox: false,
      computer_use: false,
      memory: false,
      delegation_policy: "off",
      worker_model: "",
      agent: null,
      tool_mode: "all",
      enabled_tools: [],
      skill_mode: "auto",
      enabled_skills: [],
      attachments: [],
      native_session_id: null,
      reasoning_effort: null,
      adapter: "provider",
    },
    capabilities: {
      ledger: true,
      inspectable: true,
      steering: true,
      visibility: "model_visible",
    },
    created_at_ms: Date.now() - 1000,
    updated_at_ms: Date.now(),
    completed_at_ms: null,
    error: null,
  }];
  bootstrap.queued_turns = [];
  bootstrap.pending_inputs = [];
  bootstrap.pending_approvals = [];
  await page.route("**/control/v1/bootstrap", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify(bootstrap),
  }));
  await page.route("**/control/v1/threads/e2e-ledger-thread/timeline**", (route) => route.fulfill({
    status: 200,
    contentType: "application/json",
    body: JSON.stringify({
      thread_id: "e2e-ledger-thread",
      epoch: "e2e-ledger-epoch",
      first_seq: null,
      last_seq: null,
      has_more_before: false,
      has_more_after: false,
      items: [],
    }),
  }));
  await page.route("**/control/v1/commands", async (route) => {
    const body = JSON.parse(route.request().postData() ?? "{}");
    commandBodies.push(body);
    if (body.kind === "turn.send") {
      bootstrap.queued_turns = [{
        id: "e2e-followup",
        thread_id: body.thread_id,
        command_id: body.command_id,
        accepted_at_ms: Date.now(),
        display_text: body.payload?.display_text ?? body.payload?.text ?? "",
        attachments: body.payload?.attachments ?? [],
      }];
      bootstrap.threads[0].queued_turns = 1;
    } else if (body.kind === "turn.queue_delete") {
      bootstrap.queued_turns = bootstrap.queued_turns.filter((turn) => turn.id !== body.payload?.queue_id);
      bootstrap.threads[0].queued_turns = bootstrap.queued_turns.length;
    }
    const status = body.kind === "turn.send"
      ? "queued"
      : body.kind === "turn.queue_delete" || body.kind === "turn.queue_move"
        ? "applied"
        : "accepted";
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        command_id: body.command_id,
        status,
        thread_id: body.thread_id,
        revision: 7,
        run_id: body.kind === "turn.steer" ? "e2e-active-provider-run" : null,
        queue_id: body.kind === "turn.send" ? "e2e-followup" : null,
        confirmation_token: null,
        message: null,
        data: body.kind === "turn.steer" ? { inbox_id: "e2e-steer" } : null,
      }),
    });
  });
  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.getByLabel("Queue message").waitFor();
  const composer = page.getByTestId("composer-input");
  await composer.fill("Queue this follow-up");
  await page.getByLabel("Queue message").click();
  await page.waitForFunction(() => document.body.textContent?.includes("Message queued"));
  const queuedRow = page.getByTestId("queued-message").filter({ hasText: "Queue this follow-up" });
  await queuedRow.waitFor();
  await queuedRow.getByRole("button", { name: "Interrupt", exact: true }).waitFor();
  await queuedRow.getByLabel("Remove queued message").waitFor();
  await queuedRow.getByLabel("More queued message actions").waitFor();
  await composer.fill("Steer with this input");
  await page.getByLabel("More actions for active run").click();
  await page.getByRole("menuitem", { name: "Steer next step" }).click();
  await page.waitForFunction(() => document.body.textContent?.includes("Steering will be applied"));
  if (commandBodies.length !== 2 || commandBodies[0].kind !== "turn.send" || commandBodies[1].kind !== "turn.steer") {
    throw new Error(`Busy composer should queue first and steer only explicitly: ${JSON.stringify(commandBodies)}.`);
  }
  if (commandBodies[1].payload?.run_id !== "e2e-active-provider-run") {
    throw new Error("Steering should target the exact active run id.");
  }

  await page.reload();
  await page.getByTestId("chat-shell").waitFor();
  await page.getByLabel("Queue message").waitFor();
  await page.getByLabel("More actions for active run").waitFor();
  const restoredQueuedRow = page.getByTestId("queued-message").filter({ hasText: "Queue this follow-up" });
  await restoredQueuedRow.waitFor();
  await restoredQueuedRow.getByRole("button", { name: "Interrupt", exact: true }).waitFor();
  await restoredQueuedRow.getByLabel("Remove queued message").waitFor();
  await composer.fill("");
  await restoredQueuedRow.getByLabel("More queued message actions").click();
  await page.getByRole("menuitem", { name: "Edit queued message" }).click();
  await page.waitForFunction(() =>
    document.querySelector('[data-testid="composer-input"]')?.value === "Queue this follow-up",
  );
  for (let attempt = 0; attempt < 40 && commandBodies.at(-1)?.kind !== "turn.queue_delete"; attempt += 1) {
    await delay(50);
  }
  if (commandBodies.at(-1)?.kind !== "turn.queue_delete") {
    throw new Error(`Editing a canonical queued message should delete it durably first: ${JSON.stringify(commandBodies)}.`);
  }
  await runComposerAutocompleteDismissalCheck(page, composer);
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
      (response.status() === 502 && /\/codex\/(?:models|account)\b/.test(url)) ||
      (response.status() === 400 && /\/(?:codex|claude|opencode|pi)\/run\b/.test(url))
    ) {
      expectedFailedResources.push(url);
    }
  });
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    const url = msg.location().url;
    if (text.includes("Content Security Policy directive") && text.includes("http://[::1]:*")) return;
    if (url === "about:srcdoc" && text.includes("Content Security Policy directive")) return;
    if (text.includes("404") && /\/favicon\.ico$/.test(url)) return;
    if (text.includes("favicon.ico") && text.includes("Content Security Policy directive")) return;
    if (/^Failed to load resource: the server responded with a status of (?:400 \(Bad Request\)|502 \(Bad Gateway\))$/.test(text)) {
      const responseIndex = expectedFailedResources.findIndex((resourceUrl) => !url || resourceUrl === url);
      if (/\/(?:media\/(?:models|model-schema)|codex\/(?:models|account))\b/.test(url) || responseIndex >= 0) {
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
  console.log(`settingsMinimumScreenshot=${screenshots.settingsMinimum}`);
  console.log(`settingsThemeEditorScreenshot=${screenshots.settingsThemeEditor}`);
  console.log(`chatScreenshot=${screenshots.chat}`);
  console.log(`zoomScreenshot=${screenshots.zoom}`);
  console.log(`accountUsageScreenshot=${screenshots.accountUsage}`);
  console.log(`microUiScreenshot=${screenshots.microUi}`);
  console.log(`resizeHandlesScreenshot=${screenshots.resizeHandles}`);
  console.log(`inspectorOverlayScreenshot=${screenshots.inspectorOverlay}`);
  console.log(`workersPlanScreenshot=${screenshots.workersPlan}`);
  console.log(`workersNarrowScreenshot=${screenshots.workersNarrow}`);
  console.log(`newChatSplitScreenshot=${screenshots.newChatSplit}`);
  console.log(`mcpAppsLightScreenshot=${screenshots.mcpAppsLight}`);
  console.log(`mcpAppsDarkScreenshot=${screenshots.mcpAppsDark}`);
  console.log(`nativeChartLightScreenshot=${screenshots.nativeChartLight}`);
  console.log(`nativeChartDarkScreenshot=${screenshots.nativeChartDark}`);
  console.log(`nativeChartNarrowScreenshot=${screenshots.nativeChartNarrow}`);
  console.log(`turnChangesScreenshot=${screenshots.turnChanges}`);
  console.log(`threadBarTopScreenshot=${screenshots.threadBarTop}`);
  console.log(`threadBarBottomScreenshot=${screenshots.threadBarBottom}`);
  console.log(`chatSourcesScreenshot=${screenshots.chatSources}`);
  console.log(`chatLatestScreenshot=${screenshots.chatLatest}`);
  console.log(`reasoningEffortScreenshot=${screenshots.reasoningEffort}`);
  console.log(`linkedThreadDropScreenshot=${screenshots.linkedThreadDrop}`);
  for (const theme of ["light", "dark"]) {
    for (const kind of mcpAppKinds) console.log(`mcpApp${kind}Screenshot(${theme})=${mcpAppViewScreenshot(kind, theme)}`);
  }
  console.log(`failureScreenshot=${screenshots.failure}`);
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
