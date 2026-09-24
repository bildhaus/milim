import { useEffect, useState } from "react";
import {
  getAccountRuntimeBinaries,
  getAccountRuntimeUpdates,
  getClaudeStatus,
  getCodexAccount,
  getOpenCodeStatus,
  getPiStatus,
  isCliPathWarningMessage,
  listAccountProfiles,
  logoutCodex,
  openExternalUrl,
  streamCodexDeviceLogin,
  updateAccountRuntime,
  type AccountProfileList,
  type AccountProfileRuntime,
  type AccountRuntimeBinaries,
  type AccountRuntimeKind,
  type AccountRuntimeUpdateStatus,
  type ClaudeStatusResponse,
  type CodexAccountResponse,
  type CodexLoginEvent,
  type OpenCodeStatusResponse,
  type PiStatusResponse,
} from "../api";
import {
  ACCOUNT_RUNTIME_KINDS,
  ACCOUNT_RUNTIME_LABEL,
  runtimeUpdateTargets,
  summarizeRuntime,
  type RuntimeSnapshot,
  type RuntimeSummary,
  type StatusTone,
} from "../lib/providerConnections";
import { useSettings } from "../settings/store";

export type RuntimeNote = { tone: StatusTone; message: string };
type PerRuntime<T> = Partial<Record<AccountRuntimeKind, T>>;

/**
 * Status, updates, binaries, notes, and accounts for the four coding CLIs,
 * plus the actions the Providers manager exposes for them.
 */
