#!/usr/bin/env node
// Agent-quality eval runner. Drives a running milim desktop app through the
// canonical /control/v1 API: one fresh thread per task sample, bound to a
// temporary git copy of the task fixture, then a deterministic check script
// decides pass or fail. See evals/README.md.
//
//   MILIM_CONTROL_URL=... MILIM_DEVICE_KEY=... MILIM_EVAL_MODEL=... node evals/run.mjs
//   MILIM_E2E_CONTROL_FILE=... MILIM_EVAL_MODEL=... node evals/run.mjs   # debug build, headless
//   node evals/run.mjs --dry-run      # list selected tasks, no app needed
//   node evals/run.mjs --self-test    # plumbing check with the mock-echo model

import { randomUUID } from "node:crypto";
import { appendFileSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { resolveControlCredentials, retryUntilReady } from "./lib/control.mjs";
import { EVALS, loadTasks, missingRequirement, prepareRepo, runCheck, taskTimeout } from "./lib/tasks.mjs";

const RESULTS_DIR = join(EVALS, "results");
const SCHEMA_VERSION = 2;
const TERMINAL = new Set(["completed", "failed", "cancelled", "interrupted"]);
const REQUEST_EVENTS = new Set(["model_request_resolved", "harness_request_committed"]);
/** How long a stopped run may take to reach a terminal status before the check runs anyway. */
const STOP_GRACE_MS = 30_000;
/** Tool results the agent loop produced for a call it refused to run. */
const TOOL_ARGUMENT_REJECTION = /^(Invalid arguments for `|Tool call arguments were not valid JSON|Unknown tool `)/;

const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

if (flag("--help") || flag("-h")) {
  console.log(`usage: node evals/run.mjs [--dry-run] [--self-test] [--tasks a,b] [--repeat N] [--out file.json] [--fail-on-error]

env:
  MILIM_CONTROL_URL   Mobile/control URL shown under Settings > Mobile, or the desktop API URL
  MILIM_DEVICE_KEY    Paired device bearer key (never logged), or
  MILIM_API_TOKEN     desktop API bearer token
  MILIM_E2E_CONTROL_FILE  Instead of the three above: the {api_url, token} file a debug
                      desktop build writes when launched with the same variable
  MILIM_EVAL_MODEL    Thread model id, e.g. provider:openrouter:... or codex:gpt-5 (required)
  MILIM_EVAL_TASKS    Comma list of task ids or substrings to run (optional)
  MILIM_EVAL_REPEAT   Samples per task (optional, default 1; same as --repeat)
  MILIM_EVAL_REASONING_EFFORT  Per-thread reasoning effort override (optional)
  MILIM_EVAL_TIMEOUT_SCALE     Multiply every task timeout (optional, default 1)
  MILIM_EVAL_CONNECT_TIMEOUT_MS  How long to wait for the app to accept requests (default 60000)
  MILIM_EVAL_KEEP=1   Keep temporary task repositories
  MILIM_EVAL_ARCHIVE=1  Archive each eval thread after it finishes`);
  process.exit(0);
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

// ----- control client -----

class ControlClient {
  constructor(baseUrl, credential) {
    this.base = baseUrl.replace(/\/+$/, "");
    this.key = credential;
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
            ...(this.key ? { Authorization: `Bearer ${this.key}` } : {}),
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
 * Metrics from the run ledger of one or more runs (a multi-turn task has one
 * run per turn), preferring explicit model_timing/tool_timing events and
 * falling back to event timestamps and timeline projections. Account
 * runtimes record only a harness boundary, so several values may be null
 * for them.
 */
export function collectMetrics(events, timelineItems, runIds) {
  const runs = new Set([runIds].flat().filter(Boolean));
  const items = timelineItems.filter((item) => runs.has(item.run_id));
  const itemsOf = (type) => items.filter((item) => eventType(item) === type);
  const byType = (type) => events.filter((event) => eventType(event) === type);
  // Step numbers restart in every run, so key steps by run as well.
  const stepKey = (event, step) => `${event.run_id ?? ""}:${step}`;
  const modelTimings = byType("model_timing");
  const toolTimings = byType("tool_timing").map((event) => event.data ?? {});
  const requests = events.filter((event) => REQUEST_EVENTS.has(eventType(event)));
  const responses = byType("model_response_committed");
  const toolResults = byType("tool_result_committed");

  // Per step, prefer the explicit model_timing record and fall back to the
  // request/response timestamps and repeated request events.
  const stepKeys = new Set();
  const requestCounts = new Map();
  const lastStep = new Map();
  for (const event of requests) {
    const step = stepOf(event);
    const key = step === null ? `seq-${event.seq}` : stepKey(event, step);
    stepKeys.add(key);
    requestCounts.set(key, (requestCounts.get(key) ?? 0) + 1);
    if (step !== null) lastStep.set(event.run_id ?? "", Math.max(lastStep.get(event.run_id ?? "") ?? 0, step));
  }
  const timingByStep = new Map(
    modelTimings.map((event) => [stepKey(event, event.data?.step ?? stepOf(event)), event.data ?? {}]),
  );
  for (const key of timingByStep.keys()) stepKeys.add(key);
  let retries = 0;
  const latencies = [];
  for (const key of stepKeys) {
    const timing = timingByStep.get(key);
    if (timing) {
      retries += Math.max(0, (timing.attempts ?? 1) - 1);
      latencies.push(timing.duration_ms);
      continue;
    }
    retries += Math.max(0, (requestCounts.get(key) ?? 1) - 1);
    const response = responses.find((event) => stepKey(event, stepOf(event)) === key);
    const request =
      response && requests.filter((event) => stepKey(event, stepOf(event)) === key && event.seq < response.seq).at(-1);
    if (request) latencies.push(response.created_at_ms - request.created_at_ms);
  }
  const steps = stepKeys.size;
  const firstTokens = modelTimings
    .map((event) => {
      const timing = event.data ?? {};
      if (!Number.isFinite(timing.first_token_ms)) return null;
      // Accept either an absolute timestamp or an offset from started_at_ms.
      return timing.first_token_ms > 1e12 && Number.isFinite(timing.started_at_ms)
        ? timing.first_token_ms - timing.started_at_ms
        : timing.first_token_ms;
    })
    .filter((value) => value !== null);

  // A step that ended at the output-token limit and was not the run's last
  // step was continued by the loop's length recovery.
  const finishes = new Map();
  for (const event of [...responses, ...modelTimings]) {
    const step = stepOf(event);
    if (step !== null && event.data?.finish_reason) finishes.set(stepKey(event, step), { event, step, reason: event.data.finish_reason });
  }
  const lengthFinishes = [...finishes.values()].filter((entry) => entry.reason === "length");
  const outputLimitContinuations = lengthFinishes.filter(
    (entry) => entry.step < (lastStep.get(entry.event.run_id ?? "") ?? entry.step),
  ).length;

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
    for (const item of itemsOf("tool_result")) recordTool(item.data?.name, toolResultIsError(item.data));
  }
  // Calls the loop refused before running (unknown tool, invalid JSON, or
  // arguments that fail the tool's schema). Only the timeline keeps the text.
  const toolArgumentRejections = itemsOf("tool_result").filter((item) =>
    TOOL_ARGUMENT_REJECTION.test(String(item.data?.result?.error ?? "")),
  ).length;

  const retriesByReason = {};
  for (const item of itemsOf("provider_retry")) {
    const reason = item.data?.reason ?? "unknown";
    retriesByReason[reason] = (retriesByReason[reason] ?? 0) + 1;
  }
  retries = Math.max(retries, itemsOf("provider_retry").length);

  const compactions = byType("context_compacted").map((event) => event.data ?? {});
  if (compactions.length === 0) compactions.push(...itemsOf("context_compacted").map((item) => item.data ?? {}));

  const usage = { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0, cached_input_tokens: 0, cache_write_tokens: 0 };
  const addUsage = (value) => {
    usage.prompt_tokens += value.prompt_tokens ?? 0;
    usage.completion_tokens += value.completion_tokens ?? 0;
    usage.total_tokens += value.total_tokens ?? (value.prompt_tokens ?? 0) + (value.completion_tokens ?? 0);
    usage.cached_input_tokens += value.cache_read_tokens ?? 0;
    usage.cache_write_tokens += value.cache_write_tokens ?? 0;
  };
  let cost = null;
  for (const response of responses) {
    const value = response.data?.usage ?? {};
    addUsage(value);
    if (Number.isFinite(value.cost_usd)) cost = (cost ?? 0) + value.cost_usd;
  }
  // The last assistant message of each run carries the run's summed metrics.
  const messages = [...runs]
    .map((runId) =>
      items.filter((item) => item.run_id === runId && item.data?.role === "assistant" && item.data?.metrics).at(-1)?.data?.metrics,
    )
    .filter(Boolean);
  if (responses.length === 0) {
    for (const message of messages) if (message.usage) addUsage(message.usage);
  }
  if (cost === null && messages.some((message) => Number.isFinite(message.costUsd))) {
    cost = messages.reduce((total, message) => total + (Number.isFinite(message.costUsd) ? message.costUsd : 0), 0);
  }

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
    retries_by_reason: retriesByReason,
    tool_calls: toolCalls,
    tool_errors: toolErrors,
    tool_argument_rejections: toolArgumentRejections,
    tools,
    ...usage,
    cost_usd: cost,
    context_compactions: compactions.length,
    context_elided_tool_results: compactions.reduce((sum, entry) => sum + (entry.elided_tool_results ?? 0), 0),
    context_summarized_messages: compactions.reduce((sum, entry) => sum + (entry.summarized_messages ?? 0), 0),
    output_limit_finishes: lengthFinishes.length,
    output_limit_continuations: outputLimitContinuations,
    model_latency_p50_ms: percentile(latencies, 0.5),
    model_latency_p95_ms: percentile(latencies, 0.95),
    first_token_p50_ms: percentile(firstTokens, 0.5),
    approvals_requested: approvalsRequested.length,
    approval_wait_p50_ms: percentile(approvalWaits, 0.5),
    run_errors: byType("run_error_committed").map((event) => event.data?.message ?? event.data?.code ?? "error"),
  };
}

// ----- one task sample -----

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

/**
 * Stop a timed-out turn and wait (bounded) for its run to reach a terminal
 * status, so the check does not race an agent that is still editing files.
 */
async function stopRun(client, threadId, runId) {
  await client.command("turn.stop", threadId, {}).catch(() => {});
  const deadline = Date.now() + STOP_GRACE_MS;
  let status = null;
  for (;;) {
    if (runId) {
      const inspection = await client.run(runId).catch(() => null);
      status = inspection?.run?.status ?? status;
      if (TERMINAL.has(status)) return { status, confirmed: true, inspection };
    } else {
      const bootstrap = await client.bootstrap().catch(() => null);
      if (bootstrap && !bootstrap.active_runs?.some((run) => run.thread_id === threadId)) {
        return { status: null, confirmed: true };
      }
    }
    if (Date.now() > deadline) return { status, confirmed: false };
    await sleep(1_000);
  }
}

/** Overall status of one sample from its turns and its check. */
function classify(record, turnCount) {
  const turns = record.turns;
  const timedOut = turns.some((turn) => turn.timed_out);
  const flags = [];
  for (const turn of turns) {
    if (turn.timed_out && !turn.stop_confirmed) flags.push(`turn ${turn.turn}: run still ${turn.run_status ?? "unknown"} after stop`);
    else if (!turn.timed_out && turn.run_status !== "completed") flags.push(`turn ${turn.turn}: run ${turn.run_status}`);
    if (turn.limited) flags.push(`turn ${turn.turn}: run stopped at its step or time limit`);
  }
  if (turns.length < turnCount) flags.push(`only ${turns.length} of ${turnCount} turn(s) ran`);
  const runsFinished = turns.length === turnCount && turns.every((turn) => turn.run_status === "completed" && !turn.limited);
  let status;
  if (record.check?.skipped) status = "skipped";
  else if (timedOut) status = "timeout";
  else if (!record.check_passed) status = "fail";
  // The workspace passes the check, but the agent did not finish cleanly.
  else if (!runsFinished) status = "unfinished";
  else status = "pass";
  return { status, flags };
}

/** An empty sample record; `status` stays `error` unless the sample completes. */
function newRecord(task, sample) {
  return {
    id: task.id,
    title: task.title,
    category: task.category,
    sample,
    status: "error",
    check_passed: null,
    thread_id: null,
    run_id: null,
    run_status: null,
    run_limited: false,
    turns: [],
    wall_ms: 0,
    run_ms: null,
    check: null,
    metrics: null,
    auto_approved: 0,
    flags: [],
    error: null,
  };
}

async function runTask(client, task, config, sample = 1) {
  const started = Date.now();
  const record = newRecord(task, sample);
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
      title: `[eval] ${task.id}${config.repeat > 1 ? ` #${sample}` : ""}`,
      settings,
    });
    record.thread_id = created.thread_id;
    const counters = { autoApproved: 0 };
    // Turns go to the same thread in order; a turn that did not complete
    // ends the sample because later turns build on its work.
    for (const [index, turn] of task.turns.entries()) {
      const turnStarted = Date.now();
      const entry = { turn: index + 1, run_id: null, run_status: null, timed_out: false, limited: false, run_ms: null, error: null };
      record.turns.push(entry);
      const sent = await client.command("turn.send", record.thread_id, { text: turn.prompt, attachments: [] });
      const deadline = turnStarted + turn.timeout_s * config.timeoutScale * 1000;
      const waited = await waitForRun(client, record.thread_id, sent.run_id, deadline, counters);
      entry.run_id = waited.runId ?? null;
      let inspection = waited.inspection;
      if (waited.timedOut) {
        entry.timed_out = true;
        const stopped = await stopRun(client, record.thread_id, entry.run_id);
        entry.stop_confirmed = stopped.confirmed;
        entry.run_status = stopped.status;
        inspection = stopped.inspection;
      } else {
        entry.run_status = inspection.run.status;
      }
      const run = inspection?.run;
      if (run) {
        entry.run_ms = run.completed_at_ms ? run.completed_at_ms - run.created_at_ms : null;
        entry.error = run.error ?? null;
      }
      if (entry.timed_out || entry.run_status !== "completed") break;
    }
    record.auto_approved = counters.autoApproved;
    const runIds = record.turns.map((turn) => turn.run_id).filter(Boolean);
    if (runIds.length > 0) {
      const [events, timeline] = await Promise.all([
        Promise.all(runIds.map((runId) => client.runEvents(runId))).then((pages) => pages.flat()),
        client.timeline(record.thread_id).catch(() => []),
      ]);
      // The loop reports hitting its step or time budget only in its final event.
      for (const turn of record.turns) {
        turn.limited = timeline.some(
          (item) => item.run_id === turn.run_id && eventType(item) === "done" && item.data?.stopped_at_limit === true,
        );
      }
      record.metrics = collectMetrics(events, timeline, runIds);
    }
    record.check = runCheck(task, repo);
    record.check_passed = record.check.passed;
    if (config.archive) {
      await client.command("thread.archive", record.thread_id, { archived: true }).catch(() => {});
    }
  } catch (error) {
    record.error = error.message;
    if (repo && !record.check) {
      record.check = runCheck(task, repo);
      record.check_passed = record.check.passed;
    }
  } finally {
    const last = record.turns.at(-1);
    const unfinished = record.turns.find((turn) => turn.run_status !== "completed");
    record.run_id = last?.run_id ?? null;
    record.run_status = (unfinished ?? last)?.run_status ?? null;
    record.run_limited = record.turns.some((turn) => turn.limited);
    const runTimes = record.turns.map((turn) => turn.run_ms).filter(Number.isFinite);
    record.run_ms = runTimes.length ? runTimes.reduce((a, b) => a + b, 0) : null;
    if (!record.error) Object.assign(record, classify(record, task.turns.length));
    record.wall_ms = Date.now() - started;
    if (repo && !config.keep) rmSync(repo, { recursive: true, force: true });
    else if (repo) record.repo = repo;
  }
  return record;
}

