import { create } from "zustand";
import { persist, type PersistStorage } from "zustand/middleware";
import { createChatMessageId } from "../lib/messageIds.js";
import type {
  ChatArtifact,
  ChatAttachment,
  ChatMessage,
  ChatStreamPart,
  ChildThreadInfo,
  ChildThreadStatus,
  DelegationPolicy,
  MemoryNotice,
  ToolUiDescriptor,
  PreviewAppFile,
  PreviewAppError,
  PreviewAppKind,
  PreviewAppPreflight,
  PreviewAppState,
  PrivacyMode,
  ReasoningEffort,
  ResponseMetrics,
  RunTrace,
  SavedArtifactFile,
  ThreadEvent,
  ToolApprovalMode,
  Worker,
  WorkerRun,
  WorkerRunRecord,
} from "../api";
import {
  extractArtifactsFromMessage,
  normalizeArtifactDisposition,
  normalizeArtifactBrowserUrl,
} from "../lib/artifacts.js";
import {
  DEFAULT_GOAL_SETTINGS,
  normalizeGoalSettings,
  type GoalSettings,
} from "../lib/goals.js";
import { recordPerfMeasure, startPerfMeasure } from "../lib/perf.js";
import {
  normalizeGenerationOverrides,
  type GenerationOverrides,
} from "../lib/generationSettings.js";
import { previewRuntimeKeyForThread } from "../lib/previewRuntimeKeys.js";
import { normalizeProjectColor } from "../lib/projectColors.js";
import type { PullRequestSnapshot } from "../lib/pullRequests.js";
import {
  normalizeQuickSummarySectionIds,
  type QuickSummarySectionId,
} from "../lib/quickSummary.js";
import { normalizeReasoningEffortOverrides } from "../lib/reasoningEffort.js";
import { appendPhaseStreamPart, coalesceStreamPhases } from "../lib/streamParts.js";
import { deriveThreadTitle, NEW_CHAT_TITLE } from "../lib/threadTitles.js";
import {
  readUserStateKey,
  sessionStateStorage,
  writeUserStateKey,
} from "../persistence/userStateStorage.js";
import { useSettings } from "../settings/store.js";

const COMPOSER_DRAFTS_KEY = "milim.sessionDrafts";
const LEGACY_STREAMING_PERSIST_MARKER = "__deferStreamingWrite";
let sessionComposerDrafts: Record<string, string> = {};
let sessionComposerDraftsPersistTimer: ReturnType<typeof setTimeout> | null =
  null;

function persistSessionComposerDrafts(): void {
  if (sessionComposerDraftsPersistTimer != null)
    clearTimeout(sessionComposerDraftsPersistTimer);
  sessionComposerDraftsPersistTimer = setTimeout(() => {
    sessionComposerDraftsPersistTimer = null;
    void Promise.resolve(
      writeUserStateKey(COMPOSER_DRAFTS_KEY, JSON.stringify(sessionComposerDrafts)),
    ).catch(() => {});
  }, 250);
}

export function getSessionComposerDraft(sessionId: string): string {
  return sessionComposerDrafts[sessionId] ?? "";
}

export function setSessionComposerDraft(sessionId: string, draft: string): void {
  const next = draft.trim() ? draft : "";
  if (next) sessionComposerDrafts = { ...sessionComposerDrafts, [sessionId]: next };
  else {
    const { [sessionId]: _removed, ...rest } = sessionComposerDrafts;
    sessionComposerDrafts = rest;
  }
  persistSessionComposerDrafts();
}

export async function hydrateSessionComposerDraftsFromUserState(): Promise<void> {
  const raw = await Promise.resolve(readUserStateKey(COMPOSER_DRAFTS_KEY));
  if (!raw) return;
  try {
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return;
    const next: Record<string, string> = {};
    for (const [sessionId, draft] of Object.entries(parsed)) {
      if (typeof draft === "string" && draft.trim()) next[sessionId] = draft;
    }
    sessionComposerDrafts = next;
    if (typeof window !== "undefined")
      window.dispatchEvent(new Event("milim:session-drafts-hydrated"));
  } catch {
    sessionComposerDrafts = {};
  }
}

const sessionPersistStorage = {
  ...sessionStateStorage,
  setItem: (name, value) => {
    if (!useSessions.persist.hasHydrated()) return undefined;
    return sessionStateStorage.setItem(name, value);
  },
} satisfies typeof sessionStateStorage;

export interface ThreadSettings {
  model: string;
  instructions: string;
  folder: string;
  sandbox: boolean;
  computerUse: boolean;
  memory: boolean;
  activeAgentId: string | null;
  /** Outbound privacy gate for remote providers (off | redact | block). */
  privacy: PrivacyMode;
  /** Tool execution gate for shell/computer/sandbox actions. */
  toolApproval: ToolApprovalMode;
  /** Whether this thread may delegate work, and whether proposals pause. */
  delegationPolicy: DelegationPolicy;
  /** Optional model override used for managed workers. */
  workerModel: string;
  /** Read-only planning mode for implementation planning before execution. */
  planMode: boolean;
  /** Per-model reasoning effort chosen inside this thread; absent models inherit the app-wide default. */
  reasoningEffortOverrides?: Record<string, ReasoningEffort>;
  /** Per-model provider sampling overrides; absent values inherit model/server defaults. */
  generationOverrides?: GenerationOverrides;
  goal: GoalSettings;
}

export interface ThreadWorkspace {
  mode: "current" | "worktree";
  projectFolder: string;
  worktreeFolder: string;
  branch?: string;
  baseCommit?: string;
  createdAt: number;
}

export type ThreadSettingsPatch = Partial<Omit<ThreadSettings, "goal">> & {
  goal?: Partial<GoalSettings>;
  reasoningEffort?: ReasoningEffort;
  reasoningEffortByModel?: Record<string, ReasoningEffort>;
};

export interface Session {
  id: string;
  title: string;
  messages: ChatMessage[];
  /** False while only a tail page is resident in the renderer. */
  messagesHydrated?: boolean;
  persistedMessageCount?: number;
  messagesLoadedFrom?: number;
  messagePreview?: string;
  virtualFiles?: Record<string, SessionVirtualFile>;
  contextPanelOpen?: boolean;
  contextCollapsedSectionIds?: QuickSummarySectionId[];
  /** @deprecated Read for workspace-keyed Inspector migration only. */
  inspectorOpen?: boolean;
  /** @deprecated Read for workspace-keyed Inspector migration only. */
  inspectorTab?: SessionInspectorTab;
  browserSession?: SessionBrowserSession;
  /** @deprecated Read for one-release side-panel migration only. */
  artifactPanelOpen?: boolean;
  /** @deprecated Read for one-release side-panel migration only. */
  sidePanelMode?: SessionSidePanelMode;
  /** @deprecated Read for one-release side-panel migration only. */
  artifactPanelTab?: SessionArtifactPanelTab;
  previewRuntime?: SessionPreviewRuntime;
  settings?: ThreadSettings;
  threadWorkspace?: ThreadWorkspace;
  accountRuntime?: {
    codexThreadId?: string;
    codexLastSyncedMessageId?: string;
    claudeSessionId?: string;
    claudeLastSyncedMessageId?: string;
    opencodeSessionId?: string;
    opencodeLastSyncedMessageId?: string;
    piSessionId?: string;
    piLastSyncedMessageId?: string;
  };
  pendingHotSwap?: PendingHotSwap;
  retryWorkspace?: RetryWorkspace;
  /** Approved Worker Runs that still need to return results to this thread. */
  pendingWorkerRunIds?: string[];
  parentId?: string;
  worker?: {
    status: ChildThreadStatus;
    model: string;
    agentId?: string;
    summary?: string;
    error?: string;
  };
  createdAt: number;
  updatedAt: number;
  settledAt?: number;
  archivedAt?: number;
}

export interface SessionBrowserTab {
  id: string;
  url: string | null;
  input: string;
  history: string[];
  historyIndex: number;
  title?: string;
  faviconUrl?: string;
  muted?: boolean;
}

export interface SessionBrowserSession {
  profileId: string;
  tabs: SessionBrowserTab[];
  activeTabId: string;
}

export type SessionWorker = Worker & {
  messages?: ChatMessage[];
  events?: ThreadEvent[];
};

export type SessionWorkerRunRecord = Omit<WorkerRunRecord, "workers"> & {
  workers: SessionWorker[];
};

export type HotSwapAction = "switch" | "continue" | "review" | "retry";
export type NativeSessionMode = "fresh" | "resume";

export interface PendingHotSwap {
  fromModel: string;
  toModel: string;
  action: Exclude<HotSwapAction, "retry">;
  nativeSessionMode?: NativeSessionMode;
  sourceMessageId?: string;
  createdAt: number;
}

export interface RetryWorkspace {
  sourceSessionId: string;
  sourceMessageId?: string;
  originalFolder: string;
  worktreeFolder: string;
  baseCheckpoint: string;
  createdAt: number;
  adoptedAt?: number;
  applyUndoCheckpoint?: string;
}

export interface SessionVirtualFile {
  path: string;
  content: string;
  bytes: number;
  updatedAt: number;
  version: number;
  sourceMessageIndex?: number;
  sourceRevisionNumber?: number;
}

export type SessionSidePanelMode = "artifact" | "browser" | "git";
export type SessionArtifactPanelTab = "preview" | "code";
export type SessionInspectorTab = "preview" | "code" | "git" | "workers";
export type SessionInspectorState = {
  open: boolean;
  tab: SessionInspectorTab;
};

export interface SessionPreviewRuntime {
  kind?: PreviewAppKind | string;
  status: PreviewAppState | string;
  cwd?: string;
  url?: string;
  pid?: number;
  command?: string;
  message?: string;
  active?: boolean;
  ready?: boolean;
  managed?: boolean;
  runId?: string;
  error?: PreviewAppError;
  preflight?: PreviewAppPreflight;
  updatedAt?: number;
}

export type ArchiveRetentionDays = 7 | 14 | 30;

export const PROJECT_ICON_IDS = [
  "folder",
  "code",
  "terminal",
  "cube",
  "git-branch",
  "globe",
  "lightbulb",
  "image",
  "calendar",
  "file",
  "bolt",
  "star",
] as const;

export type ProjectIconId = (typeof PROJECT_ICON_IDS)[number];

export interface Project {
  id: string;
  name: string;
  folder: string;
  icon?: ProjectIconId;
  color?: string;
  createdAt: number;
  updatedAt: number;
  archivedAt?: number;
}

export const SIDEBAR_PINNED_SECTION_ID = "pinned";
export const SIDEBAR_CHATS_SECTION_ID = "chats";
export const SIDEBAR_PROJECT_SECTION_PREFIX = "project:";
export const DEFAULT_ARCHIVE_RETENTION_DAYS: ArchiveRetentionDays = 30;
export const DAY_MS = 24 * 60 * 60 * 1000;

export interface SessionSidebarState {
  collapsedSectionIds: string[];
  pinnedSessionIds: string[];
  pinnedSectionIds: string[];
  sessionOrder: string[];
  sectionOrder: string[];
  projectFolders: string[];
}

export interface QueuedMessage {
  id: string;
  content: string;
  attachments?: ChatAttachment[];
  createdAt: number;
  mailboxOrigin?: ChatMessage["mailboxOrigin"];
}

export const DEFAULT_THREAD_SETTINGS: ThreadSettings = {
  model: "",
  instructions: "",
  folder: "",
  sandbox: false,
  computerUse: false,
  memory: true,
  activeAgentId: null,
  privacy: "off",
  toolApproval: "review",
  delegationPolicy: "ask",
  workerModel: "",
  planMode: false,
  goal: DEFAULT_GOAL_SETTINGS,
};

const DEFAULT_SIDEBAR_STATE: SessionSidebarState = {
  collapsedSectionIds: [],
  pinnedSessionIds: [],
  pinnedSectionIds: [],
  sessionOrder: [],
  sectionOrder: [],
  projectFolders: [],
};

function uid(): string {
  return crypto.randomUUID();
}

function runtimeUuid(): string {
  return crypto.randomUUID();
}

function normalizeAccountRuntime(
  value: unknown,
): Session["accountRuntime"] | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const raw = value as Record<string, unknown>;
  const codexThreadId =
    typeof raw.codexThreadId === "string" && raw.codexThreadId.trim()
      ? raw.codexThreadId.trim()
      : undefined;
  const claudeSessionId =
    typeof raw.claudeSessionId === "string" && raw.claudeSessionId.trim()
      ? raw.claudeSessionId.trim()
      : undefined;
  const opencodeSessionId =
    typeof raw.opencodeSessionId === "string" && raw.opencodeSessionId.trim()
      ? raw.opencodeSessionId.trim()
      : undefined;
  const piSessionId =
    typeof raw.piSessionId === "string" && raw.piSessionId.trim()
      ? raw.piSessionId.trim()
      : undefined;
  const codexLastSyncedMessageId =
    codexThreadId &&
    typeof raw.codexLastSyncedMessageId === "string" &&
    raw.codexLastSyncedMessageId.trim()
      ? raw.codexLastSyncedMessageId.trim()
      : undefined;
  const claudeLastSyncedMessageId =
    claudeSessionId &&
    typeof raw.claudeLastSyncedMessageId === "string" &&
    raw.claudeLastSyncedMessageId.trim()
      ? raw.claudeLastSyncedMessageId.trim()
      : undefined;
  const opencodeLastSyncedMessageId =
    opencodeSessionId &&
    typeof raw.opencodeLastSyncedMessageId === "string" &&
    raw.opencodeLastSyncedMessageId.trim()
      ? raw.opencodeLastSyncedMessageId.trim()
      : undefined;
  const piLastSyncedMessageId =
    piSessionId &&
    typeof raw.piLastSyncedMessageId === "string" &&
    raw.piLastSyncedMessageId.trim()
      ? raw.piLastSyncedMessageId.trim()
      : undefined;
  return codexThreadId || claudeSessionId || opencodeSessionId || piSessionId
    ? {
        codexThreadId,
        codexLastSyncedMessageId,
        claudeSessionId,
        claudeLastSyncedMessageId,
        opencodeSessionId,
        opencodeLastSyncedMessageId,
        piSessionId,
        piLastSyncedMessageId,
      }
    : undefined;
}

function normalizePendingHotSwap(value: unknown): PendingHotSwap | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const raw = value as Record<string, unknown>;
  const action = raw.action;
  if (action !== "switch" && action !== "continue" && action !== "review") return undefined;
  if (typeof raw.fromModel !== "string" || typeof raw.toModel !== "string") return undefined;
  const fromModel = raw.fromModel.trim();
  const toModel = raw.toModel.trim();
  if (!fromModel || !toModel) return undefined;
  const nativeSessionMode = raw.nativeSessionMode === "fresh" || raw.nativeSessionMode === "resume"
    ? raw.nativeSessionMode
    : undefined;
  return {
    fromModel,
    toModel,
    action,
    nativeSessionMode,
    sourceMessageId: typeof raw.sourceMessageId === "string" && raw.sourceMessageId.trim()
      ? raw.sourceMessageId.trim()
      : undefined,
    createdAt: timestamp(raw.createdAt) ?? Date.now(),
  };
}

