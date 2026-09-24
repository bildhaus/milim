import { useEffect, useId, useState } from "react";
import {
  createAccountProfile,
  deleteAccountProfile,
  openArtifactLocation,
  updateAccountProfile,
  type AccountProfile,
  type AccountProfileList,
  type AccountProfileLoginHint,
  type AccountProfileRuntime,
} from "../api";
import { shortenHomePath } from "../lib/providerConnections";
import { confirmApp, promptApp } from "../ui/confirmation";
import { formatCooldown, peakUsagePercent } from "./AccountProfiles";
import type { ContextMenuItem } from "./ContextMenu";
import { Check, Copy, Folder, Info, Pencil, Plus, Trash } from "./icons";
import { OverflowMenuButton } from "./OverflowMenuButton";
import { Toggle } from "./ui";

const RUNTIME_LABEL: Record<AccountProfileRuntime, string> = {
  claude: "Claude",
  codex: "Codex",
};

/**
 * How a runtime reports its own limits, which is all milim knows about an
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

/** One muted line: identity, usage or cooldown, and where the account lives. */
export function accountRowMeta(
  profile: AccountProfile,
  identity: string | null | undefined,
  now: number,
): string {
  const parts: string[] = [];
  if (identity) parts.push(identity);
  const cooldown = formatCooldown(profile.cooled_until_ms, now);
  if (cooldown) {
    const window = profile.cooldown_kind?.replace(/_/g, " ");
    parts.push(`Rate limited${window ? ` (${window})` : ""}, resets in ${cooldown}`);
  } else {
    const usage = peakUsagePercent(profile);
    if (usage != null) parts.push(`${Math.round(usage)}% used`);
  }
  parts.push(
    profile.is_default || !profile.config_dir
      ? "CLI's own folder"
      : shortenHomePath(profile.config_dir),
  );
  return parts.join(" · ");
}