function skippedRecord(task, sample, reason) {
  return { ...newRecord(task, sample), status: "skipped", flags: [reason] };
}

// ----- reporting -----

/** C(n, k) as a float; small n only. */
function choose(n, k) {
  if (k < 0 || k > n) return 0;
  let value = 1;
  for (let index = 1; index <= k; index += 1) value = (value * (n - k + index)) / index;
  return value;
}

/**
 * Unbiased pass@k from n samples with c passes: the chance that at least one
 * of k samples drawn without replacement passes (Chen et al., 2021).
 */
export function passAtK(n, c, k) {
  if (n - c < k) return 1;
  return 1 - choose(n - c, k) / choose(n, k);
}

const mean = (values) => {
  const finite = values.filter(Number.isFinite);
  return finite.length ? finite.reduce((a, b) => a + b, 0) / finite.length : null;
};

/** One row per task: pass fraction and pass@k over its samples, mean metrics. */
export function summarizeTasks(records) {
  const groups = new Map();
  for (const record of records) {
    if (!groups.has(record.id)) groups.set(record.id, []);
    groups.get(record.id).push(record);
  }
  return [...groups.entries()].map(([id, samples]) => {
    const graded = samples.filter((sample) => sample.status !== "skipped");
    const passes = graded.filter((sample) => sample.status === "pass").length;
    const statuses = {};
    for (const sample of samples) statuses[sample.status] = (statuses[sample.status] ?? 0) + 1;
    const passAt = {};
    for (let k = 1; k <= graded.length; k += 1) passAt[k] = passAtK(graded.length, passes, k);
    const metric = (pick) => mean(samples.map((sample) => (sample.metrics ? pick(sample.metrics) : null)));
    return {
      id,
      category: samples[0].category,
      samples: graded.length,
      passes,
      pass_fraction: graded.length ? passes / graded.length : null,
      pass_at_k: passAt,
      statuses,
      mean_steps: metric((m) => m.steps),
      mean_tool_calls: metric((m) => m.tool_calls),
      mean_tool_errors: metric((m) => m.tool_errors),
      mean_tokens: metric((m) => m.total_tokens),
      mean_cost_usd: metric((m) => m.cost_usd),
      mean_wall_ms: mean(samples.map((sample) => sample.wall_ms)),
    };
  });
}

