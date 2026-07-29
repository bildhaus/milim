import { useMemo, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { ModelInfo, ProviderInfo } from "../api";
import {
  isModelPickerGroupCollapsed,
  modelDevProfile,
  modelDisplayName,
  modelPickerGroups,
  modelPickerKey,
  type ModelDevCapability,
} from "../lib/modelPicker";
import { hasReasoningEffortChoices, normalizeReasoningEffortForModel, REASONING_EFFORT_LABEL, reasoningEffortDisplay, reasoningEffortOptions } from "../lib/reasoningEffort";
import { useSettings } from "../settings/store";
import { HoverScrollText } from "./HoverScrollText";
import { Bolt, Check, ChevronDown, Eye, Image, Plug, PlusSquare, Search, Sparkles, Volume2 } from "./icons";
import { ProviderIcon, providerBrandForModel } from "./ProviderIcon";

function Star({ filled }: { filled: boolean }) {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.6" strokeLinejoin="round">
      <path d="M12 3.5l2.6 5.6 5.9.8-4.3 4 1 6L12 17.2 6.8 19.9l1-6-4.3-4 5.9-.8z" />
    </svg>
  );
}

function Memory({ size = 13 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <ellipse cx="12" cy="5" rx="7" ry="3" />
      <path d="M5 5v6c0 1.7 3.1 3 7 3s7-1.3 7-3V5M5 11v6c0 1.7 3.1 3 7 3s7-1.3 7-3v-6" />
    </svg>
  );
}

const CAP_ICON: Record<ModelDevCapability, { node: JSX.Element; title: string }> = {
  vision: { node: <Eye size={11} />, title: "Vision" },
  tools: { node: <Plug size={11} />, title: "Tool use" },
  reasoning: { node: <Sparkles size={11} />, title: "Reasoning" },
  fast: { node: <Bolt size={11} />, title: "Fast" },
  image: { node: <Image size={11} />, title: "Image output" },
  video: { node: <Sparkles size={11} />, title: "Video output" },
  music: { node: <Volume2 size={11} />, title: "Music output" },
} as const;

type EffortMenuState = { modelId: string; left: number; top: number };
export type ModelPickerSelection = {
  model: string;
};

function effortMenuPosition(modelId: string, button: HTMLElement, choiceCount: number): EffortMenuState {
  const edge = 8;
  const gap = 4;
  const width = 190;
  const height = choiceCount * 28 + 8;
  const rect = button.getBoundingClientRect();
  const left = Math.max(edge, Math.min(window.innerWidth - width - edge, rect.right - width));
  const below = rect.bottom + gap;
  const top = below + height > window.innerHeight - edge ? Math.max(edge, rect.top - gap - height) : below;
  return { modelId, left, top };
}

