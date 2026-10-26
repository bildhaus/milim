import { rmSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  attempt,
  expect,
  expectEqual,
  expectOnlyChanged,
  finish,
  headFile,
  load,
  materializeTurn,
  read,
  repo,
  requireTrajectory,
  runNodeTests,
  turnAnswer,
  turnFile,
} from "../../lib/check.mjs";

// Turn 1 converts the a-m handlers and stops; turn 2 is only "continue", so
// the agent must pick up the n-z handlers from what turn 1 did and said.
const FIRST = ["archiveProject", "createUser", "deleteUser", "exportReport", "getUser", "listProjects"];
const SECOND = ["renameProject", "resetPassword", "searchUsers", "shareProject", "updateUser", "uploadAvatar"];
const file = (name) => `src/handlers/${name}.js`;

for (const path of ["test/router.test.js", "src/db.js", "src/errors.js", "src/seed.js", "package.json"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged((path) => path === "src/router.js" || /^src\/handlers\/[\w-]+\.js$/.test(path));

/** Handler names in `root` that are async functions of `req` only. */
async function converted(root, names) {
  const out = [];
  for (const name of names) {
    const handler = await attempt(`load ${file(name)}`, async () => {
      const url = pathToFileURL(join(root, file(name)));
      url.searchParams.set("v", `${Date.now()}-${Math.random()}`);
      return (await import(url.href))[name];
    });
    if (typeof handler === "function" && handler.constructor.name === "AsyncFunction" && handler.length === 1) out.push(name);
  }
  return out;
}

if (requireTrajectory()) {
  // Turn 1: exactly the a-m handlers converted, the rest untouched, tests green.
  const turnOne = materializeTurn(1);
  if (expect(turnOne, "turn 1 left no snapshot")) {
    try {
      const done = await converted(turnOne, FIRST);
      const missed = FIRST.filter((name) => !done.includes(name));
      expect(missed.length === 0, `turn 1 did not convert: ${missed.join(", ")}`);
      const early = SECOND.filter((name) => turnFile(1, file(name)) !== headFile(file(name)));
      expect(early.length === 0, `turn 1 changed handlers it was told to leave for later: ${early.join(", ")}`);
      runNodeTests("test", undefined, { root: turnOne, label: "turn 1: tests with both handler styles" });
    } finally {
      rmSync(turnOne, { recursive: true, force: true });
    }
  }
  const answer = turnAnswer(1);
  const unlisted = SECOND.filter((name) => !answer.includes(name));
  expect(unlisted.length === 0, `turn 1's answer does not list the remaining handlers: ${unlisted.join(", ")}`);

  // Turn 2 ("continue"): everything converted, tests green, behavior intact.
  const all = await converted(repo, [...FIRST, ...SECOND]);
  const left = [...FIRST, ...SECOND].filter((name) => !all.includes(name));
  expect(left.length === 0, `not converted at the end: ${left.join(", ")}`);
  runNodeTests("test");
  await attempt("hidden route cases", async () => {
    const { db } = await import(pathToFileURL(join(repo, "src/db.js")).href);
    const { dispatch } = await load("src/router.js");
    db.reset();
    expectEqual((await dispatch("POST", "/projects/nope/shares", { body: { userId: "u2" } })).status, 404, "share a missing project");
    expectEqual((await dispatch("POST", "/projects/p1/shares", { body: { userId: "zz" } })).status, 400, "share with an unknown user");
    expectEqual((await dispatch("POST", "/projects/p2/shares", { body: { userId: "u3" } })).status, 403, "share someone else's project");
    expectEqual((await dispatch("PATCH", "/projects/p2", { body: { name: "X" } })).status, 403, "rename someone else's project");
    expectEqual((await dispatch("PATCH", "/users/u1", { body: { email: "nope" } })).status, 400, "update with a bad email");
    expectEqual((await dispatch("PUT", "/users/zz/avatar", { body: { bytes: 10 } })).status, 404, "avatar for a missing user");
    expectEqual((await dispatch("PUT", "/users/u1/avatar", { body: { bytes: 0 } })).status, 400, "empty avatar");
    expectEqual((await dispatch("GET", "/users?q=  GR ")).body, ["u2"], "search trims and ignores case");
    expectEqual((await dispatch("POST", "/projects/p1/archive")).body.archived, true, "archive");
    expectEqual((await dispatch("GET", "/reports/projects.csv")).body, "id,name\np3,Analytical notes", "export skips archived");
  });
}

finish();
