import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  SIDEBAR_CHATS_SECTION_ID,
  SIDEBAR_PINNED_SECTION_ID,
  SIDEBAR_PROJECT_SECTION_PREFIX,
  isSidebarProjectSectionId,
  projectSectionId,
  useSessions,
  type Project,
  type Session,
  type SessionPreviewRuntime,
  type SessionSidebarState,
} from "../sessions/store";
import { createInteractiveChat } from "../lib/newChatCoordinator";
import { markPerfRender } from "../lib/perf";
import { previewRuntimeKeyForThread } from "../lib/previewRuntimeKeys";
import { sessionRecencyLabel } from "../lib/sessionRecency.js";
import { chatExportFilename, sessionExportPayload, sessionMarkdownExport } from "../lib/threadExport";
import { DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, normalizeSidebarWidth, useUiPreferences } from "../ui/store";
import { GitPanel } from "./GitPanel";
import { useContextMenu } from "./ContextMenu";
import { Archive, ArrowUp, Bolt, Calendar, ChevronDown, Cube, Download, Folder, FolderOpen, Gear, GitBranch, Image, Lightbulb, MoreHorizontal, Pin, Plus, Search, Sidebar as PanelIcon } from "./icons";

const SIDEBAR_KEYBOARD_STEP = 32;
const SIDEBAR_COLLAPSE_OVERSHOOT = 96;
const SIDEBAR_SNAP_ANIMATION_MS = 180;
const SIDEBAR_DRAG_THRESHOLD = 5;
const SIDEBAR_SECTION_PREVIEW_LIMIT = 5;
const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function sidebarSectionShownCount(totalSessions: number, visibleLimit: number, activeIndex: number): number {
  const baseCount = Math.max(0, Math.min(visibleLimit, totalSessions));
  return activeIndex >= baseCount && activeIndex < totalSessions ? baseCount + 1 : baseCount;
}

export function sidebarSectionNextRevealCount(totalSessions: number, visibleLimit: number, activeIndex: number): number {
  const currentCount = sidebarSectionShownCount(totalSessions, visibleLimit, activeIndex);
  const nextLimit = Math.min(totalSessions, Math.max(0, visibleLimit) + SIDEBAR_SECTION_PREVIEW_LIMIT);
  return Math.max(0, sidebarSectionShownCount(totalSessions, nextLimit, activeIndex) - currentCount);
}

export function runningWorkerParentThreadIdsKey(records: readonly { run: { parent_thread_id: string; status: string } }[]): string {
  return [...new Set(
    records
      .filter((record) => record.run.status === "running")
      .map((record) => record.run.parent_thread_id),
  )].sort().join("\0");
}

type SidebarDragItem = { type: "session" | "section"; id: string };
type SidebarDropPosition = "before" | "after" | "inside";
type SidebarDragTarget = { type: "session"; id: string; sectionId: string; position: Exclude<SidebarDropPosition, "inside"> } | { type: "section"; id: string; position: SidebarDropPosition };
type SidebarPointerDrag = {
  item: SidebarDragItem;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
  source: HTMLElement;
  captureTarget: HTMLElement;
};
type SidebarSessionSettings = {
  folder?: string;
  model?: string;
  sandbox?: boolean;
  computerUse?: boolean;
  privacy?: string;
};

export type SidebarSessionLike = {
  id: string;
  title: string;
  settings?: SidebarSessionSettings;
  previewRuntime?: SessionPreviewRuntime;
  parentId?: string;
  updatedAt: number;
  archivedAt?: number;
  retryWorkspace?: { originalFolder: string };
  worker?: unknown;
};

type SidebarSession = Omit<Session, "messages">;

type SessionGroup<T extends SidebarSessionLike = SidebarSession> = {
  id: string;
  label: string;
  subtitle?: string;
  projectId?: string;
  sessions: T[];
};

function folderLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "Project";
}

function folderFromSectionId(sectionId: string): string {
  if (isSidebarProjectSectionId(sectionId)) {
    return sectionId.slice(SIDEBAR_PROJECT_SECTION_PREFIX.length);
  }
  return "";
}

function uniqueFolders(folders: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const folder of folders) {
    const normalized = folder.trim();
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    result.push(normalized);
  }
  return result;
}

function projectFolderForSession(session: SidebarSessionLike): string {
  return session.retryWorkspace?.originalFolder || session.settings?.folder?.trim() || "";
}

function sortBySidebarOrder<T extends SidebarSessionLike>(sessions: T[], sidebar: SessionSidebarState): T[] {
  const order = new Map(sidebar.sessionOrder.map((id, index) => [id, index]));
  return sessions.slice().sort((a, b) => {
    const aOrder = order.get(a.id);
    const bOrder = order.get(b.id);
    if (aOrder != null && bOrder != null) return aOrder - bOrder;
    if (aOrder != null) return -1;
    if (bOrder != null) return 1;
    return b.updatedAt - a.updatedAt;
  });
}

