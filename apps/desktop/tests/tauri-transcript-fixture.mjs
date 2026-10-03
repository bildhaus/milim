// Bulk data loading for the isolated native benchmark. Thread lifecycle stays
// on /control/v1. Replica deltas may silently skip stale message updates, so a
// successful invoke must be followed by a native snapshot acknowledgement.
export async function seedTranscriptFixture({ activeId, threadCount, messagesPerThread }) {
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  if (!invoke) throw new Error("Tauri invoke API unavailable.");
  let lastCounts = [];
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const key = "milim.sessions";
    const raw = await invoke("user_state_get", { key });
    if (!raw) throw new Error("Canonical session state is unavailable.");
    const parsed = JSON.parse(raw);
    const state =
      parsed.state && typeof parsed.state === "object" ? parsed.state : {};
    const previousIds = new Set(
      (state.sessions ?? []).map((session) => session.id),
    );
    const previousMessageCounts = new Map(
      (state.sessions ?? []).map((session) => [
        session.id,
        session.messagesHydrated === false
          ? (session.persistedMessageCount ?? session.messages?.length ?? 0)
          : (session.messages?.length ?? 0),
      ]),
    );
    const canonical = state.sessions?.find(
      (session) => session.id === activeId,
    );
    if (!canonical)
      throw new Error("Canonical active session is unavailable.");
    if ((canonical.messages?.length ?? 0) > messagesPerThread) {
      throw new Error(
        `Canonical session already exceeds ${messagesPerThread} messages.`,
      );
    }

    const now = Date.now();
    const canonicalMessages = [...(canonical.messages ?? [])];
    while (canonicalMessages.length < messagesPerThread) {
      const index = canonicalMessages.length;
      canonicalMessages.push({
        id: `canonical-fill-${index}`,
        role: index % 2 === 0 ? "user" : "assistant",
        content: `Canonical transcript filler ${index + 1}.`,
      });
    }
    const sessions = [
      {
        ...canonical,
        messages: canonicalMessages,
        updatedAt: now,
      },
    ];
    for (let threadIndex = 1; threadIndex < threadCount; threadIndex += 1) {
      sessions.push({
        id: `canonical-fixture-${threadIndex}`,
        title: `Canonical fixture ${threadIndex}`,
        messages: Array.from(
          { length: messagesPerThread },
          (_, messageIndex) => ({
            id: `canonical-${threadIndex}-${messageIndex}`,
            role: messageIndex % 2 === 0 ? "user" : "assistant",
            content: `Fixture ${threadIndex} message ${messageIndex + 1}.`,
          }),
        ),
        settings: { ...(canonical.settings ?? {}) },
        createdAt: now - threadIndex,
        updatedAt: now - threadIndex,
      });
    }
    state.sessions = sessions;
    state.activeId = activeId;
    state.queuedMessagesBySession = {};
    state.sidebar = {
      ...(state.sidebar ?? {}),
      sessionOrder: sessions.map((session) => session.id),
    };
    parsed.state = state;
    const nextIds = new Set(sessions.map((session) => session.id));
    const meta = structuredClone(parsed);
    delete meta.state.sessions;
    await invoke("user_sessions_apply_ops", {
      delta: {
        metaJson: JSON.stringify(meta),
        sessionOrder: sessions.map((session) => session.id),
        upserts: sessions.map((session) => {
          const { messages, messagesHydrated, messagesLoadedFrom, persistedMessageCount, ...sessionMeta } = session;
          return {
            id: session.id,
            sessionJson: JSON.stringify(sessionMeta),
            baseMessageCount: previousMessageCounts.get(session.id) ?? 0,
            messageCount: messages.length,
            messages: messages.map((message, index) => ({
              index,
              messageJson: JSON.stringify(message),
            })),
          };
        }),
        deletedSessionIds: [...previousIds].filter((id) => !nextIds.has(id)),
      },
    });
    const snapshots = await Promise.all(sessions.map((session) =>
      invoke("user_session_snapshot", { sessionId: session.id }),
    ));
    if (snapshots.every((snapshot, index) =>
      snapshot.messages?.length === messagesPerThread &&
      snapshot.messages.every((message, messageIndex) =>
        message.id === sessions[index].messages[messageIndex].id &&
        message.content === sessions[index].messages[messageIndex].content),
    )) return { attempts: attempt + 1 };
    lastCounts = snapshots.map((snapshot) => snapshot.messages?.length ?? 0);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Transcript fixture was not accepted after 20 attempts; persisted counts: ${lastCounts.join(", ")}`);
}
