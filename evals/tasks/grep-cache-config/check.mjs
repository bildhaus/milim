import { attempt, expectEqual, expectOnlyChanged, finish, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged((path) => path === "src/lib/cache.js" || path.startsWith("test/"));
runNodeTests("test");

await attempt("catalog cache", async () => {
  const base = `file://${process.cwd()}/`;
  const { handle } = await import(new URL("src/index.js", base).href);
  const { lookupCount } = await import(new URL("src/handlers/catalog.js", base).href);
  const { advance } = await import(new URL("src/lib/clock.js", base).href);
  const first = handle({ method: "GET", path: "/catalog", query: { region: "eu" } });
  expectEqual(first.status, 200, "catalog status");
  handle({ method: "GET", path: "/catalog", query: { region: "eu" } });
  expectEqual(lookupCount(), 1, "second request should hit the cache");
  advance(59_000);
  handle({ method: "GET", path: "/catalog", query: { region: "eu" } });
  expectEqual(lookupCount(), 1, "still cached before 60s");
  advance(1_000);
  handle({ method: "GET", path: "/catalog", query: { region: "eu" } });
  expectEqual(lookupCount(), 2, "expires after 60s");
});

finish();
