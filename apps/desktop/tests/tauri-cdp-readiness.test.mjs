import assert from "node:assert/strict";
import { createServer } from "node:http";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { observeStartupAfterDeadline, waitForCdp } from "./tauri-cdp-readiness.mjs";

let mode = "ready";
let requests = 0;
const server = createServer((request, response) => {
  requests += 1;
  if (mode === "hang") return;
  if (mode === "warming" && requests < 3) { response.writeHead(503); response.end(); return; }
  response.setHeader("Content-Type", "application/json");
  response.end(JSON.stringify(mode === "invalid" ? { ok: true } : {
    webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/devtools/browser/fixture`,
  }));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const url = `http://127.0.0.1:${server.address().port}`;
const temp = mkdtempSync(join(tmpdir(), "milim-cdp-readiness-test-"));
const session = () => ({ child: { exitCode: null, signalCode: null }, stdout: "", stderr: "" });
try {
  assert.equal((await waitForCdp(session(), url, 1000)).attempts, 1);
  mode = "warming"; requests = 0;
  assert.equal((await waitForCdp(session(), url, 1500)).attempts, 3);
  mode = "invalid";
  await assert.rejects(waitForCdp(session(), url, 100), /Timed out waiting.*\nstartupDiagnostics=/);
  const controlFile = join(temp, "control.json");
  writeFileSync(controlFile, JSON.stringify({ api_url: url, token: "TOKEN_MUST_NOT_APPEAR_IN_DIAGNOSTICS" }));
  await assert.rejects(waitForCdp({ ...session(), controlFile }, url, 100), (error) => {
    assert.ok(error.message.includes('"controlFilePresent":true'));
    assert.ok(error.message.includes('"apiHealthStatus":200'));
    assert.ok(!error.message.includes("TOKEN_MUST_NOT_APPEAR_IN_DIAGNOSTICS"));
    return true;
  });
  setTimeout(() => { mode = "ready"; }, 150);
  await assert.rejects(waitForCdp({ ...session(), controlFile, observeStartupAfterDeadline: true }, url, 100), (error) => {
    assert.match(error.message, /Timed out waiting/);
    assert.ok(error.startupDiagnostics.afterDeadline.firstCdpReadyMs > 100);
    assert.ok(error.startupDiagnostics.afterDeadline.firstApiHealthyMs >= 100);
    assert.ok(error.startupDiagnostics.afterDeadline.finishedAtMs < 1500);
    assert.ok(!error.message.includes("TOKEN_MUST_NOT_APPEAR_IN_DIAGNOSTICS"));
    return true;
  });
  mode = "invalid";
  const invalidObservation = await observeStartupAfterDeadline({ ...session(), controlFile }, url, Date.now(), 100);
  assert.equal(invalidObservation.firstCdpReadyMs, null);
  assert.ok(invalidObservation.firstApiHealthyMs !== null);
  mode = "hang";
  const hungObservation = await observeStartupAfterDeadline({ ...session(), controlFile }, url, Date.now(), 100);
  assert.equal(hungObservation.firstCdpReadyMs, null);
  assert.equal(hungObservation.firstApiHealthyMs, null);
  assert.ok(hungObservation.finishedAtMs < 1000);
  const started = Date.now();
  await assert.rejects(waitForCdp(session(), url, 100), /Timed out waiting/);
  assert.ok(Date.now() - started < 1500, "A stalled HTTP probe escaped the readiness deadline");
  const exitsDuringProbe = session();
  setTimeout(() => { exitsDuringProbe.child.exitCode = 9; }, 25);
  await assert.rejects(waitForCdp(exitsDuringProbe, url, 100), /Tauri exited before CDP/);
  const beforeExit = requests;
  await assert.rejects(waitForCdp({ ...session(), child: { exitCode: 7 } }, url, 1000), /Tauri exited before CDP/);
  await assert.rejects(waitForCdp({ ...session(), child: { exitCode: null, signalCode: "SIGTERM" } }, url, 1000), /Tauri exited before CDP/);
  await assert.rejects(waitForCdp({ ...session(), spawnErrorCode: "ENOENT" }, url, 1000), /Tauri could not be spawned/);
  assert.equal(requests, beforeExit, "Exited or unspawned processes must not be probed");
  const stopped = await observeStartupAfterDeadline({ ...session(), child: { exitCode: 1 } }, url, Date.now(), 100);
  assert.equal(stopped.childStopped, true);
  assert.equal(requests, beforeExit);
  console.log("CDP readiness: valid endpoint, delayed readiness, invalid200, stalled HTTP, exit, signal and spawn failure checks passed.");
} finally {
  rmSync(temp, { recursive: true, force: true });
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
