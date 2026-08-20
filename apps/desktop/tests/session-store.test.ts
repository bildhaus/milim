class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

const {
  DAY_MS,
  DEFAULT_ARCHIVE_RETENTION_DAYS,
  SIDEBAR_CHATS_SECTION_ID,
  SIDEBAR_PINNED_SECTION_ID,
  getSessionComposerDraft,
  inspectorStateForSession,
  setSessionComposerDraft,
  useSessions,
  projectSectionId,
  normalizeVirtualFilePath,
  sessionVirtualProjectFiles,
} = await import("../src/sessions/store.js");
const {
  DEFAULT_GOAL_SETTINGS,
  applyGoalDecision,
  goalChipVisible,
  normalizeGoalSettings,
  parseGoalDecision,
} = await import("../src/lib/goals.js");
const {
  approvalWaitDuration,
  codexLimitsFromRateLimitPayload,
  estimateResponseCostUsd,
  formatCompactProviderLimits,
  formatProviderLimits,
  formatResponseMetrics,
  formatThreadMetrics,
  formatThreadMetricsBreakdown,
  latestProviderLimits,
  responseMetricsForTurn,
  summarizeMilimUsage,
  summarizeResponseMetrics,
} = await import("../src/lib/usageMetrics.js");
const { previewRuntimeKeyForThread } =
  await import("../src/lib/previewRuntimeKeys.js");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(
      `${message}: expected ${String(expected)}, got ${String(actual)}`,
    );
  }
}

function deepEqual(actual: unknown, expected: unknown, message: string): void {
  const a = JSON.stringify(sortKeys(actual));
  const e = JSON.stringify(sortKeys(expected));
  if (a !== e) throw new Error(`${message}: expected ${e}, got ${a}`);
}

function sortKeys(value: unknown): unknown {
  if (!value || typeof value !== "object" || Array.isArray(value)) return value;
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, item]) => [key, sortKeys(item)]),
  );
}

function snapshot<T>(value: T): T {
  return value === undefined
    ? value
    : JSON.parse(JSON.stringify(value)) as T;
}

function inspectorFor(sessionId: string) {
  const state = useSessions.getState();
  return inspectorStateForSession(
    state.inspectorByKey,
    state.sessions.find((session) => session.id === sessionId),
  );
}

const first = useSessions.getState().activeId;
equal(
  useSessions.getState().getSettings(first).planMode,
  false,
  "plan mode should default off",
);
useSessions.getState().updateSettings(first, {
  model: "model-a",
  instructions: "prompt a",
  folder: "C:\\workspace-a",
  sandbox: true,
  computerUse: true,
  memory: false,
  activeAgentId: "agent-a",
  privacy: "redact",
  toolApproval: "open",
  planMode: true,
  reasoningEffort: "high",
});
equal(
  useSessions.getState().getSettings(first).toolApproval,
  "review",
  "changing folders should reset inherited Open approval to Review",
);
useSessions.getState().updateSettings(first, { toolApproval: "open" });
useSessions.getState().setMessages(first, [{ role: "user", content: "hello" }]);
const stableHistory = useSessions.getState().sessions.find((session) => session.id === first)!.messages;
useSessions.getState().setMessages(first, [
  ...stableHistory,
  { id: "stable-new-message", role: "user", content: "next" },
]);
const extendedHistory = useSessions.getState().sessions.find((session) => session.id === first)!.messages;
equal(
  extendedHistory[0],
  stableHistory[0],
  "extending a thread should preserve existing message identity",
);
useSessions.getState().setSessionGenerating(first, true);
deepEqual(
  useSessions.getState().generatingSessionIds,
  [first],
  "generation state should track the running thread id",
);
useSessions.getState().setSessionGenerating(first, true);
deepEqual(
  useSessions.getState().generatingSessionIds,
  [first],
  "generation state should not duplicate running thread ids",
);
useSessions.getState().setSessionGenerating(first, false);
deepEqual(
  useSessions.getState().generatingSessionIds,
  [],
  "generation state should clear the stopped thread id",
);
useSessions.getState().setSessionUnread(first, true);
deepEqual(
  useSessions.getState().unreadSessionIds,
  [],
  "active thread should not be marked unread",
);
const queuedOne = useSessions
  .getState()
  .enqueueQueuedMessage(first, { content: "queued one" });
const queuedTwo = useSessions.getState().enqueueQueuedMessage(first, {
  content: "queued two",
  attachments: [
    {
      id: "queued-attachment",
      name: "note.txt",
      mime: "text/plain",
      size: 4,
      content: "note",
    },
  ],
});
deepEqual(
  useSessions
    .getState()
    .queuedMessagesBySession[first]?.map((item) => item.content),
  ["queued one", "queued two"],
  "queued messages should append per thread",
);
useSessions
  .getState()
  .moveQueuedMessage(first, queuedTwo.id, queuedOne.id, "before");
deepEqual(
  useSessions
    .getState()
    .queuedMessagesBySession[first]?.map((item) => item.id),
  [queuedTwo.id, queuedOne.id],
  "queued messages should move before another item",
);
const persistedMovedQueue = localStorage.getItem("milim.sessions") ?? "";
assert(
  persistedMovedQueue.indexOf(queuedTwo.id) <
    persistedMovedQueue.indexOf(queuedOne.id),
  "queued-message order should persist",
);
equal(
  useSessions.getState().queuedMessagesBySession[first]?.[0]?.attachments?.[0]
    ?.id,
  "queued-attachment",
  "moving a queued message should preserve attachments",
);
useSessions
  .getState()
  .moveQueuedMessage(first, queuedTwo.id, queuedOne.id, "after");
deepEqual(
  useSessions
    .getState()
    .queuedMessagesBySession[first]?.map((item) => item.id),
  [queuedOne.id, queuedTwo.id],
  "queued messages should move after another item",
);
const queuedOrderBeforeInvalidMove = snapshot(
  useSessions.getState().queuedMessagesBySession[first],
);
useSessions
  .getState()
  .moveQueuedMessage(first, queuedOne.id, queuedOne.id, "before");
useSessions
  .getState()
  .moveQueuedMessage(first, queuedOne.id, "missing", "before");
deepEqual(
  useSessions.getState().queuedMessagesBySession[first],
  queuedOrderBeforeInvalidMove,
  "invalid queued-message moves should be ignored",
);
useSessions
  .getState()
  .updateQueuedMessage(first, queuedOne.id, { content: "queued edited" });
equal(
  useSessions.getState().queuedMessagesBySession[first]?.[0]?.content,
  "queued edited",
  "queued message edits should update content",
);
useSessions.getState().removeQueuedMessage(first, queuedTwo.id);
deepEqual(
  useSessions
    .getState()
    .queuedMessagesBySession[first]?.map((item) => item.content),
  ["queued edited"],
  "queued message removal should keep the remaining queue",
);
const shiftedQueued = useSessions.getState().shiftQueuedMessage(first);
equal(
  shiftedQueued?.content,
  "queued edited",
  "shift should return the first queued message",
);
equal(
  useSessions.getState().queuedMessagesBySession[first],
  undefined,
  "shifting the last queued message should clear the thread queue",
);
useSessions.getState().enqueueQueuedMessage(first, { content: "persist me" });
assert(
  localStorage.getItem("milim.sessions")?.includes("persist me"),
  "queued messages should persist in session storage",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("generatingSessionIds"),
  "runtime generation state should not persist in session storage",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("unreadSessionIds"),
  "runtime unread state should not persist in session storage",
);
useSessions.getState().setWorkerRunPending(first, "worker-run-1", true);
deepEqual(
  useSessions
    .getState()
    .sessions.find((session) => session.id === first)?.pendingWorkerRunIds,
  ["worker-run-1"],
  "approved Worker Runs should remain pending on their parent thread",
);
assert(
  localStorage.getItem("milim.sessions")?.includes("worker-run-1"),
  "pending Worker Run joins should survive reload",
);
useSessions.getState().setWorkerRunPending(first, "worker-run-1", false);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === first)?.pendingWorkerRunIds,
  undefined,
  "completed Worker Run joins should clear their persisted marker",
);
const firstUpdatedBeforePreviewRuntime = useSessions
  .getState()
  .sessions.find((session) => session.id === first)?.updatedAt;
useSessions.getState().setPreviewRuntime(first, {
  status: "running",
  cwd: "C:\\Users\\USER\\.milim\\runtime\\preview-apps\\thread",
  url: "http://127.0.0.1:5173/",
  pid: 1234,
  command: "pnpm dev",
});
const firstPreviewRuntimeSession = useSessions
  .getState()
  .sessions.find((session) => session.id === first);
