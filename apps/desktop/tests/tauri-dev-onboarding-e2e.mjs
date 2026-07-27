import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";
import { chromium } from "playwright-core";

const root = fileURLToPath(new URL("..", import.meta.url));
const tauriCli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const cdpHost = "127.0.0.1";
const cdpPort = Number(process.env.MILIM_TAURI_DEV_E2E_CDP_PORT || 9444);
const cdpUrl = `http://${cdpHost}:${cdpPort}`;
const screenshots = {
  onboarding: join(tmpdir(), "milim-tauri-dev-onboarding.png"),
  skipped: join(tmpdir(), "milim-tauri-dev-onboarding-skipped.png"),
  failure: join(tmpdir(), "milim-tauri-dev-onboarding-failure.png"),
};

if (process.platform !== "win32") {
  console.log("Skipping Tauri dev onboarding E2E: this test currently targets Windows WebView2.");
  process.exit(0);
}

if (await isPortOpen(cdpPort)) {
  throw new Error(`CDP port ${cdpPort} is already in use.`);
}

const milimHome = mkdtempSync(join(tmpdir(), "milim-tauri-dev-onboarding-"));
const consoleErrors = [];
let session;
let failure;
let previousOnboardingState = null;

try {
  session = await launchTauriDev(milimHome);
  collectErrors(session.page, consoleErrors);
  previousOnboardingState = await setOnboardingOverride(session.page);
  await reloadForOnboarding(session.page);

  await waitForOnboarding(session.page);
  await assertOnboardingCoversApp(session.page);
  await assertStepperAboveContent(session.page);
  await assertStoryActionLayout(session.page);
  await session.page.screenshot({ path: screenshots.onboarding, fullPage: false });

  await session.page.getByRole("button", { name: "Skip for now" }).click();
  await session.page.getByTestId("onboarding-flow").waitFor({ state: "detached" });
  await session.page.getByTestId("composer-input").waitFor();
  await session.page.screenshot({ path: screenshots.skipped, fullPage: false });
  await restoreOnboardingState(session.page, previousOnboardingState);
  previousOnboardingState = null;

  previousOnboardingState = await setOnboardingOverride(session.page);
  await reloadForOnboarding(session.page);
  await waitForOnboarding(session.page);
  await completeOnboarding(session.page);
  await restoreOnboardingState(session.page, previousOnboardingState);
  previousOnboardingState = null;

  if (consoleErrors.length) {
    throw new Error(`Console errors during Tauri dev onboarding E2E:\n${consoleErrors.join("\n")}`);
  }

  console.log(`onboardingScreenshot=${screenshots.onboarding}`);
  console.log(`skippedScreenshot=${screenshots.skipped}`);
} catch (err) {
  failure = err;
} finally {
  if (session?.page && previousOnboardingState) {
    await restoreOnboardingState(session.page, previousOnboardingState).catch(() => {});
  }
  if (session) await closeSession(session).catch(() => {});
  rmWithRetry(milimHome);
}

if (failure) throw failure;

async function assertOnboardingCoversApp(page) {
  const cover = await page.evaluate(() => {
    const overlay = document.querySelector(".onboarding-overlay");
    const sheet = document.querySelector('[data-testid="onboarding-flow"]');
    if (!overlay || !sheet) return null;

    const overlayBox = overlay.getBoundingClientRect();
    const sheetBox = sheet.getBoundingClientRect();
    const overlayStyle = getComputedStyle(overlay);
    const appHit = document.elementFromPoint(24, 24)?.closest(".main, .sidebar, .content");
    const overlayHit = document.elementFromPoint(24, 24)?.closest(".onboarding-overlay");
    const color = overlayStyle.backgroundColor;
    const alpha = color.startsWith("rgba(") ? Number(color.split(",").at(-1)?.replace(")", "").trim()) : 1;

    return {
      alpha,
      color,
      overlayHeight: overlayBox.height,
      overlayWidth: overlayBox.width,
      sheetHeight: sheetBox.height,
      sheetWidth: sheetBox.width,
      sheetCenterX: (sheetBox.left + sheetBox.right) / 2,
      sheetCenterY: (sheetBox.top + sheetBox.bottom) / 2,
      viewportHeight: innerHeight,
      viewportWidth: innerWidth,
      topLeftHitsApp: Boolean(appHit),
      topLeftHitsOnboarding: Boolean(overlayHit),
    };
  });

  if (!cover) throw new Error("Onboarding overlay should exist.");
  if (cover.alpha !== 1) throw new Error(`Onboarding overlay should be opaque, got ${cover.color}.`);
  if (cover.overlayWidth < cover.viewportWidth - 1 || cover.overlayHeight < cover.viewportHeight - 1) {
    throw new Error(`Onboarding overlay should cover the viewport: ${JSON.stringify(cover)}`);
  }
  if (
    Math.abs(cover.sheetCenterX - cover.viewportWidth / 2) > cover.viewportWidth * 0.05 ||
    Math.abs(cover.sheetCenterY - cover.viewportHeight / 2) > cover.viewportHeight * 0.05 ||
    cover.sheetWidth > cover.viewportWidth ||
    cover.sheetHeight > cover.viewportHeight
  ) {
    throw new Error(`Onboarding sheet should be centered inside the viewport: ${JSON.stringify(cover)}`);
  }
  if (!cover.topLeftHitsOnboarding || cover.topLeftHitsApp) {
    throw new Error(`Onboarding should intercept the app before skip: ${JSON.stringify(cover)}`);
  }
}

