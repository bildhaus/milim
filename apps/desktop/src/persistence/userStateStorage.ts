import { invoke } from "@tauri-apps/api/core";
import type {
  PersistStorage,
  StateStorage,
  StorageValue,
} from "zustand/middleware";
import { incrementPerfCounter, recordPerfMeasure } from "../lib/perf.js";
import type { ChatSearchResult } from "../lib/chatSearch.js";

const CANONICAL_USER_STATE_KEYS = [
  "milim.sessions",
  "milim.settings",
  "milim.ui",
  "milim.onboarding",
  "milim.themeId",
  "milim.customThemes",
  "milim.appearanceSnapshot",
  "milim.window.alwaysOnTop",
  "milim.mobile.urlBase",
  "milim.sessionDrafts",
] as const;

const SYNCED_SETTINGS_KEY = "milim.settings";
const SESSIONS_KEY = "milim.sessions";
const THEME_ID_KEY = "milim.themeId";
const MOBILE_URL_BASE_KEY = "milim.mobile.urlBase";
const DEFERRED_WRITE_DELAY_MS = 150;
const DEFERRED_WRITE_MAX_LATENCY_MS = 3_000;
const FLUSH_USER_STATE_EVENT = "milim://flush-user-state";
const FLUSH_USER_STATE_AND_EXIT_EVENT = "milim://flush-user-state-and-exit";

type UserStateKey = (typeof CANONICAL_USER_STATE_KEYS)[number] | (string & {});
type InvokeFn = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;
type TestGlobal = typeof globalThis & {
  __MILIM_TEST_INVOKE__?: InvokeFn;
  __MILIM_TEST_DEFERRED_WRITE_DELAY_MS__?: number;
  __MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__?: number;
};

const memoryStorage = new Map<string, string>();
const lastWrittenValues = new Map<string, string>();
const pendingUserStateWrites = new Set<Promise<void>>();
let legacyImportPromise: Promise<void> | null = null;

type SessionStorageValue = StorageValue<Record<string, unknown>>;
type SessionMessageDelta = { index: number; messageJson: string };
type SessionDelta = {
  id: string;
  sessionJson?: string;
  runtimeBindingChanges?: string[];
  baseMessageCount: number;
  messageCount: number;
  preserveMessages?: boolean;
  messages: SessionMessageDelta[];
};
export type SessionMessagePage = {
  session_id: string;
  first_index: number;
  total: number;
  has_older: boolean;
  messages: unknown[];
};
type SessionsDelta = {
  metaJson: string;
  sessionOrder: string[];
  upserts: SessionDelta[];
  deletedSessionIds: string[];
};
type DeferredSessionWrite = {
  value: SessionStorageValue;
  timer: ReturnType<typeof setTimeout> | null;
  maxTimer: ReturnType<typeof setTimeout> | null;
  resolve: Array<() => void>;
  reject: Array<(error: unknown) => void>;
};

let committedSessionValue: SessionStorageValue | null = null;
let inFlightSessionValue: SessionStorageValue | null = null;
let deferredSessionWrite: DeferredSessionWrite | null = null;
let sessionFlushPromise: Promise<void> | null = null;
let lifecycleFlushHandlersInstalled = false;

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function getLocalStorage(): Storage | null {
  try {
    return typeof globalThis.localStorage === "undefined"
      ? null
      : globalThis.localStorage;
  } catch {
    return null;
  }
}

function localGetItem(key: string): string | null {
  const storage = getLocalStorage();
  return storage ? storage.getItem(key) : (memoryStorage.get(key) ?? null);
}

function localSetItem(key: string, value: string): void {
  const storage = getLocalStorage();
  if (storage) {
    storage.setItem(key, value);
  } else {
    memoryStorage.set(key, value);
  }
}

