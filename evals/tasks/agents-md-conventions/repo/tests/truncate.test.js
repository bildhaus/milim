import { test } from "node:test";
import assert from "node:assert/strict";
import { truncate } from "../src/index.js";

test("truncates with an ellipsis", () => {
  assert.equal(truncate("abcdef", 4), "abc…");
  assert.equal(truncate("abc", 4), "abc");
});
