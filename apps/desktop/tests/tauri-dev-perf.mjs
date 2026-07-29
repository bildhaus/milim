import { spawn, spawnSync } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";
import { chromium } from "playwright-core";

const root = fileURLToPath(new URL("..", import.meta.url));
const tauriCli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const cdpHost = "127.0.0.1";
const cdpPort = Number(process.env.MILIM_TAURI_DEV_PERF_CDP_PORT || 9555);
const cdpUrl = `http://${cdpHost}:${cdpPort}`;
const artifactDir = process.env.MILIM_PERF_ARTIFACT_DIR || join(tmpdir(), `milim-tauri-dev-perf-${Date.now()}`);
const screenshotPaths = {
  empty: join(artifactDir, "empty-chat.png"),
  configured: join(artifactDir, "model-configured.png"),
  providers: join(artifactDir, "providers.png"),
  midStream: join(artifactDir, "mid-stream.png"),
  completed: join(artifactDir, "completed-chat.png"),
};
const metricsPath = join(artifactDir, "metrics.json");
const modelId = "perf-mock";
const consoleErrors = [];

if (process.platform !== "win32") {
  console.log("Skipping Tauri dev perf benchmark: this runner targets Windows WebView2.");
  process.exit(0);
}

if (!existsSync(tauriCli)) {
  throw new Error(`Tauri CLI not found: ${tauriCli}`);
}

if (await isPortOpen(cdpPort)) {
  throw new Error(`CDP port ${cdpPort} is already in use.`);
}

mkdirSync(artifactDir, { recursive: true });
const milimHome = mkdtempSync(join(tmpdir(), "milim-tauri-dev-perf-home-"));
const fakeProvider = await startFakeOpenAiProvider();
let session;
let failure;

try {
  const startedAt = Date.now();
  session = await launchTauriDev(milimHome);
  collectErrors(session.page, consoleErrors);
  await enablePerfAndBypassOnboarding(session.page);
  await session.page.getByTestId("chat-shell").waitFor({ timeout: 60_000 });
  const bootReadyAt = Date.now();
  await assertLayout(session.page, "empty");
  await session.page.screenshot({ path: screenshotPaths.empty, fullPage: false });

  const providerSetupStartedAt = Date.now();
  await configureProvider(session.page, fakeProvider.baseUrl);
  await reloadPage(session.page);
  await session.page.getByTestId("chat-shell").waitFor({ timeout: 60_000 });
  await selectModel(session.page, modelId);
  const providerSetupEndedAt = Date.now();
  await assertLayout(session.page, "configured");
  await session.page.screenshot({ path: screenshotPaths.configured, fullPage: false });
  await screenshotProviders(session.page);

  await installRuntimeSamplers(session.page);
  await session.page.evaluate(() => window.__MILIM_PERF__?.reset());

  const sendStartedAt = Date.now();
  await session.page.getByTestId("composer-input").fill("Run the deterministic perf benchmark response.");
  await session.page.getByTestId("composer-send").click();
  await session.page.getByTestId("assistant-message").last().waitFor({ timeout: 60_000 });
  await session.page.getByTestId("assistant-message").last().getByText("Perf response").waitFor({ timeout: 60_000 });
  const firstTokenAt = Date.now();
  await session.page.getByTestId("assistant-message").last().getByText("function fibonacci").first().waitFor({ timeout: 60_000 });
  await session.page.screenshot({ path: screenshotPaths.midStream, fullPage: false });
  await session.page.getByRole("button", { name: "Stop generating" }).waitFor({ state: "hidden", timeout: 90_000 });
  await session.page.getByTestId("assistant-message").last().getByText("PERF_DONE").waitFor({ timeout: 30_000 });
  const streamCompletedAt = Date.now();
  await assertLayout(session.page, "completed");
  await session.page.screenshot({ path: screenshotPaths.completed, fullPage: false });

  if (consoleErrors.length) {
    throw new Error(`Console errors during Tauri dev perf benchmark:\n${consoleErrors.join("\n")}`);
  }

  const metrics = {
    runtime: "tauri-dev-webview2",
    platform: process.platform,
    cdpPort,
    artifactDir,
    fakeProviderRequests: fakeProvider.requests,
    timingsMs: {
      launchToChatShell: bootReadyAt - startedAt,
      providerSetup: providerSetupEndedAt - providerSetupStartedAt,
      sendToFirstToken: firstTokenAt - sendStartedAt,
      streamDuration: streamCompletedAt - firstTokenAt,
      sendToDone: streamCompletedAt - sendStartedAt,
    },
    layout: await collectLayoutMetrics(session.page),
    browser: await collectRuntimeMetrics(session.page),
    screenshots: screenshotPaths,
  };
  writeFileSync(metricsPath, JSON.stringify(metrics, null, 2));
  console.log(`metrics=${metricsPath}`);
  for (const [name, path] of Object.entries(screenshotPaths)) console.log(`${name}Screenshot=${path}`);
} catch (err) {
  failure = err;
} finally {
  fakeProvider.close();
  if (session) await closeSession(session).catch(() => {});
  rmWithRetry(milimHome);
}

