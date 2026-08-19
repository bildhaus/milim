import { createChatMessageId } from "./messageIds.js";
import {
  accountRuntimeImage,
  OMITTED_IMAGE_NOTE,
  selectOutboundImageAttachments,
  type AccountRuntimeImage,
} from "./attachmentInput.js";
import type {
  AccountRuntimeMilimContext,
  AgentEvent,
  AgentMemoryContext,
  AgentToolContext,
  AccountNativeWorkerLifecycle,
  ChatAttachment,
  ChatMessage,
  ChatStreamPart,
  ChildThreadInfo,
  ContextSnapshot,
  HarnessEvent,
  HarnessEventEnvelope,
  HarnessRunRequest,
  MemoryNotice,
  ModelInfo,
  ProviderLimitInfo,
  ReasoningEffort,
  ResponseMetrics,
  RunStep,
  RunTrace,
  ToolApprovalMode,
  TokenUsage,
  WorkerRunRecord,
} from "../api.js";
import {
  prepareAndStartTurn,
  type PreparedTurnOutbound,
  type PrepareTurnOutboundOptions,
} from "./turnContext.js";
import { reviewCommentsToPromptContext } from "./attachmentWire.js";
import { estimateMessagesTokens, estimateTextTokens, messagesForModelContext, modelContextBudget } from "./contextCompaction.js";
import {
  accountRuntimeToolPart,
  statusPart,
  toolApprovalPart,
  toolCompletedPart,
  toolErrorMessage,
  toolStartedPart,
} from "./turnEvents.js";
import {
  contextMessagesForTurn,
  toolDefinitionMessagesForRuntime,
  workspaceRuleMessagesForRuntime,
  type TurnPromptContext,
} from "./turnPrompt.js";

// ponytail: local copy avoids importing browser/Tauri API code into pure turn-runtime tests.
const MAX_ATTACHMENT_BYTES = 128 * 1024;

function fixedContextCategories(
  context: TurnPromptContext,
  contextMessages: ChatMessage[],
  reservedRules: ChatMessage[],
  toolDefinitions: ChatMessage[],
): Array<{ kind: string; label: string; tokens: number }> {
  const category = (kind: string, label: string, messages: ChatMessage[]) => ({
    kind,
    label,
    tokens: estimateMessagesTokens(messages),
  });
  const included = (messages: ChatMessage[]) => messages.filter((message) => contextMessages.includes(message));
  return [
    category("saved_instructions", "Saved instructions", included(context.instructionMessages)),
    category("repository_rules", "Repository rules", reservedRules),
    category("plan_goal", "Plan / Goal", included([...context.planMessages, ...context.goalMessages])),
    category("skills", "Skills", included(context.skillMessages)),
    category("memory", "Memory", included(context.memoryMessages)),
    category("artifacts_schedules", "Artifacts / schedules", included([...context.artifactMessages, ...context.scheduleMessages])),
    category("tool_definitions", "Tool definitions", toolDefinitions),
  ];
}

function contextSnapshot(
  context: TurnPromptContext,
  contextMessages: ChatMessage[],
  outbound: ChatMessage[],
  model: string,
  models: readonly ModelInfo[],
  reservedRules: ChatMessage[],
  toolDefinitions: ChatMessage[],
): ContextSnapshot {
  const fixed = fixedContextCategories(context, contextMessages, reservedRules, toolDefinitions);
  const attachmentMessages = outbound
    .filter((message) => message.attachments?.length)
    .map((message) => ({ role: message.role, content: "", attachments: message.attachments }) as ChatMessage);
  const attachmentTokens = estimateMessagesTokens(attachmentMessages);
  const fixedTokens = fixed.reduce((total, item) => total + item.tokens, 0);
  const reserved = [...reservedRules, ...toolDefinitions];
  const estimatedPromptTokens = estimateMessagesTokens(outbound) + estimateMessagesTokens(reserved);
  const conversationTokens = Math.max(0, estimatedPromptTokens - fixedTokens - attachmentTokens);
  const budget = modelContextBudget(model, models);
  const rules = context.workspaceContext?.instructions ?? [];
  return {
    model,
    limit: budget?.promptBudget ?? null,
    compactAt: budget ? Math.floor(budget.promptBudget * 0.85) : null,
    estimatedPromptTokens,
    freeTokens: budget ? Math.max(0, budget.promptBudget - estimatedPromptTokens) : null,
    categories: [
      { kind: "conversation", label: "Conversation", tokens: conversationTokens },
      ...fixed,
      { kind: "attachments", label: "Attachments", tokens: attachmentTokens },
    ],
    sources: rules.map((rule) => ({
      path: rule.path,
      family: rule.family,
      tokens: rule.status === "loaded" ? estimateTextTokens(rule.content) : 0,
      status: rule.status,
    })),
    warnings: [...(context.workspaceContext?.warnings ?? [])],
  };
}

type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;
type AccountRuntimeImageEvent = Extract<HarnessEvent, { type: "image_generated" }>;
type StreamHarnessRunFn = (
  harnessId: "codex" | "claude" | "opencode" | "pi",
  request: HarnessRunRequest,
  onEvent: (event: HarnessEventEnvelope) => void,
  signal?: AbortSignal,
) => Promise<void>;
type StreamChatFn = (
  model: string,
  messages: ChatMessage[],
  onToken: (text: string) => void,
  signal?: AbortSignal,
  onThinking?: (text: string) => void,
  onUsage?: (usage: TokenUsage) => void,
  reasoningEffort?: ReasoningEffort,
  toolContext?: AgentToolContext,
) => Promise<void>;
type StreamAgentRunFn = (
  agentId: string | null,
  model: string,
  messages: ChatMessage[],
  onEvent: (event: AgentEvent) => void,
  signal?: AbortSignal,
  memoryContext?: AgentMemoryContext,
  toolContext?: AgentToolContext,
  reasoningEffort?: ReasoningEffort,
) => Promise<void>;

