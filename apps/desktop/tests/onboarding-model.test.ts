import { equal } from "node:assert/strict";
import {
  ACCOUNT_RUNTIME_SIGN_IN_HELP,
  connectedSourceModel,
  isChatCapableModel,
  isMediaOnlyProviderKind,
  workspaceFolderPlaceholder,
} from "../src/lib/onboardingModel.js";
import { PROVIDER_PRESETS } from "../src/api.js";
import { RUNTIME_INSTALL_COMMANDS, runtimeMissingMessage } from "../src/lib/runtimeInstall.js";

const old = { id: "old", owned_by: "OpenAI", provider_id: "old-provider" };
const connected = { id: "new", owned_by: "OpenAI", provider_id: "new-provider" };
equal(connectedSourceModel([old], { providerId: "new-provider" }), undefined, "a cached unrelated source must not be selected while refresh is pending");
equal(connectedSourceModel([old, connected], { providerId: "new-provider" }), connected, "same-brand providers must still be selected by exact provider id");
equal(connectedSourceModel([old], { owner: "Codex" }), undefined, "an unavailable account runtime must not fall back to another source");
const codex = { id: "codex:model", owned_by: "Codex" };
equal(connectedSourceModel([old, codex], { owner: "codex" }), codex);

const fluxImage = { id: "black-forest-labs/flux-schnell", owned_by: "Replicate", provider_id: "replicate" };
const imageOutput = { id: "vendor/renderer", owned_by: "OpenRouter", provider_id: "openrouter", capabilities: { imageOutput: true } };
const chat = { id: "gpt-5", owned_by: "OpenAI", provider_id: "openrouter" };
equal(isChatCapableModel(fluxImage), false, "media model ids are not chat-capable");
equal(isChatCapableModel(imageOutput), false, "image-output models are not chat-capable");
equal(isChatCapableModel({ id: "mock-echo" }), false, "the mock echo model is not a usable chat model");
equal(isChatCapableModel(chat), true, "ordinary chat models are chat-capable");
equal(isChatCapableModel({ id: "codex:gpt-5-codex" }), true, "account-runtime models are chat-capable");
equal(connectedSourceModel([imageOutput, chat], { providerId: "openrouter" }), chat, "a newly connected source should select its first chat-capable model");
equal(connectedSourceModel([fluxImage], { providerId: "replicate" }), undefined, "a media-only source must not satisfy the runtime step");

const chatPresets = PROVIDER_PRESETS.filter((preset) => preset.needsKey && !isMediaOnlyProviderKind(preset.kind)).map((preset) => preset.name);
equal(chatPresets.includes("Replicate") || chatPresets.includes("fal"), false, "media-only presets are excluded from the chat provider step");
equal(chatPresets.includes("OpenAI"), true, "hosted chat presets remain available");

equal(workspaceFolderPlaceholder("MacIntel Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"), "/Users/you/project", "macOS gets a POSIX home path");
equal(workspaceFolderPlaceholder("Win32 Mozilla/5.0 (Windows NT 10.0; Win64; x64)"), "C:\\path\\to\\project", "Windows gets a drive path");
equal(workspaceFolderPlaceholder("Linux x86_64"), "/home/you/project", "Linux gets a POSIX home path");

for (const runtime of ["claude", "opencode", "pi"] as const) {
  equal(ACCOUNT_RUNTIME_SIGN_IN_HELP[runtime].url.startsWith("https://docs.milim.ai/"), true, `${runtime} sign-in help should link to the docs site`);
}

equal(RUNTIME_INSTALL_COMMANDS.codex, "npm install -g @openai/codex");
equal(RUNTIME_INSTALL_COMMANDS.claude, "npm install -g @anthropic-ai/claude-code");
equal(RUNTIME_INSTALL_COMMANDS.opencode, "npm install -g opencode-ai");
equal(RUNTIME_INSTALL_COMMANDS.pi, "npm install -g @earendil-works/pi-coding-agent");
for (const kind of ["codex", "claude", "opencode", "pi"] as const) {
  const message = runtimeMissingMessage(kind);
  equal(message.includes(RUNTIME_INSTALL_COMMANDS[kind]), true, `${kind} missing message should include its install command`);
  equal(message.includes("Locate binary"), true, `${kind} missing message should offer Locate binary`);
  equal(message.includes("on PATH."), false, `${kind} missing message should not be the bare PATH error`);
}
