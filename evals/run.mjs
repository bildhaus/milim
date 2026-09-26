#!/usr/bin/env node
// Agent-quality eval runner. Drives a running milim desktop app through the
// canonical /control/v1 API: one fresh thread per task, bound to a temporary
// git copy of the task fixture, then a deterministic check script decides
// pass or fail. See evals/README.md.
//
//   MILIM_CONTROL_URL=... MILIM_DEVICE_KEY=... MILIM_EVAL_MODEL=... node evals/run.mjs
//   node evals/run.mjs --dry-run      # list selected tasks, no app needed
//   node evals/run.mjs --self-test    # plumbing check with the mock-echo model

import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EVALS = dirname(fileURLToPath(import.meta.url));
const TASKS_DIR = join(EVALS, "tasks");
const RESULTS_DIR = join(EVALS, "results");
const TERMINAL = new Set(["completed", "failed", "cancelled", "interrupted"]);
const REQUEST_EVENTS = new Set(["model_request_resolved", "harness_request_committed"]);
const CHECK_TIMEOUT_MS = 120_000;
const OUTPUT_LIMIT = 2_000;

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

if (flag("--help") || flag("-h")) {
  console.log(`usage: node evals/run.mjs [--dry-run] [--self-test] [--tasks a,b] [--out file.json]

env:
  MILIM_CONTROL_URL   Mobile/control URL shown under Settings > Mobile (required)
  MILIM_DEVICE_KEY    Paired device bearer key (required, never logged)
  MILIM_EVAL_MODEL    Thread model id, e.g. provider:openrouter:... or codex:gpt-5 (required)
  MILIM_EVAL_TASKS    Comma list of task ids or substrings to run (optional)
  MILIM_EVAL_REASONING_EFFORT  Per-thread reasoning effort override (optional)
  MILIM_EVAL_TIMEOUT_SCALE     Multiply every task timeout (optional, default 1)
  MILIM_EVAL_KEEP=1   Keep temporary task repositories
  MILIM_EVAL_ARCHIVE=1  Archive each eval thread after it finishes`);
  process.exit(0);
}

// ----- tasks -----

function loadTasks(filter) {
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
      for (const key of ["title", "category", "prompt", "timeout_s", "check"]) {
        if (task[key] === undefined) throw new Error(`${entry.name}/task.json is missing ${key}`);
      }
      if (!existsSync(join(dir, "repo"))) throw new Error(`${entry.name} has no repo/ fixture`);
      return { ...task, dir };
    })
    .filter((task) => wanted.length === 0 || wanted.some((value) => task.id === value || task.id.includes(value)))
    .sort((a, b) => a.id.localeCompare(b.id));
}