function aggregate(records, taskSummary) {
  const graded = records.filter((record) => record.status !== "skipped");
  const withMetrics = graded.filter((record) => record.metrics);
  const sum = (pick) => withMetrics.reduce((total, record) => total + (pick(record.metrics) ?? 0), 0);
  const costs = withMetrics.map((record) => record.metrics.cost_usd).filter(Number.isFinite);
  const count = (status) => records.filter((record) => record.status === status).length;
  const passed = count("pass");
  const gradedTasks = taskSummary.filter((task) => task.samples > 0);
  const repeat = Math.max(0, ...gradedTasks.map((task) => task.samples));
  return {
    total: graded.length,
    tasks: gradedTasks.length,
    repeat,
    passed,
    failed: count("fail"),
    unfinished: count("unfinished"),
    timeouts: count("timeout"),
    errors: count("error"),
    skipped: count("skipped"),
    pass_rate: graded.length ? passed / graded.length : 0,
    mean_pass_at_1: mean(gradedTasks.map((task) => task.pass_at_k[1])),
    mean_pass_at_repeat: mean(gradedTasks.map((task) => task.pass_at_k[task.samples])),
    mean_steps: mean(withMetrics.map((record) => record.metrics.steps)),
    mean_tool_calls: mean(withMetrics.map((record) => record.metrics.tool_calls)),
    total_tool_errors: sum((m) => m.tool_errors),
    total_tool_argument_rejections: sum((m) => m.tool_argument_rejections),
    total_context_compactions: sum((m) => m.context_compactions),
    total_output_limit_continuations: sum((m) => m.output_limit_continuations),
    total_retries: sum((m) => m.retries),
    total_tokens: sum((m) => m.total_tokens),
    total_cached_input_tokens: sum((m) => m.cached_input_tokens),
    total_cost_usd: costs.length ? costs.reduce((a, b) => a + b, 0) : null,
    mean_wall_ms: mean(graded.map((record) => record.wall_ms)),
  };
}

