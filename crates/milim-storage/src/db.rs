//! SQLite database wrapper + a tiny ordered migration runner.
//!
//! Uses rusqlite's `bundled` SQLite (compiled from source, so it works on
//! Windows/Linux/macOS with no system SQLite). The harness subsystems (chat
//! history, agents, memory, …) build their schemas as [`Migration`] lists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{LockResult, Mutex, MutexGuard};
use std::time::Instant;

use milim_core::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::EncryptedStore;

static DB_LOCK_COUNT: AtomicU64 = AtomicU64::new(0);
static DB_LOCK_WAIT_NS: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_COUNT: AtomicU64 = AtomicU64::new(0);
static TRANSACTION_COMMIT_NS: AtomicU64 = AtomicU64::new(0);
static TIMELINE_WRITES: AtomicU64 = AtomicU64::new(0);
static ARTIFACT_BYTES_READ: AtomicU64 = AtomicU64::new(0);
static ARTIFACT_BYTES_WRITTEN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoragePerformanceSnapshot {
    pub db_lock_count: u64,
    pub db_lock_wait_ns: u64,
    pub transaction_count: u64,
    pub transaction_commit_ns: u64,
    pub timeline_writes: u64,
    pub artifact_bytes_read: u64,
    pub artifact_bytes_written: u64,
}

pub fn storage_performance_snapshot() -> StoragePerformanceSnapshot {
    StoragePerformanceSnapshot {
        db_lock_count: DB_LOCK_COUNT.load(Ordering::Relaxed),
        db_lock_wait_ns: DB_LOCK_WAIT_NS.load(Ordering::Relaxed),
        transaction_count: TRANSACTION_COUNT.load(Ordering::Relaxed),
        transaction_commit_ns: TRANSACTION_COMMIT_NS.load(Ordering::Relaxed),
        timeline_writes: TIMELINE_WRITES.load(Ordering::Relaxed),
        artifact_bytes_read: ARTIFACT_BYTES_READ.load(Ordering::Relaxed),
        artifact_bytes_written: ARTIFACT_BYTES_WRITTEN.load(Ordering::Relaxed),
    }
}

pub fn reset_storage_performance_counters() {
    for counter in [
        &DB_LOCK_COUNT,
        &DB_LOCK_WAIT_NS,
        &TRANSACTION_COUNT,
        &TRANSACTION_COMMIT_NS,
        &TIMELINE_WRITES,
        &ARTIFACT_BYTES_READ,
        &ARTIFACT_BYTES_WRITTEN,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
}

struct TimedDatabaseMutex(Mutex<Database>);

impl TimedDatabaseMutex {
    fn new(database: Database) -> Self {
        Self(Mutex::new(database))
    }

    fn lock(&self) -> LockResult<MutexGuard<'_, Database>> {
        let started = Instant::now();
        let result = self.0.lock();
        DB_LOCK_COUNT.fetch_add(1, Ordering::Relaxed);
        DB_LOCK_WAIT_NS.fetch_add(
            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        result
    }
}

/// One forward-only schema migration.
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalMode {
    Wal,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseOptions {
    pub journal_mode: JournalMode,
}

impl Default for DatabaseOptions {
    fn default() -> Self {
        Self {
            journal_mode: JournalMode::Wal,
        }
    }
}

/// A handle to an open SQLite database.
pub struct Database {
    conn: Connection,
    path: Option<PathBuf>,
    options: DatabaseOptions,
}

impl Database {
    /// Open (creating if needed) a database file with WAL + foreign keys on.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_options(path, DatabaseOptions::default())
    }

    /// Open (creating if needed) a database file with explicit SQLite options.
    pub fn open_with_options(path: &Path, options: DatabaseOptions) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(sqlite)?;
        Self::configure(&conn, options)?;
        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
            options,
        })
    }

    /// Open an ephemeral in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite)?;
        Self::configure(&conn, DatabaseOptions::default())?;
        Ok(Self {
            conn,
            path: None,
            options: DatabaseOptions::default(),
        })
    }

    fn configure(conn: &Connection, options: DatabaseOptions) -> Result<()> {
        // WAL is irrelevant for :memory: but harmless; foreign keys are opt-in.
        let journal_mode = match options.journal_mode {
            JournalMode::Wal => "WAL",
            JournalMode::Delete => "DELETE",
        };
        conn.pragma_update(None, "journal_mode", journal_mode)
            .map_err(sqlite)?;
        let actual: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(sqlite)?;
        if actual != "memory" && !actual.eq_ignore_ascii_case(journal_mode) {
            return Err(Error::Other(format!(
                "sqlite journal mode mismatch: requested {journal_mode}, got {actual}"
            )));
        }
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(sqlite)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(sqlite)?;
        Ok(())
    }

    /// The underlying connection (for subsystem-specific queries).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    fn open_reader(&self) -> Result<Option<Self>> {
        let Some(path) = self.path.as_deref() else {
            return Ok(None);
        };
        let reader = Self::open_with_options(path, self.options)?;
        reader
            .conn
            .pragma_update(None, "query_only", "ON")
            .map_err(sqlite)?;
        Ok(Some(reader))
    }

    /// Apply any migrations whose `version` exceeds the current schema version,
    /// in order, recording each in `_migrations`. Idempotent.
    pub fn migrate(&self, migrations: &[Migration]) -> Result<()> {
        self.migrate_scoped("default", migrations)
    }

    /// Apply migrations for one subsystem, allowing shared database files.
    pub fn migrate_scoped(&self, scope: &str, migrations: &[Migration]) -> Result<()> {
        self.ensure_migrations_table(scope)?;
        let current = self.schema_version_scoped(scope)?;
        for m in migrations {
            if m.version > current {
                self.conn
                    .execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                    .map_err(sqlite)?;
                let result = (|| -> Result<()> {
                    self.conn.execute_batch(m.sql).map_err(sqlite)?;
                    self.conn
                        .execute(
                            "INSERT INTO _migrations (scope, version, name) VALUES (?1, ?2, ?3)",
                            params![scope, m.version, m.name],
                        )
                        .map_err(sqlite)?;
                    Ok(())
                })();
                match result {
                    Ok(()) => self.conn.execute_batch("COMMIT;").map_err(sqlite)?,
                    Err(error) => {
                        let _ = self.conn.execute_batch("ROLLBACK;");
                        return Err(error);
                    }
                }
            }
        }
        Ok(())
    }

    fn ensure_migrations_table(&self, scope: &str) -> Result<()> {
        let exists = self.migrations_table_exists()?;
        if !exists {
            self.conn
                .execute_batch(
                    "CREATE TABLE _migrations (
                        scope      TEXT NOT NULL,
                        version    INTEGER NOT NULL,
                        name       TEXT NOT NULL,
                        applied_at TEXT NOT NULL DEFAULT (datetime('now')),
                        PRIMARY KEY (scope, version)
                    );",
                )
                .map_err(sqlite)?;
            return Ok(());
        }

        if self.migrations_table_has_scope()? {
            return Ok(());
        }

        self.conn
            .execute_batch(
                "ALTER TABLE _migrations RENAME TO _migrations_old;
                 CREATE TABLE _migrations (
                    scope      TEXT NOT NULL,
                    version    INTEGER NOT NULL,
                    name       TEXT NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (scope, version)
                 );",
            )
            .map_err(sqlite)?;
        self.conn
            .execute(
                "INSERT INTO _migrations (scope, version, name, applied_at)
                 SELECT ?1, version, name, applied_at FROM _migrations_old",
                params![scope],
            )
            .map_err(sqlite)?;
        self.conn
            .execute_batch("DROP TABLE _migrations_old;")
            .map_err(sqlite)?;
        Ok(())
    }

    fn migrations_table_exists(&self) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='_migrations'",
                [],
                |_| Ok(true),
            )
            .optional()
            .map_err(sqlite)
            .map(|v| v.unwrap_or(false))
    }

    fn migrations_table_has_scope(&self) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('_migrations') WHERE name='scope'",
                [],
                |_| Ok(true),
            )
            .optional()
            .map_err(sqlite)
            .map(|v| v.unwrap_or(false))
    }

    /// The highest applied migration version (0 if none).
    pub fn schema_version(&self) -> Result<u32> {
        self.schema_version_scoped("default")
    }

    /// The highest applied migration version for one subsystem (0 if none).
    pub fn schema_version_scoped(&self, scope: &str) -> Result<u32> {
        if !self.migrations_table_exists()? {
            return Ok(0);
        }
        if !self.migrations_table_has_scope()? {
            let v: i64 = self
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM _migrations",
                    [],
                    |r| r.get(0),
                )
                .map_err(sqlite)?;
            return Ok(v as u32);
        }
        let v: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _migrations WHERE scope = ?1",
                params![scope],
                |r| r.get(0),
            )
            .map_err(sqlite)?;
        Ok(v as u32)
    }

    /// Create and validate the one-time recovery point used before the
    /// forward-only control-ledger migration.
    pub fn vacuum_snapshot_into(&self, target: &Path) -> Result<()> {
        if target.exists() {
            return Ok(());
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        self.conn
            .execute("VACUUM INTO ?1", params![target.to_string_lossy().as_ref()])
            .map_err(sqlite)?;
        let snapshot = Connection::open(target).map_err(sqlite)?;
        let check: String = snapshot
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(sqlite)?;
        if !check.eq_ignore_ascii_case("ok") {
            return Err(Error::Other(format!(
                "pre-migration SQLite snapshot failed validation: {check}"
            )));
        }
        Ok(())
    }

    /// A key/value secret store over this DB, encrypting values at rest.
    pub fn secrets<'a>(&'a self, enc: &'a EncryptedStore) -> SecretKv<'a> {
        SecretKv { db: self, enc }
    }
}

/// Built-in migration providing the `secrets` table used by [`SecretKv`].
pub const SECRETS_MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "secrets",
    sql: "CREATE TABLE secrets (k TEXT PRIMARY KEY, v BLOB NOT NULL);",
}];

const SESSIONS_STATE_KEY: &str = "milim.sessions";
const SESSIONS_META_KEY: &str = "milim.sessions.meta";

