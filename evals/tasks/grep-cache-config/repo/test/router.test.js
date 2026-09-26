import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";
import { db } from "../src/lib/db.js";

beforeEach(() => db.reset());

test("unknown route is 404", () => {
  assert.equal(handle({ method: "GET", path: "/nope" }).status, 404);
});

test("create then get a user", () => {
  const created = handle({ method: "POST", path: "/users", body: { name: "Apollo" } });
  assert.equal(created.status, 201);
  const fetched = handle({ method: "GET", path: `/users/${created.body.id}` });
  assert.equal(fetched.body.name, "Apollo");
});
