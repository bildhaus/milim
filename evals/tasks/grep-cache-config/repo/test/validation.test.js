import { test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";

test("missing required field is 422", () => {
  const response = handle({ method: "POST", path: "/users", body: {} });
  assert.equal(response.status, 422);
  assert.equal(response.body.error.code, "VALIDATION_FAILED");
});
