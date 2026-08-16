//! SQLite database wrapper + a tiny ordered migration runner.
//!
//! Uses rusqlite's `bundled` SQLite (compiled from source, so it works on
//! Windows/Linux/macOS with no system SQLite). The harness subsystems (chat
//! history, agents, memory, …) build their schemas as [`Migration`] lists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use milim_core::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::crypto::EncryptedStore;

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
        Ok(Self { conn })
    }

    /// Open an ephemeral in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite)?;
        Self::configure(&conn, DatabaseOptions::default())?;
        Ok(Self { conn })
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
    db: Mutex<Database>,
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlBackupState {
    pub host: Option<ControlHostRecord>,
    pub threads: Vec<ControlThreadRecord>,
    pub runs: Vec<ControlRunRecord>,
    pub timeline: Vec<ControlTimelineRecord>,
    pub queued_turns: Vec<ControlQueuedTurnRecord>,
    pub command_receipts: Vec<ControlCommandReceiptRecord>,
    pub approvals: Vec<ControlApprovalRecord>,
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
    pub messages: Vec<SessionMessageDelta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessageDelta {
    pub index: usize,
    pub message_json: String,
}

impl UserDataStore {
    pub fn new(db: Database) -> Result<Self> {
        db.migrate_scoped("user_data", USER_DATA_MIGRATIONS)?;
        Ok(Self { db: Mutex::new(db) })
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
    ) -> Result<Option<ControlThreadRecord>> {
        let thread_id = required_control_text(thread_id, "thread id")?;
        validate_control_json(session_json, "thread")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<Option<ControlThreadRecord>> {
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
            Ok(Some(current))
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

    /// Delete one canonical user-session message by its stable JSON `id` and
    /// compact the positional indices used by the legacy desktop snapshot.
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
                            .get("id")
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
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
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
        let mut items = if let Some(after) = after_seq {
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
            items: std::mem::take(&mut items),
        }))
    }

    pub fn control_enqueue_turn(&self, turn: &ControlQueuedTurnRecord) -> Result<()> {
        validate_control_json(&turn.request_json, "queued turn request")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute(
                "INSERT INTO user_queued_turns
                 (id, thread_id, command_id, request_json, accepted_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
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
                "SELECT id, thread_id, command_id, request_json, accepted_at_ms
                 FROM user_queued_turns WHERE thread_id = ?1
                 ORDER BY accepted_at_ms ASC, id ASC",
                Some(required_control_text(id, "thread id")?),
            ),
            None => (
                "SELECT id, thread_id, command_id, request_json, accepted_at_ms
                 FROM user_queued_turns ORDER BY accepted_at_ms ASC, id ASC",
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

    pub fn control_remove_queued_turn(&self, id: &str) -> Result<bool> {
        let id = required_control_text(id, "queued turn id")?;
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        db.conn()
            .execute("DELETE FROM user_queued_turns WHERE id = ?1", params![id])
            .map(|changed| changed > 0)
            .map_err(sqlite)
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
                    "SELECT id, thread_id, command_id, request_json, accepted_at_ms
                     FROM user_queued_turns ORDER BY accepted_at_ms, id",
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
        Ok(ControlBackupState {
            host,
            threads,
            runs,
            timeline,
            queued_turns,
            command_receipts,
            approvals,
        })
    }

    pub fn replace_control_backup_state(&self, backup: &ControlBackupState) -> Result<()> {
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
        let db = self
            .db
            .lock()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sqlite)?;
        let result = (|| -> Result<()> {
            conn.execute_batch(
                "DELETE FROM user_pending_approvals;
                 DELETE FROM user_timeline_events;
                 DELETE FROM user_queued_turns;
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
            for turn in &backup.queued_turns {
                conn.execute(
                    "INSERT INTO user_queued_turns
                     (id, thread_id, command_id, request_json, accepted_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        turn.id,
                        turn.thread_id,
                        turn.command_id,
                        turn.request_json,
                        turn.accepted_at_ms
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
            conn.execute_batch("COMMIT").map_err(sqlite)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
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
}
