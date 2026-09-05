import { createControlCommandId, sendControlCommand, type ChatMessage } from "../api.js";
import { flushDeferredUserStateWrites, loadSessionSnapshot } from "../persistence/userStateStorage.js";
import { useSessions, type Session, type ThreadSettingsPatch } from "../sessions/store.js";
import { requestWorkspaceEditorLeave } from "./workspaceEditorGuard.js";

export type BranchBoundary = { messageId: string } | { empty: true };

export function branchBoundary(message: ChatMessage | undefined): BranchBoundary {
  if (!message) return { empty: true };
  const messageId = message.canonicalId?.trim() || message.id?.trim();
  if (!messageId) throw new Error("This message has no stable ID. Reload the thread before branching.");
  return { messageId };
}

export function canonicalMessageIndex(messages: readonly ChatMessage[], message: ChatMessage): number {
  const boundary = branchBoundary(message);
  if (!("messageId" in boundary)) return -1;
  return messages.findIndex((item) => (item.canonicalId?.trim() || item.id?.trim()) === boundary.messageId);
}

export async function completeSessionForExport(sessionId: string): Promise<Session> {
  await flushDeferredUserStateWrites("milim.sessions");
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    return loadSessionSnapshot(sessionId);
  }
  const session = useSessions.getState().sessions.find((item) => item.id === sessionId);
  if (!session || session.messagesHydrated === false) throw new Error("The complete thread history is unavailable. Reload the thread and retry.");
  return session;
}

export async function branchCanonicalSession(
  sourceId: string,
  boundary?: BranchBoundary,
  settings?: ThreadSettingsPatch,
  activateIfId = useSessions.getState().activeId,
): Promise<string | null> {
  if (!(await requestWorkspaceEditorLeave("navigate"))) return null;
  await flushDeferredUserStateWrites("milim.sessions");
  const id = crypto.randomUUID();
  const command = {
    command_id: createControlCommandId(),
    kind: "thread.create" as const,
    thread_id: id,
    payload: {
      id,
      source_thread_id: sourceId,
      ...(boundary && "messageId" in boundary ? { through_message_id: boundary.messageId } : {}),
      ...(boundary && "empty" in boundary ? { source_message_count: 0 } : {}),
      ...(settings ? { settings } : {}),
    },
  };
  const result = await sendControlCommand(command).catch(() => sendControlCommand(command));
  if (result.status !== "applied") throw new Error(result.message || `Thread branch returned ${result.status}.`);
  const data = result.data as { session?: Session } | null;
  const session = data?.session ?? await loadSessionSnapshot(id);
  if (session.id !== id || !Array.isArray(session.messages)) throw new Error("The created branch could not be loaded. Reopen it from the sidebar.");
  useSessions.getState().installCanonicalSession(session, activateIfId);
  return id;
}
