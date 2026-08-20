export function threadLinkDropDecision({
  sourceThreadId,
  targetThreadId,
  linkedThreadIds,
  sourceIsChild = false,
}: {
  sourceThreadId: string;
  targetThreadId: string;
  linkedThreadIds: readonly string[];
  sourceIsChild?: boolean;
}): "valid" | "self" | "duplicate" | "child" {
  if (sourceIsChild) return "child";
  if (sourceThreadId === targetThreadId) return "self";
  if (linkedThreadIds.includes(sourceThreadId)) return "duplicate";
  return "valid";
}
