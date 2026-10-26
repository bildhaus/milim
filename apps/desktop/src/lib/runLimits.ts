export type RunLimits = {
  maxSteps: number | null;
  maxSeconds: number | null;
  maxCostUsd: number | null;
};

export const DEFAULT_RUN_LIMITS: RunLimits = { maxSteps: null, maxSeconds: null, maxCostUsd: null };

/** Per-Worker budget for delegated Workers. `null` keeps the built-in default. */
export type WorkerLimits = {
  maxSteps: number | null;
  maxTokens: number | null;
  maxMinutes: number | null;
};

export const DEFAULT_WORKER_LIMITS: WorkerLimits = { maxSteps: null, maxTokens: null, maxMinutes: null };

/** Built-in Worker budget the server applies to unset fields. */
export const WORKER_LIMIT_DEFAULTS = { maxSteps: 60, maxTokens: 4_000_000, maxMinutes: 30 } as const;

const positive = (value: unknown, max: number, integer = false, min = 0): number | null =>
  typeof value === "number" && Number.isFinite(value) && value > min && value <= max && (!integer || Number.isInteger(value)) ? value : null;

export function normalizeRunLimits(value: unknown): RunLimits {
  const raw = value && typeof value === "object" ? value as Partial<RunLimits> : {};
  return {
    maxSteps: positive(raw.maxSteps, 10_000, true),
    maxSeconds: positive(raw.maxSeconds, 86_400, true),
    maxCostUsd: positive(raw.maxCostUsd, 1_000_000),
  };
}

export function normalizeWorkerLimits(value: unknown): WorkerLimits {
  const raw = value && typeof value === "object" ? value as Partial<WorkerLimits> : {};
  return {
    maxSteps: positive(raw.maxSteps, 1_000, true),
    maxTokens: positive(raw.maxTokens, 100_000_000, true, 9_999),
    maxMinutes: positive(raw.maxMinutes, 1_440, true),
  };
}
