import type {
  WorkspaceGitAction,
  WorkspaceGitDiffScope,
  WorkspaceGitFileChange,
  WorkspaceGitStatus,
} from "../api";

export type GitFileStagingAction = Extract<
  WorkspaceGitAction,
  "stage_file" | "unstage_file" | "discard_file"
>;
export type GitHunkStagingAction = Extract<
  WorkspaceGitAction,
  "stage_hunk" | "unstage_hunk" | "discard_hunk"
>;
export type GitStagingAction = GitFileStagingAction | GitHunkStagingAction;

export interface GitFileStaging {
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  conflicted: boolean;
}

const STAGING_ACTIONS = new Set<WorkspaceGitAction>([
  "stage_file",
  "unstage_file",
  "discard_file",
  "stage_hunk",
  "unstage_hunk",
  "discard_hunk",
]);

export function isGitStagingAction(
  action: WorkspaceGitAction,
): action is GitStagingAction {
  return STAGING_ACTIONS.has(action);
}

export function gitFileStaging(change: WorkspaceGitFileChange): GitFileStaging {
  const status = change.status;
  const untracked = status === "??";
  const conflicted =
    status.includes("U") || status === "AA" || status === "DD";
  if (typeof change.staged === "boolean" || typeof change.unstaged === "boolean") {
    return {
      staged: Boolean(change.staged),
      unstaged: Boolean(change.unstaged),
      untracked,
      conflicted,
    };
  }
  // Older backends trim the porcelain column pair, so a lone letter does not
  // say which side changed. Offer staging only; unstaging stays unavailable.
  return {
    staged: status.length === 2 && !untracked,
    unstaged: true,
    untracked,
    conflicted,
  };
}

export function gitFileStagingActions(
  staging: GitFileStaging,
): GitFileStagingAction[] {
  if (staging.conflicted) return ["stage_file"];
  const actions: GitFileStagingAction[] = [];
  if (staging.unstaged) actions.push("stage_file");
  if (staging.staged) actions.push("unstage_file");
  if (staging.unstaged) actions.push("discard_file");
  return actions;
}

/** Hunk actions only exist for diffs that compare exactly one side of the index. */
export function gitHunkStagingActions(
  scope: WorkspaceGitDiffScope,
  staging: GitFileStaging | null,
): GitHunkStagingAction[] {
  if (staging?.untracked || staging?.conflicted) return [];
  if (scope === "unstaged") return ["stage_hunk", "discard_hunk"];
  if (scope === "staged") return ["unstage_hunk"];
  return [];
}

export function gitStagingActionLabel(action: GitStagingAction): string {
  switch (action) {
    case "stage_file":
      return "Stage file";
    case "unstage_file":
      return "Unstage file";
    case "discard_file":
      return "Discard file changes";
    case "stage_hunk":
      return "Stage hunk";
    case "unstage_hunk":
      return "Unstage hunk";
    case "discard_hunk":
      return "Discard hunk";
  }
}

export function gitDiscardConfirmation(
  path: string,
  staging: GitFileStaging | null,
  target: "file" | "hunk",
): { title: string; message: string; confirmLabel: string } {
  if (target === "file" && staging?.untracked) {
    return {
      title: "Delete untracked file?",
      message: `${path} is not tracked by Git. Discarding it permanently deletes it from disk and cannot be undone.`,
      confirmLabel: "Delete file",
    };
  }
  if (target === "hunk") {
    return {
      title: "Discard this hunk?",
      message: `The unstaged lines in this hunk of ${path} are replaced with their staged or committed version. This cannot be undone.`,
      confirmLabel: "Discard hunk",
    };
  }
  return {
    title: "Discard file changes?",
    message: `Unstaged edits to ${path} are replaced with its staged or committed version. Staged changes are kept. This cannot be undone.`,
    confirmLabel: "Discard changes",
  };
}

export interface GitCommitScope {
  stagedFiles: WorkspaceGitFileChange[];
  unstagedFiles: WorkspaceGitFileChange[];
  allFiles: WorkspaceGitFileChange[];
}

export function gitCommitScope(status: WorkspaceGitStatus): GitCommitScope {
  const stagedFiles: WorkspaceGitFileChange[] = [];
  const unstagedFiles: WorkspaceGitFileChange[] = [];
  for (const change of status.changed_files) {
    const staging = gitFileStaging(change);
    if (staging.staged) stagedFiles.push(change);
    if (staging.unstaged) unstagedFiles.push(change);
  }
  return { stagedFiles, unstagedFiles, allFiles: status.changed_files };
}

/** Default to committing the index when something is staged, otherwise everything. */
export function defaultCommitStageAll(status: WorkspaceGitStatus): boolean {
  return status.staged === 0;
}
