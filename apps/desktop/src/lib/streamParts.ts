import type { ChatStreamEventIcon, ChatStreamEventStatus, ChatStreamPart } from "../api";

export type ChatStreamWorkGroup = {
  kind: "workGroup";
  parts: ChatStreamPart[];
};

export type ChatStreamDisplayPart = ChatStreamPart | ChatStreamWorkGroup;

export type WorkGroupSummary = {
  eventType: "tool" | "thinking";
  label: string;
  detail?: string;
  icon?: ChatStreamEventIcon;
  status: ChatStreamEventStatus;
};

export function liveWorkGroupSummary(group: ChatStreamWorkGroup): WorkGroupSummary | null {
  for (let i = group.parts.length - 1; i >= 0; i -= 1) {
    const part = group.parts[i];
    if (part.kind === "event") {
      return {
        eventType: "tool",
        label: part.label,
        detail: part.detail,
        icon: part.icon,
        status: part.status ?? "done",
      };
    }
    if (part.kind === "thinking" && part.content.trim()) {
      return { eventType: "thinking", label: "reasoning...", icon: "thinking", status: "running" };
    }
  }
  return null;
}

function completedInternalPart(part: ChatStreamPart): ChatStreamPart | null {
  if (part.kind === "thinking") return part;
  if (
    part.kind === "event" &&
    part.approvalId != null &&
    (part.status ?? "done") === "done"
  ) return part;
  if (part.kind === "event" && part.eventType === "tool" && !part.mcpApp) {
    // A terminal assistant message is authoritative even when a provider omitted
    // the matching tool-result event. Do not leave stale starts flat or animated.
    return part.status === "running" ? { ...part, status: "done" } : part;
  }
  return null;
}

function isLiveInternalPart(part: ChatStreamPart): boolean {
  return part.kind === "thinking" ||
    (part.kind === "event" && part.approvalId != null && (part.status ?? "done") !== "error") ||
    (part.kind === "event" && part.eventType === "tool" && !part.mcpApp);
}

export function groupCompletedStreamActivity(parts: ChatStreamPart[], streaming: boolean): ChatStreamDisplayPart[] {
  if (!streaming) {
    let finalAnswerIndex = -1;
    for (let index = parts.length - 1; index >= 0; index -= 1) {
      const part = parts[index];
      if (part.kind === "text" && part.content.trim()) {
        finalAnswerIndex = index;
        break;
      }
    }

    const visible: ChatStreamDisplayPart[] = [];
    const work: ChatStreamPart[] = [];
    let workIndex = -1;
    parts.forEach((part, index) => {
      const completed = completedInternalPart(part);
      const collapsible =
        (part.kind === "text" && index !== finalAnswerIndex) ||
        completed != null;
      if (collapsible) {
        if (workIndex < 0) workIndex = visible.length;
        work.push(completed ?? part);
      } else {
        visible.push(part);
      }
    });
    if (work.length) visible.splice(workIndex, 0, { kind: "workGroup", parts: work });
    return visible;
  }

  const next: ChatStreamDisplayPart[] = [];
  let group: ChatStreamPart[] = [];

  const push = (part: ChatStreamDisplayPart) => {
    const last = next[next.length - 1];
    if (part.kind === "text" && last?.kind === "text") {
      next[next.length - 1] = {
        ...last,
        content: last.content + part.content,
      };
      return;
    }
    next.push(part);
  };

  const flush = () => {
    if (group.length === 1) push(group[0]);
    else if (group.length > 1) push({ kind: "workGroup", parts: group });
    group = [];
  };

  for (const part of parts) {
    if (isLiveInternalPart(part)) {
      group.push(part);
    } else {
      flush();
      push(part);
    }
  }
  flush();
  return next;
}