function LoginHint({ hint }: { hint: AccountProfileLoginHint }) {
  const [copied, setCopied] = useState(false);
  const command =
    navigator.userAgent.includes("Windows") ? hint.powershell : hint.posix;
  return (
    <div className="account-profile-hint">
      <p>
        Sign this account in by running the command below in a terminal, then
        refresh. milim never sees the credential; the CLI writes it into that
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
 * Every account milim can use for one runtime, with the same layout for Codex
 * and Claude: Default first, a "Use in Auto" switch per account, an "Auto
 * picks" marker on the account Auto would choose now, and a per-row menu.
 */
export function RuntimeAccounts({
  runtime,
  list,
  error,
  disabled = false,
  identities = {},
  onReload,
  onChanged,
}: {
  runtime: AccountProfileRuntime;
  list: AccountProfileList | undefined;
  error?: string | null;
  disabled?: boolean;
  /** Signed-in identity (email or plan) keyed by profile id, when known. */
  identities?: Record<string, string | null | undefined>;
  onReload: () => Promise<void>;
  onChanged?: () => void;
}) {
  const infoId = useId();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [showInfo, setShowInfo] = useState(false);
  const [hint, setHint] = useState<{ profileId: string; hint: AccountProfileLoginHint } | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const profiles = list?.profiles ?? [];
  const label = RUNTIME_LABEL[runtime];

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
      setActionError(null);
      await onReload();
      onChanged?.();
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    const next = newLabel.trim();
    if (!next) return;
    await run(async () => {
      const created = await createAccountProfile(runtime, next);
      setNewLabel("");
      setAdding(false);
      if (created.login_hint) setHint({ profileId: created.profile.id, hint: created.login_hint });
    });
  }

  async function rename(profile: AccountProfile) {
    const next = await promptApp({
      title: `Rename ${profile.label}`,
      message: `Choose the name milim shows for this ${label} account.`,
      confirmLabel: "Rename",
      input: { label: "Account name", defaultValue: profile.label },
    });
    const trimmed = next?.trim();
    if (!trimmed || trimmed === profile.label) return;
    await run(async () => {
      await updateAccountProfile(runtime, profile.id, { label: trimmed });
    });
  }

  async function remove(profile: AccountProfile) {
    const folder = profile.config_dir ? shortenHomePath(profile.config_dir) : "its folder";
    const confirmed = await confirmApp({
      title: `Remove ${profile.label}?`,
      message:
        `milim stops offering this ${label} account. Its folder stays on disk at ${folder}, including that account's own credentials and chat history. ` +
        "Chats pinned to it fall back to the default account.",
      confirmLabel: "Remove account",
      tone: "danger",
    });
    if (!confirmed) return;
    await run(async () => {
      await deleteAccountProfile(runtime, profile.id);
      setHint((current) => (current?.profileId === profile.id ? null : current));
    });
  }

  function rowMenu(profile: AccountProfile): ContextMenuItem[] {
    if (profile.is_default) return [];
    const items: ContextMenuItem[] = [
      {
        id: "rename",
        label: "Rename...",
        icon: <Pencil size={13} />,
        disabled: disabled || busy,
        action: () => rename(profile),
      },
    ];
    const folder = profile.config_dir;
    if (folder) {
      items.push(
        {
          id: "open-folder",
          label: "Open folder",
          icon: <Folder size={13} />,
          action: () => {
            void openArtifactLocation(folder, "folder").catch((cause) =>
              setActionError(cause instanceof Error ? cause.message : String(cause)),
            );
          },
        },
        {
          id: "copy-path",
          label: "Copy path",
          icon: <Copy size={13} />,
          action: () => {
            void navigator.clipboard.writeText(folder);
          },
        },
      );
    }
    items.push({
      id: "remove",
      label: "Remove account...",
      icon: <Trash size={13} />,
      danger: true,
      separatorBefore: true,
      disabled: disabled || busy,
      action: () => remove(profile),
    });
    return items;
  }

  const showAutoPick = profiles.length > 1;

  return (
    <section className="runtime-accounts" aria-labelledby={`${runtime}-accounts-title`}>
      <div className="runtime-section-head">
        <h4 id={`${runtime}-accounts-title`}>Accounts</h4>
        <p className="runtime-accounts-lede">
          <span>Chats pin an account, or use Auto to pick the one with the most room left.</span>
          <button
            className="icon-btn runtime-info-button"
            type="button"
            aria-expanded={showInfo}
            aria-controls={infoId}
            aria-label={`About ${label} accounts`}
            title={`Each account keeps its own folder, sign-in, settings, and chat history. ${USAGE_SOURCE[runtime]}`}
            onClick={() => setShowInfo((value) => !value)}
          >
            <Info size={14} aria-hidden="true" />
          </button>
        </p>
      </div>
      <p className="runtime-accounts-info" id={infoId} hidden={!showInfo}>
        Each account keeps its own folder, sign-in, settings, and chat history.
        Chats choose one with the account chip, which appears once a second
        account exists. {USAGE_SOURCE[runtime]} Removing an account leaves its
        folder and credentials on disk.
      </p>
      {!list && !error && <p className="runtime-muted">Loading accounts...</p>}
      {profiles.length > 0 && (
        <ul className="runtime-account-list" aria-label={`${label} accounts`}>
          {profiles.map((profile) => {
            const cooling = Boolean(formatCooldown(profile.cooled_until_ms, now));
            const usage = peakUsagePercent(profile);
            const autoPick = showAutoPick && list?.auto_selection === profile.id;
            const menu = rowMenu(profile);
            return (
              <li
                key={profile.id}
                className={"runtime-account-row" + (cooling ? " cooling" : "")}
                data-testid={`${runtime}-account-${profile.id}`}
              >
                <div className="runtime-account-main">
                  <div className="runtime-account-name">
                    <strong>{profile.label}</strong>
                    {profile.is_default && profile.label !== "Default" && (
                      <span className="providers-badge">Default</span>
                    )}
                    {autoPick && (
                      <span className="providers-badge accent" title="Auto would use this account for the next turn">
                        Auto picks
                      </span>
                    )}
                  </div>
                  <span
                    className="runtime-account-meta"
                    title={profile.config_dir ?? undefined}
                  >
                    {accountRowMeta(profile, identities[profile.id], now)}
                  </span>
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
                <div className="runtime-account-auto">
                  {profile.is_default ? (
                    <span className="runtime-account-auto-fixed" title="The default account is always available to Auto">
                      Always in Auto
                    </span>
                  ) : (
                  <Toggle
                    checked={profile.enabled}
                    disabled={disabled || busy}
                    onChange={(enabled) =>
                      void run(async () => {
                        await updateAccountProfile(runtime, profile.id, { enabled });
                      })
                    }
                    label="Use in Auto"
                    ariaLabel={`Use ${profile.label} in Auto`}
                    testId={`${runtime}-account-${profile.id}-auto`}
                  />
                  )}
                </div>
                {menu.length ? (
                  <OverflowMenuButton
                    label={`${profile.label} account actions`}
                    items={menu}
                    testId={`${runtime}-account-${profile.id}-menu`}
                  />
                ) : (
                  <span className="overflow-menu-spacer" aria-hidden="true" />
                )}
              </li>
            );
          })}
        </ul>
      )}
      {hint && <LoginHint hint={hint.hint} />}
      {adding ? (
        <form
          className="account-profile-add"
          onSubmit={(event) => {
            event.preventDefault();
            void add();
          }}
        >
          <input
            className="css-input"
            autoFocus
            value={newLabel}
            placeholder={`Name this account, e.g. "Work ${label}"`}
            aria-label={`New ${label} account name`}
            data-testid={`${runtime}-new-account-name`}
            onChange={(event) => setNewLabel(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.stopPropagation();
                setAdding(false);
                setNewLabel("");
              }
            }}
          />
          <button className="btn-ghost" type="submit" disabled={busy || !newLabel.trim()}>
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
        </form>
      ) : (
        <button
          className="providers-text-button"
          type="button"
          data-testid={`${runtime}-add-account`}
          disabled={disabled || busy}
          onClick={() => setAdding(true)}
        >
          <Plus size={13} aria-hidden="true" /> Add account
        </button>
      )}
      {(actionError || error) && (
        <p className="provider-note error" role="alert">{actionError || error}</p>
      )}
    </section>
  );
}
