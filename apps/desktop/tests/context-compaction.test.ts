import { strict as assert } from "node:assert";
import {
  checkpointMessage,
  compactMessagesForModel,
  compactionSummaryMessages,
  compactionSummaryOutputCap,
  compactionSummaryReasoningEffort,
  contextSendPlan,
  estimateMessagesTokens,
  latestCompactionIndex,
  messagesForModelContext,
  modelContextBudget,
  splitCompactionTail,
  validateCompactionCheckpointSummary,
} from "../src/lib/contextCompaction.js";
import type { ChatMessage, ModelInfo } from "../src/api.js";

const tinyModel: ModelInfo = { id: "tiny", owned_by: "Test", context_length: 1200 };
const localModel: ModelInfo = { id: "local", owned_by: "milim" };
const hostedModel: ModelInfo = { id: "hosted", owned_by: "OpenRouter" };

function user(content: string): ChatMessage {
  return { role: "user", content };
}

function assistant(content: string): ChatMessage {
  return { role: "assistant", content };
}

const short = [user("hello"), assistant("hi")];
const unchanged = compactMessagesForModel(short, "tiny", [tinyModel]);
assert.equal(unchanged.compacted, false);
assert.deepEqual(unchanged.messages, short);

const longMessages: ChatMessage[] = [
  { role: "system", content: "Keep this system setup." },
  user("old user ".repeat(900)),
  assistant("old assistant ".repeat(900)),
  user("new user ".repeat(50)),
];
const compacted = compactMessagesForModel(longMessages, "tiny", [tinyModel]);
assert.equal(compacted.compacted, true);
assert.equal(compacted.messages[0].content, "Keep this system setup.");
assert.match(compacted.messages[1].content, /Context automatically compacted/);
assert.equal(compacted.messages.at(-1)?.content, longMessages.at(-1)?.content);
assert(compacted.sentTokens < compacted.originalTokens);
assert(compacted.sentTokens <= (compacted.budget?.promptBudget ?? 0));

const oversized = compactMessagesForModel(
  [{ role: "system", content: "setup ".repeat(2000) }, user("latest ".repeat(2000))],
  "tiny",
  [tinyModel],
);
assert(oversized.error?.includes("too large"), "oversized current context should fail locally");

assert.equal(modelContextBudget("local", [localModel])?.contextLength, 4096);
assert.equal(modelContextBudget("hosted", [hostedModel])?.contextLength, 32_768);
assert.equal(estimateMessagesTokens([user("12345678")]), 3);
assert.equal(estimateMessagesTokens([user("const x = call(1);")]), 7);
assert.equal(estimateMessagesTokens([user("\u3053\u3093\u306b\u3061\u306f\u4e16\u754c")]), 4, "non-ASCII text should use tokenizer counts");
assert(estimateMessagesTokens([user("こんにちは世界")]) >= 4, "non-ASCII text should not use ASCII chars-per-token estimates");

const checkpoint = checkpointMessage("Keep the API shape stable. Open task: add tests.", {
  auto: false,
  sourceTokens: 1000,
  createdAt: 42,
  baseline: {
    responseCount: 2,
    durationMs: 5000,
    usage: { prompt_tokens: 1200, completion_tokens: 300, total_tokens: 1500 },
    costUsd: 0.01,
  },
  summaryMetrics: {
    model: "hosted",
    provider: "OpenRouter",
    durationMs: 900,
    usage: { prompt_tokens: 800, completion_tokens: 100, total_tokens: 900 },
    costUsd: 0.002,
  },
});
assert.equal(checkpoint.compaction?.baseline?.costUsd, 0.01);
assert.equal(checkpoint.compaction?.summary?.costUsd, 0.002);
const checkpointThread = [
  user("older request that should not be replayed"),
  assistant("older answer that should not be replayed"),
  checkpoint,
  user("continue from the checkpoint"),
];
assert.equal(latestCompactionIndex(checkpointThread), 2);
const checkpointOutbound = messagesForModelContext([{ role: "system", content: "Use terse replies." }], checkpointThread);
assert.equal(checkpointOutbound.length, 3);
assert.equal(checkpointOutbound[0].content, "Use terse replies.");
assert.match(checkpointOutbound[1].content, /Previous thread context checkpoint/);
assert.match(checkpointOutbound[1].content, /Keep the API shape stable/);
assert.equal(checkpointOutbound[2].content, "continue from the checkpoint");
assert(!checkpointOutbound.some((message) => message.content.includes("older request")), "older visible messages should not be replayed after a checkpoint");

const approvalThread: ChatMessage[] = [
  user("please use tools"),
  {
    role: "assistant",
    content: "",
    approval: {
      kind: "tool",
      scope: "reply",
      status: "approved",
      requestedAt: 1,
      resolvedAt: 2,
      model: "tiny",
    },
  },
];
const approvalOutbound = messagesForModelContext([], approvalThread);
assert.deepEqual(approvalOutbound, [approvalThread[0]], "approval cards should stay out of model context");
assert.equal(estimateMessagesTokens(approvalThread), estimateMessagesTokens([approvalThread[0]]), "approval cards should not affect token estimates");

