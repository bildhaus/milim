import type { WorkspaceGitFileChange, WorkspaceGitStatus } from "../api";
import {
  gitCommitScope,
  gitFileStaging,
  gitFileStagingActions,
  gitStagingActionLabel,
  type GitFileStagingAction,
  type GitHunkStagingAction,
  type GitStagingAction,
} from "../lib/gitStaging";
import { Minus, Plus, Undo } from "./icons";

function StagingIcon({ action }: { action: GitStagingAction }) {
  if (action === "stage_file" || action === "stage_hunk") return <Plus size={12} />;
  if (action === "unstage_file" || action === "unstage_hunk") return <Minus size={12} />;
  return <Undo size={12} />;
}

function StagingButton({
  action,
  label,
  disabled,
  onClick,
}: {
  action: GitStagingAction;
  label: string;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={`git-icon-btn git-stage-btn${action.startsWith("discard_") ? " danger" : ""}`}
      title={label}
      aria-label={label}
      data-git-staging-action={action}
      disabled={disabled}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
    >
      <StagingIcon action={action} />
    </button>
  );
}

/** Inline stage, unstage, and discard buttons for one changed file. */
export function GitFileStageActions({
  change,
  busy,
  onAction,
}: {
  change: WorkspaceGitFileChange;
  busy: boolean;
  onAction: (action: GitFileStagingAction, change: WorkspaceGitFileChange) => void;
}) {
  const staging = gitFileStaging(change);
  const actions = gitFileStagingActions(staging);
  if (!actions.length) return null;
  return (
    <span className="git-stage-actions" role="group" aria-label="File staging">
      {actions.map((action) => (
        <StagingButton
          key={action}
          action={action}
          label={
            action === "stage_file" && staging.conflicted
              ? "Stage file to mark resolved"
              : action === "discard_file" && staging.untracked
                ? "Delete untracked file"
                : gitStagingActionLabel(action)
          }
          disabled={busy}
          onClick={() => onAction(action, change)}
        />
      ))}
    </span>
  );
}

/** Buttons shown on a diff hunk header when the diff compares one side of the index. */
export function GitHunkActions({
  actions,
  busy,
  onAction,
}: {
  actions: GitHunkStagingAction[];
  busy: boolean;
  onAction: (action: GitHunkStagingAction) => void;
}) {
  if (!actions.length) return null;
  return (
    <span className="git-hunk-actions" role="group" aria-label="Hunk staging">
      {actions.map((action) => (
        <StagingButton
          key={action}
          action={action}
          label={gitStagingActionLabel(action)}
          disabled={busy}
          onClick={() => onAction(action)}
        />
      ))}
    </span>
  );
}

function fileListLabel(files: WorkspaceGitFileChange[]): string {
  const names = files.slice(0, 4).map((file) => file.path);
  const more = files.length - names.length;
  return more > 0 ? `${names.join(", ")} and ${more} more` : names.join(", ");
}

/** Explicit choice between committing the index and staging everything first. */
export function GitCommitScopeChoice({
  status,
  stageAll,
  disabled,
  onChange,
}: {
  status: WorkspaceGitStatus;
  stageAll: boolean;
  disabled: boolean;
  onChange: (stageAll: boolean) => void;
}) {
  const scope = gitCommitScope(status);
  const stagedCount = scope.stagedFiles.length;
  const allCount = scope.allFiles.length;
  return (
    <fieldset className="git-commit-scope" disabled={disabled}>
      <legend>Commit</legend>
      <label className={`git-commit-scope-option${stageAll ? "" : " active"}`}>
        <input
          type="radio"
          name="git-commit-scope"
          checked={!stageAll}
          disabled={stagedCount === 0}
          onChange={() => onChange(false)}
        />
        <span>
          <strong>Staged changes only</strong>
          <small title={fileListLabel(scope.stagedFiles)}>
            {stagedCount
              ? `${stagedCount} staged file${stagedCount === 1 ? "" : "s"}${
                  scope.unstagedFiles.length
                    ? ` · ${scope.unstagedFiles.length} unstaged left out`
                    : ""
                }`
              : "Nothing staged yet"}
          </small>
        </span>
      </label>
      <label className={`git-commit-scope-option${stageAll ? " active" : ""}`}>
        <input
          type="radio"
          name="git-commit-scope"
          checked={stageAll}
          onChange={() => onChange(true)}
        />
        <span>
          <strong>Stage all and commit</strong>
          <small title={fileListLabel(scope.allFiles)}>
            {`${allCount} file${allCount === 1 ? "" : "s"}, including unstaged and untracked`}
          </small>
        </span>
      </label>
    </fieldset>
  );
}
