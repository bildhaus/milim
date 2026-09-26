import type { ChatStreamPart } from "../api.js";

type ChatStreamEventPart = Extract<ChatStreamPart, { kind: "event" }>;

const OUTCOME_LABELS: Record<string, string> = {
  allow: "allowed",
  approve: "approved",
  block: "blocked the turn",
  context: "added context",
  continue: "asked to continue",
  deny: "denied",
  error: "failed",
  feedback: "added feedback",
  ok: "ran",
  skipped: "skipped",
  timeout: "timed out",
};

const WARNING_OUTCOMES = new Set(["block", "deny", "error", "skipped", "timeout"]);

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

/**
 * Stream part for one user hook run (`hook` agent event). Untrusted project
 * hooks carry the workspace and config hash the Trust action needs.
 */
export function hookEventPart(data: Record<string, unknown>): ChatStreamEventPart {
  const outcome = text(data.outcome) ?? "ok";
  const trust = data.trust && typeof data.trust === "object"
    ? (data.trust as Record<string, unknown>)
    : undefined;
  const workspace = text(trust?.workspace);
  const configHash = text(trust?.config_hash);
  if (workspace && configHash) {
    return {
      kind: "event",
      eventType: "warning",
      label: "Project hooks are not trusted",
      detail: text(data.message),
      status: "done",
      hookTrust: { workspace, configHash },
    };
  }
  const event = text(data.event) ?? "Hook";
  const tool = text(data.tool_name);
  const duration = typeof data.duration_ms === "number" ? ` in ${Math.round(data.duration_ms)} ms` : "";
  const label = `${event} hook${tool ? ` (${tool})` : ""} ${OUTCOME_LABELS[outcome] ?? outcome}${duration}`;
  const hook = text(data.hook);
  const message = text(data.message);
  const detail = message && hook ? `${hook}: ${message}` : message ?? hook;
  if (WARNING_OUTCOMES.has(outcome)) {
    return { kind: "event", eventType: "warning", label, detail, status: "done" };
  }
  // Routine runs fold into the turn's work group like tool calls.
  return {
    kind: "event",
    eventType: "tool",
    label,
    name: `${event} hook`,
    icon: "tool",
    detail,
    status: "done",
  };
}

/** The hook commands a Trust confirmation lists for review. */
export function hookReviewLines(hooks: unknown): string[] {
  if (!hooks || typeof hooks !== "object") return [];
  return Object.entries(hooks as Record<string, unknown>).flatMap(([event, entries]) =>
    Array.isArray(entries)
      ? entries.flatMap((entry) => {
          if (!entry || typeof entry !== "object") return [];
          const spec = entry as Record<string, unknown>;
          const command = text(spec.command);
          if (!command) return [];
          const matcher = text(spec.matcher);
          return [`${event}${matcher ? ` [${matcher}]` : ""}: ${command}`];
        })
      : [],
  );
}
