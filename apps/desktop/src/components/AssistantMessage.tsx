import { lazy, memo, Suspense, useEffect, useRef, useState } from "react";
import {
  getControlRunEvents,
  getControlRunDetails,
  type ChatArtifact,
  type ChatStreamEventIcon,
  type ChatStreamPart,
  type NativeChartDescriptor,
  type RunEventPageV1,
  type RunEventV1,
  type RunInspectionV1,
  type ToolApprovalMode,
  type ToolUiDescriptor,
} from "../api";
import { markPerfRender } from "../lib/perf";
import {
  groupCompletedStreamActivity,
  liveWorkGroupSummary,
  type ChatStreamWorkGroup,
} from "../lib/streamParts";
import { formatDuration } from "../lib/usageMetrics";
import { Calendar, Code, Copy, Eye, FileText, Lightbulb, Pencil, X } from "./icons";

const Markdown = lazy(() =>
  import("./Markdown").then((mod) => ({ default: mod.Markdown })),
);
const MemoizedMarkdown = lazy(() =>
  import("./Markdown").then((mod) => ({ default: mod.MemoizedMarkdown })),
);
const McpAppView = lazy(() =>
  import("./McpAppView").then((mod) => ({ default: mod.McpAppView })),
);
const NativeChartView = lazy(() =>
  import("./NativeChartView").then((mod) => ({ default: mod.NativeChartView })),
);
type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;

function isNativeChart(descriptor: ToolUiDescriptor): descriptor is NativeChartDescriptor {
  return descriptor.kind === "native_chart";
}

type AssistantMessageProps = {
  content: string;
  streamParts?: ChatStreamPart[];
  previewArtifacts?: ChatArtifact[];
  onOpenPreview?: (artifact: ChatArtifact) => void;
  streaming?: boolean;
  previewArtifactsStreaming?: boolean;
  workDurationMs?: number;
  toolApproval?: ToolApprovalMode;
  workspaceFolder?: string;
  runDetailsRunId?: string;
};

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function flexiblePathPattern(value: string): string {
  let pattern = "";
  let separator = false;
  for (const char of value) {
    if (char === "/" || char === "\\") {
      if (!separator) pattern += "[\\\\/]+";
      separator = true;
    } else {
      pattern += escapeRegExp(char);
      separator = false;
    }
  }
  return pattern;
}

function shortenWorkspacePaths(command: string, workspaceFolder?: string): string {
  const folder = workspaceFolder?.trim().replace(/[\\/]+$/, "");
  if (!folder) return command;
  const path = flexiblePathPattern(folder);
  const flags = /^[A-Za-z]:[\\/]|^\\\\/.test(folder) ? "gi" : "g";
  const boundary = "(?=$|[\\\\/\\s\\\"'`,;)])";
  const fileUrl = /^[A-Za-z]:[\\/]/.test(folder)
    ? `file:\\/{2,3}${path}${boundary}`
    : `file:\\/{2}${path}${boundary}`;
  const shortened = command.replace(new RegExp(fileUrl, flags), ".");
  return shortened.replace(
    new RegExp(`(^|[\\s\\\"'=(:])${path}${boundary}`, flags),
    "$1.",
  );
}

export function formatCommandDisplay(command: string, workspaceFolder?: string): string {
  const powershell = /^(?:"[^"]*[\\/](?:powershell|pwsh)(?:\.exe)?"|(?:\S*[\\/])?(?:powershell|pwsh)(?:\.exe)?)\s+(?:(?:-(?:NoLogo|NoProfile|NonInteractive|Sta|Mta)\b|-ExecutionPolicy\s+\S+)\s+)*-(?:Command|CommandWithArgs)\b\s*/i;
  const cmd = /^(?:"[^"]*[\\/]cmd(?:\.exe)?"|(?:\S*[\\/])?cmd(?:\.exe)?)\s+(?:(?:\/[dqs])\s+)*\/c\s+/i;
  const posixShell = /^(?:"[^"]*[\\/](?:ba|z|k)?sh"|(?:\S*[\\/])?(?:ba|z|k)?sh)\s+-[a-z]*c[a-z]*\s+/i;
  return shortenWorkspacePaths(
    command.replace(powershell, "").replace(cmd, "").replace(posixShell, ""),
    workspaceFolder,
  );
}

