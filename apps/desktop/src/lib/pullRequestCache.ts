import type { PullRequestDetails, PullRequestListItem } from "../api.js";
import {
  readUserStateKey,
  writeUserStateKey,
} from "../persistence/userStateStorage.js";

const PULL_REQUEST_CACHE_KEY = "milim.pullRequests";

export type PullRequestFilter = "all" | "reviewing" | "authored";
export type PullRequestTab = "summary" | "timeline" | "code";

export interface PullRequestCache {
  items: PullRequestListItem[];
  detailsByUrl: Record<string, PullRequestDetails>;
  selectedUrl: string;
  filter: PullRequestFilter;
  query: string;
  tab: PullRequestTab;
}

export const EMPTY_PULL_REQUEST_CACHE: PullRequestCache = {
  items: [],
  detailsByUrl: {},
  selectedUrl: "",
  filter: "all",
  query: "",
  tab: "summary",
};

export function upsertPullRequestItems(
  current: PullRequestListItem[],
  incoming: PullRequestListItem[],
): PullRequestListItem[] {
  const byUrl = new Map(current.map((item) => [item.url, item]));
  for (const item of incoming) byUrl.set(item.url, item);
  return [...byUrl.values()].sort((left, right) =>
    (right.updatedAt ?? "").localeCompare(left.updatedAt ?? ""),
  );
}

export async function readPullRequestCache(): Promise<PullRequestCache> {
  try {
    const raw = await readUserStateKey(PULL_REQUEST_CACHE_KEY);
    if (!raw) return EMPTY_PULL_REQUEST_CACHE;
    const value = JSON.parse(raw) as Partial<PullRequestCache>;
    return {
      items: Array.isArray(value.items) ? value.items.slice(0, 100) : [],
      detailsByUrl:
        value.detailsByUrl && typeof value.detailsByUrl === "object"
          ? value.detailsByUrl
          : {},
      selectedUrl:
        typeof value.selectedUrl === "string" ? value.selectedUrl : "",
      filter:
        value.filter === "reviewing" || value.filter === "authored"
          ? value.filter
          : "all",
      query: typeof value.query === "string" ? value.query : "",
      tab:
        value.tab === "timeline" || value.tab === "code" ? value.tab : "summary",
    };
  } catch {
    return EMPTY_PULL_REQUEST_CACHE;
  }
}

export async function writePullRequestCache(
  cache: PullRequestCache,
): Promise<void> {
  const detailsByUrl = Object.fromEntries(
    Object.entries(cache.detailsByUrl).slice(-50),
  );
  await writeUserStateKey(
    PULL_REQUEST_CACHE_KEY,
    JSON.stringify({ ...cache, items: cache.items.slice(0, 100), detailsByUrl }),
  );
}