async function waitForOnboarding(page) {
  try {
    await page.getByTestId("onboarding-flow").waitFor({ timeout: 60_000 });
  } catch (err) {
    const state = await page.evaluate(() => ({
      bodyText: document.body?.innerText.slice(0, 800) ?? "",
      hasChatShell: Boolean(document.querySelector('[data-testid="chat-shell"]')),
      hasOnboardingPreflight: Boolean(document.querySelector('[data-testid="onboarding-preflight"]')),
      hasOnboardingFlow: Boolean(document.querySelector('[data-testid="onboarding-flow"]')),
      title: document.title,
      url: location.href,
    })).catch((evalErr) => ({ evaluateError: String(evalErr) }));
    await page.screenshot({ path: screenshots.failure, fullPage: false }).catch(() => {});
    throw new Error(
      `Timed out waiting for onboarding in Tauri dev. state=${JSON.stringify(state)} failureScreenshot=${screenshots.failure}`,
      { cause: err },
    );
  }
}

async function assertStoryActionLayout(page) {
  const metrics = await page.evaluate(() => {
    const panel = document.querySelector("#onboarding-model-title")?.closest(".onboarding-panel");
    const story = panel?.querySelector(".onboarding-story");
    const action = panel?.querySelector(".onboarding-step-body");
    const wordmark = panel?.querySelector(".onboarding-wordmark");
    if (!panel || !story || !action || !wordmark) return null;
    const panelBox = panel.getBoundingClientRect();
    const storyBox = story.getBoundingClientRect();
    const actionBox = action.getBoundingClientRect();
    return {
      split: storyBox.right <= actionBox.left || actionBox.right <= storyBox.left,
      storyCentered: Math.abs((storyBox.top + storyBox.bottom) / 2 - (panelBox.top + panelBox.bottom) / 2),
      actionCentered: Math.abs((actionBox.top + actionBox.bottom) / 2 - (panelBox.top + panelBox.bottom) / 2),
      limit: panelBox.height * 0.12,
      panelHeight: panelBox.height,
    };
  });

  if (!metrics) throw new Error("Model onboarding story/action layout should exist.");
  if (!metrics.split || metrics.storyCentered > metrics.limit || metrics.actionCentered > metrics.limit) {
    throw new Error(`Model onboarding should use a centered story/action layout: ${JSON.stringify(metrics)}`);
  }
}

async function assertStepperAboveContent(page) {
  const metrics = await page.evaluate(() => {
    const steps = document.querySelector(".onboarding-steps");
    const content = document.querySelector(".onboarding-content");
    if (!steps || !content) return null;
    const stepBox = steps.getBoundingClientRect();
    const contentBox = content.getBoundingClientRect();
    return {
      horizontal: stepBox.width > stepBox.height * 3,
      aboveContent: stepBox.bottom <= contentBox.top + 1,
      stepWidth: stepBox.width,
      stepHeight: stepBox.height,
      contentTop: contentBox.top,
      stepBottom: stepBox.bottom,
    };
  });

  if (!metrics) throw new Error("Onboarding stepper should exist.");
  if (!metrics.horizontal || !metrics.aboveContent) {
    throw new Error(`Onboarding steps should render as a horizontal stepper above content: ${JSON.stringify(metrics)}`);
  }
}

