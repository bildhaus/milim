import { useCallback, useEffect, useState } from "react";
import {
  createAccountProfile,
  deleteAccountProfile,
  listAccountProfiles,
  updateAccountProfile,
  type AccountProfile,
  type AccountProfileLoginHint,
  type AccountProfileRuntime,
} from "../api";
import { confirmApp } from "../ui/confirmation";
import { Check, Copy, Plus, Refresh, Trash } from "./icons";
import { Toggle } from "./ui";

const RUNTIME_LABEL: Record<AccountProfileRuntime, string> = {
  claude: "Claude",
  codex: "Codex",
};

/**
 * How a runtime reports its own limits, which is all Milim knows about an
 * account it holds no credential for. Codex publishes percentages up front, so
 * Auto can prefer the account with the most headroom before a turn. Claude
 * only reports a limit once a turn is at or past a cap, so its accounts show
 * usage after the fact.
 */
const USAGE_SOURCE: Record<AccountProfileRuntime, string> = {
  codex: "Usage is read from Codex before each turn.",
  claude:
    "Claude reports a limit only once a turn reaches one, so usage appears after a capped turn.",
};

export function formatCooldown(untilMs: number | undefined, now: number): string | null {
  if (!untilMs || untilMs <= now) return null;
  const minutes = Math.ceil((untilMs - now) / 60_000);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

/** The most-consumed window Milim has a figure for. */
export function peakUsagePercent(profile: AccountProfile): number | null {
  const values = [profile.short_window_percent, profile.long_window_percent].filter(
    (value): value is number => typeof value === "number",
  );
  return values.length ? Math.max(...values) : null;
}

function profileSummary(profile: AccountProfile, now: number): string {
  const cooldown = formatCooldown(profile.cooled_until_ms, now);
  if (cooldown) {
    const window = profile.cooldown_kind?.replace(/_/g, " ");
    return `Rate limited${window ? ` (${window})` : ""} · resets in ${cooldown}`;
  }
  const usage = peakUsagePercent(profile);
  if (usage != null) return `${Math.round(usage)}% of the current window used`;
  return profile.is_default
    ? "The CLI's own configuration folder"
    : (profile.config_dir ?? "");
}

function LoginHint({ hint }: { hint: AccountProfileLoginHint }) {
  const [copied, setCopied] = useState(false);
  const command =
    navigator.userAgent.includes("Windows") ? hint.powershell : hint.posix;
  return (
    <div className="account-profile-hint">
      <p>
        Sign this account in by running the command below in a terminal, then
        refresh. Milim never sees the credential; the CLI writes it into that
        folder.
      </p>
      <div className="account-profile-command">
        <code>{command}</code>
        <button
          className="btn-ghost"
          type="button"
          title="Copy the sign-in command"
          aria-label="Copy the sign-in command"
          onClick={() => {
            void navigator.clipboard.writeText(command).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1500);
            });
          }}
        >
          {copied ? <Check size={13} /> : <Copy size={13} />}
        </button>
      </div>
    </div>
  );
}

/**
 * Every signed-in account for one runtime, plus the controls to add, pause,
 * rename, and forget them.
 */