export function utilityAccountRuntimeMilimContext({
  toolContext,
  toolApproval,
  planMode,
}: {
  toolContext: AgentToolContext;
  toolApproval: ToolApprovalMode;
  planMode: boolean;
}): AccountRuntimeMilimContext {
  return {
    tool_context: {
      ...toolContext,
      tool_approval_policy: toolApproval,
      tool_approval_grant: false,
      interactive_tool_approval: false,
      plan_mode: planMode,
    },
    memory_context: {},
    tool_mode: "none",
    enabled_tools: [],
    skill_mode: "none",
    enabled_skills: [],
  };
}

export const CLAUDE_SESSION_RECOVERY_REQUIRED =
  "CLAUDE_SESSION_RECOVERY_REQUIRED";

export type AccountRuntimeEventState = {
  done: boolean;
  cancelled: string | null;
  warning: string | null;
  error: string | null;
  sessionRecoveryRequired: string | null;
};

export type TurnRuntimeErrorResult = {
  status: "aborted" | "error";
  error?: string;
};

export const TURN_ABORT_SENTINEL = Symbol("turn-abort");

type TurnAbortSentinel = { [TURN_ABORT_SENTINEL]: true };

export function turnAbortSentinel(): TurnAbortSentinel {
  return { [TURN_ABORT_SENTINEL]: true };
}

export type FinalizeTurnRuntimeStatus =
  "done" | "skipped" | "aborted" | "error";

export type TurnMetricsCapture = {
  state: {
    usage?: TokenUsage;
    costUsd?: number;
    costSource?: "provider";
    limits: ProviderLimitInfo[];
  };
  captureUsage: (usage?: TokenUsage) => void;
  captureUsageDelta: (usage?: TokenUsage) => TokenUsage | undefined;
  captureRuntimeMetrics: (event: {
    usage?: TokenUsage;
    cost_usd?: number;
  }) => void;
  captureProviderLimit: (limit?: ProviderLimitInfo) => void;
};

export function createTurnMetricsCapture(): TurnMetricsCapture {
  const state: TurnMetricsCapture["state"] = { limits: [] };
  return {
    state,
    captureUsage(usage) {
      if (usage) state.usage = usage;
    },
    captureUsageDelta(usage) {
      if (!usage) return state.usage;
      state.usage = addTokenUsage(state.usage, usage);
      return state.usage;
    },
    captureRuntimeMetrics(event) {
      if (event.usage) state.usage = event.usage;
      if (typeof event.cost_usd === "number" && event.cost_usd > 0) {
        state.costUsd = event.cost_usd;
        state.costSource = "provider";
      }
    },
    captureProviderLimit(limit) {
      if (!limit) return;
      state.limits = [
        ...state.limits.filter(
          (item) =>
            item.provider !== limit.provider || item.kind !== limit.kind,
        ),
        limit,
      ];
    },
  };
}

function addTokenUsage(
  total: TokenUsage | undefined,
  usage: TokenUsage,
): TokenUsage {
  return {
    prompt_tokens: (total?.prompt_tokens ?? 0) + usage.prompt_tokens,
    completion_tokens:
      (total?.completion_tokens ?? 0) + usage.completion_tokens,
    total_tokens: (total?.total_tokens ?? 0) + usage.total_tokens,
  };
}

export type TurnRunTraceState = {
  runRef: {
    current: RunTrace | null;
  };
  snapshot: () => void;
};

export function createTurnRunTraceState(
  commitRun: (run: RunTrace) => void,
): TurnRunTraceState {
  let run: RunTrace | null = null;
  return {
    runRef: {
      get current() {
        return run;
      },
      set current(next: RunTrace | null) {
        run = next;
      },
    },
    snapshot() {
      if (run)
        commitRun({ ...run, steps: run.steps.map((step) => ({ ...step })) });
    },
  };
}

export function isGoogleWorkspaceEditTool(name: string | null | undefined): boolean {
  return /(?:^|_)google_(?:sheets|docs|slides)_edit$/.test(name ?? "");
}

export type TurnAssistantStarter = {
  state: {
    activeConversation: ChatMessage[];
    started: boolean;
    assistantMessageId: string;
  };
  beginAssistant: (conversation: ChatMessage[]) => void;
};

export function createTurnAssistantStarter({
  conversation,
  planMode,
  setMessages,
  assistantMessageId,
}: {
  conversation: ChatMessage[];
  planMode: boolean;
  setMessages: (messages: ChatMessage[]) => void;
  assistantMessageId?: string;
}): TurnAssistantStarter {
  const resolvedAssistantMessageId =
    assistantMessageId ?? createChatMessageId();
  const state = {
    activeConversation: conversation,
    started: false,
    assistantMessageId: resolvedAssistantMessageId,
  };
  return {
    state,
    beginAssistant(nextConversation) {
      state.activeConversation = nextConversation;
      if (state.started) return;
      state.started = true;
      setMessages([
        ...nextConversation,
        {
          id: resolvedAssistantMessageId,
          role: "assistant",
          content: "",
          streamParts: [],
          ...(planMode ? { plan: { status: "proposed" as const } } : {}),
        },
      ]);
    },
  };
}

export function codexPromptFromMessages(messages: ChatMessage[]): string {
  return codexPromptWithSelectedImages(
    messages,
    selectOutboundImageAttachments(messages),
  );
}

function codexPromptWithSelectedImages(
  messages: ChatMessage[],
  selectedImages: Set<ChatAttachment>,
): string {
  return messages
    .map((message) => {
      const content = wireRuntimeMessageContent(message, selectedImages).trim();
      if (!content) return "";
      const role =
        message.role === "system"
          ? "System"
          : message.role === "assistant"
            ? "Assistant"
            : "User";
      return `${role}:\n${content}`;
    })
    .filter(Boolean)
    .join("\n\n");
}

