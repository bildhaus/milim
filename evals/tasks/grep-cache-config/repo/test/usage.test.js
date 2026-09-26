import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";
import { db } from "../src/lib/db.js";

beforeEach(() => db.reset());

test("usage under the quota is recorded", () => {
  db.upsert("plans", "acme", { limit: 10 });
  const response = handle({ method: "POST", path: "/usage/acme", body: { units: 4 } });
  assert.equal(response.status, 200);
  assert.equal(response.body.used, 4);
});
