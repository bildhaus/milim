import { useEffect, useRef, useState } from "react";
import {
  openExternalUrl,
  resolveToolApproval,
  type ChatStreamPart,
  type McpApprovalField,
  type ToolApprovalRequest,
} from "../api";
import {
  approvalChatAllowance,
  approvalResponse,
  approvalShortcut,
  approvalShortcutTarget,
  initialApprovalValues,
  updateApprovalField,
  type ToolApprovalScope,
} from "../lib/toolApproval";
import { Check, Shield, X } from "./icons";

type ApprovalPart = Extract<ChatStreamPart, { kind: "event" }>;

const HIDDEN_APPROVAL_FIELDS = new Set([
  "approvalId",
  "availableDecisions",
  "environmentId",
  "itemId",
  "threadId",
  "turnId",
]);

const APPROVAL_FIELD_LABELS: Record<string, string> = {
  additionalPermissions: "Requested access",
  command: "Command",
  cwd: "Folder",
  domains: "Domains",
  file_id: "File",
  fileId: "File",
  fileSystem: "File access",
  grantRoot: "Write access",
  network: "Network access",
  operations: "Changes",
  path: "File",
  permissions: "Requested access",
  reason: "Reason",
  url: "Link",
};

function approvalTitle(label: string): string {
  return label
    .replace(/^Approve\s+/i, "")
    .replace(/_/g, " ")
    .replace(/^google\b/i, "Google")
    .replace(/\b(docs|sheets|slides|drive)\b/gi, (word) =>
      `${word[0].toUpperCase()}${word.slice(1).toLowerCase()}`,
    );
}

function humanizeApprovalText(value: string): string {
  const text = value
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim();
  return text ? `${text[0].toUpperCase()}${text.slice(1)}` : value;
}

function approvalFieldLabel(field: string): string {
  return APPROVAL_FIELD_LABELS[field] ?? humanizeApprovalText(field);
}

function approvalDetail(detail: string): unknown {
  try {
    return JSON.parse(detail);
  } catch {
    return detail;
  }
}

function ApprovalValue({ value, field }: { value: unknown; field?: string }) {
  if (value == null) return <span className="composer-approval-empty">None</span>;
  if (typeof value === "boolean") return <span>{value ? "Yes" : "No"}</span>;
  if (Array.isArray(value)) {
    const items = value.filter((item) => item != null);
    if (!items.length) return <span className="composer-approval-empty">None</span>;
    if (field === "command" && items.every((item) => typeof item === "string")) {
      return <code>{items.join(" ")}</code>;
    }
    return (
      <ol className="composer-approval-list">
        {items.map((item, index) => (
          <li key={index}><ApprovalValue value={item} /></li>
        ))}
      </ol>
    );
  }
  if (typeof value === "object") {
    const entries = Object.entries(value).filter(
      ([key, item]) => !HIDDEN_APPROVAL_FIELDS.has(key) && item != null,
    );
    if (!entries.length) return <span className="composer-approval-empty">No additional details</span>;
    return (
      <dl className="composer-approval-fields">
        {entries.map(([key, item]) => (
          <div key={key}>
            <dt>{approvalFieldLabel(key)}</dt>
            <dd><ApprovalValue value={item} field={key} /></dd>
          </div>
        ))}
      </dl>
    );
  }
  const text = String(value);
  const readable = field && ["action", "kind", "type"].includes(field)
    ? humanizeApprovalText(text)
    : text;
  return field && ["command", "cwd", "grantRoot", "path", "url"].includes(field)
    ? <code>{readable}</code>
    : <span>{readable}</span>;
}

function ApprovalDetail({ value }: { value: unknown }) {
  return (
    <div className="composer-approval-request">
      <ApprovalValue value={value} />
    </div>
  );
}

