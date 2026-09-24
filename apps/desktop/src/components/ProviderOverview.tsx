import type { ProviderDiscovery, ProviderInfo } from "../api";
import {
  ACCOUNT_RUNTIME_KINDS,
  ACCOUNT_RUNTIME_LABEL,
  noteTone,
  providerDetail,
  providerRailGroupLabel,
  providersAttention,
  providerStatus,
  sortProvidersForRail,
  type ProvidersTarget,
  type StatusTone,
} from "../lib/providerConnections";
import { Search } from "./icons";
import { ProviderIcon, providerBrandForProvider, type ProviderBrand } from "./ProviderIcon";
import { StatusDot } from "./ProvidersRail";
import type { AccountRuntimesState } from "./useAccountRuntimes";

const HOSTED_PRESETS: Array<[string, ProviderBrand]> = [
  ["OpenRouter", "openrouter"],
  ["OpenAI", "openai"],
  ["Anthropic", "claude"],
  ["Gemini", "gemini"],
];

type OverviewRow = {
  key: string;
  brand: ProviderBrand | null;
  name: string;
  kind: string;
  tone: StatusTone;
  status: string;
  detail: string;
  target: ProvidersTarget;
  testId: string;
};

function UpdateAllButton({ state }: { state: AccountRuntimesState }) {
  const count = state.updateTargets.length;
  return (
    <button
      className="btn-accent"
      data-testid="account-runtimes-update-all"
      type="button"
      onClick={() => void state.runAllUpdates()}
      disabled={!state.updatesLoaded || count === 0 || state.updating !== null}
    >
      {state.updating === "all" && state.updateProgress
        ? `Updating ${state.updateProgress.current} of ${state.updateProgress.total}...`
        : state.confirmUpdate === "all"
          ? "Confirm update all"
          : "Update all"}
    </button>
  );
}

/**
 * The Providers landing page: at most one attention bar, a compact status
 * list of every connection, and shortcuts for adding providers.
 */
