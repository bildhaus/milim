import assert from "node:assert/strict";
import { test } from "node:test";
import vm from "node:vm";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { collectMcpEvidence, installMcpMessageEvidence, saveFailureEvidence } from "./tauri-mcp-evidence.mjs";

function recorder(frame = false) {
  const handlers = {};
  const listeners = {};
  let timestamp = 0;
  const chart = {};
  const parent = {};
  const window = {};
  window.top = frame ? parent : window;
  const context = { window, parent, Number, performance: { now: () => ++timestamp }, document: {
    body: { dataset: { view: frame ? "chart" : "" } },
    querySelector: () => ({ contentWindow: chart }),
  }, addEventListener: (name, handler) => {
    (listeners[name] ??= []).push(handler);
    handlers[name] = (event) => listeners[name].forEach((listener) => listener(event));
  } };
  vm.runInNewContext(`(${installMcpMessageEvidence.toString()})()`, context);
  return { handlers, window, source: frame ? parent : chart };
}

test("MCP diagnostics retain only bounded protocol metadata from the chart", () => {
  const { handlers, window, source } = recorder();
  const data = { jsonrpc: "2.0", id: 2, method: "tools/call", params: { token: "SECRET-CANARY" }, result: { private: "SECRET-CANARY" }, error: { code: -32602, message: "SECRET-CANARY" } };
  handlers.message({ source: {}, data });
  assert.equal(window.__milimMcpEvidence.length, 0);
  handlers.message({ source, data });
  assert.deepEqual(JSON.parse(JSON.stringify(window.__milimMcpEvidence[0])), { direction: "from-chart", kind: "request-or-notification", method: "tools/call", id: 2, errorCode: -32602 });
  handlers.message({ source, data: { ...data, method: "SECRET-CANARY", id: "SECRET-CANARY", error: { code: "SECRET-CANARY" } } });
  assert.equal(window.__milimMcpEvidence[1].method, null);
  for (let index = 0; index < 100; index++) handlers.message({ source, data });
  assert.equal(window.__milimMcpEvidence.length, 50);
  assert.ok(!JSON.stringify(window.__milimMcpEvidence).includes("SECRET-CANARY"));
});

test("chart response and rejection diagnostics omit error messages and payloads", () => {
  const { handlers, window, source } = recorder(true);
  handlers.message({ source, data: { jsonrpc: "2.0", id: 2, result: { token: "SECRET-CANARY" } } });
  assert.equal(window.__milimMcpEvidence[0].kind, "response");
  for (let index = 0; index < 20; index++) {
    handlers.unhandledrejection({ reason: { name: "McpError", code: -32602, message: "SECRET-CANARY", data: "SECRET-CANARY" } });
    handlers.click({ target: { closest: () => true } });
  }
  assert.equal(window.__milimMcpErrors.length, 10);
  assert.equal(window.__milimMcpRefreshClicks, 10);
  assert.ok(!JSON.stringify(window).includes("SECRET-CANARY"));
});

test("unavailable native state and detached chart preserve core protocol evidence", async () => {
  const core = { messages: [{ method: "tools/call", id: 2 }], globalApprovalCount: 1 };
  let calls = 0;
  const page = {
    evaluate: () => ++calls === 1 ? core : calls === 2 ? new Promise(() => {}) : { unavailable: true },
    frameLocator: () => ({ locator: () => ({ evaluate: () => { throw new Error("Detached SECRET-CANARY"); } }) }),
  };
  const evidence = await collectMcpEvidence(page, 1);
  assert.deepEqual(evidence.parent, core);
  assert.deepEqual(evidence.persisted, { timedOut: true });
  assert.deepEqual(evidence.chart, { unavailable: true });
  assert.ok(!JSON.stringify(evidence).includes("SECRET-CANARY"));
});

test("pointer evidence preserves bounded trusted event ordering without DOM values", () => {
  const { handlers, window } = recorder(true);
  window.__milimMcpTrackPointer = true;
  for (const type of ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]) {
    handlers[type]({ type, isTrusted: true, pointerType: "mouse", button: 0, buttons: type.endsWith("down") ? 1 : 0,
      clientX: 20, clientY: 30, target: { tagName: "BUTTON", value: "SECRET-CANARY", textContent: "SECRET-CANARY", closest: () => true } });
  }
  assert.deepEqual(Array.from(window.__milimMcpPointerEvents, (event) => event.type), ["pointerdown", "mousedown", "pointerup", "mouseup", "click"]);
  assert.ok(window.__milimMcpPointerEvents.every((event, index) => event.order === index + 1 && event.atMs === index + 1 && event.isTrusted && event.target === "refresh"));
  for (let index = 0; index < 100; index++) handlers.pointerdown({ target: { tagName: "SECRET-CANARY", title: "SECRET-CANARY" } });
  assert.equal(window.__milimMcpPointerEvents.length, 50);
  assert.ok(!JSON.stringify(window.__milimMcpPointerEvents).includes("SECRET-CANARY"));
});

test("artifact I/O and screenshot errors never replace the original test failure", async () => {
  const directory = mkdtempSync(join(tmpdir(), "milim-evidence-test-"));
  const blocked = join(directory, "blocked");
  writeFileSync(blocked, "fixture");
  const original = new Error("Original assertion");
  try {
    await assert.rejects(async () => {
      assert.equal(await saveFailureEvidence(blocked, "general", {}, null), false);
      throw original;
    }, (error) => error === original);
    const evidence = { errorName: "TimeoutError" };
    assert.equal(await saveFailureEvidence(directory, "mcp-apps", evidence, {
      isClosed: () => false, screenshot: async () => { throw new Error("SECRET-CANARY"); },
    }), true);
    assert.deepEqual(JSON.parse(readFileSync(join(directory, "mcp-apps-failure.json"), "utf8")), { errorName: "TimeoutError", screenshotUnavailable: true });
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