equal(
  firstPreviewRuntimeSession?.previewRuntime?.url,
  "http://127.0.0.1:5173/",
  "preview runtime URL should persist on the thread",
);
equal(
  firstPreviewRuntimeSession?.previewRuntime?.status,
  "running",
  "preview runtime status should persist on the thread",
);
equal(
  firstPreviewRuntimeSession?.updatedAt,
  firstUpdatedBeforePreviewRuntime,
  "preview runtime polling should not change thread recency",
);
assert(
  localStorage.getItem("milim.sessions")?.includes("http://127.0.0.1:5173/"),
  "preview runtime URL should persist in session storage",
);
const projectRuntimeKey = previewRuntimeKeyForThread(
  "other-thread",
  "C:\\workspace-a",
);
useSessions.getState().setPreviewRuntimeByKey(projectRuntimeKey, {
  kind: "static",
  status: "running",
  cwd: "C:\\workspace-a",
  url: "http://127.0.0.1:5999/",
});
equal(
  useSessions.getState().previewRuntimesByKey[projectRuntimeKey]?.url,
  "http://127.0.0.1:5999/",
  "project preview runtime should persist by runtime key",
);
equal(
  useSessions.getState().previewRuntimesByKey[projectRuntimeKey]?.kind,
  "static",
  "static preview kind should persist by runtime key",
);
assert(
  localStorage.getItem("milim.sessions")?.includes(projectRuntimeKey),
  "project runtime key should persist in session storage",
);
const activeBeforeModelSwitchTest = useSessions.getState().activeId;
useSessions.getState().newChat({
  model: "model-before-switch",
  folder: "C:\\workspace-a",
  sandbox: true,
  computerUse: true,
  memory: true,
  privacy: "redact",
  toolApproval: "open",
  planMode: true,
});
const modelSwitchSession = useSessions.getState().activeId;
const modelSwitchRuntimeKey = previewRuntimeKeyForThread(
  modelSwitchSession,
  "C:\\workspace-a",
);
const switchQueued = useSessions.getState().enqueueQueuedMessage(modelSwitchSession, {
  content: "switch model after this",
});
useSessions.getState().upsertVirtualFiles(modelSwitchSession, [
  { path: "src/switch-context.ts", content: "export const preserved = true;" },
]);
useSessions.getState().setPreviewRuntime(modelSwitchSession, {
  status: "running",
  cwd: "C:\\workspace-a",
  url: "http://127.0.0.1:6001/",
  pid: 4321,
  command: "pnpm dev",
});
useSessions.getState().setPreviewRuntimeByKey(modelSwitchRuntimeKey, {
  status: "running",
  cwd: "C:\\workspace-a",
  url: "http://127.0.0.1:6002/",
  command: "pnpm preview",
});
useSessions.getState().setInspectorTab(modelSwitchSession, "code");
const settingsBeforeModelSwitch = snapshot(
  useSessions.getState().getSettings(modelSwitchSession),
);
const sessionBeforeModelSwitch = useSessions
  .getState()
  .sessions.find((session) => session.id === modelSwitchSession);
assert(sessionBeforeModelSwitch, "model switch preservation session should exist");
const queuedBeforeModelSwitch = snapshot(
  useSessions.getState().queuedMessagesBySession[modelSwitchSession],
);
const virtualFilesBeforeModelSwitch = snapshot(sessionBeforeModelSwitch.virtualFiles);
const previewRuntimeBeforeModelSwitch = snapshot(
  sessionBeforeModelSwitch.previewRuntime,
);
const projectRuntimeBeforeModelSwitch = snapshot(
  useSessions.getState().previewRuntimesByKey[modelSwitchRuntimeKey],
);
const inspectorBeforeModelSwitch = snapshot(inspectorFor(modelSwitchSession));
useSessions.getState().updateSettings(modelSwitchSession, { model: "model-after-switch" });
const settingsAfterModelSwitch = useSessions.getState().getSettings(modelSwitchSession);
equal(
  settingsAfterModelSwitch.model,
  "model-after-switch",
  "model switch should update only the selected model",
);
for (const key of [
  "folder",
  "sandbox",
  "computerUse",
  "memory",
  "privacy",
  "toolApproval",
  "planMode",
] as const) {
  deepEqual(
    settingsAfterModelSwitch[key],
    settingsBeforeModelSwitch[key],
    `model switch should preserve ${key}`,
  );
}
const sessionAfterModelSwitch = useSessions
  .getState()
  .sessions.find((session) => session.id === modelSwitchSession);
assert(sessionAfterModelSwitch, "model switch preservation session should remain");
deepEqual(
  useSessions.getState().queuedMessagesBySession[modelSwitchSession],
  queuedBeforeModelSwitch,
  "model switch should preserve queued messages",
);
equal(
  useSessions.getState().queuedMessagesBySession[modelSwitchSession]?.some(
    (message) => message.id === switchQueued.id,
  ),
  true,
  "model switch should keep queued message ids stable",
);
deepEqual(
  sessionAfterModelSwitch.virtualFiles,
  virtualFilesBeforeModelSwitch,
  "model switch should preserve virtual project files",
);
deepEqual(
  sessionAfterModelSwitch.previewRuntime,
  previewRuntimeBeforeModelSwitch,
  "model switch should preserve thread preview runtime",
);
deepEqual(
  useSessions.getState().previewRuntimesByKey[modelSwitchRuntimeKey],
  projectRuntimeBeforeModelSwitch,
  "model switch should preserve project preview runtime",
);
deepEqual(
  inspectorFor(modelSwitchSession),
  inspectorBeforeModelSwitch,
  "model switch should preserve inspector shell state",
);
useSessions.getState().setPreviewRuntimeByKey(modelSwitchRuntimeKey, undefined);
useSessions.getState().remove(modelSwitchSession);
useSessions.getState().switchTo(activeBeforeModelSwitchTest);
useSessions
  .getState()
  .setAccountRuntime(first, {
    codexThreadId: "codex-thread-1",
    codexLastSyncedMessageId: "assistant-1",
  });
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime?.codexThreadId,
  "codex-thread-1",
  "Codex thread id should be stored on the Milim session",
);
useSessions.getState().clearAccountRuntime(first);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime,
  undefined,
  "clearing account runtime should reset native session ids",
);
useSessions
  .getState()
  .setAccountRuntime(first, {
    codexThreadId: "codex-thread-1",
    codexLastSyncedMessageId: "assistant-1",
  });
const claudeSessionId = useSessions.getState().ensureClaudeSessionId(first);
equal(
  useSessions.getState().ensureClaudeSessionId(first),
  claudeSessionId,
  "Claude session id should be stable per Milim session",
);
assert(
  localStorage.getItem("milim.sessions")?.includes("codex-thread-1"),
  "account runtime ids should persist in session storage",
);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime?.codexLastSyncedMessageId,
  "assistant-1",
  "account runtime sync cursors should persist with native ids",
);
useSessions.getState().setAccountRuntime(first, {
  piSessionId: "pi-session-1",
  piLastSyncedMessageId: "assistant-pi-1",
});
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime?.piSessionId,
  "pi-session-1",
  "Pi session id should persist on the Milim session",
);
useSessions.getState().clearAccountRuntimeKind(first, "pi");
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime?.piSessionId,
  undefined,
  "clearing Pi should preserve the other account runtimes and remove only Pi",
);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.accountRuntime?.codexThreadId,
  "codex-thread-1",
  "clearing Pi should preserve Codex",
);
useSessions.getState().setPendingHotSwap(first, {
  fromModel: "model-a",
  toModel: "model-b",
  action: "review",
  nativeSessionMode: "fresh",
  createdAt: Date.now(),
});
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.pendingHotSwap?.action,
  "review",
  "pending Hot Swap action should persist on the thread",
);
useSessions.getState().updateSettings(first, { model: "manual-model" });
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.pendingHotSwap,
  undefined,
  "manual model changes should clear pending Baton state",
);
useSessions.getState().updateSettings(first, { model: "model-a" });
useSessions.getState().setMessages(first, [
  { role: "user", content: "transient stream" },
  { role: "assistant", content: "", streamParts: [] },
]);
useSessions.getState().setSessionGenerating(first, true);
useSessions
  .getState()
  .appendStreamChunks(first, [{ kind: "text", content: "transient-token" }]);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.messages[1]?.content,
  "transient-token",
  "live streamed text should remain visible in the session store",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("transient-token"),
  "live streamed text should stay out of persistence while generating",
);
const streamingPersistSnapshot = localStorage.getItem("milim.sessions");
useSessions.getState().commitResponseMetrics(first, {
  startedAt: 1,
  endedAt: 2,
  durationMs: 1,
  model: "stream-model",
  provider: "Stream Provider",
});
assert(
  localStorage.getItem("milim.sessions") !== streamingPersistSnapshot,
  "non-token session state should keep persisting while generating",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"durationMs":1'),
  "response metadata should persist while its token body remains transient",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("transient-token"),
  "persisting response metadata should not persist the live token body",
);
const background = useSessions.getState().newChat();
useSessions.getState().setMessages(background, [
  { role: "user", content: "background state survives" },
]);
useSessions.getState().enqueueQueuedMessage(background, {
  content: "background queue survives",
});
useSessions.getState().switchTo(first);
assert(
  localStorage.getItem("milim.sessions")?.includes("background state survives"),
  "another thread should persist while the first thread is generating",
);
assert(
  localStorage.getItem("milim.sessions")?.includes("background queue survives"),
  "another thread's queue should persist while the first thread is generating",
);
useSessions.getState().setSessionGenerating(first, false);
assert(
  localStorage.getItem("milim.sessions")?.includes("transient-token"),
  "completed streamed text should persist once generation stops",
);
const completedStreamSnapshot = JSON.parse(
  localStorage.getItem("milim.sessions") ?? "{}",
);
const completedAssistant = completedStreamSnapshot.state.sessions.find(
  (session: { id: string }) => session.id === first,
)?.messages[1];
assert(
  !completedAssistant?.streamParts?.some(
    (part: { kind: string }) => part.kind === "text",
  ),
  "persisted stream parts should omit reconstructable text chunks",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"durationMs":1'),
  "response metrics should remain persisted after generation stops",
);
setSessionComposerDraft(first, "unsent draft");
equal(
  getSessionComposerDraft(first),
  "unsent draft",
  "composer drafts should be tracked per session",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("unsent draft"),
  "composer drafts should not dirty the full session snapshot",
);
setSessionComposerDraft(first, "");
equal(
  getSessionComposerDraft(first),
  "",
  "clearing a composer draft should remove it from the draft cache",
);
equal(
  DEFAULT_GOAL_SETTINGS.developerMaxTurns,
  null,
  "goal max-turn cap should default to uncapped",
);
equal(
  goalChipVisible(DEFAULT_GOAL_SETTINGS),
  false,
  "empty idle goals should not show the composer chip",
);
useSessions.getState().updateSettings(first, {
  goal: {
    objective: "Ship autonomous goals",
    successCriteria: "The loop can continue and complete itself.",
    constraints: "Keep it local to this thread.",
    status: "running",
    lastReason: "Goal run started.",
    nextPrompt: "Continue the implementation.",
    turns: 1,
    startedAt: 1_800_000_000_000,
    updatedAt: 1_800_000_000_001,
    developerMaxTurns: null,
  },
});
equal(
  useSessions.getState().getSettings(first).goal.status,
  "running",
  "live goal state should allow running",
);
equal(
  goalChipVisible(useSessions.getState().getSettings(first).goal),
  true,
  "running goals should show the composer chip",
);
assert(
  localStorage
    .getItem("milim.sessions")
    ?.includes('"objective":"Ship autonomous goals"'),
  "goal state should persist in session storage",
);
equal(
  normalizeGoalSettings(
    { ...useSessions.getState().getSettings(first).goal, status: "running" },
    { pauseRunning: true },
  ).status,
  "paused",
  "hydration should convert running goals to paused",
);
deepEqual(
  parseGoalDecision("{bad json"),
  { status: "blocked", reason: "Goal decision was not valid JSON.", next: "" },
  "invalid goal decision JSON should block the goal",
);
deepEqual(
  parseGoalDecision(
    '{"status":"continue","reason":"More work remains.","next":"Do step two."}',
  ),
  { status: "continue", reason: "More work remains.", next: "Do step two." },
  "strict goal decision JSON should parse",
);
deepEqual(
  parseGoalDecision(
    '```json\n{"status":"complete","reason":"Done.","next":""}\n```',
  ),
  { status: "complete", reason: "Done.", next: "" },
  "fenced goal decision JSON should parse",
);
deepEqual(
  parseGoalDecision(
    'Decision:\n{"status":"blocked","reason":"Missing {external} input.","next":""}\nThanks.',
  ),
  { status: "blocked", reason: "Missing {external} input.", next: "" },
  "prose-wrapped goal decision JSON should parse",
);
deepEqual(
  parseGoalDecision('{"status":"continue","reason":"unfinished","next":"'),
  { status: "blocked", reason: "Goal decision was not valid JSON.", next: "" },
  "incomplete goal decision JSON should block the goal",
);
let loopGoal = normalizeGoalSettings({
  ...DEFAULT_GOAL_SETTINGS,
  objective: "Loop test",
  status: "running",
});
loopGoal = applyGoalDecision(loopGoal, {
  status: "continue",
  reason: "More work remains.",
  next: "Do step two.",
});
equal(
  loopGoal.status,
  "running",
  "continue decision should keep the goal running",
);
equal(loopGoal.turns, 1, "continue decision should increment turns");
loopGoal = applyGoalDecision(loopGoal, {
  status: "continue",
  reason: "One more step remains.",
  next: "Do final step.",
});
equal(
  loopGoal.nextPrompt,
  "Do final step.",
  "continue decision should store the next prompt",
);
loopGoal = applyGoalDecision(loopGoal, {
  status: "complete",
  reason: "All criteria satisfied.",
  next: "",
});
equal(loopGoal.status, "complete", "complete decision should stop the loop");
equal(loopGoal.turns, 3, "complete decision should increment the final turn");
equal(
  goalChipVisible(
    normalizeGoalSettings({ ...loopGoal, updatedAt: 10, lastSeenAt: null }),
  ),
  true,
  "unread complete goals should show the composer chip",
);
equal(
  goalChipVisible(
    normalizeGoalSettings({ ...loopGoal, updatedAt: 10, lastSeenAt: 10 }),
  ),
  false,
  "read complete goals should hide the composer chip",
);
useSessions.getState().updateSettings(first, {
  goal: {
    ...DEFAULT_GOAL_SETTINGS,
    objective: "Edited goal",
    successCriteria: "New criteria",
  },
});
equal(
  useSessions.getState().getSettings(first).goal.objective,
  "Edited goal",
  "goal edits should update the objective",
);
equal(
  useSessions.getState().getSettings(first).goal.successCriteria,
  "New criteria",
  "goal edits should update success criteria",
);
equal(
  goalChipVisible(useSessions.getState().getSettings(first).goal),
  false,
  "saved idle goals should not show the composer chip",
);
useSessions.getState().updateSettings(first, { goal: DEFAULT_GOAL_SETTINGS });
deepEqual(
  useSessions.getState().getSettings(first).goal,
  DEFAULT_GOAL_SETTINGS,
  "deleting a goal should clear goal state",
);

