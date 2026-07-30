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

const { useSessions } = await import("../src/sessions/store.js");
const { drainQueuedMessages, hasQueuedMessages, queuedModelForSession } = await import("../src/lib/turnQueue.js");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const sessionId = useSessions.getState().activeId;
equal(queuedModelForSession(sessionId, "fallback-model"), "fallback-model", "fallback model should be used when the thread has none");

useSessions.getState().updateSettings(sessionId, {
  model: "thread-model",
  activeAgentId: "agent-1",
});
const agents = [{ id: "agent-1" }];
equal(queuedModelForSession(sessionId, undefined, agents), "thread-model", "active agents should use the thread model");

useSessions.getState().updateSettings(sessionId, { activeAgentId: null });
const firstQueued = useSessions.getState().enqueueQueuedMessage(sessionId, { content: "first" });
const secondQueued = useSessions.getState().enqueueQueuedMessage(sessionId, { content: "second" });
useSessions.getState().moveQueuedMessage(sessionId, secondQueued.id, firstQueued.id, "before");
assert(hasQueuedMessages(sessionId), "queued messages should be visible before drain");

const ran: string[] = [];
const successfulDrain = await drainQueuedMessages({
  sessionId,
  queueDrainRef: { current: new Map() },
  generationControllersRef: { current: new Map<string, AbortController>() },
  agents,
  setChatNotice: () => {},
  sessionMessages: () => [],
  runTurn: async (convo, selectedModel) => {
    equal(selectedModel, "thread-model", "drain should use the thread model");
    ran.push(convo[convo.length - 1]?.content ?? "");
    return { status: "done", messages: convo };
  },
});

equal(ran.join(","), "second,first", "drain should run queued messages in reordered order");
equal(successfulDrain?.status, "done", "drain should return the last successful queued run");
assert(!hasQueuedMessages(sessionId), "drain should clear queued messages");

useSessions.getState().enqueueQueuedMessage(sessionId, { content: "fails" });
useSessions.getState().enqueueQueuedMessage(sessionId, { content: "remains" });
const failedDrain = await drainQueuedMessages({
  sessionId,
  queueDrainRef: { current: new Map() },
  generationControllersRef: { current: new Map<string, AbortController>() },
  setChatNotice: () => {},
  sessionMessages: () => [],
  runTurn: async (convo) => ({ status: "error", messages: convo }),
});
equal(failedDrain?.status, "error", "drain should return the terminal queued run result");
equal(
  useSessions.getState().queuedMessagesBySession[sessionId]?.map((item) => item.content).join(","),
  "remains",
  "drain should leave later messages queued after a failed run",
);

useSessions.getState().shiftQueuedMessage(sessionId);
useSessions.getState().enqueueQueuedMessage(sessionId, { content: "stopped" });
useSessions.getState().enqueueQueuedMessage(sessionId, { content: "send after stop" });
const joinedDrainRef = { current: new Map() };
let finishStoppedTurn!: () => void;
const stoppedTurn = new Promise<void>((resolve) => {
  finishStoppedTurn = resolve;
});
const interruptedRuns: string[] = [];
const interruptedDrain = drainQueuedMessages({
  sessionId,
  queueDrainRef: joinedDrainRef,
  generationControllersRef: { current: new Map<string, AbortController>() },
  setChatNotice: () => {},
  sessionMessages: () => [],
  runTurn: async (convo) => {
    interruptedRuns.push(convo[convo.length - 1]?.content ?? "");
    await stoppedTurn;
    return { status: "aborted", messages: convo };
  },
});
const joinedDrain = drainQueuedMessages({
  sessionId,
  queueDrainRef: joinedDrainRef,
  generationControllersRef: { current: new Map<string, AbortController>() },
  setChatNotice: () => {},
  sessionMessages: () => [],
  runTurn: async () => {
    throw new Error("joined drain must not start another turn");
  },
});
finishStoppedTurn();
await Promise.all([interruptedDrain, joinedDrain]);
await drainQueuedMessages({
  sessionId,
  queueDrainRef: joinedDrainRef,
  generationControllersRef: { current: new Map<string, AbortController>() },
  setChatNotice: () => {},
  sessionMessages: () => [],
  runTurn: async (convo) => {
    interruptedRuns.push(convo[convo.length - 1]?.content ?? "");
    return { status: "done", messages: convo };
  },
});
equal(
  interruptedRuns.join(","),
  "stopped,send after stop",
  "a joined interrupted drain should hand off to the next queued message",
);

export {};
