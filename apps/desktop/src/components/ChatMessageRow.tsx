import {
  lazy,
  memo,
  Suspense,
  useState,
  type MouseEvent,
  type MutableRefObject,
} from "react";
import {
  openExternalUrl,
  wireMessageContent,
  type ArtifactFileStatus,
  type ArtifactOpenTarget,
  type ArtifactWritePreview,
  type ChatApprovalRequest,
  type ChatArtifact,
  type ChatAttachment,
  type ChatMessage,
  type MediaGenerationResult,
  type MemoryNotice,
  type PreviewAppStatus,
  type RunStep,
  type RunTrace,
  type SavedArtifactFile,
  type ToolApprovalMode,
  type WorkspaceCheckpoint,
  type WorkspaceGitActionResult,
} from "../api";
import { useSessions, type HotSwapAction } from "../sessions/store";
import {
  hasPreviewPackageJson,
  isPreviewableArtifact,
  previewRuntimeFiles,
} from "../lib/artifacts";
import {
  type ArtifactRevision,
  type ArtifactRevisionChoice,
} from "../lib/artifactRevisions";
import { hiddenArtifactIdsForMessage } from "../lib/artifactVisibility";
import { isCompactionCheckpoint } from "../lib/contextCompaction";
import { formatResponseMetrics } from "../lib/usageMetrics";
import { markPerfRender } from "../lib/perf";
import { shortcutMatchesEvent } from "../ui/shortcuts";
import { useUiPreferences } from "../ui/store";
import { AgentAvatar } from "./AgentAvatar";
import { AssistantMessage } from "./AssistantMessage";
import { ArtifactList } from "./ArtifactList";
import { GeneratedMedia } from "./GeneratedMedia";
import { RunTimeline } from "./RunTimeline";
import {
  TurnChangesCard,
  type TurnReviewState,
} from "./TurnChangesCard";
import { BatonMenu } from "./HotSwapDialogs";
import type { ContextMenuItem } from "./ContextMenu";
import {
  ArrowRight,
  Calendar,
  Check,
  Copy,
  Eye,
  GitBranch,
  Globe,
  Pencil,
  Refresh,
  Trash,
  UserRound,
  X,
} from "./icons";

const Markdown = lazy(() =>
  import("./Markdown").then((mod) => ({ default: mod.Markdown })),
);

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

export type MessageRowActions = {
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
  retryTurnReview: () => void;
  openGit: () => void;
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
  appSessionId: string;
  message: ChatMessage;
  index: number;
  isEditing: boolean;
  isLastAssistant: boolean;
  assistantStreaming: boolean;
  busy: boolean;
  activeMediaTargetPresent: boolean;
  folderIsEmpty: boolean;
  workspaceFolder: string;
  activeRun?: RunTrace | null;
  previewArtifacts?: ChatArtifact[];
  previewAppBusy: "start" | "stop" | "restart" | null;
  previewAppStatus: PreviewAppStatus | null;
  toolApproval: ToolApprovalMode;
  turnReview?: TurnReviewState | null;
  actionsRef: MutableRefObject<MessageRowActions | null>;
  entering?: boolean;
  onEntered?: (id: string) => void;
};

