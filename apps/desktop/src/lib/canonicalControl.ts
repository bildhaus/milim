import {
  getControlTimeline,
  streamControlEvents,
  type ChatAttachment,
  type ChatMessage,
  type ChatStreamPart,
  type ControlBootstrapV1,
  type ControlQueuedTurnV1,
  type ControlTimelineItemV1,
} from "../api.js";
import { appendPhaseStreamPart } from "./streamParts.js";
import type { QueuedMessage } from "../sessions/store.js";

type CanonicalMessage = ChatMessage;

function projectedResponseMetrics(value: unknown): ChatMessage["metrics"] | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const metrics = value as Record<string, unknown>;
  if (
    typeof metrics.startedAt !== "number"
    || !Number.isFinite(metrics.startedAt)
    || typeof metrics.model !== "string"
  ) {
    return undefined;
  }
  const usageValue = metrics.usage;
  const usage = usageValue && typeof usageValue === "object" && !Array.isArray(usageValue)
    ? usageValue as Record<string, unknown>
    : null;
  const parsedUsage = usage
    && typeof usage.prompt_tokens === "number"
    && typeof usage.completion_tokens === "number"
    && typeof usage.total_tokens === "number"
    ? {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
      }
    : undefined;
  const costUsd = typeof metrics.costUsd === "number"
    && Number.isFinite(metrics.costUsd)
    && metrics.costUsd >= 0
    ? metrics.costUsd
    : undefined;
  const costSource = costUsd != null
    && (metrics.costSource === "provider" || metrics.costSource === "estimate")
    ? metrics.costSource
    : undefined;
  return {
    startedAt: metrics.startedAt,
    endedAt: typeof metrics.endedAt === "number" ? metrics.endedAt : undefined,
    durationMs: typeof metrics.durationMs === "number" ? metrics.durationMs : undefined,
    model: metrics.model,
    provider: typeof metrics.provider === "string" ? metrics.provider : undefined,
    usage: parsedUsage,
    costUsd,
    costSource,
  };
}

export function controlAttachments(attachments?: ChatAttachment[]) {
  return (attachments ?? []).map(({ dataUrl, sourcePath: _sourcePath, ...attachment }) => ({
    ...attachment,
    data_url: dataUrl,
    truncated: Boolean(attachment.truncated),
  }));
}

export function hostBusySessionIdsFromBootstrap(
  bootstrap: Pick<ControlBootstrapV1, "threads" | "active_runs" | "queued_turns">,
): string[] {
  const ids = new Set<string>();
  for (const thread of bootstrap.threads) {
    if (thread.busy) ids.add(thread.id);
  }
  for (const run of bootstrap.active_runs) {
    if (run.thread_id) ids.add(run.thread_id);
  }
  return [...ids].sort();
}

export function shouldQueueCanonicalFollowup(
  threadId: string,
  hostBusySessionIds: readonly string[],
  canonicalRunId?: string,
): boolean {
  return Boolean(canonicalRunId) || hostBusySessionIds.includes(threadId);
}

export function controlQueuedMessage(turn: ControlQueuedTurnV1): QueuedMessage {
  return {
    id: turn.id,
    content: turn.display_text,
    createdAt: turn.accepted_at_ms,
    attachments: turn.attachments.map(({ data_url, ...attachment }) => ({
      ...attachment,
      dataUrl: data_url,
    })),
    mailboxOrigin: turn.mailbox_origin,
  };
}

function streamEventCallId(item: ControlTimelineItemV1): string | undefined {
  const data = item.data;
  return typeof data.call_id === "string"
    ? data.call_id
    : typeof data.callId === "string"
      ? data.callId
      : typeof data.id === "string"
        ? data.id
        : undefined;
}

function streamEventName(item: ControlTimelineItemV1): string {
  const data = item.data;
  return typeof data.name === "string"
    ? data.name
    : typeof data.tool_name === "string"
      ? data.tool_name
      : item.type;
}

