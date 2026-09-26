const BASE = 10;
const LARGE_ORDER = 25_000;
const CAP = 22;

/**
 * Gold customers get a base rate, more on large orders, plus their loyalty
 * bonus, but never more than the cap.
 */
export function goldRate(customer, subtotalCents, bonus) {
  let percent = BASE;
  if (subtotalCents >= LARGE_ORDER) percent += 7;
  percent += bonus;
  return Math.min(percent, CAP);
}
