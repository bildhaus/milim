import { useEffect, useMemo, useState } from "react";
import {
  archiveMemoryNode,
  deleteMemoryNode,
  getWorkspaceContext,
  listMemoryNodes,
  registerGraphMemory,
  restoreMemoryNode,
  reviewMemoryNode,
  searchGraphMemory,
  updateMemoryNode,
  type MemoryNode,
  type MemoryScopeKind,
  type MemoryScopeRef,
  type WorkspaceContext,
} from "../api";
import { DEFAULT_THREAD_SETTINGS, useSessions } from "../sessions/store";
import { Archive, Check, Pencil, Plus, Refresh, Search, Trash, X } from "./icons";
import { SheetDialog } from "./SheetDialog";
import "./MemoryManager.css";

type MemoryTab = "personal" | "project" | "legacy";
type DetailMode = "view" | "edit" | "create";

interface MemoryDraft {
  title: string;
  content: string;
}

const PERSONAL_SCOPE = { kind: "global", label: "Personal", locator: "personal" } as const;

function tabForScope(kind?: MemoryScopeKind): MemoryTab {
  return kind === "project" ? "project" : kind === "thread" ? "legacy" : "personal";
}

function projectLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "Project";
}

function tabScopes(tab: MemoryTab, threadId: string, folder: string, workspace: WorkspaceContext | null): MemoryScopeRef[] {
  if (tab === "personal") return [{ kind: PERSONAL_SCOPE.kind, locator: PERSONAL_SCOPE.locator }];
  if (tab === "legacy") return [{ kind: "thread", locator: threadId }];
  if (tab !== "project" || !folder.trim()) return [];
  return [...new Set([workspace?.project_locator, workspace?.legacy_project_locator, folder.trim()].filter(Boolean))]
    .map((locator) => ({ kind: "project" as const, locator: locator as string }));
}

function writeScope(tab: Exclude<MemoryTab, "legacy">, folder: string, workspace: WorkspaceContext | null) {
  if (tab === "personal") return PERSONAL_SCOPE;
  return folder.trim()
    ? { kind: "project" as const, label: projectLabel(folder), locator: workspace?.project_locator || folder.trim() }
    : null;
}

function titleFor(node: MemoryNode): string {
  return node.title.trim() || node.body.split(/\r?\n/).find(Boolean)?.trim() || "Untitled memory";
}

function previewFor(node: MemoryNode): string {
  return node.body.trim() || "No additional details.";
}

function isArchived(node: MemoryNode): boolean {
  return Boolean(node.archived_at);
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "" : date.toLocaleDateString();
}

function draftFrom(node: MemoryNode): MemoryDraft {
  return { title: node.title, content: node.body };
}

