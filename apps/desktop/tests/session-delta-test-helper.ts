type SessionDelta = {
  id: string;
  sessionJson?: string;
  messageCount: number;
  preserveMessages?: boolean;
  messages: Array<{ index: number; messageJson: string }>;
};
type PersistedSession = {
  id: string;
  messages?: unknown[];
  [key: string]: unknown;
};

export function applySessionDeltaSnapshot(
  snapshot: string,
  delta: {
    metaJson: string;
    sessionOrder: string[];
    upserts: SessionDelta[];
    deletedSessionIds: string[];
  },
): string {
  const current = JSON.parse(snapshot);
  const sessions = new Map<string, PersistedSession>(
    (current.state.sessions ?? []).map((session: PersistedSession) => [
      session.id,
      session,
    ]),
  );
  for (const id of delta.deletedSessionIds) sessions.delete(id);
  for (const change of delta.upserts) {
    const existing = sessions.get(change.id) ?? { id: change.id, messages: [] };
    const row = change.sessionJson
      ? { ...JSON.parse(change.sessionJson), messages: existing.messages ?? [] }
      : { ...existing };
    const messages = [...(row.messages ?? [])];
    if (!change.preserveMessages) {
      messages.length = change.messageCount;
      for (const message of change.messages) {
        messages[message.index] = JSON.parse(message.messageJson);
      }
    }
    sessions.set(change.id, { ...row, messages });
  }
  const meta = JSON.parse(delta.metaJson);
  meta.state.sessions = delta.sessionOrder.map((id) => sessions.get(id));
  return JSON.stringify(meta);
}
