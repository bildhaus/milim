import assert from "node:assert/strict";
import test from "node:test";
import { parseUser } from "../src/users.js";

test("parses a valid user", () => {
  assert.deepEqual(parseUser({ id: "u1", email: " Ada@Example.test ", age: 36 }), { id: "u1", email: "ada@example.test", age: 36 });
});

test("rejects missing and mistyped fields", () => {
  assert.throws(() => parseUser({ email: "a@b.c" }), /id is required/);
  assert.throws(() => parseUser({ id: "u1", email: "a@b.c", age: 1.5 }), /age must be an integer/);
});

test("rejects unknown fields", () => {
  assert.throws(() => parseUser({ id: "u1", email: "a@b.c", nickname: "x" }), /unknown field nickname/);
});
