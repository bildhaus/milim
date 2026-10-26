// Task loading, fixture preparation, and check execution shared by run.mjs
// (model runs) and validate.mjs (grader validation without a model).

import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync } from "node:fs";
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

/**
 * A task's optional approval policy, normalized. `deny` rules answer
 * matching approval requests with deny; every other request is approved.
 * A rule has `tool` (a tool name or a list of names; omitted matches any
 * tool) and `pattern`, a regular expression tested against the call's
 * command for shell tools, its path for file tools, and its JSON arguments
 * otherwise; `first_only` denies only the first matching request. A task
 * with a policy runs with approval mode `mode` (default `review`), so the
 * host emits approval requests at all.
 */
function normalizeApprovals(task, name) {
  if (task.approvals === undefined) return null;
  const { mode = "review", deny = [] } = task.approvals ?? {};
  if (!["review", "guarded", "open"].includes(mode)) throw new Error(`${name}/task.json approvals.mode must be review, guarded, or open`);
  if (!Array.isArray(deny)) throw new Error(`${name}/task.json approvals.deny must be an array`);
  return {
    mode,
    deny: deny.map((rule, index) => {
      if (typeof rule?.pattern !== "string") throw new Error(`${name}/task.json approvals.deny[${index}] needs a pattern`);
      new RegExp(rule.pattern); // throws on an invalid pattern
      return { tool: rule.tool === undefined ? null : [rule.tool].flat(), pattern: rule.pattern, first_only: rule.first_only === true };
    }),
  };
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
      if (task.setup !== undefined && typeof task.setup !== "string") throw new Error(`${entry.name}/task.json setup must be a command string`);
      return {
        ...task,
        turns: normalizeTurns(task, entry.name),
        requires: task.requires ?? [],
        approvals: normalizeApprovals(task, entry.name),
        dir,
      };
    })
    .filter((task) => wanted.length === 0 || wanted.some((value) => task.id === value || task.id.includes(value)))
    .sort((a, b) => a.id.localeCompare(b.id));
}

/** Total time budget for all of a task's turns, in seconds. */
export const taskTimeout = (task) => task.turns.reduce((sum, turn) => sum + turn.timeout_s, 0);

