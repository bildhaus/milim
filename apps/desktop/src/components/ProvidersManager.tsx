// Shared form and sheet styles; Providers may open before any other manager.
import "../settings.css";
import { useEffect, useRef, useState } from "react";
import {
  discoverLocalProviders,
  listProviders,
  saveProvider,
  type ProviderDiscovery,
  type ProviderInfo,
} from "../api";
import { MANAGER_DETAIL_MIN_WIDTH } from "../lib/paneSizes";
import type { ProvidersTarget } from "../lib/providerConnections";
import { useSplitPane } from "../ui/usePaneResize";
import { AccountRuntimeImportDialog } from "./AccountRuntimeImportDialog";
import { Plus, X } from "./icons";
import { PaneResizeHandle } from "./PaneResizeHandle";
import { ProviderOverview } from "./ProviderOverview";
import { ProviderPage } from "./ProviderPage";
import { ProvidersRail } from "./ProvidersRail";
import { RuntimePage } from "./RuntimePage";
import { SheetDialog } from "./SheetDialog";
import { useAccountRuntimes } from "./useAccountRuntimes";
import "./ProvidersManager.css";

export type { ProvidersTarget };

/**
 * Providers: a rail of every connection (Overview, Coding CLIs, Hosted, Local,
 * Media) beside one page for the selected item. `initialTarget` deep-links to
 * a runtime, a saved provider, or a new provider draft.
 */
export function ProvidersManager({
  onClose,
  initialTarget,
}: {
  onClose: () => void;
  initialTarget?: ProvidersTarget;
}) {
  // The providers body separates rail and detail with an 18px gap.
  const rail = useSplitPane("providersRail", "--manager-rail-width", MANAGER_DETAIL_MIN_WIDTH + 18);
  const runtimes = useAccountRuntimes();
  const detailRef = useRef<HTMLElement>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providersLoaded, setProvidersLoaded] = useState(false);
  const [target, setTarget] = useState<ProvidersTarget>(initialTarget ?? { view: "overview" });
  // A new key starts a fresh provider editing session; saving keeps the key.
  const [editorKey, setEditorKey] = useState(0);
  const [editorNote, setEditorNote] = useState<string | null>(null);
  const [discoveries, setDiscoveries] = useState<ProviderDiscovery[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [importingRuntime, setImportingRuntime] = useState<"codex" | "claude" | null>(null);

  const refresh = () =>
    listProviders().then((next) => {
      setProviders(next);
      setProvidersLoaded(true);
      return next;
    });

  useEffect(() => {
    void refresh();
  }, []);

  function select(next: ProvidersTarget) {
    setTarget(next);
    setNote(null);
    setEditorNote(null);
    setEditorKey((key) => key + 1);
    detailRef.current?.scrollTo({ top: 0 });
  }

  async function detectLocal() {
    setDetecting(true);
    setNote(null);
    const found = await discoverLocalProviders();
    setDiscoveries(found);
    setDetecting(false);
    if (found.length === 0) {
      setNote("No local provider probes completed. Check that the desktop backend is running.");
    }
  }

  async function addDiscovery(discovery: ProviderDiscovery) {
    setBusy(true);
    setNote(null);
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
    select({ view: "provider", id: saved.id });
    setEditorNote(
      saved.models.length
        ? `Connected - ${saved.models.length} models available`
        : saved.error
          ? `Error: Couldn't reach provider: ${saved.error}`
          : "Saved, but no models returned - check the local server.",
    );
  }

  const selectedProvider =
    target.view === "provider" ? providers.find((provider) => provider.id === target.id) ?? null : null;

  // A deep link or deletion can point at a provider that no longer exists.
  useEffect(() => {
    if (target.view === "provider" && providersLoaded && !selectedProvider) setTarget({ view: "overview" });
  }, [providersLoaded, selectedProvider, target]);

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
      className="sheet providers-sheet"
      resizable={{ id: "providers" }}
      onClose={onClose}
    >
      <div className="sheet-header providers-header">
        <div className="providers-title">
          <h2>Providers</h2>
          <p className="sheet-sub providers-subtitle">
            Connect hosted providers, local model servers, and coding CLIs.
            Keys stay encrypted on this device.
          </p>
        </div>
        <div className="providers-header-actions">
          <button
            className="btn-ghost providers-add-button"
            data-testid="new-provider"
            type="button"
            onClick={() => select({ view: "new" })}
          >
            <Plus size={14} aria-hidden="true" />
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

      <div ref={rail.containerRef} className="providers-body" style={rail.style}>
        <PaneResizeHandle resize={rail.resize} className="manager-rail-resize-handle" data-testid="providers-rail-resize-handle" />
        <ProvidersRail
          target={target}
          runtimes={runtimes.summaries}
          providers={providers}
          onSelect={select}
        />

        <main className="providers-detail" ref={detailRef}>
          {target.view === "overview" && (
            <ProviderOverview
              state={runtimes}
              providers={providers}
              detecting={detecting}
              discoveries={discoveries}
              busy={busy}
              note={note}
              onSelect={select}
              onStartPreset={(preset) => select({ view: "new", preset })}
              onDetectLocal={() => void detectLocal()}
              onAddDiscovery={(discovery) => void addDiscovery(discovery)}
            />
          )}
          {target.view === "runtime" && (
            <RuntimePage
              key={target.runtime}
              runtime={target.runtime}
              state={runtimes}
              onImportChats={setImportingRuntime}
            />
          )}
          {(target.view === "new" || selectedProvider) && (
            <ProviderPage
              key={editorKey}
              provider={selectedProvider}
              preset={target.view === "new" ? target.preset : undefined}
              initialNote={editorNote}
              onSaved={async (saved) => {
                await refresh();
                setTarget({ view: "provider", id: saved.id });
              }}
              onDeleted={async () => {
                await refresh();
                select({ view: "overview" });
              }}
              onRefreshList={refresh}
            />
          )}
        </main>
      </div>
    </SheetDialog>
  );
}
