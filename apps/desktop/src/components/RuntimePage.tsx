import { useState } from "react";
import { DEFAULT_ACCOUNT_PROFILE_ID, type AccountRuntimeKind } from "../api";
import {
  ACCOUNT_RUNTIME_LABEL,
  modelCount,
  shortenHomePath,
} from "../lib/providerConnections";
import type { ContextMenuItem } from "./ContextMenu";
import { Copy, Download, Folder, Refresh, Undo, X } from "./icons";
import { OverflowMenuButton } from "./OverflowMenuButton";
import { ProviderIcon } from "./ProviderIcon";
import { StatusDot } from "./ProvidersRail";
import { RuntimeAccounts } from "./RuntimeAccounts";
import {
  copyRuntimeInstallCommand,
  locateRuntimeBinary,
  resetRuntimeBinary,
  RuntimeInstallHint,
} from "./RuntimeInstallHint";
import type { AccountRuntimesState } from "./useAccountRuntimes";
import { Toggle } from "./ui";

const SIGNED_OUT_GUIDANCE: Record<AccountRuntimeKind, string> = {
  codex: "Sign in with your ChatGPT account to use Codex models.",
  claude: "Run `claude auth login` in a terminal, then refresh.",
  opencode: "Configure a provider in OpenCode, then refresh.",
  pi: "Use /login in Pi's terminal, then refresh.",
};

type PrimaryAction = {
  label: string;
  testId: string;
  disabled?: boolean;
  onClick: () => void;
};

/**
 * One coding CLI: a header with status and the Enabled switch, at most one
 * prominent action, every other action in the same overflow menu, and the
 * accounts list for runtimes that support several accounts.
 */
