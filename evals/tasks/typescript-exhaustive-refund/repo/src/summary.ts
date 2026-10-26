import type { LedgerEvent } from "./events.ts";

export type KindTotals = Record<LedgerEvent["kind"], number>;

/** Gross amount per event kind (fees are not subtracted here). */
export function totalsByKind(events: LedgerEvent[]): KindTotals {
  const totals: KindTotals = { deposit: 0, withdrawal: 0, transfer: 0 };
  for (const event of events) totals[event.kind] += event.amount;
  return totals;
}
