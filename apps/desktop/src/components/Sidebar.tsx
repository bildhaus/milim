import { useEffect, useMemo, useRef, useState, type CSSProperties, type HTMLAttributes, type KeyboardEvent, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import {
  SIDEBAR_CHATS_SECTION_ID,
  SIDEBAR_PINNED_SECTION_ID,
  SIDEBAR_PROJECT_SECTION_PREFIX,
  isSidebarProjectSectionId,
  projectSectionId,
  useSessions,
  type Project,
  type ProjectIconId,
  type Session,
  type SessionPreviewRuntime,
  type SessionSidebarState,
} from "../sessions/store";
import { openWorkspaceLauncher, runWorkspaceGitAction } from "../api";
import { createInteractiveChat } from "../lib/newChatCoordinator";
import { requestWorkspaceEditorLeave } from "../lib/workspaceEditorGuard";
import { GIT_STATUS_REFRESH_INTERVAL_MS } from "../lib/gitRefresh";
import { markPerfRender } from "../lib/perf";
import { previewRuntimeKeyForThread } from "../lib/previewRuntimeKeys";
import {
  effectiveProjectColor,
  normalizeProjectColor,
} from "../lib/projectColors";
import {
  pullRequestAccessibleLabel,
  pullRequestReadiness,
  type PullRequestSnapshot,
} from "../lib/pullRequests";
import { sessionRecencyLabel } from "../lib/sessionRecency.js";
import { chatExportFilename, sessionExportPayload, sessionMarkdownExport } from "../lib/threadExport";
import { DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, normalizeSidebarWidth, useUiPreferences } from "../ui/store";
import { useTheme } from "../theme/store";
import { GitPanel, type GitPanelView } from "./GitPanel";
import { useContextMenu } from "./ContextMenu";
import { HoverScrollText } from "./HoverScrollText";
import { SheetDialog } from "./SheetDialog";
import { ColorField } from "./ui";
import { Archive, ArrowUp, Bolt, Calendar, Check, ChevronDown, Code, Cube, Download, FileText, Folder, FolderOpen, Gear, GitBranch, GitPullRequest, Globe, Image, Lightbulb, MoreHorizontal, Pin, Plus, Search, Sidebar as PanelIcon, Star, Terminal } from "./icons";

const SIDEBAR_KEYBOARD_STEP = 32;
const SIDEBAR_COLLAPSE_OVERSHOOT = 96;
const SIDEBAR_SNAP_ANIMATION_MS = 180;
const SIDEBAR_DRAG_THRESHOLD = 5;
const SIDEBAR_SECTION_PREVIEW_LIMIT = 5;
const SIDEBAR_INBOX_SECTION_ID = "inbox";
const SIDEBAR_SETTLED_SECTION_ID = "settled";
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
  settledAt?: number;
  archivedAt?: number;
  retryWorkspace?: { originalFolder: string };
  threadWorkspace?: {
    mode: "current" | "worktree";
    projectFolder?: string;
    branch?: string;
  };
  worker?: Session["worker"];
};

type SidebarSession = Omit<Session, "messages">;

type SessionGroup<T extends SidebarSessionLike = SidebarSession> = {
  id: string;
  label: string;
  subtitle?: string;
  projectId?: string;
  project?: Project;
  sessions: T[];
  inbox?: boolean;
  settled?: boolean;
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
  return session.retryWorkspace?.originalFolder ||
    (session.threadWorkspace?.mode === "worktree"
      ? session.threadWorkspace.projectFolder?.trim()
      : session.settings?.folder?.trim()) ||
    session.settings?.folder?.trim() ||
    "";
}

type SidebarPullRequestOwner<T extends SidebarSessionLike> = {
  session: T;
  snapshot: PullRequestSnapshot;
  pullRequest: NonNullable<PullRequestSnapshot["pullRequest"]>;
};

function pullRequestOwnerForSession<T extends SidebarSessionLike>(
  session: T,
  pullRequestsBySession: Record<string, PullRequestSnapshot>,
): SidebarPullRequestOwner<T> | undefined {
  const snapshot = pullRequestsBySession[session.id];
  const folder = session.settings?.folder?.trim() ?? "";
  const pullRequest =
    snapshot?.folder === folder &&
    (!session.threadWorkspace?.branch ||
      snapshot.pullRequest?.headRefName === session.threadWorkspace.branch)
      ? snapshot.pullRequest
      : null;
  return pullRequest ? { session, snapshot, pullRequest } : undefined;
}

export function sidebarProjectPullRequestOwner<T extends SidebarSessionLike>(
  group: SessionGroup<T>,
  pullRequestsBySession: Record<string, PullRequestSnapshot>,
): SidebarPullRequestOwner<T> | undefined {
  if (!group.projectId) return undefined;
  let newest: SidebarPullRequestOwner<T> | undefined;
  for (const session of group.sessions) {
    const folder = session.settings?.folder?.trim() ?? "";
    if (!folder || projectFolderForSession(session) !== folder) continue;
    const owner = pullRequestOwnerForSession(session, pullRequestsBySession);
    if (owner && (!newest || owner.snapshot.checkedAt > newest.snapshot.checkedAt))
      newest = owner;
  }
  return newest;
}

export function sidebarThreadPullRequestOwner<T extends SidebarSessionLike>(
  session: T,
  pullRequestsBySession: Record<string, PullRequestSnapshot>,
): SidebarPullRequestOwner<T> | undefined {
  const folder = session.settings?.folder?.trim() ?? "";
  if (!folder || projectFolderForSession(session) === folder) return undefined;
  return pullRequestOwnerForSession(session, pullRequestsBySession);
}

export function sidebarInboxPullRequestOwner<T extends SidebarSessionLike>(
  session: T,
  pullRequestsBySession: Record<string, PullRequestSnapshot>,
): SidebarPullRequestOwner<T> | undefined {
  return pullRequestOwnerForSession(session, pullRequestsBySession);
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

function sortByRecentActivity<T extends SidebarSessionLike>(sessions: T[]): T[] {
  return sessions.slice().sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id));
}

