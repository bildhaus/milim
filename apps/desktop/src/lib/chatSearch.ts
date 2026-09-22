import { folderLabel } from "./projectColors.js";

export interface SearchableChatMessage {
  role?: string;
  content: string;
}

export interface SearchableChatSession {
  id: string;
  title: string;
  updatedAt: number;
  archivedAt?: number;
  settings?: {
    folder?: string;
    model?: string;
  };
  messages: SearchableChatMessage[];
}

export interface SearchableChatProject {
  name: string;
  folder: string;
  archivedAt?: number;
}

export interface ChatSearchResult {
  sessionId: string;
  title: string;
  metadata: string;
  snippet: string;
  updatedAt: number;
  score: number;
}

const DEFAULT_LIMIT = 20;
const MESSAGE_CHAR_BUDGET = 24_000;
const DAY_MS = 24 * 60 * 60 * 1000;

type SearchArchiveMode = "active" | "all" | "archived";

type ParsedChatSearchQuery = {
  text: string;
  role?: "user" | "assistant";
  archiveMode: SearchArchiveMode;
};

export function tokenizeChatSearchQuery(query: string): string[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return [];
  const terms = normalized.match(/[a-z0-9]+/g) ?? [];
  const allowSingle = normalized.length === 1;
  const filtered = terms.filter((term) => allowSingle || term.length > 1);
  return Array.from(new Set(filtered));
}

export function chatSearchSnippet(text: string, terms: readonly string[], query: string, max = 180): string {
  const clean = compactText(text);
  if (!clean) return "";
  const lower = clean.toLowerCase();
  const needle = query.trim().toLowerCase();
  let hit = needle.length > 1 ? lower.indexOf(needle) : -1;
  if (hit < 0) {
    for (const term of terms) {
      hit = lower.indexOf(term);
      if (hit >= 0) break;
    }
  }
  if (hit < 0) hit = 0;

  const context = Math.floor(max * 0.35);
  const start = Math.max(0, hit - context);
  const end = Math.min(clean.length, start + max);
  const body = clean.slice(start, end).trim();
  return `${start > 0 ? "..." : ""}${body}${end < clean.length ? "..." : ""}`;
}

export function searchChatSessions(
  sessions: readonly SearchableChatSession[],
  projects: readonly SearchableChatProject[],
  query: string,
  limit = DEFAULT_LIMIT,
  now = Date.now(),
): ChatSearchResult[] {
  const parsed = parseChatSearchQuery(query);
  const activeProjects = new Map(projects.filter((project) => !project.archivedAt).map((project) => [project.folder, project.name]));
  const searchable = sessions.filter((session) => {
    if (parsed.archiveMode === "all") return true;
    if (parsed.archiveMode === "archived") return Boolean(session.archivedAt);
    return !session.archivedAt;
  });
  const terms = tokenizeChatSearchQuery(parsed.text);
  const normalizedQuery = parsed.text.trim().toLowerCase();

  if (!terms.length) {
    return searchable
      .slice()
      .sort((a, b) => b.updatedAt - a.updatedAt)
      .slice(0, limit)
      .map((session) => buildResult(session, activeProjects, latestMessageSnippet(session.messages, parsed.role), 0));
  }

  return searchable
    .map((session) => scoreSession(session, activeProjects, terms, normalizedQuery, now, parsed.role))
    .filter((result): result is ChatSearchResult => result !== null)
    .sort((a, b) => b.score - a.score || b.updatedAt - a.updatedAt)
    .slice(0, limit);
}

function parseChatSearchQuery(query: string): ParsedChatSearchQuery {
  let role: ParsedChatSearchQuery["role"];
  let archiveMode: SearchArchiveMode = "active";
  const text = query
    .split(/\s+/)
    .filter((part) => {
      const token = part.toLowerCase();
      if (token === "from:user" || token === "role:user") {
        role = "user";
        return false;
      }
      if (token === "from:assistant" || token === "role:assistant") {
        role = "assistant";
        return false;
      }
      if (token === "in:all") {
        archiveMode = "all";
        return false;
      }
      if (token === "is:archived" || token === "in:archive" || token === "in:archived") {
        archiveMode = "archived";
        return false;
      }
      return true;
    })
    .join(" ");
  return { text, role, archiveMode };
}

