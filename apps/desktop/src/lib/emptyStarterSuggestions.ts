import type { WorkspaceGitStatus } from "../api.js";

export type EmptyStarterSuggestionIcon =
  | "arrow"
  | "code"
  | "git"
  | "pencil"
  | "refresh";

export interface EmptyStarterSuggestion {
  id: string;
  label: string;
  detail: string;
  prompt: string;
  icon: EmptyStarterSuggestionIcon;
}

export interface EmptyStarterStrip {
  context: string | null;
  loading: boolean;
  suggestions: EmptyStarterSuggestion[];
}

function plural(count: number, word: string): string {
  return `${count.toLocaleString()} ${word}${count === 1 ? "" : "s"}`;
}

function projectName(folder: string): string {
  const parts = folder.trim().split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] || "Project";
}

function branchLabel(status: WorkspaceGitStatus): string {
  return status.branch || (status.head ? `detached ${status.head}` : "No commits");
}

function changedPathHint(status: WorkspaceGitStatus): string {
  const visibleChanges = status.changed_files.slice(0, 3);
  const paths = visibleChanges
    .map((change) => `\`${change.path}\``)
    .join(", ");
  if (!paths) return "";
  const remaining = Math.max(
    0,
    status.changed_file_count - visibleChanges.length,
  );
  return ` Start with ${paths}${remaining ? ` and ${plural(remaining, "other file")}` : ""}.`;
}

function diffDetail(status: WorkspaceGitStatus): string {
  if (!status.insertions && !status.deletions) return "";
  return ` \u00b7 +${status.insertions.toLocaleString()}/-${status.deletions.toLocaleString()}`;
}

function recentCommitContext(status: WorkspaceGitStatus): string {
  return status.recent_commits
    .slice(0, 3)
    .map((commit) => `\`${commit.hash} ${commit.subject}\``)
    .join(", ");
}

export const BUILTIN_QUICK_ACTIONS: EmptyStarterSuggestion[] = [
    {
      id: "plan-feature",
      label: "Plan a feature",
      detail: "Turn an idea into a buildable scope",
      prompt: "Help me turn a feature idea into a concrete implementation plan. Ask only for the missing context you need.",
      icon: "pencil",
    },
    {
      id: "review-code",
      label: "Review pasted code",
      detail: "Find bugs, regressions, and test gaps",
      prompt: "Review code I provide for bugs, regressions, and missing tests. Prioritize concrete findings.",
      icon: "git",
    },
    {
      id: "debug-failure",
      label: "Debug a failure",
      detail: "Start from an error or reproduction",
      prompt: "Help me debug a failure. Ask for the error, reproduction steps, and relevant files first.",
      icon: "refresh",
    },
  ];

function noFolderSuggestions(): EmptyStarterSuggestion[] {
  return BUILTIN_QUICK_ACTIONS.map((suggestion) => ({ ...suggestion }));
}

function folderSuggestions(name: string): EmptyStarterSuggestion[] {
  return [
    {
      id: "map-folder",
      label: "Map this project",
      detail: "Find entrypoints, structure, and risks",
      prompt: `Explore the selected ${name} project and map its important files, entrypoints, architecture, and current risks before suggesting changes.`,
      icon: "code",
    },
    {
      id: "run-project-checks",
      label: "Run project checks",
      detail: "Discover and run the smallest useful checks",
      prompt: `Inspect the ${name} project for its existing test, typecheck, lint, and build commands. Run the smallest relevant checks and report concrete failures before editing.`,
      icon: "refresh",
    },
    {
      id: "plan-next-change",
      label: "Plan the next change",
      detail: "Use project docs, tests, and TODOs",
      prompt: `Inspect the ${name} project documentation, tests, and TODOs, then identify and rank the most useful next changes by impact, risk, and effort.`,
      icon: "pencil",
    },
  ];
}

