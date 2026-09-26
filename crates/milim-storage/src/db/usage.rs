//! Usage and cost aggregation over canonical assistant messages.
//!
//! Each completed assistant message stores a `metrics` snapshot (model,
//! provider, token usage, and cost with its provenance), and compaction
//! checkpoints store the metrics of their summary call. Partial expression
//! indexes over those timestamps (user-data migration 11) keep a bounded
//! date-range scan from touching messages without metrics.
//!
//! Harness health aggregates the canonical run ledger (`user_runs`,
//! `user_run_events`, and pending approvals) over runs created in a range.

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

/// Filters for [`UserDataStore::harness_metrics`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HarnessMetricsQuery {
    /// Runs created inside `[since_ms, until_ms)` are included.
    pub since_ms: i64,
    pub until_ms: i64,
    /// Run adapter such as `provider`, `codex`, or `claude`.
    pub runtime: Option<String>,
    /// Frozen run model, with or without its runtime prefix.
    pub model: Option<String>,
}

/// Nearest-rank percentiles over one latency sample set, in milliseconds.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LatencyPercentiles {
    pub samples: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
}

/// Calls and failures for one tool name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolHealth {
    pub name: String,
    pub calls: u64,
    pub errors: u64,
    pub error_rate: f64,
}

/// One run status and how many runs ended in it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusCount {
    pub status: String,
    pub runs: u64,
}

/// Harness health aggregated from the canonical run ledger.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HarnessMetrics {
    pub since_ms: i64,
    pub until_ms: i64,
    pub runtime: Option<String>,
    pub model: Option<String>,
    pub runs: u64,
    pub runs_by_status: Vec<StatusCount>,
    pub model_steps: u64,
    /// Steps measured by a `model_timing` event; the rest fall back to the
    /// request and response event timestamps.
    pub timed_model_steps: u64,
    pub step_latency: LatencyPercentiles,
    /// Only `model_timing` events carry time to first token.
    pub first_token: LatencyPercentiles,
    pub approval_wait: LatencyPercentiles,
    pub pending_approvals: u64,
    pub tool_calls: u64,
    pub tool_errors: u64,
    pub tool_error_rate: f64,
    pub tools: Vec<ToolHealth>,
    pub avg_steps_per_run: Option<f64>,
    /// Average over runs with a recorded cost.
    pub avg_cost_usd_per_run: Option<f64>,
    pub priced_runs: u64,
    pub retries: u64,
    pub runs_with_retries: u64,
    /// Every runtime and model seen in the range, for filter pickers.
    pub available_runtimes: Vec<String>,
    pub available_models: Vec<String>,
}

#[derive(Debug, Default)]
struct RunLedgerMetrics {
    model_timings: Vec<ModelTiming>,
    tool_timings: Vec<(String, bool)>,
    /// `step_id -> (request timestamps, response timestamp)`.
    steps: BTreeMap<String, (Vec<i64>, Option<i64>)>,
    tool_results: Vec<(Option<String>, String)>,
}

#[derive(Debug)]
struct ModelTiming {
    duration_ms: Option<u64>,
    first_token_ms: Option<u64>,
    attempts: u64,
}

const HARNESS_RUNS_SQL: &str = "SELECT id, status, adapter,
        COALESCE(json_extract(request_json, '$.config.model'), '')
     FROM user_runs
     WHERE created_at_ms >= ?1 AND created_at_ms < ?2";

const HARNESS_EVENTS_SQL: &str = "SELECT e.step_id, e.event_type, e.data_json, e.created_at_ms
     FROM user_run_events e
     WHERE e.run_id = ?1
       AND e.event_type IN ('model_timing', 'tool_timing', 'model_request_resolved',
                            'harness_request_committed', 'model_response_committed',
                            'tool_result_committed')
     ORDER BY e.seq";

const HARNESS_APPROVALS_SQL: &str = "SELECT created_at_ms, resolved_at_ms
     FROM user_pending_approvals WHERE run_id = ?1";

/// Tool results projected into the transcript keep their visible result JSON,
/// so a failed call is recognizable without decoding ledger artifacts. One
/// bounded scan serves every run in the range.
const HARNESS_TOOL_ERRORS_SQL: &str = "SELECT run_id, json_extract(data_json, '$.call_id'),
        COALESCE(json_extract(data_json, '$.name'), '')
     FROM user_timeline_events
     WHERE item_type = 'tool_result' AND run_id IS NOT NULL AND created_at_ms >= ?1
       AND json_type(data_json, '$.result.error') IS NOT NULL";

