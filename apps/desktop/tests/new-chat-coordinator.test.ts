class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  clear() { this.values.clear(); }
  getItem(key: string) { return this.values.get(key) ?? null; }
  key(index: number) { return Array.from(this.values.keys())[index] ?? null; }
  removeItem(key: string) { this.values.delete(key); }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

Object.defineProperty(globalThis, "localStorage", { value: new MemoryStorage(), configurable: true });

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const { useSessions } = await import("../src/sessions/store.js");
const { createCanonicalChat } = await import("../src/lib/newChatCoordinator.js");

useSessions.getState().setMessages(
  useSessions.getState().activeId,
  [{ role: "user", content: "existing" }],
  { autoTitle: false },
);

const requests: Array<Record<string, unknown>> = [];
globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
  const command = JSON.parse(String(init?.body)) as Record<string, unknown>;
  requests.push(command);
  const payload = command.payload as { id: string };
  assert(
    !useSessions.getState().sessions.some((session) => session.id === payload.id),
    "thread.create must reach Rust before the local session becomes visible",
  );
  return new Response(JSON.stringify({
    command_id: command.command_id,
    status: "applied",
    thread_id: payload.id,
    revision: 1,
    data: null,
  }), { status: 200, headers: { "Content-Type": "application/json" } });
}) as typeof fetch;

const createdId = await createCanonicalChat({ model: "openrouter/test" });
assert(requests.length === 1, "canonical chat creation should send one control command");
assert(requests[0].kind === "thread.create", "new chats should use the canonical thread.create command");
assert(useSessions.getState().activeId === createdId, "the applied canonical thread should become active locally");
assert(useSessions.getState().getSettings(createdId).model === "openrouter/test", "local settings should match the canonical create payload");

useSessions.getState().setMessages(createdId, [{ role: "user", content: "occupied" }], { autoTitle: false });
const retryRequests: Array<Record<string, unknown>> = [];
let retryAttempt = 0;
globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
  const command = JSON.parse(String(init?.body)) as Record<string, unknown>;
  retryRequests.push(command);
  retryAttempt += 1;
  if (retryAttempt === 1) throw new Error("ambiguous connection reset");
  const payload = command.payload as { id: string };
  return new Response(JSON.stringify({
    command_id: command.command_id,
    status: "applied",
    thread_id: payload.id,
    revision: 1,
    data: null,
  }), { status: 200, headers: { "Content-Type": "application/json" } });
}) as typeof fetch;

const retriedId = await createCanonicalChat();
assert(retriedId !== createdId, "a new canonical chat should use a fresh host-owned id");
assert(retryRequests.length === 2, "an ambiguous create response should retry once");
assert(
  retryRequests[0].command_id === retryRequests[1].command_id,
  "an ambiguous retry must retain the original command id",
);

useSessions.getState().setMessages(retriedId, [{ role: "user", content: "keep me" }], { autoTitle: false });
const beforeFailureIds = useSessions.getState().sessions.map((session) => session.id);
globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
  const command = JSON.parse(String(init?.body)) as Record<string, unknown>;
  return new Response(JSON.stringify({
    command_id: command.command_id,
    status: "failed",
    message: "storage unavailable",
    data: null,
  }), { status: 200, headers: { "Content-Type": "application/json" } });
}) as typeof fetch;

const fallbackId = await createCanonicalChat();
assert(fallbackId === retriedId, "failed canonical creation should keep the current chat active");
assert(
  JSON.stringify(useSessions.getState().sessions.map((session) => session.id)) === JSON.stringify(beforeFailureIds),
  "failed canonical creation must not leave an optimistic local thread",
);

export {};
