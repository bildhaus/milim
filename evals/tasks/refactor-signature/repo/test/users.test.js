import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { createUser, resetIds } from "../src/users.js";
import { signup } from "../src/signup.js";
import { isPrivileged, promoteInvite } from "../src/admin.js";
import { seedUsers } from "../src/seed.js";

beforeEach(() => resetIds());

test("createUser normalizes fields", () => {
  assert.deepEqual(createUser(" Ann ", "ANN@x.io"), { id: 1, name: "Ann", email: "ann@x.io", isAdmin: false });
});

test("signup welcomes a member", () => {
  const result = signup({ name: "Bo", email: "bo@x.io" });
  assert.equal(result.welcome, "Welcome, Bo!");
  assert.equal(isPrivileged(result.user), false);
});

test("invites become admins", () => {
  const result = promoteInvite({ name: "Cy", email: "cy@x.io" });
  assert.deepEqual(result.permissions, ["read", "write", "admin"]);
  assert.equal(isPrivileged(result.user), true);
});

test("seed data", () => {
  const users = seedUsers();
  assert.deepEqual(users.map((user) => user.isAdmin), [true, false, false]);
  assert.equal(users[2].email, "dana@example.com");
});
