//! `milim-automation` — scheduled (cron) agent runs.
//!
//! A [`ScheduleStore`] persists cron schedules; [`ScheduleStore::due`] reports
//! which are ready to fire. The server owns a background loop that calls `due`,
//! runs each schedule's agent, and calls [`ScheduleStore::mark_ran`]. Cron math
//! is deterministic and unit-tested independent of wall-clock.

use std::str::FromStr;
use std::sync::Mutex;

use base64::Engine;
use chrono::{DateTime, Local, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use milim_core::api::openai::{ChatMessage, Content, ContentPart, ImageUrl};
use milim_core::{Error, Result};
use milim_storage::{Database, Migration};

pub const MAX_SCHEDULE_ATTACHMENTS: usize = 12;
pub const MAX_SCHEDULE_ATTACHMENT_BYTES: usize = 128 * 1024;
pub const MAX_SCHEDULE_IMAGE_BYTES: usize = 2 * 1024 * 1024;

/// File context saved with a schedule and appended to the prompt on each fire.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleAttachment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// A cron schedule that runs an agent with a fixed prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    /// 6-field cron expression: `sec min hour day month dow`.
    pub cron: String,
    /// Agent to run (None ⇒ the generic loop).
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Model used for this unattended run. Empty only for legacy schedules.
    #[serde(default)]
    pub model: String,
    /// The user message to send on each fire.
    pub prompt: String,
    /// Optional file context appended when the schedule fires.
    #[serde(default)]
    pub attachments: Vec<ScheduleAttachment>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Workspace captured when the schedule was created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Outbound privacy mode frozen for unattended runs.
    #[serde(default = "default_privacy")]
    pub privacy: String,
    /// Cron evaluation zone: `local` for the host clock or `utc`.
    #[serde(default = "default_timezone_mode")]
    pub timezone_mode: String,
    /// Creation time used as the lower bound for the first cron occurrence.
    #[serde(default)]
    pub created_unix: i64,
    /// Last fire time (unix seconds), if any.
    #[serde(default)]
    pub last_run: Option<i64>,
}

/// One enabled schedule occurrence ready for dispatch.
#[derive(Debug, Clone)]
pub struct DueSchedule {
    pub schedule: Schedule,
    pub scheduled_for: i64,
}

/// Named payload for updating an existing schedule.
pub struct ScheduleUpdate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub cron: &'a str,
    pub agent_id: Option<String>,
    pub model: &'a str,
    pub prompt: &'a str,
    pub attachments: Vec<ScheduleAttachment>,
    pub enabled: bool,
    pub workspace: Option<String>,
    pub privacy: String,
    pub timezone_mode: String,
    pub created_unix: i64,
    pub last_run: Option<i64>,
}

fn default_true() -> bool {
    true
}

fn default_privacy() -> String {
    "off".to_string()
}

fn default_timezone_mode() -> String {
    "utc".to_string()
}

/// Schema for the schedule store.
pub const SCHEDULE_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "schedules",
        sql: "CREATE TABLE schedules (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            cron       TEXT NOT NULL,
            agent_id   TEXT,
            prompt     TEXT NOT NULL DEFAULT '',
            enabled    INTEGER NOT NULL DEFAULT 1,
            last_run   INTEGER,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
          );",
    },
    Migration {
        version: 2,
        name: "schedule_attachments",
        sql: "ALTER TABLE schedules ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';",
    },
    Migration {
        version: 3,
        name: "schedule_execution_context",
        sql: "ALTER TABLE schedules ADD COLUMN workspace TEXT;
              ALTER TABLE schedules ADD COLUMN created_unix INTEGER NOT NULL DEFAULT 0;
              UPDATE schedules
              SET created_unix = COALESCE(CAST(strftime('%s', created_at) AS INTEGER), CAST(strftime('%s', 'now') AS INTEGER))
              WHERE created_unix = 0;",
    },
    Migration {
        version: 4,
        name: "schedule_model",
        sql: "ALTER TABLE schedules ADD COLUMN model TEXT NOT NULL DEFAULT '';",
    },
    Migration {
        version: 5,
        name: "schedule_run_boundaries",
        sql: "ALTER TABLE schedules ADD COLUMN privacy TEXT NOT NULL DEFAULT 'off';
              ALTER TABLE schedules ADD COLUMN timezone_mode TEXT NOT NULL DEFAULT 'utc';",
    },
];

