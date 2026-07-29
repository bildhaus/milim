import type {
  ChatMessage,
  ProviderInfo,
  ProviderLimitInfo,
  ResponseMetrics,
  RunTrace,
  TokenUsage,
} from "../api";
import { providerOwnsModel, rawModelId } from "./modelPicker.js";

const USAGE_MONTH_COUNT = 12;
const MONTH_LABELS = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
];

export interface ThreadMetricsSummary {
  responseCount: number;
  durationMs: number;
  usage: TokenUsage;
  costUsd?: number;
}

export interface ThreadMetricsBreakdown {
  lifetime: ThreadMetricsSummary;
  checkpoint?: ChatMessage["compaction"];
  sinceCheckpoint?: ThreadMetricsSummary;
}

export interface FormattedThreadMetrics {
  label: string | null;
  title: string | null;
}

export interface MilimUsageSession {
  messages: ChatMessage[];
  settings?: { folder?: string };
  updatedAt: number;
  archivedAt?: number;
}

export function approvalWaitDuration(
  run: RunTrace | null | undefined,
  endedAt = Date.now(),
): number {
  const intervals = (run?.steps ?? [])
    .flatMap((step) => step.approval
      ? [[
          step.approval.requestedAt,
          Math.min(step.approval.resolvedAt ?? endedAt, endedAt),
        ] as const]
      : [])
    .filter(([start, end]) => end > start)
    .sort(([left], [right]) => left - right);
  let total = 0;
  let intervalStart = 0;
  let intervalEnd = 0;
  for (const [start, end] of intervals) {
    if (start > intervalEnd) {
      total += intervalEnd - intervalStart;
      intervalStart = start;
      intervalEnd = end;
    } else {
      intervalEnd = Math.max(intervalEnd, end);
    }
  }
  return total + intervalEnd - intervalStart;
}

export interface MilimUsageProject {
  folder: string;
  archivedAt?: number;
}

export interface MilimUsageMonth {
  key: string;
  label: string;
  days: number[];
}

export interface MilimUsageMetric {
  label: string;
  value: string;
}

export interface MilimUsageSummary {
  months: MilimUsageMonth[];
  metrics: MilimUsageMetric[];
  threadCount: number;
  projectCount: number;
  activeDayCount: number;
  hasUsage: boolean;
}

export function providerNameForModel(
  model: string,
  providers: ProviderInfo[],
): string | undefined {
  return providerForModel(model, providers)?.name;
}

export function estimateResponseCostUsd(
  model: string,
  usage: TokenUsage | undefined,
  providers: ProviderInfo[],
): number | undefined {
  if (!usage) return undefined;
  const pricing = providerForModel(model, providers)?.pricing?.[rawModelId(model)];
  const promptRate = usdPerToken(pricing?.prompt);
  const completionRate = usdPerToken(pricing?.completion);
  if (promptRate == null || completionRate == null) return undefined;
  const cost =
    usage.prompt_tokens * promptRate + usage.completion_tokens * completionRate;
  return Number.isFinite(cost) && cost > 0 ? cost : undefined;
}

export function responseMetricsForTurn({
  startedAt,
  endedAt,
  pausedMs = 0,
  model,
  providers,
  codexModel,
  claudeModel,
  usage,
  costUsd,
  limits,
}: {
  startedAt: number;
  endedAt: number;
  pausedMs?: number;
  model: string;
  providers: ProviderInfo[];
  codexModel?: string | null;
  claudeModel?: string | null;
  usage?: TokenUsage;
  costUsd?: number;
  limits?: ProviderLimitInfo[];
}): ResponseMetrics {
  const runtimeProvider = codexModel || model.startsWith("codex:")
    ? "Codex"
    : claudeModel || model.startsWith("claude:")
      ? "Local Claude CLI"
      : model.startsWith("opencode:")
        ? "Local OpenCode CLI"
        : model.startsWith("pi:")
          ? "Local Pi CLI"
          : undefined;
  return {
    startedAt,
    endedAt,
    durationMs: Math.max(0, endedAt - startedAt - pausedMs),
    model,
    provider: runtimeProvider ?? providerNameForModel(model, providers),
    usage,
    costUsd:
      costUsd ??
      (runtimeProvider
        ? undefined
        : estimateResponseCostUsd(model, usage, providers)),
    limits: limits?.length ? limits : undefined,
  };
}

