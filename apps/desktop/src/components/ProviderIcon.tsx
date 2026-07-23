import type { CSSProperties } from "react";
import type { ModelInfo, ProviderInfo } from "../api";
import { Plug } from "./icons";

export type ProviderBrand =
  | "openai"
  | "openrouter"
  | "groq"
  | "claude"
  | "gemini"
  | "replicate"
  | "fal"
  | "ollama"
  | "lmstudio"
  | "codex"
  | "opencode"
  | "pi";

const ICONS: Record<ProviderBrand, string> = {
  openai: new URL("../assets/providers/openai.svg", import.meta.url).href,
  openrouter: new URL("../assets/providers/openrouter.svg", import.meta.url).href,
  groq: new URL("../assets/providers/groq.svg", import.meta.url).href,
  claude: new URL("../assets/providers/claude.svg", import.meta.url).href,
  gemini: new URL("../assets/providers/gemini.svg", import.meta.url).href,
  replicate: new URL("../assets/providers/replicate.svg", import.meta.url).href,
  fal: new URL("../assets/providers/fal.svg", import.meta.url).href,
  ollama: new URL("../assets/providers/ollama.svg", import.meta.url).href,
  lmstudio: new URL("../assets/providers/lmstudio.svg", import.meta.url).href,
  codex: new URL("../assets/providers/codex.svg", import.meta.url).href,
  opencode: new URL("../assets/providers/opencode.svg", import.meta.url).href,
  pi: new URL("../assets/providers/pi.svg", import.meta.url).href,
};

const NAMES: Record<string, ProviderBrand> = {
  openai: "openai",
  openrouter: "openrouter",
  openroutermedia: "openrouter",
  groq: "groq",
  anthropic: "claude",
  claude: "claude",
  localclaudecli: "claude",
  gemini: "gemini",
  replicate: "replicate",
  replicatemedia: "replicate",
  fal: "fal",
  falmedia: "fal",
  ollama: "ollama",
  ollamalocal: "ollama",
  lmstudio: "lmstudio",
  lmstudiolocal: "lmstudio",
  codex: "codex",
  opencode: "opencode",
  localopencodecli: "opencode",
  pi: "pi",
  localpicli: "pi",
};

export function providerBrandForProvider(
  provider?: Pick<ProviderInfo, "name" | "base_url"> | null,
): ProviderBrand | null {
  if (!provider) return null;
  const url = provider.base_url.toLowerCase();
  if (url.includes("api.openai.com")) return "openai";
  if (url.includes("openrouter.ai")) return "openrouter";
  if (url.includes("groq.com")) return "groq";
  if (url.includes("anthropic.com")) return "claude";
  if (url.includes("generativelanguage.googleapis.com")) return "gemini";
  if (url.includes("replicate.com")) return "replicate";
  if (url.includes("fal.run") || url.includes("fal.ai")) return "fal";
  if (/localhost|127\.0\.0\.1/.test(url) && url.includes(":11434")) return "ollama";
  if (/localhost|127\.0\.0\.1/.test(url) && url.includes(":1234")) return "lmstudio";
  return NAMES[provider.name.toLowerCase().replace(/[^a-z0-9]+/g, "")] ?? null;
}

export function providerBrandForModel(
  model?: Pick<ModelInfo, "id" | "owned_by" | "provider_id">,
  providers: ProviderInfo[] = [],
): ProviderBrand | null {
  if (!model) return null;
  const id = model.id.toLowerCase();
  if (id.startsWith("codex:")) return "codex";
  if (id.startsWith("claude:")) return "claude";
  if (id.startsWith("opencode:")) return "opencode";
  if (id.startsWith("pi:")) return "pi";
  const provider = providers.find((item) =>
    item.id === model.provider_id ||
    item.name.toLowerCase() === model.owned_by.toLowerCase()
  );
  return providerBrandForProvider(provider ?? {
    name: model.owned_by,
    base_url: "",
  });
}

export function ProviderIcon({
  brand,
  className = "",
  size = 14,
}: {
  brand: ProviderBrand | null;
  className?: string;
  size?: number;
}) {
  const classes = `provider-brand-icon${className ? ` ${className}` : ""}`;
  if (!brand) return <Plug className={classes} size={size} aria-hidden="true" />;
  return (
    <span
      aria-hidden="true"
      className={classes}
      data-provider-brand={brand}
      style={{
        "--provider-brand-icon": `url("${ICONS[brand]}")`,
        width: size,
        height: size,
      } as CSSProperties}
    />
  );
}
