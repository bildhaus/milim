import { test } from "node:test";
import assert from "node:assert/strict";
import { paginate } from "../src/paginate.js";

const letters = "abcdefghij".split("");

test("first page starts at the first item", () => {
  assert.deepEqual(paginate(letters, 1, 3).items, ["a", "b", "c"]);
});

test("last partial page is counted", () => {
  const result = paginate(letters, 4, 3);
  assert.deepEqual(result.items, ["j"]);
  assert.equal(result.totalPages, 4);
  assert.equal(result.hasNext, false);
});

test("rejects page zero", () => {
  assert.throws(() => paginate(letters, 0, 3), RangeError);
});
