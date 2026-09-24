import {
  isOpenRouterProvider,
  PROVIDER_PRESETS,
  type AccountRuntimeKind,
  type AccountRuntimeUpdateStatus,
  type ProviderInfo,
  type ProviderKind,
} from "../api.js";
import { isLoopbackProviderEndpoint } from "./providerEndpoint.js";

/**
 * View model for the Providers manager: how each connection is grouped in the
 * rail, which one-line status it shows, and what (if anything) needs action.
 * Kept free of React so the rules can be tested directly.
 */

export type StatusTone = "ready" | "warning" | "error" | "off" | "draft";

/** Which Providers page is open. Other surfaces deep-link with this. */
export type ProvidersTarget =
  | { view: "overview" }
  | { view: "runtime"; runtime: AccountRuntimeKind }
  | { view: "provider"; id: string }
  | { view: "new"; preset?: string };

export const ACCOUNT_RUNTIME_KINDS: readonly AccountRuntimeKind[] = [
  "codex",
  "claude",
  "opencode",
  "pi",
];

export const ACCOUNT_RUNTIME_LABEL: Record<AccountRuntimeKind, string> = {
  codex: "Codex",
  claude: "Claude",
  opencode: "OpenCode",
  pi: "Pi",
};

export const PROVIDER_KIND_OPTIONS: Array<{ label: string; value: ProviderKind }> = [
  { label: "OpenAI-compatible", value: "openai_compatible" },
  { label: "Anthropic Messages", value: "anthropic" },
  { label: "Gemini API", value: "gemini" },
  { label: "Replicate media", value: "replicate" },
  { label: "fal media", value: "fal" },
];

export const KIND_LABEL: Record<ProviderKind, string> = {
  openai_compatible: "OpenAI-compatible",
  anthropic: "Anthropic Messages",
  gemini: "Gemini API",
  replicate: "Replicate media",
  fal: "fal media",
};

type ProviderShape = Pick<ProviderInfo, "kind" | "name" | "base_url">;

export function isMediaProvider(provider: ProviderShape): boolean {
  return (
    provider.kind === "replicate" ||
    provider.kind === "fal" ||
    isOpenRouterProvider(provider)
  );
}

/** Media-only providers have no chat models at all. */
export function isMediaOnlyProvider(provider: ProviderShape): boolean {
  return provider.kind === "replicate" || provider.kind === "fal";
}

export function providerNeedsKey(provider: ProviderShape): boolean {
  const normalizedBase = provider.base_url.trim().replace(/\/+$/, "").toLowerCase();
  const preset = PROVIDER_PRESETS.find(
    (p) =>
      p.kind === provider.kind &&
      p.base_url.trim().replace(/\/+$/, "").toLowerCase() === normalizedBase,
  );
  if (preset) return preset.needsKey;
  return !isLoopbackProviderEndpoint(provider.base_url);
}

export function providerCategory(provider: ProviderShape): string {
  if (isMediaOnlyProvider(provider)) return "Media";
  if (isOpenRouterProvider(provider)) return "Chat + media";
  return "Chat";
}

export type ProviderRailGroup = "hosted" | "local" | "media";

export const PROVIDER_RAIL_GROUPS: ReadonlyArray<{ id: ProviderRailGroup; label: string }> = [
  { id: "hosted", label: "Hosted" },
  { id: "local", label: "Local" },
  { id: "media", label: "Media" },
];

/**
 * Media-only kinds group under Media, loopback endpoints under Local, and
 * everything else, including chat providers that also generate media, under
 * Hosted.
 */
export function providerRailGroup(provider: ProviderShape): ProviderRailGroup {
  if (isMediaOnlyProvider(provider)) return "media";
  if (isLoopbackProviderEndpoint(provider.base_url)) return "local";
  return "hosted";
}

export function providerRailGroupLabel(provider: ProviderShape): string {
  return PROVIDER_RAIL_GROUPS.find((group) => group.id === providerRailGroup(provider))?.label ?? "Hosted";
}

export interface ConnectionStatus {
  tone: StatusTone;
  label: string;
  detail: string;
}

export function providerStatus(provider: ProviderInfo): ConnectionStatus {
  if (!provider.enabled) {
    return { tone: "off", label: "Disabled", detail: "Saved but unavailable to model pickers." };
  }
  if (provider.error) {
    return { tone: "error", label: "Unreachable", detail: provider.error };
  }
  if (isMediaProvider(provider) && providerNeedsKey(provider) && !provider.has_key) {
    return {
      tone: "warning",
      label: "Key missing",
      detail: "Add an API key before media workflows can use it.",
    };
  }
  if (isMediaOnlyProvider(provider)) {
    return { tone: "ready", label: "Media ready", detail: "Credential is available for media workflows." };
  }
  if (provider.models.length) {
    return { tone: "ready", label: "Connected", detail: modelCount(provider.models.length) };
  }
  if (isOpenRouterProvider(provider)) {
    return { tone: "ready", label: "Media ready", detail: "Credential is available for media workflows." };
  }
  return { tone: "warning", label: "No models", detail: "Saved, but no models were returned." };
}

export function providerKeyStatus(provider: ProviderInfo): { tone: StatusTone; label: string } {
  if (!providerNeedsKey(provider)) return { tone: "ready", label: "No key needed" };
  if (provider.has_key) return { tone: "ready", label: "Key saved" };
  return { tone: "warning", label: "Key missing" };
}

