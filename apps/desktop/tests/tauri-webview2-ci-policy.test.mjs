import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyWebViewPolicy, configureCiWebViewPolicy } from "./tauri-webview2-ci-policy.mjs";

const absent = { exists: false, kind: null, value: null };
const values = { AdditionalBrowserArguments: "--remote-debugging-port=9677", UserDataFolder: "C:\\private\\webview" };
function fixture(initial) {
  const state = structuredClone(initial);
  const writes = [];
  return { state, writes, read: (name) => structuredClone(state[name]), write(name, value) { writes.push(name); state[name] = structuredClone(value); } };
}
const initial = {
  AdditionalBrowserArguments: { exists: true, kind: "String", value: "ORIGINAL_ARGUMENTS" },
  UserDataFolder: { exists: true, kind: "ExpandString", value: "%TEMP%\\מקורי" },
};
const existing = fixture(initial);
const policy = applyWebViewPolicy(existing, values);
assert.equal(existing.state.UserDataFolder.value, values.UserDataFolder);
assert.ok(!JSON.stringify(policy.previousEntries).includes("ORIGINAL_ARGUMENTS"));
policy.restore();
assert.deepEqual(existing.state, initial, "Raw expandable values and types must be restored exactly");

const missing = fixture({ AdditionalBrowserArguments: absent, UserDataFolder: absent });
applyWebViewPolicy(missing, values).restore();
assert.deepEqual(missing.state, { AdditionalBrowserArguments: absent, UserDataFolder: absent });

const partial = fixture(initial);
const write = partial.write.bind(partial);
let failOnce = true;
partial.write = (name, value) => {
  write(name, value);
  if (name === "UserDataFolder" && failOnce) { failOnce = false; throw new Error("Injected partial setup failure"); }
};
assert.throws(() => applyWebViewPolicy(partial, values), /setup failed; prior values were restored/);
assert.deepEqual(partial.state, initial, "Partial setup must restore both snapshots");

const ignoredWrite = fixture(initial);
const originalWrite = ignoredWrite.write.bind(ignoredWrite);
ignoredWrite.write = (name, value) => { if (value.value !== values[name]) originalWrite(name, value); };
assert.throws(() => applyWebViewPolicy(ignoredWrite, values), /setup failed; prior values were restored/);
assert.deepEqual(ignoredWrite.state, initial, "A failed policy readback must stop setup and restore state");

const restoreFailure = fixture(initial);
const installed = applyWebViewPolicy(restoreFailure, values);
const normalWrite = restoreFailure.write.bind(restoreFailure);
restoreFailure.write = (name, value) => {
  if (name === "AdditionalBrowserArguments") throw new Error("Injected restoration failure");
  normalWrite(name, value);
};
assert.throws(() => installed.restore(), /restoration failed/);
assert.deepEqual(restoreFailure.state.UserDataFolder, initial.UserDataFolder, "A restoration failure must not skip the other value");

const invalid = fixture({ ...initial, UserDataFolder: { exists: true, kind: "DWord", value: 7 } });
assert.throws(() => applyWebViewPolicy(invalid, values), /Unexpected existing/);
assert.equal(invalid.writes.length, 0, "All prior types must be checked before any write");
console.log("WebView2 policy: exact raw restoration, absent values, partial setup rollback, restoration failure and unsupported prior type verified.");

if (process.argv.includes("--native")) {
  const directory = mkdtempSync(join(tmpdir(), "milim-webview-policy-"));
  let nativePolicy;
  try {
    nativePolicy = configureCiWebViewPolicy(directory, 9677);
    assert.ok(nativePolicy, "Native policy proof requires a GitHub-hosted Windows runner.");
    nativePolicy.restore();
    nativePolicy = null;
  } finally {
    try { nativePolicy?.restore(); }
    finally { rmSync(directory, { recursive: true, force: true }); }
  }
  console.log("Native WebView2 policy round-trip passed; both prior values restored exactly; no app launched.");
}
