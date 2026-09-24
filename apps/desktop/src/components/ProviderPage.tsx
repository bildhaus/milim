import { useState } from "react";
import {
  deleteProvider,
  PROVIDER_PRESETS,
  saveProvider,
  verifyProviderModelCapabilities,
  type ModelCapabilityOverride,
  type ModelCapabilityVerification,
  type ProviderInfo,
  type ProviderKind,
} from "../api";
import {
  isMediaOnlyProvider,
  isMediaProvider,
  KIND_LABEL,
  modelCount,
  noteTone,
  PROVIDER_KIND_OPTIONS,
  providerKeyStatus,
  providerNeedsKey,
  providerRailGroupLabel,
  providerStatus,
} from "../lib/providerConnections";
import { confirmApp } from "../ui/confirmation";
import type { ContextMenuItem } from "./ContextMenu";
import { Copy, Refresh, Trash } from "./icons";
import { OverflowMenuButton } from "./OverflowMenuButton";
import { ProviderIcon, providerBrandForProvider } from "./ProviderIcon";
import { StatusDot } from "./ProvidersRail";
import { Select, Toggle } from "./ui";

const MODEL_PREVIEW_LIMIT = 8;

/**
 * Edit one hosted, local, or media provider, or draft a new one. The parent
 * keys this component by editing session, so saving a draft keeps its state
 * and result note while the rail moves to the saved provider.
 */