if (failure) throw failure;

async function startFakeOpenAiProvider() {
  const requests = [];
  const server = createServer((req, res) => {
    requests.push({ method: req.method, url: req.url, at: Date.now() });
    if (req.method === "GET" && (req.url === "/v1/models" || req.url === "/models")) {
      json(res, {
        object: "list",
        data: [{ id: modelId, object: "model", created: 0, owned_by: "perf" }],
      });
      return;
    }
    if (req.method === "POST" && (req.url === "/v1/chat/completions" || req.url === "/chat/completions")) {
      void readBody(req).then(() => streamCompletion(res));
      return;
    }
    res.writeHead(404).end("not found");
  });
  await new Promise((resolve) => server.listen(0, cdpHost, resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Fake provider did not bind a TCP port.");
  return {
    baseUrl: `http://${cdpHost}:${address.port}/v1`,
    requests,
    close: () => server.close(),
  };
}

function streamCompletion(res) {
  const chunks = chunkText(perfCompletionText(), Number(process.env.MILIM_PERF_CHUNK_SIZE || 48));
  const delayMs = Number(process.env.MILIM_PERF_CHUNK_DELAY_MS || 6);
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  let index = 0;
  const writeNext = () => {
    if (index < chunks.length) {
      const content = chunks[index++];
      res.write(`data: ${JSON.stringify({
        id: "perf-chatcmpl",
        object: "chat.completion.chunk",
        created: 0,
        model: modelId,
        choices: [{ index: 0, delta: { content }, finish_reason: null }],
      })}\n\n`);
      setTimeout(writeNext, delayMs);
      return;
    }
    res.write(`data: ${JSON.stringify({
      id: "perf-chatcmpl",
      object: "chat.completion.chunk",
      created: 0,
      model: modelId,
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 24, completion_tokens: chunks.length, total_tokens: chunks.length + 24 },
    })}\n\n`);
    res.write("data: [DONE]\n\n");
    res.end();
  };
  writeNext();
}

function perfCompletionText() {
  const section = [
    "## TypeScript block",
    "",
    "```ts",
    "export function fibonacci(n: number): number {",
    "  if (n <= 1) return n;",
    "  return fibonacci(n - 1) + fibonacci(n - 2);",
    "}",
    "```",
    "",
    "## JSON block",
    "",
    "```json",
    "{\"status\":\"ok\",\"items\":[{\"id\":1,\"label\":\"alpha\"},{\"id\":2,\"label\":\"beta\"}]}",
    "```",
    "",
    "| metric | value |",
    "| --- | ---: |",
    "| rows | 128 |",
    "| latency_ms | 42 |",
    "",
  ].join("\n");
  return [
    "# Perf response",
    "",
    "This deterministic response stresses streaming markdown, code fences, tables, and final highlighting.",
    "",
    ...Array.from({ length: 18 }, (_, index) => `### Section ${index + 1}\n\n${section}`),
    "PERF_DONE",
  ].join("\n");
}

function chunkText(text, size) {
  const chunks = [];
  for (let i = 0; i < text.length; i += size) chunks.push(text.slice(i, i + size));
  return chunks;
}

async function configureProvider(page, baseUrl) {
  const api = await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke API unavailable.");
    return {
      base: await invoke("api_base_url"),
      token: await invoke("api_token"),
    };
  });
  const headers = { "Content-Type": "application/json" };
  if (api.token) headers.Authorization = `Bearer ${api.token}`;
  const response = await fetch(`${api.base}/providers`, {
    method: "POST",
    headers,
    body: JSON.stringify({
      name: "Perf Mock",
      kind: "openai_compatible",
      base_url: baseUrl,
      enabled: true,
    }),
  });
  if (!response.ok) throw new Error(`Failed to save fake provider: ${response.status} ${await response.text()}`);
  const saved = await response.json();
  if (!saved.models?.includes(modelId)) throw new Error(`Fake provider saved without ${modelId}: ${JSON.stringify(saved)}`);
  await waitForBackendModel(api.base, api.token);
}

