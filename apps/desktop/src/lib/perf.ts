type PerfMeasure = {
  count: number;
  totalMs: number;
  maxMs: number;
  lastMs: number;
};

export type MilimPerfSnapshot = {
  counters: Record<string, number>;
  measures: Record<string, PerfMeasure>;
};

type MilimPerfGlobal = MilimPerfSnapshot & {
  reset: () => void;
  snapshot: () => MilimPerfSnapshot;
};

declare global {
  interface Window {
    __MILIM_PERF__?: MilimPerfGlobal;
  }
}

const PERF_STORAGE_KEY = "milim.perf";

function shouldEnablePerf(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return new URLSearchParams(window.location.search).has("perf") || window.localStorage.getItem(PERF_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function ensurePerf(): MilimPerfGlobal | null {
  if (typeof window === "undefined") return null;
  if (window.__MILIM_PERF__) return window.__MILIM_PERF__;
  if (!shouldEnablePerf()) return null;
  const perf: MilimPerfGlobal = {
    counters: {},
    measures: {},
    reset() {
      this.counters = {};
      this.measures = {};
    },
    snapshot() {
      const measures: Record<string, PerfMeasure> = {};
      const entries = Object.entries(this.measures) as Array<[string, PerfMeasure]>;
      for (const [key, value] of entries) {
        measures[key] = {
          count: value.count,
          totalMs: value.totalMs,
          maxMs: value.maxMs,
          lastMs: value.lastMs,
        };
      }
      return {
        counters: { ...this.counters },
        measures,
      };
    },
  };
  window.__MILIM_PERF__ = perf;
  return perf;
}

export function incrementPerfCounter(name: string, amount = 1): void {
  const perf = ensurePerf();
  if (!perf) return;
  perf.counters[name] = (perf.counters[name] ?? 0) + amount;
}

export function recordPerfMeasure(name: string, valueMs: number): void {
  const perf = ensurePerf();
  if (!perf || !Number.isFinite(valueMs)) return;
  const current = perf.measures[name] ?? { count: 0, totalMs: 0, maxMs: 0, lastMs: 0 };
  current.count += 1;
  current.totalMs += valueMs;
  current.maxMs = Math.max(current.maxMs, valueMs);
  current.lastMs = valueMs;
  perf.measures[name] = current;
}

export function startPerfMeasure(name: string): () => void {
  const startedAt = typeof performance === "undefined" ? Date.now() : performance.now();
  return () => {
    const endedAt = typeof performance === "undefined" ? Date.now() : performance.now();
    recordPerfMeasure(name, endedAt - startedAt);
  };
}

export function markPerfRender(name: string): void {
  incrementPerfCounter(`render.${name}`);
}

export function markPerfStage(name: string): void {
  if (typeof performance === "undefined") return;
  const mark = `milim.${name}`;
  if (performance.getEntriesByName(mark, "mark").length === 0) {
    performance.mark(mark);
  }
}
