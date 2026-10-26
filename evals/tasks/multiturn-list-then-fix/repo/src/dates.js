// Calendar helpers for weekly sales reports. Dates are "YYYY-MM-DD" strings
// interpreted in UTC.

const DAY_MS = 24 * 60 * 60 * 1000;

function parse(date) {
  const [year, month, day] = date.split("-").map(Number);
  return new Date(Date.UTC(year, month - 1, day));
}

/** ISO 8601 day of week: Monday = 1 ... Sunday = 7. */
export function isoWeekday(date) {
  return parse(date).getUTCDay() || 7;
}

/**
 * ISO 8601 week number (1-53). Weeks start on Monday and week 1 is the week
 * containing the year's first Thursday, so early January can belong to the
 * previous year's last week and late December to next year's week 1.
 */
export function isoWeek(date) {
  const value = parse(date);
  const start = Date.UTC(value.getUTCFullYear(), 0, 1);
  const dayOfYear = Math.floor((value - start) / DAY_MS) + 1;
  return Math.ceil(dayOfYear / 7);
}
