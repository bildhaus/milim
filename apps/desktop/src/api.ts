// Thin client for the local milim server.

import { invoke } from "@tauri-apps/api/core";
import { qualifyDuplicateProviderModels } from "./lib/modelPicker.js";
import { wireMessages } from "./lib/attachmentWire.js";
import { assertValidImageAttachment } from "./lib/attachmentInput.js";
import { assertDesktopRequestBodyFits } from "./lib/requestBody.js";
import type { GoogleRevocationStatus } from "./lib/googleWorkspace.js";
import type { AppearanceSnapshotV1 } from "./theme/appearanceSnapshot.js";
import type {
  ControlBootstrapV1 as GeneratedControlBootstrapV1,
  ControlCommandKindV1 as GeneratedControlCommandKindV1,
  ControlCommandResultV1 as GeneratedControlCommandResultV1,
  ControlCommandV1 as GeneratedControlCommandV1,
  ControlEventV1 as GeneratedControlEventV1,
  RunEventPageV1,
  RunInspectionV1,
  PendingInputV1 as GeneratedControlPendingInputV1,
  QueuedTurnV1 as GeneratedControlQueuedTurnV1,
  RunSnapshotV1 as GeneratedControlRunSnapshotV1,
  ThreadSummaryV1 as GeneratedControlThreadSummaryV1,
  ThreadLinkV1 as GeneratedThreadLinkV1,
  TimelineItemV1 as GeneratedControlTimelineItemV1,
  TimelinePageV1 as GeneratedControlTimelinePageV1,
} from "./generated/control-v1.js";
export type { RunEventPageV1, RunEventV1, RunInspectionV1 } from "./generated/control-v1.js";
export {
  attachmentsToPromptContext,
  wireMessageContent,
  wireMessages,
} from "./lib/attachmentWire.js";
export type {
  WireChatMessage,
  WireMessageContent,
} from "./lib/attachmentWire.js";

export const MAX_ATTACHMENT_BYTES = 128 * 1024;

export interface ChatAttachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  content?: string;
  dataUrl?: string;
  truncated?: boolean;
  sourcePath?: string;
}

export interface AccountRuntimeImageInput {
  media_type: string;
  data: string;
}

export interface WorkspaceFileSuggestion {
  path: string;
  full_path: string;
  name: string;
  size: number;
}

export interface WorkspaceDirectoryEntry {
  path: string;
  name: string;
  kind: "directory" | "file";
  size?: number | null;
}

export interface WorkspaceDirectoryPage {
  entries: WorkspaceDirectoryEntry[];
  next_cursor?: string | null;
}

export interface WorkspaceTextFile {
  path: string;
  content: string;
  size: number;
  revision: string;
}

export type WorkspaceTextWriteResult =
  | { status: "saved"; path: string; size: number; revision: string }
  | { status: "conflict"; revision: string };

export type ChatArtifactKind = "code" | "json" | "csv" | "table" | "text";
export type ChatArtifactDisposition = "file" | "inline" | "preview";

export interface ChatArtifact {
  id: string;
  kind: ChatArtifactKind;
  title: string;
  mime: string;
  content: string;
  size: number;
  language?: string;
  filename?: string;
  disposition?: ChatArtifactDisposition;
  saved?: SavedArtifactFile;
}

export interface SavedArtifactFile {
  path: string;
  bytes: number;
  overwritten: boolean;
  savedAt?: number;
  sourceSessionId?: string;
  sourceMessageIndex?: number;
  sourceRevisionNumber?: number;
  source?: "artifact" | "tool_write" | "auto_artifact";
}

export interface ArtifactFileStatus {
  path: string;
  exists: boolean;
  is_file: boolean;
  is_dir: boolean;
  bytes?: number | null;
}

export interface ArtifactWritePreview {
  path: string;
  exists: boolean;
  changed: boolean;
  old_content?: string | null;
  new_content: string;
  old_bytes?: number | null;
  new_bytes: number;
  diff: string;
  truncated: boolean;
}

export type ArtifactOpenTarget = "file" | "folder";

export type WorkspaceLauncherId =
  | "vscode"
  | "zed"
  | "file_manager"
  | "terminal"
  | "git_bash"
  | "wsl"
  | "android_studio";

export interface WorkspaceLauncher {
  id: WorkspaceLauncherId;
  label: string;
  available: boolean;
  reason?: string | null;
  recommendedReason?: string | null;
}

export type PreviewSurfaceKind =
  | "artifact_iframe"
  | "native_browser"
  | "runtime_browser"
  | "google_workspace"
  | "blank"
  | "markdown"
  | "code"
  | "none";
export type PreviewSurfaceStatus =
  | "loading"
  | "ready"
  | "error"
  | "not_inspectable";
export type PreviewSurfaceCapability =
  | "dom_snapshot"
  | "click"
  | "type"
  | "key"
  | "scroll"
  | "logs"
  | "annotate"
  | "source";

export interface PreviewSurfaceTarget {
  label?: string | null;
  title?: string | null;
  url?: string | null;
  message?: string | null;
  native: boolean;
  kind: PreviewSurfaceKind;
  status: PreviewSurfaceStatus;
  capabilities: PreviewSurfaceCapability[];
}

export function previewSurfaceCanInspect(
  surface?: PreviewSurfaceTarget | null,
): boolean {
  return Boolean(
    surface?.status === "ready" &&
      surface.capabilities.includes("dom_snapshot"),
  );
}

export interface TokenUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  /** Provider-reported billed cost normalized from fields such as OpenRouter's `usage.cost`. */
  cost_usd?: number;
}

export type CostSource = "provider" | "estimate";
export type CostAggregation = CostSource | "mixed";

export interface ChatCompactionMetrics {
  responseCount: number;
  durationMs: number;
  usage: TokenUsage;
  costUsd?: number;
  costSource?: CostAggregation;
  costPartial?: boolean;
}

export interface ChatCompactionSummaryMetrics {
  model: string;
  provider?: string;
  durationMs?: number;
  usage?: TokenUsage;
  costUsd?: number;
  costSource?: CostAggregation;
}

export interface ChatCompactionCheckpoint {
  kind: "checkpoint";
  createdAt: number;
  sourceTokens: number;
  summaryTokens: number;
  auto?: boolean;
  /** Thread usage/cost total immediately before this checkpoint was created. */
  baseline?: ChatCompactionMetrics;
  /** Usage/cost of the model call that generated the checkpoint summary. */
  summary?: ChatCompactionSummaryMetrics;
}

export interface WorkspaceCheckpoint {
  ref: string;
  createdAt: number;
  folder: string;
  root?: string;
  head?: string;
}

export interface ChatApprovalRequest {
  kind: "tool" | "claude_session_recovery";
  scope: "reply" | "goal" | "claude_session_recovery";
  status: "pending" | "approved" | "denied";
  requestedAt: number;
  resolvedAt?: number;
  model?: string;
  detail?: string;
}

export interface ReviewComment {
  id: string;
  surface: "workspace" | "diff" | "preview";
  filePath?: string;
  side?: "old" | "new";
  startLine?: number;
  endLine?: number;
  selectedText?: string;
  body: string;
  timestamp: number;
  preview?: {
    url: string;
    title?: string;
    selector: string;
    tag: string;
    id?: string;
    testId?: string;
    role?: string;
    visibleText?: string;
    outerHtml?: string;
    rect?: { x: number; y: number; width: number; height: number };
  };
}

export interface ChatMessage {
  id?: string;
  /** Stable Rust-owned message ID used by canonical mutations when the UI keeps an optimistic ID. */
  canonicalId?: string;
  role: string;
  content: string;
  /** Canonical control run owning this message, when present. */
  runId?: string;
  /** Canonical timeline position for non-provider transcript projection. */
  controlSeq?: number;
  /** Durable notice that this thread will continue with a different model. */
  modelChange?: {
    previousModel: string;
    model: string;
  };
  /** Present only for runs with a durable, inspectable local ledger. */
  ledgerVersion?: number;
  attachments?: ChatAttachment[];
  mediaResults?: MediaGenerationResult[];
  mediaRequestId?: string;
  compaction?: ChatCompactionCheckpoint;
  plan?: { status: "proposed" | "executed"; executedAt?: number };
  approval?: ChatApprovalRequest;
  reviewComments?: ReviewComment[];
  artifacts?: ChatArtifact[];
  memories?: MemoryNotice[];
  /** UI-only ordered stream transcript of assistant text, reasoning, and events. */
  streamParts?: ChatStreamPart[];
  /** UI-only terminal outcome for canonical stream projection when no RunTrace is present. */
  streamTerminalOutcome?: "completed" | "interrupted" | "unknown";
  /** UI-only structured trace of an agent/tool run (never sent to the server). */
  run?: RunTrace;
  /** UI-only response timing, token usage, and optional cost snapshot. */
  metrics?: ResponseMetrics;
  /** Git worktree snapshot captured immediately before this assistant turn. */
  workspaceCheckpoint?: WorkspaceCheckpoint;
  /** UI-only link to the delegation batch represented by this turn. */
  workerRunId?: string;
  /** Provenance for a canonical input delivered by a linked-thread mailbox. */
  mailboxOrigin?: {
    exchange_id: string;
    origin_thread_id: string;
    origin_title: string;
    origin_workspace?: string | null;
    origin_project?: string | null;
  };
  /** A non-waking reply projected from a linked thread. */
  mailboxReply?: {
    exchangeId: string;
    targetThreadId: string;
    targetTitle: string;
    targetProject?: string;
    status: "replied" | "failed";
  };
}

export interface ProviderLimitInfo {
  provider: string;
  status?: string | null;
  kind?: string | null;
  reset_at?: number | null;
  remaining?: number | null;
  limit?: number | null;
  used?: number | null;
  used_percent?: number | null;
  label?: string | null;
  raw?: unknown;
}

export interface ResponseMetrics {
  startedAt: number;
  endedAt?: number;
  durationMs?: number;
  model: string;
  provider?: string;
  usage?: TokenUsage;
  costUsd?: number;
  /** How `costUsd` was produced. Omitted when cost is absent. */
  costSource?: CostSource;
  limits?: ProviderLimitInfo[];
}

export type ChatStreamEventIcon =
  | "tool"
  | "file"
  | "command"
  | "memory"
  | "schedule"
  | "screen"
  | "thinking"
  | "error";

export type ChatStreamEventStatus = "running" | "done" | "error";

export type ChatStreamPreviewPoint = {
  x: number;
  y: number;
  unit: "pixel" | "ratio" | "percent";
};

export interface McpAppDescriptor {
  kind?: "mcp_app";
  server_id: string;
  resource_uri: string;
  tool: Record<string, unknown>;
}

export interface NativeChartDescriptor {
  kind: "native_chart";
}

export type ToolUiDescriptor = McpAppDescriptor | NativeChartDescriptor;

export type ChatStreamPart =
  | { kind: "text"; content: string }
  | { kind: "thinking"; content: string }
  | {
      kind: "event";
      eventType: "tool" | "memory" | "status" | "warning" | "error";
      label: string;
      detail?: string;
      name?: string;
      callId?: string;
      icon?: ChatStreamEventIcon;
      status?: ChatStreamEventStatus;
      previewPoint?: ChatStreamPreviewPoint;
      mcpApp?: ToolUiDescriptor;
      mcpAppResult?: unknown;
      toolArguments?: string;
      approvalId?: string;
      approvalStatus?: ToolApprovalLifecycleStatus;
      approvalRequest?: ToolApprovalRequest;
    };

export type ToolApprovalLifecycleStatus =
  | "pending"
  | "decided"
  | "delivered"
  | "approved"
  | "denied"
  | "failed"
  | "canceled";

export type RunStatus = "running" | "done" | "stopped" | "aborted" | "error";

/** One tool invocation within an agent run, with client-side timing. */
export interface RunStep {
  callId?: string;
  name: string;
  arguments?: string;
  result?: unknown;
  mcpApp?: ToolUiDescriptor;
  mcpAppResult?: unknown;
  error?: string;
  startedAt: number;
  endedAt?: number;
  approval?: {
    id: string;
    status: ToolApprovalLifecycleStatus;
    requestedAt: number;
    resolvedAt?: number;
  };
}

export interface ContextSnapshot {
  model: string;
  limit: number | null;
  compactAt: number | null;
  estimatedPromptTokens: number;
  freeTokens: number | null;
  categories: Array<{ kind: string; label: string; tokens: number }>;
  sources: Array<{ path: string; family: string; tokens: number; status: string }>;
  warnings: string[];
}

/** Structured timeline of one agent/tool-use run, built from the AgentEvent
 *  stream and rendered as the run timeline in the chat UI. */
export interface RunTrace {
  model: string;
  startedAt: number;
  endedAt?: number;
  steps: RunStep[];
  workspace?: string;
  sourceSessionId?: string;
  iterations?: number;
  status: RunStatus;
  error?: string;
  context?: ContextSnapshot;
}

const DEFAULT_BASE = "http://127.0.0.1:7377";
const BASE = DEFAULT_BASE;
const STARTUP_PROVIDER_PICKER_TIMEOUT_MS = 900;
const ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS = 8_000;
const ACCOUNT_RUNTIME_PICKER_RETRY_DELAY_MS = 500;
const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let tokenPromise: Promise<string | null> | null = null;
let apiBasePromise: Promise<string> | null = null;
let startupProviderRefreshPromise: Promise<boolean> | null = null;
let codexRateLimitCache:
  | { value: unknown; expiresAt: number }
  | undefined;
let codexRateLimitInFlight: Promise<unknown> | null = null;
let accountRuntimeCatalogInFlight = new Map<
  string,
  Promise<Record<AccountRuntimeKind, ModelInfo[]>>
>();

async function localApiToken(): Promise<string | null> {
  if (!inTauri) return null;
  tokenPromise ??= invoke<string>("api_token").catch(() => null);
  return tokenPromise;
}

async function localApiBaseUrl(): Promise<string> {
  if (!inTauri) return DEFAULT_BASE;
  apiBasePromise ??= invoke<string>("api_base_url").catch(() => DEFAULT_BASE);
  return apiBasePromise;
}

export async function apiBaseUrl(): Promise<string> {
  return await localApiBaseUrl();
}

export function refreshProviderModelsAtStartup(): Promise<boolean> {
  if (!inTauri) return Promise.resolve(false);
  startupProviderRefreshPromise ??= invoke<boolean>(
    "refresh_provider_models",
  ).catch(() => true);
  return startupProviderRefreshPromise;
}

export interface HarnessMcpCandidate {
  harness: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  env?: McpEnvVar[];
  warnings?: string[];
  source_path: string;
}

export interface HarnessSkillCandidate {
  harness: string;
  name: string;
  path: string;
  skill_md: string;
}

export interface HarnessImportPreview {
  mcps: HarnessMcpCandidate[];
  skills: HarnessSkillCandidate[];
}

export async function discoverHarnessImports(): Promise<HarnessImportPreview> {
  if (!inTauri) return { mcps: [], skills: [] };
  try {
    return await invoke<HarnessImportPreview>("discover_harness_imports");
  } catch {
    return { mcps: [], skills: [] };
  }
}

async function resolveApiInput(
  input: RequestInfo | URL,
): Promise<RequestInfo | URL> {
  const base = await localApiBaseUrl();
  if (base === DEFAULT_BASE) return input;
  if (typeof input === "string") {
    return input.startsWith(DEFAULT_BASE)
      ? `${base}${input.slice(DEFAULT_BASE.length)}`
      : input;
  }
  if (input instanceof URL) {
    const text = input.toString();
    return text.startsWith(DEFAULT_BASE)
      ? new URL(`${base}${text.slice(DEFAULT_BASE.length)}`)
      : input;
  }
  return input;
}


type AttachmentFilePayload = {
  name: string;
  path: string;
  size: number;
  mime: string;
  content?: string;
  dataUrl?: string;
  truncated: boolean;
};

function attachmentFromPayload(
  payload: AttachmentFilePayload,
): Omit<ChatAttachment, "id"> {
  const attachment = {
    name: payload.name,
    mime: payload.mime || inferAttachmentMime(payload.name),
    size: payload.size,
    content: payload.content,
    dataUrl: payload.dataUrl,
    truncated: payload.truncated,
    sourcePath: payload.path,
  };
  assertValidImageAttachment(attachment);
  return attachment;
}

export async function pickAttachmentFiles(): Promise<
  Omit<ChatAttachment, "id">[]
> {
  if (!inTauri)
    throw new Error(
      "Desktop file picking is only available in the desktop app.",
    );
  const payloads = await invoke<AttachmentFilePayload[]>("pick_attachment_files", {
    maxBytes: MAX_ATTACHMENT_BYTES,
  });
  return payloads.map(attachmentFromPayload);
}

export async function readWorkspaceAttachmentFile(
  workspace: string,
  path: string,
): Promise<Omit<ChatAttachment, "id">> {
  if (!inTauri)
    throw new Error(
      "Desktop file picking is only available in the desktop app.",
    );
  const payload = await invoke<AttachmentFilePayload>(
    "read_workspace_attachment_file",
    { workspace, path, maxBytes: MAX_ATTACHMENT_BYTES },
  );
  return attachmentFromPayload(payload);
}

export async function listWorkspaceFiles(
  workspace: string,
  query: string,
  limit = 20,
): Promise<WorkspaceFileSuggestion[]> {
  if (!inTauri || !workspace.trim()) return [];
  return await invoke<WorkspaceFileSuggestion[]>("list_workspace_files", {
    workspace,
    query,
    limit,
  });
}

export async function startPreviewPicker(): Promise<void> {
  if (!inTauri) return;
  await invoke("preview_picker_start");
}

export async function takePreviewPicker(): Promise<Record<string, unknown> | null> {
  if (!inTauri) return null;
  const result = await invoke<{ selection?: Record<string, unknown> | null }>("preview_picker_take");
  return result.selection ?? null;
}

export async function cancelPreviewPicker(): Promise<void> {
  if (!inTauri) return;
  await invoke("preview_picker_cancel");
}

export async function listWorkspaceDirectory(
  workspace: string,
  path = "",
  cursor?: string,
  limit = 200,
): Promise<WorkspaceDirectoryPage> {
  if (!inTauri || !workspace.trim()) return { entries: [] };
  return await invoke<WorkspaceDirectoryPage>("list_workspace_directory", {
    workspace,
    path,
    cursor,
    limit,
  });
}

