export type ManagedPreviewRuntimeContext = {
  kind: string;
  status: string;
  active: true;
  ready: boolean;
  url?: string;
};

type PreviewRuntimeState = {
  kind?: string;
  status: string;
  url?: string;
  pid?: number;
  active?: boolean;
  ready?: boolean;
};

const ACTIVE_RUNTIME_STATUSES = new Set(["installing", "starting", "running"]);

function cleanMetadata(value: string | undefined, fallback: string, maxChars: number): string {
  const cleaned = [...(value ?? "").trim()]
    .filter((char) => !/\p{Cc}/u.test(char))
    .slice(0, maxChars)
    .join("");
  return cleaned || fallback;
}

/**
 * Returns the small, untrusted runtime snapshot that can safely cross a turn
 * boundary. Process details, commands, logs, errors, and source are excluded.
 */
export function managedPreviewRuntimeForTurn(
  runtime?: PreviewRuntimeState | null,
): ManagedPreviewRuntimeContext | null {
  if (!runtime) return null;
  const status = cleanMetadata(runtime.status, "unknown", 64);
  const active = runtime.active
    ?? (Boolean(runtime.pid) || ACTIVE_RUNTIME_STATUSES.has(status.toLowerCase()));
  if (!active) return null;
  const url = cleanMetadata(runtime.url, "", 2_048);
  return {
    kind: cleanMetadata(runtime.kind, "app", 64),
    status,
    active: true,
    ready: runtime.ready === true,
    ...(url ? { url } : {}),
  };
}
