// The per-sample trajectory: what each turn answered and left in the
// workspace, every tool call, and every approval decision. run.mjs builds it
// from the thread timeline and run ledger and hands its path to the check as
// EVAL_TRAJECTORY; validate.mjs hands the same path to a solution script,
// which simulates a run with `recordTurn`. It also derives the trajectory
// waste metrics that run.mjs reports.
//
// {
//   "version": 1,
//   "turns": [{ "turn", "prompt", "run_id", "run_status", "answer", "ended_at_ms", "snapshot", "changed_files" }],
//   "tool_calls": [{ "turn", "name", "arguments", "is_error", "denied", "error", "created_at_ms", "step" }],
//   "approvals": [{ "turn", "approval_id", "name", "arguments", "decision", "rule" }]
// }
//
// `snapshot` is a git tree id of the workspace after that turn (see
// snapshotTree in tasks.mjs) and `ended_at_ms` the wall clock when the turn's
// run finished; `rule` is the index of the task's deny rule that decided the
// approval, or null when the runner approved it.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { changedPaths, snapshotTree } from "./tasks.mjs";

export const TRAJECTORY_VERSION = 1;

const SHELL_TOOLS = /^(shell|bash|run_command|exec_command|local_shell|terminal)$/i;
const READ_TOOLS = /^(read_file|read|view|open_file)$/i;
const WRITE_TOOLS = /^(write_file|edit_file|patch_file|apply_patch|multi_edit|multiedit|create_file|str_replace_editor|str_replace_based_edit_tool|edit|write)$/i;
/** Shell commands that run a test suite, a build, or a type check. */
const TEST_OR_BUILD =
  /\b(?:npm|pnpm|yarn|bun)\s+(?:run\s+)?(?:test|build|typecheck|check|lint)\b|\bnode\s+(?:--[\w-]+\s+)*--test\b|\bcargo\s+(?:test|build|check|clippy|nextest)\b|\bpytest\b|\bpython3?\s+-m\s+(?:pytest|unittest)\b|\btsc\b|\b(?:vitest|jest|mocha)\b|\bgo\s+(?:test|build|vet)\b|\bmake\b/;

/** Tool arguments as an object; tool_call items carry them as a JSON string. */
export function parseArguments(raw) {
  if (raw && typeof raw === "object") return raw;
  if (typeof raw !== "string") return {};
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" ? value : { value };
  } catch {
    return { raw };
  }
}

