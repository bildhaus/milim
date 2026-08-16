//! Durable parent/child agent thread records.

use std::sync::Mutex;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use milim_core::{Error, Result};
use milim_storage::{Database, Migration};

pub const THREAD_STATUS_QUEUED: &str = "queued";
pub const THREAD_STATUS_RUNNING: &str = "running";
pub const THREAD_STATUS_DONE: &str = "done";
pub const THREAD_STATUS_STOPPED: &str = "stopped";
pub const THREAD_STATUS_ERROR: &str = "error";

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationPolicy {
    Off,
    #[default]
    Ask,
    Auto,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRuntime {
    #[default]
    Managed,
    Codex,
    Claude,
    Legacy,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerAccess {
    #[default]
    ReadOnly,
    WriteReview,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRunStatus {
    #[default]
    Proposed,
    Running,
    Done,
    Partial,
    Stopped,
    Error,
}

impl DelegationPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }
}

impl WorkerRuntime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Legacy => "legacy",
        }
    }
}

impl WorkerAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WriteReview => "write_review",
        }
    }
}

impl WorkerRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Running => "running",
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Stopped => "stopped",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerAgentSnapshot {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub tool_mode: String,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub skill_mode: String,
    #[serde(default)]
    pub enabled_skills: Vec<String>,
    #[serde(default)]
    pub avatar: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPlanTask {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub role: Option<String>,
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_snapshot: Option<WorkerAgentSnapshot>,
    pub model: String,
    pub access: WorkerAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRun {
    pub id: String,
    pub parent_thread_id: String,
    pub parent_turn_id: Option<String>,
    pub policy: DelegationPolicy,
    pub runtime: WorkerRuntime,
    pub status: WorkerRunStatus,
    pub tasks: Vec<WorkerPlanTask>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub privacy_mode: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

/// Persisted metadata for one child thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentThread {
    pub id: String,
    pub parent_id: String,
    pub root_id: String,
    pub title: String,
    pub status: String,
    pub model: String,
    pub agent_id: Option<String>,
    pub prompt: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub runtime: WorkerRuntime,
    #[serde(default)]
    pub access: WorkerAccess,
    #[serde(default)]
    pub external_runtime_id: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
}

pub type Worker = AgentThread;

/// One stored event from a child thread run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub id: String,
    pub thread_id: String,
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

pub const THREAD_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "threads",
        sql: "CREATE TABLE threads (
                id          TEXT PRIMARY KEY,
                parent_id   TEXT NOT NULL,
                root_id     TEXT NOT NULL,
                title       TEXT NOT NULL,
                status      TEXT NOT NULL,
                model       TEXT NOT NULL DEFAULT '',
                agent_id    TEXT,
                prompt      TEXT NOT NULL,
                summary     TEXT,
                error       TEXT,
                created_at  TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at TEXT
              );
              CREATE INDEX idx_threads_parent ON threads(parent_id, created_at);
              CREATE INDEX idx_threads_status ON threads(status, updated_at);
              CREATE TABLE thread_events (
                id         TEXT PRIMARY KEY,
                thread_id  TEXT NOT NULL,
                kind       TEXT NOT NULL,
                payload    TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
              );
              CREATE INDEX idx_thread_events_thread ON thread_events(thread_id, created_at);",
    },
    Migration {
        version: 2,
        name: "thread_event_seq",
        sql: "ALTER TABLE thread_events ADD COLUMN seq INTEGER;
              UPDATE thread_events
              SET seq = (
                SELECT COUNT(*)
                FROM thread_events AS earlier
                WHERE earlier.created_at < thread_events.created_at
                   OR (earlier.created_at = thread_events.created_at AND earlier.id <= thread_events.id)
              )
              WHERE seq IS NULL;
              CREATE INDEX idx_thread_events_thread_seq ON thread_events(thread_id, seq);
              CREATE UNIQUE INDEX idx_thread_events_seq ON thread_events(seq);",
    },
    Migration {
        version: 3,
        name: "worker_runs",
        sql: "CREATE TABLE worker_runs (
                id               TEXT PRIMARY KEY,
                parent_thread_id TEXT NOT NULL,
                parent_turn_id   TEXT,
                policy           TEXT NOT NULL,
                runtime          TEXT NOT NULL,
                status           TEXT NOT NULL,
                tasks            TEXT NOT NULL,
                error            TEXT,
                created_at       TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at       TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at      TEXT
              );
              CREATE INDEX idx_worker_runs_parent ON worker_runs(parent_thread_id, created_at);
              CREATE INDEX idx_worker_runs_status ON worker_runs(status, updated_at);
              ALTER TABLE threads ADD COLUMN run_id TEXT;
              ALTER TABLE threads ADD COLUMN runtime TEXT NOT NULL DEFAULT 'legacy';
              ALTER TABLE threads ADD COLUMN access TEXT NOT NULL DEFAULT 'read_only';
              ALTER TABLE threads ADD COLUMN external_runtime_id TEXT;
              ALTER TABLE threads ADD COLUMN worktree_path TEXT;
              CREATE INDEX idx_threads_run ON threads(run_id, created_at);",
    },
    Migration {
        version: 4,
        name: "worker_run_context",
        sql: "ALTER TABLE worker_runs ADD COLUMN context TEXT;",
    },
    Migration {
        version: 5,
        name: "worker_run_origin",
        sql: "ALTER TABLE worker_runs ADD COLUMN workspace TEXT;
              ALTER TABLE worker_runs ADD COLUMN privacy_mode TEXT
                CHECK (privacy_mode IS NULL OR privacy_mode IN ('off', 'redact', 'block'));",
    },
];

