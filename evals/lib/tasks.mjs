// Task loading, fixture preparation, and check execution shared by run.mjs
// (model runs) and validate.mjs (grader validation without a model).

import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const EVALS = dirname(dirname(fileURLToPath(import.meta.url)));
export const TASKS_DIR = join(EVALS, "tasks");
export const CHECK_TIMEOUT_MS = 120_000;
/** A check exits with this code when a requirement such as python3 is missing. */
export const SKIP_EXIT_CODE = 77;
const OUTPUT_LIMIT = 2_000;

/** Every task's `turns` normalized to `[{ prompt, timeout_s }]`. */
function normalizeTurns(task, name) {
  if (task.turns === undefined) {
    if (typeof task.prompt !== "string") throw new Error(`${name}/task.json needs prompt or turns`);
    return [{ prompt: task.prompt, timeout_s: task.timeout_s }];
  }
  if (!Array.isArray(task.turns) || task.turns.length === 0) throw new Error(`${name}/task.json turns must be a non-empty array`);
  return task.turns.map((turn, index) => {
    const prompt = typeof turn === "string" ? turn : turn?.prompt;
    if (typeof prompt !== "string") throw new Error(`${name}/task.json turn ${index + 1} has no prompt`);
    return { prompt, timeout_s: turn?.timeout_s ?? task.timeout_s };
  });
}

/** Tasks whose id equals or contains one of the comma-separated filters. */
export function loadTasks(filter) {
  const wanted = (filter ?? "")
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
  return readdirSync(TASKS_DIR, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const dir = join(TASKS_DIR, entry.name);
      const task = JSON.parse(readFileSync(join(dir, "task.json"), "utf8"));
      if (task.id !== entry.name) throw new Error(`task.json id ${task.id} does not match ${entry.name}`);
      for (const key of ["title", "category", "timeout_s", "check"]) {
        if (task[key] === undefined) throw new Error(`${entry.name}/task.json is missing ${key}`);
      }
      if (!existsSync(join(dir, "repo"))) throw new Error(`${entry.name} has no repo/ fixture`);
      return { ...task, turns: normalizeTurns(task, entry.name), requires: task.requires ?? [], dir };
    })
    .filter((task) => wanted.length === 0 || wanted.some((value) => task.id === value || task.id.includes(value)))
    .sort((a, b) => a.id.localeCompare(b.id));
}

/** Total time budget for all of a task's turns, in seconds. */
export const taskTimeout = (task) => task.turns.reduce((sum, turn) => sum + turn.timeout_s, 0);

export function git(cwd, gitArgs) {
  const result = spawnSync(
    "git",
    ["-c", "user.name=milim-eval", "-c", "user.email=eval@milim.invalid", "-c", "commit.gpgsign=false", ...gitArgs],
    { cwd, encoding: "utf8" },
  );
  if (result.status !== 0) throw new Error(`git ${gitArgs[0]} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

/** Copy the fixture to a fresh temp dir and commit it as the baseline. */
export function prepareRepo(task) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), `milim-eval-${task.id}-`)));
  cpSync(join(task.dir, "repo"), root, { recursive: true });
  git(root, ["init", "-q", "-b", "main"]);
  git(root, ["add", "-A"]);
  git(root, ["commit", "-q", "-m", "eval fixture"]);
  return root;
}

/** The first Python 3 interpreter on PATH, or null. */
export function findPython() {
  for (const command of ["python3", "python"]) {
    const result = spawnSync(command, ["-c", "import sys; print(sys.version_info[0])"], { encoding: "utf8" });
    if (result.status === 0 && result.stdout.trim() === "3") return command;
  }
  return null;
}

const REQUIREMENTS = {
  python3: () => (findPython() ? null : "python3 is not installed"),
};

/** Why the task cannot run on this machine, or null when it can. */
export function missingRequirement(task) {
  for (const name of task.requires) {
    const probe = REQUIREMENTS[name];
    if (!probe) return `unknown requirement ${name}`;
    const reason = probe();
    if (reason) return reason;
  }
  return null;
}

/**
 * Environment for check and solution scripts: the runner's own credentials
 * (device key, API token, control file, provider keys) never reach them.
 */
export function scrubbedEnv(extra = {}) {
  const env = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (/KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL/i.test(key) || key === "MILIM_E2E_CONTROL_FILE") continue;
    env[key] = value;
  }
  return { ...env, ...extra };
}

/** Run the task check with cwd = repo; script args resolve against the task dir. */
export function runCheck(task, repo) {
  const [command, ...rest] = task.check.split(/\s+/).filter(Boolean);
  const argv = rest.map((arg) => (existsSync(join(task.dir, arg)) ? join(task.dir, arg) : arg));
  const executable = command === "node" ? process.execPath : command;
  const started = Date.now();
  const result = spawnSync(executable, argv, {
    cwd: repo,
    encoding: "utf8",
    timeout: CHECK_TIMEOUT_MS,
    env: scrubbedEnv({ EVAL_REPO: repo, EVAL_TASK_DIR: task.dir }),
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  return {
    exit_code: result.status,
    passed: result.status === 0,
    skipped: result.status === SKIP_EXIT_CODE,
    timed_out: result.error?.code === "ETIMEDOUT",
    duration_ms: Date.now() - started,
    output: output.length > OUTPUT_LIMIT ? `${output.slice(0, OUTPUT_LIMIT)}...` : output,
  };
}
