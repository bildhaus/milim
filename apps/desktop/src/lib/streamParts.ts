import type { ChatStreamEventIcon, ChatStreamEventStatus, ChatStreamPart } from "../api";

export type ChatStreamWorkGroup = {
  kind: "workGroup";
  parts: ChatStreamPart[];
};

export type ChatStreamDisplayPart = ChatStreamPart | ChatStreamWorkGroup;
export type StreamTerminalOutcome = "completed" | "interrupted" | "unknown";

export type WorkGroupSummary = {
  eventType: "tool" | "thinking";
  label: string;
  detail?: string;
  icon?: ChatStreamEventIcon;
  status: ChatStreamEventStatus;
};

function phaseStartIndex(parts: readonly ChatStreamPart[]): number {
  for (let index = parts.length - 1; index >= 0; index -= 1) {
    if (parts[index].kind === "event") return index + 1;
  }
  return 0;
}

function findPhasePartIndex(
  parts: readonly ChatStreamPart[],
  kind: "text" | "thinking",
  phaseStart: number,
): number {
  for (let index = parts.length - 1; index >= phaseStart; index -= 1) {
    if (parts[index].kind === kind) return index;
  }
  return -1;
}

/** Append text or thinking onto the current tool-bounded phase, not just the tail. */
export function appendPhaseStreamPart(
  parts: ChatStreamPart[] | undefined,
  kind: "text" | "thinking",
  content: string,
): ChatStreamPart[] {
  const next = parts ? parts.slice() : [];
  if (!content) return next;
  const phaseStart = phaseStartIndex(next);
  const existingIndex = findPhasePartIndex(next, kind, phaseStart);
  if (existingIndex >= 0) {
    const current = next[existingIndex];
    if (current.kind === kind) {
      next[existingIndex] = { ...current, content: current.content + content };
    }
    return next;
  }
  next.push({ kind, content });
  return next;
}

/**
 * Drop the last `bytes` UTF-8 bytes of `text`. Rust reports output discarded
 * by a failed provider attempt in UTF-8 bytes; JavaScript strings are UTF-16.
 */
export function dropTrailingUtf8Bytes(text: string, bytes: number): string {
  let remaining = Math.max(0, Math.floor(bytes));
  let end = text.length;
  while (remaining > 0 && end > 0) {
    const code = text.charCodeAt(end - 1);
    const pair = end >= 2 && code >= 0xdc00 && code <= 0xdfff
      && (text.codePointAt(end - 2) ?? 0) >= 0x10000;
    if (pair) {
      end -= 2;
      remaining -= 4;
    } else {
      end -= 1;
      remaining -= code < 0x80 ? 1 : code < 0x800 ? 2 : 3;
    }
  }
  return text.slice(0, end);
}

/**
 * Remove the trailing text or reasoning a failed provider attempt streamed.
 * The attempt started after the last tool event, so trimming stops there.
 */
export function discardStreamContent(
  parts: readonly ChatStreamPart[],
  kind: "text" | "thinking",
  bytes: number,
): ChatStreamPart[] {
  const next = parts.slice();
  let remaining = Math.max(0, Math.floor(bytes));
  for (let index = next.length - 1; index >= 0 && remaining > 0; index -= 1) {
    const part = next[index];
    if (part.kind === "event" && part.eventType === "tool") break;
    if (part.kind !== kind) continue;
    const size = utf8Length(part.content);
    if (size <= remaining) {
      next.splice(index, 1);
      remaining -= size;
    } else {
      next[index] = { kind, content: dropTrailingUtf8Bytes(part.content, remaining) };
      remaining = 0;
    }
  }
  return next;
}

function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
}

/**
 * Text and reasoning are separate provider channels, not a shared chronology.
 * Coalesce each channel inside tool-event boundaries; tool events retain their order.
 */
export function coalesceStreamPhases(parts: readonly ChatStreamPart[]): ChatStreamPart[] {
  let changed = false;
  const next: ChatStreamPart[] = [];
  for (const part of parts) {
    if (part.kind !== "text" && part.kind !== "thinking") {
      next.push(part);
      continue;
    }
    if (!part.content) {
      changed = true;
      continue;
    }
    const phaseStart = phaseStartIndex(next);
    const existingIndex = findPhasePartIndex(next, part.kind, phaseStart);
    if (existingIndex >= 0) {
      const current = next[existingIndex];
      if (current.kind === part.kind) {
        next[existingIndex] = { ...current, content: current.content + part.content };
        changed = true;
      }
      continue;
    }
    next.push(part);
  }
  return changed ? next : parts as ChatStreamPart[];
}

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

/** Stream event name for a provider retry notice; it folds into the work group. */
export const PROVIDER_RETRY_EVENT = "provider_retry";

function isProviderRetryPart(part: ChatStreamPart): boolean {
  return part.kind === "event" && part.name === PROVIDER_RETRY_EVENT;
}

function completedInternalPart(
  part: ChatStreamPart,
  terminalOutcome: StreamTerminalOutcome,
): ChatStreamPart | null {
  if (part.kind === "thinking") return part;
  if (part.kind === "event" && isProviderRetryPart(part)) {
    return part.status === "running" ? { ...part, status: "done" } : part;
  }
  if (
    part.kind === "event" &&
    part.approvalId != null &&
    (part.status ?? "done") === "done"
  ) return part;
  if (part.kind === "event" && part.eventType === "tool" && !part.mcpApp) {
    if (part.status !== "running") return part;
    if (terminalOutcome === "completed") return { ...part, status: "done" };
    if (terminalOutcome === "interrupted") {
      return {
        ...part,
        label: part.name ? `${part.name} interrupted` : "Tool interrupted",
        icon: "error",
        status: "error",
      };
    }
    return part;
  }
  return null;
}

function isLiveInternalPart(part: ChatStreamPart): boolean {
  return part.kind === "thinking" ||
    isProviderRetryPart(part) ||
    (part.kind === "event" && part.approvalId != null && (part.status ?? "done") !== "error") ||
    (part.kind === "event" && part.eventType === "tool" && !part.mcpApp);
}

export function groupCompletedStreamActivity(
  parts: ChatStreamPart[],
  streaming: boolean,
  terminalOutcome: StreamTerminalOutcome = "completed",
): ChatStreamDisplayPart[] {
  parts = coalesceStreamPhases(parts);
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
      const completed = completedInternalPart(part, terminalOutcome);
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
    if (group.length > 0) push({ kind: "workGroup", parts: group });
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
