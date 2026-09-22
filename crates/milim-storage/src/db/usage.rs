//! Usage and cost aggregation over canonical assistant messages.
//!
//! Each completed assistant message stores a `metrics` snapshot (model,
//! provider, token usage, and cost with its provenance), and compaction
//! checkpoints store the metrics of their summary call. Partial expression
//! indexes over those timestamps (user-data migration 11) keep a bounded
//! date-range scan from touching messages without metrics.

use std::collections::{BTreeMap, HashMap};

use rusqlite::types::ValueRef;
use rusqlite::{params, Row};
use serde::Serialize;

use super::{sqlite, Error, Result, UserDataStore};

const DAY_MS: i64 = 86_400_000;
/// Longest selectable range, in days.
pub const USAGE_MAX_DAYS: u32 = 366;

/// Token and cost totals for one slice of usage.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UsageTotals {
    /// Responses and compaction summaries with a metrics snapshot.
    pub responses: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Sum of every known cost, reported and estimated.
    pub cost_usd: f64,
    /// Portion billed and reported by a provider or account runtime.
    pub reported_cost_usd: f64,
    /// Portion estimated from cached per-token pricing.
    pub estimated_cost_usd: f64,
    /// Responses that used tokens but have no recorded cost.
    pub unpriced_responses: u64,
}

/// One labelled group (a day, model, provider, or project).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageBucket {
    pub key: String,
    pub label: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

/// Usage for one local-time day range.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageSummary {
    pub days: u32,
    pub since_ms: i64,
    pub until_ms: i64,
    pub tz_offset_minutes: i32,
    pub totals: UsageTotals,
    /// Every day in the range, oldest first, including empty days.
    pub by_day: Vec<UsageBucket>,
    pub by_model: Vec<UsageBucket>,
    pub by_provider: Vec<UsageBucket>,
    pub by_project: Vec<UsageBucket>,
}

#[derive(Debug, Default)]
struct UsageRecord {
    session_id: String,
    at_ms: i64,
    model: String,
    provider: Option<String>,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cost_usd: Option<f64>,
    cost_source: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CostKind {
    Reported,
    Estimated,
}

impl UsageTotals {
    fn add(&mut self, record: &UsageRecord) {
        self.responses += 1;
        self.prompt_tokens += record.prompt_tokens;
        self.completion_tokens += record.completion_tokens;
        self.total_tokens += record.total_tokens;
        match record.cost() {
            Some((cost, kind)) => {
                self.cost_usd += cost;
                match kind {
                    CostKind::Reported => self.reported_cost_usd += cost,
                    CostKind::Estimated => self.estimated_cost_usd += cost,
                }
            }
            None if record.total_tokens > 0 => self.unpriced_responses += 1,
            None => {}
        }
    }
}

impl UsageRecord {
    /// Mirrors the desktop's `inferredCostSource`: an explicit source wins,
    /// account runtimes report their own cost, and anything else is an estimate.
    fn cost(&self) -> Option<(f64, CostKind)> {
        let cost = self
            .cost_usd
            .filter(|cost| cost.is_finite() && *cost >= 0.0)?;
        let kind = match self.cost_source.as_deref() {
            Some("provider") => CostKind::Reported,
            Some(_) => CostKind::Estimated,
            None if runtime_provider(&self.model).is_some()
                || self.provider.as_deref().is_some_and(|provider| {
                    let provider = provider.to_ascii_lowercase();
                    ["cli", "codex", "claude"]
                        .iter()
                        .any(|needle| provider.contains(needle))
                }) =>
            {
                CostKind::Reported
            }
            None => CostKind::Estimated,
        };
        Some((cost, kind))
    }

    fn provider_label(&self) -> String {
        self.provider
            .as_deref()
            .map(str::trim)
            .filter(|provider| !provider.is_empty())
            .map(str::to_string)
            .or_else(|| runtime_provider(&self.model).map(str::to_string))
            .unwrap_or_else(|| "Unknown provider".to_string())
    }
}

fn runtime_provider(model: &str) -> Option<&'static str> {
    if model.starts_with("codex:") {
        Some("Codex")
    } else if model.starts_with("claude:") {
        Some("Local Claude CLI")
    } else if model.starts_with("opencode:") {
        Some("Local OpenCode CLI")
    } else if model.starts_with("pi:") {
        Some("Local Pi CLI")
    } else {
        None
    }
}

fn value_f64(row: &Row<'_>, index: usize) -> Option<f64> {
    match row.get_ref(index).ok()? {
        ValueRef::Integer(value) => Some(value as f64),
        ValueRef::Real(value) => Some(value),
        ValueRef::Text(text) => std::str::from_utf8(text).ok()?.trim().parse().ok(),
        _ => None,
    }
}

fn value_tokens(row: &Row<'_>, index: usize) -> u64 {
    value_f64(row, index)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value.round() as u64)
        .unwrap_or(0)
}

