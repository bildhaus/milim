import { test } from "node:test";
import assert from "node:assert/strict";
import { clientSettings, retryDelays } from "../src/client.js";

test("every client has settings in both environments", () => {
  for (const env of ["staging", "production"]) {
    for (const name of ["auth", "payments", "paymentWebhooks", "search", "notifications"]) {
      assert.ok(clientSettings(env, name).baseUrl.startsWith("https://"));
    }
  }
});

test("retry delays double", () => {
  assert.deepEqual(retryDelays("staging", "auth"), [250, 500]);
});
