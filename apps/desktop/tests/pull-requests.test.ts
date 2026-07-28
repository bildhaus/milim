import { strict as assert } from "node:assert";
import type { PullRequestDetails } from "../src/api.js";
import {
  pullRequestActorAvatarUrls,
  pullRequestAccessibleLabel,
  pullRequestReadiness,
} from "../src/lib/pullRequests.js";
import { upsertPullRequestItems } from "../src/lib/pullRequestCache.js";

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
    viewerPermission: "WRITE",
    checks: [{ name: "test", state: "SUCCESS", bucket: "pass" }],
    ...patch,
  };
}

assert.deepEqual(
  pullRequestActorAvatarUrls({
    login: "tripwire-dev",
    avatarUrl: "https://avatars.githubusercontent.com/in/3162576?v=4",
  }),
  [
    "https://avatars.githubusercontent.com/in/3162576?v=4",
    "https://github.com/tripwire-dev.png?size=40",
    "https://avatars.githubusercontent.com/tripwire-dev%5Bbot%5D?size=40",
  ],
);
assert.deepEqual(pullRequestActorAvatarUrls({ login: "oshtz" }), [
  "https://github.com/oshtz.png?size=40",
  "https://avatars.githubusercontent.com/oshtz%5Bbot%5D?size=40",
]);
assert.equal(
  pullRequestActorAvatarUrls({ login: "comp-ai-code-review" })[2],
  undefined,
  "exhausting user and GitHub App avatar candidates should render the fallback icon",
);
assert.deepEqual(pullRequestActorAvatarUrls(), []);

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
assert.deepEqual(
  pullRequestReadiness(pullRequest({ viewerPermission: "READ" })),
  { tone: "blocked", label: "No merge permission", canMerge: false },
);
assert.deepEqual(
  pullRequestReadiness(pullRequest({ viewerPermission: undefined })),
  { tone: "pending", label: "Merge permission unavailable", canMerge: false },
);
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

const cached = {
  number: 1,
  title: "Cached",
  url: "https://github.com/acme/repo/pull/1",
  state: "OPEN",
  isDraft: false,
  repository: "acme/repo",
  commentsCount: 0,
  authored: true,
  reviewing: false,
  updatedAt: "2026-01-01T00:00:00Z",
};
const refreshed = {
  ...cached,
  title: "Refreshed",
  commentsCount: 2,
  updatedAt: "2026-01-02T00:00:00Z",
};
const added = {
  ...cached,
  number: 2,
  title: "New",
  url: "https://github.com/acme/repo/pull/2",
  updatedAt: "2026-01-03T00:00:00Z",
};
assert.deepEqual(
  upsertPullRequestItems([cached], [refreshed, added]).map((item) => [
    item.number,
    item.title,
    item.commentsCount,
  ]),
  [
    [2, "New", 0],
    [1, "Refreshed", 2],
  ],
);