export function ProviderOverview({
  state,
  providers,
  detecting,
  discoveries,
  busy,
  note,
  onSelect,
  onStartPreset,
  onDetectLocal,
  onAddDiscovery,
}: {
  state: AccountRuntimesState;
  providers: readonly ProviderInfo[];
  detecting: boolean;
  discoveries: readonly ProviderDiscovery[];
  busy: boolean;
  note: string | null;
  onSelect: (target: ProvidersTarget) => void;
  onStartPreset: (preset: string) => void;
  onDetectLocal: () => void;
  onAddDiscovery: (discovery: ProviderDiscovery) => void;
}) {
  const attention = providersAttention({
    updateCount: state.updateTargets.length,
    updating: state.updating === "all",
    runtimes: ACCOUNT_RUNTIME_KINDS.map((runtime) => ({ runtime, summary: state.summaries[runtime] })),
    providers,
  });
  const runtimeNotes = Array.from(
    new Map(
      ACCOUNT_RUNTIME_KINDS.flatMap((runtime) => {
        const runtimeNote = state.notes[runtime];
        return runtimeNote ? [[runtimeNote.message, runtimeNote] as const] : [];
      }),
    ).values(),
  );

  const rows: OverviewRow[] = [
    ...ACCOUNT_RUNTIME_KINDS.map((runtime): OverviewRow => {
      const summary = state.summaries[runtime];
      return {
        key: `runtime-${runtime}`,
        brand: runtime,
        name: ACCOUNT_RUNTIME_LABEL[runtime],
        kind: "Coding CLI",
        tone: summary.tone,
        status: summary.label,
        detail: summary.version
          ? `v${summary.version}${summary.updateAvailable ? " · update" : ""}`
          : "",
        target: { view: "runtime", runtime },
        testId: `provider-overview-${runtime}`,
      };
    }),
    ...sortProvidersForRail(providers).flatMap((group) =>
      group.items.map((provider): OverviewRow => {
        const status = providerStatus(provider);
        return {
          key: provider.id,
          brand: providerBrandForProvider(provider),
          name: provider.name,
          kind: providerRailGroupLabel(provider),
          tone: status.tone,
          status: status.label,
          detail: providerDetail(provider),
          target: { view: "provider", id: provider.id },
          testId: `provider-overview-provider-${provider.id}`,
        };
      }),
    ),
  ];

  return (
    <section className="providers-page providers-overview" aria-labelledby="providers-overview-title">
      <h3 className="providers-sr-only" id="providers-overview-title">Overview</h3>
      {attention?.kind === "updates" && (
        <div className="providers-attention" role="status">
          <span className="providers-attention-copy">
            <strong>
              {attention.count
                ? `${attention.count} coding CLI update${attention.count === 1 ? "" : "s"} available`
                : "Updating coding CLIs"}
            </strong>
            <span>
              {state.confirmUpdate === "all"
                ? "Finish active runs first, then confirm."
                : state.updateTargets.map((runtime) => ACCOUNT_RUNTIME_LABEL[runtime]).join(", ")}
            </span>
          </span>
          <UpdateAllButton state={state} />
        </div>
      )}
      {attention?.kind === "item" && (
        <div className={"providers-attention " + attention.tone} role="status">
          <StatusDot tone={attention.tone} />
          <span className="providers-attention-copy">
            <strong>{attention.message}</strong>
          </span>
          <button className="btn-ghost" type="button" onClick={() => onSelect(attention.target)}>
            Review
          </button>
        </div>
      )}
      {runtimeNotes.map((runtimeNote) => (
        <p key={runtimeNote.message} className={"provider-note " + runtimeNote.tone} role="status">
          {runtimeNote.message}
        </p>
      ))}

      <div className="providers-section-head">
        <h4 id="providers-connections-title">Connections</h4>
      </div>
      <ul className="providers-status-table" aria-labelledby="providers-connections-title">
        {rows.map((row) => (
          <li key={row.key}>
            <button
              className="providers-status-row"
              type="button"
              data-testid={row.testId}
              onClick={() => onSelect(row.target)}
            >
              <span className="providers-status-name">
                <ProviderIcon brand={row.brand} size={16} />
                <span>{row.name}</span>
              </span>
              <span className="providers-status-kind">{row.kind}</span>
              <span className="providers-status-state">
                <StatusDot tone={row.tone} />
                <span>{row.status}</span>
              </span>
              <span className="providers-status-detail">{row.detail}</span>
            </button>
          </li>
        ))}
      </ul>

      <div className="providers-section-head">
        <h4 id="provider-quick-title">Add providers</h4>
        <p>Detect a local model server or start a hosted API-key connection.</p>
      </div>
      <div className="provider-quick-row" role="group" aria-labelledby="provider-quick-title">
        <button
          className="provider-quick-chip"
          data-testid="detect-local-providers"
          type="button"
          onClick={onDetectLocal}
          disabled={detecting}
          title="Find Ollama, LM Studio, or vLLM on this machine."
          aria-label={detecting ? "Detecting local providers" : "Detect local providers"}
        >
          <Search size={14} aria-hidden="true" />
          <span>{detecting ? "Detecting local..." : "Detect local"}</span>
        </button>
        {HOSTED_PRESETS.map(([presetName, brand]) => (
          <button
            className="provider-quick-chip"
            type="button"
            key={presetName}
            onClick={() => onStartPreset(presetName)}
            title={`Start ${presetName} provider setup.`}
            aria-label={`Start ${presetName} provider setup`}
          >
            <ProviderIcon brand={brand} size={14} />
            <span>{presetName}</span>
          </button>
        ))}
      </div>

      {discoveries.length > 0 && (
        <div className="provider-discovery">
          <div className="provider-discovery-head">
            <span className="setting-mini-title">Local providers</span>
            <span>Reachable endpoints can be added without pasting a key.</span>
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
                <button className="btn-ghost" type="button" disabled={busy} onClick={() => onAddDiscovery(d)}>
                  Add
                </button>
              ) : (
                <span className="provider-pill muted">Start app</span>
              )}
            </div>
          ))}
        </div>
      )}

      {note && <p className={"provider-note " + noteTone(note)}>{note}</p>}
    </section>
  );
}
