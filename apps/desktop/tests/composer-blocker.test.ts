import { equal } from "node:assert/strict";
import type { AccountRuntimeEnablement, ModelInfo, ProviderInfo } from "../src/api";
import {
  composerActionLabel,
  composerNoticeAction,
  composerNoticeAutoDismissMs,
  composerNoticeIsDismissible,
  modelComposerBlocker,
  prioritizeComposerNotice,
} from "../src/lib/composerBlocker.js";
import {
  classifyErrorMessage,
  errorNotice,
  friendlyError,
  providerErrorFromValue,
} from "../src/lib/providerErrors.js";

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
equal(
  composerNoticeAction("Claude CLI was not found on PATH. Install it with `npm install -g @anthropic-ai/claude-code`."),
  "manage_models",
  "a missing CLI should link to Providers",
);

// Provider failures: HTTP status, well-known phrases, and retry hints.
equal(
  composerNoticeAction("upstream error: OpenAI chat/completions -> 401 Unauthorized: {\"error\":{\"message\":\"Incorrect API key\"}}"),
  "update_key",
);
equal(classifyErrorMessage("x chat/completions -> 403 Forbidden: denied").kind, "auth");
equal(classifyErrorMessage("x messages -> 529 <unknown status code>: overloaded").kind, "provider_unavailable");
equal(classifyErrorMessage("x chat/completions -> 503 Service Unavailable: ").status, 503);
equal(classifyErrorMessage("chat HTTP 502").kind, "provider_unavailable");
equal(classifyErrorMessage("x chat/completions -> 404 Not Found: The model `gpt-9` does not exist").kind, "model_not_found");
equal(composerNoticeAction("model_not_found: unknown model gpt-9"), "switch_model");
equal(classifyErrorMessage("x chat/completions -> 400 Bad Request: context_length_exceeded").kind, "context_length");
equal(composerNoticeAction("prompt is too long: 210000 tokens > 200000 maximum"), "switch_model");
equal(
  classifyErrorMessage("x chat/completions -> 429 Too Many Requests: {\"code\":\"insufficient_quota\"}").kind,
  "quota",
  "exhausted quota is not a transient rate limit",
);
const limited = classifyErrorMessage("x chat/completions -> 429 Too Many Requests: slow down (retry after 12s)");
equal(limited.kind, "rate_limited");
equal(limited.retryAfterSecs, 12);
equal(classifyErrorMessage("Rate limit reached. Please try again in 1.5s.").retryAfterSecs, 2);
equal(classifyErrorMessage("rate_limit_exceeded: try again in 250ms").retryAfterSecs, 1);
equal(composerNoticeAction("rate limit reached"), "retry");
equal(classifyErrorMessage("tool call 42 failed at step 500").kind, "unknown", "bare numbers are not statuses");

// Structured classifications from canonical run state win over text.
equal(
  composerNoticeAction("account runtime failed", { kind: "rate_limited", retry_after_secs: 5 }),
  "retry",
);
equal(providerErrorFromValue({ kind: "auth", status: 401 })?.status, 401);
equal(providerErrorFromValue({ kind: "nonsense" }), undefined);
equal(providerErrorFromValue(null), undefined);

const friendly = friendlyError("upstream error: x chat/completions -> 401 Unauthorized: bad key");
equal(friendly.kind, "auth");
equal(friendly.detail, "upstream error: x chat/completions -> 401 Unauthorized: bad key", "raw text stays available as technical detail");
equal(friendly.message.includes("401"), false, "friendly copy hides the raw status line");
equal(friendlyError("Attachment content is unavailable.").detail, undefined, "unrecognized errors show their own text");
const notice = errorNotice("x -> 429 Too Many Requests: busy", { kind: "rate_limited", status: 429, retry_after_secs: 30 });
equal(notice.retryAfterSecs, 30);
equal(notice.detail, "x -> 429 Too Many Requests: busy");

equal(composerActionLabel("update_key"), "Update key");
equal(composerActionLabel("switch_model"), "Switch model");
equal(composerActionLabel("retry", 7), "Retry in 7 s");
equal(composerActionLabel("retry", 0), "Retry");
equal(composerActionLabel(null), "");

const authoritative = { tone: "error" as const, message: "blocked by the privacy gate" };
const proactive = {
  tone: "warning" as const,
  message: "Model unavailable",
  action: "manage_models" as const,
};
equal(prioritizeComposerNotice(authoritative, proactive), authoritative, "authoritative failures should win");
equal(prioritizeComposerNotice({ tone: "info", message: "Saved" }, proactive), proactive, "a blocker should replace informational status");

equal(composerNoticeAutoDismissMs({ tone: "info", message: "Goal saved." }), 5000, "transient info notices should auto-dismiss");
equal(composerNoticeAutoDismissMs({ tone: "error", message: "preview app requires package.json" }), null, "errors should stay until dismissed");
equal(composerNoticeAutoDismissMs({ tone: "info", message: "Compacting thread context..." }), null, "progress notices should stay while work is running");
equal(composerNoticeAutoDismissMs({ tone: "info", message: "Goal running." }), null, "goal progress should stay while the goal is running");
equal(
  composerNoticeIsDismissible({ tone: "error", message: "preview app requires package.json" }, proactive),
  true,
  "chat notices should be dismissible",
);
equal(composerNoticeIsDismissible(proactive, proactive), false, "proactive blockers should stay until the condition clears");