export function summarizeResponseMetrics(
  messages: ChatMessage[],
): ThreadMetricsSummary {
  const summary: ThreadMetricsSummary = {
    responseCount: 0,
    durationMs: 0,
    usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  };
  let costUsd = 0;
  let hasCost = false;

  for (const message of messages) {
    const metrics = message.role === "assistant" ? message.metrics : undefined;
    if (!metrics) continue;
    summary.responseCount += 1;
    summary.durationMs += metrics.durationMs ?? 0;
    if (metrics.usage) {
      summary.usage.prompt_tokens += metrics.usage.prompt_tokens;
      summary.usage.completion_tokens += metrics.usage.completion_tokens;
      summary.usage.total_tokens += metrics.usage.total_tokens;
    }
    if (metrics.costUsd != null) {
      costUsd += metrics.costUsd;
      hasCost = true;
    }
  }

  if (hasCost) summary.costUsd = costUsd;
  return summary;
}

export function summarizeThreadMetricsBreakdown(
  messages: ChatMessage[],
): ThreadMetricsBreakdown {
  const lifetime = summarizeResponseMetrics(messages);
  addCompactionSummaryMetrics(lifetime, messages);

  const checkpointIndex = latestCompactionIndex(messages);
  if (checkpointIndex < 0) return { lifetime };

  return {
    lifetime,
    checkpoint: messages[checkpointIndex].compaction,
    sinceCheckpoint: summarizeResponseMetrics(
      messages.slice(checkpointIndex + 1),
    ),
  };
}

export function latestProviderLimits(
  messages: ChatMessage[],
  provider?: string,
): ProviderLimitInfo[] {
  const normalizedProvider = provider?.trim().toLowerCase();
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const limits =
      messages[index].role === "assistant"
        ? messages[index].metrics?.limits
        : undefined;
    const matching = normalizedProvider
      ? limits?.filter((limit) => limit.provider.toLowerCase().includes(normalizedProvider))
      : limits;
    if (matching?.length) return matching;
  }
  return [];
}

export function codexLimitsFromRateLimitPayload(
  payload: unknown,
): ProviderLimitInfo[] {
  const root =
    unwrapObjectField(payload, "rateLimits") ??
    unwrapObjectField(payload, "rate_limits") ??
    unwrapObjectField(payload, "limits") ??
    payload;
  const limits = collectLimitCandidates(root, "Codex", 0);
  if (limits.length) return limits;
  return payload && typeof payload === "object"
    ? [{ provider: "Codex", label: "Codex limits available", raw: payload }]
    : [];
}

export function formatProviderLimits(
  limits: ProviderLimitInfo[],
  now = Date.now(),
): string | null {
  const labels = limits
    .map((limit) => formatProviderLimit(limit, now))
    .filter(Boolean);
  return labels.length ? labels.join(" · ") : null;
}

export function formatCompactProviderLimits(
  limits: ProviderLimitInfo[],
): string | null {
  const values = limits
    .map((limit) => formatCompactProviderLimit(limit))
    .filter((value): value is string => Boolean(value));
  if (!values.length) return null;
  const provider = compactProviderName(limits[0]?.provider);
  return [provider, ...values].filter(Boolean).join(" · ");
}