const fmt = (value, digits = 0) =>
  value === null || value === undefined || Number.isNaN(value) ? "-" : Number(value).toFixed(digits);

export function markdownTable(result) {
  const repeat = result.summary.repeat > 1;
  const rows = [
    `| Task | Category | ${repeat ? "Passed | pass@k" : "Result"} | Steps | Tools | Tool errors | Tokens | Cost | Wall s |`,
    `|---|---|${repeat ? "---:|---:" : "---"}|---:|---:|---:|---:|---:|---:|`,
  ];
  for (const task of result.task_summary) {
    const records = result.tasks.filter((record) => record.id === task.id);
    const outcome = repeat
      ? `${task.passes}/${task.samples} | ${fmt(task.pass_at_k[task.samples], 2)}`
      : records[0].status.toUpperCase();
    rows.push(
      `| ${task.id} | ${task.category} | ${outcome} | ${fmt(task.mean_steps, repeat ? 1 : 0)} | ${fmt(task.mean_tool_calls, repeat ? 1 : 0)} | ${fmt(task.mean_tool_errors, repeat ? 1 : 0)} | ${fmt(task.mean_tokens)} | ${task.mean_cost_usd == null ? "-" : `$${fmt(task.mean_cost_usd, 4)}`} | ${fmt(task.mean_wall_ms / 1000, 1)} |`,
    );
  }
  const summary = result.summary;
  const flagged = result.tasks.filter((record) => record.flags?.length && record.status !== "skipped");
  rows.push(
    "",
    `**${summary.passed}/${summary.total} passed (${fmt(summary.pass_rate * 100, 1)}%)**` +
      (repeat ? `, mean pass@${summary.repeat} ${fmt(summary.mean_pass_at_repeat, 2)}` : "") +
      `, ${summary.unfinished} unfinished, ${summary.timeouts} timeout(s), ${summary.errors} error(s), ${summary.skipped} skipped. ` +
      `Mean steps ${fmt(summary.mean_steps, 1)}, mean tool calls ${fmt(summary.mean_tool_calls, 1)}, tool errors ${summary.total_tool_errors}, ` +
      `context compactions ${summary.total_context_compactions}, retries ${summary.total_retries}, ` +
      `tokens ${summary.total_tokens} (${summary.total_cached_input_tokens} cached), cost ${summary.total_cost_usd == null ? "-" : `$${fmt(summary.total_cost_usd, 4)}`}.`,
  );
  if (flagged.length > 0) {
    rows.push("", "Flagged samples:");
    for (const record of flagged) rows.push(`- ${record.id} #${record.sample} (${record.status}): ${record.flags.join("; ")}`);
  }
  return rows.join("\n");
}

