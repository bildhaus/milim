#!/usr/bin/env node
// Grader validation without a model. For every task, the check must FAIL on
// the untouched fixture and PASS once the reference solution is applied, each
// in a fresh temporary copy. Optional extra solutions next to it:
//   solution.patch | solution.mjs   reference solution (required)
//   alt-<name>.patch | .mjs         other correct solutions; must PASS
//   wrong-<name>.patch | .mjs       plausible wrong solutions; must FAIL
// A .patch is applied with `git apply`; a .mjs runs with the repository as
// its working directory (EVAL_TASK_DIR points at the task directory).
//
//   node evals/validate.mjs [--tasks a,b] [--repeat N] [--strict] [--verbose]

import { spawnSync } from "node:child_process";
import { readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { git, loadTasks, missingRequirement, prepareRepo, runCheck, scrubbedEnv } from "./lib/tasks.mjs";

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

if (flag("--help") || flag("-h")) {
  console.log(`usage: node evals/validate.mjs [--tasks a,b] [--repeat N] [--strict] [--verbose]

  --tasks    comma list of task ids or substrings (default: all)
  --repeat   run every passing check N times to catch flaky graders (default 1)
  --strict   treat a task skipped for a missing requirement (python3) as a failure
  --verbose  print check output for every step`);
  process.exit(0);
}

const SOLUTION_FILE = /^(solution|alt-[\w-]+|wrong-[\w-]+)\.(patch|mjs)$/;
const repeat = Number(option("--repeat") ?? 1);
const strict = flag("--strict");
const verbose = flag("--verbose");

function applySolution(task, repo, file) {
  const path = join(task.dir, file);
  if (file.endsWith(".patch")) {
    git(repo, ["apply", "--whitespace=nowarn", path]);
    return;
  }
  const result = spawnSync(process.execPath, [path], {
    cwd: repo,
    encoding: "utf8",
    timeout: 120_000,
    env: scrubbedEnv({ EVAL_REPO: repo, EVAL_TASK_DIR: task.dir }),
  });
  if (result.status !== 0) {
    throw new Error(`${file} exited ${result.status}: ${`${result.stdout}${result.stderr}`.trim().slice(0, 400)}`);
  }
}

const indent = (text) => text.split("\n").map((line) => `      ${line}`).join("\n");

/** Apply `file` (or nothing) to a fresh copy and run the check `times` times. */
function trial(task, file, times) {
  const repo = prepareRepo(task);
  try {
    if (file) applySolution(task, repo, file);
    const checks = [];
    for (let index = 0; index < times; index += 1) checks.push(runCheck(task, repo));
    return { checks };
  } catch (error) {
    return { error: error.message, checks: [] };
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
}

function validate(task) {
  const problems = [];
  const notes = [];
  const files = readdirSync(task.dir).filter((name) => SOLUTION_FILE.test(name)).sort();
  const references = files.filter((name) => name.startsWith("solution."));
  if (references.length !== 1) problems.push(`needs exactly one solution.patch or solution.mjs (found ${references.length})`);

  const baseline = trial(task, null, 1);
  const [check] = baseline.checks;
  if (baseline.error) problems.push(`baseline: ${baseline.error}`);
  else if (check.skipped) problems.push("baseline: check skipped although requirements are met");
  else if (check.passed) problems.push("check PASSES on the untouched fixture");
  if (verbose && check) notes.push(`baseline (expected FAIL):\n${indent(check.output)}`);

  for (const file of files) {
    const shouldPass = !file.startsWith("wrong-");
    const result = trial(task, file, shouldPass ? repeat : 1);
    if (result.error) {
      problems.push(`${file}: could not apply: ${result.error}`);
      continue;
    }
    for (const [index, run] of result.checks.entries()) {
      const label = result.checks.length > 1 ? `${file} (run ${index + 1})` : file;
      if (run.skipped) problems.push(`${label}: check skipped`);
      else if (shouldPass && !run.passed) problems.push(`${label}: check FAILS but should pass\n${indent(run.output)}`);
      else if (!shouldPass && run.passed) problems.push(`${label}: check PASSES but should fail`);
      if (verbose) notes.push(`${label} (expected ${shouldPass ? "PASS" : "FAIL"}):\n${indent(run.output)}`);
    }
  }
  return { problems, notes, files };
}

const tasks = loadTasks(option("--tasks"));
let failed = 0;
let skipped = 0;
const started = Date.now();
for (const task of tasks) {
  const missing = missingRequirement(task);
  if (missing) {
    skipped += 1;
    if (strict) failed += 1;
    console.log(`${strict ? "FAIL" : "SKIP"}  ${task.id}: ${missing}`);
    continue;
  }
  const taskStarted = Date.now();
  const { problems, notes, files } = validate(task);
  const extras = files.filter((name) => !name.startsWith("solution.")).length;
  const took = `${((Date.now() - taskStarted) / 1000).toFixed(1)}s`;
  if (problems.length === 0) {
    console.log(`ok    ${task.id} (${files.length - extras} solution${extras ? ` + ${extras} alt/wrong` : ""}, ${took})`);
  } else {
    failed += 1;
    console.log(`FAIL  ${task.id} (${took})`);
    for (const problem of problems) console.log(`    - ${problem}`);
  }
  for (const note of notes) console.log(`    ${note}`);
}
console.log(
  `\n${tasks.length - failed - (strict ? 0 : skipped)}/${tasks.length} task(s) validated${skipped ? `, ${skipped} skipped` : ""} in ${((Date.now() - started) / 1000).toFixed(1)}s.`,
);
process.exit(failed > 0 ? 1 : 0);
