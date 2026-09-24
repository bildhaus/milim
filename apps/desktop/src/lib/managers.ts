import type { SettingsSectionId } from "../settings/search";

/**
 * Every workbench manager reachable from the sidebar Tools launcher, the
 * command palette, and Settings search. Secondary entry points (model picker
 * footer, session controls, composer agent menu) keep opening the same
 * managers directly.
 */
export type ManagerId =
  | "providers"
  | "agents"
  | "memory"
  | "mcp"
  | "skills"
  | "schedules"
  | "media"
  | "pull-requests"
  | "usage"
  | "google-workspace"
  | "mobile";

export interface ManagerEntry {
  id: ManagerId;
  label: string;
  description: string;
  keywords: string[];
  /** Managers that live inside Settings open that section instead of a sheet. */
  settingsSection?: SettingsSectionId;
}

export const MANAGERS: readonly ManagerEntry[] = [
  {
    id: "providers",
    label: "Providers",
    description: "Hosted, local, and account-runtime connections",
    keywords: ["api key", "keys", "models", "provider", "openai", "anthropic", "openrouter", "gemini", "groq", "ollama", "lm studio", "vllm", "local", "codex", "claude", "opencode", "pi", "account runtimes", "cli", "sign in", "login", "connect"],
  },
  {
    id: "agents",
    label: "Agents",
    description: "Named personas with their own model, tools, and skills",
    keywords: ["agent", "persona", "profile", "system prompt", "tools", "workers"],
  },
  {
    id: "memory",
    label: "Memory",
    description: "Personal and project memory",
    keywords: ["memories", "remember", "personal", "project", "notes"],
  },
  {
    id: "mcp",
    label: "MCP Servers",
    description: "Model Context Protocol servers and Apps",
    keywords: ["mcp", "servers", "tools", "apps", "connectors", "integrations"],
  },
  {
    id: "skills",
    label: "Skills",
    description: "Reusable instructions loaded on demand",
    keywords: ["skill", "instructions", "prompts"],
  },
  {
    id: "schedules",
    label: "Schedules",
    description: "Recurring prompts and scheduled runs",
    keywords: ["schedule", "cron", "recurring", "automation", "timer"],
  },
  {
    id: "media",
    label: "Media",
    description: "Image, video, and music generation",
    keywords: ["media studio", "image", "video", "music", "generate", "replicate", "fal"],
  },
  {
    id: "pull-requests",
    label: "Pull requests",
    description: "GitHub pull requests authored by you or awaiting your review",
    keywords: ["github", "pr", "prs", "review", "pull request"],
  },
  {
    id: "usage",
    label: "Usage",
    description: "Tokens and spend by day, model, provider, and project",
    keywords: ["usage", "cost", "spend", "tokens", "billing", "budget", "dashboard", "metrics"],
  },
  {
    id: "google-workspace",
    label: "Google Workspace",
    description: "Drive, Docs, Sheets, and Slides access",
    keywords: ["google", "drive", "docs", "sheets", "slides", "oauth"],
    settingsSection: "google",
  },
  {
    id: "mobile",
    label: "Mobile",
    description: "Native phone pairing and direct control",
    keywords: ["phone", "companion", "pairing", "qr", "tailscale", "ios", "android"],
    settingsSection: "mobile",
  },
];

const MANAGER_BY_ID = new Map(MANAGERS.map((entry) => [entry.id, entry]));

export function managerEntry(id: ManagerId): ManagerEntry {
  return MANAGER_BY_ID.get(id)!;
}

export function isManagerId(value: unknown): value is ManagerId {
  return typeof value === "string" && MANAGER_BY_ID.has(value as ManagerId);
}

/**
 * The sidebar Tools menu stays short: work tools plus Extensions. Providers,
 * Agents, Memory, Google Workspace, and Mobile open from their in-context
 * entry points, the command palette, and Settings search.
 */
export const TOOLS_MENU_MANAGERS: readonly ManagerId[] = ["mcp", "skills", "schedules", "media", "pull-requests", "usage"];

export function toolsMenuManagers(): ManagerEntry[] {
  return TOOLS_MENU_MANAGERS.map(managerEntry);
}

/** Managers whose label, description, or keywords contain the whole query. */
export function matchingManagers(query: string): ManagerEntry[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return [];
  return MANAGERS.filter((entry) =>
    [entry.label, entry.description, ...entry.keywords]
      .join(" ")
      .toLowerCase()
      .includes(normalized),
  );
}

export const OPEN_MANAGER_EVENT = "milim:open-manager";

/**
 * Opens a manager from anywhere in the app. App owns the sheet managers and
 * Settings-backed managers; ChatView owns Providers and Memory because they
 * refresh its model and tool catalogs when closed.
 */
export function requestOpenManager(id: ManagerId): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent<ManagerId>(OPEN_MANAGER_EVENT, { detail: id }));
}

export function managerIdFromEvent(event: Event): ManagerId | null {
  const detail = (event as CustomEvent<unknown>).detail;
  return isManagerId(detail) ? detail : null;
}
