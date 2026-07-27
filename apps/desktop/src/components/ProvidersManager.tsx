import { useEffect, useRef, useState } from "react";
import {
  deleteProvider,
  discoverLocalProviders,
  getAccountRuntimeUpdates,
  getClaudeStatus,
  getCodexAccount,
  getOpenCodeStatus,
  getPiStatus,
  isCliPathWarningMessage,
  isOpenRouterProvider,
  importClaudeThread,
  listClaudeThreads,
  listCodexThreads,
  listProviders,
  logoutCodex,
  openExternalUrl,
  PROVIDER_PRESETS,
  recoverCodexThread,
  saveProvider,
  streamCodexDeviceLogin,
  updateAccountRuntime,
  type AccountRuntimeUpdateStatus,
  type ClaudeThreadSummary,
  type ClaudeStatusResponse,
  type AccountRuntimeKind,
  type CodexAccountResponse,
  type CodexLoginEvent,
  type CodexThreadSummary,
  type OpenCodeStatusResponse,
  type PiStatusResponse,
  type ProviderDiscovery,
  type ProviderInfo,
  type ProviderKind,
} from "../api";
import { recoveredCodexSession, recoveredCodexSessionId } from "../lib/codexRecovery";
import { importedClaudeSession, importedClaudeSessionId } from "../lib/claudeImport";
import { isLoopbackProviderEndpoint } from "../lib/providerEndpoint.js";
import { useSessions } from "../sessions/store";
import { useSettings } from "../settings/store";
import { Plus, Refresh, Search, X } from "./icons";
import { ProviderIcon, providerBrandForProvider, type ProviderBrand } from "./ProviderIcon";
import { SheetDialog } from "./SheetDialog";
import { Select, Toggle } from "./ui";
import "./ProvidersManager.css";

type Selection = ProviderInfo | "new" | null;
type StatusTone = "ready" | "warning" | "error" | "off" | "draft";

const PROVIDER_KIND_OPTIONS: Array<{ label: string; value: ProviderKind }> = [
  { label: "OpenAI-compatible", value: "openai_compatible" },
  { label: "Anthropic Messages", value: "anthropic" },
  { label: "Gemini API", value: "gemini" },
  { label: "Replicate media", value: "replicate" },
  { label: "fal media", value: "fal" },
];

const KIND_LABEL: Record<ProviderKind, string> = {
  openai_compatible: "OpenAI-compatible",
  anthropic: "Anthropic Messages",
  gemini: "Gemini API",
  replicate: "Replicate media",
  fal: "fal media",
};

const ACCOUNT_RUNTIME_LABEL: Record<AccountRuntimeKind, string> = {
  codex: "Codex",
  claude: "Claude",
  opencode: "OpenCode",
  pi: "Pi",
};

function isMediaProvider(
  provider: Pick<ProviderInfo, "kind" | "name" | "base_url">,
): boolean {
  return (
    provider.kind === "replicate" ||
    provider.kind === "fal" ||
    isOpenRouterProvider(provider)
  );
}

function providerNeedsKey(
  provider: Pick<ProviderInfo, "kind" | "name" | "base_url">,
): boolean {
  const normalizedBase = provider.base_url
    .trim()
    .replace(/\/+$/, "")
    .toLowerCase();
  const preset = PROVIDER_PRESETS.find(
    (p) =>
      p.kind === provider.kind &&
      p.base_url.trim().replace(/\/+$/, "").toLowerCase() === normalizedBase,
  );
  if (preset) return preset.needsKey;
  return !isLoopbackProviderEndpoint(provider.base_url);
}

function providerCategory(
  provider: Pick<ProviderInfo, "kind" | "name" | "base_url">,
): string {
  if (provider.kind === "replicate" || provider.kind === "fal") return "Media";
  if (isOpenRouterProvider(provider)) return "Chat + media";
  return "Chat";
}

function providerGroup(provider: ProviderInfo): string {
  if (!provider.enabled) return "Disabled";
  if (isLoopbackProviderEndpoint(provider.base_url)) return "Local";
  if (isMediaProvider(provider)) return "Media";
  return "Hosted chat";
}

function providerStatus(provider: ProviderInfo): {
  tone: StatusTone;
  label: string;
  detail: string;
} {
  if (!provider.enabled) {
    return {
      tone: "off",
      label: "Disabled",
      detail: "Saved but unavailable to model pickers.",
    };
  }
  if (provider.error) {
    return { tone: "error", label: "Unreachable", detail: provider.error };
  }
  if (isMediaProvider(provider)) {
    if (providerNeedsKey(provider) && !provider.has_key) {
      return {
        tone: "warning",
        label: "Key needed",
        detail: "Add an API key before media workflows can use it.",
      };
    }
    return {
      tone: "ready",
      label: "Media ready",
      detail: "Credential is available for media workflows.",
    };
  }
  if (provider.models.length) {
    return {
      tone: "ready",
      label: "Connected",
      detail: `${provider.models.length} model${provider.models.length === 1 ? "" : "s"} available.`,
    };
  }
  return {
    tone: "warning",
    label: "No models",
    detail: "Saved, but no models were returned.",
  };
}

function providerKeyStatus(provider: ProviderInfo): {
  tone: StatusTone;
  label: string;
} {
  if (!providerNeedsKey(provider))
    return { tone: "ready", label: "No key needed" };
  if (provider.has_key) return { tone: "ready", label: "Key saved" };
  return { tone: "warning", label: "No key" };
}

function noteTone(note: string): StatusTone {
  if (note.startsWith("Error:")) return "error";
  if (note.startsWith("Click Delete again")) return "warning";
  if (note.includes("no models") || note.includes("No local")) return "warning";
  return "ready";
}

