/**
 * Classify provider, runtime, and setup failures into a small set of kinds
 * so the composer can show friendly copy and one recovery action instead of
 * raw upstream text.
 *
 * Canonical runs carry a structured `provider_error` classified in Rust
 * (`milim_core::provider_error`). Legacy paths, account runtimes, and older
 * servers only provide text, so {@link classifyErrorMessage} mirrors the Rust
 * rules as a fallback. Keep the two in sync.
 */

export type ProviderErrorKind =
  | "auth"
  | "rate_limited"
  | "context_length"
  | "model_not_found"
  | "provider_unavailable"
  | "quota"
  | "unknown";

/** Local setup blockers that are not provider responses. */
export type SetupErrorKind = "privacy_blocked" | "workspace_required" | "runtime_setup";

export type ErrorKind = ProviderErrorKind | SetupErrorKind;

export interface ProviderErrorInfo {
  kind: ProviderErrorKind;
  status?: number;
  retry_after_secs?: number;
}

export interface ErrorClassification {
  kind: ErrorKind;
  status?: number;
  retryAfterSecs?: number;
}

export type ErrorRecoveryAction =
  | "update_key"
  | "switch_model"
  | "retry"
  | "manage_models"
  | "choose_folder"
  | "privacy_settings";

type Rule = { kind: ErrorKind; phrases: readonly string[] };

const SETUP_RULES: readonly Rule[] = [
  { kind: "privacy_blocked", phrases: ["blocked by the privacy gate"] },
  {
    kind: "workspace_required",
    phrases: [
      "no working folder selected",
      "no working folder is selected",
      "workspace folder is required",
      "select a milim workspace folder",
    ],
  },
  {
    kind: "runtime_setup",
    phrases: [
      "not signed in",
      "no configured models",
      "no authenticated or configured models",
      "cli is unavailable",
      "codex is unavailable",
      "codex login did not complete",
      "disabled in providers",
      "cli was not found on path",
    ],
  },
];

const QUOTA_PHRASES = [
  "insufficient_quota",
  "exceeded your current quota",
  "quota exceeded",
  "billing",
  "credit balance",
  "insufficient credit",
  "insufficient balance",
  "payment required",
  "out of credits",
];
const CONTEXT_PHRASES = [
  "context_length_exceeded",
  "context length",
  "context window",
  "maximum context",
  "prompt is too long",
  "too many tokens",
  "reduce the length",
  "input is too long",
  "exceeds the model's maximum",
];
const AUTH_PHRASES = [
  "invalid api key",
  "invalid_api_key",
  "incorrect api key",
  "invalid x-api-key",
  "api key not valid",
  "authentication_error",
  "unauthorized",
  "permission_denied",
];
const MODEL_PHRASES = [
  "model_not_found",
  "model not found",
  "unknown model",
  "no such model",
  "invalid model",
];
const RATE_PHRASES = ["rate limit", "rate_limit", "ratelimit", "too many requests", "usage limit"];
const UNAVAILABLE_PHRASES = [
  "overloaded",
  "service unavailable",
  "bad gateway",
  "gateway timeout",
  "timed out",
  "connection refused",
  "connection reset",
  "error sending request",
  "dns error",
  "failed to lookup address",
  "failed to fetch",
  "networkerror",
];

const STATUS_MARKERS = ["-> ", "http ", "status code ", "status: ", "status "];

function httpStatus(text: string): number | undefined {
  for (const marker of STATUS_MARKERS) {
    let from = 0;
    for (;;) {
      const index = text.indexOf(marker, from);
      if (index < 0) break;
      from = index + marker.length;
      const match = /^(\d{3})(?!\d)/.exec(text.slice(from));
      const status = match ? Number(match[1]) : NaN;
      if (status >= 400 && status < 600) return status;
    }
  }
  return undefined;
}

function retryAfterSecs(text: string): number | undefined {
  for (const marker of ["retry after ", "retry-after: ", "retry-after ", "try again in "]) {
    const index = text.indexOf(marker);
    if (index < 0) continue;
    const match = /^(\d+(?:\.\d+)?)\s*(ms|min|m|s)?/.exec(text.slice(index + marker.length));
    if (!match) continue;
    const value = Number(match[1]);
    if (!Number.isFinite(value) || value < 0) continue;
    const unit = match[2] ?? "s";
    const seconds = unit === "ms" ? value / 1000 : unit === "m" || unit === "min" ? value * 60 : value;
    return Math.max(1, Math.ceil(seconds));
  }
  return undefined;
}

