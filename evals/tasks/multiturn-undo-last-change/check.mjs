import {
  attempt,
  expect,
  expectEqual,
  expectOnlyChanged,
  finish,
  load,
  loadSource,
  read,
  requireTrajectory,
  runNodeTests,
  turnFile,
} from "../../lib/check.mjs";

// Turn 1 renames a setting (touching src/config.js), turn 2 bumps a default
// in the same file, turn 3 asks to undo "the last change you made" to it.
// Restoring the committed file (`git checkout`) also reverts turn 1's
// rename, so only an agent that knows what its own last change was passes.
const CONFIG = "src/config.js";
const FILES = [CONFIG, "src/client.js", "test/client.test.js", "docs/config.md"];
const normalize = (text) => (text ?? "").replace(/\s+/g, " ").trim();
const mentionsOldName = (text) => /\btimeoutMs\b/.test(text ?? "");

expectOnlyChanged(FILES);

async function defaults(source, label) {
  return attempt(label, async () => (await loadSource(source, "config.mjs")).DEFAULTS);
}

if (requireTrajectory()) {
  // Turn 1: the rename everywhere, nothing else.
  const turnOne = await defaults(turnFile(1, CONFIG), "turn 1 src/config.js");
  if (turnOne) {
    expectEqual(turnOne.requestTimeoutMs, 5000, "turn 1: DEFAULTS.requestTimeoutMs");
    expectEqual(turnOne.retries, 3, "turn 1: DEFAULTS.retries (unchanged)");
  }
  for (const path of FILES) {
    expect(!mentionsOldName(turnFile(1, path)), `turn 1: ${path} still mentions timeoutMs`);
  }

  // Turn 2: the retry bump, in code and docs.
  const turnTwo = await defaults(turnFile(2, CONFIG), "turn 2 src/config.js");
  if (turnTwo) {
    expectEqual(turnTwo.retries, 5, "turn 2: DEFAULTS.retries");
    expectEqual(turnTwo.requestTimeoutMs, 5000, "turn 2: DEFAULTS.requestTimeoutMs");
  }
  expect(/\|\s*`retries`\s*\|\s*5\s*\|/.test(turnFile(2, "docs/config.md") ?? ""), "turn 2: docs/config.md does not show 5 retries");

  // Turn 3: src/config.js back exactly as turn 1 left it; everything else as turn 2 left it.
  expect(
    normalize(read(CONFIG)) === normalize(turnFile(1, CONFIG)),
    "src/config.js is not back to the state after turn 1 (undo only the retry change, keep the rename)",
  );
  for (const path of FILES.filter((path) => path !== CONFIG)) {
    expect(read(path) === turnFile(2, path), `${path} changed in turn 3; only src/config.js was to be undone`);
  }
  const final = await defaults(read(CONFIG), "final src/config.js");
  if (final) {
    expectEqual(final.retries, 3, "final DEFAULTS.retries");
    expectEqual(final.requestTimeoutMs, 5000, "final DEFAULTS.requestTimeoutMs");
    expect(!("timeoutMs" in final), "final DEFAULTS still has timeoutMs");
  }
  await attempt("final client", async () => {
    const { requestOptions } = await load("src/client.js");
    expectEqual(requestOptions({ requestTimeoutMs: 750 }).timeout, 750, "requestOptions honors requestTimeoutMs");
    expectEqual(requestOptions().attempts, 4, "requestOptions default attempts");
  });
  runNodeTests("test");
}

finish();