export function groupSessionsByProjects<T extends SidebarSessionLike>(sessions: T[], projects: Project[], sidebar: SessionSidebarState, query: string): Array<SessionGroup<T>> {
  const needle = query.trim().toLowerCase();
  const pinnedSessions = new Set(sidebar.pinnedSessionIds);
  const activeProjects = projects.filter((project) => !project.archivedAt);
  const projectByFolder = new Map(activeProjects.map((project) => [project.folder, project]));
  const archivedProjectFolders = new Set(projects.filter((project) => project.archivedAt).map((project) => project.folder));
  const matches = (session: SidebarSessionLike) => {
    if (!needle) return true;
    const settings = session.settings;
    const folder = projectFolderForSession(session);
    return [
      session.title,
      settings?.model,
      folder,
      folderLabel(folder),
      settings?.sandbox ? "sandbox" : "",
      settings?.computerUse ? "computer" : "",
      settings?.privacy,
    ]
      .join(" ")
      .toLowerCase()
      .includes(needle);
  };

  const isVisibleSession = (session: SidebarSessionLike) => {
    if (session.archivedAt || session.worker) return false;
    const folder = projectFolderForSession(session);
    return !folder || !archivedProjectFolders.has(folder);
  };
  const visibleAllSessions = sessions.filter(isVisibleSession);
  const topLevelSessions = visibleAllSessions.filter((session) => !session.parentId);
  const visibleSessions = topLevelSessions.filter(matches);
  const childSessionsByParent = new Map<string, T[]>();
  for (const child of sortBySidebarOrder(visibleAllSessions.filter((session) => session.parentId), sidebar)) {
    const parentId = child.parentId;
    if (!parentId) continue;
    childSessionsByParent.set(parentId, [...(childSessionsByParent.get(parentId) ?? []), child]);
  }
  const withChildren = (parents: T[]): T[] => parents.flatMap((parent) => [parent, ...(childSessionsByParent.get(parent.id) ?? [])]);
  const normalSessions = visibleSessions.filter((session) => !pinnedSessions.has(session.id));
  const folders = uniqueFolders([
    ...activeProjects.map((project) => project.folder),
    ...topLevelSessions.map(projectFolderForSession),
  ]);
  const pinnedGroups: Array<SessionGroup<T>> = [];
  const projectGroups: Array<SessionGroup<T>> = [];
  const pinned = sortBySidebarOrder(
    visibleSessions.filter((session) => pinnedSessions.has(session.id)),
    sidebar,
  );

  if (pinned.length > 0) {
    pinnedGroups.push({ id: SIDEBAR_PINNED_SECTION_ID, label: "Pinned", sessions: withChildren(pinned) });
  }

  for (const folder of folders) {
    const sectionId = projectSectionId(folder);
    const project = projectByFolder.get(folder);
    const projectSessions = sortBySidebarOrder(
      normalSessions.filter((session) => projectFolderForSession(session) === folder),
      sidebar,
    );
    const folderMatches = !needle ||
      folder.toLowerCase().includes(needle) ||
      folderLabel(folder).toLowerCase().includes(needle) ||
      (project?.name ?? "").toLowerCase().includes(needle);
    if (projectSessions.length > 0 || folderMatches) {
      projectGroups.push({ id: sectionId, projectId: project?.id ?? sectionId, label: project?.name ?? folderLabel(folder), subtitle: folder, sessions: withChildren(projectSessions) });
    }
  }

  const looseSessions = withChildren(sortBySidebarOrder(
    normalSessions.filter((session) => !projectFolderForSession(session)),
    sidebar,
  ));
  const chatGroups: Array<SessionGroup<T>> = [];
  if (looseSessions.length > 0 || !needle) {
    chatGroups.push({ id: SIDEBAR_CHATS_SECTION_ID, label: "Chats", sessions: looseSessions });
  }

  const sectionOrder = new Map(sidebar.sectionOrder.map((id, index) => [id, index]));
  projectGroups.sort((a, b) => {
    return (sectionOrder.get(a.id) ?? Number.MAX_SAFE_INTEGER) - (sectionOrder.get(b.id) ?? Number.MAX_SAFE_INTEGER);
  });
  return [...pinnedGroups, ...projectGroups, ...chatGroups];
}

function WorkingSessionLoader() {
  return <span className="loader" aria-hidden="true" />;
}

function runtimePreviewState(runtime?: SessionPreviewRuntime): "running" | "error" | null {
  const status = runtime?.status.toLowerCase();
  if (status === "error") return "error";
  return status === "installing" || status === "starting" || status === "running" ? "running" : null;
}

function runtimePreviewSidebarState(session: SidebarSessionLike): "running" | "error" | null {
  return runtimePreviewState(session.previewRuntime);
}

function sessionPreviewRuntimeForSidebar(state: ReturnType<typeof useSessions.getState>, session: Session): SessionPreviewRuntime | undefined {
  const folder = session.settings?.folder?.trim() ?? "";
  return folder ? state.previewRuntimesByKey[previewRuntimeKeyForThread(session.id, folder)] : session.previewRuntime;
}

function sameSidebarSession(a: Session, b: SidebarSession, previewRuntime?: SessionPreviewRuntime): boolean {
  return a.id === b.id &&
    a.title === b.title &&
    a.settings === b.settings &&
    previewRuntime === b.previewRuntime &&
    a.parentId === b.parentId &&
    a.worker === b.worker &&
    a.createdAt === b.createdAt &&
    a.updatedAt === b.updatedAt &&
    a.archivedAt === b.archivedAt;
}

function createSidebarSessionsSelector() {
  let previous: SidebarSession[] = [];
  return (state: ReturnType<typeof useSessions.getState>): SidebarSession[] => {
    let changed = previous.length !== state.sessions.length;
    const next = state.sessions.map((session, index) => {
      const cached = previous[index];
      const previewRuntime = sessionPreviewRuntimeForSidebar(state, session);
      if (cached && sameSidebarSession(session, cached, previewRuntime)) return cached;
      changed = true;
      const { messages: _messages, ...summary } = session;
      return { ...summary, previewRuntime };
    });
    if (!changed) return previous;
    previous = next;
    return next;
  };
}

