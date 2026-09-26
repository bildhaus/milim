/**
 * A bounded least-recently-used cache whose entries also expire.
 *
 * - `capacity` is a positive integer; otherwise the constructor throws a
 *   RangeError.
 * - `ttlMs` is the default time-to-live in milliseconds (positive integer or
 *   `Infinity`, default `Infinity`); otherwise the constructor throws a
 *   RangeError.
 * - `now` is a zero-argument function returning the current time in
 *   milliseconds (default `Date.now`). Every time-based decision must use it.
 * - An entry is expired when `now() >= storedAt + ttl`. Expired entries behave
 *   exactly as if absent: `get` returns undefined, `has` returns false, and
 *   they are not counted by `size`.
 */
export class TtlCache {
  constructor({ capacity, ttlMs = Infinity, now = Date.now } = {}) {
    throw new Error("not implemented");
  }

  /**
   * Return the value for `key`, or undefined when absent or expired. A hit
   * marks the entry as most recently used. It does not extend its TTL.
   */
  get(key) {}

  /**
   * Store `value` under `key` with `ttlMs` (defaults to the cache TTL; same
   * validation as the constructor). Setting an existing key replaces its
   * value, restarts its TTL, and marks it most recently used. When a new key
   * would exceed capacity, first drop every expired entry; if still full,
   * evict the least recently used entry. Returns `this`.
   */
  set(key, value, ttlMs) {}

  /** True when `key` is present and not expired. Does not affect recency. */
  has(key) {}

  /** Remove `key`. Returns true when a live (unexpired) entry was removed. */
  delete(key) {}

  /** Number of live (unexpired) entries. */
  get size() {}

  /** Live keys from least to most recently used. */
  keys() {}
}
