import {
  getControlTimeline,
  type ChatAttachment,
  type ChatMessage,
  type ChatStreamPart,
  type ControlTimelineItemV1,
} from "../api.js";

type CanonicalMessage = ChatMessage & { runId?: string };

export function controlAttachments(attachments?: ChatAttachment[]) {
  return (attachments ?? []).map(({ dataUrl, sourcePath: _sourcePath, ...attachment }) => ({
    ...attachment,
    data_url: dataUrl,
  }));
}

function eventPart(item: ControlTimelineItemV1): ChatStreamPart | null {
  const data = item.data;
  const name = typeof data.name === "string" ? data.name : item.type;
  if (item.type === "tool_call" || item.type === "tool_start") {
    return {
      kind: "event",
      eventType: "tool",
      label: name,
      status: "running",
      callId: typeof data.call_id === "string" ? data.call_id : undefined,
    };
  }
  if (item.type === "tool_result" || item.type === "tool_end") {
    return {
      kind: "event",
      eventType: "tool",
      label: name,
      status: data.error ? "error" : "done",
      callId: typeof data.call_id === "string" ? data.call_id : undefined,
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

/** Fold the authoritative items for one run into the two user-visible turns. */
export function projectControlRunMessages(
  items: ControlTimelineItemV1[],
  runId: string,
): CanonicalMessage[] {
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
      if (text) streamParts.push({ kind: "text", content: text });
      if (reasoning) streamParts.push({ kind: "thinking", content: reasoning });
      continue;
    }
    if (item.type === "message" && typeof data.id === "string") {
      const message: CanonicalMessage = {
        id: data.id,
        role: typeof data.role === "string" ? data.role : "assistant",
        content: typeof data.content === "string" ? data.content : "",
        runId,
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
        const reasoning = typeof data.reasoning === "string" ? data.reasoning : streamingReasoning;
        message.streamParts = [
          ...(reasoning ? [{ kind: "thinking", content: reasoning } as const] : []),
          ...streamParts.filter((part) => part.kind === "event"),
          ...(message.content ? [{ kind: "text", content: message.content } as const] : []),
        ];
        assistant = message;
      }
      continue;
    }
    const part = eventPart(item);
    if (part) streamParts.push(part);
  }
  if (!assistant && (streamingText || streamingReasoning || streamParts.length)) {
    assistant = {
      id: `control-stream-${runId}`,
      role: "assistant",
      content: streamingText,
      runId,
      streamParts,
    };
  }
  return [user, assistant].filter((message): message is CanonicalMessage => message !== null);
}

export function mergeControlRunMessages(
  current: ChatMessage[],
  runId: string,
  projected: ChatMessage[],
): ChatMessage[] {
  const base = current.filter((message) => (message as CanonicalMessage).runId !== runId);
  return [...base, ...projected];
}

export async function pollControlRun(
  threadId: string,
  runId: string,
  signal: AbortSignal,
  onItems: (items: ControlTimelineItemV1[]) => void,
): Promise<{ status: string; error?: string }> {
  let afterSeq: number | undefined;
  const items: ControlTimelineItemV1[] = [];
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
    await new Promise<void>((resolve) => window.setTimeout(resolve, 120));
  }
}