export async function readWorkspaceTextFile(
  workspace: string,
  path: string,
): Promise<WorkspaceTextFile> {
  if (!inTauri) throw new Error("Workspace editing is only available in the desktop app.");
  return await invoke<WorkspaceTextFile>("read_workspace_text_file", {
    workspace,
    path,
  });
}

export async function writeWorkspaceTextFile(
  workspace: string,
  path: string,
  content: string,
  expectedRevision: string,
  force = false,
): Promise<WorkspaceTextWriteResult> {
  if (!inTauri) throw new Error("Workspace editing is only available in the desktop app.");
  return await invoke<WorkspaceTextWriteResult>("write_workspace_text_file", {
    workspace,
    path,
    content,
    expectedRevision,
    force,
  });
}

export async function saveArtifactFile(
  workspace: string,
  path: string,
  content: string,
  overwrite = false,
): Promise<SavedArtifactFile> {
  if (!inTauri)
    throw new Error(
      "Saving artifacts to a working folder is only available in the desktop app.",
    );
  return await invoke<SavedArtifactFile>("save_artifact_file", {
    workspace,
    path,
    content,
    overwrite,
  });
}

export async function previewArtifactFile(
  workspace: string,
  path: string,
  content: string,
): Promise<ArtifactWritePreview> {
  if (!inTauri) {
    return {
      path,
      exists: false,
      changed: content.length > 0,
      old_content: null,
      new_content: content,
      old_bytes: null,
      new_bytes: content.length,
      diff: content
        .split(/\r?\n/)
        .map((line) => `+${line}`)
        .join("\n"),
      truncated: false,
    };
  }
  return await invoke<ArtifactWritePreview>("preview_artifact_file", {
    workspace,
    path,
    content,
  });
}

export async function artifactFileStatus(
  path: string,
): Promise<ArtifactFileStatus> {
  if (!inTauri) {
    return { path, exists: true, is_file: true, is_dir: false, bytes: null };
  }
  return await invoke<ArtifactFileStatus>("artifact_file_status", { path });
}

export async function openArtifactLocation(
  path: string,
  target: ArtifactOpenTarget = "file",
): Promise<void> {
  if (!inTauri)
    throw new Error(
      "Opening saved artifacts is only available in the desktop app.",
    );
  await invoke("open_artifact_location", { path, target });
}

export async function recordFrontendError(
  message: string,
  detail?: string,
): Promise<void> {
  if (!inTauri) return;
  await invoke("record_frontend_error", { message, detail });
}

export async function diagnosticsPath(): Promise<string> {
  if (!inTauri)
    throw new Error("Desktop diagnostics are only available in the desktop app.");
  return await invoke<string>("diagnostics_path");
}

export async function openDiagnosticsFolder(): Promise<void> {
  await openArtifactLocation(await diagnosticsPath(), "folder");
}

export async function restartDesktopApp(): Promise<void> {
  if (!inTauri) {
    window.location.reload();
    return;
  }
  await invoke("restart_app");
}

export interface MilimBackupInspection {
  schemaVersion: number;
  appVersion: string;
  createdAt: number;
  summary: { chats: number; projects: number; stateKeys: number };
  bytes: number;
}

export async function exportMilimBackup(path: string): Promise<MilimBackupInspection> {
  if (!inTauri) throw new Error("Backup export is only available in the desktop app.");
  return invoke<MilimBackupInspection>("export_milim_backup", { path });
}

export async function inspectMilimBackup(path: string): Promise<MilimBackupInspection> {
  if (!inTauri) throw new Error("Backup restore is only available in the desktop app.");
  return invoke<MilimBackupInspection>("inspect_milim_backup", { path });
}

export async function restoreMilimBackup(path: string): Promise<string> {
  if (!inTauri) throw new Error("Backup restore is only available in the desktop app.");
  return invoke<string>("restore_milim_backup", { path });
}

export async function requestDesktopQuit(): Promise<void> {
  if (!inTauri) return;
  await invoke("request_desktop_quit");
}

export async function setWorkspaceEditorDirty(dirty: boolean): Promise<void> {
  if (!inTauri) return;
  await invoke("set_workspace_editor_dirty", { dirty });
}

export async function completeWorkspaceEditorLeave(action: "hide" | "quit"): Promise<void> {
  if (!inTauri) return;
  await invoke("complete_workspace_editor_leave", { action });
}

export async function listWorkspaceLaunchers(
  workspace: string,
): Promise<WorkspaceLauncher[]> {
  if (!inTauri || !workspace.trim()) return [];
  return await invoke<WorkspaceLauncher[]>("list_workspace_launchers", {
    workspace,
  });
}

export async function openWorkspaceLauncher(
  workspace: string,
  launcherId: WorkspaceLauncherId,
): Promise<void> {
  if (!inTauri)
    throw new Error(
      "Opening a workspace in another app is only available in the desktop app.",
    );
  await invoke("open_workspace_launcher", { workspace, launcherId });
}

export async function openExternalUrl(url: string): Promise<void> {
  if (inTauri) {
    await invoke("open_external_url", { url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export async function getSecretStorageStatus(): Promise<SecretStorageStatus | null> {
  if (!inTauri) return null;
  return await invoke<SecretStorageStatus>("secret_storage_status");
}

export async function setActivePreviewTarget(
  target: PreviewSurfaceTarget | null,
  threadId?: string,
): Promise<void> {
  if (!inTauri) return;
  await invoke("set_active_preview_target", {
    threadId: threadId?.trim() || null,
    label: target?.label ?? null,
    title: target?.title ?? null,
    url: target?.url ?? null,
    native: target?.native ?? false,
    kind: target?.kind ?? null,
    status: target?.status ?? null,
    capabilities: target?.capabilities ?? null,
  });
}

export function inferAttachmentMime(name: string): string {
  const ext = name.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "webp":
      return "image/webp";
    case "gif":
      return "image/gif";
    case "md":
    case "markdown":
      return "text/markdown";
    case "json":
      return "application/json";
    case "csv":
      return "text/csv";
    case "html":
    case "htm":
      return "text/html";
    case "css":
      return "text/css";
    case "js":
    case "jsx":
    case "ts":
    case "tsx":
    case "rs":
    case "py":
    case "go":
    case "java":
    case "c":
    case "cpp":
    case "h":
    case "hpp":
    case "toml":
    case "yaml":
    case "yml":
    case "xml":
    case "txt":
      return "text/plain";
    default:
      return "application/octet-stream";
  }
}

async function authFetch(
  input: RequestInfo | URL,
  init: RequestInit = {},
): Promise<Response> {
  assertDesktopRequestBodyFits(init.body);
  const headers = new Headers(init.headers);
  const [token, resolvedInput] = await Promise.all([
    localApiToken(),
    resolveApiInput(input),
  ]);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  return fetch(resolvedInput, { ...init, headers });
}

export interface MobileCompanionPairing {
  id: string;
  expires_at: number;
  path: string;
}

// ----- Canonical desktop/mobile control protocol v1 -----

export type ControlThreadSummaryV1 = GeneratedControlThreadSummaryV1;
export type ThreadLinkV1 = GeneratedThreadLinkV1;
export type ControlRunSnapshotV1 = GeneratedControlRunSnapshotV1;
export type ControlPendingInputV1 = GeneratedControlPendingInputV1;
export type ControlQueuedTurnV1 = GeneratedControlQueuedTurnV1;
export type ControlBootstrapV1 = Omit<GeneratedControlBootstrapV1, "appearance"> & {
  appearance?: AppearanceSnapshotV1;
};
export type ControlTimelineItemV1 = Omit<GeneratedControlTimelineItemV1, "data"> & {
  data: Record<string, unknown>;
};
export type ControlTimelinePageV1 = Omit<GeneratedControlTimelinePageV1, "items"> & {
  items: ControlTimelineItemV1[];
};
export type ControlCommandKindV1 = GeneratedControlCommandKindV1;
export type ControlCommandV1 = Omit<GeneratedControlCommandV1, "payload"> & {
  payload?: unknown;
};
export type ControlCommandResultV1 = GeneratedControlCommandResultV1;
export type ControlEventV1 = Omit<GeneratedControlEventV1, "data"> & {
  data: Record<string, unknown>;
};

export async function getControlBootstrap(): Promise<ControlBootstrapV1> {
  const response = await authFetch(`${BASE}/control/v1/bootstrap`);
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Control bootstrap failed."));
  }
  return response.json() as Promise<ControlBootstrapV1>;
}

export async function getControlTimeline(
  threadId: string,
  query: { tail?: number; afterSeq?: number; beforeSeq?: number } = { tail: 500 },
): Promise<ControlTimelinePageV1> {
  const params = new URLSearchParams();
  if (query.tail != null) params.set("tail", String(query.tail));
  if (query.afterSeq != null) params.set("after_seq", String(query.afterSeq));
  if (query.beforeSeq != null) params.set("before_seq", String(query.beforeSeq));
  const response = await authFetch(
    `${BASE}/control/v1/threads/${encodeURIComponent(threadId)}/timeline?${params}`,
  );
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Timeline fetch failed."));
  }
  return response.json() as Promise<ControlTimelinePageV1>;
}

export async function getControlRunInspection(runId: string): Promise<RunInspectionV1> {
  const response = await authFetch(`${BASE}/control/v1/runs/${encodeURIComponent(runId)}`);
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Run details fetch failed."));
  }
  return response.json() as Promise<RunInspectionV1>;
}

export async function getControlRunEvents(
  runId: string,
  afterSeq?: number,
  limit = 100,
): Promise<RunEventPageV1> {
  const params = new URLSearchParams({ limit: String(limit) });
  if (afterSeq != null) params.set("after_seq", String(afterSeq));
  const response = await authFetch(
    `${BASE}/control/v1/runs/${encodeURIComponent(runId)}/events?${params}`,
  );
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Run events fetch failed."));
  }
  return response.json() as Promise<RunEventPageV1>;
}

export async function getControlRunDetails(
  runId: string,
): Promise<{ inspection: RunInspectionV1; events: RunEventPageV1 }> {
  const [inspection, events] = await Promise.all([
    getControlRunInspection(runId),
    getControlRunEvents(runId),
  ]);
  return { inspection, events };
}

export async function sendControlCommand(
  command: ControlCommandV1,
): Promise<ControlCommandResultV1> {
  const response = await authFetch(`${BASE}/control/v1/commands`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(command),
  });
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Control command failed."));
  }
  return response.json() as Promise<ControlCommandResultV1>;
}

async function openControlEventSocket(
  signal: AbortSignal,
  onEvent: (event: ControlEventV1) => void,
): Promise<void> {
  const response = await authFetch(`${BASE}/control/v1/socket-ticket`, {
    method: "POST",
  });
  if (!response.ok) {
    throw new Error(
      await responseErrorMessage(response, "Control socket ticket failed."),
    );
  }
  const { ticket } = (await response.json()) as { ticket: string };
  const resolved = await resolveApiInput(
    `${BASE}/control/v1/ws?ticket=${encodeURIComponent(ticket)}`,
  );
  const socketUrl = new URL(String(resolved));
  socketUrl.protocol = socketUrl.protocol === "https:" ? "wss:" : "ws:";
  await new Promise<void>((resolve) => {
    const socket = new WebSocket(socketUrl);
    const abort = () => socket.close(1000, "aborted");
    signal.addEventListener("abort", abort, { once: true });
    socket.onmessage = (message) => {
      if (typeof message.data !== "string") return;
      try {
        onEvent(JSON.parse(message.data) as ControlEventV1);
      } catch {
        // A malformed event is isolated; the timeline cursor remains the authority.
      }
    };
    socket.onclose = () => {
      signal.removeEventListener("abort", abort);
      resolve();
    };
    socket.onerror = () => socket.close();
    if (signal.aborted) abort();
  });
}

/** Resumable canonical event transport. Timeline cursors recover every gap. */
export async function streamControlEvents(
  signal: AbortSignal,
  onEvent: (event: ControlEventV1) => void,
): Promise<void> {
  let retryMs = 250;
  while (!signal.aborted) {
    try {
      await openControlEventSocket(signal, onEvent);
      retryMs = 250;
    } catch {
      // Reconnect below; callers retain their timeline cursor.
    }
    if (signal.aborted) return;
    await new Promise<void>((resolve) => window.setTimeout(resolve, retryMs));
    retryMs = Math.min(4_000, retryMs * 2);
  }
}

export function createControlCommandId(client = "desktop"): string {
  return `${client}-${Date.now().toString(36)}-${crypto.randomUUID()}`;
}

export interface MobileCompanionDevice {
  id: string;
  name: string;
  key_prefix: string;
  paired_at: number;
  last_seen_at?: number | null;
}

export interface MobileCompanionPairingRequest {
  id: string;
  device_name: string;
  platform: "android" | "ios" | "mobile";
  created_at: number;
  expires_at: number;
  status: "pending" | "approved" | "denied" | "paired";
}

export interface MobileCompanionStatus {
  enabled: boolean;
  pairing?: MobileCompanionPairing | null;
  pairing_requests: MobileCompanionPairingRequest[];
  devices: MobileCompanionDevice[];
}

export interface MobileTailscaleStatus {
  installed: boolean;
  logged_in: boolean;
  serve_configured: boolean;
  public_url?: string | null;
  local_target: string;
  message?: string | null;
}

export interface MobileLanStatus {
  enabled: boolean;
  active: boolean;
  port?: number | null;
  service_type: string;
  host_id?: string | null;
  warning: string;
}

async function parseJsonResponse<T>(
  resp: Response,
  fallback: string,
): Promise<T> {
  if (!resp.ok) throw new Error(await responseErrorMessage(resp, fallback));
  return (await resp.json()) as T;
}

export interface GoogleFileCapabilities {
  can_edit: boolean;
  can_download: boolean;
  can_move: boolean;
  can_rename: boolean;
  can_share: boolean;
  can_trash: boolean;
}

export interface GoogleFileSummary {
  id: string;
  name: string;
  mime_type: string;
  web_view_link?: string | null;
  icon_link?: string | null;
  modified_time?: string | null;
  size?: number | null;
  trashed: boolean;
  parents: string[];
  drive_id?: string | null;
  capabilities: GoogleFileCapabilities;
  created_by_milim: boolean;
}

export interface GoogleWorkspaceStatus {
  available: boolean;
  connected: boolean;
  managed_folder_id?: string | null;
  files: GoogleFileSummary[];
  unavailable_reason?: string | null;
  error?: string | null;
}

export interface GooglePickerFlow {
  id: string;
  status: "pending" | "complete" | "error" | string;
  url?: string | null;
  error?: string | null;
  files: GoogleFileSummary[];
}

export interface GoogleDisconnectResult {
  localAuthorizationRemoved: boolean;
  revocation: GoogleRevocationStatus;
}

export interface SecretStorageStatus {
  mode: "native" | "restricted_file" | "unavailable";
  detail: string;
}

export type GoogleFilePreview =
  | { kind: "sheet"; file: GoogleFileSummary; range: string; sheets: unknown[]; values: unknown[][]; formulas: unknown[][] }
  | { kind: "document"; file: GoogleFileSummary; document: Record<string, unknown>; text: string }
  | {
      kind: "presentation";
      file: GoogleFileSummary;
      title?: string | null;
      pageAspectRatio?: number | null;
      pageWidth?: number | null;
      pageHeight?: number | null;
      slides: Array<{
        objectId?: string | null;
        text: string;
        notes?: string | null;
        notesObjectId?: string | null;
        elements?: Array<{
          objectId: string;
          kind: "shape" | "image" | "table" | "video" | "line" | "group" | "chart" | "element";
          order: number;
          x?: number | null;
          y?: number | null;
          width?: number | null;
          height?: number | null;
          baseWidth?: number | null;
          baseHeight?: number | null;
        }>;
        textElements: Array<{
          objectId: string;
          text: string;
          styleRuns?: Array<{ start: number; end: number; style: Record<string, unknown> }>;
          paragraphRuns?: Array<{ start: number; end: number; style: Record<string, unknown> }>;
          contentAlignment?: string | null;
          fontScale?: number | null;
          x?: number | null;
          y?: number | null;
          width?: number | null;
          height?: number | null;
        }>;
      }>;
    }
  | { kind: "folder"; file: GoogleFileSummary; children: GoogleFileSummary[] }
  | { kind: "text"; file: GoogleFileSummary; text: string; truncated: boolean }
  | { kind: "image" | "pdf" | "audio" | "video" | "unsupported"; file: GoogleFileSummary };

export type GoogleSheetEditOperation =
  | {
      action: "set_values";
      range: string;
      values: unknown[][];
      input_option?: "USER_ENTERED" | "RAW";
    }
  | {
      action: "insert_rows" | "delete_rows" | "insert_columns" | "delete_columns";
      sheet_id: number;
      start: number;
      end: number;
    };

export type GoogleDocEditOperation =
  | { action: "delete_range"; start: number; end: number }
  | { action: "insert_text"; index: number; text: string }
  | { action: "set_text_style"; start: number; end: number; bold?: boolean; italic?: boolean; underline?: boolean; font_size?: number; foreground_color?: string }
  | { action: "clear_text_style"; start: number; end: number }
  | { action: "set_paragraph_style"; start: number; end: number; named_style_type?: string; alignment?: GoogleTextAlignment }
  | { action: "create_bullets"; start: number; end: number; bullet_preset: "BULLET_DISC_CIRCLE_SQUARE" | "NUMBERED_DECIMAL_ALPHA_ROMAN" }
  | { action: "delete_bullets"; start: number; end: number };

export type GoogleTextAlignment = "START" | "CENTER" | "END" | "JUSTIFIED";

export type GoogleSlidesEditOperation =
  | { action: "create_slide"; object_id: string; layout?: string; insertion_index?: number }
  | { action: "duplicate_slide"; object_id: string; new_object_id?: string }
  | { action: "delete_slide"; object_id: string }
  | { action: "reorder_slides"; slide_object_ids: string[]; insertion_index: number }
  | { action: "delete_text"; object_id: string; start?: number; end?: number }
  | { action: "insert_text"; object_id: string; offset?: number; text: string }
  | { action: "set_text_style"; object_id: string; start: number; end: number; bold?: boolean; italic?: boolean; underline?: boolean; font_size?: number; foreground_color?: string }
  | { action: "set_paragraph_style"; object_id: string; start: number; end: number; alignment: GoogleTextAlignment }
  | { action: "update_element_transform"; object_id: string; x: number; y: number; width: number; height: number; base_width: number; base_height: number }
  | { action: "duplicate_element"; object_id: string; new_object_id?: string }
  | { action: "delete_element"; object_id: string };

