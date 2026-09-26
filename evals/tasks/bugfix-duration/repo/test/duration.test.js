import { test } from "node:test";
import assert from "node:assert/strict";
import { formatDuration, parseDuration } from "../src/duration.js";

test("single units", () => {
  assert.equal(parseDuration("250ms"), 250);
  assert.equal(parseDuration("90s"), 90_000);
  assert.equal(parseDuration("2h"), 7_200_000);
});

test("compound durations", () => {
  assert.equal(parseDuration("1h30m"), 5_400_000);
  assert.equal(parseDuration("2d 4h"), 187_200_000);
});

test("round trip", () => {
  for (const ms of [1, 999, 61_000, 5_400_000, 90_061_001]) {
    assert.equal(parseDuration(formatDuration(ms)), ms);
  }
});

test("rejects garbage", () => {
  assert.throws(() => parseDuration(""), TypeError);
  assert.throws(() => parseDuration("5 minutes"), TypeError);
});