export function summarizeMilimUsage(
  sessions: MilimUsageSession[],
  projects: MilimUsageProject[],
  now = Date.now(),
): MilimUsageSummary {
  const archivedProjectFolders = new Set(
    projects
      .filter((project) => project.archivedAt)
      .map((project) => normalizeFolder(project.folder)),
  );
  const visibleSessions = sessions.filter((session) => {
    if (session.archivedAt || session.messages.length === 0) return false;
    const folder = normalizeFolder(session.settings?.folder);
    return !folder || !archivedProjectFolders.has(folder);
  });

  const latest = new Date(now);
  const latestMonth = new Date(latest.getFullYear(), latest.getMonth(), 1);
  const months = Array.from(
    { length: USAGE_MONTH_COUNT },
    (_, index): MilimUsageMonth => {
      const date = new Date(
        latestMonth.getFullYear(),
        latestMonth.getMonth() - (USAGE_MONTH_COUNT - 1 - index),
        1,
      );
      return {
        key: monthKey(date),
        label: `${MONTH_LABELS[date.getMonth()]} ${date.getFullYear()}`,
        days: Array.from({ length: daysInMonth(date) }, () => 0),
      };
    },
  );
  const monthIndex = new Map(months.map((month, index) => [month.key, index]));
  const firstMonth = months[0].key;
  const rangeStart = new Date(
    Number(firstMonth.slice(0, 4)),
    Number(firstMonth.slice(5, 7)) - 1,
    1,
  ).getTime();
  const rangeEnd = new Date(
    latestMonth.getFullYear(),
    latestMonth.getMonth() + 1,
    1,
  ).getTime();

  for (const session of visibleSessions) {
    if (
      !Number.isFinite(session.updatedAt) ||
      session.updatedAt < rangeStart ||
      session.updatedAt >= rangeEnd
    )
      continue;
    const updated = new Date(session.updatedAt);
    const index = monthIndex.get(monthKey(updated));
    if (index == null) continue;
    months[index].days[updated.getDate() - 1] += 1;
  }

  const activeDayCount = months.reduce(
    (count, month) => count + month.days.filter((value) => value > 0).length,
    0,
  );
  const threadCount = visibleSessions.length;
  const projectCount = projects.filter(
    (project) => !project.archivedAt && normalizeFolder(project.folder),
  ).length;

  return {
    months,
    metrics: [
      { label: "Threads", value: formatCompactCount(threadCount) },
      { label: "Projects", value: formatCompactCount(projectCount) },
      { label: "Active days", value: formatCompactCount(activeDayCount) },
    ],
    threadCount,
    projectCount,
    activeDayCount,
    hasUsage: activeDayCount > 0,
  };
}

export function formatResponseMetrics(
  metrics?: ResponseMetrics,
): string | null {
  if (!metrics?.endedAt) return null;
  const model = rawModelId(metrics.model)
    .replace(/^(?:codex|claude|opencode|pi):/, "")
    .trim();
  return [
    [metrics.provider?.trim(), model].filter(Boolean).join(" · "),
    formatMetricParts(
      metrics.durationMs ?? 0,
      metrics.usage?.total_tokens ?? 0,
      metrics.costUsd,
    ),
  ].filter(Boolean).join(" · ");
}

export function formatThreadMetrics(
  summary: ThreadMetricsSummary,
): string | null {
  if (summary.responseCount === 0) return null;
  return formatMetricParts(
    summary.durationMs,
    summary.usage.total_tokens,
    summary.costUsd,
  );
}

export function formatThreadMetricsBreakdown(
  messages: ChatMessage[],
): FormattedThreadMetrics {
  const breakdown = summarizeThreadMetricsBreakdown(messages);
  const lifetimeLabel = formatThreadMetrics(breakdown.lifetime);
  if (!breakdown.checkpoint) {
    return {
      label: lifetimeLabel,
      title: lifetimeLabel ? `Thread totals: ${lifetimeLabel}` : null,
    };
  }

  const sinceLabel = breakdown.sinceCheckpoint
    ? (formatThreadMetrics(breakdown.sinceCheckpoint) ??
      "0 post-checkpoint responses")
    : "0 post-checkpoint responses";
  const checkpointLabel = breakdown.checkpoint.baseline
    ? (formatThreadMetrics(breakdown.checkpoint.baseline) ?? "0 responses")
    : "not recorded";
  const summaryLabel = formatCompactionSummaryMetrics(
    breakdown.checkpoint.summary,
  );
  const label = lifetimeLabel
    ? `${lifetimeLabel} · since compact ${sinceLabel}`
    : `since compact ${sinceLabel}`;
  const titleLines = [
    lifetimeLabel ? `Thread lifetime: ${lifetimeLabel}` : null,
    `At latest checkpoint: ${checkpointLabel}`,
    summaryLabel ? `Compaction summary: ${summaryLabel}` : null,
    `Since checkpoint: ${sinceLabel}`,
  ].filter(Boolean);

  return {
    label,
    title: titleLines.join("\n"),
  };
}