/// CRUD over parent/child thread rows.
pub struct ThreadStore {
    db: Mutex<Database>,
}

impl ThreadStore {
    pub fn new(db: Database) -> Result<Self> {
        db.migrate_scoped("threads", THREAD_MIGRATIONS)?;
        Ok(Self { db: Mutex::new(db) })
    }

    pub fn create(
        &self,
        parent_id: &str,
        title: &str,
        model: &str,
        agent_id: Option<&str>,
        prompt: &str,
    ) -> Result<AgentThread> {
        self.create_worker(
            parent_id,
            title,
            model,
            agent_id,
            prompt,
            None,
            WorkerRuntime::Legacy,
            WorkerAccess::ReadOnly,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_worker(
        &self,
        parent_id: &str,
        title: &str,
        model: &str,
        agent_id: Option<&str>,
        prompt: &str,
        run_id: Option<&str>,
        runtime: WorkerRuntime,
        access: WorkerAccess,
    ) -> Result<AgentThread> {
        let id = uuid::Uuid::new_v4().to_string();
        let db = self.db.lock().expect("threads db poisoned");
        let root_id: Option<String> = db
            .conn()
            .query_row(
                "SELECT root_id FROM threads WHERE id = ?1",
                params![parent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sqlite)?;
        db.conn()
            .execute(
                "INSERT INTO threads (id, parent_id, root_id, title, status, model, agent_id, prompt, run_id, runtime, access)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id,
                    parent_id,
                    root_id.unwrap_or_else(|| parent_id.to_string()),
                    title,
                    THREAD_STATUS_QUEUED,
                    model,
                    agent_id,
                    prompt,
                    run_id,
                    runtime.as_str(),
                    access.as_str()
                ],
            )
            .map_err(sqlite)?;
        drop(db);
        self.get(&id)?
            .ok_or_else(|| Error::Other("thread insert did not return a row".to_string()))
    }

    pub fn create_worker_run(
        &self,
        parent_thread_id: &str,
        parent_turn_id: Option<&str>,
        policy: DelegationPolicy,
        runtime: WorkerRuntime,
        tasks: Vec<WorkerPlanTask>,
        context: Option<&str>,
    ) -> Result<WorkerRun> {
        self.create_worker_run_with_origin(
            parent_thread_id,
            parent_turn_id,
            policy,
            runtime,
            tasks,
            context,
            None,
            "off",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_worker_run_with_origin(
        &self,
        parent_thread_id: &str,
        parent_turn_id: Option<&str>,
        policy: DelegationPolicy,
        runtime: WorkerRuntime,
        tasks: Vec<WorkerPlanTask>,
        context: Option<&str>,
        workspace: Option<&str>,
        privacy_mode: &str,
    ) -> Result<WorkerRun> {
        if workspace.is_some_and(|value| value.trim().is_empty()) {
            return Err(Error::InvalidRequest(
                "worker run workspace must not be empty".to_string(),
            ));
        }
        if !matches!(privacy_mode, "off" | "redact" | "block") {
            return Err(Error::InvalidRequest(
                "worker run privacy_mode must be off, redact, or block".to_string(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let tasks = serde_json::to_string(&tasks)?;
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute(
                "INSERT INTO worker_runs (
                id, parent_thread_id, parent_turn_id, policy, runtime, status, tasks,
                context, workspace, privacy_mode
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'proposed', ?6, ?7, ?8, ?9)",
                params![
                    id,
                    parent_thread_id,
                    parent_turn_id,
                    policy.as_str(),
                    runtime.as_str(),
                    tasks,
                    context,
                    workspace,
                    privacy_mode
                ],
            )
            .map_err(sqlite)?;
        drop(db);
        self.get_worker_run(&id)?
            .ok_or_else(|| Error::Other("worker run insert did not return a row".to_string()))
    }

    pub fn get_worker_run(&self, id: &str) -> Result<Option<WorkerRun>> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn().query_row(
            "SELECT id, parent_thread_id, parent_turn_id, policy, runtime, status, tasks, error, created_at, updated_at, finished_at, context, workspace, privacy_mode
             FROM worker_runs WHERE id = ?1",
            params![id], row_to_worker_run,
        ).optional().map_err(sqlite)
    }

    pub fn list_worker_runs(&self, parent_thread_id: &str, limit: usize) -> Result<Vec<WorkerRun>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db.conn().prepare(
            "SELECT id, parent_thread_id, parent_turn_id, policy, runtime, status, tasks, error, created_at, updated_at, finished_at, context, workspace, privacy_mode
             FROM worker_runs WHERE parent_thread_id = ?1 ORDER BY created_at DESC LIMIT ?2"
        ).map_err(sqlite)?;
        let rows = stmt
            .query_map(
                params![parent_thread_id, limit.clamp(1, 200) as i64],
                row_to_worker_run,
            )
            .map_err(sqlite)?;
        rows.map(|row| row.map_err(sqlite)).collect()
    }

    pub fn workers_for_run(&self, run_id: &str) -> Result<Vec<Worker>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                    summary, error, created_at, updated_at, finished_at,
                    run_id, runtime, access, external_runtime_id, worktree_path
             FROM threads WHERE run_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(sqlite)?;
        let rows = stmt
            .query_map(params![run_id], row_to_thread)
            .map_err(sqlite)?;
        rows.map(|row| row.map_err(sqlite)).collect()
    }

    pub fn delete_worker_run(&self, run_id: &str) -> Result<bool> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .map_err(sqlite)?;
        let result = (|| -> Result<bool> {
            db.conn()
                .execute(
                    "DELETE FROM thread_events WHERE thread_id IN (SELECT id FROM threads WHERE run_id = ?1)",
                    params![run_id],
                )
                .map_err(sqlite)?;
            db.conn()
                .execute("DELETE FROM threads WHERE run_id = ?1", params![run_id])
                .map_err(sqlite)?;
            Ok(db
                .conn()
                .execute("DELETE FROM worker_runs WHERE id = ?1", params![run_id])
                .map_err(sqlite)?
                > 0)
        })();
        match result {
            Ok(deleted) => {
                db.conn().execute_batch("COMMIT;").map_err(sqlite)?;
                Ok(deleted)
            }
            Err(error) => {
                let _ = db.conn().execute_batch("ROLLBACK;");
                Err(error)
            }
        }
    }

    pub fn update_worker_run_status(
        &self,
        id: &str,
        status: WorkerRunStatus,
        error: Option<&str>,
    ) -> Result<Option<WorkerRun>> {
        let db = self.db.lock().expect("threads db poisoned");
        let changed = db.conn().execute(
            "UPDATE worker_runs SET status = ?2, error = COALESCE(?3, error), updated_at = datetime('now'),
             finished_at = CASE WHEN ?2 IN ('done','partial','stopped','error') THEN datetime('now') ELSE finished_at END
             WHERE id = ?1",
            params![id, status.as_str(), error],
        ).map_err(sqlite)?;
        drop(db);
        if changed == 0 {
            Ok(None)
        } else {
            self.get_worker_run(id)
        }
    }

    pub fn update_worker_worktree(
        &self,
        id: &str,
        worktree_path: &str,
    ) -> Result<Option<AgentThread>> {
        let db = self.db.lock().expect("threads db poisoned");
        let changed = db
            .conn()
            .execute(
                "UPDATE threads SET worktree_path = ?2, updated_at = datetime('now') WHERE id = ?1",
                params![id, worktree_path],
            )
            .map_err(sqlite)?;
        drop(db);
        if changed == 0 {
            Ok(None)
        } else {
            self.get(id)
        }
    }

    pub fn refresh_worker_run_status(&self, run_id: &str) -> Result<Option<WorkerRun>> {
        let Some(run) = self.get_worker_run(run_id)? else {
            return Ok(None);
        };
        let workers = self.workers_for_run(run_id)?;
        if workers.is_empty() {
            return Ok(Some(run));
        }
        if workers
            .iter()
            .any(|worker| !thread_status_terminal(&worker.status))
        {
            return self.update_worker_run_status(run_id, WorkerRunStatus::Running, None);
        }
        let done = workers
            .iter()
            .filter(|worker| worker.status == THREAD_STATUS_DONE)
            .count();
        let stopped = workers
            .iter()
            .filter(|worker| worker.status == THREAD_STATUS_STOPPED)
            .count();
        let status = if done == workers.len() {
            WorkerRunStatus::Done
        } else if stopped == workers.len() {
            WorkerRunStatus::Stopped
        } else if done > 0 {
            WorkerRunStatus::Partial
        } else {
            WorkerRunStatus::Error
        };
        self.update_worker_run_status(run_id, status, None)
    }

    pub fn update_non_terminal_worker_runs(&self, error: &str) -> Result<usize> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn().execute(
            "UPDATE worker_runs SET status = 'error', error = COALESCE(error, ?1), updated_at = datetime('now'), finished_at = datetime('now')
             WHERE status IN ('proposed','running')",
            params![error],
        ).map_err(sqlite)
    }

    pub fn get(&self, id: &str) -> Result<Option<AgentThread>> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .query_row(
                "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                        summary, error, created_at, updated_at, finished_at,
                        run_id, runtime, access, external_runtime_id, worktree_path
                 FROM threads WHERE id = ?1",
                params![id],
                row_to_thread,
            )
            .optional()
            .map_err(sqlite)
    }

    pub fn list_children(
        &self,
        parent_id: &str,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentThread>> {
        let db = self.db.lock().expect("threads db poisoned");
        let conn = db.conn();
        let limit = limit.clamp(1, 200) as i64;
        let mut out = Vec::new();
        if let Some(status) = status {
            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                            summary, error, created_at, updated_at, finished_at,
                            run_id, runtime, access, external_runtime_id, worktree_path
                     FROM threads
                     WHERE parent_id = ?1 AND status = ?2
                     ORDER BY created_at DESC
                     LIMIT ?3",
                )
                .map_err(sqlite)?;
            for row in stmt
                .query_map(params![parent_id, status, limit], row_to_thread)
                .map_err(sqlite)?
            {
                out.push(row.map_err(sqlite)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                            summary, error, created_at, updated_at, finished_at,
                            run_id, runtime, access, external_runtime_id, worktree_path
                     FROM threads
                     WHERE parent_id = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2",
                )
                .map_err(sqlite)?;
            for row in stmt
                .query_map(params![parent_id, limit], row_to_thread)
                .map_err(sqlite)?
            {
                out.push(row.map_err(sqlite)?);
            }
        }
        Ok(out)
    }

    pub fn list(&self, status: Option<&str>, limit: usize) -> Result<Vec<AgentThread>> {
        let db = self.db.lock().expect("threads db poisoned");
        let conn = db.conn();
        let limit = limit.clamp(1, 200) as i64;
        let mut out = Vec::new();
        if let Some(status) = status {
            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                            summary, error, created_at, updated_at, finished_at,
                            run_id, runtime, access, external_runtime_id, worktree_path
                     FROM threads
                     WHERE status = ?1
                     ORDER BY created_at DESC
                     LIMIT ?2",
                )
                .map_err(sqlite)?;
            for row in stmt
                .query_map(params![status, limit], row_to_thread)
                .map_err(sqlite)?
            {
                out.push(row.map_err(sqlite)?);
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT id, parent_id, root_id, title, status, model, agent_id, prompt,
                            summary, error, created_at, updated_at, finished_at,
                            run_id, runtime, access, external_runtime_id, worktree_path
                     FROM threads
                     ORDER BY created_at DESC
                     LIMIT ?1",
                )
                .map_err(sqlite)?;
            for row in stmt
                .query_map(params![limit], row_to_thread)
                .map_err(sqlite)?
            {
                out.push(row.map_err(sqlite)?);
            }
        }
        Ok(out)
    }

    pub fn update_status(
        &self,
        id: &str,
        status: &str,
        summary: Option<&str>,
        error: Option<&str>,
    ) -> Result<Option<AgentThread>> {
        let changed = {
            let db = self.db.lock().expect("threads db poisoned");
            db.conn()
                .execute(
                    "UPDATE threads
                     SET status = ?2,
                         summary = COALESCE(?3, summary),
                         error = COALESCE(?4, error),
                         updated_at = datetime('now'),
                         finished_at = CASE
                           WHEN ?2 IN ('done', 'stopped', 'error') THEN datetime('now')
                           ELSE finished_at
                         END
                     WHERE id = ?1
                       AND status NOT IN ('done', 'stopped', 'error')",
                    params![id, status, summary, error],
                )
                .map_err(sqlite)?
        };
        if changed == 0 {
            return Ok(None);
        }
        self.get(id)
    }

    pub fn update_non_terminal_status(&self, status: &str, error: Option<&str>) -> Result<usize> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute(
                "UPDATE threads
                 SET status = ?1,
                     error = COALESCE(?2, error),
                     updated_at = datetime('now'),
                     finished_at = CASE
                       WHEN ?1 IN ('done', 'stopped', 'error') THEN datetime('now')
                       ELSE finished_at
                     END
                 WHERE status NOT IN ('done', 'stopped', 'error')",
                params![status, error],
            )
            .map_err(sqlite)
    }