export function ProvidersManager({ onClose }: { onClose: () => void }) {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [sel, setSel] = useState<Selection>(null);
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ProviderKind>("openai_compatible");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [enabled, setEnabled] = useState(true);
  const [busy, setBusy] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [codexAccount, setCodexAccount] = useState<CodexAccountResponse | null>(
    null,
  );
  const [codexBusy, setCodexBusy] = useState(false);
  const [codexNote, setCodexNote] = useState<{
    tone: StatusTone;
    message: string;
  } | null>(null);
  const [importingRuntime, setImportingRuntime] = useState<"codex" | "claude" | null>(null);
  const [claudeStatus, setClaudeStatus] = useState<ClaudeStatusResponse | null>(
    null,
  );
  const [claudeBusy, setClaudeBusy] = useState(false);
  const [claudeNote, setClaudeNote] = useState<{
    tone: StatusTone;
    message: string;
  } | null>(null);
  const [openCodeStatus, setOpenCodeStatus] = useState<OpenCodeStatusResponse | null>(null);
  const [openCodeBusy, setOpenCodeBusy] = useState(false);
  const [openCodeNote, setOpenCodeNote] = useState<{ tone: StatusTone; message: string } | null>(null);
  const [piStatus, setPiStatus] = useState<PiStatusResponse | null>(null);
  const [piBusy, setPiBusy] = useState(false);
  const [piNote, setPiNote] = useState<{ tone: StatusTone; message: string } | null>(null);
  const [runtimeUpdates, setRuntimeUpdates] = useState<
    Partial<Record<AccountRuntimeKind, AccountRuntimeUpdateStatus>>
  >({});
  const [confirmRuntimeUpdate, setConfirmRuntimeUpdate] =
    useState<AccountRuntimeKind | null>(null);
  const [updatingRuntime, setUpdatingRuntime] =
    useState<AccountRuntimeKind | null>(null);
  const [discoveries, setDiscoveries] = useState<ProviderDiscovery[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const setAccountRuntimeEnabled = useSettings(
    (s) => s.setAccountRuntimeEnabled,
  );

  const refresh = () => listProviders().then(setProviders);
  useEffect(() => {
    void refresh();
    if (accountRuntimeEnabled.codex) void refreshCodexAccount();
    if (accountRuntimeEnabled.claude) void refreshClaudeStatus();
    if (accountRuntimeEnabled.opencode) void refreshOpenCodeStatus();
    if (accountRuntimeEnabled.pi) void refreshPiStatus();
    void refreshRuntimeUpdates();
  }, []);

  function updateAccountRuntimeEnabled(
    runtime: AccountRuntimeKind,
    nextEnabled: boolean,
  ) {
    setAccountRuntimeEnabled(runtime, nextEnabled);
    if (!nextEnabled) {
      if (runtime === "codex") {
        setCodexAccount(null);
        setCodexNote(null);
      } else if (runtime === "claude") {
        setClaudeStatus(null);
        setClaudeNote(null);
      } else if (runtime === "opencode") {
        setOpenCodeStatus(null);
        setOpenCodeNote(null);
      } else {
        setPiStatus(null);
        setPiNote(null);
      }
      return;
    }
    if (runtime === "codex") void refreshCodexAccount();
    else if (runtime === "claude") void refreshClaudeStatus();
    else if (runtime === "opencode") void refreshOpenCodeStatus();
    else void refreshPiStatus();
  }

  function edit(p: ProviderInfo | "new") {
    setSel(p);
    setNote(null);
    setConfirmDeleteId(null);
    if (p === "new") {
      setName("");
      setKind("openai_compatible");
      setBaseUrl("");
      setApiKey("");
      setEnabled(true);
    } else {
      setName(p.name);
      setKind(p.kind);
      setBaseUrl(p.base_url);
      setApiKey("");
      setEnabled(p.enabled);
    }
  }

  function applyPreset(presetName: string) {
    const p = PROVIDER_PRESETS.find((x) => x.name === presetName);
    if (!p) return;
    setName(p.name);
    setKind(p.kind);
    setBaseUrl(p.base_url);
  }

  function startPreset(presetName: string) {
    edit("new");
    applyPreset(presetName);
  }

  async function refreshCodexAccount() {
    try {
      setCodexAccount(await getCodexAccount(false));
    } catch {
      setCodexAccount(null);
    }
  }

  async function refreshClaudeStatus(showNote = false) {
    setClaudeBusy(true);
    try {
      const status = await getClaudeStatus();
      setClaudeStatus(status);
      if (showNote) {
        const message =
          status.error ||
          "Run `claude auth login` in a terminal, then refresh.";
        const warning =
          Boolean(status.warning) || isCliPathWarningMessage(message);
        setClaudeNote(
          status.available && status.authenticated
            ? {
                tone: "ready",
                message:
                  "Installed Claude CLI connected. Models will appear in the picker after refresh.",
              }
            : {
                tone: status.available || warning ? "warning" : "error",
                message,
              },
        );
      }
    } catch (error) {
      setClaudeStatus(null);
      if (showNote) {
        const message =
          error instanceof Error
            ? error.message
            : "Claude CLI status check failed.";
        setClaudeNote({
          tone: isCliPathWarningMessage(message) ? "warning" : "error",
          message,
        });
      }
    } finally {
      setClaudeBusy(false);
    }
  }

  async function refreshOpenCodeStatus(showNote = false) {
    setOpenCodeBusy(true);
    try {
      const status = await getOpenCodeStatus();
      setOpenCodeStatus(status);
      if (showNote) setOpenCodeNote(status.available && status.authenticated
        ? { tone: "ready", message: `OpenCode connected with ${status.models?.length ?? 0} configured models.` }
        : { tone: status.available ? "warning" : "error", message: status.error || "Install OpenCode and configure a provider, then refresh." });
    } catch (error) {
      setOpenCodeStatus(null);
      if (showNote) setOpenCodeNote({ tone: "error", message: error instanceof Error ? error.message : "OpenCode status check failed." });
    } finally {
      setOpenCodeBusy(false);
    }
  }

  async function refreshPiStatus(showNote = false) {
    setPiBusy(true);
    try {
      const status = await getPiStatus();
      setPiStatus(status);
      if (showNote) setPiNote(status.available && status.authenticated
        ? { tone: "ready", message: `Pi connected with ${status.provider_count ?? 0} provider${status.provider_count === 1 ? "" : "s"} and ${status.models?.length ?? 0} available models.` }
        : { tone: status.available ? "warning" : "error", message: status.error || "Install Pi and use /login in its terminal, then refresh." });
    } catch (error) {
      setPiStatus(null);
      if (showNote) setPiNote({ tone: "error", message: error instanceof Error ? error.message : "Pi status check failed." });
    } finally {
      setPiBusy(false);
    }
  }

  async function refreshRuntimeUpdates() {
    try {
      setRuntimeUpdates((await getAccountRuntimeUpdates()).runtimes);
    } catch {
      setRuntimeUpdates({});
    }
  }

  function setRuntimeNote(
    runtime: AccountRuntimeKind,
    next: { tone: StatusTone; message: string },
  ) {
    if (runtime === "codex") setCodexNote(next);
    else if (runtime === "claude") setClaudeNote(next);
    else if (runtime === "opencode") setOpenCodeNote(next);
    else setPiNote(next);
  }

  async function runRuntimeUpdate(runtime: AccountRuntimeKind) {
    if (confirmRuntimeUpdate !== runtime) {
      setConfirmRuntimeUpdate(runtime);
      setRuntimeNote(runtime, {
        tone: "warning",
        message: `Finish active ${ACCOUNT_RUNTIME_LABEL[runtime]} turns, then click Confirm update.`,
      });
      return;
    }
    setUpdatingRuntime(runtime);
    try {
      const result = await updateAccountRuntime(runtime);
      await refreshRuntimeUpdates();
      if (runtime === "codex") await refreshCodexAccount();
      else if (runtime === "claude") await refreshClaudeStatus();
      else if (runtime === "opencode") await refreshOpenCodeStatus();
      else await refreshPiStatus();
      setRuntimeNote(runtime, {
        tone: "ready",
        message: result.updated
          ? `${ACCOUNT_RUNTIME_LABEL[runtime]} updated from ${result.previous_version} to ${result.version}.`
          : `${ACCOUNT_RUNTIME_LABEL[runtime]} is current at ${result.version}.`,
      });
    } catch (error) {
      setRuntimeNote(runtime, {
        tone: "error",
        message:
          error instanceof Error
            ? error.message
            : `${ACCOUNT_RUNTIME_LABEL[runtime]} update failed.`,
      });
    } finally {
      setConfirmRuntimeUpdate(null);
      setUpdatingRuntime(null);
    }
  }

  function runtimeVersion(runtime: AccountRuntimeKind): string {
    const version = runtimeUpdates[runtime]?.version?.trim();
    return version ? ` · v${version}` : "";
  }

  function runtimeUpdateButton(runtime: AccountRuntimeKind, available: boolean) {
    const updating = updatingRuntime === runtime;
    const confirming = confirmRuntimeUpdate === runtime;
    return (
      <button
        className="btn-ghost"
        data-testid={`${runtime}-update`}
        type="button"
        onClick={() => void runRuntimeUpdate(runtime)}
        disabled={
          updatingRuntime !== null ||
          !accountRuntimeEnabled[runtime] ||
          !(runtimeUpdates[runtime]?.available ?? available)
        }
      >
        {updating ? "Updating..." : confirming ? "Confirm update" : "Update"}
      </button>
    );
  }

  async function detectLocal() {
    setDetecting(true);
    setNote(null);
    const found = await discoverLocalProviders();
    setDiscoveries(found);
    setDetecting(false);
    if (found.length === 0) {
      setNote(
        "No local provider probes completed. Check that the desktop backend is running.",
      );
    }
  }

  async function addDiscovery(discovery: ProviderDiscovery) {
    setBusy(true);
    setNote(null);
    setConfirmDeleteId(null);
    const saved = await saveProvider({
      name: discovery.name,
      kind: discovery.kind,
      base_url: discovery.base_url,
      enabled: true,
    });
    setBusy(false);
    if (!saved) {
      setNote(`Error: Failed to add ${discovery.name}.`);
      return;
    }
    await refresh();
    await detectLocal();
    setSel(saved);
    setName(saved.name);
    setKind(saved.kind);
    setBaseUrl(saved.base_url);
    setApiKey("");
    setEnabled(saved.enabled);
    setNote(
      saved.models.length
        ? `Connected - ${saved.models.length} models available`
        : saved.error
          ? `Error: Couldn't reach provider: ${saved.error}`
          : "Saved, but no models returned - check the local server.",
    );
  }

  async function persistProvider(action: "save" | "test") {
    if (!name.trim() || !baseUrl.trim()) return;
    setBusy(true);
    setNote(null);
    setConfirmDeleteId(null);
    const id = sel && sel !== "new" ? sel.id : undefined;
    const saved = await saveProvider({
      id,
      name: name.trim(),
      kind,
      base_url: baseUrl.trim(),
      api_key: apiKey || undefined,
      enabled,
    });
    setBusy(false);
    if (!saved) {
      setNote("Error: Failed to save provider.");
      return;
    }
    await refresh();
    setSel(saved);
    setName(saved.name);
    setKind(saved.kind);
    setBaseUrl(saved.base_url);
    setApiKey("");
    setEnabled(saved.enabled);
    setNote(
      isMediaProvider(saved)
        ? action === "test"
          ? "Media credential checked. Image/video workflows can use this encrypted credential when those surfaces are enabled."
          : "Media provider saved. Image/video generation workflows can use this encrypted credential when those surfaces are enabled."
        : saved.models.length
          ? `Connected - ${saved.models.length} models available`
          : saved.error
            ? `Error: Couldn't reach provider: ${saved.error}`
            : "Saved, but no models returned - check the URL/key.",
    );
  }

  async function save() {
    await persistProvider("save");
  }

  async function testConnection() {
    await persistProvider("test");
  }

  async function connectCodex() {
    if (codexBusy) return;
    setCodexBusy(true);
    setCodexNote({ tone: "warning", message: "Starting Codex login." });
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
              setCodexNote({
                tone: "error",
                message: `Could not open Codex login URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setCodexNote({
            tone: "warning",
            message: "Complete Codex login in the browser, then return here.",
          });
        } else if (ev.type === "device_code") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.verification_url).catch((error) => {
              setCodexNote({
                tone: "error",
                message: `Could not open Codex device-code URL: ${error instanceof Error ? error.message : String(error)}`,
              });
            });
          }
          setCodexNote({
            tone: "warning",
            message: `Complete Codex login with code ${ev.user_code}.`,
          });
        } else if (ev.type === "done") {
          completed = ev.success;
          failed = ev.error ?? "";
        } else if (ev.type === "warning") {
          failed = ev.message;
          warning = true;
          setCodexNote({ tone: "warning", message: ev.message });
        } else if (ev.type === "error") {
          failed = ev.message;
        }
      });
      await refreshCodexAccount();
      warning ||= isCliPathWarningMessage(failed);
      setCodexNote(
        completed
          ? {
              tone: "ready",
              message:
                "Codex connected. Models will appear in the picker after refresh.",
            }
          : {
              tone: warning ? "warning" : "error",
              message: failed || "Codex login did not complete.",
            },
      );
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Codex login failed.";
      setCodexNote({
        tone: isCliPathWarningMessage(message) ? "warning" : "error",
        message,
      });
    } finally {
      setCodexBusy(false);
    }
  }

  async function disconnectCodex() {
    setCodexBusy(true);
    setCodexNote(null);
    try {
      await logoutCodex();
      await refreshCodexAccount();
      setCodexNote({ tone: "ready", message: "Codex disconnected." });
    } catch (error) {
      setCodexNote({
        tone: "error",
        message:
          error instanceof Error ? error.message : "Codex logout failed.",
      });
    } finally {
      setCodexBusy(false);
    }
  }

  async function remove() {
    if (!sel || sel === "new") return;
    if (confirmDeleteId !== sel.id) {
      setConfirmDeleteId(sel.id);
      setNote(`Click Delete again to remove "${sel.name}".`);
      return;
    }
    await deleteProvider(sel.id);
    await refresh();
    setConfirmDeleteId(null);
    setNote(null);
    setSel(null);
  }

  const codexReady = Boolean(
    accountRuntimeEnabled.codex &&
      (codexAccount?.account ||
        (codexAccount && !codexAccount.requiresOpenaiAuth)),
  );
  const claudeReady = Boolean(
    accountRuntimeEnabled.claude &&
      claudeStatus?.available &&
      claudeStatus.authenticated,
  );
  const openCodeReady = Boolean(
    accountRuntimeEnabled.opencode &&
      openCodeStatus?.available &&
      openCodeStatus.authenticated,
  );
  const piReady = Boolean(
    accountRuntimeEnabled.pi && piStatus?.available && piStatus.authenticated,
  );
  const claudeAccountLabel =
    claudeStatus?.auth?.email ?? claudeStatus?.auth?.subscriptionType ?? null;
  const selectedProvider = sel && sel !== "new" ? sel : null;
  const selectedPreset = PROVIDER_PRESETS.find(
    (p) =>
      p.kind === kind &&
      p.name === name &&
      p.base_url.trim().replace(/\/+$/, "") ===
        baseUrl.trim().replace(/\/+$/, ""),
  );
  const draftProvider = { name, kind, base_url: baseUrl };
  const draftNeedsKey = providerNeedsKey(draftProvider);
  const draftCategory = providerCategory(draftProvider);
  const hasDraftFields = Boolean(
    name.trim() || baseUrl.trim() || apiKey.trim(),
  );
  const isDirty =
    sel === "new"
      ? hasDraftFields
      : Boolean(
          selectedProvider &&
          (name !== selectedProvider.name ||
            kind !== selectedProvider.kind ||
            baseUrl !== selectedProvider.base_url ||
            enabled !== selectedProvider.enabled ||
            apiKey.length > 0),
        );
  const canSave = Boolean(name.trim() && baseUrl.trim() && !busy);
  const saveStateLabel =
    sel === "new" ? "Draft provider" : isDirty ? "Unsaved changes" : "Saved";
  const selectedStatus = selectedProvider
    ? providerStatus(selectedProvider)
    : null;
  const selectedKeyStatus = selectedProvider
    ? providerKeyStatus(selectedProvider)
    : null;
  const providerGroups = ["Local", "Hosted chat", "Media", "Disabled"]
    .map((label) => ({
      label,
      items: providers.filter((provider) => providerGroup(provider) === label),
    }))
    .filter((group) => group.items.length > 0);
  if (importingRuntime) {
    return (
      <AccountRuntimeImportDialog
        runtime={importingRuntime}
        onClose={() => setImportingRuntime(null)}
        onOpenSession={onClose}
      />
    );
  }
  return (
    <SheetDialog
      title="Providers"
      className="sheet agents-sheet providers-sheet"
      onClose={onClose}
    >
      <div className="sheet-header providers-header">
        <div className="providers-title">
          <h2>Connection Center</h2>
          <p className="sheet-sub providers-subtitle">
            Connect chat, media, local, Codex, and bring-your-own Claude CLI
            runtimes. Provider keys stay encrypted on this device.
          </p>
        </div>
        <div className="providers-header-actions">
          <button
            className="btn-accent providers-add-button"
            data-testid="new-provider"
            type="button"
            onClick={() => edit("new")}
          >
            <Plus size={14} />
            <span>Add provider</span>
          </button>
          <button
            className="icon-btn sheet-close providers-close"
            data-testid="close-providers"
            type="button"
            onClick={onClose}
            title="Close"
            aria-label="Close providers"
          >
            <X size={16} />
          </button>
        </div>
      </div>

      <div className="agents-body providers-body">
        <aside className="providers-rail" aria-label="Provider connections">
          <div className="providers-list" role="list">
            <button
              className={"provider-list-row" + (!sel ? " active" : "")}
              data-testid="provider-overview"
              type="button"
              onClick={() => {
                setSel(null);
                setNote(null);
                setConfirmDeleteId(null);
              }}
            >
              <span className="provider-row-top">
                <span className="provider-row-name">Overview</span>
              </span>
              <span className="provider-row-meta">Accounts and setup</span>
            </button>
            {providerGroups.map((group) => (
              <div className="provider-group" key={group.label}>
                <span className="provider-group-label">{group.label}</span>
                {group.items.map((p) => {
                  const status = providerStatus(p);
                  const keyStatus = providerKeyStatus(p);
                  return (
                    <button
                      key={p.id}
                      className={
                        "provider-list-row" +
                        (selectedProvider?.id === p.id ? " active" : "")
                      }
                      type="button"
                      onClick={() => edit(p)}
                    >
                      <span className="provider-row-top">
                        <span
                          className={"provider-status-dot " + status.tone}
                        />
                        <ProviderIcon brand={providerBrandForProvider(p)} size={16} />
                        <span className="provider-row-name">{p.name}</span>
                      </span>
                      <span className="provider-row-meta">
                        <span>{providerCategory(p)}</span>
                        <span>{KIND_LABEL[p.kind]}</span>
                      </span>
                      <span className="provider-row-foot">
                        <span
                          className={"provider-key-badge " + keyStatus.tone}
                        >
                          {keyStatus.label}
                        </span>
                        <span className={"provider-ready-label " + status.tone}>
                          {status.label}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            ))}
            {providers.length === 0 && (
              <div className="providers-list-empty">
                <span>No providers</span>
              </div>
            )}
          </div>
        </aside>

        <main className="providers-detail">
          <section
            className="provider-account-panel"
            aria-labelledby="provider-account-title"
            hidden={Boolean(sel)}
          >
            <div className="providers-section-head">
              <h4 id="provider-account-title">Account runtimes</h4>
              <p>
                Codex, the installed Claude CLI, OpenCode, and Pi use their
                own desktop tooling. Disable one to hide its models and block
                new runs without signing out.
              </p>
              <p>
                Milim does not include Claude Code, provide Anthropic
                credentials, or manage Claude credentials. It only invokes the
                official Claude CLI installed and authenticated separately on
                this machine.
              </p>
            </div>
            <div className="provider-account-grid">
              <div
                className={
                  "provider-account-card " + (codexReady ? "ready" : "off")
                }
              >
                <div className="provider-account-main">
                  <span
                    className={
                      "provider-status-dot " + (codexReady ? "ready" : "off")
                    }
                  />
                  <ProviderIcon brand="codex" size={24} />
                  <div>
                    <strong>Codex</strong>
                    <span>
                      {!accountRuntimeEnabled.codex
                        ? "Disabled"
                        : `${codexAccount?.account?.email ?? "ChatGPT account runtime"}${runtimeVersion("codex")}`}
                    </span>
                  </div>
                </div>
                <div className="provider-account-actions">
                  <Toggle
                    checked={accountRuntimeEnabled.codex}
                    onChange={(enabled) =>
                      updateAccountRuntimeEnabled("codex", enabled)
                    }
                    ariaLabel="Enable Codex runtime"
                    testId="codex-enabled-toggle"
                  />
                  {codexReady && (
                    <button
                      className="btn-ghost"
                      data-testid="codex-import-chats"
                      type="button"
                      onClick={() => setImportingRuntime("codex")}
                    >
                      Import chats
                    </button>
                  )}
                  {runtimeUpdateButton("codex", codexReady)}
                  <button
                    className="btn-ghost"
                    data-testid="codex-connect"
                    type="button"
                    onClick={() =>
                      void (codexReady ? disconnectCodex() : connectCodex())
                    }
                    disabled={codexBusy || !accountRuntimeEnabled.codex}
                  >
                    {codexBusy
                      ? "Working..."
                      : codexReady
                        ? "Disconnect"
                        : "Connect"}
                  </button>
                </div>
              </div>
              <div className={"provider-account-card " + (openCodeReady ? "ready" : "off")}>
                <div className="provider-account-main">
                  <span className={"provider-status-dot " + (openCodeReady ? "ready" : "off")} />
                  <ProviderIcon brand="opencode" size={24} />
                  <div>
                    <strong>Installed OpenCode CLI</strong>
                    <span>{!accountRuntimeEnabled.opencode
                      ? "Disabled"
                      : openCodeStatus?.available
                      ? `${openCodeStatus.models?.length ?? 0} configured model${openCodeStatus.models?.length === 1 ? "" : "s"}${runtimeVersion("opencode")}`
                      : "Install OpenCode separately and configure its providers."}</span>
                  </div>
                </div>
                <div className="provider-account-actions">
                  <Toggle
                    checked={accountRuntimeEnabled.opencode}
                    onChange={(enabled) =>
                      updateAccountRuntimeEnabled("opencode", enabled)
                    }
                    ariaLabel="Enable OpenCode runtime"
                    testId="opencode-enabled-toggle"
                  />
                  {runtimeUpdateButton("opencode", Boolean(openCodeStatus?.available))}
                  <button className="btn-ghost" type="button" onClick={() => void refreshOpenCodeStatus(true)} disabled={openCodeBusy || !accountRuntimeEnabled.opencode}>
                    {openCodeBusy ? "Checking..." : "Refresh"}
                  </button>
                </div>
              </div>
              <div
                className={
                  "provider-account-card " + (claudeReady ? "ready" : "off")
                }
              >
                <div className="provider-account-main">
                  <span
                    className={
                      "provider-status-dot " + (claudeReady ? "ready" : "off")
                    }
                  />
                  <ProviderIcon brand="claude" size={24} />
                  <div>
                    <strong>Installed Claude CLI</strong>
                    <span>
                      {!accountRuntimeEnabled.claude
                        ? "Disabled"
                        : `${claudeAccountLabel ??
                            (claudeStatus?.available
                              ? "Run `claude auth login`, then refresh."
                              : "Install Anthropic's official Claude CLI separately.")}${runtimeVersion("claude")}`}
                    </span>
                  </div>
                </div>
                <div className="provider-account-actions">
                  <Toggle
                    checked={accountRuntimeEnabled.claude}
                    onChange={(enabled) =>
                      updateAccountRuntimeEnabled("claude", enabled)
                    }
                    ariaLabel="Enable Claude runtime"
                    testId="claude-enabled-toggle"
                  />
                  {accountRuntimeEnabled.claude && (
                    <button
                      className="btn-ghost"
                      data-testid="claude-import-chats"
                      type="button"
                      onClick={() => setImportingRuntime("claude")}
                    >
                      Import chats
                    </button>
                  )}
                  {runtimeUpdateButton("claude", Boolean(claudeStatus?.available))}
                  <button
                    className="btn-ghost"
                    data-testid="claude-code-status"
                    type="button"
                    onClick={() => void refreshClaudeStatus(true)}
                    disabled={claudeBusy || !accountRuntimeEnabled.claude}
                  >
                    {claudeBusy ? "Checking..." : "Refresh"}
                  </button>
                </div>
              </div>
              <div className={"provider-account-card " + (piReady ? "ready" : "off")}>
                <div className="provider-account-main">
                  <span className={"provider-status-dot " + (piReady ? "ready" : "off")} />
                  <ProviderIcon brand="pi" size={24} />
                  <div>
                    <strong>Installed Pi CLI</strong>
                    <span>{!accountRuntimeEnabled.pi
                      ? "Disabled"
                      : piStatus?.available
                      ? `${piStatus.provider_count ?? 0} provider${piStatus.provider_count === 1 ? "" : "s"} / ${piStatus.models?.length ?? 0} model${piStatus.models?.length === 1 ? "" : "s"}${runtimeVersion("pi")}`
                      : "Install Pi separately and use /login in its terminal."}</span>
                  </div>
                </div>
                <div className="provider-account-actions">
                  <Toggle
                    checked={accountRuntimeEnabled.pi}
                    onChange={(enabled) =>
                      updateAccountRuntimeEnabled("pi", enabled)
                    }
                    ariaLabel="Enable Pi runtime"
                    testId="pi-enabled-toggle"
                  />
                  {runtimeUpdateButton("pi", Boolean(piStatus?.available))}
                  <button className="btn-ghost" data-testid="pi-status" type="button" onClick={() => void refreshPiStatus(true)} disabled={piBusy || !accountRuntimeEnabled.pi}>
                    {piBusy ? "Checking..." : "Refresh"}
                  </button>
                </div>
              </div>
            </div>
            {codexNote && (
              <p className={"provider-note " + codexNote.tone}>
                {codexNote.message}
              </p>
            )}
            {claudeNote && (
              <p className={"provider-note " + claudeNote.tone}>
                {claudeNote.message}
              </p>
            )}
            {openCodeNote && <p className={"provider-note " + openCodeNote.tone}>{openCodeNote.message}</p>}
            {piNote && <p className={"provider-note " + piNote.tone}>{piNote.message}</p>}
          </section>

          <section
            className="provider-quick-panel"
            aria-labelledby="provider-quick-title"
            hidden={Boolean(sel)}
          >
            <div className="providers-section-head">
              <h4 id="provider-quick-title">Add providers</h4>
              <p>Detect local runtimes or start a hosted API-key connection.</p>
            </div>
            <div className="provider-quick-grid">
              <button
                className="provider-quick-action"
                type="button"
                onClick={() => startPreset("OpenRouter")}
                title="Start OpenRouter provider setup."
                aria-label="Start OpenRouter provider setup"
              >
                <ProviderIcon brand="openrouter" size={18} />
                <strong>OpenRouter</strong>
                <span>Endpoint and encrypted key.</span>
              </button>
              <button
                className="provider-quick-action"
                data-testid="detect-local-providers"
                type="button"
                onClick={detectLocal}
                disabled={detecting}
                title="Find Ollama or LM Studio on this machine."
                aria-label={
                  detecting
                    ? "Detecting local providers"
                    : "Detect local providers"
                }
              >
                <Search size={15} />
                <strong>
                  {detecting ? "Detecting local" : "Detect local"}
                </strong>
                <span>Find Ollama or LM Studio.</span>
              </button>
              {([
                ["OpenAI", "openai"],
                ["Anthropic", "claude"],
                ["Gemini", "gemini"],
              ] satisfies Array<[string, ProviderBrand]>).map(([presetName, brand]) => (
                <button
                  className="provider-quick-action"
                  type="button"
                  key={presetName}
                  onClick={() => startPreset(presetName)}
                  title={`Start ${presetName} provider setup.`}
                  aria-label={`Start ${presetName} provider setup`}
                >
                  <ProviderIcon brand={brand} size={18} />
                  <strong>{presetName}</strong>
                  <span>Endpoint and encrypted key.</span>
                </button>
              ))}
            </div>
          </section>

          {discoveries.length > 0 && (
            <div className="provider-discovery" hidden={Boolean(sel)}>
              <div className="provider-discovery-head">
                <span className="setting-mini-title">Local providers</span>
                <span>
                  Reachable endpoints can be added without pasting a key.
                </span>
              </div>
              {discoveries.map((d) => (
                <div className="provider-discovery-row" key={d.base_url}>
                  <ProviderIcon brand={providerBrandForProvider(d)} size={16} />
                  <div>
                    <strong>{d.name}</strong>
                    <span>
                      {d.reachable
                        ? `${d.models.length} model${d.models.length === 1 ? "" : "s"} found at ${d.base_url}`
                        : d.error
                          ? "Not running"
                          : "No response"}
                    </span>
                  </div>
                  {d.configured ? (
                    <span className="provider-pill ready">Added</span>
                  ) : d.reachable ? (
                    <button
                      className="btn-ghost"
                      type="button"
                      disabled={busy}
                      onClick={() => void addDiscovery(d)}
                    >
                      Add
                    </button>
                  ) : (
                    <span className="provider-pill muted">Start app</span>
                  )}
                </div>
              ))}
            </div>
          )}

          {!sel && note && (
            <p className={"provider-note " + noteTone(note)}>{note}</p>
          )}

          {sel && (
            <>
              <div className="providers-detail-head">
                <div>
                  <span className="providers-detail-kicker">
                    {sel === "new" ? "New connection" : "Provider connection"}
                  </span>
                  <h3>{name.trim() || "Untitled provider"}</h3>
                  <p>
                    {sel === "new"
                      ? "Choose a preset or enter the endpoint details manually."
                      : (selectedStatus?.detail ??
                        "Review readiness, credentials, and endpoint details.")}
                  </p>
                </div>
                <span
                  className={
                    "provider-save-state " +
                    (isDirty || sel === "new" ? "draft" : "ready")
                  }
                >
                  {saveStateLabel}
                </span>
              </div>

              <section className="providers-section">
                <div className="providers-section-head">
                  <h4>Connection</h4>
                  <p>
                    Pick a known profile or enter any OpenAI-compatible endpoint
                    manually.
                  </p>
                </div>
                <div className="providers-field-grid three">
                  <label className="field provider-field">
                    <span>Provider preset</span>
                    <Select
                      value={selectedPreset?.name ?? ""}
                      testId="provider-preset-select"
                      placeholder="Choose a preset..."
                      options={PROVIDER_PRESETS.map((p) => ({
                        label: p.name,
                        value: p.name,
                      }))}
                      onChange={applyPreset}
                    />
                  </label>
                  <label className="field provider-field">
                    <span>Name</span>
                    <input
                      className="css-input"
                      data-testid="provider-name-input"
                      value={name}
                      onChange={(e) => setName(e.target.value)}
                      placeholder="OpenAI"
                    />
                  </label>
                  <label className="field provider-field">
                    <span>Provider type</span>
                    <Select
                      value={kind}
                      testId="provider-kind-select"
                      options={PROVIDER_KIND_OPTIONS}
                      onChange={(v) => setKind(v as ProviderKind)}
                    />
                  </label>
                </div>
                <label className="field provider-field">
                  <span>Base URL</span>
                  <input
                    className="css-input"
                    data-testid="provider-base-url-input"
                    value={baseUrl}
                    onChange={(e) => setBaseUrl(e.target.value)}
                    placeholder="https://api.openai.com/v1"
                  />
                </label>
              </section>

              <section className="providers-section">
                <div className="providers-section-head">
                  <h4>Credentials</h4>
                  <p>
                    {sel !== "new"
                      ? "Leave the key blank to keep the encrypted value already stored on this device."
                      : draftNeedsKey
                        ? "Hosted providers usually require a key before models or media jobs can run."
                        : "Local endpoints can usually be saved without an API key."}
                  </p>
                </div>
                <label className="field provider-field">
                  <span>
                    API key {sel !== "new" && <em>(leave blank to keep)</em>}
                  </span>
                  <input
                    className="css-input"
                    data-testid="provider-api-key-input"
                    type="password"
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder="sk-..."
                  />
                </label>
              </section>

              <section className="providers-section">
                <div className="providers-section-head">
                  <h4>Models and diagnostics</h4>
                  <p>
                    Refresh models after changing credentials, endpoint, or
                    local server state.
                  </p>
                </div>
                <div className="provider-capability-grid">
                  <div>
                    <span>Category</span>
                    <strong>{draftCategory}</strong>
                  </div>
                  <div>
                    <span>Credential</span>
                    <strong>
                      {selectedKeyStatus?.label ??
                        (draftNeedsKey ? "Add key" : "No key needed")}
                    </strong>
                  </div>
                  <div>
                    <span>Models</span>
                    <strong>
                      {selectedProvider
                        ? selectedProvider.models.length
                          ? String(selectedProvider.models.length)
                          : "None returned"
                        : "After save"}
                    </strong>
                  </div>
                  <div>
                    <span>Readiness</span>
                    <strong>{selectedStatus?.label ?? "Draft"}</strong>
                  </div>
                </div>
                <div className="provider-enabled-row">
                  <Toggle
                    checked={enabled}
                    onChange={setEnabled}
                    label="Enabled"
                    testId="provider-enabled-toggle"
                  />
                  <span>
                    Disabled providers remain saved but are hidden from active
                    workflows.
                  </span>
                </div>
                {selectedProvider?.models.length ? (
                  <div
                    className="provider-model-list"
                    aria-label="Provider models"
                  >
                    {selectedProvider.models.slice(0, 8).map((model) => (
                      <span key={model}>{model}</span>
                    ))}
                  </div>
                ) : (
                  <p className="provider-section-note">
                    No cached models yet. Save and test the connection to
                    populate this provider.
                  </p>
                )}
                <div className="provider-diagnostics-row">
                  <button
                    className="btn-ghost"
                    data-testid="provider-test-connection"
                    type="button"
                    disabled={!canSave}
                    onClick={() => void testConnection()}
                  >
                    <Refresh size={13} />
                    <span>{busy ? "Testing..." : "Test connection"}</span>
                  </button>
                  <button
                    className="btn-ghost"
                    type="button"
                    onClick={() => void refresh()}
                  >
                    Refresh list
                  </button>
                </div>
              </section>

              {note && (
                <p className={"provider-note " + noteTone(note)}>{note}</p>
              )}

              <div className="providers-footer-actions">
                {sel !== "new" && (
                  <button
                    className="btn-ghost danger provider-delete-action"
                    type="button"
                    disabled={busy}
                    onClick={remove}
                  >
                    {confirmDeleteId === selectedProvider?.id
                      ? "Confirm delete"
                      : "Delete"}
                  </button>
                )}
                <span className="spacer" />
                <button
                  className="btn-accent"
                  data-testid="save-provider"
                  type="button"
                  disabled={!canSave}
                  onClick={save}
                >
                  {busy
                    ? "Connecting..."
                    : sel === "new"
                      ? "Save and test"
                      : isDirty
                        ? "Save changes"
                        : "Saved"}
                </button>
              </div>
            </>
          )}
        </main>
      </div>
    </SheetDialog>
  );
}

interface RuntimeImportThread {
  id: string;
  title: string;
  cwd?: string | null;
  updatedAt: number;
  fallbackMeta: string;
  resumable: boolean;
}

function codexImportThread(thread: CodexThreadSummary): RuntimeImportThread {
  return {
    id: thread.id,
    title: thread.name?.trim() || thread.preview.trim() || "Untitled Codex chat",
    cwd: thread.cwd,
    updatedAt: thread.updated_at_ms,
    fallbackMeta: thread.model_provider,
    resumable: true,
  };
}

function claudeImportThread(thread: ClaudeThreadSummary): RuntimeImportThread {
  return {
    id: thread.id,
    title: thread.title.trim() || thread.preview.trim() || "Untitled Claude chat",
    cwd: thread.cwd,
    updatedAt: thread.updated_at_ms,
    fallbackMeta: "Claude CLI",
    resumable: thread.resumable,
  };
}

function AccountRuntimeImportDialog({
  runtime,
  onClose,
  onOpenSession,
}: {
  runtime: "codex" | "claude";
  onClose: () => void;
  onOpenSession: () => void;
}) {
  const sessions = useSessions((state) => state.sessions);
  const requestId = useRef(0);
  const [threads, setThreads] = useState<RuntimeImportThread[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [archived, setArchived] = useState(false);
  const [busy, setBusy] = useState(false);
  const [importingId, setImportingId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const runtimeLabel = runtime === "codex" ? "Codex" : "Claude";

  async function load(reset: boolean) {
    const currentRequest = ++requestId.current;
    setBusy(true);
    setError("");
    try {
      const page = runtime === "codex"
        ? await listCodexThreads({
            cursor: reset ? undefined : cursor ?? undefined,
            search,
            archived,
          })
        : await listClaudeThreads({
            cursor: reset ? undefined : cursor ?? undefined,
            search,
          });
      if (currentRequest !== requestId.current) return;
      const next = runtime === "codex"
        ? page.data.map((thread) => codexImportThread(thread as CodexThreadSummary))
        : page.data.map((thread) => claudeImportThread(thread as ClaudeThreadSummary));
      setThreads((current) => reset
        ? next
        : [...current, ...next.filter((thread) => !current.some((item) => item.id === thread.id))]);
      setCursor(page.next_cursor ?? null);
    } catch (error) {
      if (currentRequest === requestId.current)
        setError(error instanceof Error ? error.message : `${runtimeLabel} chat import failed.`);
    } finally {
      if (currentRequest === requestId.current) setBusy(false);
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => void load(true), 250);
    return () => window.clearTimeout(timer);
  }, [search, archived, runtime]);

  function openSession(id: string) {
    useSessions.getState().switchTo(id);
    onOpenSession();
  }

  function importedSessionId(threadId: string) {
    return runtime === "codex"
      ? recoveredCodexSessionId(useSessions.getState().sessions, threadId)
      : importedClaudeSessionId(useSessions.getState().sessions, threadId);
  }

  async function importThread(thread: RuntimeImportThread) {
    const existing = importedSessionId(thread.id);
    if (existing) {
      openSession(existing);
      return;
    }
    setImportingId(thread.id);
    setError("");
    try {
      const store = useSessions.getState();
      let sessionId: string | null;
      let nativeId: string;
      let resumable = true;
      if (runtime === "codex") {
        const source = await recoverCodexThread(thread.id);
        sessionId = store.importSession(recoveredCodexSession(source));
        nativeId = source.id;
      } else {
        const source = await importClaudeThread(thread.id);
        sessionId = store.importSession(importedClaudeSession(source));
        nativeId = source.id;
        resumable = source.resumable;
      }
      if (!sessionId) throw new Error("Milim could not import the selected chat.");
      const importedSession = useSessions.getState().sessions.find((session) => session.id === sessionId);
      const lastMessageId = importedSession?.messages[importedSession.messages.length - 1]?.id;
      if (!lastMessageId) throw new Error("The imported chat did not contain a sync cursor.");
      if (runtime === "codex") {
        useSessions.getState().setAccountRuntime(sessionId, {
          codexThreadId: nativeId,
          codexLastSyncedMessageId: lastMessageId,
        });
      } else {
        useSessions.getState().setAccountRuntime(sessionId, {
          claudeSessionId: nativeId,
          claudeLastSyncedMessageId: resumable ? lastMessageId : undefined,
        });
      }
      openSession(sessionId);
    } catch (error) {
      setError(error instanceof Error ? error.message : `${runtimeLabel} chat import failed.`);
    } finally {
      setImportingId(null);
    }
  }

  return (
    <SheetDialog title={`Import ${runtimeLabel} chats`} className="sheet codex-recovery-sheet" onClose={onClose}>
      <div className="sheet-header providers-header">
        <div className="providers-title">
          <h2>Import {runtimeLabel} chats</h2>
          <p className="sheet-sub">Choose a chat to import into Milim.</p>
        </div>
        <button className="icon-btn sheet-close" type="button" onClick={onClose} aria-label="Close import">
          <X size={16} />
        </button>
      </div>
      <div className="codex-recovery-controls">
        <label>
          <Search size={13} aria-hidden="true" />
          <input
            type="search"
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            placeholder={`Search ${runtimeLabel} chats`}
            aria-label={`Search ${runtimeLabel} chats`}
          />
        </label>
        {runtime === "codex" && (
          <div className="codex-recovery-tabs" role="group" aria-label="Codex chat archive filter">
            <button type="button" className={!archived ? "active" : ""} onClick={() => setArchived(false)}>Active</button>
            <button type="button" className={archived ? "active" : ""} onClick={() => setArchived(true)}>Archived</button>
          </div>
        )}
      </div>
      {error && <p className="provider-note error" role="alert">{error}</p>}
      <div className="codex-recovery-list" aria-busy={busy}>
        {threads.map((thread) => {
          const existing = runtime === "codex"
            ? recoveredCodexSessionId(sessions, thread.id)
            : importedClaudeSessionId(sessions, thread.id);
          return (
            <div className="codex-recovery-row" key={thread.id}>
              <div>
                <strong>{thread.title}</strong>
                {thread.cwd && <code>{thread.cwd}</code>}
                <span>
                  {thread.updatedAt
                    ? new Date(thread.updatedAt).toLocaleString()
                    : thread.fallbackMeta}
                  {runtime === "claude" && !thread.resumable ? " · Project missing; transcript only" : ""}
                </span>
              </div>
              <button
                className="btn-ghost"
                type="button"
                disabled={importingId !== null}
                onClick={() => {
                  if (existing) openSession(existing);
                  else void importThread(thread);
                }}
              >
                {importingId === thread.id ? "Importing..." : existing ? "Open" : "Import"}
              </button>
            </div>
          );
        })}
        {!busy && threads.length === 0 && <p className="providers-list-empty">No {runtimeLabel} chats found.</p>}
      </div>
      {cursor && (
        <button className="btn-ghost codex-recovery-more" type="button" disabled={busy} onClick={() => void load(false)}>
              {busy ? "Loading..." : "Load more"}
        </button>
      )}
    </SheetDialog>
  );
}
