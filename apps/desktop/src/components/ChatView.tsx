import {
  lazy,
  memo,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type MouseEvent,
  type MutableRefObject,
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
  generateMedia,
  getClaudeStatus,
  getOpenCodeStatus,
  getPiStatus,
  getCodexAccount,
  getMobileCompanionStatus,
  getWorkspaceGitStatus,
  getWorkerRun,
  getWorkerDiff,
  getMediaModelSchema,
  getMediaStatus,
  getPreviewAppStatus,
  isClaudeModel,
  accountRuntimeKind,
  isCliPathWarningMessage,
  isCodexModel,
  isOpenCodeModel,
  isPiModel,
  listWorkspaceFiles,
  loadStartupModels,
  listModelsDetailed,
  listMediaModels,
  listProviders,
  listSkills,
  listTools,
  listWorkerRuns,
  MAX_ATTACHMENT_BYTES,
  openArtifactLocation,
  openDiagnosticsFolder,
  openExternalUrl,
  pickAttachmentFiles,
  pollMobileCompanionEvents,
  pollScheduleRunEvents,
  preflightPreviewApp,
  publishMobileThreadSnapshot,
  previewArtifactFile,
  readWorkspaceAttachmentFile,
  restartPreviewApp,
  retryWorkerTask,
  deleteWorkerRun,
  runWorkspaceGitAction,
  saveArtifactFile,
  searchGraphMemory,
  selectSkills,
  setComputerUse,
  setPrivacyMode,
  setWorkspace,
  startPreviewApp,
  startWorkerRun,
  stopChildThread,
  stopPreviewApp,
  stopWorkerRun,
  stopWorker,
  streamAgentRun,
  streamChat,
  streamChildThreadEvents,
  streamWorkerRunEvents,
  streamClaudeRun,
  streamCodexDeviceLogin,
  streamCodexRun,
  streamOpenCodeRun,
  streamPiRun,
  opencodeRuntimeModel,
  piRuntimeModel,
  wireMessageContent,
  mediaProviders,
  type AgentEvent,
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
  type ClaudeRunEvent,
  type CodexLoginEvent,
  type CodexRunEvent,
  type OpenCodeRunEvent,
  type PiRunEvent,
  type MediaGenerationResult,
  type MediaKind,
  type MediaModelSchema,
  type MediaSchemaControl,
  type MobileThreadGroup,
  type MobileThreadSummary,
  type MobileWorkerRunSnapshot,
  type MobileRelayAttachment,
  type MobileRelayEvent,
  type MemoryNotice,
  type ModelInfo,
  type PreviewAppFile,
  type PreviewAppPreflight,
  type PreviewAppStatus,
  type PreviewAppStartOptions,
  type PreviewSurfaceTarget,
  type ReviewComment,
  type ProviderInfo,
  type ReasoningEffort,
  type RunStep,
  type RunTrace,
  type SavedArtifactFile,
  type ScheduleRunEvent,
  type SkillInfo,
  type TokenUsage,
  type ToolInfo,
  type ToolApprovalMode,
  type ThreadEvent,
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
  previewRuntimeBrowserUrl,
  previewRuntimeFiles,
} from "../lib/artifacts";
import {
  artifactOccurrenceKey,
  artifactRevisionChoiceByOccurrence,
  artifactRevisionGroups,
  type ArtifactRevision,
  type ArtifactRevisionChoice,
  type ArtifactRevisionGroup,
} from "../lib/artifactRevisions";
import { hiddenArtifactIdsForMessage } from "../lib/artifactVisibility";
import {
  workerRunReadyForSynthesis,
  workerRunSynthesisId,
} from "../lib/workerRuns";
import {
  assertValidImageAttachment,
  readBrowserAttachmentDataUrl,
} from "../lib/attachmentInput";
import {
  buildEmptyStarterStrip,
  type EmptyStarterStrip,
  type EmptyStarterSuggestionIcon,
} from "../lib/emptyStarterSuggestions";
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
import { reasoningEffortForModel } from "../lib/reasoningEffort";
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
  bestMediaResultUrl,
  defaultMediaAdvanced,
  defaultMediaModel,
  inputWithSchemaControls,
  mediaKindForModelId,
  mediaPollingMaxAttempts,
  mediaPreferenceKey,
  mediaResultContent,
  parseControlValue,
  schemaDefaults,
  shouldPollMediaStatus,
} from "../lib/media";
import { mergeModelListsForPicker, providerOwnsModel } from "../lib/modelPicker";
import { assessHotSwap, nativeRuntimeIsStale, type HotSwapAssessment } from "../lib/hotSwap";
import {
  approvalWaitDuration,
  estimateResponseCostUsd,
  formatResponseMetrics,
  responseMetricsForTurn,
  summarizeMilimUsage,
  summarizeThreadMetricsBreakdown,
  type MilimUsageSummary,
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
  claudeCompactionSummaryRequest,
  codexCompactionSummaryRequest,
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
} from "../lib/turnRuntime";
import { dismissToolApproval, pendingToolApprovals } from "../lib/toolApproval";
import {
  drainQueuedMessages as drainQueuedMessagesFromQueue,
  hasQueuedMessages,
  queuedModelForSession,
} from "../lib/turnQueue";
import {
  claimTurnGeneration,
  releaseTurnGeneration,
  startTurnStream,
} from "../lib/turnStream";
import { checkpointWorkspaceBeforeTurn } from "../lib/turnWorkspace";
import { createChatMessageId } from "../lib/messageIds.js";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage";
import { useSettings, type MediaSettings } from "../settings/store";
import { themeCssVariables } from "../theme/applyTheme";
import { useTheme } from "../theme/store";
import { shortcutLabel, shortcutMatchesEvent } from "../ui/shortcuts";
import { sendMilimNotification } from "../lib/nativeNotifications";
import { createInteractiveChat } from "../lib/newChatCoordinator";
import { isLoopbackProviderEndpoint } from "../lib/providerEndpoint.js";
import { pendingAttentionKey, playInterfaceSound } from "../ui/sounds";
import { DEFAULT_PREVIEW_PANEL_WIDTH, useUiPreferences } from "../ui/store";
import { AgentAvatar } from "./AgentAvatar";
import { Composer } from "./Composer";
import { ComposerSurface } from "./ComposerSurface";
import { ControlBar } from "./ControlBar";
import type { ModelPickerSelection } from "./ModelPicker";
import { GoalPanel, type GoalPanelDraft } from "./GoalPanel";
import {
  ArrowRight,
  Calendar,
  Check,
  Code,
  Copy,
  Eye,
  FileText,
  GitBranch,
  Globe,
  MoreHorizontal,
  Pencil,
  Refresh,
  Sidebar as PanelIcon,
  Trash,
  UserRound,
  X,
} from "./icons";
import { groupSessionsByProjects } from "./Sidebar";
import { InlineMediaControls } from "./InlineMediaControls";
import { GeneratedMedia } from "./GeneratedMedia";
import { WorkersInspector, WorkersSummary } from "./WorkersInspector";
import { AssistantMessage } from "./AssistantMessage";
import { ToolApprovalPrompt } from "./ToolApprovalPrompt";
import { ArtifactList } from "./ArtifactList";
import { CommandPalette, type RuntimeCommand } from "./ChatSearchPopover";
import { useContextMenu, type ContextMenuItem } from "./ContextMenu";
import {
  GitWorkspacePanel,
  type GitPanelDiffRequest,
  type GitPanelView,
} from "./GitPanel";
import { PreviewPanel } from "./PreviewPanel";
import { QuickSummaryPanel } from "./QuickSummaryPanel";
import { RunTimeline } from "./RunTimeline";
import {
  TurnChangesCard,
  turnChangesFromDiff,
  type TurnChanges,
} from "./TurnChangesCard";
import { SheetDialog } from "./SheetDialog";
import { WorkspaceLauncherButton } from "./WorkspaceLauncher";
import { BatonMenu, BatonTargetSheet, HotSwapPreflightSheet } from "./HotSwapDialogs";

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
const Markdown = lazy(() =>
  import("./Markdown").then((mod) => ({ default: mod.Markdown })),
);


const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const EMPTY: ChatMessage[] = [];
const EMPTY_QUEUE: QueuedMessage[] = [];
const EMPTY_CONTEXT_SECTION_IDS: QuickSummarySectionId[] = [];
const NON_EMPTY_USAGE_MESSAGES: ChatMessage[] = [{ role: "user", content: "" }];
const PREVIEW_PANEL_MIN_WIDTH = 360;
const MESSAGE_VIRTUALIZE_AFTER = 80;
const MESSAGE_ESTIMATED_HEIGHT = 152;
const MESSAGE_VIRTUAL_OVERSCAN_PX = 900;
const MESSAGE_ROW_GAP = 12;
const RECENT_THREAD_SWITCHER_CLOSE_MS = 1600;
const previewArtifactCache = new WeakMap<ChatMessage, ChatArtifact[] | null>();

function documentVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

type CompactionSummaryResult = {
  content: string;
  usage?: TokenUsage;
  costUsd?: number;
  finishReason?: string;
};

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
const MEDIA_CONTEXT_MESSAGE_LIMIT = 10;
const MEDIA_CONTEXT_CHAR_LIMIT = 1800;
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

function renderMessageAttachments(attachments?: ChatAttachment[]) {
  if (!attachments?.length) return null;
  return (
    <div className="message-attachments">
      {attachments.map((attachment) => (
        <div
          key={attachment.id}
          className="message-attachment"
          data-testid={`message-attachment-${attachment.id}`}
        >
          {attachment.dataUrl && (
            <img
              className="message-attachment-thumb"
              src={attachment.dataUrl}
              alt={`Attachment preview: ${attachment.name}`}
            />
          )}
          <span className="message-attachment-name">{attachment.name}</span>
          <span className="message-attachment-meta">
            {attachment.mime}
            {attachment.truncated ? " clipped" : ""}
          </span>
        </div>
      ))}
    </div>
  );
}

