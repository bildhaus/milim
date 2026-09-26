//! `milim-memory` - a hybrid lexical and embedding-backed memory store.
//!
//! [`MemoryStore::add`] embeds text (via any [`ModelService`]) and persists it
//! through `milim-storage`; [`MemoryStore::search`] embeds a query and returns
//! the most cosine-similar entries. Scoped memory combines embeddings with
//! exact-term retrieval for thread and project recall.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use milim_core::{Error, Result};
use milim_inference::SharedService;
use milim_storage::{Database, Migration};

const MAX_MEMORY_BENCHMARK_CASES: usize = 100;
/// Memories re-embedded per background batch.
const REEMBED_BATCH: usize = 32;
const META_EMBEDDING_MODEL: &str = "embedding_model";
const META_EMBEDDING_DIM: &str = "embedding_dim";
/// The embedding model the user pinned for memory, if any.
const META_CONFIGURED_MODEL: &str = "configured_embedding_model";
/// Rows that still need a vector from the current model `?1` with
/// dimension `?2`: vectors from another model or dimension, and entries
/// without a vector that the current model has not attempted yet.
const REEMBED_PENDING_SQL: &str = "NOT (embedding_model = ?1 AND (dim = 0 OR dim = ?2))";

/// Schema for the memory store.
pub const MEMORY_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "memories",
        sql: "CREATE TABLE memories (
            id         TEXT PRIMARY KEY,
            text       TEXT NOT NULL,
            dim        INTEGER NOT NULL,
            embedding  BLOB NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
          );",
    },
    Migration {
        version: 2,
        name: "memory_graph",
        sql: "CREATE TABLE memory_scopes (
            id           TEXT PRIMARY KEY,
            kind         TEXT NOT NULL,
            label        TEXT NOT NULL,
            locator      TEXT NOT NULL,
            locator_hash TEXT NOT NULL,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(kind, locator_hash)
          );
          CREATE INDEX memory_scopes_kind_idx ON memory_scopes(kind);

          CREATE TABLE memory_nodes (
            id          TEXT PRIMARY KEY,
            scope_id    TEXT NOT NULL REFERENCES memory_scopes(id) ON DELETE CASCADE,
            kind        TEXT NOT NULL DEFAULT 'fact',
            title       TEXT NOT NULL,
            body        TEXT NOT NULL,
            dim         INTEGER NOT NULL DEFAULT 0,
            embedding   BLOB NOT NULL DEFAULT X'',
            confidence  REAL NOT NULL DEFAULT 1.0,
            source      TEXT NOT NULL DEFAULT '',
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
            archived_at TEXT
          );
          CREATE INDEX memory_nodes_scope_idx ON memory_nodes(scope_id);
          CREATE INDEX memory_nodes_kind_idx ON memory_nodes(kind);
          CREATE INDEX memory_nodes_archived_idx ON memory_nodes(archived_at);

          CREATE TABLE memory_edges (
            id           TEXT PRIMARY KEY,
            scope_id     TEXT NOT NULL REFERENCES memory_scopes(id) ON DELETE CASCADE,
            from_node_id TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
            predicate    TEXT NOT NULL,
            to_node_id   TEXT NOT NULL REFERENCES memory_nodes(id) ON DELETE CASCADE,
            confidence   REAL NOT NULL DEFAULT 1.0,
            created_at   TEXT NOT NULL DEFAULT (datetime('now'))
          );
          CREATE INDEX memory_edges_scope_idx ON memory_edges(scope_id);
          CREATE INDEX memory_edges_from_idx ON memory_edges(from_node_id);
          CREATE INDEX memory_edges_to_idx ON memory_edges(to_node_id);

          CREATE TABLE memory_events (
            id         TEXT PRIMARY KEY,
            scope_id   TEXT NOT NULL REFERENCES memory_scopes(id) ON DELETE CASCADE,
            node_id    TEXT REFERENCES memory_nodes(id) ON DELETE SET NULL,
            action     TEXT NOT NULL,
            thread_id  TEXT NOT NULL DEFAULT '',
            message_id TEXT NOT NULL DEFAULT '',
            summary    TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
          );
          CREATE INDEX memory_events_scope_idx ON memory_events(scope_id);
          CREATE INDEX memory_events_node_idx ON memory_events(node_id);",
    },
    Migration {
        version: 3,
        name: "memory_nodes_fts",
        sql: "CREATE VIRTUAL TABLE memory_nodes_fts USING fts5(
            node_id UNINDEXED,
            title,
            body,
            tokenize = 'unicode61'
          );

          INSERT INTO memory_nodes_fts (node_id, title, body)
          SELECT id, title, body FROM memory_nodes;

          CREATE TRIGGER memory_nodes_fts_insert
          AFTER INSERT ON memory_nodes BEGIN
            INSERT INTO memory_nodes_fts (node_id, title, body)
            VALUES (new.id, new.title, new.body);
          END;

          CREATE TRIGGER memory_nodes_fts_update
          AFTER UPDATE OF title, body ON memory_nodes BEGIN
            DELETE FROM memory_nodes_fts WHERE node_id = old.id;
            INSERT INTO memory_nodes_fts (node_id, title, body)
            VALUES (new.id, new.title, new.body);
          END;

          CREATE TRIGGER memory_nodes_fts_delete
          AFTER DELETE ON memory_nodes BEGIN
            DELETE FROM memory_nodes_fts WHERE node_id = old.id;
          END;",
    },
    Migration {
        version: 4,
        name: "memory_review_lifecycle",
        sql: "ALTER TABLE memory_nodes ADD COLUMN reviewed_at TEXT;",
    },
    Migration {
        version: 5,
        name: "memory_embedding_model",
        sql: "ALTER TABLE memory_nodes ADD COLUMN embedding_model TEXT NOT NULL DEFAULT '';
          ALTER TABLE memories ADD COLUMN embedding_model TEXT NOT NULL DEFAULT '';
          CREATE INDEX memory_nodes_embedding_idx ON memory_nodes(embedding_model, dim);
          CREATE TABLE memory_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
          );",
    },
];

