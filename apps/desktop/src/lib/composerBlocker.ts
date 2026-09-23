import {
  accountRuntimeKind,
  type AccountRuntimeEnablement,
  type ModelInfo,
  type ProviderInfo,
} from "../api.js";
import { modelDevProfile } from "./modelPicker.js";
import {
  classifyError,
  errorRecoveryAction,
  type ErrorRecoveryAction,
  type ProviderErrorInfo,
} from "./providerErrors.js";

export type ComposerBlockerAction = ErrorRecoveryAction;

export type ComposerBlocker = {
  message: string;
  tone: "warning" | "error";
  action: ComposerBlockerAction;
};

export function prioritizeComposerNotice<T extends { tone: "info" | "warning" | "error" }>(
  current: T | null,
  proactive: ComposerBlocker | null,
): T | ComposerBlocker | null {
  return current && current.tone !== "info" ? current : proactive ?? current;
}

export function modelComposerBlocker({
  modelsLoaded,
  selectedModel,
  models,
  providers,
  accountRuntimeEnabled,
}: {
  modelsLoaded: boolean;
  selectedModel: string;
  models: ModelInfo[];
  providers: ProviderInfo[];
  accountRuntimeEnabled: AccountRuntimeEnablement;
}): ComposerBlocker | null {
  if (!modelsLoaded) return null;
  const selected = selectedModel.trim();
  if (!selected) {
    return {
      tone: "error",
      message: "Choose a reachable model before sending.",
      action: "manage_models",
    };
  }
  const runtime = accountRuntimeKind(selected);
  if (runtime && !accountRuntimeEnabled[runtime]) {
    return {
      tone: "error",
      message: `${runtime === "claude" ? "Claude" : runtime === "opencode" ? "OpenCode" : runtime === "pi" ? "Pi" : "Codex"} is disabled in Providers.`,
      action: "manage_models",
    };
  }
  const model = models.find((item) => item.id === selected);
  const profile = modelDevProfile(model, selected, { providers });
  if (profile.setupTone === "ready") return null;
  return {
    tone: profile.setupTone === "warning" ? "warning" : "error",
    message: model
      ? `${profile.setupLabel}: ${profile.setupDetail}`
      : `${selected} is unavailable. Configure its provider or choose another model.`,
    action: "manage_models",
  };
}

const PROGRESS_INFO_NOTICE = /^(Goal running\.|Starting Codex login\.\.\.)$/i;

export function composerNoticeAutoDismissMs(
  notice: { tone: "info" | "warning" | "error"; message: string } | null,
): number | null {
  if (!notice || notice.tone !== "info") return null;
  const message = notice.message.trim();
  if (!message || message.endsWith("...") || PROGRESS_INFO_NOTICE.test(message)) return null;
  return 5000;
}

export function composerNoticeIsDismissible<T extends { tone: "info" | "warning" | "error" }>(
  notice: T | ComposerBlocker | null,
  proactive: ComposerBlocker | null,
): boolean {
  return Boolean(notice && notice !== proactive);
}

/** The recovery action for a notice, from its structured classification when
 * present, otherwise from the shared error classifier. */
export function composerNoticeAction(
  message: string,
  providerError?: ProviderErrorInfo,
): ComposerBlockerAction | null {
  return errorRecoveryAction(classifyError(message, providerError).kind);
}

export function composerActionLabel(
  action: ComposerBlockerAction | null,
  retryInSecs = 0,
): string {
  switch (action) {
    case "manage_models":
      return "Open Providers";
    case "choose_folder":
      return "Choose folder";
    case "privacy_settings":
      return "Review privacy";
    case "update_key":
      return "Update key";
    case "switch_model":
      return "Switch model";
    case "retry":
      return retryInSecs > 0 ? `Retry in ${retryInSecs} s` : "Retry";
    default:
      return "";
  }
}