function latestCompactionIndex(messages: ChatMessage[]): number {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].compaction?.kind === "checkpoint") return index;
  }
  return -1;
}

function addCompactionSummaryMetrics(
  summary: ThreadMetricsSummary,
  messages: ChatMessage[],
): void {
  let costUsd = summary.costUsd ?? 0;
  let hasCost = summary.costUsd != null;

  for (const message of messages) {
    const metrics = message.compaction?.summary;
    if (!metrics) continue;
    summary.responseCount += 1;
    summary.durationMs += metrics.durationMs ?? 0;
    if (metrics.usage) {
      summary.usage.prompt_tokens += metrics.usage.prompt_tokens;
      summary.usage.completion_tokens += metrics.usage.completion_tokens;
      summary.usage.total_tokens += metrics.usage.total_tokens;
    }
    if (metrics.costUsd != null) {
      costUsd += metrics.costUsd;
      hasCost = true;
    }
  }

  if (hasCost) summary.costUsd = costUsd;
}

function formatCompactionSummaryMetrics(
  summary?: NonNullable<ChatMessage["compaction"]>["summary"],
): string | null {
  if (!summary) return null;
  return formatMetricParts(
    summary.durationMs ?? 0,
    summary.usage?.total_tokens ?? 0,
    summary.costUsd,
  );
}

function providerForModel(
  model: string,
  providers: ProviderInfo[],
): ProviderInfo | undefined {
  return providers.find((provider) => providerOwnsModel(provider, model));
}

function collectLimitCandidates(
  value: unknown,
  provider: string,
  depth: number,
  keyHint?: string,
): ProviderLimitInfo[] {
  if (!value || depth > 4) return [];
  if (Array.isArray(value))
    return value.flatMap((item) =>
      collectLimitCandidates(item, provider, depth + 1, keyHint),
    );
  if (typeof value !== "object") return [];
  const obj = value as Record<string, unknown>;
  const resetAt = readNumber(obj, [
    "reset_at",
    "resetAt",
    "resets_at",
    "resetsAt",
    "reset",
  ]);
  const remaining = readNumber(obj, [
    "remaining",
    "remaining_requests",
    "remainingRequests",
    "requestsRemaining",
  ]);
  const limit = readNumber(obj, ["limit", "max", "quota"]);
  const used = readNumber(obj, [
    "used",
    "used_requests",
    "usedRequests",
    "requestsUsed",
  ]);
  const usedPercent = readNumber(obj, [
    "used_percent",
    "usedPercent",
    "percent_used",
    "percentUsed",
  ]);
  const status = readString(obj, ["status", "state"]);
  const kind =
    readString(obj, ["kind", "type", "rateLimitType", "window", "name"]) ??
    keyHint;
  const hasLimitShape =
    resetAt != null ||
    remaining != null ||
    limit != null ||
    used != null ||
    usedPercent != null ||
    status != null;
  if (hasLimitShape) {
    return [
      {
        provider,
        status,
        kind,
        reset_at: resetAt,
        remaining,
        limit,
        used,
        used_percent: usedPercent,
        raw: value,
      },
    ];
  }
  return Object.entries(obj).flatMap(([key, item]) =>
    collectLimitCandidates(item, provider, depth + 1, key),
  );
}

