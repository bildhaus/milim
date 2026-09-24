import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const api = readFileSync(join(root, "src", "api.ts"), "utf8").replace(/\r\n/g, "\n");
const app = readFileSync(join(root, "src", "App.tsx"), "utf8");
const chatView = readFileSync(
  join(root, "src", "components", "ChatView.tsx"),
  "utf8",
);
const chatCatalogController = readFileSync(
  join(
    root,
    "src",
    "components",
    "chat",
    "useChatCatalogController.ts",
  ),
  "utf8",
);
// The Providers manager is split across a shell, a rail, pages, and a hook.
const providersManager = [
  ["components", "ProvidersManager.tsx"],
  ["components", "ProvidersRail.tsx"],
  ["components", "ProviderOverview.tsx"],
  ["components", "ProviderPage.tsx"],
  ["components", "RuntimePage.tsx"],
  ["components", "RuntimeAccounts.tsx"],
  ["components", "useAccountRuntimes.ts"],
  ["components", "AccountRuntimeImportDialog.tsx"],
  ["lib", "providerConnections.ts"],
].map((parts) => readFileSync(join(root, "src", ...parts), "utf8")).join("\n");
const runtimePage = readFileSync(join(root, "src", "components", "RuntimePage.tsx"), "utf8");
const providersCss = readFileSync(
  join(root, "src", "components", "ProvidersManager.css"),
  "utf8",
);
const onboarding = readFileSync(
  join(root, "src", "components", "OnboardingFlow.tsx"),
  "utf8",
);
const picker =
  api.match(
    /async function listCodexModelsForPicker\(retry = false\): Promise<ModelInfo\[]> \{[\s\S]*?\n\}\n\nexport interface CodexAccountResponse/,
  )?.[0] ?? "";
const claudePicker =
  api.match(
    /async function listClaudeModelsForPicker\(retry = false\): Promise<ModelInfo\[]> \{[\s\S]*?\n\}\n\nexport async function getClaudeStatus/,
  )?.[0] ?? "";
