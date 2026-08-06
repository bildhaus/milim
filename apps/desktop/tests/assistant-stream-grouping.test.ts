import type { ChatStreamPart } from "../src/api.js";
import { groupCompletedStreamActivity } from "../src/lib/streamParts.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
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

const grouped = groupCompletedStreamActivity(parts, false);
equal(grouped.length, 3, "completed mode should expose one work drawer and the final answer");
equal(grouped[0].kind, "workGroup", "intermediate text, tools, and reasoning should become one work group");
if (grouped[0].kind === "workGroup") {
  equal(grouped[0].parts.length, 6, "work group should include intermediate text and terminal activity");
  assert(grouped[0].parts[0].kind === "text" && grouped[0].parts[0].content === "before", "work group should include the intermediate response");
  assert(grouped[0].parts[1].kind === "event" && grouped[0].parts[1].name === "read_file", "work group should preserve first tool");
  assert(grouped[0].parts[3].kind === "thinking", "work group should preserve reasoning");
  assert(grouped[0].parts[4].kind === "event" && grouped[0].parts[4].name === "shell", "work group should preserve later tools");
  assert(grouped[0].parts[5].kind === "event" && grouped[0].parts[5].status === "error", "work group should include failed tools");
}
assert(grouped[1].kind === "event" && grouped[1].status === "running", "running tools should stay flat");
assert(grouped[2].kind === "text" && grouped[2].content === "after", "the final response should stay outside the drawer");

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