function sortBySettledActivity<T extends SidebarSessionLike>(sessions: T[]): T[] {
  return sessions.slice().sort(
    (a, b) =>
      (b.settledAt ?? 0) - (a.settledAt ?? 0) ||
      b.updatedAt - a.updatedAt ||
      a.id.localeCompare(b.id),
  );
}

export function nextInboxSessionIdAfterSettle<T extends SidebarSessionLike>(
  groups: Array<SessionGroup<T>>,
  currentId: string,
): string | undefined {
  return sortByRecentActivity(
    groups
      .filter((group) => group.inbox && !group.settled)
      .flatMap((group) => group.sessions)
      .filter((session) => session.id !== currentId),
  )[0]?.id;
}

export function groupSessionsByProjects<T extends SidebarSessionLike>(
  sessions: T[],
  projects: Project[],
  sidebar: SessionSidebarState,
  query: string,
  settledThreadsEnabled = false,
): Array<SessionGroup<T>> {
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
      projectByFolder.get(folder)?.name,
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
  if (settledThreadsEnabled) {
    const visibleSessions = visibleAllSessions.filter(matches);
    const activeSessions = visibleSessions.filter((session) => !session.settledAt);
    const pinned = sortByRecentActivity(
      activeSessions.filter((session) => pinnedSessions.has(session.id)),
    );
    const inbox = sortByRecentActivity(
      activeSessions.filter((session) => !pinnedSessions.has(session.id)),
    );
    const settled = sortBySettledActivity(
      visibleSessions.filter((session) => session.settledAt),
    );
    return [
      ...(pinned.length
        ? [{
            id: SIDEBAR_PINNED_SECTION_ID,
            label: "Pinned",
            sessions: pinned,
            inbox: true,
          }]
        : []),
      ...(inbox.length
        ? [{
            id: SIDEBAR_INBOX_SECTION_ID,
            label: "Inbox",
            sessions: inbox,
            inbox: true,
          }]
        : []),
      ...(settled.length
        ? [{
            id: SIDEBAR_SETTLED_SECTION_ID,
            label: "Settled",
            sessions: settled,
            inbox: true,
            settled: true,
          }]
        : []),
    ];
  }
  const tier = (items: T[]) => {
    const ordered = sortBySidebarOrder(items, sidebar);
    const ids = new Set(ordered.map((session) => session.id));
    const childrenByParent = new Map<string, T[]>();
    const roots: T[] = [];
    for (const session of ordered) {
      if (session.parentId && ids.has(session.parentId)) {
        childrenByParent.set(session.parentId, [
          ...(childrenByParent.get(session.parentId) ?? []),
          session,
        ]);
      } else {
        roots.push(session);
      }
    }
    const withChildren = (parents: T[]): T[] => {
      const result: T[] = [];
      const append = (session: T) => {
        result.push(session);
        for (const child of childrenByParent.get(session.id) ?? []) append(child);
      };
      for (const parent of parents) append(parent);
      return result;
    };
    return { roots, withChildren };
  };
  const activeTier = tier(
    visibleAllSessions,
  );
  const visibleSessions = activeTier.roots.filter(matches);
  const normalSessions = visibleSessions.filter(
    (session) => !pinnedSessions.has(session.id),
  );
  const folders = uniqueFolders([
    ...activeProjects.map((project) => project.folder),
    ...activeTier.roots.map(projectFolderForSession),
  ]);
  const pinnedGroups: Array<SessionGroup<T>> = [];
  const projectGroups: Array<SessionGroup<T>> = [];
  const pinned = sortBySidebarOrder(
    visibleSessions.filter((session) => pinnedSessions.has(session.id)),
    sidebar,
  );

  if (pinned.length > 0) {
    pinnedGroups.push({
      id: SIDEBAR_PINNED_SECTION_ID,
      label: "Pinned",
      sessions: activeTier.withChildren(pinned),
    });
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
      projectGroups.push({
        id: sectionId,
        projectId: project?.id ?? sectionId,
        project,
        label: project?.name ?? folderLabel(folder),
        subtitle: folder,
        sessions: activeTier.withChildren(projectSessions),
      });
    }
  }

  const looseSessions = activeTier.withChildren(
    sortBySidebarOrder(
      normalSessions.filter((session) => !projectFolderForSession(session)),
      sidebar,
    ),
  );
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

const PROJECT_ICON_OPTIONS: Array<{ id: ProjectIconId; label: string }> = [
  { id: "folder", label: "Folder" },
  { id: "code", label: "Code" },
  { id: "terminal", label: "Terminal" },
  { id: "cube", label: "Cube" },
  { id: "git-branch", label: "Git branch" },
  { id: "globe", label: "Globe" },
  { id: "lightbulb", label: "Lightbulb" },
  { id: "image", label: "Image" },
  { id: "calendar", label: "Calendar" },
  { id: "file", label: "File" },
  { id: "bolt", label: "Bolt" },
  { id: "star", label: "Star" },
];

function ProjectIcon({
  icon,
  collapsed = false,
  size = 14,
}: {
  icon?: ProjectIconId;
  collapsed?: boolean;
  size?: number;
}) {
  if (!icon || icon === "folder") {
    return collapsed ? <Folder size={size} /> : <FolderOpen size={size} />;
  }
  if (icon === "code") return <Code size={size} />;
  if (icon === "terminal") return <Terminal size={size} />;
  if (icon === "cube") return <Cube size={size} />;
  if (icon === "git-branch") return <GitBranch size={size} />;
  if (icon === "globe") return <Globe size={size} />;
  if (icon === "lightbulb") return <Lightbulb size={size} />;
  if (icon === "image") return <Image size={size} />;
  if (icon === "calendar") return <Calendar size={size} />;
  if (icon === "file") return <FileText size={size} />;
  if (icon === "bolt") return <Bolt size={size} />;
  return <Star size={size} />;
}

