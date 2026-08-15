import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type SetStateAction,
} from "react";
import { useAgents } from "../agents/store";
import {
  artifactFileStatus,
  applyWorkerDiff,
  claudeRuntimeModel,
  completeChat,
  completeChatWithMetrics,
  requestComposerCompletion,
  codexRuntimeModel,
  inferAttachmentMime,
  createControlCommandId,
  getControlBootstrap,
  getClaudeStatus,
  getOpenCodeStatus,
  getPiStatus,
  getCodexAccount,
  getWorkspaceGitStatus,
  getWorkerRun,
  getWorkerDiff,
  isClaudeModel,
  accountRuntimeKind,
  isCliPathWarningMessage,
  isCodexModel,
  isOpenCodeModel,
  isPiModel,
  listWorkspaceFiles,
  listModelsDetailed,
  listProviders,
  listTools,
  listWorkerRuns,
  MAX_ATTACHMENT_BYTES,
  openArtifactLocation,
  openDiagnosticsFolder,
  openExternalUrl,
  pickAttachmentFiles,
  pollScheduleRunEvents,
  previewArtifactFile,
  readWorkspaceAttachmentFile,
  resolveToolApproval,
  retryWorkerTask,
  deleteWorkerRun,
  runWorkspaceGitAction,
  saveArtifactFile,
  sendControlCommand,
  searchGraphMemory,
  selectSkills,
  setComputerUse,
  setPrivacyMode,
  setWorkspace,
  startWorkerRun,
  stopChildThread,
  stopWorkerRun,
  stopWorker,
  streamAgentRun,
  streamChat,
  streamChildThreadEvents,
  streamWorkerRunEvents,
  streamHarnessRun,
  streamCodexDeviceLogin,
  opencodeRuntimeModel,
  piRuntimeModel,
  wireMessageContent,
  type AgentEvent,
  type AgentToolContext,
  type AccountNativeWorkerLifecycle,
  type ArtifactFileStatus,
  type ArtifactOpenTarget,
  type ArtifactWritePreview,
  type ChatArtifact,
  type ChatAttachment,
  type ChatApprovalRequest,
  type ChatMessage,
  type ChatStreamPart,
  type ChildThreadInfo,
  type DelegationPolicy,
  type CodexLoginEvent,
  type HarnessEvent,
  type HarnessEventEnvelope,
  type HarnessRunRequest,
  type MediaGenerationResult,
  type MobileThreadGroup,
  type MobileThreadSummary,
  type MobileWorkerRunSnapshot,
  type MemoryNotice,
  type ModelInfo,
  type PreviewAppFile,
  type PreviewAppStatus,
  type PreviewAppStartOptions,
  type ReviewComment,
  type ReasoningEffort,
  type RunTrace,
  type SavedArtifactFile,
  type ScheduleRunEvent,
  type TokenUsage,
  type WorkspaceFileSuggestion,
  type WorkspaceCheckpoint,
  type WorkspaceGitActionResult,
  type WorkspaceGitStatus,
  type Worker,
  type WorkerRunRecord,
} from "../api";
import {
  DEFAULT_THREAD_SETTINGS,
  getSessionComposerDraft,
  inspectorStateForSession,
  setSessionComposerDraft,
  normalizeVirtualFilePath,
  sessionVirtualProjectFiles,
  useSessions,
  type Project,
  type QueuedMessage,
  type Session,
  type SessionBrowserSession,
  type SessionBrowserTab,
  type SessionPreviewRuntime,
  type SessionSidebarState,
  type SessionVirtualFile,
  type HotSwapAction,
  type NativeSessionMode,
} from "../sessions/store";
import {
  artifactPreviewAutoOpenKey,
  extractLivePreviewArtifactFromContent,
  extractLocalhostUrlFromRunTrace,
  hasPreviewPackageJson,
  isPreviewableArtifact,
  normalizeArtifactBrowserUrl,
  previewRuntimeBrowserUrl,
  previewRuntimeFiles,
} from "../lib/artifacts";
import {
  artifactOccurrenceKey,
  artifactRevisionChoiceByOccurrence,
  artifactRevisionGroups,
  type ArtifactRevision,
  type ArtifactRevisionGroup,
} from "../lib/artifactRevisions";
import {
  appendWorkerRunSynthesisOnce,
  rememberWorkerThreadEvent,
  workerRunReadyForSynthesis,
  workerRunSynthesisId,
} from "../lib/workerRuns";
import { readBrowserAttachmentDataUrl } from "../lib/attachmentInput";
import {
  buildEmptyStarterStrip,
  type EmptyStarterStrip,
  type EmptyStarterSuggestionIcon,
} from "../lib/emptyStarterSuggestions";
import {
  composerNoticeAction,
  modelComposerBlocker,
  prioritizeComposerNotice,
  type ComposerBlockerAction,
} from "../lib/composerBlocker";
import {
  checkpointMessage,
  compactionSummaryOutputCap,
  compactionSummaryMessages,
  compactionSummaryReasoningEffort,
  estimateMessagesTokens,
  isCompactionCheckpoint,
  messagesForModelContext,
  modelContextBudget,
  splitCompactionTail,
  validateCompactionCheckpointSummary,
} from "../lib/contextCompaction";
import {
  GIT_STATUS_REFRESH_INTERVAL_MS,
  shouldRefreshGitStatus,
} from "../lib/gitRefresh";
import { reasoningEffortForThread, reasoningEffortOverridesWithSelection } from "../lib/reasoningEffort";
import {
  buildQuickSummary,
  type QuickSummarySectionId,
  type QuickSummarySource,
} from "../lib/quickSummary";
import {
  nextRecentThreadSwitcherIndex,
  recentThreadSwitcherItems,
  rememberRecentThread,
  type RecentThreadSwitcherItem,
} from "../lib/recentThreads";
import {
  AI_THREAD_TITLE_SYSTEM_PROMPT,
  isThreadNamingModel,
  sanitizeAiThreadTitle,
  shouldReplaceThreadTitle,
} from "../lib/threadTitles";
import {
  chatExportFilename,
  exportedSessionCandidate,
  markdownSessionCandidate,
  sessionExportPayload,
  sessionMarkdownExport,
  type ThreadExportFormat,
} from "../lib/threadExport";
import {
  DEFAULT_GOAL_SETTINGS,
  applyGoalDecision,
  goalConfigured,
  goalContinuationPrompt,
  goalDecisionMessages,
  normalizeGoalSettings,
  parseGoalDecision,
  type GoalDecision,
  type GoalSettings,
} from "../lib/goals";
import { isNearScrollBottom } from "../lib/scroll";
import {
  mergeModelListsForPicker,
  providerOwnsModel,
} from "../lib/modelPicker";
import { assessHotSwap, type HotSwapAssessment } from "../lib/hotSwap";
import {
  approvalWaitDuration,
  estimateResponseCostUsd,
  responseMetricsForTurn,
  summarizeMilimUsage,
  summarizeThreadMetricsBreakdown,
} from "../lib/usageMetrics";
import { markPerfRender } from "../lib/perf";
import {
  previewControlActivityFromDebugUrl,
  previewControlActivityFromStreamParts,
} from "../lib/previewActivity";
import {
  previewRuntimeFoldersEqual,
  previewRuntimeKeyForThread,
} from "../lib/previewRuntimeKeys";
import { listenForPreviewOpenUrl } from "../lib/previewWebview";
import { statusPart } from "../lib/turnEvents";
import {
  accountRuntimeNotReadyForTurn,
  appendUserTurn,
  editResendConversation,
  prepareTurnOutbound,
  regenerateTurnConversation,
  resolveTurnSetup,
  type AccountRuntimeReady,
  type PrepareTurnOutboundOptions,
} from "../lib/turnContext";
import {
  looksLikeMcpToolRequest,
  looksLikeMemoryWriteRequest,
  looksLikeScheduleRequest,
  prepareTurnPromptContext,
  resolveTurnToolApproval,
} from "../lib/turnPrompt";
import {
  CLAUDE_SESSION_RECOVERY_REQUIRED,
  accountRuntimeInputFromMessages,
  createAgentRunEventHandler,
  createTurnAssistantStarter,
  createTurnMetricsCapture,
  createTurnRunTraceState,
  finalizeTurnRuntime,
  handleTurnRuntimeError,
  isGoogleWorkspaceEditTool,
  runModelChatTurn,
  runSelectedAccountRuntimeTurn,
  runToolAgentTurn,
  utilityAccountRuntimeMilimContext,
} from "../lib/turnRuntime";
import {
  autoApprovableToolApprovals,
  dismissToolApproval,
  pendingToolApprovals,
  toolApprovalPrompts,
} from "../lib/toolApproval";
import {
  hasQueuedMessages,
} from "../lib/turnQueue";
import {
  claimTurnGeneration,
  releaseTurnGeneration,
  startTurnStream,
} from "../lib/turnStream";
import { checkpointWorkspaceBeforeTurn } from "../lib/turnWorkspace";
import {
  controlAttachments,
  mergeControlRunMessages,
  pollControlRun,
  projectControlRunMessages,
} from "../lib/canonicalControl.js";
import { createChatMessageId } from "../lib/messageIds.js";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage";
import { useSettings } from "../settings/store";
import { themeCssVariables } from "../theme/applyTheme";
import { useTheme } from "../theme/store";
import { shortcutLabel, shortcutMatchesEvent } from "../ui/shortcuts";
import { sendMilimNotification } from "../lib/nativeNotifications";
import { createInteractiveChat } from "../lib/newChatCoordinator";
import { requestWorkspaceEditorLeave } from "../lib/workspaceEditorGuard";
import { isLoopbackProviderEndpoint } from "../lib/providerEndpoint.js";
import { pendingAttentionKey, playInterfaceSound } from "../ui/sounds";
import { DEFAULT_PREVIEW_PANEL_WIDTH, useUiPreferences } from "../ui/store";
import { Composer } from "./Composer";
import { ComposerSurface } from "./ComposerSurface";
import { ControlBar } from "./ControlBar";
import type { ModelPickerSelection } from "./ModelPicker";
import { GoalPanel, type GoalPanelDraft } from "./GoalPanel";
import {
  ArrowRight,
  ChevronDown,
  Code,
  Eye,
  FileText,
  GitBranch,
  Pencil,
  Refresh,
  Sidebar as PanelIcon,
  UserRound,
  X,
} from "./icons";
import { groupSessionsByProjects } from "./Sidebar";
import { InlineMediaControls } from "./InlineMediaControls";
import { WorkersInspector, WorkersSummary } from "./WorkersInspector";
import { ToolApprovalPrompt } from "./ToolApprovalPrompt";
import { CommandPalette, type RuntimeCommand } from "./ChatSearchPopover";
import { useContextMenu } from "./ContextMenu";
import {
  GitWorkspacePanel,
  type GitPanelDiffRequest,
  type GitPanelView,
} from "./GitPanel";
import { PreviewPanel } from "./PreviewPanel";
import { QuickSummaryPanel } from "./QuickSummaryPanel";
import {
  turnReviewFromDiff,
  type TurnReviewState,
} from "./TurnChangesCard";
import { SheetDialog } from "./SheetDialog";
import { WorkspaceLauncherButton } from "./WorkspaceLauncher";
import { BatonTargetSheet, HotSwapPreflightSheet } from "./HotSwapDialogs";
import { MessageRow, type MessageRowActions } from "./ChatMessageRow";
import { QueuedMessageTray } from "./QueuedMessageTray";
import { MilimUsageRidgeline } from "./MilimUsageRidgeline";
import { useChatCatalogController } from "./chat/useChatCatalogController";
import {
  previewRuntimeText,
  previewStatusFromRuntime,
  previewStatusMatchesFolder,
  useChatInspectorController,
} from "./chat/useChatInspectorController";
import { useChatConversationController } from "./chat/useChatConversationController";
import { useChatMobileRelayController } from "./chat/useChatMobileRelayController";
import { useChatMediaController } from "./chat/useChatMediaController";
import { useChatWorkerController } from "./chat/useChatWorkerController";

const ProvidersManager = lazy(() =>
  import("./ProvidersManager").then((mod) => ({
    default: mod.ProvidersManager,
  })),
);
const McpManager = lazy(() =>
  import("./McpManager").then((mod) => ({ default: mod.McpManager })),
);
const MemoryManager = lazy(() =>
  import("./MemoryManager").then((mod) => ({ default: mod.MemoryManager })),
);
const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const EMPTY: ChatMessage[] = [];
const EMPTY_QUEUE: QueuedMessage[] = [];
const EMPTY_CONTEXT_SECTION_IDS: QuickSummarySectionId[] = [];
const NON_EMPTY_USAGE_MESSAGES: ChatMessage[] = [{ role: "user", content: "" }];
const PREVIEW_PANEL_MIN_WIDTH = 360;
const RECENT_THREAD_SWITCHER_CLOSE_MS = 1600;
const EVENT_STREAM_RECONNECT_MAX_MS = 5_000;
const previewArtifactCache = new WeakMap<ChatMessage, ChatArtifact[] | null>();

function waitForEventReconnect(
  signal: AbortSignal,
  attempt: number,
): Promise<void> {
  if (signal.aborted) return Promise.resolve();
  return new Promise((resolve) => {
    const done = () => {
      window.clearTimeout(timer);
      signal.removeEventListener("abort", done);
      resolve();
    };
    const timer = window.setTimeout(
      done,
      Math.min(500 * 2 ** attempt, EVENT_STREAM_RECONNECT_MAX_MS),
    );
    signal.addEventListener("abort", done, { once: true });
  });
}

function documentVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

type CompactionSummaryResult = {
  content: string;
  usage?: TokenUsage;
  costUsd?: number;
  finishReason?: string;
};

async function collectHarnessUtilityRun(
  harnessId: "codex" | "claude" | "opencode" | "pi",
  request: HarnessRunRequest,
  signal?: AbortSignal,
): Promise<CompactionSummaryResult> {
  let content = "";
  let warning: string | null = null;
  let error: string | null = null;
  let usage: TokenUsage | undefined;
  let costUsd: number | undefined;
  await streamHarnessRun(
    harnessId,
    request,
    (envelope: HarnessEventEnvelope) => {
      const event = envelope.event;
      if (event.type === "text_delta" && event.text) {
        content += event.text;
      } else if (event.type === "runtime_notice") {
        if (event.level === "error") error = event.message;
        else if (event.code === "runtime_warning") warning = event.message;
      } else if (
        event.type === "turn_failed" ||
        event.type === "turn_cancelled"
      ) {
        error = event.message ?? "Harness turn was cancelled.";
      } else if (
        event.type === "usage_updated" ||
        event.type === "turn_completed"
      ) {
        if (event.usage) usage = event.usage;
        if (typeof event.cost_usd === "number" && event.cost_usd > 0)
          costUsd = event.cost_usd;
      }
    },
    signal,
  );
  if (error) throw new Error(error);
  if (warning) throw new Error(warning);
  return { content, usage, costUsd };
}

function mergeTokenUsage(
  left?: TokenUsage,
  right?: TokenUsage,
): TokenUsage | undefined {
  if (!left) return right;
  if (!right) return left;
  return {
    prompt_tokens: left.prompt_tokens + right.prompt_tokens,
    completion_tokens: left.completion_tokens + right.completion_tokens,
    total_tokens: left.total_tokens + right.total_tokens,
  };
}

function mobileThreadMessages(
  messages: ChatMessage[],
): { role: string; content: string }[] {
  return messages
    .filter(
      (message) => message.role === "user" || message.role === "assistant",
    )
    .map((message) => ({
      role: message.role,
      content: mobileThreadMessageContent(message),
    }))
    .filter((message) => message.content.trim());
}

function workerRunSynthesisMessage(record: WorkerRunRecord): ChatMessage {
  const allFailed = !record.workers.some((worker) => worker.status === "done");
  const results = record.workers.map((worker, index) => {
    const task = record.run.tasks.find(
      (item) => item.prompt === worker.prompt || item.title === worker.title,
    );
    const content = worker.summary?.trim() || worker.error?.trim() || "No result returned.";
    return [
      `Worker ${index + 1}: ${task?.title || worker.title || worker.id}`,
      `Status: ${worker.status}`,
      content,
    ].join("\n");
  });
  return {
    role: "system",
    workerRunId: record.run.id,
    content: [
      `Worker Run ${record.run.id} finished with status ${record.run.status}.`,
      allFailed
        ? "All workers failed or stopped. Acknowledge that briefly, then continue the original request yourself without delegating again."
        : "Use the joined results below to answer the original request. Treat failures as visible evidence, not successful results.",
      ...results,
    ].join("\n\n"),
  };
}

function nativeWorkerRunRecord(
  lifecycle: AccountNativeWorkerLifecycle,
  parentThreadId: string,
  parentTurnId: string,
  fallbackModel: string,
): WorkerRunRecord {
  const now = new Date().toISOString();
  const terminal = /done|complete|success/i.test(lifecycle.status);
  const failed = /error|fail/i.test(lifecycle.status);
  const runStatus = failed ? "error" : terminal ? "done" : "running";
  const runtime: Worker["runtime"] = lifecycle.runtime === "claude" ? "claude" : "codex";
  const workerIds = lifecycle.workers.length
    ? lifecycle.workers.map((worker) => worker.runtime_id)
    : lifecycle.worker_runtime_ids;
  const tasks = workerIds.map((runtimeId, index) => ({
    id: `${lifecycle.call_id}:${runtimeId}`,
    title: `Native worker ${index + 1}`,
    prompt: lifecycle.prompt || "Native account-runtime worker",
    role: lifecycle.operation || null,
    agent_id: null,
    model: lifecycle.model || fallbackModel,
    access: "read_only" as const,
  }));
  const workers = workerIds.map((runtimeId, index) => {
    const state = lifecycle.workers.find((worker) => worker.runtime_id === runtimeId);
    const status: Worker["status"] = /done|complete|success/i.test(state?.status || "")
      ? "done"
      : /error|fail/i.test(state?.status || "")
        ? "error"
        : "running";
    return {
      id: tasks[index].id,
      parent_id: parentThreadId,
      root_id: parentThreadId,
      title: tasks[index].title,
      status,
      model: tasks[index].model,
      agent_id: null,
      prompt: tasks[index].prompt,
      summary: status === "done" ? state?.message ?? null : null,
      error: status === "error" ? state?.message ?? "Native worker failed." : null,
      created_at: now,
      updated_at: now,
      finished_at: status === "running" ? null : now,
      run_id: `native:${runtime}:${lifecycle.call_id}`,
      runtime,
      access: "read_only" as const,
      external_runtime_id: runtimeId,
      worktree_path: null,
    };
  });
  return {
    run: {
      id: `native:${runtime}:${lifecycle.call_id}`,
      parent_thread_id: parentThreadId,
      parent_turn_id: parentTurnId,
      policy: "auto",
      runtime,
      status: runStatus,
      tasks,
      error: failed ? "Native worker activity failed." : null,
      created_at: now,
      updated_at: now,
      finished_at: runStatus === "running" ? null : now,
    },
    workers,
  };
}

function mobileThreadMessageContent(message: ChatMessage): string {
  if (message.content.trim()) return message.content;
  return (message.streamParts ?? [])
    .filter(
      (part): part is Extract<ChatStreamPart, { kind: "text" }> =>
        part.kind === "text",
    )
    .map((part) => part.content)
    .join("");
}

function mobileFolderLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "Project";
}

function mobileProjectByFolder(projects: Project[]): Map<string, Project> {
  return new Map(
    projects
      .filter((project) => !project.archivedAt)
      .map((project) => [project.folder, project]),
  );
}

function mobileThreadSummary(
  session: ChatSessionSummary,
  running: Set<string>,
  projectByFolder: Map<string, Project>,
): MobileThreadSummary {
  const folder = session.settings?.folder?.trim() ?? "";
  const project = folder ? projectByFolder.get(folder) : undefined;
  return {
    id: session.id,
    title: session.title || "New chat",
    model: session.model ?? null,
    updated_at: Math.floor(session.updatedAt / 1000),
    busy: running.has(session.id),
    parent_id: session.parentId ?? null,
    project_label: folder ? (project?.name ?? mobileFolderLabel(folder)) : null,
    project_path: folder || null,
  };
}

function mobileThreadSummaries(
  sessions: ChatSessionSummary[],
  projects: Project[],
  generatingSessionIds: string[],
): MobileThreadSummary[] {
  const running = new Set(generatingSessionIds);
  const projectByFolder = mobileProjectByFolder(projects);
  const archivedProjectFolders = new Set(
    projects
      .filter((project) => project.archivedAt)
      .map((project) => project.folder),
  );
  return sessions
    .filter((session) => {
      if (session.parentId) return false;
      if (session.archivedAt) return false;
      const folder = session.settings?.folder?.trim() ?? "";
      return !folder || !archivedProjectFolders.has(folder);
    })
    .slice()
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .map((session) => mobileThreadSummary(session, running, projectByFolder));
}

function mobileWorkerRun(record?: WorkerRunRecord): MobileWorkerRunSnapshot | null {
  if (!record) return null;
  return {
    id: record.run.id,
    status: record.run.status,
    tasks: record.run.tasks.map((task) => {
      const worker = record.workers.find(
        (item) => item.prompt === task.prompt || item.title === task.title,
      );
      return {
        title: task.title,
        model: task.model,
        access: worker?.access ?? task.access,
        status: worker?.status ?? (record.run.status === "proposed" ? "proposed" : "queued"),
        result: worker?.summary ?? worker?.error ?? null,
      };
    }),
  };
}

function mobileThreadGroups(
  sessions: ChatSessionSummary[],
  projects: Project[],
  sidebar: SessionSidebarState,
  generatingSessionIds: string[],
): MobileThreadGroup[] {
  const running = new Set(generatingSessionIds);
  const projectByFolder = mobileProjectByFolder(projects);
  return groupSessionsByProjects(sessions, projects, sidebar, "")
    .map((group) => ({
      id: group.id,
      label: group.label,
      subtitle: group.subtitle ?? null,
      project_id: group.projectId ?? null,
      threads: group.sessions.map((session) =>
        mobileThreadSummary(session, running, projectByFolder),
      ),
    }))
    .filter((group) => group.threads.length > 0);
}

function mobileModelSummaries(models: ModelInfo[]) {
  return models
    .filter(isUsableMobileModel)
    .slice(0, 120)
    .map((model) => ({ id: model.id, provider: model.owned_by || null }));
}

function isUsableMobileModel(model: ModelInfo): boolean {
  return (
    Boolean(model.id.trim()) &&
    !model.capabilities?.imageOutput &&
    !model.capabilities?.videoOutput &&
    !model.capabilities?.musicOutput
  );
}

const CHAT_MAIN_MIN_WIDTH = 420;
const PREVIEW_RESIZE_HANDLE_WIDTH = 8;
const CONTEXT_PANEL_WIDTH = 300;
const INSPECTOR_STACK_THRESHOLD =
  PREVIEW_PANEL_MIN_WIDTH + CHAT_MAIN_MIN_WIDTH + PREVIEW_RESIZE_HANDLE_WIDTH;
const CONTEXT_STACK_THRESHOLD = CONTEXT_PANEL_WIDTH + CHAT_MAIN_MIN_WIDTH;
const CONCURRENT_PANEL_THRESHOLD = INSPECTOR_STACK_THRESHOLD + CONTEXT_PANEL_WIDTH;
const PREVIEW_PANEL_KEYBOARD_STEP = 32;
const PREVIEW_PANEL_STAGE_OVERSHOOT = 32;
const PREVIEW_PANEL_COLLAPSE_OVERSHOOT = 96;
const PREVIEW_PANEL_ANIMATION_MS = 180;
const COLLAPSED_SIDEBAR_WIDTH = 48;
const HOT_SWAP_CONTINUE_PROMPT =
  "Continue from the current workspace and thread state. Inspect what is already complete, then finish the active task.";
const HOT_SWAP_REVIEW_PROMPT =
  "Review the previous model's response and the current workspace changes for correctness, regressions, and missing verification. Do not edit files; report findings first.";

type BatonRequest = {
  action: Exclude<HotSwapAction, "switch">;
  messageIndex: number;
};

type HotSwapPreflightRequest = {
  action: HotSwapAction;
  messageIndex?: number;
  target: ModelInfo;
  assessment: HotSwapAssessment;
  selection: ModelPickerSelection;
};
const APP_SESSION_ID = (() => {
  try {
    return crypto.randomUUID();
  } catch {
    return (
      "app-" + Math.random().toString(36).slice(2) + Date.now().toString(36)
    );
  }
})();

function attachmentId(): string {
  try {
    return crypto.randomUUID();
  } catch {
    return (
      "att-" + Math.random().toString(36).slice(2) + Date.now().toString(36)
    );
  }
}

async function browserFileAttachment(file: File): Promise<ChatAttachment> {
  const mime = file.type || inferAttachmentMime(file.name);
  const textLike =
    mime.startsWith("text/") ||
    mime === "application/json" ||
    mime === "application/xml" ||
    mime === "application/javascript";
  const [content, dataUrl] = await Promise.all([
    textLike
      ? file.slice(0, MAX_ATTACHMENT_BYTES).text()
      : Promise.resolve(undefined),
    readBrowserAttachmentDataUrl(file, mime),
  ]);
  return {
    id: attachmentId(),
    name: file.name || "attachment",
    mime,
    size: file.size,
    content,
    dataUrl,
    truncated: textLike ? file.size > MAX_ATTACHMENT_BYTES : false,
  };
}

function previewArtifactsForMessage(
  message: ChatMessage,
): ChatArtifact[] | undefined {
  const cached = previewArtifactCache.get(message);
  if (cached !== undefined) return cached ?? undefined;
  if (isCompactionCheckpoint(message)) return undefined;
  if (message.role !== "assistant") return undefined;
  const completed = message.artifacts ?? [];
  if (completed.length) return completed;
  if (!message.content) {
    previewArtifactCache.set(message, null);
    return undefined;
  }
  const live = extractLivePreviewArtifactFromContent(message.content);
  const artifacts = live ? [live] : null;
  previewArtifactCache.set(message, artifacts);
  return artifacts ?? undefined;
}

function preferredPreviewArtifact(
  artifacts?: readonly ChatArtifact[],
): ChatArtifact | null {
  const previewable = artifacts?.filter(isPreviewableArtifact) ?? [];
  if (!previewable.length) return null;
  return previewable
    .slice()
    .sort((a, b) => previewArtifactRank(a) - previewArtifactRank(b))[0];
}

function previewArtifactRank(artifact: ChatArtifact): number {
  const name =
    (artifact.filename ?? artifact.title).split(/[\\/]/).pop()?.toLowerCase() ??
    "";
  const source = (artifact.language || extensionOf(name)).toLowerCase();
  if (name === "index.html" || name === "index.htm") return 0;
  if (source === "html" || source === "htm") return 1;
  if (
    source === "js" ||
    source === "jsx" ||
    source === "mjs" ||
    source === "ts" ||
    source === "tsx"
  )
    return 2;
  if (source === "md" || source === "markdown") return 3;
  return 4;
}

function previewAutoOpenKey(
  messageIndex: number,
  message: ChatMessage,
  artifact: ChatArtifact,
): string {
  return `${messageIndex}\0${message.run?.startedAt ?? ""}\0${artifactPreviewAutoOpenKey(artifact)}`;
}

function localhostPreviewArtifact(url: string): ChatArtifact {
  return {
    id: `localhost-preview-${url.replace(/[^a-z0-9]+/gi, "-")}`,
    kind: "text",
    title: url,
    mime: "text/uri-list",
    content: url,
    size: url.length,
    language: "url",
    disposition: "preview",
  };
}

function blankBrowserArtifact(): ChatArtifact {
  return {
    id: "artifact-browser",
    kind: "text",
    title: "Browser",
    mime: "text/uri-list",
    content: "",
    size: 0,
    language: "url",
    disposition: "preview",
  };
}

function blankBrowserPreviewSelection(): PreviewSelection {
  const artifact = blankBrowserArtifact();
  return { artifact, artifacts: [artifact], previewDeferred: false };
}

function blankWorkspaceReviewSelection(): PreviewSelection {
  const artifact: ChatArtifact = {
    id: "workspace-review",
    kind: "text",
    title: "Workspace",
    mime: "text/plain",
    content: "Select a workspace file above to review it.",
    size: 0,
    language: "txt",
    disposition: "inline",
  };
  return { artifact, artifacts: [artifact], previewDeferred: false };
}

function emptyBrowserSession(): InspectorBrowserSession {
  const tab = emptyBrowserTab();
  return {
    profileId: `session-${Math.random().toString(36).slice(2)}`,
    tabs: [tab],
    activeTabId: tab.id,
  };
}

function emptyBrowserTab(url: string | null = null): InspectorBrowserTab {
  return {
    id: `tab-${Math.random().toString(36).slice(2)}`,
    url,
    input: url ?? "",
    history: url ? [url] : [],
    historyIndex: url ? 0 : -1,
  };
}

function browserPreviewSelection(
  session: InspectorBrowserSession,
): PreviewSelection {
  const tab = session.tabs.find((item) => item.id === session.activeTabId) ?? session.tabs[0];
  if (!tab?.url) return blankBrowserPreviewSelection();
  const artifact: ChatArtifact = {
    ...blankBrowserArtifact(),
    title: tab.url,
    content: tab.url,
    size: tab.url.length,
  };
  return { artifact, artifacts: [artifact], previewDeferred: false };
}

function previewIdleStatus(
  threadId: string,
  folder: string,
): PreviewAppStatus {
  const cwd = previewRuntimeText(folder) ?? "";
  return {
    thread_id: threadId,
    kind: "app",
    status: "idle",
    cwd,
    url: null,
    pid: null,
    command: null,
    message: null,
    active: false,
    ready: false,
    managed: !cwd,
    run_id: null,
    updated_at: Date.now(),
    error: null,
    preflight: null,
    logs: [],
  };
}

function mergePreviewAppFiles(
  base: readonly PreviewAppFile[],
  updates: readonly PreviewAppFile[],
): PreviewAppFile[] {
  const files = new Map<string, PreviewAppFile>();
  for (const file of [...base, ...updates]) {
    const path = normalizeVirtualFilePath(file.path);
    if (path) files.set(path, { path, content: file.content });
  }
  return [...files.values()].sort((left, right) =>
    left.path.localeCompare(right.path),
  );
}