export async function getGoogleWorkspaceStatus(): Promise<GoogleWorkspaceStatus> {
  return await parseJsonResponse<GoogleWorkspaceStatus>(
    await authFetch(`${BASE}/google-workspace/status`),
    "Google Workspace status failed",
  );
}

export async function startGoogleWorkspacePicker(
  fileIds: string[] = [],
): Promise<GooglePickerFlow> {
  return await parseJsonResponse<GooglePickerFlow>(
    await authFetch(`${BASE}/google-workspace/picker`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ file_ids: fileIds }),
    }),
    "Google Picker failed to start",
  );
}

export async function getGoogleWorkspacePicker(
  id: string,
): Promise<GooglePickerFlow> {
  return await parseJsonResponse<GooglePickerFlow>(
    await authFetch(`${BASE}/google-workspace/picker/${encodeURIComponent(id)}`),
    "Google Picker status failed",
  );
}

export async function chooseGoogleWorkspaceFiles(
  fileIds: string[] = [],
  signal?: AbortSignal,
): Promise<GooglePickerFlow> {
  const started = await startGoogleWorkspacePicker(fileIds);
  if (!started.url) throw new Error("Google Picker returned no browser URL");
  await openExternalUrl(started.url);
  const deadline = Date.now() + 5 * 60_000;
  while (Date.now() < deadline) {
    if (signal?.aborted) throw new DOMException("Google Picker cancelled", "AbortError");
    await new Promise<void>((resolve, reject) => {
      const finish = () => {
        signal?.removeEventListener("abort", cancel);
        resolve();
      };
      const cancel = () => {
        window.clearTimeout(timer);
        reject(new DOMException("Google Picker cancelled", "AbortError"));
      };
      const timer = window.setTimeout(finish, 800);
      signal?.addEventListener("abort", cancel, { once: true });
    });
    const flow = await getGoogleWorkspacePicker(started.id);
    if (flow.status === "complete") return flow;
    if (flow.status === "error") {
      throw new Error(flow.error || "Google Picker failed");
    }
  }
  throw new Error("Google Picker timed out");
}

export async function removeGoogleWorkspaceFile(id: string): Promise<void> {
  const response = await authFetch(
    `${BASE}/google-workspace/files/${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Removing Google file failed"));
  }
}

export async function disconnectGoogleWorkspace(): Promise<GoogleDisconnectResult> {
  const response = await authFetch(`${BASE}/google-workspace/disconnect`, {
    method: "POST",
  });
  return await parseJsonResponse<GoogleDisconnectResult>(
    response,
    "Disconnecting Google failed",
  );
}

export async function getGoogleFilePreview(
  id: string,
  range?: string,
): Promise<GoogleFilePreview> {
  const query = range?.trim() ? `?range=${encodeURIComponent(range.trim())}` : "";
  return await parseJsonResponse<GoogleFilePreview>(
    await authFetch(
      `${BASE}/google-workspace/preview/${encodeURIComponent(id)}${query}`,
    ),
    "Google file preview failed",
  );
}

export async function getGoogleFileContent(
  id: string,
  slideId?: string,
): Promise<Blob> {
  const query = slideId ? `?slide_id=${encodeURIComponent(slideId)}` : "";
  const response = await authFetch(
    `${BASE}/google-workspace/content/${encodeURIComponent(id)}${query}`,
  );
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, "Google file content failed"));
  }
  return await response.blob();
}

export async function editGoogleSheet(
  id: string,
  operations: GoogleSheetEditOperation[],
): Promise<void> {
  await editGoogleWorkspaceFile("sheets", id, operations, "Google Sheets edit failed");
}

export async function editGoogleDoc(
  id: string,
  operations: GoogleDocEditOperation[],
): Promise<void> {
  await editGoogleWorkspaceFile("docs", id, operations, "Google Docs edit failed");
}

export async function editGoogleSlides(
  id: string,
  operations: GoogleSlidesEditOperation[],
): Promise<void> {
  await editGoogleWorkspaceFile("slides", id, operations, "Google Slides edit failed");
}

async function editGoogleWorkspaceFile(
  kind: "sheets" | "docs" | "slides",
  id: string,
  operations: unknown[],
  fallback: string,
): Promise<void> {
  const response = await authFetch(
    `${BASE}/google-workspace/${kind}/${encodeURIComponent(id)}/edit`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ operations }),
    },
  );
  if (!response.ok) {
    throw new Error(await responseErrorMessage(response, fallback));
  }
}

export type PreviewAppState =
  | "idle"
  | "staged"
  | "staging"
  | "installing"
  | "starting"
  | "running"
  | "stopping"
  | "stopped"
  | "error";

export type PreviewAppKind = "app" | "static";

export interface PreviewAppFile {
  path: string;
  content: string;
}

export interface PreviewAppLog {
  seq?: number;
  ts: number;
  stream: "stdout" | "stderr" | "system" | string;
  line: string;
}

export interface PreviewAppError {
  code: string;
  message: string;
}

export interface PreviewAppPreflight {
  thread_id: string;
  cwd: string;
  managed: boolean;
  scope: "managed" | "selected_folder";
  package_manager: string;
  install_required: boolean;
  install_command?: string | null;
  dev_command?: string | null;
  source_fingerprint: string;
  port: number;
  url: string;
}

export interface PreviewAppStatus {
  thread_id: string;
  kind: PreviewAppKind | string;
  status: PreviewAppState | string;
  cwd: string;
  active?: boolean;
  ready?: boolean;
  managed?: boolean;
  run_id?: string | null;
  updated_at?: number;
  error?: PreviewAppError | null;
  preflight?: PreviewAppPreflight | null;
  url?: string | null;
  pid?: number | null;
  command?: string | null;
  message?: string | null;
  logs: PreviewAppLog[];
  /** Client-only marker set when polling fails and this is last-known state. */
  stale?: boolean;
}

export interface PreviewAppStartOptions {
  cwd?: string;
  files?: PreviewAppFile[];
  source_fingerprint?: string;
}

export interface PreviewAppPreflightOptions {
  cwd?: string;
  files?: PreviewAppFile[];
}

export interface PreviewStaticStartOptions {
  cwd: string;
  entry_path: string;
}

export interface PreviewAppLogs {
  logs: PreviewAppLog[];
  next_seq: number;
  truncated: boolean;
}

function previewAppUrl(threadId: string, suffix = ""): string {
  return `${BASE}/preview-apps/${encodeURIComponent(threadId)}${suffix}`;
}

export async function getPreviewAppStatus(
  threadId: string,
): Promise<PreviewAppStatus> {
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId)),
    "preview app status failed",
  );
}

export async function preflightPreviewApp(
  threadId: string,
  options: PreviewAppPreflightOptions = {},
): Promise<PreviewAppPreflight> {
  const cwd = options.cwd?.trim();
  return await parseJsonResponse<PreviewAppPreflight>(
    await authFetch(previewAppUrl(threadId, "/preflight"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        ...(cwd ? { cwd } : {}),
        ...(options.files ? { files: options.files } : {}),
      }),
    }),
    "preview app preflight failed",
  );
}

export async function stagePreviewApp(
  threadId: string,
  files: PreviewAppFile[],
): Promise<PreviewAppStatus> {
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId, "/stage"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ files }),
    }),
    "preview app stage failed",
  );
}

export async function startPreviewApp(
  threadId: string,
  options: PreviewAppStartOptions = {},
): Promise<PreviewAppStatus> {
  const cwd = options.cwd?.trim();
  const body = {
    ...(cwd ? { cwd } : {}),
    ...(options.files ? { files: options.files } : {}),
    ...(options.source_fingerprint
      ? { source_fingerprint: options.source_fingerprint }
      : {}),
  };
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId, "/start"), {
      method: "POST",
      ...(Object.keys(body).length
        ? {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
          }
        : {}),
    }),
    "preview app start failed",
  );
}

export async function startStaticPreview(
  threadId: string,
  options: PreviewStaticStartOptions,
): Promise<PreviewAppStatus> {
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId, "/static"), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(options),
    }),
    "static preview start failed",
  );
}

export async function stopPreviewApp(
  threadId: string,
): Promise<PreviewAppStatus> {
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId, "/stop"), { method: "POST" }),
    "preview app stop failed",
  );
}

export async function restartPreviewApp(
  threadId: string,
  options: PreviewAppStartOptions = {},
): Promise<PreviewAppStatus> {
  const cwd = options.cwd?.trim();
  const body = {
    ...(cwd ? { cwd } : {}),
    ...(options.files ? { files: options.files } : {}),
    ...(options.source_fingerprint
      ? { source_fingerprint: options.source_fingerprint }
      : {}),
  };
  return await parseJsonResponse<PreviewAppStatus>(
    await authFetch(previewAppUrl(threadId, "/restart"), {
      method: "POST",
      ...(Object.keys(body).length
        ? {
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
          }
        : {}),
    }),
    "preview app restart failed",
  );
}

export async function getPreviewAppLogs(
  threadId: string,
  afterSeq?: number,
): Promise<PreviewAppLogs> {
  const query =
    typeof afterSeq === "number" && Number.isFinite(afterSeq)
      ? `?after_seq=${encodeURIComponent(String(Math.max(0, Math.floor(afterSeq))))}`
      : "";
  return await parseJsonResponse<PreviewAppLogs>(
    await authFetch(previewAppUrl(threadId, `/logs${query}`)),
    "preview app logs failed",
  );
}

export async function getMobileCompanionStatus(): Promise<MobileCompanionStatus> {
  return await parseJsonResponse<MobileCompanionStatus>(
    await authFetch(`${BASE}/mobile/status`),
    "mobile companion status failed",
  );
}

export async function setMobileCompanionEnabled(
  enabled: boolean,
): Promise<MobileCompanionStatus> {
  return await parseJsonResponse<MobileCompanionStatus>(
    await authFetch(`${BASE}/mobile/enabled`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled }),
    }),
    "mobile companion update failed",
  );
}

export async function startMobileCompanionPairing(): Promise<MobileCompanionPairing> {
  return await parseJsonResponse<MobileCompanionPairing>(
    await authFetch(`${BASE}/mobile/pairing`, { method: "POST" }),
    "mobile companion pairing failed",
  );
}

export async function decideMobileCompanionPairingRequest(
  id: string,
  approved: boolean,
): Promise<MobileCompanionStatus> {
  return await parseJsonResponse<MobileCompanionStatus>(
    await authFetch(
      `${BASE}/mobile/pair-requests/${encodeURIComponent(id)}/decision`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ approved }),
      },
    ),
    "mobile pairing decision failed",
  );
}

export async function revokeMobileCompanionDevice(
  id: string,
): Promise<MobileCompanionStatus> {
  return await parseJsonResponse<MobileCompanionStatus>(
    await authFetch(`${BASE}/mobile/devices/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
    "mobile companion revoke failed",
  );
}

export async function mobileTailscaleStatus(): Promise<MobileTailscaleStatus> {
  if (!inTauri) {
    return {
      installed: false,
      logged_in: false,
      serve_configured: false,
      local_target: "",
      message: "Tailscale setup is available in the desktop app.",
    };
  }
  return await invoke<MobileTailscaleStatus>("mobile_tailscale_status");
}

export async function configureMobileTailscale(): Promise<MobileTailscaleStatus> {
  if (!inTauri) {
    return {
      installed: false,
      logged_in: false,
      serve_configured: false,
      local_target: "",
      message: "Tailscale setup is available in the desktop app.",
    };
  }
  return await invoke<MobileTailscaleStatus>(
    "configure_mobile_tailscale",
  );
}

export async function disableMobileTailscale(): Promise<MobileTailscaleStatus> {
  if (!inTauri) {
    return {
      installed: false,
      logged_in: false,
      serve_configured: false,
      local_target: "",
      message: "Tailscale setup is available in the desktop app.",
    };
  }
  return await invoke<MobileTailscaleStatus>("disable_mobile_tailscale");
}

export async function getMobileLanStatus(): Promise<MobileLanStatus> {
  if (!inTauri) {
    return {
      enabled: false,
      active: false,
      service_type: "_milim._tcp.local.",
      warning: "LAN mode is available in the desktop app.",
    };
  }
  return await invoke<MobileLanStatus>("mobile_lan_status");
}

export async function setMobileLanEnabled(enabled: boolean): Promise<MobileLanStatus> {
  if (!inTauri) return await getMobileLanStatus();
  return await invoke<MobileLanStatus>("set_mobile_lan_enabled", { enabled });
}

/// Stream a chat completion, invoking `onToken` for each content delta.
export async function streamChat(
  model: string,
  messages: ChatMessage[],
  onToken: (t: string) => void,
  signal?: AbortSignal,
  onReasoning?: (t: string) => void,
  onUsage?: (usage: TokenUsage) => void,
  reasoningEffort?: ReasoningEffort,
  toolContext?: AgentToolContext,
): Promise<void> {
  const resp = await authFetch(`${BASE}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model,
      messages: wireMessages(messages),
      stream: true,
      stream_options: { include_usage: true },
      ...reasoningEffortBody(reasoningEffort),
      ...(toolContext?.workspace !== undefined
        ? { workspace: toolContext.workspace }
        : {}),
      ...(toolContext?.privacy_mode !== undefined
        ? { privacy_mode: toolContext.privacy_mode }
        : {}),
    }),
    signal,
  });
  if (!resp.ok || !resp.body) {
    throw new Error(
      await responseErrorMessage(resp, `chat HTTP ${resp.status}`),
    );
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    let nl: number;
    while ((nl = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") return;
      let json: any;
      try {
        json = JSON.parse(data);
      } catch {
        // ignore keepalives / partial frames
        continue;
      }

      const error = json.error?.message ?? json.error;
      if (error) throw new Error(String(error));
      const usage = parseTokenUsage(json.usage);
      if (usage) onUsage?.(usage);
      const delta: string | undefined = json.choices?.[0]?.delta?.content;
      const reasoningRaw: string | undefined =
        json.choices?.[0]?.delta?.reasoning_content ??
        json.choices?.[0]?.delta?.reasoning;
      const reasoning =
        reasoningRaw && reasoningRaw.trim() && reasoningRaw !== delta
          ? reasoningRaw
          : undefined;
      if (reasoning) onReasoning?.(reasoning);
      if (delta) onToken(delta);
    }
  }
}

function reasoningEffortBody(reasoningEffort?: ReasoningEffort): {
  reasoning_effort?: ReasoningEffort;
} {
  return reasoningEffort && reasoningEffort !== "auto"
    ? { reasoning_effort: reasoningEffort }
    : {};
}

export interface ChatCompletionResult {
  content: string;
  usage?: TokenUsage;
  finishReason?: string;
}

type ChatCompletionOptions = {
  signal?: AbortSignal;
  maxTokens?: number;
  temperature?: number;
  reasoningEffort?: ReasoningEffort;
  toolContext?: Pick<AgentToolContext, "workspace" | "privacy_mode">;
};

export async function completeChatWithMetrics(
  model: string,
  messages: ChatMessage[],
  options: ChatCompletionOptions = {},
): Promise<ChatCompletionResult> {
  const resp = await authFetch(`${BASE}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model,
      messages: wireMessages(messages),
      stream: false,
      temperature: options.temperature ?? 0,
      max_tokens: options.maxTokens ?? 500,
      ...reasoningEffortBody(options.reasoningEffort),
      ...(options.toolContext?.workspace !== undefined
        ? { workspace: options.toolContext.workspace }
        : {}),
      ...(options.toolContext?.privacy_mode !== undefined
        ? { privacy_mode: options.toolContext.privacy_mode }
        : {}),
    }),
    signal: options.signal,
  });
  if (!resp.ok) {
    throw new Error(
      await responseErrorMessage(resp, `chat HTTP ${resp.status}`),
    );
  }
  const json = await resp.json();
  const content = json.choices?.[0]?.message?.content;
  if (typeof content !== "string")
    throw new Error("The model returned an empty response.");
  const usage = parseTokenUsage(json.usage);
  const finishReason = json.choices?.[0]?.finish_reason;
  return {
    content,
    ...(usage ? { usage } : {}),
    ...(typeof finishReason === "string" ? { finishReason } : {}),
  };
}

export async function requestComposerCompletion(
  model: string,
  composerText: string,
  signal?: AbortSignal,
): Promise<string | null> {
  const text = composerText.trimEnd();
  if (!model.trim() || text.replace(/\s/g, "").length < 20) return null;
  const result = await completeChatWithMetrics(
    model,
    [{ role: "user", content: text }],
    { signal, maxTokens: 64, temperature: 0 },
  );
  const completion = result.content.trim();
  if (!completion) return null;
  const suffix = completion.startsWith(text) ? completion.slice(text.length) : completion;
  return suffix.slice(0, 400) || null;
}

export async function completeChat(
  model: string,
  messages: ChatMessage[],
  options: ChatCompletionOptions = {},
): Promise<string> {
  const { content } = await completeChatWithMetrics(model, messages, options);
  return content;
}

function parseTokenUsage(value: unknown): TokenUsage | null {
  if (!value || typeof value !== "object") return null;
  const usage = value as Record<string, unknown>;
  const prompt = usage.prompt_tokens;
  const completion = usage.completion_tokens;
  const total = usage.total_tokens;
  if (
    typeof prompt !== "number" ||
    typeof completion !== "number" ||
    typeof total !== "number"
  )
    return null;
  const rawCost = usage.cost_usd ?? usage.cost;
  const costUsd = typeof rawCost === "number" && Number.isFinite(rawCost) && rawCost >= 0
    ? rawCost
    : undefined;
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
    ...(costUsd != null ? { cost_usd: costUsd } : {}),
  };
}

/// List available models (returns [] if the server is unreachable or slow).
export async function listModels(): Promise<string[]> {
  try {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 2500);
    const r = await authFetch(`${BASE}/v1/models`, { signal: ctrl.signal });
    clearTimeout(timer);
    const j = await r.json();
    return (j.data ?? [])
      .map((m: { id: string }) => m.id)
      .filter(isUsableChatModel);
  } catch {
    return [];
  }
}