/// A search hit: the stored text and its similarity to the query.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryHit {
    pub id: String,
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryScopeInput {
    pub kind: String,
    pub label: String,
    pub locator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNodeInput {
    #[serde(default = "default_node_kind")]
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub source: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryNodeUpdate {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEdgeInput {
    pub predicate: String,
    pub to_node_id: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryEventInput {
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryScopeRef {
    pub kind: String,
    pub locator: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryScope {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub locator: String,
    pub locator_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryNode {
    pub id: String,
    pub scope_id: String,
    pub scope_kind: String,
    pub scope_label: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    pub confidence: f32,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryEdge {
    pub id: String,
    pub scope_id: String,
    pub from_node_id: String,
    pub predicate: String,
    pub to_node_id: String,
    pub confidence: f32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryEvent {
    pub id: String,
    pub scope_id: String,
    pub node_id: Option<String>,
    pub action: String,
    pub thread_id: String,
    pub message_id: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryNotice {
    pub id: String,
    pub node_id: String,
    pub scope_kind: String,
    pub scope_label: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryRegistration {
    pub scope: MemoryScope,
    pub node: MemoryNode,
    pub event: MemoryEvent,
    pub notice: MemoryNotice,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryGraphHit {
    pub node: MemoryNode,
    pub score: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryBenchmarkCase {
    #[serde(default)]
    pub name: String,
    pub query: String,
    pub relevant_node_ids: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<MemoryScopeRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryBenchmarkCaseResult {
    pub name: String,
    pub query: String,
    pub relevant_node_ids: Vec<String>,
    pub retrieved_node_ids: Vec<String>,
    pub first_relevant_rank: Option<usize>,
    pub recall_at_k: f32,
    pub reciprocal_rank: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryBenchmarkReport {
    pub top_k: usize,
    pub case_count: usize,
    pub recall_at_k: f32,
    pub mean_reciprocal_rank: f32,
    pub cases: Vec<MemoryBenchmarkCaseResult>,
}

/// Embedding coverage of scoped memories relative to the current model.
///
/// The current model is the most recent model that produced an embedding
/// for this store. `stale` entries hold vectors from another model or
/// dimension, `missing` entries have no vector the current model has tried,
/// and `unavailable` entries could not be embedded by the current model.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryEmbeddingStatus {
    pub model: Option<String>,
    /// The pinned embedding model. `None` follows the models chats embed with.
    pub configured_model: Option<String>,
    pub dim: usize,
    pub total: usize,
    pub current: usize,
    pub stale: usize,
    pub missing: usize,
    pub unavailable: usize,
    pub reindexing: bool,
    pub reindexed: usize,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct ReembedProgress {
    running: bool,
    requested: u64,
    processed: usize,
    last_error: Option<String>,
}

/// Background re-embedding state shared by every view of one store.
struct EmbeddingIndex {
    /// The store's own embedder; run-scoped views never drive the job.
    embedder: SharedService,
    progress: Mutex<ReembedProgress>,
    cancel: AtomicBool,
    resumed: AtomicBool,
    /// The pinned embedding model, mirrored from `memory_meta`.
    configured: Mutex<Option<String>>,
    /// Without a pinned model, a different model must return vectors twice
    /// in a row before it replaces the current one.
    candidate: Mutex<Option<(String, usize)>>,
}

/// An embedding-backed memory store.
///
/// The `Database` lives behind a `Mutex` so the async methods stay `Send`
/// (rusqlite's `Connection` is not `Sync`); the lock is only held for the
/// synchronous SQL section, never across an `await`.
pub struct MemoryStore {
    db: Arc<Mutex<Database>>,
    embedder: SharedService,
    index: Arc<EmbeddingIndex>,
}

impl MemoryStore {
    /// Open a memory store, applying the schema migration.
    pub fn new(db: Database, embedder: SharedService) -> Result<Self> {
        db.migrate(MEMORY_MIGRATIONS)?;
        let configured = read_meta(db.conn(), META_CONFIGURED_MODEL)?;
        let store = Self {
            db: Arc::new(Mutex::new(db)),
            embedder: embedder.clone(),
            index: Arc::new(EmbeddingIndex {
                embedder,
                progress: Mutex::new(ReembedProgress::default()),
                cancel: AtomicBool::new(false),
                resumed: AtomicBool::new(false),
                configured: Mutex::new(configured),
                candidate: Mutex::new(None),
            }),
        };
        store.resume_reembed();
        Ok(store)
    }

    /// Share this store's database while using a different embedder for one
    /// request or run.
    pub fn with_embedder(&self, embedder: SharedService) -> Self {
        Self {
            db: self.db.clone(),
            embedder,
            index: self.index.clone(),
        }
    }

    /// Embed `text` with `model` and persist it. Returns the new id.
    pub async fn add(&self, model: &str, text: &str) -> Result<String> {
        let model = &self.embedding_model_for(model);
        let embedding = self.embed_one(model, text).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let bytes = vec_to_bytes(&embedding);

        let db = self.db.lock().expect("memory db poisoned");
        db.conn()
            .execute(
                "INSERT INTO memories (id, text, dim, embedding, embedding_model)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, text, embedding.len() as i64, bytes, model],
            )
            .map_err(sqlite)?;
        Ok(id)
    }

    /// Register a structured graph memory in a thread/project/global scope.
    pub async fn register(
        &self,
        model: &str,
        scope: MemoryScopeInput,
        node: MemoryNodeInput,
        edges: Vec<MemoryEdgeInput>,
        event: MemoryEventInput,
    ) -> Result<MemoryRegistration> {
        let scope = normalize_scope(scope)?;
        let node = normalize_node(node)?;
        let reviewed_by_user = node.source.eq_ignore_ascii_case("user");
        let text = memory_text(&node.title, &node.body);
        let model = self.embedding_model_for(model);
        let model = model.as_str();
        let embedding = match self.embed_one(model, &text).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("memory graph embedding unavailable; storing without vector: {e}");
                Vec::new()
            }
        };
        let bytes = vec_to_bytes(&embedding);
        let embedding_model = if embedding.is_empty() { "" } else { model };
        let node_id = uuid::Uuid::new_v4().to_string();
        let event_id = uuid::Uuid::new_v4().to_string();
        let summary = if event.summary.trim().is_empty() {
            node.title.clone()
        } else {
            event.summary.trim().to_string()
        };

        let db = self.db.lock().expect("memory db poisoned");
        upsert_scope(db.conn(), &scope)?;
        db.conn()
            .execute(
                "INSERT INTO memory_nodes
                 (id, scope_id, kind, title, body, dim, embedding, confidence, source, reviewed_at,
                  embedding_model)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                         CASE WHEN ?10 THEN datetime('now') END, ?11)",
                params![
                    node_id,
                    scope.id,
                    node.kind,
                    node.title,
                    node.body,
                    embedding.len() as i64,
                    bytes,
                    clamp_confidence(node.confidence),
                    node.source,
                    reviewed_by_user,
                    embedding_model,
                ],
            )
            .map_err(sqlite)?;

        for edge in edges
            .into_iter()
            .filter(|e| !e.to_node_id.trim().is_empty())
        {
            db.conn()
                .execute(
                    "INSERT INTO memory_edges
                     (id, scope_id, from_node_id, predicate, to_node_id, confidence)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        uuid::Uuid::new_v4().to_string(),
                        scope.id,
                        node_id,
                        clean_default(&edge.predicate, "related_to"),
                        edge.to_node_id.trim(),
                        clamp_confidence(edge.confidence)
                    ],
                )
                .map_err(sqlite)?;
        }

        db.conn()
            .execute(
                "INSERT INTO memory_events
                 (id, scope_id, node_id, action, thread_id, message_id, summary)
                 VALUES (?1, ?2, ?3, 'created', ?4, ?5, ?6)",
                params![
                    event_id,
                    scope.id,
                    node_id,
                    event.thread_id.trim(),
                    event.message_id.trim(),
                    summary
                ],
            )
            .map_err(sqlite)?;

        let scope = get_scope_by_id(db.conn(), &scope.id)?;
        let node = get_node_by_id(db.conn(), &node_id)?.ok_or_else(|| {
            Error::Other("registered memory node could not be reloaded".to_string())
        })?;
        let event = get_event_by_id(db.conn(), &event_id)?.ok_or_else(|| {
            Error::Other("registered memory event could not be reloaded".to_string())
        })?;
        let notice = MemoryNotice {
            id: event.id.clone(),
            node_id: node.id.clone(),
            scope_kind: scope.kind.clone(),
            scope_label: scope.label.clone(),
            summary: event.summary.clone(),
            created_at: event.created_at.clone(),
        };
        Ok(MemoryRegistration {
            scope,
            node,
            event,
            notice,
        })
    }

    pub fn list_scopes(&self) -> Result<Vec<MemoryScope>> {
        let db = self.db.lock().expect("memory db poisoned");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, kind, label, locator, locator_hash, created_at, updated_at
                 FROM memory_scopes
                 ORDER BY updated_at DESC, label COLLATE NOCASE",
            )
            .map_err(sqlite)?;
        let rows = stmt.query_map([], row_to_scope).map_err(sqlite)?;
        collect_rows(rows)
    }

    pub fn list_nodes(
        &self,
        scope: Option<MemoryScopeRef>,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<MemoryNode>> {
        let db = self.db.lock().expect("memory db poisoned");
        let limit = limit.clamp(1, 500) as i64;
        let archived_clause = if include_archived {
            ""
        } else {
            "AND n.archived_at IS NULL"
        };

        if let Some(scope) = scope {
            let scope_id = scope_id_for(&normalize_scope_ref(&scope)?);
            let sql = format!(
                "SELECT n.id, n.scope_id, s.kind, s.label, n.kind, n.title, n.body,
                        n.confidence, n.source, n.created_at, n.updated_at, n.archived_at,
                        n.reviewed_at
                 FROM memory_nodes n
                 JOIN memory_scopes s ON s.id = n.scope_id
                 WHERE n.scope_id = ?1 {archived_clause}
                 ORDER BY n.updated_at DESC
                 LIMIT ?2"
            );
            let mut stmt = db.conn().prepare(&sql).map_err(sqlite)?;
            let rows = stmt
                .query_map(params![scope_id, limit], row_to_node)
                .map_err(sqlite)?;
            return collect_rows(rows);
        }

        let sql = format!(
            "SELECT n.id, n.scope_id, s.kind, s.label, n.kind, n.title, n.body,
                    n.confidence, n.source, n.created_at, n.updated_at, n.archived_at,
                    n.reviewed_at
             FROM memory_nodes n
             JOIN memory_scopes s ON s.id = n.scope_id
             WHERE 1 = 1 {archived_clause}
             ORDER BY n.updated_at DESC
             LIMIT ?1"
        );
        let mut stmt = db.conn().prepare(&sql).map_err(sqlite)?;
        let rows = stmt
            .query_map(params![limit], row_to_node)
            .map_err(sqlite)?;
        collect_rows(rows)
    }

    pub fn list_edges(&self, node_id: Option<&str>) -> Result<Vec<MemoryEdge>> {
        let db = self.db.lock().expect("memory db poisoned");
        if let Some(node_id) = node_id {
            let mut stmt = db
                .conn()
                .prepare(
                    "SELECT id, scope_id, from_node_id, predicate, to_node_id, confidence, created_at
                     FROM memory_edges
                     WHERE from_node_id = ?1 OR to_node_id = ?1
                     ORDER BY created_at DESC",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map(params![node_id], row_to_edge)
                .map_err(sqlite)?;
            return collect_rows(rows);
        }
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, scope_id, from_node_id, predicate, to_node_id, confidence, created_at
                 FROM memory_edges
                 ORDER BY created_at DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt.query_map([], row_to_edge).map_err(sqlite)?;
        collect_rows(rows)
    }

    pub async fn update_node(
        &self,
        model: &str,
        id: &str,
        update: MemoryNodeUpdate,
    ) -> Result<Option<MemoryNode>> {
        let current = {
            let db = self.db.lock().expect("memory db poisoned");
            get_node_by_id(db.conn(), id)?
        };
        let Some(current) = current else {
            return Ok(None);
        };

        let kind = update
            .kind
            .as_deref()
            .map(clean_node_kind)
            .unwrap_or_else(|| current.kind.clone());
        let title = update
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&current.title)
            .to_string();
        let body = update
            .body
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current.body)
            .to_string();
        let confidence = update
            .confidence
            .map(clamp_confidence)
            .unwrap_or(current.confidence);
        let source = update.source.unwrap_or(current.source);

        let model = self.embedding_model_for(model);
        let model = model.as_str();
        let embedding = match self.embed_one(model, &memory_text(&title, &body)).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("memory graph re-embedding unavailable; keeping empty vector: {e}");
                Vec::new()
            }
        };
        let bytes = vec_to_bytes(&embedding);
        let embedding_model = if embedding.is_empty() { "" } else { model };

        let db = self.db.lock().expect("memory db poisoned");
        db.conn()
            .execute(
                "UPDATE memory_nodes
                 SET kind = ?2, title = ?3, body = ?4, dim = ?5, embedding = ?6,
                     confidence = ?7, source = ?8, reviewed_at = datetime('now'),
                     updated_at = datetime('now'), embedding_model = ?9
                 WHERE id = ?1",
                params![
                    id,
                    kind,
                    title,
                    body,
                    embedding.len() as i64,
                    bytes,
                    confidence,
                    source,
                    embedding_model
                ],
            )
            .map_err(sqlite)?;
        get_node_by_id(db.conn(), id)
    }

    pub fn delete_node(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("memory db poisoned");
        let n = db
            .conn()
            .execute("DELETE FROM memory_nodes WHERE id = ?1", params![id])
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    pub fn archive_node(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("memory db poisoned");
        let n = db
            .conn()
            .execute(
                "UPDATE memory_nodes
                 SET archived_at = COALESCE(archived_at, datetime('now')),
                     updated_at = datetime('now')
                 WHERE id = ?1",
                params![id],
            )
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    pub fn restore_node(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("memory db poisoned");
        let n = db
            .conn()
            .execute(
                "UPDATE memory_nodes
                 SET archived_at = NULL, reviewed_at = datetime('now'),
                     updated_at = datetime('now')
                 WHERE id = ?1 AND archived_at IS NOT NULL",
                params![id],
            )
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    pub fn review_node(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("memory db poisoned");
        let n = db
            .conn()
            .execute(
                "UPDATE memory_nodes
                 SET reviewed_at = datetime('now')
                 WHERE id = ?1 AND archived_at IS NULL",
                params![id],
            )
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    pub async fn search_graph(
        &self,
        model: &str,
        query: &str,
        scopes: &[MemoryScopeRef],
        top_k: usize,
        include_archived: bool,
    ) -> Result<Vec<MemoryGraphHit>> {
        let model = self.embedding_model_for(model);
        let model = model.as_str();
        let q = match self.embed_one(model, query).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("memory search embedding unavailable; using lexical recall: {e}");
                Vec::new()
            }
        };
        self.resume_reembed();
        let fts_query = safe_fts_query(query);
        let result_limit = top_k.clamp(1, 200);
        // Only vectors from the query's model and dimension are loaded; rows
        // from before model tracking (empty model) still count when their
        // dimension matches until the re-embed job replaces them.
        let query_model = model.to_string();
        let query_dim = if q.is_empty() { -1 } else { q.len() as i64 };
        let candidate_limit = result_limit.saturating_mul(4).clamp(20, 200);
        let scope_ids = scopes
            .iter()
            .map(normalize_scope_ref)
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .map(|scope| scope_id_for(&scope))
            .collect::<Vec<_>>();
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().expect("memory db poisoned");
            let archived_clause = if include_archived {
                ""
            } else {
                "AND n.archived_at IS NULL"
            };
            let rows: Vec<(MemoryNode, Vec<u8>)> = if scope_ids.is_empty() {
                let sql = format!(
                    "SELECT n.id, n.scope_id, s.kind, s.label, n.kind, n.title, n.body,
                            n.confidence, n.source, n.created_at, n.updated_at, n.archived_at,
                            n.reviewed_at,
                            CASE WHEN n.dim = ?2 AND n.embedding_model IN (?1, '')
                                 THEN n.embedding ELSE X'' END
                     FROM memory_nodes n
                     JOIN memory_scopes s ON s.id = n.scope_id
                     WHERE 1 = 1 {archived_clause}"
                );
                let mut stmt = db.conn().prepare(&sql).map_err(sqlite)?;
                let mapped = stmt
                    .query_map(params![query_model, query_dim], row_to_node_with_embedding)
                    .map_err(sqlite)?;
                collect_rows(mapped)?
            } else {
                let mut out = Vec::new();
                let sql = format!(
                    "SELECT n.id, n.scope_id, s.kind, s.label, n.kind, n.title, n.body,
                            n.confidence, n.source, n.created_at, n.updated_at, n.archived_at,
                            n.reviewed_at,
                            CASE WHEN n.dim = ?3 AND n.embedding_model IN (?2, '')
                                 THEN n.embedding ELSE X'' END
                     FROM memory_nodes n
                     JOIN memory_scopes s ON s.id = n.scope_id
                     WHERE n.scope_id = ?1 {archived_clause}"
                );
                let mut stmt = db.conn().prepare(&sql).map_err(sqlite)?;
                for id in scope_ids {
                    let mapped = stmt
                        .query_map(
                            params![id, query_model, query_dim],
                            row_to_node_with_embedding,
                        )
                        .map_err(sqlite)?;
                    for row in mapped {
                        out.push(row.map_err(sqlite)?);
                    }
                }
                out
            };

            let mut candidates = HashMap::new();
            let mut semantic = Vec::new();
            for (node, blob) in rows {
                if candidates.contains_key(&node.id) {
                    continue;
                }
                let embedding = bytes_to_vec(&blob);
                if !q.is_empty() && !embedding.is_empty() && q.len() == embedding.len() {
                    semantic.push((node.clone(), cosine(&q, &embedding)));
                }
                candidates.insert(node.id.clone(), node);
            }
            semantic.sort_by(|(a_node, a_score), (b_node, b_score)| {
                b_score
                    .partial_cmp(a_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b_node.updated_at.cmp(&a_node.updated_at))
                    .then_with(|| a_node.id.cmp(&b_node.id))
            });
            let semantic = semantic
                .into_iter()
                .take(candidate_limit)
                .map(|(node, _)| node)
                .collect::<Vec<_>>();

            let lexical = if let Some(fts_query) = fts_query {
                let mut stmt = db
                    .conn()
                    .prepare(
                        "SELECT node_id
                         FROM memory_nodes_fts
                         WHERE memory_nodes_fts MATCH ?1
                         ORDER BY bm25(memory_nodes_fts), node_id",
                    )
                    .map_err(sqlite)?;
                let rows = stmt
                    .query_map(params![fts_query], |row| row.get::<_, String>(0))
                    .map_err(sqlite)?;
                let mut ranked = Vec::new();
                for row in rows {
                    let id = row.map_err(sqlite)?;
                    if let Some(node) = candidates.get(&id) {
                        ranked.push(node.clone());
                        if ranked.len() == candidate_limit {
                            break;
                        }
                    }
                }
                ranked
            } else {
                Vec::new()
            };

            Ok(fuse_rankings(vec![semantic, lexical], result_limit))
        })
        .await
        .map_err(|error| Error::Other(format!("memory graph search task: {error}")))?
    }

    pub async fn benchmark(
        &self,
        model: &str,
        cases: Vec<MemoryBenchmarkCase>,
        top_k: usize,
        include_archived: bool,
    ) -> Result<MemoryBenchmarkReport> {
        if cases.is_empty() {
            return Err(Error::InvalidRequest(
                "memory benchmark requires at least one case".to_string(),
            ));
        }
        if cases.len() > MAX_MEMORY_BENCHMARK_CASES {
            return Err(Error::InvalidRequest(format!(
                "memory benchmark supports at most {MAX_MEMORY_BENCHMARK_CASES} cases"
            )));
        }
        let top_k = top_k.clamp(1, 50);
        let mut results = Vec::new();
        for (index, case) in cases.into_iter().enumerate() {
            if case.query.trim().is_empty() || case.relevant_node_ids.is_empty() {
                return Err(Error::InvalidRequest(format!(
                    "memory benchmark case {} requires a query and relevant_node_ids",
                    index + 1
                )));
            }
            let relevant_node_ids = case.relevant_node_ids.clone();
            let relevant = case
                .relevant_node_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            let hits = self
                .search_graph(model, &case.query, &case.scopes, top_k, include_archived)
                .await?;
            let retrieved_node_ids = hits
                .iter()
                .map(|hit| hit.node.id.clone())
                .collect::<Vec<_>>();
            let first_relevant_rank = retrieved_node_ids
                .iter()
                .position(|id| relevant.contains(id))
                .map(|index| index + 1);
            let retrieved_relevant = retrieved_node_ids
                .iter()
                .filter(|id| relevant.contains(*id))
                .count();
            let recall_at_k = retrieved_relevant as f32 / relevant.len() as f32;
            let reciprocal_rank = first_relevant_rank
                .map(|rank| 1.0 / rank as f32)
                .unwrap_or_default();
            results.push(MemoryBenchmarkCaseResult {
                name: if case.name.trim().is_empty() {
                    format!("Case {}", index + 1)
                } else {
                    case.name
                },
                query: case.query,
                relevant_node_ids,
                retrieved_node_ids,
                first_relevant_rank,
                recall_at_k,
                reciprocal_rank,
            });
        }
        let case_count = results.len();
        let recall_at_k =
            results.iter().map(|case| case.recall_at_k).sum::<f32>() / case_count as f32;
        let mean_reciprocal_rank =
            results.iter().map(|case| case.reciprocal_rank).sum::<f32>() / case_count as f32;
        Ok(MemoryBenchmarkReport {
            top_k,
            case_count,
            recall_at_k,
            mean_reciprocal_rank,
            cases: results,
        })
    }

    /// Return the `top_k` entries most similar to `query`.
    pub async fn search(&self, model: &str, query: &str, top_k: usize) -> Result<Vec<MemoryHit>> {
        let model = self.embedding_model_for(model);
        let model = model.as_str();
        let q = self.embed_one(model, query).await?;
        let query_model = model.to_string();
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let db = db.lock().expect("memory db poisoned");
            let conn = db.conn();
            let mut stmt = conn
                .prepare(
                    "SELECT id, text,
                            CASE WHEN dim = ?2 AND embedding_model IN (?1, '')
                                 THEN embedding ELSE X'' END
                     FROM memories",
                )
                .map_err(sqlite)?;
            let mapped = stmt
                .query_map(params![query_model, q.len() as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                    ))
                })
                .map_err(sqlite)?;
            let mut out = Vec::new();
            for row in mapped {
                out.push(row.map_err(sqlite)?);
            }
            let mut hits: Vec<MemoryHit> = out
                .into_iter()
                .map(|(id, text, blob)| {
                    let score = cosine(&q, &bytes_to_vec(&blob));
                    MemoryHit { id, text, score }
                })
                .collect();
            hits.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            hits.truncate(top_k);
            Ok(hits)
        })
        .await
        .map_err(|error| Error::Other(format!("memory search task: {error}")))?
    }

    /// Number of stored memories.
    pub fn count(&self) -> Result<usize> {
        let db = self.db.lock().expect("memory db poisoned");
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(sqlite)?;
        Ok(n as usize)
    }

    /// Embedding coverage of scoped memories and the re-embed job state.
    pub fn embedding_status(&self) -> Result<MemoryEmbeddingStatus> {
        self.resume_reembed();
        let db = self.db.lock().expect("memory db poisoned");
        let current = current_embedding_model(db.conn())?;
        let mut status = match &current {
            Some((model, dim)) => db
                .conn()
                .query_row(
                    "SELECT COUNT(*),
                            COALESCE(SUM(dim > 0 AND embedding_model = ?1 AND dim = ?2), 0),
                            COALESCE(SUM(dim > 0 AND NOT (embedding_model = ?1 AND dim = ?2)), 0),
                            COALESCE(SUM(dim = 0 AND embedding_model != ?1), 0),
                            COALESCE(SUM(dim = 0 AND embedding_model = ?1), 0)
                     FROM memory_nodes",
                    params![model, *dim as i64],
                    |r| {
                        Ok(MemoryEmbeddingStatus {
                            total: r.get::<_, i64>(0)? as usize,
                            current: r.get::<_, i64>(1)? as usize,
                            stale: r.get::<_, i64>(2)? as usize,
                            missing: r.get::<_, i64>(3)? as usize,
                            unavailable: r.get::<_, i64>(4)? as usize,
                            ..MemoryEmbeddingStatus::default()
                        })
                    },
                )
                .map_err(sqlite)?,
            None => MemoryEmbeddingStatus {
                total: db
                    .conn()
                    .query_row("SELECT COUNT(*) FROM memory_nodes", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(sqlite)? as usize,
                ..MemoryEmbeddingStatus::default()
            },
        };
        if let Some((model, dim)) = current {
            status.model = Some(model);
            status.dim = dim;
        }
        status.configured_model = self.configured_embedding_model();
        let progress = self.index.progress.lock().expect("memory index poisoned");
        status.reindexing = progress.running;
        status.reindexed = progress.processed;
        status.last_error = progress.last_error.clone();
        Ok(status)
    }

    /// Start (or keep running) the background job that re-embeds stale and
    /// missing memories with the current model. Progress lives in the rows
    /// themselves, so a cancelled or interrupted job resumes where it
    /// stopped. Returns `false` outside a Tokio runtime.
    pub fn start_reembed(&self) -> bool {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return false;
        };
        self.index.cancel.store(false, Ordering::SeqCst);
        {
            let mut progress = self.index.progress.lock().expect("memory index poisoned");
            progress.requested += 1;
            if progress.running {
                return true;
            }
            progress.running = true;
            progress.processed = 0;
            progress.last_error = None;
        }
        handle.spawn(run_reembed(self.db.clone(), self.index.clone()));
        true
    }

    /// Stop the re-embed job after its current batch.
    pub fn cancel_reembed(&self) {
        self.index.cancel.store(true, Ordering::SeqCst);
    }

    /// Resume pending re-embedding once per process, on the first store
    /// access that runs inside a Tokio runtime.
    fn resume_reembed(&self) {
        if self.index.resumed.load(Ordering::SeqCst)
            || tokio::runtime::Handle::try_current().is_err()
        {
            return;
        }
        if !self.index.resumed.swap(true, Ordering::SeqCst) {
            self.start_reembed();
        }
    }

    async fn embed_one(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let mut vecs = self.embedder.embed(model, vec![text.to_string()]).await?;
        let vector = vecs
            .pop()
            .ok_or_else(|| Error::Other("embedder returned no vector".to_string()))?;
        if !vector.is_empty() {
            self.note_embedding_model(model, vector.len())?;
        }
        Ok(vector)
    }

    /// The pinned embedding model, if the user chose one.
    pub fn configured_embedding_model(&self) -> Option<String> {
        self.index
            .configured
            .lock()
            .expect("memory index poisoned")
            .clone()
    }

    /// Pin the model every memory embed uses, or `None` to follow the models
    /// chats embed with. Pinning a model other than the current one starts
    /// re-embedding with it.
    pub fn set_configured_embedding_model(&self, model: Option<&str>) -> Result<()> {
        let model = model.map(str::trim).filter(|model| !model.is_empty());
        let changed = {
            let db = self.db.lock().expect("memory db poisoned");
            match model {
                Some(model) => write_meta(db.conn(), META_CONFIGURED_MODEL, model)?,
                None => {
                    db.conn()
                        .execute(
                            "DELETE FROM memory_meta WHERE key = ?1",
                            params![META_CONFIGURED_MODEL],
                        )
                        .map_err(sqlite)?;
                }
            }
            *self.index.candidate.lock().expect("memory index poisoned") = None;
            *self.index.configured.lock().expect("memory index poisoned") =
                model.map(str::to_string);
            model.is_some_and(|model| {
                current_embedding_model(db.conn())
                    .ok()
                    .flatten()
                    .is_none_or(|(current, _)| current != model)
            })
        };
        if changed {
            self.start_reembed();
        }
        Ok(())
    }

    /// The model to embed with: the pinned model when there is one,
    /// otherwise the model the caller asked for.
    fn embedding_model_for(&self, requested: &str) -> String {
        self.configured_embedding_model()
            .unwrap_or_else(|| requested.to_string())
    }

    /// Record the model that just produced a vector. A pinned model is
    /// current as soon as it returns one. Otherwise a different model must
    /// return vectors twice in a row, so alternating between two chat models
    /// does not re-embed the library on every switch. A new current model or
    /// dimension makes existing vectors stale and starts re-embedding.
    fn note_embedding_model(&self, model: &str, dim: usize) -> Result<()> {
        let changed = {
            let db = self.db.lock().expect("memory db poisoned");
            let current = current_embedding_model(db.conn())?;
            let pinned = self.configured_embedding_model().as_deref() == Some(model);
            let mut candidate = self.index.candidate.lock().expect("memory index poisoned");
            let observed = (model.to_string(), dim);
            let adopt = match &current {
                None => true,
                Some(current) if *current == observed => {
                    *candidate = None;
                    false
                }
                Some(_) if pinned || candidate.as_ref() == Some(&observed) => true,
                Some(_) => {
                    *candidate = Some(observed);
                    false
                }
            };
            if adopt {
                *candidate = None;
                set_current_embedding_model(db.conn(), model, dim)?
            } else {
                false
            }
        };
        if changed {
            tracing::info!("memory embedding model is now {model} ({dim} dimensions)");
            self.start_reembed();
        }
        Ok(())
    }
}

fn read_meta(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM memory_meta WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(sqlite)
}

fn write_meta(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(sqlite)?;
    Ok(())
}

/// The current embedding model and its dimension.
fn current_embedding_model(conn: &rusqlite::Connection) -> Result<Option<(String, usize)>> {
    let (Some(model), Some(dim)) = (
        read_meta(conn, META_EMBEDDING_MODEL)?,
        read_meta(conn, META_EMBEDDING_DIM)?,
    ) else {
        return Ok(None);
    };
    Ok(dim
        .parse::<usize>()
        .ok()
        .filter(|dim| *dim > 0)
        .map(|dim| (model, dim)))
}

/// Persist the current model; returns whether it changed.
fn set_current_embedding_model(
    conn: &rusqlite::Connection,
    model: &str,
    dim: usize,
) -> Result<bool> {
    if current_embedding_model(conn)?.is_some_and(|(m, d)| m == model && d == dim) {
        return Ok(false);
    }
    write_meta(conn, META_EMBEDDING_MODEL, model)?;
    write_meta(conn, META_EMBEDDING_DIM, &dim.to_string())?;
    Ok(true)
}

struct ReembedRow {
    id: String,
    title: String,
    body: String,
}

/// Background loop: re-embed pending rows in batches until none remain,
/// the job is cancelled, or the embedder cannot produce any vector.
async fn run_reembed(db: Arc<Mutex<Database>>, index: Arc<EmbeddingIndex>) {
    let mut seen = index
        .progress
        .lock()
        .expect("memory index poisoned")
        .requested;
    loop {
        let step = if index.cancel.load(Ordering::SeqCst) {
            Ok(0)
        } else {
            reembed_batch(&db, &index).await
        };
        let mut progress = index.progress.lock().expect("memory index poisoned");
        match step {
            Ok(n) if n > 0 => {
                progress.processed += n;
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("memory re-embedding paused: {error}");
                progress.last_error = Some(error.to_string());
                progress.running = false;
                return;
            }
        }
        // A model change or explicit start during the last batch may have
        // produced new pending rows; look again before stopping.
        if progress.requested != seen && !index.cancel.load(Ordering::SeqCst) {
            seen = progress.requested;
            continue;
        }
        progress.running = false;
        return;
    }
}

/// Re-embed one batch. Returns the number of rows processed (0 when none
/// are pending). Rows the embedder rejects individually are marked as
/// attempted by the current model so they are not retried in a loop.
async fn reembed_batch(db: &Arc<Mutex<Database>>, index: &EmbeddingIndex) -> Result<usize> {
    let configured = index
        .configured
        .lock()
        .expect("memory index poisoned")
        .clone();
    let (model, dim, rows) = {
        let db = db.lock().expect("memory db poisoned");
        let current = current_embedding_model(db.conn())?;
        // A pinned model is the target even before it has returned a vector;
        // its dimension is learned from the first batch.
        let (model, dim) = match (configured, current) {
            (Some(pinned), Some((current, dim))) if pinned == current => (pinned, dim),
            (Some(pinned), _) => (pinned, 0),
            (None, Some(current)) => current,
            (None, None) => return Ok(0),
        };
        let mut stmt = db
            .conn()
            .prepare(&format!(
                "SELECT id, title, body FROM memory_nodes
                 WHERE {REEMBED_PENDING_SQL}
                 ORDER BY archived_at IS NOT NULL, updated_at DESC, id
                 LIMIT ?3"
            ))
            .map_err(sqlite)?;
        let mapped = stmt
            .query_map(params![model, dim as i64, REEMBED_BATCH as i64], |r| {
                Ok(ReembedRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                })
            })
            .map_err(sqlite)?;
        let rows = collect_rows(mapped)?;
        (model, dim, rows)
    };
    if rows.is_empty() {
        return Ok(0);
    }
    let texts = rows
        .iter()
        .map(|row| memory_text(&row.title, &row.body))
        .collect::<Vec<_>>();
    let vectors =
        match index.embedder.embed(&model, texts.clone()).await {
            Ok(vectors) if vectors.len() == rows.len() => vectors.into_iter().map(Ok).collect(),
            batch => {
                // Retry one by one so a single rejected entry (for example one
                // blocked by the privacy gate) cannot stall the whole index.
                let batch_error = batch.err();
                let mut vectors = Vec::with_capacity(rows.len());
                for text in texts {
                    vectors.push(index.embedder.embed(&model, vec![text]).await.and_then(
                        |mut v| {
                            v.pop().ok_or_else(|| {
                                Error::Other("embedder returned no vector".to_string())
                            })
                        },
                    ));
                }
                if vectors.iter().all(Result::is_err) {
                    return Err(batch_error
                        .or_else(|| vectors.into_iter().find_map(Result::err))
                        .unwrap_or_else(|| {
                            Error::Other("embedder returned no vector".to_string())
                        }));
                }
                vectors
            }
        };
    let db = db.lock().expect("memory db poisoned");
    // The embedder may report a new dimension for the same model; adopt it
    // so rows written below are current instead of stale again.
    if let Some(Ok(first)) = vectors.iter().find(|v| matches!(v, Ok(v) if !v.is_empty())) {
        if first.len() != dim {
            set_current_embedding_model(db.conn(), &model, first.len())?;
        }
    }
    for (row, vector) in rows.iter().zip(vectors) {
        let vector = vector.unwrap_or_default();
        // Skip rows edited while this batch was embedding; the edit already
        // stored its own vector, or left the row pending for the next batch.
        db.conn()
            .execute(
                "UPDATE memory_nodes
                 SET dim = ?2, embedding = ?3, embedding_model = ?4
                 WHERE id = ?1 AND title = ?5 AND body = ?6",
                params![
                    row.id,
                    vector.len() as i64,
                    vec_to_bytes(&vector),
                    model,
                    row.title,
                    row.body
                ],
            )
            .map_err(sqlite)?;
    }
    Ok(rows.len())
}

fn default_node_kind() -> String {
    "fact".to_string()
}

fn default_confidence() -> f32 {
    1.0
}

#[derive(Debug, Clone)]
struct NormalizedScope {
    id: String,
    kind: String,
    label: String,
    locator: String,
    locator_hash: String,
}

fn normalize_scope(scope: MemoryScopeInput) -> Result<NormalizedScope> {
    let kind = clean_scope_kind(&scope.kind)?;
    let locator = scope.locator.trim().to_string();
    if locator.is_empty() {
        return Err(Error::InvalidRequest(
            "memory scope locator is required".to_string(),
        ));
    }
    let label = clean_default(&scope.label, &locator);
    let locator_hash = stable_hash_hex(&format!("{kind}:{locator}"));
    let id = format!("memscope-{kind}-{locator_hash}");
    Ok(NormalizedScope {
        id,
        kind,
        label,
        locator,
        locator_hash,
    })
}

fn normalize_scope_ref(scope: &MemoryScopeRef) -> Result<NormalizedScope> {
    normalize_scope(MemoryScopeInput {
        kind: scope.kind.clone(),
        label: scope.locator.clone(),
        locator: scope.locator.clone(),
    })
}

fn scope_id_for(scope: &NormalizedScope) -> String {
    scope.id.clone()
}

fn clean_scope_kind(kind: &str) -> Result<String> {
    let kind = kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "thread" | "project" | "global" => Ok(kind),
        _ => Err(Error::InvalidRequest(
            "memory scope kind must be thread, project, or global".to_string(),
        )),
    }
}

fn normalize_node(node: MemoryNodeInput) -> Result<MemoryNodeInput> {
    let title = node.title.trim().to_string();
    let body = node.body.trim().to_string();
    if title.is_empty() && body.is_empty() {
        return Err(Error::InvalidRequest(
            "memory title or body is required".to_string(),
        ));
    }
    Ok(MemoryNodeInput {
        kind: clean_node_kind(&node.kind),
        title: if title.is_empty() {
            first_line(&body)
        } else {
            title
        },
        body,
        confidence: clamp_confidence(node.confidence),
        source: node.source.trim().to_string(),
    })
}

fn clean_node_kind(kind: &str) -> String {
    let kind = kind.trim().to_ascii_lowercase();
    if kind.is_empty() {
        "fact".to_string()
    } else {
        kind.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
}

fn clean_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.trim().to_string()
    } else {
        value.to_string()
    }
}

fn first_line(value: &str) -> String {
    value
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
        .unwrap_or_else(|| "Memory".to_string())
}

fn clamp_confidence(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn memory_text(title: &str, body: &str) -> String {
    if body.trim().is_empty() {
        title.trim().to_string()
    } else {
        format!("{}\n{}", title.trim(), body.trim())
    }
}

fn stable_hash_hex(input: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

fn upsert_scope(conn: &rusqlite::Connection, scope: &NormalizedScope) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_scopes (id, kind, label, locator, locator_hash)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(kind, locator_hash) DO UPDATE SET
           label = excluded.label,
           locator = excluded.locator,
           updated_at = datetime('now')",
        params![
            scope.id,
            scope.kind,
            scope.label,
            scope.locator,
            scope.locator_hash
        ],
    )
    .map_err(sqlite)?;
    Ok(())
}

fn get_scope_by_id(conn: &rusqlite::Connection, id: &str) -> Result<MemoryScope> {
    conn.query_row(
        "SELECT id, kind, label, locator, locator_hash, created_at, updated_at
         FROM memory_scopes WHERE id = ?1",
        params![id],
        row_to_scope,
    )
    .map_err(sqlite)
}

fn get_node_by_id(conn: &rusqlite::Connection, id: &str) -> Result<Option<MemoryNode>> {
    conn.query_row(
        "SELECT n.id, n.scope_id, s.kind, s.label, n.kind, n.title, n.body,
                n.confidence, n.source, n.created_at, n.updated_at, n.archived_at,
                n.reviewed_at
         FROM memory_nodes n
         JOIN memory_scopes s ON s.id = n.scope_id
         WHERE n.id = ?1",
        params![id],
        row_to_node,
    )
    .optional()
    .map_err(sqlite)
}

fn get_event_by_id(conn: &rusqlite::Connection, id: &str) -> Result<Option<MemoryEvent>> {
    conn.query_row(
        "SELECT id, scope_id, node_id, action, thread_id, message_id, summary, created_at
         FROM memory_events WHERE id = ?1",
        params![id],
        row_to_event,
    )
    .optional()
    .map_err(sqlite)
}

fn row_to_scope(r: &rusqlite::Row) -> rusqlite::Result<MemoryScope> {
    Ok(MemoryScope {
        id: r.get(0)?,
        kind: r.get(1)?,
        label: r.get(2)?,
        locator: r.get(3)?,
        locator_hash: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
    })
}

fn row_to_node(r: &rusqlite::Row) -> rusqlite::Result<MemoryNode> {
    Ok(MemoryNode {
        id: r.get(0)?,
        scope_id: r.get(1)?,
        scope_kind: r.get(2)?,
        scope_label: r.get(3)?,
        kind: r.get(4)?,
        title: r.get(5)?,
        body: r.get(6)?,
        confidence: r.get::<_, f64>(7)? as f32,
        source: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
        archived_at: r.get(11)?,
        reviewed_at: r.get(12)?,
    })
}

fn row_to_node_with_embedding(r: &rusqlite::Row) -> rusqlite::Result<(MemoryNode, Vec<u8>)> {
    Ok((row_to_node(r)?, r.get(13)?))
}

fn row_to_edge(r: &rusqlite::Row) -> rusqlite::Result<MemoryEdge> {
    Ok(MemoryEdge {
        id: r.get(0)?,
        scope_id: r.get(1)?,
        from_node_id: r.get(2)?,
        predicate: r.get(3)?,
        to_node_id: r.get(4)?,
        confidence: r.get::<_, f64>(5)? as f32,
        created_at: r.get(6)?,
    })
}

fn row_to_event(r: &rusqlite::Row) -> rusqlite::Result<MemoryEvent> {
    Ok(MemoryEvent {
        id: r.get(0)?,
        scope_id: r.get(1)?,
        node_id: r.get(2)?,
        action: r.get(3)?,
        thread_id: r.get(4)?,
        message_id: r.get(5)?,
        summary: r.get(6)?,
        created_at: r.get(7)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(sqlite)?);
    }
    Ok(out)
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_vec(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

fn safe_fts_query(query: &str) -> Option<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            token.push(ch);
        } else if !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
            if tokens.len() == 16 {
                break;
            }
        }
    }
    if tokens.len() < 16 && !token.is_empty() {
        tokens.push(token);
    }
    (!tokens.is_empty()).then(|| {
        tokens
            .into_iter()
            .map(|token| format!("\"{token}\""))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}

fn fuse_rankings(rankings: Vec<Vec<MemoryNode>>, top_k: usize) -> Vec<MemoryGraphHit> {
    const RRF_K: f32 = 60.0;

    let rankings = rankings
        .into_iter()
        .filter(|ranking| !ranking.is_empty())
        .collect::<Vec<_>>();
    if rankings.is_empty() {
        return Vec::new();
    }

    let max_score = rankings.len() as f32 / (RRF_K + 1.0);
    let mut fused: HashMap<String, MemoryGraphHit> = HashMap::new();
    for ranking in rankings {
        for (index, node) in ranking.into_iter().enumerate() {
            let contribution = 1.0 / (RRF_K + index as f32 + 1.0);
            fused
                .entry(node.id.clone())
                .and_modify(|hit| hit.score += contribution)
                .or_insert(MemoryGraphHit {
                    node,
                    score: contribution,
                });
        }
    }

    let mut hits = fused
        .into_values()
        .map(|mut hit| {
            hit.score = (hit.score / max_score).clamp(0.0, 1.0);
            hit
        })
        .collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.node.updated_at.cmp(&a.node.updated_at))
            .then_with(|| a.node.id.cmp(&b.node.id))
    });
    hits.truncate(top_k);
    hits
}

