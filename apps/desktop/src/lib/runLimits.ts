export type RunLimits = {
  maxSteps: number | null;
  maxSeconds: number | null;
  maxCostUsd: number | null;
};

export const DEFAULT_RUN_LIMITS: RunLimits = { maxSteps: null, maxSeconds: null, maxCostUsd: null };

export function normalizeRunLimits(value: unknown): RunLimits {
  const raw = value && typeof value === "object" ? value as Partial<RunLimits> : {};
  const positive = (value: unknown, max: number, integer = false): number | null =>
    typeof value === "number" && Number.isFinite(value) && value > 0 && value <= max && (!integer || Number.isInteger(value)) ? value : null;
  return {
    maxSteps: positive(raw.maxSteps, 10_000, true),
    maxSeconds: positive(raw.maxSeconds, 86_400, true),
    maxCostUsd: positive(raw.maxCostUsd, 1_000_000),
  };
}