function git(cwd, gitArgs) {
  const result = spawnSync(
    "git",
    ["-c", "user.name=milim-eval", "-c", "user.email=eval@milim.invalid", "-c", "commit.gpgsign=false", ...gitArgs],
    { cwd, encoding: "utf8" },
  );
  if (result.status !== 0) throw new Error(`git ${gitArgs[0]} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

/** Copy the fixture to a fresh temp dir and commit it as the baseline. */
function prepareRepo(task) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), `milim-eval-${task.id}-`)));
  cpSync(join(task.dir, "repo"), root, { recursive: true });
  git(root, ["init", "-q", "-b", "main"]);
  git(root, ["add", "-A"]);
  git(root, ["commit", "-q", "-m", "eval fixture"]);
  return root;
}

/** Run the task check with cwd = repo; script args resolve against the task dir. */
function runCheck(task, repo) {
  const [command, ...rest] = task.check.split(/\s+/).filter(Boolean);
  const argv = rest.map((arg) => (existsSync(join(task.dir, arg)) ? join(task.dir, arg) : arg));
  const executable = command === "node" ? process.execPath : command;
  const started = Date.now();
  const result = spawnSync(executable, argv, {
    cwd: repo,
    encoding: "utf8",
    timeout: CHECK_TIMEOUT_MS,
    env: { ...process.env, MILIM_DEVICE_KEY: "", EVAL_REPO: repo, EVAL_TASK_DIR: task.dir },
  });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  return {
    exit_code: result.status,
    timed_out: result.error?.code === "ETIMEDOUT",
    duration_ms: Date.now() - started,
    output: output.length > OUTPUT_LIMIT ? `${output.slice(0, OUTPUT_LIMIT)}...` : output,
  };
}

// ----- control client -----

class ControlClient {
  constructor(baseUrl, deviceKey) {
    this.base = baseUrl.replace(/\/+$/, "");
    this.key = deviceKey;
    this.sequence = 0;
    this.tag = `eval-${Date.now().toString(36)}-${randomUUID().slice(0, 8)}`;
  }

  async request(method, path, body, { attempts = 3 } = {}) {
    let lastError;
    for (let attempt = 1; attempt <= attempts; attempt += 1) {
      try {
        const response = await fetch(`${this.base}${path}`, {
          method,
          headers: {
            Authorization: `Bearer ${this.key}`,
            ...(body === undefined ? {} : { "Content-Type": "application/json" }),
          },
          body: body === undefined ? undefined : JSON.stringify(body),
          signal: AbortSignal.timeout(30_000),
        });
        const text = await response.text();
        if (response.status >= 500 && attempt < attempts) {
          lastError = new Error(`${method} ${path}: HTTP ${response.status}`);
        } else if (!response.ok) {
          throw Object.assign(new Error(`${method} ${path}: HTTP ${response.status} ${text.slice(0, 300)}`), {
            status: response.status,
            final: true,
          });
        } else {
          return text ? JSON.parse(text) : null;
        }
      } catch (error) {
        if (error.final) throw error;
        lastError = error;
      }
      await sleep(500 * attempt);
    }
    throw new Error(`${method} ${path} failed: ${lastError?.message ?? lastError}`);
  }

  /**
   * Send one command. Retries reuse the same command_id, so an ambiguous
   * network failure replays the durable receipt instead of acting twice.
   */
  async command(kind, threadId, payload) {
    this.sequence += 1;
    const command = {
      command_id: `${this.tag}-${this.sequence}-${kind}`,
      kind,
      ...(threadId ? { thread_id: threadId } : {}),
      payload,
    };
    const result = await this.request("POST", "/control/v1/commands", command, { attempts: 4 });
    if (["failed", "conflict"].includes(result?.status)) {
      throw new Error(`${kind} ${result.status}: ${result.message ?? "no message"}`);
    }
    return result;
  }

  bootstrap() {
    return this.request("GET", "/control/v1/bootstrap");
  }

  run(runId) {
    return this.request("GET", `/control/v1/runs/${encodeURIComponent(runId)}`);
  }

  async runEvents(runId) {
    const events = [];
    let after;
    for (let page = 0; page < 500; page += 1) {
      const query = new URLSearchParams({ limit: "200", ...(after === undefined ? {} : { after_seq: String(after) }) });
      const result = await this.request("GET", `/control/v1/runs/${encodeURIComponent(runId)}/events?${query}`);
      events.push(...(result?.events ?? []));
      if (!result?.has_more || result.next_seq === undefined || result.next_seq === null) break;
      after = result.next_seq;
    }
    return events;
  }

  async timeline(threadId) {
    const items = [];
    let before;
    for (let page = 0; page < 20; page += 1) {
      const query = before === undefined ? "tail=500" : `before_seq=${before}&limit=500`;
      const result = await this.request("GET", `/control/v1/threads/${encodeURIComponent(threadId)}/timeline?${query}`);
      items.unshift(...(result?.items ?? []));
      if (!result?.has_older || result.first_seq === undefined || result.first_seq === null) break;
      before = result.first_seq;
    }
    return items;
  }
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

// ----- metrics -----

const eventType = (event) => event.type ?? event.event_type ?? event.item_type;
const stepOf = (event) => {
  const match = /^step-(\d+)$/.exec(event.step_id ?? "");
  return match ? Number(match[1]) : event.data?.step ?? null;
};

function percentile(values, fraction) {
  const sorted = values.filter((value) => Number.isFinite(value)).sort((a, b) => a - b);
  if (sorted.length === 0) return null;
  return sorted[Math.min(sorted.length - 1, Math.ceil(fraction * sorted.length) - 1)];
}

function toolResultIsError(data) {
  if (typeof data?.is_error === "boolean") return data.is_error;
  const result = data?.artifact?.result ?? data?.result;
  return Boolean(result && typeof result === "object" && (result.error !== undefined || result.is_error === true));
}

/**
 * Metrics from the run ledger, preferring explicit model_timing/tool_timing
 * events and falling back to event timestamps and timeline projections.
 * Account runtimes record only a harness boundary, so several values may be
 * null for them.
 */
export function collectMetrics(events, timelineItems, runId) {
  const byType = (type) => events.filter((event) => eventType(event) === type);
  const modelTimings = byType("model_timing").map((event) => event.data ?? {});
  const toolTimings = byType("tool_timing").map((event) => event.data ?? {});
  const requests = events.filter((event) => REQUEST_EVENTS.has(eventType(event)));
  const responses = byType("model_response_committed");
  const toolResults = byType("tool_result_committed");

  // Per step, prefer the explicit model_timing record and fall back to the
  // request/response timestamps and repeated request events.
  const stepKeys = new Set();
  const requestCounts = new Map();
  for (const event of requests) {
    const step = stepOf(event) ?? `seq-${event.seq}`;
    stepKeys.add(step);
    requestCounts.set(step, (requestCounts.get(step) ?? 0) + 1);
  }
  const timingByStep = new Map(modelTimings.map((timing) => [timing.step, timing]));
  for (const step of timingByStep.keys()) stepKeys.add(step);
  let retries = 0;
  const latencies = [];
  for (const step of stepKeys) {
    const timing = timingByStep.get(step);
    if (timing) {
      retries += Math.max(0, (timing.attempts ?? 1) - 1);
      latencies.push(timing.duration_ms);
      continue;
    }
    retries += Math.max(0, (requestCounts.get(step) ?? 1) - 1);
    const response = responses.find((event) => stepOf(event) === step);
    const request = response && requests.filter((event) => stepOf(event) === step && event.seq < response.seq).at(-1);
    if (request) latencies.push(response.created_at_ms - request.created_at_ms);
  }
  const steps = stepKeys.size;
  const firstTokens = modelTimings
    .map((timing) => {
      if (!Number.isFinite(timing.first_token_ms)) return null;
      // Accept either an absolute timestamp or an offset from started_at_ms.
      return timing.first_token_ms > 1e12 && Number.isFinite(timing.started_at_ms)
        ? timing.first_token_ms - timing.started_at_ms
        : timing.first_token_ms;
    })
    .filter((value) => value !== null);

  const tools = {};
  const recordTool = (name, isError) => {
    const entry = (tools[name ?? "unknown"] ??= { calls: 0, errors: 0 });
    entry.calls += 1;
    if (isError) entry.errors += 1;
  };
  if (toolTimings.length > 0) {
    for (const timing of toolTimings) recordTool(timing.name, timing.is_error === true);
  } else if (toolResults.length > 0) {
    for (const event of toolResults) recordTool(event.data?.name, toolResultIsError(event.data));
  } else {
    // Harness-boundary runs: fall back to projected timeline tool results.
    for (const item of timelineItems.filter((entry) => entry.run_id === runId && eventType(entry) === "tool_result")) {
      recordTool(item.data?.name, toolResultIsError(item.data));
    }
  }

  const usage = { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 };
  let cost = null;
  for (const response of responses) {
    const value = response.data?.usage ?? {};
    usage.prompt_tokens += value.prompt_tokens ?? 0;
    usage.completion_tokens += value.completion_tokens ?? 0;
    usage.total_tokens += value.total_tokens ?? (value.prompt_tokens ?? 0) + (value.completion_tokens ?? 0);
    if (Number.isFinite(value.cost_usd)) cost = (cost ?? 0) + value.cost_usd;
  }
  const message = timelineItems
    .filter((item) => item.run_id === runId && item.data?.role === "assistant" && item.data?.metrics)
    .at(-1)?.data?.metrics;
  if (responses.length === 0 && message?.usage) {
    usage.prompt_tokens = message.usage.prompt_tokens ?? 0;
    usage.completion_tokens = message.usage.completion_tokens ?? 0;
    usage.total_tokens = message.usage.total_tokens ?? usage.prompt_tokens + usage.completion_tokens;
  }
  if (cost === null && Number.isFinite(message?.costUsd)) cost = message.costUsd;

  const approvalsRequested = byType("tool_approval_required");
  const approvalWaits = approvalsRequested
    .map((event) => {
      const resolved = byType("tool_approval_resolved").find(
        (candidate) => candidate.data?.approval_id === event.data?.approval_id,
      );
      return resolved ? resolved.created_at_ms - event.created_at_ms : null;
    })
    .filter((value) => value !== null);

  const toolCalls = Object.values(tools).reduce((sum, entry) => sum + entry.calls, 0);
  const toolErrors = Object.values(tools).reduce((sum, entry) => sum + entry.errors, 0);
  return {
    ledger_events: events.length,
    steps,
    retries,
    tool_calls: toolCalls,
    tool_errors: toolErrors,
    tools,
    ...usage,
    cost_usd: cost,
    model_latency_p50_ms: percentile(latencies, 0.5),
    model_latency_p95_ms: percentile(latencies, 0.95),
    first_token_p50_ms: percentile(firstTokens, 0.5),
    approvals_requested: approvalsRequested.length,
    approval_wait_p50_ms: percentile(approvalWaits, 0.5),
    run_errors: byType("run_error_committed").map((event) => event.data?.message ?? event.data?.code ?? "error"),
  };
}

// ----- one task -----

async function waitForRun(client, threadId, runId, deadline, counters) {
  let lastApprovalCheck = 0;
  let resolvedRunId = runId;
  for (;;) {
    if (Date.now() > deadline) return { timedOut: true, runId: resolvedRunId };
    if (Date.now() - lastApprovalCheck > 4_000) {
      lastApprovalCheck = Date.now();
      const bootstrap = await client.bootstrap();
      if (!resolvedRunId) {
        resolvedRunId = bootstrap.active_runs?.find((run) => run.thread_id === threadId)?.id;
      }
      for (const approval of bootstrap.pending_approvals ?? []) {
        if (approval.thread_id !== threadId || approval.status !== "pending") continue;
        // Open mode should not ask; if the runtime still does, approve so the
        // eval measures the agent instead of stalling, and count it.
        await client.command("approval.resolve", threadId, { approval_id: approval.id, decision: "approve" });
        counters.autoApproved += 1;
      }
    }
    if (resolvedRunId) {
      const inspection = await client.run(resolvedRunId);
      const status = inspection?.run?.status;
      if (TERMINAL.has(status)) return { timedOut: false, runId: resolvedRunId, inspection };
    }
    await sleep(2_000);
  }
}

async function runTask(client, task, config) {
  const started = Date.now();
  const record = {
    id: task.id,
    title: task.title,
    category: task.category,
    status: "error",
    thread_id: null,
    run_id: null,
    run_status: null,
    wall_ms: 0,
    run_ms: null,
    check: null,
    metrics: null,
    auto_approved: 0,
    error: null,
  };
  let repo;
  try {
    repo = prepareRepo(task);
    const settings = {
      folder: repo,
      model: config.model,
      toolApproval: "open",
      privacy: "off",
      memory: false,
    };
    if (config.reasoningEffort) {
      settings.reasoningEffortOverrides = { [config.model]: config.reasoningEffort };
    }
    const created = await client.command("thread.create", null, {
      title: `[eval] ${task.id}`,
      settings,
    });
    record.thread_id = created.thread_id;
    const sent = await client.command("turn.send", record.thread_id, { text: task.prompt, attachments: [] });
    const deadline = started + task.timeout_s * config.timeoutScale * 1000;
    const counters = { autoApproved: 0 };
    const waited = await waitForRun(client, record.thread_id, sent.run_id, deadline, counters);
    record.auto_approved = counters.autoApproved;
    record.run_id = waited.runId ?? null;
    if (waited.timedOut) {
      await client.command("turn.stop", record.thread_id, {}).catch(() => {});
      record.run_status = "timeout";
    } else {
      record.run_status = waited.inspection.run.status;
      const run = waited.inspection.run;
      record.run_ms = run.completed_at_ms ? run.completed_at_ms - run.created_at_ms : null;
      record.run_error = run.error ?? null;
    }
    if (record.run_id) {
      const [events, timeline] = await Promise.all([
        client.runEvents(record.run_id),
        client.timeline(record.thread_id).catch(() => []),
      ]);
      record.metrics = collectMetrics(events, timeline, record.run_id);
    }
    record.check = runCheck(task, repo);
    record.status = record.run_status === "timeout" ? "timeout" : record.check.exit_code === 0 ? "pass" : "fail";
    if (config.archive) {
      await client.command("thread.archive", record.thread_id, { archived: true }).catch(() => {});
    }
  } catch (error) {
    record.error = error.message;
    if (repo && !record.check) record.check = runCheck(task, repo);
  } finally {
    record.wall_ms = Date.now() - started;
    if (repo && !config.keep) rmSync(repo, { recursive: true, force: true });
    else if (repo) record.repo = repo;
  }
  return record;
}

// ----- reporting -----

function aggregate(tasks) {
  const withMetrics = tasks.filter((task) => task.metrics);
  const mean = (pick) =>
    withMetrics.length === 0 ? null : withMetrics.reduce((sum, task) => sum + (pick(task) ?? 0), 0) / withMetrics.length;
  const sum = (pick) => withMetrics.reduce((total, task) => total + (pick(task) ?? 0), 0);
  const costs = withMetrics.map((task) => task.metrics.cost_usd).filter(Number.isFinite);
  const passed = tasks.filter((task) => task.status === "pass").length;
  return {
    total: tasks.length,
    passed,
    failed: tasks.filter((task) => task.status === "fail").length,
    timeouts: tasks.filter((task) => task.status === "timeout").length,
    errors: tasks.filter((task) => task.status === "error").length,
    pass_rate: tasks.length ? passed / tasks.length : 0,
    mean_steps: mean((task) => task.metrics.steps),
    mean_tool_calls: mean((task) => task.metrics.tool_calls),
    total_tool_errors: sum((task) => task.metrics.tool_errors),
    total_tokens: sum((task) => task.metrics.total_tokens),
    total_cost_usd: costs.length ? costs.reduce((a, b) => a + b, 0) : null,
    mean_wall_ms: tasks.length ? tasks.reduce((total, task) => total + task.wall_ms, 0) / tasks.length : null,
  };
}

const fmt = (value, digits = 0) =>
  value === null || value === undefined || Number.isNaN(value) ? "-" : Number(value).toFixed(digits);

export function markdownTable(result) {
  const rows = [
    "| Task | Category | Result | Steps | Tools | Tool errors | Tokens | Cost | Wall s |",
    "|---|---|---|---:|---:|---:|---:|---:|---:|",
  ];
  for (const task of result.tasks) {
    const metrics = task.metrics ?? {};
    rows.push(
      `| ${task.id} | ${task.category} | ${task.status.toUpperCase()} | ${fmt(metrics.steps)} | ${fmt(metrics.tool_calls)} | ${fmt(metrics.tool_errors)} | ${fmt(metrics.total_tokens)} | ${metrics.cost_usd == null ? "-" : `$${fmt(metrics.cost_usd, 4)}`} | ${fmt(task.wall_ms / 1000, 1)} |`,
    );
  }
  const summary = result.summary;
  rows.push(
    "",
    `**${summary.passed}/${summary.total} passed (${fmt(summary.pass_rate * 100, 1)}%)**, ${summary.timeouts} timeout(s), ${summary.errors} error(s). ` +
      `Mean steps ${fmt(summary.mean_steps, 1)}, mean tool calls ${fmt(summary.mean_tool_calls, 1)}, tool errors ${summary.total_tool_errors}, ` +
      `tokens ${summary.total_tokens}, cost ${summary.total_cost_usd == null ? "-" : `$${fmt(summary.total_cost_usd, 4)}`}.`,
  );
  return rows.join("\n");
}

// ----- main -----

async function main() {
  const selfTest = flag("--self-test");
  const tasks = loadTasks(option("--tasks") ?? process.env.MILIM_EVAL_TASKS);
  if (flag("--dry-run")) {
    for (const task of tasks) console.log(`${task.id.padEnd(28)} ${task.category.padEnd(22)} ${task.timeout_s}s  ${task.title}`);
    console.log(`\n${tasks.length} task(s) selected.`);
    return;
  }
  const url = process.env.MILIM_CONTROL_URL;
  const key = process.env.MILIM_DEVICE_KEY;
  const model = selfTest ? "mock-echo" : process.env.MILIM_EVAL_MODEL;
  if (!url || !key || !model) {
    console.error("MILIM_CONTROL_URL, MILIM_DEVICE_KEY, and MILIM_EVAL_MODEL are required (see evals/README.md).");
    process.exit(2);
  }
  const client = new ControlClient(url, key);
  const bootstrap = await client.bootstrap();
  const knownModels = (bootstrap.models ?? []).map((entry) => (typeof entry === "string" ? entry : entry?.id)).filter(Boolean);
  if (!selfTest && knownModels.length > 0 && !knownModels.includes(model)) {
    console.error(`warning: ${model} is not in the desktop model catalog; the run may fail.`);
  }

  if (selfTest) {
    const task = { ...loadTasks("bugfix-paginate")[0], id: "self-test", title: "Control API plumbing", category: "self-test", timeout_s: 60 };
    const record = await runTask(client, task, { model, timeoutScale: 1, keep: false, archive: true });
    const ok = record.run_status === "completed" && record.metrics !== null && !record.error;
    console.log(`self-test: thread ${record.thread_id}, run ${record.run_id}, status ${record.run_status}, ledger events ${record.metrics?.ledger_events ?? 0}`);
    console.log(ok ? "self-test passed" : `self-test failed${record.error ? `: ${record.error}` : ""}`);
    process.exit(ok ? 0 : 1);
  }

  const config = {
    model,
    reasoningEffort: process.env.MILIM_EVAL_REASONING_EFFORT || null,
    timeoutScale: Number(process.env.MILIM_EVAL_TIMEOUT_SCALE) > 0 ? Number(process.env.MILIM_EVAL_TIMEOUT_SCALE) : 1,
    keep: process.env.MILIM_EVAL_KEEP === "1",
    archive: process.env.MILIM_EVAL_ARCHIVE === "1",
  };
  const startedAt = new Date();
  const records = [];
  for (const task of tasks) {
    process.stderr.write(`[${records.length + 1}/${tasks.length}] ${task.id} ... `);
    const record = await runTask(client, task, config);
    process.stderr.write(`${record.status}${record.error ? ` (${record.error})` : ""} in ${fmt(record.wall_ms / 1000, 1)}s\n`);
    records.push(record);
  }
  const result = {
    schema_version: 1,
    started_at: startedAt.toISOString(),
    finished_at: new Date().toISOString(),
    model,
    reasoning_effort: config.reasoningEffort,
    host: { host_id: bootstrap.host_id, host_name: bootstrap.host_name, protocol: bootstrap.protocol },
    node: process.version,
    platform: `${process.platform}-${process.arch}`,
    tasks: records,
    summary: aggregate(records),
  };
  const out = option("--out") ? resolve(option("--out")) : join(RESULTS_DIR, `${startedAt.toISOString().replace(/[:.]/g, "-")}.json`);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, `${JSON.stringify(result, null, 2)}\n`);
  console.log(markdownTable(result));
  console.log(`\nResults: ${out}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
