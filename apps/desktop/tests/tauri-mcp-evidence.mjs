import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Serialized into the isolated fixture page and its frames; never retain payloads.
export function installMcpMessageEvidence() {
  const messages = [];
  window.__milimMcpEvidence = messages;
  window.__milimMcpErrors = [];
  window.__milimMcpRefreshClicks = 0;
  addEventListener("unhandledrejection", (event) => {
    if (window.__milimMcpErrors.length >= 10) return;
    window.__milimMcpErrors.push({
      name: ["Error", "TypeError", "ZodError", "McpError"].includes(event.reason?.name) ? event.reason.name : null,
      code: Number.isSafeInteger(event.reason?.code) ? event.reason.code : null,
    });
  });
  addEventListener("click", (event) => {
    if (document.body?.dataset.view === "chart" && event.target?.closest?.("#refresh")) window.__milimMcpRefreshClicks = Math.min(window.__milimMcpRefreshClicks + 1, 10);
  });
  addEventListener("message", (event) => {
    const chart = document.querySelector('iframe[title="MCP App ui://milim.test/chart"]');
    const fromChart = window === window.top && chart?.contentWindow === event.source;
    const fromHost = window !== window.top && document.body?.dataset.view === "chart" && event.source === parent;
    if ((!fromChart && !fromHost) || messages.length >= 50 || event.data?.jsonrpc !== "2.0") return;
    const data = event.data;
    const methods = ["tools/call", "ui/initialize", "ui/notifications/initialized", "ui/notifications/size-changed", "ui/notifications/tool-result", "ui/notifications/host-context-changed"];
    messages.push({
      direction: fromChart ? "from-chart" : "from-host",
      kind: typeof data.method === "string" ? "request-or-notification" : "response",
      method: methods.includes(data.method) ? data.method : null,
      id: Number.isSafeInteger(data.id) ? data.id : null,
      errorCode: Number.isSafeInteger(data.error?.code) ? data.error.code : null,
    });
  });
}

async function boundedEvidence(read) {
  let timer;
  try {
    return await Promise.race([
      read(),
      new Promise((resolve) => { timer = setTimeout(() => resolve({ timedOut: true }), 1200); }),
    ]);
  } catch { return { unavailable: true }; }
  finally { clearTimeout(timer); }
}

export async function saveFailureEvidence(directory, mode, evidence, page) {
  try {
    mkdirSync(directory, { recursive: true });
    if (page && !page.isClosed()) {
      await page.screenshot({ path: join(directory, `${mode}-failure.png`), fullPage: false, timeout: 5000 })
        .catch(() => { evidence.screenshotUnavailable = true; });
    }
    writeFileSync(join(directory, `${mode}-failure.json`), JSON.stringify(evidence, null, 2));
    return true;
  } catch (error) {
    console.error(`failureEvidenceWriteError=${["EACCES", "EPERM", "ENOSPC", "EEXIST", "ENOTDIR"].includes(error.code) ? error.code : "other"}`);
    return false;
  }
}

export async function collectMcpEvidence(page, browserErrorCount) {
  const parent = await boundedEvidence(() => page.evaluate(() => {
    const cards = [...document.querySelectorAll('[data-testid="mcp-app-view"]')];
    const firstTitle = cards[0]?.querySelector("iframe")?.title;
    const titles = ["chart", "diagram", "form", "dashboard", "viewer"].map((kind) => `MCP App ui://milim.test/${kind}`);
    return {
      messages: window.__milimMcpEvidence ?? [],
      rejectionClasses: window.__milimMcpErrors ?? [],
      renderedToolApproval: document.querySelector('[aria-label^="Session controls,"]')?.getAttribute("aria-label")?.match(/Tool approval (Review|Guarded|Open)/i)?.[1]?.toLowerCase() ?? null,
      intendedIframeTitle: titles[0],
      firstCardIframeTitle: titles.includes(firstTitle) ? firstTitle : null,
      firstCardApprovalCount: cards[0]?.querySelectorAll(".mcp-app-approval").length ?? 0,
      globalApprovalCount: document.querySelectorAll(".mcp-app-approval").length,
    };
  }));
  const persisted = await boundedEvidence(() => page.evaluate(async () => {
    const fixture = await window.__TAURI_INTERNALS__.invoke("user_session_snapshot", { sessionId: "e2e-mcp-apps-thread" });
    const approval = fixture?.settings?.toolApproval;
    return { toolApproval: ["review", "guarded", "open"].includes(approval) ? approval : null };
  }));
  const chart = await boundedEvidence(() => page.frameLocator('iframe[title="MCP App ui://milim.test/chart"]').locator("body").evaluate((body) => ({
    messages: window.__milimMcpEvidence ?? [],
    rejectionClasses: window.__milimMcpErrors ?? [],
    refreshClicks: window.__milimMcpRefreshClicks ?? 0,
    refreshHandlerPresent: typeof document.querySelector("#refresh")?.onclick === "function",
    ready: body.dataset.ready === "true",
    theme: ["light", "dark"].includes(body.dataset.theme) ? body.dataset.theme : null,
    teardownReceived: body.dataset.teardown === "received",
    refreshCount: /^\d+$/.test(body.dataset.refreshCount ?? "") ? Number(body.dataset.refreshCount) : null,
    parentDomBlocked: document.querySelector("#security")?.dataset.parentDom === "blocked",
    storageBlocked: document.querySelector("#security")?.dataset.storage === "blocked",
  })));
  const loadedAppBridge = await boundedEvidence(() => page.evaluate(async () => {
    const resources = performance.getEntriesByType("resource").map((entry) => new URL(entry.name)).filter((url) => url.origin === location.origin && !url.search && /^\/assets\/app-bridge-[\w-]+\.js$/.test(url.pathname));
    const urls = [...new Set(resources.map((url) => url.href))];
    if (urls.length !== 1) return { unavailable: true };
    const response = await fetch(urls[0], { signal: AbortSignal.timeout(1000) });
    if (!response.ok) return { unavailable: true };
    const bytes = await response.arrayBuffer();
    const version = new TextDecoder().decode(bytes).match(/major:(\d+),minor:(\d+),patch:(\d+)/);
    return {
      path: new URL(urls[0]).pathname, size: bytes.byteLength,
      sha256: [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((byte) => byte.toString(16).padStart(2, "0")).join(""),
      zodVersion: version ? version.slice(1).join(".") : null,
    };
  }));
  return { parent, persisted, chart, loadedAppBridge, browserErrorCount: Math.min(browserErrorCount, 100) };
}
