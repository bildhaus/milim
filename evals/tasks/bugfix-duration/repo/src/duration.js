const UNIT_MS = {
  ms: 1,
  s: 1000,
  m: 60 * 1000,
  h: 60 * 60 * 1000,
  d: 24 * 60 * 60 * 1000,
};

/**
 * Parse a duration such as "250ms", "90s", "1h30m", or "2d 4h" into
 * milliseconds. Whitespace between parts is allowed. Throws a TypeError for
 * anything else, including an empty string.
 */
export function parseDuration(text) {
  if (typeof text !== "string" || text.trim() === "") {
    throw new TypeError("duration must be a non-empty string");
  }
  const match = /^\s*(\d+)(ms|s|m|h|d)\s*$/.exec(text);
  if (!match) {
    throw new TypeError(`invalid duration: ${text}`);
  }
  return Number(match[1]) * UNIT_MS[match[2]];
}

/** Format milliseconds as the shortest compound duration, e.g. "1h30m". */
export function formatDuration(ms) {
  if (!Number.isInteger(ms) || ms < 0) {
    throw new TypeError("ms must be a non-negative integer");
  }
  if (ms === 0) return "0ms";
  let rest = ms;
  let out = "";
  for (const unit of ["d", "h", "m", "s", "ms"]) {
    const size = UNIT_MS[unit];
    const count = Math.floor(rest / size);
    if (count > 0) {
      out += `${count}${unit}`;
      rest -= count * size;
    }
  }
  return out;
}
