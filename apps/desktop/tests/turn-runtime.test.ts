import { strict as assert } from "node:assert";
import type {
  AgentEvent,
  ChatMessage,
  ChatStreamPart,
  CodexRunEvent,
  MemoryNotice,
  ProviderLimitInfo,
  RunTrace,
  TokenUsage,
} from "../src/api.js";
import {
  claudeCompactionSummaryRequest,
  accountRuntimeInputFromMessages,
  accountRuntimePromptMessages,
  codexCompactionSummaryRequest,
  codexPromptFromMessages,
  createAccountRuntimeEventHandler,
  createAgentRunEventHandler,
  createTurnAssistantStarter,
  createTurnMetricsCapture,
  createTurnRunTraceState,
  finalizeTurnRuntime,
  handleTurnRuntimeError,
  runAccountRuntimeTurn,
  runModelChatTurn,
  runSelectedAccountRuntimeTurn,
  runToolAgentTurn,
} from "../src/lib/turnRuntime.js";

const runtimeDelta = accountRuntimePromptMessages(
  [{ role: "system", content: "Context" }],
  [
    { id: "u1", role: "user", content: "First" },
    { id: "a1", role: "assistant", content: "First answer" },
    { id: "u2", role: "user", content: "Intervening" },
    { id: "a2", role: "assistant", content: "Other model answer" },
    { id: "u3", role: "user", content: "Resume" },
  ],
  "a1",
);
assert.deepEqual(runtimeDelta.map((message) => message.content), [
  "Context",
  "Intervening",
  "Other model answer",
  "Resume",
]);

const startedMessages: ChatMessage[][] = [];
const initialConversation: ChatMessage[] = [{ role: "user", content: "hello" }];
const starter = createTurnAssistantStarter({
  conversation: initialConversation,
  planMode: true,
  setMessages: (messages) => startedMessages.push(messages),
});
assert.equal(starter.state.activeConversation, initialConversation);
starter.beginAssistant(initialConversation);
assert.equal(starter.state.started, true);
assert.equal(startedMessages.length, 1);
assert.equal(startedMessages[0][1].role, "assistant");
assert.equal(startedMessages[0][1].plan?.status, "proposed");
const preparedAgain: ChatMessage[] = [{ role: "user", content: "new" }];
starter.beginAssistant(preparedAgain);
assert.equal(starter.state.activeConversation, preparedAgain);
assert.equal(startedMessages.length, 1);

