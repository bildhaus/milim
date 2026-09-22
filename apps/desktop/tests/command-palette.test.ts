import { commandPaletteResults, filterCommandPaletteItems } from "../src/lib/commandPalette.js";
import {
  buildCommandRegistry,
  type CommandRegistryActions,
  type CommandRegistryState,
} from "../src/lib/commandRegistry.js";
import {
  isManagerId,
  managerEntry,
  managerIdFromEvent,
  managersByGroup,
  matchingManagers,
  MANAGERS,
} from "../src/lib/managers.js";
import { SLASH_COMMANDS } from "../src/lib/slashCommands.js";
import { DEFAULT_APP_SHORTCUTS } from "../src/ui/shortcuts.js";

function equal(actual: unknown, expected: unknown, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const items = [
  { id: "chat.new", label: "New chat", keywords: ["thread"] },
  { id: "settings.open", label: "Open settings", keywords: ["preferences"] },
  { id: "generation.stop", label: "Stop generation", keywords: ["cancel"], available: false },
];

equal(
  filterCommandPaletteItems(items, "").map((item) => item.id).join(","),
  "chat.new,settings.open",
  "empty query should preserve available command order",
);
equal(
  filterCommandPaletteItems(items, "thread")[0]?.id,
  "chat.new",
  "keywords should match",
);
equal(
  filterCommandPaletteItems(items, "open pref")[0]?.id,
  "settings.open",
  "all query terms should match label and keywords",
);
equal(
  filterCommandPaletteItems(items, "cancel").length,
  0,
  "unavailable commands should stay hidden",
);
equal(
  filterCommandPaletteItems(
    [
      { id: "keyword", label: "Open settings", keywords: ["chat"] },
      { id: "label", label: "New chat" },
    ],
    "chat",
  ).map((item) => item.id).join(","),
  "label,keyword",
  "label matches should rank ahead of keyword-only matches",
);
equal(
  commandPaletteResults(
    [
      { id: "featured", label: "New chat", featured: true },
      { id: "hidden", label: "Open Skills" },
    ],
    "",
  ).map((item) => item.id).join(","),
  "featured",
  "an empty query should show featured commands before chats",
);
equal(
  commandPaletteResults(
    [
      { id: "featured", label: "New chat", featured: true },
      { id: "hidden", label: "Open Skills" },
    ],
    "skills",
  )[0]?.id,
  "hidden",
  "typing should search the whole registry",
);

// Managers registry
const expectedManagers = ["providers", "agents", "memory", "mcp", "skills", "schedules", "media", "pull-requests", "google-workspace", "mobile"];
equal(MANAGERS.map((entry) => entry.id).join(","), expectedManagers.join(","), "every manager should be registered once in hub order");
equal(
  managersByGroup().flatMap((group) => group.managers).length,
  MANAGERS.length,
  "every manager should belong to a rendered group",
);
equal(matchingManagers("API key")[0]?.id, "providers", "API key should find Providers");
equal(matchingManagers("ollama")[0]?.id, "providers", "local runtime names should find Providers");
equal(matchingManagers("cron")[0]?.id, "schedules", "cron should find Schedules");
equal(managerEntry("mobile").settingsSection, "mobile", "Mobile should open its Settings section");
equal(managerEntry("google-workspace").settingsSection, "google", "Google Workspace should open its Settings section");
equal(isManagerId("usage-typo"), false, "unknown manager ids should be rejected");
equal(
  managerIdFromEvent(new CustomEvent("milim:open-manager", { detail: "agents" })),
  "agents",
  "manager events should carry the manager id",
);

// Command registry
const calls: string[] = [];
const record = (name: string) => () => {
  calls.push(name);
};
const actions: CommandRegistryActions = {
  newChat: record("newChat"),
  focusComposer: record("focusComposer"),
  openComposerSuggestions: record("openComposerSuggestions"),
  toggleSidebar: record("toggleSidebar"),
  previousThread: record("previousThread"),
  stopGeneration: record("stopGeneration"),
  openSettings: record("openSettings"),
  openDiagnostics: record("openDiagnostics"),
  openManager: (id) => calls.push(`manager:${id}`),
  openModelPicker: record("openModelPicker"),
  toggleGitPanel: record("toggleGitPanel"),
  togglePreviewPanel: record("togglePreviewPanel"),
  toggleCodePanel: record("toggleCodePanel"),
  togglePlanMode: record("togglePlanMode"),
  toggleTheme: record("toggleTheme"),
  renameThread: record("renameThread"),
  branchThread: record("branchThread"),
  exportThread: record("exportThread"),
  archiveThread: record("archiveThread"),
  runSlashCommand: (id, argument) => calls.push(`slash:${id}${argument ? ` ${argument}` : ""}`),
  prefillSlashCommand: (id) => calls.push(`prefill:${id}`),
};
const state: CommandRegistryState = {
  busy: false,
  inTauri: true,
  sidebarPlacement: true,
  sidebarOpen: true,
  threadCount: 3,
  threadActionsAvailable: true,
  archiveAvailable: true,
  gitAvailable: true,
  gitOpen: false,
  previewOpen: false,
  codeAvailable: true,
  codeOpen: false,
  planMode: false,
  darkTheme: true,
};
const registry = buildCommandRegistry({ shortcuts: DEFAULT_APP_SHORTCUTS, state, actions, mac: true });
const byId = new Map(registry.map((command) => [command.id, command]));

equal(byId.size, registry.length, "command ids should be unique");
for (const entry of MANAGERS) {
  const command = byId.get(`manager.${entry.id}`);
  equal(Boolean(command), true, `${entry.label} should be in the palette`);
  command?.run();
  equal(calls.at(-1), `manager:${entry.id}`, `${entry.label} should open its manager`);
}
for (const id of ["panel.git", "panel.preview", "panel.code", "thread.archive", "thread.branch", "thread.export", "thread.rename", "model.choose", "plan.toggle", "theme.toggle"]) {
  equal(Boolean(byId.get(id)), true, `${id} should be registered`);
}
for (const command of SLASH_COMMANDS) {
  equal(
    filterCommandPaletteItems(registry, `/${command.id}`).length > 0,
    true,
    `/${command.id} should be reachable from the palette`,
  );
}

equal(byId.get("settings.open")?.shortcut, "Cmd+,", "settings should show its configured shortcut");
equal(byId.get("model.choose")?.shortcut, "Cmd+Shift+M", "model picker should show its shortcut");
equal(byId.get("panel.git")?.shortcut, "Cmd+Shift+G", "Git toggle should show its shortcut");
equal(byId.get("panel.preview")?.shortcut, "Cmd+Shift+O", "Preview toggle should show its shortcut");
equal(byId.get("plan.toggle")?.shortcut, "Cmd+Shift+P", "Plan toggle should show its shortcut");
equal(byId.get("thread.archive")?.shortcut, "Cmd+Shift+A", "archive should show its shortcut");
equal(byId.get("chat.new")?.shortcut, "Cmd+N", "existing shortcut hints should remain");
equal(byId.get("plan.toggle")?.label, "Turn on Plan mode", "plan label should reflect the current state");
equal(byId.get("theme.toggle")?.label, "Switch to light theme", "theme label should name the destination");
equal(byId.get("generation.stop")?.available, false, "stop should be hidden while idle");

byId.get("privacy.redact")?.run();
equal(calls.at(-1), "slash:privacy redact", "privacy choices should run the privacy slash command");
byId.get("slash.compact")?.run();
equal(calls.at(-1), "slash:compact", "argument-free slash commands should run directly");
byId.get("slash.agent")?.run();
equal(calls.at(-1), "prefill:agent", "slash commands that need an argument should prefill the composer");

const unbound = buildCommandRegistry({
  shortcuts: { ...DEFAULT_APP_SHORTCUTS, archiveThread: "" },
  state: { ...state, threadActionsAvailable: false, archiveAvailable: false, gitAvailable: false, planMode: true },
  actions,
  mac: false,
});
const unboundById = new Map(unbound.map((command) => [command.id, command]));
equal(unboundById.get("thread.archive")?.shortcut, undefined, "unbound shortcuts should not render a hint");
equal(unboundById.get("thread.archive")?.available, false, "thread actions should hide without an active chat");
equal(unboundById.get("panel.git")?.available, false, "Git should hide outside a repository");
equal(unboundById.get("plan.toggle")?.label, "Turn off Plan mode", "plan label should offer turning it off");
equal(unboundById.get("chat.new")?.shortcut, "Ctrl+N", "Windows hints should use Ctrl");

export {};
