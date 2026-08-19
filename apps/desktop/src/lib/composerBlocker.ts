import {
  accountRuntimeKind,
  type AccountRuntimeEnablement,
  type ModelInfo,
  type ProviderInfo,
} from "../api.js";
import { modelDevProfile } from "./modelPicker.js";

export type ComposerBlockerAction = "manage_models" | "choose_folder" | "privacy_settings";

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

export function composerNoticeAction(message: string): ComposerBlockerAction | null {
  const text = message.toLowerCase();
  if (text.includes("blocked by the privacy gate")) return "privacy_settings";
  if (
    text.includes("no working folder selected") ||
    text.includes("no working folder is selected") ||
    text.includes("workspace folder is required") ||
    text.includes("select a milim workspace folder")
  ) return "choose_folder";
  if (
    text.includes("not signed in") ||
    text.includes("no configured models") ||
    text.includes("no authenticated or configured models") ||
    text.includes("cli is unavailable") ||
    text.includes("codex is unavailable") ||
    text.includes("codex login did not complete") ||
    text.includes("disabled in providers")
  ) return "manage_models";
  return null;
}
