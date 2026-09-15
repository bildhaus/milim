import {
  DEFAULT_ACCOUNT_PROFILE_ID,
  accountRuntimeKind,
  isAccountProfileRuntime,
  type AccountProfileRuntime,
} from "../api.js";

/**
 * The account-profile runtime behind a model id. Provider models and the
 * account runtimes that do not relocate their configuration home (OpenCode,
 * Pi) have none, so no account selection applies to them.
 */
export function accountProfileRuntimeForModel(
  model: string,
): AccountProfileRuntime | null {
  const kind = accountRuntimeKind(model);
  return kind && isAccountProfileRuntime(kind) ? kind : null;
}

/** The account a thread selected for whichever runtime `model` belongs to. */
export function accountProfileForModel(
  profiles: Record<string, string> | undefined,
  model: string,
): string | undefined {
  const runtime = accountProfileRuntimeForModel(model);
  return runtime ? profiles?.[runtime] : undefined;
}

/**
 * Apply one runtime's selection, dropping entries that mean "the runtime's own
 * account" so an unset thread stores nothing.
 */
export function withAccountProfile(
  profiles: Record<string, string> | undefined,
  runtime: string,
  profileId: string | null,
): Record<string, string> {
  const next = { ...(profiles ?? {}) };
  if (!profileId || profileId === DEFAULT_ACCOUNT_PROFILE_ID) delete next[runtime];
  else next[runtime] = profileId;
  return next;
}