/** Split out a `<think>...</think>` reasoning span (reasoning models like
 *  DeepSeek-R1 / QwQ emit it inline). Handles the still-streaming case where
 *  the closing tag hasn't arrived yet. */
function splitThink(content: string): {
  think: string | null;
  answer: string;
  thinking: boolean;
} {
  const open = content.indexOf("<think>");
  if (open === -1) return { think: null, answer: content, thinking: false };
  const close = content.indexOf("</think>", open);
  if (close === -1)
    return { think: content.slice(open + 7), answer: "", thinking: true };
  return {
    think: content.slice(open + 7, close),
    answer: content.slice(close + 8),
    thinking: false,
  };
}

function fallbackParts(content: string): {
  parts: ChatStreamPart[];
  thinking: boolean;
} {
  const { think, answer, thinking } = splitThink(content);
  const parts: ChatStreamPart[] = [];
  if (think != null && think.trim())
    parts.push({ kind: "thinking", content: think });
  if (answer.trim()) parts.push({ kind: "text", content: answer });
  return { parts, thinking };
}

function StreamIcon({
  icon,
  status,
}: {
  icon?: ChatStreamEventIcon;
  status?: string;
}) {
  if (status === "error" || icon === "error") return <X size={13} />;
  switch (icon) {
    case "thinking":
      return <Lightbulb size={13} />;
    case "file":
      return <FileText size={13} />;
    case "command":
      return <Code size={13} />;
    case "memory":
      return <Lightbulb size={13} />;
    case "schedule":
      return <Calendar size={13} />;
    case "screen":
      return <Eye size={13} />;
    default:
      return <Pencil size={13} />;
  }
}

function StreamEvent({
  part,
  toolApproval,
  workspaceFolder,
}: {
  part: Extract<ChatStreamPart, { kind: "event" }>;
  toolApproval: ToolApprovalMode;
  workspaceFolder?: string;
}) {
  const status = part.status ?? "done";
  const detail = hidesApprovalDetail(part)
    ? undefined
    : part.detail;
  return (
    <>
      <div
        className={`stream-event stream-event-${part.eventType} stream-event-${status}`}
        data-testid="assistant-stream-event"
        role={status === "error" ? "alert" : "status"}
      >
        <span className="stream-event-icon" aria-hidden="true">
          <StreamIcon icon={part.icon} status={status} />
        </span>
        <span
          className={
            "stream-event-label" + (status === "running" ? " shiny-text" : "")
          }
        >
          {part.label}
        </span>
        {detail && (
          <StreamEventDetail
            detail={isCommandEvent(part) ? formatCommandDisplay(detail, workspaceFolder) : detail}
            running={status === "running"}
            copyText={isCommandEvent(part) ? detail : undefined}
          />
        )}
      </div>
      {part.mcpApp ? isNativeChart(part.mcpApp) ? (
        <Suspense fallback={<div className="native-chart-state">Loading chart...</div>}>
          <NativeChartView argumentsText={part.toolArguments} result={part.mcpAppResult} status={part.status} />
        </Suspense>
      ) : (
        <Suspense fallback={<div className="mcp-app-state">Loading app...</div>}>
          <McpAppView descriptor={part.mcpApp} argumentsText={part.toolArguments} result={part.mcpAppResult} status={part.status} approval={toolApproval} />
        </Suspense>
      ) : null}
    </>
  );
}

function hidesApprovalDetail(part: ChatStreamEventPart): boolean {
  return Boolean(
    part.approvalStatus && ["approved", "denied", "canceled"].includes(part.approvalStatus),
  );
}

function lastFailedEvent(parts: ChatStreamPart[]): ChatStreamEventPart | undefined {
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    const part = parts[index];
    if (part.kind === "event" && part.status === "error") return part;
  }
  return undefined;
}

function failureSummary(part: ChatStreamEventPart, workspaceFolder?: string): string {
  const detail = part.detail && isCommandEvent(part)
    ? formatCommandDisplay(part.detail, workspaceFolder)
    : part.detail;
  return detail ? `${part.label} · ${detail}` : part.label;
}

