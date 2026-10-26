// Wrong: assumes closed intervals, so the suite fails on the real module.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync("test", { recursive: true });
writeFileSync(
  "test/intervals.test.js",
  `import assert from "node:assert/strict";
import test from "node:test";
import { contains, mergeIntervals } from "../src/intervals.js";

test("merges", () => {
  assert.deepEqual(mergeIntervals([[1, 3], [3, 5]]), [[1, 5]]);
});

test("the end point is inside", () => {
  assert.equal(contains([[1, 5]], 5), true);
});
`,
);
