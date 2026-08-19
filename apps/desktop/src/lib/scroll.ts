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

export function nextEnteringMessageIds(
  seenIds: Set<string>,
  messageIds: readonly (string | undefined | null)[],
  shouldAnimate: boolean,
): string[] {
  const entering = peekEnteringMessageIds(seenIds, messageIds, shouldAnimate);
  for (const id of messageIds) {
    if (id) seenIds.add(id);
  }
  return entering;
}