    pub fn update_running_status(&self, status: &str, error: Option<&str>) -> Result<usize> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute(
                "UPDATE threads
                 SET status = ?1,
                     error = COALESCE(?2, error),
                     updated_at = datetime('now'),
                     finished_at = CASE
                       WHEN ?1 IN ('done', 'stopped', 'error') THEN datetime('now')
                       ELSE finished_at
                     END
                 WHERE status = 'running'",
                params![status, error],
            )
            .map_err(sqlite)
    }

    pub fn append_event(&self, thread_id: &str, kind: &str, payload: Value) -> Result<ThreadEvent> {
        let id = uuid::Uuid::new_v4().to_string();
        let payload_json = serde_json::to_string(&payload)?;
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute(
                "INSERT INTO thread_events (id, thread_id, seq, kind, payload)
                 VALUES (?1, ?2, COALESCE((SELECT MAX(seq) FROM thread_events), 0) + 1, ?3, ?4)",
                params![id, thread_id, kind, payload_json],
            )
            .map_err(sqlite)?;
        drop(db);
        self.get_event(&id)?
            .ok_or_else(|| Error::Other("thread event insert did not return a row".to_string()))
    }

    pub fn events(&self, thread_id: &str, limit: usize) -> Result<Vec<ThreadEvent>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, thread_id, seq, kind, payload, created_at
                 FROM (
                   SELECT id, thread_id, seq, kind, payload, created_at
                   FROM thread_events
                   WHERE thread_id = ?1
                   ORDER BY seq DESC
                   LIMIT ?2
                 )
                 ORDER BY seq ASC",
            )
            .map_err(sqlite)?;
        let mut out = Vec::new();
        for row in stmt
            .query_map(
                params![thread_id, limit.clamp(1, 5000) as i64],
                row_to_event,
            )
            .map_err(sqlite)?
        {
            out.push(row.map_err(sqlite)?);
        }
        Ok(out)
    }

    pub fn events_after(
        &self,
        thread_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<ThreadEvent>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id, thread_id, seq, kind, payload, created_at
                 FROM thread_events
                 WHERE thread_id = ?1 AND seq > ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )
            .map_err(sqlite)?;
        let mut out = Vec::new();
        for row in stmt
            .query_map(
                params![thread_id, after_seq, limit.clamp(1, 5000) as i64],
                row_to_event,
            )
            .map_err(sqlite)?
        {
            out.push(row.map_err(sqlite)?);
        }
        Ok(out)
    }

    pub fn child_events_after(
        &self,
        parent_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(AgentThread, ThreadEvent)>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT t.id, t.parent_id, t.root_id, t.title, t.status, t.model, t.agent_id, t.prompt,
                        t.summary, t.error, t.created_at, t.updated_at, t.finished_at,
                        t.run_id, t.runtime, t.access, t.external_runtime_id, t.worktree_path,
                        e.id, e.thread_id, e.seq, e.kind, e.payload, e.created_at
                 FROM thread_events e
                 JOIN threads t ON t.id = e.thread_id
                 WHERE t.parent_id = ?1 AND e.seq > ?2
                 ORDER BY e.seq ASC
                 LIMIT ?3",
            )
            .map_err(sqlite)?;
        let mut out = Vec::new();
        for row in stmt
            .query_map(
                params![parent_id, after_seq, limit.clamp(1, 5000) as i64],
                |r| Ok((row_to_thread(r)?, row_to_event_offset(r, 18)?)),
            )
            .map_err(sqlite)?
        {
            out.push(row.map_err(sqlite)?);
        }
        Ok(out)
    }

    pub fn worker_events_after(
        &self,
        run_id: &str,
        after_seq: i64,
        limit: usize,
    ) -> Result<Vec<(Worker, ThreadEvent)>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut stmt = db.conn().prepare(
            "SELECT t.id, t.parent_id, t.root_id, t.title, t.status, t.model, t.agent_id, t.prompt,
                    t.summary, t.error, t.created_at, t.updated_at, t.finished_at,
                    t.run_id, t.runtime, t.access, t.external_runtime_id, t.worktree_path,
                    e.id, e.thread_id, e.seq, e.kind, e.payload, e.created_at
             FROM thread_events e JOIN threads t ON t.id = e.thread_id
             WHERE t.run_id = ?1 AND e.seq > ?2 ORDER BY e.seq ASC LIMIT ?3"
        ).map_err(sqlite)?;
        let rows = stmt
            .query_map(
                params![run_id, after_seq, limit.clamp(1, 5000) as i64],
                |r| Ok((row_to_thread(r)?, row_to_event_offset(r, 18)?)),
            )
            .map_err(sqlite)?;
        rows.map(|row| row.map_err(sqlite)).collect()
    }

    pub fn event_count(&self, thread_id: &str) -> Result<usize> {
        let db = self.db.lock().expect("threads db poisoned");
        let count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM thread_events WHERE thread_id = ?1",
                params![thread_id],
                |r| r.get(0),
            )
            .map_err(sqlite)?;
        Ok(count.max(0) as usize)
    }

    pub fn tree_ids(&self, id: &str) -> Result<Vec<String>> {
        let db = self.db.lock().expect("threads db poisoned");
        let mut ids = Vec::new();
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT id FROM threads
                 WHERE id = ?1 OR parent_id = ?1 OR root_id = ?1",
            )
            .map_err(sqlite)?;
        for row in stmt
            .query_map(params![id], |r| r.get::<_, String>(0))
            .map_err(sqlite)?
        {
            ids.push(row.map_err(sqlite)?);
        }
        Ok(ids)
    }

    pub fn delete_tree(&self, id: &str) -> Result<Vec<String>> {
        let ids = self.tree_ids(id)?;
        if ids.is_empty() {
            return Ok(ids);
        }
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .execute(
                "DELETE FROM thread_events
                 WHERE thread_id IN (
                   SELECT id FROM threads
                   WHERE id = ?1 OR parent_id = ?1 OR root_id = ?1
                 )",
                params![id],
            )
            .map_err(sqlite)?;
        db.conn()
            .execute(
                "DELETE FROM worker_runs WHERE parent_thread_id = ?1 OR id IN (
                   SELECT DISTINCT run_id FROM threads WHERE id = ?1 OR parent_id = ?1 OR root_id = ?1
                 )",
                params![id],
            )
            .map_err(sqlite)?;
        db.conn()
            .execute(
                "DELETE FROM threads
                 WHERE id = ?1 OR parent_id = ?1 OR root_id = ?1",
                params![id],
            )
            .map_err(sqlite)?;
        Ok(ids)
    }

    fn get_event(&self, id: &str) -> Result<Option<ThreadEvent>> {
        let db = self.db.lock().expect("threads db poisoned");
        db.conn()
            .query_row(
                "SELECT id, thread_id, seq, kind, payload, created_at FROM thread_events WHERE id = ?1",
                params![id],
                row_to_event,
            )
            .optional()
            .map_err(sqlite)
    }
}