export function AccountProfilesPanel({
  runtime,
  disabled = false,
  onChanged,
}: {
  runtime: AccountProfileRuntime;
  disabled?: boolean;
  onChanged?: () => void;
}) {
  const [profiles, setProfiles] = useState<AccountProfile[]>([]);
  const [autoSelection, setAutoSelection] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [hint, setHint] = useState<{
    profileId: string;
    hint: AccountProfileLoginHint;
  } | null>(null);
  const [now, setNow] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    try {
      const result = await listAccountProfiles(runtime);
      setProfiles(result.profiles);
      setAutoSelection(result.auto_selection);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [runtime]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Cooldown timers are the only thing that moves on its own here.
  useEffect(() => {
    if (!profiles.some((profile) => profile.cooled_until_ms)) return;
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, [profiles]);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    try {
      await action();
      await refresh();
      onChanged?.();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    const label = newLabel.trim();
    if (!label) return;
    await run(async () => {
      const created = await createAccountProfile(runtime, label);
      setNewLabel("");
      setAdding(false);
      if (created.login_hint) {
        setHint({ profileId: created.profile.id, hint: created.login_hint });
      }
    });
  }

  async function remove(profile: AccountProfile) {
    const confirmed = await confirmApp({
      title: `Forget ${profile.label}?`,
      message:
        `Milim stops offering this ${RUNTIME_LABEL[runtime]} account. Its folder stays on disk at ${profile.config_dir}, including that account's own credentials and chat history. ` +
        "Chats pinned to it fall back to the default account.",
      confirmLabel: "Forget account",
      tone: "danger",
    });
    if (!confirmed) return;
    await run(async () => {
      await deleteAccountProfile(runtime, profile.id);
      setHint((current) => (current?.profileId === profile.id ? null : current));
    });
  }

  // Only the extra accounts are worth listing; one account needs no UI.
  if (!profiles.some((profile) => !profile.is_default) && !adding) {
    return (
      <div className="account-profile-panel">
        <button
          className="btn-ghost"
          type="button"
          data-testid={`${runtime}-add-account`}
          disabled={disabled || busy}
          onClick={() => setAdding(true)}
        >
          <Plus size={13} /> Add another {RUNTIME_LABEL[runtime]} account
        </button>
        {error && <p className="provider-note error">{error}</p>}
      </div>
    );
  }

  return (
    <div className="account-profile-panel">
      <div className="account-profile-heading">
        <strong>{RUNTIME_LABEL[runtime]} accounts</strong>
        <button
          className="btn-ghost provider-account-refresh"
          type="button"
          title={`Refresh ${RUNTIME_LABEL[runtime]} accounts`}
          aria-label={`Refresh ${RUNTIME_LABEL[runtime]} accounts`}
          disabled={busy}
          onClick={() => void refresh()}
        >
          <Refresh size={13} />
        </button>
      </div>
      <ul className="account-profile-list">
        {profiles.map((profile) => {
          const cooling = Boolean(formatCooldown(profile.cooled_until_ms, now));
          const usage = peakUsagePercent(profile);
          return (
            <li
              key={profile.id}
              className={"account-profile-row" + (cooling ? " cooling" : "")}
              data-testid={`${runtime}-account-${profile.id}`}
            >
              <div className="account-profile-identity">
                <strong>
                  {profile.label}
                  {autoSelection === profile.id && (
                    <span className="account-profile-badge">Auto picks this</span>
                  )}
                </strong>
                <span title={profile.config_dir}>{profileSummary(profile, now)}</span>
                {usage != null && (
                  <div
                    className="account-profile-meter"
                    role="img"
                    aria-label={`${Math.round(usage)} percent of the current window used`}
                  >
                    <div style={{ width: `${Math.min(100, Math.max(0, usage))}%` }} />
                  </div>
                )}
              </div>
              <div className="account-profile-actions">
                <Toggle
                  checked={profile.enabled}
                  disabled={disabled || busy || profile.is_default}
                  onChange={(enabled) =>
                    void run(async () => {
                      await updateAccountProfile(runtime, profile.id, { enabled });
                    })
                  }
                  label="Auto"
                  ariaLabel={`Include ${profile.label} in automatic switching`}
                  testId={`${runtime}-account-${profile.id}-auto`}
                />
                {!profile.is_default && (
                  <button
                    className="btn-ghost danger"
                    type="button"
                    title={`Forget ${profile.label}`}
                    aria-label={`Forget ${profile.label}`}
                    disabled={disabled || busy}
                    onClick={() => void remove(profile)}
                  >
                    <Trash size={13} />
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      {hint && <LoginHint hint={hint.hint} />}
      {adding ? (
        <div className="account-profile-add">
          <input
            autoFocus
            value={newLabel}
            placeholder={`Name this account, e.g. "Work ${RUNTIME_LABEL[runtime]}"`}
            aria-label={`New ${RUNTIME_LABEL[runtime]} account name`}
            data-testid={`${runtime}-new-account-name`}
            onChange={(event) => setNewLabel(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void add();
              if (event.key === "Escape") {
                setAdding(false);
                setNewLabel("");
              }
            }}
          />
          <button
            className="btn-accent"
            type="button"
            disabled={busy || !newLabel.trim()}
            onClick={() => void add()}
          >
            Add
          </button>
          <button
            className="btn-ghost"
            type="button"
            onClick={() => {
              setAdding(false);
              setNewLabel("");
            }}
          >
            Cancel
          </button>
        </div>
      ) : (
        <button
          className="btn-ghost"
          type="button"
          data-testid={`${runtime}-add-account`}
          disabled={disabled || busy}
          onClick={() => setAdding(true)}
        >
          <Plus size={13} /> Add another {RUNTIME_LABEL[runtime]} account
        </button>
      )}
      <p className="account-profile-footnote">
        Each account keeps its own folder, sign-in, settings, and chat history.
        Chats choose one with the account chip, or leave it on{" "}
        <strong>Auto</strong> to use whichever has the most room left.{" "}
        {USAGE_SOURCE[runtime]}
      </p>
      {error && <p className="provider-note error">{error}</p>}
    </div>
  );
}
