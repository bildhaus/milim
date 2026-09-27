import { attempt, changedLineCounts, expect, expectEqual, expectOnlyChanged, finish, load, runNodeTests } from "../../lib/check.mjs";

// The fix may live wherever it reads best (usually src/handlers/usage.js,
// possibly a shared helper in src/lib/), but it must stay small.
expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/"));
const sourceChanges = Object.entries(changedLineCounts()).filter(([path]) => path.startsWith("src/"));
const changedLines = sourceChanges.reduce((sum, [, count]) => sum + count, 0);
expect(sourceChanges.length <= 3 && changedLines <= 40, `change is not minimal: ${changedLines} line(s) in ${sourceChanges.length} source file(s)`);
runNodeTests("test");

await attempt("quota response", async () => {
  const { handle } = await load("src/index.js");
  const { db } = await import(new URL("src/lib/db.js", `file://${process.cwd()}/`).href);
  db.reset();
  db.upsert("plans", "acme", { limit: 5 });
  expectEqual(handle({ method: "POST", path: "/usage/acme", body: { units: 3 } }).status, 200, "under quota");
  const over = handle({ method: "POST", path: "/usage/acme", body: { units: 3 } });
  expectEqual(over.status, 429, "over quota status");
  expectEqual(over.body?.error?.code, "QUOTA_EXCEEDED", "over quota code");
  expectEqual(over.body?.error?.message, "usage quota exceeded", "over quota message");
  expectEqual(over.body?.error?.details, { limit: 5, used: 6 }, "over quota details");
  expectEqual(handle({ method: "GET", path: "/users/missing" }).status, 404, "other errors unchanged");
});

finish();
