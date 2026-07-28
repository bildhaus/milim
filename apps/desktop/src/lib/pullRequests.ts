import type { PullRequestActor, PullRequestDetails } from "../api";

export type PullRequestTone =
  | "draft"
  | "pending"
  | "blocked"
  | "ready"
  | "merged"
  | "closed"
  | "stale";

export interface PullRequestReadiness {
  tone: PullRequestTone;
  label: string;
  canMerge: boolean;
}

export interface PullRequestSnapshot {
  folder: string;
  pullRequest: PullRequestDetails | null;
  checkedAt: number;
  stale: boolean;
  error?: string;
}

export function pullRequestActorAvatarUrls(actor?: PullRequestActor): string[] {
  const login = actor?.login.trim();
  return Array.from(
    new Set(
      [
        actor?.avatarUrl?.trim(),
        login ? `https://github.com/${encodeURIComponent(login)}.png?size=40` : "",
        login
          ? `https://avatars.githubusercontent.com/${encodeURIComponent(`${login}[bot]`)}?size=40`
          : "",
      ].filter((url): url is string => Boolean(url)),
    ),
  );
}

export function pullRequestReadiness(
  pullRequest: PullRequestDetails,
  stale = false,
): PullRequestReadiness {
  if (stale) return { tone: "stale", label: "Status unavailable", canMerge: false };
  const state = pullRequest.state.toUpperCase();
  if (state === "MERGED")
    return { tone: "merged", label: "Merged", canMerge: false };
  if (state === "CLOSED")
    return { tone: "closed", label: "Closed", canMerge: false };
  if (pullRequest.isDraft)
    return { tone: "draft", label: "Draft", canMerge: false };

  const buckets = (pullRequest.checks ?? []).map((check) =>
    check.bucket.toLowerCase(),
  );
  if (
    pullRequest.mergeable?.toUpperCase() === "CONFLICTING" ||
    pullRequest.mergeStateStatus?.toUpperCase() === "DIRTY"
  ) {
    return { tone: "blocked", label: "Merge conflicts", canMerge: false };
  }
  if (
    buckets.some((bucket) => bucket === "fail" || bucket === "cancel")
  ) {
    return { tone: "blocked", label: "Checks failing", canMerge: false };
  }
  if (pullRequest.reviewDecision?.toUpperCase() === "CHANGES_REQUESTED") {
    return { tone: "blocked", label: "Changes requested", canMerge: false };
  }
  if (buckets.some((bucket) => bucket === "pending")) {
    return { tone: "pending", label: "Checks pending", canMerge: false };
  }
  if (pullRequest.reviewDecision?.toUpperCase() === "REVIEW_REQUIRED") {
    return { tone: "pending", label: "Review required", canMerge: false };
  }
  if (
    pullRequest.mergeable?.toUpperCase() !== "MERGEABLE" ||
    pullRequest.mergeStateStatus?.toUpperCase() !== "CLEAN"
  ) {
    return { tone: "pending", label: "Merge status pending", canMerge: false };
  }
  const permission = pullRequest.viewerPermission?.toUpperCase();
  if (!permission)
    return { tone: "pending", label: "Merge permission unavailable", canMerge: false };
  if (!["WRITE", "MAINTAIN", "ADMIN"].includes(permission))
    return { tone: "blocked", label: "No merge permission", canMerge: false };
  return { tone: "ready", label: "Ready to merge", canMerge: true };
}

export function pullRequestAccessibleLabel(
  pullRequest: PullRequestDetails,
  stale = false,
): string {
  const readiness = pullRequestReadiness(pullRequest, stale);
  const checks = pullRequest.checks ?? [];
  const passed = checks.filter((check) => check.bucket === "pass").length;
  const checksLabel = checks.length
    ? `${passed} of ${checks.length} checks passing`
    : "No checks reported";
  const review = pullRequest.reviewDecision
    ? pullRequest.reviewDecision.toLowerCase().replace(/_/g, " ")
    : "No review requirement";
  return `PR #${pullRequest.number} · ${readiness.label} · ${checksLabel} · ${review}`;
}
