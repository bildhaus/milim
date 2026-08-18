import { deepEqual, equal } from "node:assert/strict";
import {
  generationOverridesWithSelection,
  normalizeGenerationOverrides,
  normalizeGenerationSettings,
} from "../src/lib/generationSettings.js";

deepEqual(normalizeGenerationSettings({
  maxTokens: 4096,
  temperature: 0.6,
  topP: 0,
  stop: [" END ", "", "x".repeat(257)],
  topK: -1,
  minP: 0.1,
  repetitionPenalty: 1.05,
  thinkingTokenBudget: 2048,
}), {
  maxTokens: 4096,
  temperature: 0.6,
  stop: ["END"],
  topK: -1,
  minP: 0.1,
  repetitionPenalty: 1.05,
  thinkingTokenBudget: 2048,
});

const selected = generationOverridesWithSelection(undefined, "qwen", { topK: 40 });
deepEqual(selected, { qwen: { topK: 40 } });
equal(generationOverridesWithSelection(selected, "qwen", {}), undefined);

deepEqual(
  normalizeGenerationOverrides({ qwen: {}, llama: { temperature: 0.2 } }),
  { llama: { temperature: 0.2 } },
);
