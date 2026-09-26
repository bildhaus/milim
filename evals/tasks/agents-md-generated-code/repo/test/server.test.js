import { test } from "node:test";
import assert from "node:assert/strict";
import { dispatch } from "../src/server.js";

test("items round trip", () => {
  assert.equal(dispatch({ method: "POST", path: "/items", body: { name: "a" } }).status, 201);
  assert.deepEqual(dispatch({ method: "GET", path: "/items" }).body, { items: [{ name: "a" }] });
});

test("unknown route", () => {
  assert.equal(dispatch({ method: "GET", path: "/nope" }).status, 404);
});
