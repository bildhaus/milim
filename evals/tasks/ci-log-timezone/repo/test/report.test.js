import assert from "node:assert/strict";
import test from "node:test";
import { busiestDay, dailyTotals, dayKey } from "../src/report.js";

const orders = [
  { id: 1, placedAt: "2024-03-09T15:20:00Z", cents: 1_250, status: "paid" },
  { id: 2, placedAt: "2024-03-09T21:05:00Z", cents: 4_000, status: "paid" },
  { id: 3, placedAt: "2024-03-10T02:30:00Z", cents: 7_500, status: "paid" },
  { id: 4, placedAt: "2024-03-10T11:45:00Z", cents: 900, status: "refunded" },
  { id: 5, placedAt: "2024-03-10T12:00:00Z", cents: 2_100, status: "paid" },
];

test("day keys", () => {
  assert.equal(dayKey("2024-03-09T15:20:00Z"), "2024-03-09");
  assert.throws(() => dayKey("yesterday"), RangeError);
});

test("groups orders into daily totals", () => {
  assert.deepEqual(dailyTotals(orders), { "2024-03-09": 5_250, "2024-03-10": 9_600 });
});

test("busiest day", () => {
  assert.deepEqual(busiestDay(orders), { day: "2024-03-10", cents: 9_600 });
  assert.equal(busiestDay([]), null);
});
