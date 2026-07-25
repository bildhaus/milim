import type { ModelInfo, ProviderInfo } from "../api";
import { isLoopbackProviderEndpoint } from "./providerEndpoint.js";

const PROVIDER_MODEL_PREFIX = "provider:";
const CODEX_MODEL_PREFIX = "codex:";
const CLAUDE_MODEL_PREFIX = "claude:";
const OPENCODE_MODEL_PREFIX = "opencode:";
const PI_MODEL_PREFIX = "pi:";
const ACCOUNT_PROVIDER_PART_LABELS: Record<string, string> = {
  github: "GitHub",
  openai: "OpenAI",
  openrouter: "OpenRouter",
  opencode: "OpenCode",
  lmstudio: "LM Studio",
};

export type ModelDevLane =
  | "plain-chat"
  | "milim-tools"
  | "codex-runtime"
  | "claude-runtime"
  | "opencode-runtime"
  | "pi-runtime"
  | "media";

export type ModelDevCapability =
  | "vision"
  | "tools"
  | "reasoning"
  | "fast"
  | "image"
  | "video"
  | "music";

export type ModelDevSetupTone = "ready" | "warning" | "error" | "off";

export type ModelDevProfile = {
  lane: ModelDevLane;
  laneLabel: string;
  providerLabel: string;
  routeLabel: string;
  setupLabel: string;
  setupDetail: string;
  setupTone: ModelDevSetupTone;
  capabilities: ModelDevCapability[];
  detailTags: string[];
  routeDetail: string;
};

export type ModelDevProfileContext = {
  providers?: ProviderInfo[];
  toolIntent?: boolean;
  planMode?: boolean;
};

export function providerModelId(providerId: string, modelId: string): string {
  return `${PROVIDER_MODEL_PREFIX}${providerId}:${modelId}`;
}

export function parseProviderModelId(
  model: string,
): { providerId: string; modelId: string } | null {
  const value = model.trim();
  if (!value.startsWith(PROVIDER_MODEL_PREFIX)) return null;
  const rest = value.slice(PROVIDER_MODEL_PREFIX.length);
  const separator = rest.indexOf(":");
  if (separator <= 0) return null;
  const providerId = rest.slice(0, separator);
  const modelId = rest.slice(separator + 1);
  return providerId && modelId ? { providerId, modelId } : null;
}

export function rawModelId(model: string): string {
  return parseProviderModelId(model)?.modelId ?? model;
}

export function modelDisplayName(model: Pick<ModelInfo, "id" | "display_id">): string {
  if (model.display_id) return model.display_id;
  if (model.id.startsWith(CODEX_MODEL_PREFIX)) return model.id.slice(CODEX_MODEL_PREFIX.length);
  if (model.id.startsWith(CLAUDE_MODEL_PREFIX)) return model.id.slice(CLAUDE_MODEL_PREFIX.length);
  for (const prefix of [OPENCODE_MODEL_PREFIX, PI_MODEL_PREFIX]) {
    if (model.id.startsWith(prefix)) {
      const routed = model.id.slice(prefix.length);
      return routed.slice(routed.indexOf("/") + 1) || routed;
    }
  }
  return model.id;
}

export function modelPickerKey(model: Pick<ModelInfo, "id" | "owned_by" | "provider_id">): string {
  return `${model.provider_id || model.owned_by}\0${model.id}`;
}

