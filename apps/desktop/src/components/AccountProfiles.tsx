import type { AccountProfile } from "../api";

/**
 * Shared formatting for account profiles. The Providers accounts list lives
 * in RuntimeAccounts; the chat account chip reuses these helpers.
 */

export function formatCooldown(untilMs: number | undefined, now: number): string | null {
  if (!untilMs || untilMs <= now) return null;
  const minutes = Math.ceil((untilMs - now) / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** The most-consumed window milim has a figure for. */
export function peakUsagePercent(profile: AccountProfile): number | null {
  const values = [profile.short_window_percent, profile.long_window_percent].filter(
    (value): value is number => typeof value === "number",
  );
  return values.length ? Math.max(...values) : null;
}
