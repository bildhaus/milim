import { spawnSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const runner = join(root, "tests", "tauri-dev-perf.mjs");
const evidenceRoot =
  process.env.MILIM_PERF_ARTIFACT_DIR ||
  join(root, "tester-artifacts", "runtime-evidence");
const suiteRoot = join(evidenceRoot, "canonical-suite");
const runsArgument = process.argv.find((argument) => argument.startsWith("--runs="));
const runCount = Number(runsArgument?.slice("--runs=".length) || 7);
const enforce = process.argv.includes("--enforce");

if (!Number.isInteger(runCount) || runCount < 1 || runCount > 25) {
  throw new Error("--runs must be an integer between 1 and 25");
}

rmSync(suiteRoot, { recursive: true, force: true });
mkdirSync(suiteRoot, { recursive: true });

const reports = [];
for (let index = 0; index < runCount; index += 1) {
  const runNumber = index + 1;
  const runRoot = join(suiteRoot, `run-${String(runNumber).padStart(2, "0")}`);
  console.log(`[suite] canonical-thread release process ${runNumber}/${runCount}`);
  const result = spawnSync(process.execPath, [runner, "--binary"], {
    cwd: root,
    env: {
      ...process.env,
      MILIM_PERF_ARTIFACT_DIR: runRoot,
      MILIM_TAURI_PERF_CDP_PORT: String(9555 + index),
    },
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`canonical benchmark process ${runNumber} failed with exit ${result.status}`);
  }
  reports.push(
    JSON.parse(readFileSync(join(runRoot, "canonical-thread.json"), "utf8")),
  );
}

const metrics = {
  process_to_chat_shell_ms: {
    read: (report) => report.timingsMs.processToChatShell,
    limit: 2_000,
  },
  fixture_reload_to_interactive_ms: {
    read: (report) => report.timingsMs.fixtureReloadToInteractive,
    limit: 250,
  },
  input_to_next_frame_ms: {
    read: (report) => report.timingsMs.inputToNextFrame,
    limit: 16.7,
  },
  optimistic_send_ms: {
    read: (report) => report.timingsMs.longThreadSendToOptimistic,
    limit: 50,
  },
  streaming_frame_p95_ms: {
    read: (report) => report.renderer.largeTranscript.frames.p95Ms,
    limit: 20,
  },
  streaming_frame_max_ms: {
    read: (report) => report.renderer.largeTranscript.frames.maxMs,
    limit: 50,
  },
  mounted_message_rows: {
    read: (report) => report.fixture.renderedMessageRows,
    limit: 200,
  },
  initial_javascript_bytes: {
    read: (report) => report.bundles.initialJavaScriptBytes,
    limit: 850_000,
  },
  initial_css_bytes: {
    read: (report) => report.bundles.initialCssBytes,
    limit: 250_000,
  },
};

function percentile(values, fraction) {
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.max(0, Math.ceil(sorted.length * fraction) - 1);
  return sorted[index];
}

const aggregate = Object.fromEntries(
  Object.entries(metrics).map(([name, definition]) => {
    const samples = reports.map(definition.read);
    if (!samples.every(Number.isFinite)) {
      throw new Error(`metric ${name} contains a non-finite sample`);
    }
    const median = percentile(samples, 0.5);
    const p95 = percentile(samples, 0.95);
    return [
      name,
      {
        samples,
        median,
        p95,
        limit: definition.limit,
        passed: p95 <= definition.limit,
      },
    ];
  }),
);

const suite = {
  schema_version: 2,
  generated_at: new Date().toISOString(),
  scenario: "canonical-thread",
  fixture_version: reports[0]?.fixture?.version ?? null,
  build_profile: "release",
  run_count: runCount,
  gate_mode: enforce ? "enforced" : "reported",
  scope_note:
    "Canonical durability and persisted-transcript scenario only; scale, long-idle, migration/compaction, backup/restore, theme-compositor, and macOS parity scenarios require separate promotion.",
  aggregate,
  runs: reports.map((report, index) => ({
    run: index + 1,
    report: `run-${String(index + 1).padStart(2, "0")}/canonical-thread.json`,
    generated_at: report.generated_at,
    fingerprint: report.fingerprint,
  })),
};
const suitePath = join(suiteRoot, "performance-suite-v2.json");
writeFileSync(suitePath, JSON.stringify(suite, null, 2));
console.log(`performanceSuite=${suitePath}`);
for (const [name, result] of Object.entries(aggregate)) {
  console.log(
    `${result.passed ? "PASS" : "MISS"} ${name}: median=${result.median} p95=${result.p95} limit=${result.limit}`,
  );
}

if (enforce) {
  const failures = Object.entries(aggregate).filter(([, result]) => !result.passed);
  if (failures.length) {
    throw new Error(
      `performance gates missed: ${failures.map(([name]) => name).join(", ")}`,
    );
  }
}
