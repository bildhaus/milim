import { useEffect, useRef, useState } from "react";
import {
  importClaudeThread,
  listClaudeThreads,
  listCodexThreads,
  recoverCodexThread,
  type ClaudeThreadSummary,
  type CodexThreadSummary,
} from "../api";
import { importedClaudeSession, importedClaudeSessionId } from "../lib/claudeImport";
import { recoveredCodexSession, recoveredCodexSessionId } from "../lib/codexRecovery";
import { useSessions } from "../sessions/store";
import { Check, ChevronDown, Folder, Search, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import "./ProvidersManager.css";

type CodexImportScope = "all" | "active" | "archived";
type RuntimeImportSelectionState = "none" | "some" | "all";

export interface RuntimeImportThread {
  id: string;
  title: string;
  preview: string;
  projectPath?: string | null;
  cwd?: string | null;
  updatedAt: number;
  fallbackMeta: string;
  resumable: boolean;
  archived: boolean;
}

export interface RuntimeImportGroup {
  key: string;
  label: string;
  projectPath: string | null;
  updatedAt: number;
  threads: RuntimeImportThread[];
}

function importProjectLabel(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

export function groupRuntimeImportThreads(
  threads: readonly RuntimeImportThread[],
): RuntimeImportGroup[] {
  const groups = new Map<string, RuntimeImportGroup>();
  for (const thread of threads) {
    const projectPath = thread.projectPath?.trim() || null;
    const key = projectPath ?? "";
    const group = groups.get(key) ?? {
      key,
      label: projectPath ? importProjectLabel(projectPath) : "No project",
      projectPath,
      updatedAt: 0,
      threads: [],
    };
    group.updatedAt = Math.max(group.updatedAt, thread.updatedAt);
    group.threads.push(thread);
    groups.set(key, group);
  }
  return Array.from(groups.values())
    .map((group) => ({
      ...group,
      threads: group.threads.slice().sort((left, right) =>
        right.updatedAt - left.updatedAt || left.id.localeCompare(right.id)),
    }))
    .sort((left, right) => {
      if (!left.projectPath) return 1;
      if (!right.projectPath) return -1;
      return right.updatedAt - left.updatedAt || left.label.localeCompare(right.label);
    });
}

export function filterRuntimeImportGroups(
  groups: readonly RuntimeImportGroup[],
  search: string,
): RuntimeImportGroup[] {
  const query = search.trim().toLowerCase();
  if (!query) return groups.slice();
  return groups.flatMap((group) => {
    if (
      group.label.toLowerCase().includes(query)
      || group.projectPath?.toLowerCase().includes(query)
    ) {
      return [group];
    }
    const threads = group.threads.filter((thread) =>
      [thread.title, thread.preview, thread.fallbackMeta]
        .some((value) => value.toLowerCase().includes(query)));
    return threads.length ? [{ ...group, threads }] : [];
  });
}

export function runtimeImportGroupSelection(
  group: RuntimeImportGroup,
  selectedIds: ReadonlySet<string>,
  importedIds: ReadonlySet<string>,
): RuntimeImportSelectionState {
  const available = group.threads.filter((thread) => !importedIds.has(thread.id));
  const selected = available.filter((thread) => selectedIds.has(thread.id)).length;
  if (!selected) return "none";
  return selected === available.length ? "all" : "some";
}

export function setRuntimeImportGroupSelected(
  selectedIds: ReadonlySet<string>,
  group: RuntimeImportGroup,
  importedIds: ReadonlySet<string>,
  checked: boolean,
): Set<string> {
  const next = new Set(selectedIds);
  for (const thread of group.threads) {
    if (importedIds.has(thread.id)) continue;
    if (checked) next.add(thread.id);
    else next.delete(thread.id);
  }
  return next;
}

function codexImportThread(thread: CodexThreadSummary): RuntimeImportThread {
  return {
    id: thread.id,
    title: thread.name?.trim() || thread.preview.trim() || "Untitled Codex chat",
    preview: thread.preview.trim(),
    projectPath: thread.project_path ?? thread.cwd,
    cwd: thread.cwd,
    updatedAt: thread.updated_at_ms,
    fallbackMeta: thread.model_provider,
    resumable: true,
    archived: thread.archived,
  };
}

function claudeImportThread(thread: ClaudeThreadSummary): RuntimeImportThread {
  return {
    id: thread.id,
    title: thread.title.trim() || thread.preview.trim() || "Untitled Claude chat",
    preview: thread.preview.trim(),
    projectPath: thread.cwd,
    cwd: thread.cwd,
    updatedAt: thread.updated_at_ms,
    fallbackMeta: "Claude CLI",
    resumable: thread.resumable,
    archived: false,
  };
}

function RuntimeImportCheckbox({
  state,
  disabled = false,
  label,
  onChange,
}: {
  state: RuntimeImportSelectionState;
  disabled?: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      className={`account-import-checkbox ${state}`}
      type="button"
      role="checkbox"
      aria-checked={state === "some" ? "mixed" : state === "all"}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(state !== "all")}
    >
      {state === "all" && <Check size={11} />}
      {state === "some" && <span aria-hidden="true" />}
    </button>
  );
}

export function AccountRuntimeImportDialog({
  runtime,
  onClose,
  onOpenSession,
}: {
  runtime: "codex" | "claude";
  onClose: () => void;
  onOpenSession: () => void;
}) {
  const sessions = useSessions((state) => state.sessions);
  const requestId = useRef(0);
  const [threads, setThreads] = useState<RuntimeImportThread[]>([]);
  const [search, setSearch] = useState("");
  const [scope, setScope] = useState<CodexImportScope>("all");
  const [busy, setBusy] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [expandedGroupKeys, setExpandedGroupKeys] = useState<Set<string>>(() => new Set());
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [failures, setFailures] = useState<Record<string, string>>({});
  const [result, setResult] = useState<{ imported: number; failed: number } | null>(null);
  const [error, setError] = useState("");
  const runtimeLabel = runtime === "codex" ? "Codex" : "Claude";

  async function load() {
    const currentRequest = ++requestId.current;
    setBusy(true);
    setError("");
    setThreads([]);
    try {
      const pages = runtime === "codex"
        ? await Promise.all(
            (scope === "all"
              ? [false, true]
              : [scope === "archived"]
            ).map((archived) => listCodexThreads({ archived, all: true })),
          )
        : [await listClaudeThreads({ all: true })];
      if (currentRequest !== requestId.current) return;
      const next = pages.flatMap((page) => runtime === "codex"
        ? page.data.map((thread) => codexImportThread(thread as CodexThreadSummary))
        : page.data.map((thread) => claudeImportThread(thread as ClaudeThreadSummary)));
      setThreads(
        Array.from(new Map(next.map((thread) => [thread.id, thread])).values())
          .sort((left, right) => right.updatedAt - left.updatedAt || left.id.localeCompare(right.id)),
      );
    } catch (error) {
      if (currentRequest === requestId.current)
        setError(error instanceof Error ? error.message : `${runtimeLabel} chat import failed.`);
    } finally {
      if (currentRequest === requestId.current) setBusy(false);
    }
  }

  useEffect(() => {
    void load();
  }, [scope, runtime]);

  function openSession(id: string) {
    if (progress) return;
    useSessions.getState().switchTo(id);
    onOpenSession();
  }

  function importedSessionId(threadId: string) {
    return runtime === "codex"
      ? recoveredCodexSessionId(sessions, threadId)
      : importedClaudeSessionId(sessions, threadId);
  }

  async function importThread(thread: RuntimeImportThread): Promise<string> {
    const store = useSessions.getState();
    let sessionId: string | null;
    let nativeId: string;
    let resumable = true;
    if (runtime === "codex") {
      const source = await recoverCodexThread(thread.id);
      sessionId = store.importSession(recoveredCodexSession(source));
      nativeId = source.id;
    } else {
      const source = await importClaudeThread(thread.id);
      sessionId = store.importSession(importedClaudeSession(source));
      nativeId = source.id;
      resumable = source.resumable;
    }
    if (!sessionId) throw new Error("milim could not import the selected chat.");
    const importedSession = useSessions.getState().sessions.find((session) => session.id === sessionId);
    const lastMessageId = importedSession?.messages[importedSession.messages.length - 1]?.id;
    if (!lastMessageId) throw new Error("The imported chat did not contain a sync cursor.");
    if (runtime === "codex") {
      useSessions.getState().setAccountRuntime(sessionId, {
        codexThreadId: nativeId,
        codexLastSyncedMessageId: lastMessageId,
      });
    } else {
      useSessions.getState().setAccountRuntime(sessionId, {
        claudeSessionId: nativeId,
        claudeLastSyncedMessageId: resumable ? lastMessageId : undefined,
      });
    }
    return sessionId;
  }

  async function importSelected() {
    const selected = threads.filter(
      (thread) => selectedIds.has(thread.id) && !importedSessionId(thread.id),
    );
    if (!selected.length) return;
    const previousActiveId = useSessions.getState().activeId;
    const imported = new Set<string>();
    const nextFailures: Record<string, string> = {};
    setError("");
    setFailures({});
    setResult(null);
    try {
      for (const [index, thread] of selected.entries()) {
        setProgress({ current: index + 1, total: selected.length });
        try {
          await importThread(thread);
          imported.add(thread.id);
        } catch (error) {
          nextFailures[thread.id] = error instanceof Error
            ? error.message
            : `${runtimeLabel} chat import failed.`;
          setFailures({ ...nextFailures });
        }
      }
    } finally {
      if (
        previousActiveId
        && useSessions.getState().sessions.some((session) => session.id === previousActiveId)
      ) {
        useSessions.getState().switchTo(previousActiveId);
      }
      setSelectedIds((current) =>
        new Set(Array.from(current).filter((id) => !imported.has(id))));
      setResult({ imported: imported.size, failed: Object.keys(nextFailures).length });
      setProgress(null);
    }
  }

  const importedIds = new Set(
    threads
      .filter((thread) => importedSessionId(thread.id))
      .map((thread) => thread.id),
  );
  const groups = groupRuntimeImportThreads(threads);
  const visibleGroups = filterRuntimeImportGroups(groups, search);
  const selectedCount = threads.filter(
    (thread) => selectedIds.has(thread.id) && !importedIds.has(thread.id),
  ).length;
  const projectCount = groups.filter((group) => group.projectPath).length;
  const close = () => {
    if (!progress) onClose();
  };

  return (
    <SheetDialog
      title={`Import ${runtimeLabel} chats`}
      className="sheet account-import-sheet"
      testId={`${runtime}-import-dialog`}
      onClose={close}
    >
      <header className="sheet-header account-import-header">
        <div className="providers-title">
          <h2>Import {runtimeLabel} chats</h2>
          <p className="sheet-sub">
            Choose projects or individual chats. Existing imports won&apos;t be duplicated.
          </p>
        </div>
        <div className="account-import-header-meta">
          {!busy && !error && (
            <span>
              {threads.length} {threads.length === 1 ? "chat" : "chats"}
              {projectCount ? ` across ${projectCount} ${projectCount === 1 ? "project" : "projects"}` : ""}
            </span>
          )}
          <button
            className="icon-btn sheet-close"
            type="button"
            disabled={Boolean(progress)}
            onClick={close}
            aria-label="Close import"
          >
            <X size={16} />
          </button>
        </div>
      </header>
      <div className="account-import-controls">
        <label className="account-import-search">
          <Search size={13} aria-hidden="true" />
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            placeholder={`Search ${runtimeLabel} chats`}
            aria-label={`Search ${runtimeLabel} chats`}
          />
        </label>
        {runtime === "codex" && (
          <div className="account-import-scopes" role="group" aria-label="Codex chat scope">
            {(["all", "active", "archived"] as const).map((value) => (
              <button
                key={value}
                type="button"
                className={scope === value ? "active" : ""}
                aria-pressed={scope === value}
                disabled={Boolean(progress)}
                onClick={() => {
                  if (scope === value) return;
                  setScope(value);
                  setSelectedIds(new Set());
                  setExpandedGroupKeys(new Set());
                  setFailures({});
                  setResult(null);
                }}
              >
                {value[0].toUpperCase() + value.slice(1)}
              </button>
            ))}
          </div>
        )}
      </div>
      <main className="account-import-catalog" aria-busy={busy}>
        {error && (
          <div className="account-import-load-error" role="alert">
            <span>{error}</span>
            <button className="btn-ghost" type="button" onClick={() => void load()}>
              Retry
            </button>
          </div>
        )}
        {busy && (
          <div className="account-import-skeletons" aria-label={`Loading ${runtimeLabel} chats`}>
            {[0, 1, 2].map((index) => (
              <div className="account-import-skeleton" key={index}>
                <i />
                <span />
                <small />
              </div>
            ))}
          </div>
        )}
        {!busy && !error && visibleGroups.map((group, groupIndex) => {
          const allGroup = groups.find((item) => item.key === group.key) ?? group;
          const availableCount = allGroup.threads.filter(
            (thread) => !importedIds.has(thread.id),
          ).length;
          const importedCount = allGroup.threads.length - availableCount;
          const selection = runtimeImportGroupSelection(
            allGroup,
            selectedIds,
            importedIds,
          );
          const expanded = Boolean(search.trim()) || expandedGroupKeys.has(group.key);
          const contentId = `${runtime}-import-group-${groupIndex}`;
          return (
            <section className="account-import-group" key={group.key || "no-project"}>
              <div className="account-import-group-header">
                <RuntimeImportCheckbox
                  state={selection}
                  disabled={!availableCount || Boolean(progress)}
                  label={`${selection === "all" ? "Deselect" : "Select"} all chats in ${group.label}`}
                  onChange={(checked) => {
                    setSelectedIds((current) =>
                      setRuntimeImportGroupSelected(current, allGroup, importedIds, checked));
                    setResult(null);
                  }}
                />
                <button
                  className="account-import-group-toggle"
                  type="button"
                  aria-expanded={expanded}
                  aria-controls={contentId}
                  onClick={() => setExpandedGroupKeys((current) => {
                    const next = new Set(current);
                    if (next.has(group.key)) next.delete(group.key);
                    else next.add(group.key);
                    return next;
                  })}
                >
                  <Folder size={14} aria-hidden="true" />
                  <span className="account-import-group-copy">
                    <strong>{group.label}</strong>
                    <span title={group.projectPath ?? undefined}>
                      {group.projectPath ?? "Chats without a recorded project folder"}
                    </span>
                  </span>
                  <span className="account-import-group-count">
                    {allGroup.threads.length} {allGroup.threads.length === 1 ? "chat" : "chats"}
                    {importedCount ? ` · ${importedCount} imported` : ""}
                  </span>
                  <ChevronDown
                    className={expanded ? "expanded" : ""}
                    size={13}
                    aria-hidden="true"
                  />
                </button>
              </div>
              {expanded && (
                <div className="account-import-threads" id={contentId}>
                  {group.threads.map((thread) => {
                    const existing = importedSessionId(thread.id);
                    const checked = selectedIds.has(thread.id);
                    const failure = failures[thread.id];
                    const preview = thread.preview && thread.preview !== thread.title
                      ? thread.preview
                      : "";
                    return (
                      <article
                        className={`account-import-thread${checked ? " selected" : ""}${failure ? " failed" : ""}`}
                        key={thread.id}
                      >
                        {existing ? (
                          <span className="account-import-checkbox-spacer" aria-hidden="true" />
                        ) : (
                          <RuntimeImportCheckbox
                            state={checked ? "all" : "none"}
                            disabled={Boolean(progress)}
                            label={`${checked ? "Deselect" : "Select"} ${thread.title}`}
                            onChange={(nextChecked) => {
                              setSelectedIds((current) => {
                                const next = new Set(current);
                                if (nextChecked) next.add(thread.id);
                                else next.delete(thread.id);
                                return next;
                              });
                              setResult(null);
                            }}
                          />
                        )}
                        <div className="account-import-thread-copy">
                          <strong>{thread.title}</strong>
                          {preview && <p>{preview}</p>}
                          <span className="account-import-thread-meta">
                            {thread.updatedAt
                              ? new Date(thread.updatedAt).toLocaleString()
                              : thread.fallbackMeta}
                            {thread.archived && <em>Archived</em>}
                            {runtime === "codex" && thread.projectPath && !thread.cwd && (
                              <em>Folder unavailable</em>
                            )}
                            {runtime === "claude" && !thread.resumable && (
                              <em>Transcript only</em>
                            )}
                          </span>
                          {failure && <span className="account-import-thread-error">{failure}</span>}
                        </div>
                        {existing && (
                          <button
                            className="btn-ghost account-import-open"
                            type="button"
                            disabled={Boolean(progress)}
                            onClick={() => openSession(existing)}
                          >
                            Open
                          </button>
                        )}
                      </article>
                    );
                  })}
                </div>
              )}
            </section>
          );
        })}
        {!busy && !error && visibleGroups.length === 0 && (
          <div className="account-import-empty">
            <strong>{search.trim() ? "No matching chats" : `No ${runtimeLabel} chats found`}</strong>
            <span>
              {search.trim()
                ? "Try a chat title, preview, or project folder."
                : `No importable ${runtimeLabel} history is available on this device.`}
            </span>
          </div>
        )}
      </main>
      <footer className="account-import-footer" aria-live="polite">
        <div className="account-import-status">
          <strong>
            {progress
              ? `Importing ${progress.current} of ${progress.total}`
              : selectedCount
                ? `${selectedCount} selected`
                : result
                  ? "Import complete"
                  : "Select chats to import"}
          </strong>
          <span>
            {progress
              ? "Chats are imported one at a time."
              : result
                ? `${result.imported} imported${result.failed ? ` · ${result.failed} failed` : ""}`
                : importedIds.size
                  ? `${importedIds.size} already in milim`
                  : "Project selection includes every chat in the current scope."}
          </span>
        </div>
        <div className="account-import-actions">
          <button
            className="btn-ghost"
            type="button"
            disabled={Boolean(progress)}
            onClick={close}
          >
            {result ? "Done" : "Cancel"}
          </button>
          <button
            className="btn-accent"
            type="button"
            disabled={busy || Boolean(progress) || !selectedCount}
            onClick={() => void importSelected()}
          >
            {progress
              ? `Importing ${progress.current} of ${progress.total}`
              : result?.failed && selectedCount === result.failed
                ? `Retry ${selectedCount} failed`
                : `Import ${selectedCount} ${selectedCount === 1 ? "chat" : "chats"}`}
          </button>
        </div>
      </footer>
    </SheetDialog>
  );
}