function localRemoveItem(key: string): void {
  const storage = getLocalStorage();
  if (storage) {
    storage.removeItem(key);
  } else {
    memoryStorage.delete(key);
  }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

const ACCOUNT_RUNTIME_FIELDS = [
  ["codex", "codexThreadId", "codexLastSyncedMessageId"],
  ["claude", "claudeSessionId", "claudeLastSyncedMessageId"],
  ["opencode", "opencodeSessionId", "opencodeLastSyncedMessageId"],
  ["pi", "piSessionId", "piLastSyncedMessageId"],
] as const;

function runtimeBindingChanges(
  previous: Record<string, unknown> | undefined,
  next: Record<string, unknown>,
): string[] {
  const previousRuntime = asRecord(previous?.accountRuntime);
  const nextRuntime = asRecord(next.accountRuntime);
  return ACCOUNT_RUNTIME_FIELDS.flatMap(([kind, idField, cursorField]) =>
    previousRuntime?.[idField] === nextRuntime?.[idField]
      && previousRuntime?.[cursorField] === nextRuntime?.[cursorField]
      ? []
      : [kind],
  );
}

function parseSessionStorageValue(value: string): SessionStorageValue {
  const parsed = asRecord(JSON.parse(value));
  if (!parsed || !asRecord(parsed.state)) {
    throw new Error("Invalid persisted session state");
  }
  return parsed as SessionStorageValue;
}

function persistedSessions(value: SessionStorageValue | null): Array<
  Record<string, unknown>
> {
  if (!value) return [];
  const sessions = asRecord(value.state)?.sessions;
  if (!Array.isArray(sessions)) return [];
  return sessions.map((session) => {
    const record = asRecord(session);
    if (!record || typeof record.id !== "string" || !record.id.trim()) {
      throw new Error("Persisted sessions require non-empty ids");
    }
    return record;
  });
}

function sessionMetaJson(value: SessionStorageValue): string {
  const state = { ...(asRecord(value.state) ?? {}) };
  delete state.sessions;
  delete state.workerRuns;
  return JSON.stringify({ ...value, state });
}

function sessionRowJson(session: Record<string, unknown>): string {
  const row = { ...session };
  delete row.messages;
  delete row.messagesHydrated;
  delete row.persistedMessageCount;
  delete row.messagesLoadedFrom;
  delete row.messagePreview;
  return JSON.stringify(row);
}

function sessionMessages(session: Record<string, unknown>): unknown[] {
  return Array.isArray(session.messages) ? session.messages : [];
}

function sameOrder(left: string[], right: string[]): boolean {
  return (
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function buildSessionsDelta(
  previous: SessionStorageValue,
  next: SessionStorageValue,
): SessionsDelta | null {
  const previousSessions = persistedSessions(previous);
  const nextSessions = persistedSessions(next);
  const previousById = new Map(
    previousSessions.map((session) => [String(session.id), session]),
  );
  const sessionOrder = nextSessions.map((session) => String(session.id));
  if (new Set(sessionOrder).size !== sessionOrder.length) {
    throw new Error("Persisted session ids must be unique");
  }
  const nextIds = new Set(sessionOrder);
  const deletedSessionIds = previousSessions
    .map((session) => String(session.id))
    .filter((id) => !nextIds.has(id));
  const upserts: SessionDelta[] = [];

  for (const session of nextSessions) {
    const id = String(session.id);
    const previousSession = previousById.get(id);
    const messages = sessionMessages(session);
    const preserveMessages = session.messagesHydrated === false;
    const previousWasPartial = previousSession?.messagesHydrated === false;
    const previousMessages = previousSession && !previousWasPartial
      ? sessionMessages(previousSession)
      : [];
    const baseMessageCount = previousWasPartial
      ? Number(previousSession?.persistedMessageCount ?? previousMessages.length)
      : previousMessages.length;
    const changedMessages: SessionMessageDelta[] = [];
    if (!preserveMessages) {
      for (let index = 0; index < messages.length; index += 1) {
        const message = messages[index];
        const previousMessage = previousMessages[index];
        if (message === previousMessage) continue;
        const messageJson = JSON.stringify(message);
        if (messageJson === undefined) {
          throw new Error("Persisted messages must be JSON values");
        }
        if (
          previousMessage !== undefined &&
          messageJson === JSON.stringify(previousMessage)
        ) {
          continue;
        }
        changedMessages.push({ index, messageJson });
      }
    }

    const rowJson = sessionRowJson(session);
    const previousRowJson = previousSession
      ? sessionRowJson(previousSession)
      : null;
    const sessionJson = rowJson === previousRowJson ? undefined : rowJson;
    if (
      sessionJson !== undefined ||
      changedMessages.length > 0 ||
      (!preserveMessages && messages.length !== previousMessages.length)
    ) {
      upserts.push({
        id,
        sessionJson,
        runtimeBindingChanges: runtimeBindingChanges(previousSession, session),
        baseMessageCount,
        messageCount: preserveMessages
          ? Number(session.persistedMessageCount ?? messages.length)
          : messages.length,
        preserveMessages,
        messages: changedMessages,
      });
    }
  }

  const previousOrder = previousSessions.map((session) => String(session.id));
  const previousState = asRecord(previous.state) ?? {};
  const metaJson = sessionMetaJson(next);
  if (
    upserts.length === 0 &&
    deletedSessionIds.length === 0 &&
    sameOrder(previousOrder, sessionOrder) &&
    !("workerRuns" in previousState) &&
    sessionMetaJson(previous) === metaJson
  ) {
    return null;
  }
  return { metaJson, sessionOrder, upserts, deletedSessionIds };
}

function sanitizeSyncedSettings(value: string): string {
  try {
    const parsed = JSON.parse(value);
    const root = asRecord(parsed);
    const state = asRecord(root?.state);
    if (!state || !("voice" in state)) return value;
    delete state.voice;
    return JSON.stringify(parsed);
  } catch {
    return value;
  }
}

function sanitizeThemeId(value: string): string {
  try {
    const parsed = JSON.parse(value);
    return typeof parsed === "string"
      ? JSON.stringify(parsed)
      : JSON.stringify(String(parsed ?? ""));
  } catch {
    return JSON.stringify(value);
  }
}

function sanitizeJsonString(value: string): string {
  try {
    const parsed = JSON.parse(value);
    return JSON.stringify(
      typeof parsed === "string" ? parsed : String(parsed ?? ""),
    );
  } catch {
    return JSON.stringify(value);
  }
}

function readJsonString(value: string): string {
  try {
    const parsed = JSON.parse(value);
    return typeof parsed === "string" ? parsed : String(parsed ?? "");
  } catch {
    return value;
  }
}

function sanitizeUserStateValue(
  key: string,
  value: string | null,
): string | null {
  if (value === null) return null;
  if (key === THEME_ID_KEY) return sanitizeThemeId(value);
  if (key === MOBILE_URL_BASE_KEY) return sanitizeJsonString(value);
  return key === SYNCED_SETTINGS_KEY ? sanitizeSyncedSettings(value) : value;
}

function userStateReadValue(key: string, value: string): string {
  return key === MOBILE_URL_BASE_KEY ? readJsonString(value) : value;
}

async function invokeUserState<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const testInvoke = (globalThis as TestGlobal).__MILIM_TEST_INVOKE__;
  if (testInvoke) return testInvoke<T>(command, args);
  return invoke<T>(command, args);
}

function stateGetCommand(key: string): {
  command: string;
  args?: Record<string, unknown>;
} {
  return key === SESSIONS_KEY
    ? { command: "user_sessions_get" }
    : { command: "user_state_get", args: { key } };
}

function stateSetCommand(
  key: string,
  value: string,
): { command: string; args?: Record<string, unknown> } {
  return key === SESSIONS_KEY
    ? { command: "user_sessions_set", args: { value } }
    : { command: "user_state_set", args: { key, value } };
}

function stateDeleteCommand(key: string): {
  command: string;
  args?: Record<string, unknown>;
} {
  return key === SESSIONS_KEY
    ? { command: "user_sessions_delete" }
    : { command: "user_state_delete", args: { key } };
}

function deferredWriteDelayMs(): number {
  const override = (globalThis as TestGlobal)
    .__MILIM_TEST_DEFERRED_WRITE_DELAY_MS__;
  return typeof override === "number" &&
    Number.isFinite(override) &&
    override >= 0
    ? override
    : DEFERRED_WRITE_DELAY_MS;
}

function deferredWriteMaxLatencyMs(): number {
  const override = (globalThis as TestGlobal)
    .__MILIM_TEST_DEFERRED_WRITE_MAX_LATENCY_MS__;
  return typeof override === "number" &&
    Number.isFinite(override) &&
    override >= 0
    ? override
    : DEFERRED_WRITE_MAX_LATENCY_MS;
}

function sessionValuesMatch(
  left: SessionStorageValue,
  right: SessionStorageValue,
): boolean {
  return left === right || buildSessionsDelta(left, right) === null;
}

async function flushDeferredSessionWrite(): Promise<void> {
  if (sessionFlushPromise) {
    await sessionFlushPromise;
    if (deferredSessionWrite) await flushDeferredSessionWrite();
    return;
  }
  const entry = deferredSessionWrite;
  if (!entry) return;
  deferredSessionWrite = null;
  if (entry.timer != null) clearTimeout(entry.timer);
  if (entry.maxTimer != null) clearTimeout(entry.maxTimer);
  entry.timer = null;
  entry.maxTimer = null;

  inFlightSessionValue = entry.value;
  sessionFlushPromise = (async () => {
    try {
      incrementPerfCounter(`persist.${SESSIONS_KEY}.flush`);
      if (committedSessionValue) {
        const delta = buildSessionsDelta(committedSessionValue, entry.value);
        if (delta) {
          recordPerfMeasure(
            `persist.${SESSIONS_KEY}.bytes`,
            JSON.stringify(delta).length,
          );
          await invokeUserState<void>("user_sessions_apply_ops", { delta });
        }
      } else {
        const value = JSON.stringify(entry.value);
        recordPerfMeasure(`persist.${SESSIONS_KEY}.bytes`, value.length);
        await invokeUserState<void>("user_sessions_set", { value });
      }
      committedSessionValue = entry.value;
      entry.resolve.forEach((resolve) => resolve());
    } catch (error) {
      entry.reject.forEach((reject) => reject(error));
      throw error;
    }
  })();
  try {
    await sessionFlushPromise;
  } finally {
    sessionFlushPromise = null;
    inFlightSessionValue = null;
  }
  if (deferredSessionWrite) await flushDeferredSessionWrite();
}

function cancelDeferredSessionWrite(): void {
  const entry = deferredSessionWrite;
  if (!entry) return;
  deferredSessionWrite = null;
  if (entry.timer != null) clearTimeout(entry.timer);
  if (entry.maxTimer != null) clearTimeout(entry.maxTimer);
  entry.resolve.forEach((resolve) => resolve());
}

function deferSessionWrite(value: SessionStorageValue): Promise<void> {
  let entry = deferredSessionWrite;
  if (!entry) {
    entry = { value, timer: null, maxTimer: null, resolve: [], reject: [] };
    deferredSessionWrite = entry;
    entry.maxTimer = setTimeout(
      () => void flushDeferredSessionWrite().catch(() => {}),
      deferredWriteMaxLatencyMs(),
    );
  }
  entry.value = value;
  const promise = new Promise<void>((resolve, reject) => {
    entry.resolve.push(resolve);
    entry.reject.push(reject);
  });
  if (entry.timer != null) clearTimeout(entry.timer);
  entry.timer = setTimeout(
    () => void flushDeferredSessionWrite().catch(() => {}),
    deferredWriteDelayMs(),
  );
  return promise;
}

async function flushPendingUserStateWrites(): Promise<void> {
  while (pendingUserStateWrites.size) {
    await Promise.all([...pendingUserStateWrites]);
  }
}

export async function flushDeferredUserStateWrites(key?: UserStateKey): Promise<void> {
  if (!key || key === SESSIONS_KEY) await flushDeferredSessionWrite();
  if (!key) await flushPendingUserStateWrites();
}

export function installUserStateFlushHandlers(): void {
  if (lifecycleFlushHandlersInstalled || typeof window === "undefined") return;
  lifecycleFlushHandlersInstalled = true;

  const flush = () => {
    void flushDeferredUserStateWrites().catch((error) =>
      console.warn("Failed to flush user state:", error),
    );
  };
  const flushAndExit = () => {
    void flushDeferredUserStateWrites()
      .catch((error) =>
        console.warn("Failed to flush user state before exit:", error),
      )
      .finally(() => {
        if (inTauri())
          void invokeUserState<void>("quit_after_user_state_flush").catch(
            () => {},
          );
      });
  };

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flush();
  });
  window.addEventListener("pagehide", flush);
  window.addEventListener("beforeunload", flush);

  if (inTauri()) {
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        Promise.all([
          listen(FLUSH_USER_STATE_EVENT, flush),
          listen(FLUSH_USER_STATE_AND_EXIT_EVENT, flushAndExit),
        ]),
      )
      .catch(() => {});
  }
}

