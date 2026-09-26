import { attempt, expect, expectEqual, expectOnlyChanged, finish, load, runNodeTests } from "../../lib/check.mjs";

function throwsType(fn, Type) {
  try {
    fn();
  } catch (error) {
    return error instanceof Type;
  }
  return false;
}

expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/"));
runNodeTests("test");

await attempt("new signature", async () => {
  const users = await load("src/users.js");
  users.resetIds();
  expectEqual(users.createUser({ name: " Ann ", email: "ANN@x.io" }), { id: 1, name: "Ann", email: "ann@x.io", role: "member" }, "default member");
  expectEqual(users.createUser({ name: "Root", email: "r@x.io", role: "admin" }).role, "admin", "admin role");
  expect(throwsType(() => users.createUser("Ann", "ann@x.io"), TypeError), "positional call must throw TypeError");
  expect(throwsType(() => users.createUser(null), TypeError), "null must throw TypeError");
  expect(throwsType(() => users.createUser({ name: "A", email: "a@x.io", role: "owner" }), RangeError), "unknown role must throw RangeError");
});

await attempt("callers", async () => {
  const { signup } = await load("src/signup.js");
  const { promoteInvite, isPrivileged } = await load("src/admin.js");
  const { seedUsers } = await load("src/seed.js");
  const member = signup({ name: "Bo", email: "bo@x.io" });
  expectEqual(member.user.role, "member", "signup role");
  expectEqual(member.welcome, "Welcome, Bo!", "signup welcome");
  expectEqual(isPrivileged(member.user), false, "member not privileged");
  const invite = promoteInvite({ name: "Cy", email: "cy@x.io" });
  expectEqual(invite.user.role, "admin", "invite role");
  expectEqual(invite.permissions, ["read", "write", "admin"], "invite permissions");
  expectEqual(isPrivileged(invite.user), true, "admin privileged");
  const seeded = seedUsers();
  expectEqual(seeded.map((user) => user.role), ["admin", "member", "member"], "seed roles");
  expect(seeded.every((user) => !("isAdmin" in user)), "users must not keep isAdmin");
});

finish();
