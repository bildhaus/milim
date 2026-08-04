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

const invokeCalls: string[] = [];
let releaseUiWrite: (() => void) | undefined;
Object.defineProperty(globalThis, "window", {
  value: {
    __TAURI_INTERNALS__: {
      invoke: async (command: string) => {
        invokeCalls.push(command);
        if (command === "user_state_set" && releaseUiWrite) {
          await new Promise<void>((resolve) => {
            const release = releaseUiWrite;
            releaseUiWrite = () => {
              release?.();
              resolve();
            };
          });
        }
        return null;
      },
    },
  },
  configurable: true,
});

const { useUpdateStore } = await import("../src/update/store.js");
const { writeUserStateKey } = await import("../src/persistence/userStateStorage.js");
equal(useUpdateStore.getState().automaticCheck, true, "automatic checks should default on");
equal(useUpdateStore.getState().automaticDownload, true, "automatic downloads should default on");
useUpdateStore.getState().setAutomaticCheck(false);
useUpdateStore.getState().setAutomaticDownload(false);
equal(useUpdateStore.getState().automaticCheck, false, "automatic checks should update");
equal(useUpdateStore.getState().automaticDownload, false, "automatic downloads should update");

localStorage.setItem("milim.local.updates", JSON.stringify({ state: { automaticCheck: true, automaticDownload: false }, version: 0 }));
await useUpdateStore.persist.rehydrate();
equal(useUpdateStore.getState().automaticDownload, true, "legacy automatic download state should migrate on");

localStorage.setItem("milim.local.updates", JSON.stringify({ state: { automaticCheck: true, automaticDownload: false }, version: 1 }));
await useUpdateStore.persist.rehydrate();
equal(useUpdateStore.getState().automaticDownload, false, "post-migration opt-out should persist");

localStorage.setItem("milim.local.updates", JSON.stringify({ state: { automaticCheck: "yes", automaticDownload: 1 }, version: 1 }));
await useUpdateStore.persist.rehydrate();
equal(useUpdateStore.getState().automaticCheck, true, "invalid automatic check state should normalize");
equal(useUpdateStore.getState().automaticDownload, true, "invalid automatic download state should normalize");

const pendingSessionWrite = writeUserStateKey(
  "milim.sessions",
  '{"state":{"sessions":[{"id":"before-update","messages":[]}]}}',
);
releaseUiWrite = () => {};
const pendingUiWrite = writeUserStateKey(
  "milim.ui",
  '{"state":{"settledThreadsEnabled":true,"aiThreadNames":true,"autoColorThreadNames":true}}',
);
useUpdateStore.setState({ updatePath: "/tmp/milim.app", status: "ready" });
const install = useUpdateStore.getState().installNow();
await new Promise((resolve) => setTimeout(resolve, 0));
equal(
  invokeCalls.includes("apply_update"),
  false,
  "pending UI preferences should block update installation",
);
releaseUiWrite?.();
await Promise.all([install, pendingSessionWrite, pendingUiWrite]);
equal(
  invokeCalls.indexOf("user_sessions_set") < invokeCalls.indexOf("apply_update"),
  true,
  "pending session state should flush before applying an update",
);
equal(
  invokeCalls.indexOf("user_state_set") < invokeCalls.indexOf("apply_update"),
  true,
  "pending UI preferences should flush before applying an update",
);

export {};