export function ToolApprovalPrompt({
  part,
  onDismiss,
  keyboard = false,
  composerEmpty = true,
  onResolved,
}: {
  part: ApprovalPart;
  onDismiss?: () => void;
  onResolved?: (scope: ToolApprovalScope) => void;
  /** This prompt owns Enter/Y (allow once) and Escape/N (deny). */
  keyboard?: boolean;
  composerEmpty?: boolean;
}) {
  const [resolving, setResolving] = useState(false);
  const [values, setValues] = useState<Record<string, unknown>>(() =>
    initialApprovalValues(part.approvalRequest),
  );
  const [error, setError] = useState("");
  const [submitted, setSubmitted] = useState<"approve" | "deny" | null>(null);
  const title = approvalTitle(part.label);
  const showReview = part.detail && (
    !part.approvalRequest ||
    part.approvalRequest.kind === "command" ||
    part.approvalRequest.kind === "file_change"
  );

  const chatAllowance = approvalChatAllowance(part);
  const decideRef = useRef<(decision: "approve" | "deny") => void>(() => {});

  async function decide(decision: "approve" | "deny", scope: ToolApprovalScope = "once") {
    if (!part.approvalId || resolving) return;
    if (decision === "approve" && part.approvalRequest?.kind === "mcp_unsupported") return;
    const result = decision === "approve"
      ? approvalResponse(part.approvalRequest, values)
      : {};
    if (result.error) {
      setError(result.error);
      return;
    }
    setResolving(true);
    setError("");
    try {
      await resolveToolApproval(part.approvalId, decision, result.response, scope);
      setSubmitted(decision);
      onResolved?.(scope);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Approval failed.");
      setResolving(false);
    }
  }

  decideRef.current = (decision) => void decide(decision);
  const shortcutsActive = keyboard && !resolving && submitted == null;
  useEffect(() => {
    if (!shortcutsActive) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat) return;
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      // A modal sheet or confirmation owns the keyboard while it is open.
      if (document.querySelector('[aria-modal="true"]')) return;
      const decision = approvalShortcut(
        event.key,
        approvalShortcutTarget(event.target),
        composerEmpty,
      );
      if (!decision) return;
      if (decision === "approve" && part.approvalRequest?.kind === "mcp_unsupported") return;
      // Capture phase: consume the key before the composer or the global
      // Escape-to-stop shortcut sees it.
      event.preventDefault();
      event.stopPropagation();
      decideRef.current(decision);
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [composerEmpty, part.approvalRequest?.kind, shortcutsActive]);

  return (
    <section
      className="composer-approval"
      role="alertdialog"
      aria-label={`Approval required: ${title}`}
      aria-keyshortcuts={shortcutsActive && composerEmpty ? "Enter Y Escape N" : undefined}
    >
      <header className="composer-approval-header">
        <span className="composer-approval-icon" aria-hidden="true">
          <Shield size={15} />
        </span>
        <span className="composer-approval-heading">
          <small>Approval required</small>
          <strong>{title}</strong>
        </span>
      </header>
      {submitted && (
        <p className="composer-approval-progress" role="status">
          Decision delivered; waiting for the runtime to resume...
        </p>
      )}
      {showReview && (
        <details className="composer-approval-details">
          <summary>Review request</summary>
          <ApprovalDetail value={approvalDetail(part.detail!)} />
        </details>
      )}
      <ApprovalRequestBody
        request={part.approvalRequest}
        values={values}
        error={error}
        disabled={resolving || submitted != null}
        onChange={(field, value) => {
          setError("");
          setValues((current) => ({
            ...current,
            [field.name]: updateApprovalField(field, value),
          }));
        }}
        onApprove={() => void decide("approve")}
        onApproveForChat={chatAllowance ? () => void decide("approve", "thread") : undefined}
        chatAllowanceTitle={
          chatAllowance?.kind === "command"
            ? `Run exactly \`${chatAllowance.command}\` without asking again in this chat`
            : chatAllowance
              ? `Allow ${chatAllowance.tool} without asking again in this chat`
              : undefined
        }
        shortcutHint={shortcutsActive && composerEmpty}
        onDeny={() => void decide("deny")}
        onDismiss={onDismiss}
        onOpenUrl={(url) => {
          setError("");
          void openExternalUrl(url).catch((cause) => {
            setError(cause instanceof Error ? cause.message : "Could not open URL.");
          });
        }}
      />
    </section>
  );
}

