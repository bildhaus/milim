import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = fileURLToPath(new URL("..", import.meta.url));
const repoRoot = fileURLToPath(new URL("../../..", import.meta.url));
const artifactDir = resolve(
  process.env.MILIM_CONFORMANCE_ARTIFACT_DIR
    || join(desktopRoot, "tester-artifacts", "runtime-conformance"),
);
const artifactPath = join(artifactDir, "runtime-conformance.json");
const node = process.execPath;
const checks = [
  cargoCheck("providers.protocols", ["test", "-p", "milim-inference"]),
  cargoCheck("providers.routing", ["test", "-p", "milim-server", "providers::tests"]),
  cargoCheck("runtimes.codex.adapter", ["test", "-p", "milim-server", "codex_bridge::tests"]),
  cargoCheck("runtimes.claude.adapter", ["test", "-p", "milim-server", "claude_bridge::tests"]),
  cargoCheck("runtimes.opencode.adapter", ["test", "-p", "milim-server", "opencode_bridge::tests"]),
  cargoCheck("runtimes.pi.adapter", ["test", "-p", "milim-server", "pi_bridge::tests"]),
  cargoCheck("runtimes.harness-event-envelope", [
    "test",
    "-p",
    "milim-server",
    "routes::harnesses::tests",
  ]),
  cargoCheck("runtimes.approval-lifecycle", ["test", "-p", "milim-agents", "approval"]),
  desktopCheck("desktop.runtime-selection", ["tests/turn-runtime.test.ts"]),
  desktopCheck("desktop.stream", ["tests/turn-stream.test.ts"]),
  desktopCheck("desktop.hot-swap", ["tests/hot-swap.test.ts"]),
  desktopCheck("desktop.queue", ["tests/turn-queue.test.ts"]),
  desktopCheck("desktop.persistence", ["tests/session-store.test.ts"]),
  desktopCheck("desktop.approval-events", ["tests/turn-events.test.ts"]),
  nodeCheck("desktop.model-catalog-discovery", ["tests/api-model-discovery.test.mjs"]),
  nodeCheck("desktop.model-catalog-startup", ["tests/api-model-startup.test.mjs"]),
];

const results = [];
for (const check of checks) {
  console.log(`\n[conformance] ${check.id}\n$ ${check.command}`);
  const startedAt = performance.now();
  const child = spawnSync(check.executable, check.args, {
    cwd: check.cwd,
    stdio: "inherit",
    shell: check.shell,
  });
  const exitCode = child.status ?? 1;
  if (child.error) console.error(child.error);
  results.push({
    id: check.id,
    command: check.command,
    status: exitCode === 0 ? "passed" : "failed",
    duration_ms: Math.round(performance.now() - startedAt),
    exit_code: exitCode,
  });
}

const passed = results.filter((check) => check.status === "passed").length;
const report = {
  schema_version: 1,
  generated_at: new Date().toISOString(),
  commit_sha: commitSha(),
  platform: process.platform,
  arch: process.arch,
  proof: "deterministic_mocked",
  checks: results,
  summary: {
    total: results.length,
    passed,
    failed: results.length - passed,
  },
};

mkdirSync(artifactDir, { recursive: true });
writeFileSync(artifactPath, `${JSON.stringify(report, null, 2)}\n`);
console.log(`\nreport=${artifactPath}`);
process.exitCode = report.summary.failed === 0 ? 0 : 1;

function cargoCheck(id, args) {
  return {
    id,
    executable: "cargo",
    args,
    cwd: repoRoot,
    shell: false,
    command: `cargo ${args.join(" ")}`,
  };
}

function desktopCheck(id, tests) {
  return nodeCheck(id, ["tests/run-ts-test.mjs", ...tests]);
}

function nodeCheck(id, args) {
  return {
    id,
    executable: node,
    args,
    cwd: desktopRoot,
    shell: false,
    command: `node ${args.join(" ")}`,
  };
}

function commitSha() {
  const result = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: repoRoot,
    encoding: "utf8",
    shell: false,
  });
  return result.status === 0 ? result.stdout.trim() : "unknown";
}
