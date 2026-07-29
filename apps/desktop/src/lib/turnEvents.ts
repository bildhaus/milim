import type {
  AgentEvent,
  ChatMessage,
  ChatStreamEventIcon,
  ChatStreamPreviewPoint,
  ChatStreamPart,
  ClaudeRunEvent,
  CodexRunEvent,
  ToolApprovalRequest,
  ToolApprovalRequestKind,
} from "../api";

type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;
type AccountRuntimeToolEvent = Extract<CodexRunEvent | ClaudeRunEvent, { type: "tool" }>;

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function compactText(value: string, max = 96): string {
  const text = value.trim().replace(/\s+/g, " ");
  return text.length > max ? text.slice(0, max - 1).trimEnd() + "..." : text;
}

function parseToolArgs(args?: string): Record<string, unknown> | null {
  if (!args?.trim()) return null;
  try {
    return asRecord(JSON.parse(args));
  } catch {
    return null;
  }
}

function toolArg(args: Record<string, unknown> | null, key: string): string | undefined {
  const value = args?.[key];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function toolErrorMessage(result: unknown): string | undefined {
  const error = asRecord(result)?.error;
  return typeof error === "string" && error.trim() ? error.trim() : undefined;
}

function numericToolField(record: Record<string, unknown> | null, keys: string[]): number | undefined {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return undefined;
}

function previewToolPointFromArgs(name: string | undefined, args: Record<string, unknown> | null): ChatStreamPreviewPoint | undefined {
  if (name !== "preview_click" && name !== "preview_type_text") return undefined;
  const x = numericToolField(args, ["x"]);
  const y = numericToolField(args, ["y"]);
  if (x == null || y == null) return undefined;
  const unit = x >= 0 && x <= 1 && y >= 0 && y <= 1 ? "ratio" : "pixel";
  return { x, y, unit };
}

function previewToolPointFromResult(name: string | undefined, args: Record<string, unknown> | null, result: unknown): ChatStreamPreviewPoint | undefined {
  if (!name?.startsWith("preview_") || (!args?.selector && !args?.text)) return undefined;
  const record = asRecord(result);
  const target = asRecord(name === "preview_click" ? record?.clicked : record?.target);
  const rect = asRecord(target?.rect);
  const x = numericToolField(rect, ["x"]);
  const y = numericToolField(rect, ["y"]);
  const width = numericToolField(rect, ["width"]) ?? 0;
  const height = numericToolField(rect, ["height"]) ?? 0;
  if (x == null || y == null) return undefined;
  return { x: x + width / 2, y: y + height / 2, unit: "pixel" };
}

function previewToolPoint(name: string | undefined, args: Record<string, unknown> | null, result?: unknown): ChatStreamPreviewPoint | undefined {
  return previewToolPointFromArgs(name, args) ?? previewToolPointFromResult(name, args, result);
}

function toolDiffStats(record: Record<string, unknown> | null): string | undefined {
  const added = numericToolField(record, ["added", "additions", "insertions", "lines_added"]);
  const removed = numericToolField(record, ["removed", "removals", "deletions", "lines_removed"]);
  if (added == null && removed == null) return undefined;
  return [`+${added ?? 0}`, `-${removed ?? 0}`].join(" ");
}

function toolEventIcon(name?: string): ChatStreamEventIcon {
  switch (name) {
    case "read_file":
    case "read_file_anchors":
    case "list_dir":
    case "write_file":
    case "edit_file":
    case "patch_file":
    case "file_change":
      return "file";
    case "shell":
    case "run_command":
    case "command":
      return "command";
    case "memory_register":
      return "memory";
    case "schedule_create":
    case "schedule_update":
    case "schedule_list":
    case "schedule_delete":
      return "schedule";
    case "screenshot":
    case "mouse_move":
    case "mouse_click":
    case "scroll":
    case "key_press":
    case "type_text":
      return "screen";
    default:
      return "tool";
  }
}

function toolLabel(name: string | undefined, done: boolean): string {
  switch (name) {
    case "read_file":
      return done ? "Read file" : "Reading file";
    case "read_file_anchors":
      return done ? "Read anchored file" : "Reading anchored file";
    case "list_dir":
      return done ? "Listed files" : "Listing files";
    case "write_file":
      return done ? "Created file" : "Creating file";
    case "edit_file":
      return done ? "Edited file" : "Editing file";
    case "patch_file":
      return done ? "Patched file" : "Patching file";
    case "shell":
      return done ? "Ran command" : "Running command";
    case "run_command":
      return done ? "Ran sandbox command" : "Running sandbox command";
    case "http_fetch":
      return done ? "Fetched URL" : "Fetching URL";
    case "memory_register":
      return done ? "Saved memory" : "Saving memory";
    case "schedule_create":
      return done ? "Created automation" : "Creating automation";
    case "schedule_update":
      return done ? "Updated automation" : "Updating automation";
    case "schedule_delete":
      return done ? "Deleted automation" : "Deleting automation";
    case "schedule_list":
      return done ? "Checked automations" : "Checking automations";
    case "screenshot":
      return done ? "Captured screen" : "Capturing screen";
    case "mouse_move":
    case "mouse_click":
    case "scroll":
    case "key_press":
    case "type_text":
      return done ? "Used computer" : "Using computer";
    default:
      return done ? `Used ${name ?? "tool"}` : `Using ${name ?? "tool"}`;
  }
}

function toolFailedLabel(name: string | undefined): string {
  switch (name) {
    case "read_file":
      return "Read file failed";
    case "read_file_anchors":
      return "Read anchored file failed";
    case "list_dir":
      return "List files failed";
    case "write_file":
      return "Create file failed";
    case "edit_file":
      return "Edit file failed";
    case "patch_file":
      return "Patch file failed";
    case "shell":
      return "Command failed";
    case "run_command":
      return "Sandbox command failed";
    case "scroll":
    case "screenshot":
    case "mouse_move":
    case "mouse_click":
    case "key_press":
    case "type_text":
      return "Computer use failed";
    case "http_fetch":
      return "Fetch failed";
    case "memory_register":
      return "Save memory failed";
    case "schedule_create":
      return "Create automation failed";
    case "schedule_update":
      return "Update automation failed";
    case "schedule_delete":
      return "Delete automation failed";
    case "schedule_list":
      return "Check automations failed";
    default:
      return `${name ?? "Tool"} failed`;
  }
}

function toolDetail(name: string | undefined, args: Record<string, unknown> | null, result?: unknown): string | undefined {
  const path = toolArg(args, "path");
  const url = toolArg(args, "url");
  const command = toolArg(args, "command");
  const record = asRecord(result);
  const error = toolErrorMessage(result);
  if (error) return compactText(path ? `${path}: ${error}` : error, 110);
  const diffStats = name === "write_file" || name === "edit_file" || name === "patch_file" ? toolDiffStats(record) : undefined;
  if (command) return command;
  if (path) return compactText(diffStats ? `${path} ${diffStats}` : path);
  if (url) return compactText(url);

  if (name === "shell" || name === "run_command") {
    const exitCode = record?.exit_code;
    return typeof exitCode === "number" ? `exit ${exitCode}` : undefined;
  }
  if (name === "list_dir" && Array.isArray(record?.entries)) return `${record.entries.length} entries`;
  if (diffStats) return diffStats;
  if ((name === "write_file" || name === "edit_file" || name === "patch_file") && typeof record?.bytes === "number") return `${record.bytes} bytes`;
  if (name === "write_file" && typeof record?.written === "number") return `${record.written} bytes`;
  const notice = asRecord(record?.memory_notice);
  const summary = notice?.summary;
  return typeof summary === "string" ? compactText(summary) : undefined;
}

export function toolStartedPart(ev: AgentEvent): ChatStreamEventPart {
  const args = parseToolArgs(ev.arguments);
  return {
    kind: "event",
    eventType: "tool",
    label: toolLabel(ev.name, false),
    detail: toolDetail(ev.name, args),
    icon: toolEventIcon(ev.name),
    name: ev.name ?? "tool",
    callId: ev.call_id,
    status: "running",
    previewPoint: previewToolPoint(ev.name, args),
    mcpApp: ev.mcp_app,
    toolArguments: ev.arguments,
  };
}

export function toolCompletedPart(ev: AgentEvent): ChatStreamEventPart {
  const args = parseToolArgs(ev.arguments);
  const error = toolErrorMessage(ev.result);
  return {
    kind: "event",
    eventType: "tool",
    label: error ? toolFailedLabel(ev.name) : toolLabel(ev.name, true),
    detail: toolDetail(ev.name, args, ev.result),
    icon: toolEventIcon(ev.name),
    name: ev.name ?? "tool",
    callId: ev.call_id,
    status: error ? "error" : "done",
    previewPoint: previewToolPoint(ev.name, args, ev.result),
    mcpApp: ev.mcp_app,
    mcpAppResult: ev.mcp_app_result,
    toolArguments: ev.arguments,
  };
}

export function accountRuntimeToolPart(ev: AccountRuntimeToolEvent): ChatStreamEventPart {
  const status = ev.status === "done" || (ev.status as string) === "completed"
    ? "done"
    : ev.status === "error" || (ev.status as string) === "failed"
      ? "error"
      : "running";
  return {
    kind: "event",
    eventType: "tool",
    label: ev.label || (status === "running" ? `Using ${ev.name}` : status === "error" ? `${ev.name} failed` : `Used ${ev.name}`),
    detail:
      (status === "error" ? ev.error : ev.detail) ||
      ev.detail ||
      undefined,
    icon: ev.icon || toolEventIcon(ev.name),
    name: ev.id || ev.name,
    status,
  };
}

export function statusPart(label: string, detail?: string, tone: "status" | "warning" | "error" = "status"): ChatStreamEventPart {
  return {
    kind: "event",
    eventType: tone,
    label,
    detail,
    icon: tone === "error" ? "error" : "tool",
    status: tone === "error" ? "error" : "done",
  };
}

export function toolApprovalPart(
  event: {
    approval_id?: string;
    name?: string;
    arguments?: string;
    decision?: "approve" | "deny";
    status?: "decided" | "delivered";
    message?: string;
    request_kind?: ToolApprovalRequestKind;
    request?: Record<string, unknown>;
  },
): ChatStreamEventPart {
  const failed = event.message != null;
  const progressing = event.status != null;
  const resolved = event.decision != null && !progressing && !failed;
  const approvalRequest = !resolved && !progressing && !failed && event.request_kind
    ? ({ kind: event.request_kind, ...(event.request ?? {}) } as ToolApprovalRequest)
    : undefined;
  const requestLabel = approvalRequest?.kind === "permissions"
    ? "Approve additional permissions"
    : approvalRequest?.kind === "mcp_form"
      ? "Provide MCP input"
      : approvalRequest?.kind === "mcp_url"
        ? "Continue MCP authorization"
        : approvalRequest?.kind === "mcp_unsupported"
          ? "Unsupported MCP request"
          : `Approve ${event.name ?? "tool"}`;
  const outcomeTarget = event.name === "command"
    ? "Command"
    : event.name === "file_change"
      ? "File change"
      : event.name === "permissions"
        ? "Permissions"
        : event.name ?? "Tool";
  return {
    kind: "event",
    eventType: "status",
    label: failed
      ? "Approval delivery failed"
      : progressing
        ? event.status === "decided" ? "Approval submitted" : "Approval delivered"
        : resolved
          ? `${outcomeTarget} ${event.decision === "approve" ? "approved" : "denied"}`
          : requestLabel,
    detail: failed
      ? event.message
      : resolved || progressing
        ? undefined
      : approvalRequest && approvalRequest.kind !== "command" && approvalRequest.kind !== "file_change"
        ? "message" in approvalRequest ? approvalRequest.message : "reason" in approvalRequest ? approvalRequest.reason ?? undefined : undefined
        : event.arguments,
    icon: toolEventIcon(event.name),
    name: `approval:${event.approval_id}`,
    status: failed ? "error" : progressing ? "running" : "done",
    approvalId: event.approval_id,
    approvalStatus: failed
      ? "failed"
      : progressing
        ? event.status
        : resolved
          ? (event.decision === "approve" ? "approved" : "denied")
          : "pending",
    approvalRequest,
  };
}

export function runtimeWarningMessage(label: string, detail: string): ChatMessage {
  return { role: "assistant", content: "", streamParts: [statusPart(label, detail, "warning")] };
}
