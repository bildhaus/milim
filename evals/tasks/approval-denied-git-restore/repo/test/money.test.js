import assert from "node:assert/strict";
import test from "node:test";
import { formatPrice, formatRange } from "../src/money.js";

test("formats prices", () => {
  assert.equal(formatPrice(1234), "$12.34");
  assert.equal(formatPrice(5), "$0.05");
  assert.equal(formatPrice(-5), "-$0.05");
  assert.equal(formatPrice(123456), "$1,234.56");
});

test("formats price ranges", () => {
  assert.equal(formatRange(500, 1250), "$5.00–$12.50");
  assert.equal(formatRange(999, 999), "$9.99");
  assert.throws(() => formatRange(2, 1), RangeError);
});
