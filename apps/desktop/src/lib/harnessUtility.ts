import {
  claudeRuntimeModel,
  codexRuntimeModel,
  opencodeRuntimeModel,
  piRuntimeModel,
  streamHarnessRun,
  type AccountRuntimeKind,
  type HarnessEventEnvelope,
  type HarnessRunRequest,
  type TokenUsage,
} from "../api.js";

/** An account runtime and the runtime-native model id a utility call runs. */
export type UtilityHarness = { id: AccountRuntimeKind; model: string };

export type HarnessUtilityResult = {
  content: string;
  usage?: TokenUsage;
  costUsd?: number;
  costSource?: "provider";
};

/** The account runtime that owns `model`, or null for provider models. */
export function utilityHarnessForModel(model: string): UtilityHarness | null {
  const codex = codexRuntimeModel(model);
  if (codex) return { id: "codex", model: codex };
  const claude = claudeRuntimeModel(model);
  if (claude) return { id: "claude", model: claude };
  const opencode = opencodeRuntimeModel(model);
  if (opencode) return { id: "opencode", model: opencode };
  const pi = piRuntimeModel(model);
  if (pi) return { id: "pi", model: pi };
  return null;
}

/**
 * Run one non-persistent side call (compaction summary, goal decision, commit
 * message) on an account runtime and collect its text. A side call bills a
 * subscription, so callers pass the account the originating chat selected
 * rather than letting the runtime fall back to its default account.
 */
export async function collectHarnessUtilityRun(
  harnessId: AccountRuntimeKind,
  request: HarnessRunRequest,
  options: { accountProfileId?: string; signal?: AbortSignal } = {},
): Promise<HarnessUtilityResult> {
  let content = "";
  let warning: string | null = null;
  let error: string | null = null;
  let usage: TokenUsage | undefined;
  let costUsd: number | undefined;
  let costSource: "provider" | undefined;
  await streamHarnessRun(
    harnessId,
    options.accountProfileId
      ? { ...request, account_profile_id: options.accountProfileId }
      : request,
    (envelope: HarnessEventEnvelope) => {
      const event = envelope.event;
      if (event.type === "text_delta" && event.text) {
        content += event.text;
      } else if (event.type === "runtime_notice") {
        if (event.level === "error") error = event.message;
        else if (event.code === "runtime_warning") warning = event.message;
      } else if (
        event.type === "turn_failed" ||
        event.type === "turn_cancelled"
      ) {
        error = event.message ?? "Harness turn was cancelled.";
      } else if (
        event.type === "usage_updated" ||
        event.type === "turn_completed"
      ) {
        if (event.usage) usage = event.usage;
        if (typeof event.cost_usd === "number" && event.cost_usd >= 0) {
          costUsd = event.cost_usd;
          costSource = "provider";
        }
      }
    },
    options.signal,
  );
  if (error) throw new Error(error);
  if (warning) throw new Error(warning);
  return { content, usage, costUsd, costSource };
}
