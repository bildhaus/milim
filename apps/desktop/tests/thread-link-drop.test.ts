import assert from "node:assert/strict";
import { threadLinkDropDecision } from "../src/lib/threadLinks.js";

assert.equal(threadLinkDropDecision({ sourceThreadId: "a", targetThreadId: "b", linkedThreadIds: [] }), "valid");
assert.equal(threadLinkDropDecision({ sourceThreadId: "a", targetThreadId: "a", linkedThreadIds: [] }), "self");
assert.equal(threadLinkDropDecision({ sourceThreadId: "a", targetThreadId: "b", linkedThreadIds: ["a"] }), "duplicate");
assert.equal(threadLinkDropDecision({ sourceThreadId: "worker", targetThreadId: "b", linkedThreadIds: [], sourceIsChild: true }), "child");