useSessions.getState().setMessages(first, [
  { role: "user", content: "stream please" },
  { role: "assistant", content: "", streamParts: [] },
]);
useSessions.getState().appendStreamChunks(first, [
  { kind: "text", content: "Hello " },
  { kind: "text", content: "world" },
  { kind: "thinking", content: "checking" },
  { kind: "text", content: "!" },
]);
const streamedAssistant = useSessions
  .getState()
  .sessions.find((session) => session.id === first)?.messages[1];
equal(
  streamedAssistant?.content,
  "Hello world!",
  "batched stream chunks should append visible text",
);
deepEqual(
  streamedAssistant?.streamParts,
  [
    { kind: "text", content: "Hello world!" },
    { kind: "thinking", content: "checking" },
  ],
  "batched stream chunks should coalesce interleaved text around thinking",
);
useSessions.getState().appendStreamEvent(first, {
  kind: "event",
  eventType: "tool",
  label: "Using shell",
  name: "shell",
  callId: "call-a",
  status: "running",
});
useSessions.getState().appendStreamEvent(first, {
  kind: "event",
  eventType: "tool",
  label: "Using shell",
  name: "shell",
  callId: "call-b",
  status: "running",
});
useSessions.getState().completeStreamEvent(
  first,
  "shell",
  {
    kind: "event",
    eventType: "tool",
    label: "Used shell",
    name: "shell",
    callId: "call-a",
    status: "done",
  },
  "call-a",
);
const toolParts = (
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.messages[1].streamParts ?? []
).filter((part) => part.kind === "event" && part.name === "shell");
const firstToolPart = toolParts[0];
const secondToolPart = toolParts[1];
assert(firstToolPart?.kind === "event", "first shell part should be an event");
assert(
  secondToolPart?.kind === "event",
  "second shell part should be an event",
);
equal(
  firstToolPart.status,
  "done",
  "tool result should complete the matching call id",
);
equal(
  secondToolPart.status,
  "running",
  "same-name later tool call should remain running",
);
useSessions.getState().appendStreamEvent(first, {
  kind: "event",
  eventType: "status",
  label: "Approve google_docs_edit",
  detail: "{\"file_id\":\"doc\"}",
  name: "approval:approval-1",
  status: "done",
  approvalId: "approval-1",
  approvalStatus: "pending",
});
useSessions.getState().completeStreamEvent(first, "approval:approval-1", {
  kind: "event",
  eventType: "status",
  label: "Approval submitted",
  name: "approval:approval-1",
  status: "running",
  approvalId: "approval-1",
  approvalStatus: "decided",
});
useSessions.getState().completeStreamEvent(first, "approval:approval-1", {
  kind: "event",
  eventType: "status",
  label: "Approval delivered",
  name: "approval:approval-1",
  status: "running",
  approvalId: "approval-1",
  approvalStatus: "delivered",
});
useSessions.getState().completeStreamEvent(first, "approval:approval-1", {
  kind: "event",
  eventType: "status",
  label: "google_docs_edit approved",
  name: "approval:approval-1",
  status: "done",
  approvalId: "approval-1",
  approvalStatus: "approved",
});
const approvalParts = (
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.messages[1].streamParts ?? []
).filter((part) => part.kind === "event" && part.approvalId === "approval-1");
equal(approvalParts.length, 1, "approval lifecycle updates should replace the same transcript event");
assert(approvalParts[0]?.kind === "event", "resolved approval should remain an event");
equal(approvalParts[0].label, "google_docs_edit approved", "approval history should record the decision");
equal(approvalParts[0].detail, "{\"file_id\":\"doc\"}", "approval history should retain the request summary");
useSessions.getState().commitResponseMetrics(first, {
  startedAt: 1_000,
  endedAt: 4_200,
  durationMs: 3_200,
  model: "openrouter/test",
  provider: "OpenRouter",
  usage: { prompt_tokens: 1_000, completion_tokens: 400, total_tokens: 1_400 },
  costUsd: 0.004,
  costSource: "estimate",
  limits: [
    {
      provider: "Local Claude CLI",
      status: "rejected",
      kind: "five_hour",
      reset_at: 1_782_660_000,
    },
  ],
});
const assistantMetrics = useSessions
  .getState()
  .sessions.find((session) => session.id === first)?.messages[1].metrics;
