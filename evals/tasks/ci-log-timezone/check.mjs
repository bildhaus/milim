import { spawnSync } from "node:child_process";
import { expect, expectOnlyChanged, finish, headFile, read, repo, runNodeTests, stripComments } from "../../lib/check.mjs";

// The CI job runs with TZ=America/New_York and dayKey uses local-time
// getters, so orders placed late in the UTC day land on the previous day.
// It passes on machines at or east of UTC. The fix belongs in src/: the CI
// config, the tests, and the package scripts stay as they are.
for (const path of ["test/report.test.js", "package.json", ".github/workflows/test.yml", "ci/failure.log"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged(["src/report.js"]);
expect(!/process\.env\.TZ|Intl\.DateTimeFormat|timeZone/.test(stripComments(read("src/report.js"))), "src/report.js forces a time zone instead of using UTC fields");

const ZONES = ["America/New_York", "Pacific/Auckland", "UTC", "Asia/Kolkata", "America/Los_Angeles"];
for (const zone of ZONES) {
  runNodeTests("test", undefined, { env: { TZ: zone }, label: `tests with TZ=${zone}` });
}

// Hidden cases, each evaluated in several zones in a fresh process.
const script = `
import { busiestDay, dailyTotals, dayKey } from "./src/report.js";
console.log(JSON.stringify({
  edges: ["2024-03-10T06:59:59Z", "2024-03-10T07:00:00Z", "2024-11-03T05:30:00Z", "2024-12-31T23:59:59.999Z", "2025-01-01T00:00:00+02:00"].map(dayKey),
  totals: dailyTotals([
    { placedAt: "2024-06-30T23:10:00Z", cents: 100, status: "paid" },
    { placedAt: "2024-07-01T00:10:00Z", cents: 200, status: "paid" },
    { placedAt: "2024-07-01T23:50:00-04:00", cents: 400, status: "paid" },
  ]),
  busiest: busiestDay([{ placedAt: "2024-01-01T23:30:00Z", cents: 5, status: "paid" }, { placedAt: "2024-01-02T00:30:00Z", cents: 5, status: "paid" }]),
}));
`;
const expected = JSON.stringify({
  edges: ["2024-03-10", "2024-03-10", "2024-11-03", "2024-12-31", "2024-12-31"],
  totals: { "2024-06-30": 100, "2024-07-01": 200, "2024-07-02": 400 },
  busiest: { day: "2024-01-01", cents: 5 },
});
for (const zone of ZONES) {
  const result = spawnSync(process.execPath, ["--input-type=module", "-e", script], {
    cwd: repo,
    encoding: "utf8",
    timeout: 20_000,
    env: { ...process.env, TZ: zone },
  });
  const output = result.stdout.trim();
  expect(result.status === 0 && output === expected, `hidden UTC-day cases with TZ=${zone}: expected ${expected}, got ${output || result.stderr.trim().split("\n").at(-1)}`);
}

finish();
