import { test } from "node:test";
import assert from "node:assert/strict";
import { reverseWords } from "../src/index.js";

test("reverses words and keeps spacing", () => {
  assert.equal(reverseWords("a  b c"), "c  b a");
});