export function ProjectCustomizationDialog({
  project,
  onClose,
  onSave,
}: {
  project: Project;
  onClose: () => void;
  onSave: (patch: Partial<Pick<Project, "name" | "icon" | "color">>) => void;
}) {
  const theme = useTheme((state) => state.theme);
  const autoColorThreadNames = useUiPreferences((state) => state.autoColorThreadNames);
  const [name, setName] = useState(project.name);
  const [icon, setIcon] = useState<ProjectIconId | undefined>(project.icon);
  const [color, setColor] = useState<string | undefined>(project.color);
  const normalizedColor = color == null ? undefined : normalizeProjectColor(color);
  const colorValid = color == null || Boolean(normalizedColor);
  const previewColor = effectiveProjectColor(
    { folder: project.folder, color: normalizedColor },
    {
      accent: theme.colors.accent,
      sidebarBackground: theme.colors.sidebarBg,
      auto: autoColorThreadNames,
    },
  );

  function reset() {
    setName(folderLabel(project.folder));
    setIcon(undefined);
    setColor(undefined);
  }

  return (
    <SheetDialog
      title="Customize project"
      className="sheet project-customization-sheet"
      testId="project-customization-dialog"
      onClose={onClose}
    >
      <div className="sheet-header project-customization-header">
        <div>
          <h2>Customize project</h2>
          <p title={project.folder}>{project.folder}</p>
        </div>
      </div>

      <div className="project-customization-body">
        <section className="project-customization-preview" aria-label="Project preview">
          <span className="project-customization-preview-label">Sidebar preview</span>
          <div className="project-customization-preview-row">
            <span style={previewColor ? { color: previewColor } : undefined}>
              <ProjectIcon icon={icon} />
            </span>
            <div className="project-customization-preview-copy">
              <strong style={previewColor ? { color: previewColor } : undefined}>{name.trim() || folderLabel(project.folder)}</strong>
              <span style={previewColor ? { color: previewColor } : undefined}>Example thread name</span>
            </div>
          </div>
        </section>

        <section className="project-customization-name-section">
          <label className="project-customization-name">
            <span>Name</span>
            <input
              className="name-input"
              value={name}
              onChange={(event) => setName(event.currentTarget.value)}
              placeholder={folderLabel(project.folder)}
              aria-label="Project name"
              autoFocus
            />
          </label>
        </section>

        <section className="editor-section project-appearance-section">
          <div className="editor-section-head">
            <h3>Icon</h3>
            <span>{PROJECT_ICON_OPTIONS.find((option) => option.id === (icon ?? "folder"))?.label}</span>
          </div>
          <div className="project-icon-grid" role="radiogroup" aria-label="Project icon">
            {PROJECT_ICON_OPTIONS.map((option) => {
              const selected = option.id === (icon ?? "folder");
              return (
                <button
                  key={option.id}
                  className={"project-icon-option" + (selected ? " active" : "")}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  aria-label={option.label}
                  title={option.label}
                  onClick={() => setIcon(option.id === "folder" ? undefined : option.id)}
                >
                  <ProjectIcon icon={option.id} />
                </button>
              );
            })}
          </div>

          <div className="project-appearance-divider" />
          <div className="project-color-section">
            <div className="project-color-heading">
              <div>
                <h3>Color</h3>
                <p>Applied to the icon, project name, and thread names</p>
              </div>
              <button className="btn-ghost compact" type="button" disabled={color == null} onClick={() => setColor(undefined)}>
                Use default
              </button>
            </div>
            <ColorField
              value={color ?? previewColor ?? theme.colors.tertiaryText}
              onChange={setColor}
              label={color == null ? "Default" : color}
            />
            {!colorValid && <p className="sheet-hint error">Enter a 3- or 6-digit hex color.</p>}
          </div>
        </section>
      </div>

      <div className="project-customization-footer">
        <button className="btn-ghost" type="button" onClick={reset}>Reset to defaults</button>
        <div className="editor-actions">
          <button className="btn-ghost" type="button" onClick={onClose}>Cancel</button>
          <button
            className="btn-accent"
            type="button"
            disabled={!colorValid}
            onClick={() => {
              onSave({ name, icon, color: normalizedColor });
              onClose();
            }}
          >
            Save
          </button>
        </div>
      </div>
    </SheetDialog>
  );
}

function WorkingSessionLoader({ className = "", ...props }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className={`session-loader working${className ? ` ${className}` : ""}`} {...props}>
      <span className="loader" aria-hidden="true" />
    </span>
  );
}

