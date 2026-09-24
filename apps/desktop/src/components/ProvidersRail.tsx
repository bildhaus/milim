import type { KeyboardEvent } from "react";
import type { AccountRuntimeKind, ProviderInfo } from "../api";
import {
  ACCOUNT_RUNTIME_KINDS,
  ACCOUNT_RUNTIME_LABEL,
  providerRailHint,
  providerStatus,
  sortProvidersForRail,
  type ProvidersTarget,
  type RuntimeSummary,
  type StatusTone,
} from "../lib/providerConnections";
import { ProviderIcon, providerBrandForProvider, type ProviderBrand } from "./ProviderIcon";

export function StatusDot({ tone }: { tone: StatusTone }) {
  return <span className={"provider-status-dot " + tone} aria-hidden="true" />;
}

function RailRow({
  active,
  brand,
  name,
  tone,
  statusLabel,
  hint,
  muted = false,
  testId,
  onClick,
}: {
  active: boolean;
  brand: ProviderBrand | null;
  name: string;
  tone: StatusTone;
  statusLabel: string;
  hint: string | null;
  muted?: boolean;
  testId: string;
  onClick: () => void;
}) {
  return (
    <li>
      <button
        className={"providers-rail-row" + (active ? " active" : "") + (muted ? " muted" : "")}
        type="button"
        aria-current={active ? "page" : undefined}
        data-testid={testId}
        onClick={onClick}
      >
        <ProviderIcon brand={brand} size={16} />
        <span className="providers-rail-name">{name}</span>
        <span className="providers-rail-status">
          <StatusDot tone={tone} />
          <span className="providers-sr-only">{statusLabel}</span>
          {hint && <span className="providers-rail-hint">{hint}</span>}
        </span>
      </button>
    </li>
  );
}

/** Moves focus between rail rows with the arrow, Home, and End keys. */
function onRailKeyDown(event: KeyboardEvent<HTMLElement>) {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const rows = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>(".providers-rail-row"));
  const current = rows.indexOf(document.activeElement as HTMLButtonElement);
  if (current < 0) return;
  event.preventDefault();
  const next =
    event.key === "Home"
      ? 0
      : event.key === "End"
        ? rows.length - 1
        : event.key === "ArrowDown"
          ? Math.min(rows.length - 1, current + 1)
          : Math.max(0, current - 1);
  rows[next]?.focus();
}

/**
 * The Providers rail: Overview, then every coding CLI (installed or not),
 * then saved providers grouped as Hosted, Local, and Media.
 */
export function ProvidersRail({
  target,
  runtimes,
  providers,
  onSelect,
}: {
  target: ProvidersTarget;
  runtimes: Record<AccountRuntimeKind, RuntimeSummary>;
  providers: readonly ProviderInfo[];
  onSelect: (target: ProvidersTarget) => void;
}) {
  const groups = sortProvidersForRail(providers);
  return (
    <nav className="providers-rail" aria-label="Provider connections" onKeyDown={onRailKeyDown}>
      <ul className="providers-rail-list">
        <li>
          <button
            className={"providers-rail-row overview" + (target.view === "overview" ? " active" : "")}
            type="button"
            aria-current={target.view === "overview" ? "page" : undefined}
            data-testid="provider-overview"
            onClick={() => onSelect({ view: "overview" })}
          >
            <span className="providers-rail-name">Overview</span>
          </button>
        </li>
      </ul>
      <div className="providers-rail-group">
        <h3 className="providers-rail-label" id="providers-rail-clis">Coding CLIs</h3>
        <ul className="providers-rail-list" aria-labelledby="providers-rail-clis">
          {ACCOUNT_RUNTIME_KINDS.map((runtime) => {
            const summary = runtimes[runtime];
            return (
              <RailRow
                key={runtime}
                active={target.view === "runtime" && target.runtime === runtime}
                brand={runtime}
                name={ACCOUNT_RUNTIME_LABEL[runtime]}
                tone={summary.tone}
                statusLabel={summary.label}
                hint={summary.hint}
                muted={summary.label === "Not installed" || summary.label === "Disabled"}
                testId={`provider-rail-${runtime}`}
                onClick={() => onSelect({ view: "runtime", runtime })}
              />
            );
          })}
        </ul>
      </div>
      {groups.map((group) => (
        <div className="providers-rail-group" key={group.group}>
          <h3 className="providers-rail-label" id={`providers-rail-${group.group}`}>{group.label}</h3>
          <ul className="providers-rail-list" aria-labelledby={`providers-rail-${group.group}`}>
            {group.items.map((provider) => {
              const status = providerStatus(provider);
              return (
                <RailRow
                  key={provider.id}
                  active={target.view === "provider" && target.id === provider.id}
                  brand={providerBrandForProvider(provider)}
                  name={provider.name}
                  tone={status.tone}
                  statusLabel={status.label}
                  hint={providerRailHint(provider)}
                  muted={!provider.enabled}
                  testId={`provider-rail-provider-${provider.id}`}
                  onClick={() => onSelect({ view: "provider", id: provider.id })}
                />
              );
            })}
          </ul>
        </div>
      ))}
      {groups.length === 0 && (
        <p className="providers-rail-empty">No hosted, local, or media providers yet.</p>
      )}
    </nav>
  );
}
