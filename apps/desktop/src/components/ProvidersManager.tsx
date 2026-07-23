import { useEffect, useRef, useState } from "react";
import {
  deleteProvider,
  discoverLocalProviders,
  getClaudeStatus,
  getCodexAccount,
  getOpenCodeStatus,
  getPiStatus,
  isCliPathWarningMessage,
  isOpenRouterProvider,
  listCodexThreads,
  listProviders,
  logoutCodex,
  openExternalUrl,
  PROVIDER_PRESETS,
  recoverCodexThread,
  saveProvider,
  streamCodexDeviceLogin,
  type ClaudeStatusResponse,
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
import { isLoopbackProviderEndpoint } from "../lib/providerEndpoint.js";
import { useSessions } from "../sessions/store";
import { Plus, Refresh, Search, X } from "./icons";
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
  const [recoveringCodex, setRecoveringCodex] = useState(false);
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
  const [discoveries, setDiscoveries] = useState<ProviderDiscovery[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const refresh = () => listProviders().then(setProviders);
  useEffect(() => {
    void refresh();
    void refreshCodexAccount();
    void refreshClaudeStatus();
    void refreshOpenCodeStatus();
    void refreshPiStatus();
  }, []);

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
    codexAccount?.account || (codexAccount && !codexAccount.requiresOpenaiAuth),
  );
  const claudeReady = Boolean(
    claudeStatus?.available && claudeStatus.authenticated,
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
  if (recoveringCodex) {
    return (
      <CodexRecoveryDialog
        onClose={() => setRecoveringCodex(false)}
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
                own desktop tooling.
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
                  <div>
                    <strong>Codex</strong>
                    <span>
                      {codexAccount?.account?.email ??
                        "ChatGPT account runtime"}
                    </span>
                  </div>
                </div>
                <div className="provider-account-actions">
                  {codexReady && (
                    <button
                      className="btn-ghost"
                      data-testid="codex-recover-chats"
                      type="button"
                      onClick={() => setRecoveringCodex(true)}
                    >
                      Recover chats
                    </button>
                  )}
                  <button
                    className="btn-ghost"
                    data-testid="codex-connect"
                    type="button"
                    onClick={() =>
                      void (codexReady ? disconnectCodex() : connectCodex())
                    }
                    disabled={codexBusy}
                  >
                    {codexBusy
                      ? "Working..."
                      : codexReady
                        ? "Disconnect"
                        : "Connect"}
                  </button>
                </div>
              </div>
              <div className={"provider-account-card " + (openCodeStatus?.available && openCodeStatus.authenticated ? "ready" : "off")}>
                <div className="provider-account-main">
                  <span className={"provider-status-dot " + (openCodeStatus?.available && openCodeStatus.authenticated ? "ready" : "off")} />
                  <div>
                    <strong>Installed OpenCode CLI</strong>
                    <span>{openCodeStatus?.available
                      ? `${openCodeStatus.models?.length ?? 0} configured model${openCodeStatus.models?.length === 1 ? "" : "s"}`
                      : "Install OpenCode separately and configure its providers."}</span>
                  </div>
                </div>
                <button className="btn-ghost" type="button" onClick={() => void refreshOpenCodeStatus(true)} disabled={openCodeBusy}>
                  {openCodeBusy ? "Checking..." : "Refresh"}
                </button>
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
                  <div>
                    <strong>Installed Claude CLI</strong>
                    <span>
                      {claudeAccountLabel ??
                        (claudeStatus?.available
                          ? "Run `claude auth login`, then refresh."
                          : "Install Anthropic's official Claude CLI separately.")}
                    </span>
                  </div>
                </div>
                <button
                  className="btn-ghost"
                  data-testid="claude-code-status"
                  type="button"
                  onClick={() => void refreshClaudeStatus(true)}
                  disabled={claudeBusy}
                >
                  {claudeBusy ? "Checking..." : "Refresh"}
                </button>
              </div>
              <div className={"provider-account-card " + (piStatus?.available && piStatus.authenticated ? "ready" : "off")}>
                <div className="provider-account-main">
                  <span className={"provider-status-dot " + (piStatus?.available && piStatus.authenticated ? "ready" : "off")} />
                  <div>
                    <strong>Installed Pi CLI</strong>
                    <span>{piStatus?.available
                      ? `${piStatus.provider_count ?? 0} provider${piStatus.provider_count === 1 ? "" : "s"} / ${piStatus.models?.length ?? 0} model${piStatus.models?.length === 1 ? "" : "s"}`
                      : "Install Pi separately and use /login in its terminal."}</span>
                  </div>
                </div>
                <button className="btn-ghost" data-testid="pi-status" type="button" onClick={() => void refreshPiStatus(true)} disabled={piBusy}>
                  {piBusy ? "Checking..." : "Refresh"}
                </button>
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
                <Plus size={13} />
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
                <Search size={13} />
                <strong>
                  {detecting ? "Detecting local" : "Detect local"}
                </strong>
                <span>Find Ollama or LM Studio.</span>
              </button>
              {["OpenAI", "Anthropic", "Gemini"].map((presetName) => (
                <button
                  className="provider-quick-action"
                  type="button"
                  key={presetName}
                  onClick={() => startPreset(presetName)}
                  title={`Start ${presetName} provider setup.`}
                  aria-label={`Start ${presetName} provider setup`}
                >
                  <Plus size={13} />
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

function CodexRecoveryDialog({
  onClose,
  onOpenSession,
}: {
  onClose: () => void;
  onOpenSession: () => void;
}) {
  const sessions = useSessions((state) => state.sessions);
  const requestId = useRef(0);
  const [threads, setThreads] = useState<CodexThreadSummary[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [archived, setArchived] = useState(false);
  const [busy, setBusy] = useState(false);
  const [recoveringId, setRecoveringId] = useState<string | null>(null);
  const [error, setError] = useState("");

  async function load(reset: boolean) {
    const currentRequest = ++requestId.current;
    setBusy(true);
    setError("");
    try {
      const page = await listCodexThreads({
        cursor: reset ? undefined : cursor ?? undefined,
        search,
        archived,
      });
      if (currentRequest !== requestId.current) return;
      setThreads((current) => reset
        ? page.data
        : [...current, ...page.data.filter((thread) => !current.some((item) => item.id === thread.id))]);
      setCursor(page.next_cursor ?? null);
    } catch (error) {
      if (currentRequest === requestId.current)
        setError(error instanceof Error ? error.message : "Codex chat recovery failed.");
    } finally {
      if (currentRequest === requestId.current) setBusy(false);
    }
  }

  useEffect(() => {
    const timer = window.setTimeout(() => void load(true), 250);
    return () => window.clearTimeout(timer);
  }, [search, archived]);

  function openSession(id: string) {
    useSessions.getState().switchTo(id);
    onOpenSession();
  }

  async function recover(thread: CodexThreadSummary) {
    const existing = recoveredCodexSessionId(useSessions.getState().sessions, thread.id);
    if (existing) {
      openSession(existing);
      return;
    }
    setRecoveringId(thread.id);
    setError("");
    try {
      const recovered = await recoverCodexThread(thread.id);
      const store = useSessions.getState();
      const sessionId = store.importSession(recoveredCodexSession(recovered));
      if (!sessionId) throw new Error("Milim could not import the recovered chat.");
      const imported = useSessions.getState().sessions.find((session) => session.id === sessionId);
      const lastMessageId = imported?.messages[imported.messages.length - 1]?.id;
      if (!lastMessageId) throw new Error("The recovered chat did not contain a sync cursor.");
      useSessions.getState().setAccountRuntime(sessionId, {
        codexThreadId: recovered.id,
        codexLastSyncedMessageId: lastMessageId,
      });
      openSession(sessionId);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Codex chat recovery failed.");
    } finally {
      setRecoveringId(null);
    }
  }

  return (
    <SheetDialog title="Recover Codex chats" className="sheet codex-recovery-sheet" onClose={onClose}>
      <div className="sheet-header providers-header">
        <div className="providers-title">
          <h2>Recover Codex chats</h2>
          <p className="sheet-sub">Import a transcript once, then choose a Codex model to continue its native thread.</p>
        </div>
        <button className="icon-btn sheet-close" type="button" onClick={onClose} aria-label="Close recovery">
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
            placeholder="Search Codex chats"
            aria-label="Search Codex chats"
          />
        </label>
        <div className="codex-recovery-tabs" role="group" aria-label="Codex chat archive filter">
          <button type="button" className={!archived ? "active" : ""} onClick={() => setArchived(false)}>Active</button>
          <button type="button" className={archived ? "active" : ""} onClick={() => setArchived(true)}>Archived</button>
        </div>
      </div>
      {error && <p className="provider-note error" role="alert">{error}</p>}
      <div className="codex-recovery-list" aria-busy={busy}>
        {threads.map((thread) => {
          const existing = recoveredCodexSessionId(sessions, thread.id);
          return (
            <div className="codex-recovery-row" key={thread.id}>
              <div>
                <strong>{thread.name?.trim() || thread.preview.trim() || "Untitled Codex chat"}</strong>
                {thread.cwd && <code>{thread.cwd}</code>}
                <span>{thread.updated_at_ms ? new Date(thread.updated_at_ms).toLocaleString() : thread.model_provider}</span>
              </div>
              <button
                className="btn-ghost"
                type="button"
                disabled={recoveringId !== null}
                onClick={() => {
                  if (existing) openSession(existing);
                  else void recover(thread);
                }}
              >
                        {recoveringId === thread.id ? "Recovering..." : existing ? "Open" : "Recover"}
              </button>
            </div>
          );
        })}
        {!busy && threads.length === 0 && <p className="providers-list-empty">No Codex chats found.</p>}
      </div>
      {cursor && (
        <button className="btn-ghost codex-recovery-more" type="button" disabled={busy} onClick={() => void load(false)}>
              {busy ? "Loading..." : "Load more"}
        </button>
      )}
    </SheetDialog>
  );
}
