import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const api = readFileSync(join(root, "src", "api.ts"), "utf8");
const app = readFileSync(join(root, "src", "App.tsx"), "utf8");
const chatView = readFileSync(
  join(root, "src", "components", "ChatView.tsx"),
  "utf8",
);
const providersManager = readFileSync(
  join(root, "src", "components", "ProvidersManager.tsx"),
  "utf8",
);
const providersCss = readFileSync(
  join(root, "src", "components", "ProvidersManager.css"),
  "utf8",
);
const picker =
  api.match(
    /async function listCodexModelsForPicker\(\): Promise<ModelInfo\[]> \{[\s\S]*?\n\}\n\nexport interface CodexAccountResponse/,
  )?.[0] ?? "";
const claudePicker =
  api.match(
    /async function listClaudeModelsForPicker\(\): Promise<ModelInfo\[]> \{[\s\S]*?\n\}\n\nexport async function getClaudeStatus/,
  )?.[0] ?? "";
const codexRun =
  api.match(
    /export async function streamCodexRun\([\s\S]*?\n\): Promise<void> \{/,
  )?.[0] ?? "";
const claudeRun =
  api.match(
    /export async function streamClaudeRun\([\s\S]*?\n\): Promise<void> \{/,
  )?.[0] ?? "";

assert.match(api, /const ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS = 12000;/);
assert.match(api, /const ACCOUNT_RUNTIME_PICKER_RETRY_DELAY_MS = 500;/);
assert.ok(picker, "Codex picker function should exist");
assert.match(picker, /discoverAccountRuntimeModels\(async \(signal\) =>/);
assert.match(picker, /getCodexAccount\(false, signal\)/);
assert.match(
  picker,
  /authFetch\(`\$\{BASE\}\/codex\/models`, \{ signal \}\)/,
);
assert.match(api, /supportedReasoningEfforts/);
assert.match(picker, /inputModalities/);
assert.match(api, /export const CLAUDE_MODEL_PREFIX = "claude:";/);
assert.ok(claudePicker, "Claude picker function should exist");
assert.match(claudePicker, /getClaudeStatus\(signal\)/);
assert.match(claudePicker, /CLAUDE_MODEL_PREFIX/);
assert.match(
  claudePicker,
  /supported_efforts: \["low", "medium", "high", "xhigh", "max"\]/,
);
assert.equal(
  (api.match(/discoverAccountRuntimeModels\(/g) ?? []).length,
  5,
  "all four account runtimes should share one discovery retry path",
);
assert.match(
  api,
  /for \(let attempt = 0; attempt < 2; attempt \+= 1\)[\s\S]*new AbortController\(\)[\s\S]*ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS[\s\S]*ACCOUNT_RUNTIME_PICKER_RETRY_DELAY_MS/,
);
assert.match(
  api,
  /accountRuntimeEnabled\.codex\s*\?\s*listCodexModelsForPicker\(\)/,
);
assert.match(api, /accountRuntimeEnabled\.claude\s*\?\s*listClaudeModelsForPicker\(\)/);
assert.match(api, /accountRuntimeEnabled\.opencode\s*\?\s*listOpenCodeModelsForPicker\(\)/);
assert.match(api, /accountRuntimeEnabled\.pi\s*\?\s*listPiModelsForPicker\(\)/);
assert.match(api, /export const PI_MODEL_PREFIX = "pi:";/);
assert.match(api, /export async function getPiStatus/);
assert.match(api, /export async function streamPiRun/);
assert.match(api, /`\$\{BASE\}\/pi\/status`/);
assert.match(api, /`\$\{BASE\}\/pi\/run`/);
assert.match(api, /export async function getAccountRuntimeUpdates/);
assert.match(api, /export async function updateAccountRuntime/);
assert.match(api, /`\$\{BASE\}\/account-runtimes\/updates`/);
assert.match(
  api,
  /`\$\{BASE\}\/account-runtimes\/\$\{encodeURIComponent\(runtime\)\}\/update`/,
);
assert.match(providersManager, /<strong>Installed Pi CLI<\/strong>/);
assert.match(providersManager, /piStatus\.provider_count/);
assert.match(api, /export async function listClaudeThreads/);
assert.match(api, /export async function importClaudeThread/);
assert.match(providersManager, /data-testid="codex-import-chats"/);
assert.match(providersManager, /data-testid="claude-import-chats"/);
assert.match(providersManager, /Import \{runtimeLabel\} chats/);
assert.doesNotMatch(providersManager, />Recover chats</);
for (const runtime of ["codex", "claude", "opencode", "pi"]) {
  assert.match(providersManager, new RegExp(`runtimeUpdateButton\\("${runtime}"`));
}
assert.match(providersManager, /data-testid=\{`\$\{runtime\}-update`\}/);
assert.match(
  providersManager,
  /confirmRuntimeUpdate !== runtime[\s\S]*click Confirm update/,
);
assert.match(
  providersCss,
  /\.provider-account-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\);[^}]*gap:\s*0;/,
);
assert.match(
  api,
  /startupProviderRefreshPromise \?\?= invoke<boolean>\(\s*"refresh_provider_models",\s*\)/,
);
assert.match(
  api,
  /const providerRefresh = refreshProviderModelsAtStartup\(\);\s*onModels\(await listModelsDetailed\(accountRuntimeEnabled\)\);\s*if \(await providerRefresh\)\s*onModels\(await listModelsDetailed\(accountRuntimeEnabled\)\);/,
);
assert.match(app, /loadStartupModels\(\s*\(models\) =>/);
assert.match(chatView, /loadStartupModels\(\s*\(nextModels\) =>/);
for (const runtime of ["codex", "claude", "opencode", "pi"]) {
  assert.match(providersManager, new RegExp(`${runtime}-enabled-toggle`));
}
assert.match(
  api,
  /export type ReasoningEffort\s*=\s*(?:\|\s*)?"auto"\s*\|\s*"none"\s*\|\s*"minimal"\s*\|\s*"low"\s*\|\s*"medium"\s*\|\s*"high"\s*\|\s*"on"\s*\|\s*"xhigh"\s*\|\s*"max";/,
);
assert.match(
  api,
  /function reasoningEffortBody\(reasoningEffort\?: ReasoningEffort\):\s*\{\s*reasoning_effort\?: ReasoningEffort;?\s*\}/,
);
assert.match(
  api,
  /return reasoningEffort && reasoningEffort !== "auto"\s*\?\s*\{ reasoning_effort: reasoningEffort \}\s*:\s*\{\};/,
);
assert.equal(
  (api.match(/reasoningEffortBody\(reasoningEffort\)/g) ?? []).length,
  2,
);
assert.match(
  api,
  /type:\s*"image";\s*id:\s*string;\s*status:\s*string;\s*url:\s*string/,
);
assert.match(codexRun, /thread_id\?: string;/);
assert.match(codexRun, /persist_thread\?: boolean;/);
assert.match(codexRun, /tool_approval_policy\?: ToolApprovalMode;/);
assert.match(codexRun, /tool_approval_grant\?: boolean;/);
assert.match(codexRun, /plan_mode\?: boolean;/);
assert.match(claudeRun, /session_id\?: string;/);
assert.match(claudeRun, /tool_approval_policy\?: ToolApprovalMode;/);
assert.match(claudeRun, /tool_approval_grant\?: boolean;/);
assert.match(claudeRun, /plan_mode\?: boolean;/);
assert.match(api, /\/codex\/login\/chatgpt-device/);
assert.match(api, /\/codex\/login\/api-key/);