async function waitForBackendModel(base, token) {
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  const started = Date.now();
  while (Date.now() - started < 10_000) {
    const response = await fetch(`${base}/v1/models`, { headers });
    if (response.ok) {
      const body = await response.json();
      if ((body.data ?? []).some((model) => model.id === modelId)) return;
    }
    await delay(200);
  }
  throw new Error(`Timed out waiting for ${modelId} in backend model list.`);
}

async function selectModel(page, model) {
  await page.getByTestId("model-picker-trigger").click();
  await page.locator(".mp-item", { hasText: model }).locator(".mp-pick").click();
  await page.waitForFunction((id) => document.querySelector('[data-testid="model-picker-trigger"]')?.textContent?.includes(id), model);
}

async function screenshotProviders(page) {
  await page.getByTestId("model-picker-trigger").click();
  await page.getByTestId("manage-providers").click();
  await page.getByTestId("provider-overview").waitFor();
  await page.locator(".providers-sheet").getByText("Perf Mock", { exact: true }).waitFor();
  await page.screenshot({ path: screenshotPaths.providers, fullPage: false });
  await page.getByTestId("close-providers").click();
}

async function enablePerfAndBypassOnboarding(page) {
  await page.waitForFunction(() => Boolean(window.__TAURI_INTERNALS__?.invoke), { timeout: 60_000 });
  await page.evaluate(async () => {
    const perfKey = "milim.perf";
    const onboardingKey = "milim.onboarding";
    const value = JSON.stringify({
      state: {
        version: 1,
        status: "completed",
        selectedSetupPath: null,
        completedSteps: ["finish"],
        developerShowOnboarding: false,
        completedAt: Date.now(),
      },
      version: 0,
    });
    localStorage.setItem(perfKey, "1");
    localStorage.setItem(onboardingKey, value);
    await window.__TAURI_INTERNALS__.invoke("user_state_set", { key: onboardingKey, value });
  });
  await reloadPage(page);
}

async function reloadPage(page) {
  try {
    await page.reload({ waitUntil: "domcontentloaded", timeout: 30_000 });
  } catch (err) {
    const ready = await page.evaluate(() => document.readyState !== "loading").catch(() => false);
    if (!ready) throw err;
  }
}

async function installRuntimeSamplers(page) {
  await page.evaluate(() => {
    const state = {
      frames: [],
      longTasks: [],
      running: true,
    };
    window.__MILIM_PERF_RUNTIME__ = state;
    let last = performance.now();
    function tick(now) {
      state.frames.push(now - last);
      last = now;
      if (state.running) requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
    try {
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          state.longTasks.push({ startTime: entry.startTime, duration: entry.duration, name: entry.name });
        }
      });
      observer.observe({ entryTypes: ["longtask"] });
      state.longTaskObserver = observer;
    } catch {
      state.longTaskObserver = null;
    }
  });
}

