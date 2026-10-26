// Sales report helpers. Order timestamps are ISO 8601 strings in UTC, and
// reports are bucketed by UTC calendar day ("YYYY-MM-DD").

const pad = (value) => String(value).padStart(2, "0");

/** The UTC calendar day of an ISO timestamp, as "YYYY-MM-DD". */
export function dayKey(timestamp) {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) throw new RangeError(`invalid timestamp: ${timestamp}`);
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** Revenue in cents per day, days in ascending order. Refunded orders are skipped. */
export function dailyTotals(orders) {
  const totals = {};
  for (const order of orders) {
    if (order.status === "refunded") continue;
    const key = dayKey(order.placedAt);
    totals[key] = (totals[key] ?? 0) + order.cents;
  }
  return Object.fromEntries(Object.entries(totals).sort(([a], [b]) => a.localeCompare(b)));
}

/** The busiest day by revenue, or null for no orders. Ties go to the earlier day. */
export function busiestDay(orders) {
  let best = null;
  for (const [day, cents] of Object.entries(dailyTotals(orders))) {
    if (!best || cents > best.cents) best = { day, cents };
  }
  return best;
}