function normalizeRetryWorkspace(value: unknown): RetryWorkspace | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const raw = value as Record<string, unknown>;
  const required = ["sourceSessionId", "originalFolder", "worktreeFolder", "baseCheckpoint"] as const;
  if (required.some((key) => typeof raw[key] !== "string" || !(raw[key] as string).trim())) return undefined;
  return {
    sourceSessionId: (raw.sourceSessionId as string).trim(),
    sourceMessageId: typeof raw.sourceMessageId === "string" && raw.sourceMessageId.trim()
      ? raw.sourceMessageId.trim()
      : undefined,
    originalFolder: (raw.originalFolder as string).trim(),
    worktreeFolder: (raw.worktreeFolder as string).trim(),
    baseCheckpoint: (raw.baseCheckpoint as string).trim(),
    createdAt: timestamp(raw.createdAt) ?? Date.now(),
    adoptedAt: timestamp(raw.adoptedAt),
    applyUndoCheckpoint: typeof raw.applyUndoCheckpoint === "string" && raw.applyUndoCheckpoint.trim()
      ? raw.applyUndoCheckpoint.trim()
      : undefined,
  };
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function normalizePreviewRuntime(
  value: unknown,
): SessionPreviewRuntime | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const raw = value as Record<string, unknown>;
  const status = stringValue(raw.status);
  if (!status) return undefined;
  const pid =
    typeof raw.pid === "number" && Number.isFinite(raw.pid)
      ? raw.pid
      : undefined;
  return {
    kind: stringValue(raw.kind),
    status,
    cwd: stringValue(raw.cwd),
    url: stringValue(raw.url),
    pid,
    command: stringValue(raw.command),
    message: stringValue(raw.message),
    active: typeof raw.active === "boolean" ? raw.active : undefined,
    ready: typeof raw.ready === "boolean" ? raw.ready : undefined,
    managed: typeof raw.managed === "boolean" ? raw.managed : undefined,
    runId: stringValue(raw.runId),
    error:
      raw.error && typeof raw.error === "object" && !Array.isArray(raw.error)
        ? {
            code: stringValue((raw.error as Record<string, unknown>).code) ?? "preview_error",
            message:
              stringValue((raw.error as Record<string, unknown>).message) ??
              "Preview runtime failed.",
          }
        : undefined,
    preflight:
      raw.preflight &&
      typeof raw.preflight === "object" &&
      !Array.isArray(raw.preflight)
        ? (raw.preflight as PreviewAppPreflight)
        : undefined,
    updatedAt: timestamp(raw.updatedAt),
  };
}

function normalizePreviewRuntimesByKey(
  value: unknown,
): Record<string, SessionPreviewRuntime> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const runtimes: Record<string, SessionPreviewRuntime> = {};
  for (const [key, rawRuntime] of Object.entries(
    value as Record<string, unknown>,
  )) {
    if (!/^[A-Za-z0-9_.-]+$/.test(key)) continue;
    const runtime = normalizePreviewRuntime(rawRuntime);
    if (runtime) runtimes[key] = runtime;
  }
  return runtimes;
}

export function normalizeVirtualFilePath(path: string): string {
  const normalized = path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^(\.\/)+/, "")
    .replace(/\/+/g, "/");
  if (!normalized || normalized.startsWith("/") || normalized.includes(":"))
    return "";
  if (
    normalized.split("/").some((part) => !part || part === "." || part === "..")
  )
    return "";
  return normalized;
}

export function sessionVirtualProjectFiles(
  session?: Pick<Session, "virtualFiles"> | null,
): PreviewAppFile[] {
  return Object.values(session?.virtualFiles ?? {})
    .sort((left, right) => left.path.localeCompare(right.path))
    .map((file) => ({ path: file.path, content: file.content }));
}

function normalizeVirtualFiles(
  value: unknown,
): Record<string, SessionVirtualFile> | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const out: Record<string, SessionVirtualFile> = {};
  for (const raw of Object.values(value as Record<string, unknown>)) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const file = raw as Partial<SessionVirtualFile>;
    const path =
      typeof file.path === "string" ? normalizeVirtualFilePath(file.path) : "";
    const content = typeof file.content === "string" ? file.content : "";
    if (!path) continue;
    out[path] = {
      path,
      content,
      bytes: textBytes(content),
      updatedAt: timestamp(file.updatedAt) ?? Date.now(),
      version:
        typeof file.version === "number" &&
        Number.isFinite(file.version) &&
        file.version > 0
          ? Math.floor(file.version)
          : 1,
      sourceMessageIndex:
        typeof file.sourceMessageIndex === "number" &&
        Number.isFinite(file.sourceMessageIndex)
          ? file.sourceMessageIndex
          : undefined,
      sourceRevisionNumber:
        typeof file.sourceRevisionNumber === "number" &&
        Number.isFinite(file.sourceRevisionNumber)
          ? file.sourceRevisionNumber
          : undefined,
    };
  }
  return Object.keys(out).length ? out : undefined;
}

function textBytes(content: string): number {
  return new TextEncoder().encode(content).byteLength;
}

function samePreviewRuntime(
  a?: SessionPreviewRuntime,
  b?: SessionPreviewRuntime,
): boolean {
  return (
    (a?.kind ?? "app") === (b?.kind ?? "app") &&
    (a?.status ?? "") === (b?.status ?? "") &&
    (a?.cwd ?? "") === (b?.cwd ?? "") &&
    (a?.url ?? "") === (b?.url ?? "") &&
    (a?.pid ?? null) === (b?.pid ?? null) &&
    (a?.command ?? "") === (b?.command ?? "") &&
    (a?.message ?? "") === (b?.message ?? "") &&
    (a?.active ?? false) === (b?.active ?? false) &&
    (a?.ready ?? false) === (b?.ready ?? false) &&
    (a?.managed ?? false) === (b?.managed ?? false) &&
    (a?.runId ?? "") === (b?.runId ?? "") &&
    (a?.error?.code ?? "") === (b?.error?.code ?? "") &&
    (a?.error?.message ?? "") === (b?.error?.message ?? "") &&
    (a?.preflight?.source_fingerprint ?? "") ===
      (b?.preflight?.source_fingerprint ?? "")
  );
}

function normalizeInspectorTab(
  value: unknown,
  legacyMode?: unknown,
  legacyArtifactTab?: unknown,
): SessionInspectorTab | undefined {
  if (
    value === "preview" ||
    value === "code" ||
    value === "git" ||
    value === "workers"
  )
    return value;
  if (legacyMode === "git") return "git";
  if (legacyMode === "browser") return "preview";
  if (legacyArtifactTab === "code") return "code";
  if (legacyMode === "artifact") return "preview";
  return undefined;
}

const DEFAULT_INSPECTOR_STATE: SessionInspectorState = {
  open: false,
  tab: "preview",
};

function normalizeInspectorByKey(value: unknown): Record<string, SessionInspectorState> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const states: Record<string, SessionInspectorState> = {};
  for (const [key, raw] of Object.entries(value as Record<string, unknown>)) {
    if (!/^[A-Za-z0-9_.-]+$/.test(key) || !raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const entry = raw as Record<string, unknown>;
    states[key] = {
      open: entry.open === true,
      tab: normalizeInspectorTab(entry.tab) ?? "preview",
    };
  }
  return states;
}

function inspectorKeyForSession(session: Pick<Session, "id" | "settings">): string {
  return previewRuntimeKeyForThread(session.id, session.settings?.folder);
}

export function inspectorStateForSession(
  states: Record<string, SessionInspectorState>,
  session?: Pick<Session, "id" | "settings">,
): SessionInspectorState {
  return session ? states[inspectorKeyForSession(session)] ?? DEFAULT_INSPECTOR_STATE : DEFAULT_INSPECTOR_STATE;
}

function normalizeSettings(
  settings?: ThreadSettingsPatch,
  options: { pauseRunningGoal?: boolean } = {},
): ThreadSettings {
  const {
    reasoningEffort: _legacyEffort,
    reasoningEffortByModel: _legacyEffortByModel,
    ...settingsWithoutLegacyReasoning
  } = settings ?? {};
  const next: ThreadSettings = {
    ...DEFAULT_THREAD_SETTINGS,
    ...settingsWithoutLegacyReasoning,
    goal: normalizeGoalSettings(settings?.goal ?? DEFAULT_GOAL_SETTINGS, {
      pauseRunning: options.pauseRunningGoal,
    }),
  };
  if (next.model.trim().toLowerCase() === "mock-echo") {
    next.model = "";
  }
  if (
    next.toolApproval !== "review" &&
    next.toolApproval !== "guarded" &&
    next.toolApproval !== "open"
  ) {
    next.toolApproval = DEFAULT_THREAD_SETTINGS.toolApproval;
  }
  if (typeof next.planMode !== "boolean") {
    next.planMode = DEFAULT_THREAD_SETTINGS.planMode;
  }
  const reasoningEffortOverrides = normalizeReasoningEffortOverrides(next.reasoningEffortOverrides);
  if (Object.keys(reasoningEffortOverrides).length) {
    next.reasoningEffortOverrides = reasoningEffortOverrides;
  } else {
    delete next.reasoningEffortOverrides;
  }
  const generationOverrides = normalizeGenerationOverrides(next.generationOverrides);
  if (Object.keys(generationOverrides).length) next.generationOverrides = generationOverrides;
  else delete next.generationOverrides;
  if (
    next.delegationPolicy !== "off" &&
    next.delegationPolicy !== "ask" &&
    next.delegationPolicy !== "auto"
  ) {
    next.delegationPolicy = DEFAULT_THREAD_SETTINGS.delegationPolicy;
  }
  if (typeof next.workerModel !== "string") {
    next.workerModel = DEFAULT_THREAD_SETTINGS.workerModel;
  }
  return next;
}

function normalizeProjectFolder(folder?: string): string {
  return (folder ?? "").trim();
}

function resetApprovalForFolderChange(
  previousFolder: string | undefined,
  settings: ThreadSettings,
): ThreadSettings {
  return normalizeProjectFolder(previousFolder) ===
    normalizeProjectFolder(settings.folder)
    ? settings
    : { ...settings, toolApproval: DEFAULT_THREAD_SETTINGS.toolApproval };
}

function folderLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "Project";
}

function normalizeProjectIcon(value: unknown): ProjectIconId | undefined {
  return typeof value === "string" &&
    PROJECT_ICON_IDS.includes(value as ProjectIconId)
    ? value as ProjectIconId
    : undefined;
}

export function projectSectionId(folder?: string): string {
  const normalized = normalizeProjectFolder(folder);
  return normalized
    ? `${SIDEBAR_PROJECT_SECTION_PREFIX}${normalized}`
    : SIDEBAR_CHATS_SECTION_ID;
}

export function isSidebarProjectSectionId(id: string): boolean {
  return id.startsWith(SIDEBAR_PROJECT_SECTION_PREFIX);
}

export function folderFromProjectSectionId(id: string): string | null {
  if (id === SIDEBAR_CHATS_SECTION_ID) return "";
  if (isSidebarProjectSectionId(id))
    return id.slice(SIDEBAR_PROJECT_SECTION_PREFIX.length);
  return null;
}

function uniqueStrings(values: unknown): string[] {
  if (!Array.isArray(values)) return [];
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (typeof value !== "string") continue;
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    result.push(trimmed);
  }
  return result;
}

function orderedUnique(existing: string[], fallback: string[]): string[] {
  return uniqueStrings([...existing, ...fallback]);
}

type SidebarInsertPosition = "before" | "after" | "inside";

function moveInOrder(
  order: string[],
  id: string,
  targetId?: string | null,
  position: SidebarInsertPosition = "before",
): string[] {
  const next = order.filter((item) => item !== id);
  if (!targetId) return position === "after" ? [...next, id] : [id, ...next];
  const targetIndex = next.indexOf(targetId);
  if (targetIndex === -1)
    return position === "after" ? [...next, id] : [id, ...next];
  next.splice(position === "after" ? targetIndex + 1 : targetIndex, 0, id);
  return next;
}

function folderListFromSessions(sessions: Session[]): string[] {
  return uniqueStrings(
    sessions
      .filter((session) => !session.parentId)
      .map(sessionProjectFolder),
  );
}

export function sessionProjectFolder(session: Pick<Session, "settings" | "retryWorkspace" | "threadWorkspace">): string {
  return session.retryWorkspace?.originalFolder
    || (session.threadWorkspace?.mode === "worktree"
      ? session.threadWorkspace.projectFolder
      : session.settings?.folder)
    || session.settings?.folder
    || "";
}

function normalizeThreadWorkspace(value: unknown, effectiveFolder: string): ThreadWorkspace {
  const raw = value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
  const mode = raw.mode === "worktree" ? "worktree" : "current";
  const projectFolder = typeof raw.projectFolder === "string" && raw.projectFolder.trim()
    ? raw.projectFolder.trim()
    : effectiveFolder;
  const worktreeFolder = mode === "worktree" && typeof raw.worktreeFolder === "string" && raw.worktreeFolder.trim()
    ? raw.worktreeFolder.trim()
    : effectiveFolder;
  return {
    mode,
    projectFolder,
    worktreeFolder,
    ...(typeof raw.branch === "string" && raw.branch.trim() ? { branch: raw.branch.trim() } : {}),
    ...(typeof raw.baseCommit === "string" && raw.baseCommit.trim() ? { baseCommit: raw.baseCommit.trim() } : {}),
    createdAt: timestamp(raw.createdAt) ?? Date.now(),
  };
}

function timestamp(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function normalizeArchiveRetentionDays(value: unknown): ArchiveRetentionDays {
  return value === 7 || value === 14 || value === 30
    ? value
    : DEFAULT_ARCHIVE_RETENTION_DAYS;
}

function projectFromFolder(
  folder: string,
  existing?: Partial<Project>,
  now = Date.now(),
): Project {
  const normalized = normalizeProjectFolder(folder);
  const createdAt = timestamp(existing?.createdAt) ?? now;
  return {
    id: projectSectionId(normalized),
    name:
      typeof existing?.name === "string" && existing.name.trim()
        ? existing.name.trim()
        : folderLabel(normalized),
    folder: normalized,
    icon: normalizeProjectIcon(existing?.icon),
    color: normalizeProjectColor(existing?.color),
    createdAt,
    updatedAt: timestamp(existing?.updatedAt) ?? createdAt,
    archivedAt: timestamp(existing?.archivedAt),
  };
}

function normalizeProjects(
  value: unknown,
  sessions: Session[],
  legacyFolders: string[],
): Project[] {
  const now = Date.now();
  const rawProjects = Array.isArray(value) ? value : [];
  const byFolder = new Map<string, Partial<Project>>();
  for (const raw of rawProjects) {
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) continue;
    const project = raw as Partial<Project>;
    const folder = normalizeProjectFolder(project.folder);
    if (folder) byFolder.set(folder, project);
  }
  return uniqueStrings([
    ...byFolder.keys(),
    ...legacyFolders,
    ...folderListFromSessions(sessions),
  ]).map((folder) => projectFromFolder(folder, byFolder.get(folder), now));
}