function virtualArtifactPreview(
  path: string,
  content: string,
  existing?: SessionVirtualFile,
): ArtifactWritePreview {
  const oldContent = existing?.content ?? null;
  const changed = oldContent !== content;
  return {
    path,
    exists: Boolean(existing),
    changed,
    old_content: oldContent,
    new_content: content,
    old_bytes: existing?.bytes ?? null,
    new_bytes: textBytes(content),
    diff: changed ? simpleDiff(oldContent, content) : "",
    truncated: false,
  };
}

function simpleDiff(oldContent: string | null, newContent: string): string {
  const oldLines =
    oldContent == null
      ? []
      : oldContent.split(/\r?\n/).map((line) => `-${line}`);
  const newLines = newContent.split(/\r?\n/).map((line) => `+${line}`);
  return [...oldLines, ...newLines].join("\n");
}

function textBytes(content: string): number {
  return new TextEncoder().encode(content).byteLength;
}

function virtualChatArtifact(file: SessionVirtualFile): ChatArtifact {
  const language = extensionOf(file.path);
  return {
    id: `virtual-${file.path}-${file.version}`,
    kind: language === "json" ? "json" : language === "csv" ? "csv" : "code",
    title: file.path,
    filename: file.path,
    mime: mimeForVirtualFile(file.path),
    content: file.content,
    size: file.bytes,
    language,
    disposition: "file",
  };
}

function mimeForVirtualFile(path: string): string {
  const ext = extensionOf(path);
  if (ext === "json") return "application/json";
  if (ext === "csv") return "text/csv";
  if (ext === "md" || ext === "markdown") return "text/markdown";
  if (ext === "html" || ext === "htm") return "text/html";
  if (ext === "css") return "text/css";
  if (["js", "mjs", "jsx", "ts", "tsx"].includes(ext)) return "text/javascript";
  return "text/plain";
}

function previewSelectionFromRuntime(
  runtime?: SessionPreviewRuntime,
): PreviewSelection | null {
  const url = previewRuntimeBrowserUrl(runtime);
  if (!url) return null;
  const artifact = localhostPreviewArtifact(url);
  return { artifact, artifacts: [artifact], previewDeferred: false };
}

function extensionOf(filename: string): string {
  const dot = filename.lastIndexOf(".");
  return dot >= 0 ? filename.slice(dot + 1).toLowerCase() : "";
}

function maxPreviewPanelWidth(
  chatBodyWidth?: number,
  reservedWidth = 0,
  overlay = false,
): number {
  const availableWidth =
    chatBodyWidth ??
    (typeof window === "undefined" ? undefined : window.innerWidth);
  if (availableWidth === undefined) return DEFAULT_PREVIEW_PANEL_WIDTH;
  return Math.max(
    PREVIEW_PANEL_MIN_WIDTH,
    availableWidth -
      (overlay ? PREVIEW_RESIZE_HANDLE_WIDTH : reservedWidth + CHAT_MAIN_MIN_WIDTH + PREVIEW_RESIZE_HANDLE_WIDTH),
  );
}

