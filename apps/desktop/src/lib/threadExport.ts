import type { ChatMessage } from "../api";
import type { Session } from "../sessions/store";

export type ThreadExportFormat = "json" | "markdown";

export function chatExportFilename(title: string, format: ThreadExportFormat = "json"): string {
  const base = title.trim().replace(/[<>:"/\\|?*\x00-\x1f]+/g, "-").replace(/\s+/g, " ").slice(0, 80).trim();
  return `${base || "milim-chat"}${format === "markdown" ? ".md" : ".milim-chat.json"}`;
}

export function sessionExportPayload(session: Session, exportedAt = new Date().toISOString()) {
  return {
    version: 1,
    exportedAt,
    session: {
      title: session.title,
      messages: session.messages,
      settings: session.settings,
      createdAt: session.createdAt,
      updatedAt: session.updatedAt,
    },
  };
}

export function exportedSessionCandidate(value: unknown): Partial<Session> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const raw = value as { session?: unknown };
  const candidate = raw.session && typeof raw.session === "object" && !Array.isArray(raw.session) ? raw.session : value;
  return candidate && typeof candidate === "object" && !Array.isArray(candidate) ? candidate as Partial<Session> : null;
}

function messageText(message: ChatMessage): string {
  if (message.content) return message.content;
  return (message.streamParts ?? []).filter((part) => part.kind === "text").map((part) => part.content).join("");
}

function roleLabel(role: string): string {
  return role ? role.charAt(0).toUpperCase() + role.slice(1) : "Message";
}

function markerRole(role: string): string {
  return role.replace(/[^a-zA-Z0-9_-]+/g, "-") || "message";
}

export function sessionMarkdownExport(session: Session, exportedAt = new Date().toISOString()): string {
  const lines = [
    "<!-- milim-thread:v1 -->",
    `# ${session.title.trim() || "milim thread"}`,
    "",
    `Exported: ${exportedAt}`,
    "",
  ];
  for (const message of session.messages) {
    lines.push(`<!-- milim-message:${markerRole(message.role)} -->`, `## ${roleLabel(message.role)}`, "", messageText(message), "");
  }
  return `${lines.join("\n").trimEnd()}\n`;
}

export function markdownSessionCandidate(markdown: string): Partial<Session> | null {
  if (!markdown.includes("<!-- milim-thread:v1 -->")) return null;
  const title = markdown.match(/^#\s+(.+)$/m)?.[1]?.trim() || "Imported chat";
  const marker = /^<!-- milim-message:([^>\r\n]+) -->\r?\n(?:## [^\r\n]*\r?\n)?/gm;
  const matches = Array.from(markdown.matchAll(marker));
  const messages = matches.map((match, index) => {
    const start = (match.index ?? 0) + match[0].length;
    const end = matches[index + 1]?.index ?? markdown.length;
    return { role: match[1], content: markdown.slice(start, end).trim() };
  }).filter((message) => message.content.length > 0);
  return messages.length ? { title, messages } : null;
}