function legacyEntries(): Record<string, string> {
  const entries: Record<string, string> = {};
  for (const key of CANONICAL_USER_STATE_KEYS) {
    const value = localGetItem(key);
    const sanitized = sanitizeUserStateValue(key, value);
    if (sanitized !== null) entries[key] = sanitized;
  }
  return entries;
}

export function importLegacyLocalStorageOnce(): Promise<void> {
  if (!inTauri()) return Promise.resolve();
  if (!legacyImportPromise) {
    legacyImportPromise = invokeUserState<void>("user_state_import_legacy", {
      entries: legacyEntries(),
    }).catch((error) => {
      legacyImportPromise = null;
      throw error;
    });
  }
  return legacyImportPromise;
}

function readSessionStorageValue():
  | SessionStorageValue
  | null
  | Promise<SessionStorageValue | null> {
  if (!inTauri()) {
    const value = localGetItem(SESSIONS_KEY);
    committedSessionValue = value ? parseSessionStorageValue(value) : null;
    return committedSessionValue;
  }
  if (deferredSessionWrite) return deferredSessionWrite.value;
  if (inFlightSessionValue) return inFlightSessionValue;
  return importLegacyLocalStorageOnce().then(async () => {
    const value = await invokeUserState<string | null>(
      "user_sessions_manifest_get",
    );
    committedSessionValue = value ? parseSessionStorageValue(value) : null;
    return committedSessionValue;
  });
}