export function ModelPicker({
  models,
  model,
  providers,
  toolIntent,
  planMode,
  onModel,
  onManageProviders,
  onManageMcp,
  onManageMemory,
  onClose,
  showManagementActions = true,
  favoriteIds,
  favoritesOnlyValue,
  onToggleFavorite,
  onFavoritesOnlyChange,
  searchPlaceholder,
  emptyMessage,
  ariaLabel = "Choose a model",
  style,
}: {
  models: ModelInfo[];
  model: string;
  providers?: ProviderInfo[];
  toolIntent?: boolean;
  planMode?: boolean;
  onModel: (selection: ModelPickerSelection) => void;
  onManageProviders?: () => void;
  onManageMcp?: () => void;
  onManageMemory?: () => void;
  onClose: () => void;
  showManagementActions?: boolean;
  favoriteIds?: string[];
  favoritesOnlyValue?: boolean;
  onToggleFavorite?: (modelId: string) => void;
  onFavoritesOnlyChange?: (favoritesOnly: boolean) => void;
  searchPlaceholder?: string;
  emptyMessage?: string;
  ariaLabel?: string;
  style?: CSSProperties;
}) {
  const storedFavorites = useSettings((s) => s.favorites);
  const storedFavoritesOnly = useSettings((s) => s.favoritesOnly);
  const collapsedModelGroups = useSettings((s) => s.collapsedModelGroups);
  const toggleFavorite = useSettings((s) => s.toggleFavorite);
  const setFavoritesOnly = useSettings((s) => s.setFavoritesOnly);
  const setModelGroupCollapsed = useSettings((s) => s.setModelGroupCollapsed);
  const reasoningEffortByModel = useSettings((s) => s.reasoningEffortByModel);
  const setModelReasoningEffort = useSettings((s) => s.setModelReasoningEffort);
  const [q, setQ] = useState("");
  const [effortMenu, setEffortMenu] = useState<EffortMenuState | null>(null);
  const favorites = favoriteIds ?? storedFavorites;
  const favoritesOnly = favoritesOnlyValue ?? storedFavoritesOnly;
  const applyFavorite = onToggleFavorite ?? toggleFavorite;

  const groups = useMemo<Array<[string, ModelInfo[]]>>(() => {
    return modelPickerGroups(models, favorites, favoritesOnly, q, providers);
  }, [models, q, favorites, favoritesOnly, providers]);
  const filtering = Boolean(q.trim()) || favoritesOnly;
  return (
    <div
      className="mp"
      role="dialog"
      aria-label={ariaLabel}
      style={style}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
    >
      <div className="mp-search">
        <Search size={14} />
        <input
          autoFocus
          aria-label="Search models"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={favoritesOnly ? "Search favorites..." : searchPlaceholder ?? "Search models..."}
        />
      </div>
      <div className="mp-list" onScroll={() => effortMenu && setEffortMenu(null)}>
        <>
            {groups.length === 0 && (
              <div className="mp-empty" role="status">
                {favoritesOnly ? "No favorites yet." : emptyMessage ?? "No models - add a provider."}
              </div>
            )}
            {groups.map(([label, ms]) => {
              const collapsible = label !== "Favorites";
              const collapsed = isModelPickerGroupCollapsed(label, collapsedModelGroups, filtering);
              const brand = collapsible ? providerBrandForModel(ms[0], providers) : null;
              return (
                <div key={label} className="mp-group">
                  {collapsible ? (
                    <button
                      type="button"
                      className="mp-group-head mp-group-toggle"
                      aria-expanded={!collapsed}
                      aria-label={`${collapsed ? "Expand" : "Collapse"} ${label} models`}
                      disabled={filtering}
                      onClick={() => setModelGroupCollapsed(label, !collapsed)}
                    >
                      <span className="mp-group-label">
                        <ProviderIcon brand={brand} />
                        <span>{label}</span>
                      </span>
                      <ChevronDown size={12} />
                    </button>
                  ) : (
                    <div className="mp-group-head">{label}</div>
                  )}
                  {!collapsed && ms.map((m) => {
                    const hasEffortChoices = hasReasoningEffortChoices(m);
                    const effort = normalizeReasoningEffortForModel(reasoningEffortByModel[m.id] ?? "auto", m);
                    const reasoningChoices = reasoningEffortOptions(m);
                    const effortOpen = effortMenu?.modelId === m.id;
                    const profile = modelDevProfile(m, m.id, {
                      providers,
                      toolIntent,
                      planMode,
                    });
                    const rowBrand = providerBrandForModel(m, providers);
                    const modelCaps = profile.capabilities.filter((cap) => cap !== "reasoning" || !hasEffortChoices);
                    return (
                      <div key={modelPickerKey(m)} className={"mp-item" + (m.id === model ? " active" : "") + (effortOpen ? " effort-open" : "")}>
                  <button
                    type="button"
                    className={"mp-star" + (favorites.includes(m.id) ? " on" : "")}
                    title={favorites.includes(m.id) ? "Unfavorite" : "Favorite"}
                    aria-label={favorites.includes(m.id) ? `Remove ${m.id} from favorites` : `Add ${m.id} to favorites`}
                    aria-pressed={favorites.includes(m.id)}
                    onClick={() => applyFavorite(m.id)}
                  >
                    <Star filled={favorites.includes(m.id)} />
                  </button>
                  <button
                    type="button"
                    className="mp-pick"
                    title={`${profile.routeLabel}. ${profile.routeDetail} ${profile.setupDetail}`}
                    aria-label={`${modelDisplayName(m)}, ${profile.routeLabel}, ${profile.laneLabel}, ${profile.setupLabel}`}
                    onClick={() => {
                      onModel({ model: m.id });
                      onClose();
                    }}
                  >
                    <span className="mp-title">
                      <ProviderIcon brand={rowBrand} className="mp-row-provider" size={13} />
                      <HoverScrollText className="mp-name" text={modelDisplayName(m)} />
                      {effort !== "auto" && <span className="mp-effort-label">{REASONING_EFFORT_LABEL[effort]}</span>}
                      <span className="mp-route" title={profile.routeLabel}>{profile.routeLabel}</span>
                    </span>
                  </button>
                  {(modelCaps.length > 0 || hasEffortChoices) && (
                    <div className="mp-caps">
                      {modelCaps.map((c) => (
                        <span key={c} className="mp-cap" title={CAP_ICON[c].title}>
                          {CAP_ICON[c].node}
                        </span>
                      ))}
                      {hasEffortChoices && (
                        <div className="mp-effort-wrap">
                          <button
                            type="button"
                            className={"mp-effort-btn" + (effort !== "auto" ? " on" : "") + (effortOpen ? " open" : "")}
                            title="Reasoning effort"
                            aria-label={`Reasoning effort for ${m.id}: ${REASONING_EFFORT_LABEL[effort]}`}
                            aria-haspopup="menu"
                            aria-expanded={effortOpen}
                            onClick={(event) => {
                              const nextMenu = effortMenuPosition(m.id, event.currentTarget, reasoningChoices.length);
                              setEffortMenu((open) => open?.modelId === m.id ? null : nextMenu);
                            }}
                          >
                            <Sparkles size={11} fill={effort !== "auto" ? "currentColor" : "none"} />
                          </button>
                          {effortOpen && effortMenu && createPortal(
                            <div className="mp-effort-menu" data-native-preview-blocker="true" role="menu" aria-label={`Reasoning effort for ${m.id}`} style={{ left: effortMenu.left, top: effortMenu.top }}>
                              {reasoningChoices.map((choice) => {
                                const display = reasoningEffortDisplay(choice, m);
                                return (
                                  <button
                                    key={choice}
                                    type="button"
                                    className={"mp-effort-choice" + (choice === effort ? " on" : "")}
                                    role="menuitemradio"
                                    aria-checked={choice === effort}
                                    onClick={() => {
                                      setModelReasoningEffort(m.id, choice);
                                      setEffortMenu(null);
                                    }}
                                  >
                                    <span className="mp-effort-check">{choice === effort && <Check size={11} />}</span>
                                    <span>{display.label}</span>
                                    <span>{display.detail}</span>
                                  </button>
                                );
                              })}
                            </div>,
                            document.body,
                          )}
                        </div>
                      )}
                    </div>
                  )}
                      </div>
                    );
                  })}
                </div>
              );
            })}
        </>
      </div>
      <div className="mp-foot">
        <button type="button" className={"mp-foot-toggle" + (favoritesOnly ? " on" : "")} aria-pressed={favoritesOnly} onClick={() => onFavoritesOnlyChange ? onFavoritesOnlyChange(!favoritesOnly) : setFavoritesOnly(!favoritesOnly)}>
          <span className="mp-checkbox">{favoritesOnly && <Check size={10} />}</span>
          Favorites only
        </button>
        {showManagementActions && (onManageProviders || onManageMcp || onManageMemory) && <div className="mp-foot-actions">
          {onManageProviders && <button type="button" className="mp-foot-btn" data-testid="manage-providers" onClick={() => { onClose(); onManageProviders(); }}>
            <PlusSquare size={13} /> Providers
          </button>}
          {onManageMcp && <button type="button" className="mp-foot-btn" onClick={() => { onClose(); onManageMcp(); }}>
            <Plug size={13} /> MCP
          </button>}
          {onManageMemory && <button type="button" className="mp-foot-btn" onClick={() => { onClose(); onManageMemory(); }}>
            <Memory size={13} /> Memory
          </button>}
        </div>}
      </div>
    </div>
  );
}