const harnessRun =
  api.match(
    /export async function streamHarnessRun\([\s\S]*?\n\): Promise<void> \{/,
  )?.[0] ?? "";
const providerRun =
  api.match(
    /export async function streamChat\([\s\S]*?\n\): Promise<void> \{[\s\S]*?\n\}\n\nfunction reasoningEffortBody/,
  )?.[0] ?? "";

assert.match(api, /const ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS = 8_000;/);
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
  /const attempts = retry \? 2 : 1;[\s\S]*new AbortController\(\)[\s\S]*ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS[\s\S]*ACCOUNT_RUNTIME_PICKER_RETRY_DELAY_MS/,
);
assert.match(
  api,
  /accountRuntimeCatalogInFlight/,
);
assert.match(api, /length: Math\.min\(3, tasks\.length\)/);
assert.match(api, /options: \{ retry\?: boolean \} = \{\}/);
assert.match(api, /export const PI_MODEL_PREFIX = "pi:";/);
assert.match(api, /export async function getPiStatus/);
assert.match(api, /export async function streamHarnessRun/);
assert.match(api, /`\$\{BASE\}\/pi\/status`/);
assert.match(api, /\/harnesses\/\$\{encodeURIComponent\(id\)\}\/run/);
assert.match(api, /status\.model_capabilities\?\.\[id\]/);
assert.match(api, /imageInput: metadata\?\.image_input \?\? undefined/);
assert.match(api, /context_length: numberOrUndefined\(metadata\?\.context_length\)/);
assert.match(api, /export async function getAccountRuntimeUpdates/);
assert.match(api, /export async function updateAccountRuntime/);
assert.match(api, /latest_version\?: string \| null/);
assert.match(api, /update_available\?: boolean \| null/);
assert.match(api, /`\$\{BASE\}\/account-runtimes\/updates`/);
assert.match(
  api,
  /`\$\{BASE\}\/account-runtimes\/\$\{encodeURIComponent\(runtime\)\}\/update`/,
);
assert.match(providersManager, /piStatus\.provider_count/);
assert.match(api, /export async function listClaudeThreads/);
assert.match(api, /export async function importClaudeThread/);
assert.match(api, /project_path\?: string \| null;/);
assert.match(api, /if \(options\.all\) url\.searchParams\.set\("all", "true"\);/);
assert.match(runtimePage, /\(runtime === "codex" && ready\) \|\| \(runtime === "claude" && enabled\)/);
assert.match(runtimePage, /label: "Import chats\.\.\."/);
assert.match(providersManager, /Import \{runtimeLabel\} chats/);
assert.match(providersManager, /scope === "all"/);
assert.match(providersManager, /setRuntimeImportGroupSelected/);
assert.match(providersManager, /Importing \$\{progress\.current\} of \$\{progress\.total\}/);
assert.doesNotMatch(providersManager, />Recover chats</);
// Every coding CLI renders through the same page, with at most one emphasized
// action that is never the solid accent button.
assert.match(providersManager, /ACCOUNT_RUNTIME_KINDS\.map\(\(runtime\) =>/);
assert.match(runtimePage, /testId: `\$\{runtime\}-update`/);
assert.match(runtimePage, /providers-emphasis-button/);
assert.doesNotMatch(runtimePage, /btn-accent/);
assert.match(runtimePage, /update\?\.update_available === false\s*\?\s*"Up to date"/);
assert.match(runtimePage, /: "Run CLI updater"/);
assert.match(runtimePage, /if \(enabled\) \{\s*menu\.push\(\{\s*id: "locate"/);
assert.match(
  providersManager,
  /confirmUpdate !== runtime[\s\S]*click Confirm update/,
);
assert.match(providersManager, /data-testid="account-runtimes-update-all"/);
assert.match(
  providersManager,
  /for \(const \[index, runtime\] of targets\.entries\(\)\)[\s\S]*await updateAccountRuntime\(runtime\)/,
);
assert.match(providersManager, /"Confirm update all"/);
assert.match(providersManager, /coding CLI update\$\{attention\.count === 1 \? "" : "s"\} available/);
assert.match(
  providersCss,
  /\.providers-rail-row\s*\{[^}]*grid-template-columns:\s*16px minmax\(0, 1fr\) auto;/,
);
assert.match(
  api,
  /startupProviderRefreshPromise \?\?= invoke<boolean>\(\s*"refresh_provider_models",\s*\)/,
);
assert.match(
  api,
  /const cachedProviderModels = await listProviderModelsForPicker\(\s*STARTUP_PROVIDER_PICKER_TIMEOUT_MS/,
);
assert.match(api, /await Promise\.allSettled\(\[runtimeLoad, refreshedProviderLoad\]\)/);
assert.match(api, /if \(initialModels\.length\) emit\(\)/);
assert.match(app, /loadStartupModels\(\s*\(models\) =>/);
assert.match(chatCatalogController, /loadStartupModels\(\s*\(nextModels\) =>/);
assert.match(chatCatalogController, /modelsRef\.current/);
assert.match(api, /afterSeq\?: number/);
assert.match(api, /url\.searchParams\.set\(\s*"after_seq"/);
assert.match(onboarding, /<ModelPicker/);
assert.match(
  onboarding,
  /const STEPS:[\s\S]*label: "Runtime"[\s\S]*label: "Workspace"/,
);
assert.doesNotMatch(onboarding, /label: "Ready"/);
assert.match(onboarding, /"Open milim"/);
assert.match(runtimePage, /testId=\{`\$\{runtime\}-enabled-toggle`\}/);
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
  /type:\s*"image_generated";\s*id:\s*string;\s*status:\s*string;\s*url:\s*string/,
);
assert.ok(harnessRun, "canonical harness stream function should exist");
assert.match(api, /export interface HarnessRunRequest \{/);
assert.match(api, /native_session_id\?: string;/);
assert.match(api, /persist_session\?: boolean;/);
assert.match(api, /tool_approval_policy\?: ToolApprovalMode;/);
assert.match(api, /tool_approval_grant\?: boolean;/);
assert.match(api, /plan_mode\?: boolean;/);
assert.match(api, /export interface HarnessEventEnvelope \{/);
assert.match(api, /schema_version: typeof HARNESS_EVENT_SCHEMA_VERSION;/);
assert.doesNotMatch(api, /export async function streamCodexRun/);
assert.doesNotMatch(api, /export async function streamClaudeRun/);
assert.doesNotMatch(api, /export async function streamOpenCodeRun/);
assert.doesNotMatch(api, /export async function streamPiRun/);
assert.ok(providerRun, "Provider chat stream should exist");
assert.match(providerRun, /toolContext\?: AgentToolContext/);
assert.match(providerRun, /\{ workspace: toolContext\.workspace \}/);
assert.match(providerRun, /\{ privacy_mode: toolContext\.privacy_mode \}/);
assert.doesNotMatch(providerRun, /tool_approval_policy: toolContext/);
assert.match(api, /\/codex\/login\/chatgpt-device/);
