import { deepEqual, equal } from "node:assert/strict";
import type { ModelInfo, ProviderInfo } from "../src/api";
import {
  modelDevProfile,
  modelDevCapabilities,
  modelDisplayName,
  mergeModelListsForPicker,
  isModelPickerGroupCollapsed,
  modelPickerGroups,
  providerOwnsModel,
  qualifyDuplicateProviderModels,
  rawModelId,
} from "../src/lib/modelPicker.js";

equal(isModelPickerGroupCollapsed("OpenAI", ["OpenAI"], false), true, "saved provider groups should collapse");
equal(isModelPickerGroupCollapsed("OpenAI", ["OpenAI"], true), false, "filtering should reveal saved collapsed groups");
equal(isModelPickerGroupCollapsed("Favorites", ["Favorites"], false), false, "Favorites should always remain expanded");

equal(modelDisplayName({ id: "codex:gpt-5.6-sol" }), "gpt-5.6-sol");
equal(modelDisplayName({ id: "claude:opus" }), "opus");
equal(modelDisplayName({ id: "opencode:openrouter/moonshotai/kimi-k2" }), "moonshotai/kimi-k2");
equal(modelDisplayName({ id: "pi:github-copilot/gpt-5.6-sol", display_id: "GPT-5.6 Sol" }), "GPT-5.6 Sol");
equal(modelDisplayName({ id: "black-forest-labs/flux-schnell" }), "black-forest-labs/flux-schnell");

equal(
  modelDevCapabilities({
    id: "gpt-5",
    owned_by: "OpenAI",
    capabilities: { imageInput: false },
  }).includes("vision"),
  false,
  "explicit false must beat model-family vision fallbacks",
);
equal(
  modelDevCapabilities({ id: "gpt-5.4", owned_by: "OpenAI" }).includes("vision"),
  true,
  "unknown current OpenAI vision families may advertise a conservative fallback",
);
equal(
  modelDevCapabilities({ id: "codex:gpt-5.4", owned_by: "Codex" }).includes("vision"),
  false,
  "missing Codex modality metadata stays unknown instead of claiming vision",
);
equal(
  modelDevCapabilities({
    id: "google/lyria-3-pro-preview",
    owned_by: "OpenRouter media",
    capabilities: { musicOutput: true },
  }).includes("music"),
  true,
  "prompt-to-music models should expose the music capability",
);
equal(
  modelDevProfile({
    id: "google/lyria-3-pro-preview",
    owned_by: "OpenRouter media",
    capabilities: { musicOutput: true },
  }, "google/lyria-3-pro-preview").lane,
  "media",
  "music models should route through the media lane",
);

const duplicated = qualifyDuplicateProviderModels([
  { id: "same-model", owned_by: "OpenAI", provider_id: "prov-a" },
  { id: "same-model", owned_by: "Groq", provider_id: "prov-b" },
  { id: "unique-model", owned_by: "OpenAI", provider_id: "prov-a" },
] satisfies ModelInfo[]);

equal(duplicated[0].id, "provider:prov-a:same-model");
equal(duplicated[0].display_id, "same-model");
equal(duplicated[1].id, "provider:prov-b:same-model");
equal(duplicated[2].id, "unique-model");

equal(
  mergeModelListsForPicker(duplicated, [
    {
      id: "same-model",
      owned_by: "Replicate media",
      capabilities: { imageOutput: true },
    },
  ]).length,
  4,
  "same model ids from different providers should remain separate",
);

deepEqual(
  modelPickerGroups(
    [
      { id: "a1", owned_by: "Large" },
      { id: "a2", owned_by: "Large" },
      { id: "b1", owned_by: "Small" },
    ],
    [],
    false,
    "",
  ).map(([provider]) => provider),
  ["Small", "Large"],
  "smaller provider sections should appear first",
);

const providers = [
  {
    id: "prov-a",
    name: "OpenAI",
    kind: "openai_compatible",
    base_url: "https://api.openai.com/v1",
    enabled: true,
    has_key: true,
    models: ["same-model"],
  },
  {
    id: "prov-b",
    name: "Groq",
    kind: "openai_compatible",
    base_url: "https://api.groq.com/openai/v1",
    enabled: true,
    has_key: true,
    models: ["same-model"],
  },
] satisfies ProviderInfo[];

equal(providerOwnsModel(providers[1], "provider:prov-b:same-model"), true);
equal(providerOwnsModel(providers[0], "provider:prov-b:same-model"), false);
equal(rawModelId("provider:prov-b:same-model"), "same-model");

