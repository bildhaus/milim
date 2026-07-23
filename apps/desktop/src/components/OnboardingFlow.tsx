import { useEffect, useMemo, useState } from "react";
import {
  createSkill,
  discoverHarnessImports,
  discoverLocalProviders,
  getCodexAccount,
  isCliPathWarningMessage,
  listMcpServers,
  listModelsDetailed,
  listSkills,
  openExternalUrl,
  PROVIDER_PRESETS,
  saveMcpServer,
  saveProvider,
  streamCodexDeviceLogin,
  type CodexAccountResponse,
  type HarnessImportPreview,
  type CodexLoginEvent,
  type ModelInfo,
  type PrivacyMode,
  type ProviderDiscovery,
} from "../api";
import { modelDisplayName } from "../lib/modelPicker";
import { useOnboarding, type OnboardingSetupPath, type OnboardingStepId } from "../onboarding/store";
import { DEFAULT_THREAD_SETTINGS, useSessions } from "../sessions/store";
import { useSettings } from "../settings/store";
import { ArrowRight, Check, PlusSquare, Search, X } from "./icons";
import { Logo } from "./Logo";
import { ProviderIcon, providerBrandForProvider } from "./ProviderIcon";
import { SheetDialog } from "./SheetDialog";
import { Select, Toggle } from "./ui";

type StepDefinition = { id: OnboardingStepId; label: string };
type NoticeTone = "info" | "success" | "warning" | "error";

const STEPS: StepDefinition[] = [
  { id: "model", label: "Model" },
  { id: "defaults", label: "Defaults" },
  { id: "context", label: "Context" },
  { id: "finish", label: "Ready" },
];

function modelProviderLabel(model: ModelInfo | null): string {
  return model?.owned_by?.trim() || "local";
}

function isLikelyRemoteModel(model: ModelInfo | null): boolean {
  if (!model) return false;
  const provider = model.owned_by.toLowerCase();
  return !/(^|\b)(local|ollama|lm studio|lmstudio|milim)(\b|$)/.test(provider);
}

function privacyLabel(mode: PrivacyMode): string {
  if (mode === "redact") return "Redact";
  if (mode === "block") return "Block";
  return "Off";
}

function pathLabel(path: OnboardingSetupPath | null): string {
  if (path === "local_detect") return "Local detection";
  if (path === "hosted") return "Hosted provider";
  if (path === "codex") return "Codex";
  return "Not chosen";
}

