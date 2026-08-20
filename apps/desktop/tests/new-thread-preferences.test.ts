import type { ModelInfo } from "../src/api";

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
const { reasoningEffortForThread } = await import("../src/lib/reasoningEffort.js");

const models: ModelInfo[] = [{
  id: "inherited-model",
  owned_by: "OpenAI",
  reasoning: {
    supported_efforts: ["low", "medium", "high"],
    default_effort: "medium",
    default_enabled: true,
    mandatory: true,
  },
}];

let state = useSessions.getState();
const originalId = state.activeId;
state.updateSettings(state.activeId, {
  model: "inherited-model",
  folder: "C:\\project",
  computerUse: true,
  planMode: true,
  instructions: "temporary",
  memory: false,
  reasoningEffortOverrides: { "inherited-model": "low" },
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
equal(inherited.reasoningEffortOverrides, undefined, "new chats should inherit app-wide reasoning defaults instead of another chat's overrides");

useSessions.getState().updateSettings(originalId, { reasoningEffortOverrides: {} });
useSettings.getState().setModelReasoningEffort("inherited-model", "medium");
useSessions.getState().updateSettings(inheritedId, {
  reasoningEffortOverrides: { "inherited-model": "high" },
});
equal(
  reasoningEffortForThread(
    useSessions.getState().getSettings(originalId).reasoningEffortOverrides,
    useSettings.getState().reasoningEffortByModel,
    "inherited-model",
    models,
  ),
  "medium",
  "changing effort in one chat should not retune another chat without an override",
);
equal(
  reasoningEffortForThread(
    useSessions.getState().getSettings(inheritedId).reasoningEffortOverrides,
    useSettings.getState().reasoningEffortByModel,
    "inherited-model",
    models,
  ),
  "high",
  "the chat that changed effort should resolve its own override",
);

useSessions.getState().setMessages(inheritedId, [{ role: "user", content: "next" }], { autoTitle: false });
useSettings.getState().setNewThreadBehavior("configured");
useSettings.getState().setConfiguredThreadDefaults({
  model: "configured-model",
  activeAgentId: "agent-1",
  memory: true,
  privacy: "redact",
  sandbox: true,
  toolApproval: "open",
  workerModel: "worker-model",
  delegationPolicy: "auto",
});
const configuredId = useSessions.getState().newUserChat();
const configured = useSessions.getState().getSettings(configuredId);
equal(configured.model, "configured-model", "configured mode should use its model");
equal(configured.folder, "C:\\project", "configured mode should preserve project context");
equal(configured.activeAgentId, "agent-1", "configured mode should use its agent");
equal(configured.toolApproval, "open", "configured mode should allow an explicit Open approval default");
equal(configured.computerUse, false, "configured mode should still reset Computer Use");

useSessions.getState().setMessages(configuredId, [{ role: "user", content: "occupied" }], { autoTitle: false });
const sessionCountBeforePreview = useSessions.getState().sessions.length;
const preview = useSessions.getState().getNewUserChatSettings({ activeAgentId: "agent-2" });
equal(preview.activeAgentId, "agent-2", "canonical creation should be able to resolve final settings before insertion");
equal(useSessions.getState().sessions.length, sessionCountBeforePreview, "resolving new-chat settings must not expose a local session");
const canonicalId = useSessions.getState().newUserChat({ activeAgentId: "agent-2" }, "canonical-thread-id");
equal(canonicalId, "canonical-thread-id", "canonical creation should commit the host-owned thread id locally");
equal(useSessions.getState().getSettings(canonicalId).activeAgentId, "agent-2", "canonical-id insertion should retain resolved settings");

export {};
