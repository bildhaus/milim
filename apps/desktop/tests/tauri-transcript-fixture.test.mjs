import assert from "node:assert/strict";
import { test } from "node:test";
import { runInNewContext } from "node:vm";
import { seedTranscriptFixture } from "./tauri-transcript-fixture.mjs";

function fixtureStore({ staleFirstWrite = false, rejectWrites = false } = {}) {
  const original = { id: "canonical-user", role: "user", content: "Keep this turn." };
  const concurrent = { id: "canonical-reply", role: "assistant", content: "Keep this reply." };
  const sessions = new Map([
    ["active", { id: "active", messages: [original], settings: { model: "perf-a" } }],
    ["canonical-fixture-1", { id: "canonical-fixture-1", messages: [] }],
  ]);
  const writes = [];
  async function invoke(command, args) {
    if (command === "user_state_get") {
      return JSON.stringify({ state: { activeId: "active", sessions: [...sessions.values()] } });
    }
    if (command === "user_session_snapshot") return structuredClone(sessions.get(args.sessionId));
    assert.equal(command, "user_sessions_apply_ops");
    const { delta } = args;
    writes.push(delta);
    if (staleFirstWrite && writes.length === 1) sessions.get("active").messages.push(concurrent);
    for (const update of delta.upserts) {
      const current = sessions.get(update.id);
      // Mirror storage's silent rejection when a canonical commit changed the
      // message count after the renderer read its base snapshot.
      if (rejectWrites || current.messages.length !== update.baseMessageCount) continue;
      sessions.set(update.id, {
        ...JSON.parse(update.sessionJson),
        messages: update.messages.map((row) => JSON.parse(row.messageJson)),
      });
    }
  }
  const seed = runInNewContext(`(${seedTranscriptFixture.toString()})`, {
    window: { __TAURI_INTERNALS__: { invoke } },
    structuredClone,
    setTimeout: (callback) => setImmediate(callback),
  });
  return { seed, sessions, writes, original, concurrent };
}

test("native transcript fixture verifies persisted rows and preserves canonical messages", async () => {
  const store = fixtureStore();
  const result = await store.seed({ activeId: "active", threadCount: 2, messagesPerThread: 100 });
  assert.equal(result.attempts, 1);
  assert.deepEqual(store.sessions.get("active").messages[0], store.original);
  assert.equal(store.sessions.get("active").messages.length, 100);
  assert.equal(store.sessions.get("canonical-fixture-1").messages.length, 100);
});

test("a silently skipped active upsert retries with fresh base counts", async () => {
  const store = fixtureStore({ staleFirstWrite: true });
  const result = await store.seed({ activeId: "active", threadCount: 2, messagesPerThread: 100 });
  assert.equal(result.attempts, 2);
  assert.deepEqual(Array.from(store.writes[0].upserts, (row) => row.baseMessageCount), [1, 0]);
  assert.deepEqual(Array.from(store.writes[1].upserts, (row) => row.baseMessageCount), [2, 100]);
  assert.deepEqual(Array.from(store.sessions.get("active").messages).slice(0, 2), [store.original, store.concurrent]);
  assert.equal(store.sessions.get("active").messages.length, 100);
});

test("unaccepted fixture data fails after bounded attempts instead of reporting success", async () => {
  const store = fixtureStore({ rejectWrites: true });
  await assert.rejects(
    store.seed({ activeId: "active", threadCount: 2, messagesPerThread: 100 }),
    /not accepted after 20 attempts; persisted counts: 1, 0/,
  );
  assert.equal(store.writes.length, 20);
});