export function accountRuntimeInputFromMessages(messages: ChatMessage[]): {
  prompt: string;
  images: AccountRuntimeImage[];
} {
  const selectedImages = selectOutboundImageAttachments(messages);
  return {
    prompt: codexPromptWithSelectedImages(messages, selectedImages),
    images: messages.flatMap((message) =>
      message.role === "user"
        ? (message.attachments ?? [])
            .filter((attachment) => selectedImages.has(attachment))
            .map(accountRuntimeImage)
            .filter((image): image is AccountRuntimeImage => image !== null)
        : [],
    ),
  };
}

function wireRuntimeMessageContent(
  message: ChatMessage,
  selectedImages: Set<ChatAttachment>,
): string {
  if (message.approval) return "";
  const attachmentContext = attachmentsToPromptContext(
    message.attachments,
    selectedImages,
  );
  return [
    message.content,
    attachmentContext,
    reviewCommentsToPromptContext(message.reviewComments),
  ].filter(Boolean).join("\n\n");
}

function attachmentsToPromptContext(
  attachments: ChatMessage["attachments"],
  selectedImages: Set<ChatAttachment>,
): string {
  if (!attachments?.length) return "";
  const blocks = attachments.map((attachment) => {
    const meta = [
      `name=${attachment.name}`,
      `mime=${attachment.mime || "application/octet-stream"}`,
      `size=${attachment.size}`,
      attachment.truncated ? `truncated_at=${MAX_ATTACHMENT_BYTES}` : null,
      attachment.sourcePath ? `path=${attachment.sourcePath}` : null,
    ]
      .filter(Boolean)
      .join(" ");
    const content = attachment.content?.trimEnd();
    const imageNote = attachment.dataUrl
      ? selectedImages.has(attachment)
        ? "[Image attached as multimodal input.]"
        : OMITTED_IMAGE_NOTE
      : "";
    return [
      `--- attachment ${meta} ---`,
      [content, imageNote].filter(Boolean).join("\n") ||
        "[No text content available for this attachment.]",
      "--- end attachment ---",
    ].join("\n");
  });
  return ["[Attached files]", ...blocks, "[/Attached files]"].join("\n");
}

export function accountRuntimePromptMessages(
  contextMessages: ChatMessage[],
  convo: ChatMessage[],
  lastSyncedMessageId?: string,
): ChatMessage[] {
  if (lastSyncedMessageId) {
    const index = convo.findIndex((message) => message.id === lastSyncedMessageId);
    if (index >= 0) {
      const delta = convo.slice(index + 1).filter((message) => !message.approval);
      return [...contextMessages, ...delta];
    }
    return messagesForModelContext(contextMessages, convo);
  }
  const latestUser = convo
    .slice()
    .reverse()
    .find((message) => message.role === "user");
  return latestUser
    ? [...contextMessages, latestUser]
    : [...contextMessages, ...convo.slice(-1)];
}

export function createHarnessEventHandler({
  append,
  appendThinking,
  flush,
  appendStreamEvent,
  completeStreamEvent,
  captureRuntimeMetrics,
  captureProviderLimit,
  setNativeSessionId,
  appendImage,
  onNativeWorker,
  onToolCompleted,
  runRef,
  snapshot,
  now = () => Date.now(),
}: {
  append: (text: string) => void;
  appendThinking: (text: string) => void;
  flush: () => void;
  appendStreamEvent: (part: ChatStreamEventPart) => void;
  completeStreamEvent: (name: string, part: ChatStreamEventPart) => void;
  captureRuntimeMetrics: (metrics: {
    usage?: TokenUsage;
    cost_usd?: number;
  }) => void;
  captureProviderLimit?: (limit?: ProviderLimitInfo) => void;
  setNativeSessionId?: (sessionId: string) => void;
  appendImage?: (event: AccountRuntimeImageEvent) => void;
  onNativeWorker?: (lifecycle: AccountNativeWorkerLifecycle) => void;
  onToolCompleted?: (name: string) => void;
  runRef?: { current: RunTrace | null };
  snapshot?: () => void;
  now?: () => number;
}): {
  state: AccountRuntimeEventState;
  handle: (envelope: HarnessEventEnvelope) => void;
} {
  const state: AccountRuntimeEventState = {
    done: false,
    cancelled: null,
    warning: null,
    error: null,
    sessionRecoveryRequired: null,
  };
  const activeToolIds = new Set<string>();
  const settledToolIds = new Set<string>();
  return {
    state,
    handle(envelope) {
      const event = envelope.event;
      if (event.type === "text_delta") {
        if (event.text) append(event.text);
      } else if (event.type === "reasoning_delta") {
        if (event.text) appendThinking(event.text);
      } else if (
        event.type === "tool_started"
        || event.type === "tool_updated"
        || event.type === "tool_finished"
      ) {
        flush();
        const part = accountRuntimeToolPart(event);
        const run = runRef?.current;
        const toolId = event.id || event.name;
        if (event.type !== "tool_finished") {
          const step = run && lastOpenStep(run.steps, event.name ?? undefined, event.id);
          if (event.type === "tool_updated" || activeToolIds.has(toolId)) {
            if (step && event.detail) step.arguments = event.detail;
            completeStreamEvent(toolId, part);
          } else {
            activeToolIds.add(toolId);
            run?.steps.push({ callId: event.id ?? undefined, name: event.name ?? "tool", arguments: event.detail ?? undefined, startedAt: now() });
            appendStreamEvent(part);
          }
        } else {
          if (settledToolIds.has(toolId)) return;
          settledToolIds.add(toolId);
          activeToolIds.delete(toolId);
          const step = run && lastOpenStep(run.steps, event.name ?? undefined, event.id);
          if (step) {
            if (part.status === "error") {
              step.error =
                event.error ||
                event.detail ||
                `${event.name ?? "Tool"} failed`;
            } else if (event.result != null) {
              step.result = event.result;
            }
            step.endedAt = now();
          }
          completeStreamEvent(toolId, part);
          if (part.status === "done") onToolCompleted?.(event.name);
        }
        snapshot?.();
      } else if (event.type === "approval_requested") {
        flush();
        const step = runRef?.current && (
          lastOpenStep(runRef.current.steps, event.name, event.call_id)
          ?? lastOpenStep(runRef.current.steps, event.name)
        );
        if (step) {
          step.arguments = event.arguments;
          step.approval = {
            id: event.approval_id,
            status: "pending",
            requestedAt: now(),
          };
        }
        appendStreamEvent(toolApprovalPart(event));
        snapshot?.();
      } else if (event.type === "approval_status") {
        flush();
        const step = runRef?.current?.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval) step.approval.status = event.status;
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({ ...event, name: step?.name, arguments: step?.arguments }),
        );
        snapshot?.();
      } else if (event.type === "approval_resolved") {
        flush();
        const step = runRef?.current?.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval?.id === event.approval_id) {
          step.approval.status = event.decision === "approve" ? "approved" : "denied";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({
            ...event,
            name: step?.name,
            arguments: step?.arguments,
          }),
        );
        snapshot?.();
      } else if (event.type === "approval_failed") {
        flush();
        const step = runRef?.current?.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval) {
          step.approval.status = "failed";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({ ...event, name: step?.name, arguments: step?.arguments }),
        );
        snapshot?.();
      } else if (event.type === "session_established") {
        setNativeSessionId?.(event.native_session_id);
      } else if (event.type === "image_generated") {
        appendImage?.(event);
      } else if (event.type === "native_worker_updated") {
        onNativeWorker?.(event.lifecycle);
      } else if (event.type === "limit_updated") {
        captureProviderLimit?.(event.limit);
      } else if (event.type === "runtime_notice") {
        if (event.level === "warning" && event.code === "runtime_warning") {
          state.warning = event.message;
        } else {
          flush();
          appendStreamEvent(statusPart(event.message, event.detail ?? undefined, "warning"));
          snapshot?.();
        }
      } else if (event.type === "usage_updated") {
        captureRuntimeMetrics(event);
      } else if (event.type === "session_recovery_required") {
        state.sessionRecoveryRequired = event.message;
      } else if (event.type === "turn_completed") {
        captureRuntimeMetrics(event);
        state.done = true;
      } else if (event.type === "turn_cancelled") {
        captureRuntimeMetrics(event);
        state.cancelled = event.message ?? "Harness turn was cancelled.";
      } else if (event.type === "turn_failed") {
        captureRuntimeMetrics(event);
        if (
          event.code !== "runtime_warning"
          && event.code !== "session_recovery_required"
        ) {
          state.error = event.message;
        }
      }
    },
  };
}