export function ProviderPage({
  provider,
  preset,
  initialNote = null,
  onSaved,
  onDeleted,
  onRefreshList,
}: {
  provider: ProviderInfo | null;
  preset?: string;
  initialNote?: string | null;
  onSaved: (saved: ProviderInfo) => Promise<void> | void;
  onDeleted: () => Promise<void> | void;
  onRefreshList: () => Promise<unknown> | void;
}) {
  const presetInfo = !provider && preset ? PROVIDER_PRESETS.find((p) => p.name === preset) : undefined;
  const [name, setName] = useState(provider?.name ?? presetInfo?.name ?? "");
  const [kind, setKind] = useState<ProviderKind>(provider?.kind ?? presetInfo?.kind ?? "openai_compatible");
  const [baseUrl, setBaseUrl] = useState(provider?.base_url ?? presetInfo?.base_url ?? "");
  const [apiKey, setApiKey] = useState("");
  const [enabled, setEnabled] = useState(provider?.enabled ?? true);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(initialNote);
  const [modelOverrides, setModelOverrides] = useState<Record<string, ModelCapabilityOverride>>(
    provider?.model_overrides ?? {},
  );
  const [capabilityModel, setCapabilityModel] = useState(provider?.models[0] ?? "");
  const [verification, setVerification] = useState<ModelCapabilityVerification | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [showAllModels, setShowAllModels] = useState(false);

  const isNew = !provider;
  const selectedPreset = PROVIDER_PRESETS.find(
    (p) =>
      p.kind === kind &&
      p.name === name &&
      p.base_url.trim().replace(/\/+$/, "") === baseUrl.trim().replace(/\/+$/, ""),
  );
  const draft = { name, kind, base_url: baseUrl };
  const draftNeedsKey = providerNeedsKey(draft);
  const isDirty = !provider
    ? Boolean(name.trim() || baseUrl.trim() || apiKey.trim())
    : name !== provider.name ||
      kind !== provider.kind ||
      baseUrl !== provider.base_url ||
      enabled !== provider.enabled ||
      apiKey.length > 0 ||
      JSON.stringify(modelOverrides) !== JSON.stringify(provider.model_overrides ?? {});
  const canSave = Boolean(name.trim() && baseUrl.trim() && !busy);
  const status = provider ? providerStatus(provider) : null;
  const keyStatus = provider ? providerKeyStatus(provider) : null;

  function applyPreset(presetName: string) {
    const p = PROVIDER_PRESETS.find((x) => x.name === presetName);
    if (!p) return;
    setName(p.name);
    setKind(p.kind);
    setBaseUrl(p.base_url);
  }

  async function persist(action: "save" | "test") {
    if (!name.trim() || !baseUrl.trim()) return;
    setBusy(true);
    setNote(null);
    const saved = await saveProvider({
      id: provider?.id,
      name: name.trim(),
      kind,
      base_url: baseUrl.trim(),
      api_key: apiKey || undefined,
      enabled,
      model_overrides: modelOverrides,
    });
    setBusy(false);
    if (!saved) {
      setNote("Error: Failed to save provider.");
      return;
    }
    await onSaved(saved);
    setName(saved.name);
    setKind(saved.kind);
    setBaseUrl(saved.base_url);
    setApiKey("");
    setEnabled(saved.enabled);
    setModelOverrides(saved.model_overrides ?? {});
    setCapabilityModel((current) => (saved.models.includes(current) ? current : (saved.models[0] ?? "")));
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

  async function remove() {
    if (!provider) return;
    const confirmed = await confirmApp({
      title: `Delete ${provider.name}?`,
      message: "milim removes this provider and its encrypted key from this device. Its models leave the picker.",
      confirmLabel: "Delete provider",
      tone: "danger",
    });
    if (!confirmed) return;
    setBusy(true);
    try {
      await deleteProvider(provider.id);
      await onDeleted();
    } finally {
      setBusy(false);
    }
  }

  function setCapabilityOverride(
    model: string,
    key: "image_input" | "tool_use" | "reasoning",
    value: "auto" | "yes" | "no",
  ) {
    setModelOverrides((current) => {
      const next = { ...current };
      const entry = { ...(next[model] ?? {}) };
      if (value === "auto") delete entry[key];
      else entry[key] = value === "yes";
      if (Object.keys(entry).length) next[model] = entry;
      else delete next[model];
      return next;
    });
  }

  async function verifyCapabilities() {
    if (!provider || !capabilityModel || verifying) return;
    setVerifying(true);
    setVerification(null);
    const result = await verifyProviderModelCapabilities(provider.id, capabilityModel);
    setVerifying(false);
    if (!result) {
      setNote("Error: Capability verification failed before results were returned.");
      return;
    }
    setVerification(result);
  }

  function applyVerification() {
    if (!verification) return;
    const results = [
      ["image_input", verification.vision],
      ["reasoning", verification.reasoning],
      ["tool_use", verification.tools],
    ] as const;
    for (const [key, result] of results) {
      if (!result.error) setCapabilityOverride(verification.model, key, result.supported ? "yes" : "no");
    }
    setNote("Capability results applied as unsaved overrides. Save changes to keep them.");
  }

  const menu: ContextMenuItem[] = [
    {
      id: "refresh-list",
      label: "Refresh list",
      icon: <Refresh size={13} />,
      action: async () => {
        await onRefreshList();
      },
    },
  ];
  if (baseUrl.trim()) {
    menu.push({
      id: "copy-url",
      label: "Copy base URL",
      icon: <Copy size={13} />,
      action: () => {
        void navigator.clipboard.writeText(baseUrl.trim());
      },
    });
  }
  if (provider) {
    menu.push({
      id: "delete",
      label: "Delete provider...",
      icon: <Trash size={13} />,
      danger: true,
      separatorBefore: true,
      disabled: busy,
      action: () => remove(),
    });
  }

  const facts: string[] = [];
  if (provider) {
    if (!isMediaOnlyProvider(provider) && provider.models.length) facts.push(modelCount(provider.models.length));
    if (keyStatus) facts.push(keyStatus.label);
  }
  const models = provider?.models ?? [];
  const visibleModels = showAllModels ? models : models.slice(0, MODEL_PREVIEW_LIMIT);
  const showCapabilities = Boolean(provider && models.length && !isMediaOnlyProvider(provider));

  return (
    <section className="providers-page provider-page" aria-labelledby="provider-page-title">
      <header className="providers-page-head">
        <ProviderIcon brand={providerBrandForProvider(draft)} size={28} />
        <div className="providers-page-title">
          <span className="providers-page-kicker">
            {!provider ? "New connection" : `${providerRailGroupLabel(provider)} · ${KIND_LABEL[provider.kind]}`}
          </span>
          <h3 id="provider-page-title">{name.trim() || "Untitled provider"}</h3>
          <p className="providers-status-line" title={status?.detail}>
            {status ? (
              <>
                <StatusDot tone={status.tone} />
                <span className="providers-status-label">{status.label}</span>
                {facts.map((fact) => (
                  <span key={fact} className="providers-status-fact">{fact}</span>
                ))}
              </>
            ) : (
              <span className="providers-status-fact muted">Choose a preset or enter the endpoint details.</span>
            )}
          </p>
        </div>
        <div className="providers-page-actions">
          <OverflowMenuButton label={`${name.trim() || "Provider"} actions`} items={menu} testId="provider-actions" />
          <Toggle
            checked={enabled}
            onChange={setEnabled}
            label="Enabled"
            ariaLabel="Enable provider (applies on save)"
            testId="provider-enabled-toggle"
          />
        </div>
      </header>
      {status?.tone === "error" && <p className="provider-note error">{status.detail}</p>}

      <div className="providers-form-section">
        <h4>Connection</h4>
        <div className="providers-field-grid three">
          <label className="field provider-field">
            <span>Preset</span>
            <Select
              value={selectedPreset?.name ?? ""}
              testId="provider-preset-select"
              placeholder="Choose a preset..."
              options={PROVIDER_PRESETS.map((p) => ({ label: p.name, value: p.name }))}
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
            <span>Type</span>
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
        <label className="field provider-field">
          <span>
            API key{" "}
            <em>
              {!isNew
                ? "Leave blank to keep the encrypted key on this device"
                : draftNeedsKey
                  ? "Required by most hosted providers"
                  : "Usually not needed for local endpoints"}
            </em>
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
      </div>

      <div className="providers-form-section">
        <div className="providers-form-section-head">
          <h4>Models</h4>
          <button
            className="btn-ghost providers-small-button"
            data-testid="provider-test-connection"
            type="button"
            disabled={!canSave}
            onClick={() => void persist("test")}
          >
            <Refresh size={13} aria-hidden="true" />
            <span>{busy ? "Testing..." : "Test connection"}</span>
          </button>
        </div>
        {models.length ? (
          <div className="provider-model-list" aria-label="Provider models">
            {visibleModels.map((model) => (
              <span key={model} title={model}>{model}</span>
            ))}
            {models.length > MODEL_PREVIEW_LIMIT && (
              <button
                className="providers-text-button"
                type="button"
                aria-expanded={showAllModels}
                onClick={() => setShowAllModels((value) => !value)}
              >
                {showAllModels ? "Show fewer" : `+${models.length - MODEL_PREVIEW_LIMIT} more`}
              </button>
            )}
          </div>
        ) : (
          <p className="runtime-muted">
            {isNew
              ? "Models appear after you save and test the connection."
              : "No cached models yet. Test the connection to populate this provider."}
          </p>
        )}
        {showCapabilities && provider && (
          <details className="provider-model-capabilities">
            <summary>
              <strong>Per-model capabilities</strong>
              <span>Auto uses discovery. Overrides survive model refreshes.</span>
            </summary>
            <div className="provider-capabilities-body">
              <select
                className="css-input"
                aria-label="Capability model"
                value={capabilityModel}
                onChange={(event) => {
                  setCapabilityModel(event.currentTarget.value);
                  setVerification(null);
                }}
              >
                {models.map((model) => <option key={model} value={model}>{model}</option>)}
              </select>
              <div className="provider-capability-overrides">
                {([
                  ["image_input", "Vision", provider.model_capabilities?.[capabilityModel]?.image_input],
                  ["reasoning", "Reasoning", Boolean(provider.model_reasoning?.[capabilityModel])],
                  ["tool_use", "Tools", provider.model_capabilities?.[capabilityModel]?.tool_use],
                ] as const).map(([key, label, detected]) => {
                  const override = modelOverrides[capabilityModel]?.[key];
                  const value = override == null ? "auto" : override ? "yes" : "no";
                  return (
                    <label key={key}>
                      <span>{label}<small>Detected: {detected == null ? "unknown" : detected ? "yes" : "no"}</small></span>
                      <select
                        className="css-input"
                        value={value}
                        onChange={(event) => setCapabilityOverride(
                          capabilityModel,
                          key,
                          event.currentTarget.value as "auto" | "yes" | "no",
                        )}
                      >
                        <option value="auto">Auto</option>
                        <option value="yes">Yes</option>
                        <option value="no">No</option>
                      </select>
                    </label>
                  );
                })}
              </div>
              <div className="provider-capability-actions">
                <button className="btn-ghost providers-small-button" type="button" disabled={verifying} onClick={() => void verifyCapabilities()}>
                  <Refresh size={13} aria-hidden="true" />
                  <span>{verifying ? "Running 3 probes..." : "Verify vision, reasoning, and tools"}</span>
                </button>
                {verification && <button className="btn-ghost providers-small-button" type="button" onClick={applyVerification}>Apply reliable results</button>}
              </div>
              {verification && (
                <div className="provider-probe-results" aria-live="polite">
                  {(["vision", "reasoning", "tools"] as const).map((key) => {
                    const result = verification[key];
                    return <span key={key} className={result.supported ? "ready" : result.error ? "error" : "off"} title={result.error}>{key}: {result.supported ? "yes" : result.error ? "error" : "no"}</span>;
                  })}
                </div>
              )}
            </div>
          </details>
        )}
      </div>

      {note && <p className={"provider-note " + noteTone(note)} role="status">{note}</p>}

      <div className="providers-footer-actions">
        <span className="providers-save-state">
          {isNew ? "Draft" : isDirty ? "Unsaved changes" : "All changes saved"}
        </span>
        <span className="spacer" />
        <button
          className="btn-accent"
          data-testid="save-provider"
          type="button"
          disabled={!canSave || (!isNew && !isDirty)}
          onClick={() => void persist("save")}
        >
          {busy ? "Connecting..." : isNew ? "Save and test" : isDirty ? "Save changes" : "Saved"}
        </button>
      </div>
    </section>
  );
}