async function collectRuntimeMetrics(page) {
  return await page.evaluate(() => {
    const runtime = window.__MILIM_PERF_RUNTIME__ ?? { frames: [], longTasks: [] };
    runtime.running = false;
    runtime.longTaskObserver?.disconnect?.();
    const frames = runtime.frames.slice(1);
    const sortedFrames = [...frames].sort((a, b) => a - b);
    const percentile = (p) => sortedFrames.length ? sortedFrames[Math.min(sortedFrames.length - 1, Math.floor(sortedFrames.length * p))] : 0;
    const longTaskTotalMs = runtime.longTasks.reduce((sum, entry) => sum + entry.duration, 0);
    return {
      perf: window.__MILIM_PERF__?.snapshot?.() ?? null,
      frames: {
        count: frames.length,
        maxMs: Math.max(0, ...frames),
        p95Ms: percentile(0.95),
        p99Ms: percentile(0.99),
        over32ms: frames.filter((value) => value > 32).length,
        over50ms: frames.filter((value) => value > 50).length,
      },
      longTasks: {
        count: runtime.longTasks.length,
        totalMs: longTaskTotalMs,
        maxMs: Math.max(0, ...runtime.longTasks.map((entry) => entry.duration)),
      },
    };
  });
}

async function assertLayout(page, label) {
  const metrics = await collectLayoutMetrics(page);
  if (!metrics.chatShell || metrics.chatShell.width < 400 || metrics.chatShell.height < 300) {
    throw new Error(`${label}: chat shell layout is invalid: ${JSON.stringify(metrics.chatShell)}`);
  }
  if (!metrics.composer || metrics.composer.width < 240 || metrics.composer.height < 70) {
    throw new Error(`${label}: composer layout is invalid: ${JSON.stringify(metrics.composer)}`);
  }
  if (label === "completed" && metrics.codeBlocks < 1) {
    throw new Error(`${label}: expected final markdown code blocks: ${JSON.stringify(metrics)}`);
  }
}

async function collectLayoutMetrics(page) {
  return await page.evaluate(() => {
    const rect = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const box = element.getBoundingClientRect();
      return { x: box.x, y: box.y, width: box.width, height: box.height };
    };
    return {
      viewport: { width: innerWidth, height: innerHeight },
      domNodes: document.querySelectorAll("*").length,
      messages: document.querySelectorAll(".msg").length,
      codeBlocks: document.querySelectorAll(".code-block").length,
      chatShell: rect('[data-testid="chat-shell"]'),
      sidebar: rect(".sidebar"),
      composer: rect('[data-testid="composer-drop-zone"]'),
      composerInput: rect('[data-testid="composer-input"]'),
      messageColumn: rect(".messages"),
      lastAssistant: rect('[data-testid="assistant-message"]:last-of-type'),
    };
  });
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
      MILIM_PERF: "1",
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
  session.page = await tauriPage(context, session);
  session.page.setDefaultTimeout(12_000);
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

async function tauriPage(context, session) {
  const started = Date.now();
  const seen = new Set();
  while (Date.now() - started < 60_000) {
    const pages = context.pages().filter((page) => !page.isClosed());
    for (const page of pages) {
      const hasInvoke = await page.evaluate(() => Boolean(window.__TAURI_INTERNALS__?.invoke)).catch(() => false);
      if (hasInvoke) return page;
      seen.add(page.url() || "<blank>");
    }
    if (!pages.length) {
      await context.waitForEvent("page", { timeout: 1_000 }).catch(() => {});
    } else {
      await delay(250);
    }
  }
  const targets = Array.from(seen).join("\n");
  throw new Error(`Timed out waiting for a Tauri WebView page with invoke bridge.\ntargets:\n${targets}\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
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

function json(res, body) {
  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => resolve(body));
    req.on("error", reject);
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