export function Sidebar({
  open,
  onToggle,
  onOpenSettings,
  onManageSkills,
  onManageSchedules,
  onManageMedia,
  onManageMcp,
  onGitAction,
  onOpenGitPanel,
}: {
  open: boolean;
  onToggle: () => void;
  onOpenSettings: () => void;
  onManageSkills: () => void;
  onManageSchedules: () => void;
  onManageMedia: () => void;
  onManageMcp: () => void;
  onGitAction: (text: string) => void;
  onOpenGitPanel: () => void;
}) {
  markPerfRender("Sidebar");
  const { openContextMenu, openMenuAt } = useContextMenu();
  const sidebarSessionsSelector = useMemo(createSidebarSessionsSelector, []);
  const sessions = useSessions(sidebarSessionsSelector);
  const projects = useSessions((s) => s.projects);
  const previewRuntimesByKey = useSessions((s) => s.previewRuntimesByKey);
  const activeId = useSessions((s) => s.activeId);
  const generatingSessionIds = useSessions((s) => s.generatingSessionIds);
  const runningWorkerParentIdsKey = useSessions((s) => runningWorkerParentThreadIdsKey(s.workerRuns));
  const unreadSessionIds = useSessions((s) => s.unreadSessionIds);
  const sidebarState = useSessions((s) => s.sidebar);
  const switchTo = useSessions((s) => s.switchTo);
  const archiveSession = useSessions((s) => s.archiveSession);
  const archiveProject = useSessions((s) => s.archiveProject);
  const rename = useSessions((s) => s.rename);
  const addProjectFolder = useSessions((s) => s.addProjectFolder);
  const toggleSessionPinned = useSessions((s) => s.toggleSessionPinned);
  const toggleSidebarSectionCollapsed = useSessions((s) => s.toggleSidebarSectionCollapsed);
  const toggleSidebarSectionPinned = useSessions((s) => s.toggleSidebarSectionPinned);
  const moveSidebarSection = useSessions((s) => s.moveSidebarSection);
  const moveSessionInSidebar = useSessions((s) => s.moveSessionInSidebar);
  const sidebarWidth = useUiPreferences((s) => s.sidebarWidth);
  const setSidebarWidth = useUiPreferences((s) => s.setSidebarWidth);
  const newChatButtonAtBottom = useUiPreferences((s) => s.newChatButtonAtBottom);
  const toolsExpanded = useUiPreferences((s) => s.toolsExpanded);
  const setToolsExpanded = useUiPreferences((s) => s.setToolsExpanded);

  const [editing, setEditing] = useState<string | null>(null);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [confirmArchiveId, setConfirmArchiveId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const projectMenuRef = useRef<HTMLDivElement>(null);
  const sidebarElementRef = useRef<HTMLElement>(null);
  const sidebarResizeHandleRef = useRef<HTMLDivElement>(null);
  const sidebarResizeStartRef = useRef<{
    clientX: number;
    width: number;
    latestWidth: number;
    pointerId: number;
    target: HTMLDivElement;
    snappedClosed: boolean;
    resumeTimer: number | null;
  } | null>(null);
  const sidebarResizeCleanupRef = useRef<(() => void) | null>(null);
  const pointerDragRef = useRef<SidebarPointerDrag | null>(null);
  const dragOverRef = useRef<SidebarDragTarget | null>(null);
  const suppressNextClickRef = useRef(false);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [dragging, setDragging] = useState<SidebarDragItem | null>(null);
  const [dragOver, setDragOver] = useState<SidebarDragTarget | null>(null);
  const [sectionVisibleLimits, setSectionVisibleLimits] = useState<Record<string, number>>(() => ({}));

  function switchVisibleSession(id: string) {
    if (id === activeId) return;
    switchTo(id);
  }

  useEffect(() => {
    if (!projectMenuOpen && !confirmArchiveId) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target;
      if (confirmArchiveId && target instanceof HTMLElement && !target.closest(".session-side-actions")) {
        setConfirmArchiveId(null);
      }
      if (projectMenuRef.current && !projectMenuRef.current.contains(e.target as Node)) {
        setProjectMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [confirmArchiveId, projectMenuOpen]);

  function openToolsAction(action: () => void) {
    action();
  }

  function renderToolsActions(collapsedRail = false) {
    const iconSize = collapsedRail ? 15 : 13;
    const buttonClass = collapsedRail ? "icon-btn sidebar-workbench-action" : "sidebar-footer-item sidebar-workbench-action";
    const actions = [
      { key: "mcp", label: "MCP Servers", icon: <Cube size={iconSize} />, action: onManageMcp, visible: true },
      { key: "skills", label: "Skills", icon: <Lightbulb size={iconSize} />, action: onManageSkills, visible: true },
      { key: "schedules", label: "Schedules", icon: <Calendar size={iconSize} />, action: onManageSchedules, visible: true },
      { key: "media", label: "Media", icon: <Image size={iconSize} />, action: onManageMedia, visible: true },
    ];
    return actions.filter((item) => item.visible).map((item) => (
      <button
        key={item.key}
        className={buttonClass}
        type="button"
        title={collapsedRail ? item.label : undefined}
        aria-label={collapsedRail ? item.label : undefined}
        tabIndex={toolsExpanded ? undefined : -1}
        onClick={() => openToolsAction(item.action)}
      >
        {item.icon}
        {!collapsedRail && <span>{item.label}</span>}
      </button>
    ));
  }

  const groupedSessions = useMemo(() => groupSessionsByProjects(sessions, projects, sidebarState, query), [projects, query, sessions, sidebarState]);
  const runningWorkerParentThreads = useMemo(
    () => new Set(runningWorkerParentIdsKey ? runningWorkerParentIdsKey.split("\0") : []),
    [runningWorkerParentIdsKey],
  );
  const generatingSessions = useMemo(
    () => new Set([...generatingSessionIds, ...runningWorkerParentThreads]),
    [generatingSessionIds, runningWorkerParentThreads],
  );
  const unreadSessions = useMemo(() => new Set(unreadSessionIds), [unreadSessionIds]);
  const archivedProjectFoldersForStatus = useMemo(
    () => new Set(projects.filter((project) => project.archivedAt).map((project) => project.folder)),
    [projects],
  );
  const collapsedStatusSessions = useMemo(
    () => sessions.filter((session) => {
      if (session.archivedAt) return false;
      const folder = session.settings?.folder?.trim() ?? "";
      if (folder && archivedProjectFoldersForStatus.has(folder)) return false;
      return generatingSessions.has(session.id) || unreadSessions.has(session.id);
    }),
    [archivedProjectFoldersForStatus, generatingSessions, sessions, unreadSessions],
  );
  const activeSession = sessions.find((session) => session.id === activeId);
  const activeFolder = activeSession?.settings?.folder ?? "";
  const activeModel = activeSession?.settings?.model ?? "";

  function focusComposerSoon() {
    window.requestAnimationFrame(() => {
      document.querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')?.focus();
    });
  }

  function createChat(workspace?: "current" | "worktree") {
    void createInteractiveChat(undefined, workspace ? { workspace } : undefined);
    focusComposerSoon();
    setQuery("");
    setConfirmArchiveId(null);
  }

  function openNewChatMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    const trigger = event.currentTarget;
    const rect = trigger.parentElement?.getBoundingClientRect() ?? trigger.getBoundingClientRect();
    openMenuAt({ x: rect.left, y: rect.bottom + 4 }, [
      {
        id: "current-checkout",
        label: "Current checkout",
        icon: <Plus size={13} />,
        action: () => createChat("current"),
      },
      {
        id: "isolated-worktree",
        label: "Isolated worktree",
        description: "Excludes uncommitted changes",
        icon: <GitBranch size={13} />,
        action: () => createChat("worktree"),
      },
    ], "New chat workspace", trigger);
  }

  function createChatInSection(sectionId: string) {
    const folder = folderFromSectionId(sectionId);
    void createInteractiveChat({ folder });
    focusComposerSoon();
    setQuery("");
    setConfirmArchiveId(null);
  }

  function startScratchProject() {
    void createInteractiveChat({ folder: "" });
    focusComposerSoon();
    setProjectMenuOpen(false);
    setQuery("");
  }

  async function useExistingFolder() {
    if (!inTauri) {
      setProjectMenuOpen(false);
      return;
    }
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      addProjectFolder(selected);
      void createInteractiveChat({ folder: selected });
      focusComposerSoon();
      setQuery("");
    } catch {
      /* dialog unavailable */
    } finally {
      setProjectMenuOpen(false);
    }
  }

  function beginRename(id: string) {
    setEditing(id);
    setConfirmArchiveId(null);
  }

  function archiveChat(id: string) {
    if (confirmArchiveId !== id) {
      setConfirmArchiveId(id);
      return;
    }
    archiveSession(id);
    setConfirmArchiveId(null);
  }

  function branchChat(id: string) {
    useSessions.getState().forkSession(id);
    setConfirmArchiveId(null);
    focusComposerSoon();
  }

  function exportChat(id: string) {
    const session = useSessions.getState().sessions.find((item) => item.id === id);
    if (!session) return;
    const format = useUiPreferences.getState().threadExportFormat;
    const blob = new Blob(
      [format === "markdown" ? sessionMarkdownExport(session) : JSON.stringify(sessionExportPayload(session), null, 2)],
      { type: format === "markdown" ? "text/markdown" : "application/json" },
    );
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = chatExportFilename(session.title, format);
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    setConfirmArchiveId(null);
  }

  function openSessionContextMenu(event: ReactMouseEvent, session: SidebarSessionLike, pinned: boolean) {
    openContextMenu(event, [
      {
        id: "open",
        label: session.id === activeId ? "Current chat" : "Open chat",
        icon: <FolderOpen size={13} />,
        disabled: session.id === activeId,
        action: () => switchTo(session.id),
      },
      {
        id: "rename",
        label: "Rename",
        icon: <Gear size={13} />,
        action: () => beginRename(session.id),
      },
      ...(!session.parentId ? [{
        id: "pin",
        label: pinned ? "Unpin chat" : "Pin chat",
        icon: <Pin size={13} />,
        checked: pinned,
        action: () => toggleSessionPinned(session.id),
      }] : []),
      {
        id: "branch",
        label: "Branch chat",
        icon: <GitBranch size={13} />,
        separatorBefore: true,
        action: () => branchChat(session.id),
      },
      {
        id: "export",
        label: "Export chat",
        icon: <Download size={13} />,
        action: () => exportChat(session.id),
      },
      {
        id: "archive",
        label: confirmArchiveId === session.id ? "Confirm archive" : "Archive chat",
        detail: confirmArchiveId === session.id ? "Again" : undefined,
        icon: <Archive size={13} />,
        danger: true,
        separatorBefore: true,
        action: () => archiveChat(session.id),
      },
    ], session.title);
  }

  function openSectionContextMenu(event: ReactMouseEvent, group: SessionGroup, collapsed: boolean, pinned: boolean) {
    const projectSection = Boolean(group.projectId);
    openContextMenu(event, [
      {
        id: "toggle",
        label: collapsed ? "Expand section" : "Collapse section",
        icon: <ChevronDown size={13} />,
        action: () => toggleSidebarSectionCollapsed(group.id),
      },
      ...(group.id !== SIDEBAR_PINNED_SECTION_ID ? [{
        id: "new-chat",
        label: `New chat${projectSection ? " in project" : ""}`,
        icon: <Plus size={13} />,
        action: () => createChatInSection(group.id),
      }] : []),
      ...(projectSection ? [{
        id: "pin",
        label: pinned ? "Unpin section" : "Pin section",
        icon: <Pin size={13} />,
        checked: pinned,
        action: () => toggleSidebarSectionPinned(group.id),
      }] : []),
      ...(group.projectId && group.id !== SIDEBAR_CHATS_SECTION_ID ? [{
        id: "archive-project",
        label: "Archive project",
        icon: <Archive size={13} />,
        danger: true,
        separatorBefore: true,
        action: () => archiveProject(group.projectId!),
      }] : []),
    ], group.label);
  }

  function endSidebarDrag() {
    pointerDragRef.current = null;
    dragOverRef.current = null;
    setDragging(null);
    setDragOver(null);
  }

  function setSidebarDragOver(target: SidebarDragTarget | null) {
    dragOverRef.current = target;
    setDragOver(target);
  }

  function dropPositionFromElement(clientY: number, element: HTMLElement): Exclude<SidebarDropPosition, "inside"> {
    const rect = element.getBoundingClientRect();
    return clientY > rect.top + rect.height / 2 ? "after" : "before";
  }

  function sidebarDropTargetFromPoint(clientX: number, clientY: number, item: SidebarDragItem): SidebarDragTarget | null {
    const element = document.elementFromPoint(clientX, clientY);
    if (!(element instanceof HTMLElement)) return null;

    if (item.type === "session") {
      const sessionElement = element.closest<HTMLElement>("[data-sidebar-session-id]");
      if (sessionElement) {
        const id = sessionElement.dataset.sidebarSessionId;
        const sectionId = sessionElement.dataset.sidebarSessionSectionId;
        if (!id || !sectionId || id === item.id) return null;
        return { type: "session", id, sectionId, position: dropPositionFromElement(clientY, sessionElement) };
      }

      const sectionElement = element.closest<HTMLElement>("[data-sidebar-section-id]");
      const sectionId = sectionElement?.dataset.sidebarSectionId;
      return sectionId ? { type: "section", id: sectionId, position: "inside" } : null;
    }

    if (!isSidebarProjectSectionId(item.id)) return null;
    const sectionElement = element.closest<HTMLElement>("[data-sidebar-section-id]");
    const sectionId = sectionElement?.dataset.sidebarSectionId;
    if (!sectionElement || !sectionId || sectionId === item.id || !isSidebarProjectSectionId(sectionId)) return null;
    return { type: "section", id: sectionId, position: dropPositionFromElement(clientY, sectionElement) };
  }

  function applySidebarDrop(item: SidebarDragItem, target: SidebarDragTarget | null) {
    if (!target) return;
    if (item.type === "session" && target.type === "session") {
      moveSessionInSidebar(item.id, target.id, target.sectionId, target.position);
    } else if (item.type === "session" && target.type === "section") {
      moveSessionInSidebar(item.id, null, target.id, "inside");
    } else if (
      item.type === "section" &&
      target.type === "section" &&
      target.position !== "inside" &&
      isSidebarProjectSectionId(item.id) &&
      isSidebarProjectSectionId(target.id)
    ) {
      moveSidebarSection(item.id, target.id, target.position);
    }
  }

  function isSidebarDragInteractiveTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    const interactive = target.closest("button, input, textarea, select, [role='menu'], .session-menu");
    return Boolean(interactive && !interactive.classList.contains("section-toggle"));
  }

  function consumeSuppressedClick(): boolean {
    if (!suppressNextClickRef.current) return false;
    suppressNextClickRef.current = false;
    return true;
  }

  function cleanupPointerDragListeners() {
    const drag = pointerDragRef.current;
    if (drag) {
      drag.source.style.removeProperty("pointer-events");
      drag.source.style.removeProperty("translate");
      drag.source.style.removeProperty("will-change");
      if (drag.captureTarget.hasPointerCapture(drag.pointerId)) {
        drag.captureTarget.releasePointerCapture(drag.pointerId);
      }
    }
    window.removeEventListener("pointermove", moveSidebarPointerDrag);
    window.removeEventListener("pointerup", endSidebarPointerDrag);
    window.removeEventListener("pointercancel", cancelSidebarPointerDrag);
    document.body.classList.remove("sidebar-pointer-dragging");
  }

  function startPointerDrag(event: ReactPointerEvent<HTMLElement>, item: SidebarDragItem) {
    if (event.button !== 0 || editing || isSidebarDragInteractiveTarget(event.target)) return;
    if (item.type === "section" && !isSidebarProjectSectionId(item.id)) return;
    const source = event.currentTarget.closest<HTMLElement>(
      item.type === "section" ? "[data-sidebar-section-id]" : "[data-sidebar-session-id]",
    );
    if (!source) return;
    pointerDragRef.current = {
      item,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
      source,
      captureTarget: event.currentTarget,
    };
    setConfirmArchiveId(null);
    setSidebarDragOver(null);
    window.addEventListener("pointermove", moveSidebarPointerDrag, { passive: false });
    window.addEventListener("pointerup", endSidebarPointerDrag);
    window.addEventListener("pointercancel", cancelSidebarPointerDrag);
  }

  function moveSidebarPointerDrag(event: PointerEvent) {
    const drag = pointerDragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    const moved = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
    if (!drag.active && moved < SIDEBAR_DRAG_THRESHOLD) return;
    if (!drag.active) {
      drag.active = true;
      drag.captureTarget.setPointerCapture(drag.pointerId);
      drag.source.style.pointerEvents = "none";
      drag.source.style.willChange = "translate";
      document.body.classList.add("sidebar-pointer-dragging");
      setDragging(drag.item);
    }
    event.preventDefault();
    drag.source.style.translate = `0 ${event.clientY - drag.startY}px`;
    setSidebarDragOver(sidebarDropTargetFromPoint(event.clientX, event.clientY, drag.item));
  }

  function endSidebarPointerDrag(event: PointerEvent) {
    const drag = pointerDragRef.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    cleanupPointerDragListeners();
    if (drag.active) {
      event.preventDefault();
      suppressNextClickRef.current = true;
      window.setTimeout(() => {
        suppressNextClickRef.current = false;
      }, 120);
      applySidebarDrop(drag.item, sidebarDropTargetFromPoint(event.clientX, event.clientY, drag.item) ?? dragOverRef.current);
    }
    endSidebarDrag();
  }

  function cancelSidebarPointerDrag(event: PointerEvent) {
    const drag = pointerDragRef.current;
    if (drag && event.pointerId !== drag.pointerId) return;
    cleanupPointerDragListeners();
    endSidebarDrag();
  }

  const resolvedSidebarWidth = normalizeSidebarWidth(
    sidebarResizeStartRef.current?.latestWidth ?? sidebarWidth,
  );
  const sidebarStyle = {
    "--sidebar-width": `${resolvedSidebarWidth}px`,
  } as CSSProperties;
  const newChatPrimaryButton = (
    <button
      type="button"
      className="new-chat"
      title="New chat using the configured workspace preference"
      onClick={() => createChat()}
    >
      <Plus size={16} />
      <span>New chat</span>
    </button>
  );
  const newChatButton = activeFolder ? (
    <div className="new-chat-actions">
      {newChatPrimaryButton}
      <button
        type="button"
        className="new-chat new-chat-menu"
        title="Choose current checkout or isolated worktree"
        aria-label="Choose new chat workspace"
        aria-haspopup="menu"
        onClick={openNewChatMenu}
      >
        <ChevronDown size={14} />
      </button>
    </div>
  ) : (
    newChatPrimaryButton
  );

  function resizeSidebar(width: number) {
    setSidebarWidth(normalizeSidebarWidth(width));
  }

  function resizeSidebarDuringDrag(width: number) {
    const start = sidebarResizeStartRef.current;
    if (!start) return;
    const nextWidth = normalizeSidebarWidth(width);
    start.latestWidth = nextWidth;
    sidebarElementRef.current?.style.setProperty("--sidebar-width", `${nextWidth}px`);
    sidebarResizeHandleRef.current?.setAttribute("aria-valuenow", String(nextWidth));
  }

  function startSidebarResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget;
    sidebarResizeStartRef.current = {
      clientX: event.clientX,
      width: resolvedSidebarWidth,
      latestWidth: resolvedSidebarWidth,
      pointerId: event.pointerId,
      target,
      snappedClosed: false,
      resumeTimer: null,
    };
    setSidebarResizing(true);
    target.setPointerCapture(event.pointerId);
    const move = (nextEvent: PointerEvent) => moveSidebarResize(nextEvent);
    const end = (nextEvent: PointerEvent) => endSidebarResize(nextEvent);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    sidebarResizeCleanupRef.current = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      sidebarResizeCleanupRef.current = null;
    };
  }

  function moveSidebarResize(event: PointerEvent) {
    const start = sidebarResizeStartRef.current;
    if (!start || event.pointerId !== start.pointerId) return;
    const width = start.width + event.clientX - start.clientX;
    if (width < MIN_SIDEBAR_WIDTH - SIDEBAR_COLLAPSE_OVERSHOOT) {
      if (!start.snappedClosed) {
        start.snappedClosed = true;
        if (start.resumeTimer != null) window.clearTimeout(start.resumeTimer);
        start.resumeTimer = null;
        setSidebarResizing(false);
        onToggle();
      }
      return;
    }
    if (start.snappedClosed) {
      start.snappedClosed = false;
      start.latestWidth = normalizeSidebarWidth(width);
      onToggle();
      start.resumeTimer = window.setTimeout(() => {
        if (sidebarResizeStartRef.current === start && !start.snappedClosed) {
          setSidebarResizing(true);
        }
        start.resumeTimer = null;
      }, SIDEBAR_SNAP_ANIMATION_MS);
    }
    resizeSidebarDuringDrag(width);
  }

  function endSidebarResize(event: PointerEvent) {
    const start = sidebarResizeStartRef.current;
    if (!start || event.pointerId !== start.pointerId) return;
    if (start.latestWidth !== start.width) setSidebarWidth(start.latestWidth);
    sidebarResizeStartRef.current = null;
    if (start.resumeTimer != null) window.clearTimeout(start.resumeTimer);
    sidebarResizeCleanupRef.current?.();
    setSidebarResizing(false);
    if (start.target.hasPointerCapture(event.pointerId)) {
      start.target.releasePointerCapture(event.pointerId);
    }
  }

  function resizeSidebarWithKeyboard(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      resizeSidebar(resolvedSidebarWidth - SIDEBAR_KEYBOARD_STEP);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      resizeSidebar(resolvedSidebarWidth + SIDEBAR_KEYBOARD_STEP);
    } else if (event.key === "Home") {
      event.preventDefault();
      resizeSidebar(MIN_SIDEBAR_WIDTH);
    } else if (event.key === "End") {
      event.preventDefault();
      resizeSidebar(MAX_SIDEBAR_WIDTH);
    } else if (event.key === "Enter") {
      event.preventDefault();
      resizeSidebar(DEFAULT_SIDEBAR_WIDTH);
    }
  }

  if (!open) {
    return (
      <aside className="sidebar collapsed" aria-label="Chats">
        <div className="sidebar-rail" data-testid="sidebar-rail">
          <div className="sidebar-rail-main">
            <button className="icon-btn" title="Expand sidebar" onClick={onToggle}>
              <PanelIcon size={16} />
            </button>
            <button className="icon-btn" title="New chat" onClick={() => createChat()}>
              <Plus size={16} />
            </button>
            <button className="icon-btn" title="Search chats" onClick={onToggle}>
              <Search size={15} />
            </button>
            {collapsedStatusSessions.length > 0 && (
              <div className="sidebar-rail-status" aria-label="Thread activity">
                {collapsedStatusSessions.map((session) => {
                  const generating = generatingSessions.has(session.id);
                  const statusLabel = runningWorkerParentThreads.has(session.id) ? "Workers running" : generating ? "Working" : "Unread update";
                  const active = session.id === activeId;
                  return (
                    <button
                      key={session.id}
                      className={"icon-btn sidebar-status-btn" + (active ? " active" : "")}
                      type="button"
                      title={`${statusLabel}: ${session.title}`}
                      aria-label={`Open ${statusLabel.toLowerCase()} thread: ${session.title}`}
                      onClick={() => switchVisibleSession(session.id)}
                    >
                      <span className={"session-loader " + (generating ? "working" : "unread")} aria-hidden="true">
                        {generating ? <WorkingSessionLoader /> : <span className="loader" />}
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          <div className="sidebar-rail-bottom">
            <div className={"sidebar-workbench-wrap collapsed" + (toolsExpanded ? " expanded" : "")}>
              <div className="sidebar-workbench-reveal collapsed" aria-label="Tools" aria-hidden={!toolsExpanded}>
                <div className="sidebar-workbench-inline collapsed">{renderToolsActions(true)}</div>
              </div>
              <button
                className="icon-btn sidebar-workbench-toggle"
                title={toolsExpanded ? "Collapse Tools" : "Tools"}
                onClick={() => setToolsExpanded(!toolsExpanded)}
                aria-expanded={toolsExpanded}
              >
                <Bolt size={15} />
              </button>
            </div>
            <button className="icon-btn" data-testid="open-settings" title="Settings" onClick={onOpenSettings}>
              <Gear size={15} />
            </button>
          </div>
        </div>
      </aside>
    );
  }

  return (
    <aside ref={sidebarElementRef} className={"sidebar" + (sidebarResizing ? " resizing" : "")} aria-label="Chats" style={sidebarStyle}>
      <div className="sidebar-inner">
        <div className="sidebar-head">
          <button className="icon-btn" title="Collapse sidebar" onClick={onToggle}>
            <PanelIcon size={16} />
          </button>
          <label className="sidebar-search">
            <Search size={14} />
            <input
              data-testid="sidebar-search"
              value={query}
              placeholder="Search chats..."
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <div className={"projects-actions" + (projectMenuOpen ? " open" : "")} ref={projectMenuRef}>
            <button
              className="section-icon-btn"
              type="button"
              title="New project or folder"
              aria-label="New project or folder"
              data-testid="project-menu-trigger"
              onClick={() => setProjectMenuOpen((open) => !open)}
            >
              <Plus size={13} />
            </button>
            {projectMenuOpen && (
              <div className="session-menu project-menu" role="menu" aria-label="Project actions">
                <button type="button" role="menuitem" onClick={startScratchProject}>
                  <Plus size={13} />
                  <span>Start from scratch</span>
                </button>
                <button type="button" role="menuitem" onClick={() => void useExistingFolder()}>
                  <Folder size={13} />
                  <span>Use an existing folder</span>
                </button>
              </div>
            )}
          </div>
        </div>

        {!newChatButtonAtBottom && newChatButton}

        <div className="session-list">
          {groupedSessions.length === 0 ? (
            <div className="session-empty">No chats found</div>
          ) : (
            groupedSessions.map((group) => {
              const collapsed = sidebarState.collapsedSectionIds.includes(group.id);
              const projectSection = isSidebarProjectSectionId(group.id);
              const sectionPinned = group.id === SIDEBAR_PINNED_SECTION_ID || sidebarState.pinnedSectionIds.includes(group.id);
              const sectionDragOver = dragOver?.type === "section" && dragOver.id === group.id;
              const sectionDropClass = sectionDragOver ? ` drag-over drop-${dragOver.position}` : "";
              const sectionDragging = dragging?.type === "section" && dragging.id === group.id;
              const sectionRuntimeState = projectSection
                ? runtimePreviewState(previewRuntimesByKey[previewRuntimeKeyForThread("", group.subtitle ?? folderFromSectionId(group.id))])
                : null;
              const searchActive = Boolean(query.trim());
              const totalSessions = group.sessions.length;
              const visibleLimit = searchActive
                ? totalSessions
                : Math.min(sectionVisibleLimits[group.id] ?? SIDEBAR_SECTION_PREVIEW_LIMIT, totalSessions);
              const baseVisibleSessions = group.sessions.slice(0, visibleLimit);
              const activeIndex = group.sessions.findIndex((session) => session.id === activeId);
              const visibleSessions = !searchActive && activeIndex >= visibleLimit
                ? [...baseVisibleSessions, group.sessions[activeIndex]]
                : baseVisibleSessions;
              const currentShownCount = visibleSessions.length;
              const nextRevealCount = sidebarSectionNextRevealCount(totalSessions, visibleLimit, activeIndex);
              const canShowMore = !searchActive && nextRevealCount > 0;
              const canShowLess = !searchActive && visibleLimit > SIDEBAR_SECTION_PREVIEW_LIMIT;
              const sectionManuallyExpanded = visibleLimit > SIDEBAR_SECTION_PREVIEW_LIMIT;
              const shownCountLabel = `${currentShownCount} of ${totalSessions} shown`;
              const showMoreLabel = `Show ${nextRevealCount} more ${nextRevealCount === 1 ? "thread" : "threads"} in ${group.label}, ${shownCountLabel}`;
              const showLessLabel = `Show fewer threads in ${group.label}, ${shownCountLabel}`;
              const showMoreButton = canShowMore ? (
                <button
                  className="session-more-btn more"
                  type="button"
                  title={showMoreLabel}
                  aria-label={showMoreLabel}
                  aria-expanded={sectionManuallyExpanded}
                  onClick={() => setSectionVisibleLimits((current) => {
                    const currentLimit = Math.min(current[group.id] ?? SIDEBAR_SECTION_PREVIEW_LIMIT, totalSessions);
                    const nextLimit = Math.min(totalSessions, currentLimit + SIDEBAR_SECTION_PREVIEW_LIMIT);
                    const next = { ...current, [group.id]: nextLimit };
                    return next;
                  })}
                >
                  <MoreHorizontal size={16} />
                  <span>+{nextRevealCount}</span>
                </button>
              ) : null;
              const showLessButton = canShowLess ? (
                <button
                  className="session-more-btn expanded"
                  type="button"
                  title={showLessLabel}
                  aria-label={showLessLabel}
                  aria-expanded={sectionManuallyExpanded}
                  onClick={() => setSectionVisibleLimits((current) => {
                    const currentLimit = Math.min(current[group.id] ?? SIDEBAR_SECTION_PREVIEW_LIMIT, totalSessions);
                    const nextLimit = Math.max(SIDEBAR_SECTION_PREVIEW_LIMIT, currentLimit - SIDEBAR_SECTION_PREVIEW_LIMIT);
                    const next = new Map(Object.entries(current));
                    if (nextLimit === SIDEBAR_SECTION_PREVIEW_LIMIT) next.delete(group.id);
                    else next.set(group.id, nextLimit);
                    return Object.fromEntries(next);
                  })}
                >
                  <ArrowUp size={14} />
                </button>
              ) : null;
              return (
                <section
                  key={group.id}
                  data-sidebar-section-id={group.id}
                  className={
                    "session-section" +
                    (collapsed ? " collapsed" : "") +
                    (sectionPinned ? " pinned" : "") +
                    (!projectSection ? " fixed" : "") +
                    (sectionDragging ? " dragging" : "") +
                    sectionDropClass
                  }
                >
                  <div
                    className={"session-section-title" + (!projectSection ? " fixed" : "") + (sectionRuntimeState ? " runtime-preview runtime-" + sectionRuntimeState : "")}
                    onPointerDown={projectSection ? (event) => startPointerDrag(event, { type: "section", id: group.id }) : undefined}
                    onContextMenu={(event) => openSectionContextMenu(event, group, collapsed, sectionPinned)}
                  >
                    <button
                      className="section-toggle"
                      type="button"
                      aria-label={`${collapsed ? "Expand" : "Collapse"} ${group.label}`}
                      aria-expanded={!collapsed}
                      onClick={() => {
                        if (consumeSuppressedClick()) return;
                        toggleSidebarSectionCollapsed(group.id);
                      }}
                    >
                      <span className={"section-type-icon" + (group.id === SIDEBAR_CHATS_SECTION_ID ? " chat-toggle-icon" : "")} aria-hidden="true">
                        {group.id === SIDEBAR_PINNED_SECTION_ID ? <Pin size={12} /> : group.id === SIDEBAR_CHATS_SECTION_ID ? <ChevronDown size={12} /> : collapsed ? <Folder size={12} /> : <FolderOpen size={12} />}
                      </span>
                      <span className="section-copy" title={group.subtitle ?? group.label}>
                        <span className="section-label-row">
                          <span className="section-label">{group.label}</span>
                        </span>
                      </span>
                    </button>
                    <div className="section-actions-inline">
                      {projectSection && (
                        <button
                          className={"section-icon-btn" + (sectionPinned ? " active" : "")}
                          type="button"
                          title={sectionPinned ? "Unpin section" : "Pin section"}
                          aria-label={sectionPinned ? `Unpin ${group.label}` : `Pin ${group.label}`}
                          aria-pressed={sectionPinned}
                          onClick={() => toggleSidebarSectionPinned(group.id)}
                        >
                          <Pin size={12} />
                        </button>
                      )}
                      {group.id !== SIDEBAR_PINNED_SECTION_ID && (
                        <button
                          className="section-icon-btn"
                          type="button"
                          title={`New chat in ${group.label}`}
                          aria-label={`New chat in ${group.label}`}
                          onClick={() => createChatInSection(group.id)}
                        >
                          <Plus size={12} />
                        </button>
                      )}
                    </div>
                  </div>
                  <div
                    className="context-section-reveal"
                    data-collapsed={collapsed || undefined}
                    aria-hidden={collapsed}
                  >
                    <div className="context-section-inner session-section-content">
                  {visibleSessions.map((s) => {
                  const parentWorkersRunning = runningWorkerParentThreads.has(s.id);
                  const workerRunning = parentWorkersRunning || s.worker?.status === "queued" || s.worker?.status === "running";
                  const generating = generatingSessions.has(s.id) || workerRunning;
                  const unread = unreadSessions.has(s.id);
                  const runtimeState = projectSection ? null : runtimePreviewSidebarState(s);
                  const pinned = !s.parentId && sidebarState.pinnedSessionIds.includes(s.id);
                  const showStatus = generating || unread;
                  const statusLabel = parentWorkersRunning ? "Workers running" : s.worker ? `Worker ${s.worker.status}` : generating ? "Working" : unread ? "Unread update" : "Ready";
                  const sessionDragOver = dragOver?.type === "session" && dragOver.id === s.id;
                  const sessionDropClass = sessionDragOver ? ` drag-over drop-${dragOver.position}` : "";
                  const sessionDragging = dragging?.type === "session" && dragging.id === s.id;
                  return (
                    <div
                      key={s.id}
                      data-sidebar-session-id={s.id}
                      data-sidebar-session-section-id={group.id}
                      className={
                        "session-item" +
                        (s.parentId ? " child-session" : "") +
                        (s.id === activeId ? " active" : "") +
                        (generating ? " generating" : "") +
                        (runtimeState ? " runtime-preview runtime-" + runtimeState : "") +
                        (pinned ? " pinned" : "") +
                        (confirmArchiveId === s.id ? " delete-pending" : "") +
                        (sessionDragging ? " dragging" : "") +
                        sessionDropClass
                      }
                      onPointerDown={s.parentId ? undefined : (event) => startPointerDrag(event, { type: "session", id: s.id })}
                      onContextMenu={(event) => openSessionContextMenu(event, s, pinned)}
                      onClick={(event) => {
                        if (isSidebarDragInteractiveTarget(event.target)) return;
                        if (consumeSuppressedClick()) return;
                        setConfirmArchiveId(null);
                        switchVisibleSession(s.id);
                      }}
                      onDoubleClick={(event) => {
                        if (isSidebarDragInteractiveTarget(event.target)) return;
                        beginRename(s.id);
                      }}
                      title={s.title}
                    >
                      {editing === s.id ? (
                        <input
                          className="session-rename"
                          defaultValue={s.title}
                          autoFocus
                          onClick={(e) => e.stopPropagation()}
                          onBlur={(e) => {
                            rename(s.id, e.target.value.trim());
                            setEditing(null);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                            if (e.key === "Escape") setEditing(null);
                          }}
                        />
                      ) : (
                        <>
                          <span className="session-copy">
                            <span className={"session-title" + (generating ? " shiny-text" : "")}>{s.title}</span>
                          </span>
                          <div className="session-side">
                            {showStatus ? (
                              <span
                                className={
                                  "session-side-indicator session-loader " + (generating ? "working" : "unread")
                                }
                                data-testid="session-loader"
                                role="img"
                                title={statusLabel}
                                aria-label={statusLabel}
                              >
                                {generating ? (
                                  <WorkingSessionLoader />
                                ) : (
                                  <span className="loader" aria-hidden="true" />
                                )}
                              </span>
                            ) : (
                              <span
                                className="session-side-indicator session-recency"
                                data-testid="session-recency"
                                title={`Updated ${new Date(s.updatedAt).toLocaleString()}`}
                              >
                                {sessionRecencyLabel(s.updatedAt)}
                              </span>
                            )}
                            <div className="session-side-actions" aria-label="Thread actions">
                              {!s.parentId && (
                                <button
                                  className={"session-side-btn" + (pinned ? " active" : "")}
                                  type="button"
                                  aria-label={pinned ? `Unpin ${s.title}` : `Pin ${s.title}`}
                                  title={pinned ? "Unpin chat" : "Pin chat"}
                                  aria-pressed={pinned}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    setConfirmArchiveId(null);
                                    toggleSessionPinned(s.id);
                                  }}
                                >
                                  <Pin size={12} />
                                </button>
                              )}
                              <button
                                className={"session-side-btn danger" + (confirmArchiveId === s.id ? " confirm" : "")}
                                type="button"
                                aria-label={confirmArchiveId === s.id ? `Confirm archive ${s.title}` : `Archive ${s.title}`}
                                title={confirmArchiveId === s.id ? "Click again to archive" : "Archive chat"}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  archiveChat(s.id);
                                }}
                              >
                                <Archive size={12} />
                              </button>
                            </div>
                          </div>
                        </>
                      )}
                    </div>
                  );
                })}
                  {(canShowMore || canShowLess) && (
                    <div className="session-more-row">
                      {showMoreButton ?? <span className="session-more-spacer" aria-hidden="true" />}
                      {showLessButton}
                    </div>
                  )}
                  {group.sessions.length === 0 && group.id !== SIDEBAR_PINNED_SECTION_ID && (
                    <button
                      className={"session-empty project-empty" + (group.id === SIDEBAR_CHATS_SECTION_ID ? " chat-empty" : "")}
                      type="button"
                      onClick={() => createChatInSection(group.id)}
                    >
                      {group.id === SIDEBAR_CHATS_SECTION_ID ? "Start a chat" : "New chat in this project"}
                    </button>
                  )}
                    </div>
                  </div>
                </section>
              );
            })
          )}
        </div>

        <GitPanel folder={activeFolder} model={activeModel} onDraftAction={onGitAction} onOpenPanel={onOpenGitPanel} />

        {newChatButtonAtBottom && newChatButton}

        <div className="sidebar-footer" aria-label="App controls">
          <div className={"sidebar-workbench-wrap" + (toolsExpanded ? " expanded" : "")}>
            <div className="sidebar-workbench-reveal" aria-label="Tools" aria-hidden={!toolsExpanded}>
              <div className="sidebar-workbench-inline">{renderToolsActions()}</div>
            </div>
            <button
              className="sidebar-footer-item sidebar-workbench-toggle"
              data-testid="open-tools"
              type="button"
              onClick={() => setToolsExpanded(!toolsExpanded)}
              aria-expanded={toolsExpanded}
            >
              <Bolt size={14} />
              <span>Tools</span>
              <ChevronDown className="sidebar-workbench-caret" size={13} />
            </button>
          </div>
          <button className="sidebar-footer-item" data-testid="open-settings" type="button" onClick={onOpenSettings}>
            <Gear size={14} />
            <span>Settings</span>
          </button>
        </div>
      </div>
      <div
        ref={sidebarResizeHandleRef}
        className={`sidebar-resize-handle${sidebarResizing ? " dragging" : ""}`}
        data-testid="sidebar-resize-handle"
        role="separator"
        aria-label="Resize thread sidebar; double-click or press Enter to reset"
        title="Drag to resize; double-click to reset"
        aria-orientation="vertical"
        aria-valuemin={MIN_SIDEBAR_WIDTH}
        aria-valuemax={MAX_SIDEBAR_WIDTH}
        aria-valuenow={resolvedSidebarWidth}
        tabIndex={0}
        onKeyDown={resizeSidebarWithKeyboard}
        onPointerDown={startSidebarResize}
        onDoubleClick={() => resizeSidebar(DEFAULT_SIDEBAR_WIDTH)}
      />
    </aside>
  );
}
