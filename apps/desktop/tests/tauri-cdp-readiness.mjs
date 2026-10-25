import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";

export async function waitForCdp(session, url, timeoutMs) {
  const started = Date.now();
  let attempts = 0;
  let lastProbe = null;
  async function fail(message) {
    const diagnostics = {
      elapsedMs: Date.now() - started, attempts, endpoint: `${url}/json/version`, lastProbe,
      child: { pid: session.child.pid, exitCode: session.child.exitCode, signalCode: session.child.signalCode, spawnErrorCode: session.spawnErrorCode },
      ...(await startupDiagnostics(session, url)),
    };
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
        const endpoint = new URL(metadata.webSocketDebuggerUrl);
        lastProbe.validWebSocketEndpoint = ["ws:", "wss:"].includes(endpoint.protocol)
          && ["localhost", "127.0.0.1", "[::1]"].includes(endpoint.hostname)
          && endpoint.port === new URL(url).port
          && endpoint.pathname.startsWith("/devtools/browser/");
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
  await fail(`Timed out waiting for ${url}/json/version.`);
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
    const script = `$fixtureProfile = '${profile}'; $processes = @(Get-CimInstance Win32_Process -Filter "Name='milim-desktop.exe' OR Name='msedgewebview2.exe'" | ForEach-Object { $version = if ($_.ExecutablePath) { (Get-Item -LiteralPath $_.ExecutablePath -ErrorAction SilentlyContinue).VersionInfo.FileVersion } else { $null }; @{ name=$_.Name; pid=$_.ProcessId; parentPid=$_.ParentProcessId; fixtureProfileTextMatch=([bool]$fixtureProfile -and $_.CommandLine -and $_.CommandLine.Contains($fixtureProfile)); fileVersion=if ($version -match '^\\d+\\.\\d+\\.\\d+\\.\\d+$') { $version } else { $null } } }); $listeners = @(Get-NetTCPConnection -LocalPort ${port} -ErrorAction SilentlyContinue | Select-Object LocalAddress,LocalPort,State,OwningProcess); @{ processes=$processes; listeners=$listeners } | ConvertTo-Json -Depth 4 -Compress`;
    const query = spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", timeout: 5000 });
    result.processQueryExitCode = query.status;
    try { result.windows = JSON.parse(query.stdout); } catch { result.processQueryError = query.error?.code ?? "No JSON diagnostics"; }
  }
  return result;
}
