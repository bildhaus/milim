class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

const {
  DEFAULT_MEDIA_SETTINGS,
  useSettings,
} = await import("../src/settings/store.js");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function deepEqual(actual: unknown, expected: unknown, message: string): void {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) throw new Error(`${message}: expected ${e}, got ${a}`);
}

assert(!("voice" in useSettings.getState()), "obsolete voice settings should not be exposed");
equal(DEFAULT_MEDIA_SETTINGS.providerId, "", "media provider should be unset by default");
equal(useSettings.getState().media.providerId, "", "persisted media provider should start unset");
deepEqual(useSettings.getState().reasoningEffortByModel, {}, "reasoning effort should default to no global model overrides");
deepEqual(useSettings.getState().collapsedModelGroups, [], "model provider groups should default to expanded");
deepEqual(
  useSettings.getState().accountRuntimeEnabled,
  { codex: true, claude: true, opencode: true, pi: true },
  "account runtimes should default enabled",
);
equal(useSettings.getState().newThreadBehavior, "inherit", "new chats should preserve inheritance by default");
equal(useSettings.getState().unavailableModelPolicy, "ask", "unavailable defaults should ask by default");
equal(useSettings.getState().configuredThreadDefaults.toolApproval, "guarded", "configured approval should default guarded");
assert(!("modelPresets" in useSettings.getState()), "obsolete model presets should not be exposed");
assert(!("presetsOnly" in useSettings.getState()), "obsolete presets-only state should not be exposed");

useSettings.getState().setFavoritesOnly(true);
equal(useSettings.getState().favoritesOnly, true, "favorites-only mode should enable");
useSettings.getState().setFavoritesOnly(false);
useSettings.getState().toggleFavorite("gpt-5");
deepEqual(useSettings.getState().favorites, ["gpt-5"], "favorites should persist as the only model shortcut");

useSettings.getState().setModelGroupCollapsed("OpenAI", true);
useSettings.getState().setModelGroupCollapsed("OpenAI", true);
deepEqual(useSettings.getState().collapsedModelGroups, ["OpenAI"], "collapsed model groups should stay unique");
assert(localStorage.getItem("milim.settings")?.includes('"collapsedModelGroups":["OpenAI"]'), "collapsed model groups should persist in settings");
useSettings.getState().setModelGroupCollapsed("OpenAI", false);
deepEqual(useSettings.getState().collapsedModelGroups, [], "expanded model groups should leave persisted collapse state");

useSettings.getState().setAccountRuntimeEnabled("claude", false);
equal(useSettings.getState().accountRuntimeEnabled.claude, false, "account runtimes should be disableable");
assert(localStorage.getItem("milim.settings")?.includes('"claude":false'), "account runtime toggles should persist");
useSettings.getState().setAccountRuntimeEnabled("claude", true);

useSettings.getState().setMediaSettings({
  providerId: "prov-openrouter",
  modelByProvider: {
    "prov-openrouter": "google/gemini-2.5-flash-image",
  },
  parametersByProviderModel: {
    "prov-openrouter::google/gemini-2.5-flash-image": {
      aspect_ratio: "16:9",
      seed: 42,
    },
  },
  advancedByProviderModel: {
    "prov-openrouter::google/gemini-2.5-flash-image": "{\n  \"image_config\": {}\n}",
  },
  favoriteModelIdsByProvider: {
    "prov-openrouter": ["google/gemini-2.5-flash-image"],
  },
  modelSearchByProvider: {
    "prov-openrouter": "gemini image",
  },
});

equal(useSettings.getState().media.providerId, "prov-openrouter", "media provider selection should persist");
equal(
  useSettings.getState().media.modelByProvider["prov-openrouter"],
  "google/gemini-2.5-flash-image",
  "media model selection should persist per provider",
);
equal(
  useSettings.getState().media.parametersByProviderModel["prov-openrouter::google/gemini-2.5-flash-image"].aspect_ratio,
  "16:9",
  "media model parameter choices should persist",
);
assert(localStorage.getItem("milim.settings")?.includes("google/gemini-2.5-flash-image"), "media settings should be written to synced settings");
equal(
  useSettings.getState().media.favoriteModelIdsByProvider["prov-openrouter"][0],
  "google/gemini-2.5-flash-image",
  "media model favorites should persist per provider",
);
equal(
  useSettings.getState().media.modelSearchByProvider["prov-openrouter"],
  "gemini image",
  "media model search text should persist per provider",
);
assert(!localStorage.getItem("milim.settings")?.includes("openrouter-secret"), "media settings must not store provider API keys");

useSettings.getState().setModelReasoningEffort("openrouter/deepseek-r1", "high");
deepEqual(
  useSettings.getState().reasoningEffortByModel,
  { "openrouter/deepseek-r1": "high" },
  "reasoning effort should persist globally per model",
);
assert(localStorage.getItem("milim.settings")?.includes("openrouter/deepseek-r1"), "global model reasoning effort should persist in settings");
useSettings.getState().setModelReasoningEffort("openrouter/deepseek-r1", "auto");
deepEqual(useSettings.getState().reasoningEffortByModel, {}, "auto should remove the global model reasoning override");

localStorage.setItem("milim.settings", JSON.stringify({
  state: {
    collapsedModelGroups: [" OpenAI ", "OpenAI", "", 42, "Codex"],
    accountRuntimeEnabled: { codex: false, claude: "no", pi: false },
  },
  version: 0,
}));
await useSettings.persist.rehydrate();
deepEqual(useSettings.getState().collapsedModelGroups, ["OpenAI", "Codex"], "persisted model groups should normalize malformed values");
deepEqual(
  useSettings.getState().accountRuntimeEnabled,
  { codex: false, claude: true, opencode: true, pi: false },
  "persisted account runtime toggles should normalize malformed values",
);

useSettings.getState().setNewThreadBehavior("configured");
useSettings.getState().setConfiguredThreadDefaults({ model: "provider:model", toolApproval: "review", privacy: "redact" });
equal(useSettings.getState().configuredThreadDefaults.model, "provider:model", "configured model should persist");
equal(useSettings.getState().configuredThreadDefaults.toolApproval, "review", "review should be a valid configured approval");
useSettings.getState().setConfiguredThreadDefaults({ toolApproval: "open" as never });
equal(useSettings.getState().configuredThreadDefaults.toolApproval, "guarded", "open should never persist as a global approval default");
useSettings.getState().setUnavailableModelPolicy("blocked");
equal(useSettings.getState().unavailableModelPolicy, "blocked", "blocked fallback should persist");

export {};
