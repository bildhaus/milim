import type {
  ChatMessage,
  ThreadEvent,
  Worker,
  WorkerRunRecord,
} from "../src/api.js";
import {
  appendWorkerRunSynthesisOnce,
  rememberWorkerThreadEvent,
  workerRunReadyForSynthesis,
  workerRunSynthesisId,
} from "../src/lib/workerRuns.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function record(
  runStatus: WorkerRunRecord["run"]["status"],
  statuses: Worker["status"][],
): WorkerRunRecord {
  return {
    run: {
      id: "run-1",
      parent_thread_id: "parent-1",
      policy: "ask",
      runtime: "managed",
      status: runStatus,
      tasks: statuses.map((_, index) => ({
        id: `task-${index}`,
        title: `Task ${index}`,
        prompt: `Work ${index}`,
        model: "test-model",
        access: "read_only",
      })),
      created_at: "2026-07-20T00:00:00Z",
      updated_at: "2026-07-20T00:00:01Z",
    },
    workers: statuses.map((status, index) => ({
      id: `worker-${index}`,
      parent_id: "parent-1",
      root_id: "parent-1",
      title: `Task ${index}`,
      status,
      model: "test-model",
      prompt: `Work ${index}`,
      created_at: "2026-07-20T00:00:00Z",
      updated_at: "2026-07-20T00:00:01Z",
      run_id: "run-1",
      runtime: "managed",
      access: "read_only",
    })),
  };
}

assert(
  !workerRunReadyForSynthesis(record("done", ["running"])),
  "a terminal Run must not synthesize a stale running Worker",
);
assert(
  !workerRunReadyForSynthesis({
    ...record("done", ["done", "done"]),
    workers: [record("done", ["done"]).workers[0]],
  }),
  "a Run must not synthesize before every planned Worker is present",
);
assert(
  workerRunReadyForSynthesis({ ...record("error", ["error"]), workers: [] }),
  "a terminal spawn failure must return to the parent without a Worker row",
);
assert(
  workerRunReadyForSynthesis(record("done", ["done", "done", "done", "done"])),
  "four completed Workers should be ready for one synthesis",
);
assert(
  workerRunReadyForSynthesis(record("error", ["error"])),
  "a terminal failure should still resume the parent with visible evidence",
);
assert(
  workerRunSynthesisId({
    role: "system",
    content: "Worker Run legacy-run finished with status done.\n\nResult",
  }) === "legacy-run",
  "legacy synthesis messages should remain hidden and deduplicated",
);
assert(
  workerRunSynthesisId({
    role: "system",
    workerRunId: "current-run",
    content: "Worker results",
  }) === "current-run",
  "current synthesis messages should use their explicit Run id",
);

const synthesis = {
  role: "system",
  workerRunId: "run-1",
  content: "Worker results",
} satisfies ChatMessage;
const firstSynthesis = appendWorkerRunSynthesisOnce([], synthesis);
assert(firstSynthesis, "the first terminal event should claim parent synthesis");
assert(
  appendWorkerRunSynthesisOnce(firstSynthesis, synthesis) === null,
  "a duplicate terminal event must not synthesize the parent again",
);

const replayedEvents = new Map<string, ThreadEvent[]>();
for (let seq = 1; seq <= 3; seq += 1) {
  rememberWorkerThreadEvent(replayedEvents, {
    id: `event-${seq}`,
    thread_id: "terminal-child",
    seq,
    kind: "token",
    payload: { text: String(seq) },
    created_at: `2026-07-20T00:00:0${seq}Z`,
  });
}
assert(
  replayedEvents.get("terminal-child")?.length === 3,
  "backfilled events for a terminal child must accumulate across replay frames",
);
rememberWorkerThreadEvent(replayedEvents, {
  id: "event-2-replayed",
  thread_id: "terminal-child",
  seq: 2,
  kind: "token",
  payload: { text: "duplicate" },
  created_at: "2026-07-20T00:00:02Z",
});
assert(
  replayedEvents.get("terminal-child")?.length === 3,
  "replayed child events must deduplicate by sequence",
);
