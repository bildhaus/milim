import { deepEqual, equal } from "node:assert/strict";
import { readFileSync } from "node:fs";
import type { ModelInfo } from "../src/api";

const { hasReasoningEffortChoices, normalizeReasoningEffortOverrides, reasoningEffortByModelWithSelection, reasoningEffortDisplay, reasoningEffortForModel, reasoningEffortForThread, reasoningEffortOptions, reasoningEffortOverridesWithSelection } = await import("../src/lib/reasoningEffort.js");

const optional: ModelInfo = {
  id: "openrouter/deepseek-r1",
  owned_by: "openrouter",
  reasoning: {
    supported_efforts: ["none", "low", "high"],
    default_effort: "medium",
    default_enabled: true,
    mandatory: false,
  },
};

const mandatory: ModelInfo = {
  id: "claude-sonnet-4",
  owned_by: "anthropic",
  reasoning: {
    supported_efforts: ["none", "low", "high"],
    default_effort: "high",
    default_enabled: true,
    mandatory: true,
  },
};
const autoOnly: ModelInfo = {
  id: "plain-model",
  owned_by: "provider",
  reasoning: { supported_efforts: [], default_effort: undefined, default_enabled: true, mandatory: false },
};
const ollamaThinking: ModelInfo = {
  id: "deepseek-r1",
  owned_by: "Ollama",
  reasoning: {
    supported_efforts: ["none", "low", "medium", "high", "max"],
    default_effort: "medium",
    default_enabled: true,
    mandatory: false,
  },
};
const lmStudioGptOss: ModelInfo = {
  id: "openai/gpt-oss-20b",
  owned_by: "LM Studio",
  reasoning: {
    supported_efforts: ["low", "medium", "high"],
    default_effort: "medium",
    default_enabled: true,
    mandatory: true,
  },
};
const lmStudioNative: ModelInfo = {
  id: "deepseek-r1",
  owned_by: "LM Studio",
  reasoning: {
    supported_efforts: ["none", "on"],
    default_effort: "on",
    default_enabled: true,
    mandatory: false,
  },
};

deepEqual(reasoningEffortOptions(optional), ["auto", "none", "low", "high"], "optional models should expose off");
deepEqual(reasoningEffortOptions(mandatory), ["auto", "low", "high"], "mandatory models should hide off");
deepEqual(reasoningEffortOptions(ollamaThinking), ["auto", "none", "low", "medium", "high", "max"], "Ollama thinking models should expose local effort choices");
deepEqual(reasoningEffortOptions(lmStudioGptOss), ["auto", "low", "medium", "high"], "LM Studio gpt-oss should expose Responses effort choices");
deepEqual(reasoningEffortOptions(lmStudioNative), ["auto", "none", "on"], "LM Studio native reasoning should expose advertised options");
equal(hasReasoningEffortChoices(autoOnly), false, "auto-only models should not show reasoning controls");
equal(reasoningEffortForModel({ [optional.id]: "high", [mandatory.id]: "low" }, optional.id, [optional, mandatory]), "high", "effort should be read per model");
equal(reasoningEffortForModel({ [mandatory.id]: "none" }, mandatory.id, [mandatory]), "auto", "unsupported selections should fall back to auto");
equal(reasoningEffortForModel({ [lmStudioGptOss.id]: "max" }, lmStudioGptOss.id, [lmStudioGptOss]), "auto", "LM Studio unsupported saved efforts should fall back to auto");
equal(reasoningEffortForModel({ [lmStudioNative.id]: "on" }, lmStudioNative.id, [lmStudioNative]), "on", "LM Studio native on should persist");
equal(reasoningEffortForModel({ [lmStudioNative.id]: "max" }, lmStudioNative.id, [lmStudioNative]), "auto", "LM Studio native unsupported saved efforts should fall back to auto");
equal(reasoningEffortForModel({ "unknown-model": "high" }, "unknown-model", [optional]), "auto", "unknown models should not send effort overrides");
deepEqual(
  reasoningEffortByModelWithSelection({ [optional.id]: "low" }, optional.id, "auto"),
  {},
  "choosing auto should remove the per-model override",
);
equal(
  reasoningEffortForThread({ [optional.id]: "low" }, { [optional.id]: "high" }, optional.id, [optional]),
  "low",
  "a thread override should win over the app-wide default",
);
equal(
  reasoningEffortForThread({ [mandatory.id]: "low" }, { [optional.id]: "high" }, optional.id, [optional, mandatory]),
  "high",
  "a model without a thread override should inherit the app-wide default",
);
equal(
  reasoningEffortForThread({ [optional.id]: "auto" }, { [optional.id]: "high" }, optional.id, [optional]),
  "auto",
  "an explicit auto override should opt a thread out of a non-auto default",
);
equal(
  reasoningEffortForThread({ [mandatory.id]: "none" }, undefined, mandatory.id, [mandatory]),
  "auto",
  "unsupported thread overrides should fall back to auto",
);
equal(
  reasoningEffortForThread(undefined, undefined, optional.id, [optional]),
  "auto",
  "a thread with no override and no default should stay on auto",
);
deepEqual(
  reasoningEffortOverridesWithSelection({ [mandatory.id]: "low" }, optional.id, "auto"),
  { [mandatory.id]: "low", [optional.id]: "auto" },
  "choosing auto should be stored as a thread override rather than clearing it",
);
deepEqual(
  reasoningEffortOverridesWithSelection(undefined, "   ", "high"),
  {},
  "a blank model id should not create a thread override",
);
deepEqual(
  normalizeReasoningEffortOverrides({ "": "high", [optional.id]: "nope", [mandatory.id]: "low" }),
  { [mandatory.id]: "low" },
  "persisted thread overrides should drop blank models and unknown efforts",
);
deepEqual(normalizeReasoningEffortOverrides(["low"]), {}, "non-record thread overrides should normalize to empty");
equal(reasoningEffortDisplay("medium", { id: "gemini-2.5-pro", owned_by: "Gemini" }).detail, "budget 4k", "Gemini 2.5 labels should show budget mapping");
equal(reasoningEffortDisplay("high", { id: "gemini-3-pro", owned_by: "Gemini" }).detail, "thinking HIGH", "Gemini 3 labels should show thinking level mapping");
equal(reasoningEffortDisplay("xhigh", mandatory).detail, "effort xhigh", "Claude labels should use effort terminology");
equal(reasoningEffortDisplay("on", lmStudioNative).detail, "reasoning on", "LM Studio native labels should use reasoning terminology");