assert(
  assistantMetrics,
  "response metrics should be stored on the latest assistant message",
);
equal(
  formatResponseMetrics(assistantMetrics),
  "OpenRouter · openrouter/test · 3.2s · 1.4k tokens · est. $0.004",
  "response metrics should format compactly",
);
equal(
  formatThreadMetrics(
    summarizeResponseMetrics(
      useSessions.getState().sessions.find((session) => session.id === first)
        ?.messages ?? [],
    ),
  ),
  "3.2s · 1.4k tokens · est. $0.004",
  "thread metrics should be derived from assistant responses",
);
const compactedBreakdown = formatThreadMetricsBreakdown([
  {
    role: "assistant",
    content: "old answer",
    metrics: {
      startedAt: 1_000,
      endedAt: 4_200,
      durationMs: 3_200,
      model: "openrouter/test",
      usage: {
        prompt_tokens: 1_000,
        completion_tokens: 400,
        total_tokens: 1_400,
      },
      costUsd: 0.004,
      costSource: "estimate",
    },
  },
  {
    role: "assistant",
    content: "### Context checkpoint\n\nCarry these decisions forward.",
    compaction: {
      kind: "checkpoint",
      createdAt: 5_000,
      sourceTokens: 1_400,
      summaryTokens: 120,
      baseline: {
        responseCount: 1,
        durationMs: 3_200,
        usage: {
          prompt_tokens: 1_000,
          completion_tokens: 400,
          total_tokens: 1_400,
        },
        costUsd: 0.004,
        costSource: "estimate",
      },
      summary: {
        model: "openrouter/test",
        provider: "OpenRouter",
        durationMs: 800,
        usage: {
          prompt_tokens: 700,
          completion_tokens: 100,
          total_tokens: 800,
        },
        costUsd: 0.001,
        costSource: "estimate",
      },
    },
  },
  { role: "user", content: "fresh prompt" },
  {
    role: "assistant",
    content: "fresh answer",
    metrics: {
      startedAt: 6_000,
      endedAt: 8_000,
      durationMs: 2_000,
      model: "openrouter/test",
      usage: { prompt_tokens: 300, completion_tokens: 100, total_tokens: 400 },
      costUsd: 0.002,
      costSource: "estimate",
    },
  },
]);
assert(
  !compactedBreakdown.label?.includes("since compact"),
  "thread usage label should show one lifetime total instead of two windows",
);
assert(
  compactedBreakdown.label?.includes("est. $0.007"),
  "thread usage label should include compaction-summary spend in lifetime cost",
);
assert(
  compactedBreakdown.title?.includes("At latest checkpoint"),
  "thread usage tooltip should show checkpoint baseline",
);
assert(
  compactedBreakdown.title?.includes("Compaction summary"),
  "thread usage tooltip should show summary generation cost",
);
assert(
  formatProviderLimits(
    latestProviderLimits(
      useSessions.getState().sessions.find((session) => session.id === first)
        ?.messages ?? [],
    ),
    1_782_659_100_000,
  )?.includes("limit hit"),
  "provider limits should format from latest response metrics",
);
const codexLimits = codexLimitsFromRateLimitPayload({
  rateLimits: {
    five_hour: { remaining: 2, limit: 10, resetAt: 1_782_660_000 },
  },
});
equal(
  codexLimits[0]?.provider,
  "Codex",
  "Codex rate-limit payload should normalize to provider limits",
);
assert(
  formatProviderLimits(codexLimits, 1_782_659_100_000)?.includes("2/10 left"),
  "Codex rate-limit payload should format quota",
);
const codexWindowLimits = codexLimitsFromRateLimitPayload({
  rateLimits: {
    primary: {
      usedPercent: 48,
      windowDurationMins: 300,
      resetsAt: 1_782_660_000,
    },
    secondary: {
      usedPercent: 60,
      windowDurationMins: 10_080,
      resetsAt: 1_782_900_000,
    },
  },
});
const codexWindowText =
  formatProviderLimits(codexWindowLimits, 1_782_659_100_000) ?? "";
assert(
  codexWindowText.includes("5h limit") &&
    codexWindowText.includes("weekly limit"),
  "Codex primary/secondary limits should use window labels",
);
equal(
  formatCompactProviderLimits(codexWindowLimits),
  "Codex · 5h 52% left · weekly 40% left",
  "compact Codex limits should show remaining account usage",
);
equal(
  formatCompactProviderLimits([
    {
      provider: "Local Claude CLI",
      status: "rejected",
      kind: "five_hour",
      reset_at: 1_782_660_000,
    },
  ]),
  "Claude · 5h limit hit",
  "compact Claude limits should show the latest CLI status",
);
equal(
  formatCompactProviderLimits([]),
  null,
  "compact account usage should hide when no quota is known",
);
equal(
  latestProviderLimits(
    [
      {
        role: "assistant",
        content: "",
        metrics: {
          startedAt: 1,
          model: "claude:sonnet",
          limits: [{ provider: "Local Claude CLI", kind: "five_hour", status: "rejected" }],
        },
      },
      {
        role: "assistant",
        content: "",
        metrics: {
          startedAt: 2,
          model: "codex:gpt-5.5",
          limits: [{ provider: "Codex", kind: "primary", used_percent: 48 }],
        },
      },
    ],
    "claude",
  )[0]?.provider,
  "Local Claude CLI",
  "provider-specific quota lookup should ignore newer limits from another runtime",
);
assert(
  !/weekly limit 60% used .*resets/.test(codexWindowText),
  "Codex weekly limit should hide reset time until exhausted",
);
const codexWeeklyHitText =
  formatProviderLimits(
    codexLimitsFromRateLimitPayload({
      rateLimits: {
        secondary: {
          usedPercent: 100,
          windowDurationMins: 10_080,
          resetsAt: 1_782_900_000,
        },
      },
    }),
    1_782_659_100_000,
  ) ?? "";
assert(
  /weekly limit 100% used .*resets/.test(codexWeeklyHitText),
  "Codex exhausted weekly limit should show reset time",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"durationMs":3200'),
  "response metrics should persist in session storage",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"planMode":true'),
  "plan mode should persist in session storage",
);
const usage = {
  prompt_tokens: 1_000,
  completion_tokens: 500,
  total_tokens: 1_500,
};
const pricedProviders = [
  {
    id: "prov-openrouter",
    name: "OpenRouter",
    kind: "openai_compatible" as const,
    base_url: "https://openrouter.ai/api/v1",
    enabled: true,
    has_key: true,
    models: ["openrouter/test"],
    pricing: {
      "openrouter/test": { prompt: "0.000001", completion: "0.000002" },
    },
  },
];
equal(
  estimateResponseCostUsd("openrouter/test", usage, pricedProviders),
  0.002,
  "OpenRouter pricing should estimate prompt plus completion cost",
);
equal(
  estimateResponseCostUsd("missing-price", usage, pricedProviders),
  undefined,
  "unknown pricing should not estimate cost",
);
const freeProviders = [{
  ...pricedProviders[0],
  models: ["openrouter/free"],
  pricing: { "openrouter/free": { prompt: "0", completion: "0" } },
}];
equal(
  estimateResponseCostUsd("openrouter/free", usage, freeProviders),
  0,
  "known free-model pricing should remain a complete zero-cost estimate",
);
const freeMetrics = responseMetricsForTurn({
  startedAt: 10,
  endedAt: 40,
  model: "openrouter/free",
  providers: freeProviders,
  usage,
});
equal(freeMetrics.costUsd, 0, "free-model response metrics should retain zero cost");
equal(freeMetrics.costSource, "estimate", "free-model response metrics should retain pricing provenance");
const freeSummary = summarizeResponseMetrics([{ role: "assistant", content: "free", metrics: freeMetrics }]);
equal(freeSummary.costUsd, 0, "free turns should contribute a zero-dollar total");
equal(freeSummary.costPartial, undefined, "known free turns should not mark spend as incomplete");
deepEqual(
  responseMetricsForTurn({
    startedAt: 10,
    endedAt: 40,
    model: "openrouter/test",
    providers: pricedProviders,
    usage,
  }),
  {
    startedAt: 10,
    endedAt: 40,
    durationMs: 30,
    model: "openrouter/test",
    provider: "OpenRouter",
    usage,
    costUsd: 0.002,
    costSource: "estimate",
  },
  "turn response metrics should derive duration, provider, and estimated cost",
);
deepEqual(
  responseMetricsForTurn({
    startedAt: 10,
    endedAt: 40,
    model: "codex:gpt-5",
    providers: pricedProviders,
    codexModel: "gpt-5",
    usage,
    costUsd: 0.5,
    limits: [],
  }),
  {
    startedAt: 10,
    endedAt: 40,
    durationMs: 30,
    model: "codex:gpt-5",
    provider: "Codex",
    usage,
    costUsd: 0.5,
    costSource: "provider",
  },
  "account-runtime turn metrics should use runtime provider and explicit cost only",
);
deepEqual(
  responseMetricsForTurn({
    startedAt: 10,
    endedAt: 40,
    model: "opencode:opencode/north-mini-code-free",
    providers: pricedProviders,
    usage,
  }),
  {
    startedAt: 10,
    endedAt: 40,
    durationMs: 30,
    model: "opencode:opencode/north-mini-code-free",
    provider: "Local OpenCode CLI",
    usage,
  },
  "OpenCode runtime metrics should retain native provenance without provider pricing",
);
equal(
  formatResponseMetrics({
    startedAt: 10,
    endedAt: 40,
    durationMs: 30,
    model: "pi:openai-codex/gpt-5.4-mini",
    provider: "Local Pi CLI",
  }),
  "Local Pi CLI · openai-codex/gpt-5.4-mini · 30ms",
  "account-runtime response footers should identify the runtime and model",
);
equal(
  formatResponseMetrics({
    startedAt: 10,
    endedAt: 40,
    durationMs: 30,
    model: "claude:opus",
    provider: "Local Claude CLI",
    usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
    costUsd: 0.02,
    costSource: "provider",
  }),
  "Local Claude CLI · opus · 30ms · 15 tokens · $0.02",
  "provider-reported cost should not be labeled as an estimate",
);
equal(
  formatThreadMetrics(
    summarizeResponseMetrics([
      {
        role: "assistant",
        content: "priced",
        metrics: {
          startedAt: 1,
          endedAt: 2,
          durationMs: 1_000,
          model: "openrouter/test",
          usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
          costUsd: 0.006,
          costSource: "estimate",
        },
      },
      {
        role: "assistant",
        content: "unpriced",
        metrics: {
          startedAt: 3,
          endedAt: 4,
          durationMs: 1_000,
          model: "opencode:free",
          usage: { prompt_tokens: 20, completion_tokens: 10, total_tokens: 30 },
        },
      },
    ]),
  ),
  "2.0s · 45 tokens · ~est. $0.006",
  "thread totals should mark incomplete spend when some turns have no cost",
);
equal(
  approvalWaitDuration({
    model: "test",
    startedAt: 0,
    status: "running",
    steps: [
      { name: "first", startedAt: 10, approval: { id: "a", status: "approved", requestedAt: 20, resolvedAt: 50 } },
      { name: "second", startedAt: 30, approval: { id: "b", status: "pending", requestedAt: 40 } },
    ],
  }, 70),
  50,
  "overlapping approval waits should be merged instead of double counted",
);
equal(
  approvalWaitDuration({
    model: "test",
    startedAt: 0,
    status: "running",
    steps: [
      { name: "first", startedAt: 10, approval: { id: "a", status: "approved", requestedAt: 20, resolvedAt: 50 } },
      { name: "second", startedAt: 30, approval: { id: "b", status: "pending", requestedAt: 40 } },
    ],
  }, 40),
  20,
  "live approval waits should be clamped to the display clock",
);
equal(
  responseMetricsForTurn({
    startedAt: 10,
    endedAt: 100,
    pausedMs: 50,
    model: "openrouter/test",
    providers: pricedProviders,
  }).durationMs,
  40,
  "turn duration should exclude approval wait time",
);
const usageSummary = summarizeMilimUsage(
  [
    {
      messages: [{ role: "user", content: "one" }],
      updatedAt: new Date(2026, 5, 1, 9).getTime(),
      settings: { folder: "C:\\active-project" },
    },
    {
      messages: [{ role: "user", content: "two" }],
      updatedAt: new Date(2026, 5, 1, 10).getTime(),
      settings: { folder: "" },
    },
    {
      messages: [{ role: "user", content: "three" }],
      updatedAt: new Date(2026, 4, 31, 10).getTime(),
      settings: { folder: "C:\\active-project" },
    },
    {
      messages: [],
      updatedAt: new Date(2026, 5, 2, 10).getTime(),
      settings: { folder: "C:\\active-project" },
    },
    {
      messages: [{ role: "user", content: "archived" }],
      updatedAt: new Date(2026, 5, 3, 10).getTime(),
      settings: { folder: "C:\\active-project" },
      archivedAt: 1,
    },
    {
      messages: [{ role: "user", content: "archived project" }],
      updatedAt: new Date(2026, 5, 4, 10).getTime(),
      settings: { folder: "C:\\archived-project" },
    },
  ],
  [
    { folder: "C:\\active-project" },
    { folder: "C:\\archived-project", archivedAt: 1 },
  ],
  new Date(2026, 5, 28).getTime(),
);
equal(
  usageSummary.threadCount,
  3,
  "Milim usage should count only non-empty visible threads",
);
equal(
  usageSummary.projectCount,
  1,
  "Milim usage should count only active projects",
);
equal(
  usageSummary.activeDayCount,
  2,
  "Milim usage should count active days in the displayed year",
);
equal(
  usageSummary.months.find((month) => month.key === "2026-06")?.days[0],
  2,
  "Milim usage should bucket same-day June activity",
);
equal(
  usageSummary.months.find((month) => month.key === "2026-05")?.days[30],
  1,
  "Milim usage should bucket May activity by day",
);
deepEqual(
  usageSummary.metrics,
  [
    { label: "Threads", value: "3" },
    { label: "Projects", value: "1" },
    { label: "Active days", value: "2" },
  ],
  "Milim usage metrics should format compactly",
);
const emptyUsageSummary = summarizeMilimUsage(
  [],
  [],
  new Date(2026, 5, 28).getTime(),
);
equal(
  emptyUsageSummary.hasUsage,
  false,
  "empty Milim usage should report no activity",
);
deepEqual(
  emptyUsageSummary.metrics,
  [
    { label: "Threads", value: "0" },
    { label: "Projects", value: "0" },
    { label: "Active days", value: "0" },
  ],
  "empty Milim usage should render zero metrics",
);

