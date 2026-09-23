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

export type ToolApprovalScope = "once" | "thread";

/** Mirrors `SHELL_TOOL_NAMES` in `crates/milim-server/src/approval_allowances.rs`. */
const SHELL_TOOL_NAMES = new Set([
  "shell",
  "bash",
  "command",
  "sh",
  "zsh",
  "powershell",
  "terminal",
  "exec",
  "exec_command",
  "run_command",
  "run_shell",
  "shell_command",
  "execute_command",
  "local_shell",
]);

function approvalToolName(part: ApprovalPart): string {
  return part.label.replace(/^(Approve|Approval required:)\s+/i, "").trim();
}

function approvalCommand(detail?: string): string | null {
  if (!detail) return null;
  let value: unknown;
  try {
    value = JSON.parse(detail);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const object = value as Record<string, unknown>;
  const input = object.input && typeof object.input === "object"
    ? (object.input as Record<string, unknown>)
    : undefined;
  const command = object.command ?? object.cmd ?? input?.command;
  if (typeof command === "string") return command.trim() || null;
  if (Array.isArray(command) && command.length && command.every((item) => typeof item === "string")) {
    return command.join(" ");
  }
  return null;
}

/**
 * What "Allow for this chat" would allow, or null when the request must stay
 * one-shot. Shell-style tools are allowed only for their exact command.
 */
export function approvalChatAllowance(
  part: ApprovalPart,
): { kind: "command"; command: string } | { kind: "tool"; tool: string } | null {
  const requestKind = part.approvalRequest?.kind;
  if (requestKind && requestKind !== "command" && requestKind !== "file_change") return null;
  const tool = approvalToolName(part);
  if (!tool) return null;
  const command = approvalCommand(part.detail);
  if (command) return { kind: "command", command };
  const lowered = tool.toLowerCase();
  if (
    SHELL_TOOL_NAMES.has(lowered) ||
    lowered.includes("shell") ||
    lowered.includes("bash") ||
    lowered.includes("terminal")
  ) {
    return null;
  }
  return { kind: "tool", tool };
}

export type ApprovalShortcutTarget = "composer" | "editable" | "button" | "other";

/** Classify a keydown target for approval shortcuts. */
export function approvalShortcutTarget(target: EventTarget | null): ApprovalShortcutTarget {
  if (!target || typeof (target as Element).closest !== "function") return "other";
  const element = target as HTMLElement;
  if (element.closest('[data-testid="composer-input"]')) return "composer";
  // Form fields and other overlays (menus, pickers, sheets) keep their keys,
  // so Escape closes them instead of denying the approval.
  if (
    element.closest(
      "input, textarea, select, [contenteditable=''], [contenteditable='true'], [role='dialog'], [role='menu'], [role='listbox']",
    )
  ) {
    return "editable";
  }
  if (element.closest("button, a, summary")) return "button";
  return "other";
}

/**
 * Keyboard decision for a visible approval prompt. Enter/Y approve once and
 * Escape/N deny, only while the composer is empty. Letters never fire from a
 * text field, and Enter on a focused button keeps its native click.
 */
export function approvalShortcut(
  key: string,
  target: ApprovalShortcutTarget,
  composerEmpty: boolean,
): "approve" | "deny" | null {
  if (!composerEmpty || target === "editable") return null;
  const lowered = key.length === 1 ? key.toLowerCase() : key;
  if (lowered === "Escape") return "deny";
  if (lowered === "Enter") return target === "button" ? null : "approve";
  if (target === "composer") return null;
  if (lowered === "y") return "approve";
  if (lowered === "n") return "deny";
  return null;
}