function upsertProject(
  projects: Project[],
  folder: string,
  now = Date.now(),
): Project[] {
  const normalized = normalizeProjectFolder(folder);
  if (!normalized) return projects;
  const id = projectSectionId(normalized);
  if (projects.some((project) => project.id === id)) {
    return projects.map((project) =>
      project.id === id
        ? {
            ...project,
            folder: normalized,
            archivedAt: undefined,
            updatedAt: now,
          }
        : project,
    );
  }
  return [
    projectFromFolder(normalized, { createdAt: now, updatedAt: now }),
    ...projects,
  ];
}

function archivedProjectFolders(projects: Project[]): Set<string> {
  return new Set(
    projects
      .filter((project) => project.archivedAt)
      .map((project) => project.folder),
  );
}

function visibleSessions(sessions: Session[], projects: Project[]): Session[] {
  const archivedFolders = archivedProjectFolders(projects);
  return sessions.filter((session) => {
    if (session.archivedAt) return false;
    const folder = normalizeProjectFolder(sessionProjectFolder(session));
    return !folder || !archivedFolders.has(folder);
  });
}

function ensureVisibleActive(
  activeId: string,
  sessions: Session[],
  projects: Project[],
): { activeId: string; sessions: Session[] } {
  const visible = visibleSessions(sessions, projects);
  if (visible.some((session) => session.id === activeId))
    return { activeId, sessions };
  const nextActive = visible[0];
  if (nextActive) return { activeId: nextActive.id, sessions };
  const fresh = freshSession();
  return { activeId: fresh.id, sessions: [fresh, ...sessions] };
}

function sessionIdsInFolder(sessions: Session[], folder: string): Set<string> {
  const normalized = normalizeProjectFolder(folder);
  return new Set(
    sessions
      .filter(
        (session) =>
          normalizeProjectFolder(sessionProjectFolder(session)) === normalized,
      )
      .map((session) => session.id),
  );
}

function sectionIdsForState(
  sidebar: SessionSidebarState,
  sessions: Session[],
): string[] {
  const folders = orderedUnique(
    sidebar.projectFolders,
    folderListFromSessions(sessions),
  );
  return [
    SIDEBAR_PINNED_SECTION_ID,
    SIDEBAR_CHATS_SECTION_ID,
    ...folders.map((folder) => projectSectionId(folder)),
  ];
}

function projectSectionIdsForState(
  sidebar: SessionSidebarState,
  sessions: Session[],
): string[] {
  const folders = orderedUnique(
    sidebar.projectFolders,
    folderListFromSessions(sessions),
  );
  return folders.map((folder) => projectSectionId(folder));
}

function normalizeSidebarState(
  state: Partial<SessionSidebarState> | undefined,
  sessions: Session[],
): SessionSidebarState {
  const sessionIds = new Set(sessions.map((session) => session.id));
  const baseProjectFolders = orderedUnique(
    uniqueStrings(state?.projectFolders),
    folderListFromSessions(sessions),
  );
  const base: SessionSidebarState = {
    collapsedSectionIds: uniqueStrings(state?.collapsedSectionIds),
    pinnedSessionIds: uniqueStrings(state?.pinnedSessionIds).filter((id) =>
      sessionIds.has(id),
    ),
    pinnedSectionIds: uniqueStrings(state?.pinnedSectionIds),
    sessionOrder: orderedUnique(
      uniqueStrings(state?.sessionOrder).filter((id) => sessionIds.has(id)),
      sessions.map((session) => session.id),
    ),
    sectionOrder: uniqueStrings(state?.sectionOrder),
    projectFolders: baseProjectFolders,
  };
  const validSections = new Set(sectionIdsForState(base, sessions));
  const projectSections = new Set(projectSectionIdsForState(base, sessions));
  return {
    collapsedSectionIds: base.collapsedSectionIds.filter((id) =>
      validSections.has(id),
    ),
    pinnedSessionIds: base.pinnedSessionIds,
    pinnedSectionIds: base.pinnedSectionIds.filter((id) =>
      projectSections.has(id),
    ),
    sessionOrder: base.sessionOrder,
    sectionOrder: orderedUnique(
      base.sectionOrder.filter((id) => projectSections.has(id)),
      Array.from(projectSections),
    ),
    projectFolders: base.projectFolders,
  };
}

function freshSession(settings?: ThreadSettingsPatch, id = uid()): Session {
  const t = Date.now();
  const normalizedSettings = normalizeSettings(settings);
  return {
    id,
    title: "New chat",
    messages: [],
    settings: { ...normalizedSettings, goal: DEFAULT_GOAL_SETTINGS },
    threadWorkspace: normalizeThreadWorkspace(undefined, normalizedSettings.folder),
    createdAt: t,
    updatedAt: t,
  };
}

function cloneMessages(messages: ChatMessage[]): ChatMessage[] {
  try {
    return JSON.parse(JSON.stringify(messages)) as ChatMessage[];
  } catch {
    return messages.map((message) => ({ ...message }));
  }
}

function branchTitle(title: string): string {
  const base = title.trim() || NEW_CHAT_TITLE;
  return base === NEW_CHAT_TITLE ? "Chat branch" : `${base} branch`;
}

function importedTitle(title: unknown): string {
  return typeof title === "string" && title.trim()
    ? title.trim()
    : "Imported chat";
}

function importedMessages(value: unknown): ChatMessage[] {
  if (!Array.isArray(value)) return [];
  const out: ChatMessage[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const raw = item as Partial<ChatMessage>;
    if (typeof raw.role !== "string" || typeof raw.content !== "string")
      continue;
    out.push(
      normalizeMessageArtifacts({
        ...(JSON.parse(JSON.stringify(raw)) as ChatMessage),
        role: raw.role,
        content: raw.content,
      }),
    );
  }
  return out;
}

function ensureChatMessageId(message: ChatMessage): ChatMessage {
  return typeof message.id === "string" && message.id.trim()
    ? message
    : { ...message, id: createChatMessageId() };
}

function normalizeMessageArtifacts(message: ChatMessage): ChatMessage {
  const withId = ensureChatMessageId(message);
  if (withId.role !== "assistant") return withId;
  const artifacts = mergeArtifactState(
    extractArtifactsFromMessage(withId.content, withId.run),
    withId.artifacts,
  );
  return { ...withId, artifacts: artifacts.length ? artifacts : undefined };
}

function messagesWithUniqueIds(messages: ChatMessage[]): ChatMessage[] {
  const unique: ChatMessage[] = [];
  const indexById = new Map<string, number>();
  for (const message of messages) {
    const id = typeof message.id === "string" ? message.id.trim() : "";
    if (!id) {
      unique.push(message);
      continue;
    }
    const existingIndex = indexById.get(id);
    if (existingIndex === undefined) {
      indexById.set(id, unique.length);
      unique.push(message);
    } else {
      unique[existingIndex] = message;
    }
  }
  return unique;
}

function mergeArtifactState(
  artifacts: ChatArtifact[],
  previous?: ChatArtifact[],
): ChatArtifact[] {
  if (!previous?.length) return artifacts.map(normalizeArtifactDisposition);
  const savedById = new Map(
    previous
      .filter((artifact) => artifact.saved)
      .map((artifact) => [artifact.id, artifact.saved]),
  );
  return artifacts.map((artifact) => {
    const saved = savedById.get(artifact.id);
    return normalizeArtifactDisposition(
      saved ? { ...artifact, saved } : artifact,
    );
  });
}

type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;
export type BufferedStreamChunk = {
  kind: "text" | "thinking";
  content: string;
};

function appendTextStreamPart(
  parts: ChatStreamPart[] | undefined,
  token: string,
): ChatStreamPart[] {
  return appendPhaseStreamPart(parts, "text", token);
}

function appendThinkingStreamPart(
  parts: ChatStreamPart[] | undefined,
  token: string,
): ChatStreamPart[] {
  return appendPhaseStreamPart(parts, "thinking", token);
}

function appendEventStreamPart(
  parts: ChatStreamPart[] | undefined,
  part: ChatStreamEventPart,
): ChatStreamPart[] {
  return [...(parts ?? []), part];
}

function completeEventStreamPart(
  parts: ChatStreamPart[] | undefined,
  name: string,
  part: ChatStreamEventPart,
  callId?: string,
): ChatStreamPart[] {
  const next = parts ? parts.slice() : [];
  if (part.approvalId) {
    for (let i = next.length - 1; i >= 0; i -= 1) {
      const current = next[i];
      if (
        current.kind === "event" &&
        current.approvalId === part.approvalId
      ) {
        next[i] = {
          ...current,
          ...part,
          detail: part.detail ?? current.detail,
          approvalRequest: part.approvalRequest ?? current.approvalRequest,
        };
        return next;
      }
    }
  }
  if (callId) {
    for (let i = next.length - 1; i >= 0; i -= 1) {
      const current = next[i];
      if (
        current.kind === "event" &&
        current.eventType === "tool" &&
        current.status === "running" &&
        current.callId === callId
      ) {
        next[i] = {
          ...part,
          mcpApp: part.mcpApp ?? current.mcpApp,
          toolArguments: part.toolArguments ?? current.toolArguments,
        };
        return next;
      }
    }
  }
  for (let i = next.length - 1; i >= 0; i -= 1) {
    const current = next[i];
    if (
      current.kind === "event" &&
      current.eventType === "tool" &&
      current.status === "running" &&
      current.name === name
    ) {
      next[i] = {
        ...part,
        mcpApp: part.mcpApp ?? current.mcpApp,
        toolArguments: part.toolArguments ?? current.toolArguments,
      };
      return next;
    }
  }
  next.push(part);
  return next;
}

function appendStreamChunksToMessages(
  messages: ChatMessage[],
  messageId: string | undefined,
  chunks: BufferedStreamChunk[],
): ChatMessage[] {
  const cleanChunks = chunks.filter((chunk) => chunk.content.length > 0);
  if (cleanChunks.length === 0) return messages;
  const targetIndex = messageId
    ? messages.findIndex((message) => message.id === messageId)
    : messages.length - 1;
  if (targetIndex < 0) return messages;
  const next = messages.slice();
  const target = next[targetIndex];

  let content = target.content;
  let streamParts = target.streamParts;
  for (const chunk of cleanChunks) {
    if (chunk.kind === "text") {
      content += chunk.content;
      streamParts = appendTextStreamPart(streamParts, chunk.content);
    } else {
      streamParts = appendThinkingStreamPart(streamParts, chunk.content);
    }
  }

  next[targetIndex] = ensureChatMessageId({
    ...target,
    content,
    streamParts,
  });
  return next;
}

function normalizePersistedStreamParts(message: ChatMessage): ChatMessage {
  if (!message.streamParts?.length) return message;
  const streamParts = coalesceStreamPhases(message.streamParts);
  if (streamParts.some((part) => part.kind === "text")) {
    return streamParts === message.streamParts ? message : { ...message, streamParts };
  }
  if (!message.content.trim()) {
    return streamParts === message.streamParts ? message : { ...message, streamParts };
  }
  return {
    ...message,
    streamParts: [
      ...streamParts,
      { kind: "text", content: message.content },
    ],
  };
}

function normalizeStaleToolApprovals(message: ChatMessage): ChatMessage {
  const incomplete = (status?: string) =>
    status === "pending" || status === "decided" || status === "delivered";
  const hasPendingStep = message.run?.steps.some((step) => incomplete(step.approval?.status)) ?? false;
  const hasPendingPart = message.streamParts?.some(
    (part) => part.kind === "event" && incomplete(part.approvalStatus),
  ) ?? false;
  if (!hasPendingStep && !hasPendingPart) return message;
  return {
    ...message,
    run: message.run ? {
      ...message.run,
      steps: message.run.steps.map((step) => incomplete(step.approval?.status)
        ? { ...step, approval: { ...step.approval, status: "failed", resolvedAt: step.approval.resolvedAt ?? Date.now() } }
        : step),
    } : undefined,
    streamParts: message.streamParts?.map((part) =>
      part.kind === "event" && incomplete(part.approvalStatus)
        ? {
            ...part,
            label: "Tool approval interrupted",
            detail: "The runtime stopped before acknowledging this decision.",
            approvalStatus: "failed",
            status: "error",
          }
        : part
    ),
  };
}

function normalizeSessionArtifacts(session: Session): Session {
  const settledAt = timestamp(session.settledAt);
  const archivedAt = timestamp(session.archivedAt);
  const messages = Array.isArray(session.messages) ? session.messages : [];
  const {
    artifactPanelOpen: legacyOpen,
    sidePanelMode: legacyMode,
    artifactPanelTab: legacyArtifactTab,
    ...current
  } = session;
  const contextCollapsedSectionIds = normalizeQuickSummarySectionIds(
    session.contextCollapsedSectionIds,
  );
  return {
    ...current,
    virtualFiles: normalizeVirtualFiles(session.virtualFiles),
    settledAt,
    archivedAt,
    contextPanelOpen: session.contextPanelOpen === true ? true : undefined,
    contextCollapsedSectionIds: contextCollapsedSectionIds.length
      ? contextCollapsedSectionIds
      : undefined,
    inspectorOpen: session.inspectorOpen === true ||
        (session.inspectorOpen === undefined && legacyOpen === true)
      ? true
      : undefined,
    inspectorTab: normalizeInspectorTab(
      session.inspectorTab,
      legacyMode,
      legacyArtifactTab,
    ) ?? (legacyOpen === true ? "preview" : undefined),
    browserSession: normalizeBrowserSession(session.browserSession, session.id),
    previewRuntime: normalizePreviewRuntime(session.previewRuntime),
    threadWorkspace: normalizeThreadWorkspace(
      session.threadWorkspace,
      session.settings?.folder ?? "",
    ),
    accountRuntime: normalizeAccountRuntime(session.accountRuntime),
    pendingHotSwap: normalizePendingHotSwap(session.pendingHotSwap),
    retryWorkspace: normalizeRetryWorkspace(session.retryWorkspace),
    pendingWorkerRunIds: Array.isArray(session.pendingWorkerRunIds)
      ? Array.from(
          new Set(
            session.pendingWorkerRunIds.filter(
              (id) => typeof id === "string" && id.trim(),
            ),
          ),
        )
      : undefined,
    settings: normalizeSettings(session.settings, { pauseRunningGoal: true }),
    messages: messagesWithUniqueIds(
      messages.map((message) =>
        normalizeMessageArtifacts(
          normalizeStaleToolApprovals(normalizePersistedStreamParts(message)),
        ),
      ),
    ),
  };
}