function stepTitle(step: OnboardingStepId): string {
  if (step === "model") return "Choose the runtime";
  if (step === "defaults") return "Set the ground rules";
  if (step === "context") return "Set the working context";
  if (step === "finish") return "Review setup";
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
        <Logo height={118} className="onboarding-wordmark" />
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

function importCount(preview: HarnessImportPreview | null): number {
  return (preview?.mcps.length ?? 0) + (preview?.skills.length ?? 0);
}

function uniqueImportName(name: string, harness: string, used: Set<string>): string {
  const base = name.trim() || harness;
  if (!used.has(base.toLowerCase())) return base;
  const fallback = `${base} (${harness})`;
  if (!used.has(fallback.toLowerCase())) return fallback;
  for (let i = 2; ; i++) {
    const next = `${fallback} ${i}`;
    if (!used.has(next.toLowerCase())) return next;
  }
}

export function OnboardingFlow({ onModelsChanged }: { onModelsChanged?: () => Promise<void> | void }) {
  const onboarding = useOnboarding();
  const activeId = useSessions((s) => s.activeId);
  const rawThreadSettings = useSessions((s) => s.sessions.find((x) => x.id === s.activeId)?.settings);
  const updateThreadSettings = useSessions((s) => s.updateSettings);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const threadSettings = useMemo(() => ({ ...DEFAULT_THREAD_SETTINGS, ...rawThreadSettings }), [rawThreadSettings]);
  const selectedModel = threadSettings.model.trim();
  const [step, setStep] = useState<OnboardingStepId>(() => onboarding.completedSteps.includes("model") ? "defaults" : "model");
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [discovering, setDiscovering] = useState(false);
  const [discoveries, setDiscoveries] = useState<ProviderDiscovery[]>([]);
  const [providerNotice, setProviderNotice] = useState<{ tone: NoticeTone; message: string } | null>(null);
  const [activeSetupPath, setActiveSetupPath] = useState<OnboardingSetupPath>(() => onboarding.selectedSetupPath ?? "local_detect");
  const [hostedPresetName, setHostedPresetName] = useState("OpenAI");
  const [hostedApiKey, setHostedApiKey] = useState("");
  const [hostedBusy, setHostedBusy] = useState(false);
  const [codexAccount, setCodexAccount] = useState<CodexAccountResponse | null>(null);
  const [codexBusy, setCodexBusy] = useState(false);
  const [folderDraft, setFolderDraft] = useState(threadSettings.folder);
  const [harnessPreview, setHarnessPreview] = useState<HarnessImportPreview | null>(null);
  const [harnessBusy, setHarnessBusy] = useState(false);
  const [harnessNote, setHarnessNote] = useState<{ tone: NoticeTone; message: string } | null>(null);

  const steps = STEPS;
  const currentIndex = Math.max(0, steps.findIndex((item) => item.id === step));
  const selectedModelInfo = models.find((model) => model.id === selectedModel) ?? null;
  const selectedModelReady = Boolean(selectedModelInfo);
  const remoteLikely = isLikelyRemoteModel(selectedModelInfo);
  const hostedPreset = PROVIDER_PRESETS.find((preset) => preset.name === hostedPresetName) ?? PROVIDER_PRESETS[0];

  async function refreshModels(selectFirst = false, preferredOwner?: string) {
    setModelsLoading(true);
    try {
      const next = await listModelsDetailed(accountRuntimeEnabled);
      setModels(next);
      if (!next.length)
        setProviderNotice({ tone: "info", message: "No chat models found. Connect a provider or start a local runtime." });
      const preferred = preferredOwner
        ? next.find((model) => model.owned_by.toLowerCase() === preferredOwner.toLowerCase())
        : null;
      const modelToSelect = preferred ?? next[0];
      if (selectFirst && modelToSelect?.id) {
        updateThreadSettings(activeId, { model: modelToSelect.id });
        onboarding.markStepComplete("model");
      }
      await onModelsChanged?.();
    } catch (error) {
      setProviderNotice({ tone: "error", message: error instanceof Error ? error.message : "Model refresh failed." });
      setModels([]);
    } finally {
      setModelsLoading(false);
    }
  }

  useEffect(() => {
    void refreshModels();
    if (accountRuntimeEnabled.codex) void refreshCodexAccount();
    onboarding.start();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    setFolderDraft(threadSettings.folder);
  }, [threadSettings.folder]);

  useEffect(() => {
    if (steps.some((item) => item.id === step)) return;
    setStep("finish");
  }, [step, steps]);

  useEffect(() => {
    if (!inTauriRuntime()) return;
    void refreshHarnessImports();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function refreshCodexAccount() {
    try {
      setCodexAccount(await getCodexAccount(false));
    } catch {
      setCodexAccount(null);
    }
  }

  function selectModel(modelId: string) {
    updateThreadSettings(activeId, { model: modelId });
    onboarding.markStepComplete("model");
    setProviderNotice({ tone: "success", message: `Selected ${modelId}.` });
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
      setDiscoveries(found);
      setProviderNotice(
        found.length
          ? { tone: "success", message: `${found.length} local endpoint${found.length === 1 ? "" : "s"} found.` }
          : { tone: "info", message: "No Ollama or LM Studio endpoint answered. Start one and try again." },
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
    setActiveSetupPath("codex");
    onboarding.setSetupPath("codex");
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
      await refreshCodexAccount();
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

  async function refreshHarnessImports() {
    setHarnessBusy(true);
    try {
      setHarnessPreview(await discoverHarnessImports());
    } finally {
      setHarnessBusy(false);
    }
  }

  async function importHarnessSetup() {
    const preview = harnessPreview ?? await discoverHarnessImports();
    const count = importCount(preview);
    if (!count) return;
    setHarnessBusy(true);
    setHarnessNote(null);
    try {
      const [existingMcps, existingSkills] = await Promise.all([listMcpServers(), listSkills()]);
      const mcpNames = new Set(existingMcps.map((server) => server.name.toLowerCase()));
      const skillNames = new Set(existingSkills.map((skill) => skill.name.toLowerCase()));
      let mcpCount = 0;
      let skillCount = 0;

      for (const mcp of preview.mcps) {
        const name = uniqueImportName(mcp.name, mcp.harness, mcpNames);
        const saved = await saveMcpServer({
          name,
          command: mcp.command,
          args: mcp.args,
          cwd: mcp.cwd ?? null,
          env: mcp.env ?? [],
          enabled: false,
        });
        if (saved) {
          mcpNames.add(saved.name.toLowerCase());
          mcpCount++;
        }
      }
      for (const skill of preview.skills) {
        if (skillNames.has(skill.name.toLowerCase())) continue;
        const saved = await createSkill({
          skill_md: skill.skill_md,
          enabled: false,
          source_kind: `imported:${skill.harness.toLowerCase().replace(/\s+/g, "-")}`,
          source_url: skill.path,
        });
        if (saved) {
          skillNames.add(saved.name.toLowerCase());
          skillCount++;
        }
      }

      setHarnessNote({ tone: "success", message: `Imported ${mcpCount} MCP server${mcpCount === 1 ? "" : "s"} and ${skillCount} skill${skillCount === 1 ? "" : "s"} disabled.` });
    } catch (error) {
      setHarnessNote({ tone: "error", message: error instanceof Error ? error.message : "Import failed." });
    } finally {
      setHarnessBusy(false);
    }
  }

  function nextStep() {
    if (step === "model" && !selectedModelReady) {
      setProviderNotice({ tone: "error", message: "Connect and select a reachable chat model before continuing." });
      return;
    }
    const next = steps[Math.min(currentIndex + 1, steps.length - 1)];
    if (step === "defaults") onboarding.markStepComplete("defaults");
    if (step === "context") onboarding.markStepComplete("context");
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

  return (
    <SheetDialog
      title="Personalize Milim"
      className="sheet onboarding-sheet"
      overlayClassName="sheet-overlay onboarding-overlay"
      testId="onboarding-flow"
      onClose={dismiss}
    >
      <div className="onboarding-header">
        <button className="onboarding-nav-back" type="button" onClick={previousStep} disabled={currentIndex === 0}>
          Back
        </button>
        <div className="onboarding-header-title">
          <span>{currentIndex + 1} of {steps.length}</span>
          <strong>{stepTitle(step)}</strong>
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
                  title="Connect a default runtime."
                  body="Milim needs one reachable model before the first chat. Local providers, hosted providers, and Codex all fit."
                  details={[selectedModelReady ? selectedModel : "No model selected", pathLabel(activeSetupPath)]}
                />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-model-title">Choose your default model</h3>
                  <p>Once Milim can see a model, select it here and continue.</p>
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
                    <button className={"onboarding-path-option" + (activeSetupPath === "local_detect" ? " active" : "")} type="button" onClick={() => chooseSetupPath("local_detect")}>
                      <span className="onboarding-path-icon"><Search size={14} /></span>
                      <span><strong>Detect local</strong><small>Ollama or LM Studio</small></span>
                    </button>
                    <button className={"onboarding-path-option" + (activeSetupPath === "hosted" ? " active" : "")} type="button" onClick={() => chooseSetupPath("hosted")}>
                      <span className="onboarding-path-icon"><PlusSquare size={14} /></span>
                      <span><strong>Hosted</strong><small>OpenAI, OpenRouter, Gemini</small></span>
                    </button>
                    <button className={"onboarding-path-option" + (activeSetupPath === "codex" ? " active" : "")} type="button" onClick={() => chooseSetupPath("codex")}>
                      <span className="onboarding-path-icon"><ProviderIcon brand="codex" /></span>
                      <span><strong>Codex</strong><small>Account-backed runtime</small></span>
                    </button>
                  </div>

                  <div className="onboarding-path-detail">
                  {activeSetupPath === "local_detect" && (
                    <>
                      <div className="onboarding-path-head">
                        <span className="onboarding-path-icon"><Search size={15} /></span>
                        <div>
                          <h4>Detect a local runtime</h4>
                          <p>Use this if Ollama or LM Studio is already running on this machine.</p>
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
                              <button className="btn-ghost" type="button" onClick={() => void addDiscovery(discovery)} disabled={discovery.configured}>
                                {discovery.configured ? "Added" : "Add"}
                              </button>
                            </div>
                          ))}
                        </div>
                      ) : (
                        <p className="onboarding-path-note">Start Ollama or LM Studio, then run detection. No API key is needed for local endpoints.</p>
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

                  {activeSetupPath === "codex" && (
                    <>
                      <div className="onboarding-path-head">
                        <span className="onboarding-path-icon"><ProviderIcon brand="codex" size={15} /></span>
                        <div>
                          <h4>Use Codex</h4>
                          <p>Connect the account-backed Codex runtime. Codex models appear only after authentication.</p>
                        </div>
                      </div>
                      <div className="onboarding-codex-status">
                        <strong>
                          {codexAccount?.account
                            ? codexAccount.account.email ?? "Codex connected"
                            : codexAccount && !codexAccount.requiresOpenaiAuth
                              ? "Codex available"
                              : "Not connected"}
                        </strong>
                        <span>{codexAccount?.account?.planType ?? "ChatGPT login required when prompted."}</span>
                      </div>
                      <button className="btn-accent" type="button" onClick={() => void connectCodex()} disabled={codexBusy}>
                        {codexBusy ? "Connecting..." : "Connect Codex"}
                      </button>
                    </>
                  )}

                  </div>
                </div>

                {providerNotice && <p className={`onboarding-notice ${providerNotice.tone}`}>{providerNotice.message}</p>}

                {models.length > 0 && (
                  <div className="onboarding-model-list">
                    <span className="onboarding-mini-title">Available models</span>
                    {models.map((modelInfo) => (
                      <button
                        key={`${modelInfo.owned_by}:${modelInfo.id}`}
                        type="button"
                        className={"onboarding-model-row" + (modelInfo.id === selectedModel ? " active" : "")}
                        onClick={() => selectModel(modelInfo.id)}
                      >
                        <span>{modelDisplayName(modelInfo)}</span>
                        <small>{modelInfo.owned_by}</small>
                      </button>
                    ))}
                  </div>
                )}
              </div>
            </section>
          )}

          {step === "defaults" && (
            <section className="onboarding-panel onboarding-split-panel" aria-labelledby="onboarding-defaults-title">
              <OnboardingStory
                tone="tools"
                title="Set the ground rules."
                body="Choose what the starter thread can remember and how remote sends should be handled."
                details={[threadSettings.memory ? "Memory on" : "Memory off", `Privacy ${privacyLabel(threadSettings.privacy)}`]}
              />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-defaults-title">Personalize chat defaults</h3>
                  <p>Keep the defaults simple, or add guardrails for remote providers.</p>
                </div>
                <div className="onboarding-defaults">
                  <div className="onboarding-toggle-row">
                    <div>
                      <strong>Memory</strong>
                      <span>Use thread and project memories when the selected model supports embeddings.</span>
                    </div>
                    <Toggle checked={threadSettings.memory} onChange={(memory) => updateThreadSettings(activeId, { memory })} testId="onboarding-memory-toggle" />
                  </div>
                  {remoteLikely && (
                    <div className="onboarding-privacy-grid">
                      {(["off", "redact", "block"] as PrivacyMode[]).map((privacyMode) => (
                        <button
                          key={privacyMode}
                          type="button"
                          className={"onboarding-privacy" + (threadSettings.privacy === privacyMode ? " active" : "")}
                          onClick={() => updateThreadSettings(activeId, { privacy: privacyMode })}
                        >
                          <strong>{privacyLabel(privacyMode)}</strong>
                          <span>
                            {privacyMode === "off"
                              ? "No scan before remote sends."
                              : privacyMode === "redact"
                                ? "Recommended for remote providers."
                                : "Stop remote sends when PII is detected."}
                          </span>
                          {privacyMode === "redact" && <small>Recommended</small>}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </section>
          )}

          {step === "context" && (
            <section className="onboarding-panel onboarding-split-panel" aria-labelledby="onboarding-context-title">
              <OnboardingStory
                tone="context"
                title="Point Milim at a project."
                body="Folder context, sandbox tools, and computer use stay explicit so Milim only reaches where you allow it."
                details={[threadSettings.folder || "No folder yet", threadSettings.sandbox ? "Sandbox on" : "Sandbox off"]}
              />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-context-title">Set the working context</h3>
                  <p>Set the project defaults now, then tune agents, MCP, media, and schedules from the main app.</p>
                </div>
                <div className="onboarding-workbench-grid">
                  <label className="onboarding-field">
                    <span>Working folder</span>
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
                  <div className="onboarding-toggle-row">
                    <div>
                      <strong>Sandbox tools</strong>
                      <span>Allow isolated Docker command runs when tools are used.</span>
                    </div>
                    <Toggle checked={threadSettings.sandbox} onChange={(sandbox) => updateThreadSettings(activeId, { sandbox })} testId="onboarding-sandbox-toggle" />
                  </div>
                  <div className="onboarding-toggle-row">
                    <div>
                      <strong>Computer use</strong>
                      <span>Allow screen, mouse, and keyboard tools for this thread.</span>
                    </div>
                    <Toggle checked={threadSettings.computerUse} onChange={(computerUse) => updateThreadSettings(activeId, { computerUse })} testId="onboarding-computer-toggle" />
                  </div>
                  <div className="onboarding-toggle-row">
                    <div>
                      <strong>Import agent setup</strong>
                      <span>
                        {harnessPreview
                          ? `${harnessPreview.mcps.length} MCP server${harnessPreview.mcps.length === 1 ? "" : "s"} and ${harnessPreview.skills.length} skill${harnessPreview.skills.length === 1 ? "" : "s"} found from Claude/Codex.`
                          : harnessBusy
                            ? "Scanning Claude and Codex config."
                            : "Bring over existing Claude/Codex MCP servers and skills."}
                      </span>
                    </div>
                    <button className="btn-ghost" type="button" disabled={harnessBusy || importCount(harnessPreview) === 0} onClick={() => void importHarnessSetup()}>
                      {harnessBusy ? "Working..." : "Import as disabled"}
                    </button>
                  </div>
                  {harnessNote && <p className={`onboarding-notice ${harnessNote.tone}`}>{harnessNote.message}</p>}
                  {harnessPreview && importCount(harnessPreview) > 0 && (
                    <div className="onboarding-model-list">
                      <span className="onboarding-mini-title">Detected setup</span>
                      {[...harnessPreview.mcps.slice(0, 4), ...harnessPreview.skills.slice(0, 4)].slice(0, 6).map((item) => (
                        <div className="onboarding-model-row" key={`${item.harness}:${"command" in item ? item.command : item.path}:${item.name}`}>
                          <span>{item.name}</span>
                          <small>{"command" in item ? `${item.harness} MCP` : `${item.harness} skill`}</small>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </section>
          )}

          {step === "finish" && (
            <section className="onboarding-panel onboarding-split-panel" aria-labelledby="onboarding-finish-title">
              <OnboardingStory
                tone="ready"
                title="Setup is ready."
                body="Milim will open with these defaults. Settings and model/provider managers stay available after onboarding."
                details={[threadSettings.folder ? "Project ready" : "No project", selectedModelReady ? "Model ready" : "Model missing"]}
              />
              <div className="onboarding-step-body">
                <div className="onboarding-panel-head">
                  <h3 id="onboarding-finish-title">Your workspace is ready</h3>
                  <p>Review the setup, then start with the app hidden until onboarding completes.</p>
                </div>
                {!selectedModelReady && (
                  <p className="onboarding-notice error">A selected, reachable chat model is required before setup can finish.</p>
                )}
                <div className="onboarding-summary">
                  <span><strong>Setup path</strong>{pathLabel(onboarding.selectedSetupPath)}</span>
                  <span><strong>Model</strong>{selectedModelReady ? selectedModel : "Not ready"}</span>
                  <span><strong>Memory</strong>{threadSettings.memory ? "On" : "Off"}</span>
                  <span><strong>Privacy</strong>{privacyLabel(threadSettings.privacy)}</span>
                  <span><strong>Folder</strong>{threadSettings.folder || "Not set"}</span>
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
          {step === "finish" ? (
            <button className="btn-accent" type="button" onClick={finish}>
              Start chatting
            </button>
          ) : (
            <button className="btn-accent" type="button" onClick={nextStep}>
              <span>Continue</span>
              <ArrowRight size={14} />
            </button>
          )}
        </div>
      </div>
    </SheetDialog>
  );
}