function formatProviderLimit(
  limit: ProviderLimitInfo,
  now: number,
): string | null {
  if (limit.label?.trim()) return limit.label.trim();
  const provider = limit.provider || "Provider";
  const kind = humanLimitKind(limit);
  const quota = formatQuota(limit);
  const status = humanStatus(limit.status);
  const reset =
    typeof limit.reset_at === "number" && shouldShowReset(limit, kind, status)
      ? formatReset(limit.reset_at, now)
      : "";
  if (status === "limit hit" && reset)
    return `${provider} ${limitHitLabel(kind)} · resets ${reset}`
      .replace(/\s+/g, " ")
      .trim();
  if (quota && reset)
    return `${provider} ${kind}${quota} · resets ${reset}`
      .replace(/\s+/g, " ")
      .trim();
  if (reset)
    return `${provider} ${kind}resets ${reset}`.replace(/\s+/g, " ").trim();
  if (quota) return `${provider} ${kind}${quota}`.replace(/\s+/g, " ").trim();
  if (status) return `${provider} ${kind}${status}`.replace(/\s+/g, " ").trim();
  return null;
}

function formatCompactProviderLimit(limit: ProviderLimitInfo): string | null {
  const kind = humanLimitKind(limit).replace(/\s*limit\s*$/i, "").trim();
  const quota = compactRemainingQuota(limit);
  const status = humanStatus(limit.status);
  if (quota) return [kind, quota].filter(Boolean).join(" ");
  if (status) return [kind, status].filter(Boolean).join(" ");
  return null;
}

function compactProviderName(provider?: string): string {
  if (/claude/i.test(provider ?? "")) return "Claude";
  if (/codex/i.test(provider ?? "")) return "Codex";
  return provider?.trim() || "Account";
}

function compactRemainingQuota(limit: ProviderLimitInfo): string {
  if (typeof limit.remaining === "number" && typeof limit.limit === "number")
    return `${formatCompactCount(limit.remaining)}/${formatCompactCount(limit.limit)} left`;
  if (typeof limit.used === "number" && typeof limit.limit === "number")
    return `${formatCompactCount(Math.max(0, limit.limit - limit.used))}/${formatCompactCount(limit.limit)} left`;
  if (typeof limit.used_percent === "number") {
    const usedPercent = limit.used_percent <= 1 ? limit.used_percent * 100 : limit.used_percent;
    return `${Math.round(Math.max(0, 100 - usedPercent))}% left`;
  }
  if (typeof limit.remaining === "number")
    return `${formatCompactCount(limit.remaining)} left`;
  return "";
}

function formatQuota(limit: ProviderLimitInfo): string {
  if (typeof limit.remaining === "number" && typeof limit.limit === "number")
    return `${formatCompactCount(limit.remaining)}/${formatCompactCount(limit.limit)} left`;
  if (typeof limit.used === "number" && typeof limit.limit === "number")
    return `${formatCompactCount(limit.used)}/${formatCompactCount(limit.limit)} used`;
  if (typeof limit.used_percent === "number")
    return `${Math.round(limit.used_percent * (limit.used_percent <= 1 ? 100 : 1))}% used`;
  if (typeof limit.remaining === "number")
    return `${formatCompactCount(limit.remaining)} left`;
  return "";
}