/// Run costs come from the completed assistant message metrics, which carry
/// provider-reported or estimated cost. `?2` adds slack for runs that were
/// created inside the range and finished after it.
const HARNESS_RUN_COST_SQL: &str = "SELECT json_extract(m.message_json, '$.runId'),
        json_extract(m.message_json, '$.metrics.costUsd'),
        json_extract(m.message_json, '$.metrics.usage.cost_usd')
     FROM user_session_messages m
     WHERE json_extract(m.message_json, '$.metrics.endedAt') IS NOT NULL
       AND json_extract(m.message_json, '$.metrics.endedAt') >= ?1
       AND json_extract(m.message_json, '$.metrics.endedAt') < ?2
       AND json_extract(m.message_json, '$.runId') IS NOT NULL";

fn percentiles(mut samples: Vec<u64>) -> LatencyPercentiles {
    samples.sort_unstable();
    let rank = |percent: usize| -> Option<u64> {
        if samples.is_empty() {
            return None;
        }
        let index = (samples.len() * percent).div_ceil(100).max(1) - 1;
        samples.get(index).copied()
    };
    LatencyPercentiles {
        samples: samples.len() as u64,
        p50_ms: rank(50),
        p95_ms: rank(95),
    }
}

fn json_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    let value = value.get(key)?;
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| v.round() as u64)
    })
}

fn model_timing(data: &serde_json::Value) -> ModelTiming {
    let started_at = json_u64(data, "started_at_ms");
    // `first_token_ms` is a duration from the step start; an absolute
    // timestamp from an older writer is converted against `started_at_ms`.
    let first_token_ms = json_u64(data, "first_token_ms").map(|first| match started_at {
        Some(start) if start > 0 && first >= start => first - start,
        _ => first,
    });
    ModelTiming {
        duration_ms: json_u64(data, "duration_ms"),
        first_token_ms,
        attempts: json_u64(data, "attempts").unwrap_or(1).max(1),
    }
}

fn model_matches(filter: &str, adapter: &str, model: &str) -> bool {
    let filter = filter.trim();
    filter.eq_ignore_ascii_case(model)
        || filter
            .split_once(':')
            .is_some_and(|(prefix, rest)| prefix.eq_ignore_ascii_case(adapter) && rest == model)
}

