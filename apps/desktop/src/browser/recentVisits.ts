import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { normalizeArtifactBrowserUrl } from "../lib/artifacts.js";
import { userStateStorage } from "../persistence/userStateStorage.js";

const MAX_RECENT_VISITS = 24;

export interface BrowserRecentVisit {
  url: string;
  title?: string;
  visitedAt: number;
}

interface BrowserRecentVisitsState {
  visits: BrowserRecentVisit[];
  recordVisit: (url: string, title?: string) => void;
  clearVisits: () => void;
}

function normalizeVisit(value: unknown): BrowserRecentVisit | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const visit = value as Partial<BrowserRecentVisit>;
  const url = typeof visit.url === "string" ? normalizeArtifactBrowserUrl(visit.url) : null;
  if (!url) return null;
  const title = typeof visit.title === "string" ? visit.title.trim().slice(0, 160) : "";
  return {
    url,
    ...(title ? { title } : {}),
    visitedAt: Number.isFinite(visit.visitedAt) ? Number(visit.visitedAt) : 0,
  };
}

function normalizeVisits(value: unknown): BrowserRecentVisit[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value.flatMap((item) => {
    const visit = normalizeVisit(item);
    if (!visit || seen.has(visit.url)) return [];
    seen.add(visit.url);
    return [visit];
  }).slice(0, MAX_RECENT_VISITS);
}

export const useBrowserRecentVisits = create<BrowserRecentVisitsState>()(
  persist(
    (set) => ({
      visits: [],
      recordVisit: (rawUrl, rawTitle) => set((state) => {
        const url = normalizeArtifactBrowserUrl(rawUrl);
        if (!url) return {};
        const title = rawTitle?.trim().slice(0, 160);
        return {
          visits: [
            { url, ...(title ? { title } : {}), visitedAt: Date.now() },
            ...state.visits.filter((visit) => visit.url !== url),
          ].slice(0, MAX_RECENT_VISITS),
        };
      }),
      clearVisits: () => set({ visits: [] }),
    }),
    {
      name: "milim.browserRecentVisits",
      storage: createJSONStorage(() => userStateStorage),
      merge: (persisted, current) => ({
        ...current,
        visits: normalizeVisits((persisted as Partial<BrowserRecentVisitsState> | null)?.visits),
      }),
      partialize: (state) => ({ visits: state.visits }),
    },
  ),
);
