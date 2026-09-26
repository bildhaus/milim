import { test } from "node:test";
import assert from "node:assert/strict";
import { createStore } from "../src/store.js";
import { run } from "../src/cli.js";

test("add, complete, and list", () => {
  const store = createStore();
  run(store, ["add", "Buy", "milk"]);
  run(store, ["add", "Walk dog"]);
  run(store, ["done", "1"]);
  assert.equal(run(store, ["list"]).length, 1);
  assert.equal(run(store, ["list", "--all"]).length, 2);
});
