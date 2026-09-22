// Discovers tests/*.test.{ts,tsx,mjs}, compiles every TypeScript test in one
// tsc pass, then runs each test file in its own Node process. A file fails when
// its process exits non-zero (thrown assertion, failed node:test case,
// unhandled rejection) or exceeds the per-file timeout.
//
// Usage: node tests/run-tests.mjs [filter ...]
// A filter is a test path (tests/foo.test.ts) or a substring of the file name.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { availableParallelism } from "node:os";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const testsDir = join(root, "tests");
const testFilePattern = /\.test\.(?:tsx?|mjs)$/;
const timeoutMs = Number(process.env.MILIM_TEST_TIMEOUT_MS) || 180_000;
const concurrency = Math.max(1, Number(process.env.MILIM_TEST_CONCURRENCY) || availableParallelism());

const allTests = readdirSync(testsDir).filter((name) => testFilePattern.test(name)).sort();
const tests = selectTests(allTests, process.argv.slice(2));
const tsTests = tests.filter((name) => /\.tsx?$/.test(name));

const tmpName = `.tmp-tests-${process.pid}-${Date.now()}`;
const tmp = join(root, tmpName);
const started = performance.now();

try {
  if (tsTests.length) compileTypeScript(tsTests);
  const results = await runAll(tests.map(testCommand), concurrency);
  const failed = results.filter((result) => !result.ok);
  const seconds = ((performance.now() - started) / 1000).toFixed(1);
  if (failed.length) {
    console.error(`\n${failed.length} of ${results.length} test files failed (${seconds}s):`);
    for (const result of failed) console.error(`  ${result.name}: ${result.reason}`);
    process.exitCode = 1;
  } else {
    console.log(`\n${results.length} test files passed (${seconds}s)`);
  }
} finally {
  rmSync(tmp, { recursive: true, force: true });
}

function selectTests(names, filters) {
  if (!names.length) throw new Error(`No test files found in ${testsDir}`);
  if (!filters.length) return names;
  const selected = new Set();
  for (const filter of filters) {
    const exact = basename(filter);
    const matches = testFilePattern.test(exact)
      ? names.filter((name) => name === exact)
      : names.filter((name) => name.includes(filter));
    if (!matches.length) throw new Error(`No test file matches ${filter}`);
    for (const match of matches) selected.add(match);
  }
  return [...selected].sort();
}

function compileTypeScript(names) {
  mkdirSync(tmp, { recursive: true });
  const tsc = createRequire(join(root, "package.json")).resolve("typescript/bin/tsc");
  const args = [
    tsc,
    "--target",
    "ES2022",
    "--module",
    "ES2022",
    "--moduleResolution",
    "bundler",
    "--skipLibCheck",
    "--jsx",
    "react-jsx",
    "--rootDir",
    ".",
    "--outDir",
    tmpName,
    "--noEmit",
    "false",
    ...names.map((name) => join("tests", name)),
  ];
  const result = spawnSync(process.execPath, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    process.exitCode = 1;
    throw new Error(`TypeScript compilation of ${names.length} test files failed with exit ${result.status}`);
  }
}

function testCommand(name) {
  const file = /\.tsx?$/.test(name)
    ? join(tmp, "tests", name.replace(/\.tsx?$/, ".js"))
    : join(testsDir, name);
  if (!existsSync(file)) throw new Error(`Compiled test not found for ${name}: ${file}`);
  return { name, file };
}

async function runAll(commands, limit) {
  const results = [];
  let next = 0;
  async function worker() {
    while (next < commands.length) {
      const command = commands[next++];
      const result = await runOne(command);
      results.push(result);
      const label = result.ok ? "ok  " : "FAIL";
      const output = result.output.trimEnd();
      if (!result.ok || process.env.MILIM_TEST_VERBOSE) {
        console.log(`${label} ${result.name} (${result.ms} ms)${output ? `\n${indent(output)}` : ""}`);
      } else {
        console.log(`${label} ${result.name} (${result.ms} ms)`);
      }
    }
  }
  await Promise.all(Array.from({ length: Math.min(limit, commands.length) }, worker));
  return results;
}

function runOne({ name, file }) {
  return new Promise((resolve) => {
    const start = performance.now();
    const chunks = [];
    const child = spawn(process.execPath, [file], { cwd: root, stdio: ["ignore", "pipe", "pipe"] });
    child.stdout.on("data", (chunk) => chunks.push(chunk));
    child.stderr.on("data", (chunk) => chunks.push(chunk));
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, timeoutMs);
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      const ms = Math.round(performance.now() - start);
      const output = Buffer.concat(chunks).toString("utf8");
      const ok = code === 0 && !timedOut;
      const reason = timedOut
        ? `timed out after ${timeoutMs} ms (open handles keep the process alive?)`
        : signal
          ? `killed by ${signal}`
          : `exit ${code}`;
      resolve({ name, ok, ms, output, reason });
    });
  });
}

function indent(text) {
  return text.replace(/^/gm, "    ");
}
