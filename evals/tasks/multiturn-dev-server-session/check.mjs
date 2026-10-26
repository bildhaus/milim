import { changedFiles, exists, expect, finish, headFile, lines, read, requireTrajectory, turnAnswer } from "../../lib/check.mjs";

// Turn 1 starts the server, turn 2 must query that same process (its boot
// id and counter are random per process), turn 3 stops it. A restart in
// turn 2 shows up as a second boot and a different boot id.

/** JSON lines the server wrote under var/ (ignored by git). */
function entries(path) {
  if (!exists(path)) return [];
  return lines(read(path)).flatMap((line) => {
    try {
      return [JSON.parse(line)];
    } catch {
      return [];
    }
  });
}

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
const mentions = (text, value) => new RegExp(`(^|[^0-9a-z])${value}([^0-9a-z]|$)`, "i").test(text);

const boots = entries("var/boots.jsonl").filter((entry) => entry.event === "boot");
const requests = entries("var/requests.jsonl");

// Never let a leftover server outlive the check, then report it.
const running = boots.map((boot) => boot.pid).filter(alive);
for (const pid of running) {
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    // Already gone.
  }
}

const modified = changedFiles().filter((path) => headFile(path) !== null);
expect(modified.length === 0, `existing files modified: ${modified.join(", ")}`);

const trajectory = requireTrajectory();
if (trajectory && expect(boots.length > 0, "the dev server was never started (var/boots.jsonl records no boot)")) {
  expect(trajectory.turns.length === 3, `expected 3 recorded turns, got ${trajectory.turns.length}`);
  expect(
    boots.length === 1,
    `the dev server was started ${boots.length} times (boot ids ${boots.map((boot) => boot.boot_id).join(", ")}); turn 2 had to query the process from turn 1`,
  );
  const [{ boot_id: bootId }] = boots;
  const [turnOneEnd, turnTwoEnd] = trajectory.turns.map((turn) => turn.ended_at_ms ?? Infinity);

  // Turn 1: a live /health answer from that boot, reported back.
  const health = requests.filter((entry) => entry.path === "/health" && entry.boot_id === bootId && entry.at_ms <= turnOneEnd);
  expect(health.length > 0, "turn 1 never called GET /health on the server");
  expect(mentions(turnAnswer(1), bootId), `turn 1's answer does not report boot id ${bootId}`);

  // Turn 2: three counter calls on the same process, last value and boot id reported.
  const counters = requests.filter(
    (entry) => entry.path === "/counter" && entry.status === 200 && entry.at_ms > turnOneEnd && entry.at_ms <= turnTwoEnd,
  );
  const sameBoot = counters.filter((entry) => entry.boot_id === bootId);
  expect(sameBoot.length >= 3, `turn 2 made ${sameBoot.length} GET /counter call(s) on the turn-1 server (boot ${bootId}); expected 3`);
  if (counters.length > 0) {
    const last = counters.at(-1).body?.value;
    expect(mentions(turnAnswer(2), last), `turn 2's answer does not report the last counter value ${last}`);
  }
  expect(mentions(turnAnswer(2), bootId), `turn 2's answer does not report boot id ${bootId} (the server from turn 1)`);

  // Turn 3: stopped.
  expect(running.length === 0, `the dev server was left running (pid ${running.join(", ")}); turn 3 asked to stop it`);
}

finish();
