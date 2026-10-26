// Half-open numeric intervals `[start, end)`: `start` is included, `end` is
// not. Used to merge booked time ranges (in minutes) into free/busy blocks.

function validate(interval, index) {
  if (!Array.isArray(interval) || interval.length !== 2) {
    throw new TypeError(`interval ${index} must be a [start, end] pair`);
  }
  const [start, end] = interval;
  if (!Number.isFinite(start) || !Number.isFinite(end)) {
    throw new TypeError(`interval ${index} must have finite numbers`);
  }
  if (start > end) throw new RangeError(`interval ${index} ends before it starts`);
}

/**
 * Merge intervals that overlap or touch (`[1, 3)` and `[3, 5)` become
 * `[1, 5)`). Accepts intervals in any order and returns new `[start, end]`
 * arrays sorted by start; the input array and its intervals are not
 * modified. Empty intervals (`start === end`) are dropped. Throws a
 * TypeError for anything that is not a pair of finite numbers and a
 * RangeError when an interval ends before it starts.
 */
export function mergeIntervals(intervals) {
  intervals.forEach(validate);
  const sorted = intervals
    .filter(([start, end]) => start < end)
    .map(([start, end]) => [start, end])
    .sort((a, b) => a[0] - b[0]);
  const merged = [];
  for (const [start, end] of sorted) {
    const last = merged.at(-1);
    if (last && start <= last[1]) last[1] = Math.max(last[1], end);
    else merged.push([start, end]);
  }
  return merged;
}

/** Total length covered by the intervals, counting overlaps once. */
export function totalLength(intervals) {
  return mergeIntervals(intervals).reduce((sum, [start, end]) => sum + (end - start), 0);
}

/** Whether `point` lies inside any interval (the start counts, the end does not). */
export function contains(intervals, point) {
  return mergeIntervals(intervals).some(([start, end]) => start <= point && point < end);
}

/** The gaps between the merged intervals, inside `[from, to)`. */
export function gaps(intervals, from, to) {
  const out = [];
  let cursor = from;
  for (const [start, end] of mergeIntervals(intervals)) {
    if (end <= from || start >= to) continue;
    if (start > cursor) out.push([cursor, start]);
    cursor = Math.max(cursor, end);
  }
  if (cursor < to) out.push([cursor, to]);
  return out;
}
