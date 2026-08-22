import { previewRuntimeFoldersEqual, previewRuntimeKeyForThread } from "../src/lib/previewRuntimeKeys.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const firstProjectKey = previewRuntimeKeyForThread("thread-a", "C:\\Users\\USER\\app\\");
const secondProjectKey = previewRuntimeKeyForThread("thread-b", "C:/Users/USER/app");
const caseVariantProjectKey = previewRuntimeKeyForThread("thread-c", "c:/users/user/APP/");
const formerCollisionA = previewRuntimeKeyForThread(
  "thread-d",
  "c:/workspaces/project-99409",
);
const formerCollisionB = previewRuntimeKeyForThread(
  "thread-e",
  "c:/workspaces/project-271500",
);

equal(firstProjectKey, secondProjectKey, "same folder should share a runtime key across threads");
equal(firstProjectKey, caseVariantProjectKey, "same folder key should ignore Windows path case and trailing slashes");
assert(
  formerCollisionA !== formerCollisionB,
  "distinct folders that collided under the legacy 32-bit hash should remain isolated",
);
equal(previewRuntimeKeyForThread("thread-a", ""), "thread-a", "no-folder runtime should stay thread-local");
equal(previewRuntimeFoldersEqual("C:\\Users\\USER\\app\\", "c:/users/user/app"), true, "folder status matching should use the same normalization as runtime keys");
equal(previewRuntimeFoldersEqual("\\\\?\\C:\\Users\\USER\\app", "c:/users/user/app"), true, "folder status matching should ignore Windows canonical path prefixes");
assert(/^[A-Za-z0-9_.-]+$/.test(firstProjectKey), "folder runtime key should be route-safe");

export {};