function plural(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

function isCommandEvent(part: ChatStreamPart): boolean {
  return (
    part.kind === "event" &&
    part.eventType === "tool" &&
    (part.icon === "command" ||
      part.name === "shell" ||
      part.name === "run_command" ||
      /command/i.test(part.label))
  );
}

function workGroupDetail(group: ChatStreamWorkGroup): string {
  const updates = group.parts.filter((part) => part.kind === "text").length;
  const reasoning = group.parts.filter(
    (part) => part.kind === "thinking",
  ).length;
  const commands = group.parts.filter(isCommandEvent).length;
  const tools = group.parts.filter(
    (part) =>
      part.kind === "event" &&
      part.eventType === "tool" &&
      !isCommandEvent(part),
  ).length;
  return (
    [
      updates ? plural(updates, "update") : null,
      commands ? plural(commands, "command") : null,
      tools ? plural(tools, "tool") : null,
      reasoning ? `${plural(reasoning, "reasoning note")}` : null,
    ]
      .filter(Boolean)
      .join(", ") || plural(group.parts.length, "step")
  );
}

function completedWorkDetail(
  part: ChatStreamEventPart,
  workspaceFolder?: string,
): string {
  if (!part.detail || hidesApprovalDetail(part)) return part.label;
  const detail = isCommandEvent(part)
    ? formatCommandDisplay(part.detail, workspaceFolder)
    : part.detail;
  return `${part.label} · ${detail}`;
}

function StreamWorkGroup({
  group,
  durationMs,
  streaming = false,
  workspaceFolder,
  previewArtifacts,
  onOpenPreview,
  runDetailsRunId,
}: {
  group: ChatStreamWorkGroup;
  durationMs?: number;
  streaming?: boolean;
  workspaceFolder?: string;
  previewArtifacts?: ChatArtifact[];
  onOpenPreview?: (artifact: ChatArtifact) => void;
  runDetailsRunId?: string;
}) {
  const liveSummary = streaming ? liveWorkGroupSummary(group) : null;
  const latestEvent = [...group.parts].reverse().find(
    (part): part is ChatStreamEventPart => part.kind === "event",
  );
  const failure = lastFailedEvent(group.parts);
  const autoOpen = liveSummary?.status === "error";
  const [open, setOpen] = useState(autoOpen);
  const autoOpenedRef = useRef(autoOpen);
  useEffect(() => {
    if (autoOpen) {
      setOpen((current) => {
        if (!current) autoOpenedRef.current = true;
        return true;
      });
    } else if (autoOpenedRef.current) {
      autoOpenedRef.current = false;
      setOpen(false);
    }
  }, [autoOpen]);
  const status =
    liveSummary?.status ??
    (failure ? "error" : "done");
  return (
    <details
      className="stream-tool-group stream-work-group"
      data-testid="assistant-stream-work-group"
      open={open}
      onToggle={(event) => {
        const next = event.currentTarget.open;
        if (next === open) return;
        autoOpenedRef.current = false;
        setOpen(next);
      }}
    >
      <summary
        className={`stream-event stream-event-${liveSummary?.eventType ?? "tool"} stream-event-${status}`}
      >
        <span className="stream-event-icon" aria-hidden="true">
          <StreamIcon
            icon={liveSummary?.icon ?? failure?.icon ?? "thinking"}
            status={status}
          />
        </span>
        <span
          className={
            "stream-event-label" +
            (liveSummary?.status === "running" ? " shiny-text" : "")
          }
        >
          {liveSummary?.label ??
            (failure
              ? "Work stopped"
              : durationMs != null && durationMs > 0
              ? `Worked for ${formatDuration(durationMs)}`
              : `Worked through ${plural(group.parts.length, "step")}`)}
        </span>
        {liveSummary?.detail ? (
          <StreamEventDetail
            detail={latestEvent && isCommandEvent(latestEvent)
              ? formatCommandDisplay(liveSummary.detail, workspaceFolder)
              : liveSummary.detail}
            running={liveSummary.status === "running"}
            copyText={latestEvent && isCommandEvent(latestEvent) ? liveSummary.detail : undefined}
          />
        ) : failure ? (
          <StreamEventDetail
            detail={failureSummary(failure, workspaceFolder)}
            running={false}
            role="alert"
            copyText={failure.detail && isCommandEvent(failure) ? failure.detail : undefined}
          />
        ) : latestEvent ? (
          <span className="stream-work-summary-details">
            <StreamEventDetail
              detail={completedWorkDetail(latestEvent, workspaceFolder)}
              running={false}
              copyText={isCommandEvent(latestEvent) && !hidesApprovalDetail(latestEvent) ? latestEvent.detail : undefined}
            />
            <span className="stream-work-summary-meta">{workGroupDetail(group)}</span>
          </span>
        ) : (
          <code className="stream-event-detail">{workGroupDetail(group)}</code>
        )}
      </summary>
      <div className="stream-tool-group-body">
        {group.parts.map((part, index) => {
          if (part.kind === "thinking")
            return (
              <ThinkingBlock
                key={`${part.kind}-${index}`}
                content={part.content}
                streaming={streaming && index === group.parts.length - 1}
                nested
              />
            );
          if (part.kind === "event")
            return <StreamEvent key={`${part.kind}-${index}`} part={part} toolApproval="guarded" workspaceFolder={workspaceFolder} />;
          return (
            <AnswerText
              key={`${part.kind}-${index}`}
              content={part.content}
              previewArtifacts={previewArtifacts}
              onOpenPreview={onOpenPreview}
              streaming={false}
            />
          );
        })}
        {runDetailsRunId ? <RunDetails runId={runDetailsRunId} /> : null}
      </div>
    </details>
  );
}

function runEventGroup(event: RunEventV1): "model" | "tools" | "inbox" | "failure" {
  const type = event.type.toLowerCase();
  if (type.includes("tool") || type.includes("approval")) return "tools";
  if (type.includes("inbox") || type.includes("steer") || type.includes("inject")) return "inbox";
  if (type.includes("fail") || type.includes("error") || type.includes("interrupt")) return "failure";
  return "model";
}

function RunEventDetails({ event }: { event: RunEventV1 }) {
  return (
    <div className="stream-run-detail-event">
      <span>{event.type.replace(/_/g, " ")}</span>
      {event.step_id ? <small>{event.step_id}</small> : null}
      <details>
        <summary>Raw JSON</summary>
        <pre>{JSON.stringify(event.data, null, 2)}</pre>
      </details>
    </div>
  );
}

function RunDetailSection({
  title,
  events,
  emptyLabel,
}: {
  title: string;
  events: RunEventV1[];
  emptyLabel?: string;
}) {
  if (!events.length && !emptyLabel) return null;
  return (
    <details className="stream-run-detail-section">
      <summary>
        <span>{title}</span>
        <small>{events.length || emptyLabel}</small>
      </summary>
      <div>
        {events.map((event) => <RunEventDetails key={event.id} event={event} />)}
      </div>
    </details>
  );
}

function RunDetails({ runId }: { runId: string }) {
  const [inspection, setInspection] = useState<RunInspectionV1 | null>(null);
  const [page, setPage] = useState<RunEventPageV1 | null>(null);
  const [loading, setLoading] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [error, setError] = useState("");

  const load = async () => {
    if (loading || attempted) return;
    setLoading(true);
    setAttempted(true);
    setError("");
    try {
      const details = await getControlRunDetails(runId);
      setInspection(details.inspection);
      setPage(details.events);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };

  const loadMore = async () => {
    if (!page?.has_more || page.next_seq == null || loading) return;
    setLoading(true);
    try {
      const next = await getControlRunEvents(runId, page.next_seq);
      setPage({ ...next, events: [...page.events, ...next.events] });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  };

  if (attempted && !loading && !inspection?.composition && !page?.events.length && !error) {
    return null;
  }
  const events = page?.events ?? [];
  const modelEvents = events.filter((event) => runEventGroup(event) === "model");
  const toolEvents = events.filter((event) => runEventGroup(event) === "tools");
  const inboxEvents = events.filter((event) => runEventGroup(event) === "inbox");
  const failureEvents = events.filter((event) => runEventGroup(event) === "failure");
  return (
    <div className="stream-run-details">
      {!inspection ? (
        <button
          type="button"
          className="stream-run-details-action"
          onClick={() => void load()}
          disabled={loading}
        >
                {loading ? "Loading run details..." : "Run details"}
        </button>
      ) : (
        <div className="stream-run-details-body" data-testid="assistant-run-details">
          <details className="stream-run-detail-section">
            <summary><span>Composition</span><small>{inspection.run.capabilities.visibility}</small></summary>
            <div className="stream-run-composition">
              <span>{inspection.run.adapter} · {inspection.run.config.model}</span>
              {inspection.composition ? (
                <>
                  <span>{inspection.composition.environment_policy}</span>
                  <details>
                    <summary>Prompts, schemas, and raw JSON</summary>
                    <pre>{JSON.stringify(inspection.composition, null, 2)}</pre>
                  </details>
                </>
              ) : null}
            </div>
          </details>
          <RunDetailSection title="Model steps" events={modelEvents} />
          <RunDetailSection title="Tools" events={toolEvents} />
          <RunDetailSection title="Inbox" events={inboxEvents} />
          <RunDetailSection
            title="Failure information"
            events={failureEvents}
            emptyLabel={inspection.run.error ? "recorded" : undefined}
          />
          {inspection.run.error ? (
            <details className="stream-run-detail-section">
              <summary>Run error</summary>
              <pre>{JSON.stringify(inspection.run.error, null, 2)}</pre>
            </details>
          ) : null}
          {page?.has_more ? (
            <button type="button" className="stream-run-details-action" onClick={() => void loadMore()} disabled={loading}>
                  {loading ? "Loading..." : "Load more"}
            </button>
          ) : null}
        </div>
      )}
      {error ? <small className="stream-run-details-error">{error}</small> : null}
    </div>
  );
}

function StreamEventDetail({
  detail,
  running,
  copyText,
  role,
}: {
  detail: string;
  running: boolean;
  copyText?: string;
  role?: "alert";
}) {
  const tokens = detail.split(/(\s+)/);
  return (
    <>
      <code className="stream-event-detail" title={copyText ?? detail} role={role}>
        {tokens.map((token, index) => {
          const stat = token.match(/^([+-])(\d+)$/);
          if (!stat) return <span key={`${index}-text`}>{token}</span>;
          const kind = stat[1] === "+" ? "added" : "removed";
          return (
            <span
              key={`${index}-${token}`}
              className={`stream-diff-stat ${kind}${running ? " live" : ""}`}
            >
              {token}
            </span>
          );
        })}
      </code>
      {copyText && (
        <button
          type="button"
          className="stream-command-copy"
          title="Copy full command"
          aria-label="Copy full command"
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            void navigator.clipboard?.writeText(copyText);
          }}
        >
          <Copy size={11} />
        </button>
      )}
    </>
  );
}

function WaitingBlock({
  label = "reasoning...",
  icon = "thinking",
}: {
  label?: string;
  icon?: ChatStreamEventIcon;
}) {
  return (
    <div
      className="stream-event stream-event-thinking stream-event-running stream-waiting"
      data-testid="assistant-activity-cue"
      role="status"
      aria-live="polite"
    >
      <span className="stream-event-icon" aria-hidden="true">
        <StreamIcon icon={icon} />
      </span>
      <span className="stream-event-label shiny-text">{label}</span>
    </div>
  );
}

function activityCueForParts(
  parts: ChatStreamPart[],
): { label: string; icon: ChatStreamEventIcon } | null {
  const last = parts[parts.length - 1];
  if (!last) return { label: "reasoning...", icon: "thinking" };
  if (last.kind === "thinking") return null;
  if (last.kind === "text") return { label: "generating...", icon: "tool" };
  if (last.status === "running" || last.status === "error") return null;
  return { label: "reasoning...", icon: "thinking" };
}

function ThinkingBlock({
  content,
  streaming,
  nested = false,
}: {
  content: string;
  streaming: boolean;
  nested?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const lastContent = useRef(content);
  useEffect(() => {
    if (streaming) setExpanded(true);
  }, [streaming]);
  useEffect(() => {
    if (lastContent.current === content) return;
    lastContent.current = content;
    if (!streaming) setExpanded(false);
  }, [content, streaming]);
  if (!content.trim()) return streaming ? <WaitingBlock /> : null;
  const sameContent = lastContent.current === content;
  const charCount = content.trim().length;

  return (
    <details
      className={`stream-reasoning${streaming ? " stream-reasoning-live" : ""}`}
      data-testid="assistant-reasoning"
      open={streaming || (sameContent && expanded)}
      onToggle={(e) => {
        if (streaming) return;
        setExpanded((e.target as HTMLDetailsElement).open);
      }}
    >
      <summary
        className={`stream-event stream-event-thinking${streaming && !nested ? " stream-event-running" : ""}`}
      >
        <span className="stream-event-icon" aria-hidden="true">
          <StreamIcon icon="thinking" />
        </span>
        <span
          className={
            "stream-event-label" +
            (streaming && !nested ? " shiny-text" : "")
          }
        >
          {streaming && !nested ? "reasoning..." : "Reasoning"}
        </span>
        <span className="stream-reasoning-meta">
          {streaming ? "live" : `${charCount.toLocaleString()} chars`}
        </span>
      </summary>
      <div className="stream-reasoning-body">
        {streaming ? (
          <div className="md md-streaming-text" dir="auto">
            {content}
          </div>
        ) : (
          <Suspense fallback={<span className="typing">...</span>}>
            <Markdown content={content} highlight />
          </Suspense>
        )}
      </div>
    </details>
  );
}

function AnswerText({
  content,
  previewArtifacts,
  onOpenPreview,
  streaming,
  previewArtifactsStreaming,
}: {
  content: string;
  previewArtifacts?: ChatArtifact[];
  onOpenPreview?: (artifact: ChatArtifact) => void;
  streaming: boolean;
  previewArtifactsStreaming?: boolean;
}) {
  if (!content.trim()) return null;
  if (streaming) {
    return <StreamingMarkdownText content={content} />;
  }
  return (
    <Suspense fallback={<span className="typing">...</span>}>
      <Markdown
        content={content}
        previewArtifacts={previewArtifacts}
        onOpenPreview={onOpenPreview}
        highlight={!streaming}
        previewArtifactsStreaming={previewArtifactsStreaming}
        renderMermaid
        sourceLinks
      />
    </Suspense>
  );
}

function StreamingMarkdownText({ content }: { content: string }) {
  const fallback = (
    <div className="md md-streaming-text" dir="auto">
      {content}
    </div>
  );
  return (
    <Suspense fallback={fallback}>
      <MemoizedMarkdown
        content={content}
        highlight={false}
        collapseArtifacts={false}
        renderMermaid
      />
    </Suspense>
  );
}

function AssistantMessageView({
  content,
  streamParts,
  previewArtifacts,
  onOpenPreview,
  streaming = false,
  previewArtifactsStreaming = false,
  workDurationMs,
  toolApproval = "guarded",
  workspaceFolder,
  runDetailsRunId,
}: AssistantMessageProps) {
  markPerfRender("AssistantMessage");
  if (streaming) markPerfRender("StreamingAssistantMessage");
  const fallback = fallbackParts(content);
  const parts = streamParts?.length ? streamParts : fallback.parts;
  const displayParts = groupCompletedStreamActivity(parts, streaming);
  const workGroupCount = displayParts.filter(
    (part) => part.kind === "workGroup",
  ).length;
  const fallbackThinking = !streamParts?.length && fallback.thinking;
  const lastDisplayPart = displayParts[displayParts.length - 1];
  const activityCue =
    streaming && lastDisplayPart?.kind !== "workGroup"
      ? activityCueForParts(parts)
      : null;

  return (
    <div className="assistant-stream">
      {displayParts.map((part, index) => {
        const isLatest = index === displayParts.length - 1;
        if (part.kind === "workGroup")
          return (
            <StreamWorkGroup
              key={`${part.kind}-${index}`}
              group={part}
              durationMs={workGroupCount === 1 ? workDurationMs : undefined}
              streaming={streaming && isLatest}
              workspaceFolder={workspaceFolder}
              previewArtifacts={previewArtifacts}
              onOpenPreview={onOpenPreview}
              runDetailsRunId={runDetailsRunId}
            />
          );
        if (part.kind === "thinking") {
          const thinking = fallbackThinking || (streaming && isLatest);
          return (
            <ThinkingBlock
              key={`${part.kind}-${index}`}
              content={part.content}
              streaming={streaming && thinking}
            />
          );
        }
        if (part.kind === "event")
          return <StreamEvent key={`${part.kind}-${index}`} part={part} toolApproval={toolApproval} workspaceFolder={workspaceFolder} />;
        return (
          <AnswerText
            key={`${part.kind}-${index}`}
            content={part.content}
            previewArtifacts={previewArtifacts}
            onOpenPreview={onOpenPreview}
            streaming={streaming && isLatest}
            previewArtifactsStreaming={
              previewArtifactsStreaming && streaming && isLatest
            }
          />
        );
      })}
      {activityCue ? (
        <WaitingBlock label={activityCue.label} icon={activityCue.icon} />
      ) : null}
    </div>
  );
}

export const AssistantMessage = memo(AssistantMessageView);
