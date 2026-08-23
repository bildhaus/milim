import { strict as assert } from "node:assert";
import {
  controlQueuedMessage,
  hostBusySessionIdsFromBootstrap,
  mailboxMessagesFromTimeline,
  mergeMailboxMessages,
  mergeModelChangeMessages,
  mergeControlRunMessages,
  modelChangeMessagesFromTimeline,
  pendingInputsAfterProjection,
  projectControlRunMessages,
  shouldReconcileControlRunProjection,
  shouldQueueCanonicalFollowup,
} from "../src/lib/canonicalControl.js";
import type { ControlPendingInputV1, ControlQueuedTurnV1, ControlTimelineItemV1 } from "../src/api.js";

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

assert.equal(
  shouldQueueCanonicalFollowup("thread-1", ["thread-1"]),
  true,
  "a Rust-owned busy thread should queue canonically before the local run id reattaches",
);
assert.equal(
  shouldQueueCanonicalFollowup("thread-1", [], "run-1"),
  true,
  "an attached canonical run should queue canonically",
);
assert.equal(shouldQueueCanonicalFollowup("thread-1", []), false);
assert.deepEqual(
  hostBusySessionIdsFromBootstrap({
    threads: [{ id: "thread-1", busy: false, queued_turns: 1 }],
    active_runs: [],
    queued_turns: [{ thread_id: "thread-1" }],
  } as never),
  [],
  "a preserved queue without an active run must not keep the composer in stop mode",
);

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

