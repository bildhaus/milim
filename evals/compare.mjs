#!/usr/bin/env node
// Compare eval results: per-task pass rates with a significance test, plus
// aggregate deltas. `a` is the baseline, `b` the candidate. Each side may be
// several result files (repeated runs, or runs made with --repeat); all their
// samples are pooled per task.
//
//   node evals/compare.mjs evals/results/<a>.json evals/results/<b>.json [--fail-on-regression]
//   node evals/compare.mjs --a a1.json a2.json --b b1.json b2.json [--alpha 0.05] [--fail-on-regression]

import { readFileSync } from "node:fs";
import { basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// ----- statistics -----

const logFactorials = [0];
function logFactorial(n) {
  for (let index = logFactorials.length; index <= n; index += 1) {
    logFactorials[index] = logFactorials[index - 1] + Math.log(index);
  }
  return logFactorials[n];
}
const logChoose = (n, k) => (k < 0 || k > n ? -Infinity : logFactorial(n) - logFactorial(k) - logFactorial(n - k));

/**
 * One-sided Fisher's exact test on the 2x2 table
 *   baseline:  passesA  (nA - passesA)
 *   candidate: passesB  (nB - passesB)
 * Returns the probability, with both margins fixed, of the baseline getting
 * at least `passesA` of the pooled passes, i.e. a candidate drop at least as
 * large as observed if both sides had the same pass rate.
 */
export function fisherDropP(passesA, nA, passesB, nB) {
  const total = nA + nB;
  const passes = passesA + passesB;
  const denominator = logChoose(total, nA);
  let p = 0;
  for (let x = passesA; x <= Math.min(passes, nA); x += 1) {
    p += Math.exp(logChoose(passes, x) + logChoose(total - passes, nA - x) - denominator);
  }
  return Math.min(1, p);
}

/** Exact one-sided sign test: P(X >= k) for X ~ Binomial(n, 1/2). */
export function signTestP(k, n) {
  let p = 0;
  for (let x = k; x <= n; x += 1) p += Math.exp(logChoose(n, x) - n * Math.LN2);
  return Math.min(1, p);
}

// ----- loading -----

function load(path) {
  const result = JSON.parse(readFileSync(path, "utf8"));
  if (![1, 2].includes(result.schema_version)) throw new Error(`${path}: unsupported schema_version ${result.schema_version}`);
  return result;
}

/** Pool every sample record of `results` per task id. */
function poolSide(results) {
  const tasks = new Map();
  for (const result of results) {
    for (const record of result.tasks) {
      if (record.status === "skipped") continue;
      if (!tasks.has(record.id)) tasks.set(record.id, []);
      tasks.get(record.id).push(record);
    }
  }
  return tasks;
}

const mean = (values) => {
  const finite = values.filter(Number.isFinite);
  return finite.length ? finite.reduce((a, b) => a + b, 0) / finite.length : null;
};

function taskStats(samples) {
  const passes = samples.filter((sample) => sample.status === "pass").length;
  const metric = (pick) => mean(samples.map((sample) => (sample.metrics ? pick(sample.metrics) : null)));
  return {
    n: samples.length,
    passes,
    fraction: samples.length ? passes / samples.length : null,
    steps: metric((m) => m.steps),
    tool_calls: metric((m) => m.tool_calls),
    tokens: metric((m) => m.total_tokens),
  };
}

/** Aggregate over the shared tasks so deltas compare like with like. */
function sideStats(pool, ids) {
  const samples = [...ids].flatMap((id) => pool.get(id) ?? []);
  const metric = (pick) => mean(samples.map((sample) => (sample.metrics ? pick(sample.metrics) : null)));
  const passes = samples.filter((sample) => sample.status === "pass").length;
  const taskPass = mean([...ids].map((id) => taskStats(pool.get(id) ?? []).fraction));
  return {
    samples: samples.length,
    passed: passes,
    pass_rate: samples.length ? (passes / samples.length) * 100 : null,
    mean_task_pass: taskPass === null ? null : taskPass * 100,
    unfinished: samples.filter((sample) => sample.status === "unfinished").length,
    timeouts: samples.filter((sample) => sample.status === "timeout").length,
    errors: samples.filter((sample) => sample.status === "error").length,
    mean_steps: metric((m) => m.steps),
    mean_tool_calls: metric((m) => m.tool_calls),
    mean_tool_errors: metric((m) => m.tool_errors),
    mean_tool_argument_rejections: metric((m) => m.tool_argument_rejections),
    mean_context_compactions: metric((m) => m.context_compactions),
    mean_retries: metric((m) => m.retries),
    mean_tokens: metric((m) => m.total_tokens),
    mean_cached_input_tokens: metric((m) => m.cached_input_tokens),
    mean_cost_usd: metric((m) => m.cost_usd),
    mean_wall_s: mean(samples.map((sample) => sample.wall_ms / 1000)),
  };
}

// ----- report -----

const fmt = (value, digits = 1) => (value === null || value === undefined || Number.isNaN(value) ? "-" : Number(value).toFixed(digits));
const delta = (before, after, digits = 1) => {
  if (before == null || after == null || Number.isNaN(before) || Number.isNaN(after)) return "-";
  const diff = after - before;
  return `${diff > 0 ? "+" : ""}${diff.toFixed(digits)}`;
};
const describe = (paths, results) =>
  paths.map((path, index) => `${basename(path)} (${results[index].model}, ${results[index].started_at})`).join(", ");

function parseArgs(argv) {
  const sides = { a: [], b: [], positional: [] };
  let current = "positional";
  let alpha = 0.05;
  let failOnRegression = false;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--a" || arg === "--b") current = arg.slice(2);
    else if (arg === "--alpha") alpha = Number(argv[(index += 1)]);
    else if (arg === "--fail-on-regression") failOnRegression = true;
    else if (arg.startsWith("--")) throw new Error(`unknown option ${arg}`);
    else sides[current].push(arg);
  }
  if (sides.a.length === 0 && sides.b.length === 0 && sides.positional.length === 2) {
    [sides.a, sides.b] = [[sides.positional[0]], [sides.positional[1]]];
  } else if (sides.positional.length > 0) {
    throw new Error("give either two positional files or --a ... --b ...");
  }
  if (sides.a.length === 0 || sides.b.length === 0) throw new Error("both a baseline and a candidate are required");
  if (!(alpha > 0 && alpha < 1)) throw new Error("--alpha must be between 0 and 1");
  return { a: sides.a, b: sides.b, alpha, failOnRegression };
}

