import { useEffect, useState } from "react";
import { getUsageSummary, type UsageBucket, type UsageSummary } from "../api";
import {
  formatUsageCost,
  formatUsageCount,
  usageCostSource,
  usageModelLabel,
} from "../lib/usageMetrics";
import { SheetDialog } from "./SheetDialog";
import { Refresh, X } from "./icons";
import "./UsageManager.css";

const RANGE_OPTIONS = [7, 30, 90] as const;
type UsageRange = (typeof RANGE_OPTIONS)[number];
type UsageMetric = "tokens" | "cost";

function metricValue(bucket: UsageBucket, metric: UsageMetric): number {
  return metric === "tokens" ? bucket.total_tokens : bucket.cost_usd;
}

function dayLabel(key: string, options: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" }): string {
  const [year, month, day] = key.split("-").map(Number);
  if (!year || !month || !day) return key;
  return new Date(year, month - 1, day).toLocaleDateString([], options);
}

function formatDollars(value: number): string {
  return value > 0 && value < 0.01 ? "<$0.01" : `$${value.toFixed(2)}`;
}

function bucketSummary(bucket: UsageBucket): string {
  return [
    `${formatUsageCount(bucket.total_tokens)} tokens`,
    formatUsageCost(bucket),
    `${bucket.responses} response${bucket.responses === 1 ? "" : "s"}`,
  ].join(" · ");
}

function DailyChart({
  days,
  metric,
}: {
  days: UsageBucket[];
  metric: UsageMetric;
}) {
  const [hovered, setHovered] = useState<number | null>(null);
  const max = Math.max(0, ...days.map((day) => metricValue(day, metric)));
  const active = hovered == null ? null : days[hovered];
  const activeDays = days.filter((day) => day.responses > 0).length;
  return (
    <div className="usage-daily">
      <div
        className="usage-daily-bars"
        role="img"
        aria-label={`Daily ${metric === "tokens" ? "tokens" : "spend"} for the last ${days.length} days, ${activeDays} with usage`}
        style={{ gap: days.length > 45 ? 1 : 2 }}
        onMouseLeave={() => setHovered(null)}
      >
        {days.map((day, index) => {
          const value = metricValue(day, metric);
          const height = max > 0 ? Math.max(value > 0 ? 3 : 0, (value / max) * 100) : 0;
          return (
            <div
              key={day.key}
              className={`usage-daily-slot${hovered === index ? " active" : ""}`}
              title={`${dayLabel(day.key)}: ${bucketSummary(day)}`}
              onMouseEnter={() => setHovered(index)}
            >
              <span
                className="usage-daily-bar"
                data-empty={value > 0 ? undefined : "true"}
                style={{ height: `${height}%` }}
              />
            </div>
          );
        })}
      </div>
      <div className="usage-daily-axis" aria-hidden="true">
        <span>{days[0] ? dayLabel(days[0].key) : ""}</span>
        <span className="usage-daily-readout">
          {active ? `${dayLabel(active.key, { weekday: "short", month: "short", day: "numeric" })} · ${bucketSummary(active)}` : ""}
        </span>
        <span>{days.length ? dayLabel(days[days.length - 1].key) : ""}</span>
      </div>
    </div>
  );
}