/** Classify an error message. Mirrors `classify_provider_error` in Rust. */
export function classifyErrorMessage(message: string): ErrorClassification {
  const text = message.toLowerCase();
  const has = (phrases: readonly string[]) => phrases.some((phrase) => text.includes(phrase));
  for (const rule of SETUP_RULES) {
    if (has(rule.phrases)) return { kind: rule.kind };
  }
  const status = httpStatus(text);
  if (status === 402 || has(QUOTA_PHRASES)) return { kind: "quota", status };
  if (status === 413 || has(CONTEXT_PHRASES)) return { kind: "context_length", status };
  if (status === 401 || status === 403 || has(AUTH_PHRASES)) return { kind: "auth", status };
  if (has(MODEL_PHRASES) || (status === 404 && text.includes("model"))) {
    return { kind: "model_not_found", status };
  }
  if (status === 429 || has(RATE_PHRASES)) {
    return { kind: "rate_limited", status, retryAfterSecs: retryAfterSecs(text) };
  }
  if ((status !== undefined && status >= 500) || has(UNAVAILABLE_PHRASES)) {
    return { kind: "provider_unavailable", status, retryAfterSecs: retryAfterSecs(text) };
  }
  return { kind: "unknown", status };
}

const PROVIDER_KINDS = new Set<ProviderErrorKind>([
  "auth",
  "rate_limited",
  "context_length",
  "model_not_found",
  "provider_unavailable",
  "quota",
  "unknown",
]);

/** Read the optional structured `provider_error` from a canonical run error. */
export function providerErrorFromValue(value: unknown): ProviderErrorInfo | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const raw = value as Record<string, unknown>;
  if (typeof raw.kind !== "string" || !PROVIDER_KINDS.has(raw.kind as ProviderErrorKind)) {
    return undefined;
  }
  return {
    kind: raw.kind as ProviderErrorKind,
    status: typeof raw.status === "number" ? raw.status : undefined,
    retry_after_secs:
      typeof raw.retry_after_secs === "number" && raw.retry_after_secs >= 0
        ? raw.retry_after_secs
        : undefined,
  };
}

/** Prefer the server's structured classification, falling back to text. */
export function classifyError(
  message: string,
  structured?: ProviderErrorInfo,
): ErrorClassification {
  if (structured && structured.kind !== "unknown") {
    return {
      kind: structured.kind,
      status: structured.status,
      retryAfterSecs: structured.retry_after_secs,
    };
  }
  return classifyErrorMessage(message);
}

export function errorRecoveryAction(kind: ErrorKind): ErrorRecoveryAction | null {
  switch (kind) {
    case "privacy_blocked":
      return "privacy_settings";
    case "workspace_required":
      return "choose_folder";
    case "runtime_setup":
      return "manage_models";
    case "auth":
      return "update_key";
    case "model_not_found":
    case "context_length":
    case "quota":
      return "switch_model";
    case "rate_limited":
    case "provider_unavailable":
      return "retry";
    default:
      return null;
  }
}

const FRIENDLY_MESSAGES: Record<ProviderErrorKind, string> = {
  auth: "The provider rejected the API key or sign-in. Update the key in Providers, then retry.",
  rate_limited: "The provider is rate limiting requests. Wait a moment, then retry.",
  context_length: "This conversation is longer than the model's context window. Switch to a larger model or compact the thread.",
  model_not_found: "The provider does not offer the selected model. Choose another model.",
  provider_unavailable: "The provider is unreachable or having problems. Retry shortly.",
  quota: "The provider account is out of quota or credit. Check billing, or switch to another model.",
  unknown: "",
};

export interface FriendlyError {
  /** Friendly copy; equals the raw message when the error is unrecognized. */
  message: string;
  /** Raw text for the collapsible "Technical details" section. */
  detail?: string;
  kind: ErrorKind;
  retryAfterSecs?: number;
}

export interface ErrorNotice {
  tone: "error";
  message: string;
  detail?: string;
  providerError?: ProviderErrorInfo;
  retryAfterSecs?: number;
}

/** Composer notice for a failed turn: friendly copy with the raw detail. */
export function errorNotice(message: string, providerError?: ProviderErrorInfo): ErrorNotice {
  const friendly = friendlyError(message, providerError);
  return {
    tone: "error",
    message: friendly.message,
    ...(friendly.detail ? { detail: friendly.detail } : {}),
    ...(providerError ? { providerError } : {}),
    ...(friendly.retryAfterSecs ? { retryAfterSecs: friendly.retryAfterSecs } : {}),
  };
}

/** Friendly composer copy for a provider failure, keeping the raw text. */
export function friendlyError(message: string, structured?: ProviderErrorInfo): FriendlyError {
  const raw = message.trim();
  const classification = classifyError(raw, structured);
  const friendly =
    classification.kind in FRIENDLY_MESSAGES
      ? FRIENDLY_MESSAGES[classification.kind as ProviderErrorKind]
      : "";
  if (!friendly) return { message: raw, kind: classification.kind };
  return {
    message: friendly,
    detail: raw && raw !== friendly ? raw : undefined,
    kind: classification.kind,
    retryAfterSecs: classification.retryAfterSecs,
  };
}
