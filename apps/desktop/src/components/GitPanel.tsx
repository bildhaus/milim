import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import {
  claudeRuntimeModel,
  codexRuntimeModel,
  opencodeRuntimeModel,
  piRuntimeModel,
  completeChat,
  getWorkspaceGitStatus,
  listModelsDetailed,
  openArtifactLocation,
  openExternalUrl,
  runWorkspaceGitAction,
  setWorkspace,
  streamClaudeRun,
  streamCodexRun,
  streamOpenCodeRun,
  streamPiRun,
  type ClaudeRunEvent,
  type CodexRunEvent,
  type OpenCodeRunEvent,
  type PiRunEvent,
  type WorkspaceGitAction,
  type WorkspaceGitActionResult,
  type WorkspaceGitDiffScope,
  type WorkspaceGitStatus,
  type WorkspaceGitBranch,
  type WorkspaceGitFileChange,
} from "../api";
import { commitMessageModelCandidates } from "../lib/gitCommitMessageModels";
import {
  diffRows,
  diffSections,
  diffStats,
  findDiffSectionIndex,
  gitFileTree,
  shouldCollapseDiffSection,
  type DiffRow,
  type DiffSection,
  type GitFileTreeNode,
} from "../lib/gitDiffRows";
import { shouldRefreshGitStatus } from "../lib/gitRefresh";
import { useUiPreferences } from "../ui/store";
import { useContextMenu } from "./ContextMenu";
import {
  ArrowUp,
  ChevronDown,
  Code,
  Copy,
  Download,
  Folder,
  GitBranch,
  GitCommit,
  GitLogo,
  GitRemote,
  Lightbulb,
  Refresh,
  Search,
  Sidebar,
  X,
} from "./icons";

const COMMIT_MESSAGE_DIFF_LIMIT = 18_000;
const DIFF_NAVIGATOR_MIN_WIDTH = 160;
const DIFF_NAVIGATOR_DEFAULT_WIDTH = 210;
const DIFF_MAIN_MIN_WIDTH = 320;
const DIFF_NAVIGATOR_KEYBOARD_STEP = 24;
const DIFF_SCOPE_OPTIONS: { value: WorkspaceGitDiffScope; label: string }[] = [
  { value: "all", label: "All changes" },
  { value: "unstaged", label: "Unstaged" },
  { value: "staged", label: "Staged" },
  { value: "last_turn", label: "Last turn" },
  { value: "commit", label: "Commit" },
  { value: "branch", label: "Branch" },
];
export type GitPanelDiffRequest = {
  id: number;
  checkpoint: string;
  result: WorkspaceGitActionResult;
};
const COMMIT_MESSAGE_SYSTEM_PROMPT =
  "Write one professional Git commit subject. Return exactly one line, no markdown, no quotes. Use imperative mood when natural. Keep it under 72 characters.";

function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() || path;
}

function compactPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.length > 1 ? parts.slice(-2).join("/") : parts[0] || path;
}

function cleanGitPath(path: string): string {
  const trimmed = path.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    return trimmed.slice(1, -1).replace(/\\"/g, '"').replace(/\\\\/g, "\\");
  }
  return trimmed;
}

function gitStatusLabel(status: string): string {
  if (status === "??") return "new";
  if (status.includes("U")) return "conf";
  if (status.includes("R")) return "ren";
  if (status.includes("D")) return "del";
  if (status.includes("A")) return "add";
  if (status.includes("M")) return "mod";
  return status || "changed";
}