/** The one short muted hint a rail row may carry, or null when all is well. */
export function providerRailHint(provider: ProviderInfo): string | null {
  if (!provider.enabled) return "Disabled";
  if (provider.error) return "Unreachable";
  if (providerNeedsKey(provider) && !provider.has_key) return "Key missing";
  if (!isMediaProvider(provider) && provider.models.length === 0) return "No models";
  if (isOpenRouterProvider(provider)) return "+ media";
  return null;
}

/** Secondary fact for the Overview table: model count or media role. */
export function providerDetail(provider: ProviderInfo): string {
  if (isMediaOnlyProvider(provider)) return KIND_LABEL[provider.kind];
  if (provider.models.length) return modelCount(provider.models.length);
  return isOpenRouterProvider(provider) ? "Chat + media" : "No models";
}

export function modelCount(count: number): string {
  return `${count} model${count === 1 ? "" : "s"}`;
}

export function noteTone(note: string): StatusTone {
  if (note.startsWith("Error:")) return "error";
  if (note.includes("no models") || note.includes("No local")) return "warning";
  return "ready";
}

export function sortProvidersForRail(providers: readonly ProviderInfo[]): Array<{
  group: ProviderRailGroup;
  label: string;
  items: ProviderInfo[];
}> {
  return PROVIDER_RAIL_GROUPS.map((group) => ({
    group: group.id,
    label: group.label,
    items: providers.filter((provider) => providerRailGroup(provider) === group.id),
  })).filter((group) => group.items.length > 0);
}

// ----- Coding CLIs (account runtimes) -----

export interface RuntimeSnapshot {
  runtime: AccountRuntimeKind;
  enabled: boolean;
  /** A status check has finished at least once. */
  checked: boolean;
  /** False when the CLI itself could not be found. */
  installed: boolean;
  /** Signed in (Codex, Claude, Pi) or configured (OpenCode). */
  ready: boolean;
  update?: AccountRuntimeUpdateStatus;
  accountCount?: number;
}

export interface RuntimeSummary {
  tone: StatusTone;
  label: string;
  /** Short muted rail hint, or null. */
  hint: string | null;
  version: string | null;
  updateAvailable: boolean;
}

const SIGNED_OUT_LABEL: Record<AccountRuntimeKind, string> = {
  codex: "Signed out",
  claude: "Signed out",
  opencode: "Not configured",
  pi: "Signed out",
};

export function summarizeRuntime(snapshot: RuntimeSnapshot): RuntimeSummary {
  const version = snapshot.update?.version?.trim() || null;
  const updateAvailable =
    snapshot.update?.available !== false && snapshot.update?.update_available === true;
  if (!snapshot.enabled) {
    return { tone: "off", label: "Disabled", hint: "Disabled", version, updateAvailable };
  }
  if (!snapshot.installed) {
    return { tone: "off", label: "Not installed", hint: "Not installed", version: null, updateAvailable: false };
  }
  if (!snapshot.checked) {
    return { tone: "off", label: "Checking", hint: null, version, updateAvailable };
  }
  if (!snapshot.ready) {
    const label = SIGNED_OUT_LABEL[snapshot.runtime];
    return { tone: "warning", label, hint: label, version, updateAvailable };
  }
  const accounts = snapshot.accountCount ?? 0;
  return {
    tone: "ready",
    label: "Ready",
    hint: updateAvailable ? "Update" : accounts > 1 ? `${accounts} accounts` : null,
    version,
    updateAvailable,
  };
}

/** Runtimes whose update can run now: enabled, installed, and behind. */
export function runtimeUpdateTargets(
  enabled: Partial<Record<AccountRuntimeKind, boolean>>,
  updates: Partial<Record<AccountRuntimeKind, AccountRuntimeUpdateStatus>>,
): AccountRuntimeKind[] {
  return ACCOUNT_RUNTIME_KINDS.filter(
    (runtime) =>
      enabled[runtime] &&
      updates[runtime]?.available &&
      updates[runtime]?.update_available === true,
  );
}

/**
 * Show a user's home directory as `~` so the default view never carries a raw
 * absolute path. The full path stays available in titles and Copy path.
 */
export function shortenHomePath(path: string): string {
  return path
    .replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~")
    .replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\|$)/, "~");
}

/** The single item the Overview attention bar calls out, if any. */
export type ProvidersAttention =
  | { kind: "updates"; count: number }
  | { kind: "item"; tone: StatusTone; message: string; target: ProvidersTarget };

export function providersAttention(input: {
  updateCount: number;
  updating: boolean;
  runtimes: ReadonlyArray<{ runtime: AccountRuntimeKind; summary: RuntimeSummary }>;
  providers: readonly ProviderInfo[];
}): ProvidersAttention | null {
  if (input.updateCount > 0 || input.updating) return { kind: "updates", count: input.updateCount };
  for (const provider of input.providers) {
    if (provider.enabled && provider.error) {
      return {
        kind: "item",
        tone: "error",
        message: `${provider.name} is unreachable`,
        target: { view: "provider", id: provider.id },
      };
    }
  }
  for (const provider of input.providers) {
    if (provider.enabled && providerNeedsKey(provider) && !provider.has_key) {
      return {
        kind: "item",
        tone: "warning",
        message: `${provider.name} needs an API key`,
        target: { view: "provider", id: provider.id },
      };
    }
  }
  for (const { runtime, summary } of input.runtimes) {
    if (summary.tone === "warning") {
      return {
        kind: "item",
        tone: "warning",
        message: `${ACCOUNT_RUNTIME_LABEL[runtime]} is ${summary.label.toLowerCase()}`,
        target: { view: "runtime", runtime },
      };
    }
  }
  return null;
}