/// Cosine similarity; 0.0 if either vector is zero or lengths differ.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn store() -> MemoryStore {
        let db = Database::open_in_memory().unwrap();
        let embedder: SharedService = Arc::new(milim_inference::test_backend::TestBackend::new());
        MemoryStore::new(db, embedder).unwrap()
    }

    fn lexical_store() -> MemoryStore {
        let db = Database::open_in_memory().unwrap();
        let embedder: SharedService =
            Arc::new(milim_inference::unavailable::UnavailableBackend::new());
        MemoryStore::new(db, embedder).unwrap()
    }

    fn test_node(id: &str, updated_at: &str) -> MemoryNode {
        MemoryNode {
            id: id.into(),
            scope_id: "scope".into(),
            scope_kind: "project".into(),
            scope_label: "Project".into(),
            kind: "fact".into(),
            title: id.into(),
            body: String::new(),
            confidence: 1.0,
            source: "test".into(),
            created_at: updated_at.into(),
            updated_at: updated_at.into(),
            archived_at: None,
            reviewed_at: None,
        }
    }

    #[tokio::test]
    async fn add_then_search_ranks_exact_match_first() {
        let mem = store();
        mem.add("m", "alpha").await.unwrap();
        mem.add("m", "beta").await.unwrap();
        mem.add("m", "gamma").await.unwrap();
        assert_eq!(mem.count().unwrap(), 3);

        let hits = mem.search("m", "beta", 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        // Identical text -> identical test backend embedding -> cosine 1.0 -> ranked first.
        assert_eq!(hits[0].text, "beta");
        assert!(hits[0].score >= hits[1].score);
        assert!(hits[1].score >= hits[2].score);
        assert!((hits[0].score - 1.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn top_k_limits_results() {
        let mem = store();
        for t in ["one", "two", "three", "four"] {
            mem.add("m", t).await.unwrap();
        }
        let hits = mem.search("m", "three", 2).await.unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[tokio::test]
    async fn graph_memories_are_scoped_and_searchable() {
        let mem = store();
        let thread = MemoryScopeInput {
            kind: "thread".into(),
            label: "Planning chat".into(),
            locator: "thread-1".into(),
        };
        let project = MemoryScopeInput {
            kind: "project".into(),
            label: "milim".into(),
            locator: "C:\\dev\\milim".into(),
        };

        let t = mem
            .register(
                "m",
                thread.clone(),
                MemoryNodeInput {
                    kind: "decision".into(),
                    title: "Thread decision".into(),
                    body: "Use explicit memory breadcrumbs in chat.".into(),
                    confidence: 0.9,
                    source: "test".into(),
                },
                Vec::new(),
                MemoryEventInput {
                    thread_id: "thread-1".into(),
                    summary: "Remembered breadcrumb decision".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(t.notice.scope_kind, "thread");
        assert_eq!(t.notice.summary, "Remembered breadcrumb decision");

        mem.register(
            "m",
            project.clone(),
            MemoryNodeInput {
                kind: "fact".into(),
                title: "Project storage".into(),
                body: "Graph memory is stored in SQLite.".into(),
                confidence: 1.0,
                source: "test".into(),
            },
            Vec::new(),
            MemoryEventInput::default(),
        )
        .await
        .unwrap();

        let scopes = mem.list_scopes().unwrap();
        assert_eq!(scopes.len(), 2);

        let thread_nodes = mem
            .list_nodes(
                Some(MemoryScopeRef {
                    kind: "thread".into(),
                    locator: "thread-1".into(),
                }),
                false,
                10,
            )
            .unwrap();
        assert_eq!(thread_nodes.len(), 1);
        assert_eq!(thread_nodes[0].title, "Thread decision");

        let hits = mem
            .search_graph(
                "m",
                "SQLite storage",
                &[MemoryScopeRef {
                    kind: "project".into(),
                    locator: "C:\\dev\\milim".into(),
                }],
                5,
                false,
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node.scope_kind, "project");
        assert_eq!(hits[0].node.title, "Project storage");
    }

    #[tokio::test]
    async fn graph_memory_update_and_delete() {
        let mem = store();
        let reg = mem
            .register(
                "m",
                MemoryScopeInput {
                    kind: "thread".into(),
                    label: "Chat".into(),
                    locator: "thread-2".into(),
                },
                MemoryNodeInput {
                    kind: "fact".into(),
                    title: "Old".into(),
                    body: "Old body".into(),
                    confidence: 1.0,
                    source: String::new(),
                },
                Vec::new(),
                MemoryEventInput::default(),
            )
            .await
            .unwrap();

        let updated = mem
            .update_node(
                "m",
                &reg.node.id,
                MemoryNodeUpdate {
                    title: Some("New".into()),
                    body: Some("New body".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.title, "New");
        assert_eq!(updated.body, "New body");
        assert!(updated.reviewed_at.is_some());

        assert!(mem.archive_node(&reg.node.id).unwrap());
        assert!(mem
            .list_nodes(
                Some(MemoryScopeRef {
                    kind: "thread".into(),
                    locator: "thread-2".into()
                }),
                false,
                10
            )
            .unwrap()
            .is_empty());
        assert!(mem.restore_node(&reg.node.id).unwrap());
        let restored = mem
            .list_nodes(
                Some(MemoryScopeRef {
                    kind: "thread".into(),
                    locator: "thread-2".into(),
                }),
                false,
                10,
            )
            .unwrap();
        assert_eq!(restored.len(), 1);
        assert!(restored[0].reviewed_at.is_some());
        assert!(mem.delete_node(&reg.node.id).unwrap());
    }

    #[tokio::test]
    async fn memory_benchmark_reports_recall_and_reciprocal_rank() {
        let mem = lexical_store();
        let scope = MemoryScopeInput {
            kind: "project".into(),
            label: "milim".into(),
            locator: "project-a".into(),
        };
        let relevant = mem
            .register(
                "m",
                scope.clone(),
                MemoryNodeInput {
                    kind: "decision".into(),
                    title: "Preview manifest".into(),
                    body: "Use the exact token PREVIEW-4173 for repository launch metadata.".into(),
                    confidence: 1.0,
                    source: "user".into(),
                },
                Vec::new(),
                MemoryEventInput::default(),
            )
            .await
            .unwrap();
        mem.register(
            "m",
            scope.clone(),
            MemoryNodeInput {
                kind: "fact".into(),
                title: "Unrelated".into(),
                body: "Desktop colors and spacing.".into(),
                confidence: 1.0,
                source: "test".into(),
            },
            Vec::new(),
            MemoryEventInput::default(),
        )
        .await
        .unwrap();

        let report = mem
            .benchmark(
                "m",
                vec![MemoryBenchmarkCase {
                    name: "Exact preview token".into(),
                    query: "PREVIEW-4173".into(),
                    relevant_node_ids: vec![relevant.node.id],
                    scopes: vec![MemoryScopeRef {
                        kind: scope.kind,
                        locator: scope.locator,
                    }],
                }],
                5,
                false,
            )
            .await
            .unwrap();

        assert_eq!(report.case_count, 1);
        assert_eq!(report.cases[0].first_relevant_rank, Some(1));
        assert_eq!(report.recall_at_k, 1.0);
        assert_eq!(report.mean_reciprocal_rank, 1.0);
    }

    #[tokio::test]
    async fn memory_benchmark_rejects_unbounded_case_counts() {
        let case = MemoryBenchmarkCase {
            name: "Bounded case".into(),
            query: "bounded query".into(),
            relevant_node_ids: vec!["node-1".into()],
            scopes: Vec::new(),
        };
        let error = lexical_store()
            .benchmark("m", vec![case; MAX_MEMORY_BENCHMARK_CASES + 1], 5, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("supports at most 100 cases"));
    }

    #[tokio::test]
    async fn fts_migration_backfills_existing_nodes() {
        let db = Database::open_in_memory().unwrap();
        db.migrate(&MEMORY_MIGRATIONS[..2]).unwrap();
        db.conn()
            .execute(
                "INSERT INTO memory_scopes (id, kind, label, locator, locator_hash)
                 VALUES ('scope', 'project', 'Project', 'project', 'hash')",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO memory_nodes
                 (id, scope_id, kind, title, body, dim, embedding)
                 VALUES ('legacy', 'scope', 'fact', 'Legacy MILIM-4279', 'Backfilled body', 0, X'')",
                [],
            )
            .unwrap();

        let embedder: SharedService =
            Arc::new(milim_inference::unavailable::UnavailableBackend::new());
        let mem = MemoryStore::new(db, embedder).unwrap();
        let hits = mem
            .search_graph("m", "MILIM-4279", &[], 5, false)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node.id, "legacy");
    }

    #[tokio::test]
    async fn lexical_recall_survives_unavailable_embeddings_and_fts_punctuation() {
        let mem = lexical_store();
        let scope = MemoryScopeInput {
            kind: "project".into(),
            label: "milim".into(),
            locator: "C:\\dev\\milim".into(),
        };
        for (title, body) in [
            ("Release identifier", "Track exact ticket MILIM-4279"),
            ("Unrelated recent note", "Polish the settings panel"),
        ] {
            mem.register(
                "m",
                scope.clone(),
                MemoryNodeInput {
                    kind: "fact".into(),
                    title: title.into(),
                    body: body.into(),
                    confidence: 1.0,
                    source: "test".into(),
                },
                Vec::new(),
                MemoryEventInput::default(),
            )
            .await
            .unwrap();
        }

        let hits = mem
            .search_graph(
                "m",
                "MILIM-4279 \" OR * ( )",
                &[MemoryScopeRef {
                    kind: "project".into(),
                    locator: "C:\\dev\\milim".into(),
                }],
                5,
                false,
            )
            .await
            .unwrap();
        assert_eq!(hits[0].node.title, "Release identifier");
        assert!((0.0..=1.0).contains(&hits[0].score));

        let no_hits = mem.search_graph("m", "!!!", &[], 5, false).await.unwrap();
        assert!(no_hits.is_empty());
    }

    #[tokio::test]
    async fn hybrid_search_keeps_scope_archive_and_top_k_filters() {
        let mem = lexical_store();
        let project_a = MemoryScopeInput {
            kind: "project".into(),
            label: "A".into(),
            locator: "project-a".into(),
        };
        let project_b = MemoryScopeInput {
            kind: "project".into(),
            label: "B".into(),
            locator: "project-b".into(),
        };
        let mut ids = Vec::new();
        for (scope, title) in [
            (project_a.clone(), "Active TOKEN-42"),
            (project_a.clone(), "Archived TOKEN-42"),
            (project_b, "Other scope TOKEN-42"),
        ] {
            let registration = mem
                .register(
                    "m",
                    scope,
                    MemoryNodeInput {
                        kind: "fact".into(),
                        title: title.into(),
                        body: "Exact scoped identifier".into(),
                        confidence: 1.0,
                        source: "test".into(),
                    },
                    Vec::new(),
                    MemoryEventInput::default(),
                )
                .await
                .unwrap();
            ids.push(registration.node.id);
        }
        assert!(mem.archive_node(&ids[1]).unwrap());

        let scope = [MemoryScopeRef {
            kind: "project".into(),
            locator: "project-a".into(),
        }];
        let active = mem
            .search_graph("m", "TOKEN-42", &scope, 1, false)
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].node.title, "Active TOKEN-42");

        let with_archived = mem
            .search_graph("m", "TOKEN-42", &scope, 2, true)
            .await
            .unwrap();
        assert_eq!(with_archived.len(), 2);
        assert!(with_archived.iter().all(|hit| hit.node.scope_label == "A"));
    }

    /// Embeds with a dimension taken from the model name's trailing digits
    /// (`m3` -> 3), refuses any request containing "reject", and fails every
    /// request for the model `down`.
    struct FakeEmbedder {
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl milim_inference::ModelService for FakeEmbedder {
        fn name(&self) -> &str {
            "fake"
        }

        async fn list_models(&self) -> Result<Vec<milim_core::api::openai::Model>> {
            Ok(Vec::new())
        }

        async fn stream(
            &self,
            _req: milim_inference::CompletionRequest,
        ) -> Result<milim_inference::EventStream> {
            Err(Error::Other("fake embedder cannot generate".into()))
        }

        async fn embed(&self, model: &str, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            let dim = model
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .parse::<usize>()
                .unwrap_or(0);
            if dim == 0 || inputs.iter().any(|input| input.contains("reject")) {
                return Err(Error::Upstream(format!("{model} cannot embed this")));
            }
            Ok(inputs
                .iter()
                .map(|input| {
                    let mut v = vec![0.01f32; dim];
                    for (i, b) in input.bytes().enumerate() {
                        v[i % dim] += b as f32;
                    }
                    v
                })
                .collect())
        }
    }

    fn fake_store(delay_ms: u64) -> MemoryStore {
        let embedder: SharedService = Arc::new(FakeEmbedder {
            delay: std::time::Duration::from_millis(delay_ms),
        });
        MemoryStore::new(Database::open_in_memory().unwrap(), embedder).unwrap()
    }

    async fn register_fact(mem: &MemoryStore, model: &str, title: &str) -> String {
        mem.register(
            model,
            MemoryScopeInput {
                kind: "project".into(),
                label: "P".into(),
                locator: "p".into(),
            },
            MemoryNodeInput {
                kind: "fact".into(),
                title: title.into(),
                body: String::new(),
                confidence: 1.0,
                source: "test".into(),
            },
            Vec::new(),
            MemoryEventInput::default(),
        )
        .await
        .unwrap()
        .node
        .id
    }

    async fn wait_for_reembed(mem: &MemoryStore) -> MemoryEmbeddingStatus {
        for _ in 0..500 {
            let status = mem.embedding_status().unwrap();
            if !status.reindexing {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("memory re-embedding did not finish");
    }

    fn node_embedding(mem: &MemoryStore, id: &str) -> (String, i64) {
        let db = mem.db.lock().unwrap();
        db.conn()
            .query_row(
                "SELECT embedding_model, dim FROM memory_nodes WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    #[test]
    fn embedding_status_counts_stale_missing_and_unavailable_vectors() {
        let db = Database::open_in_memory().unwrap();
        db.migrate(MEMORY_MIGRATIONS).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO memory_scopes (id, kind, label, locator, locator_hash)
                 VALUES ('scope', 'project', 'Project', 'project', 'hash');
                 INSERT INTO memory_nodes (id, scope_id, title, body, dim, embedding, embedding_model)
                 VALUES ('current', 'scope', 'a', '', 4, X'00000000000000000000000000000000', 'm4'),
                        ('old-model', 'scope', 'b', '', 3, X'000000000000000000000000', 'm3'),
                        ('legacy', 'scope', 'c', '', 4, X'00000000000000000000000000000000', ''),
                        ('missing', 'scope', 'd', '', 0, X'', ''),
                        ('unavailable', 'scope', 'e', '', 0, X'', 'm4');
                 INSERT INTO memory_meta (key, value)
                 VALUES ('embedding_model', 'm4'), ('embedding_dim', '4');",
            )
            .unwrap();
        // No Tokio runtime here, so the store cannot start re-embedding.
        let mem = MemoryStore::new(
            db,
            Arc::new(FakeEmbedder {
                delay: std::time::Duration::ZERO,
            }),
        )
        .unwrap();
        let status = mem.embedding_status().unwrap();
        assert_eq!(status.model.as_deref(), Some("m4"));
        assert_eq!(status.dim, 4);
        assert_eq!(status.total, 5);
        assert_eq!(status.current, 1);
        assert_eq!(status.stale, 2);
        assert_eq!(status.missing, 1);
        assert_eq!(status.unavailable, 1);
        assert!(!status.reindexing);
        assert!(!mem.start_reembed());
    }

    #[tokio::test]
    async fn model_change_reembeds_stale_and_missing_memories() {
        let mem = fake_store(0);
        let alpha = register_fact(&mem, "m3", "alpha project").await;
        let beta = register_fact(&mem, "m3", "beta reject").await;
        let missing = register_fact(&mem, "down", "gamma notes").await;
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.model.as_deref(), Some("m3"));
        // The job embedded the entry the failing model left without a vector
        // and marked the entry m3 rejects as attempted.
        assert_eq!(node_embedding(&mem, &missing), ("m3".into(), 3));
        assert_eq!(node_embedding(&mem, &beta), ("m3".into(), 0));
        assert_eq!(status.current, 2);
        assert_eq!(status.missing, 0);
        assert_eq!(status.unavailable, 1);

        // One search with another model does not switch the index; a second
        // in a row does, and re-embeds everything.
        mem.search_graph("m5", "alpha project", &[], 3, false)
            .await
            .unwrap();
        assert_eq!(wait_for_reembed(&mem).await.model.as_deref(), Some("m3"));
        mem.search_graph("m5", "alpha project", &[], 3, false)
            .await
            .unwrap();
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.model.as_deref(), Some("m5"));
        assert_eq!(status.dim, 5);
        assert_eq!(status.stale, 0);
        assert_eq!(status.missing, 0);
        assert_eq!(status.unavailable, 1);
        assert_eq!(status.current, 2);
        assert!(status.last_error.is_none());
        assert_eq!(node_embedding(&mem, &alpha), ("m5".into(), 5));
        assert_eq!(node_embedding(&mem, &beta), ("m5".into(), 0));

        let hits = mem
            .search_graph("m5", "gamma notes", &[], 1, false)
            .await
            .unwrap();
        assert_eq!(hits[0].node.id, missing);
    }

    #[tokio::test]
    async fn alternating_chat_models_do_not_reembed_on_every_switch() {
        let mem = fake_store(0);
        let alpha = register_fact(&mem, "m3", "alpha project").await;
        wait_for_reembed(&mem).await;
        for model in ["m5", "m3", "m5", "m3", "m5"] {
            mem.search_graph(model, "alpha", &[], 1, false)
                .await
                .unwrap();
        }
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.model.as_deref(), Some("m3"));
        assert_eq!(node_embedding(&mem, &alpha), ("m3".into(), 3));
        assert_eq!(status.reindexed, 0, "no switch means no re-embedding");
    }

    #[tokio::test]
    async fn a_pinned_model_is_current_at_once_and_embeds_every_request() {
        let mem = fake_store(0);
        let alpha = register_fact(&mem, "m3", "alpha project").await;
        wait_for_reembed(&mem).await;

        mem.set_configured_embedding_model(Some("m4")).unwrap();
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.configured_model.as_deref(), Some("m4"));
        assert_eq!(status.model.as_deref(), Some("m4"));
        assert_eq!(status.current, 1);
        assert_eq!(node_embedding(&mem, &alpha), ("m4".into(), 4));

        // Chats that ask for other models still embed with the pinned one.
        let beta = register_fact(&mem, "m5", "beta notes").await;
        assert_eq!(node_embedding(&mem, &beta), ("m4".into(), 4));
        let hits = mem
            .search_graph("m6", "beta notes", &[], 1, false)
            .await
            .unwrap();
        assert_eq!(hits[0].node.id, beta);
        assert_eq!(wait_for_reembed(&mem).await.model.as_deref(), Some("m4"));

        // Unpinning keeps the current model until another one wins twice.
        mem.set_configured_embedding_model(None).unwrap();
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.configured_model, None);
        assert_eq!(status.model.as_deref(), Some("m4"));
    }

    #[tokio::test]
    async fn the_pinned_model_survives_reopening_the_store() {
        let path =
            std::env::temp_dir().join(format!("milim-memory-pin-{}.db", uuid::Uuid::new_v4()));
        let embedder: SharedService = Arc::new(FakeEmbedder {
            delay: std::time::Duration::ZERO,
        });
        {
            let mem = MemoryStore::new(Database::open(&path).unwrap(), embedder.clone()).unwrap();
            mem.set_configured_embedding_model(Some("m4")).unwrap();
        }
        let reopened = MemoryStore::new(Database::open(&path).unwrap(), embedder).unwrap();
        assert_eq!(reopened.configured_embedding_model().as_deref(), Some("m4"));
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn pending_reembedding_resumes_when_the_store_reopens() {
        let db = Database::open_in_memory().unwrap();
        db.migrate(MEMORY_MIGRATIONS).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO memory_scopes (id, kind, label, locator, locator_hash)
                 VALUES ('scope', 'project', 'Project', 'project', 'hash');
                 INSERT INTO memory_nodes (id, scope_id, title, body, dim, embedding, embedding_model)
                 VALUES ('old', 'scope', 'old vector', '', 3, X'000000000000000000000000', 'm3');
                 INSERT INTO memory_meta (key, value)
                 VALUES ('embedding_model', 'm4'), ('embedding_dim', '4');",
            )
            .unwrap();
        let mem = MemoryStore::new(
            db,
            Arc::new(FakeEmbedder {
                delay: std::time::Duration::ZERO,
            }),
        )
        .unwrap();
        let status = wait_for_reembed(&mem).await;
        assert_eq!(status.current, 1);
        assert_eq!(status.stale, 0);
        assert_eq!(node_embedding(&mem, "old"), ("m4".into(), 4));
    }

    #[tokio::test]
    async fn reembedding_can_be_cancelled_and_restarted() {
        let mem = fake_store(0);
        for index in 0..(REEMBED_BATCH * 2 + 5) {
            register_fact(&mem, "m3", &format!("memory {index}")).await;
        }
        wait_for_reembed(&mem).await;

        let slow = mem.with_embedder(mem.embedder.clone());
        let slow = MemoryStore {
            index: Arc::new(EmbeddingIndex {
                embedder: Arc::new(FakeEmbedder {
                    delay: std::time::Duration::from_millis(50),
                }),
                progress: Mutex::new(ReembedProgress::default()),
                cancel: AtomicBool::new(false),
                resumed: AtomicBool::new(true),
                configured: Mutex::new(None),
                candidate: Mutex::new(None),
            }),
            ..slow
        };
        for _ in 0..2 {
            slow.search_graph("m4", "memory", &[], 1, false)
                .await
                .unwrap();
        }
        assert!(slow.embedding_status().unwrap().reindexing);
        slow.cancel_reembed();
        let status = wait_for_reembed(&slow).await;
        assert!(status.stale > 0, "cancel should leave pending entries");
        assert!(status.reindexed <= REEMBED_BATCH);

        assert!(slow.start_reembed());
        let status = wait_for_reembed(&slow).await;
        assert_eq!(status.stale, 0);
        assert_eq!(status.current, REEMBED_BATCH * 2 + 5);
    }

    #[test]
    fn reciprocal_rank_fusion_rewards_agreement() {
        let a = test_node("a", "2025-01-01 00:00:00");
        let b = test_node("b", "2025-01-02 00:00:00");
        let c = test_node("c", "2025-01-03 00:00:00");
        let hits = fuse_rankings(vec![vec![a, b.clone()], vec![b, c]], 3);
        assert_eq!(hits[0].node.id, "b");
        assert!(hits.iter().all(|hit| (0.0..=1.0).contains(&hit.score)));
    }
}
