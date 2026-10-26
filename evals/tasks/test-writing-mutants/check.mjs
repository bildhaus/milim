import { spawnSync } from "node:child_process";
import { readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { exists, expect, expectOnlyChanged, finish, headFile, read, repo, runNodeTests } from "../../lib/check.mjs";

// The agent writes the tests. They must pass against the real module and
// fail against every hidden mutant, each a plausible bug that breaks one
// documented behavior.
const SOURCE = "src/intervals.js";
const MUTANTS = [
  ["touching intervals stay separate", "if (last && start <= last[1])", "if (last && start < last[1])"],
  ["input assumed sorted", "\n    .sort((a, b) => a[0] - b[0]);", ";"],
  ["caller's array sorted in place", "const sorted = intervals\n", "const sorted = intervals.sort((a, b) => a[0] - b[0])\n"],
  ["contains() includes the end", "start <= point && point < end", "start <= point && point <= end"],
  ["nested interval shrinks the merge", "last[1] = Math.max(last[1], end);", "last[1] = end;"],
  ["reversed interval silently dropped", '  if (start > end) throw new RangeError(`interval ${index} ends before it starts`);\n', ""],
  ["empty intervals kept", ".filter(([start, end]) => start < end)", ".filter(() => true)"],
  ["NaN and Infinity accepted", "!Number.isFinite(start) || !Number.isFinite(end)", 'typeof start !== "number" || typeof end !== "number"'],
  ["totalLength double-counts overlaps", "return mergeIntervals(intervals).reduce(", "return intervals.reduce("],
  ["gaps() reports a gap past `to`", "  if (cursor < to) out.push([cursor, to]);", "  out.push([cursor, to]);"],
];

expect(read(SOURCE) === headFile(SOURCE), `${SOURCE} was modified`);
expectOnlyChanged((path) => path.startsWith("test/"), "changed outside test/");

const tests = exists("test")
  ? readdirSync(join(repo, "test"), { recursive: true })
      .map(String)
      .filter((name) => /\.(test|spec)\.m?js$/.test(name))
      .map((name) => `test/${name.split("\\").join("/")}`)
  : [];
if (expect(tests.length > 0, "no test files were written under test/")) {
  if (runNodeTests("test", undefined, { label: "the new tests against the real src/intervals.js" })) {
    const original = headFile(SOURCE);
    const survivors = [];
    try {
      for (const [name, from, to] of MUTANTS) {
        if (!original.includes(from)) throw new Error(`mutant "${name}" no longer applies to ${SOURCE}`);
        writeFileSync(join(repo, SOURCE), original.replace(from, () => to));
        const result = spawnSync(process.execPath, ["--test", ...tests], { cwd: repo, encoding: "utf8", timeout: 30_000 });
        if (result.status === 0) survivors.push(name);
      }
    } finally {
      writeFileSync(join(repo, SOURCE), original);
    }
    expect(
      survivors.length === 0,
      `the tests miss ${survivors.length} of ${MUTANTS.length} seeded bugs: ${survivors.join("; ")}`,
    );
  }
}

finish();
