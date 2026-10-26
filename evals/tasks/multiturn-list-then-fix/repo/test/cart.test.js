import assert from "node:assert/strict";
import test from "node:test";
import { cartTotal, formatCents } from "../src/cart.js";

test("cart total sums cents times quantity", () => {
  assert.equal(cartTotal([{ cents: 250, qty: 2 }, { cents: 99, qty: 1 }]), 599);
});

test("cart total rounds half-cent totals up", () => {
  assert.equal(cartTotal([{ cents: 105, qty: 1 }], 0.5), 53);
  assert.equal(cartTotal([{ cents: 333, qty: 3 }], 0.1), 899);
});

test("formats cents as dollars", () => {
  assert.equal(formatCents(1234), "$12.34");
  assert.equal(formatCents(5), "$0.05");
});
