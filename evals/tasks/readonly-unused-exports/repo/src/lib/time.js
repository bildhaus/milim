export function isoDay(date) {
  return date.toISOString().slice(0, 10);
}

export function daysBetween(a, b) {
  return Math.round((b - a) / 86_400_000);
}