export async function runModelChatTurn({
  promptContext,
  conversation,
  prepareOutbound,
  beginAssistant,
  streamChat,
  model,
  append,
  signal,
  appendThinking,
  captureUsage,
  reasoningEffort,
  models = [],
  runRef,
  snapshot,
  workspace,
}: {
  promptContext: TurnPromptContext;
  conversation: ChatMessage[];
  prepareOutbound: (
    contextMessages: ChatMessage[],
    conversation: ChatMessage[],
    options?: PrepareTurnOutboundOptions,
  ) => Promise<PreparedTurnOutbound>;
  beginAssistant: (conversation: ChatMessage[]) => void;
  streamChat: StreamChatFn;
  model: string;
  append: (text: string) => void;
  signal?: AbortSignal;
  appendThinking: (text: string) => void;
  captureUsage: (usage: TokenUsage) => void;
  reasoningEffort?: ReasoningEffort;
  models?: ModelInfo[];
  runRef?: { current: RunTrace | null };
  snapshot?: () => void;
  workspace?: string;
}): Promise<void> {
  if (runRef) {
    runRef.current = {
      model,
      startedAt: Date.now(),
      steps: [],
      status: "running",
      workspace,
    };
    snapshot?.();
  }
  const contextMessages = contextMessagesForTurn(promptContext, "model");
  const reservedRules = workspaceRuleMessagesForRuntime(promptContext, "native");
  const toolDefinitions = toolDefinitionMessagesForRuntime(promptContext, "native");
  const reservedContextMessages = [...reservedRules, ...toolDefinitions];
  const prepared = await prepareAndStartTurn({
    contextMessages,
    conversation,
    prepareOutbound,
    beginAssistant,
    prepareOptions: {
      signal,
      reservedContextMessages,
      fixedCategories: fixedContextCategories(promptContext, contextMessages, reservedRules, toolDefinitions),
    },
  });
  if (runRef?.current) {
    runRef.current.context = contextSnapshot(
      promptContext,
      contextMessages,
      prepared.outbound,
      model,
      models,
      reservedRules,
      toolDefinitions,
    );
    snapshot?.();
  }
  throwIfTurnAborted(signal);
  await streamChat(
    model,
    prepared.outbound,
    append,
    signal,
    appendThinking,
    captureUsage,
    reasoningEffort,
    promptContext.toolContext,
  );
  if (runRef?.current?.status === "running") {
    runRef.current.status = "done";
    runRef.current.endedAt = Date.now();
    snapshot?.();
  }
}

type RunAccountRuntimeTurnParams = {
  promptContext: TurnPromptContext;
  conversation: ChatMessage[];
  prepareOutbound: (
    contextMessages: ChatMessage[],
    conversation: ChatMessage[],
    options?: PrepareTurnOutboundOptions,
  ) => Promise<PreparedTurnOutbound>;
  beginAssistant: (conversation: ChatMessage[]) => void;
  checkpointWorkspace: () => Promise<void>;
  model: string;
  workspace?: string;
  reasoningEffort?: ReasoningEffort;
  toolApproval: ToolApprovalMode;
  toolApprovalGrant: boolean;
  planMode: boolean;
  lastSyncedMessageId?: string;
  allowClaudeSessionRecovery?: boolean;
  append: (text: string) => void;
  appendThinking: (text: string) => void;
  flush: () => void;
  appendStreamEvent: (part: ChatStreamEventPart) => void;
  completeStreamEvent: (name: string, part: ChatStreamEventPart) => void;
  captureRuntimeMetrics: (metrics: {
    usage?: TokenUsage;
    cost_usd?: number;
  }) => void;
  captureProviderLimit?: (limit?: ProviderLimitInfo) => void;
  onNativeWorker?: (lifecycle: AccountNativeWorkerLifecycle) => void;
  onToolCompleted?: (name: string) => void;
  signal?: AbortSignal;
  models?: ModelInfo[];
  runRef?: { current: RunTrace | null };
  snapshot?: () => void;
  stream: StreamHarnessRunFn;
} & (
  | {
      kind: "codex";
      threadId?: string;
      setThreadId: (threadId: string) => void;
      appendImage?: (event: AccountRuntimeImageEvent) => void;
    }
  | {
      kind: "claude";
      sessionId?: string;
      hadSession: boolean;
    }
  | {
      kind: "opencode";
      sessionId?: string;
      hadSession: boolean;
      setSessionId: (sessionId: string) => void;
    }
  | {
      kind: "pi";
      sessionId?: string;
      hadSession: boolean;
      setSessionId: (sessionId: string) => void;
    }
);

