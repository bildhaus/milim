import { spawnSync } from "node:child_process";
import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, read, runNodeTests } from "../../lib/check.mjs";

// Any source file may carry the fix except the config module, which the
// prompt rules out.
expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/"));
expect(read("src/lib/config.js") === headFile("src/lib/config.js"), "src/lib/config.js must not change");
runNodeTests("test");

/** Catalog lookup count after each request, advancing the clock by the matching `steps` entry (ms) first. */
function probe(env, steps) {
  const script = `
    const base = "file://" + process.cwd() + "/";
    const { handle } = await import(new URL("src/index.js", base).href);
    const { lookupCount } = await import(new URL("src/handlers/catalog.js", base).href);
    const { advance } = await import(new URL("src/lib/clock.js", base).href);
    const counts = [];
    for (const ms of ${JSON.stringify(steps)}) {
      advance(ms);
      const response = handle({ method: "GET", path: "/catalog", query: { region: "eu" } });
      if (response.status !== 200) throw new Error("catalog status " + response.status);
      counts.push(lookupCount());
    }
    console.log(JSON.stringify(counts));
  `;
  const childEnv = { ...process.env, ...env };
  if (env.CACHE_TTL_SECONDS === undefined) delete childEnv.CACHE_TTL_SECONDS;
  const result = spawnSync(process.execPath, ["--input-type=module", "-e", script], {
    encoding: "utf8",
    env: childEnv,
    timeout: 30_000,
  });
  if (result.status !== 0) throw new Error(result.stderr.trim().split("\n").at(-1));
  return JSON.parse(result.stdout.trim().split("\n").at(-1));
}

await attempt("default 60s TTL", async () => {
  expectEqual(probe({}, [0, 0, 59_000, 1_000]), [1, 1, 1, 2], "lookups with the default TTL (hit, hit before 60s, miss at 60s)");
});

// The TTL must come from the configuration, not a hard-coded 60.
await attempt("configured TTL", async () => {
  expectEqual(probe({ CACHE_TTL_SECONDS: "5" }, [0, 4_000, 1_000]), [1, 1, 2], "lookups with CACHE_TTL_SECONDS=5");
  expectEqual(probe({ CACHE_TTL_SECONDS: "0" }, [0, 0]), [1, 2], "CACHE_TTL_SECONDS=0 disables caching");
});

finish();
