import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { WorkerRunRecord } from "../src/api.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const record: WorkerRunRecord = {
  run: {
    id: "run-1",
    parent_thread_id: "parent-1",
    parent_turn_id: "turn-1",
    policy: "ask",
    runtime: "managed",
    status: "error",
    tasks: [{
      id: "task-1",
      title: "Audit",
      prompt: "Audit the code",
      model: "test-model",
      access: "read_only",
    }],
    error: "worker failed",
    created_at: "2026-07-19T10:00:00Z",
    updated_at: "2026-07-19T10:00:01Z",
    finished_at: "2026-07-19T10:00:01Z",
  },
  workers: [{
    id: "worker-1",
    parent_id: "parent-1",
    root_id: "parent-1",
    title: "Audit",
    status: "error",
    model: "test-model",
    prompt: "Audit the code",
    error: "worker failed",
    created_at: "2026-07-19T10:00:00Z",
    updated_at: "2026-07-19T10:00:01Z",
    finished_at: "2026-07-19T10:00:01Z",
    run_id: "run-1",
    runtime: "managed",
    access: "read_only",
  }],
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  // Finish lazy Markdown compilation before rendering can race Vite teardown.
  await server.ssrLoadModule("/src/components/Markdown.tsx");
  const { WorkersInspector, elapsedLabel, retainedWorkerSelectionKey } = await server.ssrLoadModule("/src/components/WorkersInspector.tsx") as {
    WorkersInspector: ComponentType<Record<string, unknown>>;
    elapsedLabel: (start: string, end?: string | null, now?: number) => string;
    retainedWorkerSelectionKey: (current: string, focus: string, fallback: string, available: ReadonlySet<string>) => string;
  };
  assert(
    elapsedLabel("2026-07-19 10:00:00", null, Date.parse("2026-07-19T10:00:05Z")) === "5s",
    "SQLite UTC timestamps should not inherit the browser timezone",
  );
  assert(
    retainedWorkerSelectionKey("worker:run-1:worker-1", "", "worker:run-1:worker-2", new Set(["worker:run-1:worker-1", "worker:run-1:worker-2"])) === "worker:run-1:worker-1",
    "Worker progress reordering should retain the selected Worker",
  );
  const markup = renderToStaticMarkup(createElement(WorkersInspector, {
    records: [record],
    policy: "ask",
    workerModel: "",
    models: [],
    providers: [],
    agents: [],
    settingsOpen: false,
    onSettingsOpenChange: () => {},
    onPolicyChange: () => {},
    onWorkerModelChange: () => {},
    onStart: () => {},
    onStop: () => {},
    onContinueSolo: () => {},
    onStopWorker: async () => {},
    onRetryWorker: async () => {},
    onDeleteRun: async () => {},
    onLoadDiff: async () => ({ worker_id: "worker-1", diff: "", files: [] }),
    onApplyDiff: async () => ({ ok: true, message: "", stdout: "", stderr: "" }),
    onClose: () => {},
  }));
  assert(markup.includes("Retry</button>"), "failed Workers should offer same-model retry");
  assert(markup.includes("Retry with model"), "failed Workers should offer model-selecting retry");
  assert(markup.includes("Delete run"), "terminal Worker Runs should be deletable");
  assert(
    markup.indexOf("Delete run") < markup.indexOf("worker-task-prompt"),
    "terminal Run actions should stay above the Worker transcript",
  );
} finally {
  await server.close();
}

export {};