export function MemoryManager({
  initialNodeId,
  initialScopeKind,
  onClose,
}: {
  initialNodeId?: string;
  initialScopeKind?: MemoryScopeKind;
  onClose: () => void;
}) {
  const activeId = useSessions((state) => state.activeId);
  const session = useSessions((state) => state.sessions.find((item) => item.id === state.activeId));
  const settings = session?.settings ?? DEFAULT_THREAD_SETTINGS;
  const folder = settings.folder;
  const model = settings.model.trim();

  const [tab, setTab] = useState<MemoryTab>(() => tabForScope(initialScopeKind));
  const [nodes, setNodes] = useState<MemoryNode[]>([]);
  const [legacyCount, setLegacyCount] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(initialNodeId ?? null);
  const [mode, setMode] = useState<DetailMode>("view");
  const [query, setQuery] = useState("");
  const [includeArchived, setIncludeArchived] = useState(false);
  const [draft, setDraft] = useState<MemoryDraft>({ title: "", content: "" });
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [workspaceContext, setWorkspaceContext] = useState<WorkspaceContext | null>(null);

  const selected = useMemo(() => nodes.find((node) => node.id === selectedId) ?? null, [nodes, selectedId]);
  const canUseProject = Boolean(folder.trim());
  const canSave = Boolean(draft.title.trim() || draft.content.trim());

  useEffect(() => {
    if (tab === "project" && !canUseProject) setTab("personal");
  }, [canUseProject, tab]);

  useEffect(() => {
    if (!folder.trim()) {
      setWorkspaceContext(null);
      return;
    }
    void getWorkspaceContext()
      .then(setWorkspaceContext)
      .catch(() => setWorkspaceContext(null));
  }, [folder]);

  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeId, folder, includeArchived, tab, workspaceContext]);

  useEffect(() => {
    void listMemoryNodes({ scope: { kind: "thread", locator: activeId }, limit: 300 })
      .then((legacy) => setLegacyCount(legacy.length));
  }, [activeId]);

  async function load(selectId?: string | null) {
    const scopes = tabScopes(tab, activeId, folder, workspaceContext);
    if (scopes.length === 0) return;
    setBusy(true);
    setNote(null);
    try {
      const lists = await Promise.all(scopes.map((scope) => listMemoryNodes({ scope, includeArchived, limit: 300 })));
      const next = [...new Map(lists.flat().map((node) => [node.id, node])).values()];
      next.sort((a, b) => Date.parse(b.updated_at) - Date.parse(a.updated_at));
      setNodes(next);
      setSelectedId((current) => {
        if (selectId && next.some((node) => node.id === selectId)) return selectId;
        if (current && next.some((node) => node.id === current)) return current;
        return next[0]?.id ?? null;
      });
      setMode("view");
    } finally {
      setBusy(false);
    }
  }

  async function search() {
    const text = query.trim();
    if (!text) {
      await load();
      return;
    }
    const scopes = tabScopes(tab, activeId, folder, workspaceContext);
    if (scopes.length === 0) return;
    setBusy(true);
    setNote(null);
    try {
      const hits = await searchGraphMemory(text, scopes, 100, model || undefined, includeArchived);
      setNodes([...new Map(hits.map((hit) => [hit.node.id, hit.node])).values()]);
      setSelectedId(hits[0]?.node.id ?? null);
      setMode("view");
    } finally {
      setBusy(false);
    }
  }

  function changeTab(next: MemoryTab) {
    if (next === "project" && !canUseProject) return;
    setTab(next);
    setQuery("");
    setSelectedId(null);
    setMode("view");
    setNote(null);
  }

  function startCreate() {
    if (tab === "legacy") return;
    setDraft({ title: "", content: "" });
    setSelectedId(null);
    setMode("create");
    setNote(null);
  }

  async function addMemory() {
    if (tab === "legacy") return;
    const scope = writeScope(tab, folder, workspaceContext);
    const content = draft.content.trim();
    const title = draft.title.trim() || content.split(/\r?\n/).find(Boolean)?.trim() || "Memory";
    if (!scope || (!draft.title.trim() && !content)) return;
    setBusy(true);
    setNote(null);
    try {
      const saved = await registerGraphMemory({
        model: model || undefined,
        scope,
        node: { kind: "fact", title, body: content, source: "user", confidence: 1 },
        event: { thread_id: activeId, summary: title },
      });
      if (!saved) {
        setNote("Failed to store memory.");
        return;
      }
      await load(saved.node.id);
      setNote("Stored.");
    } finally {
      setBusy(false);
    }
  }

  async function saveSelected() {
    if (!selected || !canSave) return;
    const title = draft.title.trim() || draft.content.split(/\r?\n/).find(Boolean)?.trim() || "Memory";
    setBusy(true);
    setNote(null);
    try {
      const updated = await updateMemoryNode(selected.id, { title, body: draft.content.trim() }, model || undefined);
      if (!updated) {
        setNote("Failed to update memory.");
        return;
      }
      setNodes((current) => current.map((node) => node.id === updated.id ? updated : node));
      setMode("view");
      setNote("Updated.");
    } finally {
      setBusy(false);
    }
  }

  async function forgetSelected() {
    if (!selected || isArchived(selected)) return;
    setBusy(true);
    setNote(null);
    try {
      if (!await archiveMemoryNode(selected.id)) {
        setNote("Failed to forget memory.");
        return;
      }
      if (tab === "legacy") {
        if (legacyCount <= 1) setTab("personal");
        setLegacyCount((count) => Math.max(0, count - 1));
      }
      await load();
      setNote("Forgotten. Show archived memories to permanently delete it.");
    } finally {
      setBusy(false);
    }
  }

  async function restoreSelected() {
    if (!selected || !isArchived(selected)) return;
    setBusy(true);
    setNote(null);
    try {
      if (!await restoreMemoryNode(selected.id)) {
        setNote("Failed to restore memory.");
        return;
      }
      await load(selected.id);
      setNote("Restored and marked reviewed.");
    } finally {
      setBusy(false);
    }
  }

  async function reviewSelected() {
    if (!selected || isArchived(selected) || selected.reviewed_at) return;
    setBusy(true);
    setNote(null);
    try {
      if (!await reviewMemoryNode(selected.id)) {
        setNote("Failed to mark memory reviewed.");
        return;
      }
      await load(selected.id);
      setNote("Marked reviewed.");
    } finally {
      setBusy(false);
    }
  }

  async function deleteSelected() {
    if (!selected || !isArchived(selected)) return;
    if (confirmDeleteId !== selected.id) {
      setConfirmDeleteId(selected.id);
      setNote("Press Delete permanently again to confirm.");
      return;
    }
    setBusy(true);
    setNote(null);
    try {
      if (!await deleteMemoryNode(selected.id)) {
        setNote("Failed to delete memory.");
        return;
      }
      setConfirmDeleteId(null);
      await load();
      setNote("Permanently deleted.");
    } finally {
      setBusy(false);
    }
  }

  async function moveLegacy(target: "personal" | "project") {
    if (!selected || tab !== "legacy") return;
    const scope = writeScope(target, folder, workspaceContext);
    if (!scope) return;
    setBusy(true);
    setNote(null);
    try {
      const saved = await registerGraphMemory({
        model: model || undefined,
        scope,
        node: {
          kind: selected.kind || "fact",
          title: titleFor(selected),
          body: selected.body,
          source: selected.source || "user",
          confidence: selected.confidence,
        },
        event: { thread_id: activeId, summary: titleFor(selected) },
      });
      if (!saved) {
        setNote("Failed to move memory.");
        return;
      }
      if (!saved.node.reviewed_at && !(await reviewMemoryNode(saved.node.id))) {
        setNote("Copied, but the moved memory could not be marked reviewed.");
        return;
      }
      if (!await archiveMemoryNode(selected.id)) {
        setNote("Copied, but the legacy memory could not be forgotten.");
        return;
      }
      setLegacyCount((count) => Math.max(0, count - 1));
      setTab(target);
      setSelectedId(saved.node.id);
      setNote(`Moved to ${target === "personal" ? "Personal" : "Project"}.`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <SheetDialog title="Memory" className="sheet mem-sheet" onClose={onClose}>
      <div className="sheet-header mem-sheet-header">
        <div>
          <h2>Memory</h2>
          <p className="sheet-sub">Keep useful context across conversations and projects.</p>
        </div>
        <button className="icon-btn" type="button" onClick={onClose} title="Close" aria-label="Close memory manager"><X size={15} /></button>
      </div>

      <div className="mem-tabs" role="tablist" aria-label="Memory scopes">
        <button type="button" role="tab" aria-selected={tab === "personal"} className={tab === "personal" ? "active" : ""} onClick={() => changeTab("personal")}>Personal</button>
        <button type="button" role="tab" aria-selected={tab === "project"} className={tab === "project" ? "active" : ""} disabled={!canUseProject} title={canUseProject ? projectLabel(folder) : "Select a working folder first"} onClick={() => changeTab("project")}>Project</button>
        {legacyCount > 0 && <button type="button" role="tab" aria-selected={tab === "legacy"} className={tab === "legacy" ? "active secondary" : "secondary"} onClick={() => changeTab("legacy")}>Legacy thread</button>}
      </div>

      <div className="mem-toolbar">
        <form onSubmit={(event) => { event.preventDefault(); void search(); }}>
          <label className="mem-search"><Search size={14} /><input className="css-input" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search memories" aria-label="Search memories" /></label>
          <button className="btn-ghost" type="submit" disabled={busy}>Search</button>
        </form>
        <label className="mem-archived-toggle"><input type="checkbox" checked={includeArchived} onChange={(event) => setIncludeArchived(event.target.checked)} /> Show archived</label>
        <button className="btn-ghost mem-icon-action" type="button" disabled={busy} onClick={() => void load()}><Refresh size={14} /> Refresh</button>
        {tab !== "legacy" && <button className="btn-accent mem-icon-action" type="button" disabled={busy || (tab === "project" && !canUseProject)} onClick={startCreate}><Plus size={14} /> Add</button>}
      </div>

      <div className="mem-layout">
        <main className="mem-list" aria-label={`${tab} memories`} aria-busy={busy}>
          {nodes.map((node) => (
            <button key={node.id} type="button" className={`mem-row${node.id === selectedId ? " active" : ""}${isArchived(node) ? " archived" : ""}`} onClick={() => { setSelectedId(node.id); setMode("view"); setNote(null); setConfirmDeleteId(null); }}>
              <span className="mem-row-head"><strong>{titleFor(node)}</strong><small>{formatDate(node.updated_at)}</small></span>
              <span>{previewFor(node)}</span>
              {isArchived(node) && <em>Archived</em>}
            </button>
          ))}
          {!busy && nodes.length === 0 && <div className="mem-empty"><strong>No memories here</strong><span>{tab === "legacy" ? "This thread has no legacy memories." : tab === "project" && !canUseProject ? "Select a working folder first." : "Add context worth keeping."}</span></div>}
        </main>

        <aside className="mem-detail" aria-label="Memory detail">
          {mode === "create" ? (
            <MemoryForm title="New memory" draft={draft} onChange={setDraft} busy={busy} onCancel={() => setMode("view")} onSave={() => void addMemory()} saveLabel="Add" canSave={canSave} />
          ) : selected && mode === "edit" ? (
            <MemoryForm title="Edit memory" draft={draft} onChange={setDraft} busy={busy} onCancel={() => setMode("view")} onSave={() => void saveSelected()} saveLabel="Save" canSave={canSave} />
          ) : selected ? (
            <>
              <div className="mem-detail-head"><div><span>{tab === "legacy" ? "Legacy thread" : tab === "personal" ? "Personal" : "Project"}</span><h3>{titleFor(selected)}</h3></div>{isArchived(selected) ? <em>Archived</em> : !selected.reviewed_at ? <em>Needs review</em> : <em>Reviewed</em>}</div>
              <div className="mem-content">{selected.body.trim() || <span>No additional details.</span>}</div>
              <p className="mem-date">Updated {formatDate(selected.updated_at)}</p>
              {tab === "legacy" && !isArchived(selected) && <div className="mem-move"><span>Move this memory to:</span><button className="btn-ghost" type="button" disabled={busy} onClick={() => void moveLegacy("personal")}>Personal</button><button className="btn-ghost" type="button" disabled={busy || !canUseProject} onClick={() => void moveLegacy("project")}>Project</button></div>}
              {note && <p className="sheet-hint">{note}</p>}
              <div className="mem-actions">
                {!isArchived(selected) ? <>
                  <button className="btn-ghost mem-icon-action" type="button" disabled={busy} onClick={() => void forgetSelected()}><Archive size={14} /> Forget</button>
                  {!selected.reviewed_at && <button className="btn-ghost mem-icon-action" type="button" disabled={busy} onClick={() => void reviewSelected()}><Check size={14} /> Mark reviewed</button>}
                </> : <>
                  <button className="btn-ghost mem-icon-action" type="button" disabled={busy} onClick={() => void restoreSelected()}><Refresh size={14} /> Restore</button>
                  <button className="btn-ghost danger mem-icon-action" type="button" disabled={busy} onClick={() => void deleteSelected()}>{confirmDeleteId === selected.id ? <Check size={14} /> : <Trash size={14} />} Delete permanently</button>
                </>}
                <span className="spacer" />
                <button className="btn-ghost mem-icon-action" type="button" disabled={busy} onClick={() => { setDraft(draftFrom(selected)); setMode("edit"); setNote(null); }}><Pencil size={14} /> Edit</button>
              </div>
            </>
          ) : <div className="mem-empty"><strong>{busy ? "Loading..." : "Select a memory"}</strong><span>Choose an item to view or edit it.</span></div>}
          {mode !== "view" && note && <p className="sheet-hint">{note}</p>}
        </aside>
      </div>
    </SheetDialog>
  );
}

function MemoryForm({ title, draft, onChange, busy, onCancel, onSave, saveLabel, canSave }: {
  title: string;
  draft: MemoryDraft;
  onChange: (draft: MemoryDraft) => void;
  busy: boolean;
  onCancel: () => void;
  onSave: () => void;
  saveLabel: string;
  canSave: boolean;
}) {
  return <>
    <div className="mem-detail-head"><div><span>Memory</span><h3>{title}</h3></div></div>
    <label className="field"><span>Title <small>optional</small></span><input className="css-input" value={draft.title} onChange={(event) => onChange({ ...draft, title: event.target.value })} /></label>
    <label className="field"><span>Content</span><textarea className="instr-input mem-content-input" value={draft.content} onChange={(event) => onChange({ ...draft, content: event.target.value })} autoFocus /></label>
    <div className="mem-actions"><button className="btn-ghost" type="button" disabled={busy} onClick={onCancel}>Cancel</button><span className="spacer" /><button className="btn-accent mem-icon-action" type="button" disabled={busy || !canSave} onClick={onSave}><Check size={14} /> {saveLabel}</button></div>
  </>;
}