export function useAccountRuntimes() {
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const setAccountRuntimeEnabled = useSettings((s) => s.setAccountRuntimeEnabled);
  const [codexAccount, setCodexAccount] = useState<CodexAccountResponse | null>(null);
  const [claudeStatus, setClaudeStatus] = useState<ClaudeStatusResponse | null>(null);
  const [openCodeStatus, setOpenCodeStatus] = useState<OpenCodeStatusResponse | null>(null);
  const [piStatus, setPiStatus] = useState<PiStatusResponse | null>(null);
  const [checked, setChecked] = useState<PerRuntime<boolean>>({});
  const [busy, setBusyState] = useState<PerRuntime<boolean>>({});
  const [notes, setNotes] = useState<PerRuntime<RuntimeNote | null>>({});
  const [binaries, setBinaries] = useState<AccountRuntimeBinaries>({});
  const [updates, setUpdates] = useState<PerRuntime<AccountRuntimeUpdateStatus>>({});
  const [updatesLoaded, setUpdatesLoaded] = useState(false);
  const [confirmUpdate, setConfirmUpdate] = useState<AccountRuntimeKind | "all" | null>(null);
  const [updating, setUpdating] = useState<AccountRuntimeKind | "all" | null>(null);
  const [updateProgress, setUpdateProgress] = useState<{ current: number; total: number } | null>(null);
  const [profiles, setProfiles] = useState<Partial<Record<AccountProfileRuntime, AccountProfileList>>>({});
  const [profileErrors, setProfileErrors] = useState<Partial<Record<AccountProfileRuntime, string | null>>>({});

  const updateTargets = runtimeUpdateTargets(accountRuntimeEnabled, updates);

  useEffect(() => {
    if (accountRuntimeEnabled.codex) void refreshCodexAccount();
    if (accountRuntimeEnabled.claude) void refreshClaudeStatus();
    if (accountRuntimeEnabled.opencode) void refreshOpenCodeStatus();
    if (accountRuntimeEnabled.pi) void refreshPiStatus();
    void refreshUpdates();
    void refreshBinaries();
    void refreshProfiles("codex");
    void refreshProfiles("claude");
  }, []);

  function setBusy(runtime: AccountRuntimeKind, value: boolean) {
    setBusyState((current) => ({ ...current, [runtime]: value }));
  }

  function markChecked(runtime: AccountRuntimeKind) {
    setChecked((current) => (current[runtime] ? current : { ...current, [runtime]: true }));
  }

  function setNote(runtime: AccountRuntimeKind, note: RuntimeNote | null) {
    setNotes((current) => ({ ...current, [runtime]: note }));
  }

  async function refreshProfiles(runtime: AccountProfileRuntime) {
    try {
      const list = await listAccountProfiles(runtime);
      setProfiles((current) => ({ ...current, [runtime]: list }));
      setProfileErrors((current) => ({ ...current, [runtime]: null }));
    } catch (cause) {
      setProfileErrors((current) => ({
        ...current,
        [runtime]: cause instanceof Error ? cause.message : String(cause),
      }));
    }
  }

  async function refreshBinaries() {
    try {
      setBinaries(await getAccountRuntimeBinaries());
    } catch {
      setBinaries({});
    }
  }

  async function refreshUpdates() {
    try {
      setUpdates((await getAccountRuntimeUpdates()).runtimes);
    } catch {
      setUpdates({});
    } finally {
      setUpdatesLoaded(true);
    }
  }

  async function refreshCodexAccount() {
    try {
      setCodexAccount(await getCodexAccount(false));
    } catch {
      setCodexAccount(null);
    } finally {
      markChecked("codex");
    }
  }

  async function refreshClaudeStatus(showNote = false) {
    setBusy("claude", true);
    try {
      const status = await getClaudeStatus();
      setClaudeStatus(status);
      if (showNote) {
        const message = status.error || "Run `claude auth login` in a terminal, then refresh.";
        const warning = Boolean(status.warning) || isCliPathWarningMessage(message);
        setNote(
          "claude",
          status.available && status.authenticated
            ? {
                tone: "ready",
                message: "Installed Claude CLI connected. Models will appear in the picker after refresh.",
              }
            : { tone: status.available || warning ? "warning" : "error", message },
        );
      }
    } catch (error) {
      setClaudeStatus(null);
      if (showNote) {
        const message = error instanceof Error ? error.message : "Claude CLI status check failed.";
        setNote("claude", { tone: isCliPathWarningMessage(message) ? "warning" : "error", message });
      }
    } finally {
      markChecked("claude");
      setBusy("claude", false);
    }
  }

  async function refreshOpenCodeStatus(showNote = false) {
    setBusy("opencode", true);
    try {
      const status = await getOpenCodeStatus();
      setOpenCodeStatus(status);
      if (showNote)
        setNote(
          "opencode",
          status.available && status.authenticated
            ? { tone: "ready", message: `OpenCode connected with ${status.models?.length ?? 0} configured models.` }
            : {
                tone: status.available ? "warning" : "error",
                message: status.error || "Install OpenCode and configure a provider, then refresh.",
              },
        );
    } catch (error) {
      setOpenCodeStatus(null);
      if (showNote)
        setNote("opencode", {
          tone: "error",
          message: error instanceof Error ? error.message : "OpenCode status check failed.",
        });
    } finally {
      markChecked("opencode");
      setBusy("opencode", false);
    }
  }

  async function refreshPiStatus(showNote = false) {
    setBusy("pi", true);
    try {
      const status = await getPiStatus();
      setPiStatus(status);
      if (showNote)
        setNote(
          "pi",
          status.available && status.authenticated
            ? {
                tone: "ready",
                message: `Pi found ${status.provider_count ?? 0} configured provider${status.provider_count === 1 ? "" : "s"} and ${status.models?.length ?? 0} models. Sign-in is verified when a turn starts.`,
              }
            : {
                tone: status.available ? "warning" : "error",
                message: status.error || "Install Pi and use /login in its terminal, then refresh.",
              },
        );
    } catch (error) {
      setPiStatus(null);
      if (showNote)
        setNote("pi", { tone: "error", message: error instanceof Error ? error.message : "Pi status check failed." });
    } finally {
      markChecked("pi");
      setBusy("pi", false);
    }
  }

  /** Refresh one runtime's status. `showNote` reports the result inline. */
  async function refreshStatus(runtime: AccountRuntimeKind, showNote = false) {
    if (runtime === "codex") await refreshCodexAccount();
    else if (runtime === "claude") await refreshClaudeStatus(showNote);
    else if (runtime === "opencode") await refreshOpenCodeStatus(showNote);
    else await refreshPiStatus(showNote);
  }

  /** The explicit Refresh status action: status, accounts, version, binary. */
  async function refreshRuntime(runtime: AccountRuntimeKind) {
    await Promise.allSettled([
      refreshStatus(runtime, true),
      runtime === "codex" || runtime === "claude" ? refreshProfiles(runtime) : Promise.resolve(),
      refreshUpdates(),
      refreshBinaries(),
    ]);
  }

  function binaryChanged(runtime: AccountRuntimeKind) {
    void refreshBinaries();
    void refreshUpdates();
    void refreshStatus(runtime, true);
  }

  function setEnabled(runtime: AccountRuntimeKind, nextEnabled: boolean) {
    setAccountRuntimeEnabled(runtime, nextEnabled);
    if (!nextEnabled) {
      if (runtime === "codex") setCodexAccount(null);
      else if (runtime === "claude") setClaudeStatus(null);
      else if (runtime === "opencode") setOpenCodeStatus(null);
      else setPiStatus(null);
      setNote(runtime, null);
      return;
    }
    void refreshStatus(runtime);
  }

  function updatedMessage(runtime: AccountRuntimeKind, result: { updated: boolean; previous_version: string; version: string }) {
    return result.updated
      ? `${ACCOUNT_RUNTIME_LABEL[runtime]} updated from ${result.previous_version} to ${result.version}.`
      : `${ACCOUNT_RUNTIME_LABEL[runtime]} is current at ${result.version}.`;
  }

  async function runUpdate(runtime: AccountRuntimeKind) {
    if (confirmUpdate !== runtime) {
      setConfirmUpdate(runtime);
      setNote(runtime, {
        tone: "warning",
        message: `Finish active ${ACCOUNT_RUNTIME_LABEL[runtime]} turns, then click Confirm update.`,
      });
      return;
    }
    setUpdating(runtime);
    try {
      const result = await updateAccountRuntime(runtime);
      await refreshUpdates();
      await refreshStatus(runtime);
      setNote(runtime, { tone: "ready", message: updatedMessage(runtime, result) });
    } catch (error) {
      setNote(runtime, {
        tone: "error",
        message: error instanceof Error ? error.message : `${ACCOUNT_RUNTIME_LABEL[runtime]} update failed.`,
      });
    } finally {
      setConfirmUpdate(null);
      setUpdating(null);
    }
  }

  async function runAllUpdates() {
    const targets = [...updateTargets];
    if (!targets.length) return;
    if (confirmUpdate !== "all") {
      setConfirmUpdate("all");
      for (const runtime of targets) {
        setNote(runtime, {
          tone: "warning",
          message: "Finish active account-runtime turns, then click Confirm update all.",
        });
      }
      return;
    }
    setUpdating("all");
    setUpdateProgress({ current: 1, total: targets.length });
    try {
      for (const [index, runtime] of targets.entries()) {
        setUpdateProgress({ current: index + 1, total: targets.length });
        try {
          const result = await updateAccountRuntime(runtime);
          setNote(runtime, { tone: "ready", message: updatedMessage(runtime, result) });
        } catch (error) {
          setNote(runtime, {
            tone: "error",
            message: error instanceof Error ? error.message : `${ACCOUNT_RUNTIME_LABEL[runtime]} update failed.`,
          });
        }
      }
      await refreshUpdates();
      await Promise.allSettled([
        refreshCodexAccount(),
        refreshClaudeStatus(),
        refreshOpenCodeStatus(),
        refreshPiStatus(),
      ]);
    } finally {
      setConfirmUpdate(null);
      setUpdating(null);
      setUpdateProgress(null);
    }
  }

  async function connectCodex() {
    if (busy.codex) return;
    setBusy("codex", true);
    setNote("codex", { tone: "warning", message: "Starting Codex login." });
    let completed = false;
    let failed = "";
    let warning = false;
    let opened = false;
    try {
      await streamCodexDeviceLogin((ev: CodexLoginEvent) => {
        if (ev.type === "browser") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.auth_url).catch((error) => {
              setNote("codex", {
                tone: "error",
                message: `Could not open Codex login URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setNote("codex", { tone: "warning", message: "Complete Codex login in the browser, then return here." });
        } else if (ev.type === "device_code") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.verification_url).catch((error) => {
              setNote("codex", {
                tone: "error",
                message: `Could not open Codex device-code URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setNote("codex", { tone: "warning", message: `Complete Codex login with code ${ev.user_code}.` });
        } else if (ev.type === "done") {
          completed = ev.success;
          failed = ev.error ?? "";
        } else if (ev.type === "warning") {
          failed = ev.message;
          warning = true;
          setNote("codex", { tone: "warning", message: ev.message });
        } else if (ev.type === "error") {
          failed = ev.message;
        }
      });
      await refreshCodexAccount();
      warning ||= isCliPathWarningMessage(failed);
      setNote(
        "codex",
        completed
          ? { tone: "ready", message: "Codex connected. Models will appear in the picker after refresh." }
          : { tone: warning ? "warning" : "error", message: failed || "Codex login did not complete." },
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : "Codex login failed.";
      setNote("codex", { tone: isCliPathWarningMessage(message) ? "warning" : "error", message });
    } finally {
      setBusy("codex", false);
    }
  }

  async function disconnectCodex() {
    setBusy("codex", true);
    setNote("codex", null);
    try {
      await logoutCodex();
      await refreshCodexAccount();
      setNote("codex", { tone: "ready", message: "Codex disconnected." });
    } catch (error) {
      setNote("codex", {
        tone: "error",
        message: error instanceof Error ? error.message : "Codex logout failed.",
      });
    } finally {
      setBusy("codex", false);
    }
  }

  /** The CLI itself could not be found, as opposed to being signed out. */
  function missing(runtime: AccountRuntimeKind): boolean {
    if (updates[runtime]?.available === false) return true;
    if (runtime === "claude") return claudeStatus?.available === false;
    if (runtime === "opencode") return openCodeStatus?.available === false;
    if (runtime === "pi") return piStatus?.available === false;
    return false;
  }

  const ready: Record<AccountRuntimeKind, boolean> = {
    codex: Boolean(
      accountRuntimeEnabled.codex &&
        (codexAccount?.account || (codexAccount && !codexAccount.requiresOpenaiAuth)),
    ),
    claude: Boolean(accountRuntimeEnabled.claude && claudeStatus?.available && claudeStatus.authenticated),
    opencode: Boolean(accountRuntimeEnabled.opencode && openCodeStatus?.available && openCodeStatus.authenticated),
    pi: Boolean(accountRuntimeEnabled.pi && piStatus?.available && piStatus.authenticated),
  };

  function snapshot(runtime: AccountRuntimeKind): RuntimeSnapshot {
    const accountList = runtime === "codex" || runtime === "claude" ? profiles[runtime] : undefined;
    return {
      runtime,
      enabled: Boolean(accountRuntimeEnabled[runtime]),
      checked: Boolean(checked[runtime]),
      installed: !missing(runtime),
      ready: ready[runtime],
      update: updates[runtime],
      accountCount: accountList?.profiles.length,
    };
  }

  const summaries = Object.fromEntries(
    ACCOUNT_RUNTIME_KINDS.map((runtime) => [runtime, summarizeRuntime(snapshot(runtime))]),
  ) as Record<AccountRuntimeKind, RuntimeSummary>;

  return {
    enabled: accountRuntimeEnabled,
    codexAccount,
    claudeStatus,
    openCodeStatus,
    piStatus,
    busy,
    notes,
    binaries,
    updates,
    updatesLoaded,
    updateTargets,
    confirmUpdate,
    updating,
    updateProgress,
    profiles,
    profileErrors,
    ready,
    summaries,
    missing,
    refreshStatus,
    refreshRuntime,
    refreshProfiles,
    binaryChanged,
    setEnabled,
    runUpdate,
    runAllUpdates,
    connectCodex,
    disconnectCodex,
  };
}

export type AccountRuntimesState = ReturnType<typeof useAccountRuntimes>;
