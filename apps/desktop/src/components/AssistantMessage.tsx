import { lazy, memo, Suspense, useEffect, useRef, useState } from "react";
import { type ChatArtifact, type ChatStreamEventIcon, type ChatStreamPart, type NativeChartDescriptor, type ToolApprovalMode, type ToolUiDescriptor } from "../api";
import { markPerfRender } from "../lib/perf";
import {
  groupCompletedStreamActivity,
  liveWorkGroupSummary,
  type ChatStreamToolGroup,
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
const STREAMING_MARKDOWN_CHAR_LIMIT = 12000;
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
  const detail = part.approvalStatus && ["approved", "denied", "canceled"].includes(part.approvalStatus)
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

function compactToolNames(parts: ChatStreamEventPart[]): string {
  const names = parts.map(
    (part) =>
      part.name?.trim() || part.label.replace(/^Used\s+/i, "").trim() || "tool",
  );
  const joined = names.join(", ");
  return joined.length > 90 ? `${joined.slice(0, 89)}...` : joined;
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

function StreamToolGroup({ group, workspaceFolder }: { group: ChatStreamToolGroup; workspaceFolder?: string }) {
  const failure = lastFailedEvent(group.parts);
  const status = failure ? "error" : "done";
  return (
    <details
      className="stream-tool-group"
      data-testid="assistant-stream-tool-group"
    >
      <summary className={`stream-event stream-event-tool stream-event-${status}`}>
        <span className="stream-event-icon" aria-hidden="true">
          <StreamIcon icon="tool" status={status} />
        </span>
        <span className="stream-event-label">
          {failure ? "Work stopped" : `Used ${group.parts.length} tools`}
        </span>
        <StreamEventDetail
          detail={failure ? failureSummary(failure, workspaceFolder) : compactToolNames(group.parts)}
          running={false}
          role={failure ? "alert" : undefined}
          copyText={failure?.detail && isCommandEvent(failure) ? failure.detail : undefined}
        />
      </summary>
      <div className="stream-tool-group-body">
        {group.parts.map((part, index) => (
          <StreamEvent
            key={`${part.name ?? part.label}-${index}`}
            part={part}
            toolApproval="guarded"
            workspaceFolder={workspaceFolder}
          />
        ))}
      </div>
    </details>
  );
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
      commands ? plural(commands, "command") : null,
      tools ? plural(tools, "tool") : null,
      reasoning ? `${plural(reasoning, "reasoning note")}` : null,
    ]
      .filter(Boolean)
      .join(", ") || plural(group.parts.length, "step")
  );
}

function StreamWorkGroup({
  group,
  durationMs,
  streaming = false,
  workspaceFolder,
}: {
  group: ChatStreamWorkGroup;
  durationMs?: number;
  streaming?: boolean;
  workspaceFolder?: string;
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
              : `Worked through ${group.parts.length} steps`)}
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
          return null;
        })}
      </div>
    </details>
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
  if (content.length > STREAMING_MARKDOWN_CHAR_LIMIT) return fallback;
  return (
    <Suspense fallback={fallback}>
      <MemoizedMarkdown
        content={content}
        highlight={false}
        collapseArtifacts={false}
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
        if (part.kind === "toolGroup")
          return <StreamToolGroup key={`${part.kind}-${index}`} group={part} workspaceFolder={workspaceFolder} />;
        if (part.kind === "workGroup")
          return (
            <StreamWorkGroup
              key={`${part.kind}-${index}`}
              group={part}
              durationMs={workGroupCount === 1 ? workDurationMs : undefined}
              streaming={streaming && isLatest}
              workspaceFolder={workspaceFolder}
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
