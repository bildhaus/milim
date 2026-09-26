import { attempt, expectEqual, expectOnlyChanged, finish, load, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged((path) => path === "src/handlers/usage.js" || path.startsWith("test/"));
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
});

finish();
