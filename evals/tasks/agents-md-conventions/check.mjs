import { attempt, exists, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

expectOnlyChanged(["src/title-case.js", "src/index.js", "tests/title-case.test.js", "CHANGELOG.md"], "changed outside the AGENTS.md layout");
expect(read("src/legacy.js") === headFile("src/legacy.js"), "src/legacy.js was edited");

if (expect(exists("src/title-case.js"), "rule 1: src/title-case.js missing")) {
  const source = read("src/title-case.js");
  // The JSDoc block must sit directly above the export, written either as a
  // function declaration or as a const-bound function.
  const doc =
    /\/\*\*((?:(?!\*\/)[\s\S])*)\*\/\s*export\s+(?:(?:async\s+)?function\s+titleCase\s*\(|const\s+titleCase\s*=)/.exec(source)?.[1] ?? "";
  expect(doc !== "", "rule 2: titleCase has no JSDoc block directly above it");
  expect(/@param\s/.test(doc), "rule 2: JSDoc missing @param");
  expect(/@returns?\s/.test(doc), "rule 2: JSDoc missing @returns");
  expect(/@example/.test(doc), "rule 2: JSDoc missing @example");
}

const exportsList = read("src/index.js")
  .split("\n")
  .map((line) => /export\s*\{\s*(\w+)\s*\}/.exec(line)?.[1])
  .filter(Boolean);
expectEqual(exportsList, ["reverseWords", "titleCase", "truncate"], "rule 1: index.js exports in alphabetical order");

expect(exists("tests/title-case.test.js"), "rule 4: tests/title-case.test.js missing");
if (exists("tests/title-case.test.js")) {
  const test = read("tests/title-case.test.js");
  expect(/node:test/.test(test) && /node:assert\/strict/.test(test), "rule 4: test must use node:test and node:assert/strict");
}
runNodeTests("tests");

const changelog = read("CHANGELOG.md");
const unreleased = changelog.split("## 1.2.0")[0].split("## Unreleased")[1] ?? "";
expect(/^- Added `titleCase`: \S/m.test(unreleased), "rule 5: CHANGELOG Unreleased entry missing or misformatted");
expectEqual(changelog.slice(changelog.indexOf("## 1.2.0")), headFile("CHANGELOG.md").slice(headFile("CHANGELOG.md").indexOf("## 1.2.0")), "rule 5: released section edited");

await attempt("titleCase", async () => {
  const { titleCase } = await load("src/index.js");
  expectEqual(titleCase("hello WORLD-wide"), "Hello World-Wide", "example");
  expectEqual(titleCase("  two  spaces "), "  Two  Spaces ", "keeps spacing");
  expectEqual(titleCase(""), "", "empty");
  let threw = false;
  try {
    titleCase(7);
  } catch (error) {
    threw = error instanceof TypeError;
  }
  expect(threw, "rule 3: non-string input must throw TypeError");
});

finish();
