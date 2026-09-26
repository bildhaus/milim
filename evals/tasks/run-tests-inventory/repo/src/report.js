/** Group shipments by ISO week start (Monday, UTC) and sum quantities. */
export function weeklyTotals(shipments) {
  const totals = new Map();
  for (const { date, quantity } of shipments) {
    const day = new Date(`${date}T00:00:00Z`);
    const monday = new Date(day);
    monday.setUTCDate(day.getUTCDate() - day.getUTCDay() + 1);
    const key = monday.toISOString().slice(0, 10);
    totals.set(key, (totals.get(key) ?? 0) + quantity);
  }
  return Object.fromEntries([...totals].sort(([a], [b]) => a.localeCompare(b)));
}
