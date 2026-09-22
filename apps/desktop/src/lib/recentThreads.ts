import { folderLabel } from "./projectColors.js";

export type RecentThreadSession = {
  id: string;
  title: string;
  archivedAt?: number;
  settings?: {
    folder?: string;
    model?: string;
  };
};

export type RecentThreadProject = {
  name: string;
  folder: string;
  archivedAt?: number;
};

export type RecentThreadSwitcherItem = {
  id: string;
  title: string;
  metadata: string;
};

const DEFAULT_HISTORY_LIMIT = 30;
const DEFAULT_SWITCHER_LIMIT = 10;

export function rememberRecentThread(
  ids: readonly string[],
  id: string,
  limit = DEFAULT_HISTORY_LIMIT,
): string[] {
  if (!id) return ids.slice(0, limit);
  return [id, ...ids.filter((item) => item !== id)].slice(0, limit);
}

export function recentThreadSwitcherItems(
  recentIds: readonly string[],
  currentId: string,
  sessions: readonly RecentThreadSession[],
  projects: readonly RecentThreadProject[],
  limit = DEFAULT_SWITCHER_LIMIT,
): RecentThreadSwitcherItem[] {
  const sessionById = new Map(sessions.map((session) => [session.id, session]));
  const archivedProjectFolders = new Set(
    projects.filter((project) => project.archivedAt).map((project) => project.folder),
  );
  const activeProjectByFolder = new Map(
    projects.filter((project) => !project.archivedAt).map((project) => [project.folder, project]),
  );
  const visibleIds = uniqueIds(recentIds).filter((id) => {
    const session = sessionById.get(id);
    if (!session || session.archivedAt) return false;
    const folder = session.settings?.folder?.trim() ?? "";
    return !folder || !archivedProjectFolders.has(folder);
  });
  const currentVisible = visibleIds.includes(currentId);
  const previousIds = visibleIds.filter((id) => id !== currentId);
  if (previousIds.length === 0) return [];
  const orderedIds = currentVisible && limit > 1
    ? [...previousIds.slice(0, Math.max(0, limit - 1)), currentId]
    : previousIds.slice(0, limit);

  return orderedIds.map((id) => {
    const session = sessionById.get(id)!;
    const folder = session.settings?.folder?.trim() ?? "";
    const project = folder ? activeProjectByFolder.get(folder) : undefined;
    return {
      id,
      title: session.title.trim() || "New chat",
      metadata: [
        folder ? project?.name ?? folderLabel(folder) : "",
        session.settings?.model,
      ].filter(Boolean).join(" | "),
    };
  });
}

export function nextRecentThreadSwitcherIndex(index: number, length: number): number {
  return length > 0 ? (index + 1) % length : 0;
}

function uniqueIds(ids: readonly string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const id of ids) {
    if (!id || seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  return result;
}

