import { changedFiles, exists, expect, expectEqual, finish, read } from "../../lib/check.mjs";

expectEqual(changedFiles(), ["ANSWER.json"], "only ANSWER.json may be added");

// kebab is imported only by legacy-sync.js, which nothing imports; both
// readings (strict imports vs reachability) are accepted for that one entry.
const strict = [
  "src/features/legacy-sync.js#legacySync",
  "src/features/report.js#exportCsv",
  "src/features/sync.js#dryRun",
  "src/lib/args.js#helpText",
  "src/lib/args.js#parseFlags",
  "src/lib/numbers.js#median",
  "src/lib/numbers.js#sum",
  "src/lib/strings.js#shout",
];
const reachable = [...strict, "src/lib/strings.js#kebab"].sort();

if (expect(exists("ANSWER.json"), "ANSWER.json missing")) {
  let answer = null;
  try {
    answer = JSON.parse(read("ANSWER.json"));
  } catch (error) {
    expect(false, `ANSWER.json is not valid JSON: ${error.message}`);
  }
  const unused = Array.isArray(answer?.unused)
    ? answer.unused.map((entry) => String(entry).replace(/^\.\//, ""))
    : null;
  expect(unused !== null, "ANSWER.json must contain an `unused` array");
  if (unused) {
    const sorted = JSON.stringify(unused) === JSON.stringify([...unused].sort());
    expect(sorted, "`unused` must be sorted alphabetically");
    const got = JSON.stringify([...unused].sort());
    expect(
      got === JSON.stringify(strict) || got === JSON.stringify(reachable),
      `unused exports: expected ${JSON.stringify(strict)}, got ${JSON.stringify(unused)}`,
    );
  }
}

finish();
