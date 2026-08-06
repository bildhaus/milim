import { useState } from "react";
import type {
  WorkspaceCheckpoint,
  WorkspaceGitActionResult,
} from "../api";
import {
  diffRows,
  diffSections,
  diffStats,
  type DiffSection,
  type DiffStats,
} from "../lib/gitDiffRows";
import { ChevronDown, Eye, FileText, GitBranch, Refresh } from "./icons";

const COLLAPSED_FILE_COUNT = 3;

export type TurnChanges = {
  key: string;
  checkpoint: WorkspaceCheckpoint;
  result: WorkspaceGitActionResult;
  sections: DiffSection[];
  stats: DiffStats;
};

export type TurnReviewState =
  | ({ status: "ready" } & TurnChanges)
  | {
      key: string;
      status: "checking" | "no_changes";
      checkpoint: WorkspaceCheckpoint;
    }
  | {
      key: string;
      status: "unavailable";
      checkpoint: WorkspaceCheckpoint;
      message: string;
    };

export function turnChangesFromDiff(
  key: string,
  checkpoint: WorkspaceCheckpoint,
  result: WorkspaceGitActionResult,
): TurnChanges | null {
  if (!result.ok) return null;
  const output = [result.stdout, result.stderr]
    .map((part) => part.trim())
    .filter(Boolean)
    .join("\n\n");
  if (!output) return null;
  const rows = diffRows(output);
  const sections = diffSections(rows);
  if (!sections.length) return null;
  return { key, checkpoint, result, sections, stats: diffStats(rows) };
}

export function turnReviewFromDiff(
  key: string,
  checkpoint: WorkspaceCheckpoint,
  result: WorkspaceGitActionResult,
): TurnReviewState {
  if (!result.ok) {
    return {
      key,
      checkpoint,
      status: "unavailable",
      message: result.message || "Git could not load this turn's diff.",
    };
  }
  const changes = turnChangesFromDiff(key, checkpoint, result);
  return changes
    ? { ...changes, status: "ready" }
    : { key, checkpoint, status: "no_changes" };
}

export function TurnChangesCard({
  review,
  onUndo,
  onReview,
  onRetry,
  onOpenGit,
}: {
  review: TurnReviewState;
  onUndo: () => void;
  onReview: () => void;
  onRetry: () => void;
  onOpenGit: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  if (review.status === "no_changes") return null;
  if (review.status !== "ready") {
    const title =
      review.status === "checking"
        ? "Checking turn changes..."
        : "Change review unavailable";
    return (
      <section
        className={`turn-changes-card ${review.status}`}
        data-testid="turn-changes-card"
        data-review-state={review.status}
        aria-label="Turn changes"
      >
        <div className="turn-changes-head">
          <span className="turn-changes-icon" aria-hidden="true">
            {review.status === "checking" ? (
              <Refresh size={15} />
            ) : (
              <FileText size={15} />
            )}
          </span>
          <div className="turn-changes-title">
            <strong>{title}</strong>
            {review.status === "unavailable" && <span>{review.message}</span>}
          </div>
          {review.status === "unavailable" && (
            <div className="turn-changes-actions">
              <button
                data-testid="turn-changes-retry"
                type="button"
                onClick={onRetry}
              >
                <Refresh size={12} />
                <span>Retry</span>
              </button>
              <button
                data-testid="turn-changes-open-git"
                type="button"
                onClick={onOpenGit}
              >
                <GitBranch size={12} />
                <span>Open Git</span>
              </button>
            </div>
          )}
        </div>
      </section>
    );
  }

  const { sections, stats } = review;
  const hiddenCount = Math.max(0, sections.length - COLLAPSED_FILE_COUNT);
  const visibleSections = expanded
    ? sections
    : sections.slice(0, COLLAPSED_FILE_COUNT);

  return (
    <section
      className="turn-changes-card"
      data-testid="turn-changes-card"
      aria-label="Turn changes"
    >
      <div className="turn-changes-head">
        <span className="turn-changes-icon" aria-hidden="true">
          <FileText size={15} />
        </span>
        <div className="turn-changes-title">
          <strong>
            Changed {stats.files} file{stats.files === 1 ? "" : "s"}
          </strong>
          <span className="turn-changes-total">
            <span className="add">+{stats.additions}</span>
            <span className="delete">-{stats.deletions}</span>
          </span>
        </div>
        <div className="turn-changes-actions">
          <button data-testid="turn-changes-undo" type="button" onClick={onUndo}>
            <Refresh size={12} />
            <span>Undo</span>
          </button>
          <button data-testid="turn-changes-review" type="button" onClick={onReview}>
            <Eye size={12} />
            <span>Review changes</span>
          </button>
        </div>
      </div>
      <div className="turn-changes-files">
        {visibleSections.map((section) => (
          <div className="turn-changes-file" key={section.id}>
            <code title={section.path}>{section.path}</code>
            <span className="turn-changes-file-stat">
              <span className="add">+{section.additions}</span>
              <span className="delete">-{section.deletions}</span>
            </span>
          </div>
        ))}
      </div>
      {hiddenCount > 0 && (
        <button
          className="turn-changes-toggle"
          data-testid="turn-changes-toggle"
          type="button"
          aria-expanded={expanded}
          onClick={() => setExpanded((value) => !value)}
        >
          <span>{expanded ? "Show less" : `Show ${hiddenCount} more`}</span>
          <ChevronDown size={12} />
        </button>
      )}
    </section>
  );
}
