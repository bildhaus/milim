import { spawnSync } from "node:child_process";
import { isAbsolute } from "node:path";

const names = ["AdditionalBrowserArguments", "UserDataFolder"];

// Elevated WebView2 150+ ignores environment overrides. Only disposable hosted
// Windows runners get these two app-specific policies; production code is untouched.
// https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/security
export function configureCiWebViewPolicy(userDataFolder, port) {
  if (process.platform !== "win32" || process.env.GITHUB_ACTIONS !== "true") return null;
  if (process.env.RUNNER_ENVIRONMENT !== "github-hosted") throw new Error("WebView2 test policy requires a GitHub-hosted runner.");
  if (!isAbsolute(userDataFolder) || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error("Invalid private WebView2 test configuration.");
  return applyWebViewPolicy(registry, {
    AdditionalBrowserArguments: "--remote-debugging-port=" + port + " --remote-debugging-address=127.0.0.1",
    UserDataFolder: userDataFolder,
  });
}

// Keep rollback testable without changing a developer machine's registry.
export function applyWebViewPolicy(store, values) {
  const previous = names.map((name) => ({ name, state: store.read(name) }));
  for (const entry of previous) {
    if (entry.state.exists && !["String", "ExpandString"].includes(entry.state.kind)) throw new Error("Unexpected existing WebView2 policy type; no policy was changed.");
  }
  function restore() {
    let failed = false;
    for (const entry of previous) {
      try {
        store.write(entry.name, entry.state);
        const actual = store.read(entry.name);
        if (actual.exists !== entry.state.exists || (actual.exists && (actual.kind !== entry.state.kind || actual.value !== entry.state.value))) failed = true;
      } catch { failed = true; }
    }
    if (failed) throw new Error("WebView2 test policy restoration failed.");
  }
  try {
    for (const name of names) {
      const desired = { exists: true, kind: "String", value: values[name] };
      store.write(name, desired);
      const actual = store.read(name);
      if (!actual.exists || actual.kind !== desired.kind || actual.value !== desired.value) throw new Error("Policy readback failed.");
    }
  } catch {
    restore();
    throw new Error("WebView2 test policy setup failed; prior values were restored.");
  }
  return { previousEntries: previous.map(({ name, state }) => ({ name, existed: state.exists, kind: state.kind })), restore };
}

const registry = {
  read: (name) => queryRegistry({ action: "read", name }),
  write: (name, state) => queryRegistry({ action: "write", name, state }),
};

function queryRegistry(request) {
  const script = String.raw`
$ErrorActionPreference = 'Stop'
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$request = [Console]::In.ReadToEnd() | ConvertFrom-Json
if ($request.name -notin @('AdditionalBrowserArguments', 'UserDataFolder')) { throw 'Unexpected policy name.' }
$path = 'SOFTWARE\Policies\Microsoft\Edge\WebView2\' + $request.name
$app = 'milim-desktop.exe'
$key = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($path, $request.action -eq 'write')
try {
  if ($request.action -eq 'write') {
    if ($request.state.exists) {
      if ($null -eq $key) { $key = [Microsoft.Win32.Registry]::LocalMachine.CreateSubKey($path) }
      $kind = [Enum]::Parse([Microsoft.Win32.RegistryValueKind], [string]$request.state.kind)
      $key.SetValue($app, [string]$request.state.value, $kind)
    } elseif ($null -ne $key) { $key.DeleteValue($app, $false) }
    'null'
  } elseif ($request.action -eq 'read') {
    $exists = $null -ne $key -and ($key.GetValueNames() -contains $app)
    if ($exists) {
      @{ exists=$true; kind=$key.GetValueKind($app).ToString(); value=$key.GetValue($app, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } | ConvertTo-Json -Compress
    } else { '{"exists":false,"kind":null,"value":null}' }
  } else { throw 'Unexpected registry operation.' }
} finally { if ($null -ne $key) { $key.Dispose() } }
`;
  const result = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { input: JSON.stringify(request), encoding: "utf8", timeout: 10_000, windowsHide: true });
  // Prior values stay in memory; neither registry output nor stderr enters logs.
  if (result.error || result.status !== 0) throw new Error("WebView2 test registry operation failed.");
  try { return JSON.parse(result.stdout.trim()); }
  catch { throw new Error("Invalid WebView2 test registry response."); }
}