useSessions.getState().newChat({
  model: "model-b",
  instructions: "prompt b",
  activeAgentId: "agent-b",
});

const second = useSessions.getState().activeId;
assert(second !== first, "new chat should create a different active session");
useSessions.getState().setContextPanelOpen(first, true);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.contextPanelOpen,
  true,
  "context panel open state should persist per thread",
);
equal(
  useSessions.getState().sessions.find((session) => session.id === second)
    ?.contextPanelOpen,
  undefined,
  "context panel state should not bleed into another thread",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"contextPanelOpen":true'),
  "context panel state should persist in session storage",
);
useSessions.getState().setContextPanelOpen(first, false);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.contextPanelOpen,
  undefined,
  "closing the context panel should persist collapsed state",
);
useSessions.getState().setContextSectionCollapsed(first, "sources", true);
deepEqual(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.contextCollapsedSectionIds,
  ["sources"],
  "context section state should persist per thread",
);
equal(
  useSessions.getState().sessions.find((session) => session.id === second)
    ?.contextCollapsedSectionIds,
  undefined,
  "context section state should not bleed into another thread",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"contextCollapsedSectionIds":["sources"]'),
  "context section state should persist in session storage",
);
useSessions.getState().setContextSectionCollapsed(first, "sources", false);
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.contextCollapsedSectionIds,
  undefined,
  "expanding every context section should omit empty persisted state",
);
useSessions.getState().setInspectorOpen(first, true);
equal(
  inspectorFor(first).open,
  true,
  "inspector open state should persist for its workspace",
);
equal(
  inspectorFor(second).open,
  false,
  "folderless inspector state should remain thread-scoped",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"inspectorByKey"'),
  "inspector open state should persist in session storage",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes('"artifactPanelOpen"'),
  "new persistence writes should omit legacy artifact panel fields",
);
useSessions.getState().updateSettings(second, {
  folder: "C:\\workspace-a\\.worktrees\\second",
});
equal(
  inspectorFor(second).open,
  false,
  "isolated worktree paths should keep independent inspector state",
);
useSessions.getState().updateSettings(second, { folder: "c:/WORKSPACE-A/" });
deepEqual(
  inspectorFor(second),
  inspectorFor(first),
  "normalized matching directories should share inspector shell state",
);
useSessions.getState().setInspectorOpen(second, false);
equal(
  inspectorFor(first).open,
  false,
  "closing a shared inspector should collapse it in sibling threads",
);
useSessions.getState().setInspectorTab(first, "workers");
equal(
  inspectorFor(first).tab,
  "workers",
  "inspector tab should persist per workspace",
);
equal(
  inspectorFor(second).open,
  true,
  "selecting an inspector tab should open it for sibling threads",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"tab":"workers"'),
  "inspector tab should persist in session storage",
);
useSessions.getState().setBrowserSession(first, {
  profileId: "session-browser",
  activeTabId: "slides",
  tabs: [
    {
      id: "docs",
      url: "https://docs.google.com/document/d/doc-1/edit",
      input: "https://docs.google.com/document/d/doc-1/edit",
      history: ["https://docs.google.com/document/d/doc-1/edit"],
      historyIndex: 0,
      title: "Brief",
    },
    {
      id: "slides",
      url: "https://docs.google.com/presentation/d/slides-1/edit",
      input: "https://docs.google.com/presentation/d/slides-1/edit",
      history: ["https://docs.google.com/presentation/d/slides-1/edit"],
      historyIndex: 0,
      title: "Deck",
    },
  ],
});
equal(
  useSessions.getState().sessions.find((session) => session.id === first)
    ?.browserSession?.tabs.length,
  2,
  "browser tabs should persist per thread",
);
equal(
  useSessions.getState().sessions.find((session) => session.id === second)
    ?.browserSession,
  undefined,
  "browser tabs should not bleed into another thread",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"activeTabId":"slides"'),
  "browser tabs should persist in session storage",
);
useSessions.getState().setInspectorOpen(first, false);
equal(
  inspectorFor(first).open,
  false,
  "collapsing the inspector should persist closed state",
);
equal(
  inspectorFor(first).tab,
  "workers",
  "collapsing the inspector should preserve selected tab",
);
useSessions.getState().setInspectorOpen(first, true);
equal(
  inspectorFor(first).open,
  true,
  "reopening the inspector should restore open state",
);
equal(
  inspectorFor(first).tab,
  "workers",
  "reopening the inspector should keep the selected tab",
);
useSessions.getState().setInspectorTab(first, "preview");
useSessions.getState().setContextPanelOpen(first, false);
useSessions.getState().upsertWorkerRun({
  run: {
    id: "context-worker-run",
    parent_thread_id: first,
    policy: "ask",
    runtime: "managed",
    status: "proposed",
    tasks: [],
    created_at: "2026-07-13T00:00:00Z",
    updated_at: "2026-07-13T00:00:00Z",
  },
  workers: [],
});
equal(
  inspectorFor(first).open,
  true,
  "proposed Worker Runs should reveal the inspector",
);
equal(
  inspectorFor(second).tab,
  "workers",
  "proposed Worker Runs should select the shared Workers inspector",
);
useSessions.getState().updateSettings(second, { folder: "" });
useSessions.getState().setContextPanelOpen(first, false);
useSessions.getState().setSessionUnread(first, true);
deepEqual(
  useSessions.getState().unreadSessionIds,
  [first],
  "background thread updates should be marked unread",
);
useSessions.getState().switchTo(first);
deepEqual(
  useSessions.getState().unreadSessionIds,
  [],
  "opening an unread thread should mark it read",
);
useSessions.getState().switchTo(second);
const projectA = projectSectionId("C:\\workspace-a");
useSessions.getState().toggleSessionPinned(first);
deepEqual(
  useSessions.getState().sidebar.pinnedSessionIds,
  [first],
  "sidebar should track pinned chats",
);
useSessions.getState().toggleSidebarSectionCollapsed(projectA);
deepEqual(
  useSessions.getState().sidebar.collapsedSectionIds,
  [projectA],
  "sidebar should persist collapsed project sections",
);
useSessions.getState().toggleSidebarSectionPinned(projectA);
deepEqual(
  useSessions.getState().sidebar.pinnedSectionIds,
  [projectA],
  "sidebar should persist pinned project sections",
);
equal(
  useSessions.getState().sidebar.sectionOrder[0],
  projectA,
  "pinning a project section should promote it in the sidebar order",
);
useSessions.getState().moveSessionInSidebar(first, second, projectA);
deepEqual(
  useSessions.getState().sidebar.sessionOrder.slice(0, 2),
  [first, second],
  "sidebar should persist manual chat ordering",
);
useSessions.getState().moveSessionInSidebar(first, second, projectA, "after");
deepEqual(
  useSessions.getState().sidebar.sessionOrder.slice(0, 2),
  [second, first],
  "sidebar should persist directional chat ordering",
);
useSessions.getState().addProjectFolder("C:\\workspace-c");
const projectC = projectSectionId("C:\\workspace-c");
assert(
  useSessions.getState().sidebar.projectFolders.includes("C:\\workspace-c"),
  "sidebar should persist explicitly added project folders",
);
useSessions.getState().updateProject(projectC, {
  name: "  Workspace C  ",
  icon: "code",
  color: "#A3C",
});
let customizedProject = useSessions.getState().projects.find((project) => project.id === projectC);
equal(customizedProject?.name, "Workspace C", "project customization should trim names");
equal(customizedProject?.icon, "code", "project customization should store whitelisted icons");
equal(customizedProject?.color, "#aa33cc", "project customization should normalize colors");
useSessions.getState().addProjectFolder("C:\\workspace-c");
customizedProject = useSessions.getState().projects.find((project) => project.id === projectC);
equal(customizedProject?.icon, "code", "project upserts should preserve custom icons");
equal(customizedProject?.color, "#aa33cc", "project upserts should preserve custom colors");
useSessions.getState().archiveProject(projectC);
useSessions.getState().restoreProject(projectC);
customizedProject = useSessions.getState().projects.find((project) => project.id === projectC);
equal(customizedProject?.name, "Workspace C", "archive restore should preserve project customization");
useSessions.getState().updateProject(projectC, {
  name: " ",
  icon: "invalid" as never,
  color: "nope",
});
customizedProject = useSessions.getState().projects.find((project) => project.id === projectC);
equal(customizedProject?.name, "workspace-c", "empty project names should reset to the folder basename");
equal(customizedProject?.icon, undefined, "invalid project icons should reset to default");
equal(customizedProject?.color, undefined, "invalid project colors should reset to default");
deepEqual(
  useSessions
    .getState()
    .sidebar.sectionOrder.filter(
      (id: string) =>
        id === SIDEBAR_PINNED_SECTION_ID || id === SIDEBAR_CHATS_SECTION_ID,
    ),
  [],
  "fixed sidebar sections should not persist in manual project section order",
);
const sectionOrderBeforeFixedDrop = useSessions
  .getState()
  .sidebar.sectionOrder.slice();