function renderMessageMedia(results?: MediaGenerationResult[]) {
  if (!results?.length) return null;
  return (
    <div className="message-media-results" data-testid="message-media-results">
      {results.map((result) => {
        const media = result.media[0];
        const key = `${result.provider_id}-${result.id || result.model}-${result.status}`;
        const label = `Generated ${media?.kind ?? "media"} from ${result.model}`;
        return (
          <div
            className={`message-media-preview ${media?.url ? "" : "placeholder"}`}
            data-testid="message-media-result"
            key={key}
          >
            <GeneratedMedia
              item={media}
              alt={label}
              onOpenExternal={(url) => {
                void openExternalUrl(url).catch((error) =>
                  console.warn("failed to open URL", error),
                );
              }}
            />
          </div>
        );
      })}
    </div>
  );
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

function isPreviewAppActive(status: PreviewAppStatus | null): boolean {
  if (typeof status?.active === "boolean") return status.active;
  return (
    Boolean(status?.pid) ||
    status?.status === "staging" ||
    status?.status === "installing" ||
    status?.status === "starting" ||
    status?.status === "running" ||
    status?.status === "stopping"
  );
}

type MessageRowActions = {
  openContextMenu: (
    event: MouseEvent,
    items: ContextMenuItem[],
    label?: string,
  ) => boolean;
  setInspectorTab: (
    sessionId: string,
    tab: "code" | "preview",
  ) => void;
  openWorkers: (runId?: string) => void;
  preparePreviewRuntimeForArtifacts: (
    artifacts?: readonly ChatArtifact[],
  ) => Promise<void>;
  openPreviewArtifact: (
    artifact: ChatArtifact,
    artifacts?: readonly ChatArtifact[],
    previewDeferred?: boolean,
    revision?: ArtifactRevision,
  ) => void;
  artifactRevisionChoice: (
    messageIndex: number,
    artifactIndex: number,
  ) => ArtifactRevisionChoice | undefined;
  executePlan: (messageIndex: number, message: ChatMessage) => void;
  restoreWorkspaceCheckpoint: (
    checkpoint: WorkspaceCheckpoint,
  ) => Promise<void>;
  forkThreadAt: (messageIndex: number) => void;
  setEditing: (messageIndex: number | null) => void;
  deleteMessageAt: (messageIndex: number) => void;
  regenerate: () => void;
  startBaton: (
    action: Exclude<HotSwapAction, "switch">,
    messageIndex: number,
  ) => void;
  undoTurnChanges: (messageIndex: number) => Promise<void>;
  reviewTurnChanges: (
    checkpoint: WorkspaceCheckpoint,
    result: WorkspaceGitActionResult,
  ) => void;
  editResend: (messageIndex: number, text: string) => void;
  editMessageInPlace: (messageIndex: number, text: string) => void;
  approveToolApproval: (messageIndex: number, message: ChatMessage) => void;
  denyToolApproval: (messageIndex: number, message: ChatMessage) => void;
  handleSaveArtifact: (
    messageIndex: number,
    artifact: ChatArtifact,
    options?: {
      overwrite?: boolean;
      path?: string;
      source?: SavedArtifactFile["source"];
    },
    revision?: ArtifactRevision,
  ) => Promise<SavedArtifactFile>;
  handlePreviewArtifact: (
    artifact: ChatArtifact,
    path?: string,
    revision?: ArtifactRevision,
  ) => Promise<ArtifactWritePreview>;
  handleCheckArtifact: (
    saved: SavedArtifactFile,
  ) => Promise<ArtifactFileStatus>;
  handleOpenArtifact: (
    saved: SavedArtifactFile,
    target: ArtifactOpenTarget,
  ) => Promise<void>;
  onOpenSchedules: () => void;
};

type MessageRowProps = {
  activeId: string;
  message: ChatMessage;
  index: number;
  isEditing: boolean;
  isLastAssistant: boolean;
  assistantStreaming: boolean;
  busy: boolean;
  activeMediaTargetPresent: boolean;
  folderIsEmpty: boolean;
  activeRun?: RunTrace | null;
  previewArtifacts?: ChatArtifact[];
  previewAppBusy: "start" | "stop" | "restart" | null;
  previewAppStatus: PreviewAppStatus | null;
  toolApproval: ToolApprovalMode;
  turnChanges?: TurnChanges | null;
  actionsRef: MutableRefObject<MessageRowActions | null>;
};

function MessageRowView({
  activeId,
  message: m,
  index: i,
  isEditing,
  isLastAssistant,
  assistantStreaming,
  busy,
  activeMediaTargetPresent,
  folderIsEmpty,
  activeRun,
  previewArtifacts,
  previewAppBusy,
  previewAppStatus,
  toolApproval,
  turnChanges,
  actionsRef,
}: MessageRowProps) {
  markPerfRender("MessageRow");
  const [copied, setCopied] = useState(false);
  const showModelAvatar = useUiPreferences((state) => state.avatarStyle === "avatar");
  const linkedWorkerRun = useSessions((state) =>
    m.workerRunId
      ? state.workerRuns.find((record) => record.run.id === m.workerRunId)
      : undefined,
  );
  const actions = actionsRef.current;
  const messageIsCompaction = isCompactionCheckpoint(m);
  const isApprovalMessage = Boolean(m.approval);
  const artifactContext = m.artifacts?.length ? m.artifacts : previewArtifacts;
  const openMessagePreview = (
    artifact: ChatArtifact,
    revision?: ArtifactRevision,
  ) => {
    if (!actions) return;
    const artifactIndex =
      m.artifacts?.findIndex((item) => item.id === artifact.id) ?? -1;
    const choice =
      !revision && artifactIndex >= 0
        ? actions.artifactRevisionChoice(i, artifactIndex)
        : undefined;
    actions.setInspectorTab(
      activeId,
      isPreviewableArtifact(revision?.artifact ?? artifact)
        ? "preview"
        : "code",
    );
    actions.openPreviewArtifact(
      artifact,
      artifactContext ?? [artifact],
      assistantStreaming,
      revision ?? choice?.revision,
    );
  };
  const previewArtifactsStreaming =
    assistantStreaming && Boolean(previewArtifacts?.length);
  const hasStreamTranscript = Boolean(m.streamParts?.length);
  const hasAssistantOutput = Boolean(m.content || hasStreamTranscript);
  const metricsLabel = formatResponseMetrics(m.metrics);
  const modelAvatarSeed = m.role === "assistant" ? m.metrics?.model.trim() : "";
  const canExecutePlan =
    m.role === "assistant" &&
    m.plan?.status === "proposed" &&
    !assistantStreaming &&
    Boolean(m.content.trim());
  const runtimeFiles =
    folderIsEmpty && !assistantStreaming
      ? previewRuntimeFiles(m.artifacts)
      : [];
  const canStartRuntime =
    runtimeFiles.length > 0 && hasPreviewPackageJson(runtimeFiles);
  async function copyMessage() {
    if (!navigator.clipboard) return;
    try {
      await navigator.clipboard.writeText(wireMessageContent(m));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      /* clipboard unavailable */
    }
  }
  const openMessageContextMenu = (event: MouseEvent<HTMLDivElement>) => {
    if (!actions) return;
    actions.openContextMenu(
      event,
      [
        ...(canExecutePlan
          ? [
              {
                id: "execute-plan",
                label: "Execute plan",
                icon: <ArrowRight size={13} />,
                action: () => actions.executePlan(i, m),
              },
            ]
          : []),
        ...(m.workspaceCheckpoint
          ? [
              {
                id: "restore-checkpoint",
                label: "Restore workspace checkpoint",
                icon: <Refresh size={13} />,
                disabled: busy,
                action: () =>
                  void actions.restoreWorkspaceCheckpoint(
                    m.workspaceCheckpoint!,
                  ),
              },
            ]
          : []),
        ...(!messageIsCompaction
          ? [
              {
                id: "branch",
                label: "Branch from here",
                icon: <GitBranch size={13} />,
                disabled: busy,
                separatorBefore: true,
                action: () => actions.forkThreadAt(i),
              },
            ]
          : []),
        ...(!isApprovalMessage
          ? [
              {
                id: "copy",
                label: "Copy",
                icon: <Copy size={13} />,
                action: () => void copyMessage(),
              },
            ]
          : []),
        ...(m.role === "user"
          ? [
              {
                id: "edit-resend",
                label: "Edit and resend",
                icon: <Pencil size={13} />,
                disabled: busy,
                action: () => actions.setEditing(i),
              },
            ]
          : []),
        ...(m.role === "assistant" && !messageIsCompaction && !isApprovalMessage
          ? [
              {
                id: "edit",
                label: "Edit message",
                icon: <Pencil size={13} />,
                disabled: busy,
                action: () => actions.setEditing(i),
              },
            ]
          : []),
        ...(!messageIsCompaction
          ? [
              {
                id: "delete",
                label: "Delete message",
                icon: <Trash size={13} />,
                disabled: busy,
                danger: true,
                separatorBefore: true,
                action: () => actions.deleteMessageAt(i),
              },
            ]
          : []),
        ...(isLastAssistant
          ? [
              {
                id: "continue-with",
                label: "Continue with...",
                icon: <ArrowRight size={13} />,
                disabled: busy,
                separatorBefore: true,
                action: () => actions.startBaton("continue", i),
              },
              {
                id: "review-with",
                label: "Review with...",
                icon: <Eye size={13} />,
                disabled: busy,
                action: () => actions.startBaton("review", i),
              },
              {
                id: "retry-with",
                label: "Retry with...",
                icon: <Refresh size={13} />,
                disabled:
                  busy ||
                  (!folderIsEmpty && !m.workspaceCheckpoint && !m.plan),
                action: () => actions.startBaton("retry", i),
              },
              ...(m.workspaceCheckpoint
                ? [
                    {
                      id: "undo-turn",
                      label: "Undo turn changes",
                      icon: <Refresh size={13} />,
                      disabled: busy,
                      action: () => void actions.undoTurnChanges(i),
                    },
                  ]
                : []),
              {
                id: "regenerate",
                label: "Regenerate",
                icon: <Refresh size={13} />,
                disabled: busy,
                action: actions.regenerate,
              },
            ]
          : []),
      ],
      m.role === "assistant" ? "Assistant message" : "User message",
    );
  };

  if (isEditing) {
    return (
      <div className={"msg " + m.role}>
        <MessageEditor
          initial={m.content}
          saveLabel={m.role === "user" ? "Send" : "Save"}
          onCancel={() => actions?.setEditing(null)}
          onSave={(text) =>
            m.role === "user"
              ? actions?.editResend(i, text)
              : actions?.editMessageInPlace(i, text)
          }
        />
      </div>
    );
  }

  return (
    <div
      className={`msg ${m.role}${modelAvatarSeed ? " has-model-avatar" : ""}`}
      data-testid={
        m.role === "assistant" ? "assistant-message" : "user-message"
      }
      onContextMenu={openMessageContextMenu}
    >
      {showModelAvatar && modelAvatarSeed && (
        <AgentAvatar avatar={modelAvatarSeed} className="message-agent-avatar" />
      )}
      <div className="msg-content" dir="auto">
        {m.role === "assistant" ? (
          <>
            {m.run && m.run !== activeRun && <RunTimeline run={m.run} />}
            {!hasStreamTranscript && (
              <MemoryBreadcrumbs memories={m.memories} />
            )}
            {m.approval && (
              <ToolApprovalCard
                approval={m.approval}
                disabled={busy || activeMediaTargetPresent}
                onApprove={() => actions?.approveToolApproval(i, m)}
                onDeny={() => actions?.denyToolApproval(i, m)}
              />
            )}
            {linkedWorkerRun && (
              <button
                className={`worker-run-event ${linkedWorkerRun.run.status}`}
                type="button"
                data-testid="worker-run-event"
                onClick={() => actions?.openWorkers(linkedWorkerRun.run.id)}
              >
                <UserRound size={13} />
                <span>
                  {linkedWorkerRun.run.status === "proposed"
                    ? "Worker plan ready"
                    : linkedWorkerRun.run.status === "running"
                      ? "Workers running"
                      : `Worker run ${linkedWorkerRun.run.status}`}
                </span>
                <small>
                  {linkedWorkerRun.run.tasks.length} task{linkedWorkerRun.run.tasks.length === 1 ? "" : "s"}
                </small>
                <ArrowRight size={12} />
              </button>
            )}
            {(hasAssistantOutput || assistantStreaming) && (
              <AssistantMessage
                content={m.content}
                streamParts={m.streamParts}
                previewArtifacts={previewArtifacts}
                onOpenPreview={openMessagePreview}
                streaming={assistantStreaming}
                previewArtifactsStreaming={previewArtifactsStreaming}
                workDurationMs={m.metrics?.durationMs}
                toolApproval={toolApproval}
              />
            )}
            {renderMessageMedia(m.mediaResults)}
            <AutomationCards
              run={m.run}
              onOpenSchedules={() => actions?.onOpenSchedules()}
            />
            <ArtifactList
              artifacts={m.artifacts}
              currentSessionId={APP_SESSION_ID}
              hiddenArtifactIds={hiddenArtifactIdsForMessage(m, folderIsEmpty)}
              onOpenPreview={openMessagePreview}
              onSaveToWorkspace={(artifact, options, revision) =>
                actions?.handleSaveArtifact(i, artifact, options, revision) ??
                Promise.reject(new Error("message actions unavailable"))
              }
              onPreviewArtifact={(artifact, path, revision) =>
                actions?.handlePreviewArtifact(artifact, path, revision) ??
                Promise.reject(new Error("message actions unavailable"))
              }
              onCheckSavedArtifact={(saved) =>
                actions?.handleCheckArtifact(saved) ??
                Promise.reject(new Error("message actions unavailable"))
              }
              onOpenSavedArtifact={(saved, target) =>
                actions?.handleOpenArtifact(saved, target) ??
                Promise.reject(new Error("message actions unavailable"))
              }
              revisionForArtifact={(artifactIndex) =>
                actions?.artifactRevisionChoice(i, artifactIndex)
              }
              autoSaveArtifacts={toolApproval === "open" && !assistantStreaming}
              storageLabel={folderIsEmpty ? "virtual project" : "folder"}
            />
            {canStartRuntime && (
              <div className="artifact-runtime-actions">
                <button
                  className="msg-act msg-act-text"
                  data-testid="preview-app-start"
                  title="Review preview app commands before running"
                  disabled={
                    previewAppBusy != null ||
                    isPreviewAppActive(previewAppStatus)
                  }
                  onClick={() =>
                    void actions?.preparePreviewRuntimeForArtifacts(m.artifacts)
                  }
                >
                  <Globe size={13} />
                  <span>
                    {previewAppBusy === "start"
                      ? "Inspecting preview..."
                      : "Inspect preview app"}
                  </span>
                </button>
              </div>
            )}
            {turnChanges && (
              <TurnChangesCard
                sections={turnChanges.sections}
                stats={turnChanges.stats}
                onUndo={() => void actions?.undoTurnChanges(i)}
                onReview={() =>
                  actions?.reviewTurnChanges(
                    turnChanges.checkpoint,
                    turnChanges.result,
                  )
                }
              />
            )}
            {metricsLabel && (
              <div className="response-metrics">{metricsLabel}</div>
            )}
          </>
        ) : (
          <>
            {m.content && (
              <Suspense fallback={<span>{m.content}</span>}>
                <Markdown
                  content={m.content}
                  highlight={false}
                  collapseArtifacts={false}
                />
              </Suspense>
            )}
            {renderMessageAttachments(m.attachments)}
            {m.reviewComments?.length ? (
              <div className="message-review-count">
                {m.reviewComments.length} review comment{m.reviewComments.length === 1 ? "" : "s"}
              </div>
            ) : null}
          </>
        )}
      </div>
      <div className="msg-actions">
        {canExecutePlan && (
          <button
            className="msg-act msg-act-text"
            data-testid="execute-plan"
            title="Execute plan"
            onClick={() => actions?.executePlan(i, m)}
          >
            <ArrowRight size={13} />
            <span>Execute plan</span>
          </button>
        )}
        {m.workspaceCheckpoint && !busy && !turnChanges && (
          <button
            className="msg-act"
            title="Restore workspace to before this turn"
            aria-label="Restore workspace to before this turn"
            onClick={() =>
              void actions?.restoreWorkspaceCheckpoint(m.workspaceCheckpoint!)
            }
          >
            <Refresh size={13} />
          </button>
        )}
        {!messageIsCompaction && !busy && (
          <button
            className="msg-act"
            title="Branch from here"
            aria-label="Branch from here"
            onClick={() => actions?.forkThreadAt(i)}
          >
            <GitBranch size={13} />
          </button>
        )}
        {!isApprovalMessage && (
          <button
            className="msg-act"
            data-testid="message-copy"
            title={copied ? "Copied" : "Copy"}
            aria-label={copied ? "Copied" : "Copy message"}
            aria-live="polite"
            onClick={() => void copyMessage()}
          >
            {copied ? <Check size={13} /> : <Copy size={13} />}
          </button>
        )}
        {m.role === "user" && !busy && (
          <button
            className="msg-act"
            title="Edit & resend"
            onClick={() => actions?.setEditing(i)}
          >
            <Pencil size={13} />
          </button>
        )}
        {m.role === "assistant" &&
          !messageIsCompaction &&
          !isApprovalMessage &&
          !busy && (
            <button
              className="msg-act"
              title="Edit message"
              aria-label="Edit message"
              onClick={() => actions?.setEditing(i)}
            >
              <Pencil size={13} />
            </button>
          )}
        {!messageIsCompaction && !busy && (
          <button
            className="msg-act danger"
            title="Delete message"
            aria-label="Delete message"
            onClick={() => actions?.deleteMessageAt(i)}
          >
            <Trash size={13} />
          </button>
        )}
        {isLastAssistant && !busy && (
          <BatonMenu
            retryDisabled={!folderIsEmpty && !m.workspaceCheckpoint && !m.plan}
            onAction={(action) => actions?.startBaton(action, i)}
          />
        )}
        {isLastAssistant && !busy && (
          <button
            className="msg-act"
            title="Regenerate"
            onClick={actions?.regenerate}
          >
            <Refresh size={13} />
          </button>
        )}
      </div>
    </div>
  );
}

const MessageRow = memo(
  MessageRowView,
  (prev, next) =>
    prev.activeId === next.activeId &&
    prev.message === next.message &&
    prev.index === next.index &&
    prev.isEditing === next.isEditing &&
    prev.isLastAssistant === next.isLastAssistant &&
    prev.assistantStreaming === next.assistantStreaming &&
    prev.busy === next.busy &&
    prev.activeMediaTargetPresent === next.activeMediaTargetPresent &&
    prev.folderIsEmpty === next.folderIsEmpty &&
    prev.activeRun === next.activeRun &&
    prev.previewArtifacts === next.previewArtifacts &&
    prev.previewAppBusy === next.previewAppBusy &&
    prev.previewAppStatus === next.previewAppStatus &&
    prev.toolApproval === next.toolApproval &&
    prev.turnChanges === next.turnChanges &&
    prev.actionsRef === next.actionsRef,
);

type VirtualMessageItem = {
  index: number;
  message: ChatMessage;
  top: number;
};

type VirtualMessageWindow = {
  virtualized: boolean;
  items: VirtualMessageItem[];
  totalHeight: number;
};

function virtualIndexAt(offsets: number[], value: number): number {
  let low = 0;
  let high = offsets.length - 1;
  while (low < high) {
    const mid = Math.floor((low + high) / 2);
    if (offsets[mid] < value) low = mid + 1;
    else high = mid;
  }
  return Math.max(0, low - 1);
}

function virtualMessageWindow(
  messages: ChatMessage[],
  heights: readonly number[],
  scrollTop: number,
  viewportHeight: number,
): VirtualMessageWindow {
  const visible = messages.flatMap((message, index) =>
    workerRunSynthesisId(message) ? [] : [{ index, message, top: 0 }],
  );
  if (visible.length <= MESSAGE_VIRTUALIZE_AFTER) {
    return {
      virtualized: false,
      totalHeight: 0,
      items: visible,
    };
  }
  const offsets = new Array<number>(visible.length + 1);
  offsets[0] = 0;
  for (let i = 0; i < visible.length; i += 1) {
    offsets[i + 1] =
      offsets[i] + (heights[visible[i].index] || MESSAGE_ESTIMATED_HEIGHT);
  }
  const start = virtualIndexAt(
    offsets,
    Math.max(0, scrollTop - MESSAGE_VIRTUAL_OVERSCAN_PX),
  );
  const end = Math.min(
    visible.length - 1,
    virtualIndexAt(
      offsets,
      scrollTop + viewportHeight + MESSAGE_VIRTUAL_OVERSCAN_PX,
    ) + 1,
  );
  const items: VirtualMessageItem[] = [];
  for (let index = start; index <= end; index += 1) {
    items.push({ ...visible[index], top: offsets[index] });
  }
  return {
    virtualized: true,
    items,
    totalHeight: offsets[visible.length],
  };
}

function MessageVirtualRow({
  index,
  top,
  measure,
  children,
}: {
  index: number;
  top: number;
  measure: (index: number, height: number) => void;
  children: ReactNode;
}) {
  const rowRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const row = rowRef.current;
    if (!row) return;
    const update = () =>
      measure(index, row.getBoundingClientRect().height + MESSAGE_ROW_GAP);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(row);
    return () => observer.disconnect();
  }, [index, measure, children]);
  return (
    <div
      ref={rowRef}
      className="message-virtual-row"
      style={{ top: `${top}px` }}
    >
      {children}
    </div>
  );
}