export interface ModelInfo {
  id: string;
  owned_by: string;
  provider_id?: string;
  display_id?: string;
  context_length?: number;
  max_prompt_tokens?: number;
  max_completion_tokens?: number;
  reasoning?: ModelReasoningMetadata;
  capabilities?: ModelCapabilities;
}

export interface ModelCapabilities {
  imageInput?: boolean;
  imageOutput?: boolean;
  videoOutput?: boolean;
  musicOutput?: boolean;
  toolUse?: boolean;
}

export type ReasoningEffort =
  | "auto"
  | "none"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "on"
  | "xhigh"
  | "max";

export interface ModelReasoningMetadata {
  supported_efforts: ReasoningEffort[];
  default_effort?: ReasoningEffort;
  default_enabled?: boolean;
  mandatory?: boolean;
}

export const CODEX_MODEL_PREFIX = "codex:";
export const CLAUDE_MODEL_PREFIX = "claude:";
export const OPENCODE_MODEL_PREFIX = "opencode:";
export const PI_MODEL_PREFIX = "pi:";
export type AccountRuntimeKind = "codex" | "claude" | "opencode" | "pi";
export type AccountRuntimeEnablement = Record<AccountRuntimeKind, boolean>;

export const DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT: AccountRuntimeEnablement = {
  codex: true,
  claude: true,
  opencode: true,
  pi: true,
};

export function isCodexModel(model: string): boolean {
  return model.trim().toLowerCase().startsWith(CODEX_MODEL_PREFIX);
}

export function isClaudeModel(model: string): boolean {
  return model.trim().toLowerCase().startsWith(CLAUDE_MODEL_PREFIX);
}

export function isOpenCodeModel(model: string): boolean {
  return model.trim().toLowerCase().startsWith(OPENCODE_MODEL_PREFIX);
}

export function isPiModel(model: string): boolean {
  return model.trim().toLowerCase().startsWith(PI_MODEL_PREFIX);
}

export function isAccountRuntimeModel(model: string): boolean {
  return (
    isCodexModel(model) ||
    isClaudeModel(model) ||
    isOpenCodeModel(model) ||
    isPiModel(model)
  );
}

export function accountRuntimeKind(model: string): AccountRuntimeKind | null {
  if (isCodexModel(model)) return "codex";
  if (isClaudeModel(model)) return "claude";
  if (isOpenCodeModel(model)) return "opencode";
  if (isPiModel(model)) return "pi";
  return null;
}

export function isAccountRuntimeEnabled(
  model: string,
  enablement: Readonly<AccountRuntimeEnablement>,
): boolean {
  const kind = accountRuntimeKind(model);
  return !kind || enablement[kind];
}

export function codexRuntimeModel(model: string): string | null {
  const trimmed = model.trim();
  return trimmed.toLowerCase().startsWith(CODEX_MODEL_PREFIX)
    ? trimmed.slice(CODEX_MODEL_PREFIX.length).trim() || null
    : null;
}

export function claudeRuntimeModel(model: string): string | null {
  const trimmed = model.trim();
  return trimmed.toLowerCase().startsWith(CLAUDE_MODEL_PREFIX)
    ? trimmed.slice(CLAUDE_MODEL_PREFIX.length).trim() || null
    : null;
}

export function opencodeRuntimeModel(model: string): string | null {
  const trimmed = model.trim();
  return trimmed.toLowerCase().startsWith(OPENCODE_MODEL_PREFIX)
    ? trimmed.slice(OPENCODE_MODEL_PREFIX.length).trim() || null
    : null;
}

export function piRuntimeModel(model: string): string | null {
  const trimmed = model.trim();
  return trimmed.toLowerCase().startsWith(PI_MODEL_PREFIX)
    ? trimmed.slice(PI_MODEL_PREFIX.length).trim() || null
    : null;
}

export function isUsableChatModel(model: string): boolean {
  const id = model.trim().toLowerCase();
  return id.length > 0 && id !== "mock-echo";
}

async function listProviderModelsForPicker(
  timeoutMs = 2500,
): Promise<ModelInfo[]> {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const r = await authFetch(`${BASE}/v1/models`, { signal: ctrl.signal });
    if (!r.ok) return [];
    const j = await r.json();
    return qualifyDuplicateProviderModels(
      (j.data ?? [])
        .map(
          (m: {
            id: string;
            owned_by?: string;
            provider_id?: string;
            context_length?: number;
            max_prompt_tokens?: number;
            max_completion_tokens?: number;
            reasoning?: unknown;
            capabilities?: unknown;
          }) => ({
            id: m.id,
            owned_by: m.owned_by ?? "local",
            provider_id: m.provider_id?.trim() || undefined,
            context_length: numberOrUndefined(m.context_length),
            max_prompt_tokens: numberOrUndefined(m.max_prompt_tokens),
            max_completion_tokens: numberOrUndefined(m.max_completion_tokens),
            reasoning: normalizeModelReasoning(m.reasoning),
            capabilities: normalizeModelCapabilities(m.capabilities),
          }),
        )
        .filter((m: ModelInfo) => isUsableChatModel(m.id)),
    );
  } catch {
    return [];
  } finally {
    clearTimeout(timer);
  }
}

/** Models with their provider (`owned_by`) for grouping in the picker. */
export async function listModelsDetailed(
  accountRuntimeEnabled: Readonly<AccountRuntimeEnablement> =
    DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT,
  options: { retry?: boolean } = {},
): Promise<ModelInfo[]> {
  const [providerModels, runtimeModels] = await Promise.all([
    listProviderModelsForPicker(),
    accountRuntimeCatalog(accountRuntimeEnabled, options.retry === true),
  ]);
  return [
    ...providerModels,
    ...runtimeModels.codex,
    ...runtimeModels.claude,
    ...runtimeModels.opencode,
    ...runtimeModels.pi,
  ];
}

export async function loadStartupModels(
  onModels: (models: ModelInfo[]) => void,
  accountRuntimeEnabled: Readonly<AccountRuntimeEnablement> =
    DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT,
  initialModels: readonly ModelInfo[] = [],
): Promise<void> {
  const providerRefresh = refreshProviderModelsAtStartup();
  const lanes: Record<
    "provider" | AccountRuntimeKind,
    ModelInfo[]
  > = {
    provider: [],
    codex: [],
    claude: [],
    opencode: [],
    pi: [],
  };
  for (const model of initialModels) {
    const runtime = accountRuntimeKind(model.id);
    if (runtime) {
      if (accountRuntimeEnabled[runtime]) lanes[runtime].push(model);
    } else {
      lanes.provider.push(model);
    }
  }
  const emit = () => {
    const byKey = new Map<string, ModelInfo>();
    for (const models of Object.values(lanes)) {
      for (const model of models) {
        byKey.set(`${model.provider_id || model.owned_by}\0${model.id}`, model);
      }
    }
    onModels(Array.from(byKey.values()));
  };
  const loadLane = async (
    lane: keyof typeof lanes,
    load: () => Promise<ModelInfo[]>,
  ) => {
    const models = await load();
    if (models.length) {
      lanes[lane] = models;
      emit();
    }
  };

  const runtimeLoad = accountRuntimeCatalog(accountRuntimeEnabled).then(
    async (runtimeModels) => {
      await Promise.all(
        (Object.entries(runtimeModels) as Array<
          [AccountRuntimeKind, ModelInfo[]]
        >).map(([lane, models]) => loadLane(lane, () => Promise.resolve(models))),
      );
    },
  );
  if (initialModels.length) emit();
  const cachedProviderModels = await listProviderModelsForPicker(
    STARTUP_PROVIDER_PICKER_TIMEOUT_MS,
  );
  if (cachedProviderModels.length) lanes.provider = cachedProviderModels;
  emit();

  const refreshedProviderLoad = providerRefresh.then(async (refreshed) => {
    if (!refreshed) return;
    const models = await listProviderModelsForPicker(
      STARTUP_PROVIDER_PICKER_TIMEOUT_MS,
    );
    if (!models.length) return;
    lanes.provider = models;
    emit();
  });
  await Promise.allSettled([runtimeLoad, refreshedProviderLoad]);
}

async function accountRuntimeCatalog(
  enablement: Readonly<AccountRuntimeEnablement>,
  retry = false,
): Promise<Record<AccountRuntimeKind, ModelInfo[]>> {
  const key = `${Number(enablement.codex)}${Number(enablement.claude)}${Number(enablement.opencode)}${Number(enablement.pi)}:${Number(retry)}`;
  const existing = accountRuntimeCatalogInFlight.get(key);
  if (existing) return existing;
  const promise = (async () => {
    const result: Record<AccountRuntimeKind, ModelInfo[]> = {
      codex: [],
      claude: [],
      opencode: [],
      pi: [],
    };
    const allTasks: Array<
      [AccountRuntimeKind, () => Promise<ModelInfo[]>]
    > = [
      ["codex", () => listCodexModelsForPicker(retry)],
      ["claude", () => listClaudeModelsForPicker(retry)],
      ["opencode", () => listOpenCodeModelsForPicker(retry)],
      ["pi", () => listPiModelsForPicker(retry)],
    ];
    const tasks = allTasks.filter(([runtime]) => enablement[runtime]);
    let cursor = 0;
    const workers = Array.from(
      { length: Math.min(3, tasks.length) },
      async () => {
        for (;;) {
          const task = tasks[cursor];
          cursor += 1;
          if (!task) return;
          const [runtime, load] = task;
          result[runtime] = await load();
        }
      },
    );
    await Promise.allSettled(workers);
    return result;
  })();
  accountRuntimeCatalogInFlight.set(key, promise);
  void promise.then(
    () => accountRuntimeCatalogInFlight.delete(key),
    () => accountRuntimeCatalogInFlight.delete(key),
  );
  return promise;
}

function numberOrUndefined(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0
    ? value
    : undefined;
}

function normalizeModelCapability(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function normalizeModelCapabilities(
  value: unknown,
): ModelCapabilities | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const raw = value as Record<string, unknown>;
  const imageInput = normalizeModelCapability(
    raw.imageInput ?? raw.image_input,
  );
  const imageOutput = normalizeModelCapability(
    raw.imageOutput ?? raw.image_output,
  );
  const videoOutput = normalizeModelCapability(
    raw.videoOutput ?? raw.video_output,
  );
  const musicOutput = normalizeModelCapability(
    raw.musicOutput ?? raw.music_output,
  );
  const toolUse = normalizeModelCapability(raw.toolUse ?? raw.tool_use);
  if (
    typeof imageInput !== "boolean" &&
    typeof imageOutput !== "boolean" &&
    typeof videoOutput !== "boolean" &&
    typeof musicOutput !== "boolean" &&
    typeof toolUse !== "boolean"
  )
    return undefined;
  return {
    ...(typeof imageInput === "boolean" ? { imageInput } : {}),
    ...(typeof imageOutput === "boolean" ? { imageOutput } : {}),
    ...(typeof videoOutput === "boolean" ? { videoOutput } : {}),
    ...(typeof musicOutput === "boolean" ? { musicOutput } : {}),
    ...(typeof toolUse === "boolean" ? { toolUse } : {}),
  };
}

const REASONING_EFFORTS: ReasoningEffort[] = [
  "auto",
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "on",
  "xhigh",
  "max",
];

function normalizeModelReasoning(
  value: unknown,
): ModelReasoningMetadata | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value))
    return undefined;
  const raw = value as Record<string, unknown>;
  const supported = Array.isArray(raw.supported_efforts)
    ? raw.supported_efforts.filter(
        (item): item is ReasoningEffort =>
          typeof item === "string" &&
          REASONING_EFFORTS.includes(item as ReasoningEffort),
      )
    : [];
  const defaultEffort =
    typeof raw.default_effort === "string" &&
    REASONING_EFFORTS.includes(raw.default_effort as ReasoningEffort)
      ? (raw.default_effort as ReasoningEffort)
      : undefined;
  if (
    !supported.length &&
    !defaultEffort &&
    typeof raw.default_enabled !== "boolean" &&
    typeof raw.mandatory !== "boolean"
  )
    return undefined;
  return {
    supported_efforts: supported,
    default_effort: defaultEffort,
    default_enabled:
      typeof raw.default_enabled === "boolean"
        ? raw.default_enabled
        : undefined,
    mandatory: typeof raw.mandatory === "boolean" ? raw.mandatory : undefined,
  };
}

function normalizeReasoningEffort(value: unknown): ReasoningEffort | undefined {
  return typeof value === "string" &&
    REASONING_EFFORTS.includes(value as ReasoningEffort)
    ? (value as ReasoningEffort)
    : undefined;
}

function normalizeCodexReasoning(
  item: any,
): ModelReasoningMetadata | undefined {
  const supported = Array.isArray(item?.supportedReasoningEfforts)
    ? item.supportedReasoningEfforts
        .map((entry: any) => normalizeReasoningEffort(entry?.reasoningEffort))
        .filter(
          (effort: ReasoningEffort | undefined): effort is ReasoningEffort =>
            Boolean(effort),
        )
    : [];
  const defaultEffort = normalizeReasoningEffort(item?.defaultReasoningEffort);
  if (!supported.length && !defaultEffort) return undefined;
  return {
    supported_efforts: Array.from(new Set(supported)),
    default_effort: defaultEffort,
    default_enabled: true,
    mandatory: false,
  };
}

async function listCodexModelsForPicker(retry = false): Promise<ModelInfo[]> {
  return discoverAccountRuntimeModels(async (signal) => {
    const account = await getCodexAccount(false, signal);
    if (!account.account && account.requiresOpenaiAuth) return [];

    const r = await authFetch(`${BASE}/codex/models`, { signal });
    if (!r.ok) throw new Error("Codex model discovery failed");
    const j = await r.json();
    const byId = new Map<string, ModelInfo>();
    for (const item of j.data ?? []) {
      const raw =
        typeof item?.model === "string" && item.model.trim()
          ? item.model.trim()
          : typeof item?.id === "string"
            ? item.id.trim()
            : "";
      if (!raw) continue;
      const inputModalities = Array.isArray(item?.inputModalities)
        ? item.inputModalities
        : null;
      byId.set(`${CODEX_MODEL_PREFIX}${raw}`, {
        id: `${CODEX_MODEL_PREFIX}${raw}`,
        owned_by: "Codex",
        reasoning: normalizeCodexReasoning(item),
        capabilities: inputModalities
          ? { imageInput: inputModalities.includes("image") }
          : undefined,
      });
    }
    return Array.from(byId.values());
  }, retry);
}

export interface CodexAccountResponse {
  requiresOpenaiAuth: boolean;
  account?: {
    type: string;
    email?: string;
    planType?: string;
  } | null;
}

export interface CodexThreadSummary {
  id: string;
  name?: string | null;
  preview: string;
  project_path?: string | null;
  cwd?: string | null;
  model_provider: string;
  created_at_ms: number;
  updated_at_ms: number;
  archived: boolean;
}

export interface CodexThreadPage {
  data: CodexThreadSummary[];
  next_cursor?: string | null;
}

export interface CodexRecoveredThread {
  id: string;
  title: string;
  cwd?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  messages: Array<{ role: "user" | "assistant"; content: string }>;
}

export interface ClaudeThreadSummary {
  id: string;
  title: string;
  preview: string;
  cwd?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  resumable: boolean;
}

export interface ClaudeThreadPage {
  data: ClaudeThreadSummary[];
  next_cursor?: string | null;
}

export interface ClaudeImportedThread {
  id: string;
  title: string;
  cwd?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
  resumable: boolean;
  messages: Array<{ role: "user" | "assistant"; content: string }>;
}

export interface ClaudeStatusResponse {
  available: boolean;
  authenticated: boolean;
  warning?: boolean;
  auth?: {
    loggedIn?: boolean;
    authMethod?: string;
    email?: string;
    subscriptionType?: string;
  };
  models?: string[];
  model_capabilities?: Record<string, { image_input?: boolean }>;
  error?: string | null;
}

export interface OpenCodeStatusResponse {
  available: boolean;
  authenticated: boolean;
  version?: string | null;
  models?: string[];
  model_capabilities?: Record<
    string,
    {
      display_name?: string | null;
      context_length?: number | null;
      max_prompt_tokens?: number | null;
      max_completion_tokens?: number | null;
      image_input?: boolean | null;
      tool_use?: boolean | null;
      supported_efforts?: ReasoningEffort[];
    }
  >;
  error?: string | null;
}

export interface PiModelInfo {
  id: string;
  provider: string;
  model_id: string;
  name: string;
  context_length?: number | null;
  max_completion_tokens?: number | null;
  reasoning: boolean;
  image_input: boolean;
}

export interface PiStatusResponse {
  available: boolean;
  authenticated: boolean;
  version?: string | null;
  provider_count?: number;
  models?: PiModelInfo[];
  error?: string | null;
}

export interface AccountRuntimeUpdateStatus {
  available: boolean;
  version?: string | null;
  latest_version?: string | null;
  update_available?: boolean | null;
  update_error?: string | null;
  error?: string | null;
}

export interface AccountRuntimeUpdatesResponse {
  runtimes: Record<AccountRuntimeKind, AccountRuntimeUpdateStatus>;
}

export interface AccountRuntimeUpdateResult {
  runtime: AccountRuntimeKind;
  previous_version: string;
  version: string;
  updated: boolean;
  message?: string;
}

export type CodexLoginEvent =
  | { type: "browser"; login_id: string; auth_url: string }
  | {
      type: "device_code";
      login_id: string;
      user_code: string;
      verification_url: string;
    }
  | { type: "done"; success: boolean; error?: string | null }
  | { type: "warning"; message: string }
  | { type: "error"; message: string };

export const HARNESS_EVENT_SCHEMA_VERSION = 1 as const;

export type HarnessImageInput = AccountRuntimeImageInput;
export type HarnessMilimContext = AccountRuntimeMilimContext;

export interface HarnessRunRequest {
  prompt: string;
  images?: HarnessImageInput[];
  model: string;
  cwd?: string;
  reasoning_effort?: ReasoningEffort;
  native_session_id?: string;
  persist_session?: boolean;
  tool_approval_policy?: ToolApprovalMode;
  tool_approval_grant?: boolean;
  interactive_tool_approval?: boolean;
  plan_mode?: boolean;
  allow_session_recovery?: boolean;
  milim_context?: HarnessMilimContext;
}

