import type { MemoryEmbeddingStatus } from "../api";

function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/**
 * One-line Memory manager notice for memories whose vectors do not match the
 * current embedding model, or `null` when every memory is searchable
 * semantically (or no embedding model has been used yet).
 */
export function memoryEmbeddingNotice(status: MemoryEmbeddingStatus | null): string | null {
  if (!status?.model) return null;
  const parts = [
    status.stale > 0 && `${count(status.stale, "memory uses", "memories use")} an older embedding model`,
    status.missing > 0 && `${count(status.missing, "memory has", "memories have")} no embedding yet`,
  ].filter(Boolean);
  if (parts.length === 0) return null;
  const summary = parts.join(" and ");
  if (status.reindexing) return `${summary}; re-indexing...`;
  if (status.last_error) return `${summary}. Re-indexing paused: ${status.last_error}`;
  return `${summary}. Search matches them by keyword until they are re-indexed.`;
}
