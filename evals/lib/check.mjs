// Shared helpers for task checks. A check runs with its working directory set
// to the task's temporary repository copy, which the runner committed before
// the agent started, so `HEAD` is always the untouched fixture.

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { pathToFileURL } from "node:url";

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
  return git(["status", "--porcelain", "--untracked-files=all"])
    .split("\n")
    .filter(Boolean)
    .map((line) => line.slice(3).replace(/^"|"$/g, ""))
    .map((path) => (path.includes(" -> ") ? path.split(" -> ")[1] : path))
    .filter((path) => !path.split("/").includes("node_modules"));
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

function listFiles(dir, pattern) {
  const root = join(repo, dir);
  if (!existsSync(root)) return [];
  const out = [];
  for (const name of readdirSync(root)) {
    const full = join(root, name);
    if (name === "node_modules") continue;
    if (statSync(full).isDirectory()) out.push(...listFiles(relative(repo, full), pattern));
    else if (pattern.test(name)) out.push(relative(repo, full).split(sep).join("/"));
  }
  return out.sort();
}

/**
 * Run the repository's own `node:test` files under `dir` and return whether
 * they passed. Files are expanded here so behavior does not depend on the
 * Node version's `--test` glob support.
 */
export function runNodeTests(dir = "test", pattern = /\.(test|spec)\.m?js$/) {
  const files = listFiles(dir, pattern);
  if (files.length === 0) {
    fail(`no test files found under ${dir}/`);
    return false;
  }
  const result = spawnSync(process.execPath, ["--test", ...files], {
    cwd: repo,
    encoding: "utf8",
    timeout: 60_000,
  });
  if (result.status !== 0) {
    const tail = `${result.stdout}\n${result.stderr}`
      .split("\n")
      .filter((line) => /not ok|Error|expected|actual/i.test(line))
      .slice(0, 6)
      .join(" | ");
    fail(`repository tests failed: ${tail || `exit ${result.status}`}`);
    return false;
  }
  return true;
}

/** Lines of `text` split without a trailing empty entry. */
export function lines(text) {
  const parts = text.split("\n");
  if (parts.at(-1) === "") parts.pop();
  return parts;
}
