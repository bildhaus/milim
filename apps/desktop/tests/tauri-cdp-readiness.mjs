import { execFile } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

export async function waitForCdp(session, url, timeoutMs) {
  const started = Date.now();
  let attempts = 0;
  let lastProbe = null;
  async function fail(message, observe = false) {
    const later = observe && session.observeStartupAfterDeadline ? observeStartupAfterDeadline(session, url, started) : null;
    const diagnostics = {
      elapsedMs: Date.now() - started, attempts, endpoint: `${url}/json/version`, lastProbe,
      child: { pid: session.child.pid, exitCode: session.child.exitCode, signalCode: session.child.signalCode, spawnErrorCode: session.spawnErrorCode },
      ...(await startupDiagnostics(session, url)),
    };
    if (later) diagnostics.afterDeadline = await later;
    const error = new Error(`${message}\nstartupDiagnostics=${JSON.stringify(diagnostics)}\nstdout:\n${session.stdout}\nstderr:\n${session.stderr}`);
    error.startupDiagnostics = diagnostics;
    throw error;
  }
  while (Date.now() - started < timeoutMs) {
    if (session.spawnErrorCode) await fail("Tauri could not be spawned before CDP was ready.");
    if (session.child.exitCode != null || session.child.signalCode != null) await fail("Tauri exited before CDP was ready.");
    attempts += 1;
    try {
      const response = await fetch(`${url}/json/version`, {
        signal: AbortSignal.timeout(Math.max(1, Math.min(1000, timeoutMs - (Date.now() - started)))),
      });
      lastProbe = { status: response.status, validWebSocketEndpoint: false };
      if (response.ok) {
        const metadata = await response.json();
        lastProbe.validWebSocketEndpoint = validCdpEndpoint(metadata, url);
        if (lastProbe.validWebSocketEndpoint && session.child.exitCode == null && session.child.signalCode == null && !session.spawnErrorCode) return { elapsedMs: Date.now() - started, attempts };
      }
    } catch (error) {
      lastProbe = { ...lastProbe, errorName: error.name, errorCode: error.cause?.code };
    }
    if (session.spawnErrorCode) await fail("Tauri could not be spawned before CDP was ready.");
    if (session.child.exitCode != null || session.child.signalCode != null) await fail("Tauri exited before CDP was ready.");
    const remaining = timeoutMs - (Date.now() - started);
    if (remaining > 0) await new Promise((resolve) => setTimeout(resolve, Math.min(250, remaining)));
  }
  await fail(`Timed out waiting for ${url}/json/version.`, true);
}