function BreakdownTable({
  title,
  buckets,
  metric,
  label = (bucket) => bucket.label,
}: {
  title: string;
  buckets: UsageBucket[];
  metric: UsageMetric;
  label?: (bucket: UsageBucket) => string;
}) {
  const max = Math.max(0, ...buckets.map((bucket) => metricValue(bucket, metric)));
  return (
    <section className="usage-breakdown" aria-label={title}>
      <h3>{title}</h3>
      {buckets.length ? (
        <table>
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Tokens</th>
              <th scope="col">Spend</th>
            </tr>
          </thead>
          <tbody>
            {buckets.map((bucket) => {
              const share = max > 0 ? (metricValue(bucket, metric) / max) * 100 : 0;
              const name = label(bucket);
              return (
                <tr key={bucket.key} title={`${bucket.key || name}\n${bucketSummary(bucket)}`}>
                  <th scope="row">
                    <div className="usage-breakdown-cell">
                      <span className="usage-breakdown-name">{name}</span>
                      <span className="usage-breakdown-meter" aria-hidden="true">
                        <span style={{ width: `${share}%` }} />
                      </span>
                    </div>
                  </th>
                  <td>{formatUsageCount(bucket.total_tokens)}</td>
                  <td>{formatUsageCost(bucket)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      ) : (
        <p className="usage-empty-note">No usage in this range.</p>
      )}
    </section>
  );
}

export function UsageManager({ onClose }: { onClose: () => void }) {
  const [range, setRange] = useState<UsageRange>(30);
  const [metric, setMetric] = useState<UsageMetric>("tokens");
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    getUsageSummary(range)
      .then((next) => {
        if (!cancelled) setSummary(next);
      })
      .catch((reason: unknown) => {
        if (!cancelled)
          setError(reason instanceof Error ? reason.message : "Usage is unavailable.");
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [range, reloadKey]);

  const totals = summary?.totals;
  const costSource = totals ? usageCostSource(totals) : undefined;
  const activeDays = summary?.by_day.filter((day) => day.responses > 0).length ?? 0;

  return (
    <SheetDialog title="Usage" className="sheet usage-sheet" testId="usage-manager" onClose={onClose}>
      <div className="usage-header">
        <div className="usage-title">
          <h2>Usage</h2>
          <p>Tokens and spend recorded on assistant responses and compaction summaries, grouped by your local calendar.</p>
        </div>
        <div className="usage-header-actions">
          <div className="usage-segmented" role="group" aria-label="Date range">
            {RANGE_OPTIONS.map((option) => (
              <button
                key={option}
                type="button"
                aria-pressed={range === option}
                onClick={() => setRange(option)}
              >
                {option} days
              </button>
            ))}
          </div>
          <button
            className="icon-btn usage-icon-action"
            type="button"
            title="Refresh usage"
            aria-label="Refresh usage"
            disabled={loading}
            onClick={() => setReloadKey((value) => value + 1)}
          >
            <Refresh size={14} />
          </button>
          <button className="icon-btn sheet-close usage-icon-action" type="button" onClick={onClose} title="Close" aria-label="Close usage">
            <X size={15} />
          </button>
        </div>
      </div>

      <div className="usage-body" aria-busy={loading}>
        {error && (
          <p className="usage-note error" role="alert">
            Usage unavailable: {error}
          </p>
        )}
        {!summary && !error && <p className="usage-note">Loading usage...</p>}
        {summary && totals && (
          <>
            <div className="usage-stats" aria-label="Totals">
              <div className="usage-stat">
                <span className="usage-stat-label">Tokens</span>
                <strong>{formatUsageCount(totals.total_tokens)}</strong>
                <small>
                  {formatUsageCount(totals.prompt_tokens)} in · {formatUsageCount(totals.completion_tokens)} out
                </small>
              </div>
              <div className="usage-stat">
                <span className="usage-stat-label">Spend</span>
                <strong>{formatUsageCost(totals)}</strong>
                <small>
                  {costSource === "mixed"
                    ? "Reported and estimated"
                    : costSource === "estimate"
                      ? "Estimated"
                      : costSource === "provider"
                        ? "Reported"
                        : "No recorded cost"}
                </small>
              </div>
              <div className="usage-stat">
                <span className="usage-stat-label">Responses</span>
                <strong>{formatUsageCount(totals.responses)}</strong>
                <small>
                  {activeDays} active day{activeDays === 1 ? "" : "s"}
                </small>
              </div>
              <div className="usage-stat">
                <span className="usage-stat-label">Daily average</span>
                <strong>{formatUsageCount(totals.total_tokens / summary.days)}</strong>
                <small>tokens per day</small>
              </div>
            </div>

            <section className="usage-panel" aria-label="Daily usage">
              <div className="usage-panel-head">
                <h3>By day</h3>
                <div className="usage-segmented" role="group" aria-label="Chart metric">
                  <button type="button" aria-pressed={metric === "tokens"} onClick={() => setMetric("tokens")}>
                    Tokens
                  </button>
                  <button type="button" aria-pressed={metric === "cost"} onClick={() => setMetric("cost")}>
                    Spend
                  </button>
                </div>
              </div>
              <DailyChart days={summary.by_day} metric={metric} />
            </section>

            <div className="usage-breakdowns">
              <BreakdownTable
                title="By model"
                buckets={summary.by_model}
                metric={metric}
                label={(bucket) => usageModelLabel(bucket.key)}
              />
              <BreakdownTable title="By provider or runtime" buckets={summary.by_provider} metric={metric} />
              <BreakdownTable title="By project" buckets={summary.by_project} metric={metric} />
            </div>

            <div className="usage-provenance" role="note">
              <strong>Cost provenance</strong>
              <p>
                Reported costs come from the provider or account runtime that ran the turn
                ({formatDollars(totals.reported_cost_usd)}).
                Costs marked <code>est.</code> are estimated from cached per-token pricing
                ({formatDollars(totals.estimated_cost_usd)}) and can differ from your bill.
                {totals.unpriced_responses > 0
                  ? ` ${totals.unpriced_responses} response${totals.unpriced_responses === 1 ? " has" : "s have"} tokens but no recorded cost, so totals marked ~ are incomplete.`
                  : ""}
              </p>
              <p>
                Archived chats are included. Deleted chats and turns without recorded metrics are not.
              </p>
            </div>
          </>
        )}
      </div>
    </SheetDialog>
  );
}