export async function runAccountRuntimeTurn(
  params: RunAccountRuntimeTurnParams,
): Promise<{ status: "done" | "skipped"; error?: string }> {
  const {
    promptContext,
    conversation,
    prepareOutbound,
    beginAssistant,
    checkpointWorkspace,
    model,
    workspace,
    reasoningEffort,
    toolApproval,
    toolApprovalGrant,
    planMode,
    lastSyncedMessageId,
    allowClaudeSessionRecovery,
    append,
    appendThinking,
    flush,
    appendStreamEvent,
    completeStreamEvent,
    captureRuntimeMetrics,
    captureProviderLimit,
    onNativeWorker,
    onToolCompleted,
    signal,
    models = [],
    runRef,
    snapshot,
  } = params;
  const contextMessages = contextMessagesForTurn(promptContext, "model");
  const reservedRules = workspaceRuleMessagesForRuntime(promptContext, params.kind);
  const toolDefinitions = toolDefinitionMessagesForRuntime(promptContext, params.kind);
  const reservedContextMessages = [...reservedRules, ...toolDefinitions];
  if (runRef) {
    runRef.current = {
      model,
      startedAt: Date.now(),
      steps: [],
      status: "running",
      workspace,
    };
    snapshot?.();
  }
  const hasNativeHistory =
    params.kind === "codex" ? Boolean(params.threadId) : params.hadSession;
  const prepared = await prepareAndStartTurn({
    contextMessages,
    conversation,
    prepareOutbound: (contextMessages, conversation, options) =>
      prepareOutbound(contextMessages, conversation, {
        ...options,
        skipAutoCompaction: hasNativeHistory,
      }),
    beginAssistant,
    checkpointWorkspace,
    prepareOptions: {
      signal,
      reservedContextMessages,
      fixedCategories: fixedContextCategories(promptContext, contextMessages, reservedRules, toolDefinitions),
    },
  });
  throwIfTurnAborted(signal);
  const outbound = hasNativeHistory
    ? accountRuntimePromptMessages(
        contextMessages,
        prepared.conversation,
        lastSyncedMessageId,
      )
    : messagesForModelContext(contextMessages, prepared.conversation);
  const events = createHarnessEventHandler({
    append,
    appendThinking,
    flush,
    appendStreamEvent,
    completeStreamEvent,
    captureRuntimeMetrics,
    captureProviderLimit,
    onNativeWorker,
    onToolCompleted,
    setNativeSessionId:
      params.kind === "codex"
        ? params.setThreadId
        : params.kind === "opencode" || params.kind === "pi"
          ? params.setSessionId
          : undefined,
    appendImage: params.kind === "codex" ? params.appendImage : undefined,
    runRef,
    snapshot,
  });
  const input = accountRuntimeInputFromMessages(outbound);
  const interactiveToolApproval =
    toolApproval === "review" && !planMode && !toolApprovalGrant;
  const milimContext: AccountRuntimeMilimContext = {
    tool_context: {
      ...promptContext.toolContext,
      tool_approval_policy: toolApproval,
      tool_approval_grant: toolApprovalGrant,
      interactive_tool_approval: interactiveToolApproval,
      plan_mode: planMode,
    },
    memory_context: promptContext.runMemoryContext,
    tool_mode: promptContext.toolMode ?? "all",
    enabled_tools: promptContext.enabledTools ?? [],
    skill_mode: promptContext.skillMode ?? "auto",
    enabled_skills: promptContext.enabledSkills ?? [],
  };
  if (runRef?.current) {
    runRef.current.context = contextSnapshot(promptContext, contextMessages, outbound, model, models, reservedRules, toolDefinitions);
    snapshot?.();
  }

  await params.stream(
    params.kind,
    {
      model,
      prompt: input.prompt,
      images: input.images,
      cwd: workspace,
      reasoning_effort: reasoningEffort,
      native_session_id:
        params.kind === "codex" ? params.threadId : params.sessionId,
      persist_session: params.kind === "codex" || params.kind === "pi",
      tool_approval_policy: toolApproval,
      tool_approval_grant: toolApprovalGrant,
      interactive_tool_approval: interactiveToolApproval,
      plan_mode: planMode,
      allow_session_recovery:
        params.kind === "claude" ? allowClaudeSessionRecovery : undefined,
      milim_context: milimContext,
    },
    events.handle,
    signal,
  );

  if (events.state.sessionRecoveryRequired) {
    flush();
    appendStreamEvent(
      statusPart(
        "Claude session recovery needs approval",
        events.state.sessionRecoveryRequired,
        "warning",
      ),
    );
    if (runRef?.current) {
      runRef.current.status = "stopped";
      runRef.current.endedAt = Date.now();
      snapshot?.();
    }
    return {
      status: "skipped",
      error: `${CLAUDE_SESSION_RECOVERY_REQUIRED}: ${events.state.sessionRecoveryRequired}`,
    };
  }

  throwIfTurnAborted(signal);
  if (events.state.cancelled) {
    throw new Error(events.state.cancelled);
  }
  if (events.state.error) {
    throw new Error(events.state.error);
  }
  if (events.state.done) {
    if (runRef?.current) {
      runRef.current.status = "done";
      runRef.current.endedAt = Date.now();
      snapshot?.();
    }
    return { status: "done" };
  }
  if (events.state.warning) {
    flush();
    appendStreamEvent(
      statusPart(
        params.kind === "codex"
          ? "Codex not on PATH"
          : params.kind === "claude"
            ? "Claude CLI not on PATH"
            : params.kind === "opencode"
              ? "OpenCode CLI not on PATH"
              : "Pi CLI not on PATH",
        events.state.warning,
        "warning",
      ),
    );
    if (runRef?.current) {
      runRef.current.status = "stopped";
      runRef.current.endedAt = Date.now();
      snapshot?.();
    }
    return { status: "skipped", error: events.state.warning };
  }
  const runtime =
    params.kind === "codex"
      ? "Codex"
      : params.kind === "claude"
        ? "Claude CLI"
        : params.kind === "opencode"
          ? "OpenCode"
          : "Pi CLI";
  throw new Error(`${runtime} ended without reporting completion or an error.`);
}