function scoreSession(
  session: SearchableChatSession,
  projectByFolder: ReadonlyMap<string, string>,
  terms: readonly string[],
  query: string,
  now: number,
  role?: "user" | "assistant",
): ChatSearchResult | null {
  const title = compactText(session.title) || "Untitled chat";
  const titleLower = title.toLowerCase();
  const folder = session.settings?.folder?.trim() ?? "";
  const projectName = folder ? projectByFolder.get(folder) ?? "" : "";
  const metadataText = [projectName, folderLabel(folder, ""), folder, session.settings?.model].filter(Boolean).join(" ");
  const metadataLower = metadataText.toLowerCase();

  let score = 0;
  if (titleLower === query) score += 120;
  if (query && titleLower.startsWith(query)) score += 80;
  const titleMatches = countMatches(titleLower, terms);
  if (titleMatches > 0) score += titleMatches * 36;
  if (titleMatches === terms.length && terms.length > 1) score += 36;

  const metadataMatches = countMatches(metadataLower, terms);
  if (metadataMatches > 0) score += metadataMatches * 12;
  if (metadataMatches === terms.length && terms.length > 1) score += 16;

  let bestMessage: SearchableChatMessage | null = null;
  let bestMessageScore = 0;
  let scanned = 0;
  for (const message of session.messages) {
    if (role && message.role !== role) continue;
    if (scanned >= MESSAGE_CHAR_BUDGET) break;
    const remaining = MESSAGE_CHAR_BUDGET - scanned;
    const content = message.content.slice(0, remaining);
    scanned += content.length;
    const messageScore = scoreMessage(content, terms, query);
    if (messageScore > bestMessageScore) {
      bestMessageScore = messageScore;
      bestMessage = { ...message, content };
    }
  }
  score += bestMessageScore;
  if (score <= 0) return null;

  const snippet = bestMessage
    ? rolePrefix(bestMessage.role) + chatSearchSnippet(bestMessage.content, terms, query)
    : latestMessageSnippet(session.messages, role);
  const ageDays = Math.max(0, (now - session.updatedAt) / DAY_MS);
  const recencyBoost = Math.max(0, 8 - Math.log1p(ageDays) * 2);
  return buildResult(session, projectByFolder, snippet, score + recencyBoost);
}

function scoreMessage(content: string, terms: readonly string[], query: string): number {
  const lower = content.toLowerCase();
  let score = query.length > 1 && lower.includes(query) ? 24 : 0;
  const matches = countMatches(lower, terms);
  if (matches > 0) score += matches * 8;
  if (matches === terms.length && terms.length > 1) score += 24 + terms.length * 4;
  return score;
}

function buildResult(
  session: SearchableChatSession,
  projectByFolder: ReadonlyMap<string, string>,
  snippet: string,
  score: number,
): ChatSearchResult {
  const folder = session.settings?.folder?.trim() ?? "";
  const projectName = folder ? projectByFolder.get(folder) ?? "" : "";
  const projectLabel = projectName || folderLabel(folder, "");
  const metadata = [projectLabel, session.settings?.model].filter(Boolean).join(" | ");
  return {
    sessionId: session.id,
    title: compactText(session.title) || "Untitled chat",
    metadata,
    snippet,
    updatedAt: session.updatedAt,
    score,
  };
}

function latestMessageSnippet(messages: readonly SearchableChatMessage[], role?: "user" | "assistant"): string {
  const latest = messages.slice().reverse().find((message) => (!role || message.role === role) && compactText(message.content));
  return latest ? rolePrefix(latest.role) + chatSearchSnippet(latest.content, [], "") : "No messages yet";
}

function countMatches(text: string, terms: readonly string[]): number {
  if (!text) return 0;
  return terms.reduce((count, term) => count + (text.includes(term) ? 1 : 0), 0);
}

function compactText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function rolePrefix(role?: string): string {
  if (role === "user") return "You: ";
  if (role === "assistant") return "Assistant: ";
  return "";
}