function eventPart(item: ControlTimelineItemV1): ChatStreamPart | null {
  const data = item.data;
  const name = streamEventName(item);
  const callId = streamEventCallId(item);
  if (item.type === "tool_call" || item.type === "tool_start") {
    return {
      kind: "event",
      eventType: "tool",
      label: name,
      name,
      status: "running",
      callId,
    };
  }
  if (item.type === "tool_result" || item.type === "tool_end") {
    return {
      kind: "event",
      eventType: "tool",
      label: name,
      name,
      status: data.error ? "error" : "done",
      callId,
    };
  }
  if (item.type === "approval_requested" || item.type === "tool_approval_required") {
    const approvalId =
      typeof data.approval_id === "string" ? data.approval_id : undefined;
    return {
      kind: "event",
      eventType: "status",
      label: `Approval required: ${name}`,
      status: "running",
      approvalId,
      approvalStatus: "pending",
    };
  }
  if (item.type === "approval_resolved" || item.type === "tool_approval_resolved") {
    const approvalId =
      typeof data.approval_id === "string" ? data.approval_id : undefined;
    const approved = data.decision === "approve" || data.approved === true;
    return {
      kind: "event",
      eventType: "status",
      label: approved ? "Tool call approved" : "Tool call denied",
      status: "done",
      approvalId,
      approvalStatus: approved ? "approved" : "denied",
    };
  }
  if (item.type === "warning" || item.type === "error") {
    return {
      kind: "event",
      eventType: item.type,
      label:
        typeof data.message === "string" ? data.message : `${item.type} from agent runtime`,
      status: item.type === "error" ? "error" : "done",
    };
  }
  return null;
}

function appendStreamContent(
  parts: ChatStreamPart[],
  kind: "text" | "thinking",
  content: string,
) {
  if (!content) return;
  const next = appendPhaseStreamPart(parts, kind, content);
  parts.length = 0;
  parts.push(...next);
}

function appendStreamEvent(
  parts: ChatStreamPart[],
  part: ChatStreamPart,
): void {
  if (part.kind !== "event" || part.eventType !== "tool" || part.status === "running") {
    parts.push(part);
    return;
  }
  const runningIndex = (() => {
    if (part.callId) {
      for (let index = parts.length - 1; index >= 0; index -= 1) {
        const current = parts[index];
        if (
          current.kind === "event" &&
          current.eventType === "tool" &&
          current.status === "running" &&
          current.callId === part.callId
        ) return index;
      }
    }
    if (!part.callId) {
      for (let index = parts.length - 1; index >= 0; index -= 1) {
        const current = parts[index];
        if (
          current.kind === "event" &&
          current.eventType === "tool" &&
          current.status === "running" &&
          current.name === part.name
        ) return index;
      }
    }
    return -1;
  })();
  if (runningIndex < 0) {
    parts.push(part);
    return;
  }
  const started = parts[runningIndex];
  parts[runningIndex] = started.kind === "event"
    ? {
        ...started,
        ...part,
        detail: part.detail ?? started.detail,
        mcpApp: part.mcpApp ?? started.mcpApp,
        toolArguments: part.toolArguments ?? started.toolArguments,
      }
    : part;
}

/** Fold the authoritative items for one run into the two user-visible turns. */
export function projectControlRunMessages(
  items: ControlTimelineItemV1[],
  runId: string,
): CanonicalMessage[] {
  const deletedIds = new Set(
    items.flatMap((item) =>
      item.type === "message_deleted" && typeof item.data.message_id === "string"
        ? [item.data.message_id]
        : [],
    ),
  );
  const deletedRoles = new Set(
    items.flatMap((item) =>
      item.run_id === runId
      && item.type === "message"
      && typeof item.data.id === "string"
      && deletedIds.has(item.data.id)
      && typeof item.data.role === "string"
        ? [item.data.role]
        : [],
    ),
  );
  let user: CanonicalMessage | null = null;
  let assistant: CanonicalMessage | null = null;
  let streamingText = "";
  let streamingReasoning = "";
  const streamParts: ChatStreamPart[] = [];
  for (const item of items) {
    if (item.run_id !== runId) continue;
    const data = item.data;
    if (item.type === "assistant_delta") {
      const text = typeof data.text === "string" ? data.text : "";
      const reasoning = typeof data.reasoning === "string" ? data.reasoning : "";
      streamingText += text;
      streamingReasoning += reasoning;
      appendStreamContent(streamParts, "text", text);
      appendStreamContent(streamParts, "thinking", reasoning);
      continue;
    }
    if (item.type === "message" && typeof data.id === "string") {
      if (deletedIds.has(data.id)) continue;
      const message: CanonicalMessage = {
        id: data.id,
        canonicalId: data.id,
        role: typeof data.role === "string" ? data.role : "assistant",
        content: typeof data.content === "string" ? data.content : "",
        runId,
        controlSeq: item.seq,
        ledgerVersion: typeof data.ledgerVersion === "number" ? data.ledgerVersion : undefined,
        mailboxOrigin: data.mailboxOrigin && typeof data.mailboxOrigin === "object"
          ? data.mailboxOrigin as CanonicalMessage["mailboxOrigin"]
          : undefined,
      };
      if (Array.isArray(data.attachments)) {
        message.attachments = data.attachments.map((raw) => {
          const attachment = raw as Record<string, unknown>;
          return {
            id: String(attachment.id ?? ""),
            name: String(attachment.name ?? "attachment"),
            mime: String(attachment.mime ?? "application/octet-stream"),
            size: Number(attachment.size ?? 0),
            content: typeof attachment.content === "string" ? attachment.content : undefined,
            dataUrl: typeof attachment.data_url === "string" ? attachment.data_url : undefined,
            truncated: Boolean(attachment.truncated),
          };
        });
      }
      if (message.role === "user") user = message;
      if (message.role === "assistant") {
        message.metrics = projectedResponseMetrics(data.metrics);
        const reasoning = typeof data.reasoning === "string" ? data.reasoning : streamingReasoning;
        message.streamParts = message.content === streamingText && reasoning === streamingReasoning
          ? streamParts
          : [
              ...(reasoning ? [{ kind: "thinking", content: reasoning } as const] : []),
              ...streamParts.filter((part) => part.kind === "event"),
              ...(message.content ? [{ kind: "text", content: message.content } as const] : []),
            ];
        assistant = message;
      }
      continue;
    }
    const part = eventPart(item);
    if (part) appendStreamEvent(streamParts, part);
  }
  if (
    !assistant
    && !deletedRoles.has("assistant")
    && (streamingText || streamingReasoning || streamParts.length)
  ) {
    const firstStreamSeq = items.find(
      (item) => item.run_id === runId && item.type === "assistant_delta",
    )?.seq;
    assistant = {
      id: `control-stream-${runId}`,
      role: "assistant",
      content: streamingText,
      runId,
      controlSeq: firstStreamSeq,
      streamParts,
    };
  }
  if (assistant) {
    const terminalStatus = [...items]
      .reverse()
      .find((item) => item.run_id === runId && item.type === "run_status")
      ?.data.status;
    assistant.streamTerminalOutcome = terminalStatus === "completed"
      ? "completed"
      : typeof terminalStatus === "string" &&
          terminalStatus !== "accepted" &&
          terminalStatus !== "running"
        ? "interrupted"
        : "unknown";
  }
  return [user, assistant].filter((message): message is CanonicalMessage => message !== null);
}