pub fn prompt_with_attachments(prompt: &str, attachments: &[ScheduleAttachment]) -> String {
    let context = attachments_prompt_context(attachments);
    if context.is_empty() {
        prompt.to_string()
    } else if prompt.trim().is_empty() {
        context
    } else {
        format!("{prompt}\n\n{context}")
    }
}

pub fn message_with_attachments(
    prompt: &str,
    attachments: &[ScheduleAttachment],
) -> Result<ChatMessage> {
    let text = prompt_with_attachments(prompt, attachments);
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(ContentPart::Text { text });
    }
    for attachment in attachments {
        if !attachment.mime.to_ascii_lowercase().starts_with("image/") {
            continue;
        }
        if !matches!(
            attachment.mime.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(Error::InvalidRequest(format!(
                "scheduled image '{}' must be PNG, JPEG, WebP, or GIF",
                attachment.name
            )));
        }
        let data_url = attachment.data_url.as_deref().ok_or_else(|| {
            Error::InvalidRequest(format!(
                "scheduled image '{}' has no image data; reattach it before this schedule can run",
                attachment.name
            ))
        })?;
        validate_schedule_image(data_url, &attachment.mime, &attachment.name)?;
        parts.push(ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: data_url.to_string(),
                detail: None,
            },
        });
    }
    Ok(ChatMessage {
        role: "user".to_string(),
        content: Some(Content::Parts(parts)),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        reasoning_content: None,
    })
}