/** One Markdown line for a CI job summary. */
export function summaryLine(result, artifact) {
  const summary = result.summary;
  return (
    `Evals with \`${result.model}\`: **${summary.passed}/${summary.total} passed** (${fmt(summary.pass_rate * 100, 1)}%)` +
    `, ${summary.unfinished} unfinished, ${summary.timeouts} timeout(s), ${summary.errors} error(s)` +
    `, ${summary.total_tokens} tokens${summary.total_cost_usd == null ? "" : `, $${fmt(summary.total_cost_usd, 4)}`}` +
    (artifact ? `; results in artifact \`${artifact}\`` : "")
  );
}

// ----- main -----

async function main() {
  const selfTest = flag("--self-test");
  const tasks = loadTasks(option("--tasks") ?? process.env.MILIM_EVAL_TASKS);
  const repeatValue = Number(option("--repeat") ?? process.env.MILIM_EVAL_REPEAT ?? 1);
  if (!Number.isInteger(repeatValue) || repeatValue < 1) {
    console.error("--repeat / MILIM_EVAL_REPEAT must be a positive integer.");
    process.exit(2);
  }
  if (flag("--dry-run")) {
    for (const task of tasks) {
      const turns = task.turns.length > 1 ? ` (${task.turns.length} turns)` : "";
      const missing = missingRequirement(task);
      console.log(
        `${task.id.padEnd(30)} ${task.category.padEnd(22)} ${String(taskTimeout(task)).padStart(4)}s  ${task.title}${turns}${missing ? `  [skipped: ${missing}]` : ""}`,
      );
    }
    console.log(`\n${tasks.length} task(s) selected${repeatValue > 1 ? `, ${repeatValue} samples each` : ""}.`);
    return;
  }
  const connectTimeoutMs = Number(process.env.MILIM_EVAL_CONNECT_TIMEOUT_MS) > 0 ? Number(process.env.MILIM_EVAL_CONNECT_TIMEOUT_MS) : 60_000;
  const { base, credential } = await resolveControlCredentials({ timeoutMs: connectTimeoutMs });
  const model = selfTest ? "mock-echo" : process.env.MILIM_EVAL_MODEL;
  if (!base || !model) {
    console.error(
      "MILIM_EVAL_MODEL and either MILIM_CONTROL_URL (with MILIM_DEVICE_KEY or MILIM_API_TOKEN) or MILIM_E2E_CONTROL_FILE are required (see evals/README.md).",
    );
    process.exit(2);
  }
  const client = new ControlClient(base, credential);
  // A freshly launched app may not accept connections yet.
  const bootstrap = await retryUntilReady(() => client.bootstrap(), {
    timeoutMs: connectTimeoutMs,
    isConnectionError: (error) => error.status === undefined,
  });
  const knownModels = (bootstrap.models ?? []).map((entry) => (typeof entry === "string" ? entry : entry?.id)).filter(Boolean);
  if (!selfTest && knownModels.length > 0 && !knownModels.includes(model)) {
    console.error(`warning: ${model} is not in the desktop model catalog; the run may fail.`);
  }

  if (selfTest) {
    const fixture = loadTasks("bugfix-paginate")[0];
    const task = {
      ...fixture,
      id: "self-test",
      title: "Control API plumbing",
      category: "self-test",
      turns: [{ prompt: fixture.turns[0].prompt, timeout_s: 60 }],
    };
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
    repeat: repeatValue,
  };
  const startedAt = new Date();
  const out = option("--out") ? resolve(option("--out")) : join(RESULTS_DIR, `${startedAt.toISOString().replace(/[:.]/g, "-")}.json`);
  mkdirSync(dirname(out), { recursive: true });
  const records = [];
  const build = () => {
    const taskSummary = summarizeTasks(records);
    return {
      schema_version: SCHEMA_VERSION,
      started_at: startedAt.toISOString(),
      finished_at: new Date().toISOString(),
      model,
      reasoning_effort: config.reasoningEffort,
      repeat: repeatValue,
      host: { host_id: bootstrap.host_id, host_name: bootstrap.host_name, protocol: bootstrap.protocol },
      node: process.version,
      platform: `${process.platform}-${process.arch}`,
      tasks: records,
      task_summary: taskSummary,
      summary: aggregate(records, taskSummary),
    };
  };
  const missing = new Map(tasks.map((task) => [task.id, missingRequirement(task)]));
  const total = tasks.length * repeatValue;
  // Round-robin: finish one pass over every task before starting the next
  // sample, so an interrupted suite still covers each task evenly.
  for (let sample = 1; sample <= repeatValue; sample += 1) {
    for (const task of tasks) {
      const label = `[${records.length + 1}/${total}] ${task.id}${repeatValue > 1 ? ` #${sample}` : ""}`;
      if (missing.get(task.id)) {
        records.push(skippedRecord(task, sample, missing.get(task.id)));
        process.stderr.write(`${label} ... skipped (${missing.get(task.id)})\n`);
        continue;
      }
      process.stderr.write(`${label} ... `);
      const record = await runTask(client, task, config, sample);
      process.stderr.write(`${record.status}${record.error ? ` (${record.error})` : ""} in ${fmt(record.wall_ms / 1000, 1)}s\n`);
      records.push(record);
      // Keep partial results on disk in case the suite is interrupted.
      writeFileSync(out, `${JSON.stringify(build(), null, 2)}\n`);
    }
  }
  const result = build();
  writeFileSync(out, `${JSON.stringify(result, null, 2)}\n`);
  console.log(markdownTable(result));
  console.log(`\nResults: ${out}`);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${summaryLine(result, process.env.MILIM_EVAL_ARTIFACT_NAME)}\n`);
  }
  if (flag("--fail-on-error") && result.summary.errors > 0) {
    console.error(`${result.summary.errors} sample(s) could not be driven (status error).`);
    process.exit(1);
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