async function startupDiagnostics(session, url) {
  const result = { controlFilePresent: Boolean(session.controlFile && existsSync(session.controlFile)) };
  if (result.controlFilePresent) {
    try {
      const control = JSON.parse(readFileSync(session.controlFile, "utf8"));
      const api = new URL(control.api_url);
      if (api.protocol !== "http:" || !["localhost", "127.0.0.1", "[::1]"].includes(api.hostname)) throw new Error("Not loopback");
      result.apiLoopbackPort = api.port || "80";
      result.apiHealthStatus = (await fetch(new URL("/health", api), { signal: AbortSignal.timeout(1000) })).status;
    } catch (error) { result.apiHealthError = error.name; }
  }
  if (process.platform === "win32" && session.child.pid) {
    const port = Number(new URL(url).port);
    // A false literal command-line match is inconclusive (case/8.3 paths or child roles can differ).
    const profile = String(session.webviewDataFolder ?? "").replaceAll("'", "''");
    const script = `$fixtureProfile = '${profile}'; $processes = @(Get-CimInstance Win32_Process -Filter "Name='milim-desktop.exe' OR Name='msedgewebview2.exe'" | ForEach-Object { $version = if ($_.ExecutablePath) { (Get-Item -LiteralPath $_.ExecutablePath -ErrorAction SilentlyContinue).VersionInfo.FileVersion } else { $null }; @{ name=$_.Name; pid=$_.ProcessId; parentPid=$_.ParentProcessId; fixtureProfileTextMatch=([bool]$fixtureProfile -and $_.CommandLine -and $_.CommandLine.Contains($fixtureProfile)); fileVersion=if ($version -match '^\\d+\\.\\d+\\.\\d+\\.\\d+$') { $version } else { $null } } }); @{ processes=$processes } | ConvertTo-Json -Depth 4 -Compress`;
    const [query, sockets] = await Promise.all([
      diagnosticCommand("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], 5000),
      diagnosticCommand("netstat.exe", ["-ano", "-p", "tcp"], 1000),
    ]);
    result.processQueryExitCode = query.status;
    try { result.windows = JSON.parse(query.stdout); } catch { result.processQueryError = query.errorCode ?? "No JSON diagnostics"; }
    result.listenerQueryExitCode = sockets.status;
    if (sockets.status === 0) {
      result.listeners = sockets.stdout.split(/\r?\n/).map((line) => line.trim().split(/\s+/))
        .filter((parts) => parts[0] === "TCP" && parts[1]?.endsWith(`:${port}`) && parts[3] === "LISTENING" && /^\d+$/.test(parts[4] ?? ""))
        .map((parts) => ({ localPort: port, owningProcess: Number(parts[4]) }));
    } else result.listenerQueryError = sockets.errorCode;

  }
  return result;
}

function diagnosticCommand(command, args, timeout) {
  return new Promise((resolve) => {
    execFile(command, args, { encoding: "utf8", timeout, maxBuffer: 512 * 1024 }, (error, stdout) => {
      resolve({ status: error ? (typeof error.code === "number" ? error.code : null) : 0, stdout,
        errorCode: error?.killed ? "ETIMEDOUT" : error ? "PROCESS_ERROR" : null });
    });
  });
}

function validCdpEndpoint(metadata, url) {
  try {
    const endpoint = new URL(metadata.webSocketDebuggerUrl);
    return ["ws:", "wss:"].includes(endpoint.protocol)
      && ["localhost", "127.0.0.1", "[::1]"].includes(endpoint.hostname)
      && endpoint.port === new URL(url).port && endpoint.pathname.startsWith("/devtools/browser/");
  } catch { return false; }
}

// Observation never turns the original readiness deadline into a passing test.
export async function observeStartupAfterDeadline(session, url, waitStarted, observationMs = 10_000) {
  const observationStarted = Date.now();
  const result = { firstCdpReadyMs: null, firstApiHealthyMs: null, timingOrigin: "waitForCdp start", observationMs };
  while (Date.now() - observationStarted < observationMs) {
    if (session.child.exitCode != null || session.child.signalCode != null || session.spawnErrorCode) { result.childStopped = true; break; }
    const timeout = Math.max(1, Math.min(500, observationMs - (Date.now() - observationStarted)));
    await Promise.all([
      (async () => {
        if (result.firstCdpReadyMs !== null) return;
        try {
          const response = await fetch(`${url}/json/version`, { signal: AbortSignal.timeout(timeout) });
          if (response.ok && validCdpEndpoint(await response.json(), url)
            && session.child.exitCode == null && session.child.signalCode == null && !session.spawnErrorCode) result.firstCdpReadyMs = Date.now() - waitStarted;
        } catch { /* Keep observing within the diagnostic budget. */ }
      })(),
      (async () => {
        if (result.firstApiHealthyMs !== null || !session.controlFile || !existsSync(session.controlFile)) return;
        try {
          const api = new URL(JSON.parse(readFileSync(session.controlFile, "utf8")).api_url);
          if (api.protocol !== "http:" || !["localhost", "127.0.0.1", "[::1]"].includes(api.hostname)) return;
          const response = await fetch(new URL("/health", api), { signal: AbortSignal.timeout(timeout) });
          result.lastApiStatus = response.status;
          if (response.status === 200) result.firstApiHealthyMs = Date.now() - waitStarted;
        } catch { /* Never retain the control token or response body. */ }
      })(),
    ]);
    if (result.firstCdpReadyMs !== null && result.firstApiHealthyMs !== null) break;
    const remaining = observationMs - (Date.now() - observationStarted);
    if (remaining > 0) await new Promise((resolve) => setTimeout(resolve, Math.min(250, remaining)));
  }
  result.finishedAtMs = Date.now() - waitStarted;
  return result;
}