export async function runSelectedAccountRuntimeTurn({
  codexModel,
  claudeModel,
  opencodeModel,
  piModel,
  accountRuntime,
  setCodexThreadId,
  appendImage,
  ensureClaudeSessionId,
  setOpenCodeSessionId = () => {},
  setPiSessionId = () => {},
  streamHarnessRun,
  ...common
}: Omit<
  RunAccountRuntimeTurnParams,
  | "kind"
  | "model"
  | "threadId"
  | "stream"
  | "setThreadId"
  | "appendImage"
  | "sessionId"
  | "hadSession"
  | "setSessionId"
> & {
  codexModel?: string | null;
  claudeModel?: string | null;
  opencodeModel?: string | null;
  piModel?: string | null;
  accountRuntime?: {
    codexThreadId?: string | null;
    codexLastSyncedMessageId?: string | null;
    claudeSessionId?: string | null;
    claudeLastSyncedMessageId?: string | null;
    opencodeSessionId?: string | null;
    opencodeLastSyncedMessageId?: string | null;
    piSessionId?: string | null;
    piLastSyncedMessageId?: string | null;
  } | null;
  setCodexThreadId: (threadId: string) => void;
  appendImage?: (event: AccountRuntimeImageEvent) => void;
  ensureClaudeSessionId: () => string;
  setOpenCodeSessionId?: (sessionId: string) => void;
  setPiSessionId?: (sessionId: string) => void;
  streamHarnessRun: StreamHarnessRunFn;
}): Promise<null | { status: "done" | "skipped"; error?: string }> {
  if (codexModel) {
    return runAccountRuntimeTurn({
      ...common,
      kind: "codex",
      model: codexModel,
      threadId: accountRuntime?.codexThreadId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.codexLastSyncedMessageId ?? undefined,
      stream: streamHarnessRun,
      setThreadId: setCodexThreadId,
      appendImage,
    });
  }
  if (claudeModel) {
    return runAccountRuntimeTurn({
      ...common,
      kind: "claude",
      model: claudeModel,
      hadSession: Boolean(accountRuntime?.claudeSessionId),
      sessionId: ensureClaudeSessionId(),
      lastSyncedMessageId:
        accountRuntime?.claudeLastSyncedMessageId ?? undefined,
      stream: streamHarnessRun,
    });
  }
  if (opencodeModel) {
    return runAccountRuntimeTurn({
      ...common,
      kind: "opencode",
      model: opencodeModel,
      hadSession: Boolean(accountRuntime?.opencodeSessionId),
      sessionId: accountRuntime?.opencodeSessionId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.opencodeLastSyncedMessageId ?? undefined,
      stream: streamHarnessRun,
      setSessionId: setOpenCodeSessionId,
    });
  }
  if (piModel) {
    return runAccountRuntimeTurn({
      ...common,
      kind: "pi",
      model: piModel,
      hadSession: Boolean(accountRuntime?.piSessionId),
      sessionId: accountRuntime?.piSessionId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.piLastSyncedMessageId ?? undefined,
      stream: streamHarnessRun,
      setSessionId: setPiSessionId,
    });
  }
  return null;
}

export async function runToolAgentTurn({
  promptContext,
  conversation,
  prepareOutbound,
  beginAssistant,
  checkpointWorkspace,
  streamAgentRun,
  agentId,
  model,
  onEvent,
  signal,
  runMemoryContext,
  toolContext,
  reasoningEffort,
  runRef,
  snapshot,
  workspace,
  sourceSessionId,
  models = [],
  now = () => Date.now(),
}: {
  promptContext: TurnPromptContext;
  conversation: ChatMessage[];
  prepareOutbound: (
    contextMessages: ChatMessage[],
    conversation: ChatMessage[],
    options?: PrepareTurnOutboundOptions,
  ) => Promise<PreparedTurnOutbound>;
  beginAssistant: (conversation: ChatMessage[]) => void;
  checkpointWorkspace: () => Promise<void>;
  streamAgentRun: StreamAgentRunFn;
  agentId: string | null;
  model: string;
  onEvent: (event: AgentEvent) => void;
  signal?: AbortSignal;
  runMemoryContext: AgentMemoryContext;
  toolContext: AgentToolContext;
  reasoningEffort?: ReasoningEffort;
  runRef: { current: RunTrace | null };
  snapshot: () => void;
  workspace?: string;
  sourceSessionId: string;
  models?: ModelInfo[];
  now?: () => number;
}): Promise<{ status: "done" | "error"; error?: string }> {
  runRef.current = {
    model,
    startedAt: now(),
    steps: [],
    status: "running",
    workspace,
    sourceSessionId,
  };
  snapshot();
  const contextMessages = contextMessagesForTurn(
    promptContext,
    agentId ? "agent" : "tools",
  );
  const reservedRules = workspaceRuleMessagesForRuntime(promptContext, "native");
  const toolDefinitions = toolDefinitionMessagesForRuntime(promptContext, "native");
  const reservedContextMessages = [...reservedRules, ...toolDefinitions];
  const prepared = await prepareAndStartTurn({
    contextMessages,
    conversation,
    prepareOutbound,
    beginAssistant,
    checkpointWorkspace,
    afterStart: snapshot,
    prepareOptions: {
      signal,
      reservedContextMessages,
      fixedCategories: fixedContextCategories(promptContext, contextMessages, reservedRules, toolDefinitions),
    },
  });
  if (runRef.current) {
    runRef.current.context = contextSnapshot(promptContext, contextMessages, prepared.outbound, model, models, reservedRules, toolDefinitions);
    snapshot();
  }
  throwIfTurnAborted(signal);
  await streamAgentRun(
    agentId,
    model,
    prepared.outbound,
    onEvent,
    signal,
    runMemoryContext,
    toolContext,
    reasoningEffort,
  );
  const run = runRef.current;
  if (!run) return { status: "done" };
  if (run.status === "running") {
    run.status = "done";
    run.endedAt = now();
    snapshot();
    return { status: "done" };
  }
  if (run.status === "error")
    return { status: "error", error: run.error || "Agent run failed." };
  return { status: "done" };
}

