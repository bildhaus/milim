import type { ChatStreamPart } from "../src/api.js";
import {
  appendPhaseStreamPart,
  coalesceStreamPhases,
  groupCompletedStreamActivity,
} from "../src/lib/streamParts.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function deepEqual<T>(actual: T, expected: T, message: string): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`${message}: expected ${expectedJson}, got ${actualJson}`);
  }
}

function tool(name: string, status: "done" | "running" | "error" = "done"): ChatStreamPart {
  return {
    kind: "event",
    eventType: "tool",
    label: status === "running" ? `Using ${name}` : status === "error" ? `${name} failed` : `Used ${name}`,
    name,
    icon: status === "error" ? "error" : "tool",
    status,
  };
}

const parts: ChatStreamPart[] = [
  { kind: "text", content: "before" },
  tool("read_file"),
  tool("list_dir"),
  { kind: "thinking", content: "checking" },
  tool("shell"),
  tool("edit_file", "error"),
  tool("write_file", "running"),
  { kind: "text", content: "after" },
];

const streaming = groupCompletedStreamActivity(parts, true);
equal(streaming.length, 3, "streaming mode should compact live tool activity");
equal(streaming[0].kind, "text", "text before live tools should keep its order");
equal(streaming[1].kind, "workGroup", "live tools and reasoning should become one work group");
if (streaming[1].kind === "workGroup") {
  equal(streaming[1].parts.length, 6, "live work group should include all tool outcomes and reasoning");
  assert(streaming[1].parts[0].kind === "event" && streaming[1].parts[0].name === "read_file", "live work group should preserve first tool");
  assert(streaming[1].parts[2].kind === "thinking", "live work group should preserve reasoning");
  assert(streaming[1].parts[3].kind === "event" && streaming[1].parts[3].name === "shell", "live work group should preserve later tools");
  assert(streaming[1].parts[4].kind === "event" && streaming[1].parts[4].status === "error", "live work group should include failed tools");
}
equal(streaming[2].kind, "text", "text after live tools should keep its order");

const fragmentedCodexAnswer = groupCompletedStreamActivity([
  { kind: "text", content: "Morning" },
  { kind: "text", content: " tea" },
  { kind: "text", content: " warms" },
  { kind: "text", content: " my" },
  { kind: "text", content: " hands." },
], true);
equal(fragmentedCodexAnswer.length, 1, "streaming Codex answer fragments should render as one block");
assert(
  fragmentedCodexAnswer[0].kind === "text" && fragmentedCodexAnswer[0].content === "Morning tea warms my hands.",
  "streaming Codex answer fragments should preserve their exact combined text",
);

const grouped = groupCompletedStreamActivity(parts, false);
equal(grouped.length, 2, "completed mode should expose one work drawer and the final answer");
equal(grouped[0].kind, "workGroup", "intermediate text, tools, reasoning, and stale terminal starts should become one work group");
if (grouped[0].kind === "workGroup") {
  equal(grouped[0].parts.length, 7, "work group should include intermediate text and all terminal activity");
  assert(grouped[0].parts[0].kind === "text" && grouped[0].parts[0].content === "before", "work group should include the intermediate response");
  assert(grouped[0].parts[1].kind === "event" && grouped[0].parts[1].name === "read_file", "work group should preserve first tool");
  assert(grouped[0].parts[3].kind === "thinking", "work group should preserve reasoning");
  assert(grouped[0].parts[4].kind === "event" && grouped[0].parts[4].name === "shell", "work group should preserve later tools");
  assert(grouped[0].parts[5].kind === "event" && grouped[0].parts[5].status === "error", "work group should include failed tools");
  assert(grouped[0].parts[6].kind === "event" && grouped[0].parts[6].status === "done", "terminal turns should normalize an unmatched running tool to done");
}
assert(grouped[1].kind === "text" && grouped[1].content === "after", "the final response should stay outside the drawer");

const interrupted = groupCompletedStreamActivity([tool("shell", "running")], false, "interrupted");
assert(interrupted[0].kind === "workGroup", "interrupted tool activity should stay in the work drawer");
if (interrupted[0].kind === "workGroup") {
  const interruptedTool = interrupted[0].parts[0];
  assert(
    interruptedTool.kind === "event" &&
      interruptedTool.status === "error" &&
      interruptedTool.label === "shell interrupted",
    "an aborted turn must not render an unmatched tool start as successful",
  );
}

const failedThenSuccessful = groupCompletedStreamActivity([
  tool("shell", "error"),
  tool("shell"),
], false);
equal(failedThenSuccessful.length, 1, "failed and successful commands should share one drawer");
assert(failedThenSuccessful[0].kind === "workGroup", "terminal command outcomes should use the completed work drawer");