export type HarnessApprovalEffect =
  | "read_only"
  | "mutating"
  | "command"
  | "unknown";

export interface HarnessToolEvent {
  id: string;
  name: string;
  status: "pending" | "running" | "completed" | "failed" | "cancelled" | "denied";
  label?: string;
  detail?: string;
  result?: unknown;
  error?: string;
  icon?: ChatStreamEventIcon;
}

export interface HarnessNativeWorkerLifecycle {
  runtime: string;
  call_id: string;
  operation: string;
  status: string;
  parent_runtime_id?: string | null;
  worker_runtime_ids: string[];
  workers: Array<{
    runtime_id: string;
    status: string;
    message?: string | null;
  }>;
  prompt?: string | null;
  model?: string | null;
}

export type HarnessEvent =
  | {
      type: "session_established";
      native_session_id: string;
      model?: string;
    }
  | {
      type: "turn_started";
      native_session_id?: string;
      native_turn_id?: string;
    }
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | ({ type: "tool_started" } & HarnessToolEvent)
  | ({ type: "tool_updated" } & HarnessToolEvent)
  | ({ type: "tool_finished" } & HarnessToolEvent)
  | {
      type: "approval_requested";
      approval_id: string;
      call_id?: string;
      name: string;
      arguments: string;
      effect: HarnessApprovalEffect;
      request_kind?: ToolApprovalRequestKind;
      request?: Record<string, unknown>;
    }
  | {
      type: "approval_status";
      approval_id: string;
      call_id?: string;
      decision: "approve" | "deny";
      status: "decided" | "delivered";
    }
  | {
      type: "approval_resolved";
      approval_id: string;
      call_id?: string;
      decision: "approve" | "deny";
    }
  | {
      type: "approval_failed";
      approval_id: string;
      call_id?: string;
      decision?: "approve" | "deny";
      message: string;
    }
  | {
      type: "usage_updated";
      usage?: TokenUsage;
      cost_usd?: number;
    }
  | { type: "limit_updated"; limit: ProviderLimitInfo }
  | {
      type: "image_generated";
      id: string;
      status: string;
      url: string;
      revised_prompt?: string;
      saved_path?: string;
    }
  | {
      type: "native_worker_updated";
      lifecycle: HarnessNativeWorkerLifecycle;
    }
  | {
      type: "runtime_notice";
      kind: string;
      message: string;
      detail?: string;
      level?: "info" | "warning" | "error";
      code?: string;
    }
  | {
      type: "session_recovery_required";
      message: string;
      native_session_id?: string;
    }
  | {
      type: "turn_completed";
      native_session_id?: string;
      native_turn_id?: string;
      status?: string;
      usage?: TokenUsage;
      cost_usd?: number;
    }
  | {
      type: "turn_failed";
      message: string;
      kind?: string;
      code?: string;
      status?: string;
      native_session_id?: string;
      native_turn_id?: string;
      usage?: TokenUsage;
      cost_usd?: number;
    }
  | {
      type: "turn_cancelled";
      message?: string;
      status?: string;
      native_session_id?: string;
      native_turn_id?: string;
      usage?: TokenUsage;
      cost_usd?: number;
    };

export interface HarnessEventEnvelope {
  schema_version: typeof HARNESS_EVENT_SCHEMA_VERSION;
  run_id: string;
  seq: number;
  at_ms: number;
  harness_id: string;
  event: HarnessEvent;
}

/** @deprecated Use HarnessNativeWorkerLifecycle. */
export type AccountNativeWorkerLifecycle = HarnessNativeWorkerLifecycle;

export function isCliPathWarningMessage(message?: string | null): boolean {
  return Boolean(message?.includes("CLI was not found on PATH"));
}

export async function getCodexAccount(
  refresh = false,
  signal?: AbortSignal,
): Promise<CodexAccountResponse> {
  const url = new URL(`${BASE}/codex/account`);
  if (refresh) url.searchParams.set("refresh", "true");
  return await parseJsonResponse<CodexAccountResponse>(
    await authFetch(url, signal ? { signal } : undefined),
    "Codex account check failed",
  );
}

export async function logoutCodex(): Promise<void> {
  await parseJsonResponse<unknown>(
    await authFetch(`${BASE}/codex/logout`, { method: "POST" }),
    "Codex logout failed",
  );
}

export async function getCodexRateLimits(force = false): Promise<unknown> {
  if (!force && codexRateLimitCache && codexRateLimitCache.expiresAt > Date.now()) {
    return codexRateLimitCache.value;
  }
  if (codexRateLimitInFlight) return codexRateLimitInFlight;
  codexRateLimitInFlight = (async () =>
    parseJsonResponse<unknown>(
      await authFetch(`${BASE}/codex/rate-limits`),
      "Codex rate limit check failed",
    ))().then((value) => {
    codexRateLimitCache = { value, expiresAt: Date.now() + 60_000 };
    return value;
  });
  try {
    return await codexRateLimitInFlight;
  } finally {
    codexRateLimitInFlight = null;
  }
}

export async function listCodexThreads(options: {
  cursor?: string;
  search?: string;
  archived?: boolean;
  all?: boolean;
} = {}): Promise<CodexThreadPage> {
  const url = new URL(`${BASE}/codex/threads`);
  if (options.cursor) url.searchParams.set("cursor", options.cursor);
  if (options.search?.trim()) url.searchParams.set("search", options.search.trim());
  if (options.archived) url.searchParams.set("archived", "true");
  if (options.all) url.searchParams.set("all", "true");
  return await parseJsonResponse<CodexThreadPage>(
    await authFetch(url),
    "Codex chat import failed",
  );
}

export async function recoverCodexThread(id: string): Promise<CodexRecoveredThread> {
  return await parseJsonResponse<CodexRecoveredThread>(
    await authFetch(`${BASE}/codex/threads/${encodeURIComponent(id)}`),
    "Codex chat import failed",
  );
}

export async function listClaudeThreads(options: {
  cursor?: string;
  search?: string;
  all?: boolean;
} = {}): Promise<ClaudeThreadPage> {
  const url = new URL(`${BASE}/claude/threads`);
  if (options.cursor) url.searchParams.set("cursor", options.cursor);
  if (options.search?.trim()) url.searchParams.set("search", options.search.trim());
  if (options.all) url.searchParams.set("all", "true");
  return await parseJsonResponse<ClaudeThreadPage>(
    await authFetch(url),
    "Claude chat import failed",
  );
}

export async function importClaudeThread(id: string): Promise<ClaudeImportedThread> {
  return await parseJsonResponse<ClaudeImportedThread>(
    await authFetch(`${BASE}/claude/threads/${encodeURIComponent(id)}`),
    "Claude chat import failed",
  );
}

async function listClaudeModelsForPicker(retry = false): Promise<ModelInfo[]> {
  return discoverAccountRuntimeModels(async (signal) => {
    const status = await getClaudeStatus(signal);
    if (!status.available || !status.authenticated) return [];
    return (status.models ?? [])
      .filter((model) => model.trim())
      .map((model) => ({
        id: `${CLAUDE_MODEL_PREFIX}${model.trim()}`,
        owned_by: "Local Claude CLI",
        capabilities: {
          imageInput:
            status.model_capabilities?.[model.trim()]?.image_input ?? true,
        },
        reasoning: {
          supported_efforts: ["low", "medium", "high", "xhigh", "max"],
          default_effort: "high",
          default_enabled: true,
          mandatory: false,
        },
      }));
  }, retry);
}

async function listOpenCodeModelsForPicker(retry = false): Promise<ModelInfo[]> {
  return discoverAccountRuntimeModels(async (signal) => {
    const status = await getOpenCodeStatus(signal);
    if (!status.available || !status.authenticated) return [];
    return (status.models ?? [])
      .filter((model) => model.trim())
      .map((model) => {
        const id = model.trim();
        const metadata = status.model_capabilities?.[id];
        return {
          id: `${OPENCODE_MODEL_PREFIX}${id}`,
          display_id: metadata?.display_name ?? undefined,
          owned_by: "Local OpenCode CLI",
          context_length: numberOrUndefined(metadata?.context_length),
          max_prompt_tokens: numberOrUndefined(metadata?.max_prompt_tokens),
          max_completion_tokens: numberOrUndefined(
            metadata?.max_completion_tokens,
          ),
          capabilities: {
            imageInput: metadata?.image_input ?? undefined,
            toolUse: metadata?.tool_use ?? undefined,
          },
          reasoning: metadata?.supported_efforts?.length
            ? {
                supported_efforts: metadata.supported_efforts,
                default_enabled: true,
                mandatory: false,
              }
            : undefined,
        };
      });
  }, retry);
}

async function listPiModelsForPicker(retry = false): Promise<ModelInfo[]> {
  return discoverAccountRuntimeModels(async (signal) => {
    const status = await getPiStatus(signal);
    if (!status.available || !status.authenticated) return [];
    return (status.models ?? []).map((model) => ({
      id: `${PI_MODEL_PREFIX}${model.id}`,
      display_id: model.name,
      owned_by: "Local Pi CLI",
      context_length: numberOrUndefined(model.context_length),
      max_completion_tokens: numberOrUndefined(model.max_completion_tokens),
      capabilities: { imageInput: model.image_input, toolUse: true },
      reasoning: model.reasoning
        ? {
            supported_efforts: [
              "none",
              "minimal",
              "low",
              "medium",
              "high",
              "xhigh",
            ],
            default_enabled: true,
            mandatory: false,
          }
        : undefined,
    }));
  }, retry);
}

async function discoverAccountRuntimeModels(
  discover: (signal: AbortSignal) => Promise<ModelInfo[]>,
  retry = false,
): Promise<ModelInfo[]> {
  const attempts = retry ? 2 : 1;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), ACCOUNT_RUNTIME_PICKER_TIMEOUT_MS);
    try {
      return await discover(ctrl.signal);
    } catch {
      // Retry once after the embedded backend and CLI have had time to finish startup.
    } finally {
      clearTimeout(timer);
    }
    if (attempt + 1 < attempts) {
      await new Promise((resolve) => setTimeout(resolve, ACCOUNT_RUNTIME_PICKER_RETRY_DELAY_MS));
    }
  }
  return [];
}

export async function getClaudeStatus(
  signal?: AbortSignal,
): Promise<ClaudeStatusResponse> {
  return await parseJsonResponse<ClaudeStatusResponse>(
    await authFetch(`${BASE}/claude/status`, signal ? { signal } : undefined),
    "Claude CLI status check failed",
  );
}

export async function getOpenCodeStatus(
  signal?: AbortSignal,
): Promise<OpenCodeStatusResponse> {
  return await parseJsonResponse<OpenCodeStatusResponse>(
    await authFetch(`${BASE}/opencode/status`, signal ? { signal } : undefined),
    "OpenCode CLI status check failed",
  );
}

export async function getPiStatus(
  signal?: AbortSignal,
): Promise<PiStatusResponse> {
  return await parseJsonResponse<PiStatusResponse>(
    await authFetch(`${BASE}/pi/status`, signal ? { signal } : undefined),
    "Pi CLI status check failed",
  );
}

export async function getAccountRuntimeUpdates(): Promise<AccountRuntimeUpdatesResponse> {
  return await parseJsonResponse<AccountRuntimeUpdatesResponse>(
    await authFetch(`${BASE}/account-runtimes/updates`),
    "Account runtime version check failed",
  );
}

export async function updateAccountRuntime(
  runtime: AccountRuntimeKind,
): Promise<AccountRuntimeUpdateResult> {
  return await parseJsonResponse<AccountRuntimeUpdateResult>(
    await authFetch(
      `${BASE}/account-runtimes/${encodeURIComponent(runtime)}/update`,
      { method: "POST" },
    ),
    `${runtime} update failed`,
  );
}

export async function streamCodexDeviceLogin(
  onEvent: (ev: CodexLoginEvent) => void,
  signal?: AbortSignal,
  method: "chatgpt" | "chatgpt_device_code" = "chatgpt",
): Promise<void> {
  const path =
    method === "chatgpt_device_code"
      ? "/codex/login/chatgpt-device"
      : "/codex/login/device";
  const resp = await authFetch(`${BASE}${path}`, { method: "POST", signal });
  if (!resp.ok || !resp.body)
    throw new Error(
      await responseErrorMessage(resp, `Codex login HTTP ${resp.status}`),
    );
  await streamJsonSse(resp, onEvent);
}

export async function loginCodexApiKey(apiKey: string): Promise<unknown> {
  return await parseJsonResponse<unknown>(
    await authFetch(`${BASE}/codex/login/api-key`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ api_key: apiKey }),
    }),
    "Codex API-key login failed",
  );
}

const HARNESS_EVENT_TYPES = new Set<HarnessEvent["type"]>([
  "session_established",
  "turn_started",
  "text_delta",
  "reasoning_delta",
  "tool_started",
  "tool_updated",
  "tool_finished",
  "approval_requested",
  "approval_status",
  "approval_resolved",
  "approval_failed",
  "usage_updated",
  "limit_updated",
  "image_generated",
  "native_worker_updated",
  "runtime_notice",
  "session_recovery_required",
  "turn_completed",
  "turn_failed",
  "turn_cancelled",
]);

function parseHarnessEventEnvelope(value: unknown): HarnessEventEnvelope | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Harness event envelope must be an object.");
  }
  const envelope = value as Record<string, unknown>;
  if (envelope.schema_version !== HARNESS_EVENT_SCHEMA_VERSION) {
    throw new Error(
      `Unsupported harness event schema version: ${String(envelope.schema_version)}.`,
    );
  }
  const event = envelope.event;
  if (!event || typeof event !== "object" || Array.isArray(event)) {
    console.warn("[milim] Ignoring malformed harness event for schema version 1.");
    return null;
  }
  const type = (event as Record<string, unknown>).type;
  if (
    typeof type !== "string" ||
    !HARNESS_EVENT_TYPES.has(type as HarnessEvent["type"])
  ) {
    console.warn(
      `[milim] Ignoring unknown harness event type for schema version 1: ${String(type)}.`,
    );
    return null;
  }
  return value as HarnessEventEnvelope;
}

export async function streamHarnessRun(
  harnessId: string,
  request: HarnessRunRequest,
  onEvent: (envelope: HarnessEventEnvelope) => void,
  signal?: AbortSignal,
): Promise<void> {
  const id = harnessId.trim();
  if (!id) throw new Error("Harness ID is required.");
  const resp = await authFetch(
    `${BASE}/harnesses/${encodeURIComponent(id)}/run`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
      signal,
    },
  );
  if (!resp.ok || !resp.body) {
    throw new Error(
      await responseErrorMessage(resp, `Harness run HTTP ${resp.status}`),
    );
  }
  let lastSeq = 0;
  await streamJsonSse<unknown>(resp, (value) => {
    const envelope = parseHarnessEventEnvelope(value);
    if (!envelope) return;
    if (!Number.isSafeInteger(envelope.seq) || envelope.seq <= lastSeq) {
      throw new Error(
        `Harness event sequence must increase (received ${String(envelope.seq)} after ${lastSeq}).`,
      );
    }
    lastSeq = envelope.seq;
    onEvent(envelope);
  });
}

async function streamJsonSse<T>(
  resp: Response,
  onEvent: (ev: T) => void,
): Promise<void> {
  const reader = resp.body?.getReader();
  if (!reader) return;
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") return;
      let parsed: T;
      try {
        parsed = JSON.parse(data) as T;
      } catch {
        /* keepalive / partial */
        continue;
      }
      onEvent(parsed);
    }
  }
}

// ----- Agents -----

export type AgentToolMode = "all" | "custom" | "none";
export type AgentSkillMode = "auto" | "custom" | "none";

export interface Agent {
  id: string;
  name: string;
  description: string;
  system_prompt: string;
  /** @deprecated Used only as a fallback for schedules created before schedule-owned models. */
  model: string;
  tool_mode: AgentToolMode;
  enabled_tools: string[];
  skill_mode: AgentSkillMode;
  enabled_skills: string[];
  avatar: string;
}

export interface AgentDraft {
  name: string;
  description: string;
  system_prompt: string;
  avatar: string;
}

const AGENT_DRAFT_SYSTEM_PROMPT = [
  "You generate reusable Milim agent profiles.",
  "Return only one JSON object with string fields: name, description, avatar, system_prompt.",
  "name: concise display name, usually 2-4 words.",
  "description: one short sentence explaining when to choose this agent.",
  "avatar: a short deterministic seed word or phrase for a generated avatar, not an emoji.",
  "system_prompt: directly usable as an agent system prompt. Include role, priorities, behavior, boundaries, and output style.",
  "Do not include markdown, code fences, comments, explanations, or extra fields.",
].join("\n");

export function isLegacyAgentAvatar(value: string): boolean {
  return (
    value.startsWith("data:") ||
    value.startsWith("/images/") ||
    /\.(png|jpe?g|webp|gif)$/i.test(value)
  );
}

/** Deterministic avatar seed. Existing text/emoji values remain valid seeds;
 *  old image values and empty fields follow the agent name, then persisted ID. */
export function agentAvatarSeed(agent: {
  id?: string;
  name?: string;
  avatar?: string;
}): string {
  const raw = (agent.avatar ?? "").trim();
  if (raw && !isLegacyAgentAvatar(raw)) return raw;
  const name = (agent.name ?? "").trim();
  return name || (agent.id ?? "").trim();
}

export function isAgentDraftModel(model: string): boolean {
  return isUsableChatModel(model);
}

function cleanDraftText(value: unknown, maxLength: number): string {
  const text = typeof value === "string" ? value.trim() : "";
  return text.length > maxLength ? text.slice(0, maxLength).trim() : text;
}

function extractJsonObject(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) throw new Error("The model returned an empty draft.");
  const fence = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i);
  if (fence) return extractJsonObject(fence[1]);

  const start = trimmed.indexOf("{");
  if (start < 0) throw new Error("The model did not return agent JSON.");

  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = start; i < trimmed.length; i += 1) {
    const ch = trimmed[i];
    if (inString) {
      if (escaped) {
        escaped = false;
      } else if (ch === "\\") {
        escaped = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }

    if (ch === '"') {
      inString = true;
    } else if (ch === "{") {
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) return trimmed.slice(start, i + 1);
    }
  }

  throw new Error("The model returned incomplete agent JSON.");
}

