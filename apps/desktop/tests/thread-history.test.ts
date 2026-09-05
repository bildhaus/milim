import { deepEqual, equal, rejects } from "node:assert/strict";
import type { Session } from "../src/sessions/store.js";

const { useSessions } = await import("../src/sessions/store.js");
const { branchBoundary, branchCanonicalSession, canonicalMessageIndex, completeSessionForExport } = await import("../src/lib/threadHistory.js");
const { sessionExportPayload, sessionMarkdownExport } = await import("../src/lib/threadExport.js");
const full: Session = {
  id: "history-source", title: "Full history", createdAt: 1, updatedAt: 2,
  messages: Array.from({ length: 350 }, (_, index) => ({ id: `canonical-${index}`, role: index % 2 ? "assistant" : "user", content: `message ${index}` })),
  virtualFiles: { "notes.txt": { path: "notes.txt", content: "workspace context", updatedAt: 2, bytes: 17, version: 1 } },
};
useSessions.setState({ sessions: [{ ...full, messages: full.messages.slice(250), messagesHydrated: false, messagesLoadedFrom: 250, persistedMessageCount: 350 }], activeId: full.id });
const selected = { ...full.messages[270], id: "stable-render-row", canonicalId: "canonical-270" };
deepEqual(branchBoundary(selected), { messageId: "canonical-270" });
equal(canonicalMessageIndex(full.messages, selected), 270);

let snapshotCalls = 0;
Object.defineProperty(globalThis, "__MILIM_TEST_INVOKE__", { configurable: true, value: async (command: string) => {
  if (command === "user_session_snapshot") { snapshotCalls += 1; return full; }
  return null;
} });
Object.defineProperty(globalThis, "window", { configurable: true, value: { __TAURI_INTERNALS__: {}, dispatchEvent() {}, addEventListener() {}, removeEventListener() {}, setTimeout, clearTimeout } });
const exported = await completeSessionForExport(full.id);
equal(snapshotCalls, 1, "export must load one atomic snapshot instead of paging the resident tail");
equal(sessionExportPayload(exported).session.messages.length, 350);
equal((sessionMarkdownExport(exported).match(/<!-- milim-message:/g) ?? []).length, 350);
deepEqual(exported.virtualFiles, full.virtualFiles);

const requests: Array<{ command_id: string; payload: Record<string, unknown> }> = [];
let resolveResponse: (() => void) | undefined;
let called: (() => void) | undefined;
const requestStarted = new Promise<void>((resolve) => { called = resolve; });
globalThis.fetch = (async (_url: unknown, init?: RequestInit) => {
  const request = JSON.parse(String(init?.body));
  requests.push(request);
  called?.();
  await new Promise<void>((resolve) => { resolveResponse = resolve; });
  const session = { ...full, id: request.payload.id, parentId: full.id, messages: full.messages.slice(0, 271).map((message, index) => ({ ...message, id: `clone-${index}` })) };
  return Response.json({ command_id: request.command_id, status: "applied", thread_id: session.id, data: { session } });
}) as typeof fetch;
const branchPromise = branchCanonicalSession(full.id, branchBoundary(selected));
await requestStarted;
useSessions.setState({ activeId: "user-navigated-away" });
resolveResponse?.();
const created = await branchPromise;
equal(requests[0].payload.through_message_id, "canonical-270");
equal(requests[0].payload.source_thread_id, full.id);
equal("messages" in requests[0].payload, false, "canonical branching must not trust a renderer history array");
equal(useSessions.getState().activeId, "user-navigated-away", "late branch creation must not steal navigation");
const branch = useSessions.getState().sessions.find((session) => session.id === created)!;
equal(branch.messages.length, 271, "the full prefix must include messages before the loaded tail");
deepEqual(branch.virtualFiles, full.virtualFiles);
equal(branch.messagesHydrated, true);

globalThis.fetch = (async () => Response.json({ status: "failed", message: "boundary deleted" })) as typeof fetch;
const before = useSessions.getState().sessions.length;
await rejects(branchCanonicalSession(full.id, { messageId: "deleted" }), /boundary deleted/);
equal(useSessions.getState().sessions.length, before, "failed canonical branching must not create an optimistic replica");

delete (globalThis as { window?: unknown }).window;
await rejects(completeSessionForExport(full.id), /complete thread history is unavailable/, "non-native fallback must not silently export partial history");