function MessageRowView({
  activeId,
  appSessionId,
  message: m,
  index: i,
  isEditing,
  isLastAssistant,
  assistantStreaming,
  busy,
  activeMediaTargetPresent,
  folderIsEmpty,
  workspaceFolder,
  activeRun,
  previewArtifacts,
  previewAppBusy,
  previewAppStatus,
  toolApproval,
  turnReview,
  actionsRef,
  entering = false,
  onEntered,
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
      <div className={"msg " + m.role + (entering ? " msg-enter" : "")}>
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
      className={`msg ${m.role}${modelAvatarSeed ? " has-model-avatar" : ""}${entering ? " msg-enter" : ""}`}
      data-testid={
        m.role === "assistant" ? "assistant-message" : "user-message"
      }
      onContextMenu={openMessageContextMenu}
      onAnimationEnd={(event) => {
        if (event.target !== event.currentTarget) return;
        if (entering && m.id) onEntered?.(m.id);
      }}
    >
      {showModelAvatar && modelAvatarSeed && (
        <AgentAvatar avatar={modelAvatarSeed} className="message-agent-avatar" />
      )}
      <div className="msg-content" dir="auto">
        {m.role === "assistant" ? (
          <>
            {m.mailboxReply && (
              <div className={`mailbox-provenance ${m.mailboxReply.status}`} data-testid="mailbox-reply">
                <span>Reply from {m.mailboxReply.targetTitle}</span>
                {m.mailboxReply.targetProject && <small>{m.mailboxReply.targetProject}</small>}
                {m.mailboxReply.status === "failed" && <small>Failed</small>}
              </div>
            )}
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
                workspaceFolder={workspaceFolder}
                runDetailsRunId={m.ledgerVersion === 1 ? m.runId : undefined}
                terminalOutcome={
                  m.streamTerminalOutcome ??
                    (m.run?.status === "stopped" ||
                    m.run?.status === "aborted" ||
                    m.run?.status === "error"
                      ? "interrupted"
                      : m.run?.status === "running"
                        ? "unknown"
                        : "completed")
                }
              />
            )}
            {renderMessageMedia(m.mediaResults)}
            <AutomationCards
              run={m.run}
              onOpenSchedules={() => actions?.onOpenSchedules()}
            />
            <ArtifactList
              artifacts={m.artifacts}
              currentSessionId={appSessionId}
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
            {turnReview && (
              <TurnChangesCard
                review={turnReview}
                onUndo={() => void actions?.undoTurnChanges(i)}
                onReview={() => {
                  if (turnReview.status === "ready")
                    actions?.reviewTurnChanges(
                      turnReview.checkpoint,
                      turnReview.result,
                    );
                }}
                onRetry={() => actions?.retryTurnReview()}
                onOpenGit={() => actions?.openGit()}
              />
            )}
            {metricsLabel && (
              <div className="response-metrics">{metricsLabel}</div>
            )}
            {isLastAssistant && !busy && (
              <BatonMenu
                retryDisabled={
                  !folderIsEmpty && !m.workspaceCheckpoint && !m.plan
                }
                onAction={(action) => actions?.startBaton(action, i)}
              />
            )}
          </>
        ) : (
          <>
            {m.mailboxOrigin && (
              <div className="mailbox-provenance incoming" data-testid="mailbox-origin">
                <span>From {m.mailboxOrigin.origin_title}</span>
                {m.mailboxOrigin.origin_project && <small>{m.mailboxOrigin.origin_project}</small>}
              </div>
            )}
            {m.content && (
              <Suspense fallback={<span>{m.content}</span>}>
                <Markdown
                  content={m.content}
                  highlight={false}
                  collapseArtifacts={false}
                  renderMermaid
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
        {m.workspaceCheckpoint && !busy && !turnReview && (
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

export const MessageRow = memo(
  MessageRowView,
  (prev, next) =>
    prev.activeId === next.activeId &&
    prev.appSessionId === next.appSessionId &&
    prev.message === next.message &&
    prev.index === next.index &&
    prev.isEditing === next.isEditing &&
    prev.isLastAssistant === next.isLastAssistant &&
    prev.assistantStreaming === next.assistantStreaming &&
    prev.busy === next.busy &&
    prev.activeMediaTargetPresent === next.activeMediaTargetPresent &&
    prev.folderIsEmpty === next.folderIsEmpty &&
    prev.workspaceFolder === next.workspaceFolder &&
    prev.activeRun === next.activeRun &&
    prev.previewArtifacts === next.previewArtifacts &&
    prev.previewAppBusy === next.previewAppBusy &&
    prev.previewAppStatus === next.previewAppStatus &&
    prev.toolApproval === next.toolApproval &&
    prev.turnReview === next.turnReview &&
    prev.actionsRef === next.actionsRef &&
    prev.entering === next.entering &&
    prev.onEntered === next.onEntered,
);


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
