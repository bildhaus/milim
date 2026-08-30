import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createServer } from "vite";
import type { ModelInfo, ProviderInfo, ReasoningEffort } from "../src/api.js";
import { DEFAULT_GOAL_SETTINGS } from "../src/lib/goals.js";

type ModelPickerProps = {
  models: ModelInfo[];
  model: string;
  providers?: ProviderInfo[];
  toolIntent?: boolean;
  planMode?: boolean;
  onModel: (selection: { model: string; source: "model" | "preset"; reasoningEffort?: ReasoningEffort }) => void;
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
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

const models: ModelInfo[] = [
  {
    id: "gpt-5-render",
    owned_by: "OpenAI",
    provider_id: "openai-render",
    context_length: 128000,
    capabilities: { imageInput: true, toolUse: true },
    reasoning: {
      supported_efforts: ["auto", "low", "medium", "high"],
      default_effort: "medium",
    },
  },
  {
    id: "black-forest-labs/flux-render",
    owned_by: "Replicate media",
    provider_id: "replicate-render",
    capabilities: { imageOutput: true },
  },
  {
    id: "google/lyria-3-pro-preview",
    owned_by: "OpenRouter media",
    capabilities: { musicOutput: true },
  },
];

const providers: ProviderInfo[] = [
  {
    id: "openai-render",
    name: "OpenAI",
    kind: "openai_compatible",
    base_url: "https://api.openai.com/v1",
    enabled: true,
    has_key: true,
    models: ["gpt-5-render"],
  },
  {
    id: "replicate-render",
    name: "Replicate",
    kind: "replicate",
    base_url: "https://api.replicate.com",
    enabled: true,
    has_key: true,
    models: ["black-forest-labs/flux-render"],
  },
];

const server = await createServer({
  root: process.cwd(),
  appType: "custom",
  logLevel: "silent",
  server: { middlewareMode: true },
});

try {
  const { ModelPicker } = (await server.ssrLoadModule(
    "/src/components/ModelPicker.tsx",
  )) as {
    ModelPicker: ComponentType<ModelPickerProps>;
  };
  const markup = renderToStaticMarkup(
    createElement(ModelPicker, {
      models,
      model: "gpt-5-render",
      providers,
      toolIntent: true,
      onModel: () => {},
      onManageProviders: () => {},
      onManageMcp: () => {},
      onManageMemory: () => {},
      onClose: () => {},
    }),
  );

  assert(markup.includes("OpenAI"), "Picker should render provider names");
  assert(markup.includes('data-provider-brand="openai"'), "Picker provider groups should render brand icons");
  assert(markup.includes("Milim tools"), "Picker should keep the active dev runtime lane in accessible metadata");
  assert(markup.includes("Ready"), "Picker should keep setup status in accessible metadata");
  assert(!markup.includes("mp-meta"), "Picker rows should stay visually compact");
  assert(!markup.includes("128k ctx"), "Picker rows should not render context limits inline");
  assert(markup.includes("Reasoning effort for gpt-5-render: Auto"), "Picker should render reasoning effort state");
  assert(markup.includes('aria-pressed="false"'), "Favorite state should render on the star button");
  assert(markup.includes('title="Vision"'), "Vision capability badge should render");
  assert(markup.includes('title="Tool use"'), "Tool capability badge should render");
  assert(markup.includes("Replicate"), "Picker should render media providers");
  assert(markup.includes("Media"), "Picker should render media lane labels");
  assert(markup.includes('title="Music output"'), "Picker should render music capability badges");
  assert(markup.includes(">Favorites only<"), "Picker should render the favorites filter toggle");
  assert(!markup.includes(">Models<") && !markup.includes(">Presets<"), "Picker should not render redundant model or preset views");
  assert(markup.includes('aria-label="Collapse OpenAI models"'), "Provider headers should render accessible collapse controls");
  assert(markup.includes('aria-expanded="true"'), "Provider headers should expose their expanded state");
  assert(markup.includes('data-hover-scroll-text="true"'), "Model names should use the shared truncated-label hover reveal");
  assert(markup.includes('data-hover-scroll-inner="true"'), "Model hover reveal should animate only its inner text");

  const overlappingModels: ModelInfo[] = [
    { ...models[0], display_id: "Shared model" },
    { id: "codex:gpt-shared", display_id: "Shared model", owned_by: "Codex" },
    { id: "opencode:openai/gpt-shared", display_id: "Shared model", owned_by: "Local OpenCode CLI" },
    { id: "pi:github-copilot/gpt-shared", display_id: "Shared model", owned_by: "Local Pi CLI" },
  ];
  const overlapMarkup = renderToStaticMarkup(
    createElement(ModelPicker, {
      models: overlappingModels,
      model: overlappingModels[0].id,
      providers,
      onModel: () => {},
      onClose: () => {},
      favoriteIds: overlappingModels.map((item) => item.id),
      favoritesOnlyValue: false,
      onToggleFavorite: () => {},
      onFavoritesOnlyChange: () => {},
    }),
  );
  for (const route of ["OpenAI", "Codex", "OpenCode · OpenAI", "Pi · GitHub Copilot"]) {
    assert(overlapMarkup.includes(`title="${route}">${route}</span>`), `Favorites should show the ${route} route`);
  }
  for (const brand of ["openai", "codex", "opencode", "pi"]) {
    assert(overlapMarkup.includes(`data-provider-brand="${brand}"`), `Favorites should show the ${brand} route icon`);
  }
  assert(
    overlapMarkup.match(/Shared model/g)?.length === overlappingModels.length * 3,
    "Overlapping favorite names should remain present in visible text, native titles, and accessible labels",
  );

  const mediaMarkup = renderToStaticMarkup(
    createElement(ModelPicker, {
      models: [models[1]],
      model: models[1].id,
      onModel: () => {},
      onClose: () => {},
      showManagementActions: false,
      favoriteIds: [models[1].id],
      favoritesOnlyValue: false,
      onToggleFavorite: () => {},
      onFavoritesOnlyChange: () => {},
      searchPlaceholder: "Search image models...",
    }),
  );
  assert(mediaMarkup.includes("Search image models..."), "The shared picker should accept media-specific search copy");
  assert(mediaMarkup.includes('aria-pressed="true"'), "The shared picker should render provider-scoped media favorites");
  assert(!mediaMarkup.includes('data-testid="manage-providers"'), "Media picker usage should hide chat management actions");

  const largeCatalog = Array.from({ length: 200 }, (_, index): ModelInfo => ({
    id: `model-${String(index + 1).padStart(3, "0")}`,
    display_id: `Searchable model ${index + 1}`,
    owned_by: index % 2 === 0 ? "OpenAI" : "Anthropic",
    capabilities: { toolUse: true },
  }));
  const largeCatalogMarkup = renderToStaticMarkup(
    createElement(ModelPicker, {
      models: largeCatalog,
      model: largeCatalog[0].id,
      onModel: () => {},
      onClose: () => {},
      searchPlaceholder: "Search 200 models...",
    }),
  );
  assert(
    largeCatalogMarkup.includes("Search 200 models..."),
    "The shared onboarding picker should remain searchable with a 200-model catalog",
  );
  assert(
    largeCatalogMarkup.includes("Searchable model 200"),
    "The shared picker should retain all 200 discoverable models",
  );

  const { ControlBar, modelPickerPlacement } = (await server.ssrLoadModule(
    "/src/components/ControlBar.tsx",
  )) as {
    ControlBar: ComponentType<Record<string, unknown>>;
    modelPickerPlacement: (triggerTop: number, triggerBottom: number, viewportHeight: number) => {
      top: string;
      bottom: string;
      maxHeight: number;
    };
  };
  const constrainedPicker = modelPickerPlacement(260, 286, 675);
  assert(constrainedPicker.bottom === "calc(100% + 6px)", "Chat picker should prefer its existing upward placement");
  assert(constrainedPicker.maxHeight === 246, "Chat picker should fit the usable space above its trigger");
  const flippedPicker = modelPickerPlacement(80, 106, 675);
  assert(flippedPicker.top === "calc(100% + 6px)", "Chat picker should open below when there is too little room above");
  assert(flippedPicker.maxHeight === 440, "A flipped picker should retain its normal maximum height when space allows");
  const controlBarMarkup = renderToStaticMarkup(
    createElement(ControlBar, {
      models,
      model: "gpt-5-render",
      reasoningEffortByModel: { "gpt-5-render": "high" },
      providers,
      toolIntent: true,
      onModel: () => {},
      sandbox: false,
      onToggleSandbox: () => {},
      computerUse: false,
      onToggleComputer: () => {},
      memory: true,
      onToggleMemory: () => {},
      planMode: false,
      onTogglePlanMode: () => {},
      privacy: "off",
      onPrivacy: () => {},
      toolApproval: "guarded",
      onToolApproval: () => {},
      onManageProviders: () => {},
      onManageMcp: () => {},
      onManageMemory: () => {},
      goal: DEFAULT_GOAL_SETTINGS,
      goalMode: true,
      onToggleGoalMode: () => {},
      onOpenGoal: () => {},
    }),
  );
  assert(controlBarMarkup.includes('data-testid="goal-mode-chip"'), "Goal mode should show its pill before a goal starts");
  assert(controlBarMarkup.includes(">Ready<"), "The pre-send Goal pill should communicate that it is ready");
  assert(controlBarMarkup.includes('data-provider-brand="openai"'), "The active model chip should render its provider icon");
  assert(controlBarMarkup.includes('<span class="chip-detail">OpenAI</span>'), "The closed model chip should show the selected provider route");
  assert(!controlBarMarkup.includes('<span class="chip-detail">Milim tools</span>'), "The closed model chip should not replace provider identity with its execution lane");
  assert(!controlBarMarkup.includes('<span class="chip-detail">High</span>'), "The closed model chip should keep reasoning effort in picker and accessible metadata");
  assert(controlBarMarkup.includes('<span class="chip-label">Guarded</span>'), "The closed session-controls pill should prioritize tool approval");
  assert(!controlBarMarkup.includes("Privacy Off</span>"), "The closed session-controls pill should not repeat privacy state");

  const overriddenControlBarMarkup = renderToStaticMarkup(
    createElement(ControlBar, {
      models,
      model: "gpt-5-render",
      reasoningEffortByModel: { "gpt-5-render": "high" },
      reasoningEffortOverrides: { "gpt-5-render": "low" },
      providers,
      onModel: () => {},
      onReasoningEffort: () => {},
      sandbox: false,
      onToggleSandbox: () => {},
      computerUse: false,
      onToggleComputer: () => {},
      memory: true,
      onToggleMemory: () => {},
      planMode: false,
      onTogglePlanMode: () => {},
      privacy: "off",
      onPrivacy: () => {},
      toolApproval: "guarded",
      onToolApproval: () => {},
      onManageProviders: () => {},
      onManageMcp: () => {},
      onManageMemory: () => {},
      goal: DEFAULT_GOAL_SETTINGS,
      goalMode: true,
      onToggleGoalMode: () => {},
      onOpenGoal: () => {},
    }),
  );
  assert(
    overriddenControlBarMarkup.includes("reasoning effort Low"),
    "The model chip should expose this thread's effort override in accessible metadata",
  );
  assert(
    overriddenControlBarMarkup.includes('<span class="chip-detail">OpenAI</span>'),
    "A reasoning override should not replace the closed chip's provider route",
  );

  const { providerBrandForModel, providerBrandForProvider } = (await server.ssrLoadModule(
    "/src/components/ProviderIcon.tsx",
  )) as {
    providerBrandForModel: (model: Pick<ModelInfo, "id" | "owned_by" | "provider_id">, providers?: ProviderInfo[]) => string | null;
    providerBrandForProvider: (provider: Pick<ProviderInfo, "name" | "base_url">) => string | null;
  };
  assert(providerBrandForProvider(providers[0]) === "openai", "Known provider endpoints should resolve their brand");
  assert(
    providerBrandForProvider({ name: "vLLM (local)", base_url: "http://localhost:8000/v1" }) === "vllm",
    "The standard vLLM discovery candidate should resolve its brand",
  );
  assert(
    providerBrandForProvider({ name: "vLLM (Docker: qwen38-vllm)", base_url: "http://localhost:18000/v1" }) === "vllm",
    "Docker vLLM discovery candidates should resolve their brand on arbitrary published ports",
  );
  assert(
    providerBrandForModel({ id: "pi:github-copilot/claude", owned_by: "Local Pi CLI" }) === "pi",
    "Account-runtime prefixes should win over the underlying model provider",
  );
  assert(
    providerBrandForProvider({ name: "Custom", base_url: "https://example.com/v1" }) === null,
    "Custom endpoints should keep the generic connection icon",
  );

  const { BatonMenu, HotSwapPreflightSheet } = (await server.ssrLoadModule(
    "/src/components/HotSwapDialogs.tsx",
  )) as {
    BatonMenu: ComponentType<Record<string, unknown>>;
    HotSwapPreflightSheet: ComponentType<Record<string, unknown>>;
  };
  const batonMarkup = renderToStaticMarkup(
    createElement(BatonMenu, {
      retryDisabled: true,
      onAction: () => {},
    }),
  );
  assert(batonMarkup.includes('data-testid="baton-continue"'), "Continue should stay visible without hover or opening a menu");
  assert(batonMarkup.includes(">Continue with...</span>"), "The visible handoff action should be labeled Continue with");
  assert(batonMarkup.includes("<button"), "The visible Continue action should use a keyboard-focusable native button");
  assert(batonMarkup.includes('data-testid="baton-menu-trigger"'), "Review and Retry should remain available from the handoff menu");
  assert(!batonMarkup.includes("Model handoff actions"), "Closed Baton actions should not leave a hidden menu in the message layout");
  const hotSwapSource = readFileSync(resolve(process.cwd(), "src/components/HotSwapDialogs.tsx"), "utf8");
  const styles = ["chat.css", "workspaces.css"]
    .map((file) =>
      readFileSync(resolve(process.cwd(), "src", file), "utf8"),
    )
    .join("\n");
  assert(hotSwapSource.includes("createPortal("), "Baton actions should render through a body portal");
  assert(hotSwapSource.includes('className="baton-menu-popover message-popover-layer"'), "Baton actions should use the shared message popover layer");
  assert(hotSwapSource.includes("Continue with...") && hotSwapSource.includes("Review with...") && hotSwapSource.includes("Retry with..."), "Baton controls should offer all handoff actions");
  assert(
    styles.includes(".baton-menu :is(.baton-continue, .baton-menu-trigger):focus-visible"),
    "The always-visible Continue action should expose keyboard focus",
  );
  assert(styles.includes(".message-popover-layer") && styles.includes("z-index: 1200 !important"), "Message popovers should render above the sidebar layer");
  assert(styles.includes(".mp-route") && styles.includes("max-width: 42%"), "Picker routes should truncate within compact rows");
  const effortStyles = styles.match(/\.mp-effort-label\s*\{([^}]*)\}/)?.[1] ?? "";
  assert(effortStyles.includes("line-height: 1"), "Reasoning effort should share the metadata baseline");
  assert(!effortStyles.includes("transform:"), "Reasoning effort should not be vertically offset");

  const hotSwapMarkup = renderToStaticMarkup(
    createElement(HotSwapPreflightSheet, {
      fromModel: "model-a",
      targetModel: "codex:gpt-5",
      assessment: {
        parity: "degraded",
        requiresConfirmation: true,
        nativeSessionStale: true,
        nativeRuntime: "codex",
        issues: [{
          code: "native_session_stale",
          parity: "degraded",
          title: "Native session is behind",
          detail: "Choose Fresh or Resume.",
        }],
      },
      onConfirm: () => {},
      onClose: () => {},
    }),
  );
  assert(hotSwapMarkup.includes("Hot Swap"), "Hot Swap preflight should render");
  assert(hotSwapMarkup.includes("Fresh"), "Hot Swap should offer a fresh native session");
  assert(hotSwapMarkup.includes("Resume"), "Hot Swap should offer native-session resume");
} finally {
  await server.close();
}

export {};
