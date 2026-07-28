import type { ChatStreamEventIcon, ChatStreamEventStatus, ChatStreamPart } from "../api";

type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;

export type ChatStreamToolGroup = {
  kind: "toolGroup";
  parts: ChatStreamEventPart[];
};

export type ChatStreamWorkGroup = {
  kind: "workGroup";
  parts: ChatStreamPart[];
};

export type ChatStreamDisplayPart = ChatStreamPart | ChatStreamToolGroup | ChatStreamWorkGroup;

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

function isCompletedToolEvent(part: ChatStreamPart): part is ChatStreamEventPart {
  return part.kind === "event" && part.eventType === "tool" && !part.mcpApp && (part.status ?? "done") !== "running";
}

function isCompletedInternalPart(part: ChatStreamPart): boolean {
  return part.kind === "thinking" ||
    (part.kind === "event" && part.approvalId != null && (part.status ?? "done") === "done") ||
    isCompletedToolEvent(part);
}

function isLiveInternalPart(part: ChatStreamPart): boolean {
  return part.kind === "thinking" ||
    (part.kind === "event" && part.approvalId != null && (part.status ?? "done") !== "error") ||
    (part.kind === "event" && part.eventType === "tool" && !part.mcpApp);
}

export function groupCompletedStreamActivity(parts: ChatStreamPart[], streaming: boolean): ChatStreamDisplayPart[] {
  const next: ChatStreamDisplayPart[] = [];
  let group: ChatStreamPart[] = [];
  const isInternalPart = streaming ? isLiveInternalPart : isCompletedInternalPart;

  const flush = () => {
    if (group.length === 1) next.push(group[0]);
    else if (group.length > 1) {
      const toolParts = group.filter(isCompletedToolEvent);
      next.push(streaming || toolParts.length !== group.length ? { kind: "workGroup", parts: group } : { kind: "toolGroup", parts: toolParts });
    }
    group = [];
  };

  for (const part of parts) {
    if (isInternalPart(part)) {
      group.push(part);
    } else {
      flush();
      next.push(part);
    }
  }
  flush();
  return next;
}
