import { shortcutLabel, type AppShortcutAction, type AppShortcuts } from "../ui/shortcuts.js";
import type { CommandPaletteItem } from "./commandPalette.js";
import { MANAGERS, type ManagerId } from "./managers.js";
import { SLASH_COMMANDS } from "./slashCommands.js";

export interface RegistryCommand extends CommandPaletteItem {
  run: () => void;
}

export interface CommandRegistryState {
  busy: boolean;
  inTauri: boolean;
  /** Thread navigation uses the left Sidebar placement. */
  sidebarPlacement: boolean;
  sidebarOpen: boolean;
  threadCount: number;
  /** The active chat can be archived, branched, exported, or renamed. */
  threadActionsAvailable: boolean;
  /** Inbox organization archives only settled chats, matching the sidebar menu. */
  archiveAvailable: boolean;
  gitAvailable: boolean;
  gitOpen: boolean;
  previewOpen: boolean;
  codeAvailable: boolean;
  codeOpen: boolean;
  planMode: boolean;
  darkTheme: boolean;
}

export interface CommandRegistryActions {
  newChat: () => void;
  focusComposer: () => void;
  openComposerSuggestions: () => void;
  toggleSidebar: () => void;
  previousThread: () => void;
  stopGeneration: () => void;
  openSettings: () => void;
  openDiagnostics: () => void;
  openManager: (id: ManagerId) => void;
  openModelPicker: () => void;
  toggleGitPanel: () => void;
  togglePreviewPanel: () => void;
  toggleCodePanel: () => void;
  togglePlanMode: () => void;
  toggleTheme: () => void;
  renameThread: () => void;
  branchThread: () => void;
  exportThread: () => void;
  archiveThread: () => void;
  /** Runs a slash command exactly as the composer would. */
  runSlashCommand: (id: string, argument?: string) => void;
  /** Places `/<id> ` in the composer so the user can type the argument. */
  prefillSlashCommand: (id: string) => void;
}

/**
 * Slash commands already covered by a dedicated palette command. Their
 * `/name` stays searchable through that command's keywords and detail.
 */
const SLASH_COMMANDS_WITH_DEDICATED_ITEMS = new Set(["plan", "privacy", "approval", "export", "clear"]);