fn value_text(row: &Row<'_>, index: usize) -> Option<String> {
    match row.get_ref(index).ok()? {
        ValueRef::Text(text) => std::str::from_utf8(text)
            .ok()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<UsageRecord> {
    let mut record = UsageRecord {
        session_id: row.get(0)?,
        at_ms: value_f64(row, 1).map(|value| value as i64).unwrap_or(0),
        model: value_text(row, 2).unwrap_or_default(),
        provider: value_text(row, 3),
        prompt_tokens: value_tokens(row, 4),
        completion_tokens: value_tokens(row, 5),
        total_tokens: value_tokens(row, 6),
        cost_usd: value_f64(row, 7).or_else(|| value_f64(row, 8)),
        cost_source: value_text(row, 9),
    };
    if record.total_tokens == 0 {
        record.total_tokens = record.prompt_tokens + record.completion_tokens;
    }
    Ok(record)
}

/// Assistant responses whose metrics ended inside `[?1, ?2)`. The first
/// predicate repeats the partial index condition so SQLite can use it.
const RESPONSE_USAGE_SQL: &str = "SELECT m.session_id,
        json_extract(m.message_json, '$.metrics.endedAt'),
        json_extract(m.message_json, '$.metrics.model'),
        json_extract(m.message_json, '$.metrics.provider'),
        json_extract(m.message_json, '$.metrics.usage.prompt_tokens'),
        json_extract(m.message_json, '$.metrics.usage.completion_tokens'),
        json_extract(m.message_json, '$.metrics.usage.total_tokens'),
        json_extract(m.message_json, '$.metrics.costUsd'),
        json_extract(m.message_json, '$.metrics.usage.cost_usd'),
        json_extract(m.message_json, '$.metrics.costSource')
     FROM user_session_messages m
     WHERE json_extract(m.message_json, '$.metrics.endedAt') IS NOT NULL
       AND json_extract(m.message_json, '$.metrics.endedAt') >= ?1
       AND json_extract(m.message_json, '$.metrics.endedAt') < ?2
       AND COALESCE(json_extract(m.message_json, '$.role'), 'assistant') = 'assistant'";

/// Compaction checkpoints whose summary call happened inside `[?1, ?2)`.
const COMPACTION_USAGE_SQL: &str = "SELECT m.session_id,
        json_extract(m.message_json, '$.compaction.createdAt'),
        json_extract(m.message_json, '$.compaction.summary.model'),
        json_extract(m.message_json, '$.compaction.summary.provider'),
        json_extract(m.message_json, '$.compaction.summary.usage.prompt_tokens'),
        json_extract(m.message_json, '$.compaction.summary.usage.completion_tokens'),
        json_extract(m.message_json, '$.compaction.summary.usage.total_tokens'),
        json_extract(m.message_json, '$.compaction.summary.costUsd'),
        json_extract(m.message_json, '$.compaction.summary.usage.cost_usd'),
        json_extract(m.message_json, '$.compaction.summary.costSource')
     FROM user_session_messages m
     WHERE json_extract(m.message_json, '$.compaction.summary') IS NOT NULL
       AND json_extract(m.message_json, '$.compaction.createdAt') >= ?1
       AND json_extract(m.message_json, '$.compaction.createdAt') < ?2";

/// The project a session belongs to, following the desktop's
/// `sessionProjectFolder`: retry and isolated worktrees group under their origin.
const SESSION_PROJECT_SQL: &str = "SELECT id,
        COALESCE(
            NULLIF(json_extract(session_json, '$.retryWorkspace.originalFolder'), ''),
            CASE WHEN json_extract(session_json, '$.threadWorkspace.mode') = 'worktree'
                 THEN NULLIF(json_extract(session_json, '$.threadWorkspace.projectFolder'), '')
            END,
            json_extract(session_json, '$.settings.folder'),
            ''
        )
     FROM user_sessions";

fn folder_label(folder: &str) -> String {
    folder
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(folder)
        .to_string()
}

/// `YYYY-MM-DD` for a day count since 1970-01-01 (proleptic Gregorian).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn sorted_buckets(map: HashMap<String, (String, UsageTotals)>) -> Vec<UsageBucket> {
    let mut buckets: Vec<UsageBucket> = map
        .into_iter()
        .map(|(key, (label, totals))| UsageBucket { key, label, totals })
        .collect();
    buckets.sort_by(|a, b| {
        b.totals
            .cost_usd
            .total_cmp(&a.totals.cost_usd)
            .then_with(|| b.totals.total_tokens.cmp(&a.totals.total_tokens))
            .then_with(|| a.label.cmp(&b.label))
    });
    buckets
}

impl UserDataStore {
    /// Aggregate usage for the last `days` local days ending at `now_ms`.
    /// `tz_offset_minutes` follows JavaScript's `Date#getTimezoneOffset`
    /// (UTC minus local time), so day buckets match the viewer's calendar.
    pub fn usage_summary(
        &self,
        days: u32,
        now_ms: i64,
        tz_offset_minutes: i32,
    ) -> Result<UsageSummary> {
        if days == 0 || days > USAGE_MAX_DAYS {
            return Err(Error::InvalidRequest(format!(
                "usage range must be between 1 and {USAGE_MAX_DAYS} days"
            )));
        }
        if !(-14 * 60..=14 * 60).contains(&tz_offset_minutes) {
            return Err(Error::InvalidRequest("invalid timezone offset".into()));
        }
        let offset_ms = i64::from(tz_offset_minutes) * 60_000;
        let local_today = (now_ms - offset_ms).div_euclid(DAY_MS);
        let first_local_day = local_today - i64::from(days) + 1;
        let since_ms = first_local_day * DAY_MS + offset_ms;
        let until_ms = (local_today + 1) * DAY_MS + offset_ms;

        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();
        let mut records = Vec::new();
        for sql in [RESPONSE_USAGE_SQL, COMPACTION_USAGE_SQL] {
            let mut statement = conn.prepare_cached(sql).map_err(sqlite)?;
            let rows = statement
                .query_map(params![since_ms, until_ms], record_from_row)
                .map_err(sqlite)?;
            for row in rows {
                records.push(row.map_err(sqlite)?);
            }
        }
        let mut projects = HashMap::new();
        if !records.is_empty() {
            let mut statement = conn.prepare_cached(SESSION_PROJECT_SQL).map_err(sqlite)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    ))
                })
                .map_err(sqlite)?;
            for row in rows {
                let (id, folder) = row.map_err(sqlite)?;
                projects.insert(id, folder.trim().to_string());
            }
        }
        drop(db);

        let mut totals = UsageTotals::default();
        let mut by_day: BTreeMap<i64, UsageTotals> = BTreeMap::new();
        let mut by_model = HashMap::new();
        let mut by_provider = HashMap::new();
        let mut by_project = HashMap::new();
        for record in &records {
            totals.add(record);
            let day = (record.at_ms - offset_ms).div_euclid(DAY_MS);
            by_day.entry(day).or_default().add(record);
            let model = if record.model.is_empty() {
                "unknown".to_string()
            } else {
                record.model.clone()
            };
            by_model
                .entry(model.clone())
                .or_insert_with(|| (model, UsageTotals::default()))
                .1
                .add(record);
            let provider = record.provider_label();
            by_provider
                .entry(provider.clone())
                .or_insert_with(|| (provider, UsageTotals::default()))
                .1
                .add(record);
            let folder = projects
                .get(&record.session_id)
                .cloned()
                .unwrap_or_default();
            let label = if folder.is_empty() {
                "No project".to_string()
            } else {
                folder_label(&folder)
            };
            by_project
                .entry(folder)
                .or_insert_with(|| (label, UsageTotals::default()))
                .1
                .add(record);
        }

        Ok(UsageSummary {
            days,
            since_ms,
            until_ms,
            tz_offset_minutes,
            totals,
            by_day: (first_local_day..=local_today)
                .map(|day| {
                    let date = civil_date(day);
                    UsageBucket {
                        key: date.clone(),
                        label: date,
                        totals: by_day.remove(&day).unwrap_or_default(),
                    }
                })
                .collect(),
            by_model: sorted_buckets(by_model),
            by_provider: sorted_buckets(by_provider),
            by_project: sorted_buckets(by_project),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use serde_json::json;

    const NOW: i64 = 1_758_628_800_000; // 2025-09-23T12:00:00Z

    fn store_with_threads() -> UserDataStore {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        let reported = json!({
            "settings": { "folder": "/work/alpha" },
            "messages": [
                { "id": "u1", "role": "user", "content": "hi" },
                {
                    "id": "a1", "role": "assistant", "content": "hello",
                    "metrics": {
                        "startedAt": NOW - 3_000, "endedAt": NOW - 1_000,
                        "model": "provider:openrouter:anthropic/claude", "provider": "OpenRouter",
                        "usage": { "prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150 },
                        "costUsd": 0.25, "costSource": "provider"
                    }
                },
                {
                    "id": "a2", "role": "assistant", "content": "old",
                    "metrics": {
                        "startedAt": NOW - 40 * DAY_MS, "endedAt": NOW - 40 * DAY_MS,
                        "model": "provider:openrouter:anthropic/claude", "provider": "OpenRouter",
                        "usage": { "prompt_tokens": 9, "completion_tokens": 9, "total_tokens": 18 },
                        "costUsd": 9.0, "costSource": "provider"
                    }
                }
            ]
        });
        store
            .control_create_thread("thread-alpha", &reported.to_string(), "epoch")
            .unwrap();
        let worktree = json!({
            "settings": { "folder": "/runtime/threads/thread-beta" },
            "threadWorkspace": { "mode": "worktree", "projectFolder": "/work/beta" },
            "messages": [
                {
                    "id": "b1", "role": "assistant", "content": "estimated",
                    "metrics": {
                        "startedAt": NOW - DAY_MS - 5_000, "endedAt": NOW - DAY_MS,
                        "model": "gpt-local",
                        "usage": { "prompt_tokens": 10, "completion_tokens": 10, "total_tokens": 20 },
                        "costUsd": 0.5
                    }
                },
                {
                    "id": "b2", "role": "assistant", "content": "unpriced",
                    "metrics": {
                        "startedAt": NOW - 2_000, "endedAt": NOW - 1_500,
                        "model": "codex:gpt-5",
                        "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 }
                    }
                },
                {
                    "id": "b3", "role": "assistant", "content": "checkpoint",
                    "compaction": {
                        "kind": "checkpoint", "createdAt": NOW - 500,
                        "sourceTokens": 1, "summaryTokens": 1,
                        "summary": {
                            "model": "gpt-local",
                            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 },
                            "costUsd": 0.1, "costSource": "estimate"
                        }
                    }
                }
            ]
        });
        store
            .control_create_thread("thread-beta", &worktree.to_string(), "epoch")
            .unwrap();
        store
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn usage_summary_groups_tokens_and_cost_provenance() {
        let store = store_with_threads();
        let summary = store.usage_summary(7, NOW + 60_000, 0).unwrap();
        assert_eq!(summary.by_day.len(), 7);
        assert_eq!(summary.by_day.last().unwrap().key, "2025-09-23");
        assert_eq!(summary.by_day[0].key, "2025-09-17");
        assert_eq!(summary.totals.responses, 4);
        assert_eq!(summary.totals.total_tokens, 190);
        assert!(close(summary.totals.cost_usd, 0.85));
        assert!(close(summary.totals.reported_cost_usd, 0.25));
        assert!(close(summary.totals.estimated_cost_usd, 0.6));
        assert_eq!(summary.totals.unpriced_responses, 1);

        let yesterday = &summary.by_day[5];
        assert_eq!(yesterday.key, "2025-09-22");
        assert_eq!(yesterday.totals.total_tokens, 20);
        let today = &summary.by_day[6];
        assert_eq!(today.totals.total_tokens, 170);

        let beta = summary
            .by_project
            .iter()
            .find(|bucket| bucket.key == "/work/beta")
            .expect("isolated worktree usage groups under its project");
        assert_eq!(beta.label, "beta");
        assert_eq!(beta.totals.responses, 3);
        assert_eq!(summary.by_project[0].key, "/work/beta");

        let codex = summary
            .by_provider
            .iter()
            .find(|bucket| bucket.key == "Codex")
            .unwrap();
        assert_eq!(codex.totals.unpriced_responses, 1);
        assert!(summary
            .by_model
            .iter()
            .any(|bucket| bucket.key == "gpt-local" && bucket.totals.responses == 2));

        let month = store.usage_summary(90, NOW + 60_000, 0).unwrap();
        assert_eq!(month.totals.responses, 5);
        assert!(store.usage_summary(0, NOW, 0).is_err());
        assert!(store.usage_summary(400, NOW, 0).is_err());
    }

    #[test]
    fn usage_summary_buckets_days_in_the_viewer_timezone() {
        let store = store_with_threads();
        // UTC+13: noon UTC is 01:00 on the next local day, so today's
        // responses move to the 24th and yesterday's to the 23rd.
        let summary = store.usage_summary(2, NOW + 60_000, -780).unwrap();
        assert_eq!(summary.by_day[0].key, "2025-09-23");
        assert_eq!(summary.by_day[0].totals.total_tokens, 20);
        assert_eq!(summary.by_day[1].key, "2025-09-24");
        assert_eq!(summary.by_day[1].totals.total_tokens, 170);
    }

    #[test]
    fn usage_queries_use_the_partial_metric_indexes() {
        let store = store_with_threads();
        let db = store.read_db().unwrap();
        for (sql, index) in [
            (
                RESPONSE_USAGE_SQL,
                "idx_user_session_messages_metrics_ended",
            ),
            (
                COMPACTION_USAGE_SQL,
                "idx_user_session_messages_compaction_created",
            ),
        ] {
            let plan: Vec<String> = db
                .conn()
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap()
                .query_map(params![0_i64, NOW], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert!(
                plan.iter().any(|detail| detail.contains(index)),
                "expected {index} in {plan:?}"
            );
        }
    }

    #[test]
    fn civil_dates_cover_leap_years() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(19_782), "2024-02-29");
        assert_eq!(civil_date(-1), "1969-12-31");
    }
}