function ApprovalRequestBody({
  request,
  values,
  error,
  disabled,
  onChange,
  onApprove,
  onApproveForChat,
  chatAllowanceTitle,
  shortcutHint = false,
  onDeny,
  onDismiss,
  onOpenUrl,
}: {
  request?: ToolApprovalRequest;
  values: Record<string, unknown>;
  error: string;
  disabled: boolean;
  onChange: (field: McpApprovalField, value: string | boolean) => void;
  onApprove: () => void;
  onApproveForChat?: () => void;
  chatAllowanceTitle?: string;
  shortcutHint?: boolean;
  onDeny: () => void;
  onDismiss?: () => void;
  onOpenUrl: (url: string) => void;
}) {
  const approveLabel = request?.kind === "permissions"
    ? "Allow once"
    : request?.kind === "mcp_form"
      ? "Submit"
      : request?.kind === "mcp_url"
        ? "Continue"
        : "Approve";
  return (
    <div className="stream-approval-body">
      {request?.kind === "permissions" && (
        <>
          {request.reason && <p>{request.reason}</p>}
          <ApprovalDetail value={{
            cwd: request.cwd,
            permissions: request.permissions ?? {},
          }} />
        </>
      )}
      {request?.kind === "mcp_form" && (
        <div className="stream-approval-form">
          <p>{request.message}</p>
          {request.fields.map((field) => (
            <label key={field.name} className={field.kind === "boolean" ? "stream-approval-checkbox" : undefined}>
              <span>{field.label}{field.required ? " *" : ""}</span>
              {field.description && <small>{field.description}</small>}
              {field.kind === "boolean" ? (
                <input
                  type="checkbox"
                  checked={values[field.name] === true}
                  disabled={disabled}
                  onChange={(event) => onChange(field, event.currentTarget.checked)}
                />
              ) : field.kind === "enum" ? (
                <select
                  value={String(field.options?.findIndex((option) => Object.is(option.value, values[field.name])) ?? -1)}
                  disabled={disabled}
                  onChange={(event) => onChange(field, event.currentTarget.value)}
                >
                  <option value="-1">Select...</option>
                  {field.options?.map((option, index) => <option key={index} value={index}>{option.label}</option>)}
                </select>
              ) : (
                <input
                  type={field.kind === "string" ? "text" : "number"}
                  value={typeof values[field.name] === "string" || typeof values[field.name] === "number" ? String(values[field.name]) : ""}
                  min={field.minimum}
                  max={field.maximum}
                  step={field.kind === "integer" ? 1 : field.kind === "number" ? "any" : undefined}
                  minLength={field.min_length}
                  maxLength={field.max_length}
                  disabled={disabled}
                  onChange={(event) => onChange(field, event.currentTarget.value)}
                />
              )}
            </label>
          ))}
        </div>
      )}
      {request?.kind === "mcp_url" && (
        <div className="stream-approval-url">
          <p>{request.message}</p>
          <button className="approval-btn" type="button" disabled={disabled} onClick={() => onOpenUrl(request.url)}>Open link</button>
        </div>
      )}
      {request?.kind === "mcp_unsupported" && (
        <div className="stream-approval-unsupported">
          <p>{request.message}</p>
          <small>{request.reason}</small>
        </div>
      )}
      <span className="stream-approval-actions">
        {error && <span className="stream-approval-error" role="alert">{error}</span>}
        {error && onDismiss && (
          <button className="approval-btn" type="button" disabled={disabled} onClick={onDismiss}>
            Dismiss
          </button>
        )}
        <button
          className="approval-btn deny"
          type="button"
          disabled={disabled}
          onClick={onDeny}
          title={shortcutHint ? "Deny (Esc or N)" : undefined}
        >
          <X size={13} aria-hidden="true" />
          {request?.kind?.startsWith("mcp_") ? "Decline" : "Deny"}
        </button>
        {onApproveForChat && (
          <button
            className="approval-btn"
            type="button"
            disabled={disabled}
            onClick={onApproveForChat}
            title={chatAllowanceTitle}
          >
            Allow for this chat
          </button>
        )}
        {request?.kind !== "mcp_unsupported" && (
          <button
            className="approval-btn approve"
            type="button"
            disabled={disabled}
            onClick={onApprove}
            title={onApproveForChat ? `Allow once${shortcutHint ? " (Enter or Y)" : ""}` : shortcutHint ? `${approveLabel} (Enter or Y)` : undefined}
          >
            <Check size={13} aria-hidden="true" />
            {approveLabel}
          </button>
        )}
      </span>
    </div>
  );
}
