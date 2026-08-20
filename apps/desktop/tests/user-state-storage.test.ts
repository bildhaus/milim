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

type InvokeCall = { command: string; args?: Record<string, unknown> };
const calls: InvokeCall[] = [];
const dbValues = new Map<string, string>();
let sessionSnapshot: string | null = null;
let holdNextSessionSet: Promise<void> | null = null;

Object.defineProperty(globalThis, "__MILIM_TEST_INVOKE__", {
  value: async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === "user_state_get") {
      return dbValues.get(String(args?.key)) ?? null;
    }
    if (
      command === "user_sessions_get" ||
      command === "user_sessions_manifest_get"
    ) {
      return sessionSnapshot ?? dbValues.get("milim.sessions") ?? null;
    }
    if (command === "user_state_set") {
      dbValues.set(String(args?.key), String(args?.value));
      return null;
    }
    if (
      command === "user_sessions_set" ||
      command === "user_sessions_apply_ops"
    ) {
      if (holdNextSessionSet) await holdNextSessionSet;
      sessionSnapshot =
        command === "user_sessions_set"
          ? String(args?.value)
          : applySessionDeltaSnapshot(
              sessionSnapshot ??
                dbValues.get("milim.sessions") ??
                '{"state":{"sessions":[]},"version":0}',
              args?.delta as Parameters<typeof applySessionDeltaSnapshot>[1],
            );
      dbValues.delete("milim.sessions");
      return null;
    }
    if (command === "user_state_delete") {
      return dbValues.delete(String(args?.key));
    }
    if (command === "user_sessions_delete") {
      const deleted =
        sessionSnapshot !== null || dbValues.has("milim.sessions");
      sessionSnapshot = null;
      dbValues.delete("milim.sessions");
      return deleted;
    }
    if (command === "user_state_import_legacy") {
      for (const [key, value] of Object.entries(
        (args?.entries as Record<string, string>) ?? {},
      )) {
        if (!dbValues.has(key)) dbValues.set(key, value);
      }
      return null;
    }
    throw new Error(`Unexpected command: ${command}`);
  },
  configurable: true,
});

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

const {
  flushDeferredUserStateWrites,
  importLegacyLocalStorageOnce,
  readUserStateKey,
  userStateStorage,
  writeUserStateKey,
} = await import("../src/persistence/userStateStorage.js");

localStorage.setItem("other.settings", '{"state":{"favorites":["local"]}}');
equal(
  userStateStorage.getItem("milim.settings"),
  null,
  "browser fallback should ignore non-canonical state",
);
userStateStorage.setItem(
  "milim.settings",
  '{"state":{"favorites":["updated"]}}',
);
equal(
  localStorage.getItem("milim.settings"),
  '{"state":{"favorites":["updated"]}}',
  "browser fallback should write canonical state synchronously",
);
equal(
  userStateStorage.getItem("milim.settings"),
  '{"state":{"favorites":["updated"]}}',
  "browser fallback should prefer canonical state once written",
);

Object.defineProperty(globalThis, "window", {
  value: { __TAURI_INTERNALS__: {} },
  configurable: true,
});

localStorage.setItem("milim.sessions", '{"state":{"sessions":[]}}');
localStorage.setItem("milim.ui", '{"state":{"sidebarOpen":false},"version":0}');
localStorage.setItem(
  "milim.onboarding",
  '{"state":{"status":"completed"},"version":0}',
);
localStorage.setItem("milim.themeId", "mono-light");
localStorage.setItem(
  "milim.customThemes",
  '[{"id":"custom-test","name":"Custom Test"}]',
);
localStorage.setItem(
  "milim.mobile.urlBase",
  "https://milim-box.tailnet.ts.net:10000",
);
localStorage.setItem(
  "milim.settings",
  JSON.stringify({
    state: {
      voice: {
        provider: "openai",
        openAiApiKey: "legacy-stt-secret",
        ttsOpenAiApiKey: "legacy-tts-secret",
      },
    },
  }),
);
localStorage.setItem("unrelated", "skip me");
await importLegacyLocalStorageOnce();

assert(
  calls.some((call) => call.command === "user_state_import_legacy"),
  "legacy import command should be called in Tauri",
);
equal(
  dbValues.get("milim.sessions"),
  '{"state":{"sessions":[]}}',
  "local session state should be imported under canonical key",
);
equal(
  dbValues.get("milim.ui"),
  '{"state":{"sidebarOpen":false},"version":0}',
  "local UI state should be imported under canonical key",
);
equal(
  dbValues.get("milim.onboarding"),
  '{"state":{"status":"completed"},"version":0}',
  "local onboarding state should be imported under canonical key",
);
assert(
  !dbValues.has("unrelated"),
  "legacy import should only include app user state keys",
);
equal(
  dbValues.get("milim.themeId"),
  '"mono-light"',
  "legacy raw theme selection should import as JSON string",
);
equal(
  dbValues.get("milim.customThemes"),
  '[{"id":"custom-test","name":"Custom Test"}]',
  "legacy custom themes should remain a JSON array",
);
equal(
  dbValues.get("milim.mobile.urlBase"),
  '"https://milim-box.tailnet.ts.net:10000"',
  "legacy mobile URL base should be imported as JSON string",
);
equal(
  await readUserStateKey("milim.mobile.urlBase"),
  "https://milim-box.tailnet.ts.net:10000",
  "mobile URL base should read back as a plain URL",
);
assert(
  !dbValues.get("milim.settings")?.includes("voice"),
  "obsolete legacy voice settings should not be imported into DB",
);

