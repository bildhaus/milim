import { useEffect, useMemo, useState } from "react";
import "../settings.css";
import {
  accountRuntimeKind,
  discoverLocalProviders,
  getClaudeStatus,
  getCodexAccount,
  getOpenCodeStatus,
  getPiStatus,
  isCliPathWarningMessage,
  loadStartupModels,
  openExternalUrl,
  PROVIDER_PRESETS,
  saveProvider,
  streamCodexDeviceLogin,
  type AccountRuntimeKind,
  type ClaudeStatusResponse,
  type CodexAccountResponse,
  type CodexLoginEvent,
  type ModelInfo,
  type OpenCodeStatusResponse,
  type PiStatusResponse,
  type ProviderDiscovery,
} from "../api";
import { useOnboarding, type OnboardingSetupPath, type OnboardingStepId } from "../onboarding/store";
import { DEFAULT_THREAD_SETTINGS, useSessions } from "../sessions/store";
import { useSettings } from "../settings/store";
import { ArrowLeft, ArrowRight, Check, PlusSquare, Search, X } from "./icons";
import { Logo } from "./Logo";
import { ModelPicker } from "./ModelPicker";
import { ProviderIcon, providerBrandForProvider, type ProviderBrand } from "./ProviderIcon";
import { SheetDialog } from "./SheetDialog";
import { Select } from "./ui";

type StepDefinition = { id: OnboardingStepId; label: string };
type NoticeTone = "info" | "success" | "warning" | "error";

const STEPS: StepDefinition[] = [
  { id: "model", label: "Runtime" },
  { id: "context", label: "Workspace" },
];

type RuntimeStatuses = {
  codex: CodexAccountResponse | null;
  claude: ClaudeStatusResponse | null;
  opencode: OpenCodeStatusResponse | null;
  pi: PiStatusResponse | null;
};

const EMPTY_RUNTIME_STATUSES: RuntimeStatuses = {
  codex: null,
  claude: null,
  opencode: null,
  pi: null,
};

function modelProviderLabel(model: ModelInfo | null): string {
  return model?.owned_by?.trim() || "local";
}

function pathLabel(path: OnboardingSetupPath | null): string {
  if (path === "local_detect") return "Local detection";
  if (path === "hosted") return "Hosted provider";
  if (path === "account_runtime") return "Installed agent";
  return "Not chosen";
}

function stepTitle(step: OnboardingStepId): string {
  if (step === "model") return "Choose the runtime";
  if (step === "context") return "Choose the workspace";
  return "Configure Milim";
}

function OnboardingStory({
  tone,
  title,
  body,
  details,
}: {
  tone: "style" | "model" | "tools" | "context" | "ready";
  title: string;
  body: string;
  details: string[];
}) {
  return (
    <div className={`onboarding-story onboarding-story-${tone}`}>
      <div className="onboarding-brand-mark" aria-hidden="true">
        <Logo height={82} className="onboarding-wordmark" />
        <span>{details.slice(0, 2).join(" / ")}</span>
      </div>
      <div className="onboarding-story-copy">
        <h3>{title}</h3>
        <p>{body}</p>
      </div>
    </div>
  );
}

function inTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function runtimeReady(kind: AccountRuntimeKind, statuses: RuntimeStatuses): boolean {
  if (kind === "codex") return Boolean(statuses.codex && (statuses.codex.account || !statuses.codex.requiresOpenaiAuth));
  return Boolean(statuses[kind]?.available && statuses[kind]?.authenticated);
}