export async function loadSessionMessagePage(
  sessionId: string,
  beforeIndex?: number,
  limit = 100,
): Promise<SessionMessagePage> {
  return invokeUserState<SessionMessagePage>("user_session_messages_page", {
    sessionId,
    beforeIndex,
    limit,
  });
}

export async function searchUserChats(
  query: string,
  limit = 20,
): Promise<ChatSearchResult[]> {
  return invokeUserState<ChatSearchResult[]>("user_chat_search", {
    query,
    limit,
  });
}

function writeSessionStorageValue(
  value: SessionStorageValue,
): void | Promise<void> {
  if (!inTauri()) {
    const serialized = JSON.stringify(value);
    if (localGetItem(SESSIONS_KEY) === serialized) return undefined;
    incrementPerfCounter(`persist.${SESSIONS_KEY}.write`);
    recordPerfMeasure(`persist.${SESSIONS_KEY}.bytes`, serialized.length);
    localSetItem(SESSIONS_KEY, serialized);
    committedSessionValue = value;
    return undefined;
  }
  if (deferredSessionWrite) {
    if (sessionValuesMatch(deferredSessionWrite.value, value)) return undefined;
  } else if (inFlightSessionValue) {
    if (sessionValuesMatch(inFlightSessionValue, value)) return undefined;
  } else if (
    committedSessionValue &&
    sessionValuesMatch(committedSessionValue, value)
  ) {
    return undefined;
  }
  incrementPerfCounter(`persist.${SESSIONS_KEY}.write`);
  return deferSessionWrite(value);
}