const pathOf = (args) => args.path ?? args.file_path ?? args.filePath ?? args.filename ?? null;
const commandOf = (args) => {
  const command = args.command ?? args.cmd ?? args.script;
  return Array.isArray(command) ? command.join(" ") : typeof command === "string" ? command : null;
};
const normalizePath = (path) => String(path).replace(/\\/g, "/").replace(/^\.\//, "");

/** Stable JSON with sorted keys, for comparing two calls' arguments. */
function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

/**
 * What a deny rule's pattern is tested against: the command of a shell
 * call, the path of a file call, otherwise the arguments as JSON.
 */
export function ruleSubject(name, args) {
  const parsed = parseArguments(args);
  if (SHELL_TOOLS.test(name ?? "")) return commandOf(parsed) ?? canonical(parsed);
  const path = pathOf(parsed);
  if (path !== null && (READ_TOOLS.test(name ?? "") || WRITE_TOOLS.test(name ?? ""))) return normalizePath(path);
  return canonical(parsed);
}

/** Index of the first deny rule matching a call, or -1. */
export function matchingRule(rules, name, args) {
  const subject = ruleSubject(name, args);
  return (rules ?? []).findIndex(
    (rule) => (!rule.tool || rule.tool.includes(name)) && new RegExp(rule.pattern).test(subject),
  );
}

/**
 * Trajectory waste metrics over the tool calls of one sample, in call order:
 *
 * - redundant_reads: a read of the same path and range as an earlier read
 *   in the same turn with no write or edit to that path in between;
 * - failed_edits: write/edit/patch calls whose result was an error (a denied
 *   call is not counted here);
 * - repeated_identical_calls: a call with the same tool and arguments as
 *   the call directly before it;
 * - test_build_runs: shell commands that run tests, a build, or a type check;
 * - denied_calls and denied_retries: denied calls, and later calls that
 *   repeat one (same tool and command/path, or matching the same deny rule);
 * - calls_to_first_change, steps_to_first_change, first_change_ms: tool calls,
 *   model steps (`step`, numbered across turns), and milliseconds from
 *   `startedAtMs` until the first successful write/edit/patch result (null
 *   when nothing was written through a file tool).
 */
export function wasteMetrics(calls, { rules = [], startedAtMs = null } = {}) {
  let redundantReads = 0;
  let failedEdits = 0;
  let repeated = 0;
  let testRuns = 0;
  let deniedCalls = 0;
  let deniedRetries = 0;
  let firstChange = null;
  const reads = new Map();
  const denied = new Set();
  let previous = null;
  let turn = null;
  for (const [index, call] of calls.entries()) {
    const name = call.name ?? "";
    const args = parseArguments(call.arguments);
    if (call.turn !== turn) {
      reads.clear();
      previous = null;
      turn = call.turn;
    }
    const signature = `${name}:${canonical(args)}`;
    if (previous === signature) repeated += 1;
    previous = signature;

    const path = pathOf(args) === null ? null : normalizePath(pathOf(args));
    if (READ_TOOLS.test(name) && path !== null && !call.is_error) {
      const range = canonical({ ...args, path: undefined, file_path: undefined, filePath: undefined });
      const seen = reads.get(path) ?? new Set();
      if (seen.has(range) || seen.has("full")) redundantReads += 1;
      seen.add(Object.keys(args).length <= 1 ? "full" : range);
      reads.set(path, seen);
    }
    if (WRITE_TOOLS.test(name)) {
      if (path !== null) reads.delete(path);
      if (call.is_error && !call.denied) failedEdits += 1;
      if (!call.is_error && firstChange === null) firstChange = { index, call };
    }
    if (SHELL_TOOLS.test(name) && TEST_OR_BUILD.test(commandOf(args) ?? "")) testRuns += 1;

    const keys = [`${name}:${ruleSubject(name, args)}`];
    const rule = matchingRule(rules, name, args);
    if (rule >= 0) keys.push(`rule:${rule}`);
    if (keys.some((key) => denied.has(key))) deniedRetries += 1;
    if (call.denied) {
      deniedCalls += 1;
      for (const key of keys) denied.add(key);
    }
  }
  return {
    redundant_reads: redundantReads,
    failed_edits: failedEdits,
    repeated_identical_calls: repeated,
    test_build_runs: testRuns,
    denied_calls: deniedCalls,
    denied_retries: deniedRetries,
    calls_to_first_change: firstChange ? firstChange.index : null,
    steps_to_first_change: Number.isFinite(firstChange?.call.step) ? firstChange.call.step : null,
    first_change_ms:
      firstChange && Number.isFinite(firstChange.call.created_at_ms) && Number.isFinite(startedAtMs)
        ? firstChange.call.created_at_ms - startedAtMs
        : null,
  };
}

export function emptyTrajectory() {
  return { version: TRAJECTORY_VERSION, turns: [], tool_calls: [], approvals: [] };
}

export function readTrajectory(path) {
  if (!path || !existsSync(path)) return null;
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeTrajectory(path, trajectory) {
  writeFileSync(path, `${JSON.stringify(trajectory, null, 2)}\n`);
}

/**
 * For solution scripts under validate.mjs: end a simulated turn. Appends a
 * turn with `answer` and a snapshot of the current workspace, plus any
 * simulated `tool_calls` and `approvals`, to the EVAL_TRAJECTORY file.
 */
export function recordTurn({ answer = "", toolCalls = [], approvals = [], runStatus = "completed" } = {}) {
  const path = process.env.EVAL_TRAJECTORY;
  if (!path) throw new Error("EVAL_TRAJECTORY is not set (run this through validate.mjs)");
  const repo = process.cwd();
  const trajectory = readTrajectory(path) ?? emptyTrajectory();
  const turn = trajectory.turns.length + 1;
  trajectory.turns.push({
    turn,
    prompt: null,
    run_id: `simulated-${turn}`,
    run_status: runStatus,
    answer,
    ended_at_ms: Date.now(),
    snapshot: snapshotTree(repo),
    changed_files: changedPaths(repo),
  });
  for (const call of toolCalls) {
    trajectory.tool_calls.push({ turn, is_error: false, denied: false, error: null, ...call });
  }
  for (const approval of approvals) trajectory.approvals.push({ turn, rule: null, ...approval });
  writeTrajectory(path, trajectory);
}
