import type {
  ChatMessage,
  ChatStreamPart,
  McpApprovalField,
  ToolApprovalMode,
  ToolApprovalRequest,
} from "../api";

type ApprovalPart = Extract<ChatStreamPart, { kind: "event" }>;

export function pendingToolApprovals(messages: readonly ChatMessage[]): ApprovalPart[] {
  const approvals = new Map<string, ApprovalPart>();
  for (const message of messages) {
    for (const part of message.streamParts ?? []) {
      if (part.kind === "event" && part.approvalId) approvals.set(part.approvalId, part);
    }
  }
  return [...approvals.values()].filter((part) => part.approvalStatus === "pending");
}

function isAutoApprovableToolApproval(part: ApprovalPart): boolean {
  return (
    part.approvalRequest == null ||
    part.approvalRequest.kind === "command" ||
    part.approvalRequest.kind === "file_change" ||
    part.approvalRequest.kind === "permissions"
  );
}

export function autoApprovableToolApprovals(approvals: readonly ApprovalPart[]): ApprovalPart[] {
  return approvals.filter(isAutoApprovableToolApproval);
}

export function toolApprovalPrompts(
  approvals: readonly ApprovalPart[],
  mode: ToolApprovalMode,
): ApprovalPart[] {
  return mode === "open"
    ? approvals.filter((part) => !isAutoApprovableToolApproval(part))
    : [...approvals];
}

export function dismissToolApproval(
  messages: readonly ChatMessage[],
  approvalId: string,
  resolvedAt = Date.now(),
): ChatMessage[] {
  return messages.map((message) => {
    const hasPart = message.streamParts?.some(
      (part) => part.kind === "event" &&
        part.approvalId === approvalId &&
        part.approvalStatus === "pending",
    );
    const hasStep = message.run?.steps.some(
      (step) => step.approval?.id === approvalId && step.approval.status === "pending",
    );
    if (!hasPart && !hasStep) return message;
    return {
      ...message,
      streamParts: message.streamParts?.map((part) =>
        part.kind === "event" &&
        part.approvalId === approvalId &&
        part.approvalStatus === "pending"
          ? { ...part, label: "Tool approval dismissed", status: "done", approvalStatus: "canceled" }
          : part
      ),
      run: message.run ? {
        ...message.run,
        steps: message.run.steps.map((step) =>
          step.approval?.id === approvalId && step.approval.status === "pending"
            ? { ...step, approval: { ...step.approval, status: "canceled", resolvedAt } }
            : step
        ),
      } : undefined,
    };
  });
}

export function initialApprovalValues(request?: ToolApprovalRequest): Record<string, unknown> {
  if (request?.kind !== "mcp_form") return {};
  return Object.fromEntries(
    request.fields
      .filter((field) => field.default !== undefined || field.kind === "boolean")
      .map((field) => [field.name, field.default ?? false]),
  );
}

export function approvalResponse(
  request: ToolApprovalRequest | undefined,
  values: Record<string, unknown>,
): { response?: Record<string, unknown>; error?: string } {
  if (request?.kind !== "mcp_form") return {};
  const response: Record<string, unknown> = {};
  for (const field of request.fields) {
    const value = values[field.name];
    const blank = value === undefined || value === null || value === "";
    if (field.kind === "string") {
      if (!field.required && blank) continue;
      const text = typeof value === "string" ? value : "";
      if (field.min_length != null && [...text].length < field.min_length)
        return { error: `${field.label} is too short.` };
      if (field.max_length != null && [...text].length > field.max_length)
        return { error: `${field.label} is too long.` };
      response[field.name] = text;
      continue;
    }
    if (blank) {
      if (field.required) return { error: `${field.label} is required.` };
      continue;
    }
    if (field.kind === "number" || field.kind === "integer") {
      const number = typeof value === "number" ? value : Number(value);
      if (!Number.isFinite(number) || (field.kind === "integer" && !Number.isInteger(number)))
        return { error: `${field.label} must be a valid ${field.kind}.` };
      if (field.minimum != null && number < field.minimum)
        return { error: `${field.label} must be at least ${field.minimum}.` };
      if (field.maximum != null && number > field.maximum)
        return { error: `${field.label} must be at most ${field.maximum}.` };
      response[field.name] = number;
      continue;
    }
    if (field.kind === "boolean") {
      response[field.name] = value === true;
      continue;
    }
    if (!field.options?.some((option) => Object.is(option.value, value)))
      return { error: `${field.label} must match an allowed value.` };
    response[field.name] = value;
  }
  return { response };
}

export function updateApprovalField(
  field: McpApprovalField,
  rawValue: string | boolean,
): unknown {
  if (field.kind === "boolean") return rawValue === true;
  if (field.kind === "enum") {
    const index = Number(rawValue);
    return Number.isInteger(index) ? field.options?.[index]?.value : undefined;
  }
  return rawValue;
}