export function shouldReconcileControlRunProjection(
  items: ControlTimelineItemV1[],
  runId: string,
  hasCurrentProjection: boolean,
  attachedRunId?: string,
): boolean {
  if (attachedRunId === runId) return false;
  if (!hasCurrentProjection) return true;
  return items.some(
    (item) =>
      item.run_id === runId &&
      (item.type === "start" ||
        (item.type === "message" && item.data.role === "user")),
  );
}

export function mailboxMessagesFromTimeline(
  items: ControlTimelineItemV1[],
): ChatMessage[] {
  return items.flatMap((item) => {
    if (item.type !== "mailbox_reply") return [];
    const exchangeId = typeof item.data.exchange_id === "string"
      ? item.data.exchange_id
      : item.id;
    const targetThreadId = typeof item.data.target_thread_id === "string"
      ? item.data.target_thread_id
      : "";
    const status = item.data.status === "failed" ? "failed" : "replied";
    const reply = item.data.reply && typeof item.data.reply === "object"
      ? item.data.reply as Record<string, unknown>
      : {};
    const targetTitle = typeof reply.target_title === "string"
      ? reply.target_title
      : "Linked thread";
    const content = status === "failed"
      ? (typeof reply.error === "string" ? reply.error : "The linked thread failed.")
      : (typeof reply.content === "string" ? reply.content : "");
    return [{
      id: `mailbox-${exchangeId}`,
      role: "assistant",
      content,
      controlSeq: item.seq,
      mailboxReply: {
        exchangeId,
        targetThreadId,
        targetTitle,
        targetProject: typeof reply.target_project === "string" ? reply.target_project : undefined,
        status,
      },
    }];
  });
}

export function modelChangeMessagesFromTimeline(
  items: ControlTimelineItemV1[],
): ChatMessage[] {
  return items.flatMap((item) => {
    if (item.type !== "model_changed") return [];
    const previousModel = typeof item.data.previous_model === "string"
      ? item.data.previous_model.trim()
      : "";
    const model = typeof item.data.model === "string" ? item.data.model.trim() : "";
    if (!previousModel || !model || previousModel === model) return [];
    return [{
      id: `timeline-${item.id}`,
      role: "system",
      content: "",
      controlSeq: item.seq,
      modelChange: { previousModel, model },
    }];
  });
}

