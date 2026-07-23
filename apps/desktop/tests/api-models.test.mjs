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

assert.match(api, /const ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS = 5000;/);
assert.ok(picker, "Codex picker function should exist");
assert.match(
  picker,
  /const ctrl = new AbortController\(\);[\s\S]*ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS/,
);
assert.match(picker, /getCodexAccount\(false, ctrl\.signal\)/);
assert.match(
  picker,
  /authFetch\(`\$\{BASE\}\/codex\/models`, \{ signal: ctrl\.signal \}\)/,
);
assert.match(api, /supportedReasoningEfforts/);
assert.match(picker, /inputModalities/);
assert.match(picker, /finally \{[\s\S]*clearTimeout\(timer\);[\s\S]*\}/);
assert.match(api, /export const CLAUDE_MODEL_PREFIX = "claude:";/);
assert.ok(claudePicker, "Claude picker function should exist");
assert.match(claudePicker, /getClaudeStatus\(ctrl\.signal\)/);
assert.match(claudePicker, /CLAUDE_MODEL_PREFIX/);
assert.match(
  claudePicker,
  /supported_efforts: \["low", "medium", "high", "xhigh", "max"\]/,
);
assert.match(claudePicker, /finally \{[\s\S]*clearTimeout\(timer\);[\s\S]*\}/);
assert.match(
  api,
  /Promise\.all\(\[\s*listProviderModelsForPicker\(\),\s*listCodexModelsForPicker\(\),\s*listClaudeModelsForPicker\(\),\s*listOpenCodeModelsForPicker\(\),\s*listPiModelsForPicker\(\),\s*\]\)/,
);
assert.match(api, /export const PI_MODEL_PREFIX = "pi:";/);
assert.match(api, /export async function getPiStatus/);
assert.match(api, /export async function streamPiRun/);
assert.match(api, /`\$\{BASE\}\/pi\/status`/);
assert.match(api, /`\$\{BASE\}\/pi\/run`/);
assert.match(providersManager, /<strong>Installed Pi CLI<\/strong>/);
assert.match(providersManager, /piStatus\.provider_count/);
assert.match(
  providersCss,
  /\.provider-account-card:last-child:nth-child\(odd\)\s*\{\s*grid-column:\s*1\s*\/\s*-1;/,
);
assert.match(
  api,
  /startupProviderRefreshPromise \?\?= invoke<boolean>\(\s*"refresh_provider_models",\s*\)/,
);
assert.match(
  api,
  /const providerRefresh = refreshProviderModelsAtStartup\(\);\s*onModels\(await listModelsDetailed\(\)\);\s*if \(await providerRefresh\) onModels\(await listModelsDetailed\(\)\);/,
);
assert.match(app, /loadStartupModels\(\(models\) =>/);
assert.match(chatView, /loadStartupModels\(\(nextModels\) =>/);
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