fn validate_schedule_image(data_url: &str, mime: &str, name: &str) -> Result<()> {
    let prefix = format!("data:{mime};base64,");
    let data = data_url.strip_prefix(&prefix).ok_or_else(|| {
        Error::InvalidRequest(format!(
            "scheduled image '{name}' has malformed image data; reattach it"
        ))
    })?;
    if data.len() > MAX_SCHEDULE_IMAGE_BYTES * 4 / 3 + 8 {
        return Err(Error::InvalidRequest(format!(
            "scheduled image '{name}' exceeds the 2 MB image limit"
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| {
            Error::InvalidRequest(format!(
                "scheduled image '{name}' has malformed image data; reattach it"
            ))
        })?;
    if bytes.is_empty() || bytes.len() > MAX_SCHEDULE_IMAGE_BYTES {
        return Err(Error::InvalidRequest(format!(
            "scheduled image '{name}' must contain 1 byte to 2 MB of data"
        )));
    }
    let signature_matches = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    };
    if !signature_matches {
        return Err(Error::InvalidRequest(format!(
            "scheduled image '{name}' bytes do not match {mime}; reattach it"
        )));
    }
    Ok(())
}

fn validate_schedule_attachments(attachments: &[ScheduleAttachment]) -> Result<()> {
    if attachments.len() > MAX_SCHEDULE_ATTACHMENTS {
        return Err(Error::InvalidRequest(format!(
            "a schedule may contain at most {MAX_SCHEDULE_ATTACHMENTS} attachments"
        )));
    }
    for attachment in attachments {
        if !attachment.mime.to_ascii_lowercase().starts_with("image/") {
            if attachment.content.is_none() {
                return Err(Error::InvalidRequest(format!(
                    "scheduled attachment '{}' is not a supported text or image file",
                    attachment.name
                )));
            }
            continue;
        }
        if !matches!(
            attachment.mime.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(Error::InvalidRequest(format!(
                "scheduled image '{}' must be PNG, JPEG, WebP, or GIF",
                attachment.name
            )));
        }
        let data_url = attachment.data_url.as_deref().ok_or_else(|| {
            Error::InvalidRequest(format!(
                "scheduled image '{}' has no image data; reattach it",
                attachment.name
            ))
        })?;
        validate_schedule_image(data_url, &attachment.mime, &attachment.name)?;
    }
    Ok(())
}

pub fn attachments_prompt_context(attachments: &[ScheduleAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }

    let blocks = attachments
        .iter()
        .map(|attachment| {
            let mut meta = vec![
                format!("name={}", attachment.name),
                format!(
                    "mime={}",
                    if attachment.mime.is_empty() {
                        "application/octet-stream"
                    } else {
                        attachment.mime.as_str()
                    }
                ),
                format!("size={}", attachment.size),
            ];
            if attachment.truncated {
                meta.push(format!("truncated_at={MAX_SCHEDULE_ATTACHMENT_BYTES}"));
            }
            if let Some(path) = attachment
                .source_path
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                meta.push(format!("path={path}"));
            }

            let content = attachment
                .content
                .as_deref()
                .map(str::trim_end)
                .filter(|value| !value.is_empty());
            let image_note = attachment.data_url.is_some()
                || attachment.mime.to_lowercase().starts_with("image/");
            let body = content.map(ToString::to_string).unwrap_or_else(|| {
                if image_note {
                    "[Image attached as multimodal input.]".to_string()
                } else {
                    "[No text content available for this attachment.]".to_string()
                }
            });

            format!(
                "--- attachment {} ---\n{body}\n--- end attachment ---",
                meta.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("[Attached files]\n{blocks}\n[/Attached files]")
}

/// Next fire time (unix seconds) strictly after `after_unix`, per `cron_expr`.
pub fn cron_next_after(cron_expr: &str, after_unix: i64) -> Result<Option<i64>> {
    cron_next_after_in_timezone(cron_expr, after_unix, "utc")
}

pub fn cron_next_after_in_timezone(
    cron_expr: &str,
    after_unix: i64,
    timezone_mode: &str,
) -> Result<Option<i64>> {
    let schedule = cron::Schedule::from_str(cron_expr)
        .map_err(|e| Error::InvalidRequest(format!("invalid cron expression: {e}")))?;
    match timezone_mode {
        "utc" => {
            let after = DateTime::<Utc>::from_timestamp(after_unix, 0)
                .ok_or_else(|| Error::Other("timestamp out of range".to_string()))?;
            Ok(schedule.after(&after).next().map(|dt| dt.timestamp()))
        }
        "local" => {
            let after = DateTime::<Utc>::from_timestamp(after_unix, 0)
                .ok_or_else(|| Error::Other("timestamp out of range".to_string()))?
                .with_timezone(&Local);
            Ok(schedule.after(&after).next().map(|dt| dt.timestamp()))
        }
        value => Err(Error::InvalidRequest(format!(
            "unsupported schedule timezone: {value}"
        ))),
    }
}

pub fn schedule_next_run(schedule: &Schedule) -> Result<Option<i64>> {
    cron_next_after_in_timezone(
        &schedule.cron,
        schedule.last_run.unwrap_or(schedule.created_unix),
        &schedule.timezone_mode,
    )
}

fn validate_cron(cron_expr: &str) -> Result<()> {
    cron::Schedule::from_str(cron_expr)
        .map(|_| ())
        .map_err(|e| Error::InvalidRequest(format!("invalid cron expression: {e}")))
}

fn validate_run_context(privacy: &str, timezone_mode: &str) -> Result<()> {
    if !matches!(privacy, "off" | "redact" | "block") {
        return Err(Error::InvalidRequest(format!(
            "unsupported schedule privacy mode: {privacy}"
        )));
    }
    if !matches!(timezone_mode, "local" | "utc") {
        return Err(Error::InvalidRequest(format!(
            "unsupported schedule timezone: {timezone_mode}"
        )));
    }
    Ok(())
}

/// CRUD + due-selection over [`Schedule`] rows.
pub struct ScheduleStore {
    db: Mutex<Database>,
}

impl ScheduleStore {
    pub fn new(db: Database) -> Result<Self> {
        db.migrate(SCHEDULE_MIGRATIONS)?;
        Ok(Self { db: Mutex::new(db) })
    }

    /// Create a schedule (validates the cron expression).
    pub fn create(
        &self,
        name: &str,
        cron: &str,
        agent_id: Option<String>,
        prompt: &str,
    ) -> Result<Schedule> {
        self.create_with_attachments(name, cron, agent_id, prompt, Vec::new())
    }

    pub fn create_with_attachments(
        &self,
        name: &str,
        cron: &str,
        agent_id: Option<String>,
        prompt: &str,
        attachments: Vec<ScheduleAttachment>,
    ) -> Result<Schedule> {
        self.create_with_context(name, cron, agent_id, prompt, attachments, true, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_context(
        &self,
        name: &str,
        cron: &str,
        agent_id: Option<String>,
        prompt: &str,
        attachments: Vec<ScheduleAttachment>,
        enabled: bool,
        workspace: Option<String>,
    ) -> Result<Schedule> {
        self.create_with_model_context(
            name,
            cron,
            agent_id,
            "",
            prompt,
            attachments,
            enabled,
            workspace,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_model_context(
        &self,
        name: &str,
        cron: &str,
        agent_id: Option<String>,
        model: &str,
        prompt: &str,
        attachments: Vec<ScheduleAttachment>,
        enabled: bool,
        workspace: Option<String>,
    ) -> Result<Schedule> {
        self.create_with_run_context(
            name,
            cron,
            agent_id,
            model,
            prompt,
            attachments,
            enabled,
            workspace,
            "off",
            "local",
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_with_run_context(
        &self,
        name: &str,
        cron: &str,
        agent_id: Option<String>,
        model: &str,
        prompt: &str,
        attachments: Vec<ScheduleAttachment>,
        enabled: bool,
        workspace: Option<String>,
        privacy: &str,
        timezone_mode: &str,
    ) -> Result<Schedule> {
        validate_cron(cron)?;
        validate_run_context(privacy, timezone_mode)?;
        validate_schedule_attachments(&attachments)?;
        let attachments = normalize_attachments(attachments);
        validate_schedule_attachments(&attachments)?;
        let schedule = Schedule {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            cron: cron.to_string(),
            agent_id,
            model: model.trim().to_string(),
            prompt: prompt.to_string(),
            attachments,
            enabled,
            workspace,
            privacy: privacy.to_string(),
            timezone_mode: timezone_mode.to_string(),
            created_unix: Utc::now().timestamp(),
            last_run: None,
        };
        self.upsert(&schedule)?;
        Ok(schedule)
    }

    pub fn upsert(&self, s: &Schedule) -> Result<()> {
        let db = self.db.lock().expect("schedules db poisoned");
        let attachments_json = serde_json::to_string(&normalize_attachments(s.attachments.clone()))
            .map_err(|e| Error::Other(format!("schedule attachments json: {e}")))?;
        db.conn()
            .execute(
                "INSERT INTO schedules (id, name, cron, agent_id, model, prompt, attachments_json, enabled, workspace, privacy, timezone_mode, created_unix, last_run)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name, cron=excluded.cron, agent_id=excluded.agent_id,
                   model=excluded.model, prompt=excluded.prompt, attachments_json=excluded.attachments_json,
                   enabled=excluded.enabled, workspace=excluded.workspace,
                   privacy=excluded.privacy, timezone_mode=excluded.timezone_mode,
                   created_unix=excluded.created_unix, last_run=excluded.last_run",
                params![
                    s.id,
                    s.name,
                    s.cron,
                    s.agent_id,
                    s.model,
                    s.prompt,
                    attachments_json,
                    s.enabled as i64,
                    s.workspace,
                    s.privacy,
                    s.timezone_mode,
                    s.created_unix,
                    s.last_run
                ],
            )
            .map_err(sqlite)?;
        Ok(())
    }

    /// Update an existing schedule, preserving the supplied id.
    pub fn update(&self, update: ScheduleUpdate<'_>) -> Result<Schedule> {
        validate_cron(update.cron)?;
        validate_run_context(&update.privacy, &update.timezone_mode)?;
        validate_schedule_attachments(&update.attachments)?;
        let attachments = normalize_attachments(update.attachments);
        validate_schedule_attachments(&attachments)?;
        let schedule = Schedule {
            id: update.id.to_string(),
            name: update.name.to_string(),
            cron: update.cron.to_string(),
            agent_id: update.agent_id,
            model: update.model.trim().to_string(),
            prompt: update.prompt.to_string(),
            attachments,
            enabled: update.enabled,
            workspace: update.workspace,
            privacy: update.privacy,
            timezone_mode: update.timezone_mode,
            created_unix: update.created_unix,
            last_run: update.last_run,
        };
        self.upsert(&schedule)?;
        Ok(schedule)
    }

    pub fn list(&self) -> Result<Vec<Schedule>> {
        let db = self.db.lock().expect("schedules db poisoned");
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, cron, agent_id, model, prompt, attachments_json, enabled, workspace, privacy, timezone_mode, created_unix, last_run
                 FROM schedules ORDER BY created_at DESC",
            )
            .map_err(sqlite)?;
        let rows = stmt.query_map([], row_to_schedule).map_err(sqlite)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(sqlite)?);
        }
        Ok(out)
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().expect("schedules db poisoned");
        let n = db
            .conn()
            .execute("DELETE FROM schedules WHERE id = ?1", params![id])
            .map_err(sqlite)?;
        Ok(n > 0)
    }

    /// Record that a schedule fired at `now_unix`.
    pub fn mark_ran(&self, id: &str, now_unix: i64) -> Result<()> {
        let db = self.db.lock().expect("schedules db poisoned");
        let rows = db
            .conn()
            .execute(
                "UPDATE schedules SET last_run = ?2 WHERE id = ?1",
                params![id, now_unix],
            )
            .map_err(sqlite)?;
        if rows == 0 {
            return Err(Error::Other(format!("schedule not found: {id}")));
        }
        Ok(())
    }

    /// Enabled schedules whose next fire time (after their last run) is ≤ `now_unix`.
    pub fn due(&self, now_unix: i64) -> Result<Vec<DueSchedule>> {
        let mut due = Vec::new();
        for s in self.list()?.into_iter().filter(|s| s.enabled) {
            let after = s.last_run.unwrap_or(s.created_unix);
            if let Some(next) = cron_next_after_in_timezone(&s.cron, after, &s.timezone_mode)? {
                if next <= now_unix {
                    due.push(DueSchedule {
                        schedule: s,
                        scheduled_for: next,
                    });
                }
            }
        }
        Ok(due)
    }
}

fn row_to_schedule(r: &rusqlite::Row) -> rusqlite::Result<Schedule> {
    Ok(Schedule {
        id: r.get(0)?,
        name: r.get(1)?,
        cron: r.get(2)?,
        agent_id: r.get(3)?,
        model: r.get(4)?,
        prompt: r.get(5)?,
        attachments: parse_attachments_json(r.get::<_, String>(6)?),
        enabled: r.get::<_, i64>(7)? != 0,
        workspace: r.get(8)?,
        privacy: r.get(9)?,
        timezone_mode: r.get(10)?,
        created_unix: r.get(11)?,
        last_run: r.get(12)?,
    })
}

fn parse_attachments_json(json: String) -> Vec<ScheduleAttachment> {
    serde_json::from_str::<Vec<ScheduleAttachment>>(&json)
        .map(normalize_attachments)
        .unwrap_or_default()
}

fn normalize_attachments(mut attachments: Vec<ScheduleAttachment>) -> Vec<ScheduleAttachment> {
    attachments.truncate(MAX_SCHEDULE_ATTACHMENTS);
    for attachment in &mut attachments {
        if attachment.id.trim().is_empty() {
            attachment.id = uuid::Uuid::new_v4().to_string();
        }
        attachment.name = attachment.name.trim().to_string();
        if attachment.name.is_empty() {
            attachment.name = "attachment".to_string();
        }
        attachment.mime = attachment.mime.trim().to_string();
        if attachment.mime.is_empty() {
            attachment.mime = "application/octet-stream".to_string();
        }
        attachment.source_path = attachment
            .source_path
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if let Some(content) = attachment.content.take() {
            let (content, clipped) = truncate_to_bytes(content, MAX_SCHEDULE_ATTACHMENT_BYTES);
            attachment.content = Some(content);
            attachment.truncated |= clipped;
        }
    }
    attachments
}

fn truncate_to_bytes(value: String, max: usize) -> (String, bool) {
    if value.len() <= max {
        return (value, false);
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

fn sqlite(e: rusqlite::Error) -> Error {
    Error::Other(format!("sqlite: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ScheduleStore {
        ScheduleStore::new(Database::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn cron_next_is_deterministic() {
        // "sec=0 min=0 every hour" → next after epoch (00:00:00) is 01:00:00.
        assert_eq!(cron_next_after("0 0 * * * *", 0).unwrap(), Some(3600));
        // After 00:30:00 (1800) → still 01:00:00.
        assert_eq!(cron_next_after("0 0 * * * *", 1800).unwrap(), Some(3600));
    }

    #[test]
    fn rejects_bad_cron() {
        assert!(store().create("x", "not a cron", None, "p").is_err());
    }

    #[test]
    fn create_list_delete() {
        let s = store();
        let sched = s.create("hourly", "0 0 * * * *", None, "tick").unwrap();
        assert!(sched.model.is_empty());
        assert_eq!(s.list().unwrap().len(), 1);
        assert!(s.delete(&sched.id).unwrap());
        assert!(s.list().unwrap().is_empty());
    }

    #[test]
    fn legacy_schedules_migrate_with_empty_model() {
        let db = Database::open_in_memory().unwrap();
        db.migrate(&SCHEDULE_MIGRATIONS[..3]).unwrap();
        db.conn()
            .execute(
                "INSERT INTO schedules (id, name, cron, prompt) VALUES ('legacy', 'Legacy', '0 0 * * * *', 'tick')",
                [],
            )
            .unwrap();

        let store = ScheduleStore::new(db).unwrap();
        assert!(store.list().unwrap()[0].model.is_empty());
    }

    #[test]
    fn due_selects_ready_schedules() {
        let s = store();
        let mut sched = s.create("hourly", "0 0 * * * *", None, "tick").unwrap();
        assert!(s.due(sched.created_unix).unwrap().is_empty());
        sched.created_unix = 0;
        s.upsert(&sched).unwrap();
        // last_run None → after=0 → next fire 3600; now far in the future → due.
        let due = s.due(10_000).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].schedule.id, sched.id);
        assert_eq!(due[0].scheduled_for, 3600);

        // After marking it ran at now, it's no longer due until the next hour.
        s.mark_ran(&sched.id, 10_000).unwrap();
        let due_after = s.due(10_050).unwrap();
        assert!(due_after.is_empty());
    }

    #[test]
    fn mark_ran_requires_existing_schedule() {
        assert!(store().mark_ran("missing", 10_000).is_err());
    }

    #[test]
    fn attachments_round_trip_and_prompt_context() {
        let s = store();
        let sched = s
            .create_with_attachments(
                "hourly",
                "0 0 * * * *",
                None,
                "summarize this",
                vec![ScheduleAttachment {
                    id: "att-1".to_string(),
                    name: "notes.md".to_string(),
                    mime: "text/markdown".to_string(),
                    size: 14,
                    content: Some("hello\nworld".to_string()),
                    data_url: None,
                    truncated: false,
                    source_path: Some("C:\\tmp\\notes.md".to_string()),
                }],
            )
            .unwrap();

        let listed = s.list().unwrap();
        assert_eq!(listed[0].attachments.len(), 1);
        assert_eq!(listed[0].attachments[0].name, "notes.md");

        let prompt = prompt_with_attachments(&sched.prompt, &sched.attachments);
        assert!(prompt.contains("[Attached files]"));
        assert!(prompt.contains("name=notes.md"));
        assert!(prompt.contains("path=C:\\tmp\\notes.md"));
        assert!(prompt.contains("hello\nworld"));
    }

    #[test]
    fn scheduled_images_become_multimodal_user_parts() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEklEQVR4nGP4z8DAAMIM/4EAAB/uBfsL2WiLAAAAAElFTkSuQmCC";
        let message = message_with_attachments(
            "Describe the shape",
            &[ScheduleAttachment {
                id: "image-1".to_string(),
                name: "geometry.png".to_string(),
                mime: "image/png".to_string(),
                size: 75,
                content: None,
                data_url: Some(format!("data:image/png;base64,{png}")),
                truncated: false,
                source_path: None,
            }],
        )
        .unwrap();
        let Some(Content::Parts(parts)) = message.content else {
            panic!("expected multimodal scheduled message");
        };
        assert_eq!(parts.len(), 2);
        assert!(matches!(parts[0], ContentPart::Text { .. }));
        assert!(matches!(parts[1], ContentPart::ImageUrl { .. }));
    }

    #[test]
    fn legacy_scheduled_images_without_pixels_fail_visibly() {
        let error = message_with_attachments(
            "Describe the shape",
            &[ScheduleAttachment {
                id: "legacy".to_string(),
                name: "lost.png".to_string(),
                mime: "image/png".to_string(),
                size: 75,
                content: None,
                data_url: None,
                truncated: false,
                source_path: None,
            }],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("has no image data; reattach it"));
    }

    #[test]
    fn schedule_execution_context_round_trips() {
        let store = store();
        let schedule = store
            .create_with_model_context(
                "daily",
                "0 0 9 * * *",
                Some("agent-1".to_string()),
                "provider/model",
                "check",
                Vec::new(),
                false,
                Some("C:\\repo".to_string()),
            )
            .unwrap();
        let loaded = store.list().unwrap().remove(0);
        assert_eq!(loaded.workspace.as_deref(), Some("C:\\repo"));
        assert_eq!(loaded.model, "provider/model");
        assert_eq!(loaded.created_unix, schedule.created_unix);
        assert!(!loaded.enabled);
    }
}
