// Shared helpers for task checks. A check runs with its working directory set
// to the task's temporary repository copy, which the runner committed before
// the agent started, so `HEAD` is always the untouched fixture.

import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { changedPaths, SKIP_EXIT_CODE } from "./tasks.mjs";
import { readTrajectory } from "./trajectory.mjs";

export { findPython, findTsc, hasCargo } from "./tasks.mjs";
export { matchingRule, ruleSubject } from "./trajectory.mjs";

export const repo = process.cwd();

const failures = [];

export function fail(message) {
  failures.push(message);
}

export function expect(condition, message) {
  if (!condition) fail(message);
  return Boolean(condition);
}

export function expectEqual(actual, expected, label) {
  const same = JSON.stringify(actual) === JSON.stringify(expected);
  if (!same) {
    fail(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
  return same;
}

/** Run `fn`, recording a thrown error as a failure instead of crashing. */
export async function attempt(label, fn) {
  try {
    return await fn();
  } catch (error) {
    fail(`${label}: ${error?.message ?? error}`);
    return undefined;
  }
}

/** Exit 0 when every expectation held, otherwise print reasons and exit 1. */
export function finish() {
  if (failures.length === 0) {
    console.log("PASS");
    process.exit(0);
  }
  for (const message of failures.slice(0, 20)) console.log(`FAIL ${message}`);
  if (failures.length > 20) console.log(`FAIL ... ${failures.length - 20} more`);
  process.exit(1);
}

/**
 * Stop without a verdict because the machine lacks something the task needs
 * (for example python3). The runner records the task as skipped.
 */
export function skip(message) {
  console.log(`SKIP ${message}`);
  process.exit(SKIP_EXIT_CODE);
}

export function read(path) {
  return readFileSync(join(repo, path), "utf8");
}

export function exists(path) {
  return existsSync(join(repo, path));
}

function git(args) {
  const result = spawnSync("git", args, { cwd: repo, encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  }
  return result.stdout;
}

/** The committed fixture version of a file, or null when it did not exist. */
export function headFile(path) {
  const result = spawnSync("git", ["show", `HEAD:${path}`], { cwd: repo, encoding: "utf8" });
  return result.status === 0 ? result.stdout : null;
}

/** Tracked and untracked paths that differ from the committed fixture. */
export function changedFiles() {
  return changedPaths(repo);
}

// ----- trajectory (turn answers, per-turn snapshots, tool calls, approvals) -----

let cachedTrajectory;

/**
 * The sample's trajectory (see lib/trajectory.mjs), or null when the check
 * runs without one. run.mjs always provides it; under validate.mjs only a
 * solution script that calls `recordTurn` does.
 */
export function trajectory() {
  if (cachedTrajectory === undefined) cachedTrajectory = readTrajectory(process.env.EVAL_TRAJECTORY);
  return cachedTrajectory;
}

/** The trajectory, or a recorded failure and null when there is none. */
export function requireTrajectory(label = "trajectory") {
  const value = trajectory();
  if (!value) fail(`${label}: no trajectory was recorded (EVAL_TRAJECTORY)`);
  return value;
}

/** The final assistant answer of turn `number` (1-based), or "" when missing. */
export function turnAnswer(number) {
  return trajectory()?.turns?.[number - 1]?.answer ?? "";
}

/** A file as turn `number` left it, or null when it did not exist then. */
export function turnFile(number, path) {
  const tree = trajectory()?.turns?.[number - 1]?.snapshot;
  if (!tree) return null;
  const result = spawnSync("git", ["show", `${tree}:${path}`], { cwd: repo, encoding: "utf8" });
  return result.status === 0 ? result.stdout : null;
}

/**
 * Extract the workspace as turn `number` left it into a fresh temporary
 * directory (git-ignored files are not included) and return its path, or
 * null without a snapshot. The caller removes it with `rmSync`.
 */
export function materializeTurn(number) {
  const tree = trajectory()?.turns?.[number - 1]?.snapshot;
  if (!tree) return null;
  const archive = spawnSync("git", ["archive", "--format=tar", tree], { cwd: repo, maxBuffer: 256 * 1024 * 1024 });
  if (archive.status !== 0) throw new Error(`git archive failed: ${String(archive.stderr).trim()}`);
  const dir = mkdtempSync(join(tmpdir(), `milim-eval-turn-${number}-`));
  const extract = spawnSync("tar", ["-x", "-C", dir], { input: archive.stdout });
  if (extract.status !== 0) throw new Error(`tar -x failed: ${String(extract.stderr).trim()}`);
  return dir;
}

/** Paths turn `number` left different from the committed fixture. */
export function turnChangedFiles(number) {
  const tree = trajectory()?.turns?.[number - 1]?.snapshot;
  if (!tree) return null;
  return git(["diff", "--name-only", "HEAD", tree]).split("\n").filter(Boolean);
}

/**
 * Lines added plus lines removed relative to the committed fixture, per
 * changed path (untracked files count every line as added).
 */
export function changedLineCounts() {
  const counts = {};
  for (const line of git(["diff", "--numstat", "HEAD"]).split("\n").filter(Boolean)) {
    const [added, removed, path] = line.split("\t");
    counts[path] = (Number(added) || 0) + (Number(removed) || 0);
  }
  for (const path of changedFiles()) {
    if (counts[path] === undefined && exists(path)) counts[path] = lines(read(path)).length;
  }
  return counts;
}

/** Fail for every changed path the predicate does not allow. */
export function expectOnlyChanged(allowed, label = "unexpected change") {
  const allow = typeof allowed === "function" ? allowed : (path) => allowed.includes(path);
  const extra = changedFiles().filter((path) => !allow(path));
  expect(extra.length === 0, `${label}: ${extra.join(", ")}`);
}

/** Import a repository module fresh, bypassing the ESM cache. */
export async function load(path) {
  const url = pathToFileURL(join(repo, path));
  url.searchParams.set("v", `${Date.now()}-${Math.random()}`);
  return import(url.href);
}

/**
 * Import a self-contained module from source text, for example a file as an
 * earlier turn left it (`turnFile`). It is written to a temporary directory,
 * so it must not import relative paths.
 */
export async function loadSource(source, name = "module.mjs") {
  const dir = mkdtempSync(join(tmpdir(), "milim-eval-source-"));
  try {
    const path = join(dir, name.replace(/\.js$/, ".mjs"));
    writeFileSync(path, source);
    return await import(pathToFileURL(path).href);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function listFiles(dir, pattern, base = repo) {
  const root = join(base, dir);
  if (!existsSync(root)) return [];
  const out = [];
  for (const name of readdirSync(root)) {
    const full = join(root, name);
    if (name === "node_modules") continue;
    if (statSync(full).isDirectory()) out.push(...listFiles(relative(base, full), pattern, base));
    else if (pattern.test(name)) out.push(relative(base, full).split(sep).join("/"));
  }
  return out.sort();
}

/**
 * Run the repository's own `node:test` files under `dir` and return whether
 * they passed. Files are expanded here so behavior does not depend on the
 * Node version's `--test` glob support.
 */
export function runNodeTests(dir = "test", pattern = /\.(test|spec)\.m?js$/, { root = repo, label = "repository tests", env } = {}) {
  const files = listFiles(dir, pattern, root);
  if (files.length === 0) {
    fail(`${label}: no test files found under ${dir}/`);
    return false;
  }
  const result = spawnSync(process.execPath, ["--test", ...files], {
    cwd: root,
    encoding: "utf8",
    timeout: 60_000,
    ...(env ? { env: { ...process.env, ...env } } : {}),
  });
  if (result.status !== 0) {
    const tail = `${result.stdout}\n${result.stderr}`
      .split("\n")
      .filter((line) => /not ok|Error|expected|actual/i.test(line))
      .slice(0, 6)
      .join(" | ");
    fail(`${label} failed: ${tail || `exit ${result.status}`}`);
    return false;
  }
  return true;
}

/** Source text with `//` line comments and block comments removed (strings are not parsed). */
export function stripComments(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:"'`\\])\/\/.*$/gm, "$1");
}

/** Lines of `text` split without a trailing empty entry. */
export function lines(text) {
  const parts = text.split("\n");
  if (parts.at(-1) === "") parts.pop();
  return parts;
}