function UnreadSessionLoader({ className = "", ...props }: HTMLAttributes<HTMLSpanElement>) {
  return (
    <span className={`session-loader unread${className ? ` ${className}` : ""}`} {...props}>
      <span className="loader" aria-hidden="true" />
    </span>
  );
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
    a.threadWorkspace === b.threadWorkspace &&
    previewRuntime === b.previewRuntime &&
    a.parentId === b.parentId &&
    a.worker === b.worker &&
    a.createdAt === b.createdAt &&
    a.updatedAt === b.updatedAt &&
    a.settledAt === b.settledAt &&
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
  onManagePullRequests,
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
  onManagePullRequests: () => void;
  onManageMcp: () => void;
  onGitAction: (text: string) => void;
  onOpenGitPanel: (sessionId?: string, view?: GitPanelView) => void;
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
  const pullRequestsBySession = useSessions((s) => s.pullRequestsBySession);
  const setSessionPullRequest = useSessions((s) => s.setSessionPullRequest);
  const sidebarState = useSessions((s) => s.sidebar);
  const switchTo = useSessions((s) => s.switchTo);
  const settleSession = useSessions((s) => s.settleSession);
  const unsettleSession = useSessions((s) => s.unsettleSession);
  const archiveSession = useSessions((s) => s.archiveSession);
  const archiveProject = useSessions((s) => s.archiveProject);
  const updateProject = useSessions((s) => s.updateProject);
  const rename = useSessions((s) => s.rename);
  const addProjectFolder = useSessions((s) => s.addProjectFolder);
  const toggleSessionPinned = useSessions((s) => s.toggleSessionPinned);
  const toggleSidebarSectionCollapsed = useSessions((s) => s.toggleSidebarSectionCollapsed);
  const toggleSidebarSectionPinned = useSessions((s) => s.toggleSidebarSectionPinned);
  const setSessionUnread = useSessions((s) => s.setSessionUnread);
  const moveSidebarSection = useSessions((s) => s.moveSidebarSection);
  const moveSessionInSidebar = useSessions((s) => s.moveSessionInSidebar);
  const sidebarWidth = useUiPreferences((s) => s.sidebarWidth);
  const setSidebarWidth = useUiPreferences((s) => s.setSidebarWidth);
  const newChatButtonAtBottom = useUiPreferences((s) => s.newChatButtonAtBottom);
  const settledThreadsEnabled = useUiPreferences((s) => s.settledThreadsEnabled);
  const autoColorThreadNames = useUiPreferences((s) => s.autoColorThreadNames);
  const toolsExpanded = useUiPreferences((s) => s.toolsExpanded);
  const setToolsExpanded = useUiPreferences((s) => s.setToolsExpanded);
  const pushNotice = useUiPreferences((s) => s.pushNotice);
  const theme = useTheme((s) => s.theme);

  const [editing, setEditing] = useState<string | null>(null);
  const [customizingProjectId, setCustomizingProjectId] = useState<string | null>(null);
  const [projectMenuOpen, setProjectMenuOpen] = useState(false);
  const [confirmArchiveId, setConfirmArchiveId] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const projectMenuRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const focusSearchAfterOpenRef = useRef(false);
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
  const [settledExpanded, setSettledExpanded] = useState(false);

  useEffect(() => {
    if (!open || !focusSearchAfterOpenRef.current) return;
    focusSearchAfterOpenRef.current = false;
    const frame = window.requestAnimationFrame(() => {
      searchInputRef.current?.focus();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [open]);

  async function switchVisibleSession(id: string) {
    if (id === activeId) return;
    if (!(await requestWorkspaceEditorLeave("navigate"))) return;
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
      { key: "pull-requests", label: "Pull requests", icon: <GitPullRequest size={iconSize} />, action: onManagePullRequests, visible: true },
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

  const groupedSessions = useMemo(
    () =>
      groupSessionsByProjects(
        sessions,
        projects,
        sidebarState,
        query,
        settledThreadsEnabled,
      ),
    [projects, query, sessions, settledThreadsEnabled, sidebarState],
  );
  useEffect(() => {
    if (!settledThreadsEnabled) setSettledExpanded(false);
  }, [settledThreadsEnabled]);
  const projectByFolder = useMemo(
    () => new Map(projects.filter((project) => !project.archivedAt).map((project) => [project.folder, project])),
    [projects],
  );
  const projectColorsByFolder = useMemo(
    () => new Map(
      [...projectByFolder.entries()].map(([folder, project]) => [
        folder,
        effectiveProjectColor(project, {
          accent: theme.colors.accent,
          sidebarBackground: theme.colors.sidebarBg,
          auto: autoColorThreadNames,
        }),
      ]),
    ),
    [autoColorThreadNames, projectByFolder, theme.colors.accent, theme.colors.sidebarBg],
  );
  const customizingProject = customizingProjectId
    ? projects.find((project) => project.id === customizingProjectId) ?? null
    : null;
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
  const pullRequestTargets = useMemo(() => {
    const targets = new Map<
      string,
      { sessionId: string; folder: string; branch?: string }
    >();
    for (const group of groupedSessions) {
      if (group.settled) continue;
      const limit = settledThreadsEnabled
        ? group.sessions.length
        : sectionVisibleLimits[group.id] ?? SIDEBAR_SECTION_PREVIEW_LIMIT;
      const activeIndex = group.sessions.findIndex(
        (session) => session.id === activeId,
      );
      const visible = group.sessions.slice(0, limit);
      if (activeIndex >= limit) visible.push(group.sessions[activeIndex]);
      for (const session of visible) {
        const folder = session.settings?.folder?.trim() ?? "";
        const isolated = session.threadWorkspace?.mode === "worktree";
        if (
          !folder ||
          (!settledThreadsEnabled && session.id !== activeId && !isolated)
        )
          continue;
        targets.set(session.id, {
          sessionId: session.id,
          folder,
          branch: session.threadWorkspace?.branch,
        });
      }
    }
    return [...targets.values()];
  }, [activeId, groupedSessions, sectionVisibleLimits, settledThreadsEnabled]);
  const pullRequestTargetsKey = pullRequestTargets
    .map((target) => `${target.sessionId}\0${target.folder}\0${target.branch ?? ""}`)
    .join("\x01");

  useEffect(() => {
    if (!pullRequestTargets.length) return;
    let cancelled = false;
    const refresh = async (openOnly: boolean) => {
      await Promise.all(
        pullRequestTargets.map(async (target) => {
          const previous =
            useSessions.getState().pullRequestsBySession[target.sessionId];
          if (
            openOnly &&
            previous?.pullRequest?.state.toUpperCase() !== "OPEN"
          )
            return;
          try {
            const result = await runWorkspaceGitAction("pr_status", {
              folder: target.folder,
            });
            if (cancelled) return;
            if (!result.ok) throw new Error(result.message);
            setSessionPullRequest(target.sessionId, {
              folder: target.folder,
              pullRequest: result.pull_request ?? null,
              checkedAt: Date.now(),
              stale: false,
            });
          } catch (error) {
            if (cancelled || !previous?.pullRequest) return;
            setSessionPullRequest(target.sessionId, {
              ...previous,
              checkedAt: Date.now(),
              stale: true,
              error:
                error instanceof Error
                  ? error.message
                  : "Couldn't refresh pull request.",
            });
          }
        }),
      );
    };
    void refresh(false);
    const onFocus = () => void refresh(false);
    const interval = window.setInterval(
      () => void refresh(true),
      GIT_STATUS_REFRESH_INTERVAL_MS,
    );
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [pullRequestTargetsKey, setSessionPullRequest]);

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

  async function archiveChat(id: string) {
    if (confirmArchiveId !== id) {
      setConfirmArchiveId(id);
      return;
    }
    if (id === activeId && !(await requestWorkspaceEditorLeave("navigate"))) return;
    archiveSession(id);
    setConfirmArchiveId(null);
  }

  async function branchChat(id: string) {
    if (id === activeId && !(await requestWorkspaceEditorLeave("navigate"))) return;
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

  async function settleChat(id: string) {
    const allInboxGroups = query.trim()
      ? groupSessionsByProjects(
          sessions,
          projects,
          sidebarState,
          "",
          true,
        )
      : groupedSessions;
    const nextId =
      settledThreadsEnabled && id === activeId
        ? nextInboxSessionIdAfterSettle(allInboxGroups, id)
        : undefined;
    if (nextId && !(await requestWorkspaceEditorLeave("navigate"))) return;
    settleSession(id);
    if (nextId) switchTo(nextId);
    else if (settledThreadsEnabled && id === activeId) setSettledExpanded(true);
    setConfirmArchiveId(null);
  }

  function openSessionContextMenu(event: ReactMouseEvent, session: SidebarSessionLike, pinned: boolean) {
    const settleDisabled =
      !session.settledAt &&
      (generatingSessions.has(session.id) ||
        runningWorkerParentThreads.has(session.id) ||
        session.worker?.status === "queued" ||
        session.worker?.status === "running" ||
        unreadSessions.has(session.id));
    openContextMenu(event, [
      {
        id: "open",
        label: session.id === activeId ? "Current chat" : "Open chat",
        icon: <FolderOpen size={13} />,
        disabled: session.id === activeId,
        action: () => { void switchVisibleSession(session.id); },
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
      ...(settledThreadsEnabled ? [{
        id: session.settledAt ? "unsettle" : "settle",
        label: session.settledAt ? "Unsettle chat" : "Settle chat",
        detail: settleDisabled ? "Thread still active" : undefined,
        icon: session.settledAt
          ? <ArrowUp size={13} />
          : <Check size={13} />,
        disabled: settleDisabled,
        action: () => {
          if (session.settledAt) unsettleSession(session.id);
          else settleChat(session.id);
          setConfirmArchiveId(null);
        },
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
      ...(!settledThreadsEnabled || session.settledAt ? [{
        id: "archive",
        label: confirmArchiveId === session.id ? "Confirm archive" : "Archive chat",
        detail: confirmArchiveId === session.id ? "Again" : undefined,
        icon: <Archive size={13} />,
        danger: true,
        separatorBefore: true,
        action: () => archiveChat(session.id),
      }] : []),
    ], session.title);
  }

  function openSectionContextMenu(event: ReactMouseEvent, group: SessionGroup, collapsed: boolean, pinned: boolean) {
    const projectSection = Boolean(group.projectId);
    const projectFolder = projectSection ? folderFromSectionId(group.id) : "";
    const unreadProjectSessions = projectFolder
      ? sessions.filter((session) => projectFolderForSession(session) === projectFolder && unreadSessions.has(session.id))
      : [];
    const fileManagerLabel = navigator.userAgent.includes("Mac") ? "Open in Finder" : "Open in File Explorer";
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
        id: "open-file-manager",
        label: fileManagerLabel,
        icon: <FolderOpen size={13} />,
        action: async () => {
          try {
            await openWorkspaceLauncher(projectFolder, "file_manager");
          } catch (error) {
            pushNotice({ tone: "warning", message: `Could not open the project folder: ${error instanceof Error ? error.message : String(error)}` });
          }
        },
      }] : []),
      ...(group.project ? [{
        id: "customize-project",
        label: "Customize project...",
        icon: <Gear size={13} />,
        action: () => setCustomizingProjectId(group.project!.id),
      }] : []),
      ...(projectSection ? [{
        id: "pin",
        label: pinned ? "Unpin section" : "Pin section",
        icon: <Pin size={13} />,
        checked: pinned,
        action: () => toggleSidebarSectionPinned(group.id),
      }] : []),
      ...(unreadProjectSessions.length > 0 ? [{
        id: "mark-project-read",
        label: "Mark all as read",
        icon: <Check size={13} />,
        action: () => unreadProjectSessions.forEach((session) => setSessionUnread(session.id, false)),
      }] : []),
      ...(group.projectId && group.id !== SIDEBAR_CHATS_SECTION_ID ? [{
        id: "archive-project",
        label: "Archive project",
        icon: <Archive size={13} />,
        danger: true,
        separatorBefore: true,
        action: async () => {
          if (group.sessions.some((session) => session.id === activeId) && !(await requestWorkspaceEditorLeave("navigate"))) return;
          archiveProject(group.projectId!);
        },
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
        if (
          !id ||
          !sectionId ||
          id === item.id ||
          sectionId === SIDEBAR_SETTLED_SECTION_ID
        )
          return null;
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
      <aside className="sidebar collapsed" aria-label={settledThreadsEnabled ? "Thread inbox" : "Chats"}>
        <div className="sidebar-rail" data-testid="sidebar-rail">
          <div className="sidebar-rail-main">
            <button className="icon-btn" title="Expand sidebar" onClick={onToggle}>
              <PanelIcon size={16} />
            </button>
            <button className="icon-btn" title="New chat" onClick={() => createChat()}>
              <Plus size={16} />
            </button>
            <button
              className="icon-btn"
              data-testid="sidebar-rail-search"
              title="Search chats"
              aria-label="Search chats"
              onClick={() => {
                focusSearchAfterOpenRef.current = true;
                onToggle();
              }}
            >
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
                      {generating ? (
                        <WorkingSessionLoader aria-hidden="true" />
                      ) : (
                        <UnreadSessionLoader aria-hidden="true" />
                      )}
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
    <>
    <aside ref={sidebarElementRef} className={"sidebar" + (sidebarResizing ? " resizing" : "")} aria-label={settledThreadsEnabled ? "Thread inbox" : "Chats"} style={sidebarStyle}>
      <div className="sidebar-inner">
        <div className="sidebar-head">
          <button className="icon-btn" title="Collapse sidebar" onClick={onToggle}>
            <PanelIcon size={16} />
          </button>
          <label className="sidebar-search">
            <Search size={14} />
            <input
              ref={searchInputRef}
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
              const sectionProjectColor = group.subtitle
                ? projectColorsByFolder.get(group.subtitle)
                : undefined;
              const sectionProjectStyle = sectionProjectColor
                ? { "--project-color": sectionProjectColor } as CSSProperties
                : undefined;
              const projectPullRequestOwner = sidebarProjectPullRequestOwner(
                group,
                pullRequestsBySession,
              );
              const projectPullRequestState = projectPullRequestOwner
                ? pullRequestReadiness(
                    projectPullRequestOwner.pullRequest,
                    projectPullRequestOwner.snapshot.stale,
                  )
                : null;
              const searchActive = Boolean(query.trim());
              const totalSessions = group.sessions.length;
              const visibleLimit = searchActive || group.inbox
                ? totalSessions
                : Math.min(sectionVisibleLimits[group.id] ?? SIDEBAR_SECTION_PREVIEW_LIMIT, totalSessions);
              const baseVisibleSessions = group.sessions.slice(0, visibleLimit);
              const activeIndex = group.sessions.findIndex((session) => session.id === activeId);
              const visibleSessions = !searchActive && activeIndex >= visibleLimit
                ? [...baseVisibleSessions, group.sessions[activeIndex]]
                : baseVisibleSessions;
              const currentShownCount = visibleSessions.length;
              const nextRevealCount = sidebarSectionNextRevealCount(totalSessions, visibleLimit, activeIndex);
              const canShowMore = !searchActive && !group.inbox && nextRevealCount > 0;
              const canShowLess = !searchActive && !group.inbox && visibleLimit > SIDEBAR_SECTION_PREVIEW_LIMIT;
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
              const groupSessionIds = new Set(
                group.sessions.map((session) => session.id),
              );
              if (group.inbox) {
                const settledSection = Boolean(group.settled);
                const pinnedSection =
                  group.id === SIDEBAR_PINNED_SECTION_ID;
                const inboxSectionExpanded =
                  !settledSection ||
                  searchActive ||
                  settledExpanded ||
                  activeIndex >= 0;
                return (
                  <section
                    key={group.id}
                    data-sidebar-section-id={group.id}
                    className={
                      "inbox-session-section" +
                      (settledSection ? " settled-session-section" : "") +
                      (pinnedSection ? " pinned" : "")
                    }
                  >
                    {settledSection ? (
                      <button
                        className="inbox-session-divider"
                        type="button"
                        aria-expanded={inboxSectionExpanded}
                        aria-controls="sidebar-settled-list"
                        onClick={() =>
                          setSettledExpanded(
                            (expanded) => !expanded,
                          )}
                      >
                        <ChevronDown size={12} />
                        <span>Settled</span>
                        <span className="inbox-session-count">
                          {totalSessions}
                        </span>
                      </button>
                    ) : pinnedSection ? (
                      <div
                        className="inbox-session-divider"
                        role="heading"
                        aria-level={2}
                      >
                        <Pin size={12} />
                        <span>Pinned</span>
                      </div>
                    ) : (
                      <div
                        className="inbox-session-divider"
                        role="heading"
                        aria-level={2}
                      >
                        <PanelIcon size={12} />
                        <span>Inbox</span>
                      </div>
                    )}
                    {inboxSectionExpanded && (
                    <div
                      id={settledSection ? "sidebar-settled-list" : undefined}
                      className="inbox-session-list"
                    >
                      {visibleSessions.map((s) => {
                        const folder = projectFolderForSession(s);
                        const project = projectByFolder.get(folder);
                        const projectColor = projectColorsByFolder.get(folder);
                        const projectStyle = projectColor
                          ? {
                              "--project-color": projectColor,
                            } as CSSProperties
                          : undefined;
                        const projectLabel =
                          project?.name ?? (folder ? folderLabel(folder) : "");
                        const pinned =
                          !s.parentId &&
                          sidebarState.pinnedSessionIds.includes(s.id);
                        const parentWorkersRunning =
                          runningWorkerParentThreads.has(s.id);
                        const workerRunning =
                          parentWorkersRunning ||
                          s.worker?.status === "queued" ||
                          s.worker?.status === "running";
                        const generating =
                          generatingSessions.has(s.id) || workerRunning;
                        const unread = unreadSessions.has(s.id);
                        const runtimeState = settledSection
                          ? null
                          : runtimePreviewSidebarState(s);
                        const statusLabel = parentWorkersRunning
                          ? "Workers running"
                          : s.worker
                            ? `Worker ${s.worker.status}`
                            : generating
                              ? "Working"
                              : unread
                                ? "Unread update"
                                : "Ready";
                        const threadPullRequestOwner = settledSection
                          ? undefined
                          : sidebarInboxPullRequestOwner(
                              s,
                              pullRequestsBySession,
                            );
                        const pullRequestSnapshot =
                          threadPullRequestOwner?.snapshot;
                        const pullRequest =
                          threadPullRequestOwner?.pullRequest ?? null;
                        const pullRequestState = pullRequest
                          ? pullRequestReadiness(
                              pullRequest,
                              pullRequestSnapshot?.stale,
                            )
                          : null;
                        const branchLabel =
                          s.threadWorkspace?.branch || "Branch";
                        const settleDisabled =
                          generating || unread;
                        const rowClass =
                          "session-item inbox-session-item" +
                          (settledSection
                            ? " settled-session-item"
                            : "") +
                          (s.id === activeId ? " active" : "") +
                          (generating ? " generating" : "") +
                          (runtimeState
                            ? " runtime-preview runtime-" + runtimeState
                            : "") +
                          (pinned ? " pinned" : "") +
                          (confirmArchiveId === s.id
                            ? " delete-pending"
                            : "");
                        return (
                          <div
                            key={s.id}
                            data-sidebar-session-id={s.id}
                            data-sidebar-session-section-id={group.id}
                            className={rowClass}
                            style={projectStyle}
                            onContextMenu={(event) =>
                              openSessionContextMenu(event, s, pinned)}
                            onClick={(event) => {
                              if (isSidebarDragInteractiveTarget(event.target))
                                return;
                              setConfirmArchiveId(null);
                              switchVisibleSession(s.id);
                            }}
                            onDoubleClick={(event) => {
                              if (isSidebarDragInteractiveTarget(event.target))
                                return;
                              beginRename(s.id);
                            }}
                            title={s.title}
                          >
                            {editing === s.id ? (
                              <input
                                className="session-rename"
                                defaultValue={s.title}
                                autoFocus
                                onClick={(event) => event.stopPropagation()}
                                onBlur={(event) => {
                                  rename(s.id, event.target.value.trim());
                                  setEditing(null);
                                }}
                                onKeyDown={(event) => {
                                  if (event.key === "Enter")
                                    (event.target as HTMLInputElement).blur();
                                  if (event.key === "Escape") setEditing(null);
                                }}
                              />
                            ) : (
                              <>
                                <span className="session-copy inbox-session-copy">
                                  <HoverScrollText
                                    className="session-title"
                                    innerClassName={
                                      generating ? "shiny-text" : undefined
                                    }
                                    text={s.title}
                                  />
                                  {(projectLabel || s.parentId) && (
                                  <span className="inbox-session-metadata">
                                    {projectLabel && (
                                      <span
                                        className="inbox-session-project"
                                        title={projectLabel}
                                        aria-label={`Project: ${projectLabel}`}
                                      >
                                        {projectLabel}
                                      </span>
                                    )}
                                    {s.parentId && (
                                      <span
                                        className="inbox-session-branch"
                                        title={branchLabel}
                                        aria-label={branchLabel}
                                      >
                                        <GitBranch size={9} />
                                        <span>{branchLabel}</span>
                                      </span>
                                    )}
                                  </span>
                                  )}
                                </span>
                                <div
                                  className={
                                    "session-side" +
                                    (!settledSection
                                      ? " session-side-wide"
                                      : "")
                                  }
                                >
                                  {pullRequest && pullRequestState && (
                                    <button
                                      type="button"
                                      className={`session-side-indicator session-pr-state ${pullRequestState.tone}`}
                                      title={pullRequestAccessibleLabel(
                                        pullRequest,
                                        pullRequestSnapshot?.stale,
                                      )}
                                      aria-label={pullRequestAccessibleLabel(
                                        pullRequest,
                                        pullRequestSnapshot?.stale,
                                      )}
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        switchVisibleSession(s.id);
                                        onOpenGitPanel(
                                          s.id,
                                          "pull_request",
                                        );
                                      }}
                                    >
                                      <GitPullRequest size={13} />
                                      <span aria-hidden="true" />
                                    </button>
                                  )}
                                  {generating ? (
                                    <WorkingSessionLoader
                                      className="session-side-indicator"
                                      data-testid="session-loader"
                                      role="img"
                                      title={statusLabel}
                                      aria-label={statusLabel}
                                    />
                                  ) : unread ? (
                                    <UnreadSessionLoader
                                      className="session-side-indicator"
                                      data-testid="session-loader"
                                      role="img"
                                      title={statusLabel}
                                      aria-label={statusLabel}
                                    />
                                  ) : (
                                    <span
                                      className="session-side-indicator session-recency"
                                      data-testid="session-recency"
                                      title={`Updated ${new Date(s.updatedAt).toLocaleString()}`}
                                    >
                                      {sessionRecencyLabel(s.updatedAt)}
                                    </span>
                                  )}
                                  <div
                                    className="session-side-actions"
                                    aria-label="Thread actions"
                                  >
                                    {settledSection ? (
                                      <button
                                        className="session-side-btn"
                                        type="button"
                                        aria-label={`Unsettle ${s.title}`}
                                        title="Unsettle chat"
                                        onClick={(event) => {
                                          event.stopPropagation();
                                          unsettleSession(s.id);
                                          setConfirmArchiveId(null);
                                        }}
                                      >
                                        <ArrowUp size={12} />
                                      </button>
                                    ) : (
                                      <>
                                        {!s.parentId && (
                                          <button
                                            className={
                                              "session-side-btn" +
                                              (pinned ? " active" : "")
                                            }
                                            type="button"
                                            aria-label={
                                              pinned
                                                ? `Unpin ${s.title}`
                                                : `Pin ${s.title}`
                                            }
                                            title={
                                              pinned
                                                ? "Unpin chat"
                                                : "Pin chat"
                                            }
                                            aria-pressed={pinned}
                                            onClick={(event) => {
                                              event.stopPropagation();
                                              setConfirmArchiveId(null);
                                              toggleSessionPinned(s.id);
                                            }}
                                          >
                                            <Pin size={12} />
                                          </button>
                                        )}
                                        <button
                                          className="session-side-btn"
                                          type="button"
                                          aria-label={`Settle ${s.title}`}
                                          title={
                                            settleDisabled
                                              ? "Thread still active"
                                              : "Settle chat"
                                          }
                                          disabled={settleDisabled}
                                          onClick={(event) => {
                                            event.stopPropagation();
                                            settleChat(s.id);
                                          }}
                                        >
                                          <Check size={12} />
                                        </button>
                                      </>
                                    )}
                                    {settledSection && <button
                                      className={
                                        "session-side-btn danger" +
                                        (confirmArchiveId === s.id
                                          ? " confirm"
                                          : "")
                                      }
                                      type="button"
                                      aria-label={
                                        confirmArchiveId === s.id
                                          ? `Confirm archive ${s.title}`
                                          : `Archive ${s.title}`
                                      }
                                      title={
                                        confirmArchiveId === s.id
                                          ? "Click again to archive"
                                          : "Archive chat"
                                      }
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        archiveChat(s.id);
                                      }}
                                    >
                                      <Archive size={12} />
                                    </button>}
                                  </div>
                                </div>
                              </>
                            )}
                          </div>
                        );
                      })}
                      {(canShowMore || canShowLess) && (
                        <div className="session-more-row settled-session-more">
                          {showMoreButton ?? (
                            <span
                              className="session-more-spacer"
                              aria-hidden="true"
                            />
                          )}
                          {showLessButton}
                        </div>
                      )}
                    </div>
                    )}
                  </section>
                );
              }
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
                  style={sectionProjectStyle}
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
                      <span className={"section-type-icon" + (group.id === SIDEBAR_CHATS_SECTION_ID ? " chat-toggle-icon" : "") + (sectionProjectColor ? " project-colored" : "")} aria-hidden="true">
                        {group.id === SIDEBAR_PINNED_SECTION_ID
                          ? <Pin size={12} />
                          : group.id === SIDEBAR_CHATS_SECTION_ID
                            ? <ChevronDown size={12} />
                            : <ProjectIcon icon={group.project?.icon} collapsed={collapsed} size={12} />}
                      </span>
                      <span className="section-copy" title={group.subtitle ?? group.label}>
                        <span className="section-label-row">
                          <span className={"section-label" + (sectionProjectColor ? " project-colored" : "")}>{group.label}</span>
                        </span>
                      </span>
                    </button>
                    {projectPullRequestOwner && projectPullRequestState && (
                      <button
                        type="button"
                        className={`session-pr-state section-pr-state ${projectPullRequestState.tone}`}
                        title={pullRequestAccessibleLabel(
                          projectPullRequestOwner.pullRequest,
                          projectPullRequestOwner.snapshot.stale,
                        )}
                        aria-label={pullRequestAccessibleLabel(
                          projectPullRequestOwner.pullRequest,
                          projectPullRequestOwner.snapshot.stale,
                        )}
                        onClick={(event) => {
                          event.stopPropagation();
                          switchVisibleSession(projectPullRequestOwner.session.id);
                          onOpenGitPanel(
                            projectPullRequestOwner.session.id,
                            "pull_request",
                          );
                        }}
                      >
                        <GitPullRequest size={13} />
                        <span aria-hidden="true" />
                      </button>
                    )}
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
                  const sessionProjectColor = projectColorsByFolder.get(projectFolderForSession(s));
                  const sessionProjectStyle = sessionProjectColor
                    ? { "--project-color": sessionProjectColor } as CSSProperties
                    : undefined;
                  const statusLabel = parentWorkersRunning ? "Workers running" : s.worker ? `Worker ${s.worker.status}` : generating ? "Working" : unread ? "Unread update" : "Ready";
                  const threadPullRequestOwner = sidebarThreadPullRequestOwner(
                    s,
                    pullRequestsBySession,
                  );
                  const pullRequestSnapshot = threadPullRequestOwner?.snapshot;
                  const pullRequest = threadPullRequestOwner?.pullRequest ?? null;
                  const pullRequestState = pullRequest
                    ? pullRequestReadiness(
                        pullRequest,
                        pullRequestSnapshot?.stale,
                      )
                    : null;
                  const sessionDragOver = dragOver?.type === "session" && dragOver.id === s.id;
                  const sessionDropClass = sessionDragOver ? ` drag-over drop-${dragOver.position}` : "";
                  const sessionDragging = dragging?.type === "session" && dragging.id === s.id;
                  const tierChild = Boolean(
                    s.parentId && groupSessionIds.has(s.parentId),
                  );
                  return (
                    <div
                      key={s.id}
                      data-sidebar-session-id={s.id}
                      data-sidebar-session-section-id={group.id}
                      className={
                        "session-item" +
                        (tierChild ? " child-session" : "") +
                        (s.id === activeId ? " active" : "") +
                        (generating ? " generating" : "") +
                        (runtimeState ? " runtime-preview runtime-" + runtimeState : "") +
                        (pinned ? " pinned" : "") +
                        (sessionProjectColor ? " project-colored" : "") +
                        (confirmArchiveId === s.id ? " delete-pending" : "") +
                        (sessionDragging ? " dragging" : "") +
                        sessionDropClass
                      }
                      style={sessionProjectStyle}
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
                            <HoverScrollText
                              className="session-title"
                              innerClassName={generating ? "shiny-text" : undefined}
                              text={s.title}
                            />
                          </span>
                          <div
                            className={
                              "session-side" +
                              (settledThreadsEnabled && !s.parentId
                                ? " session-side-wide"
                                : "")
                            }
                          >
                            {pullRequest && pullRequestState && (
                              <button
                                type="button"
                                className={`session-side-indicator session-pr-state ${pullRequestState.tone}`}
                                title={pullRequestAccessibleLabel(
                                  pullRequest,
                                  pullRequestSnapshot?.stale,
                                )}
                                aria-label={pullRequestAccessibleLabel(
                                  pullRequest,
                                  pullRequestSnapshot?.stale,
                                )}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  switchVisibleSession(s.id);
                                  onOpenGitPanel(s.id, "pull_request");
                                }}
                              >
                                <GitPullRequest size={13} />
                                <span aria-hidden="true" />
                              </button>
                            )}
                            {generating ? (
                              <WorkingSessionLoader
                                className="session-side-indicator"
                                data-testid="session-loader"
                                role="img"
                                title={statusLabel}
                                aria-label={statusLabel}
                              />
                            ) : unread ? (
                              <UnreadSessionLoader
                                className="session-side-indicator"
                                data-testid="session-loader"
                                role="img"
                                title={statusLabel}
                                aria-label={statusLabel}
                              />
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
                              {settledThreadsEnabled && (
                                <button
                                  className="session-side-btn"
                                  type="button"
                                  aria-label={`Settle ${s.title}`}
                                  title={
                                    generating || unread
                                      ? "Thread still active"
                                      : "Settle chat"
                                  }
                                  disabled={generating || unread}
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    settleSession(s.id);
                                    setConfirmArchiveId(null);
                                  }}
                                >
                                  <Check size={12} />
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

        <GitPanel
          sessionId={activeId}
          folder={activeFolder}
          model={activeModel}
          onDraftAction={onGitAction}
          onOpenPanel={() => onOpenGitPanel(activeId, "changes")}
        />

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
    {customizingProject && (
      <ProjectCustomizationDialog
        project={customizingProject}
        onClose={() => setCustomizingProjectId(null)}
        onSave={(patch) => updateProject(customizingProject.id, patch)}
      />
    )}
    </>
  );
}
