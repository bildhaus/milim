import {
  supportsMediaMetadataProvider,
  type MediaGenerationResult,
  type MediaKind,
  type MediaModelSchema,
  type MediaSchemaControl,
  type ProviderInfo,
} from "../api";

const MODEL_PRESETS: Record<string, string> = {
  replicate: "black-forest-labs/flux-schnell",
  fal: "fal-ai/flux/schnell",
  openai_compatible: "google/gemini-2.5-flash-image",
};

export const DEFAULT_MEDIA_ADVANCED_INPUT = "{\n  \"num_outputs\": 1\n}";
export const OPENROUTER_MEDIA_ADVANCED_INPUT = "{}";

const TERMINAL_MEDIA_STATUSES = new Set(["completed", "complete", "succeeded", "successful", "failed", "canceled", "cancelled"]);

export function parseJsonObject(text: string): Record<string, unknown> {
  const trimmed = text.trim();
  if (!trimmed) return {};
  const parsed = JSON.parse(trimmed);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Advanced input must be a JSON object.");
  }
  return parsed as Record<string, unknown>;
}

export function bestMediaResultUrl(result: MediaGenerationResult): string | null {
  return (
    result.media[0]?.url ??
    result.urls.web ??
    result.urls.response ??
    result.urls.get ??
    result.urls.status ??
    null
  );
}

export function defaultMediaModel(provider?: ProviderInfo | null): string {
  return provider ? MODEL_PRESETS[provider.kind] ?? "" : "";
}

export function defaultMediaAdvanced(provider?: ProviderInfo | null): string {
  return provider && supportsMediaMetadataProvider(provider) ? OPENROUTER_MEDIA_ADVANCED_INPUT : DEFAULT_MEDIA_ADVANCED_INPUT;
}

export function mediaPreferenceKey(providerId: string, model: string): string {
  return `${providerId}::${model}`;
}

export function controlDefaultValue(control: MediaSchemaControl): unknown {
  if ("default" in control) return control.default;
  return control.kind === "select" ? control.options?.[0]?.value : "";
}

function applyValueAtPath(target: Record<string, unknown>, path: string[], value: unknown): void {
  if (path.length === 0 || value === "" || value === undefined || value === null) return;
  let cursor: Record<string, unknown> = target;
  for (const segment of path.slice(0, -1)) {
    const next = cursor[segment];
    if (!next || typeof next !== "object" || Array.isArray(next)) {
      cursor[segment] = {};
    }
    cursor = cursor[segment] as Record<string, unknown>;
  }
  cursor[path[path.length - 1]] = value;
}

export function inputWithSchemaControls(
  advanced: string,
  schema: MediaModelSchema | null,
  values: Record<string, unknown>,
): Record<string, unknown> {
  const input = parseJsonObject(advanced);
  if (!schema) return input;
  for (const control of schema.controls) {
    const stored = values[control.key];
    const value =
      typeof stored === "string" &&
      (control.kind === "array" || control.kind === "json")
        ? parseControlValue(control, stored)
        : stored;
    applyValueAtPath(input, control.path, value);
  }
  return input;
}

export function schemaDefaults(schema: MediaModelSchema | null): Record<string, unknown> {
  if (!schema) return {};
  return Object.fromEntries(schema.controls.map((control) => [control.key, controlDefaultValue(control)]));
}

export function controlValue(value: unknown): string {
  if (Array.isArray(value)) return value.join("\n");
  if (value && typeof value === "object") return JSON.stringify(value, null, 2);
  return value === undefined || value === null ? "" : String(value);
}

export function parseControlValue(control: MediaSchemaControl, value: string | boolean): unknown {
  if (control.kind === "checkbox") return Boolean(value);
  if (control.kind === "number") return value === "" ? "" : Number(value);
  if (typeof value !== "string") return value;
  if (control.kind === "array") {
    const trimmed = value.trim();
    if (!trimmed) return [];
    let items: unknown[];
    if (trimmed.startsWith("[")) {
      const parsed = JSON.parse(trimmed);
      if (!Array.isArray(parsed)) throw new Error(`${control.label} must be a JSON array.`);
      items = parsed;
    } else {
      items = trimmed
        .split(/\r?\n|,/)
        .map((item) => item.trim())
        .filter(Boolean);
    }
    if (control.item_kind !== "number") return items;
    const numbers = items.map(Number);
    if (numbers.some((item) => !Number.isFinite(item))) {
      throw new Error(`${control.label} must contain only numbers.`);
    }
    return numbers;
  }
  if (control.kind === "json") return value.trim() ? JSON.parse(value) : {};
  const option = control.options?.find((item) => String(item.value) === value);
  return option ? option.value : value;
}

export function isTerminalMediaStatus(status: string): boolean {
  return TERMINAL_MEDIA_STATUSES.has(status.trim().toLowerCase());
}

export function shouldPollMediaStatus(result: MediaGenerationResult): boolean {
  return (
    (result.provider_kind === "replicate" ||
      result.provider_kind === "fal" ||
      (result.provider_kind === "openai_compatible" && result.kind === "video")) &&
    Boolean(result.id) &&
    !isTerminalMediaStatus(result.status)
  );
}

export function mediaPollingMaxAttempts(result: MediaGenerationResult): number {
  return result.provider_kind === "openai_compatible" && result.kind === "video"
    ? 120
    : 20;
}

export function mediaKindForModelId(model: string): MediaKind | null {
  const id = model.trim().toLowerCase();
  if (!id) return null;
  if (/\b(music|text-to-music|musicgen|lyria|song|melody)\b/.test(id)) return "music";
  if (/\b(video|text-to-video|image-to-video|wan|veo|kling|runway|luma|pika|hailuo)\b/.test(id)) return "video";
  if (/\b(image|text-to-image|flux|sdxl|stable-diffusion|dall-e|midjourney|ideogram|recraft)\b/.test(id)) return "image";
  if (/gemini.*flash.*image/.test(id)) return "image";
  return null;
}

export function mediaResultContent(result: MediaGenerationResult): string {
  const kind = String(result.media[0]?.kind ?? result.kind ?? "media");
  const status = result.status.trim() || "submitted";
  return `${kind.charAt(0).toUpperCase()}${kind.slice(1)} generation ${status} with ${result.model}.`;
}
