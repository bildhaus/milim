import { strict as assert } from "node:assert";
import {
  mergeControlRunMessages,
  projectControlRunMessages,
} from "../src/lib/canonicalControl.js";
import type { ControlTimelineItemV1 } from "../src/api.js";

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

console.log("canonical control projection tests passed");
