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

function equal<T>(actual: T, expected: T, message: string) {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const { useSettings } = await import("../src/settings/store.js");
const { useSessions } = await import("../src/sessions/store.js");

let state = useSessions.getState();
state.updateSettings(state.activeId, {
  model: "inherited-model",
  folder: "C:\\project",
  computerUse: true,
  planMode: true,
  instructions: "temporary",
  memory: false,
});
state.setMessages(state.activeId, [{ role: "user", content: "existing" }], { autoTitle: false });
useSettings.getState().setNewThreadBehavior("inherit");
const inheritedId = useSessions.getState().newUserChat();
let inherited = useSessions.getState().getSettings(inheritedId);
equal(inherited.model, "inherited-model", "inherit mode should retain the current model");
equal(inherited.folder, "C:\\project", "inherit mode should retain the project");
equal(inherited.memory, false, "inherit mode should retain memory preference");
equal(inherited.computerUse, false, "new chats should always reset Computer Use");
equal(inherited.planMode, false, "new chats should always reset Plan Mode");
equal(inherited.instructions, "", "new chats should always reset temporary instructions");

useSessions.getState().setMessages(inheritedId, [{ role: "user", content: "next" }], { autoTitle: false });
useSettings.getState().setNewThreadBehavior("configured");
useSettings.getState().setConfiguredThreadDefaults({
  model: "configured-model",
  activeAgentId: "agent-1",
  memory: true,
  privacy: "redact",
  sandbox: true,
  toolApproval: "review",
  workerModel: "worker-model",
  delegationPolicy: "auto",
});
const configuredId = useSessions.getState().newUserChat();
const configured = useSessions.getState().getSettings(configuredId);
equal(configured.model, "configured-model", "configured mode should use its model");
equal(configured.folder, "C:\\project", "configured mode should preserve project context");
equal(configured.activeAgentId, "agent-1", "configured mode should use its agent");
equal(configured.toolApproval, "review", "configured mode should use safe approval defaults");
equal(configured.computerUse, false, "configured mode should still reset Computer Use");

export {};