const profileProviders = [
  {
    id: "openai",
    name: "OpenAI",
    kind: "openai_compatible",
    base_url: "https://api.openai.com/v1",
    enabled: true,
    has_key: true,
    models: ["gpt-5"],
    pricing: { "gpt-5": { prompt: "1.00", completion: "8.00" } },
  },
  {
    id: "missing-key",
    name: "Anthropic",
    kind: "anthropic",
    base_url: "https://api.anthropic.com",
    enabled: true,
    has_key: false,
    models: ["claude-sonnet-4"],
  },
  {
    id: "down",
    name: "Local LM Studio",
    kind: "openai_compatible",
    base_url: "http://127.0.0.1:1234/v1",
    enabled: true,
    has_key: false,
    models: ["qwen2.5-coder"],
    error: "Connection refused",
  },
] satisfies ProviderInfo[];

const providerProfile = modelDevProfile(
  {
    id: "gpt-5",
    owned_by: "OpenAI",
    provider_id: "openai",
    context_length: 128000,
    max_completion_tokens: 16000,
    capabilities: { toolUse: false },
  },
  "gpt-5",
  { providers: profileProviders, toolIntent: true },
);
equal(providerProfile.lane, "milim-tools");
equal(providerProfile.providerLabel, "OpenAI");
equal(providerProfile.routeLabel, "OpenAI");
equal(providerProfile.setupLabel, "Ready");
deepEqual(providerProfile.detailTags, ["128k ctx", "16k out", "pricing"]);

equal(
  modelDevProfile(
    { id: "llama3.2", owned_by: "Ollama" },
    "llama3.2",
    { toolIntent: false },
  ).lane,
  "plain-chat",
);

const localProfile = modelDevProfile(
  { id: "llama3.2", owned_by: "Ollama" },
  "llama3.2",
);
equal(localProfile.providerLabel, "Ollama");
equal(localProfile.setupLabel, "Available");

equal(
  modelDevProfile({ id: "codex:gpt-5", owned_by: "Codex" }, "codex:gpt-5").lane,
  "codex-runtime",
);
equal(
  modelDevProfile({ id: "codex:gpt-5", owned_by: "Codex" }, "codex:gpt-5").routeLabel,
  "Codex",
);
equal(
  modelDevProfile(
    { id: "claude:sonnet", owned_by: "Local Claude CLI" },
    "claude:sonnet",
  ).lane,
  "claude-runtime",
);
equal(
  modelDevProfile(
    { id: "claude:sonnet", owned_by: "Local Claude CLI" },
    "claude:sonnet",
  ).routeLabel,
  "Claude CLI",
);
equal(
  modelDevProfile(
    { id: "opencode:openai/gpt-5.4", owned_by: "Local OpenCode CLI" },
    "opencode:openai/gpt-5.4",
  ).routeLabel,
  "OpenCode · OpenAI",
);
const piProfile = modelDevProfile(
  {
    id: "pi:openai-codex/gpt-5.4",
    owned_by: "Local Pi CLI",
    context_length: 272000,
    capabilities: { imageInput: true, toolUse: true },
  },
  "pi:openai-codex/gpt-5.4",
);
equal(piProfile.lane, "pi-runtime");
equal(piProfile.providerLabel, "Pi CLI");
equal(piProfile.routeLabel, "Pi · OpenAI Codex");
equal(piProfile.setupLabel, "CLI ready");
equal(piProfile.capabilities.includes("vision"), true);
equal(
  modelDevProfile(
    { id: "pi:new_provider/model", owned_by: "Local Pi CLI" },
    "pi:new_provider/model",
  ).routeLabel,
  "Pi · New Provider",
  "unknown nested providers should remain readable",
);
deepEqual(
  modelPickerGroups(
    [
      { id: "opencode:openai/shared", display_id: "Shared", owned_by: "Local OpenCode CLI" },
      { id: "pi:github-copilot/shared", display_id: "Shared", owned_by: "Local Pi CLI" },
    ],
    [],
    false,
    "github copilot",
  ).flatMap(([, models]) => models.map((model) => model.id)),
  ["pi:github-copilot/shared"],
  "search should match humanized nested provider routes",
);
equal(
  modelDevProfile(
    {
      id: "black-forest-labs/flux-schnell",
      owned_by: "Replicate media",
      capabilities: { imageOutput: true },
    },
    "black-forest-labs/flux-schnell",
  ).lane,
  "media",
);

const keyNeeded = modelDevProfile(
  {
    id: "claude-sonnet-4",
    owned_by: "Anthropic",
    provider_id: "missing-key",
  },
  "claude-sonnet-4",
  { providers: profileProviders },
);
equal(keyNeeded.setupTone, "warning");
equal(keyNeeded.setupLabel, "Key needed");

const unreachable = modelDevProfile(
  {
    id: "qwen2.5-coder",
    owned_by: "Local LM Studio",
    provider_id: "down",
  },
  "qwen2.5-coder",
  { providers: profileProviders },
);
equal(unreachable.setupTone, "error");
equal(unreachable.setupLabel, "Unreachable");

const missingProvider = modelDevProfile(
  { id: "ghost-model", owned_by: "Ghost", provider_id: "ghost" },
  "ghost-model",
  { providers: [] },
);
equal(missingProvider.setupTone, "warning");
equal(missingProvider.setupLabel, "Provider missing");
