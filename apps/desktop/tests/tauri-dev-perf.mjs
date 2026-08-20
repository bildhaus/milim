import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { cpus, release as osRelease, totalmem, tmpdir, version as osVersion } from "node:os";
import { isAbsolute, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import net from "node:net";
import { chromium } from "playwright-core";

const root = fileURLToPath(new URL("..", import.meta.url));
const binaryMode = process.argv.includes("--binary");
const tauriCli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const perfBinary =
  process.env.MILIM_TAURI_PERF_BINARY ||
  join(root, "src-tauri", "target", "release", "milim-desktop.exe");
const cdpHost = "127.0.0.1";
const cdpPort = Number(
  (binaryMode
    ? process.env.MILIM_TAURI_PERF_CDP_PORT
    : process.env.MILIM_TAURI_DEV_PERF_CDP_PORT) ||
    process.env.MILIM_TAURI_DEV_PERF_CDP_PORT ||
    9555,
);
const cdpUrl = `http://${cdpHost}:${cdpPort}`;
const artifactDir =
  process.env.MILIM_PERF_ARTIFACT_DIR ||
  (binaryMode
    ? join(root, "tester-artifacts", "runtime-evidence")
    : join(tmpdir(), `milim-tauri-dev-perf-${Date.now()}`));
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
  console.log(
    `Skipping ${binaryMode ? "canonical" : "Tauri dev"} perf benchmark: this runner targets Windows WebView2.`,
  );
  process.exit(0);
}

if (!binaryMode && !existsSync(tauriCli)) {
  throw new Error(`Tauri CLI not found: ${tauriCli}`);
}

if (!binaryMode && (await isPortOpen(cdpPort))) {
  throw new Error(`CDP port ${cdpPort} is already in use.`);
}

mkdirSync(artifactDir, { recursive: true });
if (binaryMode) await runCanonicalBinaryBenchmark();
else await runDevBenchmark();

async function runDevBenchmark() {
  const milimHome = mkdtempSync(
    join(tmpdir(), "milim-tauri-dev-perf-home-"),
  );
  const fakeProvider = await startFakeOpenAiProvider();
  let session;
  let failure;

  try {
    const startedAt = Date.now();
    session = await launchTauriDev(milimHome, consoleErrors);
    await enablePerfAndBypassOnboarding(session.page);
    await session.page.getByTestId("chat-shell").waitFor({ timeout: 60_000 });
    const bootReadyAt = Date.now();
    await assertLayout(session.page, "empty");
    await session.page.screenshot({
      path: screenshotPaths.empty,
      fullPage: false,
    });

    const providerSetupStartedAt = Date.now();
    await configureProvider(session.page, fakeProvider.baseUrl);
    await reloadPage(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 60_000 });
    await selectModel(session.page, modelId);
    const providerSetupEndedAt = Date.now();
    await assertLayout(session.page, "configured");
    await session.page.screenshot({
      path: screenshotPaths.configured,
      fullPage: false,
    });
    await screenshotProviders(session.page);

    await installRuntimeSamplers(session.page);
    await session.page.evaluate(() => window.__MILIM_PERF__?.reset());

    const sendStartedAt = Date.now();
    await session.page
      .getByTestId("composer-input")
      .fill("Run the deterministic perf benchmark response.");
    await session.page.getByTestId("composer-send").click();
    await session.page
      .getByTestId("assistant-message")
      .last()
      .waitFor({ timeout: 60_000 });
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText("Perf response")
      .waitFor({ timeout: 60_000 });
    const firstTokenAt = Date.now();
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText("function fibonacci")
      .first()
      .waitFor({ timeout: 60_000 });
    await session.page.screenshot({
      path: screenshotPaths.midStream,
      fullPage: false,
    });
    await session.page
      .getByRole("button", { name: "Stop generating" })
      .waitFor({ state: "hidden", timeout: 90_000 });
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText("PERF_DONE")
      .waitFor({ timeout: 30_000 });
    const streamCompletedAt = Date.now();
    await assertLayout(session.page, "completed");
    await session.page.screenshot({
      path: screenshotPaths.completed,
      fullPage: false,
    });

    if (consoleErrors.length) {
      throw new Error(
        `Console errors during Tauri dev perf benchmark:\n${consoleErrors.join("\n")}`,
      );
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
    for (const [name, path] of Object.entries(screenshotPaths))
      console.log(`${name}Screenshot=${path}`);
  } catch (err) {
    failure = err;
  } finally {
    fakeProvider.close();
    if (session) await closeSession(session).catch(() => {});
    rmWithRetry(milimHome);
  }

  if (failure) throw failure;
}

