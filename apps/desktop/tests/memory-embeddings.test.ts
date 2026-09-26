import type { MemoryEmbeddingStatus } from "../src/api.js";
import { memoryEmbeddingModelLabel, memoryEmbeddingNotice } from "../src/lib/memoryEmbeddings.js";

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const base: MemoryEmbeddingStatus = {
  model: "nomic-embed-text",
  dim: 768,
  total: 10,
  current: 10,
  stale: 0,
  missing: 0,
  unavailable: 0,
  reindexing: false,
  reindexed: 0,
  last_error: null,
};

equal(memoryEmbeddingNotice(null), null, "no status");
equal(memoryEmbeddingNotice({ ...base, model: null, stale: 3 }), null, "no embedding model yet");
equal(memoryEmbeddingNotice(base), null, "fully indexed");
equal(memoryEmbeddingNotice({ ...base, unavailable: 2 }), null, "unavailable entries are not retried");
equal(
  memoryEmbeddingNotice({ ...base, stale: 4, reindexing: true }),
  "4 memories use an older embedding model; re-indexing...",
  "re-indexing stale vectors",
);
equal(
  memoryEmbeddingNotice({ ...base, stale: 1, missing: 1 }),
  "1 memory uses an older embedding model and 1 memory has no embedding yet. Search matches them by keyword until they are re-indexed.",
  "idle with stale and missing entries",
);
equal(
  memoryEmbeddingNotice({ ...base, missing: 2, last_error: "model unavailable" }),
  "2 memories have no embedding yet. Re-indexing paused: model unavailable",
  "paused job",
);

equal(memoryEmbeddingModelLabel(null), null, "no status, no model line");
equal(memoryEmbeddingModelLabel({ ...base, model: null }), null, "no embedding model yet");
equal(
  memoryEmbeddingModelLabel(base),
  "Memory embeds with nomic-embed-text, the model chats last settled on",
  "unpinned model follows chats",
);
equal(
  memoryEmbeddingModelLabel({ ...base, configured_model: "text-embedding-3-small" }),
  "Memory embeds with text-embedding-3-small (pinned)",
  "pinned model",
);
