import type { ChatMessage } from "../api";

export const NEW_CHAT_TITLE = "New chat";
const AUTO_TITLE_MAX_LENGTH = 160;
const LEGACY_AUTO_TITLE_MAX_LENGTH = 42;
export const AI_THREAD_TITLE_SYSTEM_PROMPT =
  "You name chat threads. Reply with only a concise 2-5 word title. No quotes, punctuation, emojis, prefixes, or explanations.";

export function deriveThreadTitle(messages: ChatMessage[]): string {
  const firstUser = messages.find((m) => m.role === "user");
  const t = (firstUser?.content ?? "").trim().replace(/\s+/g, " ");
  const attachmentTitle = firstUser?.attachments?.[0]?.name;
  if (!t && attachmentTitle) return `Attached: ${attachmentTitle}`;
  if (!t) return NEW_CHAT_TITLE;
  return t.length > AUTO_TITLE_MAX_LENGTH ? `${t.slice(0, AUTO_TITLE_MAX_LENGTH)}...` : t;
}

export function sanitizeAiThreadTitle(raw: string): string | null {
  const firstLine = raw
    .trim()
    .replace(/^```(?:\w+)?\s*/i, "")
    .replace(/```$/i, "")
    .split(/\r?\n/)
    .find((line) => line.trim());
  const title = (firstLine ?? "")
    .trim()
    .replace(/^(?:[-*]|\d+[.)]|\u2022)\s*/, "")
    .replace(/^(?:thread title|title)\s*:\s*/i, "")
    .replace(/^["'`]+|["'`]+$/g, "")
    .replace(/[.!?:;,]+$/g, "")
    .replace(/[\/&-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  const words = title.split(" ").filter(Boolean);
  if (words.length < 2 || words.length > 5) return null;
  if (/[^\p{L}\p{N} ]/u.test(title)) return null;
  return title;
}

export function shouldReplaceThreadTitle(currentTitle: string, messages: ChatMessage[]): boolean {
  const title = currentTitle.trim();
  const firstUserText = (messages.find((message) => message.role === "user")?.content ?? "")
    .trim()
    .replace(/\s+/g, " ");
  const legacyTitle = firstUserText.length > LEGACY_AUTO_TITLE_MAX_LENGTH
    ? `${firstUserText.slice(0, LEGACY_AUTO_TITLE_MAX_LENGTH)}...`
    : firstUserText;
  return title === NEW_CHAT_TITLE || title === deriveThreadTitle(messages) || title === legacyTitle;
}

export function isThreadNamingModel(model: string | { id: string; capabilities?: { imageOutput?: boolean; videoOutput?: boolean; musicOutput?: boolean } }): boolean {
  const id = (typeof model === "string" ? model : model.id).trim().toLowerCase();
  if (
    !id ||
    id === "mock-echo" ||
    id.startsWith("codex:") ||
    id.startsWith("claude:") ||
    id.startsWith("opencode:") ||
    id.startsWith("pi:")
  ) return false;
  return typeof model === "string" || (!model.capabilities?.imageOutput && !model.capabilities?.videoOutput && !model.capabilities?.musicOutput);
}
