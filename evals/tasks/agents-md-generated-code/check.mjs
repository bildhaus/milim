import { spawnSync } from "node:child_process";
import { mkdtempSync, cpSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { attempt, exists, expect, expectEqual, expectOnlyChanged, finish, load, read, repo, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged(
  (path) =>
    ["routes.json", "src/routes.generated.js", "src/handlers/health.js"].includes(path) || path.startsWith("test/"),
  "changed outside the documented layout",
);

const routes = JSON.parse(read("routes.json"));
expect(
  routes.some((route) => route.method === "GET" && route.path === "/health" && route.handler === "health"),
  "routes.json lacks { GET /health -> health }",
);
const sorted = [...routes].sort((a, b) => a.path.localeCompare(b.path) || a.method.localeCompare(b.method));
expectEqual(routes, sorted, "routes.json sorted by path then method");

// The generated file must be exactly what the generator produces.
await attempt("generated file", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "milim-eval-gen-"));
  try {
    cpSync(repo, scratch, { recursive: true, filter: (source) => !source.includes(`${join(repo, ".git")}`) });
    const result = spawnSync(process.execPath, ["scripts/gen-routes.mjs"], { cwd: scratch, encoding: "utf8" });
    expect(result.status === 0, `generator failed: ${result.stderr.trim()}`);
    expect(
      readFileSync(join(scratch, "src/routes.generated.js"), "utf8") === read("src/routes.generated.js"),
      "src/routes.generated.js does not match the generator output",
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

if (expect(exists("src/handlers/health.js"), "src/handlers/health.js missing")) {
  const handler = read("src/handlers/health.js");
  expect(/export function handleHealth\b/.test(handler), "handler must export handleHealth");
  expect(!/package\.json/.test(handler), "handler must read the version through src/meta.js");
}
runNodeTests("test");

await attempt("GET /health", async () => {
  const { dispatch } = await load("src/server.js");
  expectEqual(dispatch({ method: "GET", path: "/health" }), { status: 200, body: { ok: true, version: "3.4.1" } }, "health response");
});

finish();
