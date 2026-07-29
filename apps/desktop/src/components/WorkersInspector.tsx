import {
  lazy,
  Suspense,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import type {
  Agent,
  DelegationPolicy,
  ModelInfo,
  ProviderInfo,
  Worker,
  WorkerDiffReview,
  WorkerPlanTask,
  WorkerRunStatus,
  WorkspaceGitActionResult,
} from "../api";
import { modelDisplayName } from "../lib/modelPicker";
import type { SessionWorkerRunRecord } from "../sessions/store";
import { AgentAvatar } from "./AgentAvatar";
import { ArrowRight, Check, ChevronDown, Copy, Gear, Refresh, Sidebar, Square, Trash } from "./icons";
import { ModelPicker } from "./ModelPicker";

const Markdown = lazy(() =>
  import("./Markdown").then((mod) => ({ default: mod.Markdown })),
);

const STATUS_LABEL: Record<WorkerRunStatus, string> = {
  proposed: "Needs approval",
  running: "Running",
  done: "Done",
  partial: "Partial",
  stopped: "Stopped",
  error: "Error",
};

const POLICY_DESCRIPTION: Record<DelegationPolicy, string> = {
  off: "The parent handles every task itself.",
  ask: "Review a worker plan before it runs, unless approval is Open.",
  auto: "Run independent tasks automatically.",
};

function workerTimestamp(value: string) {
  return Date.parse(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)?$/.test(value)
    ? `${value.replace(" ", "T")}Z`
    : value);
}

