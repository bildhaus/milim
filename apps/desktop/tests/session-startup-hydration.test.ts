import { applySessionDeltaSnapshot } from "./session-delta-test-helper.js";

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

Object.defineProperty(globalThis, "window", {
  value: { __TAURI_INTERNALS__: {} },
  configurable: true,
});

type InvokeCall = { command: string; args?: Record<string, unknown> };

const calls: InvokeCall[] = [];
const dbValues = new Map<string, string>();
let releaseSessionGet: (() => void) | undefined;
const sessionGetGate = new Promise<void>((resolve) => {
  releaseSessionGet = resolve;
});

const persistedSessions = JSON.stringify({
  state: {
    sessions: [
      {
        id: "persisted-session",
        title: "Persisted chat",
        messages: [
          { role: "user", content: "keep me" },
          {
            role: "assistant",
            content: "restored answer",
            streamParts: [
              {
                kind: "event",
                eventType: "tool",
                label: "Used tool",
                status: "done",
              },
            ],
          },
        ],
        artifactPanelOpen: true,
        sidePanelMode: "artifact",
        artifactPanelTab: "code",
        contextPanelOpen: true,
        contextCollapsedSectionIds: ["sources", "activity", "sources", "invalid"],
        createdAt: 1,
        updatedAt: 1,
      },
      {
        id: "title-only-session",
        title: "Recovered title only",
        artifactPanelOpen: true,
        artifactPanelTab: "code",
        contextPanelOpen: "invalid",
        contextCollapsedSectionIds: "invalid",
        createdAt: 2,
        updatedAt: 2,
      },
      {
        id: "legacy-workers-session",
        title: "Legacy Workers inspector",
        messages: [],
        inspectorOpen: true,
        inspectorTab: "workers",
        createdAt: 3,
        updatedAt: 3,
      },
    ],
    projects: [
      {
        id: "project:C:\\keep",
        name: "Keep",
        folder: "C:\\keep",
        icon: "star",
        color: "#33AACC",
        createdAt: 1,
        updatedAt: 1,
      },
    ],
    activeId: "persisted-session",
    archiveRetentionDays: 30,
    sidebar: {
      sectionOrder: ["chats", "project:C:\\keep"],
      projectFolders: ["C:\\keep"],
    },
  },
  version: 0,
});

dbValues.set("milim.sessions", persistedSessions);

Object.defineProperty(globalThis, "__MILIM_TEST_INVOKE__", {
  value: async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === "user_state_import_legacy") return null;
    if (command === "user_sessions_get") {
      await sessionGetGate;
      return dbValues.get("milim.sessions") ?? null;
    }
    if (command === "user_sessions_set") {
      dbValues.set("milim.sessions", String(args?.value));
      return null;
    }
    if (command === "user_sessions_apply_delta") {
      dbValues.set(
        "milim.sessions",
        applySessionDeltaSnapshot(
          dbValues.get("milim.sessions") ??
            '{"state":{"sessions":[]},"version":0}',
          args?.delta as Parameters<typeof applySessionDeltaSnapshot>[1],
        ),
      );
      return null;
    }
    if (command === "user_state_get") {
      if (args?.key === "milim.sessions") await sessionGetGate;
      return dbValues.get(String(args?.key)) ?? null;
    }
    if (command === "user_state_set") {
      dbValues.set(String(args?.key), String(args?.value));
      return null;
    }
    if (command === "user_state_delete") {
      return dbValues.delete(String(args?.key));
    }
    throw new Error(`Unexpected command: ${command}`);
  },
  configurable: true,
});

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const { purgeExpiredArchivesAfterHydration, useSessions } =
  await import("../src/sessions/store.js");
const { flushDeferredUserStateWrites } =
  await import("../src/persistence/userStateStorage.js");

purgeExpiredArchivesAfterHydration();
useSessions.getState().setPreviewRuntime(useSessions.getState().activeId, {
  status: "idle",
});
await new Promise((resolve) => setTimeout(resolve, 0));
await flushDeferredUserStateWrites("milim.sessions");

assert(
  !calls.some(
    (call) =>
      call.command === "user_sessions_set" ||
      call.command === "user_sessions_apply_delta",
  ),
  "startup mutations should not write sessions before hydration",
);

releaseSessionGet?.();
await new Promise((resolve) => setTimeout(resolve, 0));
if (!useSessions.persist.hasHydrated()) {
  await new Promise<void>((resolve) => {
    useSessions.persist.onFinishHydration(() => resolve());
  });
}
await flushDeferredUserStateWrites("milim.sessions");

const stored = JSON.parse(dbValues.get("milim.sessions") ?? "{}");
assert(
  stored.state.sessions[0]?.id === "persisted-session",
  "startup archive cleanup should preserve hydrated sessions",
);
assert(
  stored.state.sessions[1]?.id === "title-only-session",
  "hydration should keep recovered title-only sessions",
);
assert(
  Array.isArray(stored.state.sessions[1]?.messages),
  "title-only sessions should hydrate with an empty messages array",
);
assert(
  useSessions.getState().sessions[1]?.inspectorOpen === true &&
    useSessions.getState().sessions[1]?.inspectorTab === "code",
  "legacy code tabs should migrate even when sidePanelMode was absent",
);
assert(
  useSessions.getState().sessions[0]?.contextPanelOpen === true &&
    useSessions.getState().sessions[1]?.contextPanelOpen === undefined,
  "context panel hydration should preserve only explicit true values",
);
assert(
  JSON.stringify(
    useSessions.getState().sessions[0]?.contextCollapsedSectionIds,
  ) === JSON.stringify(["sources", "activity"]) &&
    useSessions.getState().sessions[1]?.contextCollapsedSectionIds === undefined,
  "context section hydration should preserve unique known section IDs",
);
assert(
  useSessions.getState().sessions[2]?.contextPanelOpen === undefined &&
    useSessions.getState().sessions[2]?.inspectorOpen === true &&
    useSessions.getState().sessions[2]?.inspectorTab === "workers",
  "Workers inspector state should remain attached to the unified inspector",
);
assert(
  stored.state.projects[0]?.folder === "C:\\keep",
  "startup archive cleanup should preserve hydrated projects",
);
assert(
  useSessions.getState().projects[0]?.icon === "star" &&
    useSessions.getState().projects[0]?.color === "#33aacc",
  "startup hydration should preserve and normalize project customization",
);
assert(
  stored.state.sessions[0]?.messages[0]?.content === "keep me",
  "startup archive cleanup should preserve messages",
);
const hydratedAssistant = useSessions.getState().sessions[0]?.messages[1];
assert(
  hydratedAssistant?.streamParts?.some(
    (part) => part.kind === "text" && part.content === "restored answer",
  ),
  "hydration should regenerate stripped text stream parts from assistant content",
);
const migratedInspector = useSessions.getState().sessions[0];
assert(
  migratedInspector?.inspectorOpen === true &&
    migratedInspector.inspectorTab === "code",
  "hydration should migrate legacy artifact panel state to the unified inspector",
);
assert(
  stored.state.sessions[0]?.inspectorOpen === true &&
    stored.state.sessions[0]?.inspectorTab === "code",
  "the first post-hydration write should persist unified inspector fields",
);
assert(
  !("artifactPanelOpen" in stored.state.sessions[0]) &&
    !("sidePanelMode" in stored.state.sessions[0]) &&
    !("artifactPanelTab" in stored.state.sessions[0]),
  "post-migration persistence should omit all legacy side-panel fields",
);

export {};