export function parseAgentDraftResponse(text: string): AgentDraft {
  let parsed: unknown;
  try {
    parsed = JSON.parse(extractJsonObject(text));
  } catch (error) {
    if (error instanceof Error) throw error;
    throw new Error("The model returned invalid agent JSON.");
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("The model returned invalid agent JSON.");
  }

  const raw = parsed as Record<string, unknown>;
  const name = cleanDraftText(raw.name, 60).replace(/\s+/g, " ");
  const systemPrompt = cleanDraftText(
    raw.system_prompt ?? raw.systemPrompt,
    6000,
  );
  const description = cleanDraftText(raw.description, 240).replace(/\s+/g, " ");
  const rawAvatar = cleanDraftText(raw.avatar, 64);
  const avatar =
    rawAvatar && !isLegacyAgentAvatar(rawAvatar)
      ? rawAvatar
      : agentAvatarSeed({ name });

  if (!name) throw new Error("The model returned a draft without a name.");
  if (!systemPrompt)
    throw new Error("The model returned a draft without a system prompt.");

  return { name, description, system_prompt: systemPrompt, avatar };
}

export interface ToolInfo {
  name: string;
  description: string;
  effect: "read_only" | "mutating" | "command" | "unknown";
  input_schema?: unknown;
}

export async function listAgents(): Promise<Agent[]> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 2500);
    const r = await authFetch(`${BASE}/agents`, { signal: ctrl.signal });
    clearTimeout(t);
    const j = await r.json();
    return (j.agents ?? []).map(normalizeAgent);
  } catch {
    return [];
  }
}

export async function saveAgent(
  a: Omit<Agent, "id"> & { id?: string },
): Promise<Agent | null> {
  const body = JSON.stringify({
    name: a.name,
    description: a.description,
    model: a.model,
    system_prompt: a.system_prompt,
    tool_mode: a.tool_mode,
    enabled_tools: a.enabled_tools,
    skill_mode: a.skill_mode,
    enabled_skills: a.enabled_skills,
    avatar: a.avatar,
  });
  const headers = { "Content-Type": "application/json" };
  try {
    const r = a.id
      ? await authFetch(`${BASE}/agents/${encodeURIComponent(a.id)}`, {
          method: "PUT",
          headers,
          body,
        })
      : await authFetch(`${BASE}/agents`, { method: "POST", headers, body });
    return r.ok ? normalizeAgent(await r.json()) : null;
  } catch {
    return null;
  }
}

export async function generateAgentDraft(
  prompt: string,
  model: string,
): Promise<AgentDraft> {
  const request = prompt.trim();
  if (!request) throw new Error("Describe the agent first.");
  const draftModel = model.trim();
  if (!isAgentDraftModel(draftModel))
    throw new Error("Choose a model before drafting an agent.");

  const r = await authFetch(`${BASE}/v1/chat/completions`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model: draftModel,
      temperature: 0.2,
      max_tokens: 900,
      messages: [
        { role: "system", content: AGENT_DRAFT_SYSTEM_PROMPT },
        {
          role: "user",
          content: `Draft an agent profile from this request:\n${request}`,
        },
      ],
    }),
  });

  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `agent draft HTTP ${r.status}`),
    );
  }

  const json = await r.json();
  const content = json.choices?.[0]?.message?.content;
  if (typeof content !== "string")
    throw new Error("The model returned an empty draft.");
  return parseAgentDraftResponse(content);
}

function normalizeAgent(
  a: Partial<Agent> & { enabled_tools?: string[] },
): Agent {
  const enabledTools = a.enabled_tools ?? [];
  const toolMode =
    a.tool_mode === "all" || a.tool_mode === "custom" || a.tool_mode === "none"
      ? a.tool_mode
      : enabledTools.length === 0
        ? "all"
        : "custom";
  const enabledSkills = a.enabled_skills ?? [];
  const skillMode =
    a.skill_mode === "auto" ||
    a.skill_mode === "custom" ||
    a.skill_mode === "none"
      ? a.skill_mode
      : enabledSkills.length === 0
        ? "auto"
        : "custom";
  const model = a.model ?? "";
  return {
    id: a.id ?? "",
    name: a.name ?? "",
    description: a.description ?? "",
    system_prompt: a.system_prompt ?? "",
    model: isUsableChatModel(model) ? model : "",
    tool_mode: toolMode,
    enabled_tools: enabledTools,
    skill_mode: skillMode,
    enabled_skills: enabledSkills,
    avatar: a.avatar ?? "",
  };
}

export async function deleteAgent(id: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/agents/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return r.ok;
  } catch {
    return false;
  }
}

export async function listTools(): Promise<ToolInfo[]> {
  try {
    const r = await authFetch(`${BASE}/mcp/tools`);
    const j = await r.json();
    return (j.tools ?? []).map((t: { name: string; description?: string; effect?: ToolInfo["effect"]; input_schema?: unknown }) => ({
      name: t.name,
      description: t.description ?? "",
      effect: t.effect ?? "unknown",
      input_schema: t.input_schema,
    }));
  } catch {
    return [];
  }
}

/** Stream a server-side agent run (tool-use loop) via SSE. Calls onEvent for
 *  each parsed AgentEvent. Uses /agents/{id}/run when agentId is set, else
 *  /agents/run. */
export async function streamAgentRun(
  agentId: string | null,
  model: string,
  messages: ChatMessage[],
  onEvent: (ev: AgentEvent) => void,
  signal?: AbortSignal,
  memoryContext?: AgentMemoryContext,
  toolContext?: AgentToolContext,
  reasoningEffort?: ReasoningEffort,
): Promise<void> {
  const url = agentId
    ? `${BASE}/agents/${encodeURIComponent(agentId)}/run`
    : `${BASE}/agents/run`;
  const resp = await authFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      model,
      messages: wireMessages(messages),
      stream: true,
      ...(memoryContext ?? {}),
      ...(toolContext ?? {}),
      ...reasoningEffortBody(reasoningEffort),
    }),
    signal,
  });
  if (!resp.ok || !resp.body)
    throw new Error(
      await responseErrorMessage(resp, `agent run HTTP ${resp.status}`),
    );

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let nl: number;
    while ((nl = buffer.indexOf("\n")) >= 0) {
      const line = buffer.slice(0, nl).trim();
      buffer = buffer.slice(nl + 1);
      if (!line.startsWith("data:")) continue;
      const data = line.slice(5).trim();
      if (data === "[DONE]") return;
      try {
        onEvent(JSON.parse(data) as AgentEvent);
      } catch {
        /* keepalive / partial */
      }
    }
  }
}

export interface AgentMemoryContext {
  memory_enabled?: boolean;
  thread_id?: string;
  thread_label?: string;
  project_locator?: string;
  project_label?: string;
  message_id?: string;
}

export type ToolApprovalMode = "review" | "guarded" | "open";
export type DelegationPolicy = "off" | "ask" | "auto";
export type WorkerRunPolicy = Exclude<DelegationPolicy, "off">;
export type WorkerRunRuntime = "managed" | "codex" | "claude" | "legacy";
export type WorkerAccess = "read_only" | "write_review";
export type WorkerRunStatus =
  | "proposed"
  | "running"
  | "done"
  | "partial"
  | "stopped"
  | "error";

export interface AgentToolContext {
  workspace?: string | null;
  privacy_mode?: PrivacyMode;
  tool_approval_policy?: ToolApprovalMode;
  tool_approval_grant?: boolean;
  interactive_tool_approval?: boolean;
  sandbox_enabled?: boolean;
  computer_use_enabled?: boolean;
  preview_tools_enabled?: boolean;
  preview_runtime_key?: string;
  preview_surface?: PreviewSurfaceTarget | null;
  experimental_hashline_patch?: boolean;
  plan_mode?: boolean;
  delegation_policy?: DelegationPolicy;
  worker_model?: string;
  skill_mode?: AgentSkillMode;
  enabled_skills?: string[];
  skills_resolved?: boolean;
}

export interface AccountRuntimeMilimContext {
  tool_context: AgentToolContext;
  memory_context: AgentMemoryContext;
  tool_mode: "all" | "custom" | "none";
  enabled_tools: string[];
  skill_mode: AgentSkillMode;
  enabled_skills: string[];
}

export type ChildThreadStatus =
  "queued" | "running" | "done" | "stopped" | "error";

export interface ChildThreadInfo {
  id: string;
  parent_id: string;
  root_id: string;
  title: string;
  status: ChildThreadStatus;
  model: string;
  agent_id?: string | null;
  prompt: string;
  summary?: string | null;
  error?: string | null;
  created_at: string;
  updated_at: string;
  finished_at?: string | null;
  run_id?: string | null;
  runtime?: WorkerRunRuntime;
  access?: WorkerAccess;
  external_runtime_id?: string | null;
  worktree_path?: string | null;
}

export interface WorkerPlanTask {
  id: string;
  title: string;
  prompt: string;
  role?: string | null;
  agent_id?: string | null;
  model: string;
  access: WorkerAccess;
}

export interface WorkerRun {
  id: string;
  parent_thread_id: string;
  parent_turn_id?: string | null;
  workspace?: string | null;
  privacy_mode?: PrivacyMode | null;
  policy: WorkerRunPolicy;
  runtime: WorkerRunRuntime;
  status: WorkerRunStatus;
  tasks: WorkerPlanTask[];
  error?: string | null;
  created_at: string;
  updated_at: string;
  finished_at?: string | null;
}

export type Worker = ChildThreadInfo & {
  run_id?: string | null;
  runtime: WorkerRunRuntime;
  access: WorkerAccess;
  external_runtime_id?: string | null;
  worktree_path?: string | null;
};

export interface WorkerRunRecord {
  run: WorkerRun;
  workers: Worker[];
}

export interface CreateWorkerRunRequest {
  parent_thread_id: string;
  parent_turn_id?: string;
  workspace?: string | null;
  privacy_mode?: PrivacyMode;
  policy?: WorkerRunPolicy;
  runtime?: Exclude<WorkerRunRuntime, "legacy">;
  model?: string;
  tasks: Array<{
    title?: string;
    prompt: string;
    role?: string;
    agent_id?: string;
    model?: string;
    access?: WorkerAccess;
  }>;
}

export interface ThreadEvent {
  id: string;
  thread_id: string;
  seq: number;
  kind: string;
  payload: unknown;
  created_at: string;
}

export async function streamChildThreadEvents(
  parentId: string,
  onEvent: (ev: AgentEvent) => void,
  signal?: AbortSignal,
  afterSeq?: number,
): Promise<void> {
  const url = new URL(`${BASE}/threads/${encodeURIComponent(parentId)}/events`);
  if (typeof afterSeq === "number" && Number.isFinite(afterSeq))
    url.searchParams.set(
      "after_seq",
      String(Math.max(0, Math.floor(afterSeq))),
    );
  const resp = await authFetch(url.toString(), signal ? { signal } : undefined);
  if (!resp.ok || !resp.body)
    throw new Error(
      await responseErrorMessage(
        resp,
        `child thread events HTTP ${resp.status}`,
      ),
    );
  await streamJsonSse(resp, onEvent);
}

export async function stopChildThread(id: string): Promise<ChildThreadInfo> {
  const data = await parseJsonResponse<{ thread: ChildThreadInfo }>(
    await authFetch(`${BASE}/threads/${encodeURIComponent(id)}/stop`, {
      method: "POST",
    }),
    `child thread stop HTTP failed`,
  );
  return data.thread;
}

function workerRunRecord(data: {
  run: WorkerRun;
  workers?: Worker[];
}): WorkerRunRecord {
  return { run: data.run, workers: data.workers ?? [] };
}

export async function createWorkerRun(
  request: CreateWorkerRunRequest,
): Promise<WorkerRunRecord> {
  return workerRunRecord(
    await parseJsonResponse<{ run: WorkerRun; workers?: Worker[] }>(
      await authFetch(`${BASE}/worker-runs`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(request),
      }),
      "worker run create HTTP failed",
    ),
  );
}

export async function listWorkerRuns(
  parentThreadId: string,
): Promise<WorkerRunRecord[]> {
  const url = new URL(`${BASE}/worker-runs`);
  url.searchParams.set("parent_thread_id", parentThreadId);
  const data = await parseJsonResponse<{
    runs: Array<WorkerRun | WorkerRunRecord>;
  }>(await authFetch(url.toString()), "worker runs list HTTP failed");
  return data.runs.map((item) =>
    "run" in item
      ? workerRunRecord(item)
      : { run: item, workers: [] },
  );
}

export async function getWorkerRun(id: string): Promise<WorkerRunRecord> {
  return workerRunRecord(
    await parseJsonResponse<{ run: WorkerRun; workers?: Worker[] }>(
      await authFetch(`${BASE}/worker-runs/${encodeURIComponent(id)}`),
      "worker run HTTP failed",
    ),
  );
}

export async function startWorkerRun(id: string): Promise<WorkerRunRecord> {
  return workerRunRecord(
    await parseJsonResponse<{ run: WorkerRun; workers?: Worker[] }>(
      await authFetch(`${BASE}/worker-runs/${encodeURIComponent(id)}/start`, {
        method: "POST",
      }),
      "worker run start HTTP failed",
    ),
  );
}

export async function retryWorkerTask(
  runId: string,
  taskId: string,
  model?: string,
): Promise<WorkerRunRecord> {
  return workerRunRecord(
    await parseJsonResponse<{ run: WorkerRun; workers?: Worker[] }>(
      await authFetch(
        `${BASE}/worker-runs/${encodeURIComponent(runId)}/tasks/${encodeURIComponent(taskId)}/retry`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(model ? { model } : {}),
        },
      ),
      "worker retry HTTP failed",
    ),
  );
}

