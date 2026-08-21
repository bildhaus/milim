import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

const root = fileURLToPath(new URL("..", import.meta.url));
const chatView = readFileSync(join(root, "src", "components", "ChatView.tsx"), "utf8");
const inspectorController = readFileSync(
  join(root, "src", "components", "chat", "useChatInspectorController.ts"),
  "utf8",
);
const chatMessageRow = readFileSync(
  join(root, "src", "components", "ChatMessageRow.tsx"),
  "utf8",
);
const api = readFileSync(join(root, "src", "api.ts"), "utf8");
const store = readFileSync(join(root, "src", "sessions", "store.ts"), "utf8");

const functionBody = (source, name) =>
  source.match(new RegExp(`(?:async )?function ${name}\\([^]*?\\n  }`))?.[0] ?? "";

const prepareRuntime = functionBody(chatView, "preparePreviewRuntimeForArtifacts");
assert.ok(prepareRuntime, "preview preparation controller should exist");
assert.match(prepareRuntime, /preflightPreviewRuntime/);
assert.doesNotMatch(prepareRuntime, /startPreviewApp|stagePreviewApp/);
assert.doesNotMatch(chatView, /autoPreviewRuntimeStartedRef/);
assert.doesNotMatch(chatView, /\bstagePreviewApp\b/);

const openArtifact = functionBody(chatView, "openArtifactSidePanel");
assert.match(openArtifact, /if \(tab === "preview"\) selectPreviewSource\("artifact"\)/);
assert.equal(openArtifact.match(/selectPreviewSource\("artifact"\)/g)?.length, 1);

const startRuntime = functionBody(inspectorController, "startRuntime");
assert.match(startRuntime, /startPreviewApp\(activePreviewRuntimeKey, options\)/);
assert.match(chatView, /source_fingerprint: activePreviewAppPreflight\.source_fingerprint/);

const restartRuntime = functionBody(inspectorController, "restartRuntime");
assert.match(restartRuntime, /restartPreviewApp\(activePreviewRuntimeKey, options\)/);
assert.doesNotMatch(restartRuntime, /preflightPreviewApp/);

const prepareFix = functionBody(chatView, "sendArtifactFixPrompt");
assert.match(prepareFix, /enqueueQueuedMessage\(activeId, \{ content: text \}\)/);
assert.doesNotMatch(prepareFix, /setInput|setPendingAttachments|runTurnAndDrain/);

assert.match(
  inspectorController,
  /current\?\.thread_id === activePreviewRuntimeKey[\s\S]*?\{ \.\.\.current, stale: true \}/,
);
assert.match(chatView, /artifactSelectionsByThreadRef\.current\.get\(activeId\)/);
assert.match(chatView, /activeSession\?\.browserSession \?\? emptyBrowserSession\(\)/);
assert.match(chatView, /setSessionBrowserSession\(activeId, restoredBrowser\)/);
assert.match(chatView, /previewSourcesByThreadRef\.current\.get\(activeId\)/);
assert.match(chatView, /title=\{inspectorLauncherLabel\}/);
assert.match(chatView, /`Open Code: \$\{/);
assert.match(chatView, /"Open Preview: App"/);
assert.match(chatView, /"Open Workers"/);
assert.match(chatView, /id="inspector-tab-workers"/);
assert.match(chatMessageRow, /openWorkers\(linkedWorkerRun\.run\.id\)/);
assert.match(
  chatView,
  /const runningWorkerRuns = useMemo\([\s\S]*?record\.run\.status === "running"/,
  "running Worker Runs should have a stable transcript-footer projection",
);
assert.match(
  chatView,
  /data-testid="transcript-worker-runs"[\s\S]*?runningWorkerRuns\.map\(\(record\) => \([\s\S]*?<WorkerRunEvent/,
  "running Worker Runs should render after the transcript rows",
);
assert.match(
  chatMessageRow,
  /linkedWorkerRun && linkedWorkerRun\.run\.status !== "running"/,
  "running Worker Runs should not also render inside a mutable message row",
);
assert.match(
  chatView,
  /const liveWorkerRunId =[\s\S]*?if \(liveWorkerRunId\) openWorkersInspector\(liveWorkerRunId\);[\s\S]*?\[activeId, liveWorkerRunId\]/,
  "live Workers should auto-open once without reacting to manual inspector tab changes",
);
assert.doesNotMatch(
  chatView,
  /\[activeWorkerRun\?\.run\.id, activeWorkerRun\?\.run\.status, inspectorTab, sidePanelVisible\]/,
  "manual inspector tab changes should not retrigger the Worker auto-open effect",
);
assert.match(chatView, /inspectorTab === "workers"[\s\S]*?\? true/);
assert.ok(chatView.includes('[data-testid="open-artifact-browser"]'));

assert.match(api, /previewAppUrl\(threadId, "\/preflight"\)/);
assert.match(api, /source_fingerprint/);
assert.match(api, /after_seq=/);
assert.match(store, /inspectorOpen\?: boolean/);
assert.match(store, /inspectorTab\?: SessionInspectorTab/);
assert.match(store, /inspectorByKey: Record<string, SessionInspectorState>/);
assert.match(store, /inspectorStateForSession/);
assert.match(store, /"preview" \| "code" \| "git" \| "workers"/);
assert.doesNotMatch(store, /artifactPanelOpen: open/);
assert.doesNotMatch(store, /sidePanelMode: mode/);
