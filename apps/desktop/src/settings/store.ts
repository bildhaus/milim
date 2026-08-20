import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import {
  DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT,
  type AccountRuntimeEnablement,
  type AccountRuntimeKind,
  type DelegationPolicy,
  type PrivacyMode,
  type ReasoningEffort,
} from "../api.js";
import { userStateStorage } from "../persistence/userStateStorage.js";
import { isReasoningEffort, reasoningEffortByModelWithSelection } from "../lib/reasoningEffort.js";
import type { PreviewBrowserStorageMode } from "../lib/previewWebview.js";

export interface MediaSettings {
  providerId: string;
  modelByProvider: Record<string, string>;
  parametersByProviderModel: Record<string, Record<string, unknown>>;
  advancedByProviderModel: Record<string, string>;
  favoriteModelIdsByProvider: Record<string, string[]>;
  modelSearchByProvider: Record<string, string>;
}

export const DEFAULT_MEDIA_SETTINGS: MediaSettings = {
  providerId: "",
  modelByProvider: {},
  parametersByProviderModel: {},
  advancedByProviderModel: {},
  favoriteModelIdsByProvider: {},
  modelSearchByProvider: {},
};

export type NewThreadBehavior = "inherit" | "configured";
export type UnavailableModelPolicy = "ask" | "favorite" | "blocked";
export type BrowserStorageMode = PreviewBrowserStorageMode;
export const MAX_GLOBAL_INSTRUCTIONS_CHARS = 32 * 1024;

export interface ConfiguredThreadDefaults {
  model: string;
  activeAgentId: string | null;
  memory: boolean;
  privacy: PrivacyMode;
  sandbox: boolean;
  toolApproval: "review" | "guarded" | "open";
  workerModel: string;
  delegationPolicy: DelegationPolicy;
}

export const DEFAULT_CONFIGURED_THREAD_DEFAULTS: ConfiguredThreadDefaults = {
  model: "",
  activeAgentId: null,
  memory: true,
  privacy: "off",
  sandbox: false,
  toolApproval: "review",
  workerModel: "",
  delegationPolicy: "ask",
};

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

interface SettingsState {
  favorites: string[];
  favoritesOnly: boolean;
  collapsedModelGroups: string[];
  accountRuntimeEnabled: AccountRuntimeEnablement;
  reasoningEffortByModel: Record<string, ReasoningEffort>;
  globalInstructions: string;
  media: MediaSettings;
  newThreadBehavior: NewThreadBehavior;
  configuredThreadDefaults: ConfiguredThreadDefaults;
  unavailableModelPolicy: UnavailableModelPolicy;
  browserStorageMode: BrowserStorageMode;
  browserSetupSeen: boolean;
  toggleFavorite: (id: string) => void;
  setFavorites: (ids: string[]) => void;
  setFavoritesOnly: (v: boolean) => void;
  setModelGroupCollapsed: (group: string, collapsed: boolean) => void;
  setAccountRuntimeEnabled: (kind: AccountRuntimeKind, enabled: boolean) => void;
  setModelReasoningEffort: (model: string, effort: ReasoningEffort) => void;
  setGlobalInstructions: (instructions: string) => void;
  setMediaSettings: (settings: Partial<MediaSettings>) => void;
  setNewThreadBehavior: (behavior: NewThreadBehavior) => void;
  setConfiguredThreadDefaults: (settings: Partial<ConfiguredThreadDefaults>) => void;
  setUnavailableModelPolicy: (policy: UnavailableModelPolicy) => void;
  setBrowserStorageMode: (mode: BrowserStorageMode) => void;
  setBrowserSetupSeen: (seen: boolean) => void;
}

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return Array.from(new Set(
    value
      .filter((item): item is string => typeof item === "string")
      .map((item) => item.trim())
      .filter(Boolean),
  ));
}

function normalizeStringRecord(value: unknown): Record<string, string> {
  const record = asRecord(value);
  if (!record) return {};
  return Object.fromEntries(
    Object.entries(record)
      .filter((entry): entry is [string, string] => typeof entry[1] === "string"),
  );
}

function normalizeGlobalInstructions(value: unknown): string {
  return typeof value === "string"
    ? value.slice(0, MAX_GLOBAL_INSTRUCTIONS_CHARS)
    : "";
}