const modelChangeNotice: ChatMessage = {
  role: "system",
  content: "",
  modelChange: { previousModel: "codex:gpt-5", model: "claude:sonnet" },
};
assert.deepEqual(
  messagesForModelContext([], [approvalThread[0], modelChangeNotice]),
  [approvalThread[0]],
  "model-change notices should remain UI provenance rather than model-visible context",
);

const interruptedThread = [
  user("use paper-mcp for this"),
  assistant(""),
  user("what is open in paper?"),
];
assert.deepEqual(
  messagesForModelContext([], interruptedThread),
  [interruptedThread[0], interruptedThread[2]],
  "empty assistant placeholders from interrupted turns should stay out of model context",
);

const sendPlan = contextSendPlan([], longMessages, "tiny", [tinyModel]);
assert.equal(sendPlan.shouldCompact, true, "long uncheckpointed context should request a visible checkpoint");

const reservedRules = [{ role: "system", content: "repository rule ".repeat(900) }] satisfies ChatMessage[];
const fixedOverflow = contextSendPlan(
  [{ role: "system", content: "saved instructions" }],
  [user("latest")],
  "tiny",
  [tinyModel],
  {
    reservedMessages: reservedRules,
    fixedCategories: [
      { label: "saved instructions", tokens: 2 },
      { label: "repository rules", tokens: estimateMessagesTokens(reservedRules) },
    ],
  },
);
assert.equal(fixedOverflow.shouldCompact, false);
assert.match(fixedOverflow.error ?? "", /Fixed context exceeds/);
assert.match(fixedOverflow.error ?? "", /repository rules/);

const oversizedAfterCheckpoint = contextSendPlan(
  [],
  [user("old visible message ".repeat(1000)), checkpoint, user("latest ".repeat(2000))],
  "tiny",
  [tinyModel],
);
assert.equal(oversizedAfterCheckpoint.shouldCompact, false, "messages before the latest checkpoint should not trigger another checkpoint");
assert(oversizedAfterCheckpoint.error?.includes("too large"), "oversized latest context should still fail after a checkpoint");

const initialSummaryCap = compactionSummaryOutputCap("hosted", [hostedModel]);
const retrySummaryCap = compactionSummaryOutputCap("hosted", [hostedModel], true);
assert.equal(initialSummaryCap, 900);
assert(retrySummaryCap > initialSummaryCap, "retry should have more output room than the first summary attempt");
const firstSummaryPrompt = compactionSummaryMessages(longMessages, "hosted", [hostedModel], { outputCapTokens: initialSummaryCap });
assert.match(firstSummaryPrompt[0].content, /under 720 tokens/);
const retrySummaryPrompt = compactionSummaryMessages(longMessages, "hosted", [hostedModel], { retry: true, outputCapTokens: retrySummaryCap });
assert.match(retrySummaryPrompt[0].content, /previous summary was too long or incomplete/i);

assert.equal(
  validateCompactionCheckpointSummary("Keep API shape stable. Open task: add tests.", {
    model: "hosted",
    models: [hostedModel],
    sourceMessages: longMessages,
  }),
  null,
);
assert.match(
  validateCompactionCheckpointSummary("Partial summary", {
    finishReason: "length",
    model: "hosted",
    models: [hostedModel],
  }) ?? "",
  /output limit/,
);
assert.match(
  validateCompactionCheckpointSummary("Open task: update the", {
    model: "hosted",
    models: [hostedModel],
  }) ?? "",
  /mid-sentence/,
);
assert.match(
  validateCompactionCheckpointSummary("word ".repeat(1000), {
    model: "hosted",
    models: [hostedModel],
  }) ?? "",
  /too large/,
);
assert.equal(compactionSummaryReasoningEffort({ name: "LM Studio (local)", base_url: "http://localhost:1234/v1" }), "none");
assert.equal(compactionSummaryReasoningEffort({ name: "OpenRouter", base_url: "https://openrouter.ai/api/v1" }), undefined);

const tailSplit = splitCompactionTail([
  user("old one"),
  assistant("answer one"),
  user("old two"),
  assistant("answer two"),
  user("old three"),
  assistant("answer three"),
], "hosted", [hostedModel]);
assert.deepEqual(tailSplit.head, [user("old one"), assistant("answer one")]);
assert.deepEqual(tailSplit.tail, [user("old two"), assistant("answer two"), user("old three"), assistant("answer three")]);

const hugeAttachment = "attachment-body ".repeat(400);
const hugeToolResult = "tool-result ".repeat(400);
const attachmentPrompt = compactionSummaryMessages([
  {
    role: "user",
    content: "read this",
    attachments: [{ id: "att", name: "huge.txt", mime: "text/plain", size: hugeAttachment.length, content: hugeAttachment }],
  },
], "hosted", [hostedModel])[1].content;
const toolPrompt = compactionSummaryMessages([
  {
    role: "assistant",
    content: "used a tool",
    run: {
      model: "hosted",
      status: "done",
      startedAt: 1,
      steps: [{ name: "shell", startedAt: 1, endedAt: 2, result: hugeToolResult }],
    },
  },
], "hosted", [hostedModel])[1].content;
assert(!attachmentPrompt.includes(hugeAttachment), "compaction prompt should not carry full old attachment bodies");
assert(!toolPrompt.includes(hugeToolResult), "compaction prompt should not carry full old tool results");
assert.match(attachmentPrompt, /Attachment text truncated for compaction/);
assert.match(toolPrompt, /Tool result truncated for compaction/);