function runtimeDetail(kind: AccountRuntimeKind, statuses: RuntimeStatuses): string {
  if (kind === "codex") {
    const status = statuses.codex;
    if (!status) return "Codex CLI was not detected.";
    if (status.account) return status.account.email ?? status.account.planType ?? "Authenticated.";
    return status.requiresOpenaiAuth ? "Sign in with ChatGPT to use Codex models." : "Available.";
  }
  if (kind === "claude") {
    const status = statuses.claude;
    if (!status) return "Claude CLI was not detected.";
    if (status.available && status.authenticated) return status.auth?.email ?? status.auth?.subscriptionType ?? "Authenticated.";
    if (status.error) return status.error;
    return status.available ? "Run `claude auth login`, then refresh." : "CLI not found on PATH.";
  }
  const status = kind === "opencode" ? statuses.opencode : statuses.pi;
  if (!status) return `${kind === "opencode" ? "OpenCode" : "Pi"} CLI was not detected.`;
  if (status.available && status.authenticated) return status.version ? `Version ${status.version}` : "Authenticated and ready.";
  if (status.error) return status.error;
  if (!status.available) return "CLI not found on PATH.";
  return kind === "opencode" ? "Configure a provider in OpenCode, then refresh." : "Run Pi and use /login, then refresh.";
}

