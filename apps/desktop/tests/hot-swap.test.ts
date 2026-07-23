import assert from "node:assert/strict";
import type { ModelInfo, ProviderInfo } from "../src/api.js";
import { assessHotSwap, nativeRuntimeIsStale } from "../src/lib/hotSwap.js";

const providers: ProviderInfo[] = [{
  id: "ready",
  name: "Ready",
  kind: "openai_compatible",
  base_url: "https://example.com/v1",
  enabled: true,
  has_key: true,
  models: ["model-a", "model-b", "tiny", "no-tools", "text-only"],
}];
const models: ModelInfo[] = [
  { id: "model-a", owned_by: "Ready", provider_id: "ready", context_length: 32_000, capabilities: { toolUse: true, imageInput: true } },
  { id: "model-b", owned_by: "Ready", provider_id: "ready", context_length: 32_000, capabilities: { toolUse: true, imageInput: true } },
  { id: "tiny", owned_by: "Ready", provider_id: "ready", context_length: 600, max_completion_tokens: 100, capabilities: { toolUse: true } },
  { id: "no-tools", owned_by: "Ready", provider_id: "ready", context_length: 32_000, capabilities: { toolUse: false } },
  { id: "text-only", owned_by: "Ready", provider_id: "ready", context_length: 32_000, capabilities: { imageInput: false } },
  { id: "codex:gpt", owned_by: "Codex", context_length: 32_000 },
  { id: "pi:openai-codex/gpt", owned_by: "Local Pi CLI", context_length: 32_000 },
];

const baseSession = {
  messages: [
    { id: "u1", role: "user", content: "Implement it" },
    { id: "a1", role: "assistant", content: "Done" },
  ],
  accountRuntime: undefined,
};

assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[1],
  models,
  providers,
  session: baseSession,
}).parity, "full");

const longMessages = Array.from({ length: 8 }, (_, index) => ({
  id: `m${index}`,
  role: index % 2 ? "assistant" : "user",
  content: "context ".repeat(120),
}));
assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[2],
  models,
  providers,
  session: { messages: longMessages, accountRuntime: undefined },
}).issues.some((issue) => issue.code === "context_compaction_required"), true);

assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[3],
  models,
  providers,
  session: baseSession,
  toolRequired: true,
}).parity, "blocked");

const imageSession = {
  messages: [{
    id: "image-user",
    role: "user",
    content: "Inspect",
    attachments: [{ id: "img", name: "shot.png", mime: "image/png", size: 4, dataUrl: "data:image/png;base64,AAAA" }],
  }],
  accountRuntime: undefined,
};
assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[4],
  models,
  providers,
  session: imageSession,
}).issues.some((issue) => issue.code === "image_pixels_unavailable"), true);

assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[5],
  models,
  providers,
  session: imageSession,
}).issues.some((issue) => issue.code === "image_pixels_unavailable"), false);

const stale = {
  messages: [
    { id: "a1", role: "assistant", content: "Codex answer" },
    { id: "u2", role: "user", content: "Another model continued" },
  ],
  accountRuntime: { codexThreadId: "thread", codexLastSyncedMessageId: "a1" },
};
assert.equal(nativeRuntimeIsStale(stale, "codex"), true);
assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[5],
  models,
  providers,
  session: stale,
}).nativeSessionStale, true);

const stalePi = {
  messages: [
    { id: "a1", role: "assistant", content: "Pi answer" },
    { id: "u2", role: "user", content: "Another model continued" },
  ],
  accountRuntime: { piSessionId: "pi-session", piLastSyncedMessageId: "a1" },
};
assert.equal(nativeRuntimeIsStale(stalePi, "pi"), true);
assert.equal(assessHotSwap({
  currentModel: "model-a",
  target: models[6],
  models,
  providers,
  session: stalePi,
}).nativeSessionStale, true);