function normalizeBrowserSession(
  value: unknown,
  sessionId: string,
): SessionBrowserSession | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const raw = value as Record<string, unknown>;
  const tabs: SessionBrowserTab[] = [];
  const ids = new Set<string>();
  for (const value of Array.isArray(raw.tabs) ? raw.tabs.slice(0, 24) : []) {
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    const tab = value as Record<string, unknown>;
    const id = typeof tab.id === "string" && /^[A-Za-z0-9_-]+$/.test(tab.id)
      ? tab.id
      : "";
    if (!id || ids.has(id)) continue;
    ids.add(id);
    const url = typeof tab.url === "string"
      ? normalizeArtifactBrowserUrl(tab.url)
      : null;
    const history = (Array.isArray(tab.history) ? tab.history : [])
      .flatMap((item) => typeof item === "string"
        ? [normalizeArtifactBrowserUrl(item)].filter((entry): entry is string => Boolean(entry))
        : [])
      .slice(-100);
    if (url && !history.length) history.push(url);
    const historyIndex = history.length
      ? Math.min(
          Math.max(
            typeof tab.historyIndex === "number" && Number.isFinite(tab.historyIndex)
              ? Math.floor(tab.historyIndex)
              : history.length - 1,
            0,
          ),
          history.length - 1,
        )
      : -1;
    tabs.push({
      id,
      url,
      input: typeof tab.input === "string" ? tab.input.slice(0, 4096) : (url ?? ""),
      history,
      historyIndex,
      title: typeof tab.title === "string" && tab.title.trim()
        ? tab.title.trim().slice(0, 512)
        : undefined,
      faviconUrl: typeof tab.faviconUrl === "string"
        ? normalizeArtifactBrowserUrl(tab.faviconUrl) ?? undefined
        : undefined,
      muted: tab.muted === true ? true : undefined,
    });
  }
  if (!tabs.length) return undefined;
  const fallbackProfileId = `session-${sessionId}`.replace(/[^A-Za-z0-9_-]/g, "-");
  const profileId = typeof raw.profileId === "string" &&
      /^[A-Za-z0-9_-]+$/.test(raw.profileId)
    ? raw.profileId
    : fallbackProfileId;
  const activeTabId = typeof raw.activeTabId === "string" &&
      tabs.some((tab) => tab.id === raw.activeTabId)
    ? raw.activeTabId
    : tabs[0].id;
  return { profileId, tabs, activeTabId };
}

function normalizePreviewBrowserSessionsByKey(
  value: unknown,
): Record<string, SessionBrowserSession> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const sessions: Record<string, SessionBrowserSession> = {};
  for (const [key, rawSession] of Object.entries(
    value as Record<string, unknown>,
  )) {
    if (!/^[A-Za-z0-9_.-]+$/.test(key)) continue;
    const session = normalizeBrowserSession(rawSession, `${key}-app`);
    if (session) sessions[key] = session;
  }
  return sessions;
}

function legacyWorkerRunFromSession(
  session: Session,
): SessionWorkerRunRecord | null {
  if (!session.worker || !session.parentId) return null;
  const user = session.messages.find((message) => message.role === "user");
  const thread: ChildThreadInfo = {
    id: session.id,
    parent_id: session.parentId,
    root_id: session.parentId,
    title: session.title || "Worker",
    status: session.worker.status,
    model: session.worker.model,
    agent_id: session.worker.agentId ?? null,
    prompt: user?.content ?? "",
    summary: session.worker.summary ?? null,
    error: session.worker.error ?? null,
    created_at: new Date(session.createdAt).toISOString(),
    updated_at: new Date(session.updatedAt).toISOString(),
    finished_at:
      session.worker.status === "queued" || session.worker.status === "running"
        ? null
        : new Date(session.updatedAt).toISOString(),
    runtime: "legacy",
    access: "read_only",
  };
  const record = singletonRunFromThread(session.parentId, thread);
  record.workers[0] = { ...record.workers[0], messages: session.messages };
  return record;
}

function messageForPersistence(
  message: ChatMessage,
  stripBody: boolean,
): ChatMessage {
  const streamParts = message.streamParts?.filter(
    (part) => part.kind !== "text",
  );
  if (
    !stripBody &&
    streamParts?.length === message.streamParts?.length
  ) {
    return message;
  }
  const next: ChatMessage = {
    ...message,
    content: stripBody ? "" : message.content,
  };
  if (streamParts?.length) next.streamParts = streamParts;
  else delete next.streamParts;
  return next;
}

function sessionsForPersistence(
  sessions: Session[],
  generatingSessionIds: string[],
): Session[] {
  const generating = new Set(generatingSessionIds);
  return sessions.map((session) => {
    const stripTrailingBody = generating.has(session.id);
    const uniqueMessages = messagesWithUniqueIds(session.messages);
    const messages = uniqueMessages.map((message, index) =>
      messageForPersistence(
        message,
        stripTrailingBody &&
          index === uniqueMessages.length - 1 &&
          message.role === "assistant",
      ),
    );
    return messages.length === session.messages.length &&
      messages.every((message, index) => message === session.messages[index])
      ? session
      : { ...session, messages };
  });
}

function normalizeQueuedMessagesBySession(
  value: unknown,
  sessions: Session[],
): Record<string, QueuedMessage[]> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const sessionIds = new Set(sessions.map((session) => session.id));
  const next: Record<string, QueuedMessage[]> = {};
  for (const [sessionId, rawItems] of Object.entries(
    value as Record<string, unknown>,
  )) {
    if (!sessionIds.has(sessionId) || !Array.isArray(rawItems)) continue;
    const items = rawItems.flatMap((item): QueuedMessage[] => {
      if (!item || typeof item !== "object" || Array.isArray(item)) return [];
      const queued = item as Partial<QueuedMessage>;
      const id =
        typeof queued.id === "string" && queued.id.trim() ? queued.id : uid();
      const content = typeof queued.content === "string" ? queued.content : "";
      const attachments = Array.isArray(queued.attachments)
        ? queued.attachments
        : undefined;
      const createdAt =
        typeof queued.createdAt === "number" &&
        Number.isFinite(queued.createdAt)
          ? queued.createdAt
          : Date.now();
      if (!content.trim() && !attachments?.length) return [];
      return [{ id, content, attachments, createdAt }];
    });
    if (items.length) next[sessionId] = items;
  }
  return next;
}

function withoutQueuedSessions(
  queue: Record<string, QueuedMessage[]>,
  removed: Set<string>,
): Record<string, QueuedMessage[]> {
  const next = { ...queue };
  for (const id of removed) delete next[id];
  return next;
}

function runStatusFromWorker(status: ChildThreadStatus): WorkerRun["status"] {
  if (status === "done") return "done";
  if (status === "stopped") return "stopped";
  if (status === "error") return "error";
  return "running";
}

function sessionWorkerFromThread(
  thread: ChildThreadInfo,
  existing?: SessionWorker,
  events?: ThreadEvent[],
): SessionWorker {
  return {
    ...thread,
    run_id: thread.run_id ?? existing?.run_id,
    runtime: thread.runtime ?? existing?.runtime ?? "legacy",
    access: thread.access ?? existing?.access ?? "read_only",
    messages: messagesFromThread(thread, existing?.messages, events),
    events: events ?? existing?.events,
  };
}

function singletonRunFromThread(
  parentId: string,
  thread: ChildThreadInfo,
  existing?: SessionWorkerRunRecord,
  events?: ThreadEvent[],
): SessionWorkerRunRecord {
  const runId = thread.run_id?.trim() || existing?.run.id || `legacy:${thread.id}`;
  const worker = sessionWorkerFromThread(
    { ...thread, run_id: runId, parent_id: parentId || thread.parent_id },
    existing?.workers.find((item) => item.id === thread.id),
    events,
  );
  const currentTask = existing?.run.tasks.find(
    (task) => task.id === thread.id || task.prompt === thread.prompt,
  );
  const task = currentTask ?? {
    id: thread.id,
    title: thread.title || "Worker",
    prompt: thread.prompt,
    role: null,
    agent_id: thread.agent_id ?? null,
    model: thread.model,
    access: worker.access,
  };
  const workers = [
    ...(existing?.workers.filter((item) => item.id !== worker.id) ?? []),
    worker,
  ];
  const statuses = workers.map((item) => runStatusFromWorker(item.status));
  const status = statuses.some((item) => item === "running")
    ? "running"
    : statuses.some((item) => item === "error")
      ? statuses.some((item) => item === "done")
        ? "partial"
        : "error"
      : statuses.every((item) => item === "done")
        ? "done"
        : statuses.some((item) => item === "stopped")
          ? "stopped"
          : runStatusFromWorker(thread.status);
  return {
    run: {
      id: runId,
      parent_thread_id: parentId || thread.parent_id,
      policy: existing?.run.policy ?? "auto",
      runtime: thread.runtime ?? existing?.run.runtime ?? "legacy",
      status,
      tasks: [
        ...(existing?.run.tasks.filter((item) => item.id !== task.id) ?? []),
        task,
      ],
      error: thread.error ?? existing?.run.error,
      created_at: existing?.run.created_at ?? thread.created_at,
      updated_at: thread.updated_at,
      finished_at: thread.finished_at ?? existing?.run.finished_at,
    },
    workers,
  };
}

function mergeWorkerRunRecord(
  existing: SessionWorkerRunRecord | undefined,
  record: WorkerRunRecord,
): SessionWorkerRunRecord {
  return {
    run: { ...existing?.run, ...record.run },
    workers: [
      ...(existing?.workers.filter(
        (current) => !record.workers.some((worker) => worker.id === current.id),
      ) ?? []),
      ...record.workers.map((worker) => {
        const current = existing?.workers.find((item) => item.id === worker.id);
        return sessionWorkerFromThread(worker, current, current?.events);
      }),
    ],
  };
}

function eventPayload(event: ThreadEvent): Record<string, unknown> {
  return event.payload &&
    typeof event.payload === "object" &&
    !Array.isArray(event.payload)
    ? (event.payload as Record<string, unknown>)
    : {};
}

function eventText(event: ThreadEvent, key: string): string {
  const value = eventPayload(event)[key];
  return typeof value === "string" ? value : "";
}

function eventMcpApp(event: ThreadEvent): ToolUiDescriptor | undefined {
  const value = eventPayload(event).mcp_app;
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const app = value as Record<string, unknown>;
  if (app.kind === "native_chart") return { kind: "native_chart" };
  if (
    typeof app.server_id !== "string" ||
    typeof app.resource_uri !== "string" ||
    !app.tool ||
    typeof app.tool !== "object" ||
    Array.isArray(app.tool)
  ) return undefined;
  return app as unknown as ToolUiDescriptor;
}

function childToolEventLabel(name: string, done: boolean): string {
  return done ? `Used ${name}` : `Using ${name}`;
}

function childToolEventDetail(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim().replace(/\s+/g, " ");
  return trimmed
    ? trimmed.length > 110
      ? trimmed.slice(0, 109) + "..."
      : trimmed
    : undefined;
}

function childStreamFromEvents(events?: ThreadEvent[]): {
  content: string;
  streamParts?: ChatStreamPart[];
} {
  let content = "";
  let streamParts: ChatStreamPart[] = [];
  for (const event of events ?? []) {
    if (event.kind === "token") {
      const text = eventText(event, "text");
      content += text;
      streamParts = appendTextStreamPart(streamParts, text);
    } else if (event.kind === "reasoning") {
      streamParts = appendThinkingStreamPart(
        streamParts,
        eventText(event, "text"),
      );
    } else if (event.kind === "tool_call") {
      const name = eventText(event, "name") || "tool";
      const callId = eventText(event, "call_id") || undefined;
      const argumentsText = eventText(event, "arguments") || undefined;
      streamParts = appendEventStreamPart(streamParts, {
        kind: "event",
        eventType: "tool",
        label: childToolEventLabel(name, false),
        detail: childToolEventDetail(argumentsText),
        name,
        callId,
        icon: "tool",
        status: "running",
        mcpApp: eventMcpApp(event),
        toolArguments: argumentsText,
      });
    } else if (event.kind === "tool_result") {
      const payload = eventPayload(event);
      const name = eventText(event, "name") || "tool";
      const callId = eventText(event, "call_id") || undefined;
      const started = [...streamParts].reverse().find(
        (part): part is ChatStreamEventPart =>
          part.kind === "event" &&
          part.eventType === "tool" &&
          part.status === "running" &&
          (callId ? part.callId === callId : part.name === name),
      );
      const result =
        payload.result &&
        typeof payload.result === "object" &&
        !Array.isArray(payload.result)
          ? (payload.result as Record<string, unknown>)
          : {};
      const error = typeof result.error === "string" ? result.error : "";
      streamParts = completeEventStreamPart(
        streamParts,
        name,
        {
          kind: "event",
          eventType: "tool",
          label: error ? `${name} failed` : childToolEventLabel(name, true),
          detail: childToolEventDetail(error),
          name,
          callId,
          icon: error ? "error" : "tool",
          status: error ? "error" : "done",
          mcpApp: eventMcpApp(event) ?? started?.mcpApp,
          mcpAppResult: payload.mcp_app_result,
          toolArguments: started?.toolArguments,
        },
        callId,
      );
    } else if (event.kind === "final" && !content.trim()) {
      content = eventText(event, "content");
      streamParts = appendTextStreamPart(streamParts, content);
    } else if (event.kind === "error") {
      const message = eventText(event, "message");
      streamParts = appendEventStreamPart(streamParts, {
        kind: "event",
        eventType: "error",
        label: "Error",
        detail: childToolEventDetail(message),
        icon: "error",
        status: "error",
      });
    }
  }
  return { content, streamParts: streamParts.length ? streamParts : undefined };
}

function messagesFromThread(
  thread: ChildThreadInfo,
  existing?: ChatMessage[],
  events?: ThreadEvent[],
): ChatMessage[] {
  const user = existing?.find((message) => message.role === "user") ?? {
    id: createChatMessageId(),
    role: "user",
    content: "",
  };
  const assistant = existing?.find((message) => message.role === "assistant");
  const stream = childStreamFromEvents(events);
  const streamParts = events ? stream.streamParts : assistant?.streamParts;
  const streamContent = stream.content.trim() ? stream.content : "";
  const content =
    streamContent ||
    thread.summary?.trim() ||
    thread.error?.trim() ||
    (thread.status === "queued" || thread.status === "running"
      ? "Working..."
      : "");
  return [
    { ...user, content: thread.prompt },
    ...(content || streamParts
      ? [
          {
            ...(assistant ?? {
              id: createChatMessageId(),
              role: "assistant",
              content: "",
            }),
            content,
            streamParts,
          },
        ]
      : []),
  ];
}