async function setOnboardingOverride(page) {
  await page.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__?.invoke), { timeout: 60_000 });
  await page.waitForFunction(() => document.readyState !== "loading", { timeout: 60_000 });
  return await page.evaluate(async () => {
    const key = "milim.onboarding";
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    const previousLocal = localStorage.getItem(key);
    const previousUser = invoke ? await invoke("user_state_get", { key }) : null;
    const value = JSON.stringify({
      state: {
        version: 1,
        status: "in_progress",
        selectedSetupPath: null,
        completedSteps: [],
        developerShowOnboarding: true,
      },
      version: 0,
    });

    localStorage.setItem(key, value);
    if (invoke) await invoke("user_state_set", { key, value });
    return {
      previousLocal: isInterruptedTestOverride(previousLocal) ? null : previousLocal,
      previousUser: isInterruptedTestOverride(previousUser) ? null : previousUser,
    };

    function isInterruptedTestOverride(raw) {
      if (typeof raw !== "string") return false;
      try {
        const state = JSON.parse(raw)?.state;
        return (
          state?.version === 1 &&
          state?.status === "in_progress" &&
          state?.selectedSetupPath == null &&
          state?.developerShowOnboarding === true &&
          Array.isArray(state?.completedSteps) &&
          state.completedSteps.length === 0
        );
      } catch {
        return false;
      }
    }
  });
}

async function reloadForOnboarding(page) {
  try {
    await page.reload({ waitUntil: "domcontentloaded", timeout: 30_000 });
  } catch (err) {
    const ready = await page.evaluate(() => document.readyState !== "loading").catch(() => false);
    if (!ready) throw err;
  }
}

async function restoreOnboardingState(page, previous) {
  await page.evaluate(async ({ previousLocal, previousUser }) => {
    const key = "milim.onboarding";
    const invoke = window.__TAURI_INTERNALS__?.invoke;

    if (previousLocal == null) localStorage.removeItem(key);
    else localStorage.setItem(key, previousLocal);

    if (!invoke) return;
    if (previousUser == null) await invoke("user_state_delete", { key });
    else await invoke("user_state_set", { key, value: previousUser });
  }, previous);
}

async function completeOnboarding(page) {
  await page.getByRole("heading", { name: "Connect a runtime" }).waitFor();
  await page.getByRole("button", { name: /Installed agents/ }).click();
  for (const runtime of ["codex", "claude", "opencode", "pi"]) {
    await page.getByTestId(`onboarding-runtime-${runtime}`).waitFor();
  }

  const modelSummary = await page.locator(".onboarding-model-summary").innerText();
  if (modelSummary.includes("No reachable model selected")) {
    console.log("completionChecks=skipped:no reachable model configured");
    return;
  }

  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("heading", { name: "Choose a workspace" }).waitFor();
  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("heading", { name: "Your workspace is ready" }).waitFor();
  await page.getByRole("button", { name: "Open Milim" }).click();
  await page.getByTestId("onboarding-flow").waitFor({ state: "detached" });
  await page.getByTestId("composer-input").waitFor();
}

function collectErrors(page, errors) {
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    const url = msg.location().url;
    if (text.includes("404") && /\/favicon\.ico$/.test(url)) return;
    errors.push(url ? `${text} (${url})` : text);
  });
  page.on("pageerror", (err) => errors.push(err.message));
}

async function launchTauriDev(milimHome) {
  const child = spawn(process.execPath, [tauriCli, "dev"], {
    cwd: root,
    env: {
      ...process.env,
      MILIM_HOME: milimHome,
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

  await waitForCdp(session, cdpUrl, 90_000);
  session.browser = await chromium.connectOverCDP(cdpUrl);
  const context = session.browser.contexts()[0] ?? await session.browser.newContext();
  session.page = await firstPage(context);
  session.page.setDefaultTimeout(10_000);
  return session;
}

async function closeSession(session) {
  await session.page?.locator(".win-close").click({ timeout: 1_000 }).catch(() => {});
  await waitForExit(session.child, 2_500).catch(() => {});
  await session.browser?.close().catch(() => {});
  if (session.child.exitCode == null) {
    killTree(session.child.pid);
    await waitForExit(session.child, 5_000).catch(() => {});
  }
}

function appendWebViewArg(existing, arg) {
  const trimmed = existing?.trim();
  return trimmed ? `${trimmed} ${arg}` : arg;
}

function isPortOpen(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: cdpHost, port });
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

async function waitForCdp(session, url, timeoutMs) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (session.child.exitCode != null) {
      throw new Error(`Tauri dev exited before CDP was ready.\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
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
  if (!pid) return;
  spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { stdio: "ignore" });
}

function waitForExit(proc, timeoutMs) {
  if (proc.exitCode != null) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Timed out waiting for Tauri dev process exit")), timeoutMs);
    proc.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

function rmWithRetry(path) {
  for (let i = 0; i < 20; i += 1) {
    try {
      rmSync(path, { recursive: true, force: true });
      return;
    } catch (err) {
      if (i === 19) throw err;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 500);
    }
  }
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
