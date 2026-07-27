import type { ClaudeImportedThread } from "../api";
import type { Session } from "../sessions/store";

export function importedClaudeSessionId(
  sessions: readonly Session[],
  sessionId: string,
): string | null {
  return sessions.find((session) => session.accountRuntime?.claudeSessionId === sessionId)?.id ?? null;
}

export function importedClaudeSession(thread: ClaudeImportedThread) {
  return {
    title: thread.title,
    messages: thread.messages,
    settings: { model: "", folder: thread.resumable ? thread.cwd ?? "" : "" },
  };
}
