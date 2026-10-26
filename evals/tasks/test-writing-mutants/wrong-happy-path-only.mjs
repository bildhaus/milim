// Wrong: a plausible but shallow suite that only covers the obvious cases.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync("test", { recursive: true });
writeFileSync(
  "test/intervals.test.js",
  `import assert from "node:assert/strict";
import test from "node:test";
import { contains, gaps, mergeIntervals, totalLength } from "../src/intervals.js";

test("merges overlapping intervals", () => {
  assert.deepEqual(mergeIntervals([[1, 4], [2, 6]]), [[1, 6]]);
});

test("total length", () => {
  assert.equal(totalLength([[0, 10]]), 10);
});

test("contains", () => {
  assert.equal(contains([[1, 5]], 3), true);
});

test("gaps", () => {
  assert.deepEqual(gaps([[2, 4]], 0, 10), [[0, 2], [4, 10]]);
});
`,
);
