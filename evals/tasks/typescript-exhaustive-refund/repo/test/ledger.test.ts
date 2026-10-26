import assert from "node:assert/strict";
import test from "node:test";
import { balanceAfter, type LedgerEvent } from "../src/events.ts";
import { describe } from "../src/format.ts";
import { totalsByKind } from "../src/summary.ts";

const events: LedgerEvent[] = [
  { kind: "deposit", amount: 10_000 },
  { kind: "withdrawal", amount: 2_500 },
  { kind: "transfer", amount: 1_000, direction: "out" },
  { kind: "refund", amount: 1_200, fee: 200 },
];

test("balance applies every event", () => {
  assert.equal(balanceAfter(events), 7_500);
});

test("statement lines", () => {
  assert.deepEqual(events.map(describe), ["deposit 100.00", "withdrawal 25.00", "transfer out 10.00", "refund 12.00 (fee 2.00)"]);
});

test("totals per kind", () => {
  assert.deepEqual(totalsByKind(events), { deposit: 10_000, withdrawal: 2_500, transfer: 1_000, refund: 1_200 });
});
