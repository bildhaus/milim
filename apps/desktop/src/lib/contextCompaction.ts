import type { ChatAttachment, ChatCompactionMetrics, ChatCompactionSummaryMetrics, ChatMessage, ModelInfo, ProviderInfo, ReasoningEffort } from "../api";
import type { Tiktoken } from "js-tiktoken/lite";

const LOCAL_CONTEXT_WINDOW = 4096;
const HOSTED_CONTEXT_WINDOW = 32_768;
const COMPACT_AT = 0.85;
const COMPACT_TO = 0.7;
const SUMMARY_MAX_TOKENS = 900;
const SUMMARY_MIN_TOKENS = 128;
const SUMMARY_RETRY_EXTRA_TOKENS = 256;
const CHECKPOINT_HEADING = "### Context checkpoint";
const COMPACTION_TAIL_TURNS = 2;
const COMPACTION_TAIL_MAX_TOKENS = 8_000;
const COMPACTION_OLD_BODY_MAX_CHARS = 2_000;
let defaultTokenizer: Tiktoken | null | undefined;
let defaultTokenizerImport: Promise<void> | null = null;

export interface ModelContextBudget {
  contextLength: number;
  promptBudget: number;
}

export interface ContextCompactionResult {
  messages: ChatMessage[];
  compacted: boolean;
  originalTokens: number;
  sentTokens: number;
  budget: ModelContextBudget | null;
  error?: string;
}

export interface ContextSendPlan {
  messages: ChatMessage[];
  shouldCompact: boolean;
  originalTokens: number;
  sentTokens: number;
  budget: ModelContextBudget | null;
  error?: string;
}

export interface ContextSendPlanOptions {
  /** Fixed context injected by the server/runtime but not duplicated in outbound messages. */
  reservedMessages?: readonly ChatMessage[];
  fixedCategories?: ReadonlyArray<{ label: string; tokens: number }>;
}

export interface CompactionTailSplit {
  head: ChatMessage[];
  tail: ChatMessage[];
}

export interface CompactionSummaryOptions {
  retry?: boolean;
  outputCapTokens?: number;
}

export interface CompactionSummaryValidationOptions {
  finishReason?: string;
  model: string;
  models: readonly ModelInfo[];
  sourceMessages?: readonly ChatMessage[];
}

export function estimateTextTokens(text: string): number {
  const tokenizerTokens = estimateTextTokensWithTokenizer(text);
  if (tokenizerTokens != null) return tokenizerTokens;
  return estimateTextTokensFallback(text);
}

function estimateTextTokensWithTokenizer(text: string): number | null {
  try {
    if (defaultTokenizer === undefined && !defaultTokenizerImport) {
      defaultTokenizerImport = Promise.all([
        import("js-tiktoken/lite"),
        import("js-tiktoken/ranks/cl100k_base"),
      ])
        .then(([tokenizerModule, ranksModule]) => {
          defaultTokenizer = new tokenizerModule.Tiktoken(ranksModule.default);
        })
        .catch(() => {
          defaultTokenizer = null;
        });
    }
    if (!defaultTokenizer) return null;
    return defaultTokenizer.encode(text, "all").length;
  } catch {
    defaultTokenizer = null;
    return null;
  }
}

