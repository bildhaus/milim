import { createChatMessageId } from "./messageIds.js";
import {
  accountRuntimeImage,
  OMITTED_IMAGE_NOTE,
  selectOutboundImageAttachments,
  type AccountRuntimeImage,
} from "./attachmentInput.js";
import type {
  AgentEvent,
  AgentMemoryContext,
  AgentToolContext,
  AccountNativeWorkerLifecycle,
  ChatAttachment,
  ChatMessage,
  ChatStreamPart,
  ChildThreadInfo,
  ClaudeRunEvent,
  CodexRunEvent,
  OpenCodeRunEvent,
  PiRunEvent,
  ContextSnapshot,
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
type AccountRuntimeEvent =
  | CodexRunEvent
  | ClaudeRunEvent
  | OpenCodeRunEvent
  | PiRunEvent;
type AccountRuntimeImageEvent = Extract<CodexRunEvent, { type: "image" }>;
type StreamCodexRunFn = (
  request: {
    model: string;
    prompt: string;
    cwd?: string;
    reasoning_effort?: ReasoningEffort;
    thread_id?: string;
    persist_thread?: boolean;
    tool_approval_policy?: ToolApprovalMode;
    tool_approval_grant?: boolean;
    interactive_tool_approval?: boolean;
    plan_mode?: boolean;
    images?: AccountRuntimeImage[];
  },
  onEvent: (event: CodexRunEvent) => void,
  signal?: AbortSignal,
) => Promise<void>;
type StreamClaudeRunFn = (
  request: {
    model: string;
    prompt: string;
    cwd?: string;
    reasoning_effort?: ReasoningEffort;
    session_id?: string;
    tool_approval_policy?: ToolApprovalMode;
    tool_approval_grant?: boolean;
    interactive_tool_approval?: boolean;
    plan_mode?: boolean;
    allow_session_recovery?: boolean;
    images?: AccountRuntimeImage[];
  },
  onEvent: (event: ClaudeRunEvent) => void,
  signal?: AbortSignal,
) => Promise<void>;
type StreamOpenCodeRunFn = (
  request: {
    model: string;
    prompt: string;
    cwd?: string;
    session_id?: string;
    tool_approval_policy?: ToolApprovalMode;
    tool_approval_grant?: boolean;
    interactive_tool_approval?: boolean;
    plan_mode?: boolean;
    images?: AccountRuntimeImage[];
  },
  onEvent: (event: OpenCodeRunEvent) => void,
  signal?: AbortSignal,
) => Promise<void>;
type StreamPiRunFn = (
  request: {
    model: string;
    prompt: string;
    cwd?: string;
    reasoning_effort?: ReasoningEffort;
    session_id?: string;
    persist_session?: boolean;
    tool_approval_policy?: ToolApprovalMode;
    tool_approval_grant?: boolean;
    interactive_tool_approval?: boolean;
    plan_mode?: boolean;
    images?: AccountRuntimeImage[];
  },
  onEvent: (event: PiRunEvent) => void,
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

export type CodexRunRequest = Parameters<StreamCodexRunFn>[0];
export type ClaudeRunRequest = Parameters<StreamClaudeRunFn>[0];

export function codexCompactionSummaryRequest({
  model,
  prompt,
  cwd,
  reasoningEffort,
  images,
}: {
  model: string;
  prompt: string;
  cwd?: string;
  reasoningEffort?: ReasoningEffort;
  images?: AccountRuntimeImage[];
}): CodexRunRequest {
  return {
    model,
    prompt,
    cwd,
    reasoning_effort: reasoningEffort,
    images,
    persist_thread: false,
    tool_approval_policy: "guarded",
    tool_approval_grant: false,
    plan_mode: true,
  };
}

export function claudeCompactionSummaryRequest({
  model,
  prompt,
  cwd,
  reasoningEffort,
  images,
}: {
  model: string;
  prompt: string;
  cwd?: string;
  reasoningEffort?: ReasoningEffort;
  images?: AccountRuntimeImage[];
}): ClaudeRunRequest {
  return {
    model,
    prompt,
    cwd,
    reasoning_effort: reasoningEffort,
    images,
    tool_approval_policy: "guarded",
    tool_approval_grant: false,
    plan_mode: true,
  };
}

export const CLAUDE_SESSION_RECOVERY_REQUIRED =
  "CLAUDE_SESSION_RECOVERY_REQUIRED";

export type AccountRuntimeEventState = {
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
      if (typeof event.cost_usd === "number" && event.cost_usd > 0)
        state.costUsd = event.cost_usd;
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
  if (!attachmentContext) return message.content;
  return message.content
    ? `${message.content}\n\n${attachmentContext}`
    : attachmentContext;
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

export function createAccountRuntimeEventHandler({
  append,
  appendThinking,
  flush,
  appendStreamEvent,
  completeStreamEvent,
  captureRuntimeMetrics,
  captureProviderLimit,
  setCodexThreadId,
  setOpenCodeSessionId = () => {},
  setPiSessionId = () => {},
  appendImage,
  onNativeWorker,
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
  setCodexThreadId?: (threadId: string) => void;
  setOpenCodeSessionId?: (sessionId: string) => void;
  setPiSessionId?: (sessionId: string) => void;
  appendImage?: (event: AccountRuntimeImageEvent) => void;
  onNativeWorker?: (lifecycle: AccountNativeWorkerLifecycle) => void;
  runRef?: { current: RunTrace | null };
  snapshot?: () => void;
  now?: () => number;
}): {
  state: AccountRuntimeEventState;
  handle: (event: AccountRuntimeEvent) => void;
} {
  const state: AccountRuntimeEventState = {
    warning: null,
    error: null,
    sessionRecoveryRequired: null,
  };
  return {
    state,
    handle(event) {
      if (event.type === "token") {
        if (event.text) append(event.text);
      } else if (event.type === "reasoning") {
        if (event.text) appendThinking(event.text);
      } else if (event.type === "tool") {
        flush();
        const part = accountRuntimeToolPart(event);
        const run = runRef?.current;
        if (event.status === "running") {
          run?.steps.push({ callId: event.id ?? undefined, name: event.name ?? "tool", arguments: event.detail ?? undefined, startedAt: now() });
          appendStreamEvent(part);
        } else {
          const step = run && lastOpenStep(run.steps, event.name ?? undefined, event.id);
          if (step) step.endedAt = now();
          completeStreamEvent(event.id || event.name, part);
        }
        snapshot?.();
      } else if (event.type === "tool_approval_required") {
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
      } else if (event.type === "tool_approval_resolved") {
        flush();
        const step = runRef?.current?.steps
          .slice()
          .reverse()
          .find((candidate) => candidate.approval?.id === event.approval_id);
        if (step?.approval?.id === event.approval_id) {
          step.approval.status = event.decision === "approve" ? "approved" : "denied";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(`approval:${event.approval_id}`, toolApprovalPart(event));
        snapshot?.();
      } else if (event.type === "thread") {
        setCodexThreadId?.(event.thread_id);
      } else if (event.type === "session") {
        setOpenCodeSessionId?.(event.session_id);
        setPiSessionId?.(event.session_id);
      } else if (event.type === "image") {
        appendImage?.(event);
      } else if (event.type === "native_worker") {
        onNativeWorker?.(event.lifecycle);
      } else if (event.type === "rate_limit") {
        captureProviderLimit?.(event.limit);
      } else if (event.type === "protocol_notice") {
        flush();
        appendStreamEvent(statusPart(event.message, event.detail ?? undefined, "warning"));
        snapshot?.();
      } else if (event.type === "done") {
        captureRuntimeMetrics(event);
      } else if (event.type === "warning") {
        state.warning = event.message;
      } else if (event.type === "session_recovery_required") {
        state.sessionRecoveryRequired = event.message;
      } else if (event.type === "error") {
        captureRuntimeMetrics(event);
        state.error = event.message;
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
  signal?: AbortSignal;
  models?: ModelInfo[];
  runRef?: { current: RunTrace | null };
  snapshot?: () => void;
} & (
  | {
      kind: "codex";
      threadId?: string;
      stream: StreamCodexRunFn;
      setThreadId: (threadId: string) => void;
      appendImage?: (event: AccountRuntimeImageEvent) => void;
    }
  | {
      kind: "claude";
      sessionId?: string;
      hadSession: boolean;
      stream: StreamClaudeRunFn;
    }
  | {
      kind: "opencode";
      sessionId?: string;
      hadSession: boolean;
      stream: StreamOpenCodeRunFn;
      setSessionId: (sessionId: string) => void;
    }
  | {
      kind: "pi";
      sessionId?: string;
      hadSession: boolean;
      stream: StreamPiRunFn;
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
  const events = createAccountRuntimeEventHandler({
    append,
    appendThinking,
    flush,
    appendStreamEvent,
    completeStreamEvent,
    captureRuntimeMetrics,
    captureProviderLimit,
    onNativeWorker,
    setCodexThreadId: params.kind === "codex" ? params.setThreadId : undefined,
    setOpenCodeSessionId: params.kind === "opencode" ? params.setSessionId : undefined,
    setPiSessionId: params.kind === "pi" ? params.setSessionId : undefined,
    appendImage: params.kind === "codex" ? params.appendImage : undefined,
  });
  const input = accountRuntimeInputFromMessages(outbound);
  if (runRef?.current) {
    runRef.current.context = contextSnapshot(promptContext, contextMessages, outbound, model, models, reservedRules, toolDefinitions);
    snapshot?.();
  }

  if (params.kind === "codex") {
    await params.stream(
      {
        model,
        prompt: input.prompt,
        images: input.images,
        cwd: workspace,
        reasoning_effort: reasoningEffort,
        thread_id: params.threadId,
        persist_thread: true,
        tool_approval_policy: toolApproval,
        tool_approval_grant: toolApprovalGrant,
        interactive_tool_approval: toolApproval === "review" && !planMode && !toolApprovalGrant,
        plan_mode: planMode,
      },
      events.handle,
      signal,
    );
  } else if (params.kind === "claude") {
    await params.stream(
      {
        model,
        prompt: input.prompt,
        images: input.images,
        cwd: workspace,
        reasoning_effort: reasoningEffort,
        session_id: params.sessionId,
        tool_approval_policy: toolApproval,
        tool_approval_grant: toolApprovalGrant,
        interactive_tool_approval: toolApproval === "review" && !planMode && !toolApprovalGrant,
        plan_mode: planMode,
        allow_session_recovery: allowClaudeSessionRecovery,
      },
      events.handle,
      signal,
    );
  } else if (params.kind === "opencode") {
    await params.stream(
      {
        model,
        prompt: input.prompt,
        images: input.images,
        cwd: workspace,
        session_id: params.sessionId,
        tool_approval_policy: toolApproval,
        tool_approval_grant: toolApprovalGrant,
        interactive_tool_approval: toolApproval === "review" && !planMode && !toolApprovalGrant,
        plan_mode: planMode,
      },
      events.handle,
      signal,
    );
  } else {
    await params.stream(
      {
        model,
        prompt: input.prompt,
        images: input.images,
        cwd: workspace,
        reasoning_effort: reasoningEffort,
        session_id: params.sessionId,
        persist_session: true,
        tool_approval_policy: toolApproval,
        tool_approval_grant: toolApprovalGrant,
        interactive_tool_approval:
          toolApproval === "review" && !planMode && !toolApprovalGrant,
        plan_mode: planMode,
      },
      events.handle,
      signal,
    );
  }

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
  if (events.state.error) {
    if (signal?.aborted) throw turnAbortSentinel();
    throw new Error(events.state.error);
  }
  if (runRef?.current) {
    runRef.current.status = "done";
    runRef.current.endedAt = Date.now();
    snapshot?.();
  }
  return { status: "done" };
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
  streamCodexRun,
  streamClaudeRun,
  streamOpenCodeRun,
  streamPiRun,
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
  streamCodexRun: StreamCodexRunFn;
  streamClaudeRun: StreamClaudeRunFn;
  streamOpenCodeRun?: StreamOpenCodeRunFn;
  streamPiRun?: StreamPiRunFn;
}): Promise<null | { status: "done" | "skipped"; error?: string }> {
  if (codexModel) {
    return runAccountRuntimeTurn({
      ...common,
      kind: "codex",
      model: codexModel,
      threadId: accountRuntime?.codexThreadId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.codexLastSyncedMessageId ?? undefined,
      stream: streamCodexRun,
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
      stream: streamClaudeRun,
    });
  }
  if (opencodeModel) {
    if (!streamOpenCodeRun)
      throw new Error("OpenCode runtime bridge is unavailable.");
    return runAccountRuntimeTurn({
      ...common,
      kind: "opencode",
      model: opencodeModel,
      hadSession: Boolean(accountRuntime?.opencodeSessionId),
      sessionId: accountRuntime?.opencodeSessionId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.opencodeLastSyncedMessageId ?? undefined,
      stream: streamOpenCodeRun,
      setSessionId: setOpenCodeSessionId,
    });
  }
  if (piModel) {
    if (!streamPiRun) throw new Error("Pi runtime bridge is unavailable.");
    return runAccountRuntimeTurn({
      ...common,
      kind: "pi",
      model: piModel,
      hadSession: Boolean(accountRuntime?.piSessionId),
      sessionId: accountRuntime?.piSessionId ?? undefined,
      lastSyncedMessageId:
        accountRuntime?.piLastSyncedMessageId ?? undefined,
      stream: streamPiRun,
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
      case "tool_approval_resolved": {
        const step = lastOpenStep(run.steps, undefined, event.call_id);
        if (step?.approval && event.approval_id === step.approval.id) {
          step.approval.status = event.decision === "approve" ? "approved" : "denied";
          step.approval.resolvedAt = now();
        }
        completeStreamEvent(
          `approval:${event.approval_id}`,
          toolApprovalPart(event),
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