function deleteSessionStorageValue(): void | Promise<boolean> {
  cancelDeferredSessionWrite();
  if (!inTauri()) {
    committedSessionValue = null;
    localRemoveItem(SESSIONS_KEY);
    return undefined;
  }
  return (async () => {
    if (sessionFlushPromise) await sessionFlushPromise;
    cancelDeferredSessionWrite();
    committedSessionValue = null;
    return invokeUserState<boolean>("user_sessions_delete");
  })();
}

export function readUserStateKey(
  key: UserStateKey,
): string | null | Promise<string | null> {
  if (key === SESSIONS_KEY) {
    const value = readSessionStorageValue();
    return value instanceof Promise
      ? value.then((session) => (session ? JSON.stringify(session) : null))
      : value
        ? JSON.stringify(value)
        : null;
  }
  if (!inTauri()) {
    const sanitized = sanitizeUserStateValue(key, localGetItem(key));
    return sanitized === null ? null : userStateReadValue(key, sanitized);
  }
  return importLegacyLocalStorageOnce().then(async () => {
    const command = stateGetCommand(key);
    const value = await invokeUserState<string | null>(
      command.command,
      command.args,
    );
    let sanitized = sanitizeUserStateValue(key, value);
    if (value !== null && sanitized !== null && sanitized !== value) {
      const setCommand = stateSetCommand(key, sanitized);
      await invokeUserState<void>(setCommand.command, setCommand.args);
    }
    if (sanitized !== null) lastWrittenValues.set(key, sanitized);
    return sanitized === null ? null : userStateReadValue(key, sanitized);
  });
}

