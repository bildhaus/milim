import { changedFiles, exists, expect, expectEqual, finish, headFile, lines, read } from "../../lib/check.mjs";

// Stock plus confirmed shipments below the reorder point in `north`
// (a naive stock-only count is 25; counting tentative shipments gives 21).
const EXPECTED_COUNT = 23;

/** JSON lines the service wrote under var/ (ignored by git). */
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

// Stop anything still running first, so a leftover service never outlives
// the check, then report it.
const started = entries("var/servers.jsonl").filter((entry) => entry.event === "starting").map((entry) => entry.pid);
const running = started.filter(alive);
for (const pid of running) {
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    // Already gone.
  }
}
expect(started.length > 0, "the service was never started (var/servers.jsonl records no start)");
expect(running.length === 0, `the service was left running (pid ${running.join(", ")}); stop it when done`);

// New files (ANSWER.json, captured output) are fine; existing ones must not change.
const modified = changedFiles().filter((path) => headFile(path) !== null);
expect(modified.length === 0, `existing files modified: ${modified.join(", ")}`);

if (expect(exists("ANSWER.json"), "ANSWER.json missing")) {
  let answer = null;
  try {
    answer = JSON.parse(read("ANSWER.json"));
  } catch (error) {
    expect(false, `ANSWER.json is not valid JSON: ${error.message}`);
  }
  if (answer) {
    expectEqual(Number(answer.count), EXPECTED_COUNT, "count");
    // The id is random per request, so it can only come from a live query.
    const served = entries("var/requests.jsonl").find((entry) => entry.request_id === answer.request_id);
    if (expect(served, `request_id ${JSON.stringify(answer.request_id)} was never issued by the service`)) {
      expect(
        served.path === "/stats/low-stock" && served.query?.warehouse === "north" && served.status === 200,
        `request_id belongs to ${served.method} ${served.path} ${JSON.stringify(served.query)} (status ${served.status}), not the north low-stock report`,
      );
    }
  }
}

finish();
