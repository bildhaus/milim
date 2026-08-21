import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const api = readFileSync(join(root, "src", "api.ts"), "utf8");
const chatView = readFileSync(
  join(root, "src", "components", "ChatView.tsx"),
  "utf8",
);

function section(source, start, end) {
  const startIndex = source.indexOf(start);
  const endIndex = source.indexOf(end, startIndex + start.length);
  assert.notEqual(startIndex, -1, `missing contract start: ${start}`);
  assert.notEqual(endIndex, -1, `missing contract end: ${end}`);
  return source.slice(startIndex, endIndex);
}

const completion = section(
  api,
  "export async function completeChatWithMetrics(",
  "export async function requestComposerCompletion(",
);
assert.match(completion, /options\.toolContext\?\.workspace/);
assert.match(completion, /options\.toolContext\?\.privacy_mode/);

const aiTitle = section(
  chatView,
  "async function maybeGenerateAiThreadTitle(",
  "async function createCompactionCheckpoint(",
);
assert.match(aiTitle, /const titleSettings = store\.getSettings\(sessionId\)/);
assert.match(aiTitle, /workspace: titleSettings\.folder\.trim\(\) \|\| null/);
assert.match(aiTitle, /privacy_mode: titleSettings\.privacy/);
assert.match(aiTitle, /toolContext: titleToolContext/);

const compaction = section(
  chatView,
  "async function createCompactionCheckpoint(",
  "async function compactThreadManually(",
);
assert.match(compaction, /toolContext: AgentToolContext/);
assert.match(compaction, /toolContext: options\.toolContext/);
assert.ok(
  (compaction.match(/milim_context: utilityAccountRuntimeMilimContext/g) ?? [])
    .length >= 2,
  "OpenCode and Pi compaction must carry the captured run context",
);

const goalDecision = section(
  chatView,
  "async function requestGoalDecision(",
  "function goalConversation(",
);
assert.match(
  goalDecision,
  /const decisionSettings = useSessions\.getState\(\)\.getSettings\(sessionId\)/,
);
assert.match(goalDecision, /privacy_mode: decisionSettings\.privacy/);
assert.ok(
  (goalDecision.match(/milim_context:/g) ?? []).length === 1,
  "the canonical harness goal decision must carry immutable context",
);
assert.match(goalDecision, /: decisionMilimContext/);
assert.match(goalDecision, /toolContext: decisionToolContext/);
assert.doesNotMatch(goalDecision, /cwd: folder\.trim\(\)/);

const runTurn = section(
  chatView,
  "async function runTurn(",
  "function send()",
);
const persistIndex = runTurn.indexOf("setMessages(id, convo");
const flushIndex = runTurn.indexOf(
  'await flushDeferredUserStateWrites("milim.sessions")',
);
const claimIndex = runTurn.indexOf("claimTurnGeneration({");
assert.ok(
  persistIndex >= 0 && persistIndex < flushIndex && flushIndex < claimIndex,
  "submitted conversation must durably flush before generation is claimed",
);
assert.match(runTurn, /persistingTurnIdsRef\.current\.has\(id\)/);
assert.match(runTurn, /Milim could not save this turn, so it was not sent/);
assert.match(
  runTurn,
  /runtimeKind[\s\S]*runRef\.current\?\.context[\s\S]*resultStatus === "error"[\s\S]*resultStatus === "aborted"[\s\S]*clearAccountRuntimeKind\(id, runtimeKind\)/,
  "failed or canceled native turns must discard their divergent runtime session",
);

const canonicalModelWrite = section(
  chatView,
  "function writeCanonicalThreadModel(",
  "function requireChatModel()",
);
assert.match(canonicalModelWrite, /kind: "thread\.set_model"/);
assert.match(canonicalModelWrite, /reasoning_effort: reasoningEffort/);

const canonicalTurn = section(
  chatView,
  "async function runCanonicalControlTurn(",
  "async function runTurn(",
);
const modelWriteIndex = canonicalTurn.indexOf(
  "await writeCanonicalThreadModel(sessionId, selectedModel)",
);
const canonicalFlushIndex = canonicalTurn.indexOf(
  "await flushDeferredUserStateWrites()",
);
const canonicalClaimIndex = canonicalTurn.indexOf("claimTurnGeneration({");
assert.ok(
  modelWriteIndex >= 0 &&
    modelWriteIndex < canonicalFlushIndex &&
    canonicalFlushIndex < canonicalClaimIndex,
  "canonical sends must commit the selected model and persisted task state before generation",
);

for (const [name, body] of [
  [
    "normal send",
    section(chatView, "function send()", "function sendArtifactFixPrompt("),
  ],
  [
    "Plan execution",
    section(chatView, "function executePlan(", "/** Re-run the last user turn"),
  ],
  [
    "regenerate",
    section(chatView, "function regenerate()", "/** Replace the user message"),
  ],
  [
    "edit and resend",
    section(chatView, "function editResend(", "async function stopSessionRun("),
  ],
]) {
  assert.match(
    body,
    /runTurnAndDrain\(/,
    `${name} must use the shared pre-claim persistence path`,
  );
}
const goalLoop = section(
  chatView,
  "async function runGoalLoop(",
  "function startGoalRun(",
);
assert.match(
  goalLoop,
  /await runTurn\(/,
  "goal continuation must use the shared pre-claim persistence path",
);

const childReplay = section(
  chatView,
  "function applyPushedChildThreadEvent(",
  "function startChildThreadEvents(",
);
assert.match(childReplay, /rememberWorkerThreadEvent/);
assert.doesNotMatch(
  childReplay,
  /childThreadEventsRef\.current\.delete\(thread\.id\)/,
);

const workerResume = section(
  chatView,
  "async function maybeResumeAfterWorkerRun(",
  "function retryWorkerRunReconciliation(",
);
assert.match(workerResume, /appendWorkerRunSynthesisOnce/);
assert.match(workerResume, /if \(!nextMessages\)/);

assert.match(
  chatView,
  /canSteer=\{Boolean\(canonicalActiveRun\?\.steering\)\}/,
  "steering availability must render from reactive canonical run state",
);
assert.match(
  chatView,
  /data-testid="pending-steer"/,
  "accepted steering must remain visible until the canonical inbox claims it",
);

const canonicalQueueActivation = section(
  chatView,
  "async function activateCanonicalQueuedMessage(",
  "function activateQueuedMessage(",
);
assert.match(canonicalQueueActivation, /kind: "turn\.queue_resume"/);
assert.match(canonicalQueueActivation, /interrupt_active: interrupting/);
assert.doesNotMatch(canonicalQueueActivation, /stopSessionRun|getControlBootstrap|setTimeout/);
