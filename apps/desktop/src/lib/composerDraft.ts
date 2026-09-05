import type { ChatAttachment } from "../api";

export type ComposerDraft = { text: string; attachments: ChatAttachment[] };
export const EMPTY_COMPOSER_DRAFT: ComposerDraft = { text: "", attachments: [] };

/** Acknowledgments remove only the submitted snapshot, never a newer draft. */
export function consumeComposerDraft(current: ComposerDraft, submitted: ComposerDraft): ComposerDraft {
  const sentIds = new Set(submitted.attachments.map((attachment) => attachment.id));
  return {
    text: current.text.trim() === submitted.text.trim() ? "" : current.text,
    attachments: current.attachments.filter((attachment) => !sentIds.has(attachment.id)),
  };
}

export function normalizeComposerDraft(value: unknown): ComposerDraft {
  if (typeof value === "string") return { text: value, attachments: [] };
  if (!value || typeof value !== "object") return EMPTY_COMPOSER_DRAFT;
  const draft = value as Partial<ComposerDraft>;
  return {
    text: typeof draft.text === "string" ? draft.text : "",
    attachments: Array.isArray(draft.attachments)
      ? draft.attachments.filter((item) => item && typeof item.id === "string" && typeof item.name === "string" && typeof item.mime === "string")
      : [],
  };
}