export function git(cwd, gitArgs, { env } = {}) {
  const result = spawnSync(
    "git",
    ["-c", "user.name=milim-eval", "-c", "user.email=eval@milim.invalid", "-c", "commit.gpgsign=false", ...gitArgs],
    { cwd, encoding: "utf8", ...(env ? { env: { ...process.env, ...env } } : {}) },
  );
  if (result.status !== 0) throw new Error(`git ${gitArgs[0]} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

/**
 * Copy the fixture to a fresh temp dir and commit it as the baseline. A
 * task's optional `setup` command (for example `node setup.mjs`, resolved
 * against the task directory like `check`) runs after `git init` and may
 * build commit history; whatever it leaves uncommitted becomes the final
 * baseline commit, so `HEAD` is always the state the agent starts from.
 */
export function prepareRepo(task) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), `milim-eval-${task.id}-`)));
  cpSync(join(task.dir, "repo"), root, { recursive: true });
  git(root, ["init", "-q", "-b", "main"]);
  if (task.setup) {
    const result = runTaskCommand(task, task.setup, root, 60_000);
    if (result.status !== 0) {
      throw new Error(`setup exited ${result.status}: ${`${result.stdout ?? ""}${result.stderr ?? ""}`.trim().slice(0, 400)}`);
    }
  }
  const hasHead = spawnSync("git", ["rev-parse", "--verify", "-q", "HEAD"], { cwd: root }).status === 0;
  if (!hasHead || git(root, ["status", "--porcelain", "--untracked-files=all"]).trim()) {
    git(root, ["add", "-A"]);
    git(root, ["commit", "-q", "-m", "eval fixture"]);
  }
  return root;
}

/** Tracked and untracked paths that differ from the committed baseline. */
export function changedPaths(repo) {
  return git(repo, ["status", "--porcelain", "--untracked-files=all"])
    .split("\n")
    .filter(Boolean)
    .map((line) => line.slice(3).replace(/^"|"$/g, ""))
    .map((path) => (path.includes(" -> ") ? path.split(" -> ")[1] : path))
    .filter((path) => !path.split("/").some((part) => part === "node_modules" || part === "__pycache__"));
}

/**
 * Record the workspace as a git tree object without touching the
 * repository's own index or history (a throwaway index file is used). The
 * tree honors `.gitignore`. Checks read files from it with `git show
 * <tree>:<path>`, so a multi-turn grader can see what each turn left.
 */
export function snapshotTree(repo) {
  const dir = mkdtempSync(join(tmpdir(), "milim-eval-index-"));
  try {
    const env = { GIT_INDEX_FILE: join(dir, "index") };
    git(repo, ["add", "-A"], { env });
    return git(repo, ["write-tree"], { env }).trim();
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** The first Python 3 interpreter on PATH, or null. */
export function findPython() {
  for (const command of ["python3", "python"]) {
    const result = spawnSync(command, ["-c", "import sys; print(sys.version_info[0])"], { encoding: "utf8" });
    if (result.status === 0 && result.stdout.trim() === "3") return command;
  }
  return null;
}

/**
 * The TypeScript compiler as `[executable, ...args]`, or null: `tsc` on
 * PATH, else the `typescript` devDependency of the desktop or site app when
 * its node_modules are installed. The suite adds no npm dependency of its own.
 */
export function findTsc() {
  if (spawnSync("tsc", ["--version"], { encoding: "utf8" }).status === 0) return ["tsc"];
  const root = dirname(EVALS);
  for (const app of ["apps/desktop", "apps/site"]) {
    const bin = join(root, app, "node_modules", "typescript", "bin", "tsc");
    if (existsSync(bin)) return [process.execPath, bin];
  }
  return null;
}

/** Whether `cargo` runs on this machine. */
export const hasCargo = () => spawnSync("cargo", ["--version"], { encoding: "utf8" }).status === 0;

const REQUIREMENTS = {
  python3: () => (findPython() ? null : "python3 is not installed"),
  cargo: () => (hasCargo() ? null : "cargo is not installed"),
  // The fixture's tests run as .ts through Node's built-in type stripping
  // (on by default since Node 22.18 and 23.6).
  typescript: () =>
    !process.features?.typescript
      ? `Node ${process.version} does not strip TypeScript types by default`
      : findTsc()
        ? null
        : "the TypeScript compiler (tsc) is not installed",
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

/** Run a task command (check or setup) with cwd = repo; script args resolve against the task dir. */
function runTaskCommand(task, commandLine, repo, timeout, extraEnv = {}) {
  const [command, ...rest] = commandLine.split(/\s+/).filter(Boolean);
  const argv = rest.map((arg) => (existsSync(join(task.dir, arg)) ? join(task.dir, arg) : arg));
  const executable = command === "node" ? process.execPath : command;
  return spawnSync(executable, argv, {
    cwd: repo,
    encoding: "utf8",
    timeout,
    env: scrubbedEnv({ EVAL_REPO: repo, EVAL_TASK_DIR: task.dir, ...extraEnv }),
  });
}

/**
 * Run the task check with cwd = repo. `trajectory` is the path of the
 * sample's trajectory file (turn answers, per-turn workspace snapshots,
 * tool calls, approvals; see lib/trajectory.mjs), passed as EVAL_TRAJECTORY.
 */
export function runCheck(task, repo, { trajectory } = {}) {
  const started = Date.now();
  const result = runTaskCommand(task, task.check, repo, CHECK_TIMEOUT_MS, trajectory ? { EVAL_TRAJECTORY: trajectory } : {});
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