pub fn thread_status_terminal(status: &str) -> bool {
    matches!(
        status,
        THREAD_STATUS_DONE | THREAD_STATUS_STOPPED | THREAD_STATUS_ERROR
    )
}

fn row_to_thread(r: &rusqlite::Row) -> rusqlite::Result<AgentThread> {
    Ok(AgentThread {
        id: r.get(0)?,
        parent_id: r.get(1)?,
        root_id: r.get(2)?,
        title: r.get(3)?,
        status: r.get(4)?,
        model: r.get(5)?,
        agent_id: r.get(6)?,
        prompt: r.get(7)?,
        summary: r.get(8)?,
        error: r.get(9)?,
        created_at: r.get(10)?,
        updated_at: r.get(11)?,
        finished_at: r.get(12)?,
        run_id: r.get(13)?,
        runtime: parse_runtime(r.get::<_, String>(14)?.as_str()),
        access: parse_access(r.get::<_, String>(15)?.as_str()),
        external_runtime_id: r.get(16)?,
        worktree_path: r.get(17)?,
    })
}

fn row_to_worker_run(r: &rusqlite::Row) -> rusqlite::Result<WorkerRun> {
    let tasks: String = r.get(6)?;
    Ok(WorkerRun {
        id: r.get(0)?,
        parent_thread_id: r.get(1)?,
        parent_turn_id: r.get(2)?,
        policy: parse_policy(r.get::<_, String>(3)?.as_str()),
        runtime: parse_runtime(r.get::<_, String>(4)?.as_str()),
        status: parse_run_status(r.get::<_, String>(5)?.as_str()),
        tasks: serde_json::from_str(&tasks).unwrap_or_default(),
        error: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        finished_at: r.get(10)?,
        context: r.get(11)?,
        workspace: r.get(12)?,
        privacy_mode: r.get(13)?,
    })
}