export function mergeModelChangeMessages(
  current: ChatMessage[],
  changes: ChatMessage[],
): ChatMessage[] {
  if (!changes.length) return current;
  const replacements = new Map(changes.map((message) => [message.id, message]));
  const merged = current.map((message) => replacements.get(message.id) ?? message);
  const existingIds = new Set(current.map((message) => message.id));
  for (const change of changes) {
    if (existingIds.has(change.id)) continue;
    const seq = change.controlSeq;
    const nextIndex = typeof seq === "number"
      ? merged.findIndex(
          (message) => typeof message.controlSeq === "number" && message.controlSeq > seq,
        )
      : -1;
    if (nextIndex >= 0) {
      merged.splice(nextIndex, 0, change);
      existingIds.add(change.id);
      continue;
    }
    let previousIndex = -1;
    if (typeof seq === "number") {
      for (let index = merged.length - 1; index >= 0; index -= 1) {
        const candidate = merged[index].controlSeq;
        if (typeof candidate === "number" && candidate <= seq) {
          previousIndex = index;
          break;
        }
      }
    }
    merged.splice(previousIndex >= 0 ? previousIndex + 1 : merged.length, 0, change);
    existingIds.add(change.id);
  }
  return merged;
}

export function mergeMailboxMessages(
  current: ChatMessage[],
  mailbox: ChatMessage[],
): ChatMessage[] {
  if (!mailbox.length) return current;
  const byId = new Map(mailbox.map((message) => [message.id, message]));
  const merged = current.map((message) => byId.get(message.id) ?? message);
  const existingIds = new Set(current.map((message) => message.id));
  merged.push(...mailbox.filter((message) => !existingIds.has(message.id)));
  return merged;
}

export function mergeControlRunMessages(
  current: ChatMessage[],
  runId: string,
  projected: ChatMessage[],
): ChatMessage[] {
  const base = current.filter((message) => (message as CanonicalMessage).runId !== runId);
  if (projected.length === 0) return base;

  const projectedUser = projected.find((message) => message.role === "user");
  let nextBase = base;
  let nextProjected = projected;
  if (projectedUser) {
    for (let index = base.length - 1; index >= 0; index -= 1) {
      const candidate = base[index];
      if (
        candidate.role !== "user" ||
        (candidate as CanonicalMessage).runId ||
        (candidate.id !== projectedUser.id && candidate.content !== projectedUser.content)
      ) {
        continue;
      }
      nextBase = [...base.slice(0, index), ...base.slice(index + 1)];
      if (candidate.id && candidate.id !== projectedUser.id) {
        nextProjected = projected.map((message) =>
          message.id === projectedUser.id ? { ...message, id: candidate.id } : message,
        );
      }
      break;
    }
  }
  return [...nextBase, ...nextProjected];
}

export async function pollControlRun(
  threadId: string,
  runId: string,
  signal: AbortSignal,
  onItems: (items: ControlTimelineItemV1[]) => void,
): Promise<{ status: string; error?: string }> {
  let afterSeq: number | undefined;
  const items: ControlTimelineItemV1[] = [];
  let wake: (() => void) | null = null;
  let socketAvailable = false;
  void streamControlEvents(signal, (event) => {
    if (
      event.type === "sync.required" ||
      !event.thread_id ||
      event.thread_id === threadId
    ) {
      socketAvailable = true;
      wake?.();
    }
  });
  for (;;) {
    if (signal.aborted) return { status: "aborted" };
    const page = await getControlTimeline(
      threadId,
      afterSeq == null ? { tail: 500 } : { afterSeq },
    );
    for (const item of page.items) {
      if (!items.some((known) => known.epoch === item.epoch && known.seq === item.seq)) {
        items.push(item);
      }
    }
    items.sort((left, right) => left.seq - right.seq);
    afterSeq = page.last_seq ?? afterSeq;
    onItems(items);
    const terminal = [...items]
      .reverse()
      .find((item) => item.run_id === runId && item.type === "run_status");
    const status = terminal?.data.status;
    if (typeof status === "string" && status !== "accepted" && status !== "running") {
      const error = terminal?.data.error;
      return {
        status,
        error:
          error && typeof error === "object" && "message" in error
            ? String((error as { message?: unknown }).message ?? "")
            : undefined,
      };
    }
    await new Promise<void>((resolve) => {
      let settled = false;
      let timer: number | undefined;
      const finish = () => {
        if (settled) return;
        settled = true;
        wake = null;
        if (timer !== undefined) window.clearTimeout(timer);
        signal.removeEventListener("abort", finish);
        document.removeEventListener("visibilitychange", visibilityChanged);
        resolve();
      };
      const visibilityChanged = () => {
        if (document.visibilityState === "visible") finish();
      };
      wake = finish;
      signal.addEventListener("abort", finish, { once: true });
      document.addEventListener("visibilitychange", visibilityChanged);
      if (document.visibilityState !== "hidden") {
        timer = window.setTimeout(finish, socketAvailable ? 1_500 : 500);
      }
      if (signal.aborted) finish();
    });
  }
}
