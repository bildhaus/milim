import type { ChatMessage, ThreadEvent, WorkerRunRecord } from "../api";

const TERMINAL_RUN_STATUSES = new Set(["done", "partial", "stopped", "error"]);
const TERMINAL_WORKER_STATUSES = new Set(["done", "stopped", "error"]);

export function workerRunReadyForSynthesis(record: WorkerRunRecord): boolean {
  if (
    !TERMINAL_RUN_STATUSES.has(record.run.status) ||
    record.workers.some(
      (worker) => !TERMINAL_WORKER_STATUSES.has(worker.status),
    )
  )
    return false;
  return (
    record.workers.length === record.run.tasks.length ||
    record.run.status === "stopped" ||
    record.run.status === "error"
  );
}

export function workerRunSynthesisId(message: ChatMessage): string | null {
  if (message.role !== "system") return null;
  return (
    message.workerRunId?.trim() ||
    /^Worker Run (\S+) finished with status\b/.exec(message.content)?.[1] ||
    null
  );
}

export function appendWorkerRunSynthesisOnce(
  messages: readonly ChatMessage[],
  synthesis: ChatMessage,
): ChatMessage[] | null {
  const runId = workerRunSynthesisId(synthesis);
  if (
    !runId ||
    messages.some((message) => workerRunSynthesisId(message) === runId)
  )
    return null;
  return [...messages, synthesis];
}

export function rememberWorkerThreadEvent(
  eventsByThread: Map<string, ThreadEvent[]>,
  event: ThreadEvent,
): ThreadEvent[] {
  const existing = eventsByThread.get(event.thread_id) ?? [];
  if (
    existing.some((item) => item.id === event.id || item.seq === event.seq)
  )
    return existing;
  const next = [...existing, event];
  eventsByThread.set(event.thread_id, next);
  return next;
}
