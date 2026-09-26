import { test } from "node:test";
import assert from "node:assert/strict";
import { weeklyTotals } from "../src/report.js";

test("groups by Monday-start weeks, including Sundays", () => {
  assert.deepEqual(
    weeklyTotals([
      { date: "2024-03-04", quantity: 1 },
      { date: "2024-03-09", quantity: 2 },
      { date: "2024-03-10", quantity: 4 },
      { date: "2024-03-11", quantity: 8 },
    ]),
    { "2024-03-04": 7, "2024-03-11": 8 },
  );
});