export function OnboardingFlow({ onModelsChanged }: { onModelsChanged?: () => Promise<void> | void }) {
  const onboarding = useOnboarding();
  const activeId = useSessions((s) => s.activeId);
  const rawThreadSettings = useSessions((s) => s.sessions.find((x) => x.id === s.activeId)?.settings);
  const updateThreadSettings = useSessions((s) => s.updateSettings);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const setAccountRuntimeEnabled = useSettings((s) => s.setAccountRuntimeEnabled);
  const threadSettings = useMemo(() => ({ ...DEFAULT_THREAD_SETTINGS, ...rawThreadSettings }), [rawThreadSettings]);
  const selectedModel = threadSettings.model.trim();
  const [step, setStep] = useState<OnboardingStepId>(() => onboarding.completedSteps.includes("model") ? "context" : "model");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [discoveries, setDiscoveries] = useState<ProviderDiscovery[]>([]);
  const [providerNotice, setProviderNotice] = useState<{ tone: NoticeTone; message: string } | null>(null);
  const [activeSetupPath, setActiveSetupPath] = useState<OnboardingSetupPath>(() => onboarding.selectedSetupPath ?? "local_detect");
  const [hostedPresetName, setHostedPresetName] = useState("OpenAI");
  const [hostedApiKey, setHostedApiKey] = useState("");
  const [hostedBusy, setHostedBusy] = useState(false);
  const [runtimeStatuses, setRuntimeStatuses] = useState<RuntimeStatuses>(EMPTY_RUNTIME_STATUSES);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [codexBusy, setCodexBusy] = useState(false);
  const [folderDraft, setFolderDraft] = useState(threadSettings.folder);

  const steps = STEPS;
  const currentIndex = Math.max(0, steps.findIndex((item) => item.id === step));
  const selectedModelInfo = models.find((model) => model.id === selectedModel) ?? null;
  const selectedModelReady = Boolean(selectedModelInfo);
  const hostedPreset = PROVIDER_PRESETS.find((preset) => preset.name === hostedPresetName) ?? PROVIDER_PRESETS[0];

  async function refreshModels(selectFirst = false, preferredOwner?: string) {
    setModelsLoading(true);
    let firstCatalog = true;
    let selected = false;
    let latest: ModelInfo[] = [];
    const firstResult = new Promise<void>((resolve) => {
      void loadStartupModels((next) => {
        latest = next;
        setModels(next);
        if (firstCatalog) {
          firstCatalog = false;
          setModelsLoading(false);
          resolve();
        }
        if (!selectFirst || selected) return;
        const preferred = preferredOwner
          ? next.find((model) => model.owned_by.toLowerCase() === preferredOwner.toLowerCase())
          : null;
        const modelToSelect = preferred ?? (preferredOwner ? null : next[0]);
        if (!modelToSelect) return;
        selected = true;
        updateThreadSettings(activeId, { model: modelToSelect.id });
        onboarding.markStepComplete("model");
      }, accountRuntimeEnabled, models).then(() => {
        if (!latest.length) {
          setProviderNotice({ tone: "info", message: "No chat models found. Connect a provider or start a local runtime." });
        }
        if (selectFirst && !selected && latest[0]) {
          updateThreadSettings(activeId, { model: latest[0].id });
          onboarding.markStepComplete("model");
        }
        void onModelsChanged?.();
      }).catch((error) => {
        setModelsLoading(false);
        setProviderNotice({ tone: "error", message: error instanceof Error ? error.message : "Model refresh failed." });
        if (firstCatalog) resolve();
      });
    });
    await firstResult;
  }

  useEffect(() => {
    void refreshModels();
    void refreshAccountRuntimes();
    onboarding.start();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    setFolderDraft(threadSettings.folder);
  }, [threadSettings.folder]);

  useEffect(() => {
    if (steps.some((item) => item.id === step)) return;
    setStep("context");
  }, [step, steps]);

  async function refreshAccountRuntimes(refreshCodex = false) {
    setRuntimeBusy(true);
    const [codex, claude, opencode, pi] = await Promise.allSettled([
      getCodexAccount(refreshCodex),
      getClaudeStatus(),
      getOpenCodeStatus(),
      getPiStatus(),
    ]);
    setRuntimeStatuses({
      codex: codex.status === "fulfilled" ? codex.value : null,
      claude: claude.status === "fulfilled" ? claude.value : null,
      opencode: opencode.status === "fulfilled" ? opencode.value : null,
      pi: pi.status === "fulfilled" ? pi.value : null,
    });
    setRuntimeBusy(false);
  }

  function selectModel(modelId: string) {
    updateThreadSettings(activeId, { model: modelId });
    onboarding.markStepComplete("model");
    if (accountRuntimeKind(modelId)) {
      setActiveSetupPath("account_runtime");
      onboarding.setSetupPath("account_runtime");
    }
    setProviderNotice({ tone: "success", message: `Selected ${modelId}.` });
  }

  function enableRuntime(kind: AccountRuntimeKind) {
    setAccountRuntimeEnabled(kind, true);
    setActiveSetupPath("account_runtime");
    onboarding.setSetupPath("account_runtime");
  }

  function chooseSetupPath(path: OnboardingSetupPath) {
    setActiveSetupPath(path);
    onboarding.setSetupPath(path);
    setProviderNotice(null);
  }

  async function detectLocal() {
    setDiscovering(true);
    setProviderNotice(null);
    setActiveSetupPath("local_detect");
    onboarding.setSetupPath("local_detect");
    try {
      const found = await discoverLocalProviders();
      const reachableCount = found.filter((discovery) => discovery.reachable).length;
      setDiscoveries(found);
      setProviderNotice(
        reachableCount
          ? { tone: "success", message: `${reachableCount} local endpoint${reachableCount === 1 ? "" : "s"} found.` }
          : { tone: "info", message: "No Ollama, LM Studio, or vLLM endpoint answered. Start one and try again." },
      );
    } catch (error) {
      setProviderNotice({ tone: "error", message: error instanceof Error ? error.message : "Local detection failed." });
    } finally {
      setDiscovering(false);
    }
  }

  async function addDiscovery(discovery: ProviderDiscovery) {
    setProviderNotice(null);
    const saved = await saveProvider({
      name: discovery.name,
      kind: discovery.kind,
      base_url: discovery.base_url,
      enabled: true,
    });
    if (!saved) {
      setProviderNotice({ tone: "error", message: `Could not save ${discovery.name}.` });
      return;
    }
    setProviderNotice(
      saved.models.length
        ? { tone: "success", message: `${saved.name} connected with ${saved.models.length} model${saved.models.length === 1 ? "" : "s"}.` }
        : { tone: "info", message: `${saved.name} saved, but no models were returned yet.` },
    );
    await refreshModels(true);
  }

  async function saveHostedPreset() {
    if (!hostedPreset) return;
    if (hostedPreset.needsKey && !hostedApiKey.trim()) {
      setProviderNotice({ tone: "error", message: `${hostedPreset.name} needs an API key.` });
      return;
    }
    setHostedBusy(true);
    setProviderNotice(null);
    setActiveSetupPath("hosted");
    onboarding.setSetupPath("hosted");
    try {
      const saved = await saveProvider({
        name: hostedPreset.name,
        kind: hostedPreset.kind,
        base_url: hostedPreset.base_url,
        api_key: hostedApiKey.trim() || undefined,
        enabled: true,
      });
      if (!saved) {
        setProviderNotice({ tone: "error", message: "Provider save failed." });
        return;
      }
      setHostedApiKey("");
      setProviderNotice(
        saved.models.length
          ? { tone: "success", message: `${saved.name} connected with ${saved.models.length} model${saved.models.length === 1 ? "" : "s"}.` }
          : { tone: saved.error ? "error" : "info", message: saved.error ?? `${saved.name} saved, but no models were returned.` },
      );
      await refreshModels(true);
    } finally {
      setHostedBusy(false);
    }
  }

  async function connectCodex() {
    if (codexBusy) return;
    setCodexBusy(true);
    setActiveSetupPath("account_runtime");
    onboarding.setSetupPath("account_runtime");
    setProviderNotice({ tone: "info", message: "Starting Codex login." });
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
              setProviderNotice({ tone: "error", message: `Could not open Codex login URL: ${error instanceof Error ? error.message : String(error)}` });
            });
          }
          setProviderNotice({ tone: "info", message: "Complete Codex login in the browser, then return here." });
        } else if (ev.type === "device_code") {
          if (!opened) {
            opened = true;
            void openExternalUrl(ev.verification_url).catch((error) => {
              setProviderNotice({ tone: "error", message: `Could not open Codex device-code URL: ${error instanceof Error ? error.message : String(error)}` });
            });
          }
          setProviderNotice({ tone: "info", message: `Complete Codex login with code ${ev.user_code}.` });
        } else if (ev.type === "done") {
          completed = ev.success;
          failed = ev.error ?? "";
        } else if (ev.type === "warning") {
          failed = ev.message;
          warning = true;
          setProviderNotice({ tone: "warning", message: ev.message });
        } else if (ev.type === "error") {
          failed = ev.message;
        }
      });
      await refreshAccountRuntimes(true);
      if (completed) {
        setProviderNotice({ tone: "success", message: "Codex connected. Refreshing available models." });
        await refreshModels(true, "Codex");
      } else {
        warning ||= isCliPathWarningMessage(failed);
        setProviderNotice({ tone: warning ? "warning" : "error", message: failed || "Codex login did not complete." });
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : "Codex login failed.";
      setProviderNotice({ tone: isCliPathWarningMessage(message) ? "warning" : "error", message });
    } finally {
      setCodexBusy(false);
    }
  }

  async function pickFolder() {
    if (!inTauriRuntime()) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === "string") {
        setFolderDraft(selected);
        updateThreadSettings(activeId, { folder: selected });
      }
    } catch {
      /* dialog unavailable */
    }
  }

  function nextStep() {
    if (step === "model" && !selectedModelReady) {
      setProviderNotice({ tone: "error", message: "Connect and select a reachable chat model before continuing." });
      return;
    }
    if (step === "context") {
      updateThreadSettings(activeId, { folder: folderDraft.trim() });
      onboarding.markStepComplete("context");
      finish();
      return;
    }
    const next = steps[Math.min(currentIndex + 1, steps.length - 1)];
    setStep(next.id);
  }

  function previousStep() {
    const prev = steps[Math.max(currentIndex - 1, 0)];
    setStep(prev.id);
  }

  function finish() {
    if (!selectedModelReady) {
      setStep("model");
      setProviderNotice({ tone: "error", message: "Select a reachable model before finishing setup." });
      return;
    }
    onboarding.complete();
    window.requestAnimationFrame(() => {
      document
        .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
        ?.focus();
    });
  }

  function dismiss() {
    onboarding.dismiss();
  }

  function openStep(id: OnboardingStepId) {
    const targetIndex = steps.findIndex((item) => item.id === id);
    const modelIndex = steps.findIndex((item) => item.id === "model");
    if (targetIndex > modelIndex && !selectedModelReady) {
      setStep("model");
      setProviderNotice({ tone: "error", message: "Connect and select a reachable chat model before continuing." });
      return;
    }
    setStep(id);
  }

  const installedRuntimes: Array<{
    kind: AccountRuntimeKind;
    name: string;
    brand: ProviderBrand;
  }> = [
    { kind: "codex", name: "Codex", brand: "codex" },
    { kind: "claude", name: "Claude", brand: "claude" },
    { kind: "opencode", name: "OpenCode", brand: "opencode" },
    { kind: "pi", name: "Pi", brand: "pi" },
  ];

  return (
    <SheetDialog
      title="Set up Milim"
      className="sheet onboarding-sheet"
      overlayClassName="sheet-overlay onboarding-overlay"
      testId="onboarding-flow"
      onClose={dismiss}
    >
      <div className="onboarding-header">
        <button className="onboarding-nav-back" type="button" onClick={previousStep} disabled={currentIndex === 0}>
          <ArrowLeft size={14} />
          <span>Back</span>
        </button>
        <div className="onboarding-header-title">
          <strong>Set up Milim</strong>
          <span>Step {currentIndex + 1} of {steps.length} · {stepTitle(step)}</span>
        </div>
        <button className="icon-btn sheet-close" type="button" onClick={dismiss} title="Close" aria-label="Close onboarding">
          <X size={15} />
        </button>
      </div>

      <div className="onboarding-layout">
        <aside className="onboarding-steps" aria-label="Onboarding steps">
          {steps.map((item, index) => {
            const active = item.id === step;
            const done = !active && (onboarding.completedSteps.includes(item.id) || index < currentIndex);
            return (
              <button
                key={item.id}
                type="button"
                className={"onboarding-step" + (active ? " active" : "") + (done ? " done" : "")}
                onClick={() => openStep(item.id)}
                aria-current={active ? "step" : undefined}
              >
                <span className="onboarding-step-index">{done ? <Check size={12} /> : index + 1}</span>
                <span>{item.label}</span>
              </button>
            );
          })}
        </aside>

        <main className="onboarding-content">
          {step === "model" && (
            <section className="onboarding-panel onboarding-split-panel onboarding-model-panel" aria-labelledby="onboarding-model-title">
                <OnboardingStory
                  tone="model"
                  title="Connect any model source."
                  body="Use an installed coding agent, a local server, or a hosted provider. Milim keeps them in the same thread."
                  details={[selectedModelReady ? selectedModel : "No model selected", pathLabel(activeSetupPath)]}
                />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-model-title">Connect a runtime</h3>
                  <p>Detect or connect a source, then choose any reachable model.</p>
                </div>

                <div className="onboarding-model-summary">
                  <div>
                    <strong>{selectedModelReady ? selectedModel : "No reachable model selected"}</strong>
                    <span>
                      {selectedModelInfo
                        ? `Provider: ${modelProviderLabel(selectedModelInfo)}`
                        : modelsLoading
                          ? "Checking available models..."
                          : selectedModel
                            ? `${selectedModel} is not available from the current providers.`
                            : "Choose a setup path below."}
                    </span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={() => void refreshModels()} disabled={modelsLoading}>
                    Refresh
                  </button>
                </div>

                <div className="onboarding-setup-shell">
                  <div className="onboarding-path-list" aria-label="Model setup paths">
                    <button className={"onboarding-path-option" + (activeSetupPath === "local_detect" ? " active" : "")} type="button" aria-pressed={activeSetupPath === "local_detect"} onClick={() => chooseSetupPath("local_detect")}>
                      <span className="onboarding-path-icon"><Search size={14} /></span>
                      <span><strong>Detect local</strong><small>Ollama, LM Studio, or vLLM</small></span>
                    </button>
                    <button className={"onboarding-path-option" + (activeSetupPath === "hosted" ? " active" : "")} type="button" aria-pressed={activeSetupPath === "hosted"} onClick={() => chooseSetupPath("hosted")}>
                      <span className="onboarding-path-icon"><PlusSquare size={14} /></span>
                      <span><strong>Hosted</strong><small>OpenAI, OpenRouter, Gemini</small></span>
                    </button>
                    <button className={"onboarding-path-option" + (activeSetupPath === "account_runtime" ? " active" : "")} type="button" aria-pressed={activeSetupPath === "account_runtime"} onClick={() => chooseSetupPath("account_runtime")}>
                      <span className="onboarding-path-icon"><ProviderIcon brand="claude" /></span>
                      <span><strong>Installed agents</strong><small>Codex, Claude, OpenCode, Pi</small></span>
                    </button>
                  </div>

                  <div className="onboarding-path-detail">
                  {activeSetupPath === "local_detect" && (
                    <>
                      <div className="onboarding-path-head">
                        <span className="onboarding-path-icon"><Search size={15} /></span>
                        <div>
                          <h4>Detect a local runtime</h4>
                          <p>Use this if Ollama, LM Studio, or vLLM is already running on this machine.</p>
                        </div>
                      </div>
                      <button className="btn-accent" type="button" onClick={() => void detectLocal()} disabled={discovering}>
                        {discovering ? "Detecting..." : "Detect local"}
                      </button>
                      {discoveries.length > 0 ? (
                        <div className="onboarding-discoveries">
                          {discoveries.map((discovery) => (
                            <div className="onboarding-discovery" key={discovery.base_url}>
                              <ProviderIcon brand={providerBrandForProvider(discovery)} size={16} />
                              <span>
                                <strong>{discovery.name}</strong>
                                <small>{discovery.models.length ? `${discovery.models.length} models at ${discovery.base_url}` : discovery.error ?? discovery.base_url}</small>
                              </span>
                              <button className="btn-ghost" type="button" onClick={() => void addDiscovery(discovery)} disabled={discovery.configured || !discovery.reachable}>
                                {discovery.configured ? "Added" : discovery.reachable ? "Add" : "Unavailable"}
                              </button>
                            </div>
                          ))}
                        </div>
                      ) : (
                        <p className="onboarding-path-note">Start Ollama, LM Studio, or vLLM, then run detection. No API key is needed for local endpoints.</p>
                      )}
                    </>
                  )}

                  {activeSetupPath === "hosted" && (
                    <>
                      <div className="onboarding-path-head">
                        <span className="onboarding-path-icon"><PlusSquare size={15} /></span>
                        <div>
                          <h4>Add a hosted provider</h4>
                          <p>Choose a preset and save an encrypted key. Models refresh after the connection is tested.</p>
                        </div>
                      </div>
                      <div className="onboarding-hosted-form">
                        <div className="onboarding-provider-select">
                          <ProviderIcon brand={providerBrandForProvider(hostedPreset)} size={16} />
                          <Select
                            value={hostedPresetName}
                            onChange={setHostedPresetName}
                            options={PROVIDER_PRESETS.filter((preset) => preset.needsKey).map((preset) => ({ value: preset.name, label: preset.name }))}
                            testId="onboarding-hosted-preset"
                          />
                        </div>
                        <input
                          className="onboarding-input"
                          value={hostedApiKey}
                          onChange={(event) => setHostedApiKey(event.currentTarget.value)}
                          placeholder="API key"
                          type="password"
                          data-testid="onboarding-hosted-api-key"
                        />
                        <button className="btn-accent" type="button" onClick={() => void saveHostedPreset()} disabled={hostedBusy}>
                          {hostedBusy ? "Saving..." : `Save ${hostedPresetName}`}
                        </button>
                      </div>
                    </>
                  )}

                  {activeSetupPath === "account_runtime" && (
                    <>
                      <div className="onboarding-path-head">
                        <span className="onboarding-path-icon"><ProviderIcon brand="claude" size={15} /></span>
                        <div>
                          <h4>Use an installed agent</h4>
                          <p>Milim detects each CLI independently. Authenticate with that agent's own tooling, then refresh.</p>
                        </div>
                      </div>
                      <div className="onboarding-runtime-list">
                        {installedRuntimes.map(({ kind, name, brand }) => {
                          const enabled = accountRuntimeEnabled[kind];
                          const ready = enabled && runtimeReady(kind, runtimeStatuses);
                          return (
                            <div className="onboarding-runtime-row" data-testid={`onboarding-runtime-${kind}`} key={kind}>
                              <ProviderIcon brand={brand} size={16} />
                              <span>
                                <strong>{name}</strong>
                                <small>{enabled ? runtimeDetail(kind, runtimeStatuses) : "Disabled in Providers."}</small>
                              </span>
                              <em className={ready ? "ready" : ""}>{ready ? "Ready" : enabled ? "Setup needed" : "Disabled"}</em>
                              {!enabled ? (
                                <button className="btn-ghost" type="button" onClick={() => enableRuntime(kind)}>Enable</button>
                              ) : kind === "codex" && !ready ? (
                                <button className="btn-ghost" type="button" onClick={() => void connectCodex()} disabled={codexBusy}>
                                  {codexBusy ? "Connecting..." : "Connect"}
                                </button>
                              ) : null}
                            </div>
                          );
                        })}
                      </div>
                      <button className="btn-accent" type="button" onClick={() => void refreshAccountRuntimes(true)} disabled={runtimeBusy}>
                        {runtimeBusy ? "Checking..." : "Refresh agents"}
                      </button>
                    </>
                  )}

                  </div>
                </div>

                {providerNotice && (
                  <p className={`onboarding-notice ${providerNotice.tone}`} role={providerNotice.tone === "error" ? "alert" : "status"}>
                    {providerNotice.message}
                  </p>
                )}

                {models.length > 0 && (
                  <div className="onboarding-model-picker">
                    <span className="onboarding-mini-title">Available models</span>
                    <ModelPicker
                      models={models}
                      model={selectedModel}
                      onModel={({ model }) => selectModel(model)}
                      onClose={() => {}}
                      showManagementActions={false}
                      searchPlaceholder="Search available models..."
                    />
                  </div>
                )}
              </div>
            </section>
          )}

          {step === "context" && (
            <section className="onboarding-panel onboarding-split-panel" aria-labelledby="onboarding-context-title">
              <OnboardingStory
                tone="context"
                title="Choose where work happens."
                body="A workspace unlocks repository files, shell commands, Git review, and runnable previews. You can also continue without one."
                details={[threadSettings.folder || "No workspace", threadSettings.folder ? "Project tools ready" : "Chat only"]}
              />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-context-title">Choose a workspace</h3>
                  <p>Optional. Add a project folder now, or use folderless chat and choose one later.</p>
                </div>
                <div className="onboarding-workbench-grid">
                  <label className="onboarding-field">
                    <span>Workspace folder</span>
                    <span className="onboarding-field-row">
                      <input
                        value={folderDraft}
                        onChange={(event) => setFolderDraft(event.currentTarget.value)}
                        onBlur={() => updateThreadSettings(activeId, { folder: folderDraft.trim() })}
                        placeholder="C:/path/to/project"
                      />
                      <button className="btn-ghost" type="button" onClick={() => void pickFolder()} disabled={!inTauriRuntime()}>
                        Choose
                      </button>
                    </span>
                  </label>
                  <p className="onboarding-path-note">
                    Memory, privacy, sandbox, computer use, and power tools remain available after setup.
                  </p>
                </div>
              </div>
            </section>
          )}

        </main>
      </div>

      <div className="onboarding-footer">
        <button className="btn-ghost" type="button" onClick={dismiss}>
          Skip for now
        </button>
        <div className="onboarding-footer-actions">
          <button className="btn-accent" type="button" onClick={nextStep} disabled={step === "model" && !selectedModelReady}>
            <span>{step === "context" ? "Open Milim" : "Continue"}</span>
            <ArrowRight size={14} />
          </button>
        </div>
      </div>
    </SheetDialog>
  );
}
