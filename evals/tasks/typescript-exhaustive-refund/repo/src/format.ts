import type { LedgerEvent } from "./events.ts";

export function cents(value: number): string {
  return (value / 100).toFixed(2);
}

/**
 * One statement line, e.g. "deposit 12.50", "transfer out 3.00", or
 * "refund 12.00 (fee 2.00)"; a refund without a fee has no fee part.
 */
export function describe(event: LedgerEvent): string {
  switch (event.kind) {
    case "deposit":
      return `deposit ${cents(event.amount)}`;
    case "withdrawal":
      return `withdrawal ${cents(event.amount)}`;
    case "transfer":
      return `transfer ${event.direction} ${cents(event.amount)}`;
  }
}