dbValues.set(
  "milim.settings",
  JSON.stringify({
    state: {
      voice: {
        provider: "openai",
        openAiApiKey: "existing-db-stt-secret",
        ttsOpenAiApiKey: "existing-db-tts-secret",
      },
    },
  }),
);
const sanitizedSettings = await readUserStateKey("milim.settings");
assert(
  !sanitizedSettings?.includes("voice"),
  "obsolete DB voice settings should be removed on read",
);
assert(
  !dbValues.get("milim.settings")?.includes("voice"),
  "cleaned DB settings should be written back",
);

dbValues.set("milim.themeId", "mono-dark");
equal(
  await readUserStateKey("milim.themeId"),
  '"mono-dark"',
  "Tauri helper should normalize raw theme DB keys",
);
equal(
  dbValues.get("milim.themeId"),
  '"mono-dark"',
  "normalized theme DB key should be written back",
);

await writeUserStateKey("milim.themeId", "mono-light");
equal(
  dbValues.get("milim.themeId"),
  '"mono-light"',
  "Tauri helper should write theme IDs as JSON strings",
);

await writeUserStateKey("milim.window.alwaysOnTop", "true");
equal(
  await readUserStateKey("milim.window.alwaysOnTop"),
  "true",
  "Tauri helper should round-trip through DB commands",
);

await writeUserStateKey(
  "milim.mobile.urlBase",
  "https://milim-phone.tailnet.ts.net:10000",
);
equal(
  dbValues.get("milim.mobile.urlBase"),
  '"https://milim-phone.tailnet.ts.net:10000"',
  "mobile URL base should be written as valid JSON for Tauri state",
);
equal(
  await readUserStateKey("milim.mobile.urlBase"),
  "https://milim-phone.tailnet.ts.net:10000",
  "mobile URL base should round-trip as a plain URL",
);

const sessionWriteCountBefore = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
await readUserStateKey("milim.sessions");
const sessionA =
  '{"state":{"sessions":[{"id":"a","messages":[]}]},"version":0}';
const sessionB =
  '{"state":{"sessions":[{"id":"b","messages":[]}]},"version":0}';
const writeA = writeUserStateKey("milim.sessions", sessionA);
const writeB = writeUserStateKey("milim.sessions", sessionB);
equal(
  await readUserStateKey("milim.sessions"),
  sessionB,
  "pending session writes should be readable before flush",
);
await flushDeferredUserStateWrites("milim.sessions");
await Promise.all([writeA, writeB]);
equal(
  sessionSnapshot,
  sessionB,
  "deferred session writes should persist the latest value",
);
assert(
  !dbValues.has("milim.sessions"),
  "session writes should move out of the legacy JSON blob",
);
const sessionWriteCountAfter = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
equal(
  sessionWriteCountAfter - sessionWriteCountBefore,
  1,
  "rapid session writes should coalesce into one Tauri write",
);
await writeUserStateKey("milim.sessions", sessionB);
await flushDeferredUserStateWrites("milim.sessions");
const duplicateSessionWriteCount = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
equal(
  duplicateSessionWriteCount,
  sessionWriteCountAfter,
  "duplicate session snapshots should not write to Tauri state",
);
let releaseHeldSessionSet: (() => void) | undefined;
holdNextSessionSet = new Promise<void>((resolve) => {
  releaseHeldSessionSet = resolve;
});
const sessionC =
  '{"state":{"sessions":[{"id":"c","messages":[]}]},"version":0}';
const writeC = writeUserStateKey("milim.sessions", sessionC);
const flushC = flushDeferredUserStateWrites("milim.sessions");
await new Promise((resolve) => setTimeout(resolve, 0));
const inFlightSessionWriteCount = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
await writeUserStateKey("milim.sessions", sessionC);
const duplicateInFlightSessionWriteCount = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
equal(
  duplicateInFlightSessionWriteCount,
  inFlightSessionWriteCount,
  "duplicate in-flight session snapshots should not write to Tauri state",
);
holdNextSessionSet = null;
releaseHeldSessionSet?.();
await flushC;
await writeC;
equal(
  sessionSnapshot,
  sessionC,
  "in-flight session write should still complete",
);