export function RuntimePage({
  runtime,
  state,
  onImportChats,
}: {
  runtime: AccountRuntimeKind;
  state: AccountRuntimesState;
  onImportChats: (runtime: "codex" | "claude") => void;
}) {
  const [binaryBusy, setBinaryBusy] = useState(false);
  const [localNote, setLocalNote] = useState<{ tone: "ready" | "error"; message: string } | null>(null);
  const name = ACCOUNT_RUNTIME_LABEL[runtime];
  const summary = state.summaries[runtime];
  const enabled = Boolean(state.enabled[runtime]);
  const update = state.updates[runtime];
  const missing = state.missing(runtime);
  const ready = state.ready[runtime];
  const busy = Boolean(state.busy[runtime]);
  const note = state.notes[runtime];
  const overridePath = state.binaries[runtime];
  const confirming = state.confirmUpdate === runtime;
  const updatingThis = state.updating === runtime;
  const anyUpdating = state.updating !== null;
  const installed = !missing && update?.available !== false;
  const signedOut = enabled && installed && summary.label !== "Checking" && !ready;

  async function changeBinary(action: () => Promise<boolean>) {
    setBinaryBusy(true);
    setLocalNote(null);
    try {
      if (await action()) state.binaryChanged(runtime);
    } catch (cause) {
      setLocalNote({ tone: "error", message: cause instanceof Error ? cause.message : String(cause) });
    } finally {
      setBinaryBusy(false);
    }
  }

  const updateLabel = update?.update_available === true
    ? update.latest_version
      ? `Update to v${update.latest_version}`
      : "Update available"
    : "Run CLI updater";

  let primary: PrimaryAction | null = null;
  if (enabled && (confirming || updatingThis)) {
    primary = {
      label: updatingThis ? "Updating..." : "Confirm update",
      testId: `${runtime}-update`,
      disabled: anyUpdating,
      onClick: () => void state.runUpdate(runtime),
    };
  } else if (enabled && installed && runtime === "codex" && signedOut) {
    primary = {
      label: busy ? "Working..." : "Sign in",
      testId: "codex-connect",
      disabled: busy,
      onClick: () => void state.connectCodex(),
    };
  } else if (enabled && installed && summary.updateAvailable) {
    primary = {
      label: updateLabel,
      testId: `${runtime}-update`,
      disabled: anyUpdating,
      onClick: () => void state.runUpdate(runtime),
    };
  } else if (signedOut) {
    primary = {
      label: busy ? "Refreshing..." : "Refresh status",
      testId: `${runtime}-refresh`,
      disabled: busy,
      onClick: () => void state.refreshRuntime(runtime),
    };
  }

  const menu: ContextMenuItem[] = [
    {
      id: "refresh",
      label: busy ? "Refreshing status" : "Refresh status",
      icon: <Refresh size={13} />,
      disabled: !enabled || busy,
      action: () => state.refreshRuntime(runtime),
    },
  ];
  if ((runtime === "codex" && ready) || (runtime === "claude" && enabled)) {
    menu.push({
      id: "import",
      label: "Import chats...",
      icon: <Download size={13} />,
      action: () => onImportChats(runtime),
    });
  }
  if (enabled && installed && update?.update_available !== false && primary?.testId !== `${runtime}-update`) {
    menu.push({
      id: "update",
      label: updateLabel,
      icon: <Download size={13} />,
      description: update?.update_error ?? undefined,
      disabled: anyUpdating,
      action: () => state.runUpdate(runtime),
    });
  }
  if (enabled) {
    menu.push({
      id: "locate",
      label: overridePath ? "Change binary..." : "Locate binary...",
      icon: <Folder size={13} />,
      separatorBefore: true,
      disabled: binaryBusy,
      action: () => changeBinary(() => locateRuntimeBinary(runtime)),
    });
    if (overridePath) {
      menu.push({
        id: "reset-binary",
        label: "Reset binary",
        description: "Find the CLI automatically again",
        icon: <Undo size={13} />,
        disabled: binaryBusy,
        action: () => changeBinary(() => resetRuntimeBinary(runtime)),
      });
    }
  }
  menu.push({
    id: "copy-install",
    label: "Copy install command",
    icon: <Copy size={13} />,
    separatorBefore: !enabled,
    action: () =>
      copyRuntimeInstallCommand(runtime).then(
        () => setLocalNote({ tone: "ready", message: "Install command copied." }),
        (cause) => setLocalNote({ tone: "error", message: cause instanceof Error ? cause.message : String(cause) }),
      ),
  });
  if (runtime === "codex" && ready) {
    menu.push({
      id: "disconnect",
      label: "Disconnect",
      icon: <X size={13} />,
      danger: true,
      separatorBefore: true,
      disabled: busy,
      action: () => state.disconnectCodex(),
    });
  }

  const facts: string[] = [];
  const identity =
    runtime === "codex"
      ? state.codexAccount?.account?.email ?? state.codexAccount?.account?.planType
      : runtime === "claude"
        ? state.claudeStatus?.auth?.email ?? state.claudeStatus?.auth?.subscriptionType
        : null;
  if (enabled && identity) facts.push(identity);
  if (summary.version) facts.push(`v${summary.version}`);

  const versionState =
    installed && update?.update_available === false
      ? "Up to date"
      : installed && update?.update_available === true && update.latest_version
        ? `v${update.latest_version} available`
        : null;

  const details: Array<[string, string]> = [];
  if (enabled && runtime === "claude" && state.claudeStatus?.available) {
    if (state.claudeStatus.auth?.subscriptionType)
      details.push(["Plan", state.claudeStatus.auth.subscriptionType]);
    if (state.claudeStatus.models) details.push(["Models", modelCount(state.claudeStatus.models.length)]);
  }
  if (enabled && runtime === "codex" && state.codexAccount?.account?.planType) {
    details.push(["Plan", state.codexAccount.account.planType]);
  }
  if (enabled && runtime === "opencode" && state.openCodeStatus?.available) {
    details.push(["Models", `${modelCount(state.openCodeStatus.models?.length ?? 0)} configured`]);
  }
  if (enabled && runtime === "pi" && state.piStatus?.available) {
    const providers = state.piStatus.provider_count ?? 0;
    details.push(["Providers", `${providers} configured`]);
    details.push(["Models", modelCount(state.piStatus.models?.length ?? 0)]);
  }
  if (installed) {
    details.push(["Binary", overridePath ? shortenHomePath(overridePath) : "Found automatically"]);
  }

  const identities: Record<string, string | null | undefined> =
    runtime === "codex"
      ? { [state.codexAccount?.profile_id ?? DEFAULT_ACCOUNT_PROFILE_ID]: state.codexAccount?.account?.email }
      : runtime === "claude"
        ? { [state.claudeStatus?.profile_id ?? DEFAULT_ACCOUNT_PROFILE_ID]: state.claudeStatus?.auth?.email }
        : {};

  return (
    <section className="providers-page runtime-page" aria-labelledby={`${runtime}-page-title`} data-testid={`${runtime}-runtime-page`}>
      <header className="providers-page-head">
        <ProviderIcon brand={runtime} size={28} />
        <div className="providers-page-title">
          <span className="providers-page-kicker">Coding CLI</span>
          <h3 id={`${runtime}-page-title`}>{name}</h3>
          <p className="providers-status-line">
            <StatusDot tone={summary.tone} />
            <span className="providers-status-label">{summary.label}</span>
            {facts.map((fact) => (
              <span key={fact} className="providers-status-fact">{fact}</span>
            ))}
            {versionState && (
              <span className="providers-status-fact muted" title={update?.update_error ?? undefined}>
                {versionState}
              </span>
            )}
          </p>
        </div>
        <div className="providers-page-actions">
          {primary && (
            <button
              className="btn-ghost providers-emphasis-button"
              type="button"
              data-testid={primary.testId}
              disabled={primary.disabled}
              onClick={primary.onClick}
            >
              {primary.label}
            </button>
          )}
          <OverflowMenuButton label={`${name} actions`} items={menu} testId={`${runtime}-actions`} />
          <Toggle
            checked={enabled}
            onChange={(next) => state.setEnabled(runtime, next)}
            label="Enabled"
            ariaLabel={`Enable ${name} runtime`}
            testId={`${runtime}-enabled-toggle`}
          />
        </div>
      </header>

      {!enabled && (
        <p className="runtime-muted">
          Disabled. {name} models are hidden and new runs are blocked; the CLI stays signed in.
        </p>
      )}
      {enabled && missing && (
        <RuntimeInstallHint
          runtime={runtime}
          missing
          overridePath={overridePath}
          onChanged={() => state.binaryChanged(runtime)}
        />
      )}
      {signedOut && <p className="runtime-muted">{SIGNED_OUT_GUIDANCE[runtime]}</p>}
      {note && <p className={"provider-note " + note.tone} role="status">{note.message}</p>}
      {localNote && <p className={"provider-note " + localNote.tone} role="status">{localNote.message}</p>}

      {details.length > 0 && (
        <dl className="providers-facts">
          {details.map(([label, value]) => (
            <div key={label}>
              <dt>{label}</dt>
              <dd title={label === "Binary" && overridePath ? overridePath : undefined}>{value}</dd>
            </div>
          ))}
        </dl>
      )}

      {(runtime === "codex" || runtime === "claude") && (
        <RuntimeAccounts
          runtime={runtime}
          list={state.profiles[runtime]}
          error={state.profileErrors[runtime]}
          disabled={!enabled}
          identities={identities}
          onReload={() => state.refreshProfiles(runtime)}
          onChanged={() => void state.refreshStatus(runtime, runtime === "claude")}
        />
      )}
    </section>
  );
}
