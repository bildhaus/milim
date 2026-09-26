import { config } from "./config.js";
import { now } from "./clock.js";

const entries = new Map();

/** Remember `value` for the configured TTL. */
export function remember(key, value) {
  const ttlMs = config.cacheTTL * 1000;
  if (!(ttlMs > 0)) {
    return value;
  }
  entries.set(key, { value, expiresAt: now() + ttlMs });
  return value;
}

/** The cached value for `key`, or undefined once it expires. */
export function recall(key) {
  const entry = entries.get(key);
  if (!entry || entry.expiresAt <= now()) {
    entries.delete(key);
    return undefined;
  }
  return entry.value;
}

export function clearCache() {
  entries.clear();
}