export function handleTurnRuntimeError({
  error,
  assistantStarted,
  append,
  flush,
  setChatNotice,
  appendStreamEvent,
  runRef,
  snapshot,
  signal,
  now = () => Date.now(),
}: {
  error: unknown;
  assistantStarted: boolean;
  append: (text: string) => void;
  flush: () => void;
  setChatNotice: (notice: { tone: "error"; message: string }) => void;
  appendStreamEvent: (part: ChatStreamEventPart) => void;
  runRef: { current: RunTrace | null };
  snapshot: () => void;
  signal?: AbortSignal;
  now?: () => number;
}): TurnRuntimeErrorResult {
  flush();
  const aborted = isAbortError(error) || Boolean(signal?.aborted);
  const message = String(error);
  if (!aborted) {
    setChatNotice({ tone: "error", message });
    if (assistantStarted) {
      append(`\nError: ${message}`);
      flush();
      appendStreamEvent(statusPart("Error", message, "error"));
    }
  }
  const run = runRef.current;
  if (run && run.status === "running") {
    run.status = aborted ? "aborted" : "error";
    if (!aborted) run.error = message;
    run.endedAt = now();
    snapshot();
  }
  return {
    status: aborted ? "aborted" : "error",
    error: aborted ? undefined : message,
  };
}

export function finalizeTurnRuntime({
  sessionId,
  model,
  status,
  flush,
  metrics,
  commitResponseMetrics,
  finalizeMessageArtifacts,
  clearController,
  setSessionGenerating,
  setSessionUnread,
  activeSessionId,
  stopChildThreadEventsIfIdle,
  maybeGenerateAiThreadTitle,
  flushUserState,
  signal,
}: {
  sessionId: string;
  model: string;
  status: FinalizeTurnRuntimeStatus;
  flush: () => void;
  metrics?: ResponseMetrics;
  commitResponseMetrics: (sessionId: string, metrics: ResponseMetrics) => void;
  finalizeMessageArtifacts?: (sessionId: string) => void;
  clearController: (sessionId: string) => void;
  setSessionGenerating: (sessionId: string, generating: boolean) => void;
  setSessionUnread: (sessionId: string, unread: boolean) => void;
  activeSessionId: string;
  stopChildThreadEventsIfIdle: (sessionId: string) => void;
  maybeGenerateAiThreadTitle: (
    sessionId: string,
    model: string,
  ) => Promise<void>;
  flushUserState?: () => void | Promise<void>;
  signal?: AbortSignal;
}): void {
  const finalStatus: FinalizeTurnRuntimeStatus =
    status === "error" && signal?.aborted ? "aborted" : status;
  flush();
  finalizeMessageArtifacts?.(sessionId);
  if (metrics) commitResponseMetrics(sessionId, metrics);
  clearController(sessionId);
  setSessionGenerating(sessionId, false);
  setSessionUnread(sessionId, activeSessionId !== sessionId);
  stopChildThreadEventsIfIdle(sessionId);
  void Promise.resolve(flushUserState?.()).catch(() => {});
  if (finalStatus === "done")
    void maybeGenerateAiThreadTitle(sessionId, model).catch(() => {});
}