const toolOnly = groupCompletedStreamActivity([tool("read_file"), tool("list_dir")], false);
equal(toolOnly.length, 1, "completed tool-only rows should still collapse to a tool group");
assert(toolOnly[0].kind === "workGroup", "tool-only activity should use the completed work drawer");

const liveToolOnly = groupCompletedStreamActivity([tool("read_file"), tool("list_dir")], true);
equal(liveToolOnly.length, 1, "streaming tool-only rows should collapse to one live work group");
assert(liveToolOnly[0].kind === "workGroup", "streaming tool-only group should use the live work summary");

const approvalSummary: ChatStreamPart = {
  kind: "event",
  eventType: "status",
  label: "google_docs_edit approved",
  status: "done",
  approvalId: "approval-1",
  approvalStatus: "approved",
};
const toolsWithApproval = groupCompletedStreamActivity([
  tool("google_docs_edit"),
  approvalSummary,
  tool("google_docs_edit"),
], false);
equal(toolsWithApproval.length, 1, "approval history should not split collapsed tool activity");
assert(toolsWithApproval[0].kind === "workGroup", "approval history should remain inside the work group");

const openRouterFlicker: ChatStreamPart[] = [
  { kind: "text", content: "Let me see what gcloud" },
  { kind: "thinking", content: "working folder..." },
  { kind: "text", content: " has configured." },
];
const coalescedFlicker = coalesceStreamPhases(openRouterFlicker);
deepEqual(
  coalescedFlicker,
  [
    { kind: "text", content: "Let me see what gcloud has configured." },
    { kind: "thinking", content: "working folder..." },
  ],
  "interleaved OpenRouter channels should become one thinking part and one answer in a phase",
);

const liveFlicker = groupCompletedStreamActivity(openRouterFlicker, true);
equal(liveFlicker.length, 2, "live flicker should render one reasoning card and one answer");
assert(liveFlicker[0].kind === "text" && liveFlicker[0].content === "Let me see what gcloud has configured.", "live answer should stay unsplit");
assert(
  liveFlicker[1].kind === "workGroup" &&
    liveFlicker[1].parts[0]?.kind === "thinking" &&
    liveFlicker[1].parts[0].content === "working folder...",
  "live reasoning should keep a stable work wrapper without splitting the answer",
);

const growingLiveWork = groupCompletedStreamActivity([
  { kind: "thinking", content: "checking" },
], true);
const grownLiveWork = groupCompletedStreamActivity([
  { kind: "thinking", content: "checking" },
  tool("shell", "running"),
], true);
assert(
  growingLiveWork[0].kind === "workGroup" && grownLiveWork[0].kind === "workGroup",
  "a live reasoning phase should keep the same display shape when tool activity arrives",
);

const completedFlicker = groupCompletedStreamActivity(openRouterFlicker, false);
equal(completedFlicker.length, 2, "completed flicker should keep the full answer outside the drawer");
const completedFlickerAnswer = completedFlicker.find((part) => part.kind === "text");
const completedFlickerWork = completedFlicker.find((part) => part.kind === "workGroup");
assert(completedFlickerWork?.kind === "workGroup", "completed reasoning should collapse into the work drawer");
assert(completedFlickerAnswer?.kind === "text" && completedFlickerAnswer.content === "Let me see what gcloud has configured.", "completed grouping should not hide the first answer fragment");

const phaseBoundary = coalesceStreamPhases([
  { kind: "thinking", content: "before tools" },
  { kind: "text", content: "I will look." },
  tool("shell"),
  { kind: "text", content: "gcloud is ready." },
  { kind: "thinking", content: "after tools" },
]);
deepEqual(
  phaseBoundary,
  [
    { kind: "thinking", content: "before tools" },
    { kind: "text", content: "I will look." },
    tool("shell"),
    { kind: "text", content: "gcloud is ready." },
    { kind: "thinking", content: "after tools" },
  ],
  "tool events should keep later thinking and text in a new phase",
);

const ingested = appendPhaseStreamPart(
  appendPhaseStreamPart(
    appendPhaseStreamPart(undefined, "text", "Let me see what gcloud"),
    "thinking",
    "working folder...",
  ),
  "text",
  " has configured.",
);
deepEqual(
  ingested,
  [
    { kind: "text", content: "Let me see what gcloud has configured." },
    { kind: "thinking", content: "working folder..." },
  ],
  "phase-aware ingest should append into the current text and thinking parts",
);