export function elapsedLabel(start: string, end?: string | null, now = Date.now()) {
  const startedAt = workerTimestamp(start);
  const endedAt = end ? workerTimestamp(end) : now;
  if (!Number.isFinite(startedAt) || !Number.isFinite(endedAt)) return "";
  const seconds = Math.max(0, Math.round((endedAt - startedAt) / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

function ageLabel(value: string, now = Date.now()) {
  const timestamp = workerTimestamp(value);
  if (!Number.isFinite(timestamp)) return "";
  const seconds = Math.max(0, Math.floor((now - timestamp) / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}

export function retainedWorkerSelectionKey(
  current: string,
  focus: string,
  fallback: string,
  available: ReadonlySet<string>,
) {
  return available.has(current) ? current : (focus || fallback);
}

function taskForWorker(tasks: WorkerPlanTask[], worker: Worker) {
  return tasks.find(
    (task) =>
      task.id === worker.id ||
      task.prompt === worker.prompt ||
      task.title === worker.title,
  );
}

function workerResult(worker: Worker) {
  return worker.summary?.trim() || worker.error?.trim() || "No result yet.";
}

function WorkerMarkdown({ content }: { content: string }) {
  return (
    <Suspense fallback={<div className="worker-markdown-fallback">{content}</div>}>
      <Markdown content={content} collapseArtifacts={false} />
    </Suspense>
  );
}

function resultPreview(worker: Worker) {
  return workerResult(worker).replace(/\s+/g, " ");
}

function workerModelLabel(workerModel: string, models: ModelInfo[]) {
  if (!workerModel) return "Inherit parent";
  const selected = models.find((model) => model.id === workerModel);
  return selected ? modelDisplayName(selected) : workerModel;
}

function WorkerAvatar({
  agent,
  runId,
  identityId,
  className = "worker-agent-avatar",
}: {
  agent?: Agent;
  runId: string;
  identityId: string;
  className?: string;
}) {
  return agent ? (
    <AgentAvatar id={agent.id} name={agent.name} avatar={agent.avatar} className={className} />
  ) : (
    <AgentAvatar avatar={`worker:${runId}:${identityId}`} className={className} />
  );
}

function workerCounts(records: readonly SessionWorkerRunRecord[]) {
  const proposed = records
    .filter((record) => record.run.status === "proposed")
    .reduce((total, record) => total + record.run.tasks.length, 0);
  const workers = records.flatMap((record) => record.workers);
  const active = workers.filter(
    (worker) => worker.status === "queued" || worker.status === "running",
  ).length;
  return { proposed, active, done: workers.length - active };
}

export function WorkersSummary({
  records,
  policy,
  workerModel,
  models,
  agents,
  onOpen,
  onOpenSettings,
}: {
  records: readonly SessionWorkerRunRecord[];
  policy: DelegationPolicy;
  workerModel: string;
  models: ModelInfo[];
  agents: Agent[];
  onOpen: () => void;
  onOpenSettings: () => void;
}) {
  const counts = workerCounts(records);
  const status = [
    counts.proposed ? `${counts.proposed} planned` : "",
    counts.active ? `${counts.active} active` : "",
    counts.done ? `${counts.done} done` : "",
  ].filter(Boolean).join(" · ") || "No runs yet";
  const identities = records.flatMap((record) => {
    if (record.workers.length) {
      return record.workers.map((worker) => {
        const task = taskForWorker(record.run.tasks, worker);
        return {
          agent: agents.find((agent) => agent.id === task?.agent_id),
          runId: record.run.id,
          identityId: task?.id ?? worker.id,
        };
      });
    }
    return record.run.status === "proposed"
      ? record.run.tasks.map((task) => ({
          agent: agents.find((agent) => agent.id === task.agent_id),
          runId: record.run.id,
          identityId: task.id,
        }))
      : [];
  }).slice(0, 4);

  return (
    <div className="workers-summary" data-testid="workers-summary">
      <button className="workers-summary-main" type="button" onClick={onOpen}>
        <span className="workers-summary-avatars" aria-hidden="true">
          {identities.map((identity) => (
            <WorkerAvatar
              key={`${identity.runId}:${identity.identityId}`}
              {...identity}
              className="workers-summary-avatar"
            />
          ))}
        </span>
        <span className="workers-summary-copy">
          <strong>Workers</strong>
          <small>{status}</small>
          <em>{policy[0].toUpperCase() + policy.slice(1)} · {workerModelLabel(workerModel, models)}</em>
        </span>
        <ArrowRight size={12} aria-hidden="true" />
      </button>
      <button
        className="preview-action"
        type="button"
        data-testid="workers-settings-toggle"
        title="Worker settings"
        aria-label="Open Worker settings"
        onClick={onOpenSettings}
      >
        <Gear size={12} />
      </button>
    </div>
  );
}

export function WorkersInspector({
  records,
  focusRunId,
  policy,
  workerModel,
  models,
  providers,
  agents,
  busy = false,
  settingsOpen,
  closing = false,
  noEnterMotion = false,
  modeSwitcher,
  onSettingsOpenChange,
  onPolicyChange,
  onWorkerModelChange,
  onStart,
  onStop,
  onContinueSolo,
  onStopWorker,
  onRetryWorker,
  onDeleteRun,
  onLoadDiff,
  onApplyDiff,
  onClose,
}: {
  records: readonly SessionWorkerRunRecord[];
  focusRunId?: string;
  policy: DelegationPolicy;
  workerModel: string;
  models: ModelInfo[];
  providers?: ProviderInfo[];
  agents: Agent[];
  busy?: boolean;
  settingsOpen: boolean;
  closing?: boolean;
  noEnterMotion?: boolean;
  modeSwitcher?: ReactNode;
  onSettingsOpenChange: (open: boolean) => void;
  onPolicyChange: (policy: DelegationPolicy) => void;
  onWorkerModelChange: (model: string) => void;
  onStart: (runId: string) => void;
  onStop: (runId: string) => void;
  onContinueSolo: (runId: string) => void;
  onStopWorker: (runId: string, workerId: string) => Promise<void>;
  onRetryWorker: (runId: string, taskId: string, model?: string) => Promise<void>;
  onDeleteRun: (runId: string) => Promise<void>;
  onLoadDiff: (runId: string, workerId: string) => Promise<WorkerDiffReview>;
  onApplyDiff: (runId: string, workerId: string) => Promise<WorkspaceGitActionResult>;
  onClose: () => void;
}) {
  const [selectedKey, setSelectedKey] = useState("");
  const [visibleDone, setVisibleDone] = useState(10);
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const [diffReview, setDiffReview] = useState<WorkerDiffReview>();
  const [reviewBusy, setReviewBusy] = useState(false);
  const [reviewNotice, setReviewNotice] = useState("");
  const [confirmDeleteRunId, setConfirmDeleteRunId] = useState("");
  const [modelPickerPurpose, setModelPickerPurpose] = useState<"settings" | "retry" | null>(null);
  const [modelPickerStyle, setModelPickerStyle] = useState<CSSProperties>();
  const modelPickerRef = useRef<HTMLDivElement>(null);
  const retryModelPickerRef = useRef<HTMLDivElement>(null);
  const modelPickerMenuRef = useRef<HTMLDivElement>(null);
  const modelPickerOpen = modelPickerPurpose !== null;
  const proposedRecords = records.filter((record) => record.run.status === "proposed");
  const entries = records.flatMap((record) =>
    record.workers.map((worker) => ({ record, worker })),
  );
  const activeEntries = entries
    .filter(({ worker }) => worker.status === "queued" || worker.status === "running")
    .sort((a, b) =>
      workerTimestamp(a.worker.created_at) - workerTimestamp(b.worker.created_at) ||
      a.worker.id.localeCompare(b.worker.id),
    );
  const doneEntries = entries
    .filter(({ worker }) => worker.status !== "queued" && worker.status !== "running")
    .sort((a, b) => workerTimestamp(b.worker.updated_at) - workerTimestamp(a.worker.updated_at));
  const availableKeys = new Set([
    ...proposedRecords.map((record) => `run:${record.run.id}`),
    ...entries.map(({ record, worker }) => `worker:${record.run.id}:${worker.id}`),
  ]);
  const focusedRecord = records.find((record) => record.run.id === focusRunId);
  const focusKey = focusedRecord
    ? focusedRecord.run.status === "proposed"
      ? `run:${focusedRecord.run.id}`
      : focusedRecord.workers[0]
        ? `worker:${focusedRecord.run.id}:${focusedRecord.workers[0].id}`
        : ""
    : "";
  const fallbackKey =
    proposedRecords[0]
      ? `run:${proposedRecords[0].run.id}`
      : activeEntries[0]
        ? `worker:${activeEntries[0].record.run.id}:${activeEntries[0].worker.id}`
        : doneEntries[0]
          ? `worker:${doneEntries[0].record.run.id}:${doneEntries[0].worker.id}`
          : "";
  const effectiveKey = availableKeys.has(selectedKey) ? selectedKey : (focusKey || fallbackKey);
  const selectedPlan = proposedRecords.find((record) => `run:${record.run.id}` === effectiveKey);
  const selectedEntry = entries.find(
    ({ record, worker }) => `worker:${record.run.id}:${worker.id}` === effectiveKey,
  );
  const selectedRecord = selectedPlan ?? selectedEntry?.record;
  const selectedWorker = selectedEntry?.worker;
  const selectedTask = selectedWorker && selectedRecord
    ? taskForWorker(selectedRecord.run.tasks, selectedWorker)
    : undefined;
  const selectedAgent = agents.find((agent) => agent.id === selectedTask?.agent_id);
  const runningRecord = records.find((record) => record.run.status === "running");
  const counts = workerCounts(records);

  useEffect(() => {
    if (focusKey) setSelectedKey(focusKey);
  }, [focusRunId]);

  useEffect(() => {
    setSelectedKey((current) =>
      retainedWorkerSelectionKey(current, focusKey, fallbackKey, availableKeys),
    );
  }, [effectiveKey]);

  useEffect(() => {
    if (!runningRecord) return;
    const interval = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [runningRecord?.run.id]);

  useEffect(() => {
    setDiffReview(undefined);
    setReviewNotice("");
    setConfirmDeleteRunId("");
  }, [selectedRecord?.run.id, selectedWorker?.id]);

  useEffect(() => {
    if (!modelPickerOpen) return;
    const onDocumentMouseDown = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Element && target.closest(".mp-effort-menu")) return;
      if (
        target instanceof Node &&
        !modelPickerRef.current?.contains(target) &&
        !retryModelPickerRef.current?.contains(target) &&
        !modelPickerMenuRef.current?.contains(target)
      ) setModelPickerPurpose(null);
    };
    document.addEventListener("mousedown", onDocumentMouseDown);
    return () => document.removeEventListener("mousedown", onDocumentMouseDown);
  }, [modelPickerOpen]);

  function toggleModelPicker(button: HTMLButtonElement, purpose: "settings" | "retry") {
    if (modelPickerPurpose === purpose) {
      setModelPickerPurpose(null);
      return;
    }
    const edge = 8;
    const gap = 6;
    const rect = button.getBoundingClientRect();
    const width = Math.min(440, window.innerWidth - edge * 2);
    const below = window.innerHeight - rect.bottom - edge - gap;
    const above = rect.top - edge - gap;
    const openBelow = below >= Math.min(320, above);
    const maxHeight = Math.max(160, Math.min(440, openBelow ? below : above));
    setModelPickerStyle({
      position: "fixed",
      left: Math.max(edge, Math.min(window.innerWidth - width - edge, rect.right - width)),
      top: openBelow ? rect.bottom + gap : rect.top - gap - maxHeight,
      width,
      maxHeight,
    });
    setModelPickerPurpose(purpose);
  }

  async function copyResult() {
    if (!selectedWorker || !navigator.clipboard) return;
    await navigator.clipboard.writeText(workerResult(selectedWorker));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }

  async function loadDiff() {
    if (!selectedRecord || !selectedWorker) return;
    setReviewBusy(true);
    setReviewNotice("");
    try {
      setDiffReview(await onLoadDiff(selectedRecord.run.id, selectedWorker.id));
    } catch (error) {
      setReviewNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setReviewBusy(false);
    }
  }

  async function applyDiff() {
    if (
      !selectedRecord ||
      !selectedWorker ||
      !window.confirm("Apply this worker diff to the active workspace?")
    ) return;
    setReviewBusy(true);
    try {
      const result = await onApplyDiff(selectedRecord.run.id, selectedWorker.id);
      setReviewNotice(result.message);
    } catch (error) {
      setReviewNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setReviewBusy(false);
    }
  }

  function renderHistoryRow(record: SessionWorkerRunRecord, worker: Worker) {
    const task = taskForWorker(record.run.tasks, worker);
    const taskAgent = agents.find((agent) => agent.id === task?.agent_id);
    const key = `worker:${record.run.id}:${worker.id}`;
    return (
      <button
        key={key}
        type="button"
        role="listitem"
        className={key === effectiveKey ? "active" : ""}
        onClick={() => setSelectedKey(key)}
      >
        <span className={`workers-dot ${worker.status}`} />
        <WorkerAvatar agent={taskAgent} runId={record.run.id} identityId={task?.id ?? worker.id} />
        <span className="worker-history-copy">
          <strong>{task?.title || worker.title || "Worker"}</strong>
          <small>{resultPreview(worker)}</small>
        </span>
        <span className="worker-history-meta">
          <time>{ageLabel(worker.updated_at, now)}</time>
          <em>{worker.status}</em>
        </span>
      </button>
    );
  }

  return (
    <aside
      id="inspector-panel-workers"
      className={`preview-panel workers-inspector-panel${closing ? " closing" : ""}${noEnterMotion ? " no-enter" : ""}`}
      data-testid="workers-inspector"
      aria-label="Workers inspector"
      aria-labelledby="inspector-tab-workers"
      role="tabpanel"
    >
      <div className="preview-toolbar workers-inspector-toolbar">
        {modeSwitcher}
        <div className="preview-actions" aria-label="Worker panel actions">
          <button
            className="preview-action"
            type="button"
            data-testid="workers-inspector-settings-toggle"
            title="Worker settings"
            aria-label="Worker settings"
            aria-expanded={settingsOpen}
            onClick={() => onSettingsOpenChange(!settingsOpen)}
          >
            <Gear size={13} />
          </button>
          {runningRecord && (
            <button
              className="preview-action"
              type="button"
              disabled={busy}
              title="Stop all workers"
              aria-label="Stop all workers"
              onClick={() => onStop(runningRecord.run.id)}
            >
              <Square size={12} />
            </button>
          )}
          <button className="preview-action" type="button" title="Close inspector" aria-label="Close inspector" onClick={onClose}>
            <Sidebar size={16} />
          </button>
        </div>
      </div>

      {settingsOpen && (
        <div className="workers-controls" id="workers-settings">
          <div className="workers-control">
            <div className="workers-control-label">
              <span>Delegation</span>
              <small>{POLICY_DESCRIPTION[policy]}</small>
            </div>
            <div className="workers-policy" role="group" aria-label="Delegation policy">
              {(["off", "ask", "auto"] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  className={policy === value ? "active" : ""}
                  aria-pressed={policy === value}
                  onClick={() => onPolicyChange(value)}
                >
                  {value[0].toUpperCase() + value.slice(1)}
                </button>
              ))}
            </div>
          </div>
          <div className="workers-model">
            <span id="worker-model-label">Worker model</span>
            <div className="workers-model-picker" ref={modelPickerRef}>
              <button
                className="workers-model-trigger"
                type="button"
                data-testid="worker-model-picker-trigger"
                aria-labelledby="worker-model-label"
                aria-haspopup="dialog"
                aria-expanded={modelPickerPurpose === "settings"}
                onClick={(event) => toggleModelPicker(event.currentTarget, "settings")}
              >
                <span>{workerModelLabel(workerModel, models)}</span>
                <ChevronDown size={13} />
              </button>
            </div>
          </div>
        </div>
      )}

      {modelPickerOpen && modelPickerStyle && createPortal(
        <div
          ref={modelPickerMenuRef}
          className="workers-model-menu"
          data-native-preview-blocker="true"
          role="dialog"
          aria-label={modelPickerPurpose === "retry" ? "Retry Worker with model" : "Choose Worker model"}
          style={modelPickerStyle}
        >
          {modelPickerPurpose === "settings" && (
            <button
              className={`workers-model-inherit${workerModel ? "" : " active"}`}
              type="button"
              aria-pressed={!workerModel}
              onClick={() => {
                onWorkerModelChange("");
                setModelPickerPurpose(null);
              }}
            >
              <span>{!workerModel && <Check size={12} />}</span>
              <strong>Inherit parent</strong>
              <small>Use the model selected for this chat</small>
            </button>
          )}
          <ModelPicker
            models={models}
            model={modelPickerPurpose === "retry" ? selectedWorker?.model ?? "" : workerModel}
            providers={providers}
            toolIntent
            onModel={(selection) => {
              if (modelPickerPurpose === "retry" && selectedRecord && selectedTask)
                void onRetryWorker(selectedRecord.run.id, selectedTask.id, selection.model);
              else onWorkerModelChange(selection.model);
              setModelPickerPurpose(null);
            }}
            onManageProviders={() => {}}
            onManageMcp={() => {}}
            onManageMemory={() => {}}
            onClose={() => setModelPickerPurpose(null)}
            showManagementActions={false}
          />
        </div>,
        document.body,
      )}

      <div className="workers-inspector-summary">
        <strong>Workers</strong>
        <span>{[
          counts.proposed ? `${counts.proposed} planned` : "",
          counts.active ? `${counts.active} active` : "",
          counts.done ? `${counts.done} done` : "",
        ].filter(Boolean).join(" · ") || "No runs yet"}</span>
      </div>

      {!records.length ? (
        <div className="workers-empty"><span>No runs yet</span></div>
      ) : (
        <div className="workers-history-split">
          <div className="workers-history" role="list" aria-label="Worker history">
            <section aria-label="Active workers">
              <h3>Active <span>{counts.proposed + counts.active}</span></h3>
              {!proposedRecords.length && !activeEntries.length && (
                <p className="workers-history-empty">No active workers</p>
              )}
              {proposedRecords.map((record) => {
                const task = record.run.tasks[0];
                const key = `run:${record.run.id}`;
                return (
                  <button
                    key={key}
                    type="button"
                    role="listitem"
                    className={key === effectiveKey ? "active" : ""}
                    onClick={() => setSelectedKey(key)}
                  >
                    <span className="workers-dot queued" />
                    {task ? (
                      <WorkerAvatar
                        agent={agents.find((agent) => agent.id === task.agent_id)}
                        runId={record.run.id}
                        identityId={task.id}
                      />
                    ) : <span />}
                    <span className="worker-history-copy">
                      <strong>Worker plan</strong>
                      <small>{record.run.tasks.length} independent task{record.run.tasks.length === 1 ? "" : "s"}</small>
                    </span>
                    <span className="worker-history-meta"><em>approval</em></span>
                  </button>
                );
              })}
              {activeEntries.map(({ record, worker }) => renderHistoryRow(record, worker))}
            </section>

            <section aria-label="Completed workers">
              <h3>Done <span>{doneEntries.length}</span></h3>
              {!doneEntries.length && <p className="workers-history-empty">No completed workers</p>}
              {doneEntries.slice(0, visibleDone).map(({ record, worker }) => renderHistoryRow(record, worker))}
              {doneEntries.length > visibleDone && (
                <button
                  className="workers-history-more"
                  type="button"
                  onClick={() => setVisibleDone((count) => count + 10)}
                >
                  Show {Math.min(10, doneEntries.length - visibleDone)} more
                </button>
              )}
            </section>
          </div>

          {selectedPlan ? (
            <div className="workers-plan worker-history-detail" data-testid="workers-plan">
              <div className="workers-run-heading">
                <div>
                  <span>Proposed run</span>
                  <strong>{selectedPlan.run.tasks.length} independent task{selectedPlan.run.tasks.length === 1 ? "" : "s"}</strong>
                </div>
                <span className="workers-status proposed">{STATUS_LABEL.proposed}</span>
              </div>
              <div className="workers-plan-list">
                {selectedPlan.run.tasks.map((task, index) => (
                  <article key={task.id} className="workers-plan-task">
                    <WorkerAvatar
                      agent={agents.find((agent) => agent.id === task.agent_id)}
                      runId={selectedPlan.run.id}
                      identityId={task.id}
                    />
                    <div>
                      <strong>{task.title || `Task ${index + 1}`}</strong>
                      <p>{task.prompt}</p>
                      <small>{[
                        task.role,
                        task.model,
                        task.access === "write_review" ? "reviewable write" : "read-only",
                      ].filter(Boolean).join(" · ")}</small>
                    </div>
                  </article>
                ))}
              </div>
              <div className="workers-plan-actions">
                <button className="btn-accent" type="button" disabled={busy} onClick={() => onStart(selectedPlan.run.id)}>
                  <ArrowRight size={13} /> Run workers
                </button>
                <button className="btn-ghost" type="button" disabled={busy} onClick={() => onContinueSolo(selectedPlan.run.id)}>
                  Continue solo
                </button>
              </div>
            </div>
          ) : selectedRecord && selectedWorker ? (
            <section className="worker-detail worker-history-detail" aria-label="Selected worker details">
              <div className="worker-detail-head">
                <div className="worker-detail-identity">
                  <WorkerAvatar
                    agent={selectedAgent}
                    runId={selectedRecord.run.id}
                    identityId={selectedTask?.id ?? selectedWorker.id}
                  />
                  <div>
                    <span>{selectedTask?.role || selectedWorker.runtime} · {selectedWorker.model}</span>
                    <strong>{selectedTask?.title || selectedWorker.title}</strong>
                  </div>
                </div>
                <button
                  className="preview-action"
                  type="button"
                  title={copied ? "Copied" : "Copy result"}
                  aria-label={copied ? "Copied" : "Copy worker result"}
                  onClick={() => void copyResult()}
                >
                  <Copy size={13} />
                </button>
              </div>
              <div className="worker-detail-status">
                <span className={`workers-status ${selectedRecord.run.status}`}>
                  {STATUS_LABEL[selectedRecord.run.status]}
                  {selectedRecord.run.status === "running" && ` · ${elapsedLabel(selectedWorker.created_at, selectedWorker.finished_at, now)}`}
                </span>
                <small>{selectedWorker.access === "write_review" ? "review write" : "read-only"}</small>
              </div>
              {selectedRecord.run.status !== "proposed" && selectedRecord.run.status !== "running" && (
                <div className="worker-run-actions" ref={retryModelPickerRef}>
                  {(selectedWorker.status === "error" || selectedWorker.status === "stopped") && selectedTask && (
                    <>
                      <button className="btn-accent" type="button" disabled={busy} onClick={() => void onRetryWorker(selectedRecord.run.id, selectedTask.id)}>
                        <Refresh size={12} /> Retry
                      </button>
                      <button
                        className="btn-ghost"
                        type="button"
                        disabled={busy}
                        aria-haspopup="dialog"
                        aria-expanded={modelPickerPurpose === "retry"}
                        onClick={(event) => toggleModelPicker(event.currentTarget, "retry")}
                      >
                        Retry with model <ChevronDown size={12} />
                      </button>
                    </>
                  )}
                  <button
                    className="btn-ghost danger"
                    type="button"
                    disabled={busy}
                    onClick={() => {
                      if (confirmDeleteRunId === selectedRecord.run.id)
                        void onDeleteRun(selectedRecord.run.id);
                      else setConfirmDeleteRunId(selectedRecord.run.id);
                    }}
                  >
                    <Trash size={12} /> {confirmDeleteRunId === selectedRecord.run.id ? "Confirm delete" : "Delete run"}
                  </button>
                </div>
              )}
              <p className="worker-task-prompt">{selectedTask?.prompt || selectedWorker.prompt}</p>
              {selectedWorker.messages?.length ? (
                <div className="worker-activity">
                  {selectedWorker.messages.map((message, index) => (
                    <div key={message.id ?? index} className={`worker-message ${message.role}`}>
                      <span>{message.role}</span>
                      <WorkerMarkdown content={message.content} />
                    </div>
                  ))}
                </div>
              ) : (
                <div className="worker-result-markdown">
                  <WorkerMarkdown content={workerResult(selectedWorker)} />
                </div>
              )}
              {selectedWorker.worktree_path && (
                <div className="worker-worktree">
                  <span>Review worktree</span>
                  <code>{selectedWorker.worktree_path}</code>
                  <div className="workers-plan-actions">
                    <button className="btn-ghost" type="button" disabled={reviewBusy} onClick={() => void loadDiff()}>
                      Review diff
                    </button>
                    {diffReview && (
                      <button className="btn-accent" type="button" disabled={reviewBusy || !diffReview.diff.trim()} onClick={() => void applyDiff()}>
                        Apply diff
                      </button>
                    )}
                  </div>
                </div>
              )}
              {selectedRecord.run.status === "running" && (selectedWorker.status === "queued" || selectedWorker.status === "running") && (
                <button className="btn-ghost worker-stop" type="button" disabled={reviewBusy} onClick={() => void onStopWorker(selectedRecord.run.id, selectedWorker.id)}>
                  <Square size={12} /> Stop worker
                </button>
              )}
              {diffReview && <pre className="worker-result worker-diff">{diffReview.diff || "No changes."}</pre>}
              {reviewNotice && <p className="workers-run-error" role="status">{reviewNotice}</p>}
              {selectedRecord.run.error && <div className="workers-run-error" role="alert">{selectedRecord.run.error}</div>}
            </section>
          ) : (
            <div className="workers-empty worker-history-detail"><span>Waiting for workers to start...</span></div>
          )}
        </div>
      )}
    </aside>
  );
}
