import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { createPortal } from "react-dom";
import {
  completeWorkspaceEditorLeave,
  listWorkspaceDirectory,
  listWorkspaceFiles,
  readWorkspaceTextFile,
  setWorkspaceEditorDirty,
  writeWorkspaceTextFile,
  type ChatArtifact,
  type WorkspaceDirectoryEntry,
  type WorkspaceTextFile,
} from "../api";
import { registerWorkspaceEditorGuard, requestWorkspaceEditorLeave, type WorkspaceEditorLeaveReason } from "../lib/workspaceEditorGuard";
import { ChevronDown, Eye, FileText, Folder, FolderOpen, Search, Sidebar } from "./icons";
import { SourceCodeView } from "./SourceCodeView";

const WorkspaceCodeEditor = lazy(() => import("./WorkspaceCodeEditor"));
const RAIL_MIN_WIDTH = 148;
const RAIL_MAX_WIDTH = 360;
const RAIL_DEFAULT_WIDTH = 188;
const RAIL_KEYBOARD_STEP = 24;

export type GeneratedCodeFile = {
  artifact: ChatArtifact;
  path: string;
  entry: boolean;
};

type DirectoryState = {
  entries: WorkspaceDirectoryEntry[];
  cursor: string | null;
  status: "loading" | "ready" | "error";
  error?: string;
};