const modelPicker = readFileSync("src/components/ModelPicker.tsx", "utf8");
equal(modelPicker.includes("mp-effort-btn"), true, "model picker should expose a compact effort trigger");
equal(modelPicker.includes("mp-effort-menu"), true, "model picker should expose effort choices in a custom row menu");
equal(modelPicker.includes("createPortal("), true, "model picker should render the effort menu outside the clipped model list");
equal(modelPicker.includes("setModelReasoningEffort"), true, "model picker selector should persist the global per-model effort");
equal(modelPicker.includes("(onReasoningEffort ?? setModelReasoningEffort)"), true, "model picker should let a thread-scoped host own the effort choice");
equal(modelPicker.includes("reasoningEffortOverrides?.[m.id] ?? reasoningEffortByModel[m.id]"), true, "model picker rows should show a thread override before the app-wide default");
const chatView = readFileSync("src/components/ChatView.tsx", "utf8");
equal(chatView.includes("reasoningEffortForModel("), false, "chat turns should no longer resolve effort from the app-wide map alone");
equal(chatView.includes("setModelReasoningEffort"), false, "the in-chat effort menu should not retune the app-wide default");
equal(
  (chatView.match(/reasoningEffortForThread\(/g) ?? []).length,
  3,
  "sends, compaction, and goal decisions should each resolve effort thread-first",
);
equal(modelPicker.includes("hasReasoningEffortChoices(m)"), true, "model picker should hide auto-only reasoning controls");
equal(modelPicker.includes("cap !== \"reasoning\" || !hasEffortChoices"), true, "model picker should avoid duplicate reasoning icons");
equal(modelPicker.includes('setFavoritesOnly(!favoritesOnly)'), true, "model picker should expose the persisted favorites filter");
equal(modelPicker.includes("preset"), false, "model picker should keep Favorites as the only model shortcut");
equal(
  modelPicker.indexOf("className={\"mp-star\"") < modelPicker.indexOf("className=\"mp-pick\""),
  true,
  "model picker should render the favorite star before the model pick button",
);
const controlBar = readFileSync("src/components/ControlBar.tsx", "utf8");
equal(controlBar.includes("reasoning-effort-trigger"), false, "control bar should not render a separate reasoning chip");
equal(controlBar.includes('closest(".mp-effort-menu")'), true, "control bar outside-click handling should ignore the portaled effort menu");