interface SessionState {
  sessions: Session[];
  workerRuns: SessionWorkerRunRecord[];
  projects: Project[];
  previewRuntimesByKey: Record<string, SessionPreviewRuntime>;
  previewBrowserSessionsByKey: Record<string, SessionBrowserSession>;
  inspectorByKey: Record<string, SessionInspectorState>;
  pullRequestsBySession: Record<string, PullRequestSnapshot>;
  activeId: string;
  archiveRetentionDays: ArchiveRetentionDays;
  generatingSessionIds: string[];
  hostBusySessionIds: string[];
  activityBusySessionIds: string[];
  unreadSessionIds: string[];
  queuedMessagesBySession: Record<string, QueuedMessage[]>;
  sidebar: SessionSidebarState;
  newChat: (settings?: ThreadSettingsPatch, id?: string) => string;
  getNewUserChatSettings: (settings?: ThreadSettingsPatch) => ThreadSettings;
  newUserChat: (settings?: ThreadSettingsPatch, id?: string) => string;
  forkSession: (id: string, throughMessageIndex?: number) => string | null;
  importSession: (
    session: Partial<Omit<Session, "settings">> & {
      settings?: ThreadSettingsPatch;
    },
  ) => string | null;
  switchTo: (id: string) => void;
  rename: (id: string, title: string) => void;
  remove: (id: string) => void;
  settleSession: (id: string) => void;
  unsettleSession: (id: string) => void;
  archiveSession: (id: string) => void;
  restoreSession: (id: string) => void;
  archiveProject: (id: string) => void;
  restoreProject: (id: string) => void;
  removeProject: (id: string) => void;
  updateProject: (
    id: string,
    patch: Partial<Pick<Project, "name" | "icon" | "color">>,
  ) => void;
  purgeExpiredArchives: (now?: number) => void;
  setArchiveRetentionDays: (days: ArchiveRetentionDays) => void;
  setSessionGenerating: (id: string, generating: boolean) => void;
  setHostBusySessionIds: (ids: readonly string[]) => void;
  setSessionHostBusy: (id: string, busy: boolean) => void;
  setSessionActivityBusy: (id: string, busy: boolean) => void;
  setSessionUnread: (id: string, unread: boolean) => void;
  enqueueQueuedMessage: (
    id: string,
    message: { content: string; attachments?: ChatAttachment[] },
  ) => QueuedMessage;
  updateQueuedMessage: (
    id: string,
    messageId: string,
    patch: Partial<Pick<QueuedMessage, "content" | "attachments">>,
  ) => void;
  moveQueuedMessage: (
    id: string,
    messageId: string,
    targetId: string,
    position?: Exclude<SidebarInsertPosition, "inside">,
  ) => void;
  removeQueuedMessage: (id: string, messageId: string) => void;
  shiftQueuedMessage: (id: string) => QueuedMessage | null;
  clearQueuedMessages: (id: string) => void;
  addProjectFolder: (folder: string) => void;
  setSessionFolder: (id: string, folder: string) => void;
  setThreadWorkspace: (id: string, workspace: ThreadWorkspace) => void;
  toggleSessionPinned: (id: string) => void;
  toggleSidebarSectionCollapsed: (id: string) => void;
  toggleSidebarSectionPinned: (id: string) => void;
  moveSidebarSection: (
    id: string,
    targetId?: string | null,
    position?: SidebarInsertPosition,
  ) => void;
  moveSessionInSidebar: (
    id: string,
    targetId?: string | null,
    targetSectionId?: string | null,
    position?: SidebarInsertPosition,
  ) => void;
  setMessages: (
    id: string,
    messages: ChatMessage[],
    options?: { autoTitle?: boolean },
  ) => void;
  prependMessagePage: (
    id: string,
    messages: ChatMessage[],
    firstIndex: number,
    total: number,
  ) => void;
  appendStreamChunks: (
    id: string,
    messageIdOrChunks: string | BufferedStreamChunk[],
    chunks?: BufferedStreamChunk[],
  ) => void;
  finalizeMessageArtifacts: (id: string, messageId?: string) => void;
  appendStreamEvent: (
    id: string,
    messageIdOrPart: string | ChatStreamEventPart,
    part?: ChatStreamEventPart,
  ) => void;
  completeStreamEvent: (
    id: string,
    messageIdOrName: string,
    nameOrPart: string | ChatStreamEventPart,
    partOrCallId?: ChatStreamEventPart | string,
    callId?: string,
  ) => void;
  commitRun: (
    id: string,
    messageIdOrRun: string | RunTrace,
    run?: RunTrace,
  ) => void;
  commitResponseMetrics: (
    id: string,
    messageIdOrMetrics: string | ResponseMetrics,
    metrics?: ResponseMetrics,
  ) => void;
  appendMemoryNotice: (
    id: string,
    messageIdOrNotice: string | MemoryNotice,
    notice?: MemoryNotice,
  ) => void;
  upsertChildThread: (
    parentId: string,
    thread: ChildThreadInfo,
    events?: ThreadEvent[],
  ) => void;
  updateChildThread: (thread: ChildThreadInfo, events?: ThreadEvent[]) => void;
  upsertWorkerRun: (record: WorkerRunRecord) => void;
  removeWorkerRun: (runId: string) => void;
  setWorkerRunPending: (sessionId: string, runId: string, pending: boolean) => void;
  setWorkerRunEvent: (runId: string, event: ThreadEvent) => void;
  markArtifactSaved: (
    id: string,
    messageIdOrIndex: string | number,
    artifactId: string,
    saved: SavedArtifactFile,
  ) => void;
  upsertVirtualFiles: (
    id: string,
    files: PreviewAppFile[],
    source?: Pick<
      SessionVirtualFile,
      "sourceMessageIndex" | "sourceRevisionNumber"
    >,
  ) => void;
  setContextPanelOpen: (id: string, open: boolean) => void;
  setContextSectionCollapsed: (
    id: string,
    sectionId: QuickSummarySectionId,
    collapsed: boolean,
  ) => void;
  setInspectorOpen: (id: string, open: boolean) => void;
  setInspectorTab: (id: string, tab: SessionInspectorTab) => void;
  setBrowserSession: (id: string, session: SessionBrowserSession) => void;
  setPreviewBrowserSessionByKey: (
    key: string,
    session: SessionBrowserSession,
  ) => void;
  setPreviewRuntime: (
    id: string,
    runtime: SessionPreviewRuntime | undefined,
  ) => void;
  setPreviewRuntimeByKey: (
    key: string,
    runtime: SessionPreviewRuntime | undefined,
  ) => void;
  setSessionPullRequest: (
    id: string,
    snapshot: PullRequestSnapshot | undefined,
  ) => void;
  setAccountRuntime: (
    id: string,
    runtime: Partial<NonNullable<Session["accountRuntime"]>>,
  ) => void;
  clearAccountRuntime: (id: string) => void;
  clearAccountRuntimeKind: (
    id: string,
    kind: "codex" | "claude" | "opencode" | "pi",
  ) => void;
  setPendingHotSwap: (id: string, pending?: PendingHotSwap) => void;
  setRetryWorkspace: (id: string, retry?: RetryWorkspace) => void;
  ensureClaudeSessionId: (id: string) => string;
  getSettings: (id: string) => ThreadSettings;
  updateSettings: (id: string, settings: ThreadSettingsPatch) => void;
}

