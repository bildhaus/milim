import type { ChatMessage, ModelInfo, ProviderInfo } from "../api.js";
import type { Session } from "../sessions/store.js";
import { contextSendPlan, messagesForModelContext } from "./contextCompaction.js";
import { modelDevProfile } from "./modelPicker.js";

export type HotSwapParity = "full" | "translated" | "degraded" | "blocked";
export type HotSwapIssueCode =
  | "target_unavailable"
  | "context_compaction_required"
  | "context_too_large"
  | "image_pixels_unavailable"
  | "tool_use_unavailable"
  | "native_session_stale";

export interface HotSwapIssue {
  code: HotSwapIssueCode;
  parity: Exclude<HotSwapParity, "full">;
  title: string;
  detail: string;
}

export interface HotSwapAssessment {
  parity: HotSwapParity;
  issues: HotSwapIssue[];
  requiresConfirmation: boolean;
  nativeRuntime?: "codex" | "claude" | "opencode" | "pi";
  nativeSessionStale: boolean;
}

const PARITY_WEIGHT: Record<HotSwapParity, number> = {
  full: 0,
  translated: 1,
  degraded: 2,
  blocked: 3,
};

function worstParity(issues: HotSwapIssue[]): HotSwapParity {
  return issues.reduce<HotSwapParity>(
    (worst, issue) =>
      PARITY_WEIGHT[issue.parity] > PARITY_WEIGHT[worst]
        ? issue.parity
        : worst,
    "full",
  );
}

function runtimeKind(
  model: string,
): "codex" | "claude" | "opencode" | "pi" | undefined {
  const value = model.trim().toLowerCase();
  if (value.startsWith("codex:")) return "codex";
  if (value.startsWith("claude:")) return "claude";
  if (value.startsWith("opencode:")) return "opencode";
  if (value.startsWith("pi:")) return "pi";
  return undefined;
}

function hasImagePixels(messages: readonly ChatMessage[]): boolean {
  return messages.some((message) =>
    message.attachments?.some((attachment) => Boolean(attachment.dataUrl)),
  );
}

export function nativeRuntimeIsStale(
  session: Pick<Session, "messages" | "accountRuntime">,
  kind: "codex" | "claude" | "opencode" | "pi",
): boolean {
  const runtime = session.accountRuntime;
  const sessionId = kind === "codex"
    ? runtime?.codexThreadId
    : kind === "claude"
      ? runtime?.claudeSessionId
      : kind === "opencode"
        ? runtime?.opencodeSessionId
        : runtime?.piSessionId;
  if (!sessionId) return false;
  const cursor = kind === "codex"
    ? runtime?.codexLastSyncedMessageId
    : kind === "claude"
      ? runtime?.claudeLastSyncedMessageId
      : kind === "opencode"
        ? runtime?.opencodeLastSyncedMessageId
        : runtime?.piLastSyncedMessageId;
  if (!cursor) return true;
  const index = session.messages.findIndex((message) => message.id === cursor);
  if (index < 0) return true;
  return session.messages.slice(index + 1).some(
    (message) =>
      !message.approval &&
      !message.compaction &&
      (message.role === "user" || message.role === "assistant") &&
      Boolean(message.content.trim() || message.attachments?.length),
  );
}

export function assessHotSwap({
  currentModel,
  target,
  models,
  providers = [],
  session,
  contextMessages = [],
  toolRequired = false,
}: {
  currentModel: string;
  target: ModelInfo;
  models: readonly ModelInfo[];
  providers?: ProviderInfo[];
  session: Pick<Session, "messages" | "accountRuntime">;
  contextMessages?: ChatMessage[];
  toolRequired?: boolean;
}): HotSwapAssessment {
  if (target.id === currentModel) {
    return { parity: "full", issues: [], requiresConfirmation: false, nativeSessionStale: false };
  }

  const issues: HotSwapIssue[] = [];
  const profile = modelDevProfile(target, target.id, { providers, toolIntent: toolRequired });
  if (profile.setupTone !== "ready") {
    issues.push({
      code: "target_unavailable",
      parity: "blocked",
      title: profile.setupLabel,
      detail: profile.setupDetail,
    });
  }

  const sendPlan = contextSendPlan(contextMessages, session.messages, target.id, models);
  if (sendPlan.error) {
    issues.push({
      code: "context_too_large",
      parity: "blocked",
      title: "Context does not fit",
      detail: sendPlan.error,
    });
  } else if (sendPlan.shouldCompact) {
    issues.push({
      code: "context_compaction_required",
      parity: "translated",
      title: "Context will be compacted",
      detail: `${target.id} has a smaller prompt budget, so Milim will create a checkpoint before the next send.`,
    });
  }

  if (toolRequired && target.capabilities?.toolUse === false) {
    issues.push({
      code: "tool_use_unavailable",
      parity: "blocked",
      title: "Tool use unavailable",
      detail: "The current workspace flow requires tools, but this model explicitly reports no tool-use support.",
    });
  }

  const visibleMessages = messagesForModelContext(contextMessages, session.messages);
  const targetRuntime = runtimeKind(target.id);
  if (
    hasImagePixels(visibleMessages) &&
    target.capabilities?.imageInput === false
  ) {
    issues.push({
      code: "image_pixels_unavailable",
      parity: "degraded",
      title: "Image pixels will not transfer",
      detail: "The target model explicitly reports that it does not support image input.",
    });
  }

  const stale = targetRuntime ? nativeRuntimeIsStale(session, targetRuntime) : false;
  if (stale) {
    issues.push({
      code: "native_session_stale",
      parity: "degraded",
      title: "Native session is behind",
      detail: "Choose whether to start fresh from Milim context or resume the older native session with intervening turns.",
    });
  }

  const parity = worstParity(issues);
  return {
    parity,
    issues,
    requiresConfirmation: parity !== "full",
    nativeRuntime: targetRuntime,
    nativeSessionStale: stale,
  };
}
