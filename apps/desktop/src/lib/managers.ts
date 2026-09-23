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

export type ManagerGroupId = "models" | "extensions" | "work" | "connections";

export interface ManagerGroup {
  id: ManagerGroupId;
  label: string;
}

export interface ManagerEntry {
  id: ManagerId;
  label: string;
  group: ManagerGroupId;
  description: string;
  keywords: string[];
  /** Managers that live inside Settings open that section instead of a sheet. */
  settingsSection?: SettingsSectionId;
}

export const MANAGER_GROUPS: readonly ManagerGroup[] = [
  { id: "models", label: "Models & agents" },
  { id: "extensions", label: "Extensions" },
  { id: "work", label: "Work" },
  { id: "connections", label: "Connections" },
];

export const MANAGERS: readonly ManagerEntry[] = [
  {
    id: "providers",
    label: "Providers",
    group: "models",
    description: "Hosted, local, and account-runtime connections",
    keywords: ["api key", "keys", "models", "provider", "openai", "anthropic", "openrouter", "gemini", "groq", "ollama", "lm studio", "vllm", "local", "codex", "claude", "opencode", "pi", "account runtimes", "cli", "sign in", "login", "connect"],
  },
  {
    id: "agents",
    label: "Agents",
    group: "models",
    description: "Named personas with their own model, tools, and skills",
    keywords: ["agent", "persona", "profile", "system prompt", "tools", "workers"],
  },
  {
    id: "memory",
    label: "Memory",
    group: "models",
    description: "Personal and project memory",
    keywords: ["memories", "remember", "personal", "project", "notes"],
  },
  {
    id: "mcp",
    label: "MCP Servers",
    group: "extensions",
    description: "Model Context Protocol servers and Apps",
    keywords: ["mcp", "servers", "tools", "apps", "connectors", "integrations"],
  },
  {
    id: "skills",
    label: "Skills",
    group: "extensions",
    description: "Reusable instructions loaded on demand",
    keywords: ["skill", "instructions", "prompts"],
  },
  {
    id: "schedules",
    label: "Schedules",
    group: "work",
    description: "Recurring prompts and scheduled runs",
    keywords: ["schedule", "cron", "recurring", "automation", "timer"],
  },
  {
    id: "media",
    label: "Media",
    group: "work",
    description: "Image, video, and music generation",
    keywords: ["media studio", "image", "video", "music", "generate", "replicate", "fal"],
  },
  {
    id: "pull-requests",
    label: "Pull requests",
    group: "work",
    description: "GitHub pull requests authored by you or awaiting your review",
    keywords: ["github", "pr", "prs", "review", "pull request"],
  },
  {
    id: "usage",
    label: "Usage",
    group: "work",
    description: "Tokens and spend by day, model, provider, and project",
    keywords: ["usage", "cost", "spend", "tokens", "billing", "budget", "dashboard", "metrics"],
  },
  {
    id: "google-workspace",
    label: "Google Workspace",
    group: "connections",
    description: "Drive, Docs, Sheets, and Slides access",
    keywords: ["google", "drive", "docs", "sheets", "slides", "oauth"],
    settingsSection: "google",
  },
  {
    id: "mobile",
    label: "Mobile",
    group: "connections",
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

export function managersByGroup(): Array<ManagerGroup & { managers: ManagerEntry[] }> {
  return MANAGER_GROUPS.map((group) => ({
    ...group,
    managers: MANAGERS.filter((entry) => entry.group === group.id),
  })).filter((group) => group.managers.length > 0);
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
