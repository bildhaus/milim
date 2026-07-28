export function normalizePreviewRuntimeFolder(folder?: string | null): string {
  let normalized = (folder ?? "").trim().replace(/\\/g, "/");
  if (normalized.toLowerCase().startsWith("//?/unc/")) normalized = `//${normalized.slice(8)}`;
  else if (normalized.startsWith("//?/")) normalized = normalized.slice(4);
  const trimmed = normalized.length > 1 ? normalized.replace(/\/+$/, "") : normalized;
  return (/^[A-Za-z]:$/.test(trimmed) ? `${trimmed}/` : trimmed).toLowerCase();
}

function hashPreviewRuntimeKey(value: string): string {
  let hash = 0x811c9dc5;
  for (let i = 0; i < value.length; i += 1) {
    hash ^= value.charCodeAt(i);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(36);
}

export function previewRuntimeKeyForThread(threadId: string, folder?: string | null): string {
  const normalizedFolder = normalizePreviewRuntimeFolder(folder);
  return normalizedFolder ? `project-${hashPreviewRuntimeKey(normalizedFolder)}` : threadId;
}

export function previewRuntimeFoldersEqual(left?: string | null, right?: string | null): boolean {
  return normalizePreviewRuntimeFolder(left) === normalizePreviewRuntimeFolder(right);
}