function formatReset(resetAtSeconds: number, now: number): string {
  const date = new Date(
    resetAtSeconds > 10_000_000_000 ? resetAtSeconds : resetAtSeconds * 1000,
  );
  if (!Number.isFinite(date.getTime())) return "";
  const deltaMs = date.getTime() - now;
  if (deltaMs > 0 && deltaMs < 90 * 60 * 1000) {
    const minutes = Math.max(1, Math.round(deltaMs / 60_000));
    return minutes === 1 ? "in 1m" : `in ${minutes}m`;
  }
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function humanLimitKind(limit: ProviderLimitInfo): string {
  const raw =
    limit.raw && typeof limit.raw === "object" && !Array.isArray(limit.raw)
      ? (limit.raw as Record<string, unknown>)
      : {};
  const windowMins = readNumber(raw, [
    "windowDurationMins",
    "window_duration_mins",
  ]);
  if (windowMins === 300) return "5h limit ";
  if (windowMins === 10_080) return "weekly limit ";

  const normalized = (limit.kind ?? "").trim().replace(/_/g, " ");
  if (!normalized) return "";
  if (/five hour/i.test(normalized) || normalized === "primary")
    return "5h limit ";
  if (/weekly/i.test(normalized) || normalized === "secondary")
    return "weekly limit ";
  if (/daily/i.test(normalized)) return "daily limit ";
  return `${normalized} `;
}

function limitHitLabel(kind: string): string {
  return kind.endsWith("limit ") ? `${kind}hit` : `${kind}limit hit`;
}

function shouldShowReset(
  limit: ProviderLimitInfo,
  kind: string,
  status: string,
): boolean {
  return (
    !kind.startsWith("weekly limit ") ||
    status === "limit hit" ||
    quotaExhausted(limit)
  );
}

function quotaExhausted(limit: ProviderLimitInfo): boolean {
  if (typeof limit.remaining === "number") return limit.remaining <= 0;
  if (typeof limit.used === "number" && typeof limit.limit === "number")
    return limit.used >= limit.limit;
  if (typeof limit.used_percent === "number")
    return limit.used_percent >= (limit.used_percent <= 1 ? 1 : 100);
  return false;
}

function humanStatus(value?: string | null): string {
  const normalized = (value ?? "").trim().toLowerCase();
  if (!normalized || normalized === "ok" || normalized === "active") return "";
  if (
    normalized === "rejected" ||
    normalized === "limited" ||
    normalized === "rate_limited"
  )
    return "limit hit";
  return normalized.replace(/_/g, " ");
}

function unwrapObjectField(value: unknown, field: string): unknown | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)[field]
    : undefined;
}

function readNumber(
  obj: Record<string, unknown>,
  keys: string[],
): number | undefined {
  for (const key of keys) {
    const value = obj[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return undefined;
}

function readString(
  obj: Record<string, unknown>,
  keys: string[],
): string | undefined {
  for (const key of keys) {
    const value = obj[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  return undefined;
}

function usdPerToken(value: string | null | undefined): number | null {
  if (value == null || value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : null;
}

function formatMetricParts(
  durationMs: number,
  tokens: number,
  costUsd?: number,
): string | null {
  const parts = [formatDuration(durationMs)];
  if (tokens > 0) parts.push(formatTokens(tokens));
  if (costUsd != null) parts.push(`est. ${formatUsd(costUsd)}`);
  return parts.filter(Boolean).join(" · ") || null;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m ${seconds % 60}s`;
}

export function formatTokens(tokens: number): string {
  if (tokens < 1000) return `${tokens} tokens`;
  if (tokens < 1_000_000)
    return `${trimFixed(tokens / 1000, tokens < 10_000 ? 1 : 0)}k tokens`;
  return `${trimFixed(tokens / 1_000_000, tokens < 10_000_000 ? 1 : 0)}M tokens`;
}

function formatUsd(value: number): string {
  if (value >= 0.01) return `$${value.toFixed(2)}`;
  if (value >= 0.001) return `$${value.toFixed(3)}`;
  return `$${value.toFixed(4)}`;
}

function formatCompactCount(value: number): string {
  if (value < 1000) return String(value);
  if (value < 1_000_000)
    return `${trimFixed(value / 1000, value < 10_000 ? 1 : 0)}k`;
  return `${trimFixed(value / 1_000_000, value < 10_000_000 ? 1 : 0)}M`;
}

function trimFixed(value: number, digits: number): string {
  return value.toFixed(digits).replace(/\.0$/, "");
}

function normalizeFolder(folder?: string): string {
  return (folder ?? "").trim();
}

function daysInMonth(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth() + 1, 0).getDate();
}

function monthKey(date: Date): string {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}
