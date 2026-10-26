// Another correct suite: table-driven cases in a differently named file.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync("test/unit", { recursive: true });
writeFileSync(
  "test/unit/intervals.spec.mjs",
  `import assert from "node:assert/strict";
import { describe, it } from "node:test";
import * as intervals from "../../src/intervals.js";

const merges = [
  ["overlap", [[3, 6], [1, 4]], [[1, 6]]],
  ["touching", [[3, 5], [1, 3]], [[1, 5]]],
  ["nested", [[0, 9], [3, 4], [5, 6]], [[0, 9]]],
  ["disjoint unsorted", [[7, 8], [1, 2]], [[1, 2], [7, 8]]],
  ["empty interval", [[5, 5]], []],
];

describe("mergeIntervals", () => {
  for (const [name, input, expected] of merges) {
    it(name, () => {
      const before = JSON.stringify(input);
      assert.deepEqual(intervals.mergeIntervals(input), expected);
      assert.equal(JSON.stringify(input), before, "input unchanged");
    });
  }
  it("validates", () => {
    for (const bad of [[[2, 1]]]) assert.throws(() => intervals.mergeIntervals(bad), RangeError);
    for (const bad of [[[0, NaN]], [[-Infinity, 0]], [["a", 1]]]) assert.throws(() => intervals.mergeIntervals(bad), TypeError);
  });
});

describe("queries", () => {
  it("totalLength", () => assert.equal(intervals.totalLength([[0, 4], [2, 6]]), 6));
  it("contains", () => {
    assert.ok(intervals.contains([[0, 4]], 0));
    assert.ok(!intervals.contains([[0, 4]], 4));
  });
  it("gaps", () => {
    assert.deepEqual(intervals.gaps([[3, 5]], 0, 8), [[0, 3], [5, 8]]);
    assert.deepEqual(intervals.gaps([[0, 3], [3, 8]], 0, 8), []);
  });
});
`,
);
