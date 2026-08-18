export interface GenerationSettings {
  maxTokens?: number;
  temperature?: number;
  topP?: number;
  seed?: number;
  stop?: string[];
  frequencyPenalty?: number;
  presencePenalty?: number;
  topK?: number;
  minP?: number;
  repetitionPenalty?: number;
  thinkingTokenBudget?: number;
}

export type GenerationOverrides = Record<string, GenerationSettings>;

function finiteInRange(value: unknown, min: number, max: number, includeMin = true): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && (includeMin ? value >= min : value > min) && value <= max
    ? value
    : undefined;
}

export function normalizeGenerationSettings(value: unknown): GenerationSettings {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const raw = value as Record<string, unknown>;
  const integer = (input: unknown, min: number, max: number) =>
    typeof input === "number" && Number.isInteger(input) && input >= min && input <= max ? input : undefined;
  const topK = integer(raw.topK, 1, 1_000_000) ?? (raw.topK === -1 ? -1 : undefined);
  const stop = Array.isArray(raw.stop)
    ? raw.stop
        .filter((item): item is string => typeof item === "string")
        .map((item) => item.trim())
        .filter((item) => item.length > 0 && item.length <= 256)
        .slice(0, 8)
    : undefined;
  const next: GenerationSettings = {
    maxTokens: integer(raw.maxTokens, 1, 1_000_000),
    temperature: finiteInRange(raw.temperature, 0, 2),
    topP: finiteInRange(raw.topP, 0, 1, false),
    seed: typeof raw.seed === "number" && Number.isSafeInteger(raw.seed) ? raw.seed : undefined,
    stop: stop?.length ? stop : undefined,
    frequencyPenalty: finiteInRange(raw.frequencyPenalty, -2, 2),
    presencePenalty: finiteInRange(raw.presencePenalty, -2, 2),
    topK,
    minP: finiteInRange(raw.minP, 0, 1),
    repetitionPenalty: finiteInRange(raw.repetitionPenalty, 0, 2, false),
    thinkingTokenBudget: integer(raw.thinkingTokenBudget, 0, 1_000_000),
  };
  return Object.fromEntries(Object.entries(next).filter(([, item]) => item !== undefined));
}

export function normalizeGenerationOverrides(value: unknown): GenerationOverrides {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value as Record<string, unknown>)
      .map(([model, settings]) => [model.trim(), normalizeGenerationSettings(settings)] as const)
      .filter(([model, settings]) => model.length > 0 && Object.keys(settings).length > 0),
  );
}

export function generationSettingsForModel(overrides: GenerationOverrides | undefined, model: string): GenerationSettings {
  return normalizeGenerationSettings(overrides?.[model]);
}

export function generationOverridesWithSelection(
  overrides: GenerationOverrides | undefined,
  model: string,
  settings: GenerationSettings,
): GenerationOverrides | undefined {
  const next = { ...normalizeGenerationOverrides(overrides) };
  const normalized = normalizeGenerationSettings(settings);
  if (Object.keys(normalized).length) next[model] = normalized;
  else delete next[model];
  return Object.keys(next).length ? next : undefined;
}