const codexAttachmentPrompt = codexPromptFromMessages([
  {
    role: "user",
    content: "Use this context.",
    attachments: [
      {
        id: "att-1",
        name: "notes.md",
        mime: "text/markdown",
        size: 7,
        content: "# Notes",
      },
    ],
  },
]);
assert.match(codexAttachmentPrompt, /Use this context\./);
assert.match(
  codexAttachmentPrompt,
  /--- attachment name=notes\.md mime=text\/markdown size=7 ---/,
);
assert.match(codexAttachmentPrompt, /# Notes/);

const codexImageAttachmentPrompt = codexPromptFromMessages([
  {
    role: "user",
    content: "Look at this.",
    attachments: [
      {
        id: "att-image",
        name: "screen.png",
        mime: "image/png",
        size: 4,
        dataUrl: "data:image/png;base64,AAAA",
      },
    ],
  },
]);
assert.match(codexImageAttachmentPrompt, /Look at this\./);
assert.match(
  codexImageAttachmentPrompt,
  /Image attached as multimodal input/,
);
assert.doesNotMatch(codexImageAttachmentPrompt, /OCR/);
assert.deepEqual(
  accountRuntimeInputFromMessages([
    {
      role: "user",
      content: "Look at this.",
      attachments: [
        {
          id: "att-image",
          name: "screen.png",
          mime: "image/png",
          size: 4,
          dataUrl: "data:image/png;base64,AAAA",
        },
      ],
    },
  ]).images,
  [{ media_type: "image/png", data: "AAAA" }],
);

const turnMetrics = createTurnMetricsCapture();
turnMetrics.captureUsage({
  prompt_tokens: 1,
  completion_tokens: 2,
  total_tokens: 3,
});
turnMetrics.captureUsageDelta({
  prompt_tokens: 2,
  completion_tokens: 1,
  total_tokens: 3,
});
assert.equal(turnMetrics.state.usage?.total_tokens, 6);
turnMetrics.captureRuntimeMetrics({
  usage: { prompt_tokens: 4, completion_tokens: 5, total_tokens: 9 },
  cost_usd: 0,
});
turnMetrics.captureRuntimeMetrics({ cost_usd: 0.02 });
turnMetrics.captureProviderLimit({
  provider: "Claude",
  kind: "requests",
  remaining: 2,
});
turnMetrics.captureProviderLimit({
  provider: "Claude",
  kind: "requests",
  remaining: 1,
});
turnMetrics.captureProviderLimit({
  provider: "Claude",
  kind: "tokens",
  remaining: 99,
});
assert.equal(turnMetrics.state.usage?.total_tokens, 9);
assert.equal(turnMetrics.state.costUsd, 0.02);
assert.deepEqual(
  turnMetrics.state.limits
    .map((limit) => `${limit.kind}:${limit.remaining}`)
    .sort(),
  ["requests:1", "tokens:99"],
);

const committedRuns: RunTrace[] = [];
const traceState = createTurnRunTraceState((committed) =>
  committedRuns.push(committed),
);
traceState.snapshot();
const trace: RunTrace = {
  model: "m",
  startedAt: 1,
  steps: [{ name: "shell", startedAt: 2 }],
  status: "running",
};
traceState.runRef.current = trace;
traceState.snapshot();
assert.equal(traceState.runRef.current, trace);
assert.equal(committedRuns.length, 1);
assert.notEqual(committedRuns[0].steps[0], trace.steps[0]);
assert.deepEqual(committedRuns[0], trace);

const text: string[] = [];
const thinking: string[] = [];
let flushes = 0;
const appended: ChatStreamPart[] = [];
const completed: Array<{ name: string; part: ChatStreamPart }> = [];
const metrics: Array<{ usage?: TokenUsage; cost_usd?: number }> = [];
const limits: ProviderLimitInfo[] = [];
const threads: string[] = [];
const images: Extract<CodexRunEvent, { type: "image" }>[] = [];

const handler = createAccountRuntimeEventHandler({
  append: (value) => text.push(value),
  appendThinking: (value) => thinking.push(value),
  flush: () => {
    flushes += 1;
  },
  appendStreamEvent: (part) => appended.push(part),
  completeStreamEvent: (name, part) => completed.push({ name, part }),
  captureRuntimeMetrics: (value) => metrics.push(value),
  captureProviderLimit: (limit) => {
    if (limit) limits.push(limit);
  },
  setCodexThreadId: (threadId) => threads.push(threadId),
  appendImage: (event) => images.push(event),
});

handler.handle({ type: "token", text: "hello" });
handler.handle({ type: "reasoning", text: "thinking" });
handler.handle({
  type: "tool",
  id: "call-1",
  name: "shell",
  status: "running",
});
handler.handle({
  type: "tool",
  id: "call-1",
  name: "shell",
  status: "done",
  label: "Ran shell",
});
handler.handle({ type: "thread", thread_id: "codex-thread", model: "gpt-5" });
handler.handle({
  type: "image",
  id: "img",
  status: "completed",
  url: "https://example.com/image.png",
});
handler.handle({
  type: "rate_limit",
  limit: { provider: "Claude", kind: "requests", remaining: 1 },
});
handler.handle({
  type: "done",
  status: "done",
  usage: { prompt_tokens: 1, completion_tokens: 2, total_tokens: 3 },
  cost_usd: 0.01,
});
handler.handle({ type: "warning", message: "CLI missing" });
handler.handle({
  type: "error",
  message: "boom",
  usage: { prompt_tokens: 4, completion_tokens: 5, total_tokens: 9 },
});

assert.deepEqual(text, ["hello"]);
assert.deepEqual(thinking, ["thinking"]);
assert.equal(flushes, 2);
assert.equal(appended.length, 1);
assert.equal(appended[0].kind, "event");
assert.equal(completed.length, 1);
assert.equal(completed[0].name, "call-1");
assert.equal(completed[0].part.kind, "event");
assert.equal(
  completed[0].part.kind === "event" ? completed[0].part.label : "",
  "Ran shell",
);
assert.deepEqual(threads, ["codex-thread"]);
assert.equal(images[0].url, "https://example.com/image.png");
assert.equal(limits[0].provider, "Claude");
assert.equal(metrics.length, 2);
assert.equal(metrics[0].cost_usd, 0.01);
assert.equal(metrics[1].usage?.total_tokens, 9);
assert.equal(handler.state.warning, "CLI missing");
assert.equal(handler.state.error, "boom");

const run: RunTrace = {
  model: "old",
  startedAt: 10,
  steps: [],
  status: "running",
};
const agentText: string[] = [];
const agentThinking: string[] = [];
let agentFlushes = 0;
let snapshots = 0;
const agentEvents: ChatStreamPart[] = [];
const agentCompleted: Array<{
  name: string;
  callId?: string;
  part: ChatStreamPart;
}> = [];
const memoryNotices: MemoryNotice[] = [];
const childUpserts: string[] = [];
const childUpdates: string[] = [];
let agentUsage: TokenUsage | undefined;
let agentUsageDelta: TokenUsage | undefined;
const agentHandler = createAgentRunEventHandler({
  runRef: { current: run },
  append: (value) => agentText.push(value),
  appendThinking: (value) => agentThinking.push(value),
  flush: () => {
    agentFlushes += 1;
  },
  appendStreamEvent: (part) => agentEvents.push(part),
  completeStreamEvent: (name, part, callId) =>
    agentCompleted.push({ name, part, callId }),
  appendMemoryNotice: (notice) => memoryNotices.push(notice),
  upsertChildThread: (thread) => childUpserts.push(thread.id),
  updateChildThread: (thread) => childUpdates.push(thread.id),
  captureUsage: (usage) => {
    agentUsage = usage;
  },
  captureUsageDelta: (usage) => {
    agentUsageDelta = usage;
  },
  snapshot: () => {
    snapshots += 1;
  },
  now: () => 123,
});

agentHandler({ type: "start", model: "agent-model" });
agentHandler({ type: "token", text: "A" });
agentHandler({ type: "reasoning", text: "B" });
agentHandler({
  type: "tool_call",
  call_id: "tool-1",
  name: "shell",
  arguments: JSON.stringify({ command: "npm test" }),
});
agentHandler({
  type: "tool_result",
  call_id: "tool-1",
  name: "shell",
  result: { ok: true },
});
agentHandler({
  type: "memory_registered",
  id: "memory-1",
  node_id: "node-1",
  scope_kind: "thread",
  scope_label: "Thread",
  summary: "Remember this",
  created_at: "2026-07-03T00:00:00Z",
} as AgentEvent);
agentHandler({
  type: "child_thread_started",
  thread: {
    id: "child-1",
    parent_id: "s1",
    root_id: "s1",
    title: "Worker",
    status: "running",
    model: "m",
    prompt: "p",
    created_at: "",
    updated_at: "",
  },
});
agentHandler({
  type: "child_thread_done",
  thread: {
    id: "child-1",
    parent_id: "s1",
    root_id: "s1",
    title: "Worker",
    status: "done",
    model: "m",
    prompt: "p",
    summary: "Done",
    created_at: "",
    updated_at: "",
  },
});
agentHandler({
  type: "usage_delta",
  usage: { prompt_tokens: 3, completion_tokens: 4, total_tokens: 7 },
});
agentHandler({
  type: "done",
  iterations: 2,
  stopped_at_limit: true,
  usage: { prompt_tokens: 7, completion_tokens: 8, total_tokens: 15 },
});

assert.equal(run.model, "agent-model");
assert.deepEqual(agentText, ["A"]);
assert.deepEqual(agentThinking, ["B"]);
assert.equal(run.steps.length, 1);
assert.equal(run.steps[0].callId, "tool-1");
assert.equal(run.steps[0].startedAt, 123);
assert.equal(run.steps[0].endedAt, 123);
assert.deepEqual(run.steps[0].result, { ok: true });
assert.equal(agentCompleted[0].callId, "tool-1");
assert.equal(memoryNotices[0].summary, "Remember this");
assert.deepEqual(childUpserts, ["child-1"]);
assert.deepEqual(childUpdates, ["child-1"]);
assert.equal(run.status, "stopped");
assert.equal(run.iterations, 2);
assert.equal(agentUsageDelta?.total_tokens, 7);
assert.equal(agentUsage?.total_tokens, 15);
assert(agentFlushes >= 5, "agent handler should flush before event parts");
assert(snapshots >= 6, "agent handler should snapshot state-changing events");

const modelOrder: string[] = [];
const modelSignal = new AbortController().signal;
let streamedMessages: ChatMessage[] = [];
let streamedModel = "";
let streamedUsage: TokenUsage | undefined;
let preparedModelSignal: AbortSignal | undefined;
let streamedModelSignal: AbortSignal | undefined;
await runModelChatTurn({
  promptContext: {
    instructionMessages: [{ role: "system", content: "Be terse." }],
    planMessages: [],
    goalMessages: [],
    skillMessages: [],
    artifactMessages: [],
    memoryMessages: [],
    scheduleMessages: [],
    useScheduleTools: false,
    useTools: false,
    accountRuntimeMayUseTools: false,
    runMemoryContext: {},
    toolContext: {},
  },
  conversation: [{ role: "user", content: "hello" }],
  prepareOutbound: async (contextMessages, conversation, options) => {
    preparedModelSignal = options?.signal;
    modelOrder.push(`prepare:${contextMessages.length}:${conversation.length}`);
    return { conversation, outbound: contextMessages };
  },
  beginAssistant: (conversation) => {
    modelOrder.push(`begin:${conversation.length}`);
  },
  streamChat: async (
    model,
    messages,
    onToken,
    signal,
    onThinking,
    onUsage,
    reasoningEffort,
  ) => {
    streamedModelSignal = signal;
    streamedModel = `${model}:${reasoningEffort ?? ""}`;
    streamedMessages = messages;
    onToken("token");
    onThinking?.("think");
    onUsage?.({ prompt_tokens: 2, completion_tokens: 3, total_tokens: 5 });
  },
  model: "model-a",
  append: (value) => modelOrder.push(`token:${value}`),
  appendThinking: (value) => modelOrder.push(`thinking:${value}`),
  captureUsage: (usage) => {
    streamedUsage = usage;
  },
  signal: modelSignal,
  reasoningEffort: "low",
});
assert.deepEqual(modelOrder, [
  "prepare:1:1",
  "begin:1",
  "token:token",
  "thinking:think",
]);
assert.equal(streamedModel, "model-a:low");
assert.equal(preparedModelSignal, modelSignal);
assert.equal(streamedModelSignal, modelSignal);
assert.deepEqual(streamedMessages, [{ role: "system", content: "Be terse." }]);
assert.equal(streamedUsage?.total_tokens, 5);

const compactionAbortController = new AbortController();
let compactionAbortStreamed = false;
let compactionAbortBegan = false;
try {
  await runModelChatTurn({
    promptContext: {
      instructionMessages: [],
      planMessages: [],
      goalMessages: [],
      skillMessages: [],
      artifactMessages: [],
      memoryMessages: [],
      scheduleMessages: [],
      useScheduleTools: false,
      useTools: false,
      accountRuntimeMayUseTools: false,
      runMemoryContext: {},
      toolContext: {},
    },
    conversation: [{ role: "user", content: "abort during compaction" }],
    prepareOutbound: async (_contextMessages, conversation) => {
      compactionAbortController.abort();
      return { conversation, outbound: conversation };
    },
    beginAssistant: () => {
      compactionAbortBegan = true;
    },
    streamChat: async () => {
      compactionAbortStreamed = true;
    },
    model: "model-a",
    append: () => {},
    appendThinking: () => {},
    captureUsage: () => {},
    signal: compactionAbortController.signal,
  });
  assert.fail("aborted prep should stop before provider stream");
} catch (error) {
  const handled = handleTurnRuntimeError({
    error,
    assistantStarted: compactionAbortBegan,
    append: () =>
      assert.fail("aborted compaction should not append an error bubble"),
    flush: () => {},
    setChatNotice: () =>
      assert.fail("aborted compaction should not show an error notice"),
    appendStreamEvent: () =>
      assert.fail("aborted compaction should not add an error event"),
    runRef: { current: null },
    snapshot: () => {},
    signal: compactionAbortController.signal,
  });
  assert.equal(handled.status, "aborted");
}
assert.equal(compactionAbortBegan, true);
assert.equal(compactionAbortStreamed, false);

const accountPromptContext = {
  instructionMessages: [{ role: "system", content: "System rule" }],
  planMessages: [],
  goalMessages: [],
  skillMessages: [],
  artifactMessages: [],
  memoryMessages: [],
  scheduleMessages: [],
  useScheduleTools: false,
  useTools: false,
  accountRuntimeMayUseTools: true,
  runMemoryContext: {},
  toolContext: {},
} satisfies Parameters<typeof runAccountRuntimeTurn>[0]["promptContext"];
let codexPrompt = "";
let codexThreadId = "";
const accountSignal = new AbortController().signal;
let accountBeginLength = 0;
let accountSkippedAutoCompaction: boolean | undefined;
let accountPreparedSignal: AbortSignal | undefined;
let accountStreamSignal: AbortSignal | undefined;
const accountMetrics: Array<{ usage?: TokenUsage; cost_usd?: number }> = [];
const codexResult = await runAccountRuntimeTurn({
  kind: "codex",
  promptContext: accountPromptContext,
  conversation: [{ role: "user", content: "old user" }],
  prepareOutbound: async (_contextMessages, conversation, options) => {
    accountSkippedAutoCompaction = options?.skipAutoCompaction;
    accountPreparedSignal = options?.signal;
    return {
      conversation: [
        ...conversation,
        { role: "assistant", content: "old assistant" },
        {
          role: "user",
          content: "latest user",
          attachments: [{
            id: "shape",
            name: "shape.png",
            mime: "image/png",
            size: 4,
            dataUrl: "data:image/png;base64,AAAA",
          }],
        },
      ],
      outbound: [],
    };
  },
  beginAssistant: (conversation) => {
    accountBeginLength = conversation.length;
  },
  checkpointWorkspace: async () => {},
  model: "gpt-5",
  workspace: "C:\\work",
  reasoningEffort: "medium",
  toolApproval: "guarded",
  toolApprovalGrant: true,
  planMode: false,
  append: () => {},
  appendThinking: () => {},
  flush: () => {},
  appendStreamEvent: () => {},
  completeStreamEvent: () => {},
  captureRuntimeMetrics: (value) => accountMetrics.push(value),
  threadId: "codex-thread-1",
  setThreadId: (threadId) => {
    codexThreadId = threadId;
  },
  stream: async (request, onEvent, signal) => {
    accountStreamSignal = signal;
    assert.equal(request.thread_id, "codex-thread-1");
    assert.equal(request.persist_thread, true);
    assert.equal(request.tool_approval_grant, true);
    assert.deepEqual(request.images, [{ media_type: "image/png", data: "AAAA" }]);
    codexPrompt = request.prompt;
    onEvent({ type: "thread", thread_id: "codex-thread-2", model: "gpt-5" });
    onEvent({
      type: "done",
      thread_id: "codex-thread-2",
      status: "done",
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    });
  },
  signal: accountSignal,
});
assert.equal(codexResult.status, "done");
assert.equal(accountBeginLength, 3);
assert.equal(accountSkippedAutoCompaction, true);
assert.equal(accountPreparedSignal, accountSignal);
assert.equal(accountStreamSignal, accountSignal);
assert.equal(codexThreadId, "codex-thread-2");
assert(codexPrompt.includes("System:\nSystem rule"));
assert(codexPrompt.includes("User:\nlatest user"));
assert(!codexPrompt.includes("old assistant"));
assert.equal(accountMetrics[0].usage?.total_tokens, 2);

let claudePrompt = "";
const accountWarnings: ChatStreamPart[] = [];
const accountLimits: ProviderLimitInfo[] = [];
const claudeResult = await runAccountRuntimeTurn({
  kind: "claude",
  promptContext: accountPromptContext,
  conversation: [{ role: "user", content: "full user" }],
  prepareOutbound: async (contextMessages, conversation) => ({
    conversation: [
      ...conversation,
      { role: "assistant", content: "full assistant" },
    ],
    outbound: contextMessages,
  }),
  beginAssistant: () => {},
  checkpointWorkspace: async () => {},
  model: "sonnet",
  toolApproval: "review",
  toolApprovalGrant: false,
  planMode: true,
  append: () => {},
  appendThinking: () => {},
  flush: () => {},
  appendStreamEvent: (part) => accountWarnings.push(part),
  completeStreamEvent: () => {},
  captureRuntimeMetrics: () => {},
  captureProviderLimit: (limit) => {
    if (limit) accountLimits.push(limit);
  },
  hadSession: false,
  sessionId: "claude-session-1",
  stream: async (request, onEvent) => {
    assert.equal(request.session_id, "claude-session-1");
    assert.equal(request.plan_mode, true);
    claudePrompt = request.prompt;
    onEvent({
      type: "rate_limit",
      limit: { provider: "Claude", kind: "tokens", remaining: 10 },
    });
    onEvent({ type: "warning", message: "CLI was not found on PATH" });
  },
});
assert.equal(claudeResult.status, "skipped");
assert.equal(claudeResult.error, "CLI was not found on PATH");
assert(claudePrompt.includes("User:\nfull user"));
assert(claudePrompt.includes("Assistant:\nfull assistant"));
assert.equal(accountLimits[0].kind, "tokens");
assert.equal(accountWarnings[0].kind, "event");
assert.equal(
  accountWarnings[0].kind === "event" ? accountWarnings[0].label : "",
  "Claude CLI not on PATH",
);

const codexSummaryRequest = codexCompactionSummaryRequest({
  model: "gpt-5",
  prompt: "summarize",
  cwd: "C:\\work",
  reasoningEffort: "low",
});
assert.equal(codexSummaryRequest.thread_id, undefined);
assert.equal(codexSummaryRequest.persist_thread, false);
assert.equal(codexSummaryRequest.plan_mode, true);
const claudeSummaryRequest = claudeCompactionSummaryRequest({
  model: "sonnet",
  prompt: "summarize",
  cwd: "C:\\work",
  reasoningEffort: "medium",
});
assert.equal(claudeSummaryRequest.session_id, undefined);
assert.equal(claudeSummaryRequest.plan_mode, true);

let selectedCodexThread = "";
let selectedClaudeSessions = 0;
let selectedImageUrl = "";
const selectedAccountResult = await runSelectedAccountRuntimeTurn({
  codexModel: "gpt-5",
  claudeModel: "sonnet",
  accountRuntime: {
    codexThreadId: "stored-codex",
    claudeSessionId: "stored-claude",
  },
  promptContext: accountPromptContext,
  conversation: [{ role: "user", content: "selected" }],
  prepareOutbound: async (_contextMessages, conversation) => ({
    conversation,
    outbound: [],
  }),
  beginAssistant: () => {},
  checkpointWorkspace: async () => {},
  workspace: "C:\\work",
  reasoningEffort: "medium",
  toolApproval: "guarded",
  toolApprovalGrant: false,
  planMode: false,
  append: () => {},
  appendThinking: () => {},
  flush: () => {},
  appendStreamEvent: () => {},
  completeStreamEvent: () => {},
  captureRuntimeMetrics: () => {},
  captureProviderLimit: () => {},
  setCodexThreadId: (threadId) => {
    selectedCodexThread = threadId;
  },
  appendImage: (event) => {
    selectedImageUrl = event.url ?? "";
  },
  ensureClaudeSessionId: () => {
    selectedClaudeSessions += 1;
    return "new-claude";
  },
  streamCodexRun: async (request, onEvent) => {
    assert.equal(request.thread_id, "stored-codex");
    onEvent({ type: "thread", thread_id: "new-codex", model: "gpt-5" });
    onEvent({
      type: "image",
      id: "img-selected",
      status: "completed",
      url: "https://example.com/selected.png",
    });
  },
  streamClaudeRun: async () =>
    assert.fail("codex should win when both account models are present"),
});
assert.equal(selectedAccountResult?.status, "done");
assert.equal(selectedCodexThread, "new-codex");
assert.equal(selectedImageUrl, "https://example.com/selected.png");
assert.equal(selectedClaudeSessions, 0);

let selectedPiSession = "";
const selectedPiResult = await runSelectedAccountRuntimeTurn({
  piModel: "openai-codex/gpt-5.4",
  accountRuntime: {
    piSessionId: "stored-pi",
    piLastSyncedMessageId: "pi-assistant-1",
  },
  promptContext: accountPromptContext,
  conversation: [
    { id: "pi-assistant-1", role: "assistant", content: "Earlier Pi reply" },
    { id: "pi-user-2", role: "user", content: "Continue with this image", attachments: [{
      id: "pi-image",
      name: "screen.png",
      mime: "image/png",
      size: 4,
      dataUrl: "data:image/png;base64,AAAA",
    }] },
  ],
  prepareOutbound: async (_contextMessages, conversation) => ({ conversation, outbound: [] }),
  beginAssistant: () => {},
  checkpointWorkspace: async () => {},
  workspace: "C:\\work",
  reasoningEffort: "high",
  toolApproval: "review",
  toolApprovalGrant: false,
  planMode: false,
  append: () => {},
  appendThinking: () => {},
  flush: () => {},
  appendStreamEvent: () => {},
  completeStreamEvent: () => {},
  captureRuntimeMetrics: () => {},
  captureProviderLimit: () => {},
  setCodexThreadId: () => {},
  ensureClaudeSessionId: () => "unused-claude",
  streamCodexRun: async () => assert.fail("Pi should not dispatch to Codex"),
  streamClaudeRun: async () => assert.fail("Pi should not dispatch to Claude"),
  setPiSessionId: (sessionId) => { selectedPiSession = sessionId; },
  streamPiRun: async (request, onEvent) => {
    assert.equal(request.model, "openai-codex/gpt-5.4");
    assert.equal(request.session_id, "stored-pi");
    assert.equal(request.persist_session, true);
    assert.equal(request.reasoning_effort, "high");
    assert.equal(request.interactive_tool_approval, true);
    assert.deepEqual(request.images, [{ media_type: "image/png", data: "AAAA" }]);
    assert(!request.prompt.includes("Earlier Pi reply"));
    onEvent({ type: "session", session_id: "resumed-pi", model: request.model });
    onEvent({ type: "done", status: "done" });
  },
});
assert.equal(selectedPiResult?.status, "done");
assert.equal(selectedPiSession, "resumed-pi");

let toolRun: RunTrace | null = null;
const toolSignal = new AbortController().signal;
let toolSnapshots = 0;
let toolStartedConversationLength = 0;
let toolStreamAgentId: string | null = "unset";
let toolStreamMessages: ChatMessage[] = [];
let toolPreparedSignal: AbortSignal | undefined;
let toolStreamSignal: AbortSignal | undefined;
await runToolAgentTurn({
  promptContext: {
    instructionMessages: [{ role: "system", content: "Root instruction" }],
    planMessages: [],
    goalMessages: [],
    skillMessages: [],
    artifactMessages: [],
    memoryMessages: [{ role: "system", content: "Memory" }],
    scheduleMessages: [{ role: "system", content: "Schedule" }],
    useScheduleTools: true,
    useTools: true,
    accountRuntimeMayUseTools: false,
    runMemoryContext: { thread_id: "s1" },
    toolContext: { sandbox_enabled: true },
  },
  conversation: [{ role: "user", content: "use tools" }],
  prepareOutbound: async (contextMessages, conversation, options) => {
    toolPreparedSignal = options?.signal;
    return {
      conversation: [
        ...conversation,
        { role: "assistant", content: "prepared" },
      ],
      outbound: contextMessages,
    };
  },
  beginAssistant: (conversation) => {
    toolStartedConversationLength = conversation.length;
  },
  checkpointWorkspace: async () => {},
  streamAgentRun: async (agentId, _model, messages, onEvent, signal) => {
    toolStreamSignal = signal;
    toolStreamAgentId = agentId;
    toolStreamMessages = messages;
    onEvent({ type: "done", iterations: 1 });
  },
  agentId: "agent-1",
  model: "tool-model",
  onEvent: createAgentRunEventHandler({
    runRef: {
      get current() {
        return toolRun;
      },
      set current(next) {
        toolRun = next;
      },
    },
    append: () => {},
    appendThinking: () => {},
    flush: () => {},
    appendStreamEvent: () => {},
    completeStreamEvent: () => {},
    appendMemoryNotice: () => {},
    upsertChildThread: () => {},
    updateChildThread: () => {},
    captureUsage: () => {},
    captureUsageDelta: () => {},
    snapshot: () => {
      toolSnapshots += 1;
    },
    now: () => 22,
  }),
  runMemoryContext: { thread_id: "s1" },
  toolContext: { sandbox_enabled: true },
  runRef: {
    get current() {
      return toolRun;
    },
    set current(next) {
      toolRun = next;
    },
  },
  snapshot: () => {
    toolSnapshots += 1;
  },
  workspace: "C:\\work",
  signal: toolSignal,
  sourceSessionId: "app-1",
  now: () => 11,
});
assert.equal(toolRun?.model, "tool-model");
assert.equal(toolRun?.status, "done");
assert.equal(toolRun?.workspace, "C:\\work");
assert.equal(toolStartedConversationLength, 2);
assert.equal(toolPreparedSignal, toolSignal);
assert.equal(toolStreamSignal, toolSignal);
assert.equal(toolStreamAgentId, "agent-1");
assert.equal(
  toolStreamMessages.some((message) => message.content === "Root instruction"),
  false,
);
assert.equal(
  toolStreamMessages.some((message) => message.content === "Schedule"),
  true,
);
assert(toolSnapshots >= 3);

let failingRun: RunTrace | null = null;
const failing = await runToolAgentTurn({
  promptContext: {
    instructionMessages: [],
    planMessages: [],
    goalMessages: [],
    skillMessages: [],
    artifactMessages: [],
    memoryMessages: [],
    scheduleMessages: [],
    useScheduleTools: false,
    useTools: true,
    accountRuntimeMayUseTools: false,
    runMemoryContext: {},
    toolContext: {},
  },
  conversation: [{ role: "user", content: "fail" }],
  prepareOutbound: async (_contextMessages, conversation) => ({
    conversation,
    outbound: conversation,
  }),
  beginAssistant: () => {},
  checkpointWorkspace: async () => {},
  streamAgentRun: async (_agentId, _model, _messages, onEvent) => {
    onEvent({ type: "error", message: "tool failed" });
  },
  agentId: null,
  model: "tool-model",
  onEvent: createAgentRunEventHandler({
    runRef: {
      get current() {
        return failingRun;
      },
      set current(next) {
        failingRun = next;
      },
    },
    append: () => {},
    appendThinking: () => {},
    flush: () => {},
    appendStreamEvent: () => {},
    completeStreamEvent: () => {},
    appendMemoryNotice: () => {},
    upsertChildThread: () => {},
    updateChildThread: () => {},
    captureUsage: () => {},
    captureUsageDelta: () => {},
    snapshot: () => {},
  }),
  runMemoryContext: {},
  toolContext: {},
  runRef: {
    get current() {
      return failingRun;
    },
    set current(next) {
      failingRun = next;
    },
  },
  snapshot: () => {},
  sourceSessionId: "app-1",
});
assert.equal(failing.status, "error");
assert.equal(failing.error, "tool failed");

const erroredRun: RunTrace = {
  model: "m",
  startedAt: 1,
  steps: [],
  status: "running",
};
const runtimeErrorText: string[] = [];
const runtimeNotices: string[] = [];
const runtimeEvents: ChatStreamPart[] = [];
let runtimeFlushes = 0;
let runtimeSnapshots = 0;
const runtimeError = handleTurnRuntimeError({
  error: new Error("boom"),
  assistantStarted: true,
  append: (value) => runtimeErrorText.push(value),
  flush: () => {
    runtimeFlushes += 1;
  },
  setChatNotice: (notice) => runtimeNotices.push(notice.message),
  appendStreamEvent: (part) => runtimeEvents.push(part),
  runRef: { current: erroredRun },
  snapshot: () => {
    runtimeSnapshots += 1;
  },
  now: () => 99,
});
assert.deepEqual(runtimeError, { status: "error", error: "Error: boom" });
assert.deepEqual(runtimeErrorText, ["\nError: Error: boom"]);
assert.deepEqual(runtimeNotices, ["Error: boom"]);
assert.equal(runtimeEvents[0].kind, "event");
assert.equal(erroredRun.status, "error");
assert.equal(erroredRun.error, "Error: boom");
assert.equal(erroredRun.endedAt, 99);
assert.equal(runtimeFlushes, 2);
assert.equal(runtimeSnapshots, 1);

const abortedRun: RunTrace = {
  model: "m",
  startedAt: 1,
  steps: [],
  status: "running",
};
const abortResult = handleTurnRuntimeError({
  error: new DOMException("stop", "AbortError"),
  assistantStarted: true,
  append: () => assert.fail("abort should not append an error bubble"),
  flush: () => {},
  setChatNotice: () => assert.fail("abort should not show an error notice"),
  appendStreamEvent: () => assert.fail("abort should not add an error event"),
  runRef: { current: abortedRun },
  snapshot: () => {},
  now: () => 100,
});
assert.deepEqual(abortResult, { status: "aborted", error: undefined });
assert.equal(abortedRun.status, "aborted");
assert.equal(abortedRun.error, undefined);
assert.equal(abortedRun.endedAt, 100);

const stringAbortRun: RunTrace = {
  model: "m",
  startedAt: 1,
  steps: [],
  status: "running",
};
const stringAbortController = new AbortController();
stringAbortController.abort();
const stringAbortResult = handleTurnRuntimeError({
  error: new Error("account runtime stopped"),
  assistantStarted: true,
  append: () =>
    assert.fail(
      "signal-aborted runtime errors should not append an error bubble",
    ),
  flush: () => {},
  setChatNotice: () =>
    assert.fail(
      "signal-aborted runtime errors should not show an error notice",
    ),
  appendStreamEvent: () =>
    assert.fail("signal-aborted runtime errors should not add an error event"),
  runRef: { current: stringAbortRun },
  snapshot: () => {},
  signal: stringAbortController.signal,
});
assert.deepEqual(stringAbortResult, { status: "aborted", error: undefined });
assert.equal(stringAbortRun.status, "aborted");

const finalizeOrder: string[] = [];
finalizeTurnRuntime({
  sessionId: "s1",
  model: "model-a",
  status: "done",
  flush: () => finalizeOrder.push("flush"),
  metrics: { startedAt: 1, endedAt: 2, durationMs: 1, model: "model-a" },
  commitResponseMetrics: (sessionId, metrics) =>
    finalizeOrder.push(`metrics:${sessionId}:${metrics.durationMs}`),
  clearController: (sessionId) => finalizeOrder.push(`clear:${sessionId}`),
  setSessionGenerating: (sessionId, generating) =>
    finalizeOrder.push(`generating:${sessionId}:${generating}`),
  setSessionUnread: (sessionId, unread) =>
    finalizeOrder.push(`unread:${sessionId}:${unread}`),
  activeSessionId: "other",
  stopChildThreadEventsIfIdle: (sessionId) =>
    finalizeOrder.push(`stop:${sessionId}`),
  maybeGenerateAiThreadTitle: async (sessionId, model) => {
    finalizeOrder.push(`title:${sessionId}:${model}`);
  },
});
assert.deepEqual(finalizeOrder, [
  "flush",
  "metrics:s1:1",
  "clear:s1",
  "generating:s1:false",
  "unread:s1:true",
  "stop:s1",
  "title:s1:model-a",
]);

const skippedFinalizeOrder: string[] = [];
finalizeTurnRuntime({
  sessionId: "s2",
  model: "model-b",
  status: "skipped",
  flush: () => skippedFinalizeOrder.push("flush"),
  commitResponseMetrics: () =>
    assert.fail("metrics should not commit when no metrics are provided"),
  clearController: (sessionId) =>
    skippedFinalizeOrder.push(`clear:${sessionId}`),
  setSessionGenerating: () => {},
  setSessionUnread: (_sessionId, unread) =>
    skippedFinalizeOrder.push(`unread:${unread}`),
  activeSessionId: "s2",
  stopChildThreadEventsIfIdle: () => skippedFinalizeOrder.push("stop"),
  maybeGenerateAiThreadTitle: async () =>
    assert.fail("skipped turns should not generate titles"),
});
assert.deepEqual(skippedFinalizeOrder, [
  "flush",
  "clear:s2",
  "unread:false",
  "stop",
]);