function plural(count: number, word: string): string {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

function branchLabel(status: WorkspaceGitStatus): string {
  return (
    status.branch || (status.head ? `detached ${status.head}` : "No commits")
  );
}

function changedFilesCount(status: WorkspaceGitStatus): number {
  return status.changed_file_count;
}

function changeLabel(status: WorkspaceGitStatus): string {
  if (status.conflicts) return plural(status.conflicts, "conflict");
  if (status.has_changes) return `${changedFilesCount(status)} changed`;
  return "Clean";
}

function diffLabel(status: WorkspaceGitStatus): string | null {
  return status.insertions || status.deletions
    ? `+${status.insertions.toLocaleString()}/-${status.deletions.toLocaleString()}`
    : null;
}

function renderDiffStat(status: WorkspaceGitStatus): ReactNode {
  if (!status.insertions && !status.deletions) return null;
  return (
    <span
      className="git-diff-stat"
      aria-label={`${status.insertions.toLocaleString()} insertions, ${status.deletions.toLocaleString()} deletions`}
    >
      <span className="git-diff-stat-add">
        +{status.insertions.toLocaleString()}
      </span>
      <span className="git-diff-stat-separator">/</span>
      <span className="git-diff-stat-delete">
        -{status.deletions.toLocaleString()}
      </span>
    </span>
  );
}

function compactChangeLabel(status: WorkspaceGitStatus): string {
  return [changeLabel(status), diffLabel(status)].filter(Boolean).join(" / ");
}

function changeTone(
  status: WorkspaceGitStatus,
): "clean" | "dirty" | "conflict" {
  if (status.conflicts) return "conflict";
  return status.has_changes ? "dirty" : "clean";
}

function syncLabel(status: WorkspaceGitStatus): string {
  if (status.ahead || status.behind) {
    return `${status.ahead} ahead / ${status.behind} behind`;
  }
  if (status.upstream) return "Up to date";
  if (status.remote) return "No upstream";
  return "No remote";
}

function branchMetaLabel(branch: WorkspaceGitBranch): string {
  const sync =
    branch.ahead || branch.behind
      ? ` · ${branch.ahead} ahead / ${branch.behind} behind`
      : "";
  return `${branch.current ? "current" : branch.upstream ? branch.upstream : "local"}${sync}`;
}

function remoteWebUrl(remote: string | null): string | null {
  if (!remote) return null;
  const trimmed = remote.trim().replace(/\.git$/, "");
  if (/^https?:\/\//i.test(trimmed)) return trimmed;

  const ssh = trimmed.match(/^ssh:\/\/(?:[^@]+@)?([^/]+)\/(.+)$/i);
  if (ssh) return `https://${ssh[1]}/${ssh[2]}`;

  const scp = trimmed.match(
    /^(?:[^@\\/\s]+@)?([^:\\/\s]+(?:\.[^:\\/\s]+)+):([^\\\s]+\/[^\\\s]+)$/,
  );
  return scp ? `https://${scp[1]}/${scp[2]}` : null;
}

function gitStateLabel(status: WorkspaceGitStatus | null): string {
  if (!status) return "Git unavailable";
  if (status.state === "not_git") return "Not a Git repo";
  if (status.state === "no_folder") return "No folder selected";
  if (status.state === "error") return "Git unavailable";
  return status.message || "Git unavailable";
}

function updatedLabel(updatedAt: number | null, now: number): string {
  if (!updatedAt) return "";
  const minutes = Math.floor((now - updatedAt) / 60000);
  if (minutes < 1) return "Updated now";
  if (minutes < 60) return `Updated ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  return `Updated ${hours}h ago`;
}

function commitPrompt(folder: string, status: WorkspaceGitStatus): string {
  return [
    "Review the Git changes in the selected working folder and prepare a commit or push plan.",
    `Folder: ${folder}`,
    `Branch: ${branchLabel(status)}`,
    "Start by running `git status --short --branch` and inspecting the diff.",
    "Summarize the changes and propose a commit message. Do not commit or push until I confirm.",
  ].join("\n");
}

function gitTaskPrompt(
  folder: string,
  status: WorkspaceGitStatus,
  task: string,
  instruction: string,
): string {
  return [
    task,
    `Folder: ${folder}`,
    `Branch: ${branchLabel(status)}`,
    "Start by running `git status --short --branch`.",
    instruction,
  ].join("\n");
}

function agentReviewPrompt(folder: string, status: WorkspaceGitStatus): string {
  if (status.conflicts) {
    return gitTaskPrompt(
      folder,
      status,
      "Review the current Git conflicts in the selected working folder.",
      "Identify conflicted files, risk, and a resolution plan. Do not edit files until I confirm.",
    );
  }
  if (status.has_changes) {
    return commitPrompt(folder, status);
  }
  return gitTaskPrompt(
    folder,
    status,
    "Review the selected working folder's Git state.",
    "Inspect the status, branch, upstream, and recent changes if relevant. Call out risks and useful next commands. Do not change the repository until I confirm.",
  );
}

function syncCommand(
  status: WorkspaceGitStatus,
): {
  action: WorkspaceGitAction;
  label: string;
  disabled?: boolean;
  title: string;
} | null {
  if (status.behind) {
    return status.has_changes
      ? {
          action: "pull",
          label: "Pull",
          disabled: true,
          title: "Clean the worktree before pulling.",
        }
      : { action: "pull", label: "Pull", title: "Run git pull --ff-only." };
  }
  if (status.ahead && status.upstream)
    return { action: "push", label: "Push", title: "Run git push." };
  if (!status.upstream && status.remote && status.branch) {
    return {
      action: "publish",
      label: "Publish",
      title: `Run git push -u origin ${status.branch}.`,
    };
  }
  return null;
}

function actionOutputText(result: WorkspaceGitActionResult): string {
  return [result.stdout, result.stderr]
    .map((part) => part.trim())
    .filter(Boolean)
    .join("\n\n");
}

function changedFileTitle(change: WorkspaceGitFileChange): string {
  return `${gitStatusLabel(change.status)} ${cleanGitPath(change.path)}`;
}

function quoteArg(value: string): string {
  return value.trim() ? JSON.stringify(value.trim()) : "<message>";
}

function commandPreview(
  action: WorkspaceGitAction,
  status: WorkspaceGitStatus,
  message: string,
  stageAll: boolean,
): string {
  if (action === "commit") {
    return `${stageAll ? "git add -A && " : ""}git commit -m ${quoteArg(message)}`;
  }
  if (action === "commit_push") {
    const push = status.upstream
      ? "git push"
      : `git push -u origin ${status.branch || "<branch>"}`;
    return `${stageAll ? "git add -A && " : ""}git commit -m ${quoteArg(message)} && ${push}`;
  }
  if (action === "pull") return "git pull --ff-only";
  if (action === "push") return "git push";
  if (action === "publish")
    return `git push -u origin ${status.branch || "<branch>"}`;
  if (action === "fetch") return "git fetch --prune";
  if (action === "checkout_branch") return "git checkout <branch>";
  if (action === "create_branch") return "git checkout -b <branch>";
  return "git diff --no-ext-diff --stat --patch HEAD --";
}

function actionLabel(action: WorkspaceGitAction): string {
  if (action === "commit_push") return "Commit and push";
  if (action === "checkout_branch") return "Checkout branch";
  if (action === "create_branch") return "Create branch";
  return action.slice(0, 1).toUpperCase() + action.slice(1);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function changedSegments(
  text: string,
  other: string | null,
): { text: string; changed: boolean }[] {
  if (!other) return [{ text, changed: false }];
  let start = 0;
  const maxStart = Math.min(text.length, other.length);
  while (start < maxStart && text[start] === other[start]) start += 1;

  let end = 0;
  const maxEnd = maxStart - start;
  while (
    end < maxEnd &&
    text[text.length - 1 - end] === other[other.length - 1 - end]
  )
    end += 1;

  const before = text.slice(0, start);
  const middle = text.slice(start, text.length - end);
  const after = end ? text.slice(text.length - end) : "";
  return [
    before ? { text: before, changed: false } : null,
    middle ? { text: middle, changed: true } : null,
    after ? { text: after, changed: false } : null,
  ].filter(Boolean) as { text: string; changed: boolean }[];
}

function diffTextSegments(
  row: DiffRow,
  index: number,
  rows: DiffRow[],
): { text: string; changed: boolean }[] {
  if (row.kind === "add") {
    const previous = rows[index - 1];
    return changedSegments(
      row.text,
      previous?.kind === "delete" ? previous.text : null,
    );
  }
  if (row.kind === "delete") {
    const next = rows[index + 1];
    return changedSegments(row.text, next?.kind === "add" ? next.text : null);
  }
  return [{ text: row.text, changed: false }];
}

function renderDiffText(
  row: DiffRow,
  index: number,
  rows: DiffRow[],
  query: string,
): ReactNode {
  const term = query.trim();
  const matcher = term ? new RegExp(`(${escapeRegExp(term)})`, "ig") : null;
  let key = 0;
  return diffTextSegments(row, index, rows).flatMap((segment) => {
    const className = segment.changed ? "git-diff-changed-text" : undefined;
    if (!matcher)
      return [
        <span className={className} key={key++}>
          {segment.text}
        </span>,
      ];
    return segment.text
      .split(matcher)
      .filter(Boolean)
      .map((part) => {
        const isMatch = part.toLowerCase() === term.toLowerCase();
        return isMatch ? (
          <mark className={className} key={key++}>
            {part}
          </mark>
        ) : (
          <span className={className} key={key++}>
            {part}
          </span>
        );
      });
  });
}

function cleanGeneratedCommitMessage(text: string): string {
  const firstLine =
    text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find(Boolean) ?? "";
  return firstLine
    .replace(/^[-*`"'\s]+|[`"'\s]+$/g, "")
    .replace(/^(commit message|subject):\s*/i, "")
    .slice(0, 72)
    .trim();
}

function commitMessageContext(
  status: WorkspaceGitStatus,
  diff: string,
): string {
  return [
    `Branch: ${branchLabel(status)}`,
    `Files: ${status.changed_files.map((change) => `${gitStatusLabel(change.status)} ${cleanGitPath(change.path)}`).join(", ")}`,
    "Diff:",
    diff.slice(0, COMMIT_MESSAGE_DIFF_LIMIT),
  ].join("\n");
}

async function generateCommitMessage(
  model: string,
  status: WorkspaceGitStatus,
  diff: string,
): Promise<string> {
  const response = await completeChat(
    model,
    [
      {
        role: "system",
        content: COMMIT_MESSAGE_SYSTEM_PROMPT,
      },
      {
        role: "user",
        content: commitMessageContext(status, diff),
      },
    ],
    { maxTokens: 40, temperature: 0.2 },
  );
  const message = cleanGeneratedCommitMessage(response);
  if (!message) throw new Error("The model returned an empty commit message.");
  return message;
}

async function generateAccountRuntimeCommitMessage(
  preferredModel: string,
  folder: string,
  status: WorkspaceGitStatus,
  diff: string,
): Promise<string | null> {
  const codexModel = codexRuntimeModel(preferredModel);
  const claudeModel = claudeRuntimeModel(preferredModel);
  const opencodeModel = opencodeRuntimeModel(preferredModel);
  const piModel = piRuntimeModel(preferredModel);
  if (!codexModel && !claudeModel && !opencodeModel && !piModel) return null;

  let response = "";
  let runtimeError = "";
  let runtimeWarning = "";
  const prompt = `${COMMIT_MESSAGE_SYSTEM_PROMPT}\n\n${commitMessageContext(status, diff)}`;
  const onEvent = (event: CodexRunEvent | ClaudeRunEvent | OpenCodeRunEvent | PiRunEvent) => {
    if (event.type === "token" && event.text) response += event.text;
    else if (event.type === "warning") runtimeWarning = event.message;
    else if (event.type === "error") runtimeError = event.message;
  };

  if (codexModel) {
    await streamCodexRun(
      {
        model: codexModel,
        prompt,
        cwd: folder.trim() || undefined,
        tool_approval_policy: "review",
        tool_approval_grant: false,
        plan_mode: true,
      },
      onEvent,
    );
  } else if (claudeModel) {
    await streamClaudeRun(
      {
        model: claudeModel,
        prompt,
        cwd: folder.trim() || undefined,
        tool_approval_policy: "review",
        tool_approval_grant: false,
        plan_mode: true,
      },
      onEvent,
    );
  } else if (opencodeModel) {
    await streamOpenCodeRun({
      model: opencodeModel,
      prompt,
      cwd: folder.trim() || undefined,
      tool_approval_policy: "review",
      tool_approval_grant: false,
      plan_mode: true,
    }, onEvent);
  } else if (piModel) {
    await streamPiRun({
      model: piModel,
      prompt,
      cwd: folder.trim() || undefined,
      persist_session: false,
      tool_approval_policy: "review",
      tool_approval_grant: false,
      plan_mode: true,
    }, onEvent);
  }

  if (runtimeWarning) throw new Error(runtimeWarning);
  if (runtimeError) throw new Error(runtimeError);

  const message = cleanGeneratedCommitMessage(response);
  if (!message)
    throw new Error(
      `${codexModel ? "Codex" : claudeModel ? "Claude CLI" : opencodeModel ? "OpenCode CLI" : "Pi CLI"} returned an empty commit message.`,
    );
  return message;
}

async function generateCommitMessageWithFallback(
  preferredModel: string,
  folder: string,
  status: WorkspaceGitStatus,
  diff: string,
): Promise<string> {
  let lastError = "";
  try {
    const accountRuntimeMessage = await generateAccountRuntimeCommitMessage(
      preferredModel,
      folder,
      status,
      diff,
    );
    if (accountRuntimeMessage) return accountRuntimeMessage;
  } catch (error) {
    lastError =
      error instanceof Error
        ? error.message
        : "account runtime generation failed";
  }

  const candidates = commitMessageModelCandidates(
    await listModelsDetailed(),
    preferredModel,
  );
  if (!candidates.length) {
    throw new Error(
      lastError ||
        "Enter a commit message, or add a reachable chat model to generate one.",
    );
  }

  for (const candidate of candidates) {
    try {
      return await generateCommitMessage(candidate, status, diff);
    } catch (error) {
      lastError = error instanceof Error ? error.message : "generation failed";
    }
  }
  throw new Error(lastError || "Couldn't generate a commit message.");
}

export function GitPanel({
  folder,
  onDraftAction,
  model,
  onOpenPanel,
  forceExpanded = false,
  diffRequest,
}: {
  folder: string;
  onDraftAction: (text: string) => void;
  model: string;
  onOpenPanel?: () => void;
  forceExpanded?: boolean;
  diffRequest?: GitPanelDiffRequest | null;
}) {
  const { openContextMenu } = useContextMenu();
  const [status, setStatus] = useState<WorkspaceGitStatus | null>(null);
  const [statusFolder, setStatusFolder] = useState("");
  const [loading, setLoading] = useState(false);
  const [refreshKey, setRefreshKey] = useState(0);
  const lastGitStatusRunAtRef = useRef<number | null>(null);
  const expandedPreference = useUiPreferences((s) => s.gitPanelExpanded);
  const setExpanded = useUiPreferences((s) => s.setGitPanelExpanded);
  const expanded = forceExpanded || (onOpenPanel ? false : expandedPreference);
  const [notice, setNotice] = useState<string | null>(null);
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [commandBusy, setCommandBusy] = useState<WorkspaceGitAction | null>(
    null,
  );
  const [commandResult, setCommandResult] =
    useState<WorkspaceGitActionResult | null>(null);
  const [diffResult, setDiffResult] = useState<WorkspaceGitActionResult | null>(
    null,
  );
  const [commandMenu, setCommandMenu] = useState<WorkspaceGitAction | null>(
    null,
  );
  const [branchMenuOpen, setBranchMenuOpen] = useState(false);
  const [branchFilter, setBranchFilter] = useState("");
  const [newBranchName, setNewBranchName] = useState("");
  const [commitMessage, setCommitMessage] = useState("");
  const [generatingCommitMessage, setGeneratingCommitMessage] = useState(false);
  const [stageAll, setStageAll] = useState(true);
  const [collapsedDiffSections, setCollapsedDiffSections] = useState<
    Set<string>
  >(() => new Set());
  const [diffSearch, setDiffSearch] = useState("");
  const [diffSearchIndex, setDiffSearchIndex] = useState(0);
  const [diffScope, setDiffScope] = useState<WorkspaceGitDiffScope>("all");
  const [diffBase, setDiffBase] = useState("");
  const [reviewAnchor, setReviewAnchor] = useState<{ sectionId: string; index: number } | null>(null);
  const [pendingDiffFile, setPendingDiffFile] = useState<{
    path: string;
    index: number;
  } | null>(null);
  const [activeDiffSectionId, setActiveDiffSectionId] = useState<string | null>(
    null,
  );
  const [diffNavigatorWidth, setDiffNavigatorWidth] = useState(
    DIFF_NAVIGATOR_DEFAULT_WIDTH,
  );
  const [diffNavigatorVisible, setDiffNavigatorVisible] = useState(true);
  const [diffNavigatorResizing, setDiffNavigatorResizing] = useState(false);
  const [collapsedFileTreePaths, setCollapsedFileTreePaths] = useState<
    Set<string>
  >(() => new Set());
  const diffNavigatorResizeStartRef = useRef<{
    clientX: number;
    width: number;
  } | null>(null);
  const diffLayoutRef = useRef<HTMLDivElement | null>(null);
  const diffViewRef = useRef<HTMLDivElement | null>(null);
  const selectedFolder = folder.trim();

  useEffect(() => {
    setCommandResult(null);
    setDiffResult(null);
    setCommandMenu(null);
    setBranchMenuOpen(false);
    setBranchFilter("");
    setNewBranchName("");
    setCommandBusy(null);
    setCommitMessage("");
    setGeneratingCommitMessage(false);
    setStageAll(true);
    setCollapsedDiffSections(new Set());
    setDiffSearch("");
    setDiffSearchIndex(0);
    setDiffScope("all");
    setDiffBase("");
    setPendingDiffFile(null);
    setActiveDiffSectionId(null);
    setCollapsedFileTreePaths(new Set());
  }, [selectedFolder]);

  useEffect(() => {
    if (!diffRequest) return;
    setDiffScope("last_turn");
    setDiffBase(diffRequest.checkpoint);
    setDiffResult(diffRequest.result);
  }, [diffRequest]);

  useEffect(() => {
    setNotice(null);
    if (!selectedFolder) {
      setStatus(null);
      setStatusFolder("");
      setLoading(false);
      setUpdatedAt(null);
      lastGitStatusRunAtRef.current = null;
      return;
    }

    let cancelled = false;
    lastGitStatusRunAtRef.current = Date.now();
    setLoading(true);
    void (async () => {
      await setWorkspace(selectedFolder);
      const next = await getWorkspaceGitStatus();
      if (cancelled) return;
      setStatus(
        (previous) =>
          next ?? (statusFolder === selectedFolder ? previous : null),
      );
      setStatusFolder(selectedFolder);
      const timestamp = Date.now();
      setUpdatedAt(timestamp);
      setNow(timestamp);
      setLoading(false);
    })();

    return () => {
      cancelled = true;
    };
  }, [refreshKey, selectedFolder]);

  useEffect(() => {
    if (!selectedFolder) return;
    function refreshOnFocus() {
      const timestamp = Date.now();
      if (!shouldRefreshGitStatus(lastGitStatusRunAtRef.current, timestamp))
        return;
      lastGitStatusRunAtRef.current = timestamp;
      setRefreshKey((value) => value + 1);
    }
    window.addEventListener("focus", refreshOnFocus);
    return () => window.removeEventListener("focus", refreshOnFocus);
  }, [selectedFolder]);

  useEffect(() => {
    if (!updatedAt) return;
    const interval = window.setInterval(() => setNow(Date.now()), 60000);
    return () => window.clearInterval(interval);
  }, [updatedAt]);

  useEffect(() => {
    if (!commandMenu && !branchMenuOpen && !diffResult) return;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      setCommandMenu(null);
      setBranchMenuOpen(false);
      setDiffResult(null);
    }
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [commandMenu, branchMenuOpen, diffResult]);

  useEffect(() => {
    const output = diffResult ? actionOutputText(diffResult) : "";
    const rows = output ? diffRows(output) : [];
    setCollapsedDiffSections(
      new Set(
        diffSections(rows)
          .filter(shouldCollapseDiffSection)
          .map((section) => section.id),
      ),
    );
    setDiffSearch("");
    setDiffSearchIndex(0);
  }, [diffResult]);

  useEffect(() => {
    if (!diffSearch.trim()) return;
    const handle = window.setTimeout(() => {
      diffViewRef.current
        ?.querySelector(`[data-match-index="${diffSearchIndex}"]`)
        ?.scrollIntoView({ block: "center" });
    }, 0);
    return () => window.clearTimeout(handle);
  }, [collapsedDiffSections, diffSearch, diffSearchIndex]);

  useEffect(() => {
    if (pendingDiffFile && diffResult) focusDiffFile(pendingDiffFile);
  }, [diffResult, pendingDiffFile]);

  const currentStatus = statusFolder === selectedFolder ? status : null;
  const repoName = useMemo(() => {
    const root = currentStatus?.root || selectedFolder;
    return root ? basename(root) : "Repository";
  }, [selectedFolder, currentStatus?.root]);

  function forceRefreshGitStatus() {
    lastGitStatusRunAtRef.current = Date.now();
    setRefreshKey((value) => value + 1);
  }

  if (!selectedFolder) return null;

  function openGitStateContextMenu(event: ReactMouseEvent) {
    openContextMenu(
      event,
      [
        {
          id: "refresh",
          label: "Refresh Git status",
          icon: <Refresh size={13} />,
          action: forceRefreshGitStatus,
        },
        {
          id: "open-folder",
          label: "Open folder",
          icon: <Folder size={13} />,
          separatorBefore: true,
          action: () =>
            void openArtifactLocation(selectedFolder, "folder").catch(
              () => undefined,
            ),
        },
      ],
      "Git",
    );
  }

  if (
    !currentStatus ||
    currentStatus.state !== "ready" ||
    !currentStatus.is_repo
  ) {
    const label =
      loading && !currentStatus
        ? "Checking Git..."
        : gitStateLabel(currentStatus);
    const stateClass = forceExpanded
      ? "git-panel git-panel-state git-panel-workspace git-panel-workspace-state"
      : `git-panel git-panel-collapsed git-panel-state${onOpenPanel ? " git-panel-launcher" : ""}`;
    return (
      <section
        className={stateClass}
        data-testid="git-panel"
        aria-label="Git environment"
        onContextMenu={openGitStateContextMenu}
      >
        <div
          className="git-panel-state-row"
          title={currentStatus?.message || selectedFolder}
        >
          <span className="git-panel-mark" aria-hidden="true">
            <GitLogo size={16} />
          </span>
          <strong className="git-panel-state-label">{label}</strong>
          <button
            type="button"
            className="git-icon-btn"
            data-testid="git-refresh"
            title="Refresh Git status"
            aria-label="Refresh Git status"
            onClick={forceRefreshGitStatus}
          >
            <Refresh size={13} />
          </button>
        </div>
      </section>
    );
  }

  const readyStatus = currentStatus;
  const changeSummary = changeLabel(readyStatus);
  const updated = updatedLabel(updatedAt, now);
  const remoteUrl = remoteWebUrl(readyStatus.remote);
  const sync = syncCommand(readyStatus);
  const SyncIcon =
    sync?.action === "pull"
      ? Download
      : sync?.action === "push"
        ? ArrowUp
        : GitBranch;
  const commandOutput = commandResult ? actionOutputText(commandResult) : "";
  const diffOutput = diffResult ? actionOutputText(diffResult) : "";
  const renderedDiffRows = diffOutput ? diffRows(diffOutput) : [];
  const renderedDiffSections = renderedDiffRows.length
    ? diffSections(renderedDiffRows)
    : [];
  const renderedDiffStats = renderedDiffRows.length
    ? diffStats(renderedDiffRows)
    : { files: 0, additions: 0, deletions: 0 };
  const allDiffSectionsCollapsed =
    renderedDiffSections.length > 0 &&
    renderedDiffSections.every((section) =>
      collapsedDiffSections.has(section.id),
    );
  const navigableFiles = renderedDiffSections.length
    ? renderedDiffSections.map((section, index) => {
        const change = readyStatus.changed_files.find(
          (candidate) => findDiffSectionIndex([section], candidate.path) === 0,
        );
        const status = change
          ? gitStatusLabel(change.status)
          : section.deletions && !section.additions
            ? "del"
            : section.additions && !section.deletions
              ? "add"
              : "mod";
        return {
          index,
          key: section.id,
          path: section.path,
          sectionId: section.id,
          status,
          title: `${status} ${section.path}`,
        };
      })
    : readyStatus.changed_files.map((change, index) => ({
        index,
        key: `${change.status}:${change.path}`,
        path: cleanGitPath(change.path),
        sectionId: null,
        status: gitStatusLabel(change.status),
        title: changedFileTitle(change),
      }));
  const navigableFileTree = gitFileTree(
    navigableFiles.map((file) => file.path),
  );
  const scopeStats = diffResult
    ? renderedDiffStats
    : {
        files: readyStatus.changed_file_count,
        additions: readyStatus.insertions,
        deletions: readyStatus.deletions,
      };
  const recentCommits = readyStatus.recent_commits ?? [];
  const comparisonBranches = (readyStatus.branches ?? []).filter(
    (branch) => !branch.current,
  );
  const normalizedDiffSearch = diffSearch.trim().toLowerCase();
  const visibleDiffRows: {
    row: DiffRow;
    index: number;
    section: DiffSection | null;
    sectionIndex: number;
  }[] = [];
  let currentSectionIndex = -1;
  for (let index = 0; index < renderedDiffRows.length; index += 1) {
    if (renderedDiffSections[currentSectionIndex + 1]?.start === index)
      currentSectionIndex += 1;
    const section =
      currentSectionIndex >= 0
        ? renderedDiffSections[currentSectionIndex]
        : null;
    if (
      section &&
      collapsedDiffSections.has(section.id) &&
      renderedDiffRows[index].kind !== "file"
    )
      continue;
    visibleDiffRows.push({
      row: renderedDiffRows[index],
      index,
      section,
      sectionIndex: currentSectionIndex,
    });
  }
  const diffSearchMatches = normalizedDiffSearch
    ? visibleDiffRows
        .filter(({ row }) =>
          row.text.toLowerCase().includes(normalizedDiffSearch),
        )
        .map(({ index }) => index)
    : [];
  const activeDiffSearchIndex = diffSearchMatches.length
    ? Math.min(diffSearchIndex, diffSearchMatches.length - 1)
    : 0;
  const diffSearchMatchIndexByRow = new Map(
    diffSearchMatches.map((rowIndex, matchIndex) => [rowIndex, matchIndex]),
  );
  const commitReady = stageAll || readyStatus.staged > 0;
  const canCommitPush = Boolean(
    readyStatus.remote && readyStatus.branch && readyStatus.behind === 0,
  );
  const normalizedBranchFilter = branchFilter.trim().toLowerCase();
  const branches = readyStatus.branches ?? [];
  const filteredBranches = normalizedBranchFilter
    ? branches.filter(
        (branch) =>
          branch.name.toLowerCase().includes(normalizedBranchFilter) ||
          branch.upstream?.toLowerCase().includes(normalizedBranchFilter),
      )
    : branches;
  const nextBranchName = newBranchName.trim();
  const canCreateBranch =
    Boolean(nextBranchName) &&
    !branches.some((branch) => branch.name === nextBranchName);

  async function copyBranch() {
    try {
      await navigator.clipboard?.writeText(branchLabel(readyStatus));
    } catch {
      // Clipboard access can be unavailable in web previews.
    }
    setNotice("Branch copied");
  }

  async function openFolder() {
    await openArtifactLocation(
      readyStatus.root || selectedFolder,
      "folder",
    ).catch(() => undefined);
  }

  async function openRemote() {
    if (!remoteUrl) return;
    await openExternalUrl(remoteUrl).catch(() =>
      setNotice("Couldn't open remote"),
    );
  }

  function openGitContextMenu(event: ReactMouseEvent) {
    openContextMenu(
      event,
      [
        {
          id: "refresh",
          label: "Refresh",
          icon: <Refresh size={13} />,
          action: forceRefreshGitStatus,
        },
        {
          id: "copy-branch",
          label: "Copy branch",
          detail: branchLabel(readyStatus),
          icon: <Copy size={13} />,
          action: () => void copyBranch(),
        },
        {
          id: "branch",
          label: "Switch or create branch",
          icon: <GitBranch size={13} />,
          action: () => {
            setNotice(null);
            setBranchMenuOpen(true);
          },
        },
        {
          id: "open-folder",
          label: "Open folder",
          icon: <Folder size={13} />,
          separatorBefore: true,
          action: () => void openFolder(),
        },
        {
          id: "open-remote",
          label: "Open remote",
          icon: <GitRemote size={13} />,
          disabled: !remoteUrl,
          action: () => void openRemote(),
        },
        {
          id: "diff",
          label: "Diff",
          icon: <Code size={13} />,
          disabled: !readyStatus.has_changes || Boolean(commandBusy),
          separatorBefore: true,
          action: () => void runGitCommand("diff"),
        },
        {
          id: "fetch",
          label: "Fetch",
          icon: <GitRemote size={13} />,
          disabled: !readyStatus.remote || Boolean(commandBusy),
          action: () => void runGitCommand("fetch"),
        },
        {
          id: "commit",
          label: "Commit",
          icon: <GitCommit size={13} />,
          disabled:
            !readyStatus.has_changes ||
            readyStatus.conflicts > 0 ||
            Boolean(commandBusy),
          action: () => openCommandMenu("commit"),
        },
        ...(sync
          ? [
              {
                id: "sync",
                label: sync.label,
                icon: <SyncIcon size={13} />,
                disabled: sync.disabled || Boolean(commandBusy),
                action: () => openCommandMenu(sync.action),
              },
            ]
          : []),
        {
          id: "agent-review",
          label: "Ask agent to review",
          icon: <Lightbulb size={13} />,
          separatorBefore: true,
          action: () =>
            onDraftAction(agentReviewPrompt(selectedFolder, readyStatus)),
        },
      ],
      "Git",
    );
  }

  function scrollToDiffSection(sectionIndex: number) {
    window.setTimeout(() => {
      diffViewRef.current
        ?.querySelector(`[data-section-index="${sectionIndex}"]`)
        ?.scrollIntoView({ block: "start" });
    }, 0);
  }

  function focusDiffFile(target: { path: string; index: number }) {
    const sectionIndex = findDiffSectionIndex(
      renderedDiffSections,
      target.path,
      target.index,
    );
    if (sectionIndex < 0) {
      setNotice(
        diffResult?.truncated
          ? "That file is outside the truncated diff."
          : "No diff section found for that file.",
      );
      setPendingDiffFile(null);
      return;
    }
    const section = renderedDiffSections[sectionIndex];
    setCollapsedDiffSections((current) => {
      if (!current.has(section.id)) return current;
      const next = new Set(current);
      next.delete(section.id);
      return next;
    });
    setActiveDiffSectionId(section.id);
    setPendingDiffFile(null);
    setNotice(null);
    scrollToDiffSection(sectionIndex);
  }

  function openDiffFile(target: { path: string; index: number }) {
    if (diffResult) {
      focusDiffFile(target);
      return;
    }
    setPendingDiffFile(target);
    void runGitCommand("diff", {
      diff_scope: diffScope,
      diff_base: diffBase || undefined,
    });
  }

  function selectDiffScope(scope: WorkspaceGitDiffScope) {
    const base =
      scope === "commit"
        ? recentCommits[0]?.hash ?? ""
        : scope === "branch"
          ? comparisonBranches[0]?.name ?? ""
          : "";
    setDiffScope(scope);
    setDiffBase(base);
    setPendingDiffFile(null);
    void runGitCommand("diff", {
      diff_scope: scope,
      diff_base: base || undefined,
    });
  }

  function selectDiffBase(base: string) {
    setDiffBase(base);
    setPendingDiffFile(null);
    void runGitCommand("diff", {
      diff_scope: diffScope,
      diff_base: base,
    });
  }

  function toggleDiffSection(sectionId: string) {
    setCollapsedDiffSections((current) => {
      const next = new Set(current);
      if (next.has(sectionId)) next.delete(sectionId);
      else next.add(sectionId);
      return next;
    });
    setDiffSearchIndex(0);
  }

  function toggleAllDiffSections() {
    setCollapsedDiffSections(
      allDiffSectionsCollapsed
        ? new Set()
        : new Set(renderedDiffSections.map((section) => section.id)),
    );
    setDiffSearchIndex(0);
  }

  function resizeDiffNavigator(width: number) {
    const max = Math.max(
      DIFF_NAVIGATOR_MIN_WIDTH,
      (diffLayoutRef.current?.clientWidth ??
        DIFF_NAVIGATOR_DEFAULT_WIDTH + DIFF_MAIN_MIN_WIDTH) -
        DIFF_MAIN_MIN_WIDTH,
    );
    setDiffNavigatorWidth(
      Math.round(Math.min(Math.max(width, DIFF_NAVIGATOR_MIN_WIDTH), max)),
    );
  }

  function startDiffNavigatorResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    diffNavigatorResizeStartRef.current = {
      clientX: event.clientX,
      width: diffNavigatorWidth,
    };
    setDiffNavigatorResizing(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function moveDiffNavigatorResize(event: ReactPointerEvent<HTMLDivElement>) {
    const start = diffNavigatorResizeStartRef.current;
    if (!start) return;
    resizeDiffNavigator(start.width + event.clientX - start.clientX);
  }

  function endDiffNavigatorResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (!diffNavigatorResizeStartRef.current) return;
    diffNavigatorResizeStartRef.current = null;
    setDiffNavigatorResizing(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function resizeDiffNavigatorWithKeyboard(
    event: ReactKeyboardEvent<HTMLDivElement>,
  ) {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      resizeDiffNavigator(diffNavigatorWidth - DIFF_NAVIGATOR_KEYBOARD_STEP);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      resizeDiffNavigator(diffNavigatorWidth + DIFF_NAVIGATOR_KEYBOARD_STEP);
    } else if (event.key === "Home") {
      event.preventDefault();
      resizeDiffNavigator(DIFF_NAVIGATOR_MIN_WIDTH);
    } else if (event.key === "End") {
      event.preventDefault();
      resizeDiffNavigator(Number.MAX_SAFE_INTEGER);
    }
  }

  function stepDiffSearch(delta: number) {
    if (!diffSearchMatches.length) return;
    setDiffSearchIndex(
      (activeDiffSearchIndex + delta + diffSearchMatches.length) %
        diffSearchMatches.length,
    );
  }

  function openDiffScopeMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    openContextMenu(
      event,
      DIFF_SCOPE_OPTIONS.map((option) => ({
        id: option.value,
        label: option.label,
        checked: option.value === diffScope,
        disabled:
          commandBusy === "diff" ||
          (option.value === "commit" && !recentCommits.length) ||
          (option.value === "branch" && !comparisonBranches.length),
        action: () => selectDiffScope(option.value),
      })),
      "Diff scope",
    );
  }

  function openDiffBaseMenu(event: ReactMouseEvent<HTMLButtonElement>) {
    const items =
      diffScope === "commit"
        ? recentCommits.map((commit) => ({
            id: commit.hash,
            label: commit.subject,
            detail: commit.hash.slice(0, 7),
            checked: commit.hash === diffBase,
            action: () => selectDiffBase(commit.hash),
          }))
        : comparisonBranches.map((branch) => ({
            id: branch.name,
            label: branch.name,
            detail: branchMetaLabel(branch),
            checked: branch.name === diffBase,
            action: () => selectDiffBase(branch.name),
          }));
    openContextMenu(event, items, diffScope === "commit" ? "Commit" : "Branch");
  }

  function renderDiffToolbar() {
    const scopeLabel =
      DIFF_SCOPE_OPTIONS.find((option) => option.value === diffScope)?.label ??
      "All changes";
    const baseLabel =
      diffScope === "commit"
        ? recentCommits.find((commit) => commit.hash === diffBase)?.subject
        : diffBase;
    return (
      <div className="git-diff-toolbar">
        <div className="git-diff-scope-controls">
          {!diffNavigatorVisible && navigableFiles.length > 0 && (
            <button
              type="button"
              className="git-icon-btn"
              title="Show changed files"
              aria-label="Show changed files"
              aria-controls="git-changed-files"
              aria-expanded={false}
              onClick={() => setDiffNavigatorVisible(true)}
            >
              <Sidebar size={13} />
            </button>
          )}
          <button
            type="button"
            className="git-diff-scope-trigger"
            aria-label="Diff scope"
            aria-haspopup="menu"
            disabled={commandBusy === "diff"}
            onClick={openDiffScopeMenu}
          >
            <span>{scopeLabel}</span>
            <ChevronDown size={11} />
          </button>
          {(diffScope === "commit" || diffScope === "branch") && baseLabel && (
            <button
              type="button"
              className="git-diff-scope-trigger git-diff-base-trigger"
              aria-label={diffScope === "commit" ? "Commit to review" : "Branch to compare"}
              aria-haspopup="menu"
              disabled={commandBusy === "diff"}
              title={baseLabel}
              onClick={openDiffBaseMenu}
            >
              <span>{baseLabel}</span>
              <ChevronDown size={11} />
            </button>
          )}
          <div className="git-diff-stats" aria-label="Diff summary">
            {commandBusy === "diff" ? (
              <span>Loading...</span>
            ) : (
              <>
                <span>{scopeStats.files} files</span>
                <span className="add">+{scopeStats.additions}</span>
                <span className="delete">-{scopeStats.deletions}</span>
              </>
            )}
          </div>
        </div>
        <label className="git-diff-search">
          <Search size={12} />
          <input
            value={diffSearch}
            placeholder="Search diff"
            disabled={!diffResult}
            onChange={(event) => {
              setDiffSearch(event.currentTarget.value);
              setDiffSearchIndex(0);
            }}
          />
          <span>
            {diffSearchMatches.length
              ? `${activeDiffSearchIndex + 1}/${diffSearchMatches.length}`
              : "0/0"}
          </span>
          <button
            type="button"
            disabled={!diffSearchMatches.length}
            onClick={() => stepDiffSearch(-1)}
          >
            Prev
          </button>
          <button
            type="button"
            disabled={!diffSearchMatches.length}
            onClick={() => stepDiffSearch(1)}
          >
            Next
          </button>
        </label>
        <div className="git-diff-head-actions">
          {renderedDiffSections.length > 0 && (
            <button
              type="button"
              className="git-diff-bulk-toggle"
              onClick={toggleAllDiffSections}
            >
              {allDiffSectionsCollapsed ? "Expand all" : "Collapse all"}
            </button>
          )}
        </div>
      </div>
    );
  }

  function renderDiffView() {
    const addReviewComment = (row: DiffRow, index: number, section: DiffSection | undefined, shift: boolean) => {
      if (!section || !["add", "delete", "context"].includes(row.kind)) return;
      let start = index;
      if (shift && reviewAnchor?.sectionId === section.id) start = Math.min(reviewAnchor.index, index);
      const end = shift && reviewAnchor?.sectionId === section.id ? Math.max(reviewAnchor.index, index) : index;
      const rows = renderedDiffRows.slice(start, end + 1).filter((candidate) => ["add", "delete", "context"].includes(candidate.kind));
      const body = window.prompt("Review comment");
      if (!body?.trim()) { setReviewAnchor({ sectionId: section.id, index }); return; }
      const side = row.kind === "delete" ? "old" : "new";
      const lineNumbers = rows
        .map((candidate) => Number(side === "old" ? candidate.oldNo : candidate.newNo))
        .filter((value) => Number.isFinite(value) && value > 0);
      window.dispatchEvent(new CustomEvent("milim:add-review-comment", { detail: {
        id: `review-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        surface: "diff",
        filePath: section.path,
        side,
        startLine: Math.min(...lineNumbers),
        endLine: Math.max(...lineNumbers),
        selectedText: rows.map((candidate) => candidate.text).join("\n"),
        body: body.trim(),
        timestamp: Date.now(),
      } }));
      setReviewAnchor(null);
    };
    return (
      <div
        className="git-diff-view"
        role="table"
        aria-label="Unified diff"
        ref={diffViewRef}
      >
        {visibleDiffRows.map(({ row, index, section, sectionIndex }) => {
          const collapsed = section
            ? collapsedDiffSections.has(section.id)
            : false;
          const matchIndex = diffSearchMatchIndexByRow.get(index);
          const searchActive =
            matchIndex === activeDiffSearchIndex &&
            diffSearchMatches.length > 0;
          return (
            <div
              className={`git-diff-row ${row.kind}${searchActive ? " search-active" : ""}`}
              role="row"
              key={`${index}:${row.kind}:${row.oldNo}:${row.newNo}`}
              data-section-index={
                row.kind === "file" ? sectionIndex : undefined
              }
              data-match-index={matchIndex}
              onClick={(event) => {
                if (["add", "delete", "context"].includes(row.kind) && section) {
                  setReviewAnchor({ sectionId: section.id, index });
                  if (event.shiftKey) addReviewComment(row, index, section, true);
                }
              }}
            >
              <span
                className="git-diff-gutter old"
                aria-label={row.oldNo ? `Old line ${row.oldNo}` : undefined}
              >
                {row.oldNo}
              </span>
              <span
                className="git-diff-gutter new"
                aria-label={row.newNo ? `New line ${row.newNo}` : undefined}
              >
                {row.newNo}
              </span>
              <span className="git-diff-marker" aria-hidden="true">
                {row.marker}
              </span>
              {row.kind === "file" && section ? (
                <button
                  className="git-diff-file-toggle"
                  type="button"
                  onClick={() => toggleDiffSection(section.id)}
                >
                  <span>{row.text}</span>
                  <small>
                    +{section.additions}/-{section.deletions} ֲ·{" "}
                    {section.lineCount} lines ֲ·{" "}
                    {collapsed ? "Expand" : "Collapse"}
                  </small>
                </button>
              ) : (
                <>
                  <code>
                    {renderDiffText(row, index, renderedDiffRows, diffSearch)}
                  </code>
                  {["add", "delete", "context"].includes(row.kind) && section ? (
                    <button
                      type="button"
                      className="git-diff-comment"
                      title="Add review comment (Shift-click another line for a range)"
                      onClick={(event) => {
                        event.stopPropagation();
                        addReviewComment(row, index, section, event.shiftKey);
                      }}
                    >+</button>
                  ) : null}
                </>
              )}
            </div>
          );
        })}
      </div>
    );
  }

  function toggleFileTreeFolder(path: string) {
    setCollapsedFileTreePaths((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }

  function renderFileTree(nodes: GitFileTreeNode[], depth = 0): ReactNode {
    return nodes.map((node) => {
      if (node.fileIndex === undefined) {
        const collapsed = collapsedFileTreePaths.has(node.path);
        return (
          <div className="git-file-tree-branch" role="none" key={node.path}>
            <button
              className="git-file-tree-folder"
              type="button"
              role="treeitem"
              aria-expanded={!collapsed}
              style={{ "--git-file-tree-depth": depth } as CSSProperties}
              onClick={() => toggleFileTreeFolder(node.path)}
            >
              <ChevronDown size={11} />
              <Folder size={12} />
              <span>{node.name}</span>
            </button>
            {!collapsed && (
              <div role="group">{renderFileTree(node.children, depth + 1)}</div>
            )}
          </div>
        );
      }

      const file = navigableFiles[node.fileIndex];
      if (!file) return null;
      return (
        <button
          className="git-file-row"
          type="button"
          role="treeitem"
          key={file.key}
          title={`View diff for ${file.title}`}
          aria-current={
            file.sectionId === activeDiffSectionId ? "location" : undefined
          }
          style={{ "--git-file-tree-depth": depth } as CSSProperties}
          disabled={Boolean(commandBusy)}
          onClick={() => openDiffFile({ path: file.path, index: file.index })}
        >
          <span className="git-file-status">{file.status}</span>
          <span className="git-file-path">{node.name}</span>
        </button>
      );
    });
  }

  function renderFileNavigator(inDiff = false) {
    if (!navigableFiles.length) return null;
    return (
      <div
        className={`git-file-list${inDiff ? " git-diff-navigator" : ""}`}
        aria-label="Changed files"
        id={inDiff ? "git-changed-files" : undefined}
      >
        {inDiff && (
          <div className="git-diff-navigator-head">
            <div>
              <button
                type="button"
                className="git-icon-btn"
                title="Hide changed files"
                aria-label="Hide changed files"
                aria-controls="git-changed-files"
                onClick={() => setDiffNavigatorVisible(false)}
              >
                <Sidebar size={13} />
              </button>
              <strong>Changes</strong>
            </div>
            <span>{navigableFiles.length}</span>
          </div>
        )}
        {inDiff
          ? <div className="git-file-tree" role="tree">{renderFileTree(navigableFileTree)}</div>
          : navigableFiles.map((file) => (
              <button
                className="git-file-row"
                type="button"
                key={file.key}
                title={`View diff for ${file.title}`}
                disabled={Boolean(commandBusy)}
                onClick={() =>
                  openDiffFile({ path: file.path, index: file.index })
                }
              >
                <span className="git-file-status">{file.status}</span>
                <span className="git-file-path">{compactPath(file.path)}</span>
              </button>
            ))}
      </div>
    );
  }

  function renderDiffContent() {
    return (
      <div
        ref={diffLayoutRef}
        className={`git-diff-layout${diffNavigatorVisible ? "" : " files-hidden"}`}
        style={
          {
            "--git-diff-navigator-width": `${diffNavigatorWidth}px`,
          } as CSSProperties
        }
      >
        {diffNavigatorVisible && renderFileNavigator(true)}
        {diffNavigatorVisible && navigableFiles.length > 0 && (
          <div
            className={`git-diff-resize-handle${diffNavigatorResizing ? " dragging" : ""}`}
            data-testid="git-diff-resize-handle"
            role="separator"
            aria-label="Resize changed files"
            aria-orientation="vertical"
            aria-valuemin={DIFF_NAVIGATOR_MIN_WIDTH}
            aria-valuemax={Math.max(
              DIFF_NAVIGATOR_MIN_WIDTH,
              (diffLayoutRef.current?.clientWidth ??
                DIFF_NAVIGATOR_DEFAULT_WIDTH + DIFF_MAIN_MIN_WIDTH) -
                DIFF_MAIN_MIN_WIDTH,
            )}
            aria-valuenow={diffNavigatorWidth}
            tabIndex={0}
            onKeyDown={resizeDiffNavigatorWithKeyboard}
            onPointerDown={startDiffNavigatorResize}
            onPointerMove={moveDiffNavigatorResize}
            onPointerUp={endDiffNavigatorResize}
            onPointerCancel={endDiffNavigatorResize}
          />
        )}
        <div className="git-diff-main">
          {renderDiffToolbar()}
          {diffResult && visibleDiffRows.length ? (
            renderDiffView()
          ) : diffResult ? (
            <div className="git-diff-empty">{diffResult.message}</div>
          ) : (
            <div className="git-diff-empty">
              Select a changed file or diff scope to start reviewing.
            </div>
          )}
        </div>
      </div>
    );
  }

  async function runGitCommand(
    action: WorkspaceGitAction,
    options: {
      message?: string;
      stage_all?: boolean;
      branch?: string;
      diff_scope?: WorkspaceGitDiffScope;
      diff_base?: string;
    } = {},
  ) {
    if (commandBusy) return;

    setCommandBusy(action);
    setCommandMenu(null);
    setNotice(null);
    if (action === "diff") {
      setDiffResult(null);
      setActiveDiffSectionId(null);
    }
    try {
      const result = await runWorkspaceGitAction(action, options);
      if (action === "diff") {
        setDiffResult(result);
        setCommandResult(null);
      } else {
        setCommandResult(result);
        if (result.ok && (action === "commit" || action === "commit_push")) {
          setCommitMessage("");
          setStageAll(true);
        }
        if (action === "checkout_branch" || action === "create_branch") {
          setNotice(result.ok ? null : result.message);
          if (result.ok) {
            setBranchMenuOpen(false);
            setBranchFilter("");
            setNewBranchName("");
          }
        }
        forceRefreshGitStatus();
      }
    } catch (error) {
      if (action === "diff") {
        setDiffResult(null);
        setPendingDiffFile(null);
      } else setCommandResult(null);
      setNotice(error instanceof Error ? error.message : "Git command failed");
    } finally {
      setCommandBusy(null);
    }
  }

  async function createOrOpenPullRequest() {
    if (!readyStatus) return;
    setCommandBusy("pr_status");
    try {
      const existing = await runWorkspaceGitAction("pr_status");
      if (existing.ok) {
        const summary = JSON.parse(existing.stdout) as { url?: string };
        if (summary.url) await openExternalUrl(summary.url);
        return;
      }
      if (!existing.message.includes("No pull request")) throw new Error(existing.message);
      const prefill = JSON.parse(existing.stdout || "{}") as { title?: string; baseRefName?: string };
      const title = window.prompt("Pull request title", prefill.title || readyStatus.recent_commits[0]?.subject || readyStatus.branch || "Changes");
      if (!title?.trim()) return;
      const base = window.prompt("Base branch", prefill.baseRefName || "main");
      if (!base?.trim()) return;
      const body = window.prompt("Pull request body", "") ?? "";
      const created = await runWorkspaceGitAction("pr_create", {
        title: title.trim(), body, base: base.trim(), draft: true,
      });
      setCommandResult(created);
      if (!created.ok) throw new Error(created.message);
      const url = created.stdout.trim().split(/\s+/).find((value) => /^https?:\/\//.test(value));
      if (url) await openExternalUrl(url);
    } catch (error) {
      setNotice(error instanceof Error ? error.message : String(error));
    } finally {
      setCommandBusy(null);
    }
  }

  function openCommandMenu(action: WorkspaceGitAction) {
    if (commandBusy) return;
    setCommandMenu((current) => (current === action ? null : action));
    setNotice(null);
  }

  async function runCommit(push: boolean) {
    let message = commitMessage.trim();
    if (!message) {
      setGeneratingCommitMessage(true);
      setNotice(null);
      try {
        const diff = await runWorkspaceGitAction("diff", {
          staged_only: !stageAll,
        });
        const output = actionOutputText(diff);
        if (!output.trim()) {
          setNotice("No diff available for a commit message.");
          return;
        }
        message = await generateCommitMessageWithFallback(
          model,
          selectedFolder,
          readyStatus,
          output,
        );
        setCommitMessage(message);
      } catch (error) {
        setNotice(
          error instanceof Error
            ? error.message
            : "Couldn't generate a commit message.",
        );
        return;
      } finally {
        setGeneratingCommitMessage(false);
      }
    }
    void runGitCommand(push ? "commit_push" : "commit", {
      message,
      stage_all: stageAll,
    });
  }

  if (!expanded) {
    return (
      <section
        className={`git-panel git-panel-collapsed${onOpenPanel ? " git-panel-launcher" : ""}`}
        data-testid="git-panel"
        aria-label="Git environment"
      >
        <div className="git-panel-collapsed-row">
          <button
            type="button"
            className="git-panel-toggle"
            data-testid="git-panel-toggle"
            aria-expanded={false}
            title={`${onOpenPanel ? "Open" : "Expand"} Git panel for ${repoName}`}
            onClick={() => (onOpenPanel ? onOpenPanel() : setExpanded(true))}
          >
            <span className="git-panel-mark" aria-hidden="true">
              <GitLogo size={16} />
            </span>
            <span className="git-panel-toggle-copy">
              <strong>{repoName}</strong>
              <span>{branchLabel(readyStatus)}</span>
            </span>
            <span
              className={`git-panel-toggle-stat ${changeTone(readyStatus)}`}
            >
              {compactChangeLabel(readyStatus)}
            </span>
            <ChevronDown size={13} className="git-panel-chevron" />
          </button>
          <button
            type="button"
            className="git-icon-btn"
            data-testid="git-refresh"
            title="Refresh Git status"
            aria-label="Refresh Git status"
            onClick={forceRefreshGitStatus}
          >
            <Refresh size={13} />
          </button>
        </div>
      </section>
    );
  }

  return (
    <>
      <section
        className={`git-panel${forceExpanded ? " git-panel-workspace" : ""}`}
        data-testid="git-panel"
        aria-label="Git environment"
        onContextMenu={openGitContextMenu}
      >
        <div className="git-panel-head">
          <button
            type="button"
            className="git-panel-title"
            data-testid="git-panel-toggle"
            aria-expanded={true}
            title={`${forceExpanded ? "Git panel" : "Collapse Git panel"} for ${repoName}`}
            onClick={() => {
              if (!forceExpanded) setExpanded(false);
            }}
          >
            <span className="git-panel-mark" aria-hidden="true">
              <GitLogo size={16} />
            </span>
            <span className="git-panel-title-copy">
              <strong title={readyStatus.root || selectedFolder}>
                {repoName}
              </strong>
            </span>
            <ChevronDown size={13} className="git-panel-chevron expanded" />
          </button>
          <button
            type="button"
            className="git-icon-btn"
            data-testid="git-refresh"
            title="Refresh Git status"
            aria-label="Refresh Git status"
            onClick={forceRefreshGitStatus}
          >
            <Refresh size={13} />
          </button>
        </div>

        <div className="git-panel-subhead">
          <button
            className="git-branch-chip"
            type="button"
            data-testid={
              forceExpanded ? "git-branch-selector" : "git-copy-branch"
            }
            onClick={() => {
              if (!forceExpanded) {
                void copyBranch();
                return;
              }
              setNotice(null);
              setBranchMenuOpen(true);
            }}
            title={forceExpanded ? "Switch or create branch" : "Copy branch"}
          >
            <GitBranch size={12} />
            <span>{branchLabel(readyStatus)}</span>
            {forceExpanded ? <ChevronDown size={11} /> : <Copy size={11} />}
          </button>
          {updated && <span className="git-panel-updated">{updated}</span>}
        </div>

        <div
          className={`git-summary ${changeTone(readyStatus)}`}
          data-testid="git-changes-row"
        >
          <span className="git-summary-main">
            <GitCommit size={14} />
            <strong title={changeSummary}>{changeSummary}</strong>
          </span>
          {diffLabel(readyStatus) && (
            <span className="git-summary-diff">
              {renderDiffStat(readyStatus)}
            </span>
          )}
        </div>

        <div className="git-panel-details">
          <button
            className="git-row"
            type="button"
            onClick={openFolder}
            title={readyStatus.root || selectedFolder}
          >
            <Folder size={14} />
            <span className="git-row-label">Local</span>
            <span className="git-row-value">
              {compactPath(readyStatus.root || selectedFolder)}
            </span>
          </button>

          {remoteUrl ? (
            <button
              className="git-row"
              type="button"
              onClick={openRemote}
              title={remoteUrl}
            >
              <GitRemote size={14} />
              <span className="git-row-label">Remote</span>
              <span className="git-row-value">{syncLabel(readyStatus)}</span>
            </button>
          ) : (
            <div
              className="git-row git-row-static"
              title={readyStatus.upstream || readyStatus.remote || undefined}
            >
              <GitRemote size={14} />
              <span className="git-row-label">Remote</span>
              <span className="git-row-value">{syncLabel(readyStatus)}</span>
            </div>
          )}
        </div>

        <div
          className="git-panel-actions git-command-actions"
          aria-label="Git commands"
        >
          {!forceExpanded && (
            <button
              className="git-action"
              type="button"
              data-testid="git-action-diff"
              title={
                readyStatus.has_changes
                  ? "Open diff panel"
                  : "No changes to diff"
              }
              disabled={!readyStatus.has_changes || Boolean(commandBusy)}
              onClick={() => void runGitCommand("diff")}
            >
              <Code size={13} />
              <span>{commandBusy === "diff" ? "Diff..." : "Diff"}</span>
            </button>
          )}

          <button
            className="git-action"
            type="button"
            data-testid="git-action-fetch"
            title={
              readyStatus.remote
                ? "Run git fetch --prune"
                : "No remote configured"
            }
            disabled={!readyStatus.remote || Boolean(commandBusy)}
            onClick={() => void runGitCommand("fetch")}
          >
            <GitRemote size={13} />
            <span>{commandBusy === "fetch" ? "Fetch..." : "Fetch"}</span>
          </button>

          {(!forceExpanded || readyStatus.has_changes) && (
            <button
              className={`git-action ${commandMenu === "commit" ? "active" : ""}`}
              type="button"
              data-testid="git-action-commit"
              title={
                readyStatus.conflicts
                  ? "Resolve conflicts before committing"
                  : readyStatus.has_changes
                    ? "Open commit commands"
                    : "No changes to commit"
              }
              disabled={
                !readyStatus.has_changes ||
                readyStatus.conflicts > 0 ||
                Boolean(commandBusy)
              }
              onClick={() => openCommandMenu("commit")}
            >
              <GitCommit size={13} />
              <span>Commit</span>
            </button>
          )}

          {forceExpanded && (
            <button
              className="git-action"
              type="button"
              title="Ask the agent to review these changes"
              onClick={() =>
                onDraftAction(agentReviewPrompt(selectedFolder, readyStatus))
              }
            >
              <Lightbulb size={13} />
              <span>Agent review</span>
            </button>
          )}

          {forceExpanded && readyStatus.remote && (
            <button
              className="git-action"
              type="button"
              title="Open the current GitHub PR or create a draft PR"
              disabled={Boolean(commandBusy)}
              onClick={() => void createOrOpenPullRequest()}
            >
              <GitLogo size={13} />
              <span>{commandBusy === "pr_status" ? "PR..." : "Pull request"}</span>
            </button>
          )}

          {sync && (
            <button
              className={`git-action git-action-sync ${commandMenu === sync.action ? "active" : ""}`}
              type="button"
              data-testid="git-action-sync"
              title={sync.title}
              disabled={sync.disabled || Boolean(commandBusy)}
              onClick={() => openCommandMenu(sync.action)}
            >
              <SyncIcon size={13} />
              <span>
                {commandBusy === sync.action ? `${sync.label}...` : sync.label}
              </span>
            </button>
          )}

          {forceExpanded && commandResult && (
            <div
              className={`git-command-status ${commandResult.ok ? "" : "error"}`}
              role="status"
              title={
                commandOutput || commandResult.command || commandResult.message
              }
            >
              <span>{commandResult.message}</span>
            </div>
          )}
        </div>

        {!forceExpanded && commandResult && (
          <div
            className={`git-command-status ${commandResult.ok ? "" : "error"}`}
            role="status"
            title={
              commandOutput || commandResult.command || commandResult.message
            }
          >
            <span>{commandResult.message}</span>
          </div>
        )}

        {forceExpanded && (
          <section
            className={`git-workspace-review ${diffResult && !diffResult.ok ? "error" : ""}`}
            aria-label="Git review"
          >
            {renderDiffContent()}
          </section>
        )}

        {!forceExpanded && (
          <button
            className="git-agent-action"
            type="button"
            onClick={() =>
              onDraftAction(agentReviewPrompt(selectedFolder, readyStatus))
            }
          >
            <Lightbulb size={13} />
            <span>Ask agent to review</span>
          </button>
        )}

        {notice && (
          <div className="git-panel-note" role="status">
            {notice}
          </div>
        )}

      </section>
      {typeof document !== "undefined" &&
        branchMenuOpen &&
        createPortal(
          <div
            className="git-modal-backdrop"
            data-native-preview-blocker="true"
            onMouseDown={(event) =>
              event.target === event.currentTarget && setBranchMenuOpen(false)
            }
          >
            <section
              className="git-modal git-branch-modal"
              role="dialog"
              aria-modal="true"
              aria-label="Switch branch"
            >
              <div className="git-modal-head">
                <span>
                  <GitBranch size={13} />
                  <strong>Branch</strong>
                </span>
                <button
                  type="button"
                  className="git-icon-btn"
                  title="Close"
                  aria-label="Close"
                  onClick={() => setBranchMenuOpen(false)}
                >
                  <X size={13} />
                </button>
              </div>
              <label className="git-branch-search">
                <Search size={12} />
                <input
                  value={branchFilter}
                  placeholder="Search branches..."
                  onChange={(event) =>
                    setBranchFilter(event.currentTarget.value)
                  }
                />
              </label>
              <div className="git-branch-list" aria-label="Local branches">
                {filteredBranches.length ? (
                  filteredBranches.map((branch) => (
                    <button
                      type="button"
                      className={branch.current ? "active" : ""}
                      key={branch.name}
                      disabled={branch.current || Boolean(commandBusy)}
                      onClick={() =>
                        void runGitCommand("checkout_branch", {
                          branch: branch.name,
                        })
                      }
                      title={branch.name}
                    >
                      <span>
                        <GitBranch size={12} />
                        <strong>{branch.name}</strong>
                      </span>
                      <small>{branchMetaLabel(branch)}</small>
                    </button>
                  ))
                ) : (
                  <div className="git-branch-empty">No branches found.</div>
                )}
              </div>
              <div className="git-branch-create">
                <input
                  type="text"
                  value={newBranchName}
                  placeholder="New branch from HEAD"
                  disabled={Boolean(commandBusy)}
                  onChange={(event) =>
                    setNewBranchName(event.currentTarget.value)
                  }
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && canCreateBranch)
                      void runGitCommand("create_branch", {
                        branch: nextBranchName,
                      });
                  }}
                />
                <button
                  type="button"
                  disabled={!canCreateBranch || Boolean(commandBusy)}
                  onClick={() =>
                    void runGitCommand("create_branch", {
                      branch: nextBranchName,
                    })
                  }
                >
                  <GitBranch size={12} />
                  <span>
                    {commandBusy === "create_branch" ? "Creating..." : "Create"}
                  </span>
                </button>
              </div>
              {notice && (
                <div className="git-command-modal-note" role="status">
                  {notice}
                </div>
              )}
            </section>
          </div>,
          document.body,
        )}
      {typeof document !== "undefined" &&
        commandMenu &&
        createPortal(
          <div
            className="git-modal-backdrop"
            data-native-preview-blocker="true"
            onMouseDown={(event) =>
              event.target === event.currentTarget && setCommandMenu(null)
            }
          >
            <section
              className="git-modal git-command-modal"
              role="dialog"
              aria-modal="true"
              aria-label={`${actionLabel(commandMenu)} command`}
            >
              <div className="git-modal-head">
                <span>
                  <GitBranch size={13} />
                  <strong>{actionLabel(commandMenu)}</strong>
                </span>
                <button
                  type="button"
                  className="git-icon-btn"
                  title="Close"
                  aria-label="Close"
                  onClick={() => setCommandMenu(null)}
                >
                  <X size={13} />
                </button>
              </div>
              <div className="git-command-modal-meta">
                <span>{branchLabel(readyStatus)}</span>
                <strong>{renderDiffStat(readyStatus) || changeSummary}</strong>
              </div>

              {commandMenu === "commit" ? (
                <>
                  <input
                    className="git-commit-input"
                    type="text"
                    value={commitMessage}
                    placeholder="Commit message (blank generates)"
                    disabled={Boolean(commandBusy) || generatingCommitMessage}
                    onChange={(event) =>
                      setCommitMessage(event.currentTarget.value)
                    }
                  />
                  <label className="git-check-row">
                    <input
                      type="checkbox"
                      checked={stageAll}
                      disabled={Boolean(commandBusy) || generatingCommitMessage}
                      onChange={(event) =>
                        setStageAll(event.currentTarget.checked)
                      }
                    />
                    <span>Include unstaged changes</span>
                  </label>
                  <code className="git-command-preview">
                    {commandPreview(
                      "commit",
                      readyStatus,
                      commitMessage,
                      stageAll,
                    )}
                  </code>
                  {notice && (
                    <div className="git-command-modal-note" role="status">
                      {notice}
                    </div>
                  )}
                  <div className="git-command-options">
                    <button
                      type="button"
                      disabled={
                        !commitReady ||
                        Boolean(commandBusy) ||
                        generatingCommitMessage
                      }
                      onClick={() => void runCommit(false)}
                    >
                      <GitCommit size={13} />
                      <span>
                        {generatingCommitMessage
                          ? "Generating..."
                          : commandBusy === "commit"
                            ? "Committing..."
                            : "Commit"}
                      </span>
                    </button>
                    <button
                      type="button"
                      disabled={
                        !commitReady ||
                        !canCommitPush ||
                        Boolean(commandBusy) ||
                        generatingCommitMessage
                      }
                      title={
                        canCommitPush
                          ? "Commit, then push"
                          : "Remote branch required and not behind"
                      }
                      onClick={() => void runCommit(true)}
                    >
                      <ArrowUp size={13} />
                      <span>
                        {generatingCommitMessage
                          ? "Generating..."
                          : commandBusy === "commit_push"
                            ? "Pushing..."
                            : "Commit and push"}
                      </span>
                    </button>
                  </div>
                </>
              ) : (
                <>
                  <code className="git-command-preview">
                    {commandPreview(
                      commandMenu,
                      readyStatus,
                      commitMessage,
                      stageAll,
                    )}
                  </code>
                  <div className="git-command-options">
                    <button
                      type="button"
                      disabled={Boolean(commandBusy)}
                      onClick={() => void runGitCommand(commandMenu)}
                    >
                      {commandMenu === "pull" ? (
                        <Download size={13} />
                      ) : commandMenu === "push" ? (
                        <ArrowUp size={13} />
                      ) : (
                        <GitBranch size={13} />
                      )}
                      <span>
                        {commandBusy === commandMenu
                          ? `${actionLabel(commandMenu)}...`
                          : `Run ${actionLabel(commandMenu)}`}
                      </span>
                    </button>
                  </div>
                </>
              )}
            </section>
          </div>,
          document.body,
        )}
      {typeof document !== "undefined" &&
        diffResult &&
        !forceExpanded &&
        createPortal(
          <div
            className="git-modal-backdrop"
            data-native-preview-blocker="true"
            onMouseDown={(event) =>
              event.target === event.currentTarget && setDiffResult(null)
            }
          >
            <section
              className={`git-modal git-diff-panel ${diffResult.ok ? "" : "error"}`}
              role="dialog"
              aria-modal="true"
              aria-label="Git diff"
            >
              <div className="git-modal-head">
                <span>
                  <Code size={13} />
                  <strong>{diffResult.message}</strong>
                </span>
                <button
                  type="button"
                  className="git-icon-btn"
                  title="Close diff"
                  aria-label="Close diff"
                  onClick={() => setDiffResult(null)}
                >
                  <X size={13} />
                </button>
              </div>
              {diffResult.command && (
                <code className="git-command-preview">
                  {diffResult.command}
                </code>
              )}
              {visibleDiffRows.length > 0 ? (
                <>
                  <div className="git-diff-toolbar">
                    <div className="git-diff-stats" aria-label="Diff summary">
                      <span>{renderedDiffStats.files} files</span>
                      <span className="add">
                        +{renderedDiffStats.additions}
                      </span>
                      <span className="delete">
                        -{renderedDiffStats.deletions}
                      </span>
                      <span>Worktree</span>
                    </div>
                    <label className="git-diff-search">
                      <Search size={12} />
                      <input
                        value={diffSearch}
                        placeholder="Search diff"
                        onChange={(event) => {
                          setDiffSearch(event.currentTarget.value);
                          setDiffSearchIndex(0);
                        }}
                      />
                      <span>
                        {diffSearchMatches.length
                          ? `${activeDiffSearchIndex + 1}/${diffSearchMatches.length}`
                          : "0/0"}
                      </span>
                      <button
                        type="button"
                        disabled={!diffSearchMatches.length}
                        onClick={() => stepDiffSearch(-1)}
                      >
                        Prev
                      </button>
                      <button
                        type="button"
                        disabled={!diffSearchMatches.length}
                        onClick={() => stepDiffSearch(1)}
                      >
                        Next
                      </button>
                    </label>
                  </div>

                  <div
                    className="git-diff-view"
                    role="table"
                    aria-label="Unified diff"
                    ref={diffViewRef}
                  >
                    {visibleDiffRows.map(
                      ({ row, index, section, sectionIndex }) => {
                        const collapsed = section
                          ? collapsedDiffSections.has(section.id)
                          : false;
                        const matchIndex = diffSearchMatchIndexByRow.get(index);
                        const searchActive =
                          matchIndex === activeDiffSearchIndex &&
                          diffSearchMatches.length > 0;
                        return (
                          <div
                            className={`git-diff-row ${row.kind}${searchActive ? " search-active" : ""}`}
                            role="row"
                            key={`${index}:${row.kind}:${row.oldNo}:${row.newNo}`}
                            data-section-index={
                              row.kind === "file" ? sectionIndex : undefined
                            }
                            data-match-index={matchIndex}
                          >
                            <span
                              className="git-diff-gutter old"
                              aria-label={
                                row.oldNo ? `Old line ${row.oldNo}` : undefined
                              }
                            >
                              {row.oldNo}
                            </span>
                            <span
                              className="git-diff-gutter new"
                              aria-label={
                                row.newNo ? `New line ${row.newNo}` : undefined
                              }
                            >
                              {row.newNo}
                            </span>
                            <span
                              className="git-diff-marker"
                              aria-hidden="true"
                            >
                              {row.marker}
                            </span>
                            {row.kind === "file" && section ? (
                              <button
                                className="git-diff-file-toggle"
                                type="button"
                                onClick={() => toggleDiffSection(section.id)}
                              >
                                <span>{row.text}</span>
                                <small>
                                  +{section.additions}/-{section.deletions} ·{" "}
                                  {section.lineCount} lines ·{" "}
                                  {collapsed ? "Expand" : "Collapse"}
                                </small>
                              </button>
                            ) : (
                              <code>
                                {renderDiffText(
                                  row,
                                  index,
                                  renderedDiffRows,
                                  diffSearch,
                                )}
                              </code>
                            )}
                          </div>
                        );
                      },
                    )}
                  </div>
                </>
              ) : (
                <div className="git-diff-empty">No diff output.</div>
              )}
            </section>
          </div>,
          document.body,
        )}
    </>
  );
}

export function GitWorkspacePanel({
  folder,
  model,
  onDraftAction,
  diffRequest,
  closing = false,
  noEnterMotion = false,
  onClose,
  modeSwitcher,
  style,
  headerNotice,
}: {
  folder: string;
  model: string;
  onDraftAction: (text: string) => void;
  diffRequest?: GitPanelDiffRequest | null;
  closing?: boolean;
  noEnterMotion?: boolean;
  onClose: () => void;
  modeSwitcher?: ReactNode;
  style?: CSSProperties;
  headerNotice?: ReactNode;
}) {
  return (
    <aside
      className={`preview-panel git-workspace-panel${closing ? " closing" : ""}${noEnterMotion ? " no-enter" : ""}`}
      data-testid="git-workspace-panel"
      style={style}
    >
      <div className="preview-toolbar git-workspace-toolbar">
        {modeSwitcher}
        <div className="preview-actions" aria-label="Git panel actions">
          <button
            className="preview-action"
            title="Close Git panel"
            aria-label="Close Git panel"
            onClick={onClose}
          >
            <Sidebar size={16} />
          </button>
        </div>
      </div>
      <div className="git-workspace-scroll">
        {headerNotice}
        <GitPanel
          folder={folder}
          model={model}
          onDraftAction={onDraftAction}
          forceExpanded
          diffRequest={diffRequest}
        />
      </div>
    </aside>
  );
}