useSessions
  .getState()
  .moveSidebarSection(projectA, SIDEBAR_PINNED_SECTION_ID, "before");
deepEqual(
  useSessions.getState().sidebar.sectionOrder,
  sectionOrderBeforeFixedDrop,
  "pinned should stay fixed above project section ordering",
);
useSessions
  .getState()
  .moveSidebarSection(projectA, SIDEBAR_CHATS_SECTION_ID, "after");
deepEqual(
  useSessions.getState().sidebar.sectionOrder,
  sectionOrderBeforeFixedDrop,
  "chats should stay fixed below project section ordering",
);
useSessions.getState().moveSidebarSection(projectA, projectC, "before");
equal(
  useSessions.getState().sidebar.sectionOrder.indexOf(projectA),
  useSessions.getState().sidebar.sectionOrder.indexOf(projectC) - 1,
  "sidebar should persist directional project section ordering before a project target",
);
useSessions.getState().moveSidebarSection(projectA, projectC, "after");
equal(
  useSessions.getState().sidebar.sectionOrder.indexOf(projectA),
  useSessions.getState().sidebar.sectionOrder.indexOf(projectC) + 1,
  "sidebar should persist directional project section ordering after a project target",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"sidebar"'),
  "sidebar organization should persist in session storage",
);

const forkSource = useSessions.getState().activeId;
useSessions.getState().setMessages(forkSource, [
  { role: "user", content: "branch root" },
  { role: "assistant", content: "branch answer" },
  { role: "user", content: "not copied" },
]);
const forked = useSessions.getState().forkSession(forkSource, 1);
assert(forked, "forking a session should return the new session id");
const forkedSession = useSessions
  .getState()
  .sessions.find((session) => session.id === forked);
assert(forkedSession, "forked session should be stored");
equal(
  forkedSession.parentId,
  forkSource,
  "forked session should point at its source",
);
deepEqual(
  forkedSession.messages.map((message) => message.content),
  ["branch root", "branch answer"],
  "forking at a message should copy the visible prefix only",
);
assert(
  !forkedSession.accountRuntime,
  "forked sessions should not reuse native runtime thread ids",
);

const imported = useSessions.getState().importSession({
  id: "foreign-id",
  title: "Foreign chat",
  messages: [
    { role: "user", content: "imported user" },
    { role: "assistant", content: "imported assistant" },
    { role: "assistant", content: 12 as unknown as string },
  ],
  settings: { model: "import-model", folder: "C:\\imported" },
});
assert(imported, "importing a session should return the new session id");
assert(
  imported !== "foreign-id",
  "imported sessions should get a fresh local id",
);
const importedSession = useSessions
  .getState()
  .sessions.find((session) => session.id === imported);
equal(
  importedSession?.title,
  "Foreign chat",
  "import should preserve the exported title",
);
deepEqual(
  importedSession?.messages.map((message) => message.content),
  ["imported user", "imported assistant"],
  "import should ignore invalid message rows",
);
equal(
  importedSession?.settings?.folder,
  "C:\\imported",
  "import should preserve thread settings",
);

useSessions.getState().upsertChildThread(first, {
  id: "child-thread-1",
  parent_id: first,
  root_id: first,
  title: "Review worker",
  status: "running",
  model: "model-a",
  agent_id: null,
  prompt: "Review this thread",
  summary: null,
  error: null,
  created_at: "2026-06-22 10:00:00",
  updated_at: "2026-06-22 10:00:01",
  finished_at: null,
});
let child = useSessions
  .getState()
  .sessions.find((session) => session.id === "child-thread-1");
assert(!child, "legacy workers should not create sidebar sessions");
let childRun = useSessions
  .getState()
  .workerRuns.find((record) => record.run.id === "legacy:child-thread-1");
assert(childRun, "legacy child records should hydrate as singleton Worker Runs");
equal(
  childRun.workers[0]?.status,
  "running",
  "legacy Worker Run should persist worker status",
);
equal(
  childRun.workers[0]?.model,
  "model-a",
  "legacy Worker Run should use its persisted worker model",
);
equal(
  childRun.workers[0]?.prompt,
  "Review this thread",
  "legacy Worker Run should expose its delegated prompt",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes("legacy:child-thread-1"),
  "Worker Runs should reload from threads.db instead of session persistence",
);

useSessions.getState().updateChildThread({
  id: "child-thread-1",
  parent_id: first,
  root_id: first,
  title: "Review worker",
  status: "done",
  model: "model-a",
  agent_id: null,
  prompt: "Review this thread",
  summary: "Looks good.",
  error: null,
  created_at: "2026-06-22 10:00:00",
  updated_at: "2026-06-22 10:00:02",
  finished_at: "2026-06-22 10:00:02",
});
childRun = useSessions
  .getState()
  .workerRuns.find((record) => record.run.id === "legacy:child-thread-1");
equal(childRun?.workers[0]?.status, "done", "legacy worker status should update");
equal(
  childRun?.workers[0]?.messages?.[1]?.content,
  "Looks good.",
  "legacy worker summary should update its inspector transcript",
);

useSessions.getState().updateChildThread(
  {
    id: "child-thread-1",
    parent_id: first,
    root_id: first,
    title: "Review worker",
    status: "running",
    model: "model-a",
    agent_id: null,
    prompt: "Review this thread",
    summary: null,
    error: null,
    created_at: "2026-06-22 10:00:00",
    updated_at: "2026-06-22 10:00:03",
    finished_at: null,
  },
  [
    {
      id: "event-1",
      thread_id: "child-thread-1",
      seq: 1,
      kind: "reasoning",
      payload: { text: "Checking " },
      created_at: "2026-06-22 10:00:01",
    },
    {
      id: "event-2",
      thread_id: "child-thread-1",
      seq: 2,
      kind: "tool_call",
      payload: { name: "read_file", arguments: '{"path":"README.md"}' },
      created_at: "2026-06-22 10:00:02",
    },
    {
      id: "event-3",
      thread_id: "child-thread-1",
      seq: 3,
      kind: "tool_result",
      payload: { name: "read_file", result: { ok: true } },
      created_at: "2026-06-22 10:00:03",
    },
    {
      id: "event-4",
      thread_id: "child-thread-1",
      seq: 4,
      kind: "token",
      payload: { text: "Live report" },
      created_at: "2026-06-22 10:00:04",
    },
  ],
);
childRun = useSessions
  .getState()
  .workerRuns.find((record) => record.run.id === "legacy:child-thread-1");
equal(
  childRun?.workers[0]?.messages?.[1]?.content,
  "Live report",
  "legacy worker token events should update inspector content",
);
deepEqual(
  childRun?.workers[0]?.messages?.[1]?.streamParts?.map((part) => part.kind),
  ["thinking", "event", "text"],
  "legacy worker events should hydrate inspector stream parts",
);
useSessions.getState().setWorkerRunEvent("legacy:child-thread-1", {
  id: "event-5-original",
  thread_id: "child-thread-1",
  seq: 5,
  kind: "token",
  payload: { text: "stale" },
  created_at: "2026-06-22 10:00:05",
});
useSessions.getState().setWorkerRunEvent("legacy:child-thread-1", {
  id: "event-5-replayed",
  thread_id: "child-thread-1",
  seq: 5,
  kind: "token",
  payload: { text: "fresh" },
  created_at: "2026-06-22 10:00:05",
});
childRun = useSessions
  .getState()
  .workerRuns.find((record) => record.run.id === "legacy:child-thread-1");
equal(
  childRun?.workers[0]?.events?.filter((event) => event.seq === 5).length,
  1,
  "worker event replay should deduplicate by sequence",
);

deepEqual(
  useSessions.getState().getSettings(first),
  {
    model: "model-a",
    instructions: "prompt a",
    activeAgentId: "agent-a",
    folder: "C:\\workspace-a",
    sandbox: true,
    computerUse: true,
    memory: false,
    privacy: "redact",
    toolApproval: "open",
    delegationPolicy: "ask",
    workerModel: "",
    planMode: true,
    goal: DEFAULT_GOAL_SETTINGS,
  },
  "first session settings should persist",
);
deepEqual(
  useSessions.getState().getSettings(second),
  {
    model: "model-b",
    instructions: "prompt b",
    activeAgentId: "agent-b",
    folder: "",
    sandbox: false,
    computerUse: false,
    memory: true,
    privacy: "off",
    toolApproval: "review",
    delegationPolicy: "ask",
    workerModel: "",
    planMode: false,
    goal: DEFAULT_GOAL_SETTINGS,
  },
  "second session settings should persist",
);