async function runCanonicalBinaryBenchmark() {
  const canonicalPath = join(artifactDir, "canonical-thread.json");
  const failurePath = join(artifactDir, "failure.json");
  const paths = {
    queued: join(artifactDir, "canonical-queued.png"),
    resumed: join(artifactDir, "canonical-resumed.png"),
    persisted: join(artifactDir, "canonical-persisted.png"),
    largeTranscript: join(artifactDir, "canonical-large-transcript.png"),
    failure: join(artifactDir, "failure.png"),
  };
  for (const path of [canonicalPath, failurePath, ...Object.values(paths)])
    rmSync(path, { force: true });
  const modelA = "perf-a";
  const modelB = "perf-b";
  const firstPrompt = "Begin the incomplete canonical response.";
  const queuedPrompt = "Resume this exact queued prompt after reload.";
  const partialMarker = "CANONICAL_A_PARTIAL.";
  const terminalMarker = "CANONICAL_B_DONE";
  const canonicalErrors = [];
  const assertNoCanonicalErrors = (stage) =>
    ensure(
      canonicalErrors.length === 0,
      `Console errors during canonical benchmark (${stage}):\n${canonicalErrors.join("\n")}`,
    );
  const milimHome = mkdtempSync(join(tmpdir(), "milim-canonical-perf-home-"));
  const report = {
    schema_version: 2,
    generated_at: new Date().toISOString(),
    commit_sha: currentCommitSha(),
    proof: "deterministic_mocked",
    benchmark: "canonical-thread",
    runtime: {
      mode: "release-optimized-binary",
      buildProfile: "release",
      platform: process.platform,
      arch: process.arch,
      cdpPort,
      binary: repoRelativePath(perfBinary),
      isolation: {
        milim_home: "private_temp",
        webview_profile: "private_temp",
        credential_environment: "scrubbed",
        account_runtime_executables: "deterministic_stubs",
      },
    },
    fixture: {
      version: "canonical-thread-v2",
      models: [modelA, modelB],
      threadCount: 10,
      messagesPerThread: 100,
    },
    timingsMs: {},
    continuity: {},
    renderer: {},
    cdp: {},
    processMemory: {},
    layout: {},
    native: null,
    fingerprint: buildFingerprint(),
    bundles: collectInitialBundleSizes(),
    screenshots: Object.fromEntries(
      Object.entries(paths)
        .filter(([name]) => name !== "failure")
        .map(([name, path]) => [name, artifactRelativePath(path)]),
    ),
  };
  let fakeProvider;
  let session;
  let failure;

  try {
    ensure(
      existsSync(perfBinary),
      `Tauri binary not found: ${perfBinary}. Run npm run verify:tauri first.`,
    );
    ensure(
      !(await isPortOpen(cdpPort)),
      `CDP port ${cdpPort} is already in use.`,
    );
    console.log("[canonical] launch prebuilt Tauri binary");
    fakeProvider = await startCanonicalFakeProvider({
      models: [modelA, modelB],
      partialMarker,
      terminalMarker,
    });

    const launchStartedAt = Date.now();
    session = await launchTauriBinary(milimHome, canonicalErrors);
    report.runtime.accountRuntimeDiscovery =
      await mockAccountRuntimeDiscovery(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    report.timingsMs.processToChatShell = Date.now() - launchStartedAt;
    await assertLayout(session.page, "canonical-startup");

    console.log("[canonical] configure mock provider and select perf-a");
    const setupStartedAt = Date.now();
    await enablePerfAndBypassOnboarding(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    await session.page.unroute("**/*");
    report.processMemory.initial = collectWindowsProcessTreeMemory(
      session.child.pid,
    );
    assertMockProcessIsolation(report.processMemory.initial);
    await configureProvider(session.page, fakeProvider.baseUrl, [
      modelA,
      modelB,
    ]);
    await reloadPage(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    await selectModel(session.page, modelA);
    const selectedA = await waitForPersistedState(
      session.page,
      (state) => {
        const active = activePersistedSession(state);
        return active && persistedModelMatches(active.settings?.model, modelA)
          ? { state, active }
          : null;
      },
      `selected model ${modelA}`,
    );
    const activeId = selectedA.state.activeId;
    ensure(activeId, "Canonical benchmark requires an active thread id.");
    report.continuity.activeThreadId = activeId;
    report.timingsMs.setupToModelA = Date.now() - setupStartedAt;
    await assertLayout(session.page, "canonical-configured");
    assertNoCanonicalErrors("configured");

    await installRuntimeSamplers(session.page);
    await session.page.evaluate(() => window.__MILIM_PERF__?.reset());

    await session.page.getByTestId("composer-input").fill(firstPrompt);
    const initialSendStartedAt = Date.now();
    await session.page.getByTestId("composer-send").click();
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText(partialMarker)
      .waitFor({ timeout: 20_000 });
    report.timingsMs.initialSendToFirstToken =
      Date.now() - initialSendStartedAt;
    assertNoCanonicalErrors("initial turn");

    console.log("[canonical] persist queued prompt");
    const queueStartedAt = Date.now();
    await session.page.getByTestId("composer-input").fill(queuedPrompt);
    await session.page.getByTestId("composer-send").click();
    const durableQueue = await waitForControlBootstrap(
      session.page,
      (bootstrap) => {
        const queue = bootstrap.queued_turns?.filter(
          (turn) => turn.thread_id === activeId,
        );
        return queue?.length === 1
          ? queue
          : null;
      },
      "canonical queued turn durability",
    );
    const queuedId = durableQueue[0]?.id;
    ensure(queuedId, "Canonical queued turn did not expose a durable id.");
    report.timingsMs.queueDurable = Date.now() - queueStartedAt;
    report.continuity.queuedId = queuedId;
    report.continuity.queueBeforeReload = durableQueue.map((item) => ({
      id: item.id,
      threadId: item.thread_id,
      commandId: item.command_id,
    }));
    await session.page.screenshot({ path: paths.queued, fullPage: false });
    report.renderer.beforeReload = await collectRuntimeMetrics(session.page);
    report.cdp.beforeReload = await collectCdpPerformance(session);
    report.processMemory.beforeReload = collectWindowsProcessTreeMemory(
      session.child.pid,
    );

    console.log("[canonical] reload reattaches to perf-a turn");
    const reloadStartedAt = Date.now();
    await reloadPage(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    const reloadedControl = await waitForControlBootstrap(
      session.page,
      (bootstrap) => {
        const queued = bootstrap.queued_turns?.find(
          (turn) => turn.id === queuedId && turn.thread_id === activeId,
        );
        const activeRun = bootstrap.active_runs?.find(
          (run) => run.thread_id === activeId,
        );
        return queued && activeRun ? { queued, activeRun } : null;
      },
      "canonical queue and active run after reload",
    );
    report.timingsMs.reloadToQueueDurable = Date.now() - reloadStartedAt;

    await waitForPersistedState(
      session.page,
      (state) => {
        const active = activePersistedSession(state);
        return state.activeId === activeId &&
          active &&
          persistedModelMatches(active.settings?.model, modelA)
          ? active
          : null;
      },
      "canonical state after reload",
    );
    ensure(
      reloadedControl.queued.id === queuedId,
      "Reloaded canonical queue id changed.",
    );
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText(partialMarker)
      .waitFor({ timeout: 20_000 });
    const stopButton = session.page.getByRole("button", {
      name: "Stop generating",
    });
    await stopButton.waitFor({ state: "visible", timeout: 20_000 });
    await waitForSelectedModel(session.page, modelA);
    await assertLayout(session.page, "canonical-queue-reload-running");
    report.continuity.sameThreadAfterReload = true;
    report.continuity.runningAfterReload = true;
    assertNoCanonicalErrors("running reload");

    console.log("[canonical] stop reattached perf-a turn");
    const stopStartedAt = Date.now();
    await stopButton.click();
    await stopButton.waitFor({ state: "hidden", timeout: 20_000 });
    const stoppedControl = await waitForControlBootstrap(
      session.page,
      (bootstrap) => {
        const queued = bootstrap.queued_turns?.find(
          (turn) => turn.id === queuedId && turn.thread_id === activeId,
        );
        const running = bootstrap.active_runs?.some(
          (run) => run.thread_id === activeId,
        );
        return queued && !running ? queued : null;
      },
      "canonical queue preservation after cancellation",
    );
    report.timingsMs.stopAfterReload = Date.now() - stopStartedAt;
    ensure(
      (await session.page
        .getByTestId("composer-send")
        .getAttribute("aria-label")) === "Send message",
      "Stopped canonical turn did not return the composer to a nonrunning state.",
    );
    await assertLayout(session.page, "canonical-queue-reload");
    report.timingsMs.inputToNextFrame =
      await measureComposerInputToNextFrame(session.page);
    report.continuity.queueAfterReload = [{
      id: stoppedControl.id,
      threadId: stoppedControl.thread_id,
      commandId: stoppedControl.command_id,
    }];
    report.continuity.stoppedAfterReload = true;

    await installRuntimeSamplers(session.page);
    await session.page.evaluate(() => window.__MILIM_PERF__?.reset());
    const switchStartedAt = Date.now();
    await selectModel(session.page, modelB);
    await waitForPersistedState(
      session.page,
      (state) => {
        const active = activePersistedSession(state);
        return active && persistedModelMatches(active.settings?.model, modelB)
          ? active
          : null;
      },
      `selected model ${modelB}`,
    );
    report.timingsMs.modelSwitch = Date.now() - switchStartedAt;

    console.log("[canonical] resume frozen queued prompt after switching to perf-b");
    const resumeStartedAt = Date.now();
    const resumed = await sendControlTestCommand(session.page, {
      command_id: `canonical-perf-resume-${Date.now()}`,
      kind: "turn.queue_resume",
      thread_id: activeId,
      payload: { queue_id: queuedId },
    });
    ensure(
      resumed.status === "accepted" && resumed.run_id,
      `Canonical queue resume failed: ${JSON.stringify(resumed)}.`,
    );
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText(terminalMarker)
      .waitFor({ timeout: 20_000 });
    report.timingsMs.resumeSendToFirstToken = Date.now() - resumeStartedAt;
    await session.page
      .getByRole("button", { name: "Stop generating" })
      .waitFor({ state: "hidden", timeout: 20_000 });
    report.timingsMs.resumeSendToDone = Date.now() - resumeStartedAt;
    await waitForCondition(
      () => fakeProvider.completions.length === 2,
      "two canonical upstream requests",
    );

    const upstreamModels = fakeProvider.completions.map(
      (request) => request.body?.model,
    );
    ensure(
      JSON.stringify(upstreamModels) === JSON.stringify([modelA, modelA]),
      `Canonical upstream models changed: ${JSON.stringify(upstreamModels)}.`,
    );
    const terminalState = await waitForPersistedState(
      session.page,
      (state) => {
        const active = activePersistedSession(state);
        const queue = state.queuedMessagesBySession?.[activeId] ?? [];
        if (
          !active ||
          !persistedModelMatches(active.settings?.model, modelB) ||
          queue.length !== 0
        ) {
          return null;
        }
        const queuedPromptCount = active.messages?.filter(
          (message) =>
            message.role === "user" && message.content === queuedPrompt,
        ).length;
        const terminalCount = active.messages?.filter(
          (message) =>
            message.role === "assistant" &&
            message.content?.includes(terminalMarker),
        ).length;
        return queuedPromptCount === 1 && terminalCount === 1 ? active : null;
      },
      "terminal canonical transcript",
    );
    ensure(
      (await session.page
        .getByTestId("user-message")
        .filter({ hasText: queuedPrompt })
        .count()) === 1,
      "Queued prompt rendered more than once.",
    );
    ensure(
      !(await session.page
        .getByTestId("queued-message-tray")
        .isVisible()
        .catch(() => false)),
      "Queued message tray did not drain.",
    );
    report.continuity.upstreamModels = upstreamModels;
    report.continuity.queuedModelFrozenBeforeSwitch = true;
    report.continuity.selectedModelAfterQueue = modelB;
    report.continuity.queuedPromptCount = terminalState.messages.filter(
      (message) =>
        message.role === "user" && message.content === queuedPrompt,
    ).length;
    report.continuity.queueDrained = true;
    report.continuity.terminalMarkerPersisted = true;
    await assertLayout(session.page, "canonical-resumed");
    await session.page.screenshot({ path: paths.resumed, fullPage: false });
    report.renderer.resumed = await collectRuntimeMetrics(session.page);
    report.cdp.resumed = await collectCdpPerformance(session);
    report.processMemory.resumed = collectWindowsProcessTreeMemory(
      session.child.pid,
    );

    console.log("[canonical] reload terminal transcript");
    const settledReloadStartedAt = Date.now();
    await reloadPage(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    await session.page
      .getByTestId("assistant-message")
      .last()
      .getByText(terminalMarker)
      .waitFor({ timeout: 20_000 });
    await waitForSelectedModel(session.page, modelB);
    report.timingsMs.settledReloadToTranscript =
      Date.now() - settledReloadStartedAt;
    const settledState = await readPersistedState(session.page);
    const settledActive = activePersistedSession(settledState);
    ensure(
      settledState.activeId === activeId &&
        settledActive &&
        settledActive.messages.filter(
          (message) =>
            message.role === "user" && message.content === queuedPrompt,
        ).length === 1 &&
        settledActive.messages.filter(
          (message) =>
            message.role === "assistant" &&
            message.content?.includes(terminalMarker),
        ).length === 1,
      "Settled canonical transcript did not survive reload.",
    );
    ensure(
      !(await session.page
        .getByTestId("queued-message-tray")
        .isVisible()
        .catch(() => false)),
      "Settled reload restored a drained queue.",
    );
    report.continuity.settledReloadPersisted = true;
    await assertLayout(session.page, "canonical-persisted");
    await session.page.screenshot({ path: paths.persisted, fullPage: false });
    assertNoCanonicalErrors("settled reload");

    console.log("[canonical] verify 10 x 100 persisted-message fixture");
    await writeLargeTranscriptFixture(session.page, activeId, {
      threadCount: report.fixture.threadCount,
      messagesPerThread: report.fixture.messagesPerThread,
    });
    const fixtureReloadStartedAt = Date.now();
    await reloadPage(session.page);
    await session.page
      .getByTestId("chat-shell")
      .waitFor({ timeout: 20_000 });
    await session.page
      .locator(".messages .msg")
      .last()
      .waitFor({ timeout: 20_000 });
    await installRuntimeSamplers(session.page);
    await waitForAnimationFrames(session.page, 2);
    report.timingsMs.fixtureReloadToInteractive =
      Date.now() - fixtureReloadStartedAt;

    const fixtureState = await readPersistedState(session.page);
    ensure(
      fixtureState.activeId === activeId,
      "Large transcript fixture changed the active canonical thread.",
    );
    ensure(
      fixtureState.sessions?.length === report.fixture.threadCount,
      `Expected ${report.fixture.threadCount} persisted threads, got ${fixtureState.sessions?.length ?? 0}.`,
    );
    const activeFixture = activePersistedSession(fixtureState);
    ensure(activeFixture, "Large transcript active fixture is unavailable.");
    const syntheticFixtures = fixtureState.sessions.filter(
      (item) => item.id !== activeId,
    );
    ensure(
      syntheticFixtures.length === report.fixture.threadCount - 1 &&
        syntheticFixtures.every(
          (item) =>
            item.messages?.length === report.fixture.messagesPerThread,
        ),
      `Every synthetic fixture thread must persist exactly ${report.fixture.messagesPerThread} messages.`,
    );
    ensure(
      activeFixture.messages.length >= report.fixture.messagesPerThread,
      `The active canonical fixture persisted ${activeFixture.messages.length} messages; expected at least ${report.fixture.messagesPerThread}.`,
    );
    const expectedRenderedRows = activeFixture.messages.filter(
      (message) => !isHiddenWorkerRunSynthesis(message),
    ).length;
    const renderedMessageRows = session.page.locator(
      ".messages .transcript-window-row",
    );
    await waitForCondition(
      async () => (await renderedMessageRows.count()) >= expectedRenderedRows,
      `at least ${expectedRenderedRows} mounted large-transcript rows`,
      20_000,
    );
    const renderedRows = await renderedMessageRows.count();
    ensure(
      renderedRows >= expectedRenderedRows,
      `Large transcript rendered ${renderedRows} rows; expected at least ${expectedRenderedRows} mounted rows from ${report.fixture.messagesPerThread} persisted messages.`,
    );
    report.fixture.persistedThreadCount = fixtureState.sessions.length;
    report.fixture.persistedMessagesPerThread = fixtureState.sessions.map(
      (item) => item.messages.length,
    );
    report.fixture.renderedMessageRows = renderedRows;
    await assertLayout(session.page, "canonical-large-transcript");
    report.layout.largeTranscript = await collectLayoutMetrics(session.page);
    report.renderer.largeTranscript = await collectRuntimeMetrics(session.page);
    report.cdp.largeTranscript = await collectCdpPerformance(session);
    report.processMemory.largeTranscript = collectWindowsProcessTreeMemory(
      session.child.pid,
    );
    assertNoCanonicalErrors("large transcript reload");
    await session.page.screenshot({
      path: paths.largeTranscript,
      fullPage: false,
    });

    console.log("[canonical] send from large transcript");
    const longThreadPrompt = "Canonical long-thread optimistic send.";
    await session.page.getByTestId("composer-input").fill(longThreadPrompt);
    await session.page.evaluate((prompt) => {
      const scroll = document.querySelector(".chat-scroll");
      if (!(scroll instanceof HTMLElement))
        throw new Error("Chat scroll container unavailable.");
      scroll.scrollTop = scroll.scrollHeight;
      window.__MILIM_LONG_THREAD_SCROLL_SAMPLES__ = [];
      window.__MILIM_OPTIMISTIC_SEND_MS__ = new Promise((resolve, reject) => {
        const send = document.querySelector('[data-testid="composer-send"]');
        if (!(send instanceof HTMLElement)) {
          reject(new Error("Composer send button unavailable."));
          return;
        }
        send.addEventListener("click", () => {
          const startedAt = performance.now();
          const observer = new MutationObserver(() => {
            const visible = [...document.querySelectorAll('[data-testid="user-message"]')]
              .some((node) => node.textContent?.includes(prompt));
            if (!visible) return;
            observer.disconnect();
            requestAnimationFrame(() => resolve(performance.now() - startedAt));
          });
          observer.observe(document.querySelector(".messages") ?? document.body, {
            childList: true,
            subtree: true,
            characterData: true,
          });
        }, { once: true, capture: true });
      });
      let remaining = 45;
      const sample = () => {
        window.__MILIM_LONG_THREAD_SCROLL_SAMPLES__.push(
          scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight,
        );
        remaining -= 1;
        if (remaining > 0) requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    }, longThreadPrompt);
    await session.page.getByTestId("composer-send").click();
    await session.page
      .getByTestId("user-message")
      .filter({ hasText: longThreadPrompt })
      .waitFor({ timeout: 10_000 });
    report.timingsMs.longThreadSendToOptimistic = await session.page.evaluate(
      () => window.__MILIM_OPTIMISTIC_SEND_MS__,
    );
    await session.page
      .getByRole("button", { name: "Stop generating" })
      .waitFor({ state: "hidden", timeout: 20_000 });
    await waitForAnimationFrames(session.page, 45);
    const bottomGaps = await session.page.evaluate(
      () => window.__MILIM_LONG_THREAD_SCROLL_SAMPLES__,
    );
    report.fixture.longThreadMaxBottomGap = Math.max(...bottomGaps);
    ensure(
      report.fixture.longThreadMaxBottomGap <= 32,
      `Large transcript lost bottom follow by ${report.fixture.longThreadMaxBottomGap}px.`,
    );
    ensure(
      (await session.page
        .getByTestId("user-message")
        .filter({ hasText: longThreadPrompt })
        .count()) === 1,
      "Long-thread optimistic prompt rendered more than once.",
    );
    await waitForCondition(
      () => fakeProvider.completions.length === 3,
      "long-thread canonical completion",
    );

    const finalUpstreamModels = fakeProvider.completions.map(
      (request) => request.body?.model,
    );
    ensure(
      JSON.stringify(finalUpstreamModels) ===
        JSON.stringify([modelA, modelA, modelB]),
      `Canonical upstream models changed after reload: ${JSON.stringify(finalUpstreamModels)}.`,
    );
    report.continuity.upstreamModels = finalUpstreamModels;
    report.native = await session.page.evaluate(async () =>
      await window.__TAURI_INTERNALS__.invoke("perf_snapshot_v2"),
    );
    report.fingerprint.webview = await collectWebViewFingerprint(session);
    ensure(
      canonicalErrors.length === 0,
      `Console errors during canonical benchmark:\n${canonicalErrors.join("\n")}`,
    );
    assertCanonicalMetricShape(report, paths);
    assertFiniteMetrics(report);
    writeFileSync(canonicalPath, JSON.stringify(report, null, 2));
    console.log("[canonical] complete");
    console.log(`canonicalMetrics=${canonicalPath}`);
    for (const [name, path] of Object.entries(paths)) {
      if (name !== "failure") console.log(`${name}Screenshot=${path}`);
    }
  } catch (err) {
    failure = err;
    if (session) {
      report.runtime.failureProcess = {
        exitCode: session.child.exitCode,
        signalCode: session.child.signalCode,
        stdout: session.stdout,
        stderr: session.stderr,
      };
    }
    if (session?.page && !session.page.isClosed()) {
      await session.page
        .screenshot({ path: paths.failure, fullPage: false })
        .catch(() => {});
    }
    report.screenshots = Object.fromEntries(
      Object.entries(paths)
        .filter(([, path]) => existsSync(path))
        .map(([name, path]) => [name, artifactRelativePath(path)]),
    );
    const error =
      err instanceof Error
        ? { name: err.name, message: err.message, stack: err.stack }
        : { name: "Error", message: String(err) };
    writeFileSync(
      failurePath,
      JSON.stringify({ ...report, error, consoleErrors: canonicalErrors }, null, 2),
    );
    console.log(`canonicalFailure=${failurePath}`);
    if (existsSync(paths.failure))
      console.log(`failureScreenshot=${paths.failure}`);
  } finally {
    fakeProvider?.close();
    let cleanupError;
    if (session) {
      await closeSession(session).catch((error) => {
        cleanupError = error;
      });
    }
    try {
      rmWithRetry(milimHome);
    } catch (error) {
      cleanupError ??= error;
    }
    if (cleanupError)
      console.error(`canonicalCleanupError=${cleanupError.message ?? cleanupError}`);
  }

  if (failure) throw failure;
}

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

async function startCanonicalFakeProvider({
  models,
  partialMarker,
  terminalMarker,
}) {
  const requests = [];
  const completions = [];
  const openResponses = new Set();
  const server = createServer((req, res) => {
    requests.push({ method: req.method, url: req.url, at: Date.now() });
    if (
      req.method === "GET" &&
      (req.url === "/v1/models" || req.url === "/models")
    ) {
      json(res, {
        object: "list",
        data: models.map((id) => ({
          id,
          object: "model",
          created: 0,
          owned_by: "canonical-perf",
        })),
      });
      return;
    }
    if (
      req.method === "POST" &&
      (req.url === "/v1/chat/completions" ||
        req.url === "/chat/completions")
    ) {
      void readBody(req)
        .then((rawBody) => {
          let body;
          try {
            body = JSON.parse(rawBody);
          } catch {
            body = null;
          }
          const completion = {
            at: Date.now(),
            body,
            url: req.url,
          };
          completions.push(completion);
          if (completions.length === 1) {
            openResponses.add(res);
            res.once("close", () => openResponses.delete(res));
            beginCanonicalIncompleteResponse(
              res,
              body?.model ?? models[0],
              partialMarker,
            );
            return;
          }
          finishCanonicalResponse(
            res,
            body?.model ?? models[1],
            terminalMarker,
          );
        })
        .catch((error) => {
          res.writeHead(500).end(String(error));
        });
      return;
    }
    res.writeHead(404).end("not found");
  });
  await new Promise((resolve) => server.listen(0, cdpHost, resolve));
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Canonical fake provider did not bind a TCP port.");
  return {
    baseUrl: `http://${cdpHost}:${address.port}/v1`,
    requests,
    completions,
    close() {
      for (const response of openResponses) response.destroy();
      openResponses.clear();
      server.close();
    },
  };
}

function beginCanonicalIncompleteResponse(res, model, marker) {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  res.write(
    `data: ${JSON.stringify({
      id: "canonical-incomplete",
      object: "chat.completion.chunk",
      created: 0,
      model,
      choices: [
        { index: 0, delta: { content: marker }, finish_reason: null },
      ],
    })}\n\n`,
  );
}

function finishCanonicalResponse(res, model, marker) {
  res.writeHead(200, {
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  res.write(
    `data: ${JSON.stringify({
      id: "canonical-complete",
      object: "chat.completion.chunk",
      created: 0,
      model,
      choices: [
        { index: 0, delta: { content: marker }, finish_reason: null },
      ],
    })}\n\n`,
  );
  setTimeout(() => {
    if (res.destroyed) return;
    res.write(
      `data: ${JSON.stringify({
        id: "canonical-complete",
        object: "chat.completion.chunk",
        created: 0,
        model,
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 12, completion_tokens: 4, total_tokens: 16 },
      })}\n\n`,
    );
    res.write("data: [DONE]\n\n");
    res.end();
  }, 750);
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

async function configureProvider(page, baseUrl, expectedModels = [modelId]) {
  const api = await localApiCredentials(page);
  const headers = { "Content-Type": "application/json" };
  if (api.token) headers.Authorization = `Bearer ${api.token}`;
  const response = await fetch(`${api.base}/providers`, {
    method: "POST",
    headers,
    signal: AbortSignal.timeout(10_000),
    body: JSON.stringify({
      name: "Perf Mock",
      kind: "openai_compatible",
      base_url: baseUrl,
      enabled: true,
    }),
  });
  if (!response.ok) throw new Error(`Failed to save fake provider: ${response.status} ${await response.text()}`);
  const saved = await response.json();
  for (const expectedModel of expectedModels) {
    if (!saved.models?.includes(expectedModel))
      throw new Error(
        `Fake provider saved without ${expectedModel}: ${JSON.stringify(saved)}`,
      );
  }
  await waitForBackendModels(api.base, api.token, expectedModels);
}

async function localApiCredentials(page) {
  return await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke API unavailable.");
    return {
      base: await invoke("api_base_url"),
      token: await invoke("api_token"),
    };
  });
}

async function controlTestRequest(page, path, init = {}) {
  const api = await localApiCredentials(page);
  const headers = new Headers(init.headers);
  headers.set("Content-Type", "application/json");
  if (api.token) headers.set("Authorization", `Bearer ${api.token}`);
  const response = await fetch(`${api.base}${path}`, {
    ...init,
    headers,
    signal: AbortSignal.timeout(10_000),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `Control request ${path} failed: ${response.status} ${text}`,
    );
  }
  return text ? JSON.parse(text) : null;
}

async function readControlBootstrap(page) {
  return await controlTestRequest(page, "/control/v1/bootstrap");
}

async function sendControlTestCommand(page, command) {
  return await controlTestRequest(page, "/control/v1/commands", {
    method: "POST",
    body: JSON.stringify(command),
  });
}

async function waitForControlBootstrap(
  page,
  predicate,
  label,
  timeoutMs = 10_000,
) {
  const startedAt = Date.now();
  let lastBootstrap;
  while (Date.now() - startedAt < timeoutMs) {
    try {
      lastBootstrap = await readControlBootstrap(page);
      const result = predicate(lastBootstrap);
      if (result) return result;
    } catch {
      // The local API or canonical runtime is still settling.
    }
    await delay(100);
  }
  throw new Error(
    `Timed out waiting for ${label}. Last bootstrap: ${JSON.stringify(lastBootstrap)}`,
  );
}

async function mockAccountRuntimeDiscovery(page) {
  const interceptedCounts = {};
  const responses = {
    "/codex/account": { requiresOpenaiAuth: true, account: null },
    "/codex/models": { data: [] },
    "/codex/rate-limits": {},
    "/claude/status": {
      available: false,
      authenticated: false,
      models: [],
      error: "disabled for deterministic benchmark",
    },
    "/opencode/status": {
      available: false,
      authenticated: false,
      models: [],
      error: "disabled for deterministic benchmark",
    },
    "/pi/status": {
      available: false,
      authenticated: false,
      models: [],
      error: "disabled for deterministic benchmark",
    },
    "/account-runtimes/updates": {
      runtimes: Object.fromEntries(
        ["codex", "claude", "opencode", "pi"].map((runtime) => [
          runtime,
          { available: false },
        ]),
      ),
    },
  };
  await page.route("**/*", async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (!(path in responses)) {
      await route.continue();
      return;
    }
    interceptedCounts[path] = (interceptedCounts[path] ?? 0) + 1;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(responses[path]),
    });
  });
  return {
    mode: "deterministic_disabled",
    intercepted_counts: interceptedCounts,
  };
}

async function waitForBackendModels(base, token, expectedModels) {
  const headers = token ? { Authorization: `Bearer ${token}` } : {};
  const started = Date.now();
  while (Date.now() - started < 10_000) {
    const response = await fetch(`${base}/v1/models`, {
      headers,
      signal: AbortSignal.timeout(2_000),
    });
    if (response.ok) {
      const body = await response.json();
      const ids = new Set((body.data ?? []).map((model) => model.id));
      if (expectedModels.every((model) => ids.has(model))) return;
    }
    await delay(200);
  }
  throw new Error(
    `Timed out waiting for ${expectedModels.join(", ")} in backend model list.`,
  );
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
  for (let attempt = 0; attempt < 2; attempt += 1) {
    await page.evaluate(async () => {
      const perfKey = "milim.perf";
      const onboardingKey = "milim.onboarding";
      const settingsKey = "milim.settings";
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
      const settings = JSON.stringify({
        state: {
          accountRuntimeEnabled: {
            codex: false,
            claude: false,
            opencode: false,
            pi: false,
          },
        },
        version: 0,
      });
      localStorage.setItem(perfKey, "1");
      localStorage.setItem(onboardingKey, value);
      localStorage.setItem(settingsKey, settings);
      localStorage.setItem(
        "milim.local.update-cards.seen-version",
        await window.__TAURI_INTERNALS__.invoke("plugin:app|version"),
      );
      await window.__TAURI_INTERNALS__.invoke("user_state_set", {
        key: onboardingKey,
        value,
      });
      await window.__TAURI_INTERNALS__.invoke("user_state_set", {
        key: settingsKey,
        value: settings,
      });
    });
    await reloadPage(page);
    await page.getByTestId("chat-shell").waitFor({ timeout: 20_000 });
    await page
      .getByTestId("onboarding-preflight")
      .waitFor({ state: "hidden", timeout: 20_000 });
    if (
      !(await page
        .locator(".onboarding-overlay")
        .isVisible()
        .catch(() => false))
    ) {
      return;
    }
  }
  throw new Error("Onboarding remained visible after deterministic setup.");
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
      stagesMs: Object.fromEntries(
        performance
          .getEntriesByType("mark")
          .filter((entry) => entry.name.startsWith("milim."))
          .map((entry) => [entry.name.slice("milim.".length), entry.startTime]),
      ),
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
      composer: rect('[data-testid="composer"]'),
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
    if (
      text.includes("Content Security Policy directive") &&
      text.includes("http://[::1]:*")
    )
      return;
    if (text.includes("404") && /\/favicon\.ico$/.test(url)) return;
    errors.push(url ? `${text} (${url})` : text);
  });
  page.on("pageerror", (err) => errors.push(err.message));
}

async function readPersistedState(page) {
  const raw = await page.evaluate(async () => {
    const invoke = window.__TAURI_INTERNALS__?.invoke;
    if (!invoke) throw new Error("Tauri invoke API unavailable.");
    return await invoke("user_state_get", { key: "milim.sessions" });
  });
  if (!raw) throw new Error("Persisted session state is unavailable.");
  const parsed = JSON.parse(raw);
  if (!parsed.state || typeof parsed.state !== "object") {
    throw new Error("Persisted session state has an invalid shape.");
  }
  return parsed.state;
}

async function waitForPersistedState(
  page,
  predicate,
  label,
  timeoutMs = 10_000,
) {
  const startedAt = Date.now();
  let lastState;
  while (Date.now() - startedAt < timeoutMs) {
    try {
      lastState = await readPersistedState(page);
      const result = predicate(lastState);
      if (result) return result;
    } catch {
      // The reload or deferred user-state write is still settling.
    }
    await delay(100);
  }
  throw new Error(
    `Timed out waiting for ${label}. Last state: ${JSON.stringify(lastState)}`,
  );
}

function activePersistedSession(state) {
  return state.sessions?.find((session) => session.id === state.activeId) ?? null;
}

function isHiddenWorkerRunSynthesis(message) {
  if (message?.role !== "system") return false;
  return Boolean(
    message.workerRunId?.trim?.() ||
      /^Worker Run (\S+) finished with status\b/.test(message.content ?? ""),
  );
}

function persistedModelMatches(value, model) {
  return value === model || value?.endsWith(`:${model}`);
}

async function waitForSelectedModel(page, model) {
  await page.waitForFunction(
    (expected) =>
      document
        .querySelector('[data-testid="model-picker-trigger"]')
        ?.textContent?.includes(expected),
    model,
  );
}

async function waitForCondition(check, label, timeoutMs = 10_000) {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    if (await check()) return;
    await delay(50);
  }
  throw new Error(`Timed out waiting for ${label}.`);
}

async function writeLargeTranscriptFixture(
  page,
  activeId,
  { threadCount, messagesPerThread },
) {
  await page.evaluate(
    async ({ activeId, threadCount, messagesPerThread }) => {
      const invoke = window.__TAURI_INTERNALS__?.invoke;
      if (!invoke) throw new Error("Tauri invoke API unavailable.");
      const key = "milim.sessions";
      const raw = await invoke("user_state_get", { key });
      if (!raw) throw new Error("Canonical session state is unavailable.");
      const parsed = JSON.parse(raw);
      const state =
        parsed.state && typeof parsed.state === "object" ? parsed.state : {};
      const previousIds = new Set(
        (state.sessions ?? []).map((session) => session.id),
      );
      const canonical = state.sessions?.find(
        (session) => session.id === activeId,
      );
      if (!canonical)
        throw new Error("Canonical active session is unavailable.");
      if ((canonical.messages?.length ?? 0) > messagesPerThread) {
        throw new Error(
          `Canonical session already exceeds ${messagesPerThread} messages.`,
        );
      }

      const now = Date.now();
      const canonicalMessages = [...(canonical.messages ?? [])];
      while (canonicalMessages.length < messagesPerThread) {
        const index = canonicalMessages.length;
        canonicalMessages.push({
          id: `canonical-fill-${index}`,
          role: index % 2 === 0 ? "user" : "assistant",
          content: `Canonical transcript filler ${index + 1}.`,
        });
      }
      const sessions = [
        {
          ...canonical,
          messages: canonicalMessages,
          updatedAt: now,
        },
      ];
      for (let threadIndex = 1; threadIndex < threadCount; threadIndex += 1) {
        sessions.push({
          id: `canonical-fixture-${threadIndex}`,
          title: `Canonical fixture ${threadIndex}`,
          messages: Array.from(
            { length: messagesPerThread },
            (_, messageIndex) => ({
              id: `canonical-${threadIndex}-${messageIndex}`,
              role: messageIndex % 2 === 0 ? "user" : "assistant",
              content: `Fixture ${threadIndex} message ${messageIndex + 1}.`,
            }),
          ),
          settings: { ...(canonical.settings ?? {}) },
          createdAt: now - threadIndex,
          updatedAt: now - threadIndex,
        });
      }
      state.sessions = sessions;
      state.activeId = activeId;
      state.queuedMessagesBySession = {};
      state.sidebar = {
        ...(state.sidebar ?? {}),
        sessionOrder: sessions.map((session) => session.id),
      };
      parsed.state = state;
      const nextIds = new Set(sessions.map((session) => session.id));
      const meta = structuredClone(parsed);
      delete meta.state.sessions;
      await invoke("user_sessions_apply_ops", {
        delta: {
          metaJson: JSON.stringify(meta),
          sessionOrder: sessions.map((session) => session.id),
          upserts: sessions.map((session) => {
            const { messages, messagesHydrated, messagesLoadedFrom, persistedMessageCount, ...sessionMeta } = session;
            return {
              id: session.id,
              sessionJson: JSON.stringify(sessionMeta),
              messageCount: messages.length,
              messages: messages.map((message, index) => ({
                index,
                messageJson: JSON.stringify(message),
              })),
            };
          }),
          deletedSessionIds: [...previousIds].filter((id) => !nextIds.has(id)),
        },
      });
    },
    { activeId, threadCount, messagesPerThread },
  );
}

async function waitForAnimationFrames(page, count) {
  await page.evaluate(
    (frameCount) =>
      new Promise((resolve) => {
        let remaining = frameCount;
        const tick = () => {
          remaining -= 1;
          if (remaining <= 0) resolve();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
    count,
  );
}

async function measureComposerInputToNextFrame(page) {
  const input = page.getByTestId("composer-input");
  await input.fill("");
  await input.focus();
  await page.evaluate(() => {
    const composer = document.querySelector('[data-testid="composer-input"]');
    if (!(composer instanceof HTMLTextAreaElement)) {
      throw new Error("Composer input is unavailable.");
    }
    window.__MILIM_INPUT_TO_FRAME_SAMPLE__ = new Promise((resolve, reject) => {
      const timeout = setTimeout(
        () => reject(new Error("Input-to-frame sample timed out.")),
        5_000,
      );
      composer.addEventListener(
        "input",
        (event) => {
          requestAnimationFrame(() => {
            clearTimeout(timeout);
            resolve(performance.now() - event.timeStamp);
          });
        },
        { once: true },
      );
    });
  });
  await page.keyboard.insertText("x");
  const sample = await page.evaluate(async () => {
    const value = await window.__MILIM_INPUT_TO_FRAME_SAMPLE__;
    delete window.__MILIM_INPUT_TO_FRAME_SAMPLE__;
    return value;
  });
  await input.fill("");
  ensure(
    Number.isFinite(sample) && sample >= 0,
    `Input-to-next-frame sample is invalid: ${sample}.`,
  );
  return sample;
}

async function collectCdpPerformance(session) {
  try {
    if (!session.cdpClient) {
      session.cdpClient = await session.context.newCDPSession(session.page);
      await session.cdpClient.send("Performance.enable");
    }
    const response = await session.cdpClient.send("Performance.getMetrics");
    const wanted = new Set([
      "Documents",
      "Frames",
      "Nodes",
      "JSHeapUsedSize",
      "JSHeapTotalSize",
      "TaskDuration",
      "ScriptDuration",
      "LayoutDuration",
      "RecalcStyleDuration",
    ]);
    return {
      values: Object.fromEntries(
        response.metrics
          .filter((metric) => wanted.has(metric.name))
          .map((metric) => [metric.name, metric.value]),
      ),
      reason: null,
    };
  } catch (error) {
    return {
      values: null,
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}

function collectWindowsProcessTreeMemory(rootProcessId) {
  if (process.platform !== "win32") {
    return {
      workingSetBytes: null,
      privateBytes: null,
      processes: [],
      reason: "Process-tree memory sampling is implemented for Windows only.",
    };
  }
  try {
    const script = `
$rootProcessId = ${Number(rootProcessId)}
$allProcesses = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
$selectedIds = [System.Collections.Generic.HashSet[int]]::new()
[void]$selectedIds.Add($rootProcessId)
do {
  $added = $false
  foreach ($processRow in $allProcesses) {
    if ($selectedIds.Contains([int]$processRow.ParentProcessId) -and $selectedIds.Add([int]$processRow.ProcessId)) {
      $added = $true
    }
  }
} while ($added)
$rows = @(
  foreach ($selectedId in $selectedIds) {
    Get-Process -Id $selectedId -ErrorAction SilentlyContinue |
      Select-Object Id, ProcessName, WorkingSet64, PrivateMemorySize64
  }
)
$rows | ConvertTo-Json -Compress
`;
    const result = spawnSync(
      "powershell",
      ["-NoProfile", "-Command", script],
      { encoding: "utf8", timeout: 10_000 },
    );
    if (result.error) throw result.error;
    if (result.status !== 0) {
      throw new Error(result.stderr.trim() || `PowerShell exited ${result.status}`);
    }
    const text = result.stdout.trim();
    if (!text) throw new Error("PowerShell returned no process metrics.");
    const parsed = JSON.parse(text);
    const rows = (Array.isArray(parsed) ? parsed : [parsed]).map((row) => ({
      pid: Number(row.Id),
      name: String(row.ProcessName ?? ""),
      workingSetBytes: Number(row.WorkingSet64),
      privateBytes: Number(row.PrivateMemorySize64),
    }));
    if (!rows.length) throw new Error("Tauri process tree is empty.");
    return {
      workingSetBytes: rows.reduce(
        (sum, row) => sum + row.workingSetBytes,
        0,
      ),
      privateBytes: rows.reduce((sum, row) => sum + row.privateBytes, 0),
      processes: rows,
      reason: null,
    };
  } catch (error) {
    return {
      workingSetBytes: null,
      privateBytes: null,
      processes: [],
      reason: error instanceof Error ? error.message : String(error),
    };
  }
}

function assertMockProcessIsolation(metrics) {
  if (!metrics.processes.length) return;
  const disallowed = new Set([
    "claude",
    "cmd",
    "codex",
    "conhost",
    "node",
    "opencode",
    "pi",
  ]);
  const leaked = metrics.processes
    .map((process) => process.name.toLowerCase())
    .filter((name) => disallowed.has(name));
  ensure(
    leaked.length === 0,
    `Canonical benchmark launched account-runtime processes: ${leaked.join(", ")}.`,
  );
}

function assertFiniteMetrics(value, path = "report") {
  if (typeof value === "number") {
    ensure(Number.isFinite(value), `${path} is not finite.`);
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, child] of Object.entries(value)) {
    assertFiniteMetrics(child, `${path}.${key}`);
  }
}

function assertCanonicalMetricShape(report, paths) {
  ensure(report.schema_version === 2, "Canonical schema version is invalid.");
  ensure(
    !Number.isNaN(Date.parse(report.generated_at)),
    "Canonical generated_at is invalid.",
  );
  ensure(
    /^[0-9a-f]{40}$/i.test(report.commit_sha ?? ""),
    "Canonical commit_sha is invalid.",
  );
  ensure(
    report.proof === "deterministic_mocked",
    "Canonical proof mode is invalid.",
  );
  ensure(
    report.runtime.binary && !isAbsolute(report.runtime.binary),
    "Canonical runtime binary path must be repository-relative.",
  );
  ensure(report.native?.schema_version === 2, "Native perf snapshot v2 is unavailable.");
  ensure(report.native?.build_profile === "release", "Canonical binary is not release optimized.");
  ensure(report.fingerprint?.dirty_diff_sha256, "Dirty-diff fingerprint is missing.");
  ensure(report.bundles.initialJavaScriptBytes > 0, "Initial JavaScript size is missing.");
  ensure(report.bundles.initialCssBytes > 0, "Initial CSS size is missing.");
  for (const [phase, metrics] of Object.entries(report.renderer)) {
    ensure(metrics?.perf, `Renderer phase ${phase} has no Milim perf snapshot.`);
    ensure(
      metrics.frames?.count > 0,
      `Renderer phase ${phase} has no frame samples.`,
    );
    ensure(
      metrics.longTasks?.count >= 0,
      `Renderer phase ${phase} has invalid long-task metrics.`,
    );
  }
  for (const [phase, metrics] of Object.entries(report.cdp)) {
    ensure(metrics?.values, `CDP phase ${phase} is unavailable: ${metrics?.reason}`);
    for (const name of [
      "JSHeapUsedSize",
      "TaskDuration",
      "LayoutDuration",
    ]) {
      ensure(
        Number.isFinite(metrics.values[name]),
        `CDP phase ${phase} is missing ${name}.`,
      );
    }
  }
  for (const [phase, metrics] of Object.entries(report.processMemory)) {
    if (metrics.workingSetBytes === null || metrics.privateBytes === null) {
      ensure(
        Boolean(metrics.reason),
        `Process-memory phase ${phase} is unavailable without a reason.`,
      );
    } else {
      ensure(
        metrics.workingSetBytes >= 0 && metrics.privateBytes >= 0,
        `Process-memory phase ${phase} contains negative byte counts.`,
      );
    }
  }
  for (const [name, path] of Object.entries(paths)) {
    if (name === "failure") continue;
    ensure(existsSync(path), `Canonical screenshot ${name} is missing.`);
  }
  for (const [name, value] of Object.entries(report.timingsMs)) {
    ensure(value >= 0, `Timing ${name} is negative.`);
  }
}

function artifactRelativePath(path) {
  return relative(artifactDir, path).replaceAll("\\", "/");
}

function repoRelativePath(path) {
  return relative(root, path).replaceAll("\\", "/");
}

function gitOutput(args) {
  const result = spawnSync("git", args, {
    cwd: root,
    encoding: args.includes("--binary") ? "buffer" : "utf8",
    maxBuffer: 1024 * 1024 * 1024,
  });
  return result.status === 0 ? result.stdout : null;
}

function buildFingerprint() {
  const diff = gitOutput(["diff", "--binary", "HEAD"]);
  const untracked = gitOutput([
    "ls-files",
    "--others",
    "--exclude-standard",
    "-z",
    "--",
    ":/",
  ]);
  const dirtyHash = createHash("sha256").update(
    diff ?? Buffer.from("git-diff-unavailable"),
  );
  const untrackedPaths = typeof untracked === "string"
    ? untracked.split("\0").filter(Boolean).sort()
    : [];
  for (const path of untrackedPaths) {
    const absolutePath = join(root, path);
    if (!existsSync(absolutePath) || !statSync(absolutePath).isFile()) continue;
    dirtyHash.update("\0untracked\0").update(path).update("\0");
    dirtyHash.update(readFileSync(absolutePath));
  }
  const cpu = cpus()[0];
  return {
    commit_sha: currentCommitSha(),
    dirty_diff_sha256: dirtyHash.digest("hex"),
    dirty: Boolean(diff?.length || untrackedPaths.length),
    cpu: cpu ? `${cpu.model} (${cpus().length} logical)` : "unknown",
    ram_bytes: totalmem(),
    os: `${osVersion()} (${osRelease()})`,
    arch: process.arch,
    webview: null,
    build_profile: "release",
    fixture_version: "canonical-thread-v2",
  };
}

function collectInitialBundleSizes() {
  const assetDir = join(root, "dist", "assets");
  const assets = existsSync(assetDir) ? readdirSync(assetDir) : [];
  const largestIndex = (extension) =>
    assets
      .filter((name) => name.startsWith("index-") && name.endsWith(extension))
      .map((name) => ({ name, bytes: statSync(join(assetDir, name)).size }))
      .sort((left, right) => right.bytes - left.bytes)[0] ?? { name: null, bytes: 0 };
  const javascript = largestIndex(".js");
  const css = largestIndex(".css");
  return {
    initialJavaScriptBytes: javascript.bytes,
    initialCssBytes: css.bytes,
    initialJavaScriptAsset: javascript.name,
    initialCssAsset: css.name,
  };
}

async function collectWebViewFingerprint(session) {
  try {
    const client =
      session.cdpClient ?? await session.context.newCDPSession(session.page);
    const version = await client.send("Browser.getVersion");
    return {
      product: version.product,
      userAgent: version.userAgent,
      jsVersion: version.jsVersion,
    };
  } catch (error) {
    return { unavailable: error instanceof Error ? error.message : String(error) };
  }
}

function currentCommitSha() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: root,
    encoding: "utf8",
  });
  return result.status === 0 ? result.stdout.trim() : null;
}

function ensure(condition, message) {
  if (!condition) throw new Error(message);
}

async function launchTauriBinary(milimHome, errors) {
  const child = spawn(perfBinary, [], {
    cwd: root,
    env: isolatedBinaryEnvironment(milimHome),
    stdio: ["ignore", "pipe", "pipe"],
  });

  const session = {
    child,
    stdout: "",
    stderr: "",
    browser: null,
    context: null,
    page: null,
    cdpClient: null,
  };
  child.stdout?.on("data", (chunk) => {
    session.stdout += chunk.toString();
  });
  child.stderr?.on("data", (chunk) => {
    session.stderr += chunk.toString();
  });

  try {
    await waitForCdp(session, cdpUrl, 90_000);
    session.browser = await chromium.connectOverCDP(cdpUrl);
    session.page = await tauriPage(
      session.browser,
      session,
      20_000,
      errors,
    );
    session.context = session.page.context();
    session.page.setDefaultTimeout(12_000);
    return session;
  } catch (error) {
    await closeSession(session).catch(() => {});
    throw error;
  }
}

async function launchTauriDev(milimHome, errors) {
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

  try {
    await waitForCdp(session, cdpUrl, 90_000);
    session.browser = await chromium.connectOverCDP(cdpUrl);
    session.page = await tauriPage(
      session.browser,
      session,
      60_000,
      errors,
    );
    session.page.setDefaultTimeout(12_000);
    return session;
  } catch (error) {
    await closeSession(session).catch(() => {});
    throw error;
  }
}

function isolatedBinaryEnvironment(milimHome) {
  const environment = { ...process.env };
  for (const key of Object.keys(environment)) {
    if (
      /(?:KEY|TOKEN|SECRET|PASS|CREDENTIAL|COOKIE|AUTH)/i.test(key) ||
      /^(?:PATH|MILIM_REMOTE_URL|OPENCODE_CONFIG_CONTENT|WEBVIEW2_.*)$/i.test(
        key,
      )
    ) {
      delete environment[key];
    }
  }
  const profile = join(milimHome, "profile");
  const appData = join(profile, "AppData", "Roaming");
  const localAppData = join(profile, "AppData", "Local");
  const emptyPath = join(milimHome, "empty-path");
  for (const path of [profile, appData, localAppData, emptyPath])
    mkdirSync(path, { recursive: true });
  const systemRoot =
    Object.entries(process.env).find(
      ([key]) => key.toLowerCase() === "systemroot",
    )?.[1] || String.raw`C:\Windows`;
  const harmlessExecutable = join(systemRoot, "System32", "where.exe");
  const codexStub = join(emptyPath, "codex-stub.mjs");
  writeFileSync(
    codexStub,
    `import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id == null) return;
  const result = message.method === "account/read"
    ? { requiresOpenaiAuth: true, account: null }
    : message.method === "model/list"
      ? { data: [] }
      : {};
  process.stdout.write(JSON.stringify({ id: message.id, result }) + "\\n");
});
`,
  );
  for (const name of ["claude", "codex", "opencode", "pi"]) {
    copyFileSync(harmlessExecutable, join(emptyPath, `${name}.exe`));
    writeFileSync(
      join(emptyPath, `${name}.cmd`),
      name === "codex"
        ? `@"${process.execPath}" "${codexStub}" %*\r\n`
        : "@exit /b 1\r\n",
    );
  }
  return {
    ...environment,
    PATH: emptyPath,
    USERPROFILE: profile,
    HOME: profile,
    APPDATA: appData,
    LOCALAPPDATA: localAppData,
    CODEX_HOME: join(profile, ".codex"),
    CLAUDE_CONFIG_DIR: join(profile, ".claude"),
    XDG_CONFIG_HOME: join(profile, ".config"),
    XDG_DATA_HOME: join(profile, ".local", "share"),
    MILIM_HOME: milimHome,
    MILIM_PERF: "1",
    WEBVIEW2_USER_DATA_FOLDER: join(milimHome, "webview2"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${cdpPort}`,
  };
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
      throw new Error(`Tauri exited before CDP was ready.\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
    }
    try {
      const remaining = Math.max(1, timeoutMs - (Date.now() - started));
      const signal = AbortSignal.timeout(Math.min(1_000, remaining));
      const [version, targets] = await Promise.all([
        fetch(`${url}/json/version`, { signal }),
        fetch(`${url}/json/list`, { signal }),
      ]);
      if (version.ok && targets.ok) {
        const entries = await targets.json();
        if (Array.isArray(entries) && entries.length > 0) return;
      }
    } catch {}
    await delay(250);
  }
  throw new Error(`Timed out waiting for a WebView target at ${url}.\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
}

async function tauriPage(browser, session, timeoutMs, errors) {
  const started = Date.now();
  const seen = new Set();
  const observed = new WeakSet();
  while (Date.now() - started < timeoutMs) {
    const pages = browser
      .contexts()
      .flatMap((context) => context.pages())
      .filter((page) => !page.isClosed());
    for (const page of pages) {
      if (errors && !observed.has(page)) {
        observed.add(page);
        collectErrors(page, errors);
      }
      const hasInvoke = await page.evaluate(() => Boolean(window.__TAURI_INTERNALS__?.invoke)).catch(() => false);
      if (hasInvoke) return page;
      seen.add(page.url() || "<blank>");
    }
    await delay(250);
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