function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`${error.message}\nusage: node evals/compare.mjs <baseline.json> <candidate.json> [--fail-on-regression]
       node evals/compare.mjs --a <baseline.json>... --b <candidate.json>... [--alpha 0.05] [--fail-on-regression]`);
    process.exit(2);
  }
  const { alpha } = options;
  const resultsA = options.a.map(load);
  const resultsB = options.b.map(load);
  const poolA = poolSide(resultsA);
  const poolB = poolSide(resultsB);
  const shared = new Set([...poolA.keys()].filter((id) => poolB.has(id)));

  console.log(`# Eval comparison\n`);
  console.log(`- Baseline: ${describe(options.a, resultsA)}`);
  console.log(`- Candidate: ${describe(options.b, resultsB)}`);
  console.log(`- REGRESSED/IMPROVED need a one-sided Fisher exact p <= ${alpha}; other changes are within noise.\n`);

  console.log("| Task | Baseline | Candidate | Change | p | Steps | Tool calls | Tokens |");
  console.log("|---|---:|---:|---|---:|---:|---:|---:|");
  const ids = [...new Set([...poolA.keys(), ...poolB.keys()])].sort();
  let regressions = 0;
  let down = 0;
  let up = 0;
  for (const id of ids) {
    const before = poolA.has(id) ? taskStats(poolA.get(id)) : null;
    const after = poolB.has(id) ? taskStats(poolB.get(id)) : null;
    let change = "";
    let p = null;
    if (!before) change = "new";
    else if (!after) change = "removed";
    else if (after.fraction < before.fraction) {
      down += 1;
      p = fisherDropP(before.passes, before.n, after.passes, after.n);
      change = p <= alpha ? "REGRESSED" : "down (noise)";
      if (p <= alpha) regressions += 1;
    } else if (after.fraction > before.fraction) {
      up += 1;
      p = fisherDropP(after.passes, after.n, before.passes, before.n);
      change = p <= alpha ? "IMPROVED" : "up (noise)";
    }
    const cell = (stats) => (stats ? `${stats.passes}/${stats.n}` : "-");
    console.log(
      `| ${id} | ${cell(before)} | ${cell(after)} | ${change} | ${p === null ? "" : fmt(p, 3)} | ${delta(before?.steps, after?.steps)} | ${delta(before?.tool_calls, after?.tool_calls)} | ${delta(before?.tokens, after?.tokens, 0)} |`,
    );
  }

  const sa = sideStats(poolA, shared);
  const sb = sideStats(poolB, shared);
  console.log(`\n## Aggregate over ${shared.size} shared task(s)\n`);
  console.log("| Metric | Baseline | Candidate | Delta |");
  console.log("|---|---:|---:|---:|");
  for (const [label, key, digits] of [
    ["Samples", "samples", 0],
    ["Passed", "passed", 0],
    ["Pass rate %", "pass_rate", 1],
    ["Mean task pass %", "mean_task_pass", 1],
    ["Unfinished", "unfinished", 0],
    ["Timeouts", "timeouts", 0],
    ["Errors", "errors", 0],
    ["Mean steps", "mean_steps", 1],
    ["Mean tool calls", "mean_tool_calls", 1],
    ["Mean tool errors", "mean_tool_errors", 2],
    ["Mean tool-argument rejections", "mean_tool_argument_rejections", 2],
    ["Mean context compactions", "mean_context_compactions", 2],
    ["Mean provider retries", "mean_retries", 2],
    ["Mean tokens", "mean_tokens", 0],
    ["Mean cached input tokens", "mean_cached_input_tokens", 0],
    ["Mean cost USD", "mean_cost_usd", 4],
    ["Mean wall s", "mean_wall_s", 1],
  ]) {
    console.log(`| ${label} | ${fmt(sa[key], digits)} | ${fmt(sb[key], digits)} | ${delta(sa[key], sb[key], digits)} |`);
  }

  // Suite level: each shared task is one paired observation; count tasks
  // whose pass rate went down against those that went up.
  const suiteP = down + up > 0 ? signTestP(down, down + up) : 1;
  const suiteRegressed = down > up && suiteP <= alpha;
  console.log(
    `\nSuite: ${down} task(s) down, ${up} up; one-sided sign test p = ${fmt(suiteP, 3)}${suiteRegressed ? " — REGRESSED" : down > up ? " (within noise)" : ""}.`,
  );
  if (regressions > 0) console.log(`${regressions} task(s) regressed beyond noise.`);
  if (options.failOnRegression && (regressions > 0 || suiteRegressed)) process.exit(1);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
