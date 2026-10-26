import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { db } from "../src/db.js";
import { dispatch } from "../src/router.js";

beforeEach(() => db.reset());

test("archive a project as its owner", async () => {
  const response = await dispatch("POST", "/projects/p1/archive");
  assert.equal(response.status, 200);
  assert.equal(response.body.archived, true);
});

test("archiving someone else's project is forbidden", async () => {
  assert.equal((await dispatch("POST", "/projects/p2/archive")).status, 403);
});

test("create a user", async () => {
  const response = await dispatch("POST", "/users", { body: { name: " Edsger ", email: "EWD@example.test" } });
  assert.equal(response.status, 201);
  assert.equal(response.body.name, "Edsger");
  assert.equal(response.body.email, "ewd@example.test");
});

test("creating a user needs an email", async () => {
  assert.equal((await dispatch("POST", "/users", { body: { name: "X" } })).status, 400);
});

test("delete a user", async () => {
  assert.equal((await dispatch("DELETE", "/users/u3")).status, 204);
  assert.equal((await dispatch("GET", "/users/u3")).status, 404);
});

test("export the caller's active projects", async () => {
  const response = await dispatch("GET", "/reports/projects.csv");
  assert.equal(response.body, "id,name\np3,Analytical notes\np1,Engine");
});

test("get a user", async () => {
  assert.equal((await dispatch("GET", "/users/u2")).body.name, "Grace Hopper");
  assert.equal((await dispatch("GET", "/users/nope")).status, 404);
});

test("list projects by owner", async () => {
  const response = await dispatch("GET", "/projects?owner=u1");
  assert.deepEqual(response.body.map((project) => project.id), ["p3", "p1"]);
});

test("rename a project", async () => {
  assert.equal((await dispatch("PATCH", "/projects/p1", { body: { name: "Difference engine" } })).body.name, "Difference engine");
  assert.equal((await dispatch("PATCH", "/projects/p1", { body: {} })).status, 400);
});

test("start a password reset", async () => {
  assert.equal((await dispatch("POST", "/password-resets", { body: { email: "GRACE@example.test" } })).status, 202);
  assert.equal((await dispatch("POST", "/password-resets", { body: { email: "who@example.test" } })).status, 404);
});

test("search users", async () => {
  assert.deepEqual((await dispatch("GET", "/users?q=al")).body, ["u3"]);
  assert.equal((await dispatch("GET", "/users?q=a")).status, 400);
});

test("share a project", async () => {
  const response = await dispatch("POST", "/projects/p1/shares", { body: { userId: "u2" } });
  assert.deepEqual(response.body.sharedWith, ["u2"]);
});

test("update a user", async () => {
  const response = await dispatch("PATCH", "/users/u2", { body: { name: "Rear Admiral Hopper", role: "admin" } });
  assert.equal(response.body.name, "Rear Admiral Hopper");
  assert.equal(response.body.role, undefined);
});

test("upload an avatar", async () => {
  assert.equal((await dispatch("PUT", "/users/u1/avatar", { body: { bytes: 2048 } })).body.avatarBytes, 2048);
  assert.equal((await dispatch("PUT", "/users/u1/avatar", { body: { bytes: 2_000_000 } })).status, 413);
});
