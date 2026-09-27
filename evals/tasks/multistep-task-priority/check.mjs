import { spawnSync } from "node:child_process";
import { attempt, changedFiles, exists, expect, expectEqual, expectOnlyChanged, finish, load, read, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/") || path === "README.md");

function throwsRange(fn) {
  try {
    fn();
  } catch (error) {
    return error instanceof RangeError;
  }
  return false;
}

await attempt("1. store priority", async () => {
  const { addTask, createStore } = await load("src/store.js");
  const store = createStore();
  expectEqual(addTask(store, "a").priority, "normal", "default priority");
  expectEqual(addTask(store, "b", { priority: "high" }).priority, "high", "explicit priority");
  expect(throwsRange(() => addTask(store, "c", { priority: "urgent" })), "invalid priority throws RangeError");
  expectEqual(store.tasks.length, 2, "invalid task not stored");
});

await attempt("2. sortByPriority", async () => {
  const { sortByPriority } = await load("src/query.js");
  const tasks = [
    { id: 1, priority: "low" },
    { id: 2, priority: "high" },
    { id: 3, priority: "normal" },
    { id: 4, priority: "high" },
    { id: 5, priority: "low" },
  ];
  const copy = JSON.stringify(tasks);
  expectEqual(sortByPriority(tasks).map((task) => task.id), [2, 4, 3, 1, 5], "stable priority order");
  expectEqual(JSON.stringify(tasks), copy, "input not mutated");
});

await attempt("3. cli", async () => {
  const { run } = await load("src/cli.js");
  const { createStore } = await load("src/store.js");
  const store = createStore();
  run(store, ["add", "Pay", "rent", "--priority=high"]);
  run(store, ["add", "--priority=low", "Dust", "shelves"]);
  run(store, ["add", "Call", "mom"]);
  expectEqual(run(store, ["list"]), ["1. [!] Pay rent", "2. [-] Dust shelves", "3. [ ] Call mom"], "list output");
  expectEqual(run(store, ["list", "--sort=priority"]), ["1. [!] Pay rent", "3. [ ] Call mom", "2. [-] Dust shelves"], "sorted list output");
  run(store, ["done", "3"]);
  expectEqual(run(store, ["list", "--all", "--sort=priority"]).length, 3, "--all with --sort");
});

const readme = read("README.md");
const usage = readme.split("## Usage")[1]?.split("\n## ")[0] ?? "";
expect(/--priority/.test(usage), "4. README Usage does not document --priority");
expect(/--sort=priority/.test(usage), "4. README Usage does not document --sort=priority");

// Deleted test files are listed as changed too; only read the ones that exist.
const tests = changedFiles().filter((path) => path.startsWith("test/") && exists(path));
expect(tests.length > 0, "5. no tests were added or updated");
expect(tests.some((path) => /priority/.test(read(path))), "5. tests do not exercise priority");
runNodeTests("test");

finish();
