import assert from "node:assert/strict";
import test from "node:test";
import { displayName, initials } from "../src/names.js";

test("display names are trimmed and capitalized", () => {
  assert.equal(displayName("  ada   LOVELACE "), "Ada Lovelace");
});

test("initials use at most three words", () => {
  assert.equal(initials("grace brewster murray hopper"), "GBM");
});
