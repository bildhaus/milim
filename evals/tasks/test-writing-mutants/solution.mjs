// Reference: one test per documented behavior.
import { mkdirSync, writeFileSync } from "node:fs";

mkdirSync("test", { recursive: true });
writeFileSync(
  "test/intervals.test.js",
  `import assert from "node:assert/strict";
import test from "node:test";
import { contains, gaps, mergeIntervals, totalLength } from "../src/intervals.js";

test("merges overlapping intervals", () => {
  assert.deepEqual(mergeIntervals([[1, 4], [2, 6], [8, 9]]), [[1, 6], [8, 9]]);
});

test("merges touching intervals", () => {
  assert.deepEqual(mergeIntervals([[1, 3], [3, 5]]), [[1, 5]]);
});

test("a nested interval does not shrink the merge", () => {
  assert.deepEqual(mergeIntervals([[1, 10], [2, 3]]), [[1, 10]]);
});

test("accepts unsorted input and returns it sorted", () => {
  assert.deepEqual(mergeIntervals([[8, 9], [1, 2], [4, 5]]), [[1, 2], [4, 5], [8, 9]]);
});

test("does not modify the input", () => {
  const input = [[5, 7], [1, 3], [2, 6]];
  const copy = structuredClone(input);
  mergeIntervals(input);
  assert.deepEqual(input, copy);
});

test("drops empty intervals", () => {
  assert.deepEqual(mergeIntervals([[2, 2], [4, 6]]), [[4, 6]]);
  assert.deepEqual(mergeIntervals([]), []);
});

test("rejects bad intervals", () => {
  assert.throws(() => mergeIntervals([[5, 1]]), RangeError);
  assert.throws(() => mergeIntervals([[1, Number.NaN]]), TypeError);
  assert.throws(() => mergeIntervals([[0, Infinity]]), TypeError);
  assert.throws(() => mergeIntervals([[1, "2"]]), TypeError);
  assert.throws(() => mergeIntervals([[1]]), TypeError);
});

test("total length counts overlaps once", () => {
  assert.equal(totalLength([[0, 10], [5, 15], [20, 25]]), 20);
});

test("contains includes the start and excludes the end", () => {
  assert.equal(contains([[1, 5]], 1), true);
  assert.equal(contains([[1, 5]], 4.5), true);
  assert.equal(contains([[1, 5]], 5), false);
});

test("gaps inside a window", () => {
  assert.deepEqual(gaps([[2, 4], [6, 7]], 0, 10), [[0, 2], [4, 6], [7, 10]]);
  assert.deepEqual(gaps([[0, 10]], 0, 10), []);
  assert.deepEqual(gaps([[0, 12]], 2, 10), []);
});
`,
);