useSessions.getState().updateSettings(second, {
  model: "model-b2",
  folder: "C:\\workspace-b",
  sandbox: true,
  computerUse: true,
  memory: false,
  privacy: "block",
});
useSessions.getState().updateSettings(second, { toolApproval: "open" });
useSessions.getState().switchTo(first);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId).model,
  "model-a",
  "switching to the first session should restore model-a",
);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId)
    .computerUse,
  true,
  "switching to the first session should restore computer-use on",
);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId).privacy,
  "redact",
  "switching to the first session should restore privacy mode",
);

useSessions.getState().switchTo(second);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId).model,
  "model-b2",
  "switching back should restore model-b2",
);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId)
    .computerUse,
  true,
  "switching back should restore computer-use on for the second session",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"computerUse":true'),
  "computer-use state should be persisted in session storage",
);

useSessions
  .getState()
  .setMessages(second, [{ role: "user", content: "second thread" }]);
useSessions.getState().newChat();
const third = useSessions.getState().activeId;
assert(
  third !== second,
  "new chat without explicit settings should create a new session when active thread has messages",
);
deepEqual(
  useSessions.getState().getSettings(third),
  {
    model: "model-b2",
    instructions: "prompt b",
    activeAgentId: "agent-b",
    folder: "C:\\workspace-b",
    sandbox: true,
    computerUse: true,
    memory: false,
    privacy: "block",
    toolApproval: "open",
    delegationPolicy: "ask",
    workerModel: "",
    planMode: false,
    goal: DEFAULT_GOAL_SETTINGS,
  },
  "new chat should inherit active thread settings by default",
);

useSessions.getState().updateSettings(third, {
  computerUse: false,
  sandbox: false,
  memory: true,
  privacy: "off",
  reasoningEffortByModel: {
    "model-b2": "nope" as never,
    other: "low",
    "auto-model": "auto",
  },
});
assert(
  !("reasoningEffortByModel" in useSessions.getState().getSettings(third)),
  "thread settings should ignore legacy reasoning effort maps",
);
useSessions.getState().updateSettings(third, {
  reasoningEffortOverrides: {
    "model-b2": "low",
    "auto-model": "auto",
    "": "high" as never,
    broken: "nope" as never,
  },
});
deepEqual(
  useSessions.getState().getSettings(third).reasoningEffortOverrides,
  { "model-b2": "low", "auto-model": "auto" },
  "thread effort overrides should keep an explicit auto and drop invalid entries",
);
useSessions.getState().updateSettings(third, { reasoningEffortOverrides: {} });
assert(
  !("reasoningEffortOverrides" in useSessions.getState().getSettings(third)),
  "a thread with no effort overrides should fall back to the app-wide defaults",
);
useSessions.getState().switchTo(second);
deepEqual(
  useSessions.getState().getSettings(second),
  {
    model: "model-b2",
    instructions: "prompt b",
    activeAgentId: "agent-b",
    folder: "C:\\workspace-b",
    sandbox: true,
    computerUse: true,
    memory: false,
    privacy: "block",
    toolApproval: "open",
    delegationPolicy: "ask",
    workerModel: "",
    planMode: false,
    goal: DEFAULT_GOAL_SETTINGS,
  },
  "changing computer-use in one thread should not change another thread",
);
useSessions.getState().switchTo(third);

useSessions.getState().updateSettings(third, { model: "mock-echo" });
equal(
  useSessions.getState().getSettings(third).model,
  "",
  "legacy mock-echo settings should be cleared",
);
assert(
  !localStorage.getItem("milim.sessions")?.includes('"model":"mock-echo"'),
  "mock-echo should not persist in session storage",
);
useSessions.getState().updateSettings(third, { model: "model-b2" });

const artifactSession = useSessions.getState().activeId;
const artifactContent = [
  "Generated file:",
  "",
  "```ts file=src/persisted-artifact.ts",
  "export const persistedArtifact = true;",
  "```",
].join("\n");
useSessions.getState().setMessages(artifactSession, [
  { role: "user", content: "make a file" },
  { role: "assistant", content: artifactContent },
]);
const artifact = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[1].artifacts?.[0];
assert(artifact, "assistant artifact should be extracted");
const artifactSavedAt = 1_725_000_000_000;
const artifactSourceSessionId = "app-session-test";
useSessions.getState().markArtifactSaved(artifactSession, 1, artifact.id, {
  path: "C:\\workspace\\src\\persisted-artifact.ts",
  bytes: 40,
  overwritten: false,
  savedAt: artifactSavedAt,
  sourceSessionId: artifactSourceSessionId,
  sourceMessageIndex: 1,
  sourceRevisionNumber: 1,
});
const saved = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[1]
  .artifacts?.[0]?.saved;
assert(
  saved,
  "saved artifact metadata should be stored on the message artifact",
);
equal(
  saved.path,
  "C:\\workspace\\src\\persisted-artifact.ts",
  "saved artifact path should persist in session state",
);
equal(
  saved.savedAt,
  artifactSavedAt,
  "saved artifact timestamp should persist in session state",
);
equal(
  saved.sourceSessionId,
  artifactSourceSessionId,
  "saved artifact source session should persist in session state",
);
equal(
  saved.sourceMessageIndex,
  1,
  "saved artifact source message index should persist in session state",
);
equal(
  saved.sourceRevisionNumber,
  1,
  "saved artifact source revision should persist in session state",
);

useSessions
  .getState()
  .setMessages(artifactSession, [
    { role: "user", content: "make a file" },
    useSessions.getState().sessions.find((s) => s.id === artifactSession)!
      .messages[1],
  ]);
const savedAfterNormalize = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[1]
  .artifacts?.[0]?.saved;
assert(
  savedAfterNormalize,
  "saved artifact metadata should survive artifact normalization",
);
equal(
  savedAfterNormalize.path,
  "C:\\workspace\\src\\persisted-artifact.ts",
  "normalization should keep saved artifact path",
);
equal(
  savedAfterNormalize.savedAt,
  artifactSavedAt,
  "normalization should keep saved artifact timestamp",
);
equal(
  savedAfterNormalize.sourceSessionId,
  artifactSourceSessionId,
  "normalization should keep saved artifact source session",
);
equal(
  savedAfterNormalize.sourceMessageIndex,
  1,
  "normalization should keep saved artifact source message index",
);
equal(
  savedAfterNormalize.sourceRevisionNumber,
  1,
  "normalization should keep saved artifact source revision",
);

const revisionArtifactContent = [
  "Revised file:",
  "",
  "```ts file=src/persisted-artifact.ts",
  "export const persistedArtifact = 'revision two';",
  "```",
].join("\n");
useSessions
  .getState()
  .setMessages(artifactSession, [
    { role: "user", content: "make a file" },
    useSessions.getState().sessions.find((s) => s.id === artifactSession)!
      .messages[1],
    { role: "assistant", content: revisionArtifactContent },
  ]);
const revisionArtifact = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[2].artifacts?.[0];
assert(revisionArtifact, "assistant revision artifact should be extracted");
const revisionSavedAt = artifactSavedAt + 1;
useSessions
  .getState()
  .markArtifactSaved(artifactSession, 2, revisionArtifact.id, {
    path: "C:\\workspace\\src\\persisted-artifact-v2.ts",
    bytes: 52,
    overwritten: true,
    savedAt: revisionSavedAt,
    sourceSessionId: artifactSourceSessionId,
    sourceMessageIndex: 2,
    sourceRevisionNumber: 2,
  });
const firstSavedAfterRevision = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[1]
  .artifacts?.[0]?.saved;
const revisionSaved = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[2]
  .artifacts?.[0]?.saved;
assert(
  firstSavedAfterRevision,
  "first artifact revision should keep its saved metadata",
);
assert(revisionSaved, "selected artifact revision should store saved metadata");
equal(
  firstSavedAfterRevision.path,
  "C:\\workspace\\src\\persisted-artifact.ts",
  "saving a later revision should not overwrite the first revision metadata",
);
equal(
  revisionSaved.path,
  "C:\\workspace\\src\\persisted-artifact-v2.ts",
  "selected revision should store its own saved path",
);
equal(
  revisionSaved.savedAt,
  revisionSavedAt,
  "selected revision should store its own saved timestamp",
);
equal(
  revisionSaved.sourceMessageIndex,
  2,
  "selected revision should store its source message index",
);
equal(
  revisionSaved.sourceRevisionNumber,
  2,
  "selected revision should store its source revision",
);

useSessions
  .getState()
  .setMessages(artifactSession, [
    { role: "user", content: "make a file" },
    useSessions.getState().sessions.find((s) => s.id === artifactSession)!
      .messages[1],
    useSessions.getState().sessions.find((s) => s.id === artifactSession)!
      .messages[2],
  ]);
const revisionSavedAfterNormalize = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.messages[2]
  .artifacts?.[0]?.saved;
assert(
  revisionSavedAfterNormalize,
  "selected revision saved metadata should survive artifact normalization",
);
equal(
  revisionSavedAfterNormalize.path,
  "C:\\workspace\\src\\persisted-artifact-v2.ts",
  "normalization should keep selected revision saved path",
);
equal(
  revisionSavedAfterNormalize.savedAt,
  revisionSavedAt,
  "normalization should keep selected revision saved timestamp",
);
equal(
  revisionSavedAfterNormalize.sourceMessageIndex,
  2,
  "normalization should keep selected revision source message index",
);
equal(
  revisionSavedAfterNormalize.sourceRevisionNumber,
  2,
  "normalization should keep selected revision source revision",
);

equal(
  normalizeVirtualFilePath(".\\src\\virtual.ts"),
  "src/virtual.ts",
  "virtual project paths should normalize to relative slash paths",
);
equal(
  normalizeVirtualFilePath("../secret.ts"),
  "",
  "virtual project paths should reject parent traversal",
);
useSessions
  .getState()
  .upsertVirtualFiles(
    artifactSession,
    [{ path: ".\\src\\virtual.ts", content: "export const virtual = 1;" }],
    {
      sourceMessageIndex: 1,
      sourceRevisionNumber: 1,
    },
  );