export function createAgentRunEventHandler({
  runRef,
  append,
  appendThinking,
  flush,
  appendStreamEvent,
  completeStreamEvent,
  appendMemoryNotice,
  upsertChildThread,
  updateChildThread,
  upsertWorkerRun,
  onToolCompleted,
  captureUsage,
  captureUsageDelta,
  snapshot,
  now = () => Date.now(),
}: {
  runRef: { current: RunTrace | null };
  append: (text: string) => void;
  appendThinking: (text: string) => void;
  flush: () => void;
  appendStreamEvent: (part: ChatStreamEventPart) => void;
  completeStreamEvent: (
    name: string,
    part: ChatStreamEventPart,
    callId?: string,
  ) => void;
  appendMemoryNotice: (notice: MemoryNotice) => void;
  upsertChildThread: (thread: ChildThreadInfo) => void;
  updateChildThread: (thread: ChildThreadInfo) => void;
  upsertWorkerRun?: (record: WorkerRunRecord) => void;
  onToolCompleted?: (name: string) => void;
  captureUsage: (usage?: TokenUsage) => void;
  captureUsageDelta: (usage?: TokenUsage) => void;
  snapshot: () => void;
  now?: () => number;
}): (event: AgentEvent) => void {
  return (event) => {
    const run = runRef.current;
    if (!run) return;
    switch (event.type) {
      case "start":
        if (event.model) run.model = event.model;
        break;
      case "token":
        if (event.text) append(event.text);
        return;
      case "reasoning":
        if (event.text) appendThinking(event.text);
        return;
      case "usage_delta":
        captureUsageDelta(event.usage);
        break;
      case "tool_call":
        flush();
        run.steps.push({
          callId: event.call_id,
          name: event.name ?? "tool",
          arguments: event.arguments,
          mcpApp: event.mcp_app,
          startedAt: now(),
        });
        appendStreamEvent(toolStartedPart(event));
        break;
      case "tool_result": {
        flush();
        const step = lastOpenStep(run.steps, event.name, event.call_id);
        const error = toolErrorMessage(event.result);
        if (step) {
          if (error) step.error = error;
          else step.result = event.result;
          step.mcpApp = event.mcp_app ?? step.mcpApp;
          step.mcpAppResult = event.mcp_app_result;
          step.endedAt = now();
        }
        completeStreamEvent(
          event.name ?? "tool",
          toolCompletedPart({ ...event, arguments: step?.arguments }),
          event.call_id,
        );
        if (!error) onToolCompleted?.(step?.name ?? event.name ?? "tool");
        break;
      }
      case "tool_approval_required": {
        const step = lastOpenStep(run.steps, event.name, event.call_id);
        if (step && event.approval_id) {
          step.approval = {
            id: event.approval_id,
            status: "pending",
            requestedAt: now(),
          };
        }
        appendStreamEvent(toolApprovalPart(event));
        break;
      }
      case "tool_approval_status": {
        if (!event.status) break;
        const step = run.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval) step.approval.status = event.status;
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({ ...event, name: step?.name, arguments: step?.arguments }),
        );
        break;
      }
      case "tool_approval_resolved": {
        const step = lastOpenStep(run.steps, undefined, event.call_id);
        if (step?.approval && event.approval_id === step.approval.id) {
          step.approval.status = event.decision === "approve" ? "approved" : "denied";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({
            ...event,
            name: step?.name,
            arguments: step?.arguments,
          }),
        );
        break;
      }
      case "tool_approval_failed": {
        const step = run.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval) {
          step.approval.status = "failed";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart({ ...event, name: step?.name, arguments: step?.arguments }),
        );
        break;
      }
      case "memory_registered": {
        flush();
        const notice = normalizeMemoryNotice(event);
        if (notice) appendMemoryNotice(notice);
        break;
      }
      case "child_thread_started":
        flush();
        if (event.thread) upsertChildThread(event.thread);
        appendStreamEvent(
          statusPart("Worker started", childThreadDetail(event.thread)),
        );
        break;
      case "child_thread_done":
        flush();
        if (event.thread) updateChildThread(event.thread);
        appendStreamEvent(
          statusPart("Worker done", childThreadDetail(event.thread)),
        );
        break;
      case "child_thread_error":
        flush();
        if (event.thread) updateChildThread(event.thread);
        appendStreamEvent(
          statusPart(
            "Worker error",
            event.message ?? childThreadDetail(event.thread),
            "error",
          ),
        );
        break;
      case "child_thread_stopped":
        flush();
        if (event.thread) updateChildThread(event.thread);
        appendStreamEvent(
          statusPart(
            "Worker stopped",
            event.message ?? childThreadDetail(event.thread),
          ),
        );
        break;
      case "worker_run_proposed":
      case "worker_run_started":
      case "worker_run_done":
      case "worker_run_error":
        flush();
        if (event.run)
          upsertWorkerRun?.({ run: event.run, workers: event.workers ?? [] });
        appendStreamEvent(
          statusPart(
            event.type === "worker_run_proposed"
              ? "Worker plan ready"
              : event.type === "worker_run_started"
                ? "Workers started"
                : event.type === "worker_run_done"
                  ? "Workers done"
                  : "Worker run error",
            event.run
              ? `${event.run.tasks.length} task${event.run.tasks.length === 1 ? "" : "s"}`
              : event.message,
            event.type === "worker_run_error" ? "error" : "status",
          ),
        );
        break;
      case "error":
        flush();
        run.status = "error";
        run.error = event.message;
        appendStreamEvent(statusPart("Error", event.message, "error"));
        break;
      case "done":
        flush();
        captureUsage(event.usage);
        run.iterations = event.iterations;
        run.endedAt = now();
        if (run.status !== "error")
          run.status = event.stopped_at_limit ? "stopped" : "done";
        if (event.stopped_at_limit)
          appendStreamEvent(statusPart("Stopped before final answer"));
        break;
      // "final": answer text already arrived via token events.
    }
    snapshot();
  };
}

function throwIfTurnAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw turnAbortSentinel();
}

function isAbortError(error: unknown): boolean {
  return (
    isTurnAbortSentinel(error) ||
    (typeof DOMException !== "undefined" &&
      error instanceof DOMException &&
      error.name === "AbortError")
  );
}

function isTurnAbortSentinel(error: unknown): error is TurnAbortSentinel {
  return Boolean(
    error && typeof error === "object" && TURN_ABORT_SENTINEL in error,
  );
}

function normalizeMemoryNotice(event: AgentEvent): MemoryNotice | null {
  if (
    event.type !== "memory_registered" ||
    !event.id ||
    !event.node_id ||
    !event.scope_kind ||
    !event.scope_label ||
    !event.summary ||
    !event.created_at
  ) {
    return null;
  }
  return {
    id: event.id,
    node_id: event.node_id,
    scope_kind: event.scope_kind,
    scope_label: event.scope_label,
    summary: event.summary,
    created_at: event.created_at,
  };
}

function childThreadDetail(thread?: ChildThreadInfo): string | undefined {
  if (!thread) return undefined;
  const summary = thread.summary?.trim() || thread.error?.trim();
  return summary ? `${thread.title}: ${summary.slice(0, 120)}` : thread.title;
}

function lastOpenStep(
  steps: RunStep[],
  name?: string,
  callId?: string,
): RunStep | undefined {
  if (callId) {
    for (let i = steps.length - 1; i >= 0; i -= 1) {
      if (steps[i].endedAt == null && steps[i].callId === callId)
        return steps[i];
    }
  }
  for (let i = steps.length - 1; i >= 0; i -= 1) {
    if (steps[i].endedAt == null && (name == null || steps[i].name === name))
      return steps[i];
  }
  return undefined;
}