export function buildEmptyStarterStrip(
  folder: string,
  status: WorkspaceGitStatus | null,
  loading = false,
): EmptyStarterStrip {
  const trimmedFolder = folder.trim();
  if (!trimmedFolder) {
    return {
      context: null,
      loading: false,
      suggestions: noFolderSuggestions(),
    };
  }

  const name = projectName(trimmedFolder);
  if (loading) {
    return {
      context: null,
      loading: true,
      suggestions: [],
    };
  }

  if (!status || status.state !== "ready" || !status.is_repo) {
    return {
      context: null,
      loading: false,
      suggestions: folderSuggestions(name),
    };
  }

  const branch = branchLabel(status);
  const latest = status.recent_commits[0] ?? null;
  const contextParts = [branch];
  if (status.conflicts) contextParts.push(plural(status.conflicts, "conflict"));
  else if (status.has_changes)
    contextParts.push(plural(status.changed_file_count, "change"));
  else contextParts.push("Clean");
  if (status.behind) contextParts.push(`${status.behind.toLocaleString()} behind`);
  if (status.ahead) contextParts.push(`${status.ahead.toLocaleString()} ahead`);
  if (latest) contextParts.push(`latest ${latest.hash}`);

  let immediate: EmptyStarterSuggestion;
  if (status.conflicts) {
    immediate = {
      id: "resolve-conflicts",
      label: "Resolve conflicts",
      detail: `${plural(status.conflicts, "conflict")} on ${branch}`,
      prompt: `Resolve the current ${plural(status.conflicts, "Git conflict")} on \`${branch}\`. Re-check the repository state, inspect each side before editing, and explain the intended resolution.${changedPathHint(status)}`,
      icon: "git",
    };
  } else if (status.has_changes) {
    immediate = {
      id: "review-changes",
      label: "Review changes",
      detail: `${plural(status.changed_file_count, "file")}${diffDetail(status)}`,
      prompt: `Review the current working-tree changes on \`${branch}\` for bugs, regressions, and missing tests. Re-check Git status and inspect the diff before drawing conclusions.${changedPathHint(status)}`,
      icon: "git",
    };
  } else if (status.behind) {
    immediate = {
      id: "inspect-incoming",
      label: "Inspect incoming work",
      detail: `${plural(status.behind, "commit")} behind ${status.upstream || "upstream"}`,
      prompt: `Inspect why \`${branch}\` is ${plural(status.behind, "commit")} behind \`${status.upstream || "its upstream"}\`. Re-check and fetch Git state if needed, then summarize incoming changes, conflicts, and risks without pulling until I confirm.`,
      icon: "refresh",
    };
  } else if (status.ahead) {
    immediate = {
      id: "review-unpushed",
      label: "Review unpushed work",
      detail: `${plural(status.ahead, "commit")} ahead of ${status.upstream || "upstream"}`,
      prompt: `Review the ${plural(status.ahead, "unpushed commit")} on \`${branch}\`. Re-check Git state, inspect the commits and cumulative diff against \`${status.upstream || "the upstream branch"}\`, and flag risks or missing tests.`,
      icon: "git",
    };
  } else if (latest) {
    immediate = {
      id: "continue-latest",
      label: "Continue latest work",
      detail: `${latest.hash} ${latest.subject}`,
      prompt: `Continue the work around \`${latest.hash} ${latest.subject}\` on \`${branch}\`. Re-check the repository state, inspect that commit and its surrounding code, then identify the most coherent next improvement before editing.`,
      icon: "arrow",
    };
  } else {
    immediate = {
      id: "map-git-project",
      label: "Map this project",
      detail: "No commits yet",
      prompt: `Map the project on \`${branch}\` before making changes. Inspect its entrypoints, architecture, scripts, and risks, and verify the current repository state first.`,
      icon: "code",
    };
  }

  let recent: EmptyStarterSuggestion;
  if (latest && immediate.id === "continue-latest") {
    recent = {
      id: "review-recent-direction",
      label: "Review recent direction",
      detail: `Last ${Math.min(3, status.recent_commits.length)} commits on ${branch}`,
      prompt: `Review the recent direction on \`${branch}\`: ${recentCommitContext(status)}. Re-check the repository state, summarize the intent and quality of this work, and identify unfinished or risky areas.`,
      icon: "git",
    };
  } else if (latest) {
    recent = {
      id: "continue-latest",
      label: "Continue latest work",
      detail: `${latest.hash} ${latest.subject}`,
      prompt: `Continue the work around \`${latest.hash} ${latest.subject}\` on \`${branch}\`. Re-check the repository state, inspect that commit and its surrounding code, then identify the most coherent next improvement before editing.`,
      icon: "arrow",
    };
  } else {
    recent = {
      id: "run-project-checks",
      label: "Run project checks",
      detail: "Discover the existing scripts and baseline",
      prompt: `Inspect the project on \`${branch}\` for its test, typecheck, lint, and build commands. Re-check the repository state, run the smallest useful baseline, and report concrete failures before editing.`,
      icon: "refresh",
    };
  }

  const history = recentCommitContext(status);
  const next: EmptyStarterSuggestion = {
    id: "find-next-task",
    label: "Find the next task",
    detail: "Use repo docs, tests, and recent work",
    prompt: `Find the best next task for \`${branch}\`. Re-check the repository state, inspect project docs, tests, and TODOs${history ? `, and use recent commits ${history} as context` : ""}. Rank concrete options by impact and risk, then recommend one.`,
    icon: "pencil",
  };

  return {
    context: contextParts.join(" \u00b7 "),
    loading: false,
    suggestions: [immediate, recent, next],
  };
}
