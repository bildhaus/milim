import { test } from "node:test";
import assert from "node:assert/strict";
import { convert, format } from "../src/convert.js";

test("USD to EUR", () => {
  assert.equal(convert(10_000, "USD", "EUR"), 9_200);
  assert.equal(format(9_200, "EUR"), "92.00 EUR");
});

test("USD to JPY", () => {
  assert.equal(convert(10_000, "USD", "JPY"), 15_150);
  assert.equal(format(15_150, "JPY"), "15150 JPY");
});

test("JPY back to USD", () => {
  assert.equal(convert(15_150, "JPY", "USD"), 10_000);
});
