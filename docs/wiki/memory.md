---
id: memory
path: memory
label: Memory
title: Memory and RAG
summary: Personal and Project hybrid memory, bounded provenance-aware context injection, review/revoke lifecycle, and measurable retrieval quality.
group: Local data
order: 60
updated: 2026-09-26
---

The normal memory library has two scopes. **Personal** follows you across projects; **Project** uses a sanitized Git-origin identity when one exists, so clones and worktrees of the same remote share memory. Different origins stay isolated. Outside Git, Project falls back to the canonical folder path. Existing exact-folder and thread-scoped memories remain searchable and can be moved into Personal or Project from the library's Legacy view.

When Memory is enabled, normal chat turns search scoped memory with exact-term and embedding retrieval. The two rankings are combined with equal-weight reciprocal rank fusion, so exact identifiers remain findable while semantic matches still work. Lexical retrieval remains available if embeddings fail, and a search with no candidates returns no memory context. Recall does not force the agent/tool loop by itself. Durable writes use `memory_register` only when the user explicitly asks to remember/save/store context, or when the turn is already running through a tool-capable agent path.

## Memory systems

| System | Route | Behavior |
|---|---|---|
| Classic RAG | `/memory/ingest` and `/memory/search` | Embeds text through the configured embedding-capable provider and retrieves nearby memories. |
| Scoped hybrid memory | `/memory/register` and `/memory/graph/search` | Retrieves scoped, non-archived nodes with lexical and semantic ranking. The `/memory/graph/search` name is retained for compatibility; retrieval does not traverse memory edges. |
| Memory library | `/memory/scopes`, `/memory/nodes`, node update/delete/archive/review/restore routes | Searches, adds, edits, reviews, archives, restores, permanently deletes, and moves legacy entries. |
| Embedding index | `GET /memory/embeddings`, `POST /memory/embeddings/reindex`, `POST /memory/embeddings/cancel`, `PUT /memory/embeddings/model` | Reports which memories match the current embedding model, starts or stops background re-embedding, and pins or unpins the embedding model. |
| Retrieval benchmark | `/memory/benchmark` | Runs labeled queries through the production scoped retriever and reports recall@k and mean reciprocal rank without mutating memory. |
| Agent memory tool | `memory_register` | Saves `content` plus an optional `title` to `personal` or `project`; it defaults to Project when a folder exists and Personal otherwise. |

## Scopes

| Scope | Use it for |
|---|---|
| Personal | Durable preferences and facts that should follow you across projects. |
| Project | Repo conventions, architecture decisions, and product facts tied to one workspace folder. |

Project memory requires an active project folder. Each enabled turn requests 20 candidates across Personal, the stable Project identity, the legacy exact-folder identity, and legacy memories from that same thread. The desktop injects at most five entries inside a hard 1,024-token memory budget. If the highest-ranked entry alone is too large, it is truncated with a visible marker; oversized later entries are skipped so smaller candidates can still fit. Every injected entry includes its scope, kind, source, and updated date, plus an instruction that memory is untrusted historical context and current user statements and workspace files take precedence.

Results are deduplicated by memory node id. New writes use only the stable identity. Changing a repository's origin starts a new stable scope while legacy folder memories remain readable. The FTS index is backfilled without rewriting node records, and new thread-scoped memories are not created.

## Remote embedding boundary

Embeddings follow the selected provider route. Local Ollama or LM Studio embeddings stay on the machine. Remote embedding calls pass through the same privacy gate as remote chat and media prompts. Exact-term retrieval uses bundled SQLite FTS5 locally and does not add a remote boundary.

## Embedding model changes

Every stored vector records the model that produced it and its dimension. A pinned embedding model, set with `PUT /memory/embeddings/model` (`{"model": "..."}`, or `null` to unpin) or **Pin this model** in the Memory library, is always the current model: every memory write and search embeds with it whatever model the chat uses, and pinning a model other than the current one re-embeds the library with it. Without a pin, memory embeds with the chat's model and the current model is the one chats settle on: a different model becomes current only after it returns vectors twice in a row, so alternating between two embedding-capable chat models does not re-embed the library on every switch. The pin is stored with the memory database and survives restarts. A new current model with a different name or dimension makes older vectors stale. Semantic search loads only vectors from the query's model and dimension, so stale entries are filtered in SQL instead of being compared and discarded. Vectors saved before models were tracked still count while their dimension matches.

When the current model changes, and on the first memory access after milim starts, a background job re-embeds stale entries and entries that were saved without a vector. It works in batches of 32, newest active entries first, through the store's own embedding route and privacy gate. Progress lives in the memory rows themselves, so a cancelled, failed, or interrupted job resumes where it stopped. An entry the current model rejects individually, for example text blocked by the privacy gate, is marked as attempted and is not retried until the model changes again. If the model rejects a whole batch, the job pauses and reports the error.

`GET /memory/embeddings` returns the current `model` and `dim`, the pinned `configured_model` (or `null`), plus counts of `total`, `current`, `stale` (vectors from another model or dimension), `missing` (no vector yet), and `unavailable` (rejected by the current model), along with `reindexing`, `reindexed`, and `last_error`. The Memory library shows a notice such as "3 memories use an older embedding model; re-indexing..." with **Cancel** while the job runs, and **Re-index** when it is paused. Until an entry is re-embedded it remains findable through exact-term retrieval.

Without a pin, a search that runs with a model other than the current one only matches vectors from that model semantically until it becomes current; pin a model to keep recall on one index.

## Plan-mode guard

Plan mode disables memory search and memory writes. Planning remains read-only: the assistant can inspect context, but it cannot register durable memory while the plan is still unapproved.

## Review, revoke, and restore

Every memory exposes its lifecycle in the Memory library. User-created and manually edited entries are marked reviewed. Tool-created or migrated entries remain **Needs review** until explicitly acknowledged. **Forget** revokes an entry from normal retrieval by archiving it; **Restore** returns it to retrieval and marks it reviewed; permanent deletion remains a second, confirmed action available only for archived entries. Review status is provenance for the user and does not silently change ranking.

## Retrieval benchmark

`POST /memory/benchmark` accepts labeled cases containing a query, relevant memory node IDs, and optional scopes. It calls the same hybrid retrieval path used by chat, then returns per-case retrieved IDs, first relevant rank, recall@k, reciprocal rank, macro-average recall@k, and mean reciprocal rank. Empty cases and cases without relevance labels are rejected, `top_k` is bounded to 50, and archived entries remain excluded unless explicitly requested. This makes retrieval changes comparable against stable project-specific fixtures instead of relying on anecdotal prompts.

## Register memory over HTTP

```bash Register a project memory
curl http://127.0.0.1:7377/memory/register \
  -H "Content-Type: application/json" \
  -d '{
    "model": "default",
    "scope": { "kind": "project", "label": "milim", "locator": "C:\\repo\\milim" },
    "node": {
      "kind": "decision",
      "title": "Use markdown docs source",
      "body": "The site imports docs/wiki markdown and builds search from headings.",
      "confidence": 1,
      "source": "user"
    }
  }'
```