function normalizeAccountRuntimeEnablement(
  value: unknown,
): AccountRuntimeEnablement {
  const record = asRecord(value);
  return {
    codex:
      typeof record?.codex === "boolean"
        ? record.codex
        : DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT.codex,
    claude:
      typeof record?.claude === "boolean"
        ? record.claude
        : DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT.claude,
    opencode:
      typeof record?.opencode === "boolean"
        ? record.opencode
        : DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT.opencode,
    pi:
      typeof record?.pi === "boolean"
        ? record.pi
        : DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT.pi,
  };
}

function normalizeReasoningEffortByModel(value: unknown): Record<string, ReasoningEffort> {
  const record = asRecord(value);
  if (!record) return {};
  const result: Record<string, ReasoningEffort> = {};
  for (const [model, effort] of Object.entries(record)) {
    if (!model.trim() || !isReasoningEffort(effort) || effort === "auto") continue;
    result[model] = effort;
  }
  return result;
}

function normalizeParameterRecord(value: unknown): Record<string, Record<string, unknown>> {
  const record = asRecord(value);
  if (!record) return {};
  return Object.fromEntries(
    Object.entries(record)
      .map(([key, parameters]) => [key, asRecord(parameters) ?? {}] as const),
  );
}

function normalizeStringArrayRecord(value: unknown): Record<string, string[]> {
  const record = asRecord(value);
  if (!record) return {};
  return Object.fromEntries(
    Object.entries(record).map(([key, items]) => [
      key,
      Array.isArray(items) ? items.filter((item): item is string => typeof item === "string") : [],
    ]),
  );
}

function normalizeMediaSettings(settings?: Partial<MediaSettings>): MediaSettings {
  return {
    providerId: typeof settings?.providerId === "string" ? settings.providerId : "",
    modelByProvider: normalizeStringRecord(settings?.modelByProvider),
    parametersByProviderModel: normalizeParameterRecord(settings?.parametersByProviderModel),
    advancedByProviderModel: normalizeStringRecord(settings?.advancedByProviderModel),
    favoriteModelIdsByProvider: normalizeStringArrayRecord(settings?.favoriteModelIdsByProvider),
    modelSearchByProvider: normalizeStringRecord(settings?.modelSearchByProvider),
  };
}

function normalizeConfiguredThreadDefaults(value: unknown): ConfiguredThreadDefaults {
  const settings = asRecord(value) ?? {};
  const privacy = settings.privacy;
  const delegationPolicy = settings.delegationPolicy;
  return {
    model: typeof settings.model === "string" && settings.model.trim().toLowerCase() !== "mock-echo" ? settings.model.trim() : "",
    activeAgentId: typeof settings.activeAgentId === "string" && settings.activeAgentId.trim() ? settings.activeAgentId.trim() : null,
    memory: typeof settings.memory === "boolean" ? settings.memory : true,
    privacy: privacy === "redact" || privacy === "block" ? privacy : "off",
    sandbox: typeof settings.sandbox === "boolean" ? settings.sandbox : false,
    toolApproval: settings.toolApproval === "guarded" || settings.toolApproval === "open"
      ? settings.toolApproval
      : "review",
    workerModel: typeof settings.workerModel === "string" && settings.workerModel.trim().toLowerCase() !== "mock-echo" ? settings.workerModel.trim() : "",
    delegationPolicy: delegationPolicy === "off" || delegationPolicy === "auto" ? delegationPolicy : "ask",
  };
}

