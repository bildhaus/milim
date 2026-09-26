import { test } from "node:test";
import assert from "node:assert/strict";
import { loadConfig } from "../src/config.js";

test("parses v1", () => {
  assert.deepEqual(loadConfig("host = a\nport = 1\nfeatures = x, y\n"), { host: "a", port: 1, features: ["x", "y"] });
});

test("defaults", () => {
  assert.deepEqual(loadConfig("# empty\n"), { host: "127.0.0.1", port: 8080, features: [] });
});
