export {};

import { createServer } from "vite";
import type { Project, SessionSidebarState } from "../src/sessions/store.js";

class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

type SidebarSession = {
  id: string;
  title: string;
  hasMessages?: boolean;
  settings?: { folder?: string };
  retryWorkspace?: { originalFolder: string };
  threadWorkspace?: {
    mode: "current" | "worktree";
    projectFolder?: string;
    branch?: string;
  };
  parentId?: string;
  settledAt?: number;
  archivedAt?: number;
  createdAt: number;
  updatedAt: number;
};
type SessionGroup = {
  id: string;
  projectId?: string;
  project?: Project;
  sessions: SidebarSession[];
  inbox?: boolean;
  settled?: boolean;
};

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const {
    createSidebarSessionsSelector,
    groupSessionsByProjects,
    nextInboxSessionIdAfterSettle,
    reconcileWorkingSessionActivityAt,
    runningWorkerParentThreadIdsKey,
    sidebarInboxPullRequestOwner,
    sidebarProjectPullRequestOwner,
    sidebarSectionNextRevealCount,
    sidebarSectionUsesPagination,
    sidebarSessionHasMessages,
    sidebarThreadPullRequestOwner,
    workingSessionIdsFromSignals,
  } = await server.ssrLoadModule("/src/components/Sidebar.tsx") as {
    createSidebarSessionsSelector: () => (state: unknown) => Array<SidebarSession & { hasMessages: boolean }>;
    groupSessionsByProjects: (sessions: SidebarSession[], projects: Project[], sidebar: SessionSidebarState, query: string, settledThreadsEnabled?: boolean, activityAtBySession?: ReadonlyMap<string, number>) => SessionGroup[];
    nextInboxSessionIdAfterSettle: (groups: SessionGroup[], currentId: string, activityAtBySession?: ReadonlyMap<string, number>) => string | undefined;
    reconcileWorkingSessionActivityAt: (previous: ReadonlyMap<string, number>, sessions: SidebarSession[], workingSessionIds: ReadonlySet<string>) => ReadonlyMap<string, number>;
    runningWorkerParentThreadIdsKey: (records: Array<{ run: { parent_thread_id: string; status: string } }>) => string;
    sidebarInboxPullRequestOwner: (
      session: SidebarSession,
      snapshots: Record<string, {
        folder: string;
        pullRequest: { number: number; headRefName: string } | null;
        checkedAt: number;
        stale: boolean;
      }>,
    ) => { session: SidebarSession; pullRequest: { number: number } } | undefined;
    sidebarProjectPullRequestOwner: (
      group: SessionGroup,
      snapshots: Record<string, {
        folder: string;
        pullRequest: { number: number; headRefName: string } | null;
        checkedAt: number;
        stale: boolean;
      }>,
    ) => { session: SidebarSession; pullRequest: { number: number } } | undefined;
    sidebarSectionNextRevealCount: (totalSessions: number, visibleLimit: number, activeIndex: number) => number;
    sidebarSectionUsesPagination: (sectionId: string, inbox: boolean | undefined, searchActive: boolean) => boolean;
    sidebarSessionHasMessages: (session: { messages: unknown[]; messagesHydrated?: boolean; persistedMessageCount?: number }) => boolean;
    workingSessionIdsFromSignals: (signals: {
      generatingSessionIds?: string[];
      hostBusySessionIds?: string[];
      activityBusySessionIds?: string[];
      runningWorkerParentIds?: string[];
    }) => string[];
    sidebarThreadPullRequestOwner: (
      session: SidebarSession,
      snapshots: Record<string, {
        folder: string;
        pullRequest: { number: number; headRefName: string } | null;
        checkedAt: number;
        stale: boolean;
      }>,
    ) => { session: SidebarSession; pullRequest: { number: number } } | undefined;
  };
  const { SIDEBAR_CHATS_SECTION_ID, projectSectionId, useSessions } = await server.ssrLoadModule("/src/sessions/store.ts") as {
    SIDEBAR_CHATS_SECTION_ID: string;
    projectSectionId: (folder?: string) => string;
    useSessions: {
      getState: () => {
        newChat: (settings?: Record<string, unknown>, id?: string) => string;
        setMessages: (id: string, messages: Array<{ role: string; content: string }>, options?: { autoTitle?: boolean }) => void;
      } & Record<string, unknown>;
    };
  };

  const folder = "C:\\workspace-a";
  const now = 1;
  const sidebar = {
    collapsedSectionIds: [],
    pinnedSessionIds: [],
    pinnedSectionIds: [],
    sessionOrder: [],
    sectionOrder: [],
    projectFolders: [folder],
  };
  const groups = groupSessionsByProjects(
    [
      {
        id: "project-chat",
        title: "Project chat",
        settings: { folder },
        createdAt: now,
        updatedAt: now,
      },
      {
        id: "retry-chat",
        title: "Retry chat",
        settings: { folder: "C:\\Users\\test\\.milim\\runtime\\hot-swap\\retry" },
        retryWorkspace: { originalFolder: folder },
        createdAt: now,
        updatedAt: now,
      },
    ],
    [{
      id: projectSectionId(folder),
      name: "Workspace A",
      folder,
      icon: "terminal",
      color: "#22aa88",
      createdAt: now,
      updatedAt: now,
    }],
    sidebar,
    "",
  );

  const projectGroup = groups.find((group) => group.id === projectSectionId(folder));
  assert(projectGroup?.sessions.some((session) => session.id === "retry-chat"), "retry worktrees should stay grouped under the original project");
  assert(projectGroup?.project?.icon === "terminal", "project groups should retain custom icons");
  assert(projectGroup?.project?.color === "#22aa88", "project groups should retain custom colors");

  const chats = groups.find((group) => group.id === SIDEBAR_CHATS_SECTION_ID);
  assert(chats, "empty Chats section should render when unfiltered");
  assert(chats.sessions.length === 0, "empty Chats section should have zero sessions");

  const filteredGroups = groupSessionsByProjects([], [], { ...sidebar, projectFolders: [] }, "missing");
  assert(filteredGroups.length === 0, "filtered empty sidebar should still show no result groups");

  const draftVisibilityGroups = groupSessionsByProjects(
    [
      { id: "draft", title: "New chat", hasMessages: false, createdAt: now, updatedAt: now },
      { id: "sent", title: "Sent chat", hasMessages: true, createdAt: now, updatedAt: now },
    ],
    [],
    { ...sidebar, projectFolders: [] },
    "",
  );
  const draftVisibilityIds = draftVisibilityGroups.flatMap((group) => group.sessions.map((session) => session.id));
  assert(!draftVisibilityIds.includes("draft"), "empty new chats should stay out of thread navigation");
  assert(draftVisibilityIds.includes("sent"), "a chat should enter thread navigation after its first message");

  assert(
    sidebarSessionHasMessages({ messages: [], messagesHydrated: false, persistedMessageCount: 2 }),
    "lazy thread summaries should remain visible when their persisted transcript is not resident",
  );
  const sidebarSelector = createSidebarSessionsSelector();
  const draftId = useSessions.getState().newChat(undefined, "selector-draft");
  assert(
    sidebarSelector(useSessions.getState()).find((session) => session.id === draftId)?.hasMessages === false,
    "the sidebar selector should mark a new chat as empty",
  );
  useSessions.getState().setMessages(draftId, [{ role: "user", content: "first message" }], { autoTitle: false });
  assert(
    sidebarSelector(useSessions.getState()).find((session) => session.id === draftId)?.hasMessages === true,
    "the sidebar selector should react when the first message is sent",
  );

  const sharedOld: SidebarSession = {
    id: "shared-old",
    title: "Shared old",
    settings: { folder },
    threadWorkspace: { mode: "current", projectFolder: folder },
    createdAt: now,
    updatedAt: now,
  };
  const sharedNew: SidebarSession = {
    ...sharedOld,
    id: "shared-new",
    title: "Shared new",
  };
  const worktreeOne: SidebarSession = {
    id: "worktree-one",
    title: "Worktree one",
    settings: { folder: "C:\\worktrees\\one" },
    threadWorkspace: {
      mode: "worktree",
      projectFolder: folder,
      branch: "feature-one",
    },
    createdAt: now,
    updatedAt: now,
  };
  const worktreeTwo: SidebarSession = {
    ...worktreeOne,
    id: "worktree-two",
    title: "Worktree two",
    settings: { folder: "C:\\worktrees\\two" },
    threadWorkspace: {
      mode: "worktree",
      projectFolder: folder,
      branch: "feature-two",
    },
  };
  const pullRequestSnapshots = {
    "shared-old": {
      folder,
      pullRequest: { number: 1, headRefName: "main" },
      checkedAt: 1,
      stale: false,
    },
    "shared-new": {
      folder,
      pullRequest: { number: 2, headRefName: "main" },
      checkedAt: 2,
      stale: false,
    },
    "worktree-one": {
      folder: "C:\\worktrees\\one",
      pullRequest: { number: 3, headRefName: "feature-one" },
      checkedAt: 3,
      stale: false,
    },
    "worktree-two": {
      folder: "C:\\worktrees\\two",
      pullRequest: { number: 4, headRefName: "feature-two" },
      checkedAt: 4,
      stale: false,
    },
  };
  const projectPullRequestOwner = sidebarProjectPullRequestOwner(
    {
      id: projectSectionId(folder),
      projectId: projectSectionId(folder),
      sessions: [sharedOld, worktreeTwo, sharedNew, worktreeOne],
    },
    pullRequestSnapshots,
  );
  assert(
    projectPullRequestOwner?.session.id === "shared-new",
    "project headers should use the newest shared-checkout PR snapshot",
  );
  assert(
    !sidebarThreadPullRequestOwner(sharedNew, pullRequestSnapshots),
    "shared-checkout PRs should not remain on thread rows",
  );
  assert(
    sidebarInboxPullRequestOwner(sharedNew, pullRequestSnapshots)?.pullRequest.number === 2,
    "Inbox rows should own their shared-checkout PR state",
  );
  assert(
    sidebarThreadPullRequestOwner(worktreeOne, pullRequestSnapshots)?.pullRequest.number === 3 &&
      sidebarThreadPullRequestOwner(worktreeTwo, pullRequestSnapshots)?.pullRequest.number === 4,
    "worktree threads should retain their independent PR snapshots",
  );

  const tierSessions: SidebarSession[] = [
    {
      id: "active-parent",
      title: "Active parent",
      settings: { folder },
      createdAt: now,
      updatedAt: 20,
    },
    {
      id: "settled-parent",
      title: "Settled parent",
      settings: { folder },
      settledAt: 40,
      createdAt: now,
      updatedAt: 10,
    },
    {
      id: "active-child",
      title: "Active child",
      settings: { folder },
      parentId: "settled-parent",
      createdAt: now,
      updatedAt: 50,
    },
    {
      id: "settled-child",
      title: "Settled child",
      settings: { folder },
      parentId: "active-parent",
      settledAt: 60,
      createdAt: now,
      updatedAt: 30,
    },
    {
      id: "settled-pinned",
      title: "Pinned settled",
      settledAt: 50,
      createdAt: now,
      updatedAt: 40,
    },
    {
      id: "other-project-active",
      title: "Other project active",
      settings: { folder: "C:\\workspace-b" },
      createdAt: now,
      updatedAt: 45,
    },
  ];
  const inboxProjects = [
    ...(groups[0]?.project ? [groups[0].project] : []),
    {
      id: projectSectionId("C:\\workspace-b"),
      name: "Workspace B",
      folder: "C:\\workspace-b",
      icon: "code" as const,
      color: "#aa6622",
      createdAt: now,
      updatedAt: now,
    },
  ];
  const tierSidebar = {
    ...sidebar,
    pinnedSessionIds: ["active-parent", "settled-pinned"],
    sessionOrder: tierSessions.map((session) => session.id),
  };
  const currentModeGroups = groupSessionsByProjects(
    tierSessions,
    inboxProjects,
    tierSidebar,
    "",
  );
  assert(
    currentModeGroups.find((group) => group.id === "pinned")?.sessions[0]?.id ===
      "active-parent",
    "disabled settle mode should preserve current pinned rendering",
  );
  assert(
    !currentModeGroups.some((group) => group.settled),
    "disabled settle mode should not add a settled tier",
  );

  const inboxGroups = groupSessionsByProjects(
    tierSessions,
    inboxProjects,
    tierSidebar,
    "",
    true,
  );
  assert(
    !inboxGroups.some(
      (group) =>
        group.id === projectSectionId(folder) ||
        group.id === SIDEBAR_CHATS_SECTION_ID,
    ),
    "Inbox mode should omit project and empty Chats sections",
  );
  assert(
    JSON.stringify(
      inboxGroups.find((group) => group.id === "pinned")?.sessions.map(
        (session) => session.id,
      ),
    ) === JSON.stringify(["active-parent"]),
    "Inbox mode should keep active pinned threads in a separate activity-sorted group",
  );
  assert(
    JSON.stringify(
      inboxGroups.find((group) => group.id === "inbox")?.sessions.map(
        (session) => session.id,
      ),
    ) === JSON.stringify(["active-child", "other-project-active"]),
    "Inbox mode should flatten branches and sort active threads across projects by activity",
  );
  const concurrentWorkingSessions: SidebarSession[] = [
    { id: "working-a", title: "Working A", createdAt: now, updatedAt: 20 },
    { id: "working-b", title: "Working B", createdAt: now, updatedAt: 10 },
  ];
  const concurrentWorkingIds = new Set(["working-a", "working-b"]);
  let workingActivityAt = reconcileWorkingSessionActivityAt(
    new Map(),
    concurrentWorkingSessions,
    concurrentWorkingIds,
  );
  const afterInterleavedEvents = concurrentWorkingSessions.map((session) => ({
    ...session,
    updatedAt: session.id === "working-a" ? 40 : 50,
  }));
  workingActivityAt = reconcileWorkingSessionActivityAt(
    workingActivityAt,
    afterInterleavedEvents,
    concurrentWorkingIds,
  );
  const stableWorkingGroups = groupSessionsByProjects(
    afterInterleavedEvents,
    [],
    { ...sidebar, pinnedSessionIds: [], projectFolders: [] },
    "",
    true,
    workingActivityAt,
  );
  assert(
    JSON.stringify(stableWorkingGroups[0]?.sessions.map((session) => session.id)) ===
      JSON.stringify(["working-a", "working-b"]),
    "interleaved progress events should not reorder concurrent working threads",
  );
  workingActivityAt = reconcileWorkingSessionActivityAt(
    workingActivityAt,
    afterInterleavedEvents,
    new Set(["working-a"]),
  );
  const completedWorkingGroups = groupSessionsByProjects(
    afterInterleavedEvents,
    [],
    { ...sidebar, pinnedSessionIds: [], projectFolders: [] },
    "",
    true,
    workingActivityAt,
  );
  assert(
    JSON.stringify(completedWorkingGroups[0]?.sessions.map((session) => session.id)) ===
      JSON.stringify(["working-b", "working-a"]),
    "a completed thread should rejoin normal recency ordering once",
  );
  assert(
    !inboxGroups.some(
      (group) =>
        group.id === "pinned" &&
        group.sessions.some((session) => session.id === "settled-pinned"),
    ),
    "settlement should temporarily override pinned placement",
  );
  const settledGroup = inboxGroups.at(-1);
  assert(
    settledGroup?.settled && settledGroup.id === "settled",
    "settled threads should render in one global bottom tier",
  );
  assert(
    JSON.stringify(settledGroup.sessions.map((session) => session.id)) ===
      JSON.stringify(["settled-child", "settled-pinned", "settled-parent"]),
    "settled threads should sort by settlement time",
  );
  assert(
    nextInboxSessionIdAfterSettle(inboxGroups, "active-child") ===
      "other-project-active",
    "settling the active thread should advance to the most recently active remaining thread",
  );
  assert(
    nextInboxSessionIdAfterSettle(
      [{
        id: "inbox",
        sessions: [tierSessions[2]],
        inbox: true,
      }],
      "active-child",
    ) == null,
    "settling the only active thread should keep it open",
  );
  const settledSearch = groupSessionsByProjects(
    tierSessions,
    inboxProjects,
    tierSidebar,
    "Pinned settled",
    true,
  );
  assert(
    settledSearch.at(-1)?.sessions[0]?.id === "settled-pinned",
    "search should include settled threads",
  );
  const projectSearch = groupSessionsByProjects(
    tierSessions,
    inboxProjects,
    tierSidebar,
    "Workspace B",
    true,
  );
  assert(
    projectSearch.find((group) => group.id === "inbox")?.sessions[0]?.id ===
      "other-project-active",
    "Inbox search should match compact project metadata",
  );

  assert(sidebarSectionNextRevealCount(12, 5, -1) === 5, "expanded sidebar sections should reveal a full next batch");
  assert(sidebarSectionNextRevealCount(7, 5, -1) === 2, "expanded sidebar sections should reveal only the remaining threads");
  assert(sidebarSectionNextRevealCount(12, 5, 8) === 4, "active overflow thread should not be counted as newly revealed");
  assert(
    !sidebarSectionUsesPagination(SIDEBAR_CHATS_SECTION_ID, false, false),
    "the ungrouped Chats section should always show every thread",
  );
  assert(
    sidebarSectionUsesPagination(projectSectionId(folder), false, false),
    "project sections should retain five-at-a-time pagination",
  );

  const runningWorkerParents = runningWorkerParentThreadIdsKey(
    ["proposed", "running", "done", "partial", "stopped", "error"].map((status) => ({
      run: { parent_thread_id: `thread-${status}`, status },
    })).concat([{ run: { parent_thread_id: "thread-running", status: "running" } }]),
  );
  assert(runningWorkerParents === "thread-running", "only running Worker Runs should activate their parent thread");
  assert(
    JSON.stringify(workingSessionIdsFromSignals({
      generatingSessionIds: ["generation", "shared"],
      hostBusySessionIds: ["host"],
      activityBusySessionIds: ["title", "shared"],
      runningWorkerParentIds: ["worker"],
    })) === JSON.stringify(["generation", "host", "shared", "title", "worker"]),
    "sidebar working state should union generation, host, background activity, and worker signals",
  );
} finally {
  await server.close();
}