impl UserDataStore {
    /// Aggregate run-ledger health over runs created inside the query range.
    /// `model_timing` and `tool_timing` events are authoritative for the runs
    /// that recorded them; older runs fall back to event timestamps and the
    /// projected tool results.
    pub fn harness_metrics(&self, query: &HarnessMetricsQuery) -> Result<HarnessMetrics> {
        if query.until_ms <= query.since_ms {
            return Err(Error::InvalidRequest(
                "metrics range must end after it starts".into(),
            ));
        }
        let runtime = query
            .runtime
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let model = query
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let db = self
            .read_db()
            .map_err(|_| Error::Other("user data DB lock poisoned".into()))?;
        let conn = db.conn();

        let mut runtimes = BTreeMap::<String, ()>::new();
        let mut models = BTreeMap::<String, ()>::new();
        let mut runs = Vec::new();
        {
            let mut statement = conn.prepare_cached(HARNESS_RUNS_SQL).map_err(sqlite)?;
            let rows = statement
                .query_map(params![query.since_ms, query.until_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sqlite)?;
            for row in rows {
                let (id, status, adapter, run_model) = row.map_err(sqlite)?;
                runtimes.insert(adapter.clone(), ());
                if !run_model.is_empty() {
                    models.insert(run_model.clone(), ());
                }
                if runtime.is_some_and(|runtime| !runtime.eq_ignore_ascii_case(&adapter))
                    || model.is_some_and(|model| !model_matches(model, &adapter, &run_model))
                {
                    continue;
                }
                runs.push((id, status));
            }
        }

        let mut metrics = HarnessMetrics {
            since_ms: query.since_ms,
            until_ms: query.until_ms,
            runtime: runtime.map(str::to_string),
            model: model.map(str::to_string),
            runs: runs.len() as u64,
            available_runtimes: runtimes.into_keys().collect(),
            available_models: models.into_keys().collect(),
            ..HarnessMetrics::default()
        };
        let mut statuses = BTreeMap::<String, u64>::new();
        let mut step_latency = Vec::new();
        let mut first_token = Vec::new();
        let mut approval_wait = Vec::new();
        let mut tools = BTreeMap::<String, (u64, u64)>::new();
        let mut steps_per_run = Vec::new();
        let mut run_ids = HashMap::new();
        let mut failed_tools = HashMap::<String, Vec<(Option<String>, String)>>::new();
        if !runs.is_empty() {
            let mut statement = conn
                .prepare_cached(HARNESS_TOOL_ERRORS_SQL)
                .map_err(sqlite)?;
            let rows = statement
                .query_map(params![query.since_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(sqlite)?;
            for row in rows {
                let (run_id, call_id, name) = row.map_err(sqlite)?;
                failed_tools
                    .entry(run_id)
                    .or_default()
                    .push((call_id, name));
            }
        }
        let mut events = conn.prepare_cached(HARNESS_EVENTS_SQL).map_err(sqlite)?;
        let mut approvals = conn.prepare_cached(HARNESS_APPROVALS_SQL).map_err(sqlite)?;
        for (run_id, status) in &runs {
            *statuses.entry(status.clone()).or_default() += 1;
            run_ids.insert(run_id.clone(), ());

            let mut ledger = RunLedgerMetrics::default();
            let rows = events
                .query_map(params![run_id], |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })
                .map_err(sqlite)?;
            for row in rows {
                let (step_id, event_type, data_json, created_at_ms) = row.map_err(sqlite)?;
                let data: serde_json::Value =
                    serde_json::from_str(&data_json).unwrap_or(serde_json::Value::Null);
                let step_key = step_id.unwrap_or_default();
                match event_type.as_str() {
                    "model_timing" => ledger.model_timings.push(model_timing(&data)),
                    "tool_timing" => ledger.tool_timings.push((
                        data.get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        data.get("is_error")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    )),
                    "model_request_resolved" | "harness_request_committed" => ledger
                        .steps
                        .entry(step_key)
                        .or_default()
                        .0
                        .push(created_at_ms),
                    "model_response_committed" => {
                        ledger.steps.entry(step_key).or_default().1 = Some(created_at_ms);
                    }
                    "tool_result_committed" => ledger.tool_results.push((
                        data.get("call_id")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        data.get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    )),
                    _ => {}
                }
            }

            let mut run_retries = 0;
            if ledger.model_timings.is_empty() {
                let mut steps = 0;
                for (requests, response) in ledger.steps.values() {
                    let Some(first_request) = requests.first() else {
                        continue;
                    };
                    steps += 1;
                    run_retries += requests.len() as u64 - 1;
                    if let Some(response) = response {
                        step_latency.push(u64::try_from(response - first_request).unwrap_or(0));
                    }
                }
                steps_per_run.push(steps);
            } else {
                metrics.timed_model_steps += ledger.model_timings.len() as u64;
                for timing in &ledger.model_timings {
                    run_retries += timing.attempts - 1;
                    step_latency.extend(timing.duration_ms);
                    first_token.extend(timing.first_token_ms);
                }
                steps_per_run.push(ledger.model_timings.len() as u64);
            }
            metrics.retries += run_retries;
            if run_retries > 0 {
                metrics.runs_with_retries += 1;
            }

            if ledger.tool_timings.is_empty() {
                let mut unmatched = failed_tools.remove(run_id).unwrap_or_default();
                for (call_id, name) in &ledger.tool_results {
                    let position = unmatched.iter().position(|(failed_id, failed_name)| {
                        match (call_id, failed_id) {
                            (Some(call_id), Some(failed_id)) => call_id == failed_id,
                            _ => failed_name == name,
                        }
                    });
                    let is_error = position.map(|index| unmatched.remove(index)).is_some();
                    let entry = tools.entry(name.clone()).or_default();
                    entry.0 += 1;
                    entry.1 += u64::from(is_error);
                }
            } else {
                for (name, is_error) in &ledger.tool_timings {
                    let entry = tools.entry(name.clone()).or_default();
                    entry.0 += 1;
                    entry.1 += u64::from(*is_error);
                }
            }

            let rows = approvals
                .query_map(params![run_id], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
                })
                .map_err(sqlite)?;
            for row in rows {
                match row.map_err(sqlite)? {
                    (created, Some(resolved)) => {
                        approval_wait.push(u64::try_from(resolved - created).unwrap_or(0));
                    }
                    (_, None) => metrics.pending_approvals += 1,
                }
            }
        }
        drop(events);
        drop(approvals);

        let mut run_costs = HashMap::<String, f64>::new();
        if !run_ids.is_empty() {
            let mut statement = conn.prepare_cached(HARNESS_RUN_COST_SQL).map_err(sqlite)?;
            let rows = statement
                .query_map(
                    params![query.since_ms, query.until_ms.saturating_add(DAY_MS)],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            value_f64(row, 1).or_else(|| value_f64(row, 2)),
                        ))
                    },
                )
                .map_err(sqlite)?;
            for row in rows {
                let (run_id, cost) = row.map_err(sqlite)?;
                if let Some(cost) = cost.filter(|cost| cost.is_finite() && *cost >= 0.0) {
                    if run_ids.contains_key(&run_id) {
                        *run_costs.entry(run_id).or_default() += cost;
                    }
                }
            }
        }
        drop(db);

        metrics.runs_by_status = statuses
            .into_iter()
            .map(|(status, runs)| StatusCount { status, runs })
            .collect();
        metrics.model_steps = steps_per_run.iter().sum();
        metrics.avg_steps_per_run = (!steps_per_run.is_empty())
            .then(|| metrics.model_steps as f64 / steps_per_run.len() as f64);
        metrics.priced_runs = run_costs.len() as u64;
        metrics.avg_cost_usd_per_run = (!run_costs.is_empty())
            .then(|| run_costs.values().sum::<f64>() / run_costs.len() as f64);
        metrics.step_latency = percentiles(step_latency);
        metrics.first_token = percentiles(first_token);
        metrics.approval_wait = percentiles(approval_wait);
        let mut tools = tools
            .into_iter()
            .map(|(name, (calls, errors))| ToolHealth {
                name,
                calls,
                errors,
                error_rate: if calls == 0 {
                    0.0
                } else {
                    errors as f64 / calls as f64
                },
            })
            .collect::<Vec<_>>();
        tools.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
        metrics.tool_calls = tools.iter().map(|tool| tool.calls).sum();
        metrics.tool_errors = tools.iter().map(|tool| tool.errors).sum();
        metrics.tool_error_rate = if metrics.tool_calls == 0 {
            0.0
        } else {
            metrics.tool_errors as f64 / metrics.tool_calls as f64
        };
        metrics.tools = tools;
        Ok(metrics)
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

    fn put_run(store: &UserDataStore, id: &str, status: &str, adapter: &str, model: &str) {
        store
            .control_put_run(&crate::ControlRunRecord {
                id: id.into(),
                thread_id: "thread-harness".into(),
                status: status.into(),
                adapter: adapter.into(),
                request_json: json!({ "config": { "model": model, "adapter": adapter } })
                    .to_string(),
                agent_snapshot_json: None,
                native_session_json: None,
                created_at_ms: NOW - 60_000,
                updated_at_ms: NOW,
                completed_at_ms: Some(NOW),
                error_json: None,
            })
            .unwrap();
    }

    fn put_event(
        store: &UserDataStore,
        run_id: &str,
        step: Option<&str>,
        event_type: &str,
        data: serde_json::Value,
        at_ms: i64,
    ) {
        let db = store.db.lock().unwrap();
        db.conn()
            .execute(
                "INSERT INTO user_run_events
                 (run_id, seq, event_id, step_id, event_type, data_json, created_at_ms)
                 VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM user_run_events
                              WHERE run_id = ?1), ?2, ?3, ?4, ?5, ?6)",
                params![
                    run_id,
                    uuid_like(run_id, event_type, at_ms),
                    step,
                    event_type,
                    data.to_string(),
                    at_ms
                ],
            )
            .unwrap();
    }

    fn uuid_like(run_id: &str, event_type: &str, at_ms: i64) -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{run_id}-{event_type}-{at_ms}-{n}")
    }

    fn harness_store() -> UserDataStore {
        let store = UserDataStore::new(Database::open_in_memory().unwrap()).unwrap();
        let thread = json!({
            "messages": [
                { "id": "m1", "role": "assistant", "runId": "run-timed",
                  "metrics": { "endedAt": NOW - 1_000, "costUsd": 0.3 } },
                { "id": "m2", "role": "assistant", "runId": "run-legacy",
                  "metrics": { "endedAt": NOW - 1_000, "usage": { "cost_usd": 0.1 } } }
            ]
        });
        store
            .control_create_thread("thread-harness", &thread.to_string(), "epoch")
            .unwrap();

        // New-format run: timing events are authoritative.
        put_run(&store, "run-timed", "completed", "provider", "gpt-fixture");
        for (step, duration, first_token, attempts) in
            [(1, 1_000, Some(200), 1), (2, 3_000, None, 3)]
        {
            put_event(
                &store,
                "run-timed",
                Some(&format!("step-{step}")),
                "model_timing",
                json!({
                    "step": step, "started_at_ms": NOW - 50_000,
                    "first_token_ms": first_token, "duration_ms": duration,
                    "attempts": attempts, "finish_reason": "stop"
                }),
                NOW - 40_000,
            );
        }
        // Request/response events are ignored for latency once timing exists.
        put_event(
            &store,
            "run-timed",
            Some("step-1"),
            "model_request_resolved",
            json!({}),
            NOW - 50_000,
        );
        put_event(
            &store,
            "run-timed",
            Some("step-1"),
            "model_response_committed",
            json!({}),
            NOW - 10_000,
        );
        for (name, is_error) in [("read_file", false), ("shell", true), ("shell", false)] {
            put_event(
                &store,
                "run-timed",
                Some("step-1"),
                "tool_timing",
                json!({ "step": 1, "call_id": null, "name": name, "duration_ms": 5, "is_error": is_error }),
                NOW - 30_000,
            );
        }

        // Legacy run: latency from event timestamps, errors from the transcript.
        put_run(&store, "run-legacy", "failed", "provider", "gpt-fixture");
        put_event(
            &store,
            "run-legacy",
            Some("step-1"),
            "model_request_resolved",
            json!({}),
            NOW - 50_000,
        );
        put_event(
            &store,
            "run-legacy",
            Some("step-1"),
            "model_request_resolved",
            json!({}),
            NOW - 49_000,
        );
        put_event(
            &store,
            "run-legacy",
            Some("step-1"),
            "model_response_committed",
            json!({}),
            NOW - 48_000,
        );
        put_event(
            &store,
            "run-legacy",
            Some("step-2"),
            "model_request_resolved",
            json!({}),
            NOW - 47_000,
        );
        put_event(
            &store,
            "run-legacy",
            Some("step-2"),
            "model_response_committed",
            json!({}),
            NOW - 43_000,
        );
        for call_id in ["call-1", "call-2"] {
            put_event(
                &store,
                "run-legacy",
                Some("step-1"),
                "tool_result_committed",
                json!({ "call_id": call_id, "name": "shell", "artifact_digest": "sha256:x" }),
                NOW - 48_500,
            );
        }
        store
            .control_append_timeline(
                "thread-harness",
                "tool-result-1",
                Some("run-legacy"),
                "tool_result",
                &json!({ "type": "tool_result", "call_id": "call-2", "name": "shell",
                         "result": { "error": "exit 1" } })
                .to_string(),
            )
            .unwrap();
        store
            .control_put_approval(&crate::ControlApprovalRecord {
                id: "approval-1".into(),
                run_id: "run-legacy".into(),
                thread_id: "thread-harness".into(),
                kind: "command".into(),
                request_json: "{}".into(),
                status: "approved".into(),
                decision_json: None,
                created_at_ms: NOW - 48_400,
                resolved_at_ms: Some(NOW - 46_400),
            })
            .unwrap();
        store
            .control_put_approval(&crate::ControlApprovalRecord {
                id: "approval-2".into(),
                run_id: "run-legacy".into(),
                thread_id: "thread-harness".into(),
                kind: "command".into(),
                request_json: "{}".into(),
                status: "pending".into(),
                decision_json: None,
                created_at_ms: NOW - 40_000,
                resolved_at_ms: None,
            })
            .unwrap();

        put_run(&store, "run-codex", "completed", "codex", "gpt-5");
        put_event(
            &store,
            "run-codex",
            Some("step-1"),
            "harness_request_committed",
            json!({}),
            NOW - 30_000,
        );
        put_event(
            &store,
            "run-codex",
            Some("step-1"),
            "model_response_committed",
            json!({}),
            NOW - 20_000,
        );
        store
    }

    fn range() -> HarnessMetricsQuery {
        HarnessMetricsQuery {
            since_ms: NOW - DAY_MS,
            until_ms: NOW,
            ..HarnessMetricsQuery::default()
        }
    }

    #[test]
    fn harness_metrics_prefer_timing_events_and_fall_back_to_timestamps() {
        let store = harness_store();
        let metrics = store.harness_metrics(&range()).unwrap();
        assert_eq!(metrics.runs, 3);
        assert_eq!(
            metrics.runs_by_status,
            vec![
                StatusCount {
                    status: "completed".into(),
                    runs: 2
                },
                StatusCount {
                    status: "failed".into(),
                    runs: 1
                },
            ]
        );
        // Timed: 1000, 3000. Legacy: 2000 (first request), 4000. Codex: 10000.
        assert_eq!(metrics.model_steps, 5);
        assert_eq!(metrics.timed_model_steps, 2);
        assert_eq!(metrics.step_latency.samples, 5);
        assert_eq!(metrics.step_latency.p50_ms, Some(3_000));
        assert_eq!(metrics.step_latency.p95_ms, Some(10_000));
        assert_eq!(metrics.first_token.samples, 1);
        assert_eq!(metrics.first_token.p50_ms, Some(200));
        // Two extra timed attempts plus one repeated legacy request.
        assert_eq!(metrics.retries, 3);
        assert_eq!(metrics.runs_with_retries, 2);
        assert!(close(metrics.avg_steps_per_run.unwrap(), 5.0 / 3.0));

        assert_eq!(metrics.tool_calls, 5);
        assert_eq!(metrics.tool_errors, 2);
        assert!(close(metrics.tool_error_rate, 0.4));
        let shell = metrics
            .tools
            .iter()
            .find(|tool| tool.name == "shell")
            .unwrap();
        assert_eq!((shell.calls, shell.errors), (4, 2));
        assert_eq!(metrics.tools[0].name, "shell");

        assert_eq!(metrics.approval_wait.samples, 1);
        assert_eq!(metrics.approval_wait.p50_ms, Some(2_000));
        assert_eq!(metrics.pending_approvals, 1);
        assert_eq!(metrics.priced_runs, 2);
        assert!(close(metrics.avg_cost_usd_per_run.unwrap(), 0.2));
        assert_eq!(metrics.available_runtimes, vec!["codex", "provider"]);
        assert_eq!(metrics.available_models, vec!["gpt-5", "gpt-fixture"]);
    }

    #[test]
    fn harness_metrics_filter_by_runtime_model_and_range() {
        let store = harness_store();
        let codex = store
            .harness_metrics(&HarnessMetricsQuery {
                runtime: Some("codex".into()),
                ..range()
            })
            .unwrap();
        assert_eq!(codex.runs, 1);
        assert_eq!(codex.step_latency.p50_ms, Some(10_000));
        assert_eq!(codex.tool_calls, 0);
        assert_eq!(codex.available_runtimes.len(), 2);

        let prefixed = store
            .harness_metrics(&HarnessMetricsQuery {
                model: Some("codex:gpt-5".into()),
                ..range()
            })
            .unwrap();
        assert_eq!(prefixed.runs, 1);
        let provider = store
            .harness_metrics(&HarnessMetricsQuery {
                model: Some("gpt-fixture".into()),
                ..range()
            })
            .unwrap();
        assert_eq!(provider.runs, 2);

        let empty = store
            .harness_metrics(&HarnessMetricsQuery {
                since_ms: NOW,
                until_ms: NOW + DAY_MS,
                ..HarnessMetricsQuery::default()
            })
            .unwrap();
        assert_eq!(empty.runs, 0);
        assert_eq!(empty.step_latency.p50_ms, None);
        assert_eq!(empty.avg_steps_per_run, None);
        assert!(store
            .harness_metrics(&HarnessMetricsQuery {
                since_ms: NOW,
                until_ms: NOW,
                ..HarnessMetricsQuery::default()
            })
            .is_err());
    }

    #[test]
    fn first_token_accepts_relative_and_absolute_timestamps() {
        let relative = model_timing(&json!({ "started_at_ms": 1_000_000, "first_token_ms": 250 }));
        assert_eq!(relative.first_token_ms, Some(250));
        let absolute =
            model_timing(&json!({ "started_at_ms": 1_000_000, "first_token_ms": 1_000_400 }));
        assert_eq!(absolute.first_token_ms, Some(400));
        assert_eq!(absolute.attempts, 1);
        assert_eq!(percentiles(vec![5, 1, 3, 2, 4]).p50_ms, Some(3));
        assert_eq!(percentiles((1..=20).collect()).p95_ms, Some(19));
    }

    #[test]
    fn civil_dates_cover_leap_years() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(19_782), "2024-02-29");
        assert_eq!(civil_date(-1), "1969-12-31");
    }
}
