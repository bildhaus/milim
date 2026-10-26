// Ledger events. Amounts are integer cents.

export interface Deposit {
  kind: "deposit";
  amount: number;
}

export interface Withdrawal {
  kind: "withdrawal";
  amount: number;
}

export interface Transfer {
  kind: "transfer";
  amount: number;
  /** "in" credits this account, "out" debits it. */
  direction: "in" | "out";
}

/**
 * A refund of an earlier purchase: `amount` comes back to the account minus
 * the merchant's `fee`, if any (a missing fee means no fee).
 */
export interface Refund {
  kind: "refund";
  amount: number;
  fee?: number;
}

export type LedgerEvent = Deposit | Withdrawal | Transfer | Refund;

export function assertNever(value: never): never {
  throw new Error(`unhandled ledger event: ${JSON.stringify(value)}`);
}

/** The balance after `event`. */
export function applyEvent(balance: number, event: LedgerEvent): number {
  switch (event.kind) {
    case "deposit":
      return balance + event.amount;
    case "withdrawal":
      return balance - event.amount;
    case "transfer":
      return event.direction === "in" ? balance + event.amount : balance - event.amount;
    default:
      return assertNever(event);
  }
}

export function balanceAfter(events: LedgerEvent[], opening = 0): number {
  return events.reduce(applyEvent, opening);
}
