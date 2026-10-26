import assert from "node:assert/strict";
import test from "node:test";
import { requestOptions } from "../src/client.js";
import { loadConfig } from "../src/config.js";

test("overrides win over defaults", () => {
  const options = requestOptions({ timeoutMs: 750, retries: 2, backoffMs: 100 });
  assert.equal(options.timeout, 750);
  assert.equal(options.attempts, 3);
  assert.deepEqual(options.delays, [100, 200]);
});

test("rejects a non-positive timeout", () => {
  assert.throws(() => loadConfig({ timeoutMs: 0 }), RangeError);
});

test("rejects a fractional retry count", () => {
  assert.throws(() => loadConfig({ retries: 1.5 }), RangeError);
});
