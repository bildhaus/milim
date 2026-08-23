import type { ChatMessage } from "../api.js";

export const CHAT_SCROLL_BOTTOM_THRESHOLD = 32;

type ScrollMetrics = {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
};

export function isNearScrollBottom(metrics: ScrollMetrics, threshold = CHAT_SCROLL_BOTTOM_THRESHOLD): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold;
}

export function followScrollTop(metrics: Pick<ScrollMetrics, "scrollHeight" | "clientHeight">): number {
  return Math.max(0, metrics.scrollHeight - metrics.clientHeight);
}

export function scrollTopAfterLayoutChange(metrics: ScrollMetrics, following: boolean): number {
  return following ? followScrollTop(metrics) : metrics.scrollTop;
}

export function transcriptSpacerHeight(
  rowCount: number,
  estimatedRowHeight: number,
  rowGap: number,
  knownRowHeights: readonly number[] = [],
): number {
  if (rowCount <= 0) return 0;
  const knownHeightAdjustment = knownRowHeights.reduce(
    (total, height) => total + height - estimatedRowHeight,
    0,
  );
  return rowCount * estimatedRowHeight + knownHeightAdjustment +
    Math.max(0, rowCount - 1) * rowGap;
}

export function scrollTopForRestoredAnchor(
  scrollTop: number,
  previousOffset: number,
  nextOffset: number,
): number {
  return Math.max(0, scrollTop + nextOffset - previousOffset);
}

export function transcriptMessageRenderId(
  threadId: string,
  message: Pick<ChatMessage, "id" | "role" | "runId">,
  index: number,
): string {
  if (message.role === "assistant" && message.runId) {
    return `${threadId}:run:${message.runId}:assistant`;
  }
  return `${threadId}:message:${message.id ?? index}`;
}

export function peekEnteringMessageIds(
  seenIds: ReadonlySet<string>,
  messageIds: readonly (string | undefined | null)[],
  shouldAnimate: boolean,
): string[] {
  if (!shouldAnimate) return [];
  const entering: string[] = [];
  for (const id of messageIds) {
    if (!id || seenIds.has(id) || entering.includes(id)) continue;
    entering.push(id);
  }
  return entering;
}