function clampPreviewPanelWidth(
  width: number,
  chatBodyWidth?: number,
  reservedWidth = 0,
  overlay = false,
): number {
  return Math.round(
    Math.min(
      Math.max(width, PREVIEW_PANEL_MIN_WIDTH),
      maxPreviewPanelWidth(chatBodyWidth, reservedWidth, overlay),
    ),
  );
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function emptyStarterIcon(icon: EmptyStarterSuggestionIcon): ReactNode {
  switch (icon) {
    case "arrow":
      return <ArrowRight size={13} />;
    case "git":
      return <GitBranch size={13} />;
    case "pencil":
      return <Pencil size={13} />;
    case "refresh":
      return <Refresh size={13} />;
    default:
      return <Code size={13} />;
  }
}

function EmptyStarterActions({
  strip,
  onSelect,
}: {
  strip: EmptyStarterStrip;
  onSelect: (id: string, prompt: string) => void;
}) {
  return (
    <section
      className={`empty-starter-strip${strip.context ? " has-context" : ""}${strip.loading ? " loading" : ""}`}
      data-testid="empty-starter-strip"
    >
      {strip.context && (
        <div
          className="empty-starter-context"
          data-testid="empty-starter-context"
          title={strip.context}
        >
          <span className="empty-starter-context-icon" aria-hidden="true">
            <GitBranch size={12} />
          </span>
          <span>{strip.context}</span>
        </div>
      )}
      <div
        className="empty-starter-actions"
        aria-label={strip.loading ? undefined : "Starter prompts"}
        aria-hidden={strip.loading || undefined}
      >
        {strip.loading
          ? [0, 1, 2].map((index) => (
              <span className="empty-starter-placeholder" key={index} />
            ))
          : strip.suggestions.map((suggestion) => (
              <button
                type="button"
                className="empty-starter-action"
                data-testid="empty-starter-action"
                key={suggestion.id}
                title={`${suggestion.label}: ${suggestion.detail}`}
                onClick={() => onSelect(suggestion.id, suggestion.prompt)}
              >
                <span className="empty-starter-icon" aria-hidden="true">
                  {emptyStarterIcon(suggestion.icon)}
                </span>
                <span className="empty-starter-copy">
                  <span className="empty-starter-label">
                    {suggestion.label}
                  </span>
                  <span className="empty-starter-detail">
                    {suggestion.detail}
                  </span>
                </span>
              </button>
            ))}
      </div>
    </section>
  );
}

type ChatNotice = {
  message: string;
  tone: "info" | "warning" | "error";
};

type RunTurnResult = {
  status: "done" | "aborted" | "error" | "skipped";
  messages: ChatMessage[];
  error?: string;
};

type RunTurnOptions = {
  goal?: GoalSettings;
  toolApprovalGrant?: boolean;
  claudeSessionRecoveryGrant?: boolean;
  delegationPolicyOverride?: DelegationPolicy;
  canonicalAction?: "send" | "regenerate";
  legacyRuntime?: boolean;
};

type ToolApprovalScope = ChatApprovalRequest["scope"];

function toolApprovalMessage(
  scope: ToolApprovalScope,
  model: string,
  detail?: string,
): ChatMessage {
  const kind =
    scope === "claude_session_recovery" ? "claude_session_recovery" : "tool";
  return {
    role: "assistant",
    content: "",
    approval: {
      kind,
      scope,
      status: "pending",
      requestedAt: Date.now(),
      model: model.trim() || undefined,
      detail,
    },
  };
}

function resolveApprovalMessage(
  message: ChatMessage,
  status: "approved" | "denied",
): ChatMessage {
  if (!message.approval) return message;
  return {
    ...message,
    approval: {
      ...message.approval,
      status,
      resolvedAt: Date.now(),
    },
  };
}

type PreviewSelection = {
  artifact: ChatArtifact;
  artifacts: ChatArtifact[];
  revision?: ArtifactRevision;
  revisionGroup?: ArtifactRevisionGroup;
  previewDeferred?: boolean;
  autoOpenKey?: string;
};

type InspectorPreviewSource = "artifact" | "app" | "url";

type InspectorBrowserTab = SessionBrowserTab;
type InspectorBrowserSession = SessionBrowserSession;

type ChatSessionSummary = {
  id: string;
  title: string;
  messages: ChatMessage[];
  settings?: {
    folder?: string;
    model?: string;
    sandbox?: boolean;
    computerUse?: boolean;
    privacy?: string;
  };
  model?: string | null;
  parentId?: string;
  updatedAt: number;
  archivedAt?: number;
  retryWorkspace?: { originalFolder: string };
};

type RecentThreadSwitcherState = {
  items: RecentThreadSwitcherItem[];
  activeIndex: number;
};

function RecentThreadSwitcherOverlay({
  state,
  onSelect,
}: {
  state: RecentThreadSwitcherState;
  onSelect: (id: string) => void;
}) {
  return (
    <div className="recent-thread-switcher" data-native-preview-blocker="true" aria-live="polite">
      <div
        className="recent-thread-switcher-popover"
        role="listbox"
        aria-label="Recently viewed threads"
      >
        <div className="recent-thread-switcher-title">Recently viewed</div>
        <div className="recent-thread-switcher-list">
          {state.items.map((item, index) => {
            const active = index === state.activeIndex;
            return (
              <button
                key={item.id}
                type="button"
                role="option"
                aria-selected={active}
                className={"recent-thread-switcher-row" + (active ? " active" : "")}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => onSelect(item.id)}
              >
                <span className="recent-thread-switcher-row-title">
                  {item.title}
                </span>
                {item.metadata && (
                  <span className="recent-thread-switcher-row-meta">
                    {item.metadata}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function usageDateKey(value: number): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime())
    ? `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
    : "";
}

function sameChatSessionSummary(
  session: Session,
  summary: ChatSessionSummary,
): boolean {
  return (
    session.id === summary.id &&
    session.title === summary.title &&
    Boolean(session.messages.length) === Boolean(summary.messages.length) &&
    session.settings?.folder === summary.settings?.folder &&
    session.settings?.model === summary.settings?.model &&
    session.settings?.sandbox === summary.settings?.sandbox &&
    session.settings?.computerUse === summary.settings?.computerUse &&
    session.settings?.privacy === summary.settings?.privacy &&
    (session.worker?.model || session.settings?.model || null) ===
      summary.model &&
    session.parentId === summary.parentId &&
    usageDateKey(session.updatedAt) === usageDateKey(summary.updatedAt) &&
    session.archivedAt === summary.archivedAt &&
    session.retryWorkspace?.originalFolder ===
      summary.retryWorkspace?.originalFolder
  );
}

function createChatSessionSummariesSelector() {
  let previous: ChatSessionSummary[] = [];
  return (
    state: ReturnType<typeof useSessions.getState>,
  ): ChatSessionSummary[] => {
    let changed = previous.length !== state.sessions.length;
    const next = state.sessions.map((session, index) => {
      const cached = previous[index];
      if (cached && sameChatSessionSummary(session, cached)) return cached;
      changed = true;
      return {
        id: session.id,
        title: session.title,
        messages: session.messages.length ? NON_EMPTY_USAGE_MESSAGES : EMPTY,
        settings: session.settings
          ? {
              folder: session.settings.folder,
              model: session.settings.model,
              sandbox: session.settings.sandbox,
              computerUse: session.settings.computerUse,
              privacy: session.settings.privacy,
            }
          : undefined,
        model: session.worker?.model || session.settings?.model || null,
        parentId: session.parentId,
        updatedAt: session.updatedAt,
        archivedAt: session.archivedAt,
        retryWorkspace: session.retryWorkspace
          ? { originalFolder: session.retryWorkspace.originalFolder }
          : undefined,
      };
    });
    if (!changed) return previous;
    previous = next;
    return next;
  };
}

function executePlanPrompt(plan: string): string {
  return [
    "Execute the approved implementation plan below.",
    "Apply the changes in the current workspace. Keep the implementation scoped to the plan unless the code proves a small adjustment is necessary.",
    "",
    "Approved plan:",
    plan,
  ].join("\n");
}

function codexImageMediaResult(
  ev: Extract<HarnessEvent, { type: "image_generated" }>,
  model: string,
): MediaGenerationResult {
  return {
    id: ev.id || attachmentId(),
    object: "media.generation",
    provider_id: "codex",
    provider: "Codex",
    provider_kind: "openai_compatible",
    kind: "image",
    model,
    status: ev.status || "completed",
    output: ev.revised_prompt ?? undefined,
    media: [{ url: ev.url, kind: "image", mime: "image/png" }],
    urls: { web: ev.url },
    privacy: { mode: "off", redacted: false, detections: 0, kinds: "" },
    raw: ev,
  };
}

function appendAssistantMediaResult(
  sessionId: string,
  result: MediaGenerationResult,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.slice();
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== "assistant") continue;
    const current = message.mediaResults ?? [];
    const mediaResults = current.some(
      (item) =>
        item.provider_id === result.provider_id && item.id === result.id,
    )
      ? current.map((item) =>
          item.provider_id === result.provider_id && item.id === result.id
            ? result
            : item,
        )
      : [...current, result];
    messages[index] = { ...message, mediaResults };
    store.setMessages(sessionId, messages, { autoTitle: false });
    return;
  }
}

function attachAssistantWorkspaceCheckpoint(
  sessionId: string,
  checkpoint: WorkspaceCheckpoint,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.slice();
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role !== "assistant") continue;
    messages[index] = { ...messages[index], workspaceCheckpoint: checkpoint };
    store.setMessages(sessionId, messages, { autoTitle: false });
    return;
  }
}

function compactText(value: string, max = 96): string {
  const text = value.trim().replace(/\s+/g, " ");
  return text.length > max ? text.slice(0, max - 1).trimEnd() + "..." : text;
}

export function ChatView({
  onManageAgents,
  onOpenSchedules,
  onOpenSettings,
  composerDraft,
  gitPanelRequest = null,
  mcpManagerRequest = 0,
  chatSearchRequest = 0,
  onComposerDraftConsumed,
  skillsRevision = 0,
}: {
  onManageAgents: () => void;
  onOpenSchedules: () => void;
  onOpenSettings: () => void;
  composerDraft?: { id: number; text: string } | null;
  gitPanelRequest?: {
    id: number;
    sessionId?: string;
    view: GitPanelView;
  } | null;
  mcpManagerRequest?: number;
  chatSearchRequest?: number;
  onComposerDraftConsumed?: (id: number) => void;
  skillsRevision?: number;
}) {
  markPerfRender("ChatView");
  const { openContextMenu } = useContextMenu();
  const messageRowActionsRef = useRef<MessageRowActions | null>(null);
  const [input, setInputState] = useState(() =>
    getSessionComposerDraft(useSessions.getState().activeId),
  );
  const [providersOpen, setProvidersOpen] = useState(false);
  const [mcpOpen, setMcpOpen] = useState(false);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [memoryTarget, setMemoryTarget] = useState<MemoryNotice | null>(null);
  const [attachmentPreview, setAttachmentPreview] = useState<ChatAttachment | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [pendingAttachments, setPendingAttachments] = useState<
    ChatAttachment[]
  >([]);
  const [reviewCommentsBySession, setReviewCommentsBySession] = useState<Record<string, ReviewComment[]>>({});
  const [, setPreviewSelection] =
    useState<PreviewSelection | null>(null);
  const [workerActionBusy, setWorkerActionBusy] = useState(false);
  const [workerFocusRunId, setWorkerFocusRunId] = useState("");
  const [workerSettingsOpen, setWorkerSettingsOpen] = useState(false);
  const [previewPanelClosing, setPreviewPanelClosing] = useState(false);
  const [, setPreviewSource] =
    useState<InspectorPreviewSource>("url");
  const [chatBodyWidth, setChatBodyWidth] = useState(() =>
    typeof window === "undefined" ? INSPECTOR_STACK_THRESHOLD : window.innerWidth,
  );
  const [gitStatus, setGitStatus] = useState<WorkspaceGitStatus | null>(null);
  const [gitStatusLoading, setGitStatusLoading] = useState(false);
  const [gitPanelView, setGitPanelView] = useState<GitPanelView>("changes");
  const [turnReview, setTurnReview] = useState<TurnReviewState | null>(null);
  const [turnReviewRevision, setTurnReviewRevision] = useState(0);
  const [gitDiffRequest, setGitDiffRequest] = useState<
    (GitPanelDiffRequest & { sessionId: string; folder: string }) | null
  >(null);
  const gitDiffRequestIdRef = useRef(0);
  const [dismissedPreviewKey, setDismissedPreviewKey] = useState<string | null>(
    null,
  );
  const [chatNotice, setChatNotice] = useState<ChatNotice | null>(null);
  const [goalPanelOpen, setGoalPanelOpen] = useState(false);
  const [goalPrefill, setGoalPrefill] = useState<string | null>(null);
  const [goalComposerSessions, setGoalComposerSessions] = useState<
    Record<string, boolean>
  >({});
  const [chatSearchOpen, setChatSearchOpen] = useState(false);
  const [batonRequest, setBatonRequest] = useState<BatonRequest | null>(null);
  const [hotSwapPreflight, setHotSwapPreflight] =
    useState<HotSwapPreflightRequest | null>(null);
  const [queueInterrupts, setQueueInterrupts] = useState<
    Record<string, string>
  >({});
  const [sessionsHydrated, setSessionsHydrated] = useState(() =>
    useSessions.persist.hasHydrated(),
  );
  const activeId = useSessions((s) => s.activeId);
  const pendingReviewComments = reviewCommentsBySession[activeId] ?? [];
  const setPendingReviewComments = useCallback((next: ReviewComment[] | ((current: ReviewComment[]) => ReviewComment[])) => {
    setReviewCommentsBySession((current) => ({
      ...current,
      [activeId]: typeof next === "function" ? next(current[activeId] ?? []) : next,
    }));
  }, [activeId]);
  useEffect(() => {
    const add = (event: Event) => {
      const comment = (event as CustomEvent<ReviewComment>).detail;
      if (!comment?.id || !comment.body?.trim()) return;
      setPendingReviewComments((current) => {
        const next = [...current, comment].slice(0, 20);
        if (new TextEncoder().encode(JSON.stringify(next)).byteLength > 64 * 1024) {
          setChatNotice({ tone: "error", message: "Review context is limited to 64 KiB per send." });
          return current;
        }
        return next;
      });
    };
    window.addEventListener("milim:add-review-comment", add);
    return () => window.removeEventListener("milim:add-review-comment", add);
  }, [setPendingReviewComments]);
  const sessionSummariesSelector = useMemo(
    createChatSessionSummariesSelector,
    [],
  );
  const sessionSummaries = useSessions(sessionSummariesSelector);
  const messages = useSessions(
    (s) => s.sessions.find((x) => x.id === s.activeId)?.messages ?? EMPTY,
  );
  const pendingApprovals = useMemo(
    () => pendingToolApprovals(messages),
    [messages],
  );
  const artifactRevisionGroupsForThread = useMemo(
    () => artifactRevisionGroups(messages),
    [messages],
  );
  const artifactRevisionsByOccurrence = useMemo(
    () => artifactRevisionChoiceByOccurrence(artifactRevisionGroupsForThread),
    [artifactRevisionGroupsForThread],
  );
  const promptHistoryScope = useUiPreferences((s) => s.promptHistoryScope);
  const globalPromptHistory = useUiPreferences((s) => s.globalPromptHistory);
  const recordGlobalPrompt = useUiPreferences((s) => s.recordGlobalPrompt);
  const sentHistory = useMemo(() => {
    if (promptHistoryScope === "off") return [];
    if (promptHistoryScope === "global") return globalPromptHistory.slice().reverse();
    return messages
      .filter((message) => message.role === "user" && message.content.trim())
      .map((message) => message.content.trim());
  }, [globalPromptHistory, messages, promptHistoryScope]);
  const activeTitle = useSessions(
    (s) =>
      s.sessions.find((x) => x.id === s.activeId)?.title ?? "Current thread",
  );
  const activeWorker = useSessions(
    (s) => s.sessions.find((x) => x.id === s.activeId)?.worker,
  );
  const activeSession = useSessions(
    (s) => s.sessions.find((x) => x.id === s.activeId),
  );
  const workerRuns = useSessions((s) => s.workerRuns);
  const activeWorkerRuns = useMemo(
    () =>
      workerRuns.filter((record) => record.run.parent_thread_id === activeId),
    [activeId, workerRuns],
  );
  const activeWorkerRun = activeWorkerRuns[0];
  const announcedAttentionKeysRef = useRef(new Set<string>());
  const attentionKey = useMemo(
    () => pendingAttentionKey(
      messages,
      activeWorkerRun?.run.status === "proposed" ? activeWorkerRun.run.id : undefined,
    ),
    [activeWorkerRun?.run.id, activeWorkerRun?.run.status, messages],
  );
  useEffect(() => {
    if (!attentionKey || announcedAttentionKeysRef.current.has(attentionKey)) return;
    announcedAttentionKeysRef.current.add(attentionKey);
    const preferences = useUiPreferences.getState();
    if (
      document.visibilityState === "visible" &&
      preferences.interfaceSounds &&
      preferences.soundOnAttention
    ) playInterfaceSound(preferences.attentionSound);
    if (preferences.notifyNeedsAttention) {
      void sendMilimNotification("attention", {
        threadTitle: activeTitle,
        includeThreadTitle: preferences.notificationIncludeThreadTitle,
        onlyWhenUnfocused: preferences.notifyOnlyWhenUnfocused,
      });
    }
  }, [activeTitle, attentionKey]);
  const projects = useSessions((s) => s.projects);
  const sidebarState = useSessions((s) => s.sidebar);
  const generatingSessionIds = useSessions((s) => s.generatingSessionIds);
  const liveWorkerSessionIdsKey = useSessions((s) =>
    s.sessions
      .filter(
        (session) =>
          session.worker?.status === "queued" ||
          session.worker?.status === "running",
      )
      .map((session) => session.id)
      .join("\0"),
  );
  const queuedMessages = useSessions(
    (s) => s.queuedMessagesBySession[s.activeId] ?? EMPTY_QUEUE,
  );
  const inspectorState = useSessions((s) =>
    inspectorStateForSession(
      s.inspectorByKey,
      s.sessions.find((session) => session.id === s.activeId),
    ),
  );
  const inspectorTab = inspectorState.tab;
  const inspectorOpen = inspectorState.open;
  const contextPanelOpen = useSessions(
    (s) =>
      s.sessions.find((x) => x.id === s.activeId)?.contextPanelOpen === true,
  );
  const contextCollapsedSectionIds = useSessions(
    (s) =>
      s.sessions.find((x) => x.id === s.activeId)
        ?.contextCollapsedSectionIds ?? EMPTY_CONTEXT_SECTION_IDS,
  );
  const activePreviewRuntime = useSessions((s) => {
    const session = s.sessions.find((x) => x.id === s.activeId);
    const activeFolder = session?.settings?.folder ?? "";
    return activeFolder.trim()
      ? s.previewRuntimesByKey[
          previewRuntimeKeyForThread(s.activeId, activeFolder)
        ]
      : session?.previewRuntime;
  });
  const setMessages = useSessions((s) => s.setMessages);
  const markArtifactSaved = useSessions((s) => s.markArtifactSaved);
  const upsertVirtualFiles = useSessions((s) => s.upsertVirtualFiles);
  const commitResponseMetrics = useSessions((s) => s.commitResponseMetrics);
  const setSessionContextPanelOpen = useSessions((s) => s.setContextPanelOpen);
  const setSessionContextSectionCollapsed = useSessions(
    (s) => s.setContextSectionCollapsed,
  );
  const setSessionInspectorOpen = useSessions((s) => s.setInspectorOpen);
  const setSessionInspectorTab = useSessions((s) => s.setInspectorTab);
  const setSessionBrowserSession = useSessions((s) => s.setBrowserSession);
  const updateThreadSettings = useSessions((s) => s.updateSettings);
  const switchToSession = useSessions((s) => s.switchTo);
  const enqueueQueuedMessage = useSessions((s) => s.enqueueQueuedMessage);
  const moveQueuedMessage = useSessions((s) => s.moveQueuedMessage);
  const removeQueuedMessage = useSessions((s) => s.removeQueuedMessage);
  const rawThreadSettings = useSessions(
    (s) => s.sessions.find((x) => x.id === s.activeId)?.settings,
  );
  const threadSettings = useMemo(
    () => ({
      ...DEFAULT_THREAD_SETTINGS,
      ...rawThreadSettings,
      goal: normalizeGoalSettings(
        rawThreadSettings?.goal ?? DEFAULT_GOAL_SETTINGS,
      ),
    }),
    [rawThreadSettings],
  );
  const agents = useAgents((s) => s.agents);
  const mediaSettings = useSettings((s) => s.media);
  const favoriteModels = useSettings((s) => s.favorites);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const reasoningEffortByModel = useSettings((s) => s.reasoningEffortByModel);
  const configuredNewThreads = useSettings((s) => s.newThreadBehavior === "configured");
  const unavailableModelPolicy = useSettings((s) => s.unavailableModelPolicy);
  const setMediaSettings = useSettings((s) => s.setMediaSettings);
  const {
    models,
    modelsLoaded,
    providers,
    skills,
    composerTools,
    setModels,
    setProviders,
    setComposerTools,
  } = useChatCatalogController(accountRuntimeEnabled, skillsRevision);
  const previewPanelWidth = useUiPreferences((s) => s.previewPanelWidth);
  const setPreviewPanelWidth = useUiPreferences((s) => s.setPreviewPanelWidth);
  const sidebarOpen = useUiPreferences((s) => s.sidebarOpen);
  const sidebarWidth = useUiPreferences((s) => s.sidebarWidth);
  const threadNavigationPlacement = useUiPreferences((s) => s.threadNavigationPlacement);
  const setSidebarOpen = useUiPreferences((s) => s.setSidebarOpen);
  const appShortcuts = useUiPreferences((s) => s.appShortcuts);
  const toggleSidebar = useUiPreferences((s) => s.toggleSidebar);
  const verticalSidebarOpen = threadNavigationPlacement === "sidebar" && sidebarOpen;
  const autoTitleChats = useUiPreferences((s) => s.autoTitleChats);
  const experimentalHashlinePatch = useUiPreferences(
    (s) => s.experimentalHashlinePatch,
  );
  const activeTheme = useTheme((s) => s.theme);
  const backgroundFit = useUiPreferences((s) => s.backgroundFit);
  const backgroundTreatment = useUiPreferences((s) => s.backgroundTreatment);
  const showEmptyChatRidgeline = useUiPreferences((s) => s.showEmptyChatRidgeline);
  const pushNotice = useUiPreferences((s) => s.pushNotice);
  const quickActionMode = useUiPreferences((s) => s.quickActionMode);
  const pinnedQuickActions = useUiPreferences((s) => s.pinnedQuickActions);
  const projectQuickActionOverrides = useUiPreferences((s) => s.projectQuickActionOverrides);
  const recordSuggestionUse = useUiPreferences((s) => s.recordSuggestionUse);
  const personalizedSuggestions = useUiPreferences((s) => s.personalizedSuggestions);
  const suggestionUsage = useUiPreferences((s) => s.suggestionUsage);
  const threadExportFormat = useUiPreferences((s) => s.threadExportFormat);
  const composerCompletionMode = useUiPreferences((s) => s.composerCompletionMode);
  const remoteCompletionConfirmed = useUiPreferences((s) => s.remoteCompletionConfirmed);
  const {
    model,
    instructions,
    folder,
    sandbox,
    computerUse,
    memory,
    activeAgentId,
    privacy,
    toolApproval,
    delegationPolicy,
    workerModel,
    planMode,
    reasoningEffortOverrides,
    goal,
  } = threadSettings;
  const visibleApprovalPrompts = useMemo(
    () => toolApprovalPrompts(pendingApprovals, toolApproval),
    [pendingApprovals, toolApproval],
  );
  const autoApprovingToolIdsRef = useRef(new Set<string>());
  useEffect(() => {
    if (toolApproval !== "open") return;
    for (const approval of autoApprovableToolApprovals(pendingApprovals)) {
      const id = approval.approvalId;
      if (!id || autoApprovingToolIdsRef.current.has(id)) continue;
      autoApprovingToolIdsRef.current.add(id);
      void resolveToolApproval(id, "approve")
        .catch(() => {})
        .finally(() => autoApprovingToolIdsRef.current.delete(id));
    }
  }, [pendingApprovals, toolApproval]);
  const {
    activePreviewRuntimeKey,
    activePreviewSurface,
    previewAppBusy,
    previewAppPreflight,
    previewAppPreflightBusy,
    previewAppStatus,
    preflightRuntime,
    restartRuntime,
    setActivePreviewSurface,
    startRuntime,
    startStaticRuntime,
    stopRuntime,
  } = useChatInspectorController({
    activeId,
    folder,
    sessionsHydrated,
    onNotice: setChatNotice,
  });
  const goalComposerMode = Boolean(goalComposerSessions[activeId]);
  const activePreviewAppStatus =
    previewAppStatus?.thread_id === activePreviewRuntimeKey &&
    previewStatusMatchesFolder(previewAppStatus, folder)
      ? previewAppStatus
      : null;
  const activePreviewAppPreflight =
    previewAppPreflight?.thread_id === activePreviewRuntimeKey &&
    previewRuntimeFoldersEqual(previewAppPreflight.cwd, folder)
      ? previewAppPreflight
      : !folder.trim() &&
          previewAppPreflight?.thread_id === activePreviewRuntimeKey &&
          previewAppPreflight.managed
        ? previewAppPreflight
        : null;
  const canOpenGitPanel = gitStatus?.state === "ready" && gitStatus.is_repo;
  const gitPanelChecking = Boolean(folder.trim()) && gitStatus === null;
  const canShowGitPanel =
    canOpenGitPanel || (inspectorTab === "git" && gitPanelChecking);
  const gitStatusMatchesActiveFolder =
    !gitStatus?.folder || previewRuntimeFoldersEqual(gitStatus.folder, folder);
  const emptyStarterGitStatus = gitStatusMatchesActiveFolder ? gitStatus : null;
  const emptyStarterStatusLoading =
    gitStatusLoading ||
    Boolean(folder.trim() && gitStatus?.folder && !gitStatusMatchesActiveFolder);
  const emptyStarterStrip = useMemo(() => {
    if (quickActionMode === "pinned") {
      const projectId = projects.find((project) => project.folder === folder)?.id;
      const actions = projectId && Object.prototype.hasOwnProperty.call(projectQuickActionOverrides, projectId)
        ? projectQuickActionOverrides[projectId]
        : pinnedQuickActions;
      return {
        context: null,
        loading: false,
        suggestions: actions.map((action) => ({
          id: action.id,
          label: action.label,
          detail: "Pinned prompt",
          prompt: action.prompt,
          icon: "pencil" as const,
        })),
      };
    }
    const strip = buildEmptyStarterStrip(folder, emptyStarterGitStatus, emptyStarterStatusLoading);
    if (!personalizedSuggestions || strip.loading) return strip;
    return {
      ...strip,
      suggestions: strip.suggestions
        .map((suggestion, index) => ({ suggestion, index, count: suggestionUsage[`quick:${suggestion.id}`]?.count ?? 0 }))
        .sort((left, right) => right.count - left.count || left.index - right.index)
        .map(({ suggestion }) => suggestion),
    };
  }, [emptyStarterGitStatus, emptyStarterStatusLoading, folder, personalizedSuggestions, pinnedQuickActions, projectQuickActionOverrides, projects, quickActionMode, suggestionUsage]);
  const activeAgent = useMemo(
    () => agents.find((agent) => agent.id === activeAgentId) ?? null,
    [activeAgentId, agents],
  );
  const workspaceProjects = useMemo(
    () =>
      projects
        .filter((project) => !project.archivedAt)
        .map((project) => ({ name: project.name, folder: project.folder })),
    [projects],
  );
  const milimUsage = useMemo(
    () => summarizeMilimUsage(sessionSummaries, projects),
    [projects, sessionSummaries],
  );
  const effectiveModel = activeWorker?.model || model;
  const {
    compactionInFlightRef,
    deleteGoal,
    drainQueuedMessages,
    generationControllersRef,
    goalLoopRef,
    openGoalPanel,
    pauseGoalRun,
    runTurnAndDrain,
    saveGoalDraft,
    sessionGoal,
    updateGoalState,
  } = useChatConversationController({
    queueInterrupts,
    setQueueInterrupts,
    generatingSessionIds,
    liveWorkerSessionIdsKey,
    setChatNotice,
    setGoalPanelOpen,
    setGoalPrefill,
    sessionMessages,
    runTurn,
  });
  const persistingTurnIdsRef = useRef<Set<string>>(new Set());
  const canonicalRunIdsRef = useRef<Map<string, string>>(new Map());
  const {
    activeMediaTarget,
    mediaAdvanced,
    mediaError,
    mediaKind,
    mediaModelEntries,
    mediaParameterValues,
    mediaSchema,
    mediaSchemaLoading,
    sendMediaPrompt,
    setMediaKind,
    updateInlineMediaAdvanced,
    updateInlineMediaParameter,
  } = useChatMediaController({
    providers,
    effectiveModel,
    mediaSettings,
    setMediaSettings,
    pendingAttachments,
    setInput,
    setPendingAttachments,
    setChatNotice,
    generationControllersRef,
    createRequestId: attachmentId,
  });
  const quickSummary = useMemo(
    () =>
      buildQuickSummary({
        folder,
        model: effectiveModel,
        privacy,
        memory,
        planMode,
        goal,
        gitStatus,
        messages,
        pendingAttachments,
        previewUrl: activePreviewRuntime?.url ?? null,
        turnRunning: generatingSessionIds.includes(activeId),
      }),
    [
      activePreviewRuntime?.url,
      activeId,
      effectiveModel,
      folder,
      gitStatus,
      goal,
      memory,
      messages,
      pendingAttachments,
      planMode,
      privacy,
      generatingSessionIds,
    ],
  );
  function currentVirtualProjectFiles(sessionId = activeId): PreviewAppFile[] {
    return sessionVirtualProjectFiles(
      useSessions
        .getState()
        .sessions.find((session) => session.id === sessionId),
    );
  }

  function currentVirtualFile(
    path: string,
    sessionId = activeId,
  ): SessionVirtualFile | undefined {
    const normalized = normalizeVirtualFilePath(path);
    if (!normalized) return undefined;
    return useSessions
      .getState()
      .sessions.find((session) => session.id === sessionId)?.virtualFiles?.[
      normalized
    ];
  }

  function virtualRuntimeFilesWith(
    updates: readonly PreviewAppFile[],
  ): PreviewAppFile[] {
    return mergePreviewAppFiles(currentVirtualProjectFiles(), updates);
  }

  const pickerModels = useMemo(
    () => mergeModelListsForPicker(models, mediaModelEntries),
    [models, mediaModelEntries],
  );
  const proactiveModelBlocker = useMemo(
    () => modelComposerBlocker({
      modelsLoaded,
      selectedModel: effectiveModel,
      models: pickerModels,
      providers,
      accountRuntimeEnabled,
    }),
    [accountRuntimeEnabled, effectiveModel, modelsLoaded, pickerModels, providers],
  );
  const composerNotice = prioritizeComposerNotice(chatNotice, proactiveModelBlocker);
  const composerAction: ComposerBlockerAction | null = composerNotice
    ? composerNotice === proactiveModelBlocker
      ? proactiveModelBlocker.action
      : composerNoticeAction(composerNotice.message)
    : null;
  const composerActionLabel = composerAction === "manage_models"
    ? "Manage models"
    : composerAction === "choose_folder"
      ? "Choose folder"
      : composerAction === "privacy_settings"
        ? "Review privacy"
        : "";
  const composerCompletionRequest = useMemo(() => {
    if (composerCompletionMode === "off" || !model.trim() || isCodexModel(model) || isClaudeModel(model) || isOpenCodeModel(model) || isPiModel(model)) return undefined;
    const modelInfo = pickerModels.find((item) => item.id === model);
    const provider = providers.find((item) => item.id === modelInfo?.provider_id);
    if (!provider) return undefined;
    if (composerCompletionMode === "local" && !isLoopbackProviderEndpoint(provider.base_url)) return undefined;
    if (composerCompletionMode === "current" && !remoteCompletionConfirmed) return undefined;
    return (text: string, signal: AbortSignal) => requestComposerCompletion(model, text, signal);
  }, [composerCompletionMode, model, pickerModels, providers, remoteCompletionConfirmed]);
  const unavailableDefaultHandledRef = useRef("");
  useEffect(() => {
    if (!modelsLoaded || !configuredNewThreads || messages.length || !model.trim()) return;
    if (pickerModels.some((item) => item.id === model)) return;
    const key = `${activeId}:${model}:${unavailableModelPolicy}`;
    if (unavailableDefaultHandledRef.current === key) return;
    unavailableDefaultHandledRef.current = key;
    if (unavailableModelPolicy === "blocked") {
      setChatNotice({ tone: "error", message: `${model} is unavailable. Configure its provider or choose another model.` });
      return;
    }
    const favorite = unavailableModelPolicy === "favorite"
      ? favoriteModels.find((favoriteId) => pickerModels.some((item) => item.id === favoriteId))
      : undefined;
    updateThreadSettings(activeId, { model: favorite ?? "" });
    if (!favorite) {
      setChatNotice({ tone: "info", message: `${model} is unavailable. Choose a model to continue.` });
      setProvidersOpen(true);
    }
  }, [activeId, configuredNewThreads, favoriteModels, messages.length, model, modelsLoaded, pickerModels, unavailableModelPolicy, updateThreadSettings]);
  const activeWorkerRunning =
    activeWorker?.status === "queued" ||
    activeWorker?.status === "running" ||
    activeWorkerRun?.run.status === "running";
  const busy = generatingSessionIds.includes(activeId) || activeWorkerRunning;
  const latestTurnMessage = messages[messages.length - 1];
  const latestTurnCheckpoint = latestTurnMessage?.workspaceCheckpoint;
  const latestTurnChangesKey =
    latestTurnMessage?.role === "assistant" && latestTurnCheckpoint
      ? `${activeId}:${latestTurnMessage.id ?? messages.length - 1}:${latestTurnCheckpoint.ref}`
      : "";

  useEffect(() => {
    if (
      busy ||
      !latestTurnChangesKey ||
      latestTurnMessage?.role !== "assistant" ||
      latestTurnMessage.plan ||
      !latestTurnCheckpoint ||
      !previewRuntimeFoldersEqual(latestTurnCheckpoint.folder, folder)
    ) {
      setTurnReview(null);
      return;
    }

    let cancelled = false;
    setTurnReview({
      key: latestTurnChangesKey,
      status: "checking",
      checkpoint: latestTurnCheckpoint,
    });
    void (async () => {
      try {
        const selected = await setWorkspace(latestTurnCheckpoint.folder);
        if (cancelled) return;
        if (!selected) {
          setTurnReview({
            key: latestTurnChangesKey,
            status: "unavailable",
            checkpoint: latestTurnCheckpoint,
            message: "The turn workspace is unavailable.",
          });
          return;
        }
        const result = await runWorkspaceGitAction("diff", {
          diff_scope: "last_turn",
          diff_base: latestTurnCheckpoint.ref,
        });
        if (cancelled) return;
        setTurnReview(
          turnReviewFromDiff(
            latestTurnChangesKey,
            latestTurnCheckpoint,
            result,
          ),
        );
      } catch (error) {
        if (!cancelled)
          setTurnReview({
            key: latestTurnChangesKey,
            status: "unavailable",
            checkpoint: latestTurnCheckpoint,
            message:
              error instanceof Error
                ? error.message
                : "Git could not load this turn's diff.",
          });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    activeId,
    busy,
    folder,
    latestTurnChangesKey,
    latestTurnCheckpoint,
    latestTurnMessage,
    turnReviewRevision,
  ]);

  const chatScrollRef = useRef<HTMLDivElement>(null);
  const chatBodyRef = useRef<HTMLDivElement>(null);
  const previewResizeHandleRef = useRef<HTMLDivElement>(null);
  const contextLauncherRef = useRef<HTMLButtonElement>(null);
  const stickToBottomRef = useRef(true);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  const {
    approvedWorkerRunsRef,
    childThreadEventControllersRef,
    childThreadEventsRef,
    childThreadLiveIdsRef,
    resumingWorkerRunsRef,
    workerRunEventControllersRef,
    workerRunReconcileRetriesRef,
  } = useChatWorkerController();
  const previewResizeStartRef = useRef<{
    clientX: number;
    width: number;
    intentWidth: number;
    latestWidth: number;
    pointerId: number;
    target: HTMLDivElement;
    snappedClosed: boolean;
    sidebarWasOpen: boolean;
    sidebarAutoCollapsed: boolean;
    sidebarCollapseBoundary: number;
    overlayBoundary: number;
    overlayActive: boolean;
  } | null>(null);
  const previewResizeCleanupRef = useRef<(() => void) | null>(null);
  const previewCloseTimeoutRef = useRef<number | null>(null);
  const stopShortcutConfirmUntilRef = useRef(0);
  const stopShortcutConfirmTimerRef = useRef<number | null>(null);
  const currentThreadIdRef = useRef(activeId);
  const recentThreadIdsRef = useRef<string[]>([activeId]);
  const recentThreadSwitcherTimerRef = useRef<number | null>(null);
  const restoredPreviewThreadRef = useRef<string | null>(null);
  const sidePanelOpenRef = useRef(false);
  const inspectorInvokerRef = useRef<HTMLElement | null>(null);
  const artifactSelectionsByThreadRef = useRef(
    new Map<string, PreviewSelection>(),
  );
  const previewSourcesByThreadRef = useRef(
    new Map<string, InspectorPreviewSource>(),
  );
  const preparedPreviewFilesByThreadRef = useRef(
    new Map<string, PreviewAppFile[]>(),
  );
  const scheduleRunPollingRef = useRef(false);
  const gitStatusUpdatedAtRef = useRef<number | null>(null);
  const [previewResizing, setPreviewResizing] = useState(false);
  const [previewPanelOverlay, setPreviewPanelOverlay] = useState(false);
  const [recentThreadSwitcher, setRecentThreadSwitcher] =
    useState<RecentThreadSwitcherState | null>(null);

  useEffect(() => {
    const body = chatBodyRef.current;
    if (!body) return;
    const update = () => setChatBodyWidth(body.getBoundingClientRect().width);
    update();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update);
      return () => window.removeEventListener("resize", update);
    }
    const observer = new ResizeObserver(update);
    observer.observe(body);
    return () => observer.disconnect();
  }, []);

  function setInput(nextInput: SetStateAction<string>) {
    setInputState((current) => {
      const next =
        typeof nextInput === "function" ? nextInput(current) : nextInput;
      setSessionComposerDraft(activeId, next);
      return next;
    });
  }

  function clearPreviewCloseTimer() {
    if (previewCloseTimeoutRef.current == null) return;
    window.clearTimeout(previewCloseTimeoutRef.current);
    previewCloseTimeoutRef.current = null;
  }

  useEffect(() => {
    if (sessionsHydrated) return;
    if (useSessions.persist.hasHydrated()) {
      setSessionsHydrated(true);
      return;
    }
    return useSessions.persist.onFinishHydration(() =>
      setSessionsHydrated(true),
    );
  }, [sessionsHydrated]);

  useEffect(() => {
    if (currentThreadIdRef.current === activeId) return;
    currentThreadIdRef.current = activeId;
    recentThreadIdsRef.current = rememberRecentThread(
      recentThreadIdsRef.current,
      activeId,
    );
    const nextDraft = getSessionComposerDraft(activeId);
    setInputState(nextDraft);
    if (!nextDraft && messages.length === 0) focusComposer();
  }, [activeId, messages.length]);

  useEffect(() => {
    const syncDraft = () => {
      if (input) return;
      const nextDraft = getSessionComposerDraft(activeId);
      if (nextDraft) setInputState(nextDraft);
    };
    window.addEventListener("milim:session-drafts-hydrated", syncDraft);
    return () =>
      window.removeEventListener("milim:session-drafts-hydrated", syncDraft);
  }, [activeId, input]);

  useEffect(() => {
    return () => {
      if (recentThreadSwitcherTimerRef.current != null) {
        window.clearTimeout(recentThreadSwitcherTimerRef.current);
      }
    };
  }, []);

  function scrollToChatBottom() {
    const el = chatScrollRef.current;
    if (!el) return;
    el.scrollTo({ top: el.scrollHeight, behavior: "auto" });
  }

  function jumpToLatest() {
    stickToBottomRef.current = true;
    setShowJumpToLatest(false);
    scrollToChatBottom();
  }

  function isLiveChildThread(thread: ChildThreadInfo): boolean {
    return thread.status === "queued" || thread.status === "running";
  }

  function stopChildThreadEventsIfIdle(parentId: string) {
    const live = childThreadLiveIdsRef.current.get(parentId);
    if (live?.size) return;
    childThreadLiveIdsRef.current.delete(parentId);
    const controller = childThreadEventControllersRef.current.get(parentId);
    if (!controller) return;
    controller.abort();
    childThreadEventControllersRef.current.delete(parentId);
  }

  function applyPushedChildThreadEvent(parentId: string, ev: AgentEvent) {
    const thread = ev.thread;
    if (!thread || thread.parent_id !== parentId) return;
    if (thread.run_id?.trim()) return;
    const live =
      childThreadLiveIdsRef.current.get(parentId) ?? new Set<string>();
    childThreadLiveIdsRef.current.set(parentId, live);
    if (isLiveChildThread(thread)) live.add(thread.id);
    else live.delete(thread.id);

    const store = useSessions.getState();
    const events = ev.event
      ? rememberWorkerThreadEvent(childThreadEventsRef.current, ev.event)
      : childThreadEventsRef.current.get(thread.id);
    if (ev.type === "child_thread_started") {
      store.upsertChildThread(parentId, thread, events);
    } else if (
      ev.type === "child_thread_event" ||
      ev.type === "child_thread_done" ||
      ev.type === "child_thread_error" ||
      ev.type === "child_thread_stopped"
    ) {
      store.updateChildThread(thread, events);
    }
    if (
      !isLiveChildThread(thread) &&
      !generationControllersRef.current.has(parentId)
    ) {
      stopChildThreadEventsIfIdle(parentId);
    }
  }

  function startChildThreadEvents(parentId: string) {
    if (childThreadEventControllersRef.current.has(parentId)) return;
    const controller = new AbortController();
    childThreadEventControllersRef.current.set(parentId, controller);
    childThreadLiveIdsRef.current.set(
      parentId,
      childThreadLiveIdsRef.current.get(parentId) ?? new Set(),
    );
    void (async () => {
      let afterSeq = 0;
      let retry = 0;
      while (!controller.signal.aborted) {
        try {
          await streamChildThreadEvents(
            parentId,
            (event) => {
              if (event.event?.seq)
                afterSeq = Math.max(afterSeq, event.event.seq);
              retry = 0;
              applyPushedChildThreadEvent(parentId, event);
            },
            controller.signal,
            afterSeq,
          );
        } catch (error) {
          if (!controller.signal.aborted)
            console.warn("child thread event stream failed", error);
        }
        if (controller.signal.aborted) break;
        const live = childThreadLiveIdsRef.current.get(parentId);
        if (!live?.size && !generationControllersRef.current.has(parentId))
          break;
        await waitForEventReconnect(controller.signal, retry);
        retry = Math.min(retry + 1, 4);
      }
    })().finally(() => {
        if (
          childThreadEventControllersRef.current.get(parentId) === controller
        ) {
          childThreadEventControllersRef.current.delete(parentId);
        }
      });
  }

  function applyWorkerRunEvent(event: AgentEvent) {
    const store = useSessions.getState();
    const run =
      event.run ??
      store.workerRuns.find((record) => record.run.id === event.run_id)?.run;
    if (run) {
      const record = {
        run,
        workers: event.workers ?? (event.worker ? [event.worker] : []),
      };
      store.upsertWorkerRun(record);
      const merged = useSessions.getState().workerRuns.find((item) => item.run.id === run.id);
      if (merged) void maybeResumeAfterWorkerRun(merged);
    }
    if (event.event?.id && (run?.id || event.run_id))
      store.setWorkerRunEvent(run?.id ?? event.run_id!, event.event);
  }

  async function maybeResumeAfterWorkerRun(record: WorkerRunRecord) {
    const store = useSessions.getState();
    const sessionId = record.run.parent_thread_id;
    const session = store.sessions.find((item) => item.id === sessionId);
    const pending =
      approvedWorkerRunsRef.current.has(record.run.id) ||
      session?.pendingWorkerRunIds?.includes(record.run.id);
    if (
      !pending ||
      !["done", "partial", "stopped", "error"].includes(record.run.status) ||
      resumingWorkerRunsRef.current.has(record.run.id)
    ) return;

    resumingWorkerRunsRef.current.add(record.run.id);
    try {
      const canonical = await getWorkerRun(record.run.id);
      store.upsertWorkerRun(canonical);
      if (!workerRunReadyForSynthesis(canonical)) {
        retryWorkerRunReconciliation(canonical);
        return;
      }
      workerRunReconcileRetriesRef.current.delete(canonical.run.id);

      const currentMessages = sessionMessages(sessionId);
      const nextMessages = appendWorkerRunSynthesisOnce(
        currentMessages,
        workerRunSynthesisMessage(canonical),
      );
      approvedWorkerRunsRef.current.delete(canonical.run.id);
      workerRunEventControllersRef.current.get(canonical.run.id)?.abort();
      if (!nextMessages) {
        store.setWorkerRunPending(sessionId, canonical.run.id, false);
        return;
      }
      setMessages(sessionId, nextMessages, { autoTitle: false });
      const settings = store.getSettings(sessionId);
      if (settings.goal.status === "waiting_for_worker_approval") {
        const runningGoal = updateGoalState(sessionId, {
          status: "running",
          lastReason: "Worker results joined. Goal resumed.",
        });
        goalLoopRef.current = { sessionId, stopped: false };
        void runGoalLoop(sessionId, settings.model, runningGoal, true);
        store.setWorkerRunPending(sessionId, canonical.run.id, false);
        return;
      }
      const resumed = runTurn(
        nextMessages,
        settings.model,
        { delegationPolicyOverride: "off" },
        sessionId,
      );
      store.setWorkerRunPending(sessionId, canonical.run.id, false);
      void resumed.then((result) => {
        if (result.status === "done")
          void drainQueuedMessages(sessionId, settings.model);
      });
    } catch (error) {
      console.warn("worker run reconciliation failed", error);
      retryWorkerRunReconciliation(record);
    } finally {
      resumingWorkerRunsRef.current.delete(record.run.id);
    }
  }

  function retryWorkerRunReconciliation(record: WorkerRunRecord) {
    const attempts = workerRunReconcileRetriesRef.current.get(record.run.id) ?? 0;
    if (attempts >= 3) return;
    workerRunReconcileRetriesRef.current.set(record.run.id, attempts + 1);
    window.setTimeout(() => {
      const latest = useSessions
        .getState()
        .workerRuns.find((item) => item.run.id === record.run.id);
      void maybeResumeAfterWorkerRun(latest ?? record);
    }, 500 * 2 ** attempts);
  }

  function startWorkerRunEvents(record: WorkerRunRecord) {
    const run = record.run;
    if (
      run.status !== "proposed" &&
      run.status !== "running"
    )
      return;
    if (workerRunEventControllersRef.current.has(run.id)) return;
    const controller = new AbortController();
    workerRunEventControllersRef.current.set(run.id, controller);
    void (async () => {
      const persisted = useSessions
        .getState()
        .workerRuns.find((item) => item.run.id === run.id);
      let afterSeq = (persisted?.workers ?? []).reduce(
        (runMax, worker) =>
          Math.max(
            runMax,
            ...(worker.events ?? []).map((event) => event.seq),
          ),
        0,
      );
      let retry = 0;
      while (!controller.signal.aborted) {
        let terminalEvent = false;
        try {
          await streamWorkerRunEvents(
            run.id,
            (event) => {
              if (event.event?.seq)
                afterSeq = Math.max(afterSeq, event.event.seq);
              retry = 0;
              applyWorkerRunEvent(event);
              terminalEvent = Boolean(
                event.run &&
                  ["done", "partial", "stopped", "error"].includes(
                    event.run.status,
                  ),
              );
              if (terminalEvent) controller.abort();
            },
            controller.signal,
            afterSeq,
          );
        } catch (error) {
          if (!controller.signal.aborted)
            console.warn("worker run event stream failed", error);
        }
        if (controller.signal.aborted && !terminalEvent) break;
        try {
          const canonical = await getWorkerRun(run.id);
          useSessions.getState().upsertWorkerRun(canonical);
          await maybeResumeAfterWorkerRun(canonical);
          if (
            ["done", "partial", "stopped", "error"].includes(
              canonical.run.status,
            )
          )
            break;
        } catch (error) {
          if (!controller.signal.aborted)
            console.warn("worker run reconciliation failed", error);
        }
        if (controller.signal.aborted) break;
        await waitForEventReconnect(controller.signal, retry);
        retry = Math.min(retry + 1, 4);
      }
    })().finally(() => {
        if (workerRunEventControllersRef.current.get(run.id) === controller)
          workerRunEventControllersRef.current.delete(run.id);
      });
  }

  useEffect(() => {
    if (!sessionsHydrated || !activeId) return;
    let cancelled = false;
    void listWorkerRuns(activeId)
      .then((records) => {
        if (cancelled) return;
        const store = useSessions.getState();
        const pending = new Set(
          store.sessions.find((session) => session.id === activeId)
            ?.pendingWorkerRunIds ?? [],
        );
        for (const record of records) {
          store.upsertWorkerRun(record);
          if (pending.has(record.run.id)) {
            approvedWorkerRunsRef.current.add(record.run.id);
            void maybeResumeAfterWorkerRun(record);
          }
          startWorkerRunEvents(record);
        }
      })
      .catch(() => {
        // Older embedded servers do not expose Worker Runs yet.
      });
    return () => {
      cancelled = true;
    };
  }, [
    activeId,
    activeSession?.browserSession,
    sessionsHydrated,
    setSessionBrowserSession,
  ]);

  useEffect(() => {
    for (const record of activeWorkerRuns) startWorkerRunEvents(record);
  }, [activeWorkerRuns]);

  useEffect(() => {
    const running = new Set(generatingSessionIds);
    generationControllersRef.current.forEach((controller, id) => {
      if (running.has(id)) return;
      controller.abort();
      generationControllersRef.current.delete(id);
      stopChildThreadEventsIfIdle(id);
    });
  }, [generatingSessionIds]);

  useEffect(() => {
    if (!sessionsHydrated) return;
    let disposed = false;
    let controller: AbortController | null = null;
    let retryTimer: number | null = null;
    const syncActiveRun = async () => {
      if (
        disposed ||
        canonicalRunIdsRef.current.has(activeId) ||
        generationControllersRef.current.has(activeId)
      ) {
        if (!disposed) retryTimer = window.setTimeout(syncActiveRun, 500);
        return;
      }
      try {
        const bootstrap = await getControlBootstrap();
        const run = bootstrap.active_runs.find((item) => item.thread_id === activeId);
        if (!run || disposed || generationControllersRef.current.has(activeId)) return;
        const store = useSessions.getState();
        controller = claimTurnGeneration({
          sessionId: activeId,
          store,
          generationControllersRef,
        });
        if (!controller) return;
        canonicalRunIdsRef.current.set(activeId, run.id);
        await pollControlRun(activeId, run.id, controller.signal, (items) => {
          if (disposed) return;
          const projected = projectControlRunMessages(items, run.id);
          if (!projected.length) return;
          const current = sessionMessages(activeId);
          setMessages(
            activeId,
            mergeControlRunMessages(current, run.id, projected),
            { autoTitle: autoTitleChats },
          );
        });
      } catch {
        // Startup remains usable with an older embedded server; control
        // compatibility is surfaced by the next explicit send.
      } finally {
        if (controller) {
          releaseTurnGeneration({
            sessionId: activeId,
            store: useSessions.getState(),
            generationControllersRef,
          });
        }
        if (canonicalRunIdsRef.current.get(activeId)) {
          canonicalRunIdsRef.current.delete(activeId);
        }
        controller = null;
        if (!disposed) retryTimer = window.setTimeout(syncActiveRun, 500);
      }
    };
    void syncActiveRun();
    return () => {
      disposed = true;
      if (retryTimer !== null) window.clearTimeout(retryTimer);
      controller?.abort();
    };
  }, [activeId, autoTitleChats, sessionsHydrated, setMessages]);

  function updateAutoScrollCoupling() {
    const el = chatScrollRef.current;
    if (!el) return;
    const following = isNearScrollBottom(el);
    stickToBottomRef.current = following;
    setShowJumpToLatest(!following);
  }

  useEffect(() => {
    if (modelsLoaded && !model && models[0]?.id) {
      updateThreadSettings(activeId, { model: models[0].id });
    }
  }, [activeId, model, models, modelsLoaded, updateThreadSettings]);

  useLayoutEffect(() => {
    if (stickToBottomRef.current) scrollToChatBottom();
  }, [messages]);

  useEffect(() => {
    stickToBottomRef.current = true;
    setShowJumpToLatest(false);
    scrollToChatBottom();
    setPendingAttachments([]);
    setChatNotice(null);
  }, [activeId]);

  useEffect(() => {
    if (!composerDraft) return;
    setInput(composerDraft.text);
    setChatNotice({
      tone: "info",
      message: "Git action loaded into composer.",
    });
    window.requestAnimationFrame(() => {
      document
        .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
        ?.focus();
    });
    onComposerDraftConsumed?.(composerDraft.id);
  }, [composerDraft, onComposerDraftConsumed]);

  useEffect(() => {
    if (!gitPanelRequest) return;
    if (gitPanelRequest.sessionId && gitPanelRequest.sessionId !== activeId)
      return;
    openGitPanel(gitPanelRequest.view);
  }, [activeId, gitPanelRequest]);

  useEffect(() => {
    if (!mcpManagerRequest) return;
    setMcpOpen(true);
  }, [mcpManagerRequest]);

  useEffect(() => {
    if (!chatSearchRequest) return;
    setChatSearchOpen(true);
  }, [chatSearchRequest]);

  useEffect(() => {
    return () => {
      if (previewCloseTimeoutRef.current != null) {
        window.clearTimeout(previewCloseTimeoutRef.current);
        previewCloseTimeoutRef.current = null;
      }
      if (stopShortcutConfirmTimerRef.current != null) {
        window.clearTimeout(stopShortcutConfirmTimerRef.current);
        stopShortcutConfirmTimerRef.current = null;
      }
    };
  }, []);

  // Keep the server's host working folder in sync with the picked folder, so
  // the read_file/write_file/edit_file/list_dir/shell tools operate within it.
  useEffect(() => {
    let cancelled = false;
    const nextFolder = folder ?? "";
    gitStatusUpdatedAtRef.current = null;
    setGitStatus(null);
    setGitStatusLoading(Boolean(nextFolder.trim()));
    void (async () => {
      const workspaceSet = await setWorkspace(nextFolder);
      if (cancelled) return;
      if (!workspaceSet || !nextFolder.trim()) {
        gitStatusUpdatedAtRef.current = Date.now();
        setGitStatusLoading(false);
        return;
      }
      const nextStatus = await getWorkspaceGitStatus();
      if (cancelled) return;
      gitStatusUpdatedAtRef.current = Date.now();
      setGitStatus(nextStatus);
      setGitStatusLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [folder]);

  useEffect(() => {
    if (!folder.trim() || messages.length !== 0) return;
    let cancelled = false;
    async function refreshGitStatusIfStale() {
      if (
        !documentVisible() ||
        !shouldRefreshGitStatus(gitStatusUpdatedAtRef.current, Date.now())
      ) {
        return;
      }
      const nextStatus = await getWorkspaceGitStatus();
      if (cancelled) return;
      gitStatusUpdatedAtRef.current = Date.now();
      if (nextStatus) setGitStatus(nextStatus);
    }
    const timer = window.setInterval(
      () => void refreshGitStatusIfStale(),
      GIT_STATUS_REFRESH_INTERVAL_MS,
    );
    const onVisible = () => void refreshGitStatusIfStale();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [folder, messages.length]);

  // Keep the server's computer-use gate in sync with the toggle.
  useEffect(() => {
    void setComputerUse(computerUse);
  }, [computerUse]);

  // Keep the server's process-global outbound privacy gate in sync with this thread's selected mode.
  useEffect(() => {
    void setPrivacyMode(privacy);
  }, [privacy]);

  function setPlanModeActive(active: boolean): boolean {
    if (active) {
      setGoalComposerSessions((current) => {
        if (!current[activeId]) return current;
        const next = { ...current };
        delete next[activeId];
        return next;
      });
    }
    updateThreadSettings(activeId, { planMode: active });
    setChatNotice(
      active
        ? {
            tone: "info",
            message: "Plan Mode on. Tools are limited to read-only inspection.",
          }
        : null,
    );
    return true;
  }

  function setGoalComposerModeActive(active: boolean): boolean {
    if (
      active &&
      (goal.status === "running" ||
        goal.status === "waiting_for_worker_approval")
    ) {
      setChatNotice({
        tone: "info",
        message: "A goal is already active. Open its Goal pill to review or pause it.",
      });
      return true;
    }
    setGoalComposerSessions((current) => {
      if (Boolean(current[activeId]) === active) return current;
      const next = { ...current };
      if (active) next[activeId] = true;
      else delete next[activeId];
      return next;
    });
    if (active) updateThreadSettings(activeId, { planMode: false });
    setChatNotice(
      active
        ? {
            tone: "info",
            message: "Goal mode on. Your next prompt becomes the goal objective.",
          }
        : null,
    );
    return true;
  }

  const tokens = useMemo(() => {
    const fixed: ChatMessage[] = instructions.trim()
      ? [{ role: "system", content: instructions.trim() }]
      : [];
    const draft: ChatMessage[] = input.trim() || pendingAttachments.length
      ? [{ role: "user", content: input, attachments: pendingAttachments }]
      : [];
    return estimateMessagesTokens(
      messagesForModelContext(fixed, [...messages, ...draft]),
    );
  }, [messages, input, instructions, pendingAttachments]);
  const activeContextBudget = useMemo(
    () => modelContextBudget(effectiveModel.trim(), pickerModels),
    [effectiveModel, pickerModels],
  );

  function artifactRevisionChoice(messageIndex: number, artifactIndex: number) {
    return artifactRevisionsByOccurrence.get(
      artifactOccurrenceKey(messageIndex, artifactIndex),
    );
  }

  const latestPreviewSelection = useMemo((): PreviewSelection | null => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const message = messages[i];
      if (message.role !== "assistant") continue;
      const previewDeferred = busy && i === messages.length - 1;
      const localhostUrl = extractLocalhostUrlFromRunTrace(message.run);
      if (localhostUrl) {
        const artifact = localhostPreviewArtifact(localhostUrl);
        return {
          artifact,
          artifacts: [artifact],
          previewDeferred: false,
          autoOpenKey: `${i}\0${message.run?.startedAt ?? ""}\0localhost\0${localhostUrl}`,
        };
      }
      const completed = preferredPreviewArtifact(message.artifacts);
      if (completed) {
        const artifactIndex =
          message.artifacts?.findIndex(
            (artifact) => artifact.id === completed.id,
          ) ?? -1;
        const choice =
          artifactIndex >= 0
            ? artifactRevisionChoice(i, artifactIndex)
            : undefined;
        return {
          artifact: choice?.revision.artifact ?? completed,
          artifacts: choice?.revision.artifacts ??
            message.artifacts ?? [completed],
          revision: choice?.revision,
          revisionGroup: choice?.group,
          previewDeferred,
          autoOpenKey: previewAutoOpenKey(i, message, completed),
        };
      }
      if (i === messages.length - 1 && message.content) {
        const live = extractLivePreviewArtifactFromContent(message.content);
        if (live)
          return {
            artifact: live,
            artifacts: [live],
            previewDeferred,
            autoOpenKey: previewAutoOpenKey(i, message, live),
          };
      }
    }
    return null;
  }, [artifactRevisionsByOccurrence, busy, messages]);

  const latestRuntimePreview = useMemo((): {
    key: string;
    artifacts: ChatArtifact[];
  } | null => {
    if (folder.trim() || busy) return null;
    for (let i = messages.length - 1; i >= 0; i--) {
      const message = messages[i];
      if (message.role !== "assistant" || !message.artifacts?.length) continue;
      if (extractLocalhostUrlFromRunTrace(message.run)) return null;
      const files = previewRuntimeFiles(message.artifacts);
      if (!files.length || !hasPreviewPackageJson(files)) continue;
      return {
        key: `${i}\0${message.run?.startedAt ?? ""}\0${message.artifacts.map((artifact) => artifact.id).join("\0")}`,
        artifacts: [...message.artifacts],
      };
    }
    return null;
  }, [busy, folder, messages]);

  const activeArtifactSelection =
    artifactSelectionsByThreadRef.current.get(activeId) ??
    latestPreviewSelection;

  const matchingPreviewRuntime = useMemo(() => {
    const status = previewStatusFromRuntime(
      activePreviewRuntimeKey,
      activePreviewRuntime,
    );
    return previewStatusMatchesFolder(status, folder)
      ? activePreviewRuntime
      : undefined;
  }, [activePreviewRuntime, activePreviewRuntimeKey, folder]);
  const runtimePreviewSelection = useMemo(
    () => previewSelectionFromRuntime(matchingPreviewRuntime),
    [matchingPreviewRuntime],
  );
  const activeInspectorPreviewSource =
    previewSourcesByThreadRef.current.get(activeId) ??
    (inspectorTab === "code" || activeArtifactSelection
      ? "artifact"
      : runtimePreviewSelection
        ? "app"
        : "url");
  const activeInspectorBrowserSession =
    activeSession?.browserSession ?? emptyBrowserSession();

  useEffect(() => {
    const restoreKey = `${activeId}:${sessionsHydrated ? "hydrated" : "initial"}`;
    if (restoredPreviewThreadRef.current === restoreKey) return;
    restoredPreviewThreadRef.current = restoreKey;
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    const restoredArtifact =
      artifactSelectionsByThreadRef.current.get(activeId) ??
      latestPreviewSelection ??
      null;
    if (restoredArtifact)
      artifactSelectionsByThreadRef.current.set(activeId, restoredArtifact);
    setPreviewSelection(restoredArtifact);
    const restoredBrowser =
      activeSession?.browserSession ?? emptyBrowserSession();
    if (!activeSession?.browserSession)
      setSessionBrowserSession(activeId, restoredBrowser);
    const restoredSource =
      previewSourcesByThreadRef.current.get(activeId) ??
      (inspectorTab === "code" || restoredArtifact
        ? "artifact"
        : runtimePreviewSelection
          ? "app"
          : "url");
    previewSourcesByThreadRef.current.set(activeId, restoredSource);
    setPreviewSource(restoredSource);
    setDismissedPreviewKey(
      inspectorOpen ? null : (latestPreviewSelection?.autoOpenKey ?? null),
    );
  }, [activeId, sessionsHydrated]);

  useEffect(() => {
    const openUrl = (value: unknown) => {
      if (typeof value !== "string") return;
      const url = normalizeArtifactBrowserUrl(value);
      if (!url) return;
      const current =
        useSessions.getState().sessions.find((session) => session.id === activeId)
          ?.browserSession ??
        emptyBrowserSession();
      const tab = emptyBrowserTab(url);
      const next = {
        ...current,
        tabs: [...current.tabs, tab],
        activeTabId: tab.id,
      };
      setSessionBrowserSession(activeId, next);
      previewSourcesByThreadRef.current.set(activeId, "url");
      setPreviewSource("url");
      setPreviewPanelClosing(false);
      setSessionInspectorTab(activeId, "preview");
      setSessionInspectorOpen(activeId, true);
    };
    const openWindowUrl = (event: Event) =>
      openUrl((event as CustomEvent<{ url?: unknown }>).detail?.url);
    let disposed = false;
    let stopListening: () => void = () => undefined;
    void listenForPreviewOpenUrl((request) => openUrl(request.url))
      .then((unlisten) => {
        if (disposed) unlisten();
        else stopListening = unlisten;
      })
      .catch(() => undefined);
    window.addEventListener("milim-open-browser-url", openWindowUrl);
    return () => {
      disposed = true;
      stopListening();
      window.removeEventListener("milim-open-browser-url", openWindowUrl);
    };
  }, [
    activeId,
    setSessionBrowserSession,
    setSessionInspectorOpen,
    setSessionInspectorTab,
  ]);

  useEffect(() => {
    if (
      !latestPreviewSelection ||
      dismissedPreviewKey === latestPreviewSelection.autoOpenKey
    )
      return;
    if (
      !inspectorOpen ||
      inspectorTab === "git" ||
      inspectorTab === "workers" ||
      activeInspectorPreviewSource !== "artifact" ||
      (activeArtifactSelection?.revision != null &&
        activeArtifactSelection.revision.revisionNumber <
          activeArtifactSelection.revision.totalRevisions)
    ) {
      setDismissedPreviewKey(latestPreviewSelection.autoOpenKey ?? null);
      return;
    }
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    artifactSelectionsByThreadRef.current.set(activeId, latestPreviewSelection);
    setPreviewSelection(latestPreviewSelection);
  }, [
    activeId,
    dismissedPreviewKey,
    inspectorOpen,
    inspectorTab,
    latestPreviewSelection,
    latestPreviewSelection?.artifact.content,
    activeInspectorPreviewSource,
    activeArtifactSelection?.revision?.revisionNumber,
    activeArtifactSelection?.revision?.totalRevisions,
  ]);

  async function openGitPanel(view: GitPanelView = "changes") {
    if (!folder.trim() || (gitStatus && !canOpenGitPanel)) return;
    if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    setDismissedPreviewKey(latestPreviewSelection?.autoOpenKey ?? null);
    setGitPanelView(view);
    setSessionInspectorTab(activeId, "git");
  }

  function reviewTurnChanges(
    checkpoint: WorkspaceCheckpoint,
    result: WorkspaceGitActionResult,
  ) {
    gitDiffRequestIdRef.current += 1;
    setGitDiffRequest({
      id: gitDiffRequestIdRef.current,
      sessionId: activeId,
      folder: checkpoint.folder,
      checkpoint: checkpoint.ref,
      result,
    });
    openGitPanel();
  }

  async function approveWorkerRun(runId: string) {
    setWorkerActionBusy(true);
    const store = useSessions.getState();
    const pendingRun = store.workerRuns.find((item) => item.run.id === runId);
    approvedWorkerRunsRef.current.add(runId);
    if (pendingRun)
      store.setWorkerRunPending(pendingRun.run.parent_thread_id, runId, true);
    try {
      const record = await startWorkerRun(runId);
      store.upsertWorkerRun(record);
      void maybeResumeAfterWorkerRun(record);
      startWorkerRunEvents(record);
    } catch (error) {
      approvedWorkerRunsRef.current.delete(runId);
      if (pendingRun)
        store.setWorkerRunPending(pendingRun.run.parent_thread_id, runId, false);
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setWorkerActionBusy(false);
    }
  }

  async function stopActiveWorkerRun(runId: string) {
    setWorkerActionBusy(true);
    try {
      const store = useSessions.getState();
      const record = store.workerRuns.find((item) => item.run.id === runId);
      approvedWorkerRunsRef.current.delete(runId);
      if (record)
        store.setWorkerRunPending(record.run.parent_thread_id, runId, false);
      store.upsertWorkerRun(await stopWorkerRun(runId));
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setWorkerActionBusy(false);
    }
  }

  async function stopOneWorker(runId: string, workerId: string) {
    const result = await stopWorker(runId, workerId);
    const current = useSessions.getState().workerRuns.find((item) => item.run.id === runId);
    if (!current || !result.run) return;
    useSessions.getState().upsertWorkerRun({
      run: result.run,
      workers: current.workers.map((worker) => worker.id === workerId ? result.worker : worker),
    });
  }

  async function retryFailedWorker(runId: string, taskId: string, model?: string) {
    setWorkerActionBusy(true);
    try {
      const record = await retryWorkerTask(runId, taskId, model);
      approvedWorkerRunsRef.current.add(record.run.id);
      useSessions
        .getState()
        .setWorkerRunPending(record.run.parent_thread_id, record.run.id, true);
      useSessions.getState().upsertWorkerRun(record);
      void maybeResumeAfterWorkerRun(record);
      startWorkerRunEvents(record);
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setWorkerActionBusy(false);
    }
  }

  async function deleteFinishedWorkerRun(runId: string) {
    setWorkerActionBusy(true);
    try {
      const store = useSessions.getState();
      const record = store.workerRuns.find((item) => item.run.id === runId);
      await deleteWorkerRun(runId);
      approvedWorkerRunsRef.current.delete(runId);
      workerRunEventControllersRef.current.get(runId)?.abort();
      workerRunEventControllersRef.current.delete(runId);
      if (record)
        store.setWorkerRunPending(record.run.parent_thread_id, runId, false);
      store.removeWorkerRun(runId);
      setWorkerFocusRunId("");
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setWorkerActionBusy(false);
    }
  }

  async function continueWorkerRunSolo(runId: string) {
    if (busy) return;
    setWorkerActionBusy(true);
    try {
      const store = useSessions.getState();
      const record = store.workerRuns.find((item) => item.run.id === runId);
      approvedWorkerRunsRef.current.delete(runId);
      if (record)
        store.setWorkerRunPending(record.run.parent_thread_id, runId, false);
      store.upsertWorkerRun(await stopWorkerRun(runId));
      const conversation = regenerateTurnConversation(messages);
      if (conversation)
        await runTurnAndDrain(conversation, undefined, {
          delegationPolicyOverride: "off",
        });
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setWorkerActionBusy(false);
    }
  }

  function closeGitPanel() {
    clearPreviewCloseTimer();
    if (prefersReducedMotion()) {
      setPreviewPanelClosing(false);
      setSessionInspectorOpen(activeId, false);
      restoreInspectorInvokerFocus();
      return;
    }
    setPreviewPanelClosing(true);
    previewCloseTimeoutRef.current = window.setTimeout(() => {
      const state = useSessions.getState();
      if (inspectorStateForSession(
        state.inspectorByKey,
        state.sessions.find((session) => session.id === activeId),
      ).tab === "git") {
        setSessionInspectorOpen(activeId, false);
      }
      setPreviewPanelClosing(false);
      previewCloseTimeoutRef.current = null;
      restoreInspectorInvokerFocus();
    }, PREVIEW_PANEL_ANIMATION_MS);
  }

  function rememberInspectorInvoker() {
    if (inspectorOpen || typeof document === "undefined") return;
    const active = document.activeElement;
    if (active instanceof HTMLElement) inspectorInvokerRef.current = active;
  }

  function restoreInspectorInvokerFocus() {
    const target = inspectorInvokerRef.current;
    inspectorInvokerRef.current = null;
    window.requestAnimationFrame(() => {
      const fallback = document.querySelector<HTMLElement>(
        '[data-testid="open-artifact-browser"]',
      );
      (target?.isConnected ? target : fallback)?.focus();
    });
  }

  function loadGitActionDraft(text: string) {
    setInput(text);
    setChatNotice({
      tone: "info",
      message: "Git action loaded into composer.",
    });
    focusComposerInput();
  }

  function focusComposerInput() {
    window.requestAnimationFrame(() => {
      document
        .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
        ?.focus();
    });
  }

  function prefillEmptyStarter(prompt: string) {
    setInput(prompt);
    focusComposerInput();
  }

  async function closePreview() {
    if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    setDismissedPreviewKey(activeArtifactSelection?.autoOpenKey ?? null);
    if (prefersReducedMotion()) {
      clearPreviewCloseTimer();
      setPreviewPanelClosing(false);
      setSessionInspectorOpen(activeId, false);
      restoreInspectorInvokerFocus();
      return;
    }
    clearPreviewCloseTimer();
    setPreviewPanelClosing(true);
    previewCloseTimeoutRef.current = window.setTimeout(() => {
      setSessionInspectorOpen(activeId, false);
      setPreviewPanelClosing(false);
      previewCloseTimeoutRef.current = null;
      restoreInspectorInvokerFocus();
    }, PREVIEW_PANEL_ANIMATION_MS);
  }

  async function openPreviewArtifact(
    artifact: ChatArtifact,
    artifacts?: readonly ChatArtifact[],
    previewDeferred = false,
    revision?: ArtifactRevision,
  ) {
    if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    setDismissedPreviewKey(latestPreviewSelection?.autoOpenKey ?? null);
    const choice = revision
      ? artifactRevisionChoice(revision.messageIndex, revision.artifactIndex)
      : undefined;
    const selection: PreviewSelection = {
      artifact: revision?.artifact ?? artifact,
      artifacts: [
        ...(revision?.artifacts ??
          (artifacts?.length ? artifacts : [artifact])),
      ],
      revision,
      revisionGroup: choice?.group,
      previewDeferred,
    };
    const target = selection.artifact;
    if (target.mime === "text/uri-list") {
      const url = target.content.trim() || null;
      const tab = emptyBrowserTab(url);
      const nextBrowser: InspectorBrowserSession = {
        profileId: `session-${Math.random().toString(36).slice(2)}`,
        tabs: [tab],
        activeTabId: tab.id,
      };
      setSessionBrowserSession(activeId, nextBrowser);
      selectPreviewSource("url");
    } else {
      artifactSelectionsByThreadRef.current.set(activeId, selection);
      setPreviewSelection(selection);
      selectPreviewSource("artifact");
    }
    setSessionInspectorTab(
      activeId,
      isPreviewableArtifact(target) ? "preview" : "code",
    );
  }

  function openQuickSummarySource(source: QuickSummarySource) {
    if (source.kind === "artifact") {
      const revision = artifactRevisionChoice(
        source.messageIndex,
        source.artifactIndex,
      )?.revision;
      openPreviewArtifact(
        revision?.artifact ?? source.artifact,
        revision?.artifacts ?? source.artifacts,
        false,
        revision,
      );
      return;
    }
    if (source.kind === "memory") {
      setMemoryTarget(source.memory);
      setMemoryOpen(true);
      return;
    }
    const attachment = source.attachment;
    if (attachment.sourcePath) {
      void openArtifactLocation(attachment.sourcePath).catch((error) =>
        setChatNotice({
          tone: "error",
          message: `Could not open attachment: ${error instanceof Error ? error.message : String(error)}`,
        }),
      );
      return;
    }
    if (attachment.dataUrl) {
      setAttachmentPreview(attachment);
      return;
    }
    if (attachment.content != null) {
      const artifact: ChatArtifact = {
        id: `attachment-${attachment.id}`,
        kind: attachment.mime === "application/json" ? "json" : "text",
        title: attachment.name,
        mime: attachment.mime,
        content: attachment.content,
        size: textBytes(attachment.content),
        filename: attachment.name,
        language: extensionOf(attachment.name) || undefined,
      };
      openPreviewArtifact(artifact, [artifact]);
      return;
    }
    setChatNotice({ tone: "error", message: "Attachment content is unavailable." });
  }

  function selectPreviewSource(source: InspectorPreviewSource) {
    previewSourcesByThreadRef.current.set(activeId, source);
    setPreviewSource(source);
  }

  function updateBrowserSession(session: InspectorBrowserSession) {
    setSessionBrowserSession(activeId, session);
  }

  function managedPreviewFiles(
    artifacts?: readonly ChatArtifact[],
  ): PreviewAppFile[] {
    const files = previewRuntimeFiles(artifacts);
    if (files.length) return virtualRuntimeFilesWith(files);
    return (
      preparedPreviewFilesByThreadRef.current.get(activeId) ??
      currentVirtualProjectFiles()
    );
  }

  async function preparePreviewRuntimeForArtifacts(
    artifacts?: readonly ChatArtifact[],
  ) {
    const files = folder.trim() ? [] : managedPreviewFiles(artifacts);
    if (!folder.trim() && (!files.length || !hasPreviewPackageJson(files))) {
      setChatNotice({
        tone: "error",
        message: "Preview runtime needs a named package.json artifact.",
      });
      return;
    }
    if (!folder.trim())
      preparedPreviewFilesByThreadRef.current.set(activeId, files);
    selectPreviewSource("app");
    setSessionInspectorTab(activeId, "preview");
    await preflightPreviewRuntime(files);
  }

  async function preflightPreviewRuntime(files?: PreviewAppFile[]) {
    const managedFiles = folder.trim()
      ? undefined
      : (files ??
        preparedPreviewFilesByThreadRef.current.get(activeId) ??
        managedPreviewFiles(latestRuntimePreview?.artifacts));
    if (managedFiles)
      preparedPreviewFilesByThreadRef.current.set(activeId, managedFiles);
    await preflightRuntime(
      folder.trim() ? { cwd: folder } : { files: managedFiles },
    );
  }

  function previewRuntimeRunOptions(): PreviewAppStartOptions | null {
    if (!activePreviewAppPreflight) {
      setChatNotice({
        tone: "info",
        message: "Review the preview commands before running them.",
      });
      return null;
    }
    return folder.trim()
      ? {
          cwd: folder,
          source_fingerprint: activePreviewAppPreflight.source_fingerprint,
        }
      : {
          files: managedPreviewFiles(),
          source_fingerprint: activePreviewAppPreflight.source_fingerprint,
        };
  }

  async function startWorkspaceHtmlPreview(path: string) {
    if (!folder.trim()) return;
    const status = await startStaticRuntime({
      cwd: folder,
      entry_path: path,
    });
    if (!status) return;
    selectPreviewSource("app");
    setSessionInspectorTab(activeId, "preview");
  }

  async function startPreviewRuntime() {
    const options = previewRuntimeRunOptions();
    if (!options) return;
    const status = await startRuntime(options);
    if (!status) return;
    if (!folder.trim() && options.files?.length)
      upsertVirtualFiles(activeId, options.files);
    selectPreviewSource("app");
    setSessionInspectorTab(activeId, "preview");
  }

  async function stopPreviewRuntime() {
    await stopRuntime();
  }

  async function restartPreviewRuntime() {
    const options = previewRuntimeRunOptions();
    if (!options) return;
    const status = await restartRuntime(options);
    if (status && !folder.trim() && options.files?.length)
      upsertVirtualFiles(activeId, options.files);
  }

  async function openArtifactSidePanel(tab: "preview" | "code" = "preview") {
    const selection =
      activeArtifactSelection ??
      latestPreviewSelection ??
      (tab === "code" && folder.trim() ? blankWorkspaceReviewSelection() : null);
    if (!selection) return;
    if (inspectorTab === "code" && tab !== "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    artifactSelectionsByThreadRef.current.set(activeId, selection);
    if (tab === "preview") selectPreviewSource("artifact");
    setSessionInspectorTab(activeId, tab);
    setDismissedPreviewKey(null);
    setPreviewSelection(selection);
  }

  function openSelectedSidePanel() {
    if (inspectorTab === "git" && (canOpenGitPanel || gitPanelChecking)) {
      openGitPanel();
    } else if (inspectorTab === "workers") {
      openWorkersInspector();
    } else if (inspectorTab === "code") {
      openArtifactSidePanel("code");
    } else {
      openPreviewInspector();
    }
  }

  function selectPreviewRevision(revision: ArtifactRevision) {
    const choice = artifactRevisionChoice(
      revision.messageIndex,
      revision.artifactIndex,
    );
    const current = activeArtifactSelection;
    if (!current) return;
    const next = {
      ...current,
      artifact: revision.artifact,
      artifacts: [...revision.artifacts],
      revision,
      revisionGroup: choice?.group ?? current.revisionGroup,
    };
    artifactSelectionsByThreadRef.current.set(activeId, next);
    setPreviewSelection(next);
  }

  const availablePreviewSources: InspectorPreviewSource[] = [
    ...(activeArtifactSelection && isPreviewableArtifact(activeArtifactSelection.artifact)
      ? (["artifact"] as const)
      : []),
    ...(folder.trim() || latestRuntimePreview || activePreviewAppPreflight || activePreviewAppStatus
      ? (["app"] as const)
      : []),
    "url",
  ];
  const visiblePreviewSelection =
    inspectorTab === "code"
      ? (activeArtifactSelection ?? (folder.trim() ? blankWorkspaceReviewSelection() : null))
      : activeInspectorPreviewSource === "artifact"
        ? activeArtifactSelection
        : activeInspectorPreviewSource === "app"
          ? (runtimePreviewSelection ?? blankBrowserPreviewSelection())
          : browserPreviewSelection(activeInspectorBrowserSession);
  const sidePanelVisible = Boolean(
    inspectorOpen &&
    (inspectorTab === "workers"
      ? true
      : inspectorTab === "git"
        ? canShowGitPanel
        : visiblePreviewSelection),
  );
  const contextStacked = contextPanelOpen && chatBodyWidth < CONTEXT_STACK_THRESHOLD;
  const inspectorStacked = sidePanelVisible && chatBodyWidth < INSPECTOR_STACK_THRESHOLD;
  const panelsStacked = contextStacked || inspectorStacked;
  const reservedContextWidth = contextPanelOpen && !contextStacked ? CONTEXT_PANEL_WIDTH : 0;
  const dockedPreviewPanelWidth = maxPreviewPanelWidth(
    chatBodyWidth,
    reservedContextWidth,
  );
  const resolvedPreviewPanelWidth = clampPreviewPanelWidth(
    previewResizeStartRef.current?.latestWidth ?? previewPanelWidth,
    chatBodyWidth,
    reservedContextWidth,
    previewPanelOverlay,
  );
  const previewPanelStyle = {
    "--preview-panel-width": `${resolvedPreviewPanelWidth}px`,
    "--preview-panel-docked-width": `${dockedPreviewPanelWidth}px`,
  } as CSSProperties;
  const inspectorLauncherLabel =
    inspectorTab === "workers"
      ? "Open Workers"
      : inspectorTab === "git"
      ? "Open Git"
      : inspectorTab === "code"
        ? `Open Code: ${activeArtifactSelection?.artifact.filename ?? activeArtifactSelection?.artifact.title ?? "artifact"}`
        : activeInspectorPreviewSource === "artifact"
          ? `Open Preview: ${activeArtifactSelection?.artifact.filename ?? activeArtifactSelection?.artifact.title ?? "artifact"}`
          : activeInspectorPreviewSource === "app"
            ? "Open Preview: App"
            : "Open Preview: URL";
  const previewToolsIntent = Boolean(
    sidePanelVisible &&
      (inspectorTab === "preview" || inspectorTab === "code") &&
      activePreviewSurface?.status === "ready" &&
      activePreviewSurface.capabilities.includes("dom_snapshot"),
  );
  const contextualModelToolIntent = Boolean(
    !planMode &&
      (folder.trim() ||
        sandbox ||
        computerUse ||
        previewToolsIntent ||
        looksLikeMcpToolRequest(input, composerTools) ||
        looksLikeScheduleRequest(input) ||
        (memory && looksLikeMemoryWriteRequest(input))),
  );
  const modelToolIntent = contextualModelToolIntent || Boolean(!planMode && activeAgentId != null);

  function hotSwapAssessment(target: ModelInfo): HotSwapAssessment {
    return assessHotSwap({
      currentModel: model,
      target,
      models: pickerModels,
      providers,
      session: activeSession ?? { messages, accountRuntime: undefined },
      toolRequired: contextualModelToolIntent || Boolean(activeAgentId && activeAgent?.tool_mode !== "none"),
    });
  }

  async function prepareRetryWithModel(
    targetModel: string,
    messageIndex: number,
  ): Promise<void> {
    const source = useSessions.getState().sessions.find((item) => item.id === activeId);
    const assistant = source?.messages[messageIndex];
    if (!source || assistant?.role !== "assistant") return;
    let userIndex = messageIndex - 1;
    while (userIndex >= 0 && source.messages[userIndex].role !== "user") userIndex -= 1;
    const userMessage = source.messages[userIndex];
    if (userIndex < 0 || !userMessage) return;

    let retryFolder = "";
    const originalFolder = source.settings?.folder?.trim() ?? "";
    const readOnlyTurn = Boolean(assistant.plan);
    if (originalFolder && !readOnlyTurn) {
      if (!assistant.workspaceCheckpoint) {
        setChatNotice({
          tone: "error",
          message: "Clean Retry requires a Git workspace checkpoint for this turn.",
        });
        return;
      }
      try {
        await setWorkspace(originalFolder);
        const result = await runWorkspaceGitAction("create_retry_worktree", {
          checkpoint: assistant.workspaceCheckpoint.ref,
        });
        if (!result.ok || !result.worktree) throw new Error(result.message);
        retryFolder = result.worktree;
      } catch (error) {
        setChatNotice({
          tone: "error",
          message: `Retry workspace failed: ${error instanceof Error ? error.message : String(error)}`,
        });
        return;
      }
    }

    const forkedId = useSessions.getState().forkSession(activeId, userIndex - 1);
    if (!forkedId) return;
    useSessions.getState().updateSettings(forkedId, {
      model: targetModel,
      ...(retryFolder ? { folder: retryFolder } : {}),
    });
    if (retryFolder && assistant.workspaceCheckpoint) {
      useSessions.getState().setRetryWorkspace(forkedId, {
        sourceSessionId: activeId,
        sourceMessageId: assistant.id,
        originalFolder,
        worktreeFolder: retryFolder,
        baseCheckpoint: assistant.workspaceCheckpoint.ref,
        createdAt: Date.now(),
      });
    }
    setSessionComposerDraft(forkedId, userMessage.content);
    setInputState(userMessage.content);
    setPendingAttachments(userMessage.attachments ? [...userMessage.attachments] : []);
    setBatonRequest(null);
    setChatNotice({ tone: "info", message: "Retry prepared in an isolated thread." });
    focusComposer();
  }

  function commitHotSwap(
    target: ModelInfo,
    action: HotSwapAction,
    messageIndex?: number,
    nativeSessionMode?: NativeSessionMode,
    _selection: ModelPickerSelection = { model: target.id },
  ) {
    const fromModel = model;
    const kind = target.id.startsWith("codex:")
      ? "codex"
      : target.id.startsWith("claude:")
        ? "claude"
        : target.id.startsWith("opencode:")
          ? "opencode"
          : target.id.startsWith("pi:")
            ? "pi"
          : null;
    if (kind && nativeSessionMode === "fresh") {
      useSessions.getState().clearAccountRuntimeKind(activeId, kind);
    } else if (kind && nativeSessionMode === "resume") {
      const runtime = useSessions.getState().sessions.find((item) => item.id === activeId)?.accountRuntime;
      if (kind === "codex" && runtime?.codexThreadId && !runtime.codexLastSyncedMessageId) {
        useSessions.getState().setAccountRuntime(activeId, {
          codexLastSyncedMessageId: "__milim_hot_swap_full__",
        });
      } else if (kind === "claude" && runtime?.claudeSessionId && !runtime.claudeLastSyncedMessageId) {
        useSessions.getState().setAccountRuntime(activeId, {
          claudeLastSyncedMessageId: "__milim_hot_swap_full__",
        });
      } else if (kind === "opencode" && runtime?.opencodeSessionId && !runtime.opencodeLastSyncedMessageId) {
        useSessions.getState().setAccountRuntime(activeId, {
          opencodeLastSyncedMessageId: "__milim_hot_swap_full__",
        });
      } else if (kind === "pi" && runtime?.piSessionId && !runtime.piLastSyncedMessageId) {
        useSessions.getState().setAccountRuntime(activeId, {
          piLastSyncedMessageId: "__milim_hot_swap_full__",
        });
      }
    }
    if (action === "retry" && messageIndex != null) {
      void prepareRetryWithModel(target.id, messageIndex);
      return;
    }
    updateThreadSettings(activeId, { model: target.id });
    if (action === "continue" || action === "review") {
      useSessions.getState().setPendingHotSwap(activeId, {
        fromModel,
        toModel: target.id,
        action,
        nativeSessionMode,
        sourceMessageId:
          messageIndex == null ? undefined : messages[messageIndex]?.id,
        createdAt: Date.now(),
      });
      setInput(action === "continue" ? HOT_SWAP_CONTINUE_PROMPT : HOT_SWAP_REVIEW_PROMPT);
      focusComposer();
    }
    setBatonRequest(null);
  }

  function requestHotSwap(
    selection: ModelPickerSelection,
    action: HotSwapAction = "switch",
    messageIndex?: number,
  ) {
    if (busy || compactionInFlightRef.current || activeWorker) {
      setChatNotice({ tone: "warning", message: "Wait for the current model-controlled work to finish before switching." });
      return;
    }
    const target = pickerModels.find((item) => item.id === selection.model);
    if (!target) return;
    if (target.capabilities?.imageOutput || target.capabilities?.videoOutput || target.capabilities?.musicOutput) {
      if (action === "switch") commitHotSwap(target, action, messageIndex, undefined, selection);
      return;
    }
    const assessment = hotSwapAssessment(target);
    if (assessment.requiresConfirmation) {
      setBatonRequest(null);
      setHotSwapPreflight({ action, messageIndex, target, assessment, selection });
      return;
    }
    commitHotSwap(target, action, messageIndex, undefined, selection);
  }

  async function applyRetryWorkspace() {
    const retry = activeSession?.retryWorkspace;
    if (!retry || busy) return;
    if (!window.confirm(`Apply this retry diff to the original workspace?\n\n${retry.originalFolder}`)) return;
    try {
      await setWorkspace(retry.originalFolder);
      const result = await runWorkspaceGitAction("apply_retry_worktree", {
        checkpoint: retry.baseCheckpoint,
        worktree: retry.worktreeFolder,
      });
      await setWorkspace(retry.worktreeFolder);
      if (!result.ok) {
        const conflicts = result.conflicts?.length
          ? ` Conflicting paths: ${result.conflicts.join(", ")}`
          : "";
        throw new Error(`${result.message}${conflicts}`);
      }
      useSessions.getState().setRetryWorkspace(activeId, {
        ...retry,
        adoptedAt: Date.now(),
        applyUndoCheckpoint: result.undo_checkpoint,
      });
      setChatNotice({ tone: "info", message: result.message });
    } catch (error) {
      await setWorkspace(retry.worktreeFolder).catch(() => undefined);
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function discardRetryWorkspace() {
    const retry = activeSession?.retryWorkspace;
    if (!retry || busy) return;
    if (!window.confirm("Discard this retry thread and its isolated worktree?")) return;
    try {
      await setWorkspace(retry.originalFolder);
      const result = await runWorkspaceGitAction("remove_retry_worktree", {
        worktree: retry.worktreeFolder,
      });
      if (!result.ok) throw new Error(result.message);
      useSessions.getState().remove(activeId);
      setChatNotice({ tone: "info", message: "Retry worktree discarded." });
    } catch (error) {
      await setWorkspace(retry.worktreeFolder).catch(() => undefined);
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }
  const sidePanelAlreadyOpen =
    sidePanelOpenRef.current && sidePanelVisible && !previewPanelClosing;

  useEffect(() => {
    if (!sidePanelVisible || previewPanelClosing) {
      sidePanelOpenRef.current = false;
      return;
    }
    const timer = window.setTimeout(() => {
      sidePanelOpenRef.current = true;
    }, PREVIEW_PANEL_ANIMATION_MS);
    return () => window.clearTimeout(timer);
  }, [previewPanelClosing, sidePanelVisible]);

  useEffect(() => {
    if (
      contextPanelOpen &&
      sidePanelVisible &&
      chatBodyWidth < CONCURRENT_PANEL_THRESHOLD
    ) {
      setSessionContextPanelOpen(activeId, false);
    }
  }, [activeId, chatBodyWidth, contextPanelOpen, setSessionContextPanelOpen, sidePanelVisible]);

  useEffect(() => {
    setPreviewPanelOverlay(false);
    if (previewResizeStartRef.current) {
      previewResizeStartRef.current.overlayActive = false;
    }
  }, [activeId]);

  useEffect(() => {
    if (sidePanelVisible && !panelsStacked) return;
    setPreviewPanelOverlay(false);
    if (previewResizeStartRef.current) {
      previewResizeStartRef.current.overlayActive = false;
    }
  }, [panelsStacked, sidePanelVisible]);

  useEffect(() => {
    if (
      previewPanelOverlay &&
      !previewResizeStartRef.current &&
      previewPanelWidth <= dockedPreviewPanelWidth
    ) {
      setPreviewPanelOverlay(false);
    }
  }, [dockedPreviewPanelWidth, previewPanelOverlay, previewPanelWidth]);

  useEffect(() => {
    const start = previewResizeStartRef.current;
    if (!start || start.snappedClosed) return;
    resizePreviewPanelDuringDrag(start.intentWidth);
  }, [chatBodyWidth, reservedContextWidth]);

  useEffect(() => {
    if (
      (activeWorkerRun?.run.status === "proposed" ||
        activeWorkerRun?.run.status === "running") &&
      (!sidePanelVisible || inspectorTab !== "workers")
    ) openWorkersInspector(activeWorkerRun.run.id);
  }, [activeWorkerRun?.run.id, activeWorkerRun?.run.status, inspectorTab, sidePanelVisible]);

  useEffect(() => {
    if (
      !sidePanelVisible ||
      (inspectorTab !== "preview" && inspectorTab !== "code")
    ) setActivePreviewSurface(null);
  }, [inspectorTab, sidePanelVisible]);

  async function openWorkersInspector(runId?: string, settings = false) {
    if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    setWorkerFocusRunId(runId ?? "");
    if (settings) setWorkerSettingsOpen(true);
    setSessionInspectorTab(activeId, "workers");
  }

  async function openContextPanel() {
    if (chatBodyWidth < CONCURRENT_PANEL_THRESHOLD && inspectorOpen) {
      if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
      clearPreviewCloseTimer();
      setPreviewPanelClosing(false);
      setSessionInspectorOpen(activeId, false);
    }
    setSessionContextPanelOpen(activeId, true);
  }

  function closeContextPanel() {
    setSessionContextPanelOpen(activeId, false);
    window.requestAnimationFrame(() => contextLauncherRef.current?.focus());
  }

  function resizePreviewPanel(width: number, overlay = previewPanelOverlay) {
    setPreviewPanelWidth(
      clampPreviewPanelWidth(width, chatBodyWidth, reservedContextWidth, overlay),
    );
  }

  function resizePreviewPanelDuringDrag(width: number) {
    const start = previewResizeStartRef.current;
    if (!start) return;
    const bodyWidth = chatBodyRef.current?.getBoundingClientRect().width ?? chatBodyWidth;
    const nextWidth = clampPreviewPanelWidth(
      width,
      bodyWidth,
      reservedContextWidth,
      start.overlayActive,
    );
    start.latestWidth = nextWidth;
    chatBodyRef.current?.style.setProperty("--preview-panel-width", `${nextWidth}px`);
    previewResizeHandleRef.current?.setAttribute("aria-valuenow", String(nextWidth));
    previewResizeHandleRef.current?.setAttribute(
      "aria-valuetext",
      `${nextWidth} pixels, ${start.overlayActive ? "overlay" : "docked"}`,
    );
  }

  function startPreviewResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    const target = event.currentTarget;
    const bodyWidth = chatBodyRef.current?.getBoundingClientRect().width ?? chatBodyWidth;
    const dockedLimit = maxPreviewPanelWidth(bodyWidth, reservedContextWidth);
    const sidebarGain = verticalSidebarOpen
      ? Math.max(0, sidebarWidth - COLLAPSED_SIDEBAR_WIDTH)
      : 0;
    previewResizeStartRef.current = {
      clientX: event.clientX,
      width: resolvedPreviewPanelWidth,
      intentWidth: resolvedPreviewPanelWidth,
      latestWidth: resolvedPreviewPanelWidth,
      pointerId: event.pointerId,
      target,
      snappedClosed: false,
      sidebarWasOpen: verticalSidebarOpen,
      sidebarAutoCollapsed: false,
      sidebarCollapseBoundary: dockedLimit,
      overlayBoundary: dockedLimit + sidebarGain,
      overlayActive: previewPanelOverlay,
    };
    setPreviewResizing(true);
    target.setPointerCapture(event.pointerId);
    const move = (nextEvent: PointerEvent) => movePreviewResize(nextEvent);
    const end = (nextEvent: PointerEvent) => endPreviewResize(nextEvent);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
    previewResizeCleanupRef.current = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      previewResizeCleanupRef.current = null;
    };
  }

  function movePreviewResize(event: PointerEvent) {
    const start = previewResizeStartRef.current;
    if (!start || event.pointerId !== start.pointerId) return;
    const width = start.width + start.clientX - event.clientX;
    start.intentWidth = width;
    if (width < PREVIEW_PANEL_MIN_WIDTH - PREVIEW_PANEL_COLLAPSE_OVERSHOOT) {
      if (!start.snappedClosed) {
        start.snappedClosed = true;
        if (inspectorTab === "git") closeGitPanel();
        else closePreview();
      }
      return;
    }
    if (start.snappedClosed) {
      start.snappedClosed = false;
      start.latestWidth = clampPreviewPanelWidth(
        width,
        chatBodyRef.current?.getBoundingClientRect().width ?? chatBodyWidth,
        reservedContextWidth,
        start.overlayActive,
      );
      clearPreviewCloseTimer();
      setPreviewPanelClosing(false);
      setSessionInspectorOpen(activeId, true);
    }

    if (start.overlayActive && width <= start.overlayBoundary) {
      start.overlayActive = false;
      setPreviewPanelOverlay(false);
    }
    if (
      start.sidebarWasOpen &&
      start.sidebarAutoCollapsed &&
      width <= start.sidebarCollapseBoundary
    ) {
      start.sidebarAutoCollapsed = false;
      setSidebarOpen(true);
    } else if (
      start.sidebarWasOpen &&
      !start.sidebarAutoCollapsed &&
      width >= start.sidebarCollapseBoundary + PREVIEW_PANEL_STAGE_OVERSHOOT
    ) {
      start.sidebarAutoCollapsed = true;
      setSidebarOpen(false);
    }
    if (
      !start.overlayActive &&
      (!start.sidebarWasOpen || start.sidebarAutoCollapsed) &&
      width >= start.overlayBoundary + PREVIEW_PANEL_STAGE_OVERSHOOT
    ) {
      start.overlayActive = true;
      setPreviewPanelOverlay(true);
    }
    resizePreviewPanelDuringDrag(width);
  }

  function endPreviewResize(event: PointerEvent) {
    const start = previewResizeStartRef.current;
    if (!start || event.pointerId !== start.pointerId) return;
    const bodyWidth = chatBodyRef.current?.getBoundingClientRect().width ?? chatBodyWidth;
    const finalWidth = clampPreviewPanelWidth(
      start.latestWidth,
      bodyWidth,
      reservedContextWidth,
      start.overlayActive,
    );
    if (finalWidth !== start.width) setPreviewPanelWidth(finalWidth);
    if (
      start.overlayActive &&
      finalWidth <= maxPreviewPanelWidth(bodyWidth, reservedContextWidth)
    ) {
      setPreviewPanelOverlay(false);
    }
    previewResizeStartRef.current = null;
    previewResizeCleanupRef.current?.();
    setPreviewResizing(false);
    if (start.target.hasPointerCapture(event.pointerId)) {
      start.target.releasePointerCapture(event.pointerId);
    }
  }

  function resizePreviewWithKeyboard(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      if (previewPanelOverlay) {
        resizePreviewPanel(resolvedPreviewPanelWidth + PREVIEW_PANEL_KEYBOARD_STEP, true);
      } else if (resolvedPreviewPanelWidth < dockedPreviewPanelWidth) {
        resizePreviewPanel(resolvedPreviewPanelWidth + PREVIEW_PANEL_KEYBOARD_STEP, false);
      } else if (verticalSidebarOpen) {
        setSidebarOpen(false);
      } else {
        setPreviewPanelOverlay(true);
        resizePreviewPanel(resolvedPreviewPanelWidth + PREVIEW_PANEL_KEYBOARD_STEP, true);
      }
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      const nextWidth = resolvedPreviewPanelWidth - PREVIEW_PANEL_KEYBOARD_STEP;
      if (previewPanelOverlay && nextWidth <= dockedPreviewPanelWidth) {
        setPreviewPanelOverlay(false);
        resizePreviewPanel(dockedPreviewPanelWidth, false);
      } else {
        resizePreviewPanel(nextWidth, previewPanelOverlay);
      }
    } else if (event.key === "Home") {
      event.preventDefault();
      setPreviewPanelOverlay(false);
      resizePreviewPanel(PREVIEW_PANEL_MIN_WIDTH, false);
    } else if (event.key === "End") {
      event.preventDefault();
      resizePreviewPanel(
        maxPreviewPanelWidth(chatBodyWidth, reservedContextWidth, previewPanelOverlay),
        previewPanelOverlay,
      );
    } else if (event.key === "Enter") {
      event.preventDefault();
      setPreviewPanelOverlay(false);
      resizePreviewPanel(DEFAULT_PREVIEW_PANEL_WIDTH, false);
    }
  }

  function requireChatModel(): string | null {
    const selected = effectiveModel.trim();
    if (selected) {
      const runtime = accountRuntimeKind(selected);
      if (!runtime || accountRuntimeEnabled[runtime]) return selected;
      setChatNotice({
        tone: "error",
        message: `${runtime === "claude" ? "Claude CLI" : runtime === "opencode" ? "OpenCode" : runtime === "pi" ? "Pi" : "Codex"} is disabled in Providers.`,
      });
      setProvidersOpen(true);
      return null;
    }
    setChatNotice({
      tone: "error",
      message:
        "Choose a model before sending. Add Ollama, LM Studio, or another provider in Providers.",
    });
    setProvidersOpen(true);
    return null;
  }

  function sessionMessages(
    sessionId: string,
    fallback: ChatMessage[] = [],
  ): ChatMessage[] {
    return (
      useSessions
        .getState()
        .sessions.find((session) => session.id === sessionId)?.messages ??
      fallback
    );
  }

  async function maybeGenerateAiThreadTitle(
    sessionId: string,
    turnModel: string,
  ): Promise<void> {
    const prefs = useUiPreferences.getState();
    if (!prefs.autoTitleChats || !prefs.aiThreadNames) return;
    const store = useSessions.getState();
    const session = store.sessions.find((item) => item.id === sessionId);
    if (
      !session ||
      session.messages.filter((message) => message.role === "user").length !== 1
    )
      return;
    const titleSettings = store.getSettings(sessionId);
    const titleToolContext: AgentToolContext = {
      workspace: titleSettings.folder.trim() || null,
      privacy_mode: titleSettings.privacy,
    };
    if (!shouldReplaceThreadTitle(session.title, session.messages)) return;
    const namingModel = (prefs.aiThreadNameModel || turnModel).trim();
    const namingModelInfo = pickerModels.find(
      (item) => item.id === namingModel,
    );
    if (!isThreadNamingModel(namingModelInfo ?? namingModel)) {
      console.info(
        "AI thread naming skipped: choose a provider chat model for Codex, Claude, or media chats.",
      );
      return;
    }
    const firstUser = session.messages.find(
      (message) => message.role === "user",
    );
    const firstAssistant = session.messages.find(
      (message) => message.role === "assistant" && message.content.trim(),
    );
    if (!firstUser || !firstAssistant) return;
    let rawTitle: string;
    try {
      rawTitle = await completeChat(
        namingModel,
        [
          { role: "system", content: AI_THREAD_TITLE_SYSTEM_PROMPT },
          {
            role: "user",
            content: [
              `User: ${compactText(wireMessageContent(firstUser), 700)}`,
              `Assistant: ${compactText(firstAssistant.content, 700)}`,
            ].join("\n"),
          },
        ],
        {
          maxTokens: 16,
          temperature: 0,
          toolContext: titleToolContext,
        },
      );
    } catch (error) {
      console.warn(
        `AI thread naming failed: ${error instanceof Error ? error.message : String(error)}`,
      );
      return;
    }
    const title = sanitizeAiThreadTitle(rawTitle);
    if (!title) {
      console.info(
        "AI thread naming skipped: model returned an unusable title.",
      );
      return;
    }
    const latest = useSessions
      .getState()
      .sessions.find((item) => item.id === sessionId);
    if (
      latest &&
      latest.messages.filter((message) => message.role === "user").length ===
        1 &&
      shouldReplaceThreadTitle(latest.title, latest.messages)
    ) {
      useSessions.getState().rename(sessionId, title);
    }
  }

  async function createCompactionCheckpoint(
    _sessionId: string,
    sourceMessages: ChatMessage[],
    model: string,
    options: {
      auto: boolean;
      folder: string;
      toolContext: AgentToolContext;
      reasoningEffort: ReasoningEffort;
      signal?: AbortSignal;
    },
  ): Promise<ChatMessage> {
    const sourceContext = messagesForModelContext([], sourceMessages);
    if (!sourceContext.some((message) => wireMessageContent(message).trim())) {
      throw new Error("There is no thread context to compact.");
    }
    const baseline = summarizeThreadMetricsBreakdown(sourceMessages).lifetime;
    const codexModel = codexRuntimeModel(model);
    const claudeModel = claudeRuntimeModel(model);
    const opencodeModel = opencodeRuntimeModel(model);
    const piModel = piRuntimeModel(model);
    const summaryStartedAt = Date.now();
    const selectedProvider = providers.find(
      (item) => providerOwnsModel(item, model),
    );
    const provider = codexModel
      ? "Codex"
      : claudeModel
        ? "Local Claude CLI"
        : opencodeModel
          ? "Local OpenCode CLI"
          : piModel
            ? "Local Pi CLI"
        : selectedProvider?.name;
    const summaryReasoningEffort =
      compactionSummaryReasoningEffort(selectedProvider);
    let usage: TokenUsage | undefined;
    let costUsd: number | undefined;
    let lastError = "Compaction failed.";

    for (let attempt = 0; attempt < 2; attempt += 1) {
      const retry = attempt > 0;
      const outputCapTokens = compactionSummaryOutputCap(
        model,
        pickerModels,
        retry,
      );
      const promptMessages = compactionSummaryMessages(
        sourceMessages,
        model,
        pickerModels,
        { retry, outputCapTokens },
      );
      let summary: CompactionSummaryResult;
      if (codexModel) {
        const ready = await ensureCodexAccount();
        if (!ready.ok) throw new Error(ready.message);
        summary = await summarizeWithCodex(
          codexModel,
          promptMessages,
          options.folder,
          options.reasoningEffort,
          options.toolContext,
          options.signal,
        );
      } else if (claudeModel) {
        const ready = await ensureClaudeAccount();
        if (!ready.ok) throw new Error(ready.message);
        summary = await summarizeWithClaude(
          claudeModel,
          promptMessages,
          options.folder,
          options.reasoningEffort,
          options.toolContext,
          options.signal,
        );
      } else if (opencodeModel) {
        const ready = await ensureOpenCodeAccount();
        if (!ready.ok) throw new Error(ready.message);
        summary = await summarizeWithOpenCode(
          opencodeModel,
          promptMessages,
          options.folder,
          options.toolContext,
          options.signal,
        );
      } else if (piModel) {
        const ready = await ensurePiAccount();
        if (!ready.ok) throw new Error(ready.message);
        summary = await summarizeWithPi(
          piModel,
          promptMessages,
          options.folder,
          options.reasoningEffort,
          options.toolContext,
          options.signal,
        );
      } else {
        const completion = await completeChatWithMetrics(
          model,
          promptMessages,
          {
            maxTokens: outputCapTokens,
            temperature: 0,
            reasoningEffort: summaryReasoningEffort,
            signal: options.signal,
            toolContext: options.toolContext,
          },
        );
        summary = {
          content: completion.content,
          usage: completion.usage,
          finishReason: completion.finishReason,
          costUsd: estimateResponseCostUsd(model, completion.usage, providers),
        };
      }

      usage = mergeTokenUsage(usage, summary.usage);
      if (typeof summary.costUsd === "number")
        costUsd = (costUsd ?? 0) + summary.costUsd;

      const clean = summary.content.trim();
      const validationError = validateCompactionCheckpointSummary(clean, {
        finishReason: summary.finishReason,
        model,
        models: pickerModels,
        sourceMessages,
      });
      if (!validationError) {
        return checkpointMessage(clean, {
          auto: options.auto,
          sourceTokens: estimateMessagesTokens(sourceContext),
          baseline,
          summaryMetrics: {
            model,
            provider,
            durationMs: Date.now() - summaryStartedAt,
            usage,
            costUsd,
          },
        });
      }
      lastError = validationError;
    }

    throw new Error(lastError);
  }

  async function summarizeWithCodex(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    reasoningEffort: ReasoningEffort,
    toolContext: AgentToolContext,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    return await collectHarnessUtilityRun(
      "codex",
      {
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        reasoning_effort: reasoningEffort,
        images: runtimeInput.images,
        persist_session: false,
        tool_approval_policy: "guarded",
        tool_approval_grant: false,
        plan_mode: true,
        milim_context: utilityAccountRuntimeMilimContext({
          toolContext,
          toolApproval: "guarded",
          planMode: true,
        }),
      },
      signal,
    );
  }

  async function summarizeWithClaude(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    reasoningEffort: ReasoningEffort,
    toolContext: AgentToolContext,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    return await collectHarnessUtilityRun(
      "claude",
      {
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        reasoning_effort: reasoningEffort,
        images: runtimeInput.images,
        persist_session: false,
        tool_approval_policy: "guarded",
        tool_approval_grant: false,
        plan_mode: true,
        milim_context: utilityAccountRuntimeMilimContext({
          toolContext,
          toolApproval: "guarded",
          planMode: true,
        }),
      },
      signal,
    );
  }

  async function summarizeWithOpenCode(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    toolContext: AgentToolContext,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    return await collectHarnessUtilityRun(
      "opencode",
      {
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        images: runtimeInput.images,
        persist_session: false,
        tool_approval_policy: "guarded",
        tool_approval_grant: false,
        plan_mode: true,
        milim_context: utilityAccountRuntimeMilimContext({
          toolContext,
          toolApproval: "guarded",
          planMode: true,
        }),
      },
      signal,
    );
  }

  async function summarizeWithPi(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    reasoningEffort: ReasoningEffort,
    toolContext: AgentToolContext,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    return await collectHarnessUtilityRun(
      "pi",
      {
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        images: runtimeInput.images,
        reasoning_effort: reasoningEffort,
        persist_session: false,
        tool_approval_policy: "guarded",
        tool_approval_grant: false,
        plan_mode: true,
        milim_context: utilityAccountRuntimeMilimContext({
          toolContext,
          toolApproval: "guarded",
          planMode: true,
        }),
      },
      signal,
    );
  }

  async function compactThreadManually() {
    if (busy || compactionInFlightRef.current) {
      setChatNotice({
        tone: "info",
        message: "Wait for the current run to finish before compacting.",
      });
      return;
    }
    if (activeMediaTarget) {
      setChatNotice({
        tone: "error",
        message: "Switch to a chat model before compacting context.",
      });
      return;
    }
    const selectedModel = requireChatModel();
    if (!selectedModel) return;
    const targetSessionId = activeId;
    const sessionsStore = useSessions.getState();
    const targetSettings = sessionsStore.getSettings(targetSessionId);
    const toolContext: AgentToolContext = {
      workspace: targetSettings.folder.trim() || null,
      privacy_mode: targetSettings.privacy,
    };
    const currentMessages =
      sessionsStore.sessions.find((session) => session.id === targetSessionId)
        ?.messages ?? messages;
    if (!currentMessages.length) {
      setChatNotice({
        tone: "info",
        message: "There is no thread context to compact.",
      });
      return;
    }
    compactionInFlightRef.current = true;
    setChatNotice({ tone: "info", message: "Compacting thread context..." });
    try {
      const reasoningEffort = reasoningEffortForThread(
        targetSettings.reasoningEffortOverrides,
        useSettings.getState().reasoningEffortByModel,
        selectedModel,
        pickerModels,
      );
      const split = splitCompactionTail(
        currentMessages,
        selectedModel,
        pickerModels,
      );
      const checkpoint = await createCompactionCheckpoint(
        targetSessionId,
        split.head,
        selectedModel,
        {
          auto: false,
          folder: targetSettings.folder,
          toolContext,
          reasoningEffort,
        },
      );
      const store = useSessions.getState();
      store.setMessages(
        targetSessionId,
        [...split.head, checkpoint, ...split.tail],
        {
          autoTitle: false,
        },
      );
      store.clearAccountRuntime(targetSessionId);
      setChatNotice({
        tone: "info",
        message:
          "Context checkpoint created. Future replies start from the summary.",
      });
      focusComposer();
    } catch (e) {
      setChatNotice({
        tone: "error",
        message: `Compaction failed: ${e instanceof Error ? e.message : String(e)}`,
      });
    } finally {
      compactionInFlightRef.current = false;
    }
  }

  function requestClaudeSessionRecoveryCard(
    sessionId: string,
    convo: ChatMessage[],
    selectedModel: string,
    detail: string,
  ) {
    const next = [
      ...convo.filter(
        (message) =>
          !(
            message.approval?.kind === "claude_session_recovery" &&
            message.approval.status === "pending"
          ),
      ),
      toolApprovalMessage("claude_session_recovery", selectedModel, detail),
    ];
    setMessages(sessionId, next, { autoTitle: autoTitleChats });
    setChatNotice({
      tone: "warning",
      message:
        "Claude session recovery needs approval before Milim stops a local Claude CLI process.",
    });
  }

  function updateApprovalAt(
    messageIndex: number,
    status: "approved" | "denied",
    sessionId = activeId,
  ): ChatMessage[] {
    const latest = sessionMessages(sessionId);
    if (!latest[messageIndex]?.approval) return latest;
    const next = latest.map((message, index) =>
      index === messageIndex
        ? resolveApprovalMessage(message, status)
        : message,
    );
    setMessages(sessionId, next, { autoTitle: false });
    return next;
  }

  function startApprovedGoalRun(
    sessionId: string,
    selectedModel: string,
    compatibilityGrant = false,
  ) {
    if (goalLoopRef.current && !goalLoopRef.current.stopped) return;
    const savedGoal = sessionGoal(sessionId);
    if (!goalConfigured(savedGoal)) {
      openGoalPanel();
      setChatNotice({
        tone: "info",
        message: "Add a goal objective before running.",
      });
      return;
    }
    const now = Date.now();
    const runningGoal = normalizeGoalSettings({
      ...savedGoal,
      status: "running",
      lastReason:
        savedGoal.status === "paused" ? "Goal resumed." : "Goal run started.",
      startedAt: savedGoal.startedAt ?? now,
      updatedAt: now,
    });
    useSessions.getState().updateSettings(sessionId, { goal: runningGoal });
    goalLoopRef.current = { sessionId, stopped: false };
    setGoalPrefill(null);
    setGoalPanelOpen(false);
    setChatNotice({ tone: "info", message: "Goal running." });
    void runGoalLoop(sessionId, selectedModel, runningGoal, compatibilityGrant || undefined);
  }

  function approveToolApproval(messageIndex: number, message: ChatMessage) {
    const approval = message.approval;
    if (!approval || approval.status !== "pending" || busy || activeMediaTarget)
      return;
    const selectedModel = approval.model || requireChatModel();
    if (!selectedModel) return;
    const approvedMessages = updateApprovalAt(messageIndex, "approved");
    if (approval.kind === "claude_session_recovery") {
      setChatNotice({
        tone: "warning",
        message:
          "Claude session recovery approved. Milim will try to stop the matching local Claude CLI process and retry.",
      });
      void runTurnAndDrain(approvedMessages, selectedModel, {
        claudeSessionRecoveryGrant: true,
      });
      return;
    }
    if (approval.scope === "goal") {
      startApprovedGoalRun(activeId, selectedModel, true);
      return;
    }
    setChatNotice({
      tone: "info",
      message: "Tool access approved for this reply.",
    });
    void runTurnAndDrain(approvedMessages, selectedModel, {
      toolApprovalGrant: true,
    });
  }

  function denyToolApproval(messageIndex: number, message: ChatMessage) {
    const approval = message.approval;
    if (!approval || approval.status !== "pending" || busy) return;
    updateApprovalAt(messageIndex, "denied");
    if (approval.kind === "claude_session_recovery") {
      setChatNotice({
        tone: "info",
        message:
          "Claude session recovery canceled. Resume or stop the Claude CLI process manually.",
      });
      return;
    }
    if (approval.scope === "goal") {
      updateGoalState(activeId, {
        status: "paused",
        lastReason: "Goal run canceled before tool approval.",
      });
    }
    setChatNotice({ tone: "info", message: "Tool access denied." });
  }

  async function requestGoalDecision(
    sessionId: string,
    turnModel: string,
    currentGoal: GoalSettings,
    latestMessages: ChatMessage[],
  ): Promise<GoalDecision> {
    const controller = new AbortController();
    const loop = goalLoopRef.current;
    if (loop?.sessionId === sessionId) loop.decisionController = controller;
    try {
      const decisionSettings = useSessions.getState().getSettings(sessionId);
      const decisionWorkspace = decisionSettings.folder.trim();
      const decisionToolContext: AgentToolContext = {
        workspace: decisionWorkspace || null,
        privacy_mode: decisionSettings.privacy,
      };
      const decisionMilimContext = utilityAccountRuntimeMilimContext({
        toolContext: decisionToolContext,
        toolApproval: "review",
        planMode: false,
      });
      const decisionMessages = goalDecisionMessages(
        currentGoal,
        latestMessages,
      );
      const decisionReasoningEffort = reasoningEffortForThread(
        decisionSettings.reasoningEffortOverrides,
        useSettings.getState().reasoningEffortByModel,
        turnModel,
        pickerModels,
      );
      const codexModel = codexRuntimeModel(turnModel);
      const claudeModel = claudeRuntimeModel(turnModel);
      const opencodeModel = opencodeRuntimeModel(turnModel);
      const piModel = piRuntimeModel(turnModel);
      const runtimeInput = accountRuntimeInputFromMessages(decisionMessages);
      let content = "";
      const selectedHarness = codexModel
        ? { id: "codex" as const, model: codexModel }
        : claudeModel
          ? { id: "claude" as const, model: claudeModel }
          : opencodeModel
            ? { id: "opencode" as const, model: opencodeModel }
            : piModel
              ? { id: "pi" as const, model: piModel }
              : null;
      if (selectedHarness) {
        const guarded = selectedHarness.id === "pi";
        const result = await collectHarnessUtilityRun(
          selectedHarness.id,
          {
            model: selectedHarness.model,
            prompt: runtimeInput.prompt,
            images: runtimeInput.images,
            cwd: decisionWorkspace || undefined,
            ...(selectedHarness.id === "opencode"
              ? {}
              : { reasoning_effort: decisionReasoningEffort }),
            persist_session: false,
            tool_approval_policy: guarded ? "guarded" : "review",
            tool_approval_grant: false,
            plan_mode: guarded,
            milim_context: guarded
              ? utilityAccountRuntimeMilimContext({
                  toolContext: decisionToolContext,
                  toolApproval: "guarded",
                  planMode: true,
                })
              : decisionMilimContext,
          },
          controller.signal,
        );
        content = result.content;
      } else {
        content = await completeChat(turnModel, decisionMessages, {
          signal: controller.signal,
          maxTokens: 500,
          temperature: 0,
          toolContext: decisionToolContext,
        });
      }
      return parseGoalDecision(content);
    } finally {
      if (goalLoopRef.current?.decisionController === controller) {
        goalLoopRef.current.decisionController = undefined;
      }
    }
  }

  function goalConversation(
    sessionId: string,
    currentGoal: GoalSettings,
    nextPrompt?: string,
  ): ChatMessage[] {
    const latest = sessionMessages(sessionId);
    const last = latest[latest.length - 1];
    if (last?.role === "user") return latest;
    return [
      ...latest,
      {
        role: "user",
        content: goalContinuationPrompt(
          currentGoal,
          nextPrompt ?? currentGoal.nextPrompt,
        ),
      },
    ];
  }

  async function runGoalLoop(
    sessionId: string,
    selectedModel: string,
    initialGoal: GoalSettings,
    toolApprovalGrant?: boolean,
  ) {
    const loop = goalLoopRef.current;
    let currentGoal = initialGoal;
    try {
      for (;;) {
        if (!loop || loop.stopped) return;
        currentGoal = sessionGoal(sessionId);
        if (currentGoal.status !== "running") return;
        if (
          currentGoal.developerMaxTurns &&
          currentGoal.turns >= currentGoal.developerMaxTurns
        ) {
          updateGoalState(
            sessionId,
            {
              status: "paused",
              lastReason: "Developer max-turn cap reached.",
            },
            currentGoal,
          );
          return;
        }

        const turnResult = await runTurn(
          goalConversation(sessionId, currentGoal),
          selectedModel,
          {
            goal: currentGoal,
            toolApprovalGrant,
          },
          sessionId,
        );
        if (loop.stopped) return;
        if (turnResult.status === "aborted") {
          updateGoalState(sessionId, {
            status: "paused",
            lastReason: "Goal paused.",
          });
          return;
        }
        if (turnResult.status !== "done") {
          updateGoalState(sessionId, {
            status: turnResult.status === "skipped" ? "paused" : "error",
            lastReason:
              turnResult.error || "Goal run stopped before the turn completed.",
          });
          return;
        }
        if (loop.stopped) {
          updateGoalState(sessionId, {
            status: "paused",
            lastReason: "Goal paused.",
          });
          return;
        }
        const proposedRun = useSessions.getState().workerRuns.find(
          (record) =>
            record.run.parent_thread_id === sessionId &&
            record.run.status === "proposed",
        );
        if (proposedRun) {
          updateGoalState(sessionId, {
            status: "waiting_for_worker_approval",
            lastReason: "Goal waiting for worker approval.",
          });
          return;
        }
        if (hasQueuedMessages(sessionId)) {
          updateGoalState(sessionId, {
            status: "paused",
            lastReason: "Goal paused for queued user messages.",
          });
          await drainQueuedMessages(sessionId, selectedModel);
          return;
        }

        const decision = await requestGoalDecision(
          sessionId,
          selectedModel,
          currentGoal,
          turnResult.messages,
        );
        const afterDecisionGoal = sessionGoal(sessionId);
        currentGoal = updateGoalState(
          sessionId,
          applyGoalDecision(afterDecisionGoal, decision),
          afterDecisionGoal,
        );
        if (currentGoal.status !== "running") return;
      }
    } catch (e) {
      if (loop?.stopped) return;
      const aborted = e instanceof DOMException && e.name === "AbortError";
      updateGoalState(sessionId, {
        status: aborted ? "paused" : "error",
        lastReason: aborted
          ? "Goal paused."
          : `Goal controller failed: ${e instanceof Error ? e.message : String(e)}`,
      });
    } finally {
      if (goalLoopRef.current === loop) goalLoopRef.current = null;
    }
  }

  function startGoalRun(
    draft?: GoalPanelDraft,
    initialAttachments: ChatAttachment[] = [],
  ): boolean {
    if (activeMediaTarget) {
      setChatNotice({
        tone: "error",
        message: "Switch back to chat before running a goal.",
      });
      return false;
    }
    const selectedModel = requireChatModel();
    if (!selectedModel) return false;
    const sessionId = activeId;
    if (goalLoopRef.current && !goalLoopRef.current.stopped) return false;
    const savedGoal = draft
      ? saveGoalDraft(draft, sessionId)
      : sessionGoal(sessionId);
    if (!goalConfigured(savedGoal)) {
      openGoalPanel();
      setChatNotice({
        tone: "info",
        message: "Add a goal objective before running.",
      });
      return false;
    }
    if (initialAttachments.length > 0) {
      setMessages(
        sessionId,
        appendUserTurn(
          sessionMessages(sessionId),
          savedGoal.objective,
          initialAttachments,
        ),
      );
    }
    startApprovedGoalRun(sessionId, selectedModel);
    return true;
  }

  async function handleSaveArtifact(
    messageIndex: number,
    artifact: ChatArtifact,
    options?: {
      overwrite?: boolean;
      path?: string;
      source?: SavedArtifactFile["source"];
    },
    revision?: ArtifactRevision,
  ): Promise<SavedArtifactFile> {
    const target =
      options?.path?.trim() || (artifact.filename ?? artifact.title);
    const targetMessageIndex = revision?.messageIndex ?? messageIndex;
    const targetMessage = useSessions
      .getState()
      .sessions.find((s) => s.id === activeId)?.messages[targetMessageIndex];
    const targetMessageId = targetMessage?.id ?? targetMessageIndex;
    if (!folder.trim()) {
      const path = normalizeVirtualFilePath(target);
      if (!path) throw new Error("virtual project file path must be relative");
      const existing = currentVirtualFile(path);
      if (existing && !options?.overwrite)
        throw new Error("file already exists in virtual project");
      const saved: SavedArtifactFile = {
        path,
        bytes: textBytes(artifact.content),
        overwritten: Boolean(existing),
        savedAt: Date.now(),
        sourceSessionId: APP_SESSION_ID,
        sourceMessageIndex: targetMessageIndex,
        sourceRevisionNumber: revision?.revisionNumber,
        source: options?.source ?? "artifact",
      };
      upsertVirtualFiles(activeId, [{ path, content: artifact.content }], {
        sourceMessageIndex: targetMessageIndex,
        sourceRevisionNumber: revision?.revisionNumber,
      });
      markArtifactSaved(activeId, targetMessageId, artifact.id, saved);
      return saved;
    }
    const saved = await saveArtifactFile(
      folder,
      target,
      artifact.content,
      options?.overwrite ?? false,
    );
    const tracedSaved: SavedArtifactFile = {
      ...saved,
      savedAt: Date.now(),
      sourceSessionId: APP_SESSION_ID,
      sourceMessageIndex: targetMessageIndex,
      sourceRevisionNumber: revision?.revisionNumber,
      source: options?.source ?? "artifact",
    };
    markArtifactSaved(activeId, targetMessageId, artifact.id, tracedSaved);
    if (options?.source === "auto_artifact") {
      const message = useSessions
        .getState()
        .sessions.find((s) => s.id === activeId)?.messages[targetMessageIndex];
      if (message?.streamParts?.length && message.id) {
        useSessions.getState().appendStreamEvent(activeId, message.id, {
          kind: "event",
          eventType: "tool",
          label: "Created file",
          detail: target,
          icon: "file",
          name: "write_file",
          status: "done",
        });
      }
    }
    return tracedSaved;
  }

  async function handlePreviewArtifact(
    artifact: ChatArtifact,
    path?: string,
    _revision?: ArtifactRevision,
  ): Promise<ArtifactWritePreview> {
    const target = path?.trim() || (artifact.filename ?? artifact.title);
    if (!folder.trim()) {
      const normalized = normalizeVirtualFilePath(target);
      if (!normalized)
        throw new Error("virtual project file path must be relative");
      return virtualArtifactPreview(
        normalized,
        artifact.content,
        currentVirtualFile(normalized),
      );
    }
    return await previewArtifactFile(folder, target, artifact.content);
  }

  async function handleOpenArtifact(
    saved: SavedArtifactFile,
    target: ArtifactOpenTarget,
  ) {
    if (!folder.trim()) {
      const file = currentVirtualFile(saved.path);
      if (!file) throw new Error("virtual project file is unavailable");
      openPreviewArtifact(virtualChatArtifact(file), [
        virtualChatArtifact(file),
      ]);
      if (target === "folder")
        setChatNotice({
          tone: "info",
          message: "Opened the virtual project file.",
        });
      return;
    }
    await openArtifactLocation(saved.path, target);
  }

  async function handleCheckArtifact(
    saved: SavedArtifactFile,
  ): Promise<ArtifactFileStatus> {
    if (!folder.trim()) {
      const file = currentVirtualFile(saved.path);
      return {
        path: saved.path,
        exists: Boolean(file),
        is_file: Boolean(file),
        is_dir: false,
        bytes: file?.bytes ?? null,
      };
    }
    return await artifactFileStatus(saved.path);
  }

  async function openFolderPicker(): Promise<string | null> {
    if (!inTauri) {
      setChatNotice({
        tone: "info",
        message:
          "Folder picker is available in the desktop app. Use /folder <path> to set a folder here.",
      });
      return null;
    }
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const sel = await open({ directory: true, multiple: false });
      return typeof sel === "string" ? sel : null;
    } catch {
      /* dialog unavailable */
    }
    return null;
  }

  async function pickFolder() {
    const selected = await openFolderPicker();
    if (selected && await requestWorkspaceEditorLeave("navigate")) updateThreadSettings(activeId, { folder: selected });
  }

  async function startChatInFolder(nextFolder: string) {
    if (messages.length === 0) {
      if (!(await requestWorkspaceEditorLeave("navigate"))) return;
      updateThreadSettings(activeId, { folder: nextFolder });
    } else {
      await createInteractiveChat({ folder: nextFolder });
    }
    setChatNotice(null);
    focusComposer();
  }

  async function pickProjectFolder() {
    const selected = await openFolderPicker();
    if (selected) await startChatInFolder(selected);
  }

  async function handleAttachFiles(files?: File[]) {
    try {
      let next: ChatAttachment[] = [];
      if (files?.length) {
        next = await Promise.all(files.map(browserFileAttachment));
      } else if (inTauri) {
        next = (await pickAttachmentFiles()).map((attachment) => ({
          id: attachmentId(),
          ...attachment,
        }));
      }
      if (next.length)
        setPendingAttachments((current) => [...current, ...next].slice(0, 12));
    } catch (e) {
      setChatNotice({
        tone: "error",
        message: `Could not attach file: ${e instanceof Error ? e.message : String(e)}`,
      });
    }
  }

  async function handleAttachWorkspaceFile(
    file: WorkspaceFileSuggestion,
  ): Promise<boolean> {
    try {
      const next = {
        id: attachmentId(),
        ...(await readWorkspaceAttachmentFile(folder, file.path)),
      };
      setPendingAttachments((current) => [...current, next].slice(0, 12));
      return true;
    } catch (e) {
      setChatNotice({
        tone: "error",
        message: `Could not attach file: ${e instanceof Error ? e.message : String(e)}`,
      });
      return false;
    }
  }

  function exportSessionById(
    sessionId = activeId,
    format: ThreadExportFormat = "json",
  ) {
    const session = useSessions
      .getState()
      .sessions.find((item) => item.id === sessionId);
    if (!session) return;
    const content =
      format === "markdown"
        ? sessionMarkdownExport(session)
        : JSON.stringify(sessionExportPayload(session), null, 2);
    const blob = new Blob([content], {
      type: format === "markdown" ? "text/markdown" : "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = chatExportFilename(session.title, format);
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
    setChatNotice({
      tone: "info",
      message:
        format === "markdown"
          ? "Thread exported as Markdown."
          : "Thread exported.",
    });
  }

  function importSessionFromFile() {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "application/json,text/markdown,.json,.md,.milim-chat.json";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return;
      void file
        .text()
        .then((text) => {
          const isMarkdown =
            file.name.toLowerCase().endsWith(".md") ||
            file.type === "text/markdown";
          const candidate = isMarkdown
            ? markdownSessionCandidate(text)
            : exportedSessionCandidate(JSON.parse(text));
          if (!candidate)
            throw new Error("The selected file is not a Milim thread export.");
          const importedId = useSessions.getState().importSession(candidate);
          if (!importedId)
            throw new Error(
              "The selected file did not contain a usable thread.",
            );
          setChatNotice({ tone: "info", message: "Thread imported." });
        })
        .catch((error) =>
          setChatNotice({
            tone: "error",
            message: `Import failed: ${error instanceof Error ? error.message : String(error)}`,
          }),
        );
    };
    input.click();
  }

  function forkThreadAt(messageIndex: number) {
    if (busy) return;
    const forkedId = useSessions.getState().forkSession(activeId, messageIndex);
    if (!forkedId) return;
    setEditing(null);
    setChatNotice({ tone: "info", message: "Thread branched." });
    focusComposer();
  }

  function deleteMessageAt(messageIndex: number) {
    if (busy) return;
    const latest = sessionMessages(activeId);
    if (messageIndex < 0 || messageIndex >= latest.length) return;
    setEditing(null);
    setMessages(
      activeId,
      latest.filter((_, index) => index !== messageIndex),
      { autoTitle: false },
    );
    useSessions.getState().clearAccountRuntime(activeId);
    setChatNotice({ tone: "info", message: "Message deleted." });
  }

  async function restoreWorkspaceCheckpoint(checkpoint: WorkspaceCheckpoint) {
    if (busy) return;
    if (
      !window.confirm(
        `Restore workspace files to before this turn?\n\n${checkpoint.folder}`,
      )
    )
      return;
    try {
      await setWorkspace(checkpoint.folder);
      const result = await runWorkspaceGitAction("restore_checkpoint", {
        checkpoint: checkpoint.ref,
      });
      if (!result.ok) throw new Error(result.message);
      useSessions.getState().clearAccountRuntime(activeId);
      useSessions.getState().setPendingHotSwap(activeId, undefined);
      setChatNotice({
        tone: "info",
        message: "Workspace restored to before this turn.",
      });
      openGitPanel();
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async function undoTurnChanges(messageIndex: number) {
    if (busy) return;
    const latest = sessionMessages(activeId);
    const assistant = latest[messageIndex];
    const checkpoint = assistant?.workspaceCheckpoint;
    if (!checkpoint || assistant.role !== "assistant") return;
    if (
      !window.confirm(
        `Undo this turn's workspace changes and remove its response?\n\n${checkpoint.folder}`,
      )
    )
      return;
    try {
      await setWorkspace(checkpoint.folder);
      const result = await runWorkspaceGitAction("restore_checkpoint", {
        checkpoint: checkpoint.ref,
      });
      if (!result.ok) throw new Error(result.message);
      setMessages(activeId, latest.slice(0, messageIndex), { autoTitle: false });
      useSessions.getState().clearAccountRuntime(activeId);
      useSessions.getState().setPendingHotSwap(activeId, undefined);
      setChatNotice({ tone: "info", message: "Turn changes undone. The original request is ready to retry." });
      focusComposer();
      openGitPanel();
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    }
  }

  function editMessageInPlace(messageIndex: number, newText: string) {
    const content = newText.trim();
    setEditing(null);
    if (busy || !content) return;
    const latest = sessionMessages(activeId);
    if (!latest[messageIndex]) return;
    const next = latest.map((message, index) =>
      index === messageIndex ? { ...message, content } : message,
    );
    setMessages(activeId, next, { autoTitle: false });
    useSessions.getState().clearAccountRuntime(activeId);
    setChatNotice({ tone: "info", message: "Message updated." });
  }

  function runSlashCommand(id: string, argument: string): boolean {
    const arg = argument.trim();
    switch (id) {
      case "plan": {
        const normalized = arg.toLowerCase();
        if (!arg || normalized === "on") {
          setPlanModeActive(true);
          return true;
        }
        if (normalized === "off") {
          setPlanModeActive(false);
          return true;
        }
        if (activeMediaTarget) {
          setChatNotice({
            tone: "error",
            message: "Switch to a chat model before starting Plan Mode.",
          });
          return true;
        }
        const selectedModel = requireChatModel();
        if (!selectedModel) return true;
        updateThreadSettings(activeId, { planMode: true });
        const attachments = pendingAttachments;
        setPendingAttachments([]);
        if (busy) {
          enqueueQueuedMessage(activeId, {
            content: arg,
            attachments: attachments.length ? attachments : undefined,
          });
          setChatNotice({
            tone: "info",
            message:
              "Plan request queued. Tools will be limited to read-only inspection.",
          });
          return true;
        }
        setChatNotice(null);
        void runTurnAndDrain(
          appendUserTurn(
            messages,
            arg,
            attachments.length ? attachments : undefined,
          ),
          selectedModel,
        );
        return true;
      }
      case "goal":
        if (arg) {
          setGoalComposerModeActive(false);
          startGoalRun({
            objective: arg,
            successCriteria: "",
            constraints: "",
            developerMaxTurns: null,
          });
        } else {
          setGoalComposerModeActive(true);
        }
        return true;
      case "model": {
        if (arg) {
          updateThreadSettings(activeId, { model: arg });
          setChatNotice(null);
        } else {
          setChatNotice({
            tone: "info",
            message:
              "Choose a model from the picker, or run /model <model-id>.",
          });
        }
        return true;
      }
      case "folder": {
        if (arg) updateThreadSettings(activeId, { folder: arg });
        else void pickFolder();
        return true;
      }
      case "sandbox":
        updateThreadSettings(activeId, { sandbox: true });
        return true;
      case "nosandbox":
        updateThreadSettings(activeId, { sandbox: false });
        return true;
      case "computer":
        updateThreadSettings(activeId, { computerUse: true });
        return true;
      case "nocomputer":
        updateThreadSettings(activeId, { computerUse: false });
        return true;
      case "memory":
        updateThreadSettings(activeId, { memory: true });
        return true;
      case "nomemory":
        updateThreadSettings(activeId, { memory: false });
        return true;
      case "privacy": {
        if (arg === "off" || arg === "redact" || arg === "block") {
          updateThreadSettings(activeId, { privacy: arg });
        } else {
          setChatNotice({ tone: "info", message: "Use /privacy off, /privacy redact, or /privacy block." });
        }
        return true;
      }
      case "approval": {
        if (arg === "review" || arg === "guarded" || arg === "open") {
          updateThreadSettings(activeId, { toolApproval: arg });
        } else {
          setChatNotice({ tone: "info", message: "Use /approval review, /approval guarded, or /approval open." });
        }
        return true;
      }
      case "agent": {
        const target = arg.toLowerCase();
        if (
          !target ||
          target === "none" ||
          target === "off" ||
          target === "default"
        ) {
          updateThreadSettings(activeId, { activeAgentId: null });
          return true;
        }
        const agent = agents.find(
          (a) =>
            a.id.toLowerCase() === target || a.name.toLowerCase() === target,
        );
        if (agent) {
          updateThreadSettings(activeId, { activeAgentId: agent.id });
          setChatNotice(null);
        } else {
          setChatNotice({ tone: "error", message: `Agent not found: ${arg}` });
        }
        return true;
      }
      case "compact":
        void compactThreadManually();
        return true;
      case "export":
        exportSessionById(
          activeId,
          arg === "md" || arg === "markdown"
            ? "markdown"
            : arg === "json"
              ? "json"
              : threadExportFormat,
        );
        return true;
      case "import":
        importSessionFromFile();
        return true;
      case "clear":
        setPendingAttachments([]);
        void createInteractiveChat();
        focusComposer();
        return true;
      default:
        return false;
    }
  }

  async function ensureCodexAccount(): Promise<AccountRuntimeReady> {
    try {
      const account = await getCodexAccount(false);
      if (account.account || !account.requiresOpenaiAuth) return { ok: true };
    } catch (e) {
      const message = `Codex is unavailable: ${e instanceof Error ? e.message : String(e)}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    }

    let completed = false;
    let failed: string | null = null;
    let warning = false;
    let opened = false;
    setChatNotice({ tone: "info", message: "Starting Codex login..." });
    try {
      await streamCodexDeviceLogin((ev: CodexLoginEvent) => {
        if (ev.type === "browser") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.auth_url).catch((error) => {
              setChatNotice({
                tone: "error",
                message: `Could not open Codex login URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setChatNotice({
            tone: "info",
            message:
              "Complete Codex login in the browser. This turn will continue when login finishes.",
          });
        } else if (ev.type === "device_code") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.verification_url).catch((error) => {
              setChatNotice({
                tone: "error",
                message: `Could not open Codex device-code URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setChatNotice({
            tone: "info",
            message: `Complete Codex login with code ${ev.user_code}. This turn will continue when login finishes.`,
          });
        } else if (ev.type === "done") {
          completed = ev.success;
          failed = ev.error ?? null;
        } else if (ev.type === "warning") {
          failed = ev.message;
          warning = true;
          setChatNotice({ tone: "warning", message: ev.message });
        } else if (ev.type === "error") {
          failed = ev.message;
        }
      });
    } catch (e) {
      const message = `Codex login failed: ${e instanceof Error ? e.message : String(e)}`;
      warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    }
    if (completed) {
      setChatNotice({ tone: "info", message: "Codex login completed." });
      return { ok: true };
    }
    const message = failed || "Codex login did not complete.";
    warning ||= isCliPathWarningMessage(message);
    setChatNotice({ tone: warning ? "warning" : "error", message });
    return { ok: false, message, warning };
  }

  async function ensureClaudeAccount(): Promise<AccountRuntimeReady> {
    try {
      const status = await getClaudeStatus();
      if (status.available && status.authenticated) return { ok: true };
      const message = status.available
        ? "Claude CLI is not signed in. Authenticate through Claude's own tooling with `claude auth login`, then refresh models."
        : `Claude CLI is unavailable: ${status.error || "install Anthropic's official Claude CLI separately and make sure `claude` is on PATH."}`;
      const warning =
        Boolean(status.warning) || isCliPathWarningMessage(message);
      setChatNotice({
        tone: warning ? "warning" : "error",
        message,
      });
      return { ok: false, message, warning };
    } catch (e) {
      const message = `Claude CLI is unavailable: ${e instanceof Error ? e.message : String(e)}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    }
  }

  async function ensureOpenCodeAccount(): Promise<AccountRuntimeReady> {
    try {
      const status = await getOpenCodeStatus();
      if (status.available && status.authenticated) return { ok: true };
      const message = status.available
        ? "OpenCode has no configured models. Configure a provider with the OpenCode CLI, then refresh models."
        : `OpenCode CLI is unavailable: ${status.error || "install OpenCode separately and make sure `opencode` is on PATH."}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    } catch (e) {
      const message = `OpenCode CLI is unavailable: ${e instanceof Error ? e.message : String(e)}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    }
  }

  async function ensurePiAccount(): Promise<AccountRuntimeReady> {
    try {
      const status = await getPiStatus();
      if (status.available && status.authenticated) return { ok: true };
      const message = status.available
        ? "Pi has no authenticated or configured models. Run Pi and use /login, then refresh models."
        : `Pi CLI is unavailable: ${status.error || "install Pi separately and make sure `pi` is on PATH."}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    } catch (e) {
      const message = `Pi CLI is unavailable: ${e instanceof Error ? e.message : String(e)}`;
      const warning = isCliPathWarningMessage(message);
      setChatNotice({ tone: warning ? "warning" : "error", message });
      return { ok: false, message, warning };
    }
  }

  async function runCanonicalControlTurn(
    convo: ChatMessage[],
    options: RunTurnOptions,
    sessionId: string,
  ): Promise<RunTurnResult> {
    const store = useSessions.getState();
    const last = convo[convo.length - 1];
    if (!last || last.role !== "user") {
      return {
        status: "error",
        messages: sessionMessages(sessionId, convo),
        error: "A canonical turn must end with a user message.",
      };
    }
    await flushDeferredUserStateWrites("milim.sessions");
    const controller = claimTurnGeneration({
      sessionId,
      store,
      generationControllersRef,
    });
    if (!controller) {
      return {
        status: "skipped",
        messages: sessionMessages(sessionId, convo),
        error: "A turn is already running.",
      };
    }
    const baseMessages =
      options.canonicalAction === "regenerate" ? convo : convo.slice(0, -1);
    try {
      const command = await sendControlCommand({
        command_id: createControlCommandId(),
        kind:
          options.canonicalAction === "regenerate"
            ? "turn.regenerate"
            : "turn.send",
        thread_id: sessionId,
        payload:
          options.canonicalAction === "regenerate"
            ? {}
            : {
                text: wireMessageContent(last),
                display_text: last.content,
                attachments: controlAttachments(last.attachments),
              },
      });
      if (command.status === "queued") {
        setChatNotice({
          tone: "info",
          message: "Message queued by the Milim runtime and safe to close or reload.",
        });
        return { status: "skipped", messages: sessionMessages(sessionId) };
      }
      if (command.status !== "accepted" || !command.run_id) {
        throw new Error(command.message || `Control command ${command.status}.`);
      }
      const runId = command.run_id;
      canonicalRunIdsRef.current.set(sessionId, runId);
      const terminal = await pollControlRun(
        sessionId,
        runId,
        controller.signal,
        (items) => {
          const projected = projectControlRunMessages(items, runId);
          if (!projected.length) return;
          setMessages(
            sessionId,
            mergeControlRunMessages(baseMessages, runId, projected),
            { autoTitle: autoTitleChats },
          );
        },
      );
      await flushDeferredUserStateWrites("milim.sessions");
      if (terminal.status === "completed") {
        return { status: "done", messages: sessionMessages(sessionId) };
      }
      if (terminal.status === "cancelled" || terminal.status === "aborted") {
        return { status: "aborted", messages: sessionMessages(sessionId) };
      }
      return {
        status: "error",
        messages: sessionMessages(sessionId),
        error: terminal.error || `Run ended with status ${terminal.status}.`,
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setChatNotice({ tone: "error", message });
      return { status: "error", messages: sessionMessages(sessionId), error: message };
    } finally {
      canonicalRunIdsRef.current.delete(sessionId);
      releaseTurnGeneration({
        sessionId,
        store,
        generationControllersRef,
      });
    }
  }

  /** Stream the assistant's reply to a conversation that ends with a user turn. */
  async function runTurn(
    convo: ChatMessage[],
    selectedModel?: string,
    options: RunTurnOptions = {},
    sessionId = activeId,
  ): Promise<RunTurnResult> {
    const id = sessionId;
    if (generationControllersRef.current.has(id)) {
      return {
        status: "skipped",
        messages: sessionMessages(id, convo),
        error: "A turn is already running.",
      };
    }
    const turnSetup = resolveTurnSetup({
      sessionId: id,
      selectedModel,
      sessions: useSessions.getState().sessions,
      settings: useSessions.getState().getSettings(id),
      agents: useAgents.getState().agents,
      activeTitle,
      requireModel: requireChatModel,
      codexRuntimeModel,
      claudeRuntimeModel,
      opencodeRuntimeModel,
      piRuntimeModel,
      isCodexModel,
      isClaudeModel,
      isOpenCodeModel,
      isPiModel,
      accountRuntimeEnabled: useSettings.getState().accountRuntimeEnabled,
    });
    if (!turnSetup.ok) {
      setChatNotice({ tone: "error", message: turnSetup.error });
      return {
        status: "error",
        messages: sessionMessages(id, convo),
        error: turnSetup.error,
      };
    }
    const turnSettings = turnSetup.settings;
    const turnActiveAgent = turnSetup.activeAgent;
    const turnModel = turnSetup.model;
    const turnInstructions = turnSettings.instructions;
    const turnFolder = turnSettings.folder;
    const turnSandbox = turnSettings.sandbox;
    const turnComputerUse = turnSettings.computerUse;
    const turnMemory = turnSettings.memory;
    const turnPrivacy = turnSettings.privacy;
    const turnActiveAgentId = turnSettings.activeAgentId ?? null;
    const turnToolApproval = turnSettings.toolApproval;
    const turnDelegationPolicy =
      options.delegationPolicyOverride ?? turnSettings.delegationPolicy;
    const pendingHotSwap = useSessions
      .getState()
      .sessions.find((item) => item.id === id)?.pendingHotSwap;
    const hotSwapReview =
      pendingHotSwap?.toModel === turnSetup.model &&
      pendingHotSwap.action === "review";
    const turnPlanMode = turnSettings.planMode || hotSwapReview;
    const turnReasoningEffort = reasoningEffortForThread(
      turnSettings.reasoningEffortOverrides,
      useSettings.getState().reasoningEffortByModel,
      turnModel,
      pickerModels,
    );
    const turnTitle = turnSetup.title;
    const codexModel = turnSetup.codexModel;
    const claudeModel = turnSetup.claudeModel;
    const opencodeModel = turnSetup.opencodeModel;
    const piModel = turnSetup.piModel;
    const canonicalEligible =
      !options.legacyRuntime &&
      !options.goal &&
      options.toolApprovalGrant == null &&
      options.claudeSessionRecoveryGrant == null &&
      options.delegationPolicyOverride == null;
    if (canonicalEligible) {
      return runCanonicalControlTurn(convo, options, id);
    }
    const store = useSessions.getState();
    if (persistingTurnIdsRef.current.has(id)) {
      return {
        status: "skipped",
        messages: sessionMessages(id, convo),
        error: "This turn is still being saved.",
      };
    }
    persistingTurnIdsRef.current.add(id);
    try {
      setMessages(id, convo, { autoTitle: autoTitleChats });
      await flushDeferredUserStateWrites("milim.sessions");
    } catch (error) {
      const detail = error instanceof Error ? error.message : String(error);
      setChatNotice({
        tone: "error",
        message: `Milim could not save this turn, so it was not sent: ${detail}`,
      });
      return {
        status: "error",
        messages: sessionMessages(id, convo),
        error: detail,
      };
    } finally {
      persistingTurnIdsRef.current.delete(id);
    }
    const controller = claimTurnGeneration({
      sessionId: id,
      store,
      generationControllersRef,
    });
    if (!controller) {
      return {
        status: "skipped",
        messages: sessionMessages(id, convo),
        error: "A turn is already running.",
      };
    }
    let notReady: Awaited<ReturnType<typeof accountRuntimeNotReadyForTurn>>;
    try {
      notReady = await accountRuntimeNotReadyForTurn({
        codexModel,
        claudeModel,
        opencodeModel,
        piModel,
        conversation: convo,
        ensureCodexAccount,
        ensureClaudeAccount,
        ensureOpenCodeAccount,
        ensurePiAccount,
      });
    } catch (e) {
      releaseTurnGeneration({
        sessionId: id,
        store,
        generationControllersRef,
      });
      throw e;
    }
    if (notReady) {
      releaseTurnGeneration({
        sessionId: id,
        store,
        generationControllersRef,
      });
      setMessages(id, notReady.messages, { autoTitle: autoTitleChats });
      return {
        status: notReady.status,
        messages: sessionMessages(id, convo),
        error: notReady.error,
      };
    }
    const startedAt = Date.now();
    let resultStatus: RunTurnResult["status"] = "done";
    let resultError: string | undefined;
    const turnId = attachmentId();
    const assistantMessageId = createChatMessageId();
    setChatNotice(null);

    const { streamBatcher, append, appendThinking } = startTurnStream({
      sessionId: id,
      messageId: assistantMessageId,
      store,
      controller,
    });
    const assistantStart = createTurnAssistantStarter({
      conversation: convo,
      planMode: turnSettings.planMode,
      setMessages: (nextMessages) =>
        setMessages(id, nextMessages, { autoTitle: autoTitleChats }),
      assistantMessageId,
    });
    const beginAssistant = assistantStart.beginAssistant;
    const metricsCapture = createTurnMetricsCapture();
    let promptContext: Awaited<ReturnType<typeof prepareTurnPromptContext>>;
    try {
      promptContext = await prepareTurnPromptContext({
        sessionId: id,
        threadTitle: turnTitle,
        folder: turnFolder,
        instructions: turnInstructions,
        planMode: turnPlanMode,
        memory: turnMemory,
        conversation: convo,
        activeAgent: turnActiveAgent ?? null,
        skills,
        goal: options.goal,
        turnId,
        codexModel,
        claudeModel,
        accountRuntimeKind: codexModel
          ? "codex"
          : claudeModel
            ? "claude"
            : opencodeModel
              ? "opencode"
              : piModel
                ? "pi"
                : undefined,
        model: turnModel,
        sandbox: turnSandbox,
        computerUse: turnComputerUse,
        privacy: turnPrivacy,
        previewSurface:
          sidePanelVisible &&
          (inspectorTab === "preview" || inspectorTab === "code")
            ? activePreviewSurface
            : null,
        activeAgentId: turnActiveAgentId,
        toolApproval: turnToolApproval,
        toolApprovalGrant: false,
        experimentalHashlinePatch,
        delegationPolicy: turnDelegationPolicy,
        workerModel: turnSettings.workerModel,
        messageContent: wireMessageContent,
        searchMemory: searchGraphMemory,
        selectSkills,
        virtualProjectFiles: turnFolder.trim()
          ? []
          : sessionVirtualProjectFiles(
              store.sessions.find((session) => session.id === id),
            ),
        tools: composerTools,
      });
    } catch (e) {
      releaseTurnGeneration({
        sessionId: id,
        store,
        generationControllersRef,
      });
      throw e;
    }
    const { useTools, accountRuntimeMayUseTools, runMemoryContext } =
      promptContext;
    const toolApprovalDecision = resolveTurnToolApproval({
      useTools,
      accountRuntimeMayUseTools,
      toolApproval: turnToolApproval,
      planMode: turnPlanMode,
      requestedGrant: options.toolApprovalGrant,
    });
    if (toolApprovalDecision.status === "denied") {
      generationControllersRef.current.delete(id);
      store.setSessionGenerating(id, false);
      setMessages(id, convo, { autoTitle: autoTitleChats });
      setChatNotice({ tone: "info", message: "Tool run canceled." });
      return {
        status: "skipped",
        messages: sessionMessages(id, convo),
        error: toolApprovalDecision.error,
      };
    }
    const toolApprovalGrant = toolApprovalDecision.grant;
    const toolContext = {
      ...promptContext.toolContext,
      tool_approval_grant: toolApprovalGrant,
    };
    const createWorkspaceCheckpoint = () =>
      checkpointWorkspaceBeforeTurn({
        sessionId: id,
        turnId,
        folder: turnFolder,
        planMode: turnPlanMode,
        useTools,
        accountRuntimeMayUseTools,
        setWorkspace,
        runWorkspaceGitAction,
        attachCheckpoint: attachAssistantWorkspaceCheckpoint,
        appendStreamEvent: (targetId, part) =>
          store.appendStreamEvent(targetId, assistantMessageId, part),
      });

    const { runRef, snapshot } = createTurnRunTraceState((run) =>
      store.commitRun(id, assistantMessageId, run),
    );
    const captureAgentUsageDelta = (usage?: TokenUsage) => {
      const totalUsage = metricsCapture.captureUsageDelta(usage);
      if (!totalUsage || !assistantStart.state.started) return;
      commitResponseMetrics(
        id,
        assistantMessageId,
        responseMetricsForTurn({
          startedAt,
          endedAt: Date.now(),
          pausedMs: approvalWaitDuration(runRef.current),
          model: turnModel,
          providers,
          codexModel,
          claudeModel,
          usage: totalUsage,
          limits: metricsCapture.state.limits,
        }),
      );
    };
    const onToolCompleted = (name: string) => {
      if (isGoogleWorkspaceEditTool(name)) {
        window.dispatchEvent(new Event("milim-google-workspace-refresh"));
      }
    };
    const onEvent = createAgentRunEventHandler({
      runRef,
      append,
      appendThinking,
      flush: () => streamBatcher.flush(),
      appendStreamEvent: (part) =>
        store.appendStreamEvent(id, assistantMessageId, part),
      completeStreamEvent: (name, part, callId) =>
        store.completeStreamEvent(id, assistantMessageId, name, part, callId),
      appendMemoryNotice: (notice) =>
        store.appendMemoryNotice(id, assistantMessageId, notice),
      upsertChildThread: (thread) => store.upsertChildThread(id, thread),
      updateChildThread: (thread) => store.updateChildThread(thread),
      upsertWorkerRun: (record) => {
        store.upsertWorkerRun(record);
        startWorkerRunEvents(record);
      },
      onToolCompleted,
      captureUsage: metricsCapture.captureUsage,
      captureUsageDelta: captureAgentUsageDelta,
      snapshot,
    });
    const onAgentEvent = (event: AgentEvent) => {
      onEvent(event);
      if (event.type === "tool_result" && event.name === "mcp_server_save") {
        void listTools().then(setComposerTools).catch(() => {});
      }
    };
    const prepareOutbound = (
      contextMessages: ChatMessage[],
      conversation: ChatMessage[],
      options?: PrepareTurnOutboundOptions,
    ) =>
      prepareTurnOutbound({
        sessionId: id,
        contextMessages,
        conversation,
        model: turnModel,
        models: pickerModels,
        folder: turnFolder,
        toolContext: promptContext.toolContext,
        reasoningEffort: turnReasoningEffort,
        compactionInFlightRef,
        setChatNotice,
        createCompactionCheckpoint,
        clearAccountRuntime: (targetId) => store.clearAccountRuntime(targetId),
        skipAutoCompaction: options?.skipAutoCompaction,
        signal: options?.signal,
        reservedContextMessages: options?.reservedContextMessages,
        fixedCategories: options?.fixedCategories,
      });

    try {
      if (codexModel || claudeModel || opencodeModel || piModel) {
        const accountRuntime = useSessions
          .getState()
          .sessions.find((session) => session.id === id)?.accountRuntime;
        const accountResult = await runSelectedAccountRuntimeTurn({
          codexModel,
          claudeModel,
          opencodeModel,
          piModel,
          accountRuntime,
          promptContext,
          conversation: assistantStart.state.activeConversation,
          prepareOutbound,
          beginAssistant,
          checkpointWorkspace: createWorkspaceCheckpoint,
          workspace: turnFolder.trim() || undefined,
          reasoningEffort: turnReasoningEffort,
          toolApproval: turnToolApproval,
          toolApprovalGrant,
          planMode: turnPlanMode,
          allowClaudeSessionRecovery: options.claudeSessionRecoveryGrant,
          append,
          appendThinking,
          flush: () => streamBatcher.flush(),
          appendStreamEvent: (part) =>
            store.appendStreamEvent(id, assistantMessageId, part),
          completeStreamEvent: (name, part) =>
            store.completeStreamEvent(id, assistantMessageId, name, part),
          captureRuntimeMetrics: metricsCapture.captureRuntimeMetrics,
          captureProviderLimit: metricsCapture.captureProviderLimit,
          onToolCompleted,
          onNativeWorker:
            turnDelegationPolicy === "auto" &&
            (turnToolApproval === "guarded" || turnPlanMode)
              ? (lifecycle) =>
                  store.upsertWorkerRun(
                    nativeWorkerRunRecord(
                      lifecycle,
                      id,
                      assistantMessageId,
                      turnModel,
                    ),
                  )
              : undefined,
          setCodexThreadId: (threadId) =>
            store.setAccountRuntime(id, { codexThreadId: threadId }),
          appendImage: (ev) => {
            appendAssistantMediaResult(
              id,
              codexImageMediaResult(ev, codexModel ?? ""),
            );
            store.appendStreamEvent(
              id,
              assistantMessageId,
              statusPart(
                "Generated image",
                ev.revised_prompt ? compactText(ev.revised_prompt) : undefined,
              ),
            );
          },
          ensureClaudeSessionId: () =>
            useSessions.getState().ensureClaudeSessionId(id),
          setOpenCodeSessionId: (sessionId) =>
            store.setAccountRuntime(id, { opencodeSessionId: sessionId }),
          setPiSessionId: (sessionId) =>
            store.setAccountRuntime(id, { piSessionId: sessionId }),
          streamHarnessRun,
          signal: controller.signal,
          models: pickerModels,
          runRef,
          snapshot,
        });
        if (accountResult?.status === "skipped") {
          resultStatus = "skipped";
          resultError = accountResult.error;
          if (
            accountResult.error?.startsWith(CLAUDE_SESSION_RECOVERY_REQUIRED)
          ) {
            const detail = accountResult.error
              .slice(CLAUDE_SESSION_RECOVERY_REQUIRED.length + 1)
              .trim();
            requestClaudeSessionRecoveryCard(
              id,
              sessionMessages(id).filter(
                (message) => message.id !== assistantMessageId,
              ),
              turnModel,
              detail,
            );
            resultError = undefined;
          }
        }
      } else if (useTools) {
        startChildThreadEvents(id);
        const agentResult = await runToolAgentTurn({
          promptContext,
          conversation: assistantStart.state.activeConversation,
          prepareOutbound,
          beginAssistant,
          checkpointWorkspace: createWorkspaceCheckpoint,
          streamAgentRun,
          agentId: turnActiveAgentId,
          model: turnModel,
          onEvent: onAgentEvent,
          signal: controller.signal,
          runMemoryContext,
          toolContext,
          reasoningEffort: turnReasoningEffort,
          runRef,
          snapshot,
          workspace: turnFolder.trim() || undefined,
          sourceSessionId: APP_SESSION_ID,
          models: pickerModels,
        });
        if (agentResult.status === "error") {
          resultStatus = "error";
          resultError = agentResult.error || "Agent run failed.";
        }
      } else {
        await runModelChatTurn({
          promptContext,
          conversation: assistantStart.state.activeConversation,
          prepareOutbound,
          beginAssistant,
          streamChat,
          model: turnModel,
          append,
          signal: controller.signal,
          appendThinking,
          captureUsage: metricsCapture.captureUsage,
          reasoningEffort: turnReasoningEffort,
          models: pickerModels,
          runRef,
          snapshot,
          workspace: turnFolder.trim() || undefined,
        });
      }
      if (resultStatus === "done") await streamBatcher.drain();
      else streamBatcher.flush();
    } catch (e) {
      const errorResult = handleTurnRuntimeError({
        error: e,
        assistantStarted: assistantStart.state.started,
        append,
        flush: () => streamBatcher.flush(),
        setChatNotice,
        appendStreamEvent: (part) =>
          store.appendStreamEvent(id, assistantMessageId, part),
        runRef,
        snapshot,
        signal: controller.signal,
      });
      resultStatus = errorResult.status;
      resultError = errorResult.error;
    } finally {
      const endedAt = Date.now();
      const pendingApprovals = runRef.current?.steps.filter(
        (step) =>
          step.approval?.status === "pending" ||
          step.approval?.status === "decided" ||
          step.approval?.status === "delivered",
      ) ?? [];
      for (const step of pendingApprovals) {
        const approval = step.approval;
        if (!approval) continue;
        approval.status = "canceled";
        approval.resolvedAt = endedAt;
        store.completeStreamEvent(
          id,
          assistantMessageId,
          `approval:${approval.id}`,
          {
            kind: "event",
            eventType: "status",
            label: "Tool approval canceled",
            detail: step.arguments,
            icon: "tool",
            name: `approval:${approval.id}`,
            status: "done",
            approvalId: approval.id,
            approvalStatus: "canceled",
          },
        );
      }
      if (pendingApprovals.length) snapshot();
      const runtimeKind = accountRuntimeKind(turnModel);
      if (
        assistantStart.state.started &&
        runtimeKind &&
        runRef.current?.context &&
        (resultStatus === "error" || resultStatus === "aborted")
      ) {
        store.clearAccountRuntimeKind(id, runtimeKind);
      } else if (resultStatus === "done" && assistantStart.state.started) {
        if (codexModel) {
          store.setAccountRuntime(id, {
            codexLastSyncedMessageId: assistantMessageId,
          });
        } else if (claudeModel) {
          store.setAccountRuntime(id, {
            claudeLastSyncedMessageId: assistantMessageId,
          });
        } else if (opencodeModel) {
          store.setAccountRuntime(id, {
            opencodeLastSyncedMessageId: assistantMessageId,
          });
        } else if (piModel) {
          store.setAccountRuntime(id, {
            piLastSyncedMessageId: assistantMessageId,
          });
        }
      }
      if (assistantStart.state.started && pendingHotSwap?.toModel === turnModel) {
        store.setPendingHotSwap(id, undefined);
      }
      const finalMetrics = assistantStart.state.started
        ? responseMetricsForTurn({
            startedAt,
            endedAt,
            pausedMs: approvalWaitDuration(runRef.current, endedAt),
            model: turnModel,
            providers,
            codexModel,
            claudeModel,
            usage: metricsCapture.state.usage,
            costUsd: metricsCapture.state.costUsd,
            limits: metricsCapture.state.limits,
          })
        : undefined;
      finalizeTurnRuntime({
        sessionId: id,
        model: turnModel,
        status: resultStatus,
        flush: () => streamBatcher.flush(),
        metrics: finalMetrics,
        commitResponseMetrics: (targetId, metrics) =>
          commitResponseMetrics(targetId, assistantMessageId, metrics),
        finalizeMessageArtifacts: (targetId) =>
          store.finalizeMessageArtifacts(targetId, assistantMessageId),
        clearController: (targetId) =>
          generationControllersRef.current.delete(targetId),
        setSessionGenerating: store.setSessionGenerating,
        setSessionUnread: store.setSessionUnread,
        activeSessionId: useSessions.getState().activeId,
        stopChildThreadEventsIfIdle,
        maybeGenerateAiThreadTitle,
        flushUserState: () => flushDeferredUserStateWrites("milim.sessions"),
        signal: controller.signal,
      });
    }
    return {
      status: resultStatus,
      messages: sessionMessages(id, assistantStart.state.activeConversation),
      error: resultError,
    };
  }

  function send() {
    const text = input.trim();
    if (!text && pendingAttachments.length === 0 && pendingReviewComments.length === 0) return;
    if (compactionInFlightRef.current) {
      setChatNotice({
        tone: "info",
        message: "Wait for context compaction to finish before sending.",
      });
      return;
    }
    if (busy) {
      if (pendingReviewComments.length) {
        setChatNotice({ tone: "info", message: "Wait for the current reply before sending review comments." });
        return;
      }
      if (goalComposerMode) {
        setChatNotice({
          tone: "info",
          message: "Wait for the current reply to finish before starting a goal.",
        });
        return;
      }
      if (activeMediaTarget) {
        setChatNotice({
          tone: "error",
          message:
            "Wait for media generation to finish before sending another media prompt.",
        });
        return;
      }
      enqueueQueuedMessage(activeId, {
        content: text,
        attachments: pendingAttachments,
      });
      if (text) recordGlobalPrompt(text);
      setInput("");
      setPendingAttachments([]);
      setChatNotice({
        tone: "info",
        message:
          "Message queued. It will run after the current reply finishes.",
      });
      return;
    }
    if (activeMediaTarget) {
      if (!text) {
        setChatNotice({
          tone: "error",
          message: "Describe the media to generate before sending.",
        });
        return;
      }
      recordGlobalPrompt(text);
      void sendMediaPrompt(text, activeMediaTarget);
      return;
    }
    if (goalComposerMode) {
      if (!text) return;
      if (
        startGoalRun(
          {
            objective: text,
            successCriteria: "",
            constraints: "",
            developerMaxTurns: null,
          },
          pendingAttachments,
        )
      ) {
        recordGlobalPrompt(text);
        setInput("");
        setPendingAttachments([]);
        setGoalComposerSessions((current) => {
          const next = { ...current };
          delete next[activeId];
          return next;
        });
      }
      return;
    }
    const selectedModel = requireChatModel();
    if (!selectedModel) return;
    const attachments = pendingAttachments;
    const reviewComments = pendingReviewComments;
    setInput("");
    setPendingAttachments([]);
    setPendingReviewComments([]);
    const conversation = appendUserTurn(messages, text, attachments);
    if (text) recordGlobalPrompt(text);
    if (reviewComments.length) {
      conversation[conversation.length - 1] = {
        ...conversation[conversation.length - 1],
        reviewComments,
      };
    }
    void runTurnAndDrain(
      conversation,
      selectedModel,
    );
  }

  function sendArtifactFixPrompt(prompt: string) {
    const text = prompt.trim();
    if (!text) return;
    enqueueQueuedMessage(activeId, { content: text });
    setChatNotice({
      tone: "info",
      message: "Fix prepared in the editable message queue.",
    });
  }

  function executePlan(messageIndex: number, planMessage: ChatMessage) {
    if (busy || activeMediaTarget) return;
    const planText = wireMessageContent(planMessage).trim();
    if (!planText) return;
    const selectedModel = requireChatModel();
    if (!selectedModel) return;
    const now = Date.now();
    const baseMessages = messages.map((message, index) =>
      index === messageIndex
        ? { ...message, plan: { status: "executed" as const, executedAt: now } }
        : message,
    );
    updateThreadSettings(activeId, { planMode: false });
    setChatNotice({ tone: "info", message: "Plan approved. Executing..." });
    void runTurnAndDrain(
      appendUserTurn(baseMessages, executePlanPrompt(planText)),
      selectedModel,
    );
  }

  /** Re-run the last user turn (drop trailing assistant message(s)). */
  function regenerate() {
    if (busy) return;
    const convo = regenerateTurnConversation(messages);
    if (!convo) return;
    const last = convo[convo.length - 1];
    if (activeMediaTarget && last?.role === "user" && last.content.trim()) {
      void sendMediaPrompt(
        last.content.trim(),
        activeMediaTarget,
        convo.slice(0, -1),
        false,
      );
      return;
    }
    void runTurnAndDrain(convo, undefined, { canonicalAction: "regenerate" });
  }

  /** Replace the user message at `index`, drop everything after it, re-run. */
  function editResend(index: number, newText: string) {
    setEditing(null);
    const text = newText.trim();
    if (busy || !text) return;
    if (activeMediaTarget) {
      void sendMediaPrompt(
        text,
        activeMediaTarget,
        messages.slice(0, index),
        false,
      );
      return;
    }
    const convo = editResendConversation(messages, index, text);
    if (!convo) return;
    void runTurnAndDrain(convo, undefined, { legacyRuntime: true });
  }

  async function stopSessionRun(sessionId: string) {
    const canonicalRunId = canonicalRunIdsRef.current.get(sessionId);
    if (canonicalRunId) {
      const result = await sendControlCommand({
        command_id: createControlCommandId(),
        kind: "turn.stop",
        thread_id: sessionId,
        payload: { run_id: canonicalRunId },
      });
      if (result.status !== "applied") {
        throw new Error(result.message || "The canonical run could not be stopped.");
      }
      generationControllersRef.current.get(sessionId)?.abort();
      return;
    }
    const workerRun = useSessions
      .getState()
      .workerRuns.find(
        (record) =>
          record.run.parent_thread_id === sessionId &&
          record.run.status === "running",
      );
    if (workerRun) {
      approvedWorkerRunsRef.current.delete(workerRun.run.id);
      const store = useSessions.getState();
      store.setWorkerRunPending(sessionId, workerRun.run.id, false);
      store.upsertWorkerRun(await stopWorkerRun(workerRun.run.id));
    }
    const session = useSessions
      .getState()
      .sessions.find((item) => item.id === sessionId);
    if (
      session?.worker?.status === "queued" ||
      session?.worker?.status === "running"
    ) {
      const thread = await stopChildThread(sessionId);
      useSessions.getState().updateChildThread(thread);
      return;
    }
    generationControllersRef.current.get(sessionId)?.abort();
  }

  function stop() {
    void stopSessionRun(activeId).catch((error) =>
      setChatNotice({
        tone: "error",
        message: `Worker stop failed: ${error instanceof Error ? error.message : String(error)}`,
      }),
    );
  }

  function focusComposer() {
    window.requestAnimationFrame(() => {
      document
        .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
        ?.focus();
    });
  }

  function openChatSearch() {
    setChatSearchOpen(true);
  }

  function clearRecentThreadSwitcherTimer() {
    if (recentThreadSwitcherTimerRef.current == null) return;
    window.clearTimeout(recentThreadSwitcherTimerRef.current);
    recentThreadSwitcherTimerRef.current = null;
  }

  function scheduleRecentThreadSwitcherClose() {
    clearRecentThreadSwitcherTimer();
    recentThreadSwitcherTimerRef.current = window.setTimeout(() => {
      setRecentThreadSwitcher(null);
      recentThreadSwitcherTimerRef.current = null;
    }, RECENT_THREAD_SWITCHER_CLOSE_MS);
  }

  function closeRecentThreadSwitcher() {
    clearRecentThreadSwitcherTimer();
    setRecentThreadSwitcher(null);
  }

  async function switchVisibleSession(id: string) {
    if (id === activeId) return;
    if (!(await requestWorkspaceEditorLeave("navigate"))) return;
    switchToSession(id);
  }

  function selectRecentThread(id: string) {
    closeRecentThreadSwitcher();
    switchVisibleSession(id);
  }

  function switchToPreviousThread() {
    if (recentThreadSwitcher?.items.length) {
      const activeIndex = nextRecentThreadSwitcherIndex(
        recentThreadSwitcher.activeIndex,
        recentThreadSwitcher.items.length,
      );
      const next = { ...recentThreadSwitcher, activeIndex };
      const nextId = next.items[activeIndex]?.id;
      setRecentThreadSwitcher(next);
      scheduleRecentThreadSwitcherClose();
      if (nextId) switchVisibleSession(nextId);
      return;
    }

    const items = recentThreadSwitcherItems(
      recentThreadIdsRef.current,
      activeId,
      sessionSummaries,
      projects,
    );
    const nextId = items[0]?.id;
    if (!nextId) return;
    setRecentThreadSwitcher({ items, activeIndex: 0 });
    scheduleRecentThreadSwitcherClose();
    switchVisibleSession(nextId);
  }

  function startShortcutNewChat() {
    setInput("");
    setPendingAttachments([]);
    setChatNotice(null);
    void createInteractiveChat();
    focusComposer();
  }

  function stopFromShortcut() {
    if (!busy) return;
    const now = Date.now();
    if (now <= stopShortcutConfirmUntilRef.current) {
      stopShortcutConfirmUntilRef.current = 0;
      if (stopShortcutConfirmTimerRef.current != null) {
        window.clearTimeout(stopShortcutConfirmTimerRef.current);
        stopShortcutConfirmTimerRef.current = null;
      }
      stop();
      return;
    }

    const message = `Press ${shortcutLabel(appShortcuts.stopGeneration)} again to stop generation.`;
    stopShortcutConfirmUntilRef.current = now + 2000;
    setChatNotice({ tone: "info", message });
    if (stopShortcutConfirmTimerRef.current != null)
      window.clearTimeout(stopShortcutConfirmTimerRef.current);
    stopShortcutConfirmTimerRef.current = window.setTimeout(() => {
      stopShortcutConfirmUntilRef.current = 0;
      setChatNotice((notice) => (notice?.message === message ? null : notice));
      stopShortcutConfirmTimerRef.current = null;
    }, 2000);
  }

  const paletteCommands: RuntimeCommand[] = [
    {
      id: "chat.new",
      label: "New chat",
      keywords: ["thread", "conversation"],
      shortcut: shortcutLabel(appShortcuts.newChat),
      run: startShortcutNewChat,
    },
    {
      id: "composer.focus",
      label: "Focus composer",
      keywords: ["prompt", "input"],
      shortcut: shortcutLabel(appShortcuts.focusComposer),
      run: focusComposer,
    },
    {
      id: "composer.suggestions",
      label: "Open composer suggestions",
      keywords: ["autocomplete", "commands", "skills", "files"],
      shortcut: shortcutLabel(appShortcuts.openComposerSuggestions),
      run: () => window.dispatchEvent(new Event("milim:open-composer-suggestions")),
    },
    {
      id: "sidebar.toggle",
      label: sidebarOpen ? "Hide sidebar" : "Show sidebar",
      keywords: ["toggle", "navigation"],
      shortcut: shortcutLabel(appShortcuts.toggleSidebar),
      available: threadNavigationPlacement === "sidebar",
      run: toggleSidebar,
    },
    {
      id: "thread.previous",
      label: "Previous thread",
      keywords: ["chat", "recent", "switch"],
      shortcut: shortcutLabel(appShortcuts.previousThread),
      available: sessionSummaries.length > 1,
      run: switchToPreviousThread,
    },
    {
      id: "generation.stop",
      label: "Stop generation",
      keywords: ["cancel", "abort"],
      shortcut: shortcutLabel(appShortcuts.stopGeneration),
      available: busy,
      run: stop,
    },
    {
      id: "settings.open",
      label: "Open settings",
      keywords: ["preferences", "configuration"],
      run: onOpenSettings,
    },
    {
      id: "diagnostics.open",
      label: "Open diagnostics",
      keywords: ["logs", "recovery", "debug"],
      available: inTauri,
      run: () => {
        void openDiagnosticsFolder().catch((error) =>
          setChatNotice({
            tone: "error",
            message: error instanceof Error ? error.message : String(error),
          }),
        );
      },
    },
  ];

  function shortcutTargetBlocked(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) return false;
    return Boolean(
      target.closest('[role="dialog"], [data-shortcut-recorder="true"]'),
    );
  }

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.defaultPrevented || shortcutTargetBlocked(event.target)) return;
      if (recentThreadSwitcher && event.key === "Escape") {
        event.preventDefault();
        closeRecentThreadSwitcher();
        return;
      }
      if (shortcutMatchesEvent(appShortcuts.newChat, event)) {
        event.preventDefault();
        startShortcutNewChat();
      } else if (shortcutMatchesEvent(appShortcuts.focusSearch, event)) {
        event.preventDefault();
        openChatSearch();
      } else if (shortcutMatchesEvent(appShortcuts.focusComposer, event)) {
        event.preventDefault();
        focusComposer();
      } else if (shortcutMatchesEvent(appShortcuts.openComposerSuggestions, event)) {
        event.preventDefault();
        window.dispatchEvent(new Event("milim:open-composer-suggestions"));
      } else if (
        threadNavigationPlacement === "sidebar" &&
        shortcutMatchesEvent(appShortcuts.toggleSidebar, event)
      ) {
        event.preventDefault();
        toggleSidebar();
      } else if (shortcutMatchesEvent(appShortcuts.previousThread, event)) {
        event.preventDefault();
        switchToPreviousThread();
      } else if (shortcutMatchesEvent(appShortcuts.stopGeneration, event)) {
        event.preventDefault();
        stopFromShortcut();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [
    activeId,
    appShortcuts,
    busy,
    projects,
    recentThreadSwitcher,
    sessionSummaries,
    switchToSession,
    threadSettings,
    threadNavigationPlacement,
    toggleSidebar,
  ]);

  function promoteQueuedMessage(messageId: string) {
    const first =
      useSessions.getState().queuedMessagesBySession[activeId]?.[0]?.id;
    if (first && first !== messageId)
      moveQueuedMessage(activeId, messageId, first, "before");
  }

  function activateQueuedMessage(item: QueuedMessage) {
    if (queueInterrupts[activeId]) return;
    if (activeMediaTarget) {
      setChatNotice({
        tone: "error",
        message: "Choose a chat model before running queued messages.",
      });
      return;
    }
    const selectedModel = requireChatModel();
    if (!selectedModel) return;
    promoteQueuedMessage(item.id);
    if (!busy) {
      void drainQueuedMessages(activeId, selectedModel);
      return;
    }
    const sessionId = activeId;
    setQueueInterrupts((current) => ({
      ...current,
      [sessionId]: item.id,
    }));
    void stopSessionRun(sessionId).catch((error) => {
      setQueueInterrupts((current) => {
        if (current[sessionId] !== item.id) return current;
        const next = { ...current };
        delete next[sessionId];
        return next;
      });
      setChatNotice({
        tone: "error",
        message: `Interrupt failed: ${error instanceof Error ? error.message : String(error)}`,
      });
    });
  }

  function editQueuedMessage(item: QueuedMessage) {
    if (input.trim() || pendingAttachments.length > 0) {
      setChatNotice({
        tone: "info",
        message: "Clear the composer before editing a queued message.",
      });
      return;
    }
    removeQueuedMessage(activeId, item.id);
    setInput(item.content);
    setPendingAttachments(item.attachments ?? []);
    setChatNotice({
      tone: "info",
      message: "Queued message moved back to composer.",
    });
    window.requestAnimationFrame(() => {
      document
        .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
        ?.focus();
    });
  }

  function applyScheduleRunEvent(event: ScheduleRunEvent) {
    const title = `Schedule: ${event.schedule_name || event.schedule_id}`;
    const importedId = useSessions.getState().importSession({
      title,
      messages: [
        { role: "user", content: event.prompt },
        { role: "assistant", content: event.response || "(No response.)" },
      ],
      settings: { model: event.model },
    });
    if (importedId) {
      const notice = { tone: "info", message: `${title} completed.` } as const;
      setChatNotice(notice);
      pushNotice(notice);
    }
  }

  useEffect(() => {
    let cancelled = false;
    async function pollScheduleRuns() {
      if (!documentVisible()) return;
      if (scheduleRunPollingRef.current) return;
      scheduleRunPollingRef.current = true;
      try {
        const events = await pollScheduleRunEvents();
        if (!cancelled) {
          for (const event of events) applyScheduleRunEvent(event);
        }
      } finally {
        scheduleRunPollingRef.current = false;
      }
    }
    void pollScheduleRuns();
    const timer = window.setInterval(() => void pollScheduleRuns(), 5000);
    const onVisible = () => {
      if (documentVisible()) void pollScheduleRuns();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  const mobileSnapshot = useMemo(
    () => ({
      session_id: activeId,
      title: activeTitle,
      model: effectiveModel.trim() || null,
      busy,
      messages: mobileThreadMessages(messages),
      threads: mobileThreadSummaries(
        sessionSummaries,
        projects,
        generatingSessionIds,
      ),
      groups: mobileThreadGroups(
        sessionSummaries,
        projects,
        sidebarState,
        generatingSessionIds,
      ),
      models: mobileModelSummaries(pickerModels),
      theme: {
        is_dark: activeTheme.isDark,
        css_vars: themeCssVariables(activeTheme),
        background_fit: backgroundFit,
        background_treatment: backgroundTreatment,
      },
      worker_run: mobileWorkerRun(activeWorkerRun),
    }),
    [
      activeId,
      activeTheme,
      activeWorkerRun,
      activeTitle,
      backgroundFit,
      backgroundTreatment,
      busy,
      effectiveModel,
      generatingSessionIds,
      messages,
      pickerModels,
      projects,
      sessionSummaries,
      sidebarState,
    ],
  );
  useChatMobileRelayController({
    pollKey: `${activeId}\u0000${busy}\u0000${effectiveModel}`,
    snapshot: mobileSnapshot,
    setInput,
    setPendingAttachments,
    setChatNotice,
    setProvidersOpen,
    pushNotice,
    createAttachmentId: attachmentId,
    runTurn,
    runTurnAndDrain,
    drainQueuedMessages,
    approveWorkerRun,
    continueWorkerRunSolo,
    stopActiveWorkerRun,
    stop,
    regenerate,
    deleteMessageAt,
  });

  const emptyThread = messages.length === 0;
  const activeAssistantRuntime = useMemo(() => {
    if (!busy) return { run: null, streamParts: undefined };
    let run: RunTrace | null = null;
    let streamParts: ChatStreamPart[] | undefined;
    for (let i = messages.length - 1; i >= 0; i -= 1) {
      const message = messages[i];
      if (message.role !== "assistant") continue;
      run ??= message.run?.status === "running" ? message.run : null;
      streamParts ??= message.streamParts?.length
        ? message.streamParts
        : undefined;
      if (run && streamParts) break;
    }
    return { run, streamParts };
  }, [busy, messages]);
  const activeRun = activeAssistantRuntime.run;
  const activeStreamParts = activeAssistantRuntime.streamParts;
  const debugPreviewControlActivity =
    typeof window === "undefined"
      ? null
      : previewControlActivityFromDebugUrl(window.location.href);
  const streamPreviewControlActivity =
    previewControlActivityFromStreamParts(activeStreamParts);
  const streamPreviewControlActivityId =
    streamPreviewControlActivity?.id ?? null;
  const [recentPreviewControlActivity, setRecentPreviewControlActivity] =
    useState<ReturnType<typeof previewControlActivityFromStreamParts>>(null);
  useEffect(() => {
    if (!streamPreviewControlActivity) return;
    setRecentPreviewControlActivity(streamPreviewControlActivity);
    const timer = window.setTimeout(
      () => setRecentPreviewControlActivity(null),
      1900,
    );
    return () => window.clearTimeout(timer);
  }, [streamPreviewControlActivityId]);
  const previewControlActivity =
    debugPreviewControlActivity ??
    streamPreviewControlActivity ??
    recentPreviewControlActivity;
  const canOpenArtifactPanel = Boolean(activeArtifactSelection || folder.trim());

  async function openPreviewInspector() {
    if (inspectorTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    rememberInspectorInvoker();
    if (activeInspectorPreviewSource === "artifact" && !canOpenArtifactPanel) {
      selectPreviewSource(
        availablePreviewSources.includes("app") ? "app" : "url",
      );
    }
    setSessionInspectorTab(activeId, "preview");
  }

  function moveInspectorTabFocus(event: KeyboardEvent<HTMLDivElement>) {
    if (![
      "ArrowLeft",
      "ArrowRight",
      "Home",
      "End",
    ].includes(event.key)) return;
    const tabs = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>(
        '[role="tab"]:not(:disabled)',
      ),
    );
    const currentIndex = tabs.indexOf(document.activeElement as HTMLButtonElement);
    if (!tabs.length || currentIndex < 0) return;
    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : (currentIndex + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    tabs[nextIndex]?.focus();
    tabs[nextIndex]?.click();
  }

  const inspectorTabSwitcher = (
    <div
      className="side-panel-switcher"
      role="tablist"
      aria-label="Inspector"
      onKeyDown={moveInspectorTabFocus}
    >
      <button
        id="inspector-tab-preview"
        type="button"
        className={inspectorTab === "preview" ? "active" : ""}
        role="tab"
        aria-selected={inspectorTab === "preview"}
        aria-controls="inspector-panel-preview"
        tabIndex={inspectorTab === "preview" ? 0 : -1}
        onClick={openPreviewInspector}
      >
        <Eye size={14} />
        <span>Preview</span>
      </button>
      {canOpenArtifactPanel && (
        <button
          id="inspector-tab-code"
          type="button"
          className={inspectorTab === "code" ? "active" : ""}
          role="tab"
          aria-selected={inspectorTab === "code"}
          aria-controls="inspector-panel-code"
          tabIndex={inspectorTab === "code" ? 0 : -1}
          onClick={() => openArtifactSidePanel("code")}
        >
          <Code size={14} />
          <span>Code</span>
        </button>
      )}
      {canShowGitPanel && (
        <button
          id="inspector-tab-git"
          type="button"
          className={inspectorTab === "git" ? "active" : ""}
          role="tab"
          aria-selected={inspectorTab === "git"}
          aria-controls="inspector-panel-git"
          tabIndex={inspectorTab === "git" ? 0 : -1}
          onClick={() => openGitPanel()}
        >
          <GitBranch size={14} />
          <span>Git</span>
        </button>
      )}
      <button
        id="inspector-tab-workers"
        type="button"
        className={inspectorTab === "workers" ? "active" : ""}
        role="tab"
        aria-selected={inspectorTab === "workers"}
        aria-controls="inspector-panel-workers"
        tabIndex={inspectorTab === "workers" ? 0 : -1}
        onClick={() => openWorkersInspector()}
      >
        <UserRound size={14} />
        <span>Workers</span>
      </button>
    </div>
  );

  messageRowActionsRef.current = {
    openContextMenu,
    setInspectorTab: setSessionInspectorTab,
    openWorkers: openWorkersInspector,
    preparePreviewRuntimeForArtifacts,
    openPreviewArtifact,
    artifactRevisionChoice,
    executePlan,
    restoreWorkspaceCheckpoint,
    forkThreadAt,
    setEditing,
    deleteMessageAt,
    regenerate,
    startBaton: (action, messageIndex) =>
      setBatonRequest({ action, messageIndex }),
    undoTurnChanges,
    reviewTurnChanges,
    retryTurnReview: () => setTurnReviewRevision((value) => value + 1),
    openGit: () => openGitPanel(),
    editResend,
    editMessageInPlace,
    approveToolApproval,
    denyToolApproval,
    handleSaveArtifact,
    handlePreviewArtifact,
    handleCheckArtifact,
    handleOpenArtifact,
    onOpenSchedules,
  };

  return (
    <div
      className={"chat" + (emptyThread ? " chat-empty" : "")}
      data-testid="chat-shell"
    >
      <div
        ref={chatBodyRef}
        className={`chat-body${panelsStacked ? " inspector-stacked" : ""}${previewPanelOverlay && !panelsStacked ? " inspector-overlay" : ""}`}
        style={previewPanelStyle}
      >
        <div className={`chat-main${contextPanelOpen ? " context-open" : ""}`}>
          <div className="chat-main-actions">
            {folder.trim() && <WorkspaceLauncherButton folder={folder} />}
            {!contextPanelOpen && (
              <button
                ref={contextLauncherRef}
                className="icon-btn context-open-btn"
                data-testid="open-context-panel"
                type="button"
                title="Open context"
                aria-label="Open context"
                aria-expanded="false"
                aria-controls="quick-summary-panel"
                onClick={openContextPanel}
              >
                <FileText size={15} />
              </button>
            )}
            {!sidePanelVisible && (
              <button
                className="icon-btn preview-open-btn"
                data-testid="open-artifact-browser"
                title={inspectorLauncherLabel}
                aria-label={inspectorLauncherLabel}
                onClick={openSelectedSidePanel}
              >
                <PanelIcon size={16} />
              </button>
            )}
          </div>
          <div className="chat-scroll-shell">
            <div
              className="chat-scroll"
              ref={chatScrollRef}
              onScroll={updateAutoScrollCoupling}
            >
              {!emptyThread && (
                <div className="messages">
                  {messages.map((m, i) => {
                    if (workerRunSynthesisId(m)) return null;
                    const messageIsCompaction = isCompactionCheckpoint(m);
                    const isApprovalMessage = Boolean(m.approval);
                    const isLastAssistant =
                      m.role === "assistant" &&
                      !messageIsCompaction &&
                      !isApprovalMessage &&
                      i === messages.length - 1;
                    const messageTurnChangesKey = m.workspaceCheckpoint
                      ? `${activeId}:${m.id ?? i}:${m.workspaceCheckpoint.ref}`
                      : "";
                    return (
                      <MessageRow
                        key={m.id ?? i}
                        activeId={activeId}
                        appSessionId={APP_SESSION_ID}
                        message={m}
                        index={i}
                        isEditing={editing === i}
                        isLastAssistant={isLastAssistant}
                        assistantStreaming={busy && isLastAssistant}
                        busy={busy}
                        activeMediaTargetPresent={Boolean(activeMediaTarget)}
                        folderIsEmpty={!folder.trim()}
                        workspaceFolder={folder}
                        activeRun={activeRun}
                        previewArtifacts={previewArtifactsForMessage(m)}
                        previewAppBusy={previewAppBusy}
                        previewAppStatus={activePreviewAppStatus}
                        toolApproval={toolApproval}
                        turnReview={
                          isLastAssistant &&
                          turnReview?.key === messageTurnChangesKey
                            ? turnReview
                            : null
                        }
                        actionsRef={messageRowActionsRef}
                      />
                    );
                  })}
                </div>
              )}
            </div>
            {showJumpToLatest && !emptyThread && (
              <button
                type="button"
                className="chat-jump-latest"
                data-testid="chat-jump-latest"
                title="Jump to latest"
                aria-label="Jump to latest message"
                onClick={jumpToLatest}
              >
                <ChevronDown size={14} aria-hidden="true" />
                <span>Latest</span>
              </button>
            )}
          </div>

          <div className="dock">
            {emptyThread && showEmptyChatRidgeline && <MilimUsageRidgeline usage={milimUsage} />}
            {composerNotice && (
              <div
                className={`sheet-hint dock-notice ${composerNotice.tone}`}
                data-testid="chat-notice"
                role={composerNotice.tone === "error" ? "alert" : "status"}
                aria-live={composerNotice.tone === "error" ? "assertive" : "polite"}
              >
                <span>{composerNotice.message}</span>
                {composerAction && (
                  <button
                    type="button"
                    onClick={() => {
                      if (composerAction === "manage_models") setProvidersOpen(true);
                      else if (composerAction === "choose_folder") void pickFolder();
                      else onOpenSettings();
                    }}
                  >
                    {composerActionLabel}
                  </button>
                )}
              </div>
            )}
            <ComposerSurface>
              {visibleApprovalPrompts.map((approval) => (
                <ToolApprovalPrompt
                  key={approval.approvalId}
                  part={approval}
                  onDismiss={() => {
                    if (!approval.approvalId) return;
                    setMessages(
                      activeId,
                      dismissToolApproval(messages, approval.approvalId),
                      { autoTitle: false },
                    );
                  }}
                />
              ))}
              <ControlBar
                models={pickerModels}
                model={model}
                reasoningEffortByModel={reasoningEffortByModel}
                reasoningEffortOverrides={reasoningEffortOverrides}
                onReasoningEffort={(modelId, effort) => {
                  updateThreadSettings(activeId, {
                    reasoningEffortOverrides: reasoningEffortOverridesWithSelection(
                      reasoningEffortOverrides,
                      modelId,
                      effort,
                    ),
                  });
                }}
                providers={providers}
                toolIntent={modelToolIntent}
                onModel={(m) => requestHotSwap(m)}
                sandbox={sandbox}
                onToggleSandbox={() =>
                  updateThreadSettings(activeId, { sandbox: !sandbox })
                }
                computerUse={computerUse}
                onToggleComputer={() =>
                  updateThreadSettings(activeId, { computerUse: !computerUse })
                }
                memory={memory}
                onToggleMemory={() =>
                  updateThreadSettings(activeId, { memory: !memory })
                }
                planMode={planMode}
                onTogglePlanMode={() => setPlanModeActive(!planMode)}
                privacy={privacy}
                onPrivacy={(next) => updateThreadSettings(activeId, { privacy: next })}
                toolApproval={toolApproval}
                onToolApproval={(next) => updateThreadSettings(activeId, { toolApproval: next })}
                onManageProviders={() => setProvidersOpen(true)}
                onManageMcp={() => setMcpOpen(true)}
                onManageMemory={() => {
                  setMemoryTarget(null);
                  setMemoryOpen(true);
                }}
                goal={goal}
                goalMode={goalComposerMode}
                onToggleGoalMode={() => setGoalComposerModeActive(false)}
                onOpenGoal={() => openGoalPanel()}
                activeRun={activeRun}
                inlineControls={
                  activeMediaTarget ? (
                    <InlineMediaControls
                      providerName={activeMediaTarget.provider.name}
                      model={activeMediaTarget.model}
                      kind={mediaKind}
                      supportedKinds={activeMediaTarget.supportedKinds}
                      schema={mediaSchema}
                      schemaLoading={mediaSchemaLoading}
                      parameterValues={mediaParameterValues}
                      advanced={mediaAdvanced}
                      error={mediaError}
                      popover
                      onKindChange={setMediaKind}
                      onParameterChange={updateInlineMediaParameter}
                      onAdvancedChange={updateInlineMediaAdvanced}
                    />
                  ) : undefined
                }
              />
              <QueuedMessageTray
                items={queuedMessages}
                busy={busy}
                canActivate={!activeMediaTarget}
                interruptingMessageId={queueInterrupts[activeId]}
                openContextMenu={openContextMenu}
                onActivate={activateQueuedMessage}
                onEdit={editQueuedMessage}
                onMove={(messageId, targetId, position) =>
                  moveQueuedMessage(activeId, messageId, targetId, position)
                }
                onRemove={(id) => removeQueuedMessage(activeId, id)}
              />
              {pendingReviewComments.length ? (
                <div className="review-comment-tray" aria-label="Pending review comments">
                  <span>{pendingReviewComments.length} review comment{pendingReviewComments.length === 1 ? "" : "s"}</span>
                  {pendingReviewComments.map((comment) => (
                    <button
                      type="button"
                      key={comment.id}
                      title={comment.body}
                      onClick={() => {
                        const body = window.prompt("Edit review comment", comment.body);
                        if (body === null) return;
                        setPendingReviewComments((current) => body.trim()
                          ? current.map((item) => item.id === comment.id ? { ...item, body: body.trim() } : item)
                          : current.filter((item) => item.id !== comment.id));
                      }}
                    >
                      {comment.filePath || comment.preview?.selector || "Preview"}:{comment.startLine ?? "element"}
                    </button>
                  ))}
                  <button type="button" onClick={() => setPendingReviewComments([])}>Clear</button>
                </div>
              ) : null}
              <Composer
                value={input}
                onChange={setInput}
                onSend={send}
                onStop={stop}
                attachments={pendingAttachments}
                onAttachFiles={handleAttachFiles}
                onAttachWorkspaceFile={handleAttachWorkspaceFile}
                onRemoveAttachment={(id) =>
                  setPendingAttachments((current) =>
                    current.filter((attachment) => attachment.id !== id),
                  )
                }
                onSlashCommand={runSlashCommand}
                agents={agents}
                activeAgentId={activeAgentId}
                onAgent={(agent) => {
                  const target = agent?.model || model;
                  updateThreadSettings(activeId, {
                    activeAgentId: agent?.id ?? null,
                    ...(target ? { model: target } : {}),
                  });
                }}
                onManageAgents={onManageAgents}
                instructions={instructions}
                onInstructions={(v) =>
                  updateThreadSettings(activeId, { instructions: v })
                }
                skills={skills}
                tools={composerTools}
                workspaceFolder={folder}
                workspaceProjects={workspaceProjects}
                workspaceChangeStartsNewChat={!emptyThread}
                onWorkspaceFolder={startChatInFolder}
                onPickWorkspaceFolder={() => void pickProjectFolder()}
                listWorkspaceFiles={listWorkspaceFiles}
                mediaActive={Boolean(activeMediaTarget)}
                mediaKind={
                  activeMediaTarget?.supportedKinds.includes(mediaKind)
                    ? mediaKind
                    : (activeMediaTarget?.kind ?? mediaKind)
                }
                mediaTargetLabel={
                  activeMediaTarget
                    ? `${activeMediaTarget.kind} / ${activeMediaTarget.provider.name}`
                    : undefined
                }
                sentHistory={sentHistory}
                requestCompletion={composerCompletionRequest}
                tokens={tokens}
                contextBudgetTokens={activeContextBudget?.promptBudget}
                busy={busy}
                hasReviewComments={pendingReviewComments.length > 0}
              />
            </ComposerSurface>
            {emptyThread && !input.trim() && !activeMediaTarget && quickActionMode !== "hidden" && (
              <EmptyStarterActions
                strip={emptyStarterStrip}
                onSelect={(id, prompt) => {
                  recordSuggestionUse(`quick:${id}`);
                  prefillEmptyStarter(prompt);
                }}
              />
            )}
          </div>
          <QuickSummaryPanel
            summary={quickSummary}
            open={contextPanelOpen}
            workerPanel={(
              <WorkersSummary
                records={activeWorkerRuns}
                policy={delegationPolicy}
                workerModel={workerModel}
                agents={agents}
                models={pickerModels.filter(
                  (item) => !item.capabilities?.imageOutput && !item.capabilities?.videoOutput && !item.capabilities?.musicOutput,
                )}
                onOpen={() => openWorkersInspector()}
                onOpenSettings={() => openWorkersInspector(undefined, true)}
              />
            )}
            collapsedSections={contextCollapsedSectionIds}
            canOpenGit={canOpenGitPanel}
            onOpenChange={(open) => open ? openContextPanel() : closeContextPanel()}
            onSectionCollapsedChange={(sectionId, collapsed) =>
              setSessionContextSectionCollapsed(activeId, sectionId, collapsed)
            }
            onOpenGit={openGitPanel}
            onOpenGoal={() => openGoalPanel()}
            onOpenSource={openQuickSummarySource}
          />
        </div>
        {sidePanelVisible && (
          <>
            {previewPanelOverlay && !panelsStacked && (
              <div className="preview-overlay-spacer" aria-hidden="true" />
            )}
            {!panelsStacked && (
              <div
                ref={previewResizeHandleRef}
                className={`preview-resize-handle${previewResizing ? " dragging" : ""}${previewPanelClosing ? " closing" : ""}${sidePanelAlreadyOpen ? " no-enter" : ""}`}
                data-testid="preview-resize-handle"
                role="separator"
                aria-label="Resize side panel; keep expanding at the limit to collapse the sidebar, then overlay the transcript"
                title="Drag to resize; keep dragging at the limit for more space; double-click to reset"
                aria-orientation="vertical"
                aria-valuemin={PREVIEW_PANEL_MIN_WIDTH}
                aria-valuemax={maxPreviewPanelWidth(
                  chatBodyWidth,
                  reservedContextWidth,
                  previewPanelOverlay,
                )}
                aria-valuenow={resolvedPreviewPanelWidth}
                aria-valuetext={`${resolvedPreviewPanelWidth} pixels, ${previewPanelOverlay ? "overlay" : "docked"}`}
                tabIndex={previewPanelClosing ? -1 : 0}
                onKeyDown={resizePreviewWithKeyboard}
                onPointerDown={startPreviewResize}
                onDoubleClick={() => {
                  setPreviewPanelOverlay(false);
                  resizePreviewPanel(DEFAULT_PREVIEW_PANEL_WIDTH, false);
                }}
              />
            )}
            <div
              id={inspectorTab === "git" ? "inspector-panel-git" : undefined}
              className={`inspector-shell${previewPanelClosing ? " closing" : ""}${sidePanelAlreadyOpen ? " no-enter" : ""}`}
              data-testid="inspector-shell"
              role={inspectorTab === "git" ? "tabpanel" : undefined}
              aria-labelledby={inspectorTab === "git" ? "inspector-tab-git" : undefined}
            >
            {inspectorTab === "workers" ? (
              <WorkersInspector
                records={activeWorkerRuns}
                focusRunId={workerFocusRunId}
                policy={delegationPolicy}
                workerModel={workerModel}
                agents={agents}
                models={pickerModels.filter(
                  (item) => !item.capabilities?.imageOutput && !item.capabilities?.videoOutput && !item.capabilities?.musicOutput,
                )}
                providers={providers}
                busy={workerActionBusy}
                settingsOpen={workerSettingsOpen}
                closing={previewPanelClosing}
                noEnterMotion={sidePanelAlreadyOpen}
                modeSwitcher={inspectorTabSwitcher}
                onSettingsOpenChange={setWorkerSettingsOpen}
                onPolicyChange={(next) =>
                  updateThreadSettings(activeId, { delegationPolicy: next })
                }
                onWorkerModelChange={(next) =>
                  updateThreadSettings(activeId, { workerModel: next })
                }
                onStart={(runId) => void approveWorkerRun(runId)}
                onStop={(runId) => void stopActiveWorkerRun(runId)}
                onContinueSolo={(runId) => void continueWorkerRunSolo(runId)}
                onStopWorker={stopOneWorker}
                onRetryWorker={retryFailedWorker}
                onDeleteRun={deleteFinishedWorkerRun}
                onLoadDiff={getWorkerDiff}
                onApplyDiff={applyWorkerDiff}
                onClose={closePreview}
              />
            ) : inspectorTab === "git" ? (
              <GitWorkspacePanel
                sessionId={activeId}
                folder={folder}
                model={effectiveModel}
                onDraftAction={loadGitActionDraft}
                requestedView={gitPanelView}
                diffRequest={
                  gitDiffRequest?.sessionId === activeId &&
                  previewRuntimeFoldersEqual(gitDiffRequest.folder, folder)
                    ? gitDiffRequest
                    : null
                }
                closing={previewPanelClosing}
                noEnterMotion={sidePanelAlreadyOpen}
                onClose={closeGitPanel}
                modeSwitcher={inspectorTabSwitcher}
                headerNotice={
                  activeSession?.retryWorkspace ? (
                    <div className="hot-swap-retry-banner">
                      <div>
                        <strong>Isolated Hot Swap retry</strong>
                        <span>
                          {activeSession.retryWorkspace.adoptedAt
                            ? "Applied to the original workspace; the retry remains available."
                            : "Review this diff before applying it to the original workspace."}
                        </span>
                      </div>
                      <div className="hot-swap-retry-actions">
                        <button className="btn-accent" type="button" disabled={busy} onClick={() => void applyRetryWorkspace()}>
                          Apply to original
                        </button>
                        <button className="btn-ghost" type="button" disabled={busy} onClick={() => void discardRetryWorkspace()}>
                          Discard retry
                        </button>
                      </div>
                    </div>
                  ) : undefined
                }
              />
            ) : (
              visiblePreviewSelection && (
                <PreviewPanel
                  artifact={visiblePreviewSelection.artifact}
                  artifacts={visiblePreviewSelection.artifacts}
                  revision={visiblePreviewSelection.revision}
                  revisionGroup={visiblePreviewSelection.revisionGroup}
                  previewDeferred={visiblePreviewSelection.previewDeferred}
                  closing={previewPanelClosing}
                  noEnterMotion={sidePanelAlreadyOpen}
                  onClose={closePreview}
                  onSelectRevision={selectPreviewRevision}
                  onPrepareArtifactFix={sendArtifactFixPrompt}
                  fixArtifact={activeArtifactSelection?.artifact}
                  fixArtifacts={activeArtifactSelection?.artifacts}
                  fixRevision={activeArtifactSelection?.revision}
                  activeTab={inspectorTab === "code" ? "code" : "preview"}
                  onActiveTabChange={(tab) =>
                    setSessionInspectorTab(activeId, tab)
                  }
                  previewSource={activeInspectorPreviewSource}
                  availablePreviewSources={availablePreviewSources}
                  onPreviewSourceChange={(source) => {
                    selectPreviewSource(source);
                    setSessionInspectorTab(activeId, "preview");
                  }}
                  browserSession={activeInspectorPreviewSource === "url" ? activeInspectorBrowserSession : undefined}
                  onBrowserSessionChange={
                    activeInspectorPreviewSource === "url" ? updateBrowserSession : undefined
                  }
                  runtimeStatus={
                    activeInspectorPreviewSource === "app"
                      ? (activePreviewAppStatus ??
                        previewIdleStatus(activePreviewRuntimeKey, folder))
                      : null
                  }
                  runtimePreflight={activePreviewAppPreflight}
                  runtimePreflightBusy={previewAppPreflightBusy}
                  runtimeStale={activePreviewAppStatus?.stale === true}
                  onRuntimePreflight={activePreviewAppStatus?.kind === "static" ? undefined : () => void preflightPreviewRuntime()}
                  runtimeBusy={previewAppBusy != null}
                  onRuntimeStart={activePreviewAppStatus?.kind === "static" ? undefined : () => void startPreviewRuntime()}
                  onRuntimeStop={() => void stopPreviewRuntime()}
                  onRuntimeRestart={activePreviewAppStatus?.kind === "static" ? undefined : () => void restartPreviewRuntime()}
                  controlActivity={previewControlActivity}
                  onSurfaceChange={setActivePreviewSurface}
                  modeSwitcher={inspectorTabSwitcher}
                  workspaceFolder={folder}
                  onPreviewWorkspaceFile={(path) => void startWorkspaceHtmlPreview(path)}
                />
              )
            )}
            </div>
          </>
        )}
      </div>

      {chatSearchOpen && (
        <CommandPalette
          projects={projects}
          activeId={activeId}
          commands={paletteCommands}
          onSelect={switchVisibleSession}
          onClose={() => setChatSearchOpen(false)}
        />
      )}

      {recentThreadSwitcher && (
        <RecentThreadSwitcherOverlay
          state={recentThreadSwitcher}
          onSelect={selectRecentThread}
        />
      )}

      {batonRequest && (
        <BatonTargetSheet
          action={batonRequest.action}
          models={pickerModels.filter(
            (item) =>
              item.id !== model &&
              !item.capabilities?.imageOutput &&
              !item.capabilities?.videoOutput &&
              !item.capabilities?.musicOutput,
          )}
          model={model}
          providers={providers}
          toolIntent={modelToolIntent || Boolean(folder.trim())}
          onSelect={(target) =>
            requestHotSwap(
              target,
              batonRequest.action,
              batonRequest.messageIndex,
            )
          }
          onManageProviders={() => {
            setBatonRequest(null);
            setProvidersOpen(true);
          }}
          onClose={() => setBatonRequest(null)}
        />
      )}

      {hotSwapPreflight && (
        <HotSwapPreflightSheet
          fromModel={model}
          targetModel={hotSwapPreflight.target.id}
          assessment={hotSwapPreflight.assessment}
          onConfirm={(nativeMode) => {
            const request = hotSwapPreflight;
            setHotSwapPreflight(null);
            commitHotSwap(
              request.target,
              request.action,
              request.messageIndex,
              nativeMode,
              request.selection,
            );
          }}
          onClose={() => setHotSwapPreflight(null)}
        />
      )}

      <Suspense fallback={null}>
        {providersOpen && (
          <ProvidersManager
            onClose={() => {
              setProvidersOpen(false);
              listModelsDetailed(
                useSettings.getState().accountRuntimeEnabled,
              ).then(setModels);
              listProviders().then(setProviders);
            }}
          />
        )}

        {mcpOpen && (
          <McpManager
            onClose={() => {
              setMcpOpen(false);
              void listTools().then(setComposerTools).catch(() => setComposerTools([]));
            }}
          />
        )}

        {attachmentPreview?.dataUrl && (
          <SheetDialog
            title={attachmentPreview.name}
            className="generated-media-dialog"
            overlayClassName="generated-media-overlay"
            testId="quick-summary-attachment-preview"
            onClose={() => setAttachmentPreview(null)}
          >
            <div className="generated-media-toolbar">
              <span>{attachmentPreview.name}</span>
              <button className="icon-btn" type="button" aria-label="Close attachment preview" onClick={() => setAttachmentPreview(null)}>
                <X size={18} />
              </button>
            </div>
            <div className="generated-media-stage">
              <img src={attachmentPreview.dataUrl} alt={attachmentPreview.name} />
            </div>
          </SheetDialog>
        )}

        {memoryOpen && (
          <MemoryManager
            initialNodeId={memoryTarget?.node_id}
            initialScopeKind={memoryTarget?.scope_kind}
            onClose={() => setMemoryOpen(false)}
          />
        )}

        {goalPanelOpen && (
          <GoalPanel
            goal={goal}
            prefillObjective={goalPrefill}
            onSave={(draft) => {
              saveGoalDraft(draft);
              setChatNotice({ tone: "info", message: "Goal saved." });
            }}
            onRun={(draft) => startGoalRun(draft)}
            onPause={() => pauseGoalRun()}
            onDelete={() => deleteGoal()}
            onClose={() => {
              setGoalPrefill(null);
              setGoalPanelOpen(false);
            }}
          />
        )}
      </Suspense>
    </div>
  );
}
