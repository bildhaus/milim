import { strict as assert } from "node:assert";
import {
  controlQueuedMessage,
  mergeControlRunMessages,
  projectControlRunMessages,
} from "../src/lib/canonicalControl.js";
import type { ControlQueuedTurnV1, ControlTimelineItemV1 } from "../src/api.js";

const item = (
  seq: number,
  type: string,
  data: Record<string, unknown>,
): ControlTimelineItemV1 => ({
  id: `event-${seq}`,
  thread_id: "thread-1",
  epoch: "epoch-1",
  seq,
  run_id: "run-1",
  type,
  data,
  created_at_ms: seq,
});

const streaming = projectControlRunMessages(
  [
    item(1, "message", { id: "user-1", role: "user", content: "hello" }),
    item(2, "assistant_delta", { text: "hel", reasoning: "think" }),
    item(3, "assistant_delta", { text: "lo", reasoning: "" }),
  ],
  "run-1",
);
assert.equal(streaming.length, 2);
assert.equal(streaming[1].content, "hello");
assert.equal(streaming[1].id, "control-stream-run-1");

const streamedReasoning = projectControlRunMessages(
  [
    item(1, "assistant_delta", { text: "", reasoning: "first " }),
    item(2, "assistant_delta", { text: "", reasoning: "second " }),
    item(3, "assistant_delta", { text: "answer", reasoning: "" }),
    item(4, "assistant_delta", { text: " continued", reasoning: "" }),
  ],
  "run-1",
)[0];
assert.deepEqual(streamedReasoning.streamParts, [
  { kind: "thinking", content: "first second " },
  { kind: "text", content: "answer continued" },
]);

const completed = projectControlRunMessages(
  [
    item(1, "message", { id: "user-1", role: "user", content: "hello" }),
    item(2, "assistant_delta", { text: "hello", reasoning: "think" }),
    item(3, "message", {
      id: "assistant-1",
      role: "assistant",
      content: "hello",
      reasoning: "think",
    }),
  ],
  "run-1",
);
assert.equal(completed[1].id, "assistant-1");
const approval = projectControlRunMessages(
  [
    item(1, "tool_approval_required", {
      approval_id: "approval-1",
      name: "run_command",
    }),
    item(2, "approval_resolved", {
      approval_id: "approval-1",
      decision: "approve",
    }),
  ],
  "run-1",
)[0];
const approvalPart = approval.streamParts?.at(-1);
assert.equal(
  approvalPart?.kind === "event" ? approvalPart.approvalStatus : undefined,
  "approved",
);
assert.equal(
  mergeControlRunMessages(
    [{ id: "old", role: "user", content: "old" }, ...streaming],
    "run-1",
    completed,
  ).length,
  3,
);

const queued = controlQueuedMessage({
  id: "queue-1",
  thread_id: "thread-1",
  command_id: "command-1",
  accepted_at_ms: 42,
  display_text: "Queued content",
  attachments: [{
    id: "attachment-1",
    name: "note.txt",
    mime: "text/plain",
    size: 4,
    content: "note",
    truncated: false,
  }],
} satisfies ControlQueuedTurnV1);
assert.equal(queued.content, "Queued content");
assert.equal(queued.createdAt, 42);
assert.equal(queued.attachments?.[0].content, "note");

console.log("canonical control projection tests passed");