(
  globalThis as typeof globalThis & {
    __MILIM_TEST_DEFERRED_WRITE_DELAY_MS__?: number;
    __MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__?: number;
  }
).__MILIM_TEST_DEFERRED_WRITE_DELAY_MS__ = 1_000;
(
  globalThis as typeof globalThis & {
    __MILIM_TEST_DEFERRED_WRITE_DELAY_MS__?: number;
    __MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__?: number;
  }
).__MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__ = 100;
const maxLatencyWriteCountBefore = calls.filter(
  (call) => call.command === "user_sessions_apply_ops",
).length;
const maxLatencyWrites: Array<void | Promise<void>> = [];
for (let i = 0; i < 3; i += 1) {
  maxLatencyWrites.push(
    writeUserStateKey(
      "milim.sessions",
      `{"state":{"sessions":[{"id":"max-${i}","messages":[]}]},"version":0}`,
    ),
  );
  await new Promise((resolve) => setTimeout(resolve, 5));
}
await new Promise((resolve) => setTimeout(resolve, 120));
await Promise.all(maxLatencyWrites);
equal(
  calls.filter((call) => call.command === "user_sessions_apply_ops").length,
  maxLatencyWriteCountBefore + 1,
  "session writes should flush at max latency under sustained writes",
);
equal(
  sessionSnapshot,
  '{"state":{"sessions":[{"id":"max-2","messages":[]}]},"version":0}',
  "max-latency session flush should persist the latest pending value",
);
delete (
  globalThis as typeof globalThis & {
    __MILIM_TEST_DEFERRED_WRITE_DELAY_MS__?: number;
    __MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__?: number;
  }
).__MILIM_TEST_DEFERRED_WRITE_DELAY_MS__;
delete (
  globalThis as typeof globalThis & {
    __MILIM_TEST_DEFERRED_WRITE_DELAY_MS__?: number;
    __MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__?: number;
  }
).__MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__;

const largeUnchangedPayload = `unchanged-${"x".repeat(10_000)}`;
const largeBase = JSON.stringify({
  state: {
    sessions: [
      {
        id: "stable",
        messages: [{ role: "user", content: largeUnchangedPayload }],
      },
      {
        id: "changing",
        messages: [{ role: "user", content: "before" }],
      },
    ],
    workerRuns: [{ id: "server-owned" }],
  },
  version: 0,
});
await writeUserStateKey("milim.sessions", largeBase);
await flushDeferredUserStateWrites("milim.sessions");
const deltaCallStart = calls.length;
await writeUserStateKey(
  "milim.sessions",
  JSON.stringify({
    state: {
      sessions: [
        {
          id: "stable",
          messages: [{ role: "user", content: largeUnchangedPayload }],
        },
        {
          id: "changing",
          messages: [{ role: "user", content: "after" }],
        },
      ],
    },
    version: 0,
  }),
);
await flushDeferredUserStateWrites("milim.sessions");
const deltaCall = calls
  .slice(deltaCallStart)
  .find((call) => call.command === "user_sessions_apply_ops");
assert(deltaCall, "changed session state should use the delta command");
const deltaWire = JSON.stringify(deltaCall.args?.delta);
assert(
  !deltaWire.includes(largeUnchangedPayload),
  "session deltas should not transmit unchanged large messages",
);
assert(
  deltaWire.includes("after"),
  "session deltas should transmit changed messages",
);
assert(
  !deltaWire.includes("server-owned"),
  "session metadata should not persist canonical Worker Runs",
);

const preservedBody = `preserved-${"z".repeat(4_000)}`;
await writeUserStateKey(
  "milim.sessions",
  JSON.stringify({
    state: {
      sessions: [
        {
          id: "partial",
          title: "Before",
          messages: [{ id: "hidden", role: "user", content: preservedBody }],
        },
      ],
    },
    version: 0,
  }),
);
await flushDeferredUserStateWrites("milim.sessions");
const partialCallStart = calls.length;
await writeUserStateKey(
  "milim.sessions",
  JSON.stringify({
    state: {
      sessions: [
        {
          id: "partial",
          title: "After",
          messages: [],
          messagesHydrated: false,
          messagesLoadedFrom: 1,
          persistedMessageCount: 1,
          messagePreview: "preserved",
        },
      ],
    },
    version: 0,
  }),
);
await flushDeferredUserStateWrites("milim.sessions");
const partialDeltaCall = calls
  .slice(partialCallStart)
  .find((call) => call.command === "user_sessions_apply_ops");
assert(partialDeltaCall, "partial metadata edits should use a session delta");
const partialDelta = partialDeltaCall.args?.delta as {
  upserts: Array<{ preserveMessages?: boolean; sessionJson?: string }>;
};
equal(
  partialDelta.upserts[0]?.preserveMessages,
  true,
  "partial sessions must preserve unseen SQLite message rows",
);
assert(
  !JSON.stringify(partialDelta).includes(preservedBody),
  "partial deltas should not reserialize unloaded message bodies",
);
assert(
  sessionSnapshot?.includes(preservedBody),
  "partial metadata changes must not truncate unseen messages",
);
assert(
  !sessionSnapshot?.includes("messagesHydrated"),
  "manifest hydration markers must remain transient",
);

export {};