const interleavedReasoning = projectControlRunMessages(
  [
    item(1, "assistant_delta", { text: "Let me see what gcloud", reasoning: "" }),
    item(2, "assistant_delta", { text: "", reasoning: "working folder... " }),
    item(3, "assistant_delta", { text: " has configured.", reasoning: "" }),
  ],
  "run-1",
)[0];
assert.deepEqual(interleavedReasoning.streamParts, [
  { kind: "text", content: "Let me see what gcloud has configured." },
  { kind: "thinking", content: "working folder... " },
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

const completedWithSteer = projectControlRunMessages(
  [
    item(1, "message", { id: "user-1", role: "user", content: "original" }),
    item(2, "assistant_delta", { text: "working", reasoning: "" }),
    item(3, "message", {
      id: "steer-1",
      role: "user",
      content: "change direction",
      steering: true,
      steeringInboxId: "inbox-steer-1",
    }),
    item(4, "message", { id: "assistant-1", role: "assistant", content: "done" }),
  ],
  "run-1",
);
assert.deepEqual(
  completedWithSteer.map((message) => message.id),
  ["user-1", "steer-1", "assistant-1"],
  "an applied steer must remain a separate user turn without replacing the original prompt",
);
assert.equal(completedWithSteer[1].steering, true);
assert.equal(completedWithSteer[1].steeringInboxId, "inbox-steer-1");
assert.deepEqual(
  mergeControlRunMessages(
    [{ id: "optimistic-user", role: "user", content: "original" }],
    "run-1",
    completedWithSteer,
  ).map((message) => message.id),
  ["optimistic-user", "steer-1", "assistant-1"],
  "run reconciliation must preserve both the optimistic original prompt and the applied steer",
);
const pendingInputs = [
  { id: "inbox-steer-1", kind: "steer" },
  { id: "inbox-steer-2", kind: "steer" },
] as ControlPendingInputV1[];
assert.deepEqual(
  pendingInputsAfterProjection(pendingInputs, completedWithSteer).map((input) => input.id),
  ["inbox-steer-2"],
  "projecting an applied steer must clear only its matching pending indicator",
);

const completedWithProviderCost = projectControlRunMessages(
  [
    item(1, "message", {
      id: "assistant-cost",
      role: "assistant",
      content: "priced",
      metrics: {
        startedAt: 10,
        endedAt: 30,
        durationMs: 20,
        model: "provider:openrouter:openai/gpt-5",
        provider: "OpenRouter",
        usage: {
          prompt_tokens: 120_000,
          completion_tokens: 656,
          total_tokens: 120_656,
          cost_usd: 0.1745104,
        },
        costUsd: 0.1745104,
        costSource: "provider",
      },
    }),
  ],
  "run-1",
)[0];
assert.deepEqual(completedWithProviderCost.metrics, {
  startedAt: 10,
  endedAt: 30,
  durationMs: 20,
  model: "provider:openrouter:openai/gpt-5",
  provider: "OpenRouter",
  usage: {
    prompt_tokens: 120_000,
    completion_tokens: 656,
    total_tokens: 120_656,
  },
  costUsd: 0.1745104,
  costSource: "provider",
});

const longRunTimeline = [
  item(1, "start", {}),
  item(2, "message", { id: "user-long", role: "user", content: "long run" }),
  ...Array.from({ length: 600 }, (_, index) =>
    item(index + 3, "assistant_delta", { text: "", reasoning: `step-${index}` }),
  ),
  item(603, "message", {
    id: "assistant-long",
    role: "assistant",
    content: "done",
    reasoning: "complete reasoning",
  }),
];
const longRunTail = longRunTimeline.slice(-500);
assert.equal(
  shouldReconcileControlRunProjection(longRunTimeline, "run-1", true),
  true,
  "a complete timeline may refresh an existing run projection",
);
assert.equal(
  shouldReconcileControlRunProjection(longRunTail, "run-1", true),
  false,
  "an incomplete tail must not replace an existing full run projection",
);
assert.equal(
  shouldReconcileControlRunProjection(longRunTail, "run-1", false),
  true,
  "an incomplete tail may recover a run that has no renderer projection",
);
assert.equal(
  shouldReconcileControlRunProjection(longRunTimeline, "run-1", true, "run-1"),
  false,
  "background reconciliation must not compete with an attached run poller",
);

const afterUserDeletion = projectControlRunMessages(
  [
    item(1, "message", { id: "user-deleted", role: "user", content: "remove me" }),
    item(2, "message", { id: "assistant-kept", role: "assistant", content: "still here" }),
    {
      ...item(3, "message_deleted", { message_id: "user-deleted" }),
      run_id: null,
    },
  ],
  "run-1",
);
assert.deepEqual(
  afterUserDeletion.map((message) => message.id),
  ["assistant-kept"],
  "a canonical deletion tombstone must remove the projected message even though the tombstone is not run-scoped",
);

const afterAssistantDeletion = projectControlRunMessages(
  [
    item(1, "message", { id: "user-kept", role: "user", content: "keep me" }),
    item(2, "assistant_delta", { text: "temporary", reasoning: "" }),
    item(3, "message", { id: "assistant-deleted", role: "assistant", content: "temporary" }),
    {
      ...item(4, "message_deleted", { message_id: "assistant-deleted" }),
      run_id: null,
    },
  ],
  "run-1",
);
assert.deepEqual(
  afterAssistantDeletion.map((message) => message.id),
  ["user-kept"],
  "deleting a completed assistant message must not resurrect its streamed placeholder",
);

const completedWithOrderedWork = projectControlRunMessages(
  [
    item(1, "assistant_delta", { text: "I'll inspect.", reasoning: "Planning." }),
    item(2, "tool_call", { call_id: "call-1", name: "shell" }),
    item(3, "tool_result", { call_id: "call-1", name: "shell" }),
    item(4, "assistant_delta", { text: "Complete.", reasoning: "" }),
    item(5, "message", {
      id: "assistant-ordered",
      role: "assistant",
      content: "I'll inspect.Complete.",
      reasoning: "Planning.",
    }),
  ],
  "run-1",
)[0];
assert.deepEqual(
  completedWithOrderedWork.streamParts?.map((part) =>
    part.kind === "event"
      ? `${part.kind}:${part.name}:${part.status}`
      : `${part.kind}:${part.content}`,
  ),
  [
    "text:I'll inspect.",
    "thinking:Planning.",
    "event:shell:done",
    "text:Complete.",
  ],
  "completion should preserve the streamed order around tool activity",
);

const reconciledTools = projectControlRunMessages(
  [
    item(1, "tool_call", {
      call_id: "call-1",
      name: "read_file",
    }),
    item(2, "tool_result", {
      call_id: "call-1",
      name: "read_file",
    }),
    item(3, "tool_start", {
      callId: "call-2",
      tool_name: "list_dir",
    }),
    item(4, "tool_end", {
      callId: "call-2",
      tool_name: "list_dir",
    }),
    item(5, "message", {
      id: "assistant-tools",
      role: "assistant",
      content: "done",
    }),
  ],
  "run-1",
)[0];
const reconciledParts = reconciledTools.streamParts ?? [];
assert.equal(reconciledParts.length, 3, "tool results should replace their starts instead of duplicating them");
assert.deepEqual(
  reconciledParts.filter((part) => part.kind === "event").map((part) => part.status),
  ["done", "done"],
  "canonical tool rows should be terminal when the assistant message is complete",
);

const sameNameTools = projectControlRunMessages(
  [
    item(1, "tool_call", { call_id: "call-a", name: "edit_file" }),
    item(2, "tool_call", { call_id: "call-b", name: "edit_file" }),
    item(3, "tool_result", { call_id: "call-a", name: "edit_file" }),
    item(4, "tool_result", { call_id: "call-b", name: "edit_file" }),
  ],
  "run-1",
)[0];
assert.deepEqual(
  sameNameTools.streamParts?.map((part) => part.kind === "event" ? [part.callId, part.status] : null),
  [["call-a", "done"], ["call-b", "done"]],
  "parallel tools with the same name should reconcile by call id",
);

const mismatchedToolIds = projectControlRunMessages(
  [
    item(1, "tool_call", { call_id: "call-left", name: "read_file" }),
    item(2, "tool_result", { call_id: "call-right", name: "read_file" }),
  ],
  "run-1",
)[0];
assert.deepEqual(
  mismatchedToolIds.streamParts?.map((part) => part.kind === "event" ? [part.callId, part.status] : null),
  [["call-left", "running"], ["call-right", "done"]],
  "a present call id should never fall back to a different same-name start",
);

const interruptedCanonicalTool = projectControlRunMessages(
  [
    item(1, "tool_call", { call_id: "call-interrupted", name: "shell" }),
    item(2, "run_status", { status: "aborted" }),
  ],
  "run-1",
)[0];
assert.equal(
  interruptedCanonicalTool.streamTerminalOutcome,
  "interrupted",
  "canonical aborted turns should not let the renderer synthesize a successful tool result",
);

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

const optimisticUser = { id: "local-user", role: "user" as const, content: "hello" };
assert.deepEqual(
  mergeControlRunMessages(
    [{ id: "old", role: "user", content: "old" }, optimisticUser],
    "run-1",
    streaming.filter((message) => message.role === "assistant"),
  ).map((message) => message.id),
  ["old", "local-user", "control-stream-run-1"],
  "an optimistic user turn should stay visible until the control plane echoes it",
);
assert.deepEqual(
  mergeControlRunMessages(
    [{ id: "old", role: "user", content: "old" }, optimisticUser],
    "run-1",
    completed,
  ).map((message) => message.id),
  ["old", "local-user", "assistant-1"],
  "the echoed user turn should reuse the optimistic message id",
);
assert.equal(
  mergeControlRunMessages(
    [{ id: "old", role: "user", content: "old" }, optimisticUser],
    "run-1",
    completed,
  )[1].canonicalId,
  "user-1",
  "the optimistic row must retain the canonical ID used by durable message actions",
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

const mailbox = mailboxMessagesFromTimeline([{
  ...item(30, "mailbox_reply", {
    exchange_id: "exchange-1",
    target_thread_id: "thread-2",
    status: "replied",
    reply: {
      target_title: "Research",
      target_project: "milim",
      content: "The result is ready.",
    },
  }),
  run_id: null,
}]);
assert.equal(mailbox[0].content, "The result is ready.");
assert.equal(mailbox[0].mailboxReply?.targetTitle, "Research");
assert.deepEqual(
  mergeMailboxMessages(
    [{ id: "mailbox-exchange-1", role: "assistant", content: "stale" }, { id: "later", role: "user", content: "later" }],
    mailbox,
  ).map((message) => message.id),
  ["mailbox-exchange-1", "later"],
  "mailbox refreshes should update in place instead of moving replies to the transcript tail",
);

const provenance = projectControlRunMessages([
  item(31, "message", {
    id: "mail-user",
    role: "user",
    content: "Question",
    mailboxOrigin: {
      exchange_id: "exchange-1",
      origin_thread_id: "thread-1",
      origin_title: "Origin",
    },
  }),
], "run-1")[0];
assert.equal(provenance.mailboxOrigin?.origin_title, "Origin");

const modelChanges = modelChangeMessagesFromTimeline([{
  ...item(32, "model_changed", {
    previous_model: "codex:gpt-5",
    model: "claude:sonnet",
  }),
  run_id: null,
}]);
assert.deepEqual(modelChanges[0].modelChange, {
  previousModel: "codex:gpt-5",
  model: "claude:sonnet",
});
assert.deepEqual(
  mergeModelChangeMessages(
    [
      { id: "before", role: "assistant", content: "before", controlSeq: 31 },
      { id: "after", role: "user", content: "after", controlSeq: 33 },
    ],
    modelChanges,
  ).map((message) => message.id),
  ["before", "timeline-event-32", "after"],
  "model changes should keep their canonical position between turns",
);

const coalescedModelChanges = modelChangeMessagesFromTimeline([
  { ...item(33, "model_changed", { previous_model: "codex:gpt-5", model: "claude:sonnet" }), run_id: null },
  { ...item(34, "model_changed", { previous_model: "claude:sonnet", model: "openai:gpt-5" }), run_id: null },
  { ...item(35, "message", { id: "next-user", role: "user", content: "Continue" }), run_id: null },
  { ...item(36, "model_changed", { previous_model: "openai:gpt-5", model: "codex:gpt-5" }), run_id: null },
]);
assert.deepEqual(
  coalescedModelChanges.map((message) => message.modelChange),
  [
    { previousModel: "codex:gpt-5", model: "openai:gpt-5" },
    { previousModel: "openai:gpt-5", model: "codex:gpt-5" },
  ],
  "model switches should coalesce only until the next user message",
);
assert.equal(coalescedModelChanges[0].id, "timeline-event-33");
assert.equal(coalescedModelChanges[0].controlSeq, 34);

const canceledModelChange = modelChangeMessagesFromTimeline([
  { ...item(37, "model_changed", { previous_model: "codex:gpt-5", model: "claude:sonnet" }), run_id: null },
  { ...item(38, "model_changed", { previous_model: "claude:sonnet", model: "codex:gpt-5" }), run_id: null },
]);
assert.deepEqual(
  mergeModelChangeMessages(modelChanges, canceledModelChange),
  [],
  "returning to the original model before a user message should remove the pending divider",
);

console.log("canonical control projection tests passed");