export async function deleteWorkerRun(id: string): Promise<void> {
  await parseJsonResponse<{ deleted: boolean }>(
    await authFetch(`${BASE}/worker-runs/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
    "worker run delete HTTP failed",
  );
}

export async function stopWorkerRun(id: string): Promise<WorkerRunRecord> {
  return workerRunRecord(
    await parseJsonResponse<{ run: WorkerRun; workers?: Worker[] }>(
      await authFetch(`${BASE}/worker-runs/${encodeURIComponent(id)}/stop`, {
        method: "POST",
      }),
      "worker run stop HTTP failed",
    ),
  );
}

export async function stopWorker(
  runId: string,
  workerId: string,
): Promise<{ run?: WorkerRun | null; worker: Worker }> {
  return await parseJsonResponse<{ run?: WorkerRun | null; worker: Worker }>(
    await authFetch(
      `${BASE}/worker-runs/${encodeURIComponent(runId)}/workers/${encodeURIComponent(workerId)}/stop`,
      { method: "POST" },
    ),
    "worker stop HTTP failed",
  );
}

export interface WorkerDiffReview {
  worker_id: string;
  status: WorkspaceGitStatus;
  diff: string;
}

export async function getWorkerDiff(
  runId: string,
  workerId: string,
): Promise<WorkerDiffReview> {
  return await parseJsonResponse<WorkerDiffReview>(
    await authFetch(
      `${BASE}/worker-runs/${encodeURIComponent(runId)}/workers/${encodeURIComponent(workerId)}/diff`,
    ),
    "worker diff HTTP failed",
  );
}

export async function applyWorkerDiff(
  runId: string,
  workerId: string,
): Promise<WorkspaceGitActionResult> {
  return await parseJsonResponse<WorkspaceGitActionResult>(
    await authFetch(
      `${BASE}/worker-runs/${encodeURIComponent(runId)}/workers/${encodeURIComponent(workerId)}/apply`,
      { method: "POST" },
    ),
    "worker diff apply HTTP failed",
  );
}

export async function streamWorkerRunEvents(
  id: string,
  onEvent: (ev: AgentEvent) => void,
  signal?: AbortSignal,
  afterSeq?: number,
): Promise<void> {
  const url = new URL(`${BASE}/worker-runs/${encodeURIComponent(id)}/events`);
  if (typeof afterSeq === "number" && Number.isFinite(afterSeq))
    url.searchParams.set(
      "after_seq",
      String(Math.max(0, Math.floor(afterSeq))),
    );
  const resp = await authFetch(url.toString(), signal ? { signal } : undefined);
  if (!resp.ok || !resp.body)
    throw new Error(
      await responseErrorMessage(resp, `worker run events HTTP ${resp.status}`),
    );
  await streamJsonSse(resp, onEvent);
}

export async function deleteThreadTree(id: string): Promise<void> {
  await parseJsonResponse<{ deleted: number }>(
    await authFetch(`${BASE}/threads/${encodeURIComponent(id)}`, {
      method: "DELETE",
    }),
    `thread delete HTTP failed`,
  );
}

/** A parsed SSE event from an agent run (milim-agents `AgentEvent`, tagged by
 *  `type`). */
export interface AgentEvent {
  type?:
    | "start"
    | "token"
    | "reasoning"
    | "usage_delta"
    | "tool_call"
    | "tool_result"
    | "tool_approval_required"
    | "tool_approval_status"
    | "tool_approval_resolved"
    | "tool_approval_failed"
    | "memory_registered"
    | "child_thread_started"
    | "child_thread_done"
    | "child_thread_error"
    | "child_thread_stopped"
    | "child_thread_event"
    | "worker_run_proposed"
    | "worker_run_started"
    | "worker_run_done"
    | "worker_run_error"
    | "worker_run_worker_event"
    | "worker_run_worker_started"
    | "worker_run_worker_done"
    | "worker_run_worker_error"
    | "worker_run_worker_stopped"
    | "final"
    | "done"
    | "error";
  text?: string;
  call_id?: string;
  name?: string;
  arguments?: string;
  result?: unknown;
  mcp_app?: ToolUiDescriptor;
  mcp_app_result?: unknown;
  content?: string;
  message?: string;
  iterations?: number;
  /** `start`: the model that actually ran (agent runs may override the request). */
  model?: string;
  /** Legacy field; current agent runs do not stop at an iteration limit. */
  stopped_at_limit?: boolean;
  usage?: TokenUsage;
  id?: string;
  node_id?: string;
  scope_kind?: MemoryScopeKind;
  scope_label?: string;
  summary?: string;
  created_at?: string;
  thread?: ChildThreadInfo;
  event?: ThreadEvent;
  run?: WorkerRun;
  workers?: Worker[];
  worker?: Worker;
  run_id?: string;
  approval_id?: string;
  effect?: "read_only" | "mutating" | "command" | "unknown";
  decision?: "approve" | "deny";
  status?: "decided" | "delivered";
}

export type ToolApprovalEvent =
  | {
      type: "tool_approval_required";
      approval_id: string;
      call_id?: string;
      name: string;
      arguments: string;
      effect: "read_only" | "mutating" | "command" | "unknown";
      request_kind?: ToolApprovalRequestKind;
      request?: Record<string, unknown>;
    }
  | {
      type: "tool_approval_status";
      approval_id: string;
      call_id?: string;
      decision: "approve" | "deny";
      status: "decided" | "delivered";
    }
  | {
      type: "tool_approval_resolved";
      approval_id: string;
      call_id?: string;
      decision: "approve" | "deny";
    }
  | {
      type: "tool_approval_failed";
      approval_id: string;
      call_id?: string;
      decision?: "approve" | "deny";
      message: string;
    };

export type ToolApprovalRequestKind =
  | "command"
  | "file_change"
  | "permissions"
  | "mcp_form"
  | "mcp_url"
  | "mcp_unsupported";

export interface McpApprovalField {
  name: string;
  label: string;
  description?: string | null;
  kind: "string" | "number" | "integer" | "boolean" | "enum";
  value_type?: "string" | "number" | "integer" | "boolean";
  required: boolean;
  default?: unknown;
  minimum?: number;
  maximum?: number;
  min_length?: number;
  max_length?: number;
  options?: Array<{ value: string | number | boolean; label: string }>;
}

export type ToolApprovalRequest =
  | { kind: "command" | "file_change" }
  | { kind: "permissions"; reason?: string | null; cwd?: string | null; permissions?: unknown }
  | { kind: "mcp_form"; server_name: string; message: string; fields: McpApprovalField[] }
  | { kind: "mcp_url"; server_name: string; message: string; url: string }
  | { kind: "mcp_unsupported"; server_name: string; message: string; reason: string };

// ----- Providers (LLM remotes and media credentials) -----

export type ProviderKind =
  "openai_compatible" | "anthropic" | "gemini" | "replicate" | "fal";

export interface ProviderInfo {
  id: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  enabled: boolean;
  has_key: boolean;
  models: string[];
  pricing?: Record<string, ProviderModelPricing>;
  model_reasoning?: Record<string, ModelReasoningMetadata>;
  model_context?: Record<string, {
    context_length?: number;
    max_prompt_tokens?: number;
    max_completion_tokens?: number;
  }>;
  model_capabilities?: Record<string, {
    image_input?: boolean;
    tool_use?: boolean;
  }>;
  model_overrides?: Record<string, ModelCapabilityOverride>;
  error?: string | null;
}

export interface ModelCapabilityOverride {
  image_input?: boolean;
  tool_use?: boolean;
  reasoning?: boolean;
  supported_efforts?: ReasoningEffort[];
}

export interface CapabilityProbeResult {
  supported: boolean;
  error?: string;
}

export interface ModelCapabilityVerification {
  model: string;
  vision: CapabilityProbeResult;
  reasoning: CapabilityProbeResult;
  tools: CapabilityProbeResult;
}

export interface ProviderModelPricing {
  prompt?: string | null;
  completion?: string | null;
}

export interface ProviderDiscovery {
  name: string;
  kind: ProviderKind;
  base_url: string;
  configured: boolean;
  provider_id?: string | null;
  reachable: boolean;
  models: string[];
  error?: string | null;
}

/** One-click presets; base URLs include the provider version segment. */
export const PROVIDER_PRESETS: Array<{
  name: string;
  kind: ProviderKind;
  base_url: string;
  needsKey: boolean;
}> = [
  {
    name: "OpenAI",
    kind: "openai_compatible",
    base_url: "https://api.openai.com/v1",
    needsKey: true,
  },
  {
    name: "OpenRouter",
    kind: "openai_compatible",
    base_url: "https://openrouter.ai/api/v1",
    needsKey: true,
  },
  {
    name: "Groq",
    kind: "openai_compatible",
    base_url: "https://api.groq.com/openai/v1",
    needsKey: true,
  },
  {
    name: "Anthropic",
    kind: "anthropic",
    base_url: "https://api.anthropic.com/v1",
    needsKey: true,
  },
  {
    name: "Gemini",
    kind: "gemini",
    base_url: "https://generativelanguage.googleapis.com/v1beta",
    needsKey: true,
  },
  {
    name: "Replicate",
    kind: "replicate",
    base_url: "https://api.replicate.com/v1",
    needsKey: true,
  },
  {
    name: "fal",
    kind: "fal",
    base_url: "https://queue.fal.run",
    needsKey: true,
  },
  {
    name: "Ollama (local)",
    kind: "openai_compatible",
    base_url: "http://localhost:11434/v1",
    needsKey: false,
  },
  {
    name: "LM Studio (local)",
    kind: "openai_compatible",
    base_url: "http://localhost:1234/v1",
    needsKey: false,
  },
];

export async function listProviders(): Promise<ProviderInfo[]> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 2500);
    const r = await authFetch(`${BASE}/providers`, { signal: ctrl.signal });
    clearTimeout(t);
    const j = await r.json();
    return (j.providers ?? []) as ProviderInfo[];
  } catch {
    return [];
  }
}

export async function discoverLocalProviders(): Promise<ProviderDiscovery[]> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 5000);
    const r = await authFetch(`${BASE}/providers/discover`, {
      signal: ctrl.signal,
    });
    clearTimeout(t);
    const j = await r.json();
    return (j.providers ?? []) as ProviderDiscovery[];
  } catch {
    return [];
  }
}

export async function saveProvider(p: {
  id?: string;
  name: string;
  kind: ProviderKind;
  base_url: string;
  api_key?: string;
  enabled: boolean;
  model_overrides?: Record<string, ModelCapabilityOverride>;
}): Promise<ProviderInfo | null> {
  try {
    const r = await authFetch(`${BASE}/providers`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(p),
    });
    return r.ok ? ((await r.json()) as ProviderInfo) : null;
  } catch {
    return null;
  }
}

export async function verifyProviderModelCapabilities(
  providerId: string,
  model: string,
): Promise<ModelCapabilityVerification | null> {
  try {
    const r = await authFetch(
      `${BASE}/providers/${encodeURIComponent(providerId)}/models/verify`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ model }),
      },
    );
    return r.ok ? ((await r.json()) as ModelCapabilityVerification) : null;
  } catch {
    return null;
  }
}

export async function deleteProvider(id: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/providers/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return r.ok;
  } catch {
    return false;
  }
}

// ----- Media generation -----

export type MediaKind = "image" | "video" | "music";

export interface MediaGenerationRequest {
  provider_id: string;
  kind: MediaKind;
  model: string;
  prompt: string;
  input?: Record<string, unknown>;
}

export interface MediaModelInfo {
  id: string;
  name: string;
  description: string;
  output_modalities: string[];
  supported_parameters: string[];
  default_parameters?: Record<string, unknown> | null;
  pricing?: Record<string, unknown> | null;
}

export interface MediaModelListOptions {
  query?: string;
  refresh?: boolean;
}

export interface MediaSchemaControlOption {
  label: string;
  value: unknown;
}

export interface MediaSchemaControl {
  key: string;
  label: string;
  kind: "select" | "number" | string;
  path: string[];
  description?: string;
  options?: MediaSchemaControlOption[];
  default?: unknown;
  min?: number;
  max?: number;
  step?: number;
  item_kind?: string;
  placeholder?: string;
}

export interface MediaModelSchema {
  model: string;
  provider_id: string;
  provider: string;
  supported_parameters: string[];
  controls: MediaSchemaControl[];
  raw?: unknown;
}

export interface MediaStatusRequest {
  provider_id: string;
  id: string;
  model?: string;
  response_url?: string;
  status_url?: string;
  kind?: MediaKind;
}

export interface MediaResultItem {
  url: string;
  kind: MediaKind | string;
  mime?: string | null;
  requires_auth?: boolean;
}

export interface MediaGenerationResult {
  id: string;
  object: "media.generation";
  provider_id: string;
  provider: string;
  provider_kind: ProviderKind;
  kind: MediaKind | string;
  model: string;
  status: string;
  output?: unknown;
  media: MediaResultItem[];
  urls: Record<string, string>;
  library_id?: string | null;
  save_state?: MediaLibraryStatus | null;
  privacy: {
    mode: PrivacyMode;
    redacted: boolean;
    detections: number;
    kinds: string;
  };
  raw?: unknown;
}

export type MediaLibraryStatus = "running" | "saving" | "ready" | "failed";

export interface MediaLibraryMediaItem extends MediaResultItem {
  source_url: string;
  file_name?: string | null;
  local_path?: string | null;
  size_bytes?: number | null;
}

export interface MediaLibraryItem {
  id: string;
  provider_run_id: string;
  created_at_ms: number;
  updated_at_ms: number;
  provider_id: string;
  provider: string;
  provider_kind: ProviderKind;
  kind: MediaKind;
  model: string;
  prompt: string;
  input: Record<string, unknown>;
  status: string;
  save_state: MediaLibraryStatus;
  error?: string | null;
  privacy: {
    mode: PrivacyMode;
    redacted: boolean;
    detections?: number;
    kinds?: string;
  };
  urls: Record<string, string>;
  media: MediaLibraryMediaItem[];
}

export interface MediaLibraryPage {
  items: MediaLibraryItem[];
  next_cursor?: string | null;
}

export interface MediaLibraryListOptions {
  query?: string;
  kind?: MediaKind;
  provider?: string;
  status?: MediaLibraryStatus;
  cursor?: string;
  limit?: number;
}

export function isOpenRouterProvider(
  provider: Pick<ProviderInfo, "kind" | "name" | "base_url">,
): boolean {
  return (
    provider.kind === "openai_compatible" &&
    (provider.name.trim().toLowerCase() === "openrouter" ||
      provider.base_url.trim().toLowerCase().includes("openrouter.ai/"))
  );
}

export function supportsMediaMetadataProvider(
  provider: Pick<ProviderInfo, "kind" | "name" | "base_url">,
): boolean {
  return (
    provider.kind === "replicate" ||
    provider.kind === "fal" ||
    (provider.kind === "openai_compatible" && isOpenRouterProvider(provider))
  );
}

export function mediaProviders(providers: ProviderInfo[]): ProviderInfo[] {
  return providers.filter(
    (provider) => provider.enabled && supportsMediaMetadataProvider(provider),
  );
}

export async function generateMedia(
  request: MediaGenerationRequest,
  signal?: AbortSignal,
): Promise<MediaGenerationResult> {
  const r = await authFetch(`${BASE}/media/generate`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(request),
    signal,
  });
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `media generation HTTP ${r.status}`),
    );
  }
  return (await r.json()) as MediaGenerationResult;
}

export async function listMediaModels(
  providerId: string,
  kind: MediaKind = "image",
  options: MediaModelListOptions = {},
): Promise<MediaModelInfo[]> {
  const url = new URL(`${BASE}/media/models`);
  url.searchParams.set("provider_id", providerId);
  url.searchParams.set("kind", kind);
  if (options.query?.trim()) url.searchParams.set("q", options.query.trim());
  if (options.refresh) url.searchParams.set("refresh", "true");
  const r = await authFetch(url);
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `media models HTTP ${r.status}`),
    );
  }
  const j = await r.json();
  return (j.models ?? []) as MediaModelInfo[];
}

export async function getMediaModelSchema(
  providerId: string,
  model: string,
  kind: MediaKind = "image",
  options: { refresh?: boolean } = {},
): Promise<MediaModelSchema> {
  const url = new URL(`${BASE}/media/model-schema`);
  url.searchParams.set("provider_id", providerId);
  url.searchParams.set("model", model);
  url.searchParams.set("kind", kind);
  if (options.refresh) url.searchParams.set("refresh", "true");
  const r = await authFetch(url);
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `media model schema HTTP ${r.status}`),
    );
  }
  return (await r.json()) as MediaModelSchema;
}

export async function getMediaStatus(
  request: MediaStatusRequest,
): Promise<MediaGenerationResult> {
  const url = new URL(`${BASE}/media/status`);
  url.searchParams.set("provider_id", request.provider_id);
  url.searchParams.set("id", request.id);
  if (request.model) url.searchParams.set("model", request.model);
  if (request.response_url)
    url.searchParams.set("response_url", request.response_url);
  if (request.status_url)
    url.searchParams.set("status_url", request.status_url);
  if (request.kind) url.searchParams.set("kind", request.kind);
  const r = await authFetch(url);
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `media status HTTP ${r.status}`),
    );
  }
  return (await r.json()) as MediaGenerationResult;
}

export async function loadAuthenticatedMedia(
  url: string,
  signal?: AbortSignal,
): Promise<Blob> {
  const input = url.startsWith("/") ? `${BASE}${url}` : url;
  const r = await authFetch(input, signal ? { signal } : undefined);
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `media content HTTP ${r.status}`),
    );
  }
  return r.blob();
}

export async function listMediaLibrary(
  options: MediaLibraryListOptions = {},
): Promise<MediaLibraryPage> {
  const url = new URL(`${BASE}/media/library`);
  if (options.query) url.searchParams.set("query", options.query);
  if (options.kind) url.searchParams.set("kind", options.kind);
  if (options.provider) url.searchParams.set("provider", options.provider);
  if (options.status) url.searchParams.set("status", options.status);
  if (options.cursor) url.searchParams.set("cursor", options.cursor);
  if (options.limit) url.searchParams.set("limit", String(options.limit));
  const r = await authFetch(url);
  if (!r.ok) {
    throw new Error(await responseErrorMessage(r, `media library HTTP ${r.status}`));
  }
  return (await r.json()) as MediaLibraryPage;
}

export async function refreshMediaLibraryItem(id: string): Promise<MediaLibraryItem> {
  const r = await authFetch(`${BASE}/media/library/${encodeURIComponent(id)}/refresh`, {
    method: "POST",
  });
  if (!r.ok) {
    throw new Error(await responseErrorMessage(r, `media library refresh HTTP ${r.status}`));
  }
  return (await r.json()) as MediaLibraryItem;
}

export async function deleteMediaLibraryItem(id: string): Promise<boolean> {
  const r = await authFetch(`${BASE}/media/library/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
  if (!r.ok) {
    throw new Error(await responseErrorMessage(r, `media library delete HTTP ${r.status}`));
  }
  return Boolean(((await r.json()) as { deleted?: boolean }).deleted);
}

/**
 * Point the host filesystem/shell tools at `folder` (the working folder). Pass
 * "" to clear. The agent's read_file/write_file/edit_file/list_dir/shell tools
 * operate within this folder; until it's set they refuse to run.
 */
export async function setWorkspace(folder: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/workspace`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ folder }),
    });
    return r.ok;
  } catch {
    return false;
  }
}

export type WorkspaceGitState = "no_folder" | "not_git" | "ready" | "error";

export interface WorkspaceGitStatus {
  state: WorkspaceGitState;
  folder: string | null;
  is_repo: boolean;
  root: string | null;
  branch: string | null;
  head: string | null;
  upstream: string | null;
  remote: string | null;
  ahead: number;
  behind: number;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicts: number;
  insertions: number;
  deletions: number;
  has_changes: boolean;
  changed_file_count: number;
  changed_files: WorkspaceGitFileChange[];
  branches: WorkspaceGitBranch[];
  recent_commits: WorkspaceGitCommit[];
  message: string | null;
}

export interface WorkspaceInstruction {
  family: "agents" | "claude";
  scope: "global" | "project";
  path: string;
  content: string;
  bytes: number;
  status: "loaded" | "conditional" | "limit_exceeded";
}

export interface WorkspaceContext {
  root: string | null;
  project_locator: string | null;
  legacy_project_locator: string | null;
  origin: string | null;
  instructions: WorkspaceInstruction[];
  warnings: string[];
}

export interface WorkspaceGitFileChange {
  status: string;
  path: string;
}

export interface WorkspaceGitBranch {
  name: string;
  current: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
}

export interface WorkspaceGitCommit {
  hash: string;
  subject: string;
}

export type WorkspaceGitAction =
  | "diff"
  | "fetch"
  | "pull"
  | "push"
  | "publish"
  | "commit"
  | "commit_push"
  | "checkout_branch"
  | "create_branch"
  | "checkpoint"
  | "restore_checkpoint"
  | "create_retry_worktree"
  | "apply_retry_worktree"
  | "remove_retry_worktree"
  | "create_thread_worktree"
  | "remove_thread_worktree"
  | "pr_list"
  | "pr_view"
  | "pr_status"
  | "pr_create"
  | "pr_ready"
  | "pr_comment"
  | "pr_review"
  | "pr_merge";

export type WorkspaceGitDiffScope =
  | "all"
  | "unstaged"
  | "staged"
  | "last_turn"
  | "commit"
  | "branch";

export interface WorkspaceGitActionResult {
  ok: boolean;
  action: WorkspaceGitAction;
  command: string;
  stdout: string;
  stderr: string;
  exit_code: number | null;
  message: string;
  truncated: boolean;
  checkpoint?: string;
  root?: string;
  head?: string;
  worktree?: string;
  undo_checkpoint?: string;
  conflicts?: string[];
  pull_request?: PullRequestDetails | null;
}

export interface PullRequestActor {
  login: string;
  name?: string;
  avatarUrl?: string;
}

export interface PullRequestCheck {
  bucket: "pass" | "fail" | "pending" | "skipping" | "cancel" | string;
  name: string;
  state: string;
  workflow?: string;
  description?: string;
  link?: string;
  startedAt?: string;
  completedAt?: string;
}

export interface PullRequestReview {
  author?: PullRequestActor;
  state: string;
  body?: string;
  submittedAt?: string;
}

export interface PullRequestComment {
  author?: PullRequestActor;
  body: string;
  createdAt?: string;
  url?: string;
}

export interface PullRequestFile {
  path: string;
  additions: number;
  deletions: number;
}

export interface PullRequestListItem {
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
  body?: string;
  author?: PullRequestActor;
  repository: string;
  updatedAt?: string;
  commentsCount: number;
  authored: boolean;
  reviewing: boolean;
}

export interface PullRequestDetails {
  exists: true;
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
  baseRefName: string;
  headRefName: string;
  headRefOid: string;
  body: string;
  author?: PullRequestActor;
  reviewRequests?: Array<{ login?: string; name?: string }>;
  latestReviews?: PullRequestReview[];
  reviewDecision?: string;
  mergeStateStatus?: string;
  mergeable?: string;
  viewerPermission?: string;
  comments?: PullRequestComment[];
  checks?: PullRequestCheck[];
  checksError?: string;
  additions: number;
  deletions: number;
  changedFiles: number;
  updatedAt?: string;
  repository?: string;
  files?: PullRequestFile[];
}

export async function getWorkspaceGitStatus(): Promise<WorkspaceGitStatus | null> {
  try {
    const r = await authFetch(`${BASE}/workspace/git`);
    return r.ok ? ((await r.json()) as WorkspaceGitStatus) : null;
  } catch {
    return null;
  }
}

export async function getWorkspaceContext(): Promise<WorkspaceContext | null> {
  try {
    const r = await authFetch(`${BASE}/workspace/context`);
    return r.ok ? ((await r.json()) as WorkspaceContext) : null;
  } catch {
    return null;
  }
}

export async function resolveToolApproval(
  approvalId: string,
  decision: "approve" | "deny",
  responseBody?: Record<string, unknown>,
): Promise<ToolApprovalSnapshot | null> {
  const response = await authFetch(`${BASE}/tool-approvals/${encodeURIComponent(approvalId)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ decision, ...(responseBody ? { response: responseBody } : {}) }),
  });
  if (!response.ok) throw new Error(await responseErrorMessage(response, "Tool approval failed"));
  return response.status === 204 ? null : ((await response.json()) as ToolApprovalSnapshot);
}