function previewRuntimeText(value?: string | null): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function previewRuntimeFromStatus(
  status: PreviewAppStatus,
  previous?: SessionPreviewRuntime,
): SessionPreviewRuntime {
  const state = previewRuntimeText(status.status) ?? "idle";
  const url =
    previewRuntimeText(status.url) ??
    (status.active === true ||
    state === "staging" ||
    state === "installing" ||
    state === "starting" ||
    state === "stopping" ||
    state === "error"
      ? previous?.url
      : undefined);
  return {
    status: state,
    cwd: previewRuntimeText(status.cwd),
    url,
    pid:
      typeof status.pid === "number" && Number.isFinite(status.pid)
        ? status.pid
        : undefined,
    command: previewRuntimeText(status.command),
    message: previewRuntimeText(status.message),
    active: status.active,
    ready: status.ready,
    managed: status.managed,
    runId: previewRuntimeText(status.run_id),
    error: status.error ?? undefined,
    preflight: status.preflight ?? undefined,
  };
}

function previewStatusFromRuntime(
  threadId: string,
  runtime?: SessionPreviewRuntime,
): PreviewAppStatus | null {
  if (!runtime) return null;
  return {
    thread_id: threadId,
    status: runtime.status,
    cwd: runtime.cwd ?? "",
    url: runtime.url ?? null,
    pid: runtime.pid ?? null,
    command: runtime.command ?? null,
    message: runtime.message ?? null,
    active: runtime.active,
    ready: runtime.ready,
    managed: runtime.managed,
    run_id: runtime.runId ?? null,
    updated_at: runtime.updatedAt,
    error: runtime.error ?? null,
    preflight: runtime.preflight ?? null,
    logs: [],
  };
}

function previewStatusMatchesFolder(
  status: PreviewAppStatus | null,
  folder: string,
): boolean {
  const cwd = previewRuntimeText(folder);
  return !cwd || previewRuntimeFoldersEqual(status?.cwd, cwd);
}