export function writeUserStateKey(
  key: UserStateKey,
  value: string,
): void | Promise<void> {
  const sanitized = sanitizeUserStateValue(key, value) ?? value;
  if (key === SESSIONS_KEY) {
    return writeSessionStorageValue(parseSessionStorageValue(sanitized));
  }
  if (!inTauri()) {
    if (localGetItem(key) === sanitized) return undefined;
    incrementPerfCounter(`persist.${key}.write`);
    recordPerfMeasure(`persist.${key}.bytes`, sanitized.length);
    localSetItem(key, sanitized);
    lastWrittenValues.set(key, sanitized);
    return undefined;
  }
  if (lastWrittenValues.get(key) === sanitized) return undefined;
  incrementPerfCounter(`persist.${key}.write`);
  recordPerfMeasure(`persist.${key}.bytes`, sanitized.length);
  const command = stateSetCommand(key, sanitized);
  const pending = invokeUserState<void>(command.command, command.args).then(() => {
    lastWrittenValues.set(key, sanitized);
  });
  pendingUserStateWrites.add(pending);
  void pending.then(
    () => pendingUserStateWrites.delete(pending),
    () => pendingUserStateWrites.delete(pending),
  );
  return pending;
}

export function deleteUserStateKey(key: UserStateKey): void | Promise<boolean> {
  if (key === SESSIONS_KEY) return deleteSessionStorageValue();
  lastWrittenValues.delete(key);
  if (!inTauri()) {
    localRemoveItem(key);
    return undefined;
  }
  const command = stateDeleteCommand(key);
  return invokeUserState<boolean>(command.command, command.args);
}

export const userStateStorage: StateStorage = {
  getItem: readUserStateKey,
  setItem: writeUserStateKey,
  removeItem: deleteUserStateKey,
};

export const sessionStateStorage: PersistStorage<Record<string, unknown>> = {
  getItem: () => readSessionStorageValue(),
  setItem: (_name, value) => writeSessionStorageValue(value),
  removeItem: () => deleteSessionStorageValue(),
};
