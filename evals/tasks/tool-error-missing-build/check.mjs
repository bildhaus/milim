import { spawnSync } from "node:child_process";
import { attempt, exists, expect, expectEqual, expectOnlyChanged, finish, load, read, repo, runNodeTests } from "../../lib/check.mjs";

// src/generated/ is ignored by git, so the table never shows up as a change;
// the tests, the rate data, and package.json must stay as they are.
expectOnlyChanged(["scripts/build-rates.mjs", "src/convert.js"], "changed outside the build script and converter");
// `npm test` fails until the table is built, so a run that followed the
// prompt leaves the generated file behind.
expect(exists("src/generated/rates.js"), "src/generated/rates.js is missing: the build was never run");
expect(/from\s*["']\.\/generated\/rates\.js["']/.test(read("src/convert.js")), "src/convert.js must keep reading the generated table");

// Rebuild from the (fixed) script, so a hand-edited generated file cannot pass.
const build = spawnSync(
  process.execPath,
  ["scripts/build-rates.mjs", "--input", "data/rates.csv", "--out", "src/generated/rates.js"],
  { cwd: repo, encoding: "utf8", timeout: 30_000 },
);
if (expect(build.status === 0, `the build failed: ${`${build.stdout}${build.stderr}`.trim().split("\n").at(-1)}`)) {
  runNodeTests("test");
  await attempt("hidden conversions", async () => {
    const { convert, format } = await load("src/convert.js");
    expectEqual(convert(1_000_000, "KRW", "USD"), 73_964, "KRW (zero decimals) to USD");
    expectEqual(convert(10_000, "USD", "KRW"), 135_200, "USD to KRW");
    expectEqual(format(135_200, "KRW"), "135200 KRW", "KRW format");
    expectEqual(convert(1_000, "USD", "BHD"), 3_760, "USD to BHD (three decimals)");
    expectEqual(format(3_760, "BHD"), "3.760 BHD", "BHD format");
    expectEqual(convert(500, "JPY", "JPY"), 500, "JPY to itself");
  });
}

finish();