function accountRuntimeProviderLabel(id: string, prefix: string): string | null {
  const provider = id.slice(prefix.length).split("/", 1)[0];
  if (!provider) return null;
  return provider
    .split(/[-_.]+/)
    .filter(Boolean)
    .map((part) => ACCOUNT_PROVIDER_PART_LABELS[part.toLowerCase()] ?? `${part[0].toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

export function qualifyDuplicateProviderModels(models: ModelInfo[]): ModelInfo[] {
  const counts = new Map<string, number>();
  for (const model of models) {
    if (model.provider_id) counts.set(model.id, (counts.get(model.id) ?? 0) + 1);
  }
  return models.map((model) =>
    model.provider_id && (counts.get(model.id) ?? 0) > 1
      ? { ...model, id: providerModelId(model.provider_id, model.id), display_id: model.id }
      : model,
  );
}

export function providerOwnsModel(provider: ProviderInfo, model: string): boolean {
  const routed = parseProviderModelId(model);
  if (routed) {
    return provider.enabled && provider.id === routed.providerId && provider.models.includes(routed.modelId);
  }
  return provider.enabled && provider.models.includes(model);
}

export function modelDevCapabilities(model: ModelInfo): ModelDevCapability[] {
  const id = model.id;
  const s = id.toLowerCase();
  const owner = model.owned_by.toLowerCase();
  const out: ModelDevCapability[] = [];
  if (model.capabilities?.imageInput === true) out.push("vision");
  if (model.capabilities?.toolUse) out.push("tools");
  if (model.capabilities?.imageOutput) out.push("image");
  if (model.capabilities?.videoOutput) out.push("video");
  if (model.capabilities?.musicOutput) out.push("music");
  if (
    model.capabilities?.imageInput === undefined &&
    (/(vision|llava|pixtral|llama-4|(?:^|[-/])qwen\d[^/]*vl|(?:^|[-/])qwen3\.6|-vl)/.test(s) ||
      (/openai/.test(owner) && /(gpt-4o|gpt-4\.1|gpt-5(?:[.:-]|$)|o[134](?:[.:-]|$))/.test(s)) ||
      (/anthropic/.test(owner) && /claude-(?:3|opus|sonnet|haiku)/.test(s)) ||
      (/(gemini|google)/.test(owner) && /gemini/.test(s)) ||
      (/groq/.test(owner) && /(llama-4|qwen3\.6)/.test(s)))
  ) out.push("vision");
  if (model.reasoning || /(r1|reason|qwq|o1|o3|-think|deepseek-r)/.test(s)) out.push("reasoning");
  if (/(flash|mini|haiku|turbo|instant|nano|0\.5b|1\.5b|-1b|-3b|-8b|small)/.test(s)) out.push("fast");
  return out;
}

export function modelDevProfile(
  model: ModelInfo | undefined,
  selectedId: string,
  context: ModelDevProfileContext = {},
): ModelDevProfile {
  const id = model?.id || selectedId;
  const capabilities = model ? modelDevCapabilities(model) : [];
  const media = Boolean(
    model?.capabilities?.imageOutput ||
      model?.capabilities?.videoOutput ||
      model?.capabilities?.musicOutput,
  );
  const codex = id.startsWith(CODEX_MODEL_PREFIX);
  const claude = id.startsWith(CLAUDE_MODEL_PREFIX);
  const opencode = id.startsWith(OPENCODE_MODEL_PREFIX);
  const pi = id.startsWith(PI_MODEL_PREFIX);
  const provider = model ? providerForModel(model, context.providers ?? []) : null;
  const setup = modelSetupStatus(model, provider, id);
  const detailTags = model ? modelDetailTags(model, provider) : [];
  const providerLabel = codex
    ? "Codex"
    : claude
      ? "Claude CLI"
      : opencode
        ? "OpenCode CLI"
        : pi
          ? "Pi CLI"
          : provider?.name || model?.owned_by || "Unknown provider";
  const nestedProvider = opencode
    ? accountRuntimeProviderLabel(id, OPENCODE_MODEL_PREFIX)
    : pi
      ? accountRuntimeProviderLabel(id, PI_MODEL_PREFIX)
      : null;
  const routeLabel = opencode
    ? `OpenCode${nestedProvider ? ` · ${nestedProvider}` : ""}`
    : pi
      ? `Pi${nestedProvider ? ` · ${nestedProvider}` : ""}`
      : providerLabel;

  if (codex) {
    return {
      lane: "codex-runtime",
      laneLabel: "Codex runtime",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: context.planMode
        ? "Plan mode keeps this turn read-only."
        : "Next turn uses the Codex account-runtime bridge with this thread context.",
    };
  }
  if (claude) {
    return {
      lane: "claude-runtime",
      laneLabel: "Claude runtime",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: context.planMode
        ? "Plan mode keeps this turn read-only."
        : "Next turn uses the installed Claude CLI runtime bridge with this thread context.",
    };
  }
  if (opencode) {
    return {
      lane: "opencode-runtime",
      laneLabel: "OpenCode runtime",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: context.planMode
        ? "Plan mode keeps this turn read-only."
        : "Next turn uses the installed OpenCode ACP runtime with this thread context.",
    };
  }
  if (pi) {
    return {
      lane: "pi-runtime",
      laneLabel: "Pi runtime",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: context.planMode
        ? "Plan mode keeps this turn read-only."
        : "Next turn uses the installed Pi RPC runtime with this thread context.",
    };
  }
  if (media) {
    return {
      lane: "media",
      laneLabel: "Media",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: "Next send uses the media generation flow for image, video, or music output.",
    };
  }
  if (context.toolIntent && !context.planMode) {
    return {
      lane: "milim-tools",
      laneLabel: "Milim tools",
      providerLabel,
      routeLabel,
      ...setup,
      capabilities,
      detailTags,
      routeDetail: "Next turn keeps the thread context and routes this provider model through Milim's tool-agent loop.",
    };
  }
  return {
    lane: "plain-chat",
    laneLabel: context.planMode ? "Plan/chat" : "Plain chat",
    providerLabel,
    routeLabel,
    ...setup,
    capabilities,
    detailTags,
    routeDetail: context.planMode
      ? "Plan mode keeps the next turn read-only."
      : "Next turn stays on the plain chat path until workspace, tool, memory-write, schedule, preview, or agent context is active.",
  };
}

function providerForModel(
  model: ModelInfo,
  providers: ProviderInfo[],
): ProviderInfo | null {
  const routed = parseProviderModelId(model.id);
  if (model.provider_id) {
    return providers.find((provider) => provider.id === model.provider_id) ?? null;
  }
  if (routed) {
    return providers.find((provider) => provider.id === routed.providerId) ?? null;
  }
  return providers.find((provider) => providerOwnsModel(provider, model.id)) ?? null;
}

function modelSetupStatus(
  model: ModelInfo | undefined,
  provider: ProviderInfo | null,
  selectedId: string,
): Pick<ModelDevProfile, "setupLabel" | "setupDetail" | "setupTone"> {
  if (!model) {
    return {
      setupTone: "warning",
      setupLabel: "Not loaded",
      setupDetail: "The selected model is not in the current picker list.",
    };
  }
  if (selectedId.startsWith(CODEX_MODEL_PREFIX)) {
    return {
      setupTone: "ready",
      setupLabel: "Account ready",
      setupDetail: "Codex appears in the picker after account setup.",
    };
  }
  if (selectedId.startsWith(CLAUDE_MODEL_PREFIX)) {
    return {
      setupTone: "ready",
      setupLabel: "CLI ready",
      setupDetail: "Claude appears in the picker after CLI setup.",
    };
  }
  if (selectedId.startsWith(OPENCODE_MODEL_PREFIX)) {
    return {
      setupTone: "ready",
      setupLabel: "CLI ready",
      setupDetail: "OpenCode appears in the picker after CLI and provider setup.",
    };
  }
  if (selectedId.startsWith(PI_MODEL_PREFIX)) {
    return {
      setupTone: "ready",
      setupLabel: "CLI ready",
      setupDetail: "Pi appears in the picker after CLI and provider setup.",
    };
  }
  if (!provider && model.provider_id) {
    return {
      setupTone: "warning",
      setupLabel: "Provider missing",
      setupDetail: "This provider model is selected, but the provider record is not loaded.",
    };
  }
  if (!provider) {
    return {
      setupTone: "ready",
      setupLabel: "Available",
      setupDetail: "The model is available from the local model list.",
    };
  }
  if (!provider.enabled) {
    return {
      setupTone: "off",
      setupLabel: "Disabled",
      setupDetail: "Enable this provider before using the model.",
    };
  }
  if (provider.error) {
    return {
      setupTone: "error",
      setupLabel: "Unreachable",
      setupDetail: provider.error,
    };
  }
  if (providerNeedsKey(provider) && !provider.has_key) {
    return {
      setupTone: "warning",
      setupLabel: "Key needed",
      setupDetail: "Add this provider's API key before using the model.",
    };
  }
  if (!provider.models.includes(rawModelId(model.id))) {
    return {
      setupTone: "warning",
      setupLabel: "Model unavailable",
      setupDetail: "The provider is connected, but this model is not in its discovered model list.",
    };
  }
  return {
    setupTone: "ready",
    setupLabel: "Ready",
    setupDetail: "The provider and model are ready for the next turn.",
  };
}

function modelDetailTags(model: ModelInfo, provider: ProviderInfo | null): string[] {
  const tags: string[] = [];
  const limit = model.context_length ?? model.max_prompt_tokens;
  if (limit) tags.push(`${formatTokenLimit(limit)} ctx`);
  if (model.max_completion_tokens) {
    tags.push(`${formatTokenLimit(model.max_completion_tokens)} out`);
  }
  const pricing = provider?.pricing?.[rawModelId(model.id)];
  if (pricing) tags.push("pricing");
  return tags;
}

function formatTokenLimit(value: number): string {
  if (value >= 1_000_000) {
    return `${trimTrailingZero(value / 1_000_000)}M`;
  }
  if (value >= 1_000) {
    return `${trimTrailingZero(value / 1_000)}k`;
  }
  return String(value);
}

function trimTrailingZero(value: number): string {
  return value.toFixed(value >= 10 ? 0 : 1).replace(/\.0$/, "");
}

function providerNeedsKey(
  provider: Pick<ProviderInfo, "kind" | "base_url">,
): boolean {
  if (provider.kind === "openai_compatible" && isLoopbackProviderEndpoint(provider.base_url)) {
    return false;
  }
  return true;
}

export function mergeModelListsForPicker(
  chatModels: ModelInfo[],
  mediaModels: ModelInfo[],
): ModelInfo[] {
  const byKey = new Map<string, ModelInfo>();
  for (const model of chatModels) byKey.set(modelPickerKey(model), model);
  for (const model of mediaModels) {
    const key = modelPickerKey(model);
    const existing = byKey.get(key);
    byKey.set(
      key,
      existing
        ? {
            ...existing,
            owned_by: existing.owned_by || model.owned_by,
            capabilities: {
              ...existing.capabilities,
              ...model.capabilities,
            },
          }
        : model,
    );
  }
  return Array.from(byKey.values());
}

export function modelPickerGroups(
  models: ModelInfo[],
  favorites: string[],
  favoritesOnly: boolean,
  query: string,
  providers: ProviderInfo[] = [],
): Array<[string, ModelInfo[]]> {
  const normalizedQuery = query.trim().toLowerCase();
  let list = models;
  if (normalizedQuery) {
    list = list.filter((model) => {
      const routeLabel = modelDevProfile(model, model.id, { providers }).routeLabel;
      return [modelDisplayName(model), model.id, model.owned_by, routeLabel]
        .some((value) => value.toLowerCase().includes(normalizedQuery));
    });
  }
  if (favoritesOnly) list = list.filter((model) => favorites.includes(model.id));
  const favs = list.filter((model) => favorites.includes(model.id));
  const rest = list.filter((model) => !favorites.includes(model.id));
  const providerOrder = new Map<string, number>();
  const byProvider = new Map<string, ModelInfo[]>();
  for (const model of rest) {
    if (!providerOrder.has(model.owned_by)) providerOrder.set(model.owned_by, providerOrder.size);
    const group = byProvider.get(model.owned_by) ?? [];
    group.push(model);
    byProvider.set(model.owned_by, group);
  }
  const groups = Array.from(byProvider);
  groups.sort(
    ([left, leftModels], [right, rightModels]) =>
      leftModels.length - rightModels.length ||
      (providerOrder.get(left) ?? 0) - (providerOrder.get(right) ?? 0),
  );
  return favs.length ? [["Favorites", favs], ...groups] : groups;
}

export function isModelPickerGroupCollapsed(
  label: string,
  collapsedGroups: string[],
  filtering: boolean,
): boolean {
  return label !== "Favorites" && !filtering && collapsedGroups.includes(label);
}