function estimateTextTokensFallback(text: string): number {
  if (!text) return 0;
  if (/^[\x00-\x7f]*$/.test(text)) {
    return Math.max(1, Math.ceil(text.length / 4) + (/[^\w\s'-]/.test(text) ? 2 : 1));
  }
  const chunks = text.match(/[\p{L}\p{M}\p{N}_'-]+|[^\s]/gu) ?? [];
  let total = 0;
  for (const chunk of chunks) {
    if (/^[\x00-\x7f]+$/.test(chunk) && /[\p{L}\p{N}_]/u.test(chunk)) {
      total += Math.max(1, Math.ceil(chunk.length / 4));
    } else if (/[\p{L}\p{M}\p{N}]/u.test(chunk)) {
      total += Math.max(1, Math.ceil(chunk.length / 2));
    } else {
      total += 1;
    }
  }
  const newlineTokens = Math.floor((text.match(/\n/g)?.length ?? 0) / 2);
  return total + newlineTokens;
}

export function estimateMessagesTokens(messages: readonly ChatMessage[]): number {
  return messages.reduce((total, message) => total + estimateTextTokens(messageContentForEstimate(message)), 0);
}

export function modelContextBudget(model: string, models: readonly ModelInfo[]): ModelContextBudget | null {
  if (!model.trim()) return null;
  const info = models.find((item) => item.id === model);
  const contextLength = positive(info?.context_length) ?? fallbackContextLength(model, info);
  if (!contextLength) return null;
  const reserve = positive(info?.max_completion_tokens) ?? outputReserve(contextLength);
  const promptBudget = positive(info?.max_prompt_tokens) ?? Math.max(1, contextLength - reserve);
  return { contextLength, promptBudget };
}

export function compactMessagesForModel(
  messages: readonly ChatMessage[],
  model: string,
  models: readonly ModelInfo[],
): ContextCompactionResult {
  const budget = modelContextBudget(model, models);
  const originalTokens = estimateMessagesTokens(messages);
  if (!budget) {
    return { messages: [...messages], compacted: false, originalTokens, sentTokens: originalTokens, budget };
  }

  const threshold = Math.floor(budget.promptBudget * COMPACT_AT);
  if (originalTokens <= threshold) {
    return { messages: [...messages], compacted: false, originalTokens, sentTokens: originalTokens, budget };
  }

  const setup = messages.filter((message) => message.role === "system");
  const conversation = messages.filter((message) => message.role !== "system");
  const target = Math.max(1, Math.floor(budget.promptBudget * COMPACT_TO));
  const setupTokens = estimateMessagesTokens(setup);
  const latest = conversation.slice(-1);
  const latestTokens = estimateMessagesTokens(latest);

  if (setupTokens + latestTokens > budget.promptBudget) {
    return {
      messages: [...messages],
      compacted: false,
      originalTokens,
      sentTokens: originalTokens,
      budget,
      error: `The current message is too large for ${model}'s context window.`,
    };
  }

  const summaryBudget = compactionSummaryTokenBudget(model, models);
  const suffix: ChatMessage[] = [];
  let suffixTokens = 0;
  const suffixTarget = Math.max(latestTokens, target - setupTokens - summaryBudget);
  for (let i = conversation.length - 1; i >= 0; i--) {
    const message = conversation[i];
    const nextTokens = estimateTextTokens(messageContentForEstimate(message));
    if (suffix.length > 0 && suffixTokens + nextTokens > suffixTarget) break;
    suffix.unshift(message);
    suffixTokens += nextTokens;
  }

  const older = conversation.slice(0, conversation.length - suffix.length);
  if (older.length === 0) {
    const sentTokens = setupTokens + suffixTokens;
    return { messages: [...setup, ...suffix], compacted: false, originalTokens, sentTokens, budget };
  }

  const summary = compactedSummaryMessage(older, summaryBudget);
  const compacted = [...setup, summary, ...suffix];
  const sentTokens = estimateMessagesTokens(compacted);
  if (sentTokens > budget.promptBudget) {
    return {
      messages: compacted,
      compacted: true,
      originalTokens,
      sentTokens,
      budget,
      error: `The compacted prompt is still too large for ${model}'s context window.`,
    };
  }
  return { messages: compacted, compacted: true, originalTokens, sentTokens, budget };
}

export function isCompactionCheckpoint(message: ChatMessage): boolean {
  return message.compaction?.kind === "checkpoint" && message.content.trim().length > 0;
}

export function isTranscriptControlMessage(message: ChatMessage): boolean {
  return Boolean(message.approval || message.modelChange);
}

export function latestCompactionIndex(messages: readonly ChatMessage[]): number {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (isCompactionCheckpoint(messages[i])) return i;
  }
  return -1;
}

export function checkpointMessage(
  summary: string,
  options: {
    auto: boolean;
    sourceTokens: number;
    createdAt?: number;
    baseline?: ChatCompactionMetrics;
    summaryMetrics?: ChatCompactionSummaryMetrics;
  },
): ChatMessage {
  const clean = summary.trim();
  return {
    role: "assistant",
    content: `${CHECKPOINT_HEADING}\n\n${clean}`,
    compaction: {
      kind: "checkpoint",
      createdAt: options.createdAt ?? Date.now(),
      sourceTokens: options.sourceTokens,
      summaryTokens: estimateTextTokens(clean),
      auto: options.auto || undefined,
      baseline: options.baseline,
      summary: options.summaryMetrics,
    },
  };
}

export function checkpointSummary(message: ChatMessage): string {
  return message.content
    .trim()
    .replace(/^#{1,6}\s*Context checkpoint\s*/i, "")
    .trim();
}

export function messagesForModelContext(contextMessages: readonly ChatMessage[], conversation: readonly ChatMessage[]): ChatMessage[] {
  const checkpointIndex = latestCompactionIndex(conversation);
  const modelMessage = (message: ChatMessage) =>
    !isCompactionCheckpoint(message)
    && !isTranscriptControlMessage(message)
    && !(message.role === "assistant" && !message.content.trim() && !message.attachments?.length);
  if (checkpointIndex < 0) {
    return [...contextMessages, ...conversation.filter(modelMessage)];
  }
  const checkpoint = conversation[checkpointIndex];
  return [
    ...contextMessages,
    {
      role: "system",
      content: [
        "Previous thread context checkpoint. Treat this as the durable state for earlier messages that remain visible in the UI but are not replayed below.",
        checkpointSummary(checkpoint),
      ].join("\n\n"),
    },
    ...conversation.slice(checkpointIndex + 1).filter(modelMessage),
  ];
}

export function compactionSummaryTokenBudget(model: string, models: readonly ModelInfo[]): number {
  const promptBudget = modelContextBudget(model, models)?.promptBudget ?? HOSTED_CONTEXT_WINDOW;
  return Math.min(SUMMARY_MAX_TOKENS, Math.max(SUMMARY_MIN_TOKENS, Math.floor(promptBudget * 0.1)));
}

export function compactionSummaryTargetTokens(model: string, models: readonly ModelInfo[], retry = false): number {
  const budget = compactionSummaryTokenBudget(model, models);
  const ratio = retry ? 0.55 : 0.8;
  return Math.max(SUMMARY_MIN_TOKENS, Math.floor(budget * ratio));
}

export function compactionSummaryOutputCap(model: string, models: readonly ModelInfo[], retry = false): number {
  const budget = compactionSummaryTokenBudget(model, models);
  if (!retry) return budget;
  return Math.max(budget + SUMMARY_RETRY_EXTRA_TOKENS, Math.ceil(budget * 1.75));
}

export function compactionSummaryReasoningEffort(provider?: Pick<ProviderInfo, "name" | "base_url">): ReasoningEffort | undefined {
  const name = provider?.name.toLowerCase() ?? "";
  const baseUrl = provider?.base_url.toLowerCase() ?? "";
  return name.includes("lm studio") || name.includes("lmstudio") || baseUrl.includes(":1234/")
    ? "none"
    : undefined;
}

export function validateCompactionCheckpointSummary(
  summary: string,
  options: CompactionSummaryValidationOptions,
): string | null {
  const clean = summary.trim();
  if (!clean) return "The model returned an empty compaction summary.";
  if (options.finishReason === "length") return "The compaction summary hit the model output limit.";

  const summaryTokens = estimateTextTokens(clean);
  const summaryBudget = compactionSummaryTokenBudget(options.model, options.models);
  if (summaryTokens > summaryBudget) {
    return `The compaction summary is too large: ${summaryTokens} tokens, max ${summaryBudget}.`;
  }

  const incompleteReason = incompleteSummaryReason(clean);
  if (incompleteReason) return incompleteReason;

  if (options.sourceMessages) {
    const checkpoint = checkpointMessage(clean, { auto: false, sourceTokens: 0 });
    const compactedContext = messagesForModelContext([], [...options.sourceMessages, checkpoint]);
    const budget = modelContextBudget(options.model, options.models);
    const sentTokens = estimateMessagesTokens(compactedContext);
    if (budget && sentTokens > budget.promptBudget) {
      return `The compaction checkpoint is still too large for ${options.model}'s context window.`;
    }
  }

  return null;
}

export function contextSendPlan(
  contextMessages: readonly ChatMessage[],
  conversation: readonly ChatMessage[],
  model: string,
  models: readonly ModelInfo[],
  options: ContextSendPlanOptions = {},
): ContextSendPlan {
  const budget = modelContextBudget(model, models);
  const messages = messagesForModelContext(contextMessages, conversation);
  const reservedTokens = estimateMessagesTokens(options.reservedMessages ?? []);
  const sentTokens = estimateMessagesTokens(messages) + reservedTokens;
  if (!budget) {
    return { messages, shouldCompact: false, originalTokens: sentTokens, sentTokens, budget };
  }
  const fixedTokens = estimateMessagesTokens(contextMessages) + reservedTokens;
  if (fixedTokens > budget.promptBudget) {
    const categories = options.fixedCategories?.length
      ? options.fixedCategories
      : [{ label: "fixed context", tokens: fixedTokens }];
    const breakdown = categories
      .filter((category) => category.tokens > 0)
      .map((category) => `${category.label}: ${category.tokens.toLocaleString()}`)
      .join(", ");
    return {
      messages,
      shouldCompact: false,
      originalTokens: sentTokens,
      sentTokens,
      budget,
      error: `Fixed context exceeds ${model}'s prompt budget (${fixedTokens.toLocaleString()} / ${budget.promptBudget.toLocaleString()} tokens). ${breakdown}.`,
    };
  }
  const canCompact = canCompactBeforeLatestUser(conversation);
  const shouldCompact = canCompact && sentTokens > Math.floor(budget.promptBudget * COMPACT_AT);
  const error = sentTokens > budget.promptBudget && !shouldCompact
    ? `The current message is too large for ${model}'s context window.`
    : undefined;
  return {
    messages,
    shouldCompact,
    originalTokens: sentTokens,
    sentTokens,
    budget,
    error,
  };
}

export function splitCompactionTail(
  messages: readonly ChatMessage[],
  model: string,
  models: readonly ModelInfo[],
): CompactionTailSplit {
  const start = latestCompactionIndex(messages) + 1;
  const turnStarts: number[] = [];
  for (let i = start; i < messages.length; i += 1) {
    const message = messages[i];
    if (message.role === "user" && !isCompactionCheckpoint(message) && !isTranscriptControlMessage(message)) {
      turnStarts.push(i);
    }
  }
  if (turnStarts.length <= COMPACTION_TAIL_TURNS) return { head: [...messages], tail: [] };

  const maxTailTokens = Math.min(
    COMPACTION_TAIL_MAX_TOKENS,
    Math.max(1, Math.floor((modelContextBudget(model, models)?.promptBudget ?? HOSTED_CONTEXT_WINDOW) * 0.25)),
  );
  for (const tailStart of turnStarts.slice(-COMPACTION_TAIL_TURNS)) {
    const tail = messages.slice(tailStart);
    if (estimateMessagesTokens(tail) <= maxTailTokens) {
      return { head: messages.slice(0, tailStart), tail };
    }
  }
  return { head: [...messages], tail: [] };
}

function canCompactBeforeLatestUser(conversation: readonly ChatMessage[]): boolean {
  const checkpointIndex = latestCompactionIndex(conversation);
  for (let i = conversation.length - 1; i >= 0; i -= 1) {
    if (conversation[i].role !== "user") continue;
    const start = checkpointIndex >= 0 ? checkpointIndex + 1 : 0;
    if (i <= start) return false;
    return conversation.slice(start, i).some((message) => !isCompactionCheckpoint(message));
  }
  return false;
}

export function compactionSummaryMessages(
  messages: readonly ChatMessage[],
  model: string,
  models: readonly ModelInfo[],
  options: CompactionSummaryOptions = {},
): ChatMessage[] {
  const source = messagesForModelContext([], messages);
  const sourceTokens = estimateCompactionMessagesTokens(source);
  const budget = modelContextBudget(model, models);
  const outputCapTokens = options.outputCapTokens ?? compactionSummaryOutputCap(model, models, Boolean(options.retry));
  const targetTokens = compactionSummaryTargetTokens(model, models, Boolean(options.retry));
  const maxPromptTokens = Math.max(
    SUMMARY_MIN_TOKENS,
    Math.min(sourceTokens, (budget?.promptBudget ?? HOSTED_CONTEXT_WINDOW) - outputCapTokens - 256),
  );
  const transcript = boundedTranscript(source, Math.max(SUMMARY_MIN_TOKENS, maxPromptTokens));
  return [
    {
      role: "system",
      content: [
        "Summarize this Milim thread so a fresh model session can continue without replaying earlier messages.",
        "Keep durable decisions, requirements, constraints, user preferences, active plans, open tasks, important file paths, tool results, and unresolved errors.",
        `Return a complete checkpoint under ${targetTokens} tokens.`,
        "Omit filler, greetings, and transient wording. Return only the summary.",
        "Do not use trailing ellipses or stop mid-sentence.",
        ...(options.retry ? ["The previous summary was too long or incomplete. Rewrite it shorter and keep only durable facts."] : []),
      ].join("\n"),
    },
    {
      role: "user",
      content: `Thread transcript to compact:\n\n${transcript}`,
    },
  ];
}

function estimateCompactionMessagesTokens(messages: readonly ChatMessage[]): number {
  return messages.reduce((total, message) => total + estimateTextTokens(messageContentForCompaction(message)), 0);
}

function incompleteSummaryReason(summary: string): string | null {
  const fenceCount = summary.match(/```/g)?.length ?? 0;
  if (fenceCount % 2 === 1) return "The compaction summary has an unclosed code fence.";

  const lines = summary.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  const lastLine = lines[lines.length - 1] ?? "";
  if (/^(?:[-*+]|\d+[.)])\s*$/.test(lastLine)) return "The compaction summary ends with an empty list item.";
  if (/(?:\.\.\.|\u2026)$/.test(lastLine)) return "The compaction summary appears to end with a truncation marker.";
  if (/[,:;([{]$/.test(lastLine)) return "The compaction summary appears to stop mid-thought.";
  if (/\b(?:and|or|but|because|with|for|to|the|a|an|of|in|on|at|from|as)$/i.test(lastLine)) {
    return "The compaction summary appears to stop mid-sentence.";
  }
  return null;
}

function compactedSummaryMessage(messages: readonly ChatMessage[], tokenBudget: number): ChatMessage {
  const maxChars = tokenBudget * 4;
  const lines = ["Context automatically compacted. Earlier conversation digest:"];
  let used = lines[0].length + 1;
  for (const message of messages) {
    const prefix = `${message.role}: `;
    const text = messageContentForCompaction(message).replace(/\s+/g, " ").trim();
    if (!text) continue;
    const remaining = maxChars - used - prefix.length - 1;
    if (remaining <= 24) break;
    const clipped = clipCompactionLine(text, remaining);
    const line = `${prefix}${clipped}`;
    lines.push(line);
    used += line.length + 1;
  }
  return { role: "system", content: lines.join("\n") };
}

function boundedTranscript(messages: readonly ChatMessage[], tokenBudget: number): string {
  const maxChars = tokenBudget * 4;
  const lines: string[] = [];
  let used = 0;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    const text = messageContentForCompaction(message).replace(/\s+/g, " ").trim();
    if (!text) continue;
    const role = message.role === "assistant" ? "Assistant" : message.role === "system" ? "System" : "User";
    const prefix = `${role}: `;
    const remaining = maxChars - used - prefix.length - 1;
    if (remaining <= 80) break;
    const clipped = clipCompactionLine(text, remaining);
    const line = `${prefix}${clipped}`;
    lines.unshift(line);
    used += line.length + 1;
  }
  return lines.join("\n\n");
}

function positive(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : undefined;
}

function fallbackContextLength(model: string, info?: ModelInfo): number {
  const id = model.toLowerCase();
  const owner = info?.owned_by.toLowerCase() ?? "";
  if (owner === "milim" || owner === "local") return LOCAL_CONTEXT_WINDOW;
  if (id.startsWith("claude:") || owner.includes("anthropic") || owner.includes("claude")) return 200_000;
  if (id.startsWith("codex:") || owner.includes("codex")) return 128_000;
  if (id.includes("gpt-4o") || id.includes("gpt-4.1") || id.startsWith("o1") || id.startsWith("o3")) return 128_000;
  if (id.includes("gpt-3.5")) return 16_385;
  return HOSTED_CONTEXT_WINDOW;
}

function outputReserve(contextLength: number): number {
  return Math.min(4096, Math.max(512, Math.floor(contextLength * 0.15)));
}

function messageContentForEstimate(message: ChatMessage): string {
  if (isTranscriptControlMessage(message)) return "";
  const attachmentContext = attachmentEstimateContext(message.attachments);
  if (!attachmentContext) return message.content;
  return message.content ? `${message.content}\n\n${attachmentContext}` : attachmentContext;
}

function messageContentForCompaction(message: ChatMessage): string {
  if (isTranscriptControlMessage(message)) return "";
  return [
    message.content,
    attachmentCompactionContext(message.attachments),
    runCompactionContext(message.run),
  ].filter(Boolean).join("\n\n");
}

function attachmentEstimateContext(attachments?: ChatAttachment[]): string {
  if (!attachments?.length) return "";
  return attachments
    .map((attachment) => [
      attachment.name,
      attachment.mime,
      String(attachment.size),
      attachment.content ?? "",
      attachment.dataUrl ? "[image preview]" : "",
    ].filter(Boolean).join(" "))
    .join("\n");
}

function attachmentCompactionContext(attachments?: ChatAttachment[]): string {
  if (!attachments?.length) return "";
  return attachments
    .map((attachment) => {
      const meta = [
        attachment.name,
        attachment.mime,
        String(attachment.size),
        attachment.truncated ? "truncated" : "",
        attachment.sourcePath ?? "",
      ].filter(Boolean).join(" ");
      const body = attachment.content
        ? truncateCompactionBody(attachment.content.trimEnd(), "Attachment text")
        : attachment.dataUrl ? "[Image preview omitted from compaction transcript]" : "[No text content]";
      return `Attachment ${meta}: ${body}`;
    })
    .join("\n");
}

function runCompactionContext(run: ChatMessage["run"]): string {
  if (!run?.steps.length) return "";
  return run.steps
    .map((step) => {
      const result = step.error ?? stringifyCompactionValue(step.result);
      return result
        ? `Tool ${step.name}: ${truncateCompactionBody(result, "Tool result")}`
        : `Tool ${step.name}: ${step.endedAt ? "done" : "started"}`;
    })
    .join("\n");
}

function stringifyCompactionValue(value: unknown): string {
  if (value == null) return "";
  if (typeof value === "string") return value;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function truncateCompactionBody(text: string, label: string): string {
  if (text.length <= COMPACTION_OLD_BODY_MAX_CHARS) return text;
  const omitted = text.length - COMPACTION_OLD_BODY_MAX_CHARS;
  const suffix = `\n[${label} truncated for compaction: omitted ${omitted} chars]`;
  return `${text.slice(0, Math.max(0, COMPACTION_OLD_BODY_MAX_CHARS - suffix.length)).trimEnd()}${suffix}`;
}

function clipCompactionLine(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  const marker = text.match(/\[(?:Attachment text|Tool result) truncated for compaction: omitted \d+ chars\]$/)?.[0];
  if (marker && maxChars > marker.length + 24) {
    const head = text.slice(0, Math.max(0, maxChars - marker.length - 5)).trimEnd();
    return `${head}... ${marker}`;
  }
  return `${text.slice(0, Math.max(0, maxChars - 3)).trimEnd()}...`;
}
