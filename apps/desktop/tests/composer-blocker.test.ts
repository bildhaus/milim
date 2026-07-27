import { equal } from "node:assert/strict";
import type { AccountRuntimeEnablement, ModelInfo, ProviderInfo } from "../src/api";
import {
  composerNoticeAction,
  modelComposerBlocker,
  prioritizeComposerNotice,
} from "../src/lib/composerBlocker.js";

const enabled: AccountRuntimeEnablement = {
  codex: true,
  claude: true,
  opencode: true,
  pi: true,
};

equal(modelComposerBlocker({
  modelsLoaded: false,
  selectedModel: "",
  models: [],
  providers: [],
  accountRuntimeEnabled: enabled,
}), null, "model loading should not flash a blocker");

equal(modelComposerBlocker({
  modelsLoaded: true,
  selectedModel: "",
  models: [],
  providers: [],
  accountRuntimeEnabled: enabled,
})?.action, "manage_models", "a missing model should link to model setup");

equal(modelComposerBlocker({
  modelsLoaded: true,
  selectedModel: "llama3.2",
  models: [{ id: "llama3.2", owned_by: "Ollama" }] satisfies ModelInfo[],
  providers: [],
  accountRuntimeEnabled: enabled,
}), null, "a ready model should not show readiness UI");

const unreachableProviders = [{
  id: "local",
  name: "LM Studio",
  kind: "openai_compatible",
  base_url: "http://127.0.0.1:1234/v1",
  enabled: true,
  has_key: false,
  models: ["qwen"],
  error: "Connection refused",
}] satisfies ProviderInfo[];
equal(modelComposerBlocker({
  modelsLoaded: true,
  selectedModel: "qwen",
  models: [{ id: "qwen", owned_by: "LM Studio", provider_id: "local" }],
  providers: unreachableProviders,
  accountRuntimeEnabled: enabled,
})?.message, "Unreachable: Connection refused", "an unreachable provider should explain the blocker");

equal(modelComposerBlocker({
  modelsLoaded: true,
  selectedModel: "claude:sonnet",
  models: [{ id: "claude:sonnet", owned_by: "Local Claude CLI" }],
  providers: [],
  accountRuntimeEnabled: { ...enabled, claude: false },
})?.action, "manage_models", "a disabled account runtime should link to model setup");

equal(composerNoticeAction("blocked by the privacy gate: outbound message contains email"), "privacy_settings");
equal(composerNoticeAction("no working folder selected - pick one first"), "choose_folder");
equal(composerNoticeAction("Claude CLI is not signed in."), "manage_models");
equal(composerNoticeAction("Codex is unavailable: login expired"), "manage_models");
equal(composerNoticeAction("Attachment content is unavailable."), null);

const authoritative = { tone: "error" as const, message: "blocked by the privacy gate" };
const proactive = {
  tone: "warning" as const,
  message: "Model unavailable",
  action: "manage_models" as const,
};
equal(prioritizeComposerNotice(authoritative, proactive), authoritative, "authoritative failures should win");
equal(prioritizeComposerNotice({ tone: "info", message: "Saved" }, proactive), proactive, "a blocker should replace informational status");
