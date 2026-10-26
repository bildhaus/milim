// HTTP client configuration: defaults merged with caller overrides.

export const DEFAULTS = Object.freeze({
  baseUrl: "https://api.example.test",
  timeoutMs: 5000,
  retries: 3,
  backoffMs: 200,
});

/** Defaults merged with `overrides`, validated. */
export function loadConfig(overrides = {}) {
  const config = { ...DEFAULTS, ...overrides };
  if (!(config.timeoutMs > 0)) throw new RangeError("timeoutMs must be a positive number of milliseconds");
  if (!Number.isInteger(config.retries) || config.retries < 0) throw new RangeError("retries must be a non-negative integer");
  if (!(config.backoffMs >= 0)) throw new RangeError("backoffMs must not be negative");
  return config;
}