const USER_DATA_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "user_json_state",
        sql: "CREATE TABLE IF NOT EXISTS user_json_state (
        key TEXT PRIMARY KEY,
        value_json TEXT NOT NULL,
        updated_at_ms INTEGER NOT NULL
    );",
    },
    Migration {
        version: 2,
        name: "user_session_rows",
        sql: "CREATE TABLE IF NOT EXISTS user_sessions (
            id TEXT PRIMARY KEY,
            session_json TEXT NOT NULL,
            sort_order INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );",
    },
    Migration {
        version: 3,
        name: "user_session_message_rows",
        sql: "CREATE TABLE IF NOT EXISTS user_session_messages (
            session_id TEXT NOT NULL,
            message_index INTEGER NOT NULL,
            message_json TEXT NOT NULL,
            PRIMARY KEY (session_id, message_index),
            FOREIGN KEY (session_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );",
    },
    Migration {
        version: 4,
        name: "user_control_runtime",
        sql: "CREATE TABLE IF NOT EXISTS user_control_host (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            host_id TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_thread_control (
            thread_id TEXT PRIMARY KEY,
            epoch TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 0,
            next_seq INTEGER NOT NULL DEFAULT 1,
            updated_at_ms INTEGER NOT NULL,
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS user_runs (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            status TEXT NOT NULL,
            adapter TEXT NOT NULL,
            request_json TEXT NOT NULL,
            agent_snapshot_json TEXT,
            native_session_json TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            completed_at_ms INTEGER,
            error_json TEXT,
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_user_runs_thread_created
            ON user_runs(thread_id, created_at_ms DESC);
        CREATE INDEX IF NOT EXISTS idx_user_runs_status
            ON user_runs(status, updated_at_ms);
        CREATE TABLE IF NOT EXISTS user_timeline_events (
            thread_id TEXT NOT NULL,
            epoch TEXT NOT NULL,
            seq INTEGER NOT NULL,
            item_id TEXT NOT NULL UNIQUE,
            run_id TEXT,
            item_type TEXT NOT NULL,
            data_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (thread_id, epoch, seq),
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (run_id) REFERENCES user_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_timeline_thread_seq
            ON user_timeline_events(thread_id, epoch, seq);
        CREATE TABLE IF NOT EXISTS user_queued_turns (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            command_id TEXT NOT NULL UNIQUE,
            request_json TEXT NOT NULL,
            accepted_at_ms INTEGER NOT NULL,
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_user_queued_turns_order
            ON user_queued_turns(thread_id, accepted_at_ms, id);
        CREATE TABLE IF NOT EXISTS user_command_receipts (
            command_id TEXT PRIMARY KEY,
            device_id TEXT,
            thread_id TEXT,
            command_kind TEXT NOT NULL,
            request_json TEXT NOT NULL,
            result_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_pending_approvals (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            request_json TEXT NOT NULL,
            status TEXT NOT NULL,
            decision_json TEXT,
            created_at_ms INTEGER NOT NULL,
            resolved_at_ms INTEGER,
            FOREIGN KEY (run_id) REFERENCES user_runs(id) ON DELETE CASCADE,
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_user_pending_approvals_status
            ON user_pending_approvals(status, created_at_ms);",
    },
    Migration {
        version: 5,
        name: "user_run_ledger_and_inbox",
        sql: "CREATE TABLE IF NOT EXISTS user_run_events (
            run_id TEXT NOT NULL,
            seq INTEGER NOT NULL,
            event_id TEXT NOT NULL UNIQUE,
            step_id TEXT,
            event_type TEXT NOT NULL,
            data_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (run_id, seq),
            FOREIGN KEY (run_id) REFERENCES user_runs(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_user_run_events_type
            ON user_run_events(run_id, event_type, seq);
        CREATE TABLE IF NOT EXISTS user_run_artifacts (
            run_id TEXT NOT NULL,
            digest TEXT NOT NULL,
            kind TEXT NOT NULL,
            data_json TEXT NOT NULL,
            byte_len INTEGER NOT NULL,
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (run_id, digest),
            FOREIGN KEY (run_id) REFERENCES user_runs(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS user_run_inbox (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            target_run_id TEXT,
            command_id TEXT UNIQUE,
            kind TEXT NOT NULL CHECK (kind IN ('followup', 'steer', 'inject')),
            state TEXT NOT NULL CHECK (state IN ('pending', 'claimed', 'cancelled', 'discarded')),
            payload_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            claimed_at_ms INTEGER,
            resolved_at_ms INTEGER,
            FOREIGN KEY (thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (target_run_id) REFERENCES user_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_run_inbox_pending
            ON user_run_inbox(thread_id, state, created_at_ms, id);
        INSERT OR IGNORE INTO user_run_inbox
            (id, thread_id, target_run_id, command_id, kind, state, payload_json,
             created_at_ms, claimed_at_ms, resolved_at_ms)
        SELECT id, thread_id, NULL, command_id, 'followup', 'pending', request_json,
               accepted_at_ms, NULL, NULL
        FROM user_queued_turns;
        DROP TABLE user_queued_turns;",
    },
    Migration {
        version: 6,
        name: "user_run_inbox_queue_order",
        sql: "ALTER TABLE user_run_inbox ADD COLUMN sort_key INTEGER;
        UPDATE user_run_inbox
        SET sort_key = created_at_ms
        WHERE kind = 'followup' AND state = 'pending';
        CREATE INDEX IF NOT EXISTS idx_user_run_inbox_queue_order
            ON user_run_inbox(thread_id, state, sort_key, created_at_ms, id);",
    },
    Migration {
        version: 7,
        name: "user_thread_links_and_mailbox",
        sql: "CREATE TABLE IF NOT EXISTS user_thread_links (
            owner_thread_id TEXT NOT NULL,
            target_thread_id TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (owner_thread_id, target_thread_id),
            CHECK (owner_thread_id <> target_thread_id),
            FOREIGN KEY (owner_thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (target_thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_user_thread_links_target
            ON user_thread_links(target_thread_id, owner_thread_id);
        CREATE TABLE IF NOT EXISTS user_thread_mailbox (
            id TEXT PRIMARY KEY,
            origin_thread_id TEXT NOT NULL,
            target_thread_id TEXT NOT NULL,
            origin_run_id TEXT,
            target_run_id TEXT,
            status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'replied', 'failed', 'discarded')),
            request_json TEXT NOT NULL,
            reply_json TEXT,
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            consumed_at_ms INTEGER,
            projected_at_ms INTEGER,
            FOREIGN KEY (origin_thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (target_thread_id) REFERENCES user_sessions(id) ON DELETE CASCADE,
            FOREIGN KEY (target_run_id) REFERENCES user_runs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_user_thread_mailbox_origin
            ON user_thread_mailbox(origin_thread_id, status, consumed_at_ms, created_at_ms, id);
        CREATE INDEX IF NOT EXISTS idx_user_thread_mailbox_target_run
            ON user_thread_mailbox(target_run_id, status);",
    },
    Migration {
        version: 8,
        name: "compressed_run_artifact_blobs",
        sql: "CREATE TABLE IF NOT EXISTS user_run_artifact_blobs (
            digest TEXT PRIMARY KEY,
            encoding TEXT NOT NULL,
            base_digest TEXT,
            payload BLOB NOT NULL,
            byte_len INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS user_run_artifact_refs (
            run_id TEXT NOT NULL,
            digest TEXT NOT NULL,
            kind TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (run_id, digest),
            FOREIGN KEY (run_id) REFERENCES user_runs(id) ON DELETE CASCADE,
            FOREIGN KEY (digest) REFERENCES user_run_artifact_blobs(digest)
        );
        CREATE INDEX IF NOT EXISTS idx_user_run_artifact_refs_kind
            ON user_run_artifact_refs(run_id, kind, created_at_ms, digest);
        CREATE INDEX IF NOT EXISTS idx_user_run_events_step
            ON user_run_events(run_id, step_id, seq);
        CREATE TRIGGER IF NOT EXISTS delete_unreferenced_run_artifact_blob
        AFTER DELETE ON user_run_artifact_refs
        BEGIN
            DELETE FROM user_run_artifact_blobs
            WHERE digest = OLD.digest
              AND NOT EXISTS (
                SELECT 1 FROM user_run_artifact_refs WHERE digest = OLD.digest
              );
        END;",
    },
    Migration {
        version: 9,
        name: "session_message_fts",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS user_session_message_fts USING fts5(
            source_rowid UNINDEXED,
            session_id UNINDEXED,
            message_index UNINDEXED,
            role UNINDEXED,
            content,
            tokenize = 'unicode61'
        );
        CREATE TRIGGER IF NOT EXISTS user_session_message_fts_insert
        AFTER INSERT ON user_session_messages
        BEGIN
            INSERT INTO user_session_message_fts
                (source_rowid, session_id, message_index, role, content)
            VALUES (
                NEW.rowid,
                NEW.session_id,
                NEW.message_index,
                COALESCE(json_extract(NEW.message_json, '$.role'), ''),
                COALESCE(json_extract(NEW.message_json, '$.content'), '')
            );
        END;
        CREATE TRIGGER IF NOT EXISTS user_session_message_fts_update
        AFTER UPDATE ON user_session_messages
        BEGIN
            DELETE FROM user_session_message_fts WHERE source_rowid = OLD.rowid;
            INSERT INTO user_session_message_fts
                (source_rowid, session_id, message_index, role, content)
            VALUES (
                NEW.rowid,
                NEW.session_id,
                NEW.message_index,
                COALESCE(json_extract(NEW.message_json, '$.role'), ''),
                COALESCE(json_extract(NEW.message_json, '$.content'), '')
            );
        END;
        CREATE TRIGGER IF NOT EXISTS user_session_message_fts_delete
        AFTER DELETE ON user_session_messages
        BEGIN
            DELETE FROM user_session_message_fts WHERE source_rowid = OLD.rowid;
        END;",
    },
];

/// Encrypted key/value store (API keys, OAuth tokens, agent secrets).
pub struct SecretKv<'a> {
    db: &'a Database,
    enc: &'a EncryptedStore,
}

impl SecretKv<'_> {
    /// Store (upsert) an encrypted value.
    pub fn put(&self, key: &str, value: &[u8]) -> Result<()> {
        let blob = self.enc.encrypt(value)?;
        self.db
            .conn
            .execute(
                "INSERT INTO secrets (k, v) VALUES (?1, ?2)
                 ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                params![key, blob],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    /// Fetch and decrypt a value, if present.
    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let blob: Option<Vec<u8>> = self
            .db
            .conn
            .query_row("SELECT v FROM secrets WHERE k = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sqlite)?;
        match blob {
            Some(b) => Ok(Some(self.enc.decrypt(&b)?)),
            None => Ok(None),
        }
    }

    /// Delete a value. Returns whether a row was removed.
    pub fn delete(&self, key: &str) -> Result<bool> {
        let n = self
            .db
            .conn
            .execute("DELETE FROM secrets WHERE k = ?1", params![key])
            .map_err(sqlite)?;
        Ok(n > 0)
    }
}

pub struct UserDataStore {
    db: TimedDatabaseMutex,
    read_pool: Vec<TimedDatabaseMutex>,
    read_cursor: AtomicUsize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlHostRecord {
    pub host_id: String,
    pub display_name: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlThreadRecord {
    pub id: String,
    pub session_json: String,
    pub revision: u64,
    pub epoch: String,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRunRecord {
    pub id: String,
    pub thread_id: String,
    pub status: String,
    pub adapter: String,
    pub request_json: String,
    pub agent_snapshot_json: Option<String>,
    pub native_session_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub error_json: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlTimelineRecord {
    pub thread_id: String,
    pub epoch: String,
    pub seq: u64,
    pub item_id: String,
    pub run_id: Option<String>,
    pub item_type: String,
    pub data_json: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlTimelinePage {
    pub epoch: String,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub has_older: bool,
    pub has_newer: bool,
    pub items: Vec<ControlTimelineRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlQueuedTurnRecord {
    pub id: String,
    pub thread_id: String,
    pub command_id: String,
    pub request_json: String,
    pub accepted_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlCommandReceiptRecord {
    pub command_id: String,
    pub device_id: Option<String>,
    pub thread_id: Option<String>,
    pub command_kind: String,
    pub request_json: String,
    pub result_json: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlApprovalRecord {
    pub id: String,
    pub run_id: String,
    pub thread_id: String,
    pub kind: String,
    pub request_json: String,
    pub status: String,
    pub decision_json: Option<String>,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRunEventRecord {
    pub run_id: String,
    pub seq: u64,
    pub event_id: String,
    pub step_id: Option<String>,
    pub event_type: String,
    pub data_json: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRunArtifactRecord {
    pub run_id: String,
    pub digest: String,
    pub kind: String,
    pub data_json: String,
    pub byte_len: u64,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunArtifactMigrationProgress {
    pub migrated: usize,
    pub raw_bytes: u64,
    pub remaining: usize,
}

#[derive(Debug)]
struct EncodedRunArtifact {
    encoding: String,
    base_digest: Option<String>,
    payload: Vec<u8>,
    byte_len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlInboxRecord {
    pub id: String,
    pub thread_id: String,
    pub target_run_id: Option<String>,
    pub command_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub payload_json: String,
    pub created_at_ms: i64,
    pub claimed_at_ms: Option<i64>,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlThreadLinkRecord {
    pub owner_thread_id: String,
    pub target_thread_id: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlMailboxRecord {
    pub id: String,
    pub origin_thread_id: String,
    pub target_thread_id: String,
    pub origin_run_id: Option<String>,
    pub target_run_id: Option<String>,
    pub status: String,
    pub request_json: String,
    pub reply_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub consumed_at_ms: Option<i64>,
    pub projected_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlBackupState {
    #[serde(default = "default_control_backup_version")]
    pub schema_version: u32,
    pub host: Option<ControlHostRecord>,
    pub threads: Vec<ControlThreadRecord>,
    pub runs: Vec<ControlRunRecord>,
    pub timeline: Vec<ControlTimelineRecord>,
    pub queued_turns: Vec<ControlQueuedTurnRecord>,
    pub command_receipts: Vec<ControlCommandReceiptRecord>,
    pub approvals: Vec<ControlApprovalRecord>,
    #[serde(default)]
    pub run_events: Vec<ControlRunEventRecord>,
    #[serde(default)]
    pub run_artifacts: Vec<ControlRunArtifactRecord>,
    #[serde(default)]
    pub inbox: Vec<ControlInboxRecord>,
    #[serde(default)]
    pub thread_links: Vec<ControlThreadLinkRecord>,
    #[serde(default)]
    pub mailbox: Vec<ControlMailboxRecord>,
}

fn default_control_backup_version() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsDelta {
    pub meta_json: String,
    pub session_order: Vec<String>,
    pub upserts: Vec<SessionDelta>,
    pub deleted_session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDelta {
    pub id: String,
    pub session_json: Option<String>,
    pub message_count: usize,
    #[serde(default)]
    pub preserve_messages: bool,
    pub messages: Vec<SessionMessageDelta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageDelta {
    pub index: usize,
    pub message_json: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionMessagesPage {
    pub session_id: String,
    pub first_index: usize,
    pub total: usize,
    pub has_older: bool,
    pub messages: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserChatSearchResult {
    pub session_id: String,
    pub title: String,
    pub metadata: String,
    pub snippet: String,
    pub updated_at: i64,
    pub score: f64,
}

const RUN_ARTIFACT_COMPRESSION_THRESHOLD: usize = 1024;
const RUN_ARTIFACT_CHECKPOINT_INTERVAL: i64 = 25;
const RUN_ARTIFACT_MAX_DELTA_DEPTH: usize = 32;

fn artifact_digest(data: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(data))
}

fn verify_artifact_digest(digest: &str, data: &[u8]) -> Result<()> {
    if digest.starts_with("sha256:") && artifact_digest(data) != digest {
        return Err(Error::Other(format!(
            "run artifact {digest} failed digest verification"
        )));
    }
    Ok(())
}

fn compress_artifact(data: &[u8]) -> Result<(String, Vec<u8>)> {
    if data.len() < RUN_ARTIFACT_COMPRESSION_THRESHOLD {
        return Ok(("json-v1".into(), data.to_vec()));
    }
    let compressed = zstd::stream::encode_all(data, 3)
        .map_err(|error| Error::Other(format!("compress run artifact: {error}")))?;
    if compressed.len() >= data.len() {
        Ok(("json-v1".into(), data.to_vec()))
    } else {
        Ok(("zstd-json-v1".into(), compressed))
    }
}

fn prefix_delta(base: &[u8], next: &[u8]) -> Vec<u8> {
    let prefix = base
        .iter()
        .zip(next)
        .take_while(|(left, right)| left == right)
        .count();
    let max_suffix = base.len().min(next.len()).saturating_sub(prefix);
    let suffix = base
        .iter()
        .rev()
        .zip(next.iter().rev())
        .take(max_suffix)
        .take_while(|(left, right)| left == right)
        .count();
    let middle = &next[prefix..next.len().saturating_sub(suffix)];
    let mut delta = Vec::with_capacity(16 + middle.len());
    delta.extend_from_slice(&(prefix as u64).to_le_bytes());
    delta.extend_from_slice(&(suffix as u64).to_le_bytes());
    delta.extend_from_slice(middle);
    delta
}

fn apply_prefix_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>> {
    if delta.len() < 16 {
        return Err(Error::Other("run artifact delta is truncated".into()));
    }
    let prefix = usize::try_from(u64::from_le_bytes(
        delta[..8].try_into().expect("eight-byte prefix"),
    ))
    .map_err(|_| Error::Other("run artifact delta prefix is too large".into()))?;
    let suffix = usize::try_from(u64::from_le_bytes(
        delta[8..16].try_into().expect("eight-byte suffix"),
    ))
    .map_err(|_| Error::Other("run artifact delta suffix is too large".into()))?;
    if prefix + suffix > base.len() {
        return Err(Error::Other("run artifact delta exceeds its base".into()));
    }
    let mut data = Vec::with_capacity(prefix + delta.len().saturating_sub(16) + suffix);
    data.extend_from_slice(&base[..prefix]);
    data.extend_from_slice(&delta[16..]);
    data.extend_from_slice(&base[base.len() - suffix..]);
    Ok(data)
}

fn decode_run_artifact_blob_locked(
    conn: &Connection,
    digest: &str,
    depth: usize,
) -> Result<Vec<u8>> {
    if depth > RUN_ARTIFACT_MAX_DELTA_DEPTH {
        return Err(Error::Other(format!(
            "run artifact {digest} exceeded the delta checkpoint depth"
        )));
    }
    let row = conn
        .query_row(
            "SELECT encoding, base_digest, payload, byte_len
             FROM user_run_artifact_blobs WHERE digest = ?1",
            params![digest],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite)?
        .ok_or_else(|| Error::Other(format!("run artifact blob {digest} is missing")))?;
    let (encoding, base_digest, payload, byte_len) = row;
    ARTIFACT_BYTES_READ.fetch_add(
        u64::try_from(payload.len()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
    let data = match encoding.as_str() {
        "json-v1" => payload,
        "zstd-json-v1" => zstd::stream::decode_all(payload.as_slice())
            .map_err(|error| Error::Other(format!("decompress run artifact: {error}")))?,
        "prefix-delta-zstd-v1" => {
            let base_digest = base_digest
                .ok_or_else(|| Error::Other(format!("run artifact {digest} has no delta base")))?;
            let base = decode_run_artifact_blob_locked(conn, &base_digest, depth + 1)?;
            let delta = zstd::stream::decode_all(payload.as_slice())
                .map_err(|error| Error::Other(format!("decompress run artifact delta: {error}")))?;
            apply_prefix_delta(&base, &delta)?
        }
        _ => {
            return Err(Error::Other(format!(
                "run artifact {digest} uses unsupported encoding {encoding}"
            )))
        }
    };
    let expected_len = usize::try_from(byte_len)
        .map_err(|_| Error::Other("run artifact byte length is outside usize".into()))?;
    if data.len() != expected_len {
        return Err(Error::Other(format!(
            "run artifact {digest} decoded to {} bytes, expected {expected_len}",
            data.len()
        )));
    }
    verify_artifact_digest(digest, &data)?;
    Ok(data)
}

fn encode_run_artifact_locked(
    conn: &Connection,
    artifact: &ControlRunArtifactRecord,
) -> Result<EncodedRunArtifact> {
    let data = artifact.data_json.as_bytes();
    verify_artifact_digest(&artifact.digest, data)?;
    let (encoding, payload) = compress_artifact(data)?;
    let mut encoded = EncodedRunArtifact {
        encoding,
        base_digest: None,
        payload,
        byte_len: u64::try_from(data.len()).unwrap_or(u64::MAX),
    };
    if artifact.kind != "provider_request" || data.len() < RUN_ARTIFACT_COMPRESSION_THRESHOLD {
        return Ok(encoded);
    }
    let request_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM user_run_artifact_refs
             WHERE run_id = ?1 AND kind = 'provider_request'",
            params![artifact.run_id],
            |row| row.get(0),
        )
        .map_err(sqlite)?;
    if request_count % RUN_ARTIFACT_CHECKPOINT_INTERVAL == 0 {
        return Ok(encoded);
    }
    let base_digest = conn
        .query_row(
            "SELECT digest FROM user_run_artifact_refs
             WHERE run_id = ?1 AND kind = 'provider_request'
             ORDER BY created_at_ms DESC, digest DESC LIMIT 1",
            params![artifact.run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite)?;
    let Some(base_digest) = base_digest else {
        return Ok(encoded);
    };
    let base = decode_run_artifact_blob_locked(conn, &base_digest, 0)?;
    let delta = prefix_delta(&base, data);
    let compressed_delta = zstd::stream::encode_all(delta.as_slice(), 3)
        .map_err(|error| Error::Other(format!("compress run artifact delta: {error}")))?;
    if compressed_delta.len() + 64 < encoded.payload.len() {
        encoded.encoding = "prefix-delta-zstd-v1".into();
        encoded.base_digest = Some(base_digest);
        encoded.payload = compressed_delta;
    }
    Ok(encoded)
}

fn put_run_artifact_locked(conn: &Connection, artifact: &ControlRunArtifactRecord) -> Result<()> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM user_run_artifact_blobs WHERE digest = ?1",
            params![artifact.digest],
            |_| Ok(()),
        )
        .optional()
        .map_err(sqlite)?
        .is_some();
    if !exists {
        let encoded = encode_run_artifact_locked(conn, artifact)?;
        ARTIFACT_BYTES_WRITTEN.fetch_add(
            u64::try_from(encoded.payload.len()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        conn.execute(
            "INSERT INTO user_run_artifact_blobs
             (digest, encoding, base_digest, payload, byte_len)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                artifact.digest,
                encoded.encoding,
                encoded.base_digest,
                encoded.payload,
                u64_to_i64(encoded.byte_len)?,
            ],
        )
        .map_err(sqlite)?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO user_run_artifact_refs
         (run_id, digest, kind, created_at_ms) VALUES (?1, ?2, ?3, ?4)",
        params![
            artifact.run_id,
            artifact.digest,
            artifact.kind,
            artifact.created_at_ms,
        ],
    )
    .map_err(sqlite)?;
    Ok(())
}

impl UserDataStore {
    pub fn new(db: Database) -> Result<Self> {
        if db.schema_version_scoped("user_data")? == 4 {
            let mut statement = db
                .conn()
                .prepare("SELECT id, request_json FROM user_queued_turns ORDER BY id")
                .map_err(sqlite)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            for (id, request_json) in rows {
                validate_control_json(&request_json, &format!("queued turn {id}"))?;
            }
        }
        db.migrate_scoped("user_data", USER_DATA_MIGRATIONS)?;
        let mut read_pool = Vec::with_capacity(2);
        for _ in 0..2 {
            if let Some(reader) = db.open_reader()? {
                read_pool.push(TimedDatabaseMutex::new(reader));
            }
        }
        Ok(Self {
            db: TimedDatabaseMutex::new(db),
            read_pool,
            read_cursor: AtomicUsize::new(0),
        })
    }

    fn read_db(&self) -> LockResult<MutexGuard<'_, Database>> {
        if self.read_pool.is_empty() {
            return self.db.lock();
        }
        let index = self.read_cursor.fetch_add(1, Ordering::Relaxed) % self.read_pool.len();
        self.read_pool[index].lock()
    }

    pub fn get_json(&self, key: &str) -> Result<Option<String>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT value_json FROM user_json_state WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn get_sessions_snapshot(&self) -> Result<Option<String>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let mut sessions = session_rows(conn)?;
        if let Some(legacy) = get_json_locked(conn, SESSIONS_STATE_KEY)? {
            if should_migrate_sessions_snapshot(&legacy, &sessions)? {
                set_sessions_snapshot_locked(conn, &legacy)?;
                sessions = session_rows(conn)?;
            }
        }
        let mut root = get_json_locked(conn, SESSIONS_META_KEY)?
            .as_deref()
            .map(parse_json)
            .transpose()?
            .unwrap_or_else(|| {
                if sessions.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!({ "state": {}, "version": 0 })
                }
            });
        if root.is_null() {
            return Ok(None);
        }
        let state = root
            .get_mut("state")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| Error::Other("invalid sessions metadata".into()))?;
        state.insert("sessions".to_string(), serde_json::Value::Array(sessions));
        serde_json::to_string(&root).map(Some).map_err(json_error)
    }

    pub fn get_sessions_manifest_snapshot(&self, tail_limit: usize) -> Result<Option<String>> {
        let has_legacy = {
            let db = self
                .read_db()
                .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
            get_json_locked(db.conn(), SESSIONS_STATE_KEY)?.is_some()
        };
        if has_legacy {
            let _ = self.get_sessions_snapshot()?;
        }
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let mut root = get_json_locked(conn, SESSIONS_META_KEY)?
            .as_deref()
            .map(parse_json)
            .transpose()?
            .unwrap_or_else(|| serde_json::json!({ "state": {}, "version": 0 }));
        let active_id = root
            .get("state")
            .and_then(|state| state.get("activeId"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let mut load_ids = active_id.into_iter().collect::<BTreeSet<_>>();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT thread_id FROM user_runs
                     WHERE status IN ('accepted', 'running')",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(sqlite)?;
            for row in rows {
                load_ids.insert(row.map_err(sqlite)?);
            }
        }
        let mut stmt = conn
            .prepare(
                "SELECT id, session_json,
                        (SELECT COUNT(*) FROM user_session_messages WHERE session_id = user_sessions.id)
                 FROM user_sessions ORDER BY sort_order ASC, updated_at_ms DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(sqlite)?;
        let mut sessions = Vec::new();
        for row in rows {
            let (id, session_json, count) = row.map_err(sqlite)?;
            let total = usize::try_from(count)
                .map_err(|_| Error::Other("session message count is negative".into()))?;
            let page = if load_ids.contains(&id) {
                session_messages_page_locked(conn, &id, None, tail_limit)?
            } else {
                SessionMessagesPage {
                    session_id: id.clone(),
                    first_index: total,
                    total,
                    has_older: total > 0,
                    messages: Vec::new(),
                }
            };
            let mut session = parse_json(&session_json)?;
            let object = session
                .as_object_mut()
                .ok_or_else(|| Error::Other("invalid session row".into()))?;
            object.insert("messages".into(), serde_json::Value::Array(page.messages));
            object.insert("persistedMessageCount".into(), page.total.into());
            object.insert("messagesLoadedFrom".into(), page.first_index.into());
            object.insert(
                "messagesHydrated".into(),
                serde_json::Value::Bool(!page.has_older),
            );
            if let Some(preview) = session_message_preview_locked(conn, &id)? {
                object.insert("messagePreview".into(), preview.into());
            }
            sessions.push(session);
        }
        let state = root
            .get_mut("state")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| Error::Other("invalid sessions metadata".into()))?;
        state.insert("sessions".into(), serde_json::Value::Array(sessions));
        serde_json::to_string(&root).map(Some).map_err(json_error)
    }

    pub fn session_messages_page(
        &self,
        session_id: &str,
        before_index: Option<usize>,
        limit: usize,
    ) -> Result<SessionMessagesPage> {
        let session_id = required_control_text(session_id, "session id")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        session_messages_page_locked(db.conn(), session_id, before_index, limit)
    }

    /// Index a bounded number of legacy messages that predate the FTS trigger.
    /// New and updated rows are indexed synchronously by SQLite triggers.
    pub fn index_session_messages_batch(&self, limit: usize) -> Result<usize> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_session_message_fts
                    (source_rowid, session_id, message_index, role, content)
                 SELECT m.rowid, m.session_id, m.message_index,
                        COALESCE(json_extract(m.message_json, '$.role'), ''),
                        COALESCE(json_extract(m.message_json, '$.content'), '')
                 FROM user_session_messages m
                 WHERE NOT EXISTS (
                    SELECT 1 FROM user_session_message_fts f
                    WHERE f.source_rowid = m.rowid
                 )
                 ORDER BY m.rowid
                 LIMIT ?1",
                params![limit.clamp(1, 2_000) as i64],
            )
            .map_err(sqlite)
    }

    pub fn user_chat_search(&self, query: &str, limit: usize) -> Result<Vec<UserChatSearchResult>> {
        let parsed = parse_chat_search_query(query);
        let limit = limit.clamp(1, 20);
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let mut sessions = BTreeMap::<String, (String, String, i64, bool)>::new();
        let mut stmt = conn
            .prepare(
                "SELECT id, session_json, updated_at_ms
                 FROM user_sessions ORDER BY updated_at_ms DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(sqlite)?;
        for row in rows {
            let (id, session_json, updated_at) = row.map_err(sqlite)?;
            let value = parse_json(&session_json)?;
            let title = value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Untitled chat")
                .to_string();
            let folder = value
                .pointer("/settings/folder")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let model = value
                .pointer("/settings/model")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let metadata = [folder_label(folder), model]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
            let archived = value
                .get("archivedAt")
                .is_some_and(|value| !value.is_null());
            sessions.insert(id, (title, metadata, updated_at, archived));
        }
        let mut matches = BTreeMap::<String, UserChatSearchResult>::new();
        for (id, (title, metadata, updated_at, archived)) in &sessions {
            if !parsed.includes_archive(*archived) {
                continue;
            }
            let haystack = format!("{title} {metadata}").to_lowercase();
            if parsed.terms.is_empty() || parsed.terms.iter().all(|term| haystack.contains(term)) {
                let title_lower = title.to_lowercase();
                let score = if parsed.text.is_empty() {
                    0.0
                } else if title_lower == parsed.text {
                    120.0
                } else if title_lower.starts_with(&parsed.text) {
                    80.0
                } else {
                    36.0 * parsed
                        .terms
                        .iter()
                        .filter(|term| title_lower.contains(term.as_str()))
                        .count() as f64
                        + 12.0
                            * parsed
                                .terms
                                .iter()
                                .filter(|term| metadata.to_lowercase().contains(term.as_str()))
                                .count() as f64
                };
                matches.insert(
                    id.clone(),
                    UserChatSearchResult {
                        session_id: id.clone(),
                        title: title.clone(),
                        metadata: metadata.clone(),
                        snippet: String::new(),
                        updated_at: *updated_at,
                        score,
                    },
                );
            }
        }
        if !parsed.terms.is_empty() {
            let fts_query = parsed
                .terms
                .iter()
                .map(|term| format!("\"{term}\""))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut stmt = conn
                .prepare(
                    "SELECT session_id, role,
                            snippet(user_session_message_fts, 4, '', '', '...', 28),
                            bm25(user_session_message_fts)
                     FROM user_session_message_fts
                     WHERE user_session_message_fts MATCH ?1
                       AND (?2 IS NULL OR role = ?2)
                     ORDER BY bm25(user_session_message_fts), rowid DESC
                     LIMIT 100",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map(params![fts_query, parsed.role], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                    ))
                })
                .map_err(sqlite)?;
            for row in rows {
                let (id, role, snippet, rank) = row.map_err(sqlite)?;
                let Some((title, metadata, updated_at, archived)) = sessions.get(&id) else {
                    continue;
                };
                if !parsed.includes_archive(*archived) {
                    continue;
                }
                let score = 40.0 - rank;
                let prefix = match role.as_str() {
                    "user" => "You: ",
                    "assistant" => "Assistant: ",
                    _ => "",
                };
                let candidate = UserChatSearchResult {
                    session_id: id.clone(),
                    title: title.clone(),
                    metadata: metadata.clone(),
                    snippet: format!("{prefix}{snippet}"),
                    updated_at: *updated_at,
                    score,
                };
                if matches
                    .get(&id)
                    .is_none_or(|existing| candidate.score > existing.score)
                {
                    matches.insert(id, candidate);
                }
            }
        }
        let mut matches = matches.into_values().collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
        });
        matches.truncate(limit);
        for result in &mut matches {
            if result.snippet.is_empty() {
                result.snippet = chat_search_preview_locked(conn, &result.session_id, parsed.role)?
                    .unwrap_or_else(|| "No messages yet".into());
            }
        }
        Ok(matches)
    }

    pub fn set_sessions_snapshot(&self, value_json: &str) -> Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let sessions = session_rows(conn)?;
        if should_ignore_default_sessions_snapshot(value_json, &sessions)? {
            return Ok(());
        }
        set_sessions_snapshot_locked(conn, value_json)
    }

    pub fn apply_sessions_delta(&self, delta: SessionsDelta) -> Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        apply_sessions_delta_locked(db.conn(), delta)
    }

    pub fn delete_sessions_snapshot(&self) -> Result<bool> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let removed_sessions = conn
            .execute("DELETE FROM user_session_messages", [])
            .map_err(sqlite)?
            + conn
                .execute("DELETE FROM user_sessions", [])
                .map_err(sqlite)?;
        let removed_meta = conn
            .execute(
                "DELETE FROM user_json_state WHERE key IN (?1, ?2)",
                params![SESSIONS_STATE_KEY, SESSIONS_META_KEY],
            )
            .map_err(sqlite)?;
        Ok(removed_sessions + removed_meta > 0)
    }

    pub fn set_json(&self, key: &str, value_json: &str) -> Result<()> {
        serde_json::from_str::<serde_json::Value>(value_json)
            .map_err(|e| Error::InvalidRequest(format!("invalid JSON for {key}: {e}")))?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_json_state (key, value_json, updated_at_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at_ms = excluded.updated_at_ms",
                params![key, value_json, now_ms()],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn delete_json(&self, key: &str) -> Result<bool> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let changed = db
            .conn()
            .execute("DELETE FROM user_json_state WHERE key = ?1", params![key])
            .map_err(sqlite)?;
        Ok(changed > 0)
    }

    pub fn replace_backup_state(
        &self,
        replace_keys: &[String],
        entries: BTreeMap<String, String>,
        sessions_json: &str,
    ) -> Result<()> {
        for (key, value) in &entries {
            serde_json::from_str::<serde_json::Value>(value)
                .map_err(|e| Error::InvalidRequest(format!("invalid JSON for {key}: {e}")))?;
        }
        serde_json::from_str::<serde_json::Value>(sessions_json)
            .map_err(|e| Error::InvalidRequest(format!("invalid sessions JSON: {e}")))?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<()> {
            for key in replace_keys {
                conn.execute("DELETE FROM user_json_state WHERE key = ?1", params![key])
                    .map_err(sqlite)?;
            }
            for (key, value) in entries {
                conn.execute(
                    "INSERT INTO user_json_state (key, value_json, updated_at_ms)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                        value_json = excluded.value_json,
                        updated_at_ms = excluded.updated_at_ms",
                    params![key, value, now_ms()],
                )
                .map_err(sqlite)?;
            }
            conn.execute("DELETE FROM user_session_messages", [])
                .map_err(sqlite)?;
            conn.execute("DELETE FROM user_sessions", [])
                .map_err(sqlite)?;
            conn.execute(
                "DELETE FROM user_json_state WHERE key IN (?1, ?2)",
                params![SESSIONS_STATE_KEY, SESSIONS_META_KEY],
            )
            .map_err(sqlite)?;
            set_sessions_snapshot_locked(conn, sessions_json)?;
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT").map_err(sqlite),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn import_json_entries(&self, entries: BTreeMap<String, String>) -> Result<()> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        for (key, value) in entries {
            serde_json::from_str::<serde_json::Value>(&value)
                .map_err(|e| Error::InvalidRequest(format!("invalid JSON for {key}: {e}")))?;
            if key == SESSIONS_STATE_KEY {
                let sessions = session_rows(db.conn())?;
                if should_migrate_sessions_snapshot(&value, &sessions)? {
                    set_sessions_snapshot_locked(db.conn(), &value)?;
                }
                continue;
            }
            db.conn()
                .execute(
                    "INSERT OR IGNORE INTO user_json_state (key, value_json, updated_at_ms)
                     VALUES (?1, ?2, ?3)",
                    params![key, value, now_ms()],
                )
                .map_err(sqlite)?;
        }
        Ok(())
    }

    /// Return the stable identity of this desktop host, creating it once.
    /// The first caller wins so reinstalling or renaming a client cannot
    /// silently replace the identity used to partition mobile replicas.
    pub fn ensure_control_host(
        &self,
        proposed_host_id: &str,
        display_name: &str,
    ) -> Result<ControlHostRecord> {
        let proposed_host_id = required_control_text(proposed_host_id, "host id")?;
        let display_name = required_control_text(display_name, "host display name")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let now = now_ms();
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO user_control_host
                 (singleton, host_id, display_name, created_at_ms, updated_at_ms)
                 VALUES (1, ?1, ?2, ?3, ?3)",
                params![proposed_host_id, display_name, now],
            )
            .map_err(sqlite)?;
        db.conn()
            .query_row(
                "SELECT host_id, display_name, created_at_ms, updated_at_ms
                 FROM user_control_host WHERE singleton = 1",
                [],
                |row| {
                    Ok(ControlHostRecord {
                        host_id: row.get(0)?,
                        display_name: row.get(1)?,
                        created_at_ms: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    })
                },
            )
            .map_err(sqlite)
    }

    pub fn update_control_host_name(&self, display_name: &str) -> Result<ControlHostRecord> {
        let display_name = required_control_text(display_name, "host display name")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_control_host SET display_name = ?1, updated_at_ms = ?2
                 WHERE singleton = 1",
                params![display_name, now_ms()],
            )
            .map_err(sqlite)?;
        db.conn()
            .query_row(
                "SELECT host_id, display_name, created_at_ms, updated_at_ms
                 FROM user_control_host WHERE singleton = 1",
                [],
                |row| {
                    Ok(ControlHostRecord {
                        host_id: row.get(0)?,
                        display_name: row.get(1)?,
                        created_at_ms: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    })
                },
            )
            .map_err(sqlite)
    }

    /// Materialize control metadata for legacy sessions without rewriting the
    /// existing session JSON or message rows.
    pub fn control_threads(&self) -> Result<Vec<ControlThreadRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute(
            "INSERT OR IGNORE INTO user_thread_control
             (thread_id, epoch, revision, next_seq, updated_at_ms)
             SELECT id, lower(hex(randomblob(16))), 0, 1, updated_at_ms
             FROM user_sessions",
            [],
        )
        .map_err(sqlite)?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.session_json, c.revision, c.epoch, s.updated_at_ms
                 FROM user_sessions s
                 JOIN user_thread_control c ON c.thread_id = s.id
                 ORDER BY s.sort_order ASC, s.updated_at_ms DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map([], control_thread_from_row)
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    /// Legacy threads that still have messages but no canonical timeline.
    /// This avoids deserializing every message on every desktop launch.
    pub fn control_threads_missing_message_timeline(&self) -> Result<Vec<ControlThreadRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute(
            "INSERT OR IGNORE INTO user_thread_control
             (thread_id, epoch, revision, next_seq, updated_at_ms)
             SELECT id, lower(hex(randomblob(16))), 0, 1, updated_at_ms
             FROM user_sessions",
            [],
        )
        .map_err(sqlite)?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.session_json, c.revision, c.epoch, s.updated_at_ms
                 FROM user_sessions s
                 JOIN user_thread_control c ON c.thread_id = s.id
                 WHERE EXISTS (
                    SELECT 1 FROM user_session_messages m WHERE m.session_id = s.id
                 )
                   AND NOT EXISTS (
                    SELECT 1 FROM user_timeline_events t WHERE t.thread_id = s.id
                 )
                 ORDER BY s.sort_order ASC, s.updated_at_ms DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map([], control_thread_from_row)
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_thread(&self, thread_id: &str) -> Result<Option<ControlThreadRecord>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute(
            "INSERT OR IGNORE INTO user_thread_control
             (thread_id, epoch, revision, next_seq, updated_at_ms)
             SELECT id, lower(hex(randomblob(16))), 0, 1, updated_at_ms
             FROM user_sessions WHERE id = ?1",
            params![thread_id],
        )
        .map_err(sqlite)?;
        conn.query_row(
            "SELECT s.id, s.session_json, c.revision, c.epoch, s.updated_at_ms
             FROM user_sessions s
             JOIN user_thread_control c ON c.thread_id = s.id
             WHERE s.id = ?1",
            params![thread_id],
            control_thread_from_row,
        )
        .optional()
        .map_err(sqlite)
    }

    pub fn control_create_thread(
        &self,
        thread_id: &str,
        session_json: &str,
        epoch: &str,
    ) -> Result<ControlThreadRecord> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let epoch = required_control_text(epoch, "thread epoch")?;
        validate_control_json(session_json, "thread")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let now = now_ms();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<ControlThreadRecord> {
            let sort_order: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM user_sessions",
                    [],
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            conn.execute(
                "INSERT INTO user_sessions (id, session_json, sort_order, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)",
                params![thread_id, session_json, sort_order, now],
            )
            .map_err(sqlite)?;
            conn.execute(
                "INSERT INTO user_thread_control
                 (thread_id, epoch, revision, next_seq, updated_at_ms)
                 VALUES (?1, ?2, 1, 1, ?3)",
                params![thread_id, epoch, now],
            )
            .map_err(sqlite)?;
            Ok(ControlThreadRecord {
                id: thread_id.to_string(),
                session_json: session_json.to_string(),
                revision: 1,
                epoch: epoch.to_string(),
                updated_at_ms: now,
            })
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_update_thread(
        &self,
        thread_id: &str,
        session_json: &str,
        expected_revision: Option<u64>,
        timeline: Option<(&str, &str, &str)>,
    ) -> Result<Option<(ControlThreadRecord, Option<ControlTimelineRecord>)>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        validate_control_json(session_json, "thread")?;
        if let Some((item_id, item_type, data_json)) = timeline {
            required_control_text(item_id, "timeline item id")?;
            required_control_text(item_type, "timeline item type")?;
            validate_control_json(data_json, "timeline data")?;
        }
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result =
            (|| -> Result<Option<(ControlThreadRecord, Option<ControlTimelineRecord>)>> {
                let Some(mut current) = conn
                    .query_row(
                        "SELECT s.id, s.session_json, c.revision, c.epoch, s.updated_at_ms
                     FROM user_sessions s
                     JOIN user_thread_control c ON c.thread_id = s.id
                     WHERE s.id = ?1",
                        params![thread_id],
                        control_thread_from_row,
                    )
                    .optional()
                    .map_err(sqlite)?
                else {
                    return Ok(None);
                };
                if let Some(expected) = expected_revision {
                    if expected != current.revision {
                        return Err(Error::InvalidRequest(format!(
                            "thread revision conflict: expected {expected}, current {}",
                            current.revision
                        )));
                    }
                }
                let now = now_ms();
                current.revision = current.revision.saturating_add(1);
                current.session_json = session_json.to_string();
                current.updated_at_ms = now;
                conn.execute(
                    "UPDATE user_sessions SET session_json = ?2, updated_at_ms = ?3 WHERE id = ?1",
                    params![thread_id, session_json, now],
                )
                .map_err(sqlite)?;
                conn.execute(
                "UPDATE user_thread_control SET revision = ?2, updated_at_ms = ?3 WHERE thread_id = ?1",
                params![thread_id, u64_to_i64(current.revision)?, now],
            )
            .map_err(sqlite)?;
                let timeline = timeline
                    .map(|(item_id, item_type, data_json)| {
                        append_control_timeline_locked(
                            conn, thread_id, item_id, None, item_type, data_json, now,
                        )
                    })
                    .transpose()?;
                if timeline.is_some() {
                    current.revision = current.revision.saturating_add(1);
                }
                Ok(Some((current, timeline)))
            })();
        finish_control_transaction(conn, result)
    }

    pub fn control_delete_thread(&self, thread_id: &str) -> Result<bool> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "DELETE FROM user_sessions WHERE id = ?1",
                params![thread_id],
            )
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_thread_links(
        &self,
        owner_thread_id: Option<&str>,
    ) -> Result<Vec<ControlThreadLinkRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let (sql, owner_thread_id) = match owner_thread_id {
            Some(owner_thread_id) => (
                "SELECT owner_thread_id, target_thread_id, created_at_ms
                 FROM user_thread_links WHERE owner_thread_id = ?1
                 ORDER BY created_at_ms, target_thread_id",
                Some(required_control_text(owner_thread_id, "owner thread id")?),
            ),
            None => (
                "SELECT owner_thread_id, target_thread_id, created_at_ms
                 FROM user_thread_links ORDER BY owner_thread_id, created_at_ms, target_thread_id",
                None,
            ),
        };
        let mut stmt = db.conn().prepare(sql).map_err(sqlite)?;
        let rows = match owner_thread_id {
            Some(owner_thread_id) => stmt
                .query_map(params![owner_thread_id], control_thread_link_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?,
            None => stmt
                .query_map([], control_thread_link_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?,
        };
        Ok(rows)
    }

    pub fn control_add_thread_link(
        &self,
        owner_thread_id: &str,
        target_thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ControlTimelineRecord>> {
        let owner_thread_id = required_control_text(owner_thread_id, "owner thread id")?;
        let target_thread_id = required_control_text(target_thread_id, "target thread id")?;
        let item_id = required_control_text(item_id, "timeline item id")?;
        if owner_thread_id == target_thread_id {
            return Err(Error::InvalidRequest(
                "a thread cannot link to itself".into(),
            ));
        }
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<Option<ControlTimelineRecord>> {
            let now = now_ms();
            let changed = conn
                .execute(
                    "INSERT OR IGNORE INTO user_thread_links
                     (owner_thread_id, target_thread_id, created_at_ms) VALUES (?1, ?2, ?3)",
                    params![owner_thread_id, target_thread_id, now],
                )
                .map_err(sqlite)?;
            if changed == 0 {
                return Ok(None);
            }
            append_control_timeline_locked(
                conn,
                owner_thread_id,
                item_id,
                None,
                "thread_link_added",
                &serde_json::json!({ "target_thread_id": target_thread_id }).to_string(),
                now,
            )
            .map(Some)
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_remove_thread_link(
        &self,
        owner_thread_id: &str,
        target_thread_id: &str,
        item_id: &str,
    ) -> Result<Option<ControlTimelineRecord>> {
        let owner_thread_id = required_control_text(owner_thread_id, "owner thread id")?;
        let target_thread_id = required_control_text(target_thread_id, "target thread id")?;
        let item_id = required_control_text(item_id, "timeline item id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<Option<ControlTimelineRecord>> {
            let changed = conn
                .execute(
                    "DELETE FROM user_thread_links
                     WHERE owner_thread_id = ?1 AND target_thread_id = ?2",
                    params![owner_thread_id, target_thread_id],
                )
                .map_err(sqlite)?;
            if changed == 0 {
                return Ok(None);
            }
            let now = now_ms();
            append_control_timeline_locked(
                conn,
                owner_thread_id,
                item_id,
                None,
                "thread_link_removed",
                &serde_json::json!({ "target_thread_id": target_thread_id }).to_string(),
                now,
            )
            .map(Some)
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_timeline_max_seq(&self, thread_id: &str) -> Result<Option<(String, u64)>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT epoch, MAX(next_seq - 1, 0) FROM user_thread_control WHERE thread_id = ?1",
                params![thread_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(sqlite)?
            .map(|(epoch, seq)| Ok((epoch, i64_to_u64(seq, "timeline sequence")?)))
            .transpose()
    }

    pub fn control_visible_timeline_messages(
        &self,
        thread_id: &str,
        epoch: &str,
        max_seq: u64,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ControlTimelineRecord>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let epoch = required_control_text(epoch, "thread epoch")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        query_timeline_rows(
            db.conn(),
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2 AND item_type = 'message'
               AND seq <= ?3 AND seq > ?4
             ORDER BY seq ASC LIMIT ?5",
            params![
                thread_id,
                epoch,
                u64_to_i64(max_seq)?,
                u64_to_i64(after_seq.unwrap_or_default())?,
                i64::try_from(limit.clamp(1, 500))
                    .map_err(|_| Error::InvalidRequest("message limit is too large".into()))?
            ],
        )
    }

    pub fn control_append_message(&self, thread_id: &str, message_json: &str) -> Result<usize> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        validate_control_json(message_json, "message")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<usize> {
            let index: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(message_index), -1) + 1
                     FROM user_session_messages WHERE session_id = ?1",
                    params![thread_id],
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            conn.execute(
                "INSERT INTO user_session_messages
                 (session_id, message_index, message_json) VALUES (?1, ?2, ?3)",
                params![thread_id, index, message_json],
            )
            .map_err(sqlite)?;
            bump_thread_revision_locked(conn, thread_id)?;
            usize::try_from(index)
                .map_err(|_| Error::Other("message index is outside usize range".into()))
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_messages(&self, thread_id: &str) -> Result<Vec<String>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT message_json FROM user_session_messages
                 WHERE session_id = ?1 ORDER BY message_index ASC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map(params![thread_id], |row| row.get::<_, String>(0))
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    /// Rebuild the provider-visible transcript from the canonical timeline.
    /// Compatibility message rows remain a projection, never the replay
    /// authority for new model steps.
    pub fn control_projected_messages(&self, thread_id: &str) -> Result<Vec<String>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT item_type, data_json
                 FROM user_timeline_events
                 WHERE thread_id = ?1 AND item_type IN ('message', 'message_deleted')
                 ORDER BY seq ASC",
            )
            .map_err(sqlite)?;
        let events = stmt
            .query_map(params![thread_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(sqlite)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)?;
        let mut messages = Vec::<(String, String)>::new();
        for (event_type, data_json) in events {
            let data: serde_json::Value = serde_json::from_str(&data_json)
                .map_err(|error| Error::Other(format!("invalid timeline message JSON: {error}")))?;
            if event_type == "message" {
                if let Some(id) = data.get("id").and_then(serde_json::Value::as_str) {
                    if let Some(existing) =
                        messages.iter_mut().find(|(message_id, _)| message_id == id)
                    {
                        existing.1 = data_json;
                    } else {
                        messages.push((id.to_string(), data_json));
                    }
                }
            } else if let Some(id) = data.get("message_id").and_then(serde_json::Value::as_str) {
                messages.retain(|(message_id, _)| message_id != id);
            }
        }
        Ok(messages.into_iter().map(|(_, json)| json).collect())
    }

    /// Delete one canonical user-session message by its stable JSON `id` (or
    /// a replica's retained `canonicalId`) and compact legacy positional indices.
    pub fn control_delete_message(&self, thread_id: &str, message_id: &str) -> Result<bool> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let message_id = required_control_text(message_id, "message id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<bool> {
            let mut stmt = conn
                .prepare(
                    "SELECT message_index, message_json FROM user_session_messages
                     WHERE session_id = ?1 ORDER BY message_index ASC",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map(params![thread_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            drop(stmt);
            let Some((index, _)) = rows.iter().find(|(_, raw)| {
                serde_json::from_str::<serde_json::Value>(raw)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("canonicalId")
                            .or_else(|| value.get("id"))
                            .and_then(|id| id.as_str())
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(message_id)
            }) else {
                return Ok(false);
            };
            conn.execute(
                "DELETE FROM user_session_messages
                 WHERE session_id = ?1 AND message_index = ?2",
                params![thread_id, index],
            )
            .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_session_messages SET message_index = message_index - 1
                 WHERE session_id = ?1 AND message_index > ?2",
                params![thread_id, index],
            )
            .map_err(sqlite)?;
            bump_thread_revision_locked(conn, thread_id)?;
            Ok(true)
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_run(&self, run_id: &str) -> Result<Option<ControlRunRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT id, thread_id, status, adapter, request_json, agent_snapshot_json,
                        native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json
                 FROM user_runs WHERE id = ?1",
                params![run_id],
                control_run_from_row,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn control_put_run(&self, run: &ControlRunRecord) -> Result<()> {
        validate_control_json(&run.request_json, "run request")?;
        validate_optional_control_json(run.agent_snapshot_json.as_deref(), "agent snapshot")?;
        validate_optional_control_json(run.native_session_json.as_deref(), "native session")?;
        validate_optional_control_json(run.error_json.as_deref(), "run error")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_runs
                 (id, thread_id, status, adapter, request_json, agent_snapshot_json,
                  native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                    status = excluded.status,
                    native_session_json = excluded.native_session_json,
                    updated_at_ms = excluded.updated_at_ms,
                    completed_at_ms = excluded.completed_at_ms,
                    error_json = excluded.error_json",
                params![
                    run.id,
                    run.thread_id,
                    run.status,
                    run.adapter,
                    run.request_json,
                    run.agent_snapshot_json,
                    run.native_session_json,
                    run.created_at_ms,
                    run.updated_at_ms,
                    run.completed_at_ms,
                    run.error_json,
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_runs(&self, nonterminal_only: bool) -> Result<Vec<ControlRunRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let sql = if nonterminal_only {
            "SELECT id, thread_id, status, adapter, request_json, agent_snapshot_json,
                    native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json
             FROM user_runs WHERE status IN ('accepted', 'running', 'waiting_approval', 'stopping')
             ORDER BY created_at_ms ASC"
        } else {
            "SELECT id, thread_id, status, adapter, request_json, agent_snapshot_json,
                    native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json
             FROM user_runs ORDER BY created_at_ms ASC"
        };
        let mut stmt = db.conn().prepare(sql).map_err(sqlite)?;
        let rows = stmt.query_map([], control_run_from_row).map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_append_timeline(
        &self,
        thread_id: &str,
        item_id: &str,
        run_id: Option<&str>,
        item_type: &str,
        data_json: &str,
    ) -> Result<ControlTimelineRecord> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let item_id = required_control_text(item_id, "timeline item id")?;
        let item_type = required_control_text(item_type, "timeline item type")?;
        validate_control_json(data_json, "timeline data")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<ControlTimelineRecord> {
            let (epoch, seq): (String, i64) = conn
                .query_row(
                    "SELECT epoch, next_seq FROM user_thread_control WHERE thread_id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite)?;
            let now = now_ms();
            conn.execute(
                "INSERT INTO user_timeline_events
                 (thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![thread_id, epoch, seq, item_id, run_id, item_type, data_json, now],
            )
            .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_thread_control
                 SET next_seq = ?2, revision = revision + 1, updated_at_ms = ?3
                 WHERE thread_id = ?1",
                params![thread_id, seq.saturating_add(1), now],
            )
            .map_err(sqlite)?;
            Ok(ControlTimelineRecord {
                thread_id: thread_id.to_string(),
                epoch,
                seq: i64_to_u64(seq, "timeline sequence")?,
                item_id: item_id.to_string(),
                run_id: run_id.map(str::to_string),
                item_type: item_type.to_string(),
                data_json: data_json.to_string(),
                created_at_ms: now,
            })
        })();
        finish_control_transaction(conn, result)
    }

    /// Atomically append a compatibility message row, its canonical timeline
    /// projection, and the immutable ledger event that accounts for it.
    #[allow(clippy::too_many_arguments)]
    pub fn control_commit_message_projection_and_event(
        &self,
        thread_id: &str,
        run_id: &str,
        item_id: &str,
        message_json: &str,
        event_id: &str,
        step_id: Option<&str>,
        event_type: &str,
        event_data_json: &str,
    ) -> Result<(ControlTimelineRecord, ControlRunEventRecord)> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let run_id = required_control_text(run_id, "run id")?;
        let item_id = required_control_text(item_id, "timeline item id")?;
        let event_id = required_control_text(event_id, "run event id")?;
        let event_type = required_control_text(event_type, "run event type")?;
        validate_control_json(message_json, "message projection")?;
        validate_control_json(event_data_json, "run event")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<(ControlTimelineRecord, ControlRunEventRecord)> {
            let message_index: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(message_index), -1) + 1
                     FROM user_session_messages WHERE session_id = ?1",
                    params![thread_id],
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            conn.execute(
                "INSERT INTO user_session_messages
                 (session_id, message_index, message_json) VALUES (?1, ?2, ?3)",
                params![thread_id, message_index, message_json],
            )
            .map_err(sqlite)?;

            let (epoch, timeline_seq): (String, i64) = conn
                .query_row(
                    "SELECT epoch, next_seq FROM user_thread_control WHERE thread_id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite)?;
            let created_at_ms = now_ms();
            conn.execute(
                "INSERT INTO user_timeline_events
                 (thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'message', ?6, ?7)",
                params![
                    thread_id,
                    epoch,
                    timeline_seq,
                    item_id,
                    run_id,
                    message_json,
                    created_at_ms
                ],
            )
            .map_err(sqlite)?;

            let event_seq: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM user_run_events WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            conn.execute(
                "INSERT INTO user_run_events
                 (run_id, seq, event_id, step_id, event_type, data_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    run_id,
                    event_seq,
                    event_id,
                    step_id,
                    event_type,
                    event_data_json,
                    created_at_ms
                ],
            )
            .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_thread_control
                 SET next_seq = ?2, revision = revision + 2, updated_at_ms = ?3
                 WHERE thread_id = ?1",
                params![thread_id, timeline_seq.saturating_add(1), created_at_ms],
            )
            .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_sessions SET updated_at_ms = ?2 WHERE id = ?1",
                params![thread_id, created_at_ms],
            )
            .map_err(sqlite)?;

            Ok((
                ControlTimelineRecord {
                    thread_id: thread_id.to_string(),
                    epoch,
                    seq: i64_to_u64(timeline_seq, "timeline sequence")?,
                    item_id: item_id.to_string(),
                    run_id: Some(run_id.to_string()),
                    item_type: "message".to_string(),
                    data_json: message_json.to_string(),
                    created_at_ms,
                },
                ControlRunEventRecord {
                    run_id: run_id.to_string(),
                    seq: i64_to_u64(event_seq, "run event sequence")?,
                    event_id: event_id.to_string(),
                    step_id: step_id.map(str::to_string),
                    event_type: event_type.to_string(),
                    data_json: event_data_json.to_string(),
                    created_at_ms,
                },
            ))
        })();
        finish_control_transaction(conn, result)
    }

    /// Seed an existing desktop transcript into the canonical timeline before
    /// the first control event is appended. The operation is idempotent and
    /// intentionally leaves the thread revision unchanged.
    pub fn control_seed_message_timeline_if_empty(
        &self,
        thread_id: &str,
        messages: &[(String, String, i64)],
    ) -> Result<usize> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        for (item_id, data_json, _) in messages {
            required_control_text(item_id, "timeline item id")?;
            validate_control_json(data_json, "timeline data")?;
        }
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<usize> {
            let (epoch, next_seq): (String, i64) = conn
                .query_row(
                    "SELECT epoch, next_seq FROM user_thread_control WHERE thread_id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite)?;
            let has_timeline: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM user_timeline_events
                        WHERE thread_id = ?1 AND epoch = ?2
                     )",
                    params![thread_id, epoch],
                    |row| row.get(0),
                )
                .map_err(sqlite)?;
            if has_timeline || messages.is_empty() {
                return Ok(0);
            }

            let mut seq = next_seq;
            for (item_id, data_json, created_at_ms) in messages {
                conn.execute(
                    "INSERT INTO user_timeline_events
                     (thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, NULL, 'message', ?5, ?6)",
                    params![thread_id, epoch, seq, item_id, data_json, created_at_ms],
                )
                .map_err(sqlite)?;
                seq = seq.saturating_add(1);
            }
            conn.execute(
                "UPDATE user_thread_control SET next_seq = ?2 WHERE thread_id = ?1",
                params![thread_id, seq],
            )
            .map_err(sqlite)?;
            Ok(messages.len())
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_timeline_page(
        &self,
        thread_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        tail: bool,
        limit: usize,
    ) -> Result<Option<ControlTimelinePage>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        if after_seq.is_some() && before_seq.is_some() {
            return Err(Error::InvalidRequest(
                "timeline reads cannot combine after_seq and before_seq".into(),
            ));
        }
        let limit = limit.clamp(1, 500);
        let page = {
            let db = self
                .read_db()
                .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
            query_control_timeline_page(db.conn(), thread_id, after_seq, before_seq, tail, limit)?
        };
        if page.is_some() {
            return Ok(page);
        }

        // A pooled reader can briefly retain a pre-commit WAL snapshot when a
        // session has just been imported or attached. Confirm a miss against
        // the ordered writer before reporting that the thread does not exist.
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        query_control_timeline_page(db.conn(), thread_id, after_seq, before_seq, tail, limit)
    }

    pub fn control_enqueue_turn(&self, turn: &ControlQueuedTurnRecord) -> Result<()> {
        validate_control_json(&turn.request_json, "queued turn request")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_run_inbox
                 (id, thread_id, target_run_id, command_id, kind, state, payload_json,
                  created_at_ms, claimed_at_ms, resolved_at_ms, sort_key)
                 VALUES (?1, ?2, NULL, ?3, 'followup', 'pending', ?4, ?5, NULL, NULL, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    thread_id = excluded.thread_id,
                    target_run_id = NULL,
                    command_id = excluded.command_id,
                    kind = 'followup',
                    state = 'pending',
                    payload_json = excluded.payload_json,
                    created_at_ms = excluded.created_at_ms,
                    sort_key = COALESCE(user_run_inbox.sort_key, excluded.sort_key),
                    claimed_at_ms = NULL,
                    resolved_at_ms = NULL",
                params![
                    turn.id,
                    turn.thread_id,
                    turn.command_id,
                    turn.request_json,
                    turn.accepted_at_ms
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_queued_turns(
        &self,
        thread_id: Option<&str>,
    ) -> Result<Vec<ControlQueuedTurnRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let (sql, value) = match thread_id {
            Some(id) => (
                "SELECT id, thread_id, COALESCE(command_id, id), payload_json, created_at_ms
                 FROM user_run_inbox
                 WHERE kind = 'followup' AND state = 'pending' AND thread_id = ?1
                 ORDER BY COALESCE(sort_key, created_at_ms) ASC, created_at_ms ASC, id ASC",
                Some(required_control_text(id, "thread id")?),
            ),
            None => (
                "SELECT id, thread_id, COALESCE(command_id, id), payload_json, created_at_ms
                 FROM user_run_inbox
                 WHERE kind = 'followup' AND state = 'pending'
                 ORDER BY COALESCE(sort_key, created_at_ms) ASC, created_at_ms ASC, id ASC",
                None,
            ),
        };
        let mut stmt = db.conn().prepare(sql).map_err(sqlite)?;
        let rows = if let Some(value) = value {
            stmt.query_map(params![value], control_queued_turn_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?
        } else {
            stmt.query_map([], control_queued_turn_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?
        };
        Ok(rows)
    }

    pub fn control_move_queued_turn(
        &self,
        thread_id: &str,
        id: &str,
        target_id: &str,
        after: bool,
    ) -> Result<bool> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let id = required_control_text(id, "queued turn id")?;
        let target_id = required_control_text(target_id, "target queued turn id")?;
        if id == target_id {
            return Ok(false);
        }
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<bool> {
            let mut statement = conn
                .prepare(
                    "SELECT id
                     FROM user_run_inbox
                     WHERE kind = 'followup' AND state = 'pending' AND thread_id = ?1
                     ORDER BY COALESCE(sort_key, created_at_ms) ASC, created_at_ms ASC, id ASC",
                )
                .map_err(sqlite)?;
            let mut ids = statement
                .query_map(params![thread_id], |row| row.get::<_, String>(0))
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            let Some(source_index) = ids.iter().position(|item| item == id) else {
                return Ok(false);
            };
            if !ids.iter().any(|item| item == target_id) {
                return Ok(false);
            }
            let moved = ids.remove(source_index);
            let target_index = ids
                .iter()
                .position(|item| item == target_id)
                .expect("target checked before queue move");
            ids.insert(target_index + usize::from(after), moved);
            let base = now_ms().saturating_sub(i64::try_from(ids.len()).unwrap_or(i64::MAX));
            for (index, queued_id) in ids.iter().enumerate() {
                let sort_key = base.saturating_add(i64::try_from(index).unwrap_or(i64::MAX));
                conn.execute(
                    "UPDATE user_run_inbox SET sort_key = ?2
                     WHERE id = ?1 AND kind = 'followup' AND state = 'pending'",
                    params![queued_id, sort_key],
                )
                .map_err(sqlite)?;
            }
            Ok(true)
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_remove_queued_turn(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "queued turn id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_run_inbox
                 SET state = 'claimed', claimed_at_ms = ?2, resolved_at_ms = ?2
                 WHERE id = ?1 AND kind = 'followup' AND state = 'pending'",
                params![id, now_ms()],
            )
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_append_run_event(
        &self,
        run_id: &str,
        event_id: &str,
        step_id: Option<&str>,
        event_type: &str,
        data_json: &str,
    ) -> Result<ControlRunEventRecord> {
        let run_id = required_control_text(run_id, "run id")?;
        let event_id = required_control_text(event_id, "run event id")?;
        let event_type = required_control_text(event_type, "run event type")?;
        validate_control_json(data_json, "run event")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<ControlRunEventRecord> {
            let seq = conn
                .query_row(
                    "SELECT COALESCE(MAX(seq), 0) + 1 FROM user_run_events WHERE run_id = ?1",
                    params![run_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(sqlite)?;
            let created_at_ms = now_ms();
            conn.execute(
                "INSERT INTO user_run_events
                 (run_id, seq, event_id, step_id, event_type, data_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    run_id,
                    seq,
                    event_id,
                    step_id,
                    event_type,
                    data_json,
                    created_at_ms
                ],
            )
            .map_err(sqlite)?;
            Ok(ControlRunEventRecord {
                run_id: run_id.to_string(),
                seq: i64_to_u64(seq, "run event sequence")?,
                event_id: event_id.to_string(),
                step_id: step_id.map(str::to_string),
                event_type: event_type.to_string(),
                data_json: data_json.to_string(),
                created_at_ms,
            })
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_run_events(
        &self,
        run_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ControlRunEventRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let after_seq = u64_to_i64(after_seq.unwrap_or_default())?;
        let limit = i64::try_from(limit.clamp(1, 500))
            .map_err(|_| Error::InvalidRequest("run event limit is too large".into()))?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT run_id, seq, event_id, step_id, event_type, data_json, created_at_ms
                 FROM user_run_events
                 WHERE run_id = ?1 AND seq > ?2
                 ORDER BY seq ASC LIMIT ?3",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map(
                params![run_id, after_seq, limit],
                control_run_event_from_row,
            )
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_run_events_for_step(
        &self,
        run_id: &str,
        step_id: &str,
    ) -> Result<Vec<ControlRunEventRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let step_id = required_control_text(step_id, "step id")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT run_id, seq, event_id, step_id, event_type, data_json, created_at_ms
                 FROM user_run_events
                 WHERE run_id = ?1 AND step_id = ?2 ORDER BY seq ASC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map(params![run_id, step_id], control_run_event_from_row)
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_put_run_artifact(&self, artifact: &ControlRunArtifactRecord) -> Result<()> {
        validate_control_json(&artifact.data_json, "run artifact")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        put_run_artifact_locked(db.conn(), artifact)
    }

    pub fn control_run_artifact(
        &self,
        run_id: &str,
        digest: &str,
    ) -> Result<Option<ControlRunArtifactRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let digest = required_control_text(digest, "artifact digest")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let reference = conn
            .query_row(
                "SELECT kind, created_at_ms, byte_len
                 FROM user_run_artifact_refs
                 JOIN user_run_artifact_blobs USING (digest)
                 WHERE run_id = ?1 AND digest = ?2",
                params![run_id, digest],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(sqlite)?;
        if let Some((kind, created_at_ms, byte_len)) = reference {
            let data = decode_run_artifact_blob_locked(conn, digest, 0)?;
            let data_json = String::from_utf8(data)
                .map_err(|error| Error::Other(format!("decode run artifact JSON: {error}")))?;
            return Ok(Some(ControlRunArtifactRecord {
                run_id: run_id.to_string(),
                digest: digest.to_string(),
                kind,
                data_json,
                byte_len: u64::try_from(byte_len)
                    .map_err(|_| Error::Other("run artifact byte length is negative".into()))?,
                created_at_ms,
            }));
        }
        conn.query_row(
            "SELECT run_id, digest, kind, data_json, byte_len, created_at_ms
             FROM user_run_artifacts WHERE run_id = ?1 AND digest = ?2",
            params![run_id, digest],
            control_run_artifact_from_row,
        )
        .optional()
        .map_err(sqlite)
    }

    pub fn control_run_artifacts_by_kind(
        &self,
        run_id: &str,
        kind: &str,
    ) -> Result<Vec<ControlRunArtifactRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let kind = required_control_text(kind, "artifact kind")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let refs = {
            let mut stmt = conn
                .prepare(
                    "SELECT digest, created_at_ms FROM user_run_artifact_refs
                     WHERE run_id = ?1 AND kind = ?2 ORDER BY created_at_ms, digest",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map(params![run_id, kind], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let mut artifacts = Vec::with_capacity(refs.len());
        for (digest, created_at_ms) in &refs {
            let data = decode_run_artifact_blob_locked(conn, digest, 0)?;
            artifacts.push(ControlRunArtifactRecord {
                run_id: run_id.to_string(),
                digest: digest.clone(),
                kind: kind.to_string(),
                byte_len: u64::try_from(data.len()).unwrap_or(u64::MAX),
                data_json: String::from_utf8(data)
                    .map_err(|error| Error::Other(format!("decode run artifact JSON: {error}")))?,
                created_at_ms: *created_at_ms,
            });
        }
        let migrated = refs
            .into_iter()
            .map(|(digest, _)| digest)
            .collect::<BTreeSet<_>>();
        let mut legacy = conn
            .prepare(
                "SELECT run_id, digest, kind, data_json, byte_len, created_at_ms
                 FROM user_run_artifacts
                 WHERE run_id = ?1 AND kind = ?2 ORDER BY created_at_ms, digest",
            )
            .map_err(sqlite)?;
        let rows = legacy
            .query_map(params![run_id, kind], control_run_artifact_from_row)
            .map_err(sqlite)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)?;
        artifacts.extend(
            rows.into_iter()
                .filter(|artifact| !migrated.contains(&artifact.digest)),
        );
        artifacts.sort_by_key(|artifact| artifact.created_at_ms);
        Ok(artifacts)
    }

    pub fn control_run_artifacts_by_digests(
        &self,
        run_id: &str,
        digests: &[String],
    ) -> Result<Vec<ControlRunArtifactRecord>> {
        let mut artifacts = Vec::with_capacity(digests.len());
        for digest in digests {
            if let Some(artifact) = self.control_run_artifact(run_id, digest)? {
                artifacts.push(artifact);
            }
        }
        Ok(artifacts)
    }

    pub fn control_run_artifacts(&self, run_id: &str) -> Result<Vec<ControlRunArtifactRecord>> {
        let run_id = required_control_text(run_id, "run id")?;
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT digest, kind, created_at_ms, byte_len
                 FROM user_run_artifact_refs
                 JOIN user_run_artifact_blobs USING (digest)
                 WHERE run_id = ?1 ORDER BY created_at_ms, digest",
            )
            .map_err(sqlite)?;
        let refs = stmt
            .query_map(params![run_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(sqlite)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)?;
        drop(stmt);
        let mut artifacts = Vec::with_capacity(refs.len());
        let mut migrated = BTreeSet::new();
        for (digest, kind, created_at_ms, byte_len) in refs {
            let data = decode_run_artifact_blob_locked(conn, &digest, 0)?;
            let data_json = String::from_utf8(data)
                .map_err(|error| Error::Other(format!("decode run artifact JSON: {error}")))?;
            migrated.insert(digest.clone());
            artifacts.push(ControlRunArtifactRecord {
                run_id: run_id.to_string(),
                digest,
                kind,
                data_json,
                byte_len: u64::try_from(byte_len)
                    .map_err(|_| Error::Other("run artifact byte length is negative".into()))?,
                created_at_ms,
            });
        }
        let mut legacy = conn
            .prepare(
                "SELECT run_id, digest, kind, data_json, byte_len, created_at_ms
                 FROM user_run_artifacts WHERE run_id = ?1 ORDER BY created_at_ms, digest",
            )
            .map_err(sqlite)?;
        let rows = legacy
            .query_map(params![run_id], control_run_artifact_from_row)
            .map_err(sqlite)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)?;
        artifacts.extend(
            rows.into_iter()
                .filter(|artifact| !migrated.contains(&artifact.digest)),
        );
        artifacts.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.digest.cmp(&right.digest))
        });
        Ok(artifacts)
    }

    pub fn control_has_active_runs(&self) -> Result<bool> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM user_runs WHERE status IN ('accepted', 'running')
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(sqlite)
    }

    pub fn migrate_run_artifacts_batch(
        &self,
        max_raw_bytes: usize,
    ) -> Result<RunArtifactMigrationProgress> {
        let max_raw_bytes = max_raw_bytes.max(1);
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let rows = {
            let mut stmt = conn
                .prepare(
                    "SELECT legacy.run_id, legacy.digest, legacy.kind, legacy.data_json,
                            legacy.byte_len, legacy.created_at_ms
                     FROM user_run_artifacts AS legacy
                     LEFT JOIN user_run_artifact_refs AS refs
                       ON refs.run_id = legacy.run_id AND refs.digest = legacy.digest
                     WHERE refs.digest IS NULL
                     ORDER BY legacy.run_id, legacy.created_at_ms, legacy.digest
                     LIMIT 256",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_run_artifact_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let mut selected = Vec::new();
        let mut raw_bytes = 0usize;
        for artifact in rows {
            let next = artifact.data_json.len();
            if !selected.is_empty() && raw_bytes.saturating_add(next) > max_raw_bytes {
                break;
            }
            raw_bytes = raw_bytes.saturating_add(next);
            selected.push(artifact);
        }
        if !selected.is_empty() {
            conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
            let result = (|| -> Result<()> {
                for artifact in &selected {
                    put_run_artifact_locked(conn, artifact)?;
                    conn.execute(
                        "DELETE FROM user_run_artifacts WHERE run_id = ?1 AND digest = ?2",
                        params![artifact.run_id, artifact.digest],
                    )
                    .map_err(sqlite)?;
                }
                Ok(())
            })();
            finish_control_transaction(conn, result)?;
        }
        let remaining = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM user_run_artifacts AS legacy
                 LEFT JOIN user_run_artifact_refs AS refs
                   ON refs.run_id = legacy.run_id AND refs.digest = legacy.digest
                 WHERE refs.digest IS NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sqlite)?;
        Ok(RunArtifactMigrationProgress {
            migrated: selected.len(),
            raw_bytes: u64::try_from(raw_bytes).unwrap_or(u64::MAX),
            remaining: usize::try_from(remaining)
                .map_err(|_| Error::Other("artifact migration count is negative".into()))?,
        })
    }

    pub fn control_put_inbox(&self, item: &ControlInboxRecord) -> Result<()> {
        if !matches!(item.kind.as_str(), "followup" | "steer" | "inject") {
            return Err(Error::InvalidRequest("invalid inbox kind".into()));
        }
        if !matches!(
            item.state.as_str(),
            "pending" | "claimed" | "cancelled" | "discarded"
        ) {
            return Err(Error::InvalidRequest("invalid inbox state".into()));
        }
        validate_control_json(&item.payload_json, "inbox payload")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_run_inbox
                 (id, thread_id, target_run_id, command_id, kind, state, payload_json,
                  created_at_ms, claimed_at_ms, resolved_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    item.id,
                    item.thread_id,
                    item.target_run_id,
                    item.command_id,
                    item.kind,
                    item.state,
                    item.payload_json,
                    item.created_at_ms,
                    item.claimed_at_ms,
                    item.resolved_at_ms
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_pending_inbox(
        &self,
        thread_id: Option<&str>,
    ) -> Result<Vec<ControlInboxRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let (sql, thread_id) = match thread_id {
            Some(thread_id) => (
                "SELECT id, thread_id, target_run_id, command_id, kind, state, payload_json,
                        created_at_ms, claimed_at_ms, resolved_at_ms
                 FROM user_run_inbox WHERE state = 'pending' AND thread_id = ?1
                 ORDER BY created_at_ms, id",
                Some(required_control_text(thread_id, "thread id")?),
            ),
            None => (
                "SELECT id, thread_id, target_run_id, command_id, kind, state, payload_json,
                        created_at_ms, claimed_at_ms, resolved_at_ms
                 FROM user_run_inbox WHERE state = 'pending'
                 ORDER BY created_at_ms, id",
                None,
            ),
        };
        let mut stmt = db.conn().prepare(sql).map_err(sqlite)?;
        let rows = match thread_id {
            Some(thread_id) => stmt
                .query_map(params![thread_id], control_inbox_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?,
            None => stmt
                .query_map([], control_inbox_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?,
        };
        Ok(rows)
    }

    pub fn control_claim_step_inputs(
        &self,
        thread_id: &str,
        run_id: &str,
    ) -> Result<Vec<ControlInboxRecord>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        let run_id = required_control_text(run_id, "run id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<Vec<ControlInboxRecord>> {
            let mut stmt = conn
                .prepare(
                    "SELECT id, thread_id, target_run_id, command_id, kind, state, payload_json,
                            created_at_ms, claimed_at_ms, resolved_at_ms
                     FROM user_run_inbox
                     WHERE thread_id = ?1 AND state = 'pending'
                       AND ((kind = 'steer' AND target_run_id = ?2) OR kind = 'inject')
                     ORDER BY created_at_ms, id",
                )
                .map_err(sqlite)?;
            let items = stmt
                .query_map(params![thread_id, run_id], control_inbox_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            drop(stmt);
            let now = now_ms();
            for item in &items {
                conn.execute(
                    "UPDATE user_run_inbox SET state = 'claimed', claimed_at_ms = ?2, resolved_at_ms = ?2
                     WHERE id = ?1 AND state = 'pending'",
                    params![item.id, now],
                )
                .map_err(sqlite)?;
            }
            Ok(items
                .into_iter()
                .map(|mut item| {
                    item.state = "claimed".into();
                    item.claimed_at_ms = Some(now);
                    item.resolved_at_ms = Some(now);
                    item
                })
                .collect())
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_cancel_inbox(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "inbox id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_run_inbox SET state = 'cancelled', resolved_at_ms = ?2
                 WHERE id = ?1 AND state = 'pending'",
                params![id, now_ms()],
            )
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_discard_inbox(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "inbox id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_run_inbox SET state = 'discarded', resolved_at_ms = ?2
                 WHERE id = ?1 AND state = 'pending'",
                params![id, now_ms()],
            )
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_retarget_pending_steers(&self, run_id: &str) -> Result<usize> {
        let run_id = required_control_text(run_id, "run id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_run_inbox
                 SET kind = 'followup', target_run_id = NULL
                 WHERE kind = 'steer' AND state = 'pending' AND target_run_id = ?1",
                params![run_id],
            )
            .map_err(sqlite)
    }

    pub fn control_put_mailbox(&self, item: &ControlMailboxRecord) -> Result<()> {
        if !matches!(
            item.status.as_str(),
            "queued" | "running" | "replied" | "failed" | "discarded"
        ) {
            return Err(Error::InvalidRequest("invalid mailbox status".into()));
        }
        validate_control_json(&item.request_json, "mailbox request")?;
        validate_optional_control_json(item.reply_json.as_deref(), "mailbox reply")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_thread_mailbox
                 (id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                  status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                  projected_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                    origin_run_id = COALESCE(excluded.origin_run_id, user_thread_mailbox.origin_run_id),
                    target_run_id = COALESCE(excluded.target_run_id, user_thread_mailbox.target_run_id),
                    status = excluded.status,
                    reply_json = excluded.reply_json,
                    updated_at_ms = excluded.updated_at_ms,
                    consumed_at_ms = excluded.consumed_at_ms,
                    projected_at_ms = excluded.projected_at_ms",
                params![
                    item.id,
                    item.origin_thread_id,
                    item.target_thread_id,
                    item.origin_run_id,
                    item.target_run_id,
                    item.status,
                    item.request_json,
                    item.reply_json,
                    item.created_at_ms,
                    item.updated_at_ms,
                    item.consumed_at_ms,
                    item.projected_at_ms
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_mailbox(&self, id: &str) -> Result<Option<ControlMailboxRecord>> {
        let id = required_control_text(id, "mailbox id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                        status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                        projected_at_ms
                 FROM user_thread_mailbox WHERE id = ?1",
                params![id],
                control_mailbox_from_row,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn control_mailbox_for_origin(
        &self,
        origin_thread_id: &str,
    ) -> Result<Vec<ControlMailboxRecord>> {
        let origin_thread_id = required_control_text(origin_thread_id, "origin thread id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                        status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                        projected_at_ms
                 FROM user_thread_mailbox WHERE origin_thread_id = ?1
                 ORDER BY created_at_ms, id",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map(params![origin_thread_id], control_mailbox_from_row)
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_mailbox_for_target_run(
        &self,
        target_run_id: &str,
    ) -> Result<Option<ControlMailboxRecord>> {
        let target_run_id = required_control_text(target_run_id, "target run id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                        status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                        projected_at_ms
                 FROM user_thread_mailbox WHERE target_run_id = ?1",
                params![target_run_id],
                control_mailbox_from_row,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn control_delete_mailbox(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "mailbox id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute("DELETE FROM user_thread_mailbox WHERE id = ?1", params![id])
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_mark_mailbox_projected(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "mailbox id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "UPDATE user_thread_mailbox
                 SET projected_at_ms = COALESCE(projected_at_ms, ?2), updated_at_ms = ?2
                 WHERE id = ?1 AND status IN ('replied', 'failed')",
                params![id, now_ms()],
            )
            .map(|changed| changed > 0)
            .map_err(sqlite)
    }

    pub fn control_claim_mailbox_replies(
        &self,
        origin_thread_id: &str,
        origin_run_id: &str,
        limit: usize,
    ) -> Result<Vec<ControlMailboxRecord>> {
        let origin_thread_id = required_control_text(origin_thread_id, "origin thread id")?;
        let origin_run_id = required_control_text(origin_run_id, "origin run id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<Vec<ControlMailboxRecord>> {
            let mut stmt = conn
                .prepare(
                    "SELECT id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                            status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                            projected_at_ms
                     FROM user_thread_mailbox
                     WHERE origin_thread_id = ?1 AND status IN ('replied', 'failed')
                       AND consumed_at_ms IS NULL
                     ORDER BY created_at_ms, id LIMIT ?2",
                )
                .map_err(sqlite)?;
            let mut items = stmt
                .query_map(
                    params![
                        origin_thread_id,
                        i64::try_from(limit.clamp(1, 50)).map_err(|_| {
                            Error::InvalidRequest("mailbox claim limit is too large".into())
                        })?
                    ],
                    control_mailbox_from_row,
                )
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            drop(stmt);
            let now = now_ms();
            for item in &items {
                conn.execute(
                    "UPDATE user_thread_mailbox
                     SET consumed_at_ms = ?2, origin_run_id = ?3, updated_at_ms = ?2
                     WHERE id = ?1 AND consumed_at_ms IS NULL",
                    params![item.id, now, origin_run_id],
                )
                .map_err(sqlite)?;
            }
            for item in &mut items {
                item.consumed_at_ms = Some(now);
                item.origin_run_id = Some(origin_run_id.to_string());
                item.updated_at_ms = now;
            }
            Ok(items)
        })();
        finish_control_transaction(conn, result)
    }

    pub fn control_command_receipt(
        &self,
        command_id: &str,
    ) -> Result<Option<ControlCommandReceiptRecord>> {
        let command_id = required_control_text(command_id, "command id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT command_id, device_id, thread_id, command_kind,
                        request_json, result_json, created_at_ms
                 FROM user_command_receipts WHERE command_id = ?1",
                params![command_id],
                control_command_receipt_from_row,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn control_put_command_receipt(&self, receipt: &ControlCommandReceiptRecord) -> Result<()> {
        validate_control_json(&receipt.request_json, "command request")?;
        validate_control_json(&receipt.result_json, "command result")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_command_receipts
                 (command_id, device_id, thread_id, command_kind, request_json, result_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    receipt.command_id,
                    receipt.device_id,
                    receipt.thread_id,
                    receipt.command_kind,
                    receipt.request_json,
                    receipt.result_json,
                    receipt.created_at_ms,
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_put_approval(&self, approval: &ControlApprovalRecord) -> Result<()> {
        validate_control_json(&approval.request_json, "approval request")?;
        validate_optional_control_json(approval.decision_json.as_deref(), "approval decision")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_pending_approvals
                 (id, run_id, thread_id, kind, request_json, status, decision_json,
                  created_at_ms, resolved_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    status = excluded.status,
                    decision_json = excluded.decision_json,
                    resolved_at_ms = excluded.resolved_at_ms",
                params![
                    approval.id,
                    approval.run_id,
                    approval.thread_id,
                    approval.kind,
                    approval.request_json,
                    approval.status,
                    approval.decision_json,
                    approval.created_at_ms,
                    approval.resolved_at_ms,
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    pub fn control_pending_approvals(&self) -> Result<Vec<ControlApprovalRecord>> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, run_id, thread_id, kind, request_json, status,
                        decision_json, created_at_ms, resolved_at_ms
                 FROM user_pending_approvals WHERE status = 'pending'
                 ORDER BY created_at_ms ASC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map([], control_approval_from_row)
            .map_err(sqlite)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite)
    }

    pub fn control_approval(&self, id: &str) -> Result<Option<ControlApprovalRecord>> {
        let id = required_control_text(id, "approval id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .query_row(
                "SELECT id, run_id, thread_id, kind, request_json, status,
                        decision_json, created_at_ms, resolved_at_ms
                 FROM user_pending_approvals WHERE id = ?1",
                params![id],
                control_approval_from_row,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn control_backup_state(&self) -> Result<ControlBackupState> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let host = conn
            .query_row(
                "SELECT host_id, display_name, created_at_ms, updated_at_ms
                 FROM user_control_host WHERE singleton = 1",
                [],
                |row| {
                    Ok(ControlHostRecord {
                        host_id: row.get(0)?,
                        display_name: row.get(1)?,
                        created_at_ms: row.get(2)?,
                        updated_at_ms: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(sqlite)?;
        let threads = {
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, s.session_json, c.revision, c.epoch, s.updated_at_ms
                     FROM user_sessions s JOIN user_thread_control c ON c.thread_id = s.id
                     ORDER BY s.sort_order ASC, s.updated_at_ms DESC",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_thread_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let runs = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, thread_id, status, adapter, request_json, agent_snapshot_json,
                            native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json
                     FROM user_runs ORDER BY created_at_ms ASC",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_run_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let timeline = query_timeline_rows(
            conn,
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events ORDER BY thread_id, epoch, seq",
            [],
        )?;
        let queued_turns = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, thread_id, COALESCE(command_id, id), payload_json, created_at_ms
                     FROM user_run_inbox
                     WHERE kind = 'followup' AND state = 'pending'
                     ORDER BY created_at_ms, id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_queued_turn_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let command_receipts = {
            let mut stmt = conn
                .prepare(
                    "SELECT command_id, device_id, thread_id, command_kind,
                            request_json, result_json, created_at_ms
                     FROM user_command_receipts ORDER BY created_at_ms, command_id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_command_receipt_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let approvals = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, run_id, thread_id, kind, request_json, status,
                            decision_json, created_at_ms, resolved_at_ms
                     FROM user_pending_approvals ORDER BY created_at_ms, id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_approval_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let run_events = {
            let mut stmt = conn
                .prepare(
                    "SELECT run_id, seq, event_id, step_id, event_type, data_json, created_at_ms
                     FROM user_run_events ORDER BY run_id, seq",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_run_event_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let run_artifacts = {
            let mut stmt = conn
                .prepare(
                    "SELECT refs.run_id, refs.digest, refs.kind, refs.created_at_ms, blobs.byte_len
                     FROM user_run_artifact_refs AS refs
                     JOIN user_run_artifact_blobs AS blobs USING (digest)
                     ORDER BY refs.run_id, refs.created_at_ms, refs.digest",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            drop(stmt);
            let mut artifacts = Vec::with_capacity(rows.len());
            let mut migrated = BTreeSet::new();
            for (run_id, digest, kind, created_at_ms, byte_len) in rows {
                let data = decode_run_artifact_blob_locked(conn, &digest, 0)?;
                migrated.insert((run_id.clone(), digest.clone()));
                artifacts.push(ControlRunArtifactRecord {
                    run_id,
                    digest,
                    kind,
                    data_json: String::from_utf8(data).map_err(|error| {
                        Error::Other(format!("decode run artifact JSON: {error}"))
                    })?,
                    byte_len: u64::try_from(byte_len)
                        .map_err(|_| Error::Other("run artifact byte length is negative".into()))?,
                    created_at_ms,
                });
            }
            let mut legacy = conn
                .prepare(
                    "SELECT run_id, digest, kind, data_json, byte_len, created_at_ms
                     FROM user_run_artifacts ORDER BY run_id, created_at_ms, digest",
                )
                .map_err(sqlite)?;
            let rows = legacy
                .query_map([], control_run_artifact_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            artifacts.extend(rows.into_iter().filter(|artifact| {
                !migrated.contains(&(artifact.run_id.clone(), artifact.digest.clone()))
            }));
            artifacts
        };
        let inbox = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, thread_id, target_run_id, command_id, kind, state, payload_json,
                            created_at_ms, claimed_at_ms, resolved_at_ms
                     FROM user_run_inbox ORDER BY created_at_ms, id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_inbox_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let thread_links = {
            let mut stmt = conn
                .prepare(
                    "SELECT owner_thread_id, target_thread_id, created_at_ms
                     FROM user_thread_links ORDER BY owner_thread_id, created_at_ms, target_thread_id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_thread_link_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        let mailbox = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                            status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                            projected_at_ms
                     FROM user_thread_mailbox ORDER BY created_at_ms, id",
                )
                .map_err(sqlite)?;
            let rows = stmt
                .query_map([], control_mailbox_from_row)
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            rows
        };
        Ok(ControlBackupState {
            schema_version: 3,
            host,
            threads,
            runs,
            timeline,
            queued_turns,
            command_receipts,
            approvals,
            run_events,
            run_artifacts,
            inbox,
            thread_links,
            mailbox,
        })
    }

    pub fn replace_control_backup_state(&self, backup: &ControlBackupState) -> Result<()> {
        if !matches!(backup.schema_version, 1..=3) {
            return Err(Error::InvalidRequest(format!(
                "unsupported control backup version {}",
                backup.schema_version
            )));
        }
        for run in &backup.runs {
            validate_control_json(&run.request_json, "run request")?;
            validate_optional_control_json(run.agent_snapshot_json.as_deref(), "agent snapshot")?;
            validate_optional_control_json(run.native_session_json.as_deref(), "native session")?;
            validate_optional_control_json(run.error_json.as_deref(), "run error")?;
        }
        for item in &backup.timeline {
            validate_control_json(&item.data_json, "timeline data")?;
        }
        for turn in &backup.queued_turns {
            validate_control_json(&turn.request_json, "queued turn request")?;
        }
        for receipt in &backup.command_receipts {
            validate_control_json(&receipt.request_json, "command request")?;
            validate_control_json(&receipt.result_json, "command result")?;
        }
        for approval in &backup.approvals {
            validate_control_json(&approval.request_json, "approval request")?;
            validate_optional_control_json(approval.decision_json.as_deref(), "approval decision")?;
        }
        for event in &backup.run_events {
            validate_control_json(&event.data_json, "run event")?;
        }
        for artifact in &backup.run_artifacts {
            validate_control_json(&artifact.data_json, "run artifact")?;
        }
        for item in &backup.inbox {
            validate_control_json(&item.payload_json, "inbox payload")?;
        }
        for item in &backup.mailbox {
            validate_control_json(&item.request_json, "mailbox request")?;
            validate_optional_control_json(item.reply_json.as_deref(), "mailbox reply")?;
        }
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<()> {
            conn.execute_batch(
                "DELETE FROM user_run_events;
                 DELETE FROM user_run_artifact_refs;
                 DELETE FROM user_run_artifact_blobs;
                 DELETE FROM user_run_artifacts;
                 DELETE FROM user_run_inbox;
                 DELETE FROM user_thread_mailbox;
                 DELETE FROM user_thread_links;
                 DELETE FROM user_pending_approvals;
                 DELETE FROM user_timeline_events;
                 DELETE FROM user_command_receipts;
                 DELETE FROM user_runs;
                 DELETE FROM user_thread_control;
                 DELETE FROM user_control_host;",
            )
            .map_err(sqlite)?;
            if let Some(host) = &backup.host {
                conn.execute(
                    "INSERT INTO user_control_host
                     (singleton, host_id, display_name, created_at_ms, updated_at_ms)
                     VALUES (1, ?1, ?2, ?3, ?4)",
                    params![
                        host.host_id,
                        host.display_name,
                        host.created_at_ms,
                        host.updated_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            for thread in &backup.threads {
                let next_seq = backup
                    .timeline
                    .iter()
                    .filter(|item| item.thread_id == thread.id && item.epoch == thread.epoch)
                    .map(|item| item.seq)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1);
                conn.execute(
                    "INSERT INTO user_thread_control
                     (thread_id, epoch, revision, next_seq, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        thread.id,
                        thread.epoch,
                        u64_to_i64(thread.revision)?,
                        u64_to_i64(next_seq)?,
                        thread.updated_at_ms,
                    ],
                )
                .map_err(sqlite)?;
            }
            for run in &backup.runs {
                conn.execute(
                    "INSERT INTO user_runs
                     (id, thread_id, status, adapter, request_json, agent_snapshot_json,
                      native_session_json, created_at_ms, updated_at_ms, completed_at_ms, error_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![run.id, run.thread_id, run.status, run.adapter, run.request_json,
                        run.agent_snapshot_json, run.native_session_json, run.created_at_ms,
                        run.updated_at_ms, run.completed_at_ms, run.error_json],
                )
                .map_err(sqlite)?;
            }
            for item in &backup.timeline {
                conn.execute(
                    "INSERT INTO user_timeline_events
                     (thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        item.thread_id,
                        item.epoch,
                        u64_to_i64(item.seq)?,
                        item.item_id,
                        item.run_id,
                        item.item_type,
                        item.data_json,
                        item.created_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            let inbox = if backup.inbox.is_empty() {
                backup
                    .queued_turns
                    .iter()
                    .map(|turn| ControlInboxRecord {
                        id: turn.id.clone(),
                        thread_id: turn.thread_id.clone(),
                        target_run_id: None,
                        command_id: Some(turn.command_id.clone()),
                        kind: "followup".into(),
                        state: "pending".into(),
                        payload_json: turn.request_json.clone(),
                        created_at_ms: turn.accepted_at_ms,
                        claimed_at_ms: None,
                        resolved_at_ms: None,
                    })
                    .collect::<Vec<_>>()
            } else {
                backup.inbox.clone()
            };
            for item in &inbox {
                conn.execute(
                    "INSERT INTO user_run_inbox
                     (id, thread_id, target_run_id, command_id, kind, state, payload_json,
                      created_at_ms, claimed_at_ms, resolved_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        item.id,
                        item.thread_id,
                        item.target_run_id,
                        item.command_id,
                        item.kind,
                        item.state,
                        item.payload_json,
                        item.created_at_ms,
                        item.claimed_at_ms,
                        item.resolved_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            for item in &backup.thread_links {
                conn.execute(
                    "INSERT INTO user_thread_links
                     (owner_thread_id, target_thread_id, created_at_ms) VALUES (?1, ?2, ?3)",
                    params![
                        item.owner_thread_id,
                        item.target_thread_id,
                        item.created_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            for item in &backup.mailbox {
                conn.execute(
                    "INSERT INTO user_thread_mailbox
                     (id, origin_thread_id, target_thread_id, origin_run_id, target_run_id,
                      status, request_json, reply_json, created_at_ms, updated_at_ms, consumed_at_ms,
                      projected_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        item.id,
                        item.origin_thread_id,
                        item.target_thread_id,
                        item.origin_run_id,
                        item.target_run_id,
                        item.status,
                        item.request_json,
                        item.reply_json,
                        item.created_at_ms,
                        item.updated_at_ms,
                        item.consumed_at_ms,
                        item.projected_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            for receipt in &backup.command_receipts {
                conn.execute(
                    "INSERT INTO user_command_receipts
                     (command_id, device_id, thread_id, command_kind, request_json, result_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![receipt.command_id, receipt.device_id, receipt.thread_id,
                        receipt.command_kind, receipt.request_json, receipt.result_json,
                        receipt.created_at_ms],
                )
                .map_err(sqlite)?;
            }
            for approval in &backup.approvals {
                conn.execute(
                    "INSERT INTO user_pending_approvals
                     (id, run_id, thread_id, kind, request_json, status, decision_json,
                      created_at_ms, resolved_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        approval.id,
                        approval.run_id,
                        approval.thread_id,
                        approval.kind,
                        approval.request_json,
                        approval.status,
                        approval.decision_json,
                        approval.created_at_ms,
                        approval.resolved_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            for artifact in &backup.run_artifacts {
                put_run_artifact_locked(conn, artifact)?;
            }
            for event in &backup.run_events {
                conn.execute(
                    "INSERT INTO user_run_events
                     (run_id, seq, event_id, step_id, event_type, data_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        event.run_id,
                        u64_to_i64(event.seq)?,
                        event.event_id,
                        event.step_id,
                        event.event_type,
                        event.data_json,
                        event.created_at_ms
                    ],
                )
                .map_err(sqlite)?;
            }
            Ok(())
        })();
        finish_control_transaction(conn, result)?;
        drop(db);
        self.reconcile_control_startup()?;
        Ok(())
    }

    /// Mark work that cannot survive a process restart as interrupted before
    /// any client can observe bootstrap state.
    pub fn reconcile_control_startup(&self) -> Result<(usize, usize)> {
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<(usize, usize)> {
            let now = now_ms();
            let runs = conn
                .execute(
                    "UPDATE user_runs
                     SET status = 'interrupted', updated_at_ms = ?1, completed_at_ms = ?1,
                         error_json = COALESCE(error_json, '{\"code\":\"process_restarted\",\"message\":\"Milim stopped before this run completed.\"}')
                     WHERE status IN ('accepted', 'running', 'waiting_approval', 'stopping')",
                    params![now],
                )
                .map_err(sqlite)?;
            let approvals = conn
                .execute(
                    "UPDATE user_pending_approvals
                     SET status = 'interrupted', resolved_at_ms = ?1
                     WHERE status = 'pending'",
                    params![now],
                )
                .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_run_inbox
                 SET kind = 'followup', target_run_id = NULL
                 WHERE kind = 'steer' AND state = 'pending'
                   AND target_run_id IN (SELECT id FROM user_runs WHERE status = 'interrupted')",
                [],
            )
            .map_err(sqlite)?;
            conn.execute(
                "UPDATE user_thread_mailbox
                 SET status = 'failed', updated_at_ms = ?1,
                     reply_json = COALESCE(reply_json, json_object(
                        'content', '',
                        'error', 'Milim stopped before the linked thread completed.',
                        'code', 'process_restarted'
                     ))
                 WHERE status = 'running' AND target_run_id IN
                    (SELECT id FROM user_runs WHERE status = 'interrupted')",
                params![now],
            )
            .map_err(sqlite)?;
            Ok((runs, approvals))
        })();
        finish_control_transaction(conn, result)
    }
}

fn required_control_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        Err(Error::InvalidRequest(format!("{label} cannot be empty")))
    } else {
        Ok(value)
    }
}

fn validate_control_json(value: &str, label: &str) -> Result<()> {
    serde_json::from_str::<serde_json::Value>(value)
        .map(|_| ())
        .map_err(|error| Error::InvalidRequest(format!("invalid {label} JSON: {error}")))
}

fn validate_optional_control_json(value: Option<&str>, label: &str) -> Result<()> {
    value.map_or(Ok(()), |value| validate_control_json(value, label))
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::InvalidRequest("integer is too large".into()))
}

fn i64_to_u64(value: i64, label: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Other(format!("invalid negative {label}")))
}

fn finish_control_transaction<T>(conn: &Connection, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            let started = Instant::now();
            conn.execute_batch("COMMIT").map_err(sqlite)?;
            TRANSACTION_COUNT.fetch_add(1, Ordering::Relaxed);
            TRANSACTION_COMMIT_NS.fetch_add(
                u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
                Ordering::Relaxed,
            );
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn append_control_timeline_locked(
    conn: &Connection,
    thread_id: &str,
    item_id: &str,
    run_id: Option<&str>,
    item_type: &str,
    data_json: &str,
    created_at_ms: i64,
) -> Result<ControlTimelineRecord> {
    validate_control_json(data_json, "timeline data")?;
    let (epoch, seq): (String, i64) = conn
        .query_row(
            "SELECT epoch, next_seq FROM user_thread_control WHERE thread_id = ?1",
            params![thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(sqlite)?;
    conn.execute(
        "INSERT INTO user_timeline_events
         (thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            thread_id,
            epoch,
            seq,
            item_id,
            run_id,
            item_type,
            data_json,
            created_at_ms
        ],
    )
    .map_err(sqlite)?;
    TIMELINE_WRITES.fetch_add(1, Ordering::Relaxed);
    conn.execute(
        "UPDATE user_thread_control
         SET next_seq = ?2, revision = revision + 1, updated_at_ms = ?3
         WHERE thread_id = ?1",
        params![thread_id, seq.saturating_add(1), created_at_ms],
    )
    .map_err(sqlite)?;
    conn.execute(
        "UPDATE user_sessions SET updated_at_ms = ?2 WHERE id = ?1",
        params![thread_id, created_at_ms],
    )
    .map_err(sqlite)?;
    Ok(ControlTimelineRecord {
        thread_id: thread_id.to_string(),
        epoch,
        seq: i64_to_u64(seq, "timeline sequence")?,
        item_id: item_id.to_string(),
        run_id: run_id.map(str::to_string),
        item_type: item_type.to_string(),
        data_json: data_json.to_string(),
        created_at_ms,
    })
}

fn bump_thread_revision_locked(conn: &Connection, thread_id: &str) -> Result<()> {
    let now = now_ms();
    let changed = conn
        .execute(
            "UPDATE user_thread_control
             SET revision = revision + 1, updated_at_ms = ?2 WHERE thread_id = ?1",
            params![thread_id, now],
        )
        .map_err(sqlite)?;
    if changed == 0 {
        return Err(Error::InvalidRequest(format!(
            "thread {thread_id} has no control metadata"
        )));
    }
    conn.execute(
        "UPDATE user_sessions SET updated_at_ms = ?2 WHERE id = ?1",
        params![thread_id, now],
    )
    .map_err(sqlite)?;
    Ok(())
}

fn control_thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlThreadRecord> {
    let revision: i64 = row.get(2)?;
    Ok(ControlThreadRecord {
        id: row.get(0)?,
        session_json: row.get(1)?,
        revision: revision.max(0) as u64,
        epoch: row.get(3)?,
        updated_at_ms: row.get(4)?,
    })
}

fn control_thread_link_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ControlThreadLinkRecord> {
    Ok(ControlThreadLinkRecord {
        owner_thread_id: row.get(0)?,
        target_thread_id: row.get(1)?,
        created_at_ms: row.get(2)?,
    })
}

fn control_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlRunRecord> {
    Ok(ControlRunRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        status: row.get(2)?,
        adapter: row.get(3)?,
        request_json: row.get(4)?,
        agent_snapshot_json: row.get(5)?,
        native_session_json: row.get(6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
        completed_at_ms: row.get(9)?,
        error_json: row.get(10)?,
    })
}

fn control_timeline_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlTimelineRecord> {
    let seq: i64 = row.get(2)?;
    Ok(ControlTimelineRecord {
        thread_id: row.get(0)?,
        epoch: row.get(1)?,
        seq: seq.max(0) as u64,
        item_id: row.get(3)?,
        run_id: row.get(4)?,
        item_type: row.get(5)?,
        data_json: row.get(6)?,
        created_at_ms: row.get(7)?,
    })
}

fn query_control_timeline_page(
    conn: &Connection,
    thread_id: &str,
    after_seq: Option<u64>,
    before_seq: Option<u64>,
    tail: bool,
    limit: usize,
) -> Result<Option<ControlTimelinePage>> {
    let Some(epoch) = conn
        .query_row(
            "SELECT epoch FROM user_thread_control WHERE thread_id = ?1",
            params![thread_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite)?
    else {
        return Ok(None);
    };
    let limit = i64::try_from(limit)
        .map_err(|_| Error::InvalidRequest("timeline limit is too large".into()))?;
    let items = if let Some(after) = after_seq {
        query_timeline_rows(
            conn,
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2 AND seq > ?3
             ORDER BY seq ASC LIMIT ?4",
            params![thread_id, epoch, u64_to_i64(after)?, limit],
        )?
    } else if let Some(before) = before_seq {
        let mut rows = query_timeline_rows(
            conn,
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2 AND seq < ?3
             ORDER BY seq DESC LIMIT ?4",
            params![thread_id, epoch, u64_to_i64(before)?, limit],
        )?;
        rows.reverse();
        rows
    } else if tail {
        let mut rows = query_timeline_rows(
            conn,
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2
             ORDER BY seq DESC LIMIT ?3",
            params![thread_id, epoch, limit],
        )?;
        rows.reverse();
        rows
    } else {
        query_timeline_rows(
            conn,
            "SELECT thread_id, epoch, seq, item_id, run_id, item_type, data_json, created_at_ms
             FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2
             ORDER BY seq ASC LIMIT ?3",
            params![thread_id, epoch, limit],
        )?
    };
    let bounds: (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT MIN(seq), MAX(seq) FROM user_timeline_events
             WHERE thread_id = ?1 AND epoch = ?2",
            params![thread_id, epoch],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(sqlite)?;
    let first_seq = items.first().map(|item| item.seq);
    let last_seq = items.last().map(|item| item.seq);
    let has_older = match (bounds.0, first_seq) {
        (Some(min), Some(first)) => i64_to_u64(min, "timeline bound")? < first,
        _ => false,
    };
    let has_newer = match (bounds.1, last_seq) {
        (Some(max), Some(last)) => i64_to_u64(max, "timeline bound")? > last,
        _ => false,
    };
    Ok(Some(ControlTimelinePage {
        epoch,
        first_seq,
        last_seq,
        has_older,
        has_newer,
        items,
    }))
}

fn query_timeline_rows<P>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<ControlTimelineRecord>>
where
    P: rusqlite::Params,
{
    let mut stmt = conn.prepare(sql).map_err(sqlite)?;
    let rows = stmt
        .query_map(params, control_timeline_from_row)
        .map_err(sqlite)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(sqlite)
}

fn control_queued_turn_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ControlQueuedTurnRecord> {
    Ok(ControlQueuedTurnRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        command_id: row.get(2)?,
        request_json: row.get(3)?,
        accepted_at_ms: row.get(4)?,
    })
}

fn control_run_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlRunEventRecord> {
    let seq = row.get::<_, i64>(1)?;
    Ok(ControlRunEventRecord {
        run_id: row.get(0)?,
        seq: u64::try_from(seq).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, seq))?,
        event_id: row.get(2)?,
        step_id: row.get(3)?,
        event_type: row.get(4)?,
        data_json: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

fn control_run_artifact_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ControlRunArtifactRecord> {
    let byte_len = row.get::<_, i64>(4)?;
    Ok(ControlRunArtifactRecord {
        run_id: row.get(0)?,
        digest: row.get(1)?,
        kind: row.get(2)?,
        data_json: row.get(3)?,
        byte_len: u64::try_from(byte_len)
            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, byte_len))?,
        created_at_ms: row.get(5)?,
    })
}

fn control_inbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlInboxRecord> {
    Ok(ControlInboxRecord {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        target_run_id: row.get(2)?,
        command_id: row.get(3)?,
        kind: row.get(4)?,
        state: row.get(5)?,
        payload_json: row.get(6)?,
        created_at_ms: row.get(7)?,
        claimed_at_ms: row.get(8)?,
        resolved_at_ms: row.get(9)?,
    })
}

fn control_mailbox_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlMailboxRecord> {
    Ok(ControlMailboxRecord {
        id: row.get(0)?,
        origin_thread_id: row.get(1)?,
        target_thread_id: row.get(2)?,
        origin_run_id: row.get(3)?,
        target_run_id: row.get(4)?,
        status: row.get(5)?,
        request_json: row.get(6)?,
        reply_json: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
        consumed_at_ms: row.get(10)?,
        projected_at_ms: row.get(11)?,
    })
}

fn control_command_receipt_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ControlCommandReceiptRecord> {
    Ok(ControlCommandReceiptRecord {
        command_id: row.get(0)?,
        device_id: row.get(1)?,
        thread_id: row.get(2)?,
        command_kind: row.get(3)?,
        request_json: row.get(4)?,
        result_json: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

fn control_approval_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ControlApprovalRecord> {
    Ok(ControlApprovalRecord {
        id: row.get(0)?,
        run_id: row.get(1)?,
        thread_id: row.get(2)?,
        kind: row.get(3)?,
        request_json: row.get(4)?,
        status: row.get(5)?,
        decision_json: row.get(6)?,
        created_at_ms: row.get(7)?,
        resolved_at_ms: row.get(8)?,
    })
}

fn get_json_locked(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value_json FROM user_json_state WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
    .map_err(sqlite)
}

fn parse_json(value_json: &str) -> Result<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(value_json)
        .map_err(|e| Error::InvalidRequest(format!("invalid sessions JSON: {e}")))
}

fn json_error(e: serde_json::Error) -> Error {
    Error::Other(format!("json: {e}"))
}

fn session_rows(conn: &Connection) -> Result<Vec<serde_json::Value>> {
    let messages_by_session = session_messages_by_id(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT session_json FROM user_sessions ORDER BY sort_order ASC, updated_at_ms DESC",
        )
        .map_err(sqlite)?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(sqlite)?;
    let mut sessions = Vec::new();
    for row in rows {
        let mut session = parse_json(&row.map_err(sqlite)?)?;
        let messages = session
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| messages_by_session.get(id))
            .cloned()
            .unwrap_or_default();
        let obj = session
            .as_object_mut()
            .ok_or_else(|| Error::Other("invalid session row".into()))?;
        obj.insert("messages".to_string(), serde_json::Value::Array(messages));
        sessions.push(session);
    }
    Ok(sessions)
}

fn session_messages_page_locked(
    conn: &Connection,
    session_id: &str,
    before_index: Option<usize>,
    limit: usize,
) -> Result<SessionMessagesPage> {
    let total = conn
        .query_row(
            "SELECT COUNT(*) FROM user_session_messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(sqlite)?;
    let total = usize::try_from(total)
        .map_err(|_| Error::Other("session message count is negative".into()))?;
    let end = before_index.unwrap_or(total).min(total);
    let first_index = end.saturating_sub(limit.clamp(1, 500));
    let mut stmt = conn
        .prepare(
            "SELECT message_json FROM user_session_messages
             WHERE session_id = ?1 AND message_index >= ?2 AND message_index < ?3
             ORDER BY message_index ASC",
        )
        .map_err(sqlite)?;
    let rows = stmt
        .query_map(params![session_id, first_index as i64, end as i64], |row| {
            row.get::<_, String>(0)
        })
        .map_err(sqlite)?;
    let messages = rows
        .map(|row| row.map_err(sqlite).and_then(|json| parse_json(&json)))
        .collect::<Result<Vec<_>>>()?;
    Ok(SessionMessagesPage {
        session_id: session_id.to_string(),
        first_index,
        total,
        has_older: first_index > 0,
        messages,
    })
}

fn session_message_preview_locked(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    let message = conn
        .query_row(
            "SELECT message_json FROM user_session_messages
             WHERE session_id = ?1 ORDER BY message_index DESC LIMIT 1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite)?;
    let Some(message) = message else {
        return Ok(None);
    };
    let value = parse_json(&message)?;
    let content = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if content.is_empty() {
        Ok(None)
    } else {
        Ok(Some(content.chars().take(180).collect()))
    }
}

#[derive(Debug)]
struct ParsedChatSearchQuery {
    text: String,
    terms: Vec<String>,
    role: Option<&'static str>,
    archive_mode: &'static str,
}

impl ParsedChatSearchQuery {
    fn includes_archive(&self, archived: bool) -> bool {
        match self.archive_mode {
            "all" => true,
            "archived" => archived,
            _ => !archived,
        }
    }
}

fn parse_chat_search_query(query: &str) -> ParsedChatSearchQuery {
    let mut role = None;
    let mut archive_mode = "active";
    let mut text = Vec::new();
    let bounded_query = query.trim().chars().take(256).collect::<String>();
    for part in bounded_query.split_whitespace() {
        match part.to_ascii_lowercase().as_str() {
            "from:user" | "role:user" => role = Some("user"),
            "from:assistant" | "role:assistant" => role = Some("assistant"),
            "in:all" => archive_mode = "all",
            "is:archived" | "in:archive" | "in:archived" => archive_mode = "archived",
            _ => text.push(part),
        }
    }
    let text = text.join(" ").to_lowercase();
    let allow_single = text.chars().count() == 1;
    let terms = text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| !term.is_empty() && (allow_single || term.chars().count() > 1))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ParsedChatSearchQuery {
        text,
        terms,
        role,
        archive_mode,
    }
}

fn folder_label(folder: &str) -> &str {
    folder
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or_default()
}

fn chat_search_preview_locked(
    conn: &Connection,
    session_id: &str,
    role: Option<&str>,
) -> Result<Option<String>> {
    let message = conn
        .query_row(
            "SELECT message_json FROM user_session_messages
             WHERE session_id = ?1
               AND (?2 IS NULL OR json_extract(message_json, '$.role') = ?2)
             ORDER BY message_index DESC LIMIT 1",
            params![session_id, role],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite)?;
    let Some(message) = message else {
        return Ok(None);
    };
    let value = parse_json(&message)?;
    let content = value
        .get("content")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if content.is_empty() {
        return Ok(None);
    }
    let prefix = match value.get("role").and_then(serde_json::Value::as_str) {
        Some("user") => "You: ",
        Some("assistant") => "Assistant: ",
        _ => "",
    };
    Ok(Some(format!(
        "{prefix}{}",
        content.chars().take(180).collect::<String>()
    )))
}

fn session_messages_by_id(conn: &Connection) -> Result<BTreeMap<String, Vec<serde_json::Value>>> {
    let mut stmt = conn
        .prepare(
            "SELECT session_id, message_json
             FROM user_session_messages
             ORDER BY session_id ASC, message_index ASC",
        )
        .map_err(sqlite)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite)?;
    let mut messages = BTreeMap::<String, Vec<serde_json::Value>>::new();
    for row in rows {
        let (session_id, message_json) = row.map_err(sqlite)?;
        messages
            .entry(session_id)
            .or_default()
            .push(parse_json(&message_json)?);
    }
    Ok(messages)
}

#[derive(Debug)]
struct StoredSessionRow {
    session_json: String,
    sort_order: i64,
    messages: Vec<String>,
}

fn stored_session_rows(conn: &Connection) -> Result<BTreeMap<String, StoredSessionRow>> {
    let mut rows = BTreeMap::new();
    let mut stmt = conn
        .prepare("SELECT id, session_json, sort_order FROM user_sessions")
        .map_err(sqlite)?;
    let session_rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(sqlite)?;
    for row in session_rows {
        let (id, session_json, sort_order) = row.map_err(sqlite)?;
        rows.insert(
            id,
            StoredSessionRow {
                session_json,
                sort_order,
                messages: Vec::new(),
            },
        );
    }

    let mut stmt = conn
        .prepare(
            "SELECT session_id, message_json
             FROM user_session_messages
             ORDER BY session_id ASC, message_index ASC",
        )
        .map_err(sqlite)?;
    let message_rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(sqlite)?;
    for row in message_rows {
        let (session_id, message_json) = row.map_err(sqlite)?;
        if let Some(session) = rows.get_mut(&session_id) {
            session.messages.push(message_json);
        }
    }
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionSnapshotStats {
    sessions: usize,
    messages: usize,
    updated_at_ms: i64,
}

fn session_updated_at_ms(session: &serde_json::Value) -> i64 {
    session
        .get("updatedAt")
        .or_else(|| session.get("updated_at_ms"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default()
}

fn session_snapshot_stats(sessions: &[serde_json::Value]) -> SessionSnapshotStats {
    SessionSnapshotStats {
        sessions: sessions.len(),
        messages: sessions
            .iter()
            .filter_map(|session| {
                session
                    .get("messages")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len)
            })
            .sum(),
        updated_at_ms: sessions
            .iter()
            .map(session_updated_at_ms)
            .max()
            .unwrap_or_default(),
    }
}

fn session_snapshot_stats_from_json(value_json: &str) -> Result<SessionSnapshotStats> {
    let root = parse_json(value_json)?;
    let sessions = root
        .get("state")
        .and_then(serde_json::Value::as_object)
        .and_then(|state| state.get("sessions"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            Error::InvalidRequest("sessions state must include a sessions array".into())
        })?;
    Ok(session_snapshot_stats(sessions))
}

fn should_migrate_sessions_snapshot(
    value_json: &str,
    current_sessions: &[serde_json::Value],
) -> Result<bool> {
    let incoming = session_snapshot_stats_from_json(value_json)?;
    let current = session_snapshot_stats(current_sessions);
    Ok(current.sessions == 0
        || incoming.sessions > current.sessions
        || (incoming.sessions == current.sessions && incoming.messages > current.messages)
        || (incoming.sessions == current.sessions
            && incoming.messages == current.messages
            && incoming.updated_at_ms > current.updated_at_ms))
}

fn should_ignore_default_sessions_snapshot(
    value_json: &str,
    current_sessions: &[serde_json::Value],
) -> Result<bool> {
    let current = session_snapshot_stats(current_sessions);
    if current.sessions <= 1 && current.messages == 0 {
        return Ok(false);
    }

    let root = parse_json(value_json)?;
    let sessions = root
        .get("state")
        .and_then(serde_json::Value::as_object)
        .and_then(|state| state.get("sessions"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            Error::InvalidRequest("sessions state must include a sessions array".into())
        })?;
    let Some(session) = sessions.first().filter(|_| sessions.len() == 1) else {
        return Ok(false);
    };
    let messages = session
        .get("messages")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let title = session
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    Ok(messages == 0 && title == "New chat")
}

fn apply_sessions_delta_locked(conn: &Connection, delta: SessionsDelta) -> Result<()> {
    let mut meta = parse_json(&delta.meta_json)?;
    let state = meta
        .get_mut("state")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| Error::InvalidRequest("sessions metadata must include state".into()))?;
    state.remove("sessions");
    state.remove("workerRuns");
    let meta_json = serde_json::to_string(&meta).map_err(json_error)?;

    let session_order = delta
        .session_order
        .iter()
        .map(|id| id.trim())
        .collect::<BTreeSet<_>>();
    if session_order.len() != delta.session_order.len() || session_order.contains("") {
        return Err(Error::InvalidRequest(
            "session order must contain unique non-empty ids".into(),
        ));
    }
    let deleted_ids = delta
        .deleted_session_ids
        .iter()
        .map(|id| id.trim())
        .collect::<BTreeSet<_>>();
    if deleted_ids.len() != delta.deleted_session_ids.len() || deleted_ids.contains("") {
        return Err(Error::InvalidRequest(
            "deleted session ids must be unique and non-empty".into(),
        ));
    }
    if deleted_ids.iter().any(|id| session_order.contains(id)) {
        return Err(Error::InvalidRequest(
            "deleted sessions cannot remain in session order".into(),
        ));
    }

    let mut upsert_ids = BTreeSet::new();
    for session in &delta.upserts {
        let id = session.id.trim();
        if id.is_empty() || !upsert_ids.insert(id) || !session_order.contains(id) {
            return Err(Error::InvalidRequest(
                "session upserts require unique ordered ids".into(),
            ));
        }
        if deleted_ids.contains(id) {
            return Err(Error::InvalidRequest(
                "a session cannot be deleted and upserted together".into(),
            ));
        }
        if let Some(session_json) = &session.session_json {
            let value = parse_json(session_json)?;
            let object = value
                .as_object()
                .ok_or_else(|| Error::InvalidRequest("session row must be a JSON object".into()))?;
            if object.get("id").and_then(serde_json::Value::as_str) != Some(id)
                || object.contains_key("messages")
            {
                return Err(Error::InvalidRequest(
                    "session row id must match and messages must be separate".into(),
                ));
            }
        }
        if session.preserve_messages && !session.messages.is_empty() {
            return Err(Error::InvalidRequest(
                "preserved session deltas cannot include message rows".into(),
            ));
        }
        let mut message_indices = BTreeSet::new();
        for message in &session.messages {
            if message.index >= session.message_count || !message_indices.insert(message.index) {
                return Err(Error::InvalidRequest(
                    "message deltas require unique in-range indices".into(),
                ));
            }
            if !parse_json(&message.message_json)?.is_object() {
                return Err(Error::InvalidRequest(
                    "message rows must be JSON objects".into(),
                ));
            }
        }
    }

    let owns_transaction = conn.is_autocommit();
    if owns_transaction {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(sqlite)?;
    }
    let result = (|| -> Result<()> {
        for id in &delta.deleted_session_ids {
            conn.execute("DELETE FROM user_sessions WHERE id = ?1", params![id])
                .map_err(sqlite)?;
        }

        let now = now_ms();
        for session in &delta.upserts {
            if let Some(session_json) = &session.session_json {
                conn.execute(
                    "INSERT INTO user_sessions (id, session_json, sort_order, updated_at_ms)
                     VALUES (?1, ?2, 0, ?3)
                     ON CONFLICT(id) DO UPDATE SET
                        session_json = excluded.session_json,
                        updated_at_ms = excluded.updated_at_ms",
                    params![session.id, session_json, now],
                )
                .map_err(sqlite)?;
            } else {
                let exists = conn
                    .query_row(
                        "SELECT 1 FROM user_sessions WHERE id = ?1",
                        params![session.id],
                        |_| Ok(true),
                    )
                    .optional()
                    .map_err(sqlite)?
                    .unwrap_or(false);
                if !exists {
                    return Err(Error::InvalidRequest(format!(
                        "missing session metadata for {}",
                        session.id
                    )));
                }
            }

            // While Rust owns a turn, renderer snapshots are replicas. They may
            // contain an intentionally incomplete streaming assistant message,
            // so accepting their positional message delta could overwrite the
            // authoritative user turn or race the final assistant commit.
            let server_owned_run = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM user_runs
                        WHERE thread_id = ?1 AND status IN ('accepted', 'running')
                    )",
                    params![session.id],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(sqlite)?;
            if server_owned_run {
                continue;
            }
            if session.preserve_messages {
                continue;
            }

            // A terminal control commit can land between the renderer's last
            // acknowledged snapshot and its deferred flush. If that stale
            // positional delta tries to append an ID that already survives at
            // another index, keep the canonical rows instead of turning the
            // idempotent projection race into a persistence error.
            let changed_indices = session
                .messages
                .iter()
                .map(|message| message.index)
                .collect::<BTreeSet<_>>();
            let current_message_indices = conn
                .prepare(
                    "SELECT message_index, message_json
                     FROM user_session_messages WHERE session_id = ?1",
                )
                .map_err(sqlite)?
                .query_map(params![session.id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            let mut current_index_by_id = BTreeMap::new();
            for (index, message_json) in current_message_indices {
                let message = parse_json(&message_json)?;
                let Some(id) = message
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let index = usize::try_from(index)
                    .map_err(|_| Error::Other("message index is outside usize range".into()))?;
                current_index_by_id.insert(id.to_owned(), index);
            }
            let mut stale_projection = false;
            for message_delta in &session.messages {
                let message = parse_json(&message_delta.message_json)?;
                let Some(id) = message
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                let Some(existing_index) = current_index_by_id.get(id).copied() else {
                    continue;
                };
                if existing_index != message_delta.index
                    && existing_index < session.message_count
                    && !changed_indices.contains(&existing_index)
                {
                    stale_projection = true;
                    break;
                }
            }
            if stale_projection {
                continue;
            }

            conn.execute(
                "DELETE FROM user_session_messages
                 WHERE session_id = ?1 AND message_index >= ?2",
                params![session.id, session.message_count as i64],
            )
            .map_err(sqlite)?;
            for message in &session.messages {
                conn.execute(
                    "INSERT INTO user_session_messages
                     (session_id, message_index, message_json)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(session_id, message_index) DO UPDATE SET
                        message_json = excluded.message_json",
                    params![session.id, message.index as i64, message.message_json],
                )
                .map_err(sqlite)?;
            }
            let message_rows = conn
                .prepare(
                    "SELECT message_json FROM user_session_messages
                     WHERE session_id = ?1 ORDER BY message_index ASC",
                )
                .map_err(sqlite)?
                .query_map(params![session.id], |row| row.get::<_, String>(0))
                .map_err(sqlite)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite)?;
            let mut message_ids = BTreeSet::new();
            for message_json in message_rows {
                let message = parse_json(&message_json)?;
                let Some(id) = message
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                if !message_ids.insert(id.to_owned()) {
                    return Err(Error::InvalidRequest(format!(
                        "session {} message ids must be unique",
                        session.id
                    )));
                }
            }
            let (message_count, max_message_index): (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(MAX(message_index), -1)
                     FROM user_session_messages WHERE session_id = ?1",
                    params![session.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(sqlite)?;
            if message_count != session.message_count as i64
                || max_message_index + 1 != session.message_count as i64
            {
                return Err(Error::InvalidRequest(format!(
                    "session {} message delta left gaps",
                    session.id
                )));
            }
        }

        let current_ids = conn
            .prepare("SELECT id FROM user_sessions")
            .map_err(sqlite)?
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sqlite)?
            .collect::<std::result::Result<BTreeSet<_>, _>>()
            .map_err(sqlite)?;
        let expected_ids = delta.session_order.iter().cloned().collect::<BTreeSet<_>>();
        if current_ids != expected_ids {
            return Err(Error::InvalidRequest(
                "session order does not match stored sessions".into(),
            ));
        }
        for (sort_order, id) in delta.session_order.iter().enumerate() {
            conn.execute(
                "UPDATE user_sessions SET sort_order = ?1 WHERE id = ?2",
                params![sort_order as i64, id],
            )
            .map_err(sqlite)?;
        }

        conn.execute(
            "INSERT INTO user_json_state (key, value_json, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at_ms = excluded.updated_at_ms",
            params![SESSIONS_META_KEY, meta_json, now],
        )
        .map_err(sqlite)?;
        conn.execute(
            "DELETE FROM user_json_state WHERE key = ?1",
            params![SESSIONS_STATE_KEY],
        )
        .map_err(sqlite)?;
        Ok(())
    })();

    match result {
        Ok(()) if owns_transaction => conn.execute_batch("COMMIT;").map_err(sqlite),
        Ok(()) => Ok(()),
        Err(error) => {
            if owns_transaction {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            Err(error)
        }
    }
}

fn set_sessions_snapshot_locked(conn: &Connection, value_json: &str) -> Result<()> {
    let mut root = parse_json(value_json)?;
    let state = root
        .get_mut("state")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| Error::InvalidRequest("sessions state must be an object".into()))?;
    let sessions = state
        .remove("sessions")
        .and_then(|value| value.as_array().cloned())
        .ok_or_else(|| {
            Error::InvalidRequest("sessions state must include a sessions array".into())
        })?;
    let meta_json = serde_json::to_string(&root).map_err(json_error)?;
    let now = now_ms();

    let owns_transaction = conn.is_autocommit();
    if owns_transaction {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(sqlite)?;
    }
    let result = (|| -> Result<()> {
        let existing = stored_session_rows(conn)?;
        let mut incoming_ids = BTreeSet::new();
        for (index, session) in sessions.iter().enumerate() {
            let id = session
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| Error::InvalidRequest("session row is missing id".into()))?;
            if !incoming_ids.insert(id.to_string()) {
                return Err(Error::InvalidRequest(format!("duplicate session id: {id}")));
            }
            let mut session_meta = session.clone();
            let messages = session_meta
                .as_object_mut()
                .and_then(|object| object.remove("messages"))
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
            let session_json = serde_json::to_string(&session_meta).map_err(json_error)?;
            let message_jsons = messages
                .iter()
                .map(serde_json::to_string)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(json_error)?;
            let sort_order = index as i64;
            let changed = existing.get(id).is_none_or(|row| {
                row.session_json != session_json
                    || row.sort_order != sort_order
                    || row.messages != message_jsons
            });
            if !changed {
                continue;
            }
            conn.execute(
                "INSERT INTO user_sessions (id, session_json, sort_order, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                    session_json = excluded.session_json,
                    sort_order = excluded.sort_order,
                    updated_at_ms = excluded.updated_at_ms",
                params![id, session_json, sort_order, now],
            )
            .map_err(sqlite)?;
            conn.execute(
                "DELETE FROM user_session_messages WHERE session_id = ?1",
                params![id],
            )
            .map_err(sqlite)?;
            for (message_index, message_json) in message_jsons.iter().enumerate() {
                conn.execute(
                    "INSERT INTO user_session_messages (session_id, message_index, message_json)
                     VALUES (?1, ?2, ?3)",
                    params![id, message_index as i64, message_json],
                )
                .map_err(sqlite)?;
            }
        }
        for id in existing.keys() {
            if !incoming_ids.contains(id) {
                conn.execute("DELETE FROM user_sessions WHERE id = ?1", params![id])
                    .map_err(sqlite)?;
            }
        }
        conn.execute(
            "INSERT INTO user_json_state (key, value_json, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at_ms = excluded.updated_at_ms",
            params![SESSIONS_META_KEY, meta_json, now],
        )
        .map_err(sqlite)?;
        conn.execute(
            "DELETE FROM user_json_state WHERE key = ?1",
            params![SESSIONS_STATE_KEY],
        )
        .map_err(sqlite)?;
        Ok(())
    })();

    match result {
        Ok(()) if owns_transaction => conn.execute_batch("COMMIT;").map_err(sqlite),
        Ok(()) => Ok(()),
        Err(error) => {
            if owns_transaction {
                let _ = conn.execute_batch("ROLLBACK;");
            }
            Err(error)
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn migrations_apply_once_and_track_version() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), 0);
        db.migrate(SECRETS_MIGRATIONS).unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
        // Idempotent: re-running doesn't error or double-apply.
        db.migrate(SECRETS_MIGRATIONS).unwrap();
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn secret_kv_round_trips_and_persists() {
        let dir = std::env::temp_dir().join(format!("milim-storage-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("test.db");
        let key = EncryptedStore::random_key();

        {
            let db = Database::open(&path).unwrap();
            db.migrate(SECRETS_MIGRATIONS).unwrap();
            let enc = EncryptedStore::from_key(&key);
            let kv = db.secrets(&enc);
            kv.put("openai", b"sk-123").unwrap();
            assert_eq!(kv.get("openai").unwrap().unwrap(), b"sk-123");
            assert!(kv.get("missing").unwrap().is_none());
        }

        // Reopen the file with the same key: data survives.
        {
            let db = Database::open(&path).unwrap();
            let enc = EncryptedStore::from_key(&key);
            let kv = db.secrets(&enc);
            assert_eq!(kv.get("openai").unwrap().unwrap(), b"sk-123");
            assert!(kv.delete("openai").unwrap());
            assert!(kv.get("openai").unwrap().is_none());
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_key_cannot_read_secret() {
        let db = Database::open_in_memory().unwrap();
        db.migrate(SECRETS_MIGRATIONS).unwrap();
        let enc = EncryptedStore::from_key(&[1u8; 32]);
        db.secrets(&enc).put("k", b"v").unwrap();

        let other = EncryptedStore::from_key(&[2u8; 32]);
        assert!(db.secrets(&other).get("k").is_err());
    }

    #[test]
    fn scoped_migrations_do_not_collide() {
        let db = Database::open_in_memory().unwrap();
        let a = [Migration {
            version: 1,
            name: "a_table",
            sql: "CREATE TABLE a_table (id TEXT PRIMARY KEY);",
        }];
        let b = [Migration {
            version: 1,
            name: "b_table",
            sql: "CREATE TABLE b_table (id TEXT PRIMARY KEY);",
        }];

        db.migrate_scoped("a", &a).unwrap();
        db.migrate_scoped("b", &b).unwrap();

        let a_exists: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='a_table'",
                [],
                |_| Ok(true),
            )
            .unwrap();
        let b_exists: bool = db
            .conn()
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='b_table'",
                [],
                |_| Ok(true),
            )
            .unwrap();
        assert!(a_exists);
        assert!(b_exists);
        assert_eq!(db.schema_version_scoped("a").unwrap(), 1);
        assert_eq!(db.schema_version_scoped("b").unwrap(), 1);
    }

    #[test]
    fn configured_delete_journal_mode_is_enforced() {
        let dir =
            std::env::temp_dir().join(format!("milim-syncable-db-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("milim.db");

        let db = Database::open_with_options(
            &path,
            DatabaseOptions {
                journal_mode: JournalMode::Delete,
            },
        )
        .unwrap();

        let mode: String = db
            .conn()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "delete");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wal_allows_writer_while_reader_transaction_is_open() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("milim-wal-test-{unique}"));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("milim.db");
        let reader = Database::open(&path).unwrap();
        reader
            .conn()
            .execute_batch(
                "CREATE TABLE values_table (value INTEGER); INSERT INTO values_table VALUES (1);",
            )
            .unwrap();
        let writer = Database::open(&path).unwrap();

        reader.conn().execute_batch("BEGIN;").unwrap();
        let _: i64 = reader
            .conn()
            .query_row("SELECT COUNT(*) FROM values_table", [], |row| row.get(0))
            .unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = writer
                .conn()
                .execute("INSERT INTO values_table VALUES (2)", [])
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = tx.send(result);
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(1));
        reader.conn().execute_batch("COMMIT;").unwrap();
        handle.join().unwrap();
        result
            .expect("WAL writer should not wait for the reader transaction")
            .unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_json_state_round_trips_by_key() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();

        store
            .set_json(
                "milim.sessions",
                r#"{"state":{"sessions":[],"activeId":"a"},"version":0}"#,
            )
            .unwrap();

        assert_eq!(
            store.get_json("milim.sessions").unwrap().as_deref(),
            Some(r#"{"state":{"sessions":[],"activeId":"a"},"version":0}"#)
        );
    }

    #[test]
    fn user_sessions_snapshot_uses_rows_and_metadata() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let snapshot = r#"{"state":{"sessions":[{"id":"a","title":"A","messages":[{"role":"user","content":"hello"}]},{"id":"b","title":"B","messages":[{"role":"assistant","content":"hi"}]}],"activeId":"b","sidebar":{"sessionOrder":["b","a"]}},"version":0}"#;

        store.set_sessions_snapshot(snapshot).unwrap();

        assert!(store.get_json("milim.sessions").unwrap().is_none());
        {
            let db = store.db.lock().unwrap();
            let session_json: String = db
                .conn()
                .query_row(
                    "SELECT session_json FROM user_sessions WHERE id = 'a'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let session: serde_json::Value = serde_json::from_str(&session_json).unwrap();
            assert!(session.get("messages").is_none());
            let message_count: i64 = db
                .conn()
                .query_row("SELECT COUNT(*) FROM user_session_messages", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(message_count, 2);
        }
        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(parsed["state"]["sessions"][0]["id"], "a");
        assert_eq!(
            parsed["state"]["sessions"][0]["messages"][0]["content"],
            "hello"
        );
        assert_eq!(parsed["state"]["sessions"][1]["id"], "b");
        assert_eq!(parsed["state"]["activeId"], "b");
        assert_eq!(parsed["state"]["sidebar"]["sessionOrder"][0], "b");
    }

    #[test]
    fn user_sessions_set_keeps_unchanged_session_rows() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let snapshot = r#"{"state":{"sessions":[{"id":"a","title":"A","updatedAt":10,"messages":[{"role":"user","content":"hello"}]}],"activeId":"a"},"version":0}"#;

        store.set_sessions_snapshot(snapshot).unwrap();
        {
            let db = store.db.lock().unwrap();
            db.conn()
                .execute(
                    "UPDATE user_sessions SET updated_at_ms = 123 WHERE id = 'a'",
                    [],
                )
                .unwrap();
        }

        store.set_sessions_snapshot(snapshot).unwrap();

        let updated_at_ms: i64 = store
            .db
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT updated_at_ms FROM user_sessions WHERE id = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(updated_at_ms, 123);
    }

    #[test]
    fn user_sessions_set_diffs_upserts_and_deletes_rows() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let initial = r#"{"state":{"sessions":[{"id":"a","title":"A","messages":[{"role":"user","content":"old"}]},{"id":"b","title":"B","messages":[{"role":"assistant","content":"remove"}]}],"activeId":"a"},"version":0}"#;
        let next = r#"{"state":{"sessions":[{"id":"a","title":"A","messages":[{"role":"user","content":"new"}]},{"id":"c","title":"C","messages":[]}],"activeId":"c"},"version":0}"#;

        store.set_sessions_snapshot(initial).unwrap();
        store.set_sessions_snapshot(next).unwrap();

        let db = store.db.lock().unwrap();
        let session_ids: Vec<String> = db
            .conn()
            .prepare("SELECT id FROM user_sessions ORDER BY sort_order ASC")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect();
        assert_eq!(session_ids, vec!["a".to_string(), "c".to_string()]);
        let message: String = db
            .conn()
            .query_row(
                "SELECT message_json FROM user_session_messages WHERE session_id = 'a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(message.contains("new"));
        let removed_messages: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM user_session_messages WHERE session_id = 'b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(removed_messages, 0);
    }

    #[test]
    fn user_sessions_delta_updates_only_changed_rows() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let initial = r#"{"state":{"sessions":[{"id":"a","title":"A","messages":[{"role":"user","content":"old"}]},{"id":"b","title":"B","messages":[{"role":"assistant","content":"keep"}]}],"activeId":"a"},"version":0}"#;
        store.set_sessions_snapshot(initial).unwrap();
        store
            .db
            .lock()
            .unwrap()
            .conn()
            .execute(
                "UPDATE user_sessions SET updated_at_ms = 123 WHERE id = 'b'",
                [],
            )
            .unwrap();

        store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"a","workerRuns":[{"id":"cache"}]},"version":0}"#
                    .into(),
                session_order: vec!["b".into(), "a".into()],
                upserts: vec![SessionDelta {
                    id: "a".into(),
                    session_json: Some(r#"{"id":"a","title":"A2"}"#.into()),
                    message_count: 2,
                    preserve_messages: false,
                    messages: vec![
                        SessionMessageDelta {
                            index: 0,
                            message_json: r#"{"role":"user","content":"edited"}"#.into(),
                        },
                        SessionMessageDelta {
                            index: 1,
                            message_json: r#"{"role":"assistant","content":"added"}"#.into(),
                        },
                    ],
                }],
                deleted_session_ids: Vec::new(),
            })
            .unwrap();

        let restored: serde_json::Value =
            serde_json::from_str(&store.get_sessions_snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(restored["state"]["sessions"][0]["id"], "b");
        assert_eq!(restored["state"]["sessions"][1]["title"], "A2");
        assert_eq!(
            restored["state"]["sessions"][1]["messages"][0]["content"],
            "edited"
        );
        assert_eq!(
            restored["state"]["sessions"][1]["messages"][1]["content"],
            "added"
        );
        assert!(restored["state"].get("workerRuns").is_none());
        let untouched_updated_at: i64 = store
            .db
            .lock()
            .unwrap()
            .conn()
            .query_row(
                "SELECT updated_at_ms FROM user_sessions WHERE id = 'b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(untouched_updated_at, 123);

        store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"a"},"version":0}"#.into(),
                session_order: vec!["a".into()],
                upserts: vec![SessionDelta {
                    id: "a".into(),
                    session_json: None,
                    message_count: 1,
                    preserve_messages: false,
                    messages: Vec::new(),
                }],
                deleted_session_ids: vec!["b".into()],
            })
            .unwrap();
        let restored: serde_json::Value =
            serde_json::from_str(&store.get_sessions_snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(restored["state"]["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(
            restored["state"]["sessions"][0]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn user_sessions_delta_rolls_back_incomplete_message_changes() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let initial = r#"{"state":{"sessions":[{"id":"a","title":"A","messages":[{"role":"user","content":"original"}]},{"id":"b","title":"B","messages":[]}],"activeId":"a"},"version":0}"#;
        store.set_sessions_snapshot(initial).unwrap();

        let result = store.apply_sessions_delta(SessionsDelta {
            meta_json: r#"{"state":{"activeId":"a"},"version":0}"#.into(),
            session_order: vec!["a".into()],
            upserts: vec![SessionDelta {
                id: "a".into(),
                session_json: Some(r#"{"id":"a","title":"Changed"}"#.into()),
                message_count: 2,
                preserve_messages: false,
                messages: vec![SessionMessageDelta {
                    index: 0,
                    message_json: r#"{"role":"user","content":"changed"}"#.into(),
                }],
            }],
            deleted_session_ids: vec!["b".into()],
        });
        assert!(result.is_err());

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(initial).unwrap()
        );
    }

    #[test]
    fn partial_session_delta_preserves_unloaded_messages() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let initial = r#"{"state":{"sessions":[{"id":"a","title":"Before","messages":[{"id":"one","role":"user","content":"one"},{"id":"two","role":"assistant","content":"two"}]}],"activeId":"a"},"version":0}"#;
        store.set_sessions_snapshot(initial).unwrap();

        store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"a"},"version":0}"#.into(),
                session_order: vec!["a".into()],
                upserts: vec![SessionDelta {
                    id: "a".into(),
                    session_json: Some(r#"{"id":"a","title":"After"}"#.into()),
                    message_count: 2,
                    preserve_messages: true,
                    messages: Vec::new(),
                }],
                deleted_session_ids: Vec::new(),
            })
            .unwrap();

        let restored: serde_json::Value =
            serde_json::from_str(&store.get_sessions_snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(restored["state"]["sessions"][0]["title"], "After");
        assert_eq!(
            restored["state"]["sessions"][0]["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            restored["state"]["sessions"][0]["messages"][1]["content"],
            "two"
        );
    }

    #[test]
    fn legacy_message_search_indexes_in_idle_batches_and_honors_filters() {
        let db = Database::open_in_memory().unwrap();
        db.migrate_scoped("user_data", &USER_DATA_MIGRATIONS[..8])
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_sessions (id, session_json, sort_order, updated_at_ms)
                 VALUES ('active', ?1, 0, 20), ('archived', ?2, 1, 10)",
                params![
                    r#"{"id":"active","title":"Active","messages":[]}"#,
                    r#"{"id":"archived","title":"Archived","archivedAt":1,"messages":[]}"#
                ],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_session_messages (session_id, message_index, message_json)
                 VALUES ('active', 0, ?1), ('archived', 0, ?2)",
                params![
                    r#"{"id":"m1","role":"assistant","content":"hidden search needle"}"#,
                    r#"{"id":"m2","role":"user","content":"archived search needle"}"#
                ],
            )
            .unwrap();
        let store = UserDataStore::new(db).unwrap();

        assert!(store
            .user_chat_search("needle from:assistant", 20)
            .unwrap()
            .is_empty());
        assert_eq!(store.index_session_messages_batch(1).unwrap(), 1);
        let active = store.user_chat_search("needle from:assistant", 20).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "active");
        assert!(active[0].snippet.contains("Assistant:"));
        assert!(store
            .user_chat_search("needle from:user", 20)
            .unwrap()
            .is_empty());

        assert_eq!(store.index_session_messages_batch(10).unwrap(), 1);
        let archived = store
            .user_chat_search("needle from:user is:archived", 20)
            .unwrap();
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].session_id, "archived");
        assert!(store
            .user_chat_search("needle from:user", 20)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn user_sessions_get_migrates_legacy_blob() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let legacy = r#"{"state":{"sessions":[{"id":"legacy","title":"Legacy"}],"activeId":"legacy"},"version":0}"#;
        store.set_json("milim.sessions", legacy).unwrap();

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        let restored: serde_json::Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(restored["state"]["sessions"][0]["id"], "legacy");
        assert_eq!(
            restored["state"]["sessions"][0]["messages"],
            serde_json::json!([])
        );
        assert_eq!(restored["state"]["activeId"], "legacy");
        assert!(store.get_json("milim.sessions").unwrap().is_none());
        assert!(store.delete_sessions_snapshot().unwrap());
        assert!(store.get_sessions_snapshot().unwrap().is_none());
    }

    #[test]
    fn user_sessions_get_prefers_newer_legacy_blob_when_counts_tie() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let current = r#"{"state":{"sessions":[{"id":"current","title":"Current","updatedAt":1,"messages":[{"role":"user","content":"old"}]}],"activeId":"current"},"version":0}"#;
        let legacy = r#"{"state":{"sessions":[{"id":"legacy","title":"Legacy","updatedAt":2,"messages":[{"role":"user","content":"new"}]}],"activeId":"legacy"},"version":0}"#;

        store.set_sessions_snapshot(current).unwrap();
        store.set_json("milim.sessions", legacy).unwrap();

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(legacy).unwrap()
        );
        assert!(store.get_json("milim.sessions").unwrap().is_none());
    }

    #[test]
    fn user_sessions_get_prefers_richer_legacy_blob_over_default_row() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let default = r#"{"state":{"sessions":[{"id":"new","title":"New chat","messages":[]}],"activeId":"new"},"version":0}"#;
        let legacy = r#"{"state":{"sessions":[{"id":"legacy","title":"Legacy","messages":[{"role":"user","content":"saved"}]}],"activeId":"legacy"},"version":0}"#;

        store.set_sessions_snapshot(default).unwrap();
        store.set_json("milim.sessions", legacy).unwrap();

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(legacy).unwrap()
        );
        assert!(store.get_json("milim.sessions").unwrap().is_none());
    }

    #[test]
    fn user_sessions_set_ignores_startup_default_over_richer_rows() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let saved = r#"{"state":{"sessions":[{"id":"saved","title":"Saved","messages":[{"role":"user","content":"keep"}]}],"activeId":"saved"},"version":0}"#;
        let startup_default = r#"{"state":{"sessions":[{"id":"new","title":"New chat","messages":[]}],"activeId":"new"},"version":0}"#;

        store.set_sessions_snapshot(saved).unwrap();
        store.set_sessions_snapshot(startup_default).unwrap();

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(saved).unwrap()
        );
    }

    #[test]
    fn user_json_bulk_import_migrates_richer_sessions_even_with_stale_legacy_key() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        let default = r#"{"state":{"sessions":[{"id":"new","title":"New chat","messages":[]}],"activeId":"new"},"version":0}"#;
        let stale = r#"{"state":{"sessions":[]},"version":0}"#;
        let legacy = r#"{"state":{"sessions":[{"id":"legacy","title":"Legacy","messages":[{"role":"user","content":"saved"}]}],"activeId":"legacy"},"version":0}"#;

        store.set_sessions_snapshot(default).unwrap();
        store.set_json("milim.sessions", stale).unwrap();
        store
            .import_json_entries(BTreeMap::from([(
                "milim.sessions".to_string(),
                legacy.to_string(),
            )]))
            .unwrap();

        let restored = store.get_sessions_snapshot().unwrap().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&restored).unwrap(),
            serde_json::from_str::<serde_json::Value>(legacy).unwrap()
        );
        assert!(store.get_json("milim.sessions").unwrap().is_none());
    }

    #[test]
    fn user_json_bulk_import_keeps_existing_when_input_is_empty() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();

        store
            .set_json("milim.sessions", r#"{"state":{"sessions":[]},"version":0}"#)
            .unwrap();
        store.import_json_entries(BTreeMap::new()).unwrap();

        assert!(store.get_json("milim.sessions").unwrap().is_some());
    }

    #[test]
    fn user_json_bulk_import_preserves_existing_keys() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();

        store
            .set_json("milim.settings", r#"{"state":{"theme":"db"},"version":0}"#)
            .unwrap();
        store
            .import_json_entries(BTreeMap::from([(
                "milim.settings".to_string(),
                r#"{"state":{"theme":"legacy"},"version":0}"#.to_string(),
            )]))
            .unwrap();

        assert_eq!(
            store.get_json("milim.settings").unwrap().as_deref(),
            Some(r#"{"state":{"theme":"db"},"version":0}"#)
        );
    }

    #[test]
    fn user_backup_restore_replaces_keys_and_sessions_atomically() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        store.set_json("milim.ui", r#"{"old":true}"#).unwrap();
        store.set_json("milim.settings", r#"{"old":true}"#).unwrap();
        store.set_sessions_snapshot(r#"{"state":{"sessions":[{"id":"old","messages":[]}],"activeId":"old"},"version":0}"#).unwrap();

        store.replace_backup_state(
            &["milim.ui".into(), "milim.settings".into()],
            BTreeMap::from([("milim.ui".into(), r#"{"restored":true}"#.into())]),
            r#"{"state":{"sessions":[{"id":"restored","messages":[{"role":"user","content":"hello"}]}],"activeId":"restored"},"version":0}"#,
        ).unwrap();

        assert_eq!(
            store.get_json("milim.ui").unwrap().as_deref(),
            Some(r#"{"restored":true}"#)
        );
        assert!(store.get_json("milim.settings").unwrap().is_none());
        let sessions: serde_json::Value =
            serde_json::from_str(&store.get_sessions_snapshot().unwrap().unwrap()).unwrap();
        assert_eq!(sessions["state"]["sessions"][0]["id"], "restored");
    }

    #[test]
    fn user_backup_restore_rolls_back_on_invalid_session_shape() {
        let db = Database::open_in_memory().unwrap();
        let store = UserDataStore::new(db).unwrap();
        store.set_json("milim.ui", r#"{"old":true}"#).unwrap();
        let original =
            r#"{"state":{"sessions":[{"id":"old","messages":[]}],"activeId":"old"},"version":0}"#;
        store.set_sessions_snapshot(original).unwrap();

        let result = store.replace_backup_state(
            &["milim.ui".into()],
            BTreeMap::from([("milim.ui".into(), r#"{"restored":true}"#.into())]),
            r#"{"state":{"sessions":"invalid"},"version":0}"#,
        );
        assert!(result.is_err());
        assert_eq!(
            store.get_json("milim.ui").unwrap().as_deref(),
            Some(r#"{"old":true}"#)
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                &store.get_sessions_snapshot().unwrap().unwrap()
            )
            .unwrap(),
            serde_json::from_str::<serde_json::Value>(original).unwrap(),
        );
    }

    #[test]
    fn control_runtime_migrates_legacy_threads_without_rewriting_them() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        let snapshot = r#"{"state":{"sessions":[{"id":"legacy","title":"Kept","unknown":{"future":true},"messages":[{"id":"m1","role":"user","content":"hello"}]}],"activeId":"legacy"},"version":0}"#;
        store.set_sessions_snapshot(snapshot).unwrap();

        let threads = store.control_threads().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "legacy");
        assert_eq!(threads[0].revision, 0);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&threads[0].session_json).unwrap()["unknown"]
                ["future"],
            true
        );
        assert_eq!(store.control_messages("legacy").unwrap().len(), 1);
    }

    #[test]
    fn control_timeline_pages_have_stable_epoch_and_gap_cursors() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"New chat","createdAt":1,"updatedAt":1}"#,
                "epoch-1",
            )
            .unwrap();
        for index in 1..=5 {
            store
                .control_append_timeline(
                    "thread-1",
                    &format!("item-{index}"),
                    None,
                    "message",
                    &format!(r#"{{"index":{index}}}"#),
                )
                .unwrap();
        }

        let tail = store
            .control_timeline_page("thread-1", None, None, true, 2)
            .unwrap()
            .unwrap();
        assert_eq!(tail.epoch, "epoch-1");
        assert_eq!(
            tail.items.iter().map(|item| item.seq).collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert!(tail.has_older);
        assert!(!tail.has_newer);

        let middle = store
            .control_timeline_page("thread-1", Some(2), None, false, 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            middle.items.iter().map(|item| item.seq).collect::<Vec<_>>(),
            vec![3, 4]
        );
        assert!(middle.has_older);
        assert!(middle.has_newer);
    }

    #[test]
    fn control_seeds_existing_messages_into_an_empty_timeline_once() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Existing chat","createdAt":1,"updatedAt":2}"#,
                "epoch-1",
            )
            .unwrap();
        let messages = vec![
            (
                "history:user-1".into(),
                r#"{"id":"user-1","role":"user","content":"hello"}"#.into(),
                10,
            ),
            (
                "history:assistant-1".into(),
                r#"{"id":"assistant-1","role":"assistant","content":"hi"}"#.into(),
                11,
            ),
        ];

        assert_eq!(
            store
                .control_seed_message_timeline_if_empty("thread-1", &messages)
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .control_seed_message_timeline_if_empty("thread-1", &messages)
                .unwrap(),
            0
        );
        let page = store
            .control_timeline_page("thread-1", None, None, true, 10)
            .unwrap()
            .unwrap();
        assert_eq!(
            page.items
                .iter()
                .map(|item| (item.item_id.as_str(), item.seq))
                .collect::<Vec<_>>(),
            vec![("history:user-1", 1), ("history:assistant-1", 2)]
        );
        assert_eq!(
            store
                .control_append_timeline(
                    "thread-1",
                    "live-1",
                    None,
                    "message",
                    r#"{"id":"live-1","role":"user","content":"next"}"#,
                )
                .unwrap()
                .seq,
            3
        );
    }

    #[test]
    fn control_receipts_queue_and_startup_reconciliation_are_durable() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        let host = store
            .ensure_control_host("host-1", "Milim desktop")
            .unwrap();
        assert_eq!(host.host_id, "host-1");
        assert_eq!(
            store
                .ensure_control_host("host-2", "Replacement")
                .unwrap()
                .host_id,
            "host-1"
        );
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"New chat","createdAt":1,"updatedAt":1}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_enqueue_turn(&ControlQueuedTurnRecord {
                id: "queued-1".into(),
                thread_id: "thread-1".into(),
                command_id: "command-1".into(),
                request_json: r#"{"text":"next"}"#.into(),
                accepted_at_ms: 3,
            })
            .unwrap();
        store
            .control_put_command_receipt(&ControlCommandReceiptRecord {
                command_id: "command-1".into(),
                device_id: Some("phone-1".into()),
                thread_id: Some("thread-1".into()),
                command_kind: "turn.send".into(),
                request_json: r#"{"text":"next"}"#.into(),
                result_json: r#"{"status":"queued"}"#.into(),
                created_at_ms: 3,
            })
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "mock".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 2,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        store
            .control_put_approval(&ControlApprovalRecord {
                id: "approval-1".into(),
                run_id: "run-1".into(),
                thread_id: "thread-1".into(),
                kind: "command".into(),
                request_json: r#"{"command":"echo ok"}"#.into(),
                status: "pending".into(),
                decision_json: None,
                created_at_ms: 2,
                resolved_at_ms: None,
            })
            .unwrap();

        assert_eq!(store.control_queued_turns(None).unwrap().len(), 1);
        assert!(store
            .control_command_receipt("command-1")
            .unwrap()
            .is_some());
        assert_eq!(store.reconcile_control_startup().unwrap(), (1, 1));
        assert!(store.control_runs(true).unwrap().is_empty());
        assert!(store.control_pending_approvals().unwrap().is_empty());
        let run = store.control_runs(false).unwrap().pop().unwrap();
        assert_eq!(run.status, "interrupted");
    }

    #[test]
    fn control_queued_turns_can_be_reordered_without_changing_acceptance_time() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Queue"}"#,
                "epoch-1",
            )
            .unwrap();
        for (id, accepted_at_ms) in [("queued-1", 10), ("queued-2", 20), ("queued-3", 30)] {
            store
                .control_enqueue_turn(&ControlQueuedTurnRecord {
                    id: id.into(),
                    thread_id: "thread-1".into(),
                    command_id: format!("command-{id}"),
                    request_json: format!(r#"{{"text":"{id}"}}"#),
                    accepted_at_ms,
                })
                .unwrap();
        }

        assert!(store
            .control_move_queued_turn("thread-1", "queued-3", "queued-1", false)
            .unwrap());
        assert!(store
            .control_move_queued_turn("thread-1", "queued-1", "queued-2", true)
            .unwrap());
        let queued = store.control_queued_turns(Some("thread-1")).unwrap();
        assert_eq!(
            queued
                .iter()
                .map(|turn| turn.id.as_str())
                .collect::<Vec<_>>(),
            ["queued-3", "queued-2", "queued-1"]
        );
        assert_eq!(
            queued
                .iter()
                .map(|turn| turn.accepted_at_ms)
                .collect::<Vec<_>>(),
            [30, 20, 10]
        );
        assert!(store.control_remove_queued_turn("queued-3").unwrap());
        store
            .control_enqueue_turn(&ControlQueuedTurnRecord {
                id: "queued-3".into(),
                thread_id: "thread-1".into(),
                command_id: "command-queued-3".into(),
                request_json: r#"{"text":"queued-3"}"#.into(),
                accepted_at_ms: 30,
            })
            .unwrap();
        assert_eq!(
            store
                .control_queued_turns(Some("thread-1"))
                .unwrap()
                .iter()
                .map(|turn| turn.id.as_str())
                .collect::<Vec<_>>(),
            ["queued-3", "queued-2", "queued-1"]
        );
        assert!(!store
            .control_move_queued_turn("thread-1", "missing", "queued-1", false)
            .unwrap());
    }

    #[test]
    fn renderer_message_delta_cannot_overwrite_an_active_control_run() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_append_message(
                "thread-1",
                r#"{"id":"server-user","role":"user","content":"authoritative"}"#,
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "mock".into(),
                request_json: r#"{"text":"authoritative"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"thread-1"},"version":0}"#.into(),
                session_order: vec!["thread-1".into()],
                upserts: vec![SessionDelta {
                    id: "thread-1".into(),
                    session_json: None,
                    message_count: 1,
                    preserve_messages: false,
                    messages: vec![SessionMessageDelta {
                        index: 0,
                        message_json: r#"{"id":"renderer-copy","role":"user","content":"stale"}"#
                            .into(),
                    }],
                }],
                deleted_session_ids: Vec::new(),
            })
            .unwrap();
        let messages = store.control_messages("thread-1").unwrap();
        assert!(messages[0].contains("authoritative"));
        assert!(!messages[0].contains("stale"));
    }

    #[test]
    fn renderer_message_delta_ignores_a_stale_completed_control_projection() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_append_message(
                "thread-1",
                r#"{"id":"assistant-1","role":"assistant","content":"authoritative","controlSeq":12,"streamTerminalOutcome":"completed"}"#,
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "completed".into(),
                adapter: "mock".into(),
                request_json: r#"{"text":"fixture"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 2,
                completed_at_ms: Some(2),
                error_json: None,
            })
            .unwrap();

        store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"thread-1"},"version":0}"#.into(),
                session_order: vec!["thread-1".into()],
                upserts: vec![SessionDelta {
                    id: "thread-1".into(),
                    session_json: None,
                    message_count: 2,
                    preserve_messages: false,
                    messages: vec![SessionMessageDelta {
                        index: 1,
                        message_json:
                            r#"{"id":"assistant-1","role":"assistant","content":"stale renderer completion"}"#
                                .into(),
                    }],
                }],
                deleted_session_ids: Vec::new(),
            })
            .unwrap();

        let messages = store.control_messages("thread-1").unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("authoritative"));
        assert!(!messages[0].contains("stale renderer completion"));
    }

    #[test]
    fn renderer_message_delta_rejects_duplicate_stable_ids() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();

        let error = store
            .apply_sessions_delta(SessionsDelta {
                meta_json: r#"{"state":{"activeId":"thread-1"},"version":0}"#.into(),
                session_order: vec!["thread-1".into()],
                upserts: vec![SessionDelta {
                    id: "thread-1".into(),
                    session_json: None,
                    message_count: 2,
                    preserve_messages: false,
                    messages: vec![
                        SessionMessageDelta {
                            index: 0,
                            message_json: r#"{"id":"message-1","role":"user","content":"one"}"#
                                .into(),
                        },
                        SessionMessageDelta {
                            index: 1,
                            message_json:
                                r#"{"id":"message-1","role":"assistant","content":"two"}"#.into(),
                        },
                    ],
                }],
                deleted_session_ids: Vec::new(),
            })
            .unwrap_err();

        assert!(error.to_string().contains("message ids must be unique"));
        assert!(store.control_messages("thread-1").unwrap().is_empty());
    }

    #[test]
    fn v5_migration_validates_and_converts_the_v4_queue() {
        let db = Database::open_in_memory().unwrap();
        db.migrate_scoped("user_data", &USER_DATA_MIGRATIONS[..4])
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_sessions
                 (id, sort_order, session_json, updated_at_ms)
                 VALUES ('thread-1', 0, '{}', 1)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_thread_control
                 (thread_id, epoch, revision, next_seq, updated_at_ms)
                 VALUES ('thread-1', 'epoch-1', 0, 1, 1)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_queued_turns
                 (id, thread_id, command_id, request_json, accepted_at_ms)
                 VALUES ('queue-1', 'thread-1', 'command-1', '{\"text\":\"next\"}', 2)",
                [],
            )
            .unwrap();

        let store = UserDataStore::new(db).unwrap();
        let pending = store.control_pending_inbox(Some("thread-1")).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, "followup");
        assert_eq!(pending[0].command_id.as_deref(), Some("command-1"));
    }

    #[test]
    fn v5_migration_refuses_an_invalid_v4_queue_payload() {
        let db = Database::open_in_memory().unwrap();
        db.migrate_scoped("user_data", &USER_DATA_MIGRATIONS[..4])
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_sessions
                 (id, sort_order, session_json, updated_at_ms)
                 VALUES ('thread-1', 0, '{}', 1)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_thread_control
                 (thread_id, epoch, revision, next_seq, updated_at_ms)
                 VALUES ('thread-1', 'epoch-1', 0, 1, 1)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_queued_turns
                 (id, thread_id, command_id, request_json, accepted_at_ms)
                 VALUES ('queue-1', 'thread-1', 'command-1', 'not-json', 2)",
                [],
            )
            .unwrap();

        let error = UserDataStore::new(db)
            .err()
            .expect("invalid queue must block v5");
        assert!(error.to_string().contains("queued turn queue-1"));
    }

    #[test]
    fn durable_inbox_claim_cancel_retarget_and_restart_are_atomic() {
        let store = Arc::new(UserDataStore::new(Database::open_in_memory().unwrap()).unwrap());
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        for item in [
            ControlInboxRecord {
                id: "steer-1".into(),
                thread_id: "thread-1".into(),
                target_run_id: Some("run-1".into()),
                command_id: Some("command-steer-1".into()),
                kind: "steer".into(),
                state: "pending".into(),
                payload_json: r#"{"text":"first"}"#.into(),
                created_at_ms: 1,
                claimed_at_ms: None,
                resolved_at_ms: None,
            },
            ControlInboxRecord {
                id: "inject-1".into(),
                thread_id: "thread-1".into(),
                target_run_id: None,
                command_id: Some("command-inject-1".into()),
                kind: "inject".into(),
                state: "pending".into(),
                payload_json: r#"{"text":"second"}"#.into(),
                created_at_ms: 2,
                claimed_at_ms: None,
                resolved_at_ms: None,
            },
        ] {
            store.control_put_inbox(&item).unwrap();
        }

        let left = store.clone();
        let right = store.clone();
        let first = std::thread::spawn(move || {
            left.control_claim_step_inputs("thread-1", "run-1").unwrap()
        });
        let second = std::thread::spawn(move || {
            right
                .control_claim_step_inputs("thread-1", "run-1")
                .unwrap()
        });
        let mut claims = [first.join().unwrap(), second.join().unwrap()];
        claims.sort_by_key(Vec::len);
        assert!(claims[0].is_empty());
        assert_eq!(
            claims[1]
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["steer-1", "inject-1"]
        );
        assert!(!store.control_cancel_inbox("steer-1").unwrap());

        for item in [
            ControlInboxRecord {
                id: "steer-retarget".into(),
                thread_id: "thread-1".into(),
                target_run_id: Some("run-1".into()),
                command_id: None,
                kind: "steer".into(),
                state: "pending".into(),
                payload_json: r#"{"text":"later"}"#.into(),
                created_at_ms: 3,
                claimed_at_ms: None,
                resolved_at_ms: None,
            },
            ControlInboxRecord {
                id: "inject-persist".into(),
                thread_id: "thread-1".into(),
                target_run_id: None,
                command_id: None,
                kind: "inject".into(),
                state: "pending".into(),
                payload_json: r#"{"text":"context"}"#.into(),
                created_at_ms: 4,
                claimed_at_ms: None,
                resolved_at_ms: None,
            },
            ControlInboxRecord {
                id: "followup-cancel".into(),
                thread_id: "thread-1".into(),
                target_run_id: None,
                command_id: None,
                kind: "followup".into(),
                state: "pending".into(),
                payload_json: r#"{"text":"future"}"#.into(),
                created_at_ms: 5,
                claimed_at_ms: None,
                resolved_at_ms: None,
            },
        ] {
            store.control_put_inbox(&item).unwrap();
        }
        assert!(store.control_cancel_inbox("followup-cancel").unwrap());
        assert!(!store.control_cancel_inbox("followup-cancel").unwrap());
        assert_eq!(store.reconcile_control_startup().unwrap(), (1, 0));

        let pending = store.control_pending_inbox(Some("thread-1")).unwrap();
        let retargeted = pending
            .iter()
            .find(|item| item.id == "steer-retarget")
            .unwrap();
        assert_eq!(retargeted.kind, "followup");
        assert!(retargeted.target_run_id.is_none());
        assert!(pending
            .iter()
            .any(|item| item.id == "inject-persist" && item.kind == "inject"));
        assert!(!pending.iter().any(|item| item.id == "followup-cancel"));
    }

    #[test]
    fn message_projection_and_ledger_event_commit_or_rollback_together() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        let failed = store.control_commit_message_projection_and_event(
            "thread-1",
            "missing-run",
            "message-1",
            r#"{"id":"message-1","role":"assistant","content":"hi"}"#,
            "event-1",
            None,
            "assistant_message_projected",
            r#"{"ledger_version":1}"#,
        );
        assert!(failed.is_err());
        assert!(store.control_messages("thread-1").unwrap().is_empty());
        assert!(store
            .control_timeline_page("thread-1", None, None, true, 10)
            .unwrap()
            .unwrap()
            .items
            .is_empty());

        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        let (timeline, event) = store
            .control_commit_message_projection_and_event(
                "thread-1",
                "run-1",
                "message-1",
                r#"{"id":"message-1","role":"assistant","content":"hi"}"#,
                "event-1",
                Some("step-1"),
                "assistant_message_projected",
                r#"{"ledger_version":1}"#,
            )
            .unwrap();
        assert_eq!(timeline.seq, 1);
        assert_eq!(event.seq, 1);
        assert_eq!(store.control_messages("thread-1").unwrap().len(), 1);
        assert_eq!(
            store.control_run_events("run-1", None, 10).unwrap().len(),
            1
        );
    }

    #[test]
    fn run_artifacts_are_compressed_delta_encoded_and_migrated_losslessly() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        store
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        store
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "completed".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 2,
                completed_at_ms: Some(2),
                error_json: None,
            })
            .unwrap();
        let history = (0..5_000)
            .map(|index| format!("token-{index:05}-{:x}", index * 7_919))
            .collect::<Vec<_>>()
            .join(" ");
        let first = serde_json::json!({
            "messages": [{ "role": "user", "content": history.clone() }],
            "tools": [{ "name": "read_file" }]
        })
        .to_string();
        let second = serde_json::json!({
            "messages": [
                { "role": "user", "content": history },
                { "role": "assistant", "content": "next" }
            ],
            "tools": [{ "name": "read_file" }]
        })
        .to_string();
        let raw_bytes = first.len() + second.len();
        let first_digest = artifact_digest(first.as_bytes());
        let second_digest = artifact_digest(second.as_bytes());
        for (index, (digest, data_json)) in [
            (first_digest.clone(), first.clone()),
            (second_digest.clone(), second.clone()),
        ]
        .into_iter()
        .enumerate()
        {
            store
                .control_put_run_artifact(&ControlRunArtifactRecord {
                    run_id: "run-1".into(),
                    digest,
                    kind: "provider_request".into(),
                    byte_len: data_json.len() as u64,
                    data_json,
                    created_at_ms: index as i64 + 1,
                })
                .unwrap();
        }
        assert_eq!(
            store
                .control_run_artifact("run-1", &second_digest)
                .unwrap()
                .unwrap()
                .data_json,
            second
        );
        {
            let db = store.db.lock().unwrap();
            let encoding: String = db
                .conn()
                .query_row(
                    "SELECT encoding FROM user_run_artifact_blobs WHERE digest = ?1",
                    params![second_digest],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(encoding, "prefix-delta-zstd-v1");
            let stored_bytes: i64 = db
                .conn()
                .query_row(
                    "SELECT COALESCE(SUM(length(payload)), 0)
                     FROM user_run_artifact_blobs",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                stored_bytes < (raw_bytes / 4) as i64,
                "compressed fixture should use less than 25% of raw bytes: {stored_bytes}/{raw_bytes}"
            );
            db.conn()
                .execute(
                    "INSERT INTO user_run_artifacts
                     (run_id, digest, kind, data_json, byte_len, created_at_ms)
                     VALUES ('run-1', 'legacy-digest', 'tool_result', '{\"ok\":true}', 11, 3)",
                    [],
                )
                .unwrap();
        }
        assert_eq!(
            store
                .control_run_artifact("run-1", "legacy-digest")
                .unwrap()
                .unwrap()
                .data_json,
            r#"{"ok":true}"#
        );
        let progress = store.migrate_run_artifacts_batch(4 * 1024 * 1024).unwrap();
        assert_eq!(progress.migrated, 1);
        assert_eq!(progress.remaining, 0);
        assert_eq!(store.control_run_artifacts("run-1").unwrap().len(), 3);
    }

    #[test]
    fn v1_and_v2_control_backups_restore_inbox_and_ledger() {
        let source = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        source
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        source
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "completed".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 2,
                completed_at_ms: Some(2),
                error_json: None,
            })
            .unwrap();
        source
            .control_append_run_event(
                "run-1",
                "event-1",
                Some("step-1"),
                "model_request_resolved",
                r#"{"artifact_digest":"digest-1"}"#,
            )
            .unwrap();
        source
            .control_put_run_artifact(&ControlRunArtifactRecord {
                run_id: "run-1".into(),
                digest: "digest-1".into(),
                kind: "provider_request".into(),
                data_json: r#"{"messages":[]}"#.into(),
                byte_len: 15,
                created_at_ms: 1,
            })
            .unwrap();
        source
            .control_enqueue_turn(&ControlQueuedTurnRecord {
                id: "queue-1".into(),
                thread_id: "thread-1".into(),
                command_id: "command-1".into(),
                request_json: r#"{"text":"next"}"#.into(),
                accepted_at_ms: 3,
            })
            .unwrap();
        let v2 = source.control_backup_state().unwrap();
        assert_eq!(v2.schema_version, 3);

        let target_v2 = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        target_v2
            .set_sessions_snapshot(
                r#"{"state":{"sessions":[{"id":"thread-1","messages":[]}]},"version":0}"#,
            )
            .unwrap();
        target_v2.replace_control_backup_state(&v2).unwrap();
        assert_eq!(
            target_v2
                .control_run_events("run-1", None, 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(target_v2.control_run_artifacts("run-1").unwrap().len(), 1);
        assert_eq!(target_v2.control_pending_inbox(None).unwrap().len(), 1);

        let mut v1 = v2;
        v1.schema_version = 1;
        v1.run_events.clear();
        v1.run_artifacts.clear();
        v1.inbox.clear();
        let target_v1 = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        target_v1
            .set_sessions_snapshot(
                r#"{"state":{"sessions":[{"id":"thread-1","messages":[]}]},"version":0}"#,
            )
            .unwrap();
        target_v1.replace_control_backup_state(&v1).unwrap();
        let converted = target_v1.control_pending_inbox(None).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].kind, "followup");
    }

    #[test]
    fn control_backup_round_trips_and_interrupts_restored_work() {
        let source = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        source
            .ensure_control_host("stable-host", "Fixture")
            .unwrap();
        source
            .control_create_thread(
                "thread-1",
                r#"{"id":"thread-1","title":"Fixture"}"#,
                "epoch-1",
            )
            .unwrap();
        source
            .control_put_run(&ControlRunRecord {
                id: "run-1".into(),
                thread_id: "thread-1".into(),
                status: "running".into(),
                adapter: "mock".into(),
                request_json: r#"{"text":"hello"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        source
            .control_append_timeline(
                "thread-1",
                "event-1",
                Some("run-1"),
                "assistant_delta",
                r#"{"text":"hi"}"#,
            )
            .unwrap();
        source
            .control_enqueue_turn(&ControlQueuedTurnRecord {
                id: "queue-1".into(),
                thread_id: "thread-1".into(),
                command_id: "command-1".into(),
                request_json: r#"{"text":"next"}"#.into(),
                accepted_at_ms: 2,
            })
            .unwrap();
        let backup = source.control_backup_state().unwrap();

        let target = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        target
            .set_sessions_snapshot(
                r#"{"state":{"sessions":[{"id":"thread-1","title":"Fixture","messages":[]}]},"version":0}"#,
            )
            .unwrap();
        target.replace_control_backup_state(&backup).unwrap();
        assert_eq!(
            target.control_backup_state().unwrap().host.unwrap().host_id,
            "stable-host"
        );
        assert_eq!(target.control_queued_turns(None).unwrap().len(), 1);
        assert_eq!(target.control_runs(false).unwrap()[0].status, "interrupted");
        assert_eq!(
            target
                .control_timeline_page("thread-1", None, None, true, 10)
                .unwrap()
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[test]
    fn thread_links_and_mailbox_are_durable_idempotent_and_cascading() {
        let source = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        for (id, epoch) in [("origin", "epoch-origin"), ("target", "epoch-target")] {
            source
                .control_create_thread(
                    id,
                    &serde_json::json!({ "id": id, "title": id }).to_string(),
                    epoch,
                )
                .unwrap();
        }
        let before = source.control_thread("origin").unwrap().unwrap().revision;
        let added = source
            .control_add_thread_link("origin", "target", "link-event")
            .unwrap()
            .unwrap();
        assert_eq!(added.item_type, "thread_link_added");
        let linked_revision = source.control_thread("origin").unwrap().unwrap().revision;
        assert_eq!(linked_revision, before + 1);
        assert!(source
            .control_add_thread_link("origin", "target", "duplicate-event")
            .unwrap()
            .is_none());
        assert_eq!(
            source.control_thread("origin").unwrap().unwrap().revision,
            linked_revision
        );

        source
            .control_put_mailbox(&ControlMailboxRecord {
                id: "exchange-1".into(),
                origin_thread_id: "origin".into(),
                target_thread_id: "target".into(),
                origin_run_id: None,
                target_run_id: None,
                status: "replied".into(),
                request_json: r#"{"message":"hello"}"#.into(),
                reply_json: Some(r#"{"content":"world"}"#.into()),
                created_at_ms: 1,
                updated_at_ms: 2,
                consumed_at_ms: None,
                projected_at_ms: None,
            })
            .unwrap();
        let claimed = source
            .control_claim_mailbox_replies("origin", "origin-run", 20)
            .unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].origin_run_id.as_deref(), Some("origin-run"));
        assert!(source
            .control_claim_mailbox_replies("origin", "later-run", 20)
            .unwrap()
            .is_empty());

        let backup = source.control_backup_state().unwrap();
        assert_eq!(backup.schema_version, 3);
        assert_eq!(backup.thread_links.len(), 1);
        assert_eq!(backup.mailbox.len(), 1);
        assert!(source.control_delete_thread("origin").unwrap());
        assert!(source.control_thread("target").unwrap().is_some());
        assert!(source
            .control_mailbox_for_origin("origin")
            .unwrap()
            .is_empty());
        let target_store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        target_store
            .set_sessions_snapshot(
                r#"{"state":{"sessions":[{"id":"origin","messages":[]},{"id":"target","messages":[]}]},"version":0}"#,
            )
            .unwrap();
        target_store.replace_control_backup_state(&backup).unwrap();
        assert_eq!(
            target_store
                .control_thread_links(Some("origin"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            target_store
                .control_mailbox_for_origin("origin")
                .unwrap()
                .len(),
            1
        );

        assert!(target_store.control_delete_thread("target").unwrap());
        assert!(target_store
            .control_thread_links(Some("origin"))
            .unwrap()
            .is_empty());
        assert!(target_store
            .control_mailbox_for_origin("origin")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn running_mailbox_delivers_a_restart_failure_receipt() {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        for id in ["origin", "target"] {
            store
                .control_create_thread(
                    id,
                    &serde_json::json!({ "id": id, "title": id }).to_string(),
                    &format!("epoch-{id}"),
                )
                .unwrap();
        }
        store
            .control_put_run(&ControlRunRecord {
                id: "target-run".into(),
                thread_id: "target".into(),
                status: "running".into(),
                adapter: "provider".into(),
                request_json: r#"{"text":"mailbox work"}"#.into(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                completed_at_ms: None,
                error_json: None,
            })
            .unwrap();
        store
            .control_put_mailbox(&ControlMailboxRecord {
                id: "exchange-restart".into(),
                origin_thread_id: "origin".into(),
                target_thread_id: "target".into(),
                origin_run_id: None,
                target_run_id: Some("target-run".into()),
                status: "running".into(),
                request_json: r#"{"message":"continue"}"#.into(),
                reply_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
                consumed_at_ms: None,
                projected_at_ms: None,
            })
            .unwrap();

        assert_eq!(store.reconcile_control_startup().unwrap(), (1, 0));
        let run = store.control_run("target-run").unwrap().unwrap();
        assert_eq!(run.status, "interrupted");
        let exchange = store.control_mailbox("exchange-restart").unwrap().unwrap();
        assert_eq!(exchange.status, "failed");
        assert!(exchange
            .reply_json
            .as_deref()
            .unwrap()
            .contains("process_restarted"));
    }
}