fn parse_policy(value: &str) -> DelegationPolicy {
    match value {
        "off" => DelegationPolicy::Off,
        "auto" => DelegationPolicy::Auto,
        _ => DelegationPolicy::Ask,
    }
}

fn parse_runtime(value: &str) -> WorkerRuntime {
    match value {
        "codex" => WorkerRuntime::Codex,
        "claude" => WorkerRuntime::Claude,
        "legacy" => WorkerRuntime::Legacy,
        _ => WorkerRuntime::Managed,
    }
}

fn parse_access(value: &str) -> WorkerAccess {
    match value {
        "write_review" => WorkerAccess::WriteReview,
        _ => WorkerAccess::ReadOnly,
    }
}

fn parse_run_status(value: &str) -> WorkerRunStatus {
    match value {
        "running" => WorkerRunStatus::Running,
        "done" => WorkerRunStatus::Done,
        "partial" => WorkerRunStatus::Partial,
        "stopped" => WorkerRunStatus::Stopped,
        "error" => WorkerRunStatus::Error,
        _ => WorkerRunStatus::Proposed,
    }
}

fn row_to_event(r: &rusqlite::Row) -> rusqlite::Result<ThreadEvent> {
    row_to_event_offset(r, 0)
}

fn row_to_event_offset(r: &rusqlite::Row, offset: usize) -> rusqlite::Result<ThreadEvent> {
    let payload: String = r.get(offset + 4)?;
    Ok(ThreadEvent {
        id: r.get(offset)?,
        thread_id: r.get(offset + 1)?,
        seq: r.get(offset + 2)?,
        kind: r.get(offset + 3)?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        created_at: r.get(offset + 5)?,
    })
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ThreadStore {
        ThreadStore::new(Database::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn create_list_and_update_child_threads() {
        let s = store();
        let thread = s
            .create("parent-1", "Research", "test-echo", None, "look this up")
            .unwrap();
        assert_eq!(thread.parent_id, "parent-1");
        assert_eq!(thread.root_id, "parent-1");
        assert_eq!(thread.status, THREAD_STATUS_QUEUED);

        let running = s
            .update_status(&thread.id, THREAD_STATUS_RUNNING, None, None)
            .unwrap()
            .unwrap();
        assert_eq!(running.status, THREAD_STATUS_RUNNING);

        let done = s
            .update_status(&thread.id, THREAD_STATUS_DONE, Some("finished"), None)
            .unwrap()
            .unwrap();
        assert_eq!(done.summary.as_deref(), Some("finished"));
        assert!(done.finished_at.is_some());

        let children = s.list_children("parent-1", None, 20).unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, thread.id);
        assert!(s
            .list_children("parent-1", Some(THREAD_STATUS_DONE), 20)
            .unwrap()
            .iter()
            .any(|t| t.id == thread.id));
    }

    #[test]
    fn persists_and_finishes_worker_run() {
        let s = store();
        let run = s
            .create_worker_run_with_origin(
                "parent-1",
                Some("turn-1"),
                DelegationPolicy::Ask,
                WorkerRuntime::Managed,
                vec![WorkerPlanTask {
                    id: "task-1".into(),
                    title: "Research".into(),
                    prompt: "check".into(),
                    role: None,
                    agent_id: None,
                    agent_snapshot: None,
                    model: "test-echo".into(),
                    access: WorkerAccess::ReadOnly,
                }],
                Some("Current request: check"),
                Some("C:\\repo"),
                "redact",
            )
            .unwrap();
        assert_eq!(run.context.as_deref(), Some("Current request: check"));
        assert_eq!(run.workspace.as_deref(), Some("C:\\repo"));
        assert_eq!(run.privacy_mode.as_deref(), Some("redact"));
        let worker = s
            .create_worker(
                "parent-1",
                "Research",
                "test-echo",
                None,
                "check",
                Some(&run.id),
                WorkerRuntime::Managed,
                WorkerAccess::ReadOnly,
            )
            .unwrap();
        s.update_status(&worker.id, THREAD_STATUS_RUNNING, None, None)
            .unwrap();
        s.update_worker_run_status(&run.id, WorkerRunStatus::Running, None)
            .unwrap();
        s.update_status(&worker.id, THREAD_STATUS_DONE, Some("done"), None)
            .unwrap();

        let finished = s.refresh_worker_run_status(&run.id).unwrap().unwrap();
        assert_eq!(finished.status, WorkerRunStatus::Done);
        assert_eq!(
            s.list_worker_runs("parent-1", 10).unwrap()[0].tasks[0].prompt,
            "check"
        );
        s.append_event(&worker.id, "final", serde_json::json!({"content":"done"}))
            .unwrap();
        assert!(s.delete_worker_run(&run.id).unwrap());
        assert!(s.get_worker_run(&run.id).unwrap().is_none());
        assert!(s.get(&worker.id).unwrap().is_none());
        assert!(s.events(&worker.id, 10).unwrap().is_empty());
    }

    #[test]
    fn migration_keeps_legacy_worker_runs_readable_without_origin() {
        let db = Database::open_in_memory().unwrap();
        db.migrate_scoped("threads", &THREAD_MIGRATIONS[..4])
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO worker_runs (
                    id, parent_thread_id, policy, runtime, status, tasks
                 ) VALUES ('legacy', 'parent-1', 'ask', 'managed', 'done', '[]')",
                [],
            )
            .unwrap();

        let store = ThreadStore::new(db).unwrap();
        let run = store.get_worker_run("legacy").unwrap().unwrap();
        assert_eq!(run.status, WorkerRunStatus::Done);
        assert!(run.workspace.is_none());
        assert!(run.privacy_mode.is_none());
    }

    #[test]
    fn stores_thread_events() {
        let s = store();
        let thread = s
            .create("parent-1", "Research", "test-echo", None, "look this up")
            .unwrap();
        let event = s
            .append_event(&thread.id, "final", serde_json::json!({"content": "done"}))
            .unwrap();
        assert_eq!(event.thread_id, thread.id);
        assert_eq!(event.payload["content"], "done");

        let events = s.events(&thread.id, 20).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "final");
        assert_eq!(events[0].seq, event.seq);
        assert_eq!(s.event_count(&thread.id).unwrap(), 1);
    }

    #[test]
    fn four_workers_append_ordered_events_concurrently() {
        let store = std::sync::Arc::new(store());
        let worker_ids = (0..4)
            .map(|index| {
                store
                    .create(
                        "parent-1",
                        &format!("Worker {index}"),
                        "test-echo",
                        None,
                        "work",
                    )
                    .unwrap()
                    .id
            })
            .collect::<Vec<_>>();
        let handles = worker_ids
            .into_iter()
            .map(|worker_id| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for index in 0..25 {
                        store
                            .append_event(&worker_id, "token", serde_json::json!({"index": index}))
                            .unwrap();
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let events = store.child_events_after("parent-1", 0, 500).unwrap();
        assert_eq!(events.len(), 100);
        assert!(events.windows(2).all(|pair| pair[0].1.seq < pair[1].1.seq));
    }

    #[test]
    fn event_reads_preserve_tail_and_cursor_order() {
        let s = store();
        let thread = s
            .create("parent-1", "Research", "test-echo", None, "look this up")
            .unwrap();
        let other = s
            .create("parent-1", "Other", "test-echo", None, "look elsewhere")
            .unwrap();
        let first = s
            .append_event(&thread.id, "token", serde_json::json!({"text": "one"}))
            .unwrap();
        let second = s
            .append_event(&thread.id, "token", serde_json::json!({"text": "two"}))
            .unwrap();
        let third = s
            .append_event(&thread.id, "final", serde_json::json!({"content": "three"}))
            .unwrap();
        let other_event = s
            .append_event(&other.id, "final", serde_json::json!({"content": "other"}))
            .unwrap();

        assert!(first.seq < second.seq);
        assert!(second.seq < third.seq);
        assert!(third.seq < other_event.seq);

        let tail = s.events(&thread.id, 2).unwrap();
        assert_eq!(
            tail.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["token", "final"]
        );
        assert_eq!(tail[0].seq, second.seq);
        assert_eq!(tail[1].seq, third.seq);

        let after = s.events_after(&thread.id, first.seq, 10).unwrap();
        assert_eq!(
            after.iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![second.seq, third.seq]
        );

        let child_events = s.child_events_after("parent-1", second.seq, 10).unwrap();
        assert_eq!(
            child_events.iter().map(|(_, e)| e.seq).collect::<Vec<_>>(),
            vec![third.seq, other_event.seq]
        );
    }

    #[test]
    fn terminal_status_is_not_overwritten() {
        let s = store();
        let thread = s
            .create("parent-1", "Research", "test-echo", None, "look this up")
            .unwrap();
        let stopped = s
            .update_status(&thread.id, THREAD_STATUS_STOPPED, None, Some("stopped"))
            .unwrap()
            .unwrap();
        assert_eq!(stopped.status, THREAD_STATUS_STOPPED);

        let done = s
            .update_status(&thread.id, THREAD_STATUS_DONE, Some("finished"), None)
            .unwrap();
        assert!(done.is_none());
        assert_eq!(
            s.get(&thread.id).unwrap().unwrap().status,
            THREAD_STATUS_STOPPED
        );
    }

    #[test]
    fn deletes_parent_thread_tree_and_events() {
        let s = store();
        let child = s
            .create("parent-1", "Research", "test-echo", None, "look this up")
            .unwrap();
        let grandchild = s
            .create(&child.id, "Follow-up", "test-echo", None, "look deeper")
            .unwrap();
        s.append_event(&child.id, "final", serde_json::json!({"content": "done"}))
            .unwrap();
        s.append_event(
            &grandchild.id,
            "final",
            serde_json::json!({"content": "deeper"}),
        )
        .unwrap();

        let deleted = s.delete_tree("parent-1").unwrap();

        assert!(deleted.contains(&child.id));
        assert!(deleted.contains(&grandchild.id));
        assert!(s.list_children("parent-1", None, 20).unwrap().is_empty());
        assert!(s.events(&child.id, 20).unwrap().is_empty());
        assert!(s.events(&grandchild.id, 20).unwrap().is_empty());
    }
}