export interface ToolApprovalSnapshot {
  approval_id: string;
  state: "requested" | "decided" | "delivered" | "acknowledged" | "failed" | "canceled";
  decision?: "approve" | "deny" | null;
  error?: string | null;
  created_at_ms: number;
  updated_at_ms: number;
}

export async function getToolApproval(approvalId: string): Promise<ToolApprovalSnapshot> {
  return parseJsonResponse<ToolApprovalSnapshot>(
    await authFetch(`${BASE}/tool-approvals/${encodeURIComponent(approvalId)}`),
    "Tool approval status failed",
  );
}

export async function runWorkspaceGitAction(
  action: WorkspaceGitAction,
  options: {
    message?: string;
    checkpoint?: string;
    stage_all?: boolean;
    staged_only?: boolean;
    diff_scope?: WorkspaceGitDiffScope;
    diff_base?: string;
    branch?: string;
    worktree?: string;
    thread_id?: string;
    force?: boolean;
    title?: string;
    body?: string;
    base?: string;
    draft?: boolean;
    folder?: string;
    pull_request?: number;
    review_action?: "approve" | "request_changes" | "comment";
    merge_method?: "merge" | "squash" | "rebase";
    expected_head?: string;
    repository?: string;
  } = {},
): Promise<WorkspaceGitActionResult> {
  const r = await authFetch(`${BASE}/workspace/git/action`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ action, ...options }),
  });
  if (r.status === 404)
    throw new Error(
      "Restart the desktop backend to enable Git command buttons.",
    );
  if (!r.ok)
    throw new Error(
      await responseErrorMessage(r, `git action HTTP ${r.status}`),
    );
  return (await r.json()) as WorkspaceGitActionResult;
}

/**
 * Enable/disable the computer-use layer (screen capture + mouse/keyboard).
 * Off by default; the agent's screenshot/mouse/keyboard tools refuse to run
 * until this is enabled.
 */
export async function setComputerUse(enabled: boolean): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/computer`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled }),
    });
    return r.ok;
  } catch {
    return false;
  }
}

/** Outbound privacy gate for remote providers. `redact` replaces PII with
 *  reversible placeholders; `block` refuses a remote send that contains PII.
 *  Local models are never scanned. */
export type PrivacyMode = "off" | "redact" | "block";

export interface PrivacyDetection {
  kind: string;
  value: string;
  start: number;
  end: number;
}

export interface PrivacyScanResult {
  clean: boolean;
  detections: PrivacyDetection[];
  redacted: string;
  map: Record<string, string>;
}

export async function getPrivacyMode(): Promise<PrivacyMode> {
  const r = await authFetch(`${BASE}/privacy/mode`);
  if (!r.ok) {
    throw new Error(await responseErrorMessage(r, `privacy mode HTTP ${r.status}`));
  }
  return ((await r.json()) as { mode: PrivacyMode }).mode;
}

/** Set the outbound privacy gate mode on the server (applies to remote sends). */
export async function setPrivacyMode(mode: PrivacyMode): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/privacy/mode`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ mode }),
    });
    return r.ok;
  } catch {
    return false;
  }
}

async function responseErrorMessage(
  resp: Response,
  fallback: string,
): Promise<string> {
  const text = await resp.text().catch(() => "");
  if (!text.trim()) return fallback;
  try {
    const json = JSON.parse(text);
    return String(json.error?.message ?? json.error ?? fallback);
  } catch {
    return text.trim();
  }
}

export async function scanPrivacyText(
  text: string,
): Promise<PrivacyScanResult> {
  const r = await authFetch(`${BASE}/privacy/scan`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ text }),
  });
  if (!r.ok) {
    throw new Error(
      await responseErrorMessage(r, `privacy scan HTTP ${r.status}`),
    );
  }
  return (await r.json()) as PrivacyScanResult;
}

// ----- Memory / RAG -----

export type MemoryScopeKind = "thread" | "project" | "global";

export interface MemoryScopeRef {
  kind: MemoryScopeKind;
  locator: string;
}

export interface MemoryScope {
  id: string;
  kind: MemoryScopeKind;
  label: string;
  locator: string;
  locator_hash: string;
  created_at: string;
  updated_at: string;
}

export interface MemoryNode {
  id: string;
  scope_id: string;
  scope_kind: MemoryScopeKind;
  scope_label: string;
  kind: string;
  title: string;
  body: string;
  confidence: number;
  source: string;
  created_at: string;
  updated_at: string;
  archived_at?: string | null;
}

export interface MemoryNotice {
  id: string;
  node_id: string;
  scope_kind: MemoryScopeKind;
  scope_label: string;
  summary: string;
  created_at: string;
}

export interface MemoryGraphHit {
  node: MemoryNode;
  score: number;
}

export interface RegisterMemoryInput {
  scope: {
    kind: MemoryScopeKind;
    label: string;
    locator: string;
  };
  node: {
    kind: string;
    title: string;
    body: string;
    confidence?: number;
    source?: string;
  };
  event?: {
    thread_id?: string;
    message_id?: string;
    summary?: string;
  };
  model?: string;
}

export interface MemoryRegistration {
  scope: MemoryScope;
  node: MemoryNode;
  notice: MemoryNotice;
}

export async function registerGraphMemory(
  input: RegisterMemoryInput,
): Promise<MemoryRegistration | null> {
  try {
    const memoryModel =
      input.model && isUsableChatModel(input.model) ? input.model : "default";
    const r = await authFetch(`${BASE}/memory/register`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        model: memoryModel,
        scope: input.scope,
        node: {
          confidence: 1,
          source: "user",
          ...input.node,
        },
        event: input.event ?? {},
      }),
    });
    if (!r.ok) return null;
    return (await r.json()) as MemoryRegistration;
  } catch {
    return null;
  }
}

export async function listMemoryNodes(
  options: {
    scope?: MemoryScopeRef;
    includeArchived?: boolean;
    limit?: number;
  } = {},
): Promise<MemoryNode[]> {
  try {
    const params = new URLSearchParams();
    if (options.scope) {
      params.set("scope_kind", options.scope.kind);
      params.set("scope_locator", options.scope.locator);
    }
    if (options.includeArchived) params.set("include_archived", "true");
    if (options.limit) params.set("limit", String(options.limit));
    const suffix = params.toString() ? `?${params.toString()}` : "";
    const r = await authFetch(`${BASE}/memory/nodes${suffix}`);
    if (!r.ok) return [];
    const j = await r.json();
    return (j.nodes ?? []) as MemoryNode[];
  } catch {
    return [];
  }
}

export async function searchGraphMemory(
  query: string,
  scopes: MemoryScopeRef[],
  topK = 5,
  model?: string,
  includeArchived = false,
): Promise<MemoryGraphHit[]> {
  try {
    const memoryModel = model && isUsableChatModel(model) ? model : "default";
    const r = await authFetch(`${BASE}/memory/graph/search`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        model: memoryModel,
        query,
        scopes,
        top_k: topK,
        include_archived: includeArchived,
      }),
    });
    if (!r.ok) return [];
    const j = await r.json();
    return (j.hits ?? []) as MemoryGraphHit[];
  } catch {
    return [];
  }
}

export async function updateMemoryNode(
  id: string,
  update: Partial<
    Pick<MemoryNode, "kind" | "title" | "body" | "confidence" | "source">
  >,
  model?: string,
): Promise<MemoryNode | null> {
  try {
    const memoryModel = model && isUsableChatModel(model) ? model : "default";
    const r = await authFetch(
      `${BASE}/memory/nodes/${encodeURIComponent(id)}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ model: memoryModel, ...update }),
      },
    );
    return r.ok ? ((await r.json()) as MemoryNode) : null;
  } catch {
    return null;
  }
}

export async function deleteMemoryNode(id: string): Promise<boolean> {
  try {
    const r = await authFetch(
      `${BASE}/memory/nodes/${encodeURIComponent(id)}`,
      { method: "DELETE" },
    );
    if (!r.ok) return false;
    const j = await r.json();
    return Boolean(j.deleted);
  } catch {
    return false;
  }
}

export async function archiveMemoryNode(id: string): Promise<boolean> {
  try {
    const r = await authFetch(
      `${BASE}/memory/nodes/${encodeURIComponent(id)}/archive`,
      { method: "POST" },
    );
    if (!r.ok) return false;
    const j = await r.json();
    return Boolean(j.archived);
  } catch {
    return false;
  }
}

// ----- Schedules -----

export interface ScheduleInfo {
  id: string;
  name: string;
  cron: string;
  agent_id?: string | null;
  model: string;
  prompt: string;
  attachments?: ChatAttachment[];
  enabled: boolean;
  last_run?: number | null;
}

export interface ScheduleRunEvent {
  id: string;
  schedule_id: string;
  schedule_name: string;
  prompt: string;
  response: string;
  model: string;
  ran_at: number;
}

export async function listSchedules(): Promise<ScheduleInfo[]> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 2500);
    const r = await authFetch(`${BASE}/schedules`, { signal: ctrl.signal });
    clearTimeout(t);
    if (!r.ok) return [];
    const j = await r.json();
    return (j.schedules ?? []) as ScheduleInfo[];
  } catch {
    return [];
  }
}

export async function pollScheduleRunEvents(): Promise<ScheduleRunEvent[]> {
  try {
    const r = await authFetch(`${BASE}/schedules/events`);
    if (!r.ok) return [];
    const j = await r.json();
    return (j.events ?? []) as ScheduleRunEvent[];
  } catch {
    return [];
  }
}

export async function createSchedule(s: {
  name: string;
  cron: string;
  agent_id?: string | null;
  model: string;
  prompt: string;
  attachments?: ChatAttachment[];
}): Promise<ScheduleInfo | null> {
  try {
    const r = await authFetch(`${BASE}/schedules`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(s),
    });
    return r.ok ? ((await r.json()) as ScheduleInfo) : null;
  } catch {
    return null;
  }
}

export async function updateSchedule(
  s: ScheduleInfo,
): Promise<ScheduleInfo | null> {
  try {
    const r = await authFetch(`${BASE}/schedules/${encodeURIComponent(s.id)}`, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(s),
    });
    return r.ok ? ((await r.json()) as ScheduleInfo) : null;
  } catch {
    return null;
  }
}

export async function deleteSchedule(id: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/schedules/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return r.ok;
  } catch {
    return false;
  }
}

// ----- MCP client (external MCP servers whose tools we consume) -----

export interface McpServerInfo {
  id: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  env?: McpEnvVar[];
  enabled: boolean;
  connected: boolean;
  tool_count: number;
  capabilities?: { tools: boolean; resources: boolean; prompts: boolean; apps: boolean };
  missing_env?: string[];
  error: string | null;
}

export interface McpEnvVar {
  key: string;
  value?: string | null;
  secret?: boolean;
  required?: boolean;
  has_value?: boolean;
}

export interface McpTestResult {
  ok: boolean;
  connected: boolean;
  tool_count: number;
  capabilities?: { tools: boolean; resources: boolean; prompts: boolean; apps: boolean };
  missing_env?: string[];
  error?: string | null;
}

/** Suggested MCP servers (command + args) for quick setup. */
export const MCP_PRESETS: Array<{
  name: string;
  command: string;
  args: string[];
  note?: string;
}> = [
  {
    name: "Filesystem",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "."],
    note: "Replace '.' with a folder to expose.",
  },
  {
    name: "Fetch",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-fetch"],
    note: "Web fetch as an MCP tool.",
  },
  {
    name: "Memory",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-memory"],
    note: "Knowledge-graph memory.",
  },
  {
    name: "Sequential Thinking",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-sequential-thinking"],
  },
];

export async function listMcpServers(): Promise<McpServerInfo[]> {
  try {
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), 2500);
    const r = await authFetch(`${BASE}/mcp/servers`, { signal: ctrl.signal });
    clearTimeout(t);
    const j = await r.json();
    return (j.servers ?? []) as McpServerInfo[];
  } catch {
    return [];
  }
}

/** Add/update an MCP server. Connects immediately (may take a while as the
 *  server's package is fetched), so this call is intentionally untimed. */
export async function saveMcpServer(s: {
  id?: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  env?: McpEnvVar[];
  enabled: boolean;
}): Promise<McpServerInfo | null> {
  try {
    const r = await authFetch(`${BASE}/mcp/servers`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(s),
    });
    if (!r.ok) return null;
    const j = await r.json();
    return (j.server ?? null) as McpServerInfo | null;
  } catch {
    return null;
  }
}

export async function testMcpServer(s: {
  id?: string;
  name: string;
  command: string;
  args: string[];
  cwd?: string | null;
  env?: McpEnvVar[];
  enabled: boolean;
}): Promise<McpTestResult | null> {
  try {
    const r = await authFetch(`${BASE}/mcp/servers/test`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(s),
    });
    if (!r.ok) return null;
    return (await r.json()) as McpTestResult;
  } catch {
    return null;
  }
}

export async function testSavedMcpServer(id: string): Promise<McpTestResult | null> {
  try {
    const r = await authFetch(`${BASE}/mcp/servers/${encodeURIComponent(id)}/test`, {
      method: "POST",
    });
    if (!r.ok) return null;
    return (await r.json()) as McpTestResult;
  } catch {
    return null;
  }
}

export async function deleteMcpServer(id: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/mcp/servers/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    return r.ok;
  } catch {
    return false;
  }
}

export async function readMcpAppResource(
  serverId: string,
  uri: string,
): Promise<unknown> {
  const response = await readMcpAppResourceResponse(serverId, uri, false);
  return response.result;
}

export async function readMcpAppView(
  serverId: string,
  uri: string,
): Promise<{ viewUrl: string }> {
  const response = await readMcpAppResourceResponse(serverId, uri, true);
  if (!response.view_path) throw new Error("MCP App host did not create a view document");
  const resolved = await resolveApiInput(`${BASE}${response.view_path}`);
  return { viewUrl: String(resolved) };
}

async function readMcpAppResourceResponse(
  serverId: string,
  uri: string,
  render: boolean,
): Promise<{ result: unknown; view_path?: string | null }> {
  return await parseJsonResponse<{ result: unknown; view_path?: string | null }>(
    await authFetch(`${BASE}/mcp/apps/resources/read`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ server_id: serverId, uri, render }),
    }),
    "MCP App resource read failed",
  );
}

export async function callMcpAppTool(
  serverId: string,
  name: string,
  argumentsValue: Record<string, unknown>,
  approval: ToolApprovalMode,
  approvalGranted = false,
): Promise<unknown> {
  const response = await parseJsonResponse<{ result: unknown }>(
    await authFetch(`${BASE}/mcp/apps/tools/call`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        server_id: serverId,
        name,
        arguments: argumentsValue,
        approval,
        approval_granted: approvalGranted,
      }),
    }),
    "MCP App tool call failed",
  );
  return response.result;
}

// ----- Skills -----

export interface SkillInfo {
  id: string;
  name: string;
  description: string;
  instructions: string;
  enabled: boolean;
  source_kind: string;
  source_url?: string | null;
  updated_at?: string;
}

export async function listSkills(): Promise<SkillInfo[]> {
  try {
    const r = await authFetch(`${BASE}/skills`);
    if (!r.ok) return [];
    const j = await r.json();
    return (j.skills ?? []) as SkillInfo[];
  } catch {
    return [];
  }
}

export async function createSkill(input: {
  skill_md?: string;
  skill_url?: string;
  name?: string;
  description?: string;
  instructions?: string;
  source_kind?: string;
  source_url?: string | null;
  enabled?: boolean;
}): Promise<SkillInfo | null> {
  try {
    const r = await authFetch(`${BASE}/skills`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ enabled: true, ...input }),
    });
    if (!r.ok)
      throw new Error(
        await responseErrorMessage(r, `skill create HTTP ${r.status}`),
      );
    return (await r.json()) as SkillInfo;
  } catch {
    return null;
  }
}

export async function updateSkill(skill: SkillInfo): Promise<SkillInfo | null> {
  try {
    const r = await authFetch(
      `${BASE}/skills/${encodeURIComponent(skill.id)}`,
      {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(skill),
      },
    );
    return r.ok ? ((await r.json()) as SkillInfo) : null;
  } catch {
    return null;
  }
}

export async function deleteSkill(id: string): Promise<boolean> {
  try {
    const r = await authFetch(`${BASE}/skills/${encodeURIComponent(id)}`, {
      method: "DELETE",
    });
    if (!r.ok) return false;
    const j = await r.json();
    return Boolean(j.deleted);
  } catch {
    return false;
  }
}

export async function selectSkills(
  query: string,
  limit = 3,
  ids?: string[],
): Promise<SkillInfo[]> {
  try {
    if (!query.trim()) return [];
    const r = await authFetch(`${BASE}/skills/select`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query, limit, ids }),
    });
    if (!r.ok) return [];
    const j = await r.json();
    return (j.skills ?? []) as SkillInfo[];
  } catch {
    return [];
  }
}