function previewIdleStatus(
  threadId: string,
  folder: string,
): PreviewAppStatus {
  const cwd = previewRuntimeText(folder) ?? "";
  return {
    thread_id: threadId,
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

function MessageEditor({
  initial,
  onSave,
  onCancel,
  saveLabel = "Send",
}: {
  initial: string;
  onSave: (t: string) => void;
  onCancel: () => void;
  saveLabel?: string;
}) {
  const [v, setV] = useState(initial);
  return (
    <div className="msg-editor">
      <textarea
        className="msg-edit-input"
        value={v}
        autoFocus
        onChange={(e) => setV(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onCancel();
          if (shortcutMatchesEvent("Mod+Enter", e)) onSave(v);
        }}
      />
      <div className="msg-edit-actions">
        <button className="btn-ghost" onClick={onCancel}>
          Cancel
        </button>
        <button className="btn-accent" onClick={() => onSave(v)}>
          {saveLabel}
        </button>
      </div>
    </div>
  );
}

function MemoryBreadcrumbs({ memories }: { memories?: MemoryNotice[] }) {
  if (!memories?.length) return null;
  return (
    <div
      className="memory-breadcrumbs"
      data-testid="memory-breadcrumbs"
      aria-label="Registered memories"
    >
      {memories.map((memory) => (
        <span
          className="memory-crumb"
          key={memory.id}
          title={`${memory.scope_label}: ${memory.summary}`}
        >
          Remembered in {memory.scope_kind}: {memory.summary}
        </span>
      ))}
    </div>
  );
}

type AutomationCard = {
  id: string;
  name: string;
  cron: string;
  enabled: boolean;
  operation: "created" | "updated";
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function scheduleFromToolResult(step: RunStep): AutomationCard | null {
  if (step.name !== "schedule_create" && step.name !== "schedule_update")
    return null;
  const root = asRecord(step.result);
  const schedule = asRecord(root?.schedule);
  if (!schedule) return null;
  const id = schedule?.id;
  const name = schedule?.name;
  const cron = schedule?.cron;
  if (
    typeof id !== "string" ||
    typeof name !== "string" ||
    typeof cron !== "string"
  )
    return null;
  return {
    id,
    name,
    cron,
    enabled: schedule.enabled !== false,
    operation: step.name === "schedule_create" ? "created" : "updated",
  };
}

function automationCardsFromRun(run?: RunTrace): AutomationCard[] {
  const cards: AutomationCard[] = [];
  const seen = new Set<string>();
  for (const step of run?.steps ?? []) {
    const card = scheduleFromToolResult(step);
    if (!card || seen.has(card.id)) continue;
    seen.add(card.id);
    cards.push(card);
  }
  return cards;
}

function plural(value: number, unit: string): string {
  return `${value} ${unit}${value === 1 ? "" : "s"}`;
}

function describeScheduleCron(cron: string): string {
  const parts = cron.trim().replace(/\s+/g, " ").split(" ");
  if (parts.length !== 6) return cron;
  const [sec, min, hour, day, month, dow] = parts;
  const minuteInterval = min.match(/^\*\/(\d+)$/);
  const secondInterval = sec.match(/^\*\/(\d+)$/);
  const daily = day === "*" && month === "*";
  if (daily && dow === "*" && hour === "*" && sec === "0" && minuteInterval) {
    return `Every ${plural(Number(minuteInterval[1]), "minute")}`;
  }
  if (daily && dow === "*" && hour === "*" && min === "0" && sec === "0") {
    return "Hourly";
  }
  if (daily && dow === "*" && hour === "*" && min === "*" && secondInterval) {
    return `Every ${plural(Number(secondInterval[1]), "second")}`;
  }
  if (daily && sec === "0" && /^\d+$/.test(min) && /^\d+$/.test(hour)) {
    const time = `${hour.padStart(2, "0")}:${min.padStart(2, "0")}`;
    return dow === "*" ? `Daily at ${time}` : `Runs at ${time}`;
  }
  return cron;
}

function AutomationCards({
  run,
  onOpenSchedules,
}: {
  run?: RunTrace;
  onOpenSchedules: () => void;
}) {
  const cards = automationCardsFromRun(run);
  if (cards.length === 0) return null;
  return (
    <div
      className="automation-cards"
      data-testid="automation-cards"
      aria-label="Automations"
    >
      {cards.map((card) => (
        <div
          className="automation-card"
          key={card.id}
          data-testid="automation-card"
        >
          <div className="automation-card-icon" aria-hidden="true">
            <Calendar size={16} />
          </div>
          <div className="automation-card-copy">
            <div className="automation-card-title">
              <span>{card.name}</span>
              <span
                className={
                  "automation-card-status " +
                  (card.enabled ? "active" : "paused")
                }
              >
                {card.enabled ? "Active" : "Paused"}
              </span>
            </div>
            <div className="automation-card-meta">
              Automation {card.operation} - {describeScheduleCron(card.cron)}
            </div>
          </div>
          <button
            className="automation-card-open"
            type="button"
            onClick={onOpenSchedules}
          >
            Open
          </button>
        </div>
      ))}
    </div>
  );
}

function MilimUsageRidgeline({ usage }: { usage: MilimUsageSummary }) {
  const months = visibleUsageMonths(usage);
  const width = 440;
  const gradientId = "usage-ridge-fill-gradient";
  const amplitude = usage.hasUsage ? 40 : 0;
  const lineSpacing = 17;
  const lineWidth = 1.35;
  const topPad = amplitude + 8;
  const height = topPad + lineSpacing * (months.length - 1) + 10;
  const maxValue = Math.max(1, ...months.flatMap((month) => month.days));
  const ridges = months.map((month, index) => {
    const base = topPad + index * lineSpacing;
    const line = ridgePath(month.days, base, amplitude, width, maxValue);
    const empty = month.days.every((value) => value === 0);
    const depth =
      months.length <= 1 ? 1 : index / Math.max(1, months.length - 1);
    return {
      base,
      closed: `${line} L${width},${base} L0,${base} Z`,
      depth,
      empty,
      fillOpacity: empty ? 0 : 0.18 + depth * 0.18,
      index,
      key: month.key,
      line,
      lineOpacity: empty ? 0.16 : 0.5 + depth * 0.22,
    };
  });
  const activeRidges = ridges.filter((ridge) => !ridge.empty);
  const drawRidges = [
    ...ridges.filter((ridge) => ridge.empty),
    ...ridges.filter((ridge) => !ridge.empty),
  ];
  const blockersForRidge = (ridge: (typeof ridges)[number]) =>
    ridge.empty
      ? activeRidges
      : activeRidges.filter((blocker) => blocker.index > ridge.index);
  const maskIdForRidge = (ridge: (typeof ridges)[number]) =>
    `usage-ridge-occlusion-${ridge.index}`;

  return (
    <section
      className="usage-empty-panel"
      data-testid="empty-usage-ridgeline"
      aria-label="Milim usage"
    >
      <svg
        className="usage-ridgeline"
        role="img"
        aria-label="Monthly ridgeline chart of local thread activity"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
            <stop
              offset="0%"
              stopColor="var(--accent-light)"
              stopOpacity="0.64"
            />
            <stop offset="54%" stopColor="var(--accent)" stopOpacity="0.28" />
            <stop
              offset="100%"
              stopColor="var(--panel-bg)"
              stopOpacity="0"
            />
          </linearGradient>
          {ridges.map((ridge) => {
            const blockers = blockersForRidge(ridge);
            if (!blockers.length) return null;
            return (
              <mask
                id={maskIdForRidge(ridge)}
                key={ridge.key}
                maskUnits="userSpaceOnUse"
              >
                <rect width={width} height={height} fill="white" />
                {blockers.map((blocker) => (
                  <path key={blocker.key} d={blocker.closed} fill="black" />
                ))}
              </mask>
            );
          })}
        </defs>
        {drawRidges.map((ridge) => {
          const blockers = blockersForRidge(ridge);
          return (
            <g
              key={ridge.key}
              className="usage-ridge-row"
              mask={
                blockers.length ? `url(#${maskIdForRidge(ridge)})` : undefined
              }
            >
              <path
                className="usage-ridge-fill"
                d={ridge.closed}
                style={{ opacity: ridge.fillOpacity }}
                data-empty={ridge.empty || undefined}
              />
              <path
                className="usage-ridge-line"
                d={ridge.line}
                style={{ opacity: ridge.lineOpacity }}
                strokeWidth={lineWidth}
                data-empty={ridge.empty || undefined}
              />
            </g>
          );
        })}
      </svg>
      <div className="usage-empty-footer">
        <div className="usage-empty-metrics">
          {usage.metrics.map((metric) => (
            <div className="usage-empty-metric" key={metric.label}>
              <span className="usage-empty-metric-value">{metric.value}</span>
              <span className="usage-empty-metric-label">{metric.label}</span>
            </div>
          ))}
        </div>
        <div className="usage-empty-latest">
          {usage.months[usage.months.length - 1]?.label}
        </div>
      </div>
    </section>
  );
}

function visibleUsageMonths(
  usage: MilimUsageSummary,
): MilimUsageSummary["months"] {
  const indexed = usage.months.map((month, index) => ({ month, index }));
  const active = indexed.filter(({ month }) =>
    month.days.some((value) => value > 0),
  );
  if (active.length === 0) return usage.months.slice(-3);

  const empty = indexed.filter(({ month }) =>
    month.days.every((value) => value === 0),
  );
  const selected = new Set([
    ...active.map(({ index }) => index),
    ...empty.slice(-Math.max(0, 3 - active.length)).map(({ index }) => index),
  ]);
  return indexed
    .filter(({ index }) => selected.has(index))
    .map(({ month }) => month);
}

function ridgePath(
  values: number[],
  base: number,
  amplitude: number,
  width: number,
  maxValue: number,
): string {
  const points = values.map((value, index) => {
    const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
    const y = base - (value / maxValue) * amplitude;
    return [x, y];
  });
  let line = `M${points[0][0]},${points[0][1]}`;
  for (let index = 1; index < points.length; index += 1) {
    const [x0, y0] = points[index - 1];
    const [x1, y1] = points[index];
    const midX = (x0 + x1) / 2;
    line += ` C${midX},${y0} ${midX},${y1} ${x1},${y1}`;
  }
  return line;
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

function queuedAttachmentLabel(count: number): string {
  return count === 1 ? "1 attachment" : `${count} attachments`;
}

type QueuedDropTarget = {
  id: string;
  position: "before" | "after";
};

type QueuedPointerDrag = {
  id: string;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
  source: HTMLElement;
  captureTarget: HTMLButtonElement;
};

const QUEUED_DRAG_THRESHOLD = 4;

function QueuedMessageTray({
  items,
  busy,
  canActivate,
  interruptingMessageId,
  openContextMenu,
  onActivate,
  onEdit,
  onMove,
  onRemove,
}: {
  items: QueuedMessage[];
  busy: boolean;
  canActivate: boolean;
  interruptingMessageId?: string;
  openContextMenu: (
    event: MouseEvent,
    items: ContextMenuItem[],
    label?: string,
  ) => boolean;
  onActivate: (item: QueuedMessage) => void;
  onEdit: (item: QueuedMessage) => void;
  onMove: (
    messageId: string,
    targetId: string,
    position: "before" | "after",
  ) => void;
  onRemove: (id: string) => void;
}) {
  const pointerDragRef = useRef<QueuedPointerDrag | null>(null);
  const dropTargetRef = useRef<QueuedDropTarget | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<QueuedDropTarget | null>(null);
  const [reorderStatus, setReorderStatus] = useState("");

  if (items.length === 0) return null;

  function setQueuedDropTarget(target: QueuedDropTarget | null) {
    dropTargetRef.current = target;
    setDropTarget(target);
  }

  function clearQueuedDrag() {
    const drag = pointerDragRef.current;
    if (drag) {
      drag.source.style.removeProperty("pointer-events");
      drag.source.style.removeProperty("translate");
      drag.source.style.removeProperty("will-change");
      if (drag.captureTarget.hasPointerCapture(drag.pointerId)) {
        drag.captureTarget.releasePointerCapture(drag.pointerId);
      }
    }
    pointerDragRef.current = null;
    setDraggingId(null);
    setQueuedDropTarget(null);
  }

  function dropTargetAt(clientX: number, clientY: number, sourceId: string) {
    const element = document.elementFromPoint(clientX, clientY);
    const row =
      element instanceof Element
        ? element.closest<HTMLElement>("[data-queued-message-id]")
        : null;
    const id = row?.dataset.queuedMessageId;
    if (!row || !id || id === sourceId) return null;
    const rect = row.getBoundingClientRect();
    return {
      id,
      position: clientY > rect.top + rect.height / 2 ? "after" : "before",
    } as QueuedDropTarget;
  }

  function startQueuedDrag(
    event: ReactPointerEvent<HTMLButtonElement>,
    id: string,
  ) {
    if (
      event.button !== 0 ||
      items.length < 2 ||
      Boolean(interruptingMessageId)
    )
      return;
    const source = event.currentTarget.closest<HTMLElement>(
      "[data-queued-message-id]",
    );
    if (!source) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerDragRef.current = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
      source,
      captureTarget: event.currentTarget,
    };
  }

  function moveQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const moved = Math.hypot(
      event.clientX - drag.startX,
      event.clientY - drag.startY,
    );
    if (!drag.active && moved < QUEUED_DRAG_THRESHOLD) return;
    if (!drag.active) {
      drag.active = true;
      drag.source.style.pointerEvents = "none";
      drag.source.style.willChange = "translate";
      setDraggingId(drag.id);
    }
    event.preventDefault();
    drag.source.style.translate = `0 ${event.clientY - drag.startY}px`;
    setQueuedDropTarget(dropTargetAt(event.clientX, event.clientY, drag.id));
  }

  function endQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (drag.active) {
      event.preventDefault();
      const target =
        dropTargetAt(event.clientX, event.clientY, drag.id) ??
        dropTargetRef.current;
      if (target) {
        onMove(drag.id, target.id, target.position);
        const nextItems = items.filter((item) => item.id !== drag.id);
        const targetIndex = nextItems.findIndex(
          (item) => item.id === target.id,
        );
        const nextIndex =
          targetIndex + (target.position === "after" ? 1 : 0) + 1;
        setReorderStatus(
          `Queued message moved to position ${nextIndex} of ${items.length}.`,
        );
      }
    }
    clearQueuedDrag();
  }

  function cancelQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    clearQueuedDrag();
  }

  function moveQueuedWithKeyboard(
    event: KeyboardEvent<HTMLButtonElement>,
    item: QueuedMessage,
    index: number,
  ) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const targetIndex = index + (event.key === "ArrowUp" ? -1 : 1);
    const target = items[targetIndex];
    if (!target) return;
    onMove(
      item.id,
      target.id,
      event.key === "ArrowUp" ? "before" : "after",
    );
    setReorderStatus(
      `Queued message moved to position ${targetIndex + 1} of ${items.length}.`,
    );
  }

  return (
    <div className="queued-tray" data-testid="queued-message-tray">
      <div className="queued-list">
        {items.map((item, index) => {
          const text = item.content.trim();
          const attachmentCount = item.attachments?.length ?? 0;
          const rowDrop = dropTarget?.id === item.id ? dropTarget : null;
          const interrupting = interruptingMessageId === item.id;
          return (
            <div
              className={`queued-item${draggingId === item.id ? " dragging" : ""}${rowDrop ? ` drag-over drop-${rowDrop.position}` : ""}`}
              data-testid="queued-message"
              data-queued-message-id={item.id}
              key={item.id}
            >
              <button
                className="queued-drag-handle"
                type="button"
                aria-label={`Reorder queued message ${index + 1} of ${items.length}`}
                disabled={items.length < 2 || Boolean(interruptingMessageId)}
                onPointerDown={(event) => startQueuedDrag(event, item.id)}
                onPointerMove={moveQueuedDrag}
                onPointerUp={endQueuedDrag}
                onPointerCancel={cancelQueuedDrag}
                onKeyDown={(event) =>
                  moveQueuedWithKeyboard(event, item, index)
                }
              />
              <span
                className="queued-copy"
                title={text || queuedAttachmentLabel(attachmentCount)}
              >
                {text || "Attached files"}
              </span>
              {attachmentCount > 0 && (
                <span className="queued-meta">
                  {queuedAttachmentLabel(attachmentCount)}
                </span>
              )}
              <button
                className="queued-activate"
                type="button"
                title={
                  canActivate
                    ? busy
                      ? "Interrupt the current response and run this message"
                      : "Run this queued message next"
                    : "Choose a chat model to run queued messages"
                }
                disabled={!canActivate || Boolean(interruptingMessageId)}
                onClick={() => onActivate(item)}
              >
                <ArrowRight size={12} />
                <span>
                  {interrupting ? "Interrupting..." : busy ? "Interrupt" : "Run"}
                </span>
              </button>
              <button
                className="queued-action"
                type="button"
                title="Remove queued message"
                aria-label="Remove queued message"
                disabled={Boolean(interruptingMessageId)}
                onClick={() => onRemove(item.id)}
              >
                <Trash size={12} />
              </button>
              <button
                className="queued-action"
                type="button"
                title="More queued message actions"
                aria-label="More queued message actions"
                disabled={Boolean(interruptingMessageId)}
                onClick={(event) =>
                  openContextMenu(
                    event,
                    [
                      {
                        id: "edit",
                        label: "Edit queued message",
                        icon: <Pencil size={13} />,
                        action: () => onEdit(item),
                      },
                    ],
                    "Queued message actions",
                  )
                }
              >
                <MoreHorizontal size={13} />
              </button>
            </div>
          );
        })}
      </div>
      <span className="queued-reorder-status" role="status" aria-live="polite">
        {reorderStatus}
      </span>
    </div>
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

function toolApprovalCardTitle(approval: ChatApprovalRequest): string {
  if (approval.kind === "claude_session_recovery")
    return "Claude session recovery";
  if (approval.scope === "goal") return "Goal tool access";
  return "Tool access request";
}

function toolApprovalCardDetail(approval: ChatApprovalRequest): string {
  const model = approval.model ? ` for ${approval.model}` : "";
  if (approval.kind === "claude_session_recovery") {
    if (approval.status === "approved")
      return `Approved Claude session recovery${model}.`;
    if (approval.status === "denied")
      return `Canceled Claude session recovery${model}.`;
    return (
      approval.detail ||
      "This Claude session appears to be in use by another Claude CLI process. Milim can try to stop the matching local Claude process and retry, or you can cancel and resume manually."
    );
  }
  if (approval.status === "approved") return `Approved${model}.`;
  if (approval.status === "denied") return `Denied${model}.`;
  return approval.scope === "goal"
    ? `Allow this goal run to use tools${model}.`
    : `Allow this reply to use tools${model}.`;
}

function ToolApprovalCard({
  approval,
  disabled,
  onApprove,
  onDeny,
}: {
  approval: ChatApprovalRequest;
  disabled: boolean;
  onApprove: () => void;
  onDeny: () => void;
}) {
  return (
    <div
      className={`approval-card ${approval.status}`}
      data-testid="tool-approval-card"
    >
      <div className="approval-copy">
        <div className="approval-title">{toolApprovalCardTitle(approval)}</div>
        <div className="approval-detail">
          {toolApprovalCardDetail(approval)}
        </div>
      </div>
      {approval.status === "pending" && (
        <div className="approval-actions">
          <button
            className="approval-btn approve"
            data-testid="approve-tools"
            type="button"
            title="Approve tool access"
            onClick={onApprove}
            disabled={disabled}
          >
            <Check size={13} />
            <span>Approve</span>
          </button>
          <button
            className="approval-btn deny"
            data-testid="deny-tools"
            type="button"
            title="Deny tool access"
            onClick={onDeny}
            disabled={disabled}
          >
            <X size={13} />
            <span>Deny</span>
          </button>
        </div>
      )}
    </div>
  );
}

type GoalLoopState = {
  sessionId: string;
  stopped: boolean;
  decisionController?: AbortController;
};

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

type ActiveMediaTarget = {
  provider: ProviderInfo;
  model: string;
  kind: MediaKind;
  supportedKinds: MediaKind[];
};

type MediaProviderCatalog = Record<
  string,
  Partial<Record<MediaKind, string[]>>
>;

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

function addMediaCandidate(
  candidates: Map<string, Set<MediaKind>>,
  model: string | undefined,
  fallbackKind: MediaKind,
  force = false,
): void {
  const trimmed = model?.trim();
  if (!trimmed) return;
  const kind = mediaKindForModelId(trimmed);
  if (!kind && !force) return;
  const kinds = candidates.get(trimmed) ?? new Set<MediaKind>();
  kinds.add(kind ?? fallbackKind);
  candidates.set(trimmed, kinds);
}

function mediaCandidatesForProvider(
  provider: ProviderInfo,
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): Map<string, Set<MediaKind>> {
  const candidates = new Map<string, Set<MediaKind>>();
  addMediaCandidate(candidates, defaultMediaModel(provider), "image", true);
  addMediaCandidate(
    candidates,
    settings.modelByProvider[provider.id],
    "image",
    true,
  );
  for (const id of settings.favoriteModelIdsByProvider[provider.id] ?? []) {
    addMediaCandidate(candidates, id, "image", true);
  }
  for (const id of provider.models ?? []) {
    addMediaCandidate(candidates, id, "image");
  }
  for (const kind of ["image", "video", "music"] as MediaKind[]) {
    for (const id of catalog[provider.id]?.[kind] ?? []) {
      addMediaCandidate(candidates, id, kind, true);
    }
  }
  return candidates;
}

function mediaModelsForPicker(
  providers: ProviderInfo[],
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): ModelInfo[] {
  return providers.flatMap((provider) =>
    Array.from(
      mediaCandidatesForProvider(provider, settings, catalog),
      ([id, kinds]) => ({
        id,
        owned_by: `${provider.name} media`,
        capabilities: {
          imageOutput: kinds.has("image"),
          videoOutput: kinds.has("video"),
          musicOutput: kinds.has("music"),
        },
      }),
    ),
  );
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

function resolveActiveMediaTarget(
  model: string,
  providers: ProviderInfo[],
  settings: MediaSettings,
  catalog: MediaProviderCatalog,
): ActiveMediaTarget | null {
  const selected = model.trim();
  if (!selected) return null;
  for (const provider of providers) {
    const candidates = mediaCandidatesForProvider(provider, settings, catalog);
    const kinds = candidates.get(selected);
    if (kinds?.size) {
      const supportedKinds = Array.from(kinds);
      return {
        provider,
        model: selected,
        kind: supportedKinds[0],
        supportedKinds,
      };
    }
  }
  return null;
}

function updateMediaMessage(
  sessionId: string,
  requestId: string,
  patch: Partial<ChatMessage>,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.map((message) =>
    message.mediaRequestId === requestId ? { ...message, ...patch } : message,
  );
  store.setMessages(sessionId, messages, { autoTitle: false });
}

function replaceMediaResult(
  sessionId: string,
  requestId: string,
  result: MediaGenerationResult,
): void {
  const store = useSessions.getState();
  const session = store.sessions.find((item) => item.id === sessionId);
  if (!session) return;
  const messages = session.messages.map((message) => {
    if (message.mediaRequestId !== requestId) return message;
    const current = message.mediaResults ?? [];
    const index = current.findIndex(
      (item) =>
        item.provider_id === result.provider_id && item.id === result.id,
    );
    const mediaResults =
      index >= 0
        ? current.map((item, itemIndex) =>
            itemIndex === index ? { ...item, ...result } : item,
          )
        : [result, ...current];
    return {
      ...message,
      content: bestMediaResultUrl(result) ? "" : mediaResultContent(result),
      mediaResults,
    };
  });
  store.setMessages(sessionId, messages, { autoTitle: false });
}

function codexImageMediaResult(
  ev: Extract<CodexRunEvent, { type: "image" }>,
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

function mediaContextLine(message: ChatMessage): string | null {
  const text = message.content.trim();
  if (message.role === "user") {
    return text ? `User: ${compactText(text, 420)}` : null;
  }
  if (message.mediaResults?.length) {
    const summaries = message.mediaResults.slice(0, 2).map((result) => {
      const kind = String(result.media[0]?.kind ?? result.kind ?? "media");
      const status = result.status.trim() || "submitted";
      return bestMediaResultUrl(result)
        ? `generated ${kind} (${status})`
        : `${kind} generation ${status}`;
    });
    return summaries.length ? `Assistant: ${summaries.join(", ")}.` : null;
  }
  if (!text || text.startsWith("Generating ")) return null;
  return `Assistant: ${compactText(text, 420)}`;
}

function boundedMediaContextLines(messages: ChatMessage[]): string[] {
  const lines = messages
    .map(mediaContextLine)
    .filter((line): line is string => Boolean(line))
    .slice(-MEDIA_CONTEXT_MESSAGE_LIMIT);
  const selected: string[] = [];
  let chars = 0;
  for (const line of lines.slice().reverse()) {
    const nextChars = chars + line.length + 1;
    if (selected.length && nextChars > MEDIA_CONTEXT_CHAR_LIMIT) break;
    selected.push(line);
    chars = nextChars;
  }
  return selected.reverse();
}

function mediaPromptWithHistory(
  baseMessages: ChatMessage[],
  currentPrompt: string,
): string {
  const context = boundedMediaContextLines(baseMessages);
  if (!context.length) return currentPrompt;
  return [
    "Use the recent chat context only to resolve references and maintain continuity. Create the latest requested media.",
    "",
    "Recent chat:",
    ...context,
    "",
    `Latest request: ${currentPrompt}`,
  ].join("\n");
}

export function ChatView({
  onManageAgents,
  onOpenSchedules,
  onOpenSettings,
  composerDraft,
  gitPanelRequest = null,
  mcpManagerRequest = 0,
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
  onComposerDraftConsumed?: (id: number) => void;
  skillsRevision?: number;
}) {
  markPerfRender("ChatView");
  const { openContextMenu } = useContextMenu();
  const messageRowActionsRef = useRef<MessageRowActions | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [input, setInputState] = useState(() =>
    getSessionComposerDraft(useSessions.getState().activeId),
  );
  const [providersOpen, setProvidersOpen] = useState(false);
  const [mcpOpen, setMcpOpen] = useState(false);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [memoryTarget, setMemoryTarget] = useState<MemoryNotice | null>(null);
  const [attachmentPreview, setAttachmentPreview] = useState<ChatAttachment | null>(null);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [composerTools, setComposerTools] = useState<ToolInfo[]>([]);
  const [editing, setEditing] = useState<number | null>(null);
  const [modelsLoaded, setModelsLoaded] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [mediaCatalog, setMediaCatalog] = useState<MediaProviderCatalog>({});
  const [mediaKind, setMediaKind] = useState<MediaKind>("image");
  const [mediaAdvanced, setMediaAdvanced] = useState("{}");
  const [mediaSchema, setMediaSchema] = useState<MediaModelSchema | null>(null);
  const [mediaParameterValues, setMediaParameterValues] = useState<
    Record<string, unknown>
  >({});
  const [mediaSchemaLoading, setMediaSchemaLoading] = useState(false);
  const [mediaError, setMediaError] = useState<string | null>(null);
  const [pendingAttachments, setPendingAttachments] = useState<
    ChatAttachment[]
  >([]);
  const [reviewCommentsBySession, setReviewCommentsBySession] = useState<Record<string, ReviewComment[]>>({});
  const [, setPreviewSelection] =
    useState<PreviewSelection | null>(null);
  const [activePreviewSurface, setActivePreviewSurface] =
    useState<PreviewSurfaceTarget | null>(null);
  const [previewAppStatus, setPreviewAppStatus] =
    useState<PreviewAppStatus | null>(null);
  const [previewAppPreflight, setPreviewAppPreflight] =
    useState<PreviewAppPreflight | null>(null);
  const [previewAppPreflightBusy, setPreviewAppPreflightBusy] = useState(false);
  const [previewAppBusy, setPreviewAppBusy] = useState<
    "start" | "stop" | "restart" | null
  >(null);
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
  const [turnChanges, setTurnChanges] = useState<TurnChanges | null>(null);
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
  const inspectorTab = useSessions(
    (s) => s.sessions.find((x) => x.id === s.activeId)?.inspectorTab ?? "preview",
  );
  const inspectorOpen = useSessions(
    (s) =>
      s.sessions.find((x) => x.id === s.activeId)?.inspectorOpen === true,
  );
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
  const setSessionPreviewRuntime = useSessions((s) => s.setPreviewRuntime);
  const setPreviewRuntimeByKey = useSessions((s) => s.setPreviewRuntimeByKey);
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
  const configuredNewThreads = useSettings((s) => s.newThreadBehavior === "configured");
  const unavailableModelPolicy = useSettings((s) => s.unavailableModelPolicy);
  const setMediaSettings = useSettings((s) => s.setMediaSettings);
  const previewPanelWidth = useUiPreferences((s) => s.previewPanelWidth);
  const setPreviewPanelWidth = useUiPreferences((s) => s.setPreviewPanelWidth);
  const sidebarOpen = useUiPreferences((s) => s.sidebarOpen);
  const sidebarWidth = useUiPreferences((s) => s.sidebarWidth);
  const setSidebarOpen = useUiPreferences((s) => s.setSidebarOpen);
  const appShortcuts = useUiPreferences((s) => s.appShortcuts);
  const toggleSidebar = useUiPreferences((s) => s.toggleSidebar);
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
    goal,
  } = threadSettings;
  const goalComposerMode = Boolean(goalComposerSessions[activeId]);
  const activePreviewRuntimeKey = previewRuntimeKeyForThread(activeId, folder);
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
  const enabledMediaProviders = useMemo(
    () => mediaProviders(providers),
    [providers],
  );
  const mediaModelEntries = useMemo(
    () => mediaModelsForPicker(enabledMediaProviders, mediaSettings, mediaCatalog),
    [enabledMediaProviders, mediaSettings, mediaCatalog],
  );

  function persistPreviewRuntimeStatus(status: PreviewAppStatus) {
    const state = useSessions.getState();
    const previous = folder.trim()
      ? state.previewRuntimesByKey[activePreviewRuntimeKey]
      : state.sessions.find((session) => session.id === activeId)
          ?.previewRuntime;
    const runtime = previewRuntimeFromStatus(status, previous);
    if (folder.trim()) setPreviewRuntimeByKey(activePreviewRuntimeKey, runtime);
    else setSessionPreviewRuntime(activeId, runtime);
  }

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

  useEffect(() => {
    const state = useSessions.getState();
    const runtime = folder.trim()
      ? state.previewRuntimesByKey[activePreviewRuntimeKey]
      : state.sessions.find((session) => session.id === activeId)
          ?.previewRuntime;
    const status = previewStatusFromRuntime(activePreviewRuntimeKey, runtime);
    const matchingStatus = previewStatusMatchesFolder(status, folder)
      ? status
      : null;
    setPreviewAppStatus(matchingStatus);
    setPreviewAppPreflight(matchingStatus?.preflight ?? null);
  }, [activeId, activePreviewRuntimeKey, folder, sessionsHydrated]);

  useEffect(() => {
    let cancelled = false;
    async function pollPreviewApp() {
      if (!documentVisible()) return;
      try {
        const status = await getPreviewAppStatus(activePreviewRuntimeKey);
        if (!cancelled) {
          if (previewStatusMatchesFolder(status, folder)) {
            const freshStatus = { ...status, stale: false };
            setPreviewAppStatus(freshStatus);
            setPreviewAppPreflight(status.preflight ?? null);
            persistPreviewRuntimeStatus(freshStatus);
          } else {
            setPreviewAppStatus(null);
            if (folder.trim())
              setPreviewRuntimeByKey(activePreviewRuntimeKey, undefined);
            else setSessionPreviewRuntime(activeId, undefined);
          }
        }
      } catch {
        if (!cancelled) {
          setPreviewAppStatus((current) =>
            current?.thread_id === activePreviewRuntimeKey
              ? { ...current, stale: true }
              : current,
          );
        }
      }
    }
    void pollPreviewApp();
    const timer = window.setInterval(() => void pollPreviewApp(), 2500);
    const onVisible = () => {
      if (documentVisible()) void pollPreviewApp();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [
    activeId,
    activePreviewRuntimeKey,
    folder,
    setPreviewRuntimeByKey,
    setSessionPreviewRuntime,
  ]);
  const pickerModels = useMemo(
    () => mergeModelListsForPicker(models, mediaModelEntries),
    [models, mediaModelEntries],
  );
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
  const activeMediaTarget = useMemo(
    () => resolveActiveMediaTarget(
      effectiveModel,
      enabledMediaProviders,
      mediaSettings,
      mediaCatalog,
    ),
    [
      effectiveModel,
      enabledMediaProviders,
      mediaSettings,
      mediaCatalog,
    ],
  );
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
      setTurnChanges(null);
      return;
    }

    let cancelled = false;
    void (async () => {
      try {
        const selected = await setWorkspace(latestTurnCheckpoint.folder);
        if (!selected || cancelled) return;
        const result = await runWorkspaceGitAction("diff", {
          diff_scope: "last_turn",
          diff_base: latestTurnCheckpoint.ref,
        });
        if (cancelled) return;
        setTurnChanges(
          turnChangesFromDiff(
            latestTurnChangesKey,
            latestTurnCheckpoint,
            result,
          ),
        );
      } catch {
        if (!cancelled) setTurnChanges(null);
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
  ]);

  const chatScrollRef = useRef<HTMLDivElement>(null);
  const chatBodyRef = useRef<HTMLDivElement>(null);
  const previewResizeHandleRef = useRef<HTMLDivElement>(null);
  const contextLauncherRef = useRef<HTMLButtonElement>(null);
  const stickToBottomRef = useRef(true);
  const generationControllersRef = useRef<Map<string, AbortController>>(
    new Map(),
  );
  const childThreadEventControllersRef = useRef<Map<string, AbortController>>(
    new Map(),
  );
  const workerRunEventControllersRef = useRef<Map<string, AbortController>>(
    new Map(),
  );
  const approvedWorkerRunsRef = useRef<Set<string>>(new Set());
  const resumingWorkerRunsRef = useRef<Set<string>>(new Set());
  const workerRunReconcileRetriesRef = useRef<Map<string, number>>(new Map());
  const childThreadLiveIdsRef = useRef<Map<string, Set<string>>>(new Map());
  const childThreadEventsRef = useRef<Map<string, ThreadEvent[]>>(new Map());
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
  const mobileRelayPollingRef = useRef(false);
  const scheduleRunPollingRef = useRef(false);
  const mobileRelayReadyRef = useRef(false);
  const goalLoopRef = useRef<GoalLoopState | null>(null);
  const queueDrainRef = useRef<Set<string>>(new Set());
  const compactionInFlightRef = useRef(false);
  const messageHeightsRef = useRef<number[]>([]);
  const gitStatusUpdatedAtRef = useRef<number | null>(null);
  const [previewResizing, setPreviewResizing] = useState(false);
  const [previewPanelOverlay, setPreviewPanelOverlay] = useState(false);
  const [recentThreadSwitcher, setRecentThreadSwitcher] =
    useState<RecentThreadSwitcherState | null>(null);
  const [messageScrollSnapshot, setMessageScrollSnapshot] = useState({
    top: 0,
    height: 0,
  });
  const [messageHeightsVersion, setMessageHeightsVersion] = useState(0);

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

  const measureMessageRow = useCallback((index: number, height: number) => {
    const next = Math.max(1, Math.ceil(height));
    if (messageHeightsRef.current[index] === next) return;
    messageHeightsRef.current[index] = next;
    setMessageHeightsVersion((version) => version + 1);
  }, []);

  function updateMessageScrollSnapshot() {
    const el = chatScrollRef.current;
    if (!el) return;
    setMessageScrollSnapshot({
      top: el.scrollTop,
      height: el.clientHeight,
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
    updateMessageScrollSnapshot();
  }

  function isLiveChildThread(thread: ChildThreadInfo): boolean {
    return thread.status === "queued" || thread.status === "running";
  }

  function rememberedChildEvents(event: ThreadEvent): ThreadEvent[] {
    const existing = childThreadEventsRef.current.get(event.thread_id) ?? [];
    if (existing.some((item) => item.id === event.id)) return existing;
    const next = [...existing, event];
    childThreadEventsRef.current.set(event.thread_id, next);
    return next;
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
      ? rememberedChildEvents(ev.event)
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
    if (!isLiveChildThread(thread))
      childThreadEventsRef.current.delete(thread.id);
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
    void streamChildThreadEvents(
      parentId,
      (ev) => applyPushedChildThreadEvent(parentId, ev),
      controller.signal,
    )
      .catch((error) => {
        if (!controller.signal.aborted)
          console.warn("child thread event stream failed", error);
      })
      .finally(() => {
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
      const alreadySynthesized = currentMessages.some(
        (message) => workerRunSynthesisId(message) === canonical.run.id,
      );

      approvedWorkerRunsRef.current.delete(canonical.run.id);
      workerRunEventControllersRef.current.get(canonical.run.id)?.abort();
      const nextMessages = alreadySynthesized
        ? currentMessages
        : [...currentMessages, workerRunSynthesisMessage(canonical)];
      if (!alreadySynthesized)
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
    void streamWorkerRunEvents(run.id, applyWorkerRunEvent, controller.signal)
      .catch((error) => {
        if (!controller.signal.aborted)
          console.warn("worker run event stream failed", error);
      })
      .finally(() => {
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

  function updateAutoScrollCoupling() {
    const el = chatScrollRef.current;
    if (!el) return;
    stickToBottomRef.current = isNearScrollBottom(el);
    updateMessageScrollSnapshot();
  }

  useEffect(() => {
    let cancelled = false;
    void loadStartupModels(
      (nextModels) => {
        if (cancelled) return;
        setModels(nextModels);
        setModelsLoaded(true);
      },
      accountRuntimeEnabled,
    );
    return () => {
      cancelled = true;
    };
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    listProviders().then(setProviders);
  }, []);

  useEffect(() => {
    listSkills().then(setSkills);
  }, [skillsRevision]);

  useEffect(() => {
    let cancelled = false;
    void listTools()
      .then((tools) => {
        if (!cancelled) setComposerTools(tools);
      })
      .catch(() => {
        if (!cancelled) setComposerTools([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (enabledMediaProviders.length === 0) {
      setMediaCatalog({});
      return;
    }
    let cancelled = false;
    async function loadMediaCatalogs() {
      const rows = await Promise.all(
        enabledMediaProviders.flatMap((provider) =>
          (["image", "video", "music"] as MediaKind[]).map(async (kind) => {
            try {
              const models = await listMediaModels(provider.id, kind);
              return {
                providerId: provider.id,
                kind,
                ids: models.map((item) => item.id).filter(Boolean),
              };
            } catch {
              return { providerId: provider.id, kind, ids: [] };
            }
          }),
        ),
      );
      if (cancelled) return;
      const next: MediaProviderCatalog = {};
      for (const row of rows) {
        next[row.providerId] ??= {};
        next[row.providerId][row.kind] = row.ids;
      }
      setMediaCatalog(next);
    }
    void loadMediaCatalogs();
    return () => {
      cancelled = true;
    };
  }, [
    enabledMediaProviders.map((provider) => provider.id).join("\u0000"),
  ]);

  useEffect(() => {
    if (modelsLoaded && !model && models[0]?.id) {
      updateThreadSettings(activeId, { model: models[0].id });
    }
  }, [activeId, model, models, modelsLoaded, updateThreadSettings]);

  useEffect(() => {
    if (!activeMediaTarget) {
      setMediaSchema(null);
      setMediaParameterValues({});
      setMediaSchemaLoading(false);
      setMediaError(null);
      return;
    }
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    const saved = useSettings.getState().media;
    setMediaKind(activeMediaTarget.kind);
    setMediaAdvanced(
      saved.advancedByProviderModel[key] ??
        defaultMediaAdvanced(activeMediaTarget.provider),
    );
    setMediaParameterValues(saved.parametersByProviderModel[key] ?? {});
    setMediaSettings({
      providerId: activeMediaTarget.provider.id,
      modelByProvider: {
        ...saved.modelByProvider,
        [activeMediaTarget.provider.id]: activeMediaTarget.model,
      },
    });
  }, [activeMediaTarget?.provider.id, activeMediaTarget?.model]);

  useEffect(() => {
    if (!activeMediaTarget) return;
    const kind = activeMediaTarget.supportedKinds.includes(mediaKind)
      ? mediaKind
      : activeMediaTarget.kind;
    const key = mediaPreferenceKey(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
    );
    let cancelled = false;
    setMediaSchemaLoading(true);
    setMediaError(null);
    getMediaModelSchema(
      activeMediaTarget.provider.id,
      activeMediaTarget.model,
      kind,
    )
      .then((schema) => {
        if (cancelled) return;
        setMediaSchema(schema);
        const nextSaved =
          useSettings.getState().media.parametersByProviderModel[key];
        setMediaParameterValues({ ...schemaDefaults(schema), ...nextSaved });
      })
      .catch((e) => {
        if (!cancelled) {
          setMediaSchema(null);
          setMediaError(e instanceof Error ? e.message : String(e));
        }
      })
      .finally(() => {
        if (!cancelled) setMediaSchemaLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activeMediaTarget?.provider.id, activeMediaTarget?.model, mediaKind]);

  useEffect(() => {
    if (stickToBottomRef.current) scrollToChatBottom();
    else updateMessageScrollSnapshot();
  }, [messages]);

  useEffect(() => {
    stickToBottomRef.current = true;
    messageHeightsRef.current = [];
    setMessageHeightsVersion((version) => version + 1);
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
    return () => {
      if (previewCloseTimeoutRef.current != null) {
        window.clearTimeout(previewCloseTimeoutRef.current);
        previewCloseTimeoutRef.current = null;
      }
      if (stopShortcutConfirmTimerRef.current != null) {
        window.clearTimeout(stopShortcutConfirmTimerRef.current);
        stopShortcutConfirmTimerRef.current = null;
      }
      const store = useSessions.getState();
      generationControllersRef.current.forEach((controller, id) => {
        controller.abort();
        store.setSessionGenerating(id, false);
      });
      generationControllersRef.current.clear();
      childThreadEventControllersRef.current.forEach((controller) =>
        controller.abort(),
      );
      childThreadEventControllersRef.current.clear();
      childThreadLiveIdsRef.current.clear();
      workerRunEventControllersRef.current.forEach((controller) =>
        controller.abort(),
      );
      workerRunEventControllersRef.current.clear();
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
    const openUrl = (event: Event) => {
      const url = (event as CustomEvent<{ url?: unknown }>).detail?.url;
      if (typeof url !== "string") return;
      let parsed: URL;
      try {
        parsed = new URL(url);
      } catch {
        return;
      }
      if (parsed.protocol !== "https:") return;
      const current =
        useSessions.getState().sessions.find((session) => session.id === activeId)
          ?.browserSession ??
        emptyBrowserSession();
      const tab = emptyBrowserTab(parsed.toString());
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
    window.addEventListener("milim-open-browser-url", openUrl);
    return () => window.removeEventListener("milim-open-browser-url", openUrl);
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

  function openGitPanel(view: GitPanelView = "changes") {
    if (!folder.trim() || (gitStatus && !canOpenGitPanel)) return;
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
      if (
        useSessions
          .getState()
          .sessions.find((session) => session.id === activeId)
          ?.inspectorTab === "git"
      ) {
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

  function closePreview() {
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

  function openPreviewArtifact(
    artifact: ChatArtifact,
    artifacts?: readonly ChatArtifact[],
    previewDeferred = false,
    revision?: ArtifactRevision,
  ) {
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

  function openArtifactBrowser() {
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    setDismissedPreviewKey(latestPreviewSelection?.autoOpenKey ?? null);
    selectPreviewSource("url");
    setSessionInspectorTab(activeId, "preview");
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
    setPreviewAppPreflightBusy(true);
    try {
      const managedFiles = folder.trim()
        ? undefined
        : (files ??
          preparedPreviewFilesByThreadRef.current.get(activeId) ??
          managedPreviewFiles(latestRuntimePreview?.artifacts));
      if (managedFiles)
        preparedPreviewFilesByThreadRef.current.set(activeId, managedFiles);
      const preflight = await preflightPreviewApp(
        activePreviewRuntimeKey,
        folder.trim()
          ? { cwd: folder }
          : { files: managedFiles },
      );
      setPreviewAppPreflight(preflight);
      setPreviewAppStatus((current) =>
        current
          ? { ...current, preflight, stale: false }
          : current,
      );
      setChatNotice({ tone: "info", message: "Preview commands are ready to review." });
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setPreviewAppPreflightBusy(false);
    }
  }

  function previewRuntimeRunOptions(): PreviewAppStartOptions | null {
    if (!activePreviewAppPreflight) {
      setChatNotice({
        tone: "info",
        message: "Review the preview preflight before running commands.",
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

  function applyPreviewAppStatus(status: PreviewAppStatus) {
    const freshStatus = { ...status, stale: false };
    setPreviewAppStatus(freshStatus);
    setPreviewAppPreflight(status.preflight ?? activePreviewAppPreflight);
    persistPreviewRuntimeStatus(freshStatus);
  }

  async function startPreviewRuntime() {
    const options = previewRuntimeRunOptions();
    if (!options) return;
    setPreviewAppBusy("start");
    try {
      const status = await startPreviewApp(activePreviewRuntimeKey, options);
      applyPreviewAppStatus(status);
      if (!folder.trim() && options.files?.length)
        upsertVirtualFiles(activeId, options.files);
      selectPreviewSource("app");
      setSessionInspectorTab(activeId, "preview");
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setPreviewAppBusy(null);
    }
  }

  async function stopPreviewRuntime() {
    setPreviewAppBusy("stop");
    try {
      const status = await stopPreviewApp(activePreviewRuntimeKey);
      applyPreviewAppStatus(status);
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setPreviewAppBusy(null);
    }
  }

  async function restartPreviewRuntime() {
    const options = previewRuntimeRunOptions();
    if (!options) return;
    setPreviewAppBusy("restart");
    try {
      const status = await restartPreviewApp(activePreviewRuntimeKey, options);
      applyPreviewAppStatus(status);
      if (!folder.trim() && options.files?.length)
        upsertVirtualFiles(activeId, options.files);
    } catch (error) {
      setChatNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setPreviewAppBusy(null);
    }
  }

  function openArtifactSidePanel(tab: "preview" | "code" = "preview") {
    const selection =
      activeArtifactSelection ??
      latestPreviewSelection ??
      (tab === "code" && folder.trim() ? blankWorkspaceReviewSelection() : null);
    if (!selection) return;
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    artifactSelectionsByThreadRef.current.set(activeId, selection);
    selectPreviewSource("artifact");
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

  function openWorkersInspector(runId?: string, settings = false) {
    rememberInspectorInvoker();
    clearPreviewCloseTimer();
    setPreviewPanelClosing(false);
    setWorkerFocusRunId(runId ?? "");
    if (settings) setWorkerSettingsOpen(true);
    setSessionInspectorTab(activeId, "workers");
  }

  function openContextPanel() {
    if (chatBodyWidth < CONCURRENT_PANEL_THRESHOLD && inspectorOpen) {
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
    const sidebarGain = sidebarOpen
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
      sidebarWasOpen: sidebarOpen,
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
      } else if (sidebarOpen) {
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
    const session = useSessions
      .getState()
      .sessions.find((item) => item.id === sessionId);
    if (
      !session ||
      session.messages.filter((message) => message.role === "user").length !== 1
    )
      return;
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
        { maxTokens: 16, temperature: 0 },
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
          options.signal,
        );
      } else if (opencodeModel) {
        const ready = await ensureOpenCodeAccount();
        if (!ready.ok) throw new Error(ready.message);
        summary = await summarizeWithOpenCode(
          opencodeModel,
          promptMessages,
          options.folder,
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
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    let text = "";
    let error: string | null = null;
    let warning: string | null = null;
    let usage: TokenUsage | undefined;
    let costUsd: number | undefined;
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    await streamCodexRun(
      codexCompactionSummaryRequest({
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        reasoningEffort,
        images: runtimeInput.images,
      }),
      (ev: CodexRunEvent) => {
        if (ev.type === "token" && ev.text) text += ev.text;
        else if (ev.type === "warning") warning = ev.message;
        else if (ev.type === "error") error = ev.message;
        else if (ev.type === "done") {
          usage = ev.usage;
          costUsd =
            typeof ev.cost_usd === "number" && ev.cost_usd > 0
              ? ev.cost_usd
              : undefined;
        }
      },
      signal,
    );
    if (error) throw new Error(error);
    if (warning) throw new Error(warning);
    return { content: text, usage, costUsd };
  }

  async function summarizeWithClaude(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    reasoningEffort: ReasoningEffort,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    let text = "";
    let error: string | null = null;
    let warning: string | null = null;
    let usage: TokenUsage | undefined;
    let costUsd: number | undefined;
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    await streamClaudeRun(
      claudeCompactionSummaryRequest({
        model,
        prompt: runtimeInput.prompt,
        cwd: folder.trim() || undefined,
        reasoningEffort,
        images: runtimeInput.images,
      }),
      (ev: ClaudeRunEvent) => {
        if (ev.type === "token" && ev.text) text += ev.text;
        else if (ev.type === "warning") warning = ev.message;
        else if (ev.type === "error") error = ev.message;
        else if (ev.type === "done") {
          usage = ev.usage;
          costUsd =
            typeof ev.cost_usd === "number" && ev.cost_usd > 0
              ? ev.cost_usd
              : undefined;
        }
      },
      signal,
    );
    if (error) throw new Error(error);
    if (warning) throw new Error(warning);
    return { content: text, usage, costUsd };
  }

  async function summarizeWithOpenCode(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    let text = "";
    let error: string | null = null;
    let warning: string | null = null;
    let usage: TokenUsage | undefined;
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    await streamOpenCodeRun({
      model,
      prompt: runtimeInput.prompt,
      cwd: folder.trim() || undefined,
      images: runtimeInput.images,
      tool_approval_policy: "guarded",
      plan_mode: true,
    }, (ev: OpenCodeRunEvent) => {
      if (ev.type === "token" && ev.text) text += ev.text;
      else if (ev.type === "warning") warning = ev.message;
      else if (ev.type === "error") error = ev.message;
      else if (ev.type === "done") usage = ev.usage;
    }, signal);
    if (error) throw new Error(error);
    if (warning) throw new Error(warning);
    return { content: text, usage };
  }

  async function summarizeWithPi(
    model: string,
    promptMessages: ChatMessage[],
    folder: string,
    reasoningEffort: ReasoningEffort,
    signal?: AbortSignal,
  ): Promise<CompactionSummaryResult> {
    let text = "";
    let error: string | null = null;
    let warning: string | null = null;
    let usage: TokenUsage | undefined;
    const runtimeInput = accountRuntimeInputFromMessages(promptMessages);
    await streamPiRun({
      model,
      prompt: runtimeInput.prompt,
      cwd: folder.trim() || undefined,
      images: runtimeInput.images,
      reasoning_effort: reasoningEffort,
      persist_session: false,
      tool_approval_policy: "guarded",
      plan_mode: true,
    }, (ev: PiRunEvent) => {
      if (ev.type === "token" && ev.text) text += ev.text;
      else if (ev.type === "warning") warning = ev.message;
      else if (ev.type === "error") error = ev.message;
      else if (ev.type === "done") usage = ev.usage;
    }, signal);
    if (error) throw new Error(error);
    if (warning) throw new Error(warning);
    return { content: text, usage };
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
    const currentMessages =
      useSessions.getState().sessions.find((session) => session.id === activeId)
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
      const reasoningEffort = reasoningEffortForModel(
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
        activeId,
        split.head,
        selectedModel,
        {
          auto: false,
          folder,
          reasoningEffort,
        },
      );
      const store = useSessions.getState();
      store.setMessages(activeId, [...split.head, checkpoint, ...split.tail], {
        autoTitle: false,
      });
      store.clearAccountRuntime(activeId);
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

  function sessionGoal(sessionId: string): GoalSettings {
    return useSessions.getState().getSettings(sessionId).goal;
  }

  function updateGoalState(
    sessionId: string,
    patch: Partial<GoalSettings>,
    baseGoal = sessionGoal(sessionId),
  ): GoalSettings {
    const next = normalizeGoalSettings({
      ...baseGoal,
      ...patch,
      updatedAt: Date.now(),
    });
    useSessions.getState().updateSettings(sessionId, { goal: next });
    return next;
  }

  async function drainQueuedMessages(
    sessionId: string,
    fallbackModel?: string,
  ) {
    return drainQueuedMessagesFromQueue({
      sessionId,
      fallbackModel,
      queueDrainRef,
      generationControllersRef,
      agents: useAgents.getState().agents,
      setChatNotice,
      sessionMessages,
      runTurn: (convo, selectedModel, targetSessionId) =>
        runTurn(convo, selectedModel, {}, targetSessionId),
    });
  }

  useEffect(() => {
    const interruptedSessionIds = Object.keys(queueInterrupts);
    if (interruptedSessionIds.length === 0) return;
    const generating = new Set(generatingSessionIds);
    const liveWorkers = new Set(
      liveWorkerSessionIdsKey ? liveWorkerSessionIdsKey.split("\0") : [],
    );
    const ready = interruptedSessionIds.filter(
      (sessionId) =>
        !generating.has(sessionId) && !liveWorkers.has(sessionId),
    );
    if (ready.length === 0) return;
    setQueueInterrupts((current) => {
      const next = { ...current };
      for (const sessionId of ready) delete next[sessionId];
      return next;
    });
    for (const sessionId of ready) void drainQueuedMessages(sessionId);
  }, [generatingSessionIds, liveWorkerSessionIdsKey, queueInterrupts]);

  async function runTurnAndDrain(
    convo: ChatMessage[],
    selectedModel?: string,
    options: RunTurnOptions = {},
  ) {
    const sessionId = activeId;
    const result = await runTurn(convo, selectedModel, options, sessionId);
    let terminalResult = result;
    if (result.status === "done") {
      terminalResult = await drainQueuedMessages(sessionId, selectedModel) ?? result;
    }
    if (
      document.visibilityState === "visible" &&
      useSessions.getState().activeId === sessionId
    ) {
      const preferences = useUiPreferences.getState();
      if (
        preferences.interfaceSounds &&
        terminalResult.status === "done" &&
        preferences.soundOnFinished
      ) playInterfaceSound(preferences.finishedSound);
      else if (
        preferences.interfaceSounds &&
        terminalResult.status === "error" &&
        preferences.soundOnAttention
      ) playInterfaceSound(preferences.attentionSound);
    }
    const preferences = useUiPreferences.getState();
    const threadTitle = useSessions.getState().sessions.find((session) => session.id === sessionId)?.title;
    if (terminalResult.status === "done" && preferences.notifyRunFinished) {
      void sendMilimNotification("finished", {
        threadTitle,
        includeThreadTitle: preferences.notificationIncludeThreadTitle,
        onlyWhenUnfocused: preferences.notifyOnlyWhenUnfocused,
      });
    } else if (terminalResult.status === "error" && preferences.notifyNeedsAttention) {
      void sendMilimNotification("attention", {
        threadTitle,
        includeThreadTitle: preferences.notificationIncludeThreadTitle,
        onlyWhenUnfocused: preferences.notifyOnlyWhenUnfocused,
      });
    }
    return result;
  }

  function pauseGoalRun(reason = "Goal paused.", sessionId = activeId) {
    const loop = goalLoopRef.current;
    if (loop?.sessionId === sessionId) {
      loop.stopped = true;
      loop.decisionController?.abort();
    }
    generationControllersRef.current.get(sessionId)?.abort();
    const current = sessionGoal(sessionId);
    if (current.status === "running") {
      updateGoalState(
        sessionId,
        { status: "paused", lastReason: reason },
        current,
      );
    }
  }

  function draftToGoal(
    draft: GoalPanelDraft,
    current: GoalSettings,
  ): GoalSettings {
    const contentChanged =
      draft.objective !== current.objective ||
      draft.successCriteria !== current.successCriteria ||
      draft.constraints !== current.constraints;
    const status = !draft.objective.trim()
      ? "idle"
      : current.status === "running"
        ? "paused"
        : contentChanged &&
            (current.status === "complete" ||
              current.status === "blocked" ||
              current.status === "error")
          ? "paused"
          : current.status;
    return normalizeGoalSettings({
      ...current,
      objective: draft.objective,
      successCriteria: draft.successCriteria,
      constraints: draft.constraints,
      developerMaxTurns: draft.developerMaxTurns,
      status,
      lastReason:
        current.status === "running"
          ? "Goal paused for edits."
          : current.lastReason,
      updatedAt: Date.now(),
    });
  }

  function saveGoalDraft(
    draft: GoalPanelDraft,
    sessionId = activeId,
  ): GoalSettings {
    const current = sessionGoal(sessionId);
    if (current.status === "running")
      pauseGoalRun("Goal paused for edits.", sessionId);
    const next = draftToGoal(draft, current);
    useSessions.getState().updateSettings(sessionId, { goal: next });
    setGoalPrefill(null);
    return next;
  }

  function deleteGoal(sessionId = activeId) {
    pauseGoalRun("Goal deleted.", sessionId);
    useSessions
      .getState()
      .updateSettings(sessionId, { goal: DEFAULT_GOAL_SETTINGS });
    setGoalPrefill(null);
    setGoalPanelOpen(false);
  }

  function markGoalSeen(sessionId = activeId) {
    const current = sessionGoal(sessionId);
    const updatedAt = current.updatedAt ?? 0;
    if (!updatedAt || (current.lastSeenAt ?? 0) >= updatedAt) return;
    useSessions.getState().updateSettings(sessionId, {
      goal: { ...current, lastSeenAt: Date.now() },
    });
  }

  function openGoalPanel(prefill: string | null = null) {
    setGoalPrefill(prefill);
    markGoalSeen();
    setGoalPanelOpen(true);
    setChatNotice(null);
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
      const decisionMessages = goalDecisionMessages(
        currentGoal,
        latestMessages,
      );
      const decisionReasoningEffort = reasoningEffortForModel(
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
      if (codexModel) {
        let codexError: string | null = null;
        let codexWarning: string | null = null;
        await streamCodexRun(
          {
            model: codexModel,
            prompt: runtimeInput.prompt,
            images: runtimeInput.images,
            cwd: folder.trim() || undefined,
            reasoning_effort: decisionReasoningEffort,
            tool_approval_policy: "review",
            tool_approval_grant: false,
            plan_mode: false,
          },
          (ev: CodexRunEvent) => {
            if (ev.type === "token" && ev.text) content += ev.text;
            else if (ev.type === "warning") codexWarning = ev.message;
            else if (ev.type === "error") codexError = ev.message;
          },
          controller.signal,
        );
        if (codexWarning) throw new Error(codexWarning);
        if (codexError) throw new Error(codexError);
      } else if (claudeModel) {
        let claudeError: string | null = null;
        let claudeWarning: string | null = null;
        await streamClaudeRun(
          {
            model: claudeModel,
            prompt: runtimeInput.prompt,
            images: runtimeInput.images,
            cwd: folder.trim() || undefined,
            reasoning_effort: decisionReasoningEffort,
            tool_approval_policy: "review",
            tool_approval_grant: false,
            plan_mode: false,
          },
          (ev: ClaudeRunEvent) => {
            if (ev.type === "token" && ev.text) content += ev.text;
            else if (ev.type === "warning") claudeWarning = ev.message;
            else if (ev.type === "error") claudeError = ev.message;
          },
          controller.signal,
        );
        if (claudeWarning) throw new Error(claudeWarning);
        if (claudeError) throw new Error(claudeError);
      } else if (opencodeModel) {
        let runtimeError: string | null = null;
        let runtimeWarning: string | null = null;
        await streamOpenCodeRun({
          model: opencodeModel,
          prompt: runtimeInput.prompt,
          images: runtimeInput.images,
          cwd: folder.trim() || undefined,
          tool_approval_policy: "review",
          tool_approval_grant: false,
          plan_mode: false,
        }, (ev: OpenCodeRunEvent) => {
          if (ev.type === "token" && ev.text) content += ev.text;
          else if (ev.type === "warning") runtimeWarning = ev.message;
          else if (ev.type === "error") runtimeError = ev.message;
        }, controller.signal);
        if (runtimeWarning) throw new Error(runtimeWarning);
        if (runtimeError) throw new Error(runtimeError);
      } else if (piModel) {
        let runtimeError: string | null = null;
        let runtimeWarning: string | null = null;
        await streamPiRun({
          model: piModel,
          prompt: runtimeInput.prompt,
          images: runtimeInput.images,
          cwd: folder.trim() || undefined,
          reasoning_effort: decisionReasoningEffort,
          persist_session: false,
          tool_approval_policy: "guarded",
          tool_approval_grant: false,
          plan_mode: true,
        }, (ev: PiRunEvent) => {
          if (ev.type === "token" && ev.text) content += ev.text;
          else if (ev.type === "warning") runtimeWarning = ev.message;
          else if (ev.type === "error") runtimeError = ev.message;
        }, controller.signal);
        if (runtimeWarning) throw new Error(runtimeWarning);
        if (runtimeError) throw new Error(runtimeError);
      } else {
        content = await completeChat(turnModel, decisionMessages, {
          signal: controller.signal,
          maxTokens: 500,
          temperature: 0,
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
    if (selected) updateThreadSettings(activeId, { folder: selected });
  }

  function startChatInFolder(nextFolder: string) {
    void createInteractiveChat({ folder: nextFolder });
    setChatNotice(null);
    focusComposer();
  }

  async function pickProjectFolder() {
    const selected = await openFolderPicker();
    if (selected) startChatInFolder(selected);
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

  function updateInlineMediaAdvanced(value: string) {
    setMediaAdvanced(value);
    const target = activeMediaTarget;
    if (!target) return;
    const key = mediaPreferenceKey(target.provider.id, target.model);
    const saved = useSettings.getState().media;
    setMediaSettings({
      advancedByProviderModel: {
        ...saved.advancedByProviderModel,
        [key]: value,
      },
    });
  }

  function updateInlineMediaParameter(
    control: MediaSchemaControl,
    value: string | boolean,
  ) {
    const target = activeMediaTarget;
    if (!target) return;
    let parsed: unknown;
    try {
      parsed = parseControlValue(control, value);
    } catch (e) {
      setMediaError(e instanceof Error ? e.message : String(e));
      return;
    }
    const next = { ...mediaParameterValues, [control.key]: parsed };
    const key = mediaPreferenceKey(target.provider.id, target.model);
    const saved = useSettings.getState().media;
    setMediaParameterValues(next);
    setMediaSettings({
      parametersByProviderModel: {
        ...saved.parametersByProviderModel,
        [key]: next,
      },
    });
    setMediaError(null);
  }

  async function pollInlineMediaStatus(
    initial: MediaGenerationResult,
    sessionId: string,
    requestId: string,
  ) {
    if (!shouldPollMediaStatus(initial)) return;
    let current = initial;
    try {
      for (let attempt = 0; attempt < mediaPollingMaxAttempts(initial); attempt += 1) {
        await new Promise((resolve) => window.setTimeout(resolve, 3000));
        const next = await getMediaStatus({
          provider_id: current.provider_id,
          id: current.id,
          model: current.model,
          response_url: current.urls.response,
          status_url: current.urls.status,
          kind: current.kind as MediaKind,
        });
        current = next;
        replaceMediaResult(sessionId, requestId, next);
        if (!shouldPollMediaStatus(next) || next.media.length > 0) break;
      }
    } catch (e) {
      updateMediaMessage(sessionId, requestId, {
        content: `Media status failed: ${e instanceof Error ? e.message : String(e)}`,
      });
    }
  }

  async function sendMediaPrompt(
    text: string,
    target: ActiveMediaTarget,
    baseMessages: ChatMessage[] = messages,
    checkPendingAttachments = true,
  ) {
    if (checkPendingAttachments && pendingAttachments.length > 0) {
      setChatNotice({
        tone: "error",
        message:
          "Media generation uses the prompt text only. Remove attachments or choose a chat model.",
      });
      return;
    }
    if (generationControllersRef.current.has(activeId)) return;

    let requestInput: Record<string, unknown>;
    try {
      requestInput = inputWithSchemaControls(
        mediaAdvanced,
        mediaSchema,
        mediaParameterValues,
      );
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setMediaError(message);
      setChatNotice({ tone: "error", message });
      return;
    }

    const sessionId = activeId;
    const requestId = attachmentId();
    const kind = target.supportedKinds.includes(mediaKind)
      ? mediaKind
      : target.kind;
    const prompt = mediaPromptWithHistory(baseMessages, text);
    const userMessage: ChatMessage = { role: "user", content: text };
    const assistantMessage: ChatMessage = {
      role: "assistant",
      content: `Generating ${kind} with ${target.model}...`,
      mediaRequestId: requestId,
    };
    setInput("");
    setPendingAttachments([]);
    setChatNotice(null);
    setMediaError(null);
    setMessages(sessionId, [...baseMessages, userMessage, assistantMessage], {
      autoTitle: autoTitleChats,
    });

    const store = useSessions.getState();
    const controller = new AbortController();
    generationControllersRef.current.set(sessionId, controller);
    store.setSessionGenerating(sessionId, true);
    try {
      const result = await generateMedia(
        {
          provider_id: target.provider.id,
          kind,
          model: target.model,
          prompt,
          input: requestInput,
        },
        controller.signal,
      );
      replaceMediaResult(sessionId, requestId, result);
      void pollInlineMediaStatus(result, sessionId, requestId);
    } catch (e) {
      const aborted = e instanceof DOMException && e.name === "AbortError";
      updateMediaMessage(sessionId, requestId, {
        content: aborted
          ? "Media generation stopped."
          : `Media generation failed: ${e instanceof Error ? e.message : String(e)}`,
      });
    } finally {
      generationControllersRef.current.delete(sessionId);
      store.setSessionGenerating(sessionId, false);
      store.setSessionUnread(
        sessionId,
        useSessions.getState().activeId !== sessionId,
      );
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
    const turnReasoningEffort = reasoningEffortForModel(
      useSettings.getState().reasoningEffortByModel,
      turnModel,
      pickerModels,
    );
    const turnTitle = turnSetup.title;
    const codexModel = turnSetup.codexModel;
    const claudeModel = turnSetup.claudeModel;
    const opencodeModel = turnSetup.opencodeModel;
    const piModel = turnSetup.piModel;
    const store = useSessions.getState();
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
          streamCodexRun,
          streamClaudeRun,
          streamOpenCodeRun,
          streamPiRun,
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
        (step) => step.approval?.status === "pending",
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
      if (resultStatus === "done" && assistantStart.state.started) {
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
    void runTurnAndDrain(convo);
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
    void runTurnAndDrain(convo);
  }

  async function stopSessionRun(sessionId: string) {
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

  function switchVisibleSession(id: string) {
    if (id === activeId) return;
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
      } else if (shortcutMatchesEvent(appShortcuts.toggleSidebar, event)) {
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
    toggleSidebar,
  ]);

  function mobileRelayAttachments(
    attachments?: MobileRelayAttachment[],
  ): ChatAttachment[] {
    return (attachments ?? [])
      .filter(
        (attachment) =>
          attachment.name && attachment.mime && attachment.size >= 0,
      )
      .flatMap((attachment) => {
        const next = {
          id: attachment.id || attachmentId(),
          name: attachment.name,
          mime: attachment.mime,
          size: attachment.size,
          content: attachment.content,
          dataUrl: attachment.dataUrl,
          truncated: Boolean(attachment.truncated),
        };
        try {
          assertValidImageAttachment(next);
          return [next];
        } catch (error) {
          setChatNotice({
            tone: "error",
            message: error instanceof Error ? error.message : String(error),
          });
          return [];
        }
      });
  }

  function appendMobileRelayText(
    text: string,
    attachments: ChatAttachment[] = [],
  ) {
    if (text) {
      setInput((current) => {
        const trimmed = current.trimEnd();
        return trimmed ? `${trimmed}\n${text}` : text;
      });
    }
    if (attachments.length) {
      setPendingAttachments((current) =>
        [...current, ...attachments].slice(0, 12),
      );
    }
  }

  function sendMobileRelayText(event: MobileRelayEvent) {
    const text = event.text.trim();
    const attachments = mobileRelayAttachments(event.attachments);
    if (!text && attachments.length === 0) return;
    if (busy) {
      enqueueQueuedMessage(activeId, { content: text, attachments });
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} queued.`,
      });
      return;
    }
    const selectedModel = effectiveModel.trim();
    if (!selectedModel) {
      appendMobileRelayText(text, attachments);
      setProvidersOpen(true);
      setChatNotice({
        tone: "error",
        message:
          "Mobile relay is waiting in the composer. Choose a model before sending.",
      });
      return;
    }
    setInput("");
    setPendingAttachments([]);
    setChatNotice({
      tone: "info",
      message: `Mobile relay from ${event.device_name} sent.`,
    });
    void runTurnAndDrain(
      appendUserTurn(
        messages,
        text,
        attachments.length ? attachments : undefined,
      ),
      selectedModel,
    );
  }

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

  function startMobileThread(event: MobileRelayEvent) {
    const text = event.text.trim();
    const attachments = mobileRelayAttachments(event.attachments);
    useSessions.getState().newChat(threadSettings);
    const sessionId = useSessions.getState().activeId;
    setInput("");
    setPendingAttachments([]);
    if (!text && attachments.length === 0) {
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} created a thread.`,
      });
      return;
    }
    const selectedModel = queuedModelForSession(
      sessionId,
      effectiveModel.trim(),
      agents,
    );
    if (!selectedModel) {
      enqueueQueuedMessage(sessionId, { content: text, attachments });
      setProvidersOpen(true);
      setChatNotice({
        tone: "error",
        message:
          "Mobile relay created a thread, but a model is needed before sending.",
      });
      return;
    }
    setChatNotice({
      tone: "info",
      message: `Mobile relay from ${event.device_name} started a thread.`,
    });
    void runTurn(
      appendUserTurn([], text, attachments.length ? attachments : undefined),
      selectedModel,
      {},
      sessionId,
    ).then((result) => {
      if (result.status === "done")
        void drainQueuedMessages(sessionId, selectedModel);
    });
  }

  function activeOrPayloadThreadId(text: string): string {
    return text.trim() || activeId;
  }

  function deleteMobileMessage(text: string) {
    const index = Number.parseInt(text.trim(), 10);
    if (!Number.isFinite(index)) return;
    deleteMessageAt(index);
  }

  async function deleteThreadWithRetryCleanup(sessionId: string) {
    const session = useSessions.getState().sessions.find((item) => item.id === sessionId);
    const isolated = session?.threadWorkspace?.mode === "worktree"
      ? session.threadWorkspace
      : null;
    if (isolated) {
      try {
        await setWorkspace(isolated.projectFolder);
        let result = await runWorkspaceGitAction("remove_thread_worktree", {
          thread_id: sessionId,
        });
        if (!result.ok && result.message.includes("uncommitted changes")) {
          const discard = window.confirm(
            `${result.message}\n\nForce-discard the worktree? The branch will be retained.`,
          );
          if (!discard) return;
          result = await runWorkspaceGitAction("remove_thread_worktree", {
            thread_id: sessionId,
            force: true,
          });
        }
        if (!result.ok) throw new Error(result.message);
      } catch (error) {
        setChatNotice({
          tone: "error",
          message: `Thread was kept because worktree cleanup failed: ${error instanceof Error ? error.message : String(error)}`,
        });
        return;
      }
    }
    const retry = session?.retryWorkspace;
    if (retry) {
      try {
        await setWorkspace(retry.originalFolder);
        const result = await runWorkspaceGitAction("remove_retry_worktree", {
          worktree: retry.worktreeFolder,
        });
        if (!result.ok) throw new Error(result.message);
      } catch (error) {
        setChatNotice({
          tone: "error",
          message: `Retry thread was kept because cleanup failed: ${error instanceof Error ? error.message : String(error)}`,
        });
        return;
      }
    }
    useSessions.getState().remove(sessionId);
  }

  function applyMobileRelayEvent(event: MobileRelayEvent) {
    if (event.action === "new_thread") {
      startMobileThread(event);
      return;
    }
    if (event.action === "worker_run_start") {
      const runId = event.text.trim() || activeWorkerRun?.run.id;
      if (runId) void approveWorkerRun(runId);
      return;
    }
    if (event.action === "worker_run_continue_solo") {
      const runId = event.text.trim() || activeWorkerRun?.run.id;
      if (runId) void continueWorkerRunSolo(runId);
      return;
    }
    if (event.action === "worker_run_stop") {
      const runId = event.text.trim() || activeWorkerRun?.run.id;
      if (runId) void stopActiveWorkerRun(runId);
      return;
    }
    if (event.action === "stop") {
      stop();
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} stopped generation.`,
      });
      return;
    }
    if (event.action === "regenerate") {
      regenerate();
      return;
    }
    if (event.action === "delete_message") {
      deleteMobileMessage(event.text);
      return;
    }
    if (event.action === "rename_thread") {
      const title = event.text.trim();
      if (title) useSessions.getState().rename(activeId, title);
      return;
    }
    if (event.action === "archive_thread") {
      useSessions
        .getState()
        .archiveSession(activeOrPayloadThreadId(event.text));
      return;
    }
    if (event.action === "delete_thread") {
      void deleteThreadWithRetryCleanup(activeOrPayloadThreadId(event.text));
      return;
    }
    if (event.action === "set_model") {
      const nextModel = event.text.trim();
      if (nextModel) {
        const session = useSessions.getState().sessions.find((item) => item.id === activeId);
        const kind = nextModel.startsWith("codex:")
          ? "codex"
          : nextModel.startsWith("claude:")
            ? "claude"
            : nextModel.startsWith("opencode:")
              ? "opencode"
              : nextModel.startsWith("pi:")
                ? "pi"
              : null;
        if (session && kind && nativeRuntimeIsStale(session, kind)) {
          useSessions.getState().clearAccountRuntimeKind(activeId, kind);
        }
        updateThreadSettings(activeId, { model: nextModel });
      }
      return;
    }
    if (event.action === "attach") {
      appendMobileRelayText(
        event.text.trim(),
        mobileRelayAttachments(event.attachments),
      );
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} added attachments.`,
      });
      return;
    }
    if (event.action === "switch_thread") {
      const targetId = event.text.trim();
      const target = useSessions
        .getState()
        .sessions.find(
          (session) => session.id === targetId && !session.archivedAt,
        );
      if (target) {
        switchToSession(target.id);
        setChatNotice({
          tone: "info",
          message: `Mobile relay switched to ${target.title || "thread"}.`,
        });
      }
      return;
    }
    if (event.action === "replace") {
      setInput(event.text);
      const attachments = mobileRelayAttachments(event.attachments);
      setPendingAttachments(attachments);
      setChatNotice({
        tone: "info",
        message: `Mobile relay from ${event.device_name} replaced the composer.`,
      });
      return;
    }
    if (event.action === "send") {
      sendMobileRelayText(event);
      return;
    }
    appendMobileRelayText(
      event.text,
      mobileRelayAttachments(event.attachments),
    );
    const notice = {
      tone: "info",
      message: `Mobile relay from ${event.device_name} added to the composer.`,
    } as const;
    setChatNotice(notice);
    pushNotice(notice);
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
    async function refreshMobileRelayReady() {
      if (!documentVisible()) return;
      try {
        const status = await getMobileCompanionStatus();
        mobileRelayReadyRef.current =
          status.enabled && status.devices.length > 0;
        if (mobileRelayReadyRef.current) void pollMobileRelay();
      } catch {
        mobileRelayReadyRef.current = false;
      }
    }
    async function pollMobileRelay() {
      if (!documentVisible() || !mobileRelayReadyRef.current) return;
      if (mobileRelayPollingRef.current) return;
      mobileRelayPollingRef.current = true;
      try {
        const events = await pollMobileCompanionEvents();
        if (!cancelled) {
          for (const event of events) applyMobileRelayEvent(event);
        }
      } catch {
        // The bridge is disabled in normal web previews and before pairing.
      } finally {
        mobileRelayPollingRef.current = false;
      }
    }
    void refreshMobileRelayReady();
    const timer = window.setInterval(() => void pollMobileRelay(), 1500);
    const statusTimer = window.setInterval(
      () => void refreshMobileRelayReady(),
      30_000,
    );
    const onVisible = () => {
      if (documentVisible()) void refreshMobileRelayReady();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.clearInterval(statusTimer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [activeId, busy, effectiveModel, switchToSession]);

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

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void publishMobileThreadSnapshot({
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
      }).catch(() => {});
    }, 300);
    return () => window.clearTimeout(timer);
  }, [
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
  ]);

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
  const virtualMessages = useMemo(
    () =>
      virtualMessageWindow(
        messages,
        messageHeightsRef.current,
        messageScrollSnapshot.top,
        messageScrollSnapshot.height,
      ),
    [
      messages,
      messageHeightsVersion,
      messageScrollSnapshot.height,
      messageScrollSnapshot.top,
    ],
  );
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

  function openPreviewInspector() {
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
          <div
            className="chat-scroll"
            ref={chatScrollRef}
            onScroll={updateAutoScrollCoupling}
          >
            {!emptyThread && (
              <div
                className={
                  "messages" +
                  (virtualMessages.virtualized ? " messages-virtualized" : "")
                }
                style={
                  virtualMessages.virtualized
                    ? ({ height: virtualMessages.totalHeight } as CSSProperties)
                    : undefined
                }
              >
                {virtualMessages.items.map(({ message: m, index: i, top }) => {
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
                  const row = (
                    <MessageRow
                      key={m.id ?? i}
                      activeId={activeId}
                      message={m}
                      index={i}
                      isEditing={editing === i}
                      isLastAssistant={isLastAssistant}
                      assistantStreaming={busy && isLastAssistant}
                      busy={busy}
                      activeMediaTargetPresent={Boolean(activeMediaTarget)}
                      folderIsEmpty={!folder.trim()}
                      activeRun={activeRun}
                      previewArtifacts={previewArtifactsForMessage(m)}
                      previewAppBusy={previewAppBusy}
                      previewAppStatus={activePreviewAppStatus}
                      toolApproval={toolApproval}
                      turnChanges={
                        isLastAssistant &&
                        turnChanges?.key === messageTurnChangesKey
                          ? turnChanges
                          : null
                      }
                      actionsRef={messageRowActionsRef}
                    />
                  );
                  return virtualMessages.virtualized ? (
                    <MessageVirtualRow
                      key={m.id ?? i}
                      index={i}
                      top={top}
                      measure={measureMessageRow}
                    >
                      {row}
                    </MessageVirtualRow>
                  ) : (
                    row
                  );
                })}
              </div>
            )}
          </div>

          <div className="dock">
            {emptyThread && showEmptyChatRidgeline && <MilimUsageRidgeline usage={milimUsage} />}
            {chatNotice && (
              <div
                className={`sheet-hint dock-notice ${chatNotice.tone}`}
                data-testid="chat-notice"
                role={chatNotice.tone === "error" ? "alert" : "status"}
                aria-live={chatNotice.tone === "error" ? "assertive" : "polite"}
              >
                {chatNotice.message}
              </div>
            )}
            <ComposerSurface>
              {pendingApprovals.map((approval) => (
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
                  onOpenBrowser={openArtifactBrowser}
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
                  onRuntimePreflight={() => void preflightPreviewRuntime()}
                  runtimeBusy={previewAppBusy != null}
                  onRuntimeStart={() => void startPreviewRuntime()}
                  onRuntimeStop={() => void stopPreviewRuntime()}
                  onRuntimeRestart={() => void restartPreviewRuntime()}
                  controlActivity={previewControlActivity}
                  onSurfaceChange={setActivePreviewSurface}
                  modeSwitcher={inspectorTabSwitcher}
                  workspaceFolder={folder}
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