export function buildCommandRegistry({
  shortcuts,
  state,
  actions,
  mac,
}: {
  shortcuts: AppShortcuts;
  state: CommandRegistryState;
  actions: CommandRegistryActions;
  mac?: boolean;
}): RegistryCommand[] {
  const hint = (action: AppShortcutAction) => shortcutLabel(shortcuts[action], mac) || undefined;

  const app: RegistryCommand[] = [
    { id: "chat.new", label: "New chat", keywords: ["thread", "conversation", "/clear"], shortcut: hint("newChat"), featured: true, run: actions.newChat },
    { id: "composer.focus", label: "Focus composer", keywords: ["prompt", "input"], shortcut: hint("focusComposer"), featured: true, run: actions.focusComposer },
    { id: "composer.suggestions", label: "Open composer suggestions", keywords: ["autocomplete", "commands", "skills", "files"], shortcut: hint("openComposerSuggestions"), featured: true, run: actions.openComposerSuggestions },
    { id: "model.choose", label: "Choose model...", keywords: ["model picker", "switch model", "runtime", "provider"], shortcut: hint("openModelPicker"), featured: true, run: actions.openModelPicker },
    {
      id: "sidebar.toggle",
      label: state.sidebarOpen ? "Hide sidebar" : "Show sidebar",
      keywords: ["toggle", "navigation"],
      shortcut: hint("toggleSidebar"),
      available: state.sidebarPlacement,
      featured: true,
      run: actions.toggleSidebar,
    },
    { id: "thread.previous", label: "Previous thread", keywords: ["chat", "recent", "switch"], shortcut: hint("previousThread"), available: state.threadCount > 1, featured: true, run: actions.previousThread },
    { id: "generation.stop", label: "Stop generation", keywords: ["cancel", "abort"], shortcut: hint("stopGeneration"), available: state.busy, featured: true, run: actions.stopGeneration },
    { id: "settings.open", label: "Open settings", keywords: ["preferences", "configuration"], shortcut: hint("openSettings"), featured: true, run: actions.openSettings },
    {
      id: "theme.toggle",
      label: state.darkTheme ? "Switch to light theme" : "Switch to dark theme",
      keywords: ["theme", "appearance", "dark", "light", "mode"],
      run: actions.toggleTheme,
    },
    { id: "diagnostics.open", label: "Open diagnostics", keywords: ["logs", "recovery", "debug"], available: state.inTauri, run: actions.openDiagnostics },
  ];

  const managers: RegistryCommand[] = MANAGERS.map((entry) => ({
    id: `manager.${entry.id}`,
    label: `Open ${entry.label}`,
    keywords: ["manager", "tools", "manage", ...entry.keywords],
    detail: "Tools",
    run: () => actions.openManager(entry.id),
  }));

  const panels: RegistryCommand[] = [
    {
      id: "panel.git",
      label: state.gitOpen ? "Close Git panel" : "Open Git panel",
      keywords: ["toggle", "diff", "changes", "review", "commit", "inspector"],
      shortcut: hint("toggleGitPanel"),
      available: state.gitAvailable || state.gitOpen,
      run: actions.toggleGitPanel,
    },
    {
      id: "panel.preview",
      label: state.previewOpen ? "Close Preview panel" : "Open Preview panel",
      keywords: ["toggle", "browser", "app", "url", "artifact", "inspector"],
      shortcut: hint("togglePreviewPanel"),
      run: actions.togglePreviewPanel,
    },
    {
      id: "panel.code",
      label: state.codeOpen ? "Close Code panel" : "Open Code panel",
      keywords: ["toggle", "editor", "files", "workspace", "inspector"],
      available: state.codeAvailable || state.codeOpen,
      run: actions.toggleCodePanel,
    },
  ];

  const session: RegistryCommand[] = [
    {
      id: "plan.toggle",
      label: state.planMode ? "Turn off Plan mode" : "Turn on Plan mode",
      keywords: ["plan", "read-only", "planning", "/plan"],
      shortcut: hint("togglePlanMode"),
      detail: "/plan",
      run: actions.togglePlanMode,
    },
    ...(["off", "redact", "block"] as const).map((mode): RegistryCommand => ({
      id: `privacy.${mode}`,
      label: `Privacy: ${mode[0].toUpperCase()}${mode.slice(1)}`,
      keywords: ["privacy gate", "pii", "secrets", "/privacy"],
      detail: `/privacy ${mode}`,
      run: () => actions.runSlashCommand("privacy", mode),
    })),
    ...(["review", "guarded", "open"] as const).map((mode): RegistryCommand => ({
      id: `approval.${mode}`,
      label: `Approval: ${mode[0].toUpperCase()}${mode.slice(1)}`,
      keywords: ["tool approval", "permissions", "/approval"],
      detail: `/approval ${mode}`,
      run: () => actions.runSlashCommand("approval", mode),
    })),
  ];

  const thread: RegistryCommand[] = [
    { id: "thread.rename", label: "Rename chat...", keywords: ["thread", "title"], available: state.threadActionsAvailable, run: actions.renameThread },
    { id: "thread.branch", label: "Branch chat", keywords: ["thread", "fork", "copy"], available: state.threadActionsAvailable, run: actions.branchThread },
    { id: "thread.export", label: "Export chat", keywords: ["thread", "download", "json", "markdown", "/export"], detail: "/export", available: state.threadActionsAvailable, run: actions.exportThread },
    { id: "thread.archive", label: "Archive chat...", keywords: ["thread", "hide", "remove"], shortcut: hint("archiveThread"), available: state.archiveAvailable, run: actions.archiveThread },
  ];

  const slash: RegistryCommand[] = SLASH_COMMANDS
    .filter((command) => !SLASH_COMMANDS_WITH_DEDICATED_ITEMS.has(command.id))
    .map((command) => ({
      id: `slash.${command.id}`,
      label: command.palette === "prefill" ? `${command.label}...` : command.label,
      keywords: [`/${command.id}`, command.id, command.hint, "slash command"],
      detail: `/${command.id}`,
      run: command.palette === "prefill"
        ? () => actions.prefillSlashCommand(command.id)
        : () => actions.runSlashCommand(command.id),
    }));

  return [...app, ...managers, ...panels, ...session, ...thread, ...slash];
}
