import assert from "node:assert/strict";
import test from "node:test";
import { isoWeek, isoWeekday } from "../src/dates.js";

test("iso weekday counts Monday as 1 and Sunday as 7", () => {
  assert.equal(isoWeekday("2024-03-04"), 1);
  assert.equal(isoWeekday("2024-03-10"), 7);
});

test("iso week of a mid-year date", () => {
  assert.equal(isoWeek("2024-06-12"), 24);
});

test("early January can belong to the previous ISO year's last week", () => {
  assert.equal(isoWeek("2021-01-03"), 53);
  assert.equal(isoWeek("2022-01-02"), 52);
});