export const useSettings = create<SettingsState>()(
  persist(
    (set) => ({
      favorites: [],
      favoritesOnly: false,
      collapsedModelGroups: [],
      accountRuntimeEnabled: { ...DEFAULT_ACCOUNT_RUNTIME_ENABLEMENT },
      reasoningEffortByModel: {},
      globalInstructions: "",
      media: DEFAULT_MEDIA_SETTINGS,
      newThreadBehavior: "inherit",
      configuredThreadDefaults: DEFAULT_CONFIGURED_THREAD_DEFAULTS,
      unavailableModelPolicy: "ask",
      browserStorageMode: "persistent",
      browserSetupSeen: false,
      toggleFavorite: (id) =>
        set((s) => ({
          favorites: s.favorites.includes(id)
            ? s.favorites.filter((x) => x !== id)
            : [...s.favorites, id],
        })),
      setFavorites: (ids) => set({ favorites: normalizeStringArray(ids) }),
      setFavoritesOnly: (favoritesOnly) => set({ favoritesOnly }),
      setModelGroupCollapsed: (group, collapsed) =>
        set((s) => {
          const groups = new Set(s.collapsedModelGroups);
          const key = group.trim();
          if (!key) return {};
          if (collapsed) groups.add(key);
          else groups.delete(key);
          return { collapsedModelGroups: Array.from(groups) };
        }),
      setAccountRuntimeEnabled: (kind, enabled) =>
        set((state) => ({
          accountRuntimeEnabled: {
            ...state.accountRuntimeEnabled,
            [kind]: enabled,
          },
        })),
      setModelReasoningEffort: (model, effort) =>
        set((s) => ({
          reasoningEffortByModel: reasoningEffortByModelWithSelection(s.reasoningEffortByModel, model, effort),
        })),
      setGlobalInstructions: (globalInstructions) => set({
        globalInstructions: normalizeGlobalInstructions(globalInstructions),
      }),
      setMediaSettings: (settings) =>
        set((s) => ({
          media: normalizeMediaSettings({ ...s.media, ...settings }),
        })),
      setNewThreadBehavior: (newThreadBehavior) => set({ newThreadBehavior: newThreadBehavior === "configured" ? "configured" : "inherit" }),
      setConfiguredThreadDefaults: (settings) => set((state) => ({
        configuredThreadDefaults: normalizeConfiguredThreadDefaults({ ...state.configuredThreadDefaults, ...settings }),
      })),
      setUnavailableModelPolicy: (unavailableModelPolicy) => set({
        unavailableModelPolicy: unavailableModelPolicy === "favorite" || unavailableModelPolicy === "blocked" ? unavailableModelPolicy : "ask",
      }),
      setBrowserStorageMode: (browserStorageMode) => set({
        browserStorageMode: browserStorageMode === "private" ? "private" : "persistent",
      }),
      setBrowserSetupSeen: (browserSetupSeen) => set({ browserSetupSeen: Boolean(browserSetupSeen) }),
    }),
    {
      name: "milim.settings",
      storage: createJSONStorage(() => userStateStorage),
      merge: (persisted, current) => {
        const saved = { ...((persisted ?? {}) as Partial<SettingsState> & { voice?: unknown }) };
        delete saved.voice;
        return {
          ...current,
          ...saved,
          favorites: normalizeStringArray(saved?.favorites),
          favoritesOnly: Boolean(saved?.favoritesOnly),
          collapsedModelGroups: normalizeStringArray(saved?.collapsedModelGroups),
          accountRuntimeEnabled: normalizeAccountRuntimeEnablement(saved?.accountRuntimeEnabled),
          reasoningEffortByModel: normalizeReasoningEffortByModel(saved?.reasoningEffortByModel),
          globalInstructions: normalizeGlobalInstructions(saved?.globalInstructions),
          media: normalizeMediaSettings(saved?.media),
          newThreadBehavior: saved?.newThreadBehavior === "configured" ? "configured" : "inherit",
          configuredThreadDefaults: normalizeConfiguredThreadDefaults(saved?.configuredThreadDefaults),
          unavailableModelPolicy: saved?.unavailableModelPolicy === "favorite" || saved?.unavailableModelPolicy === "blocked" ? saved.unavailableModelPolicy : "ask",
          browserStorageMode: saved?.browserStorageMode === "private" ? "private" : "persistent",
          browserSetupSeen: Boolean(saved?.browserSetupSeen),
        };
      },
      partialize: (s) => ({
        favorites: s.favorites,
        favoritesOnly: s.favoritesOnly,
        collapsedModelGroups: s.collapsedModelGroups,
        accountRuntimeEnabled: normalizeAccountRuntimeEnablement(s.accountRuntimeEnabled),
        reasoningEffortByModel: s.reasoningEffortByModel,
        globalInstructions: normalizeGlobalInstructions(s.globalInstructions),
        media: s.media,
        newThreadBehavior: s.newThreadBehavior,
        configuredThreadDefaults: normalizeConfiguredThreadDefaults(s.configuredThreadDefaults),
        unavailableModelPolicy: s.unavailableModelPolicy,
        browserStorageMode: s.browserStorageMode,
        browserSetupSeen: s.browserSetupSeen,
      }),
    },
  ),
);
