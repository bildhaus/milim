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
  settings?: { folder?: string };
  retryWorkspace?: { originalFolder: string };
  parentId?: string;
  archivedAt?: number;
  createdAt: number;
  updatedAt: number;
};
type SessionGroup = { id: string; project?: Project; sessions: SidebarSession[] };

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { groupSessionsByProjects, runningWorkerParentThreadIdsKey, sidebarSectionNextRevealCount } = await server.ssrLoadModule("/src/components/Sidebar.tsx") as {
    groupSessionsByProjects: (sessions: SidebarSession[], projects: Project[], sidebar: SessionSidebarState, query: string) => SessionGroup[];
    runningWorkerParentThreadIdsKey: (records: Array<{ run: { parent_thread_id: string; status: string } }>) => string;
    sidebarSectionNextRevealCount: (totalSessions: number, visibleLimit: number, activeIndex: number) => number;
  };
  const { SIDEBAR_CHATS_SECTION_ID, projectSectionId } = await server.ssrLoadModule("/src/sessions/store.ts") as {
    SIDEBAR_CHATS_SECTION_ID: string;
    projectSectionId: (folder?: string) => string;
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

  assert(sidebarSectionNextRevealCount(12, 5, -1) === 5, "expanded sidebar sections should reveal a full next batch");
  assert(sidebarSectionNextRevealCount(7, 5, -1) === 2, "expanded sidebar sections should reveal only the remaining threads");
  assert(sidebarSectionNextRevealCount(12, 5, 8) === 4, "active overflow thread should not be counted as newly revealed");

  const runningWorkerParents = runningWorkerParentThreadIdsKey(
    ["proposed", "running", "done", "partial", "stopped", "error"].map((status) => ({
      run: { parent_thread_id: `thread-${status}`, status },
    })).concat([{ run: { parent_thread_id: "thread-running", status: "running" } }]),
  );
  assert(runningWorkerParents === "thread-running", "only running Worker Runs should activate their parent thread");
} finally {
  await server.close();
}
