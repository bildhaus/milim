import type { BufferedStreamChunk } from "../src/sessions/store.js";
import { accountRuntimeToolPart, runtimeWarningMessage, statusPart, toolApprovalPart, toolCompletedPart, toolStartedPart } from "../src/lib/turnEvents.js";
import { claimTurnGeneration, createStreamUpdateBatcher, startTurnStream } from "../src/lib/turnStream.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const started = toolStartedPart({
  type: "tool_call",
  name: "edit_file",
  call_id: "call-1",
  arguments: JSON.stringify({ path: "src/App.tsx" }),
} as never);
equal(started.callId, "call-1", "started tool part should preserve call id");
equal(started.label, "Editing file", "started edit_file label should be readable");
equal(started.detail, "src/App.tsx", "started tool part should show target path");

const completed = toolCompletedPart({
  type: "tool_result",
  name: "edit_file",
  call_id: "call-1",
  arguments: JSON.stringify({ path: "src/App.tsx" }),
  result: { added: 3, removed: 1 },
} as never);
equal(completed.callId, "call-1", "completed tool part should preserve call id");
equal(completed.label, "Edited file", "completed edit_file label should be readable");
equal(completed.detail, "src/App.tsx +3 -1", "completed tool part should include diff stats");

const failed = toolCompletedPart({
  type: "tool_result",
  name: "shell",
  call_id: "call-2",
  arguments: JSON.stringify({ command: "npm test" }),
  result: { error: "exit 1" },
} as never);
equal(failed.status, "error", "failed tool part should mark error status");
equal(failed.label, "Command failed", "failed shell tool should use error label");

const longCommand = `node -e "${"x".repeat(140)}"`;
equal(toolStartedPart({
  type: "tool_call",
  name: "shell",
  call_id: "call-long",
  arguments: JSON.stringify({ command: longCommand }),
} as never).detail, longCommand, "command detail should keep its full copyable text");

const scroll = toolStartedPart({
  type: "tool_call",
  name: "scroll",
  call_id: "call-3",
  arguments: JSON.stringify({ delta_y: 400 }),
} as never);
equal(scroll.icon, "screen", "scroll should use the screen icon");
equal(scroll.label, "Using computer", "scroll should be grouped with computer-use tools");

const previewClick = toolStartedPart({
  type: "tool_call",
  name: "preview_click",
  call_id: "call-4",
  arguments: JSON.stringify({ x: 0.25, y: 0.5 }),
} as never);
equal(previewClick.previewPoint?.unit, "ratio", "relative preview coordinates should be marked as ratios");
equal(previewClick.previewPoint?.x, 0.25, "preview click should preserve x");
equal(previewClick.previewPoint?.y, 0.5, "preview click should preserve y");

const previewTargetResult = toolCompletedPart({
  type: "tool_result",
  name: "preview_type_text",
  call_id: "call-5",
  arguments: JSON.stringify({ selector: "#search", text: "query" }),
  result: { ok: true, target: { rect: { x: 10, y: 20, width: 30, height: 10 } } },
} as never);
equal(previewTargetResult.previewPoint?.unit, "pixel", "selector-target preview result should use pixel coordinates");
equal(previewTargetResult.previewPoint?.x, 25, "selector-target preview result should use rect center x");
equal(previewTargetResult.previewPoint?.y, 25, "selector-target preview result should use rect center y");

const runtime = accountRuntimeToolPart({
  type: "tool",
  id: "tool-1",
  name: "shell",
  status: "running",
} as never);
equal(runtime.name, "tool-1", "account runtime tool should prefer runtime id");
equal(runtime.label, "Using shell", "account runtime tool should default running label");
equal(accountRuntimeToolPart({
  type: "tool",
  id: "tool-2",
  name: "edit",
  status: "completed",
} as never).status, "done", "ACP completed tools should normalize to done");
equal(accountRuntimeToolPart({
  type: "tool",
  id: "tool-3",
  name: "edit",
  status: "in_progress",
} as never).status, "running", "ACP in-progress tools should normalize to running");
equal(accountRuntimeToolPart({
  type: "tool",
  id: "tool-4",
  name: "shell",
  status: "failed",
  detail: "full command",
  error: "permission denied",
} as never).detail, "permission denied", "account runtime failures should show their error detail");

const warning = runtimeWarningMessage("Codex not on PATH", "Install Codex");
assert(warning.streamParts?.[0]?.kind === "event", "runtime warning should be an event part");
equal(warning.streamParts[0].eventType, "warning", "runtime warning event should be warning");
equal(statusPart("Error", "boom", "error").status, "error", "error status part should mark error");

const permissionApproval = toolApprovalPart({
  type: "tool_approval_required",
  approval_id: "approval-1",
  name: "permissions",
  arguments: "{}",
  request_kind: "permissions",
  request: { reason: "Needs network", permissions: { network: { enabled: true } } },
} as never);
equal(permissionApproval.label, "Approve additional permissions", "permission approval should have a specific label");
equal(permissionApproval.approvalRequest?.kind, "permissions", "permission details should survive event mapping");

const commandDecision = toolApprovalPart({
  type: "tool_approval_resolved",
  approval_id: "approval-2",
  name: "command",
  arguments: "{\"availableDecisions\":[\"accept\",\"decline\"]}",
  decision: "approve",
} as never);
equal(commandDecision.label, "Command approved", "command decisions should have a readable outcome");
equal(commandDecision.detail, undefined, "resolved decisions should omit protocol payloads");
equal(commandDecision.icon, "command", "command decisions should keep the command icon");

const appended: { sessionId: string; chunks: BufferedStreamChunk[] }[] = [];
const batcher = createStreamUpdateBatcher("s1", {
  appendStreamChunks: (sessionId: string, chunks: BufferedStreamChunk[]) => {
    appended.push({ sessionId, chunks });
  },
} as never);
batcher.appendToken("a");
batcher.appendToken("b");
batcher.appendThinking("c");
batcher.flush();

equal(appended.length, 1, "manual flush should append one batch");
equal(appended[0].sessionId, "s1", "batcher should append to the requested session");
equal(appended[0].chunks.length, 2, "batcher should merge adjacent chunks by kind");
equal(appended[0].chunks[0].content, "ab", "batcher should merge adjacent text tokens");
equal(appended[0].chunks[1].kind, "thinking", "batcher should keep thinking as a separate chunk");

const generating: Array<{ sessionId: string; value: boolean }> = [];
const controllers = { current: new Map<string, AbortController>() };
const startedChunks: BufferedStreamChunk[] = [];
const streamStore = {
  setSessionGenerating: (sessionId: string, value: boolean) => generating.push({ sessionId, value }),
  appendStreamChunks: (_sessionId: string, chunks: BufferedStreamChunk[]) => startedChunks.push(...chunks),
} as never;
const controller = claimTurnGeneration({
  sessionId: "s2",
  generationControllersRef: controllers,
  store: streamStore,
});
assert(controller, "claimTurnGeneration should claim an idle session");
const startedStream = startTurnStream({
  sessionId: "s2",
  controller,
  store: streamStore,
});
assert(controllers.current.get("s2") === startedStream.controller, "claimTurnGeneration should register the controller");
equal(generating.length, 1, "claimTurnGeneration should mark the session generating once");
equal(generating[0].sessionId, "s2", "claimTurnGeneration should mark the requested session generating");
startedStream.append("x");
startedStream.appendThinking("y");
startedStream.streamBatcher.flush();
equal(startedChunks.length, 2, "startTurnStream append helpers should write through the batcher");
