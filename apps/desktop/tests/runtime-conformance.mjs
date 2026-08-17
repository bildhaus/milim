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
  cargoCheck("contract.control-v1-generated", [
    "run",
    "-p",
    "milim-control-contract",
    "--bin",
    "generate-control-contract",
    "--",
    "--check",
  ]),
  cargoCheck("storage.v5-migration", [
    "test",
    "-p",
    "milim-storage",
    "v5_migration",
  ]),
  cargoCheck("storage.ledger-projection-atomicity", [
    "test",
    "-p",
    "milim-storage",
    "message_projection_and_ledger_event_commit_or_rollback_together",
  ]),
  cargoCheck("storage.inbox-lifecycle", [
    "test",
    "-p",
    "milim-storage",
    "durable_inbox_claim_cancel_retarget_and_restart_are_atomic",
  ]),
  cargoCheck("storage.control-backup-v1-v2", [
    "test",
    "-p",
    "milim-storage",
    "v1_and_v2_control_backups_restore_inbox_and_ledger",
  ]),
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
  cargoCheck("runtimes.ledger-failure-barrier", [
    "test",
    "-p",
    "milim-agents",
    "commit_prevents",
  ]),
  cargoCheck("tools.model-call-result-order", [
    "test",
    "-p",
    "milim-agents",
    "parallel_tool_results_preserve_model_call_order",
  ]),
  cargoCheck("runtimes.run-ledger", ["test", "-p", "milim-server", "run_ledger"]),
  cargoCheck("runtimes.run-ledger-sqlite-authority", [
    "test",
    "-p",
    "milim-server",
    "subsequent_model_step_rebuilds_text_and_tool_context_from_sqlite",
  ]),
  cargoCheck("runtimes.run-ledger-privacy-block", [
    "test",
    "-p",
    "milim-server",
    "privacy_block_rejection_leaves_no_request_ledger_rows",
  ]),
  cargoCheck("runtimes.durable-inbox-contract", ["test", "-p", "milim-server", "inbox_"]),
  cargoCheck("environment.account-runtime-inherited", [
    "test",
    "-p",
    "milim-server",
    "account_runtime_processes_inherit_the_user_environment",
  ]),
  cargoCheck("environment.mcp-sanitized", [
    "test",
    "-p",
    "milim-mcp-client",
    "configured_mcp_environment_excludes_host_credentials_and_keeps_explicit_grants",
  ]),
  cargoCheck("environment.sandbox-sanitized", [
    "test",
    "-p",
    "milim-sandbox",
    "sandbox_payload_does_not_forward_host_environment",
  ]),
  cargoCheck("tools.execution-pipeline", ["test", "-p", "milim-tools"]),
  cargoCheck("tools.mcp-app-pipeline", [
    "test",
    "-p",
    "milim-server",
    "mcp_apps_http_bridge_auth_validation_and_isolation",
  ]),
  desktopCheck("desktop.runtime-selection", ["tests/turn-runtime.test.ts"]),
  desktopCheck("desktop.stream", ["tests/turn-stream.test.ts"]),
  desktopCheck("desktop.hot-swap", ["tests/hot-swap.test.ts"]),
  desktopCheck("desktop.queue", ["tests/turn-queue.test.ts"]),
  desktopCheck("desktop.persistence", ["tests/session-store.test.ts"]),
  desktopCheck("desktop.approval-events", ["tests/turn-events.test.ts"]),
  desktopCheck("desktop.quiet-run-details", ["tests/codex-runtime-rendering.test.tsx"]),
  nodeCheck("desktop.model-catalog-discovery", ["tests/api-model-discovery.test.mjs"]),
  nodeCheck("desktop.model-catalog-startup", ["tests/api-model-startup.test.mjs"]),
  nodeCheck("desktop.run-details-lazy-fetch", ["tests/run-details-lazy.test.mjs"]),
  nodeCheck("environment.host-shell-inherited", [
    "tests/run-tauri-rust-test.mjs",
    "host_shell_inherits_user_environment_and_declares_the_policy",
  ]),
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
