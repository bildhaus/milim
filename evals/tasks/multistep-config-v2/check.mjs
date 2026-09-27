import { attempt, changedFiles, exists, expect, expectEqual, expectOnlyChanged, finish, load, read, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged(
  (path) => path.startsWith("src/") || path.startsWith("test/") || ["docs/CONFIG.md", "config/example.conf", "config/example.json"].includes(path),
);

function throwsPortRange(fn) {
  try {
    fn();
  } catch (error) {
    return error instanceof RangeError && /port/i.test(error.message);
  }
  return false;
}

const V2 = `{
  "version": 2,
  "server": {
    "host": "0.0.0.0",
    "port": 9090
  },
  "features": [
    "metrics",
    "audit-log"
  ]
}
`;

await attempt("1-2. loadConfig", async () => {
  const { loadConfig } = await load("src/config.js");
  const expected = { host: "0.0.0.0", port: 9090, features: ["metrics", "audit-log"] };
  expectEqual(loadConfig("host = 0.0.0.0\nport = 9090\nfeatures = metrics, audit-log\n"), expected, "v1 still works");
  expectEqual(loadConfig(V2), expected, "v2 loads");
  expectEqual(loadConfig(`  \n${V2}`), expected, "v2 with leading whitespace");
  for (const port of ["0", "65536", "80.5", "http"]) {
    expect(throwsPortRange(() => loadConfig(`port = ${port}`)), `v1 port ${port} must throw RangeError mentioning port`);
  }
  expect(
    throwsPortRange(() => loadConfig('{"version":2,"server":{"host":"h","port":70000},"features":[]}')),
    "v2 port 70000 must throw RangeError mentioning port",
  );
});

await attempt("3. migrateV1ToV2", async () => {
  const { migrateV1ToV2 } = await load("src/migrate.js");
  expectEqual(migrateV1ToV2("host = 0.0.0.0\nport = 9090\nfeatures = metrics, audit-log\n"), V2, "migrated document");
  expectEqual(
    migrateV1ToV2("# nothing\n"),
    `${JSON.stringify({ version: 2, server: { host: "127.0.0.1", port: 8080 }, features: [] }, null, 2)}\n`,
    "defaults migrate",
  );
});

expect(!exists("config/example.conf"), "4. config/example.conf should be deleted");
if (expect(exists("config/example.json"), "4. config/example.json missing")) {
  let parsed = null;
  try {
    parsed = JSON.parse(read("config/example.json"));
  } catch {
    expect(false, "4. config/example.json is not valid JSON");
  }
  if (parsed) {
    expectEqual(parsed, JSON.parse(V2), "4. example.json content");
  }
}

const docs = read("docs/CONFIG.md");
expect(/"version"\s*:\s*2/.test(docs), "5. docs/CONFIG.md lacks a v2 example");
expect(/deprecat/i.test(docs), "5. docs/CONFIG.md does not mark v1 deprecated");

// Deleted test files are listed as changed too; only read the ones that exist.
const tests = changedFiles().filter((path) => path.startsWith("test/") && exists(path));
expect(tests.some((path) => /migrateV1ToV2/.test(read(path))), "6. no test covers migrateV1ToV2");
expect(tests.some((path) => /version/.test(read(path))), "6. no test covers v2 loading");
runNodeTests("test");

finish();
