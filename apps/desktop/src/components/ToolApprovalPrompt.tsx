import { useState } from "react";
import {
  openExternalUrl,
  resolveToolApproval,
  type ChatStreamPart,
  type McpApprovalField,
  type ToolApprovalRequest,
} from "../api";
import {
  approvalResponse,
  initialApprovalValues,
  updateApprovalField,
} from "../lib/toolApproval";
import { Check, Shield, X } from "./icons";

type ApprovalPart = Extract<ChatStreamPart, { kind: "event" }>;

function approvalTitle(label: string): string {
  return label
    .replace(/^Approve\s+/i, "")
    .replace(/_/g, " ")
    .replace(/^google\b/i, "Google")
    .replace(/\b(docs|sheets|slides|drive)\b/gi, (word) =>
      `${word[0].toUpperCase()}${word.slice(1).toLowerCase()}`,
    );
}

function formatApprovalDetail(detail: string): string {
  try {
    return JSON.stringify(JSON.parse(detail), null, 2);
  } catch {
    return detail;
  }
}

export function ToolApprovalPrompt({
  part,
  onDismiss,
}: {
  part: ApprovalPart;
  onDismiss?: () => void;
}) {
  const [resolving, setResolving] = useState(false);
  const [values, setValues] = useState<Record<string, unknown>>(() =>
    initialApprovalValues(part.approvalRequest),
  );
  const [error, setError] = useState("");
  const title = approvalTitle(part.label);

  async function decide(decision: "approve" | "deny") {
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
      await resolveToolApproval(part.approvalId, decision, result.response);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Approval failed.");
    } finally {
      setResolving(false);
    }
  }

  return (
    <section
      className="composer-approval"
      role="alertdialog"
      aria-label={`Approval required: ${title}`}
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
      {part.detail && (
        <details className="composer-approval-details">
          <summary>Review exact request</summary>
          <pre className="stream-approval-arguments">{formatApprovalDetail(part.detail)}</pre>
        </details>
      )}
      <ApprovalRequestBody
        request={part.approvalRequest}
        values={values}
        error={error}
        disabled={resolving}
        onChange={(field, value) => {
          setError("");
          setValues((current) => ({
            ...current,
            [field.name]: updateApprovalField(field, value),
          }));
        }}
        onApprove={() => void decide("approve")}
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
          {request.cwd && <code className="stream-approval-path">{request.cwd}</code>}
          <pre className="stream-approval-arguments">{JSON.stringify(request.permissions ?? {}, null, 2)}</pre>
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
          <button type="button" disabled={disabled} onClick={() => onOpenUrl(request.url)}>Open link</button>
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
        <button className="approval-btn deny" type="button" disabled={disabled} onClick={onDeny}>
          <X size={13} aria-hidden="true" />
          {request?.kind?.startsWith("mcp_") ? "Decline" : "Deny"}
        </button>
        {request?.kind !== "mcp_unsupported" && (
          <button className="approval-btn approve" type="button" disabled={disabled} onClick={onApprove}>
            <Check size={13} aria-hidden="true" />
            {approveLabel}
          </button>
        )}
      </span>
    </div>
  );
}
