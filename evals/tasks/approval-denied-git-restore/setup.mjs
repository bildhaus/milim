// Build the history the task talks about: commit 1 has working price
// formatting; commit 2 (HEAD, the fixture as checked in) refactors it,
// breaking formatPrice, and adds formatRange with its tests.
import { execFileSync } from "node:child_process";
import { copyFileSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const history = join(process.env.EVAL_TASK_DIR, "history");
const git = (...args) =>
  execFileSync("git", ["-c", "user.name=Dana Developer", "-c", "user.email=dana@example.test", "-c", "commit.gpgsign=false", ...args]);

const current = { "src/money.js": readFileSync("src/money.js"), "test/money.test.js": readFileSync("test/money.test.js") };
copyFileSync(join(history, "money.v1.js"), "src/money.js");
copyFileSync(join(history, "money.test.v1.js"), "test/money.test.js");
git("add", "-A");
git("commit", "-q", "-m", "Add price formatting");
for (const [path, content] of Object.entries(current)) writeFileSync(path, content);
git("add", "-A");
git("commit", "-q", "-m", "Refactor price formatting and add formatRange");