type OpenWorkspaceFile = WorkspaceTextFile & { draft: string };

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function WorkspaceCodePanel({
  workspaceFolder,
  files,
  selectedArtifactId,
  onSelectArtifact,
  onPreviewWorkspaceFile,
  runtimeBusy = false,
}: {
  workspaceFolder?: string;
  files: readonly GeneratedCodeFile[];
  selectedArtifactId: string;
  onSelectArtifact: (id: string) => void;
  onPreviewWorkspaceFile?: (path: string) => void;
  runtimeBusy?: boolean;
}) {
  const workspace = workspaceFolder?.trim() ?? "";
  const generatedFiles = useMemo(() => files.filter((file) => file.artifact.id !== "workspace-review"), [files]);
  const [directories, setDirectories] = useState<Record<string, DirectoryState>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<WorkspaceDirectoryEntry[]>([]);
  const [searchStatus, setSearchStatus] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [searchError, setSearchError] = useState("");
  const [openFile, setOpenFile] = useState<OpenWorkspaceFile | null>(null);
  const [activeSource, setActiveSource] = useState<"workspace" | "generated">(
    generatedFiles.some((file) => file.artifact.id === selectedArtifactId) ? "generated" : "workspace",
  );
  const [loadingPath, setLoadingPath] = useState<string | null>(null);
  const [fileError, setFileError] = useState("");
  const [saveStatus, setSaveStatus] = useState("");
  const [saving, setSaving] = useState(false);
  const [conflictRevision, setConflictRevision] = useState<string | null>(null);
  const [leaveRequest, setLeaveRequest] = useState<{ reason: WorkspaceEditorLeaveReason; resolve: (leave: boolean) => void } | null>(null);
  const [railWidth, setRailWidth] = useState(RAIL_DEFAULT_WIDTH);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const resizeStartRef = useRef<{ x: number; width: number } | null>(null);
  const previousSelectedArtifactRef = useRef(selectedArtifactId);
  const dirty = Boolean(openFile && openFile.draft !== openFile.content);
  const selectedArtifact = generatedFiles.find((file) => file.artifact.id === selectedArtifactId) ?? generatedFiles[0];

  const loadDirectory = useCallback(async (path: string, cursor?: string) => {
    if (!workspace) return;
    setDirectories((current) => ({
      ...current,
      [path]: { entries: cursor ? current[path]?.entries ?? [] : [], cursor: null, status: "loading" },
    }));
    try {
      const page = await listWorkspaceDirectory(workspace, path, cursor);
      setDirectories((current) => ({
        ...current,
        [path]: {
          entries: cursor ? [...(current[path]?.entries ?? []), ...page.entries] : page.entries,
          cursor: page.next_cursor ?? null,
          status: "ready",
        },
      }));
    } catch (error) {
      setDirectories((current) => ({
        ...current,
        [path]: { entries: current[path]?.entries ?? [], cursor: null, status: "error", error: errorMessage(error) },
      }));
    }
  }, [workspace]);

  useEffect(() => {
    setDirectories({});
    setExpanded(new Set());
    setOpenFile(null);
    setFileError("");
    setSearchQuery("");
    if (workspace) void loadDirectory("");
  }, [loadDirectory, workspace]);

  useEffect(() => {
    if (!generatedFiles.some((file) => file.artifact.id === selectedArtifactId)) return;
    if (previousSelectedArtifactRef.current !== selectedArtifactId) {
      previousSelectedArtifactRef.current = selectedArtifactId;
      setOpenFile(null);
      setActiveSource("generated");
    } else if (!openFile) setActiveSource("generated");
  }, [generatedFiles, openFile, selectedArtifactId]);

  useEffect(() => {
    const query = searchQuery.trim();
    if (!workspace || !query) {
      setSearchResults([]);
      setSearchStatus("idle");
      return;
    }
    let canceled = false;
    setSearchStatus("loading");
    const timeout = window.setTimeout(() => {
      void listWorkspaceFiles(workspace, query, 50)
        .then((results) => {
          if (canceled) return;
          setSearchResults(results.map(({ path, name, size }) => ({ path, name, size, kind: "file" })));
          setSearchStatus("ready");
        })
        .catch((error) => {
          if (canceled) return;
          setSearchError(errorMessage(error));
          setSearchStatus("error");
        });
    }, 150);
    return () => {
      canceled = true;
      window.clearTimeout(timeout);
    };
  }, [searchQuery, workspace]);

  const confirmLeave = useCallback((reason: WorkspaceEditorLeaveReason) => {
    if (!dirty) return Promise.resolve(true);
    return new Promise<boolean>((resolve) => setLeaveRequest({ reason, resolve }));
  }, [dirty]);

  useEffect(() => dirty ? registerWorkspaceEditorGuard(confirmLeave) : undefined, [confirmLeave, dirty]);
  useEffect(() => {
    void setWorkspaceEditorDirty(dirty);
    return () => { void setWorkspaceEditorDirty(false); };
  }, [dirty]);
  useEffect(() => {
    if (!dirty) return;
    const preventUnload = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", preventUnload);
    return () => window.removeEventListener("beforeunload", preventUnload);
  }, [dirty]);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void import("@tauri-apps/api/event").then(({ listen }) => listen<string>("milim://workspace-editor-leave-requested", async ({ payload }) => {
      if (payload !== "hide" && payload !== "quit") return;
      if (await requestWorkspaceEditorLeave(payload)) await completeWorkspaceEditorLeave(payload);
    })).then((stop) => {
      if (disposed) void stop();
      else unlisten = stop;
    });
    return () => {
      disposed = true;
      void unlisten?.();
    };
  }, []);

  async function saveFile(force = false): Promise<boolean> {
    if (!openFile || !dirty || saving) return !dirty;
    setSaving(true);
    setFileError("");
    setSaveStatus("Saving...");
    try {
      const result = await writeWorkspaceTextFile(workspace, openFile.path, openFile.draft, openFile.revision, force);
      if (result.status === "conflict") {
        setConflictRevision(result.revision);
        setSaveStatus("");
        return false;
      }
      setOpenFile((current) => current && current.path === result.path
        ? { ...current, content: current.draft, size: result.size, revision: result.revision }
        : current);
      setConflictRevision(null);
      setSaveStatus("Saved");
      return true;
    } catch (error) {
      setFileError(errorMessage(error));
      setSaveStatus("");
      return false;
    } finally {
      setSaving(false);
    }
  }

  async function openWorkspacePath(path: string) {
    if (openFile?.path === path && activeSource === "workspace") return;
    if (!(await requestWorkspaceEditorLeave("navigate"))) return;
    setLoadingPath(path);
    setFileError("");
    setSaveStatus("");
    setConflictRevision(null);
    try {
      const file = await readWorkspaceTextFile(workspace, path);
      setOpenFile({ ...file, draft: file.content });
      setActiveSource("workspace");
    } catch (error) {
      setFileError(errorMessage(error));
    } finally {
      setLoadingPath(null);
    }
  }

  async function selectGenerated(id: string) {
    if (!(await requestWorkspaceEditorLeave("navigate"))) return;
    setOpenFile(null);
    setActiveSource("generated");
    setFileError("");
    onSelectArtifact(id);
  }

  async function reloadConflict() {
    if (!openFile) return;
    setLoadingPath(openFile.path);
    try {
      const file = await readWorkspaceTextFile(workspace, openFile.path);
      setOpenFile({ ...file, draft: file.content });
      setConflictRevision(null);
      setFileError("");
      setSaveStatus("Reloaded");
    } catch (error) {
      setFileError(errorMessage(error));
    } finally {
      setLoadingPath(null);
    }
  }

  function toggleDirectory(path: string) {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path); else next.add(path);
      return next;
    });
    if (!directories[path]) void loadDirectory(path);
  }

  function renderDirectory(path: string, depth = 0) {
    const directory = directories[path];
    if (!directory || directory.status === "loading") return <div className="workspace-code-state">Loading...</div>;
    if (directory.status === "error") return <div className="workspace-code-state error">{directory.error}</div>;
    if (!directory.entries.length) return <div className="workspace-code-state">Empty folder</div>;
    return (
      <ul className="workspace-code-tree" style={{ "--workspace-tree-depth": depth } as React.CSSProperties}>
        {directory.entries.map((entry) => {
          const isOpen = entry.kind === "directory" && expanded.has(entry.path);
          return <li key={entry.path}>
            <button
              type="button"
              className={`workspace-code-file${openFile?.path === entry.path && activeSource === "workspace" ? " active" : ""}`}
              aria-expanded={entry.kind === "directory" ? isOpen : undefined}
              title={entry.path}
              onClick={() => entry.kind === "directory" ? toggleDirectory(entry.path) : void openWorkspacePath(entry.path)}
            >
              {entry.kind === "directory" ? <><ChevronDown className={isOpen ? "" : "collapsed"} size={11} />{isOpen ? <FolderOpen size={13} /> : <Folder size={13} />}</> : <><span className="workspace-code-chevron-space" /><FileText size={13} /></>}
              <span>{entry.name}</span>
            </button>
            {isOpen ? renderDirectory(entry.path, depth + 1) : null}
          </li>;
        })}
        {directory.cursor ? <li><button type="button" className="workspace-code-load-more" onClick={() => void loadDirectory(path, directory.cursor ?? undefined)}>Load more</button></li> : null}
      </ul>
    );
  }

  function finishLeave(leave: boolean) {
    const request = leaveRequest;
    if (!request) return;
    if (leave) setOpenFile((current) => current ? { ...current, draft: current.content } : current);
    setLeaveRequest(null);
    request.resolve(leave);
  }

  async function saveAndLeave() {
    const saved = await saveFile();
    const request = leaveRequest;
    setLeaveRequest(null);
    request?.resolve(saved);
  }

  function clampRail(width: number) {
    return Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, Math.round(width)));
  }

  function resizeRailKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft") setRailWidth((width) => clampRail(width - RAIL_KEYBOARD_STEP));
    else if (event.key === "ArrowRight") setRailWidth((width) => clampRail(width + RAIL_KEYBOARD_STEP));
    else if (event.key === "Home") setRailWidth(RAIL_MIN_WIDTH);
    else if (event.key === "End") setRailWidth(RAIL_MAX_WIDTH);
    else return;
    event.preventDefault();
  }

  function resizeRailMove(event: PointerEvent<HTMLDivElement>) {
    if (!resizeStartRef.current) return;
    setRailWidth(clampRail(resizeStartRef.current.width + event.clientX - resizeStartRef.current.x));
  }

  const searchEntries = searchQuery.trim() ? searchResults : null;
  return <div className={`workspace-code-layout${railCollapsed ? " rail-collapsed" : ""}`} style={{ "--workspace-code-rail-width": `${railWidth}px` } as React.CSSProperties}>
    <aside className="workspace-code-rail" aria-label="Code sources">
      <div className="workspace-code-rail-head">
        <strong>Code</strong>
        <button type="button" className="icon-btn" aria-label="Collapse file rail" title="Collapse file rail" onClick={() => setRailCollapsed(true)}><Sidebar size={14} /></button>
      </div>
      {workspace ? <>
        <label className="workspace-code-search">
          <Search size={13} />
          <input value={searchQuery} onChange={(event) => setSearchQuery(event.currentTarget.value)} placeholder="Search workspace" aria-label="Search workspace files" />
        </label>
        <section className="workspace-code-group" aria-labelledby="workspace-code-workspace-label">
          <h3 id="workspace-code-workspace-label">Workspace</h3>
          {searchEntries ? (
            searchStatus === "loading" ? <div className="workspace-code-state">Searching...</div>
              : searchStatus === "error" ? <div className="workspace-code-state error">{searchError}</div>
                : searchEntries.length ? <ul className="workspace-code-search-results">{searchEntries.map((entry) => <li key={entry.path}><button type="button" className={openFile?.path === entry.path && activeSource === "workspace" ? "active" : ""} title={entry.path} onClick={() => void openWorkspacePath(entry.path)}><FileText size={13} /><span>{entry.path}</span></button></li>)}</ul>
                  : <div className="workspace-code-state">No matching files</div>
          ) : renderDirectory("")}
        </section>
      </> : <div className="workspace-code-state">Choose a working folder to browse files.</div>}
      {generatedFiles.length ? <section className="workspace-code-group generated" aria-labelledby="workspace-code-generated-label">
        <h3 id="workspace-code-generated-label">Generated <span>read-only</span></h3>
        <ul>{generatedFiles.map((file) => <li key={file.artifact.id}><button type="button" className={activeSource === "generated" && selectedArtifact?.artifact.id === file.artifact.id ? "active" : ""} title={file.path} onClick={() => void selectGenerated(file.artifact.id)}><FileText size={13} /><span>{file.path}</span>{file.entry ? <small>entry</small> : null}</button></li>)}</ul>
      </section> : null}
    </aside>
    {railCollapsed ? <button type="button" className="workspace-code-rail-open icon-btn" aria-label="Show file rail" title="Show file rail" onClick={() => setRailCollapsed(false)}><Sidebar size={14} /></button> : <div
      className="workspace-code-rail-resizer"
      data-testid="workspace-code-rail-resizer"
      role="separator"
      aria-label="Resize file rail"
      aria-orientation="vertical"
      aria-valuemin={RAIL_MIN_WIDTH}
      aria-valuemax={RAIL_MAX_WIDTH}
      aria-valuenow={railWidth}
      tabIndex={0}
      onKeyDown={resizeRailKeyDown}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        resizeStartRef.current = { x: event.clientX, width: railWidth };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={resizeRailMove}
      onPointerUp={(event) => { resizeStartRef.current = null; event.currentTarget.releasePointerCapture(event.pointerId); }}
      onPointerCancel={() => { resizeStartRef.current = null; }}
    />}
    <main className="workspace-code-main">
      {activeSource === "workspace" && openFile ? <>
        <header className="workspace-code-toolbar">
          <div><strong title={openFile.path}>{openFile.path}</strong>{dirty ? <span className="workspace-code-dirty" aria-label="Unsaved changes" title="Unsaved changes" /> : null}<small>{saveStatus || (loadingPath ? "Loading..." : `${openFile.size.toLocaleString()} bytes`)}</small></div>
          <div>
            {/\.html?$/i.test(openFile.path) && onPreviewWorkspaceFile ? <button type="button" data-testid="workspace-html-preview" disabled={dirty || runtimeBusy} title={dirty ? "Save before previewing" : "Preview HTML"} onClick={() => onPreviewWorkspaceFile(openFile.path)}><Eye size={13} /> Preview</button> : null}
            <button type="button" className="primary" disabled={!dirty || saving} onClick={() => void saveFile()}>Save <kbd>⌘/Ctrl S</kbd></button>
          </div>
        </header>
        {conflictRevision ? <div className="workspace-code-conflict" role="alert"><div><strong>File changed on disk</strong><span>Your draft is safe. Reload the external version or overwrite it.</span></div><button type="button" onClick={() => void reloadConflict()}>Reload</button><button type="button" className="danger" onClick={() => void saveFile(true)}>Overwrite</button></div> : null}
        {fileError ? <div className="workspace-code-error" role="alert">{fileError}</div> : null}
        <Suspense fallback={<div className="workspace-code-state">Loading editor...</div>}><WorkspaceCodeEditor filename={openFile.path} source={openFile.draft} onChange={(draft) => { setOpenFile((current) => current ? { ...current, draft } : current); setSaveStatus(""); }} onSave={() => void saveFile()} /></Suspense>
        <span className="workspace-code-keyboard-hint">Escape then Tab moves focus out of the editor</span>
      </> : activeSource === "generated" && selectedArtifact ? <>
        <header className="workspace-code-toolbar read-only"><div><strong>{selectedArtifact.path}</strong><small>Generated artifact · Read-only</small></div></header>
        <SourceCodeView className="workspace-code-generated-source" testId="preview-code-source" source={selectedArtifact.artifact.content} language={selectedArtifact.artifact.language ?? selectedArtifact.artifact.filename?.split(".").pop()} ariaLabel={`${selectedArtifact.path} source`} />
      </> : <div className="workspace-code-empty"><FileText size={24} /><strong>Select a workspace file</strong><span>Open a file from the rail to edit it.</span></div>}
    </main>
    {leaveRequest && createPortal(<div className="git-modal-backdrop" data-native-preview-blocker="true"><section className="git-modal workspace-code-leave-dialog" role="dialog" aria-modal="true" aria-labelledby="workspace-code-leave-title"><div className="git-modal-head"><strong id="workspace-code-leave-title">Save changes?</strong></div><p>{openFile?.path} has unsaved changes.</p><div className="workspace-code-dialog-actions"><button type="button" onClick={() => finishLeave(false)}>Cancel</button><button type="button" onClick={() => finishLeave(true)}>Discard</button><button type="button" className="primary" disabled={saving} onClick={() => void saveAndLeave()}>Save</button></div></section></div>, document.body)}
  </div>;
}