useSessions
  .getState()
  .upsertVirtualFiles(
    artifactSession,
    [{ path: "src/virtual.ts", content: "export const virtual = 2;" }],
    {
      sourceMessageIndex: 2,
      sourceRevisionNumber: 2,
    },
  );
const virtualFile = useSessions
  .getState()
  .sessions.find((s) => s.id === artifactSession)?.virtualFiles?.[
  "src/virtual.ts"
];
assert(virtualFile, "virtual project file should be persisted on the session");
equal(
  virtualFile.version,
  2,
  "virtual project file version should increment when content changes",
);
equal(
  virtualFile.sourceMessageIndex,
  2,
  "virtual project file should track source message",
);
equal(
  sessionVirtualProjectFiles({
    virtualFiles: { "src/virtual.ts": virtualFile },
  })[0].path,
  "src/virtual.ts",
  "virtual project files should convert to preview files",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"virtualFiles"'),
  "virtual project files should persist in session storage",
);

useSessions.getState().newChat({ model: "model-manual-title" });
const manualTitleSession = useSessions.getState().activeId;
useSessions
  .getState()
  .setMessages(
    manualTitleSession,
    [{ role: "user", content: "do not derive this title" }],
    { autoTitle: false },
  );
equal(
  useSessions.getState().sessions.find((s) => s.id === manualTitleSession)
    ?.title,
  "New chat",
  "auto-title opt-out should keep the default title",
);
useSessions
  .getState()
  .setMessages(
    manualTitleSession,
    [{ role: "user", content: "derive this title now" }],
    { autoTitle: true },
  );
equal(
  useSessions.getState().sessions.find((s) => s.id === manualTitleSession)
    ?.title,
  "derive this title now",
  "auto-title opt-in should derive the title from the first user message",
);

useSessions.getState().newChat({ model: "queue-remove-model" });
const queueRemoveSession = useSessions.getState().activeId;
useSessions
  .getState()
  .enqueueQueuedMessage(queueRemoveSession, { content: "remove with session" });
assert(
  useSessions.getState().queuedMessagesBySession[queueRemoveSession]?.length ===
    1,
  "queue cleanup test should create a queued item",
);
useSessions.getState().remove(queueRemoveSession);
equal(
  useSessions.getState().queuedMessagesBySession[queueRemoveSession],
  undefined,
  "removing a session should remove its queued messages",
);

useSessions.getState().updateSettings(useSessions.getState().activeId, {
  toolApproval: "open",
});
useSessions.getState().newChat({
  ...useSessions.getState().getSettings(useSessions.getState().activeId),
  folder: "C:\\composer-project",
});
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId).folder,
  "C:\\composer-project",
  "project selector should start a chat in the selected project",
);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId)
    .toolApproval,
  "review",
  "starting a chat in another project should reset Open approval to Review",
);
assert(
  useSessions
    .getState()
    .projects.some((project) => project.folder === "C:\\composer-project"),
  "project selector should register the selected project",
);
const projectCountBeforeNoProject = useSessions.getState().projects.length;
useSessions.getState().updateSettings(useSessions.getState().activeId, {
  toolApproval: "open",
});
useSessions
  .getState()
  .setMessages(useSessions.getState().activeId, [
    { role: "user", content: "project chat" },
  ]);
useSessions.getState().newChat({
  ...useSessions.getState().getSettings(useSessions.getState().activeId),
  folder: "",
});
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId).folder,
  "",
  "no-project selector should start a chat without a folder",
);
equal(
  useSessions.getState().getSettings(useSessions.getState().activeId)
    .toolApproval,
  "review",
  "moving to a scratch project should reset Open approval to Review",
);
equal(
  useSessions.getState().projects.length,
  projectCountBeforeNoProject,
  "no-project selector should not create a blank project",
);

equal(
  useSessions.getState().archiveRetentionDays,
  DEFAULT_ARCHIVE_RETENTION_DAYS,
  "archive retention should default to 30 days",
);
useSessions.getState().setArchiveRetentionDays(7);
equal(
  useSessions.getState().archiveRetentionDays,
  7,
  "archive retention should support 7 days",
);
useSessions
  .getState()
  .newChat({ model: "archive-model", folder: "C:\\workspace-archive" });
const archiveSession = useSessions.getState().activeId;
const archiveProject = projectSectionId("C:\\workspace-archive");
useSessions.getState().updateSettings(archiveSession, {
  toolApproval: "open",
});
useSessions
  .getState()
  .setSessionFolder(archiveSession, "C:\\workspace-archive-moved");
equal(
  useSessions.getState().getSettings(archiveSession).toolApproval,
  "review",
  "setting a different session folder should reset Open approval to Review",
);
useSessions.getState().updateSettings(archiveSession, {
  toolApproval: "open",
});
useSessions
  .getState()
  .moveSessionInSidebar(archiveSession, null, archiveProject, "inside");
equal(
  useSessions.getState().getSettings(archiveSession).toolApproval,
  "review",
  "moving a session between project sections should reset Open approval to Review",
);
useSessions
  .getState()
  .setMessages(archiveSession, [{ role: "user", content: "archive me" }]);
const settleBranch = useSessions.getState().forkSession(archiveSession);
assert(settleBranch, "settled-thread test should create a branch");
useSessions.getState().switchTo(archiveSession);
const settledUpdatedAt = useSessions
  .getState()
  .sessions.find((session) => session.id === archiveSession)?.updatedAt;
const settledOrder = [...useSessions.getState().sidebar.sessionOrder];
useSessions.getState().settleSession(archiveSession);
const settledAt = useSessions
  .getState()
  .sessions.find((session) => session.id === archiveSession)?.settledAt;
assert(settledAt, "settling a chat should record a timestamp");
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === settleBranch)?.settledAt,
  undefined,
  "settling a chat should not cascade to branches",
);
equal(
  useSessions.getState().activeId,
  archiveSession,
  "settling the active chat should keep it active",
);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.updatedAt,
  settledUpdatedAt,
  "settling should preserve last-activity recency",
);
deepEqual(
  useSessions.getState().sidebar.sessionOrder,
  settledOrder,
  "settling should preserve sidebar order",
);
assert(
  localStorage.getItem("milim.sessions")?.includes('"settledAt":'),
  "settled state should persist with the session",
);
useSessions.getState().purgeExpiredArchives(settledAt + 8 * DAY_MS);
assert(
  useSessions
    .getState()
    .sessions.some((session) => session.id === archiveSession),
  "settled chats should not expire under archive retention",
);
useSessions.getState().setSessionGenerating(archiveSession, true);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  undefined,
  "generation should auto-unsettle a chat",
);
useSessions.getState().setSessionGenerating(archiveSession, false);
useSessions.getState().settleSession(archiveSession);
useSessions.getState().switchTo(settleBranch);
useSessions.getState().setSessionUnread(archiveSession, true);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  undefined,
  "unread activity should auto-unsettle a chat",
);
useSessions.getState().setSessionUnread(archiveSession, false);
useSessions.getState().settleSession(archiveSession);
useSessions.getState().upsertWorkerRun({
  run: {
    id: "settled-worker-run",
    parent_thread_id: archiveSession,
    policy: "ask",
    runtime: "managed",
    status: "done",
    tasks: [],
    created_at: "2026-07-29T00:00:00Z",
    updated_at: "2026-07-29T00:01:00Z",
  },
  workers: [],
});
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  undefined,
  "Worker Run updates should auto-unsettle their parent chat",
);
useSessions.getState().switchTo(archiveSession);
useSessions.getState().settleSession(archiveSession);
assert(
  useSessions
    .getState()
    .projects.some((project) => project.id === archiveProject),
  "setting a folder should create a project record",
);
useSessions.getState().archiveSession(archiveSession);
assert(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.archivedAt,
  "archiving a chat should mark it archived",
);
assert(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  "archiving should preserve the independent settled marker",
);
assert(
  useSessions.getState().activeId !== archiveSession,
  "archiving the active chat should switch to a visible chat",
);
useSessions.getState().restoreSession(archiveSession);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.archivedAt,
  undefined,
  "restoring a chat should clear its archive marker",
);
assert(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  "restoring should preserve the independent settled marker",
);
useSessions.getState().unsettleSession(archiveSession);
equal(
  useSessions
    .getState()
    .sessions.find((session) => session.id === archiveSession)?.settledAt,
  undefined,
  "unsettling should clear the marker",
);
useSessions.getState().archiveProject(archiveProject);
assert(
  useSessions
    .getState()
    .projects.find((project) => project.id === archiveProject)?.archivedAt,
  "archiving a project should mark the project archived",
);
assert(
  useSessions.getState().activeId !== archiveSession,
  "archiving a project should move the active chat out of that project",
);
useSessions.getState().restoreProject(archiveProject);
equal(
  useSessions
    .getState()
    .projects.find((project) => project.id === archiveProject)?.archivedAt,
  undefined,
  "restoring a project should clear its archive marker",
);
useSessions.getState().archiveProject(archiveProject);
const projectArchivedAt = useSessions
  .getState()
  .projects.find((project) => project.id === archiveProject)?.archivedAt;
assert(projectArchivedAt, "project archive time should be recorded");
useSessions.getState().purgeExpiredArchives(projectArchivedAt + 6 * DAY_MS);
assert(
  useSessions
    .getState()
    .projects.some((project) => project.id === archiveProject),
  "project should remain before the retention window expires",
);
useSessions.getState().purgeExpiredArchives(projectArchivedAt + 8 * DAY_MS);
assert(
  !useSessions
    .getState()
    .projects.some((project) => project.id === archiveProject),
  "expired archived project should be deleted",
);
assert(
  !useSessions
    .getState()
    .sessions.some((session) => session.id === archiveSession),
  "deleting an expired project should delete its chats",
);

export {};
