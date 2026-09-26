#!/usr/bin/env node
// Compare two eval result files: per-task pass/fail changes and aggregate
// deltas. `a` is the baseline, `b` the candidate.
//
//   node evals/compare.mjs evals/results/<a>.json evals/results/<b>.json [--fail-on-regression]

import { readFileSync } from "node:fs";
import { basename } from "node:path";

const [pathA, pathB] = process.argv.slice(2).filter((arg) => !arg.startsWith("--"));
const failOnRegression = process.argv.includes("--fail-on-regression");
if (!pathA || !pathB) {
  console.error("usage: node evals/compare.mjs <baseline.json> <candidate.json>");
  process.exit(2);
}

const load = (path) => {
  const result = JSON.parse(readFileSync(path, "utf8"));
  if (result.schema_version !== 1) throw new Error(`${path}: unsupported schema_version ${result.schema_version}`);
  return result;
};
const a = load(pathA);
const b = load(pathB);

const fmt = (value, digits = 1) => (value === null || value === undefined || Number.isNaN(value) ? "-" : Number(value).toFixed(digits));
const delta = (before, after, digits = 1) => {
  if (before == null || after == null) return "-";
  const diff = after - before;
  return `${diff > 0 ? "+" : ""}${diff.toFixed(digits)}`;
};

/** Aggregate over tasks present in both runs so deltas compare like with like. */
function stats(result, ids) {
  const tasks = result.tasks.filter((task) => ids.has(task.id));
  const metric = (pick) => {
    const values = tasks.map((task) => (task.metrics ? pick(task.metrics) : null)).filter(Number.isFinite);
    return values.length ? values.reduce((sum, value) => sum + value, 0) / values.length : null;
  };
  const total = (pick) => {
    const values = tasks.map((task) => (task.metrics ? pick(task.metrics) : null)).filter(Number.isFinite);
    return values.length ? values.reduce((sum, value) => sum + value, 0) : null;
  };
  const passed = tasks.filter((task) => task.status === "pass").length;
  return {
    tasks: tasks.length,
    passed,
    pass_rate: tasks.length ? (passed / tasks.length) * 100 : null,
    mean_steps: metric((m) => m.steps),
    mean_tool_calls: metric((m) => m.tool_calls),
    tool_errors: total((m) => m.tool_errors),
    mean_tokens: metric((m) => m.total_tokens),
    total_cost_usd: total((m) => m.cost_usd),
    mean_wall_s: tasks.length ? tasks.reduce((sum, task) => sum + task.wall_ms, 0) / tasks.length / 1000 : null,
  };
}

const byId = (result) => new Map(result.tasks.map((task) => [task.id, task]));
const tasksA = byId(a);
const tasksB = byId(b);
const shared = new Set([...tasksA.keys()].filter((id) => tasksB.has(id)));

console.log(`# Eval comparison\n`);
console.log(`- Baseline: ${basename(pathA)} (${a.model}, ${a.started_at})`);
console.log(`- Candidate: ${basename(pathB)} (${b.model}, ${b.started_at})\n`);

console.log("| Task | Baseline | Candidate | Change | Steps | Tool calls | Tokens |");
console.log("|---|---|---|---|---:|---:|---:|");
const ids = [...new Set([...tasksA.keys(), ...tasksB.keys()])].sort();
let regressions = 0;
for (const id of ids) {
  const before = tasksA.get(id);
  const after = tasksB.get(id);
  let change = "";
  if (!before) change = "new";
  else if (!after) change = "removed";
  else if (before.status === "pass" && after.status !== "pass") {
    change = "REGRESSED";
    regressions += 1;
  } else if (before.status !== "pass" && after.status === "pass") change = "fixed";
  console.log(
    `| ${id} | ${before?.status ?? "-"} | ${after?.status ?? "-"} | ${change} | ${delta(before?.metrics?.steps, after?.metrics?.steps, 0)} | ${delta(before?.metrics?.tool_calls, after?.metrics?.tool_calls, 0)} | ${delta(before?.metrics?.total_tokens, after?.metrics?.total_tokens, 0)} |`,
  );
}

const sa = stats(a, shared);
const sb = stats(b, shared);
console.log(`\n## Aggregate over ${shared.size} shared task(s)\n`);
console.log("| Metric | Baseline | Candidate | Delta |");
console.log("|---|---:|---:|---:|");
for (const [label, key, digits] of [
  ["Passed", "passed", 0],
  ["Pass rate %", "pass_rate", 1],
  ["Mean steps", "mean_steps", 1],
  ["Mean tool calls", "mean_tool_calls", 1],
  ["Tool errors", "tool_errors", 0],
  ["Mean tokens", "mean_tokens", 0],
  ["Total cost USD", "total_cost_usd", 4],
  ["Mean wall s", "mean_wall_s", 1],
]) {
  console.log(`| ${label} | ${fmt(sa[key], digits)} | ${fmt(sb[key], digits)} | ${delta(sa[key], sb[key], digits)} |`);
}
if (regressions > 0) {
  console.log(`\n${regressions} task(s) regressed.`);
  if (failOnRegression) process.exit(1);
}