export const useSessions = create<SessionState>()(
  persist(
    (set, get) => {
      const first = freshSession();
      return {
        sessions: [first],
        workerRuns: [],
        projects: [],
        previewRuntimesByKey: {},
        previewBrowserSessionsByKey: {},
        inspectorByKey: {},
        pullRequestsBySession: {},
        activeId: first.id,
        archiveRetentionDays: DEFAULT_ARCHIVE_RETENTION_DAYS,
        generatingSessionIds: [],
        hostBusySessionIds: [],
        activityBusySessionIds: [],
        unreadSessionIds: [],
        queuedMessagesBySession: {},
        sidebar: normalizeSidebarState(DEFAULT_SIDEBAR_STATE, [first]),

        newChat: (settings, id) => {
          const cur = get().sessions.find((s) => s.id === get().activeId);
          const existing = id
            ? get().sessions.find((session) => session.id === id)
            : undefined;
          const normalizedSettings = normalizeSettings(
            settings ?? cur?.settings,
          );
          const settingsAnchor = existing ?? cur;
          const nextSettings = settingsAnchor
            ? resetApprovalForFolderChange(
                settingsAnchor.settings?.folder,
                normalizedSettings,
              )
            : normalizedSettings;
          const reusable = existing ?? (!id && cur?.messages.length === 0 ? cur : undefined);
          if (reusable) {
            if (settings && reusable.messages.length === 0) {
              get().updateSettings(reusable.id, nextSettings);
            }
            if (reusable.id !== get().activeId) get().switchTo(reusable.id);
            return reusable.id;
          }
          const s = freshSession(nextSettings, id);
          set((st) => ({
            sessions: [s, ...st.sessions],
            projects: upsertProject(st.projects, nextSettings.folder),
            activeId: s.id,
            sidebar: normalizeSidebarState(
              {
                ...st.sidebar,
                projectFolders: nextSettings.folder
                  ? [nextSettings.folder, ...st.sidebar.projectFolders]
                  : st.sidebar.projectFolders,
                sessionOrder: [s.id, ...st.sidebar.sessionOrder],
              },
              [s, ...st.sessions],
            ),
          }));
          return s.id;
        },

        getNewUserChatSettings: (settings) => {
          const current = get().sessions.find((session) => session.id === get().activeId);
          const preferences = useSettings.getState();
          const base: ThreadSettingsPatch = preferences.newThreadBehavior === "configured"
            ? {
                ...DEFAULT_THREAD_SETTINGS,
                ...preferences.configuredThreadDefaults,
                folder: current?.settings?.folder ?? "",
              }
            : normalizeSettings(current?.settings);
          const newThreadBase = { ...base };
          delete newThreadBase.reasoningEffortOverrides;
          delete newThreadBase.generationOverrides;
          return resetApprovalForFolderChange(current?.settings?.folder, normalizeSettings({
            ...newThreadBase,
            ...settings,
            instructions: "",
            computerUse: false,
            planMode: false,
            goal: DEFAULT_GOAL_SETTINGS,
          }));
        },

        newUserChat: (settings, id) => {
          return get().newChat(get().getNewUserChatSettings(settings), id);
        },

        forkSession: (id, throughMessageIndex) => {
          let forkId: string | null = null;
          set((st) => {
            const source = st.sessions.find((session) => session.id === id);
            if (!source) return {};
            const end =
              throughMessageIndex == null
                ? source.messages.length
                : Math.max(
                    0,
                    Math.min(source.messages.length, throughMessageIndex + 1),
                  );
            const messages = cloneMessages(source.messages.slice(0, end)).map(
              normalizeMessageArtifacts,
            );
            const now = Date.now();
            const fork: Session = {
              id: uid(),
              title: branchTitle(source.title),
              messages,
              settings: normalizeSettings(source.settings, {
                pauseRunningGoal: true,
              }),
              parentId: source.id,
              createdAt: now,
              updatedAt: now,
            };
            forkId = fork.id;
            return {
              sessions: [fork, ...st.sessions],
              projects: upsertProject(st.projects, fork.settings?.folder ?? ""),
              activeId: fork.id,
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: fork.settings?.folder
                    ? [fork.settings.folder, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                  sessionOrder: [fork.id, ...st.sidebar.sessionOrder],
                },
                [fork, ...st.sessions],
              ),
            };
          });
          return forkId;
        },

        importSession: (session) => {
          let importedId: string | null = null;
          set((st) => {
            const settings = normalizeSettings(session.settings, {
              pauseRunningGoal: true,
            });
            const messages = importedMessages(session.messages);
            const now = Date.now();
            const imported: Session = {
              id: uid(),
              title: importedTitle(session.title),
              messages,
              settings,
              createdAt: now,
              updatedAt: now,
            };
            importedId = imported.id;
            return {
              sessions: [imported, ...st.sessions],
              projects: upsertProject(st.projects, settings.folder),
              activeId: imported.id,
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: settings.folder
                    ? [settings.folder, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                  sessionOrder: [imported.id, ...st.sidebar.sessionOrder],
                },
                [imported, ...st.sessions],
              ),
            };
          });
          return importedId;
        },

        switchTo: (id) =>
          set((st) => ({
            activeId: id,
            unreadSessionIds: st.unreadSessionIds.filter(
              (unreadId) => unreadId !== id,
            ),
          })),

        rename: (id, title) =>
          set((st) => ({
            sessions: st.sessions.map((s) =>
              s.id === id ? { ...s, title: title || "Untitled" } : s,
            ),
          })),

        remove: (id) =>
          set((st) => {
            const removed = new Set([id]);
            let changed = true;
            while (changed) {
              changed = false;
              for (const session of st.sessions) {
                if (
                  session.parentId &&
                  removed.has(session.parentId) &&
                  !removed.has(session.id)
                ) {
                  removed.add(session.id);
                  changed = true;
                }
              }
            }
            const sessions = st.sessions.filter((s) => !removed.has(s.id));
            if (sessions.length === 0) {
              const s = freshSession();
              return {
                sessions: [s],
                projects: st.projects,
                activeId: s.id,
                generatingSessionIds: [],
                hostBusySessionIds: [],
                activityBusySessionIds: [],
                unreadSessionIds: [],
                queuedMessagesBySession: {},
                sidebar: normalizeSidebarState(DEFAULT_SIDEBAR_STATE, [s]),
              };
            }
            const active = ensureVisibleActive(
              st.activeId,
              sessions,
              st.projects,
            );
            const activeId = active.activeId;
            return {
              sessions: active.sessions,
              activeId,
              generatingSessionIds: st.generatingSessionIds.filter(
                (runningId) =>
                  !removed.has(runningId) &&
                  active.sessions.some((s) => s.id === runningId),
              ),
              hostBusySessionIds: st.hostBusySessionIds.filter(
                (runningId) =>
                  !removed.has(runningId) &&
                  active.sessions.some((s) => s.id === runningId),
              ),
              activityBusySessionIds: st.activityBusySessionIds.filter(
                (runningId) =>
                  !removed.has(runningId) &&
                  active.sessions.some((s) => s.id === runningId),
              ),
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) =>
                  !removed.has(unreadId) &&
                  unreadId !== activeId &&
                  active.sessions.some((s) => s.id === unreadId),
              ),
              queuedMessagesBySession: withoutQueuedSessions(
                st.queuedMessagesBySession,
                removed,
              ),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  pinnedSessionIds: st.sidebar.pinnedSessionIds.filter(
                    (pinnedId) => !removed.has(pinnedId),
                  ),
                  sessionOrder: st.sidebar.sessionOrder.filter(
                    (orderedId) => !removed.has(orderedId),
                  ),
                },
                active.sessions,
              ),
            };
          }),

        settleSession: (id) =>
          set((st) => ({
            sessions: st.sessions.map((session) =>
              session.id === id && !session.archivedAt
                ? {
                    ...session,
                    settledAt: session.settledAt ?? Date.now(),
                  }
                : session,
            ),
          })),

        unsettleSession: (id) =>
          set((st) => ({
            sessions: st.sessions.map((session) =>
              session.id === id && session.settledAt
                ? { ...session, settledAt: undefined }
                : session,
            ),
          })),

        archiveSession: (id) =>
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            const archived = new Set([id]);
            let changed = true;
            while (changed) {
              changed = false;
              for (const session of st.sessions) {
                if (
                  session.parentId &&
                  archived.has(session.parentId) &&
                  !archived.has(session.id)
                ) {
                  archived.add(session.id);
                  changed = true;
                }
              }
            }
            const now = Date.now();
            const sessions = st.sessions.map((session) =>
              archived.has(session.id)
                ? {
                    ...session,
                    archivedAt: session.archivedAt ?? now,
                    updatedAt: now,
                  }
                : session,
            );
            const active = ensureVisibleActive(
              st.activeId,
              sessions,
              st.projects,
            );
            return {
              sessions: active.sessions,
              activeId: active.activeId,
              generatingSessionIds: st.generatingSessionIds.filter(
                (runningId) => !archived.has(runningId),
              ),
              hostBusySessionIds: st.hostBusySessionIds.filter(
                (runningId) => !archived.has(runningId),
              ),
              activityBusySessionIds: st.activityBusySessionIds.filter(
                (runningId) => !archived.has(runningId),
              ),
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) => !archived.has(unreadId),
              ),
              queuedMessagesBySession: withoutQueuedSessions(
                st.queuedMessagesBySession,
                archived,
              ),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  pinnedSessionIds: st.sidebar.pinnedSessionIds.filter(
                    (pinnedId) => !archived.has(pinnedId),
                  ),
                },
                active.sessions,
              ),
            };
          }),

        restoreSession: (id) =>
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            const restored = new Set([id]);
            let changed = true;
            while (changed) {
              changed = false;
              for (const session of st.sessions) {
                if (
                  session.parentId &&
                  restored.has(session.parentId) &&
                  !restored.has(session.id)
                ) {
                  restored.add(session.id);
                  changed = true;
                }
              }
            }
            const sessions = st.sessions.map((session) =>
              restored.has(session.id)
                ? { ...session, archivedAt: undefined, updatedAt: Date.now() }
                : session,
            );
            const active = ensureVisibleActive(
              st.activeId,
              sessions,
              st.projects,
            );
            return {
              sessions: active.sessions,
              activeId: active.activeId,
              sidebar: normalizeSidebarState(st.sidebar, active.sessions),
            };
          }),

        archiveProject: (id) =>
          set((st) => {
            const project = st.projects.find((item) => item.id === id);
            if (!project) return {};
            const now = Date.now();
            const projects = st.projects.map((item) =>
              item.id === id
                ? {
                    ...item,
                    archivedAt: item.archivedAt ?? now,
                    updatedAt: now,
                  }
                : item,
            );
            const hidden = sessionIdsInFolder(st.sessions, project.folder);
            const active = ensureVisibleActive(
              st.activeId,
              st.sessions,
              projects,
            );
            return {
              sessions: active.sessions,
              projects,
              activeId: active.activeId,
              generatingSessionIds: st.generatingSessionIds.filter(
                (runningId) => !hidden.has(runningId),
              ),
              hostBusySessionIds: st.hostBusySessionIds.filter(
                (runningId) => !hidden.has(runningId),
              ),
              activityBusySessionIds: st.activityBusySessionIds.filter(
                (runningId) => !hidden.has(runningId),
              ),
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) => !hidden.has(unreadId),
              ),
              queuedMessagesBySession: withoutQueuedSessions(
                st.queuedMessagesBySession,
                hidden,
              ),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  collapsedSectionIds: st.sidebar.collapsedSectionIds.filter(
                    (sectionId) => sectionId !== id,
                  ),
                  pinnedSectionIds: st.sidebar.pinnedSectionIds.filter(
                    (sectionId) => sectionId !== id,
                  ),
                },
                active.sessions,
              ),
            };
          }),

        restoreProject: (id) =>
          set((st) => {
            if (!st.projects.some((project) => project.id === id)) return {};
            const projects = st.projects.map((project) =>
              project.id === id
                ? { ...project, archivedAt: undefined, updatedAt: Date.now() }
                : project,
            );
            const active = ensureVisibleActive(
              st.activeId,
              st.sessions,
              projects,
            );
            return {
              projects,
              sessions: active.sessions,
              activeId: active.activeId,
              sidebar: normalizeSidebarState(st.sidebar, active.sessions),
            };
          }),

        updateProject: (id, patch) =>
          set((st) => {
            const project = st.projects.find((item) => item.id === id);
            if (!project) return {};
            const icon = Object.prototype.hasOwnProperty.call(patch, "icon")
              ? normalizeProjectIcon(patch.icon)
              : project.icon;
            const color = Object.prototype.hasOwnProperty.call(patch, "color")
              ? normalizeProjectColor(patch.color)
              : project.color;
            const name = Object.prototype.hasOwnProperty.call(patch, "name")
              ? patch.name?.trim() || folderLabel(project.folder)
              : project.name;
            return {
              projects: st.projects.map((item) =>
                item.id === id
                  ? {
                      ...item,
                      name,
                      icon,
                      color,
                      updatedAt: Date.now(),
                    }
                  : item,
              ),
            };
          }),

        removeProject: (id) =>
          set((st) => {
            const project = st.projects.find((item) => item.id === id);
            if (!project) return {};
            const removed = sessionIdsInFolder(st.sessions, project.folder);
            let changed = true;
            while (changed) {
              changed = false;
              for (const session of st.sessions) {
                if (
                  session.parentId &&
                  removed.has(session.parentId) &&
                  !removed.has(session.id)
                ) {
                  removed.add(session.id);
                  changed = true;
                }
              }
            }
            const projects = st.projects.filter((item) => item.id !== id);
            const sessions = st.sessions.filter(
              (session) => !removed.has(session.id),
            );
            const active = ensureVisibleActive(st.activeId, sessions, projects);
            return {
              projects,
              sessions: active.sessions,
              activeId: active.activeId,
              generatingSessionIds: st.generatingSessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              hostBusySessionIds: st.hostBusySessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              activityBusySessionIds: st.activityBusySessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) => !removed.has(unreadId),
              ),
              queuedMessagesBySession: withoutQueuedSessions(
                st.queuedMessagesBySession,
                removed,
              ),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  collapsedSectionIds: st.sidebar.collapsedSectionIds.filter(
                    (sectionId) => sectionId !== id,
                  ),
                  pinnedSectionIds: st.sidebar.pinnedSectionIds.filter(
                    (sectionId) => sectionId !== id,
                  ),
                  projectFolders: st.sidebar.projectFolders.filter(
                    (folder) =>
                      normalizeProjectFolder(folder) !== project.folder,
                  ),
                  sessionOrder: st.sidebar.sessionOrder.filter(
                    (sessionId) => !removed.has(sessionId),
                  ),
                },
                active.sessions,
              ),
            };
          }),

        purgeExpiredArchives: (now = Date.now()) =>
          set((st) => {
            const cutoff = now - st.archiveRetentionDays * DAY_MS;
            const expiredProjectIds = new Set(
              st.projects
                .filter((project) => (project.archivedAt ?? Infinity) <= cutoff)
                .map((project) => project.id),
            );
            const expiredProjectFolders = new Set(
              st.projects
                .filter((project) => expiredProjectIds.has(project.id))
                .map((project) => project.folder),
            );
            const removed = new Set(
              st.sessions
                .filter(
                  (session) =>
                    (session.archivedAt ?? Infinity) <= cutoff ||
                    expiredProjectFolders.has(
                      normalizeProjectFolder(sessionProjectFolder(session)),
                    ),
                )
                .map((session) => session.id),
            );
            let changed = true;
            while (changed) {
              changed = false;
              for (const session of st.sessions) {
                if (
                  session.parentId &&
                  removed.has(session.parentId) &&
                  !removed.has(session.id)
                ) {
                  removed.add(session.id);
                  changed = true;
                }
              }
            }
            if (expiredProjectIds.size === 0 && removed.size === 0) return {};
            const projects = st.projects.filter(
              (project) => !expiredProjectIds.has(project.id),
            );
            const sessions = st.sessions.filter(
              (session) => !removed.has(session.id),
            );
            const active = ensureVisibleActive(st.activeId, sessions, projects);
            return {
              projects,
              sessions: active.sessions,
              activeId: active.activeId,
              generatingSessionIds: st.generatingSessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              hostBusySessionIds: st.hostBusySessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              activityBusySessionIds: st.activityBusySessionIds.filter(
                (runningId) => !removed.has(runningId),
              ),
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) => !removed.has(unreadId),
              ),
              queuedMessagesBySession: withoutQueuedSessions(
                st.queuedMessagesBySession,
                removed,
              ),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  collapsedSectionIds: st.sidebar.collapsedSectionIds.filter(
                    (sectionId) => !expiredProjectIds.has(sectionId),
                  ),
                  pinnedSectionIds: st.sidebar.pinnedSectionIds.filter(
                    (sectionId) => !expiredProjectIds.has(sectionId),
                  ),
                  projectFolders: st.sidebar.projectFolders.filter(
                    (folder) =>
                      !expiredProjectFolders.has(
                        normalizeProjectFolder(folder),
                      ),
                  ),
                  pinnedSessionIds: st.sidebar.pinnedSessionIds.filter(
                    (sessionId) => !removed.has(sessionId),
                  ),
                  sessionOrder: st.sidebar.sessionOrder.filter(
                    (sessionId) => !removed.has(sessionId),
                  ),
                },
                active.sessions,
              ),
            };
          }),

        setArchiveRetentionDays: (days) =>
          set(() => ({
            archiveRetentionDays: normalizeArchiveRetentionDays(days),
          })),

        setSessionGenerating: (id, generating) =>
          set((st) => {
            const running = new Set(st.generatingSessionIds);
            if (generating) running.add(id);
            else running.delete(id);
            return {
              sessions: st.sessions.map((session) =>
                session.id === id && session.settledAt
                  ? { ...session, settledAt: undefined }
                  : session,
              ),
              generatingSessionIds: Array.from(running),
              unreadSessionIds: generating
                ? st.unreadSessionIds.filter((unreadId) => unreadId !== id)
                : st.unreadSessionIds,
            };
          }),

        setHostBusySessionIds: (ids) =>
          set((st) => {
            const next = [...new Set(ids)].sort();
            if (
              next.length === st.hostBusySessionIds.length &&
              next.every((id, index) => id === st.hostBusySessionIds[index])
            ) {
              return {};
            }
            const busy = new Set(next);
            return {
              sessions: st.sessions.map((session) =>
                busy.has(session.id) && session.settledAt
                  ? { ...session, settledAt: undefined }
                  : session,
              ),
              hostBusySessionIds: next,
              unreadSessionIds: st.unreadSessionIds.filter(
                (unreadId) => !busy.has(unreadId),
              ),
            };
          }),

        setSessionHostBusy: (id, busy) =>
          set((st) => {
            const running = new Set(st.hostBusySessionIds);
            if (busy) running.add(id);
            else running.delete(id);
            const next = Array.from(running).sort();
            if (
              next.length === st.hostBusySessionIds.length &&
              next.every((value, index) => value === st.hostBusySessionIds[index])
            ) {
              return {};
            }
            return {
              sessions: st.sessions.map((session) =>
                session.id === id && busy && session.settledAt
                  ? { ...session, settledAt: undefined }
                  : session,
              ),
              hostBusySessionIds: next,
              unreadSessionIds: busy
                ? st.unreadSessionIds.filter((unreadId) => unreadId !== id)
                : st.unreadSessionIds,
            };
          }),

        setSessionActivityBusy: (id, busy) =>
          set((st) => {
            const running = new Set(st.activityBusySessionIds);
            if (busy) running.add(id);
            else running.delete(id);
            const next = Array.from(running).sort();
            if (
              next.length === st.activityBusySessionIds.length &&
              next.every((value, index) => value === st.activityBusySessionIds[index])
            ) {
              return {};
            }
            return {
              sessions: st.sessions.map((session) =>
                session.id === id && busy && session.settledAt
                  ? { ...session, settledAt: undefined }
                  : session,
              ),
              activityBusySessionIds: next,
              unreadSessionIds: busy
                ? st.unreadSessionIds.filter((unreadId) => unreadId !== id)
                : st.unreadSessionIds,
            };
          }),

        setSessionUnread: (id, unread) =>
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            const unreadIds = new Set(st.unreadSessionIds);
            if (unread && id !== st.activeId) unreadIds.add(id);
            else unreadIds.delete(id);
            return {
              sessions: unread
                ? st.sessions.map((session) =>
                    session.id === id && session.settledAt
                      ? { ...session, settledAt: undefined }
                      : session,
                  )
                : st.sessions,
              unreadSessionIds: Array.from(unreadIds),
            };
          }),

        enqueueQueuedMessage: (id, message) => {
          const item: QueuedMessage = {
            id: uid(),
            content: message.content,
            attachments: message.attachments?.length
              ? message.attachments
              : undefined,
            createdAt: Date.now(),
          };
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            return {
              queuedMessagesBySession: {
                ...st.queuedMessagesBySession,
                [id]: [...(st.queuedMessagesBySession[id] ?? []), item],
              },
            };
          });
          return item;
        },

        updateQueuedMessage: (id, messageId, patch) =>
          set((st) => {
            const queue = st.queuedMessagesBySession[id];
            if (!queue?.some((message) => message.id === messageId)) return {};
            const nextQueue = queue.flatMap((message) => {
              if (message.id !== messageId) return [message];
              const next = {
                ...message,
                ...patch,
                attachments:
                  "attachments" in patch
                    ? patch.attachments?.length
                      ? patch.attachments
                      : undefined
                    : message.attachments,
              };
              return next.content.trim() || next.attachments?.length
                ? [next]
                : [];
            });
            const nextBySession = { ...st.queuedMessagesBySession };
            if (nextQueue.length) nextBySession[id] = nextQueue;
            else delete nextBySession[id];
            return { queuedMessagesBySession: nextBySession };
          }),

        moveQueuedMessage: (id, messageId, targetId, position = "before") =>
          set((st) => {
            const queue = st.queuedMessagesBySession[id];
            if (
              !queue ||
              messageId === targetId ||
              !queue.some((message) => message.id === messageId) ||
              !queue.some((message) => message.id === targetId)
            )
              return {};
            const order = moveInOrder(
              queue.map((message) => message.id),
              messageId,
              targetId,
              position,
            );
            const byId = new Map(queue.map((message) => [message.id, message]));
            return {
              queuedMessagesBySession: {
                ...st.queuedMessagesBySession,
                [id]: order.map((messageId) => byId.get(messageId)!),
              },
            };
          }),

        removeQueuedMessage: (id, messageId) =>
          set((st) => {
            const queue = st.queuedMessagesBySession[id];
            if (!queue?.some((message) => message.id === messageId)) return {};
            const nextQueue = queue.filter(
              (message) => message.id !== messageId,
            );
            const nextBySession = { ...st.queuedMessagesBySession };
            if (nextQueue.length) nextBySession[id] = nextQueue;
            else delete nextBySession[id];
            return { queuedMessagesBySession: nextBySession };
          }),

        shiftQueuedMessage: (id) => {
          const item = get().queuedMessagesBySession[id]?.[0] ?? null;
          if (!item) return null;
          get().removeQueuedMessage(id, item.id);
          return item;
        },

        clearQueuedMessages: (id) =>
          set((st) => {
            if (!st.queuedMessagesBySession[id]?.length) return {};
            const nextBySession = { ...st.queuedMessagesBySession };
            delete nextBySession[id];
            return { queuedMessagesBySession: nextBySession };
          }),

        addProjectFolder: (folder) =>
          set((st) => {
            const normalized = normalizeProjectFolder(folder);
            if (!normalized) return {};
            return {
              projects: upsertProject(st.projects, normalized),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: [normalized, ...st.sidebar.projectFolders],
                  sectionOrder: [
                    projectSectionId(normalized),
                    ...st.sidebar.sectionOrder,
                  ],
                },
                st.sessions,
              ),
            };
          }),

        setSessionFolder: (id, folder) =>
          set((st) => {
            const normalized = normalizeProjectFolder(folder);
            const sessions = st.sessions.map((s) =>
              s.id === id
                && s.threadWorkspace?.mode !== "worktree"
                ? {
                    ...s,
                    settings: resetApprovalForFolderChange(
                      s.settings?.folder,
                      normalizeSettings({
                        ...s.settings,
                        folder: normalized,
                      }),
                    ),
                    threadWorkspace: normalizeThreadWorkspace(
                      {
                        ...s.threadWorkspace,
                        mode: "current",
                        projectFolder: normalized,
                        worktreeFolder: normalized,
                      },
                      normalized,
                    ),
                    updatedAt: Date.now(),
                  }
                : s,
            );
            return {
              sessions,
              projects: upsertProject(st.projects, normalized),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: normalized
                    ? [normalized, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                },
                sessions,
              ),
            };
          }),

        setThreadWorkspace: (id, workspace) =>
          set((st) => {
            const normalized = normalizeThreadWorkspace(
              workspace,
              workspace.worktreeFolder,
            );
            const sessions = st.sessions.map((session) =>
              session.id === id
                ? {
                    ...session,
                    threadWorkspace: normalized,
                    settings: normalizeSettings({
                      ...session.settings,
                      folder: normalized.worktreeFolder,
                    }),
                    updatedAt: Date.now(),
                  }
                : session,
            );
            return {
              sessions,
              projects: upsertProject(st.projects, normalized.projectFolder),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: normalized.projectFolder
                    ? [normalized.projectFolder, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                },
                sessions,
              ),
            };
          }),

        toggleSessionPinned: (id) =>
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            const pinned = new Set(st.sidebar.pinnedSessionIds);
            if (pinned.has(id)) pinned.delete(id);
            else pinned.add(id);
            return {
              sidebar: normalizeSidebarState(
                { ...st.sidebar, pinnedSessionIds: Array.from(pinned) },
                st.sessions,
              ),
            };
          }),

        toggleSidebarSectionCollapsed: (id) =>
          set((st) => {
            const collapsed = new Set(st.sidebar.collapsedSectionIds);
            if (collapsed.has(id)) collapsed.delete(id);
            else collapsed.add(id);
            return {
              sidebar: normalizeSidebarState(
                { ...st.sidebar, collapsedSectionIds: Array.from(collapsed) },
                st.sessions,
              ),
            };
          }),

        toggleSidebarSectionPinned: (id) =>
          set((st) => {
            if (!isSidebarProjectSectionId(id)) return {};
            const pinned = new Set(st.sidebar.pinnedSectionIds);
            const willPin = !pinned.has(id);
            if (willPin) pinned.add(id);
            else pinned.delete(id);
            return {
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  pinnedSectionIds: Array.from(pinned),
                  sectionOrder: willPin
                    ? moveInOrder(st.sidebar.sectionOrder, id)
                    : st.sidebar.sectionOrder,
                },
                st.sessions,
              ),
            };
          }),

        moveSidebarSection: (id, targetId, position) =>
          set((st) => {
            if (!targetId) return {};
            if (
              !isSidebarProjectSectionId(id) ||
              !isSidebarProjectSectionId(targetId)
            )
              return {};
            return {
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  sectionOrder: moveInOrder(
                    st.sidebar.sectionOrder,
                    id,
                    targetId,
                    position,
                  ),
                },
                st.sessions,
              ),
            };
          }),

        moveSessionInSidebar: (id, targetId, targetSectionId, position) =>
          set((st) => {
            if (!st.sessions.some((session) => session.id === id)) return {};
            const targetFolder = targetSectionId
              ? folderFromProjectSectionId(targetSectionId)
              : null;
            const targetPinned = targetSectionId === SIDEBAR_PINNED_SECTION_ID;
            const pinned = new Set(st.sidebar.pinnedSessionIds);
            if (targetSectionId) {
              if (targetPinned) pinned.add(id);
              else pinned.delete(id);
            }
            const sessions = st.sessions.map((s) =>
              s.id === id && targetFolder !== null
                ? {
                    ...s,
                    settings: resetApprovalForFolderChange(
                      s.settings?.folder,
                      normalizeSettings({
                        ...s.settings,
                        folder: targetFolder,
                      }),
                    ),
                    updatedAt: Date.now(),
                  }
                : s,
            );
            return {
              sessions,
              projects: upsertProject(st.projects, targetFolder ?? ""),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  pinnedSessionIds: Array.from(pinned),
                  projectFolders: targetFolder
                    ? [targetFolder, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                  sessionOrder: moveInOrder(
                    st.sidebar.sessionOrder,
                    id,
                    targetId,
                    position,
                  ),
                },
                sessions,
              ),
            };
          }),

        setMessages: (id, messages, options) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const normalizedMessages = messagesWithUniqueIds(
                messages.map((message, index) =>
                  s.messages[index] === message
                    ? message
                    : normalizeMessageArtifacts(message),
                ),
              );
              return {
                ...s,
                messages: normalizedMessages,
                updatedAt: Date.now(),
                title:
                  s.title === NEW_CHAT_TITLE && options?.autoTitle !== false
                    ? deriveThreadTitle(normalizedMessages)
                    : s.title,
              };
            }),
          })),

        prependMessagePage: (id, pageMessages, firstIndex, total) =>
          set((st) => ({
            sessions: st.sessions.map((session) => {
              if (session.id !== id) return session;
              const normalizedPage = pageMessages.map((message) =>
                normalizeMessageArtifacts(message),
              );
              const pageIds = new Set(
                normalizedPage.map((message) => message.id),
              );
              const merged = [
                ...normalizedPage,
                ...session.messages.filter((message) => !pageIds.has(message.id)),
              ];
              const seen = new Set<string>();
              const messages = merged.filter((message) => {
                const messageId = String(message.id);
                if (seen.has(messageId)) return false;
                seen.add(messageId);
                return true;
              });
              return {
                ...session,
                messages,
                messagesLoadedFrom: firstIndex,
                persistedMessageCount: total,
                messagesHydrated: firstIndex === 0,
              };
            }),
          })),

        appendStreamChunks: (id, messageIdOrChunks, chunksArg) => {
          const messageId = Array.isArray(messageIdOrChunks)
            ? undefined
            : messageIdOrChunks;
          const chunks = Array.isArray(messageIdOrChunks)
            ? messageIdOrChunks
            : (chunksArg ?? []);
          const stop = startPerfMeasure("store.appendStreamChunks");
          recordPerfMeasure(
            "store.appendStreamChunks.batchChunks",
            chunks.length,
          );
          recordPerfMeasure(
            "store.appendStreamChunks.batchChars",
            chunks.reduce((sum, chunk) => sum + chunk.content.length, 0),
          );
          try {
            set((st) => ({
              sessions: st.sessions.map((s) =>
                s.id === id
                  ? {
                      ...s,
                      messages: appendStreamChunksToMessages(
                        s.messages,
                        messageId,
                        chunks,
                      ),
                    }
                  : s,
              ),
            }));
          } finally {
            stop();
          }
        },

        finalizeMessageArtifacts: (id, messageId) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : s.messages.length - 1;
              if (targetIndex < 0) return s;
              const target = s.messages[targetIndex];
              if (target.role !== "assistant") return s;
              const messages = s.messages.slice();
              messages[targetIndex] = normalizeMessageArtifacts(target);
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        appendStreamEvent: (id, messageIdOrPart, partArg) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const messageId =
                typeof messageIdOrPart === "string"
                  ? messageIdOrPart
                  : undefined;
              const part =
                typeof messageIdOrPart === "string" ? partArg : messageIdOrPart;
              if (!part) return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : s.messages.length - 1;
              if (targetIndex < 0) return s;
              const messages = s.messages.slice();
              const target = messages[targetIndex];
              messages[targetIndex] = {
                ...target,
                streamParts: appendEventStreamPart(target.streamParts, part),
              };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        completeStreamEvent: (
          id,
          messageIdOrName,
          nameOrPart,
          partOrCallId,
          callIdArg,
        ) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const hasMessageId = typeof nameOrPart === "string";
              const messageId = hasMessageId ? messageIdOrName : undefined;
              const name = hasMessageId ? nameOrPart : messageIdOrName;
              const part = hasMessageId ? partOrCallId : nameOrPart;
              const callId = hasMessageId ? callIdArg : partOrCallId;
              if (!part || typeof part === "string") return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : s.messages.length - 1;
              if (targetIndex < 0) return s;
              const messages = s.messages.slice();
              const target = messages[targetIndex];
              messages[targetIndex] = {
                ...target,
                streamParts: completeEventStreamPart(
                  target.streamParts,
                  name,
                  part,
                  typeof callId === "string" ? callId : undefined,
                ),
              };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        commitRun: (id, messageIdOrRun, runArg) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const messageId =
                typeof messageIdOrRun === "string" ? messageIdOrRun : undefined;
              const run =
                typeof messageIdOrRun === "string" ? runArg : messageIdOrRun;
              if (!run) return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : s.messages.length - 1;
              if (targetIndex < 0) return s;
              const messages = s.messages.slice();
              messages[targetIndex] = {
                ...messages[targetIndex],
                run,
              };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        commitResponseMetrics: (id, messageIdOrMetrics, metricsArg) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const messageId =
                typeof messageIdOrMetrics === "string"
                  ? messageIdOrMetrics
                  : undefined;
              const metrics =
                typeof messageIdOrMetrics === "string"
                  ? metricsArg
                  : messageIdOrMetrics;
              if (!metrics) return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : (() => {
                    for (
                      let index = s.messages.length - 1;
                      index >= 0;
                      index -= 1
                    ) {
                      if (s.messages[index].role === "assistant") return index;
                    }
                    return -1;
                  })();
              if (targetIndex < 0) return s;
              const messages = s.messages.slice();
              messages[targetIndex] = { ...messages[targetIndex], metrics };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        appendMemoryNotice: (id, messageIdOrNotice, noticeArg) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const messageId =
                typeof messageIdOrNotice === "string"
                  ? messageIdOrNotice
                  : undefined;
              const notice =
                typeof messageIdOrNotice === "string"
                  ? noticeArg
                  : messageIdOrNotice;
              if (!notice) return s;
              const targetIndex = messageId
                ? s.messages.findIndex((message) => message.id === messageId)
                : s.messages.length - 1;
              if (targetIndex < 0) return s;
              const messages = s.messages.slice();
              const target = messages[targetIndex];
              if (target.role !== "assistant") return s;
              const existing = target.memories ?? [];
              if (
                existing.some(
                  (memory) =>
                    memory.id === notice.id ||
                    memory.node_id === notice.node_id,
                )
              )
                return s;
              messages[targetIndex] = {
                ...target,
                memories: [...existing, notice],
              };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        upsertChildThread: (parentId, thread, events) =>
          set((st) => {
            const runId = thread.run_id?.trim() || `legacy:${thread.id}`;
            const current = st.workerRuns.find((item) => item.run.id === runId);
            const record = singletonRunFromThread(
              parentId,
              thread,
              current,
              events,
            );
            return {
              workerRuns: [
                record,
                ...st.workerRuns.filter((item) => item.run.id !== runId),
              ],
            };
          }),

        updateChildThread: (thread, events) =>
          set((st) => {
            const runId =
              thread.run_id?.trim() ||
              st.workerRuns.find((item) =>
                item.workers.some((worker) => worker.id === thread.id),
              )?.run.id ||
              `legacy:${thread.id}`;
            const current = st.workerRuns.find((item) => item.run.id === runId);
            const record = singletonRunFromThread(
              current?.run.parent_thread_id ?? thread.parent_id,
              thread,
              current,
              events,
            );
            return {
              workerRuns: [
                record,
                ...st.workerRuns.filter((item) => item.run.id !== runId),
              ],
            };
          }),

        upsertWorkerRun: (record) =>
          set((st) => {
            const existing = st.workerRuns.find(
              (item) => item.run.id === record.run.id,
            );
            const next = mergeWorkerRunRecord(existing, record);
            const live =
              next.run.status === "proposed" || next.run.status === "running";
            const wasLive =
              existing?.run.status === "proposed" ||
              existing?.run.status === "running";
            const shouldReveal =
              live && !wasLive && next.run.parent_thread_id === st.activeId;
            const parent = st.sessions.find(
              (session) => session.id === next.run.parent_thread_id,
            );
            const inspectorKey = parent ? inspectorKeyForSession(parent) : "";
            return {
              workerRuns: [
                next,
                ...st.workerRuns.filter((item) => item.run.id !== next.run.id),
              ],
              sessions: st.sessions.map((session) => {
                if (session.id !== next.run.parent_thread_id) return session;
                const targetIndex = next.run.parent_turn_id
                  ? session.messages.findIndex(
                      (message) => message.id === next.run.parent_turn_id,
                    )
                  : -1;
                const messages = session.messages.map((message, index) =>
                  index === targetIndex ||
                  (targetIndex < 0 &&
                    index === session.messages.length - 1 &&
                    message.role === "assistant")
                    ? { ...message, workerRunId: next.run.id }
                    : message,
                );
                if (!shouldReveal)
                  return { ...session, messages, settledAt: undefined };
                return {
                  ...session,
                  messages,
                  settledAt: undefined,
                };
              }),
              ...(shouldReveal && inspectorKey
                ? {
                    inspectorByKey: {
                      ...st.inspectorByKey,
                      [inspectorKey]: { open: true, tab: "workers" as const },
                    },
                  }
                : {}),
            };
          }),

        removeWorkerRun: (runId) =>
          set((st) => ({
            workerRuns: st.workerRuns.filter((record) => record.run.id !== runId),
            sessions: st.sessions.map((session) => ({
              ...session,
              messages: session.messages.map((message) =>
                message.workerRunId === runId
                  ? { ...message, workerRunId: undefined }
                  : message,
              ),
            })),
          })),

        setWorkerRunPending: (sessionId, runId, pending) =>
          set((st) => ({
            sessions: st.sessions.map((session) => {
              if (session.id !== sessionId) return session;
              const ids = new Set(session.pendingWorkerRunIds ?? []);
              if (pending) ids.add(runId);
              else ids.delete(runId);
              return {
                ...session,
                pendingWorkerRunIds: ids.size ? Array.from(ids) : undefined,
                updatedAt: Date.now(),
              };
            }),
          })),

        setWorkerRunEvent: (runId, event) =>
          set((st) => ({
            workerRuns: st.workerRuns.map((record) => {
              if (record.run.id !== runId) return record;
              const workerIndex = record.workers.findIndex(
                (worker) => worker.id === event.thread_id,
              );
              if (workerIndex < 0) return record;
              const workers = record.workers.slice();
              const worker = workers[workerIndex];
              const events = [
                ...(worker.events ?? []).filter(
                  (item) => item.id !== event.id && item.seq !== event.seq,
                ),
                event,
              ].sort((a, b) => a.seq - b.seq);
              workers[workerIndex] = {
                ...worker,
                events,
                messages: messagesFromThread(worker, worker.messages, events),
              };
              return { ...record, workers };
            }),
          })),

        markArtifactSaved: (id, messageIdOrIndex, artifactId, saved) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const messageIndex =
                typeof messageIdOrIndex === "number"
                  ? messageIdOrIndex
                  : s.messages.findIndex(
                      (message) => message.id === messageIdOrIndex,
                    );
              if (messageIndex < 0) return s;
              const messages = s.messages.slice();
              const message = messages[messageIndex];
              if (!message.artifacts?.length) return s;
              messages[messageIndex] = {
                ...message,
                artifacts: message.artifacts.map((artifact) =>
                  artifact.id === artifactId
                    ? { ...artifact, saved }
                    : artifact,
                ),
              };
              return { ...s, messages, updatedAt: Date.now() };
            }),
          })),

        upsertVirtualFiles: (id, files, source) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id || !files.length) return s;
              const now = Date.now();
              const virtualFiles = { ...(s.virtualFiles ?? {}) };
              for (const file of files) {
                const path = normalizeVirtualFilePath(file.path);
                if (!path) continue;
                const previous = virtualFiles[path];
                virtualFiles[path] = {
                  path,
                  content: file.content,
                  bytes: textBytes(file.content),
                  updatedAt: now,
                  version:
                    previous && previous.content !== file.content
                      ? previous.version + 1
                      : (previous?.version ?? 1),
                  sourceMessageIndex: source?.sourceMessageIndex,
                  sourceRevisionNumber: source?.sourceRevisionNumber,
                };
              }
              return { ...s, virtualFiles, updatedAt: now };
            }),
          })),

        setContextPanelOpen: (id, open) =>
          set((st) => ({
            sessions: st.sessions.map((s) =>
              s.id === id
                ? {
                    ...s,
                    contextPanelOpen: open || undefined,
                    updatedAt: Date.now(),
                  }
                : s,
            ),
          })),

        setContextSectionCollapsed: (id, sectionId, collapsed) =>
          set((st) => ({
            sessions: st.sessions.map((session) => {
              if (session.id !== id) return session;
              const sectionIds = new Set(
                normalizeQuickSummarySectionIds(
                  session.contextCollapsedSectionIds,
                ),
              );
              if (collapsed) sectionIds.add(sectionId);
              else sectionIds.delete(sectionId);
              const contextCollapsedSectionIds = Array.from(sectionIds);
              return {
                ...session,
                contextCollapsedSectionIds:
                  contextCollapsedSectionIds.length
                    ? contextCollapsedSectionIds
                    : undefined,
                updatedAt: Date.now(),
              };
            }),
          })),

        setInspectorOpen: (id, open) =>
          set((st) => {
            const session = st.sessions.find((item) => item.id === id);
            if (!session) return {};
            const key = inspectorKeyForSession(session);
            const current = st.inspectorByKey[key] ?? DEFAULT_INSPECTOR_STATE;
            return {
              inspectorByKey: {
                ...st.inspectorByKey,
                [key]: { ...current, open },
              },
            };
          }),

        setInspectorTab: (id, tab) =>
          set((st) => {
            const session = st.sessions.find((item) => item.id === id);
            if (!session) return {};
            const key = inspectorKeyForSession(session);
            return {
              inspectorByKey: {
                ...st.inspectorByKey,
                [key]: { open: true, tab },
              },
            };
          }),

        setBrowserSession: (id, browserSession) =>
          set((st) => ({
            sessions: st.sessions.map((session) =>
              session.id === id
                ? {
                    ...session,
                    browserSession: normalizeBrowserSession(browserSession, id),
                  }
                : session,
            ),
          })),

        setPreviewBrowserSessionByKey: (key, browserSession) =>
          set((st) => {
            if (!/^[A-Za-z0-9_.-]+$/.test(key)) return {};
            const normalized = normalizeBrowserSession(
              browserSession,
              `${key}-app`,
            );
            if (!normalized) return {};
            return {
              previewBrowserSessionsByKey: {
                ...st.previewBrowserSessionsByKey,
                [key]: normalized,
              },
            };
          }),

        setPreviewRuntime: (id, runtime) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const previewRuntime = normalizePreviewRuntime(runtime);
              if (samePreviewRuntime(s.previewRuntime, previewRuntime))
                return s;
              return {
                ...s,
                previewRuntime: previewRuntime
                  ? { ...previewRuntime, updatedAt: Date.now() }
                  : undefined,
              };
            }),
          })),

        setPreviewRuntimeByKey: (key, runtime) =>
          set((st) => {
            if (!/^[A-Za-z0-9_.-]+$/.test(key)) return {};
            const previewRuntime = normalizePreviewRuntime(runtime);
            if (
              samePreviewRuntime(st.previewRuntimesByKey[key], previewRuntime)
            )
              return {};
            const previewRuntimesByKey = { ...st.previewRuntimesByKey };
            if (previewRuntime)
              previewRuntimesByKey[key] = {
                ...previewRuntime,
                updatedAt: Date.now(),
              };
            else delete previewRuntimesByKey[key];
            return { previewRuntimesByKey };
          }),

        setSessionPullRequest: (id, snapshot) =>
          set((st) => {
            const pullRequestsBySession = { ...st.pullRequestsBySession };
            if (snapshot) pullRequestsBySession[id] = snapshot;
            else delete pullRequestsBySession[id];
            return { pullRequestsBySession };
          }),

        setAccountRuntime: (id, runtime) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const accountRuntime = normalizeAccountRuntime({
                ...s.accountRuntime,
                ...runtime,
              });
              return { ...s, accountRuntime, updatedAt: Date.now() };
            }),
          })),

        clearAccountRuntime: (id) =>
          set((st) => ({
            sessions: st.sessions.map((s) =>
              s.id === id
                ? { ...s, accountRuntime: undefined, updatedAt: Date.now() }
                : s,
            ),
          })),

        clearAccountRuntimeKind: (id, kind) =>
          set((st) => ({
            sessions: st.sessions.map((s) => {
              if (s.id !== id) return s;
              const current = s.accountRuntime;
              if (!current) return s;
              const accountRuntime = normalizeAccountRuntime(
                kind === "codex"
                  ? {
                      claudeSessionId: current.claudeSessionId,
                      claudeLastSyncedMessageId: current.claudeLastSyncedMessageId,
                      opencodeSessionId: current.opencodeSessionId,
                      opencodeLastSyncedMessageId: current.opencodeLastSyncedMessageId,
                      piSessionId: current.piSessionId,
                      piLastSyncedMessageId: current.piLastSyncedMessageId,
                    }
                  : kind === "claude"
                  ? {
                      codexThreadId: current.codexThreadId,
                      codexLastSyncedMessageId: current.codexLastSyncedMessageId,
                      opencodeSessionId: current.opencodeSessionId,
                      opencodeLastSyncedMessageId: current.opencodeLastSyncedMessageId,
                      piSessionId: current.piSessionId,
                      piLastSyncedMessageId: current.piLastSyncedMessageId,
                    }
                  : kind === "opencode"
                  ? {
                      codexThreadId: current.codexThreadId,
                      codexLastSyncedMessageId: current.codexLastSyncedMessageId,
                      claudeSessionId: current.claudeSessionId,
                      claudeLastSyncedMessageId: current.claudeLastSyncedMessageId,
                      piSessionId: current.piSessionId,
                      piLastSyncedMessageId: current.piLastSyncedMessageId,
                    }
                  : {
                      codexThreadId: current.codexThreadId,
                      codexLastSyncedMessageId: current.codexLastSyncedMessageId,
                      claudeSessionId: current.claudeSessionId,
                      claudeLastSyncedMessageId: current.claudeLastSyncedMessageId,
                      opencodeSessionId: current.opencodeSessionId,
                      opencodeLastSyncedMessageId: current.opencodeLastSyncedMessageId,
                    },
              );
              return { ...s, accountRuntime, updatedAt: Date.now() };
            }),
          })),

        setPendingHotSwap: (id, pending) =>
          set((st) => ({
            sessions: st.sessions.map((s) =>
              s.id === id
                ? {
                    ...s,
                    pendingHotSwap: normalizePendingHotSwap(pending),
                    updatedAt: Date.now(),
                  }
                : s,
            ),
          })),

        setRetryWorkspace: (id, retry) =>
          set((st) => ({
            sessions: st.sessions.map((s) =>
              s.id === id
                ? {
                    ...s,
                    retryWorkspace: normalizeRetryWorkspace(retry),
                    updatedAt: Date.now(),
                  }
                : s,
            ),
          })),

        ensureClaudeSessionId: (id) => {
          const existing = get().sessions.find((s) => s.id === id)
            ?.accountRuntime?.claudeSessionId;
          if (existing) return existing;
          const claudeSessionId = runtimeUuid();
          get().setAccountRuntime(id, { claudeSessionId });
          return claudeSessionId;
        },

        getSettings: (id) => {
          const session = get().sessions.find((s) => s.id === id);
          return normalizeSettings(session?.settings);
        },

        updateSettings: (id, settings) =>
          set((st) => {
            const target = st.sessions.find((session) => session.id === id);
            const guardedSettings = target?.threadWorkspace?.mode === "worktree" && "folder" in settings
              ? { ...settings, folder: target.threadWorkspace.worktreeFolder }
              : settings;
            const nextFolder =
              "folder" in guardedSettings
                ? normalizeProjectFolder(guardedSettings.folder)
                : null;
            const sessions = st.sessions.map((s) =>
              s.id === id
                ? (() => {
                    const merged = { ...s.settings, ...guardedSettings };
                    if ("goal" in guardedSettings) {
                      merged.goal = {
                        ...(s.settings?.goal ?? DEFAULT_GOAL_SETTINGS),
                        ...(guardedSettings.goal ?? {}),
                      };
                    }
                    const normalized = normalizeSettings(merged);
                    return {
                      ...s,
                      settings:
                        "folder" in guardedSettings
                          ? resetApprovalForFolderChange(
                              s.settings?.folder,
                              normalized,
                            )
                          : normalized,
                      threadWorkspace:
                        "folder" in guardedSettings && s.threadWorkspace?.mode !== "worktree"
                          ? normalizeThreadWorkspace({
                              ...s.threadWorkspace,
                              mode: "current",
                              projectFolder: normalized.folder,
                              worktreeFolder: normalized.folder,
                            }, normalized.folder)
                          : s.threadWorkspace,
                      pendingHotSwap:
                        "model" in guardedSettings && guardedSettings.model !== s.settings?.model
                          ? undefined
                          : s.pendingHotSwap,
                      updatedAt: Date.now(),
                    };
                  })()
                : s,
            );
            return {
              sessions,
              projects: upsertProject(st.projects, nextFolder ?? ""),
              sidebar: normalizeSidebarState(
                {
                  ...st.sidebar,
                  projectFolders: nextFolder
                    ? [nextFolder, ...st.sidebar.projectFolders]
                    : st.sidebar.projectFolders,
                },
                sessions,
              ),
            };
          }),
      };
    },
    {
      name: "milim.sessions",
      storage: sessionPersistStorage as PersistStorage<Partial<SessionState>>,
      merge: (persisted, current) => {
        const state = (persisted ?? {}) as Partial<SessionState>;
        if (
          (state as Record<string, unknown> | undefined)?.[
            LEGACY_STREAMING_PERSIST_MARKER
          ]
        )
          return current;
        const normalizedSessions =
          state.sessions?.map((session) =>
            normalizeSessionArtifacts(session),
          ) ?? current.sessions;
        const legacyWorkerRuns = normalizedSessions
          .map(legacyWorkerRunFromSession)
          .filter((record): record is SessionWorkerRunRecord => Boolean(record));
        const persistedSessions = normalizedSessions.filter((session) => !session.worker);
        const workerRuns = legacyWorkerRuns;
        const inspectorByKey = normalizeInspectorByKey(state.inspectorByKey);
        for (const session of [...persistedSessions].sort((a, b) => b.updatedAt - a.updatedAt)) {
          const tab = normalizeInspectorTab(session.inspectorTab);
          if (session.inspectorOpen !== true && !tab) continue;
          const key = inspectorKeyForSession(session);
          if (!Object.prototype.hasOwnProperty.call(inspectorByKey, key)) {
            inspectorByKey[key] = {
              open: session.inspectorOpen === true,
              tab: tab ?? "preview",
            };
          }
        }
        const sessions = persistedSessions.map((session) => {
          const next = { ...session };
          delete next.inspectorOpen;
          delete next.inspectorTab;
          return next;
        });
        const previewRuntimesByKey = normalizePreviewRuntimesByKey(
          state.previewRuntimesByKey,
        );
        const previewBrowserSessionsByKey =
          normalizePreviewBrowserSessionsByKey(
            state.previewBrowserSessionsByKey,
          );
        for (const session of sessions) {
          const folder = normalizeProjectFolder(sessionProjectFolder(session));
          if (folder && session.previewRuntime) {
            previewRuntimesByKey[
              previewRuntimeKeyForThread(session.id, folder)
            ] ??= session.previewRuntime;
          }
        }
        const archiveRetentionDays = normalizeArchiveRetentionDays(
          state.archiveRetentionDays,
        );
        const projects = normalizeProjects(
          state.projects,
          sessions,
          uniqueStrings(state.sidebar?.projectFolders),
        );
        const cutoff = Date.now() - archiveRetentionDays * DAY_MS;
        const expiredProjectFolders = new Set(
          projects
            .filter((project) => (project.archivedAt ?? Infinity) <= cutoff)
            .map((project) => project.folder),
        );
        const liveProjects = projects.filter(
          (project) => (project.archivedAt ?? Infinity) > cutoff,
        );
        const liveSessions = sessions.filter(
          (session) =>
            (session.archivedAt ?? Infinity) > cutoff &&
            !expiredProjectFolders.has(
              normalizeProjectFolder(sessionProjectFolder(session)),
            ),
        );
        const legacyActiveParent = normalizedSessions.find(
          (session) => session.id === state.activeId && session.worker,
        )?.parentId;
        const active = ensureVisibleActive(
          legacyActiveParent ?? state.activeId ?? current.activeId,
          liveSessions,
          liveProjects,
        );
        return {
          ...current,
          ...state,
          sessions: active.sessions,
          workerRuns,
          projects: liveProjects,
          previewRuntimesByKey,
          previewBrowserSessionsByKey,
          inspectorByKey,
          pullRequestsBySession: {},
          activeId: active.activeId,
          archiveRetentionDays,
          generatingSessionIds: [],
          hostBusySessionIds: [],
          activityBusySessionIds: [],
          unreadSessionIds: [],
          queuedMessagesBySession: normalizeQueuedMessagesBySession(
            state.queuedMessagesBySession,
            active.sessions,
          ),
          sidebar: normalizeSidebarState(state.sidebar, active.sessions),
        };
      },
      partialize: (state) => ({
        sessions: sessionsForPersistence(
          state.sessions,
          state.generatingSessionIds,
        ),
        projects: state.projects,
        previewRuntimesByKey: state.previewRuntimesByKey,
        previewBrowserSessionsByKey: state.previewBrowserSessionsByKey,
        inspectorByKey: state.inspectorByKey,
        activeId: state.activeId,
        archiveRetentionDays: state.archiveRetentionDays,
        queuedMessagesBySession: state.queuedMessagesBySession,
        sidebar: state.sidebar,
      }),
    },
  ),
);

export function purgeExpiredArchivesAfterHydration(): () => void {
  if (useSessions.persist.hasHydrated()) {
    useSessions.getState().purgeExpiredArchives();
    return () => {};
  }
  return useSessions.persist.onFinishHydration(() => {
    useSessions.getState().purgeExpiredArchives();
  });
}
