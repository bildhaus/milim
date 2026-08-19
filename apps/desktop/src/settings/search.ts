export type SettingsSectionId = "app" | "chat" | "appearance" | "models" | "workspace" | "history" | "google" | "mobile" | "system" | "about" | "developer";

export type SettingSearchEntry = {
  id: string;
  label: string;
  section: SettingsSectionId;
  aliases?: string[];
};

export const SETTINGS_SEARCH_ENTRIES: SettingSearchEntry[] = [
  { id: "app-window-layout", label: "Window and layout", section: "app", aliases: ["app", "general", "always on top", "ui size", "zoom", "account usage", "account quota", "quota", "codex", "claude", "new chat"] },
  { id: "app-thread-navigation", label: "Thread navigation", section: "app", aliases: ["sidebar", "sidebar organization", "thread organization", "top bar", "bottom bar", "placement", "projects", "inbox", "settle", "settled", "finished threads", "recent activity", "folders", "organization"] },
  { id: "chat-composer", label: "Composer", section: "chat", aliases: ["send shortcut", "enter", "density"] },
  { id: "chat-threads", label: "Threads", section: "chat", aliases: ["auto title", "ai names", "naming model"] },
  { id: "chat-new-thread", label: "New-chat behavior", section: "chat", aliases: ["inherit", "configured defaults", "memory", "privacy", "sandbox", "approval", "delegation"] },
  { id: "chat-quick-actions", label: "Quick actions", section: "chat", aliases: ["smart", "pinned prompts", "starter suggestions", "project overrides"] },
  { id: "chat-autocomplete", label: "Autocomplete", section: "chat", aliases: ["suggestions", "commands", "files", "skills", "mcp", "personalized ranking"] },
  { id: "chat-prompt-history", label: "Prompt history", section: "chat", aliases: ["current chat", "across chats", "clear history"] },
  { id: "chat-ai-completion", label: "AI composer completion", section: "chat", aliases: ["ghost text", "local provider", "privacy", "tab"] },
  { id: "appearance-theme", label: "Theme", section: "appearance", aliases: ["custom", "edit", "delete", "palette"] },
  { id: "appearance-chat-surface", label: "Chat surface", section: "appearance", aliases: ["layout", "message width", "avatars"] },
  { id: "appearance-sidebar-colors", label: "Sidebar appearance", section: "appearance", aliases: ["collapsed rail", "regular rail", "split rail", "centered rail", "thread colors", "thread names", "automatic color", "icons"] },
  { id: "appearance-empty-chat-ridgeline", label: "Empty-chat ridgeline", section: "appearance", aliases: ["usage", "activity", "chart", "composer", "empty thread"] },
  { id: "appearance-code-blocks", label: "Code blocks", section: "appearance", aliases: ["theme", "syntax"] },
  { id: "appearance-interface-sounds", label: "Interface sounds", section: "appearance", aliases: ["sound", "audio", "feedback", "cuelume", "finished", "completion", "attention", "approval", "alert"] },
  { id: "appearance-background", label: "Background image", section: "appearance", aliases: ["fit", "treatment"] },
  { id: "models-defaults", label: "Model and agent defaults", section: "models", aliases: ["chat model", "worker model", "agent", "favorite", "fallback", "unavailable"] },
  { id: "workspace-opener", label: "Workspace opener", section: "workspace", aliases: ["launcher", "vscode", "zed", "terminal", "remember per project"] },
  { id: "workspace-new-chat", label: "New project-chat workspace", section: "workspace", aliases: ["checkout", "ask", "isolated worktree"] },
  { id: "history-retention", label: "Archive retention", section: "history", aliases: ["delete", "purge", "7 days", "14 days", "30 days"] },
  { id: "history-projects", label: "Archived projects", section: "history", aliases: ["restore", "delete"] },
  { id: "history-chats", label: "Archived chats", section: "history", aliases: ["threads", "restore", "delete"] },
  { id: "google-workspace", label: "Google Workspace", section: "google", aliases: ["drive", "docs", "sheets", "slides", "picker", "oauth", "selected files"] },
  { id: "browser-data", label: "Browser data", section: "history", aliases: ["cookies", "sign in", "credentials", "private", "persistent", "clear"] },
  { id: "data-export", label: "Thread export defaults", section: "history", aliases: ["json", "markdown"] },
  { id: "data-backup", label: "Backup and restore", section: "history", aliases: ["export backup", "restore backup", "recovery snapshot"] },
  { id: "app-startup", label: "Startup behavior", section: "app", aliases: ["restore last chat", "new chat", "open panels"] },
  { id: "app-notifications", label: "Notifications", section: "app", aliases: ["run finished", "needs attention", "thread title", "unfocused"] },
  { id: "app-update-policy", label: "Update policy", section: "app", aliases: ["automatic check", "automatic download"] },
  { id: "mobile-companion", label: "Mobile companion", section: "mobile", aliases: ["phone", "pairing", "qr", "tailscale"] },
  { id: "system-secret-storage", label: "Credential storage", section: "system", aliases: ["keychain", "credential manager", "secrets", "encryption", "fallback"] },
  { id: "system-shortcuts", label: "Keyboard shortcuts", section: "system", aliases: ["hotkey", "command", "reset"] },
  { id: "about-version", label: "Version", section: "about", aliases: ["current", "latest"] },
  { id: "about-updates", label: "Updates", section: "about", aliases: ["github release", "download", "restart"] },
  { id: "about-diagnostics", label: "Diagnostics", section: "about", aliases: ["logs", "recovery", "debug"] },
  { id: "developer-mode", label: "Developer mode", section: "developer", aliases: ["debug", "experimental", "onboarding"] },
  { id: "developer-update-cards", label: "Update cards preview", section: "developer", aliases: ["release notes", "what's new", "debug", "replay"] },
];

export function matchingSettingsEntries(query: string): SettingSearchEntry[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return [];
  return SETTINGS_SEARCH_ENTRIES.filter((entry) =>
    [entry.label, entry.section, ...(entry.aliases ?? [])]
      .join(" ")
      .toLowerCase()
      .includes(normalized),
  );
}
