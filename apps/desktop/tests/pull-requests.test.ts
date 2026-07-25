import { strict as assert } from "node:assert";
import type { PullRequestDetails } from "../src/api.js";
import {
  pullRequestAccessibleLabel,
  pullRequestReadiness,
} from "../src/lib/pullRequests.js";

function pullRequest(patch: Partial<PullRequestDetails> = {}): PullRequestDetails {
  return {
    exists: true,
    number: 12,
    title: "Ship PR cockpit",
    url: "https://github.com/acme/repo/pull/12",
    state: "OPEN",
    isDraft: false,
    baseRefName: "main",
    headRefName: "feature",
    headRefOid: "abc123",
    body: "",
    additions: 10,
    deletions: 2,
    changedFiles: 3,
    mergeable: "MERGEABLE",
    mergeStateStatus: "CLEAN",
    checks: [{ name: "test", state: "SUCCESS", bucket: "pass" }],
    ...patch,
  };
}

assert.equal(pullRequestReadiness(pullRequest({ isDraft: true })).tone, "draft");
assert.equal(
  pullRequestReadiness(
    pullRequest({ checks: [{ name: "test", state: "PENDING", bucket: "pending" }] }),
  ).tone,
  "pending",
);
assert.equal(
  pullRequestReadiness(
    pullRequest({ checks: [{ name: "test", state: "FAILURE", bucket: "fail" }] }),
  ).tone,
  "blocked",
);
assert.equal(
  pullRequestReadiness(pullRequest({ reviewDecision: "CHANGES_REQUESTED" })).tone,
  "blocked",
);
assert.equal(
  pullRequestReadiness(
    pullRequest({ mergeable: "CONFLICTING", mergeStateStatus: "DIRTY" }),
  ).tone,
  "blocked",
);
assert.equal(pullRequestReadiness(pullRequest()).tone, "ready");
assert.equal(
  pullRequestReadiness(pullRequest({ state: "MERGED" })).tone,
  "merged",
);
assert.equal(
  pullRequestReadiness(pullRequest({ state: "CLOSED" })).tone,
  "closed",
);
assert.equal(pullRequestReadiness(pullRequest(), true).tone, "stale");
assert.match(
  pullRequestAccessibleLabel(pullRequest({ reviewDecision: "APPROVED" })),
  /PR #12.*1 of 1 checks passing.*approved/,
);
