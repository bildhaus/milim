class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function persistedUiState(): Record<string, unknown> {
  const raw = localStorage.getItem("milim.ui");
  if (!raw) throw new Error("milim.ui was not persisted");
  const parsed = JSON.parse(raw) as { state?: Record<string, unknown> };
  return parsed.state ?? {};
}

const { DEFAULT_APP_SHORTCUTS, uiSizeShortcutDelta } = await import("../src/ui/shortcuts.js");
const {
  DEFAULT_MEDIA_COMPOSER_WIDTH,
  DEFAULT_MEDIA_LIBRARY_WIDTH,
  DEFAULT_MEDIA_STUDIO_HEIGHT,
  DEFAULT_MEDIA_STUDIO_WIDTH,
  DEFAULT_SIDEBAR_WIDTH,
  DEFAULT_UI_SIZE,
  MAX_SIDEBAR_WIDTH,
  MAX_UI_SIZE,
  MIN_MEDIA_STUDIO_HEIGHT,
  MIN_MEDIA_STUDIO_WIDTH,
  MIN_SIDEBAR_WIDTH,
  MIN_UI_SIZE,
  UI_SIZE_STEP,
  useUiPreferences,
} = await import("../src/ui/store.js");

equal(useUiPreferences.getState().sidebarOpen, true, "sidebar should default open");
equal(useUiPreferences.getState().sidebarWidth, DEFAULT_SIDEBAR_WIDTH, "sidebar should have a default width");
equal(useUiPreferences.getState().previewPanelWidth, 420, "preview panel should have a default width");
equal(useUiPreferences.getState().mediaStudioWidth, DEFAULT_MEDIA_STUDIO_WIDTH, "media studio should have a default width");
equal(useUiPreferences.getState().mediaStudioHeight, DEFAULT_MEDIA_STUDIO_HEIGHT, "media studio should have a default height");
equal(useUiPreferences.getState().mediaComposerPlacement, "bottom", "media composer should default below Output");
equal(useUiPreferences.getState().mediaComposerWidth, DEFAULT_MEDIA_COMPOSER_WIDTH, "media composer should have a default width");
equal(useUiPreferences.getState().mediaLibraryWidth, DEFAULT_MEDIA_LIBRARY_WIDTH, "media library should have a default width");
equal(useUiPreferences.getState().uiSize, DEFAULT_UI_SIZE, "UI size should default to 100%");
equal(useUiPreferences.getState().showAccountUsageInTitleBar, true, "title-bar account usage should default on");
equal(useUiPreferences.getState().windowAlwaysOnTop, false, "window always-on-top should default off");
equal(useUiPreferences.getState().interfaceSounds, false, "interface sounds should default off");
equal(useUiPreferences.getState().soundOnFinished, true, "finished sounds should default on within the disabled master setting");
equal(useUiPreferences.getState().soundOnAttention, true, "attention sounds should default on within the disabled master setting");
equal(useUiPreferences.getState().soundOnInteractions, false, "interaction sounds should default off");
equal(useUiPreferences.getState().finishedSound, "ready", "finished sounds should use ready by default");
equal(useUiPreferences.getState().attentionSound, "error", "attention sounds should use error by default");
equal(useUiPreferences.getState().composerSendShortcut, "enter", "Enter should send by default");
equal(useUiPreferences.getState().composerDensity, "comfortable", "composer should default to comfortable density");
equal(useUiPreferences.getState().autoTitleChats, true, "new chats should auto-title by default");
equal(useUiPreferences.getState().aiThreadNames, false, "AI thread names should default off");
equal(useUiPreferences.getState().aiThreadNameModel, "", "AI thread naming should default to the chat model");
equal(useUiPreferences.getState().autoColorThreadNames, false, "automatic thread colors should default off");
equal(useUiPreferences.getState().newChatButtonAtBottom, false, "new chat button should default to top");
equal("interfaceMode" in useUiPreferences.getState(), false, "obsolete interface mode should not be exposed");
equal(useUiPreferences.getState().developerMode, false, "developer mode should default off");
equal(useUiPreferences.getState().experimentalHashlinePatch, false, "hashline patching should default off");
equal(useUiPreferences.getState().chatLayoutStyle, "transcript", "chat layout should default to transcript");
equal(useUiPreferences.getState().sidebarRailStyle, "regular", "collapsed sidebar rail should default to regular");
equal(useUiPreferences.getState().settledThreadsEnabled, false, "settled threads should default off");
equal(useUiPreferences.getState().showEmptyChatRidgeline, true, "empty-chat ridgeline should default on");
equal(useUiPreferences.getState().messageWidth, "standard", "message width should default to standard");
equal(useUiPreferences.getState().avatarStyle, "none", "avatar style should default to none");
equal(useUiPreferences.getState().codeBlockTheme, "match", "code theme should default to match");
equal(useUiPreferences.getState().backgroundFit, "cover", "background fit should default to cover");
equal(useUiPreferences.getState().backgroundTreatment, "clear", "background treatment should default to clear");
equal(useUiPreferences.getState().startupBehavior, "restore", "startup should restore the last chat by default");
equal(useUiPreferences.getState().restoreOpenPanels, true, "open panels should restore by default");
equal(useUiPreferences.getState().notifyRunFinished, false, "finished notifications should default off");
equal(useUiPreferences.getState().notifyNeedsAttention, false, "attention notifications should default off");
equal(useUiPreferences.getState().notifyOnlyWhenUnfocused, true, "notifications should default to unfocused only");
equal(useUiPreferences.getState().quickActionMode, "smart", "quick actions should default smart");
equal(useUiPreferences.getState().autocompleteMode, "automatic", "autocomplete should default automatic");
equal(useUiPreferences.getState().promptHistoryScope, "thread", "prompt history should default to the current chat");
equal(useUiPreferences.getState().threadExportFormat, "json", "thread export should default JSON");
equal(useUiPreferences.getState().workspaceLauncherPreference, "remember", "workspace openers should remember per project");
equal(useUiPreferences.getState().newProjectChatWorkspace, "current", "new project chats should use the current checkout");
equal(useUiPreferences.getState().composerCompletionMode, "off", "AI composer completion should default off");
equal(useUiPreferences.getState().gitPanelExpanded, false, "git panel should default collapsed");
equal(useUiPreferences.getState().appShortcuts.newChat, DEFAULT_APP_SHORTCUTS.newChat, "new chat shortcut should default");
equal(useUiPreferences.getState().appShortcuts.focusSearch, DEFAULT_APP_SHORTCUTS.focusSearch, "search shortcut should default");
equal(useUiPreferences.getState().appShortcuts.focusComposer, DEFAULT_APP_SHORTCUTS.focusComposer, "composer focus shortcut should default");
equal(useUiPreferences.getState().appShortcuts.openComposerSuggestions, DEFAULT_APP_SHORTCUTS.openComposerSuggestions, "suggestion shortcut should default");
equal(useUiPreferences.getState().appShortcuts.stopGeneration, DEFAULT_APP_SHORTCUTS.stopGeneration, "stop shortcut should default");
equal(useUiPreferences.getState().appShortcuts.toggleSidebar, DEFAULT_APP_SHORTCUTS.toggleSidebar, "sidebar shortcut should default");
equal(useUiPreferences.getState().appShortcuts.previousThread, DEFAULT_APP_SHORTCUTS.previousThread, "previous thread shortcut should default");

useUiPreferences.getState().setSidebarOpen(false);
equal(useUiPreferences.getState().sidebarOpen, false, "sidebar should close when set");
equal(persistedUiState().sidebarOpen, false, "sidebar preference should be persisted");

useUiPreferences.getState().setSidebarWidth(320);
equal(useUiPreferences.getState().sidebarWidth, 320, "sidebar width should update");
equal(persistedUiState().sidebarWidth, 320, "sidebar width should be persisted");

useUiPreferences.getState().setSidebarWidth(9999);
equal(useUiPreferences.getState().sidebarWidth, MAX_SIDEBAR_WIDTH, "sidebar width should be capped");

useUiPreferences.getState().setSidebarWidth(1);
equal(useUiPreferences.getState().sidebarWidth, MIN_SIDEBAR_WIDTH, "sidebar width should have a floor");

useUiPreferences.getState().setPreviewPanelWidth(540);
equal(useUiPreferences.getState().previewPanelWidth, 540, "preview panel width should update");
equal(persistedUiState().previewPanelWidth, 540, "preview panel width should be persisted");

useUiPreferences.getState().setPreviewPanelWidth(9999);
equal(useUiPreferences.getState().previewPanelWidth, 9999, "preview panel width should not have a fixed cap");

useUiPreferences.getState().setMediaStudioSize(980, 680);
equal(useUiPreferences.getState().mediaStudioWidth, 980, "media studio width should update");
equal(useUiPreferences.getState().mediaStudioHeight, 680, "media studio height should update");
equal(persistedUiState().mediaStudioWidth, 980, "media studio width should be persisted");
equal(persistedUiState().mediaStudioHeight, 680, "media studio height should be persisted");

useUiPreferences.getState().setMediaComposerPlacement("side");
equal(useUiPreferences.getState().mediaComposerPlacement, "side", "media composer placement should update");
equal(persistedUiState().mediaComposerPlacement, "side", "media composer placement should be persisted");

useUiPreferences.getState().setMediaComposerWidth(336);
useUiPreferences.getState().setMediaLibraryWidth(312);
equal(persistedUiState().mediaComposerWidth, 336, "media composer width should be persisted");
equal(persistedUiState().mediaLibraryWidth, 312, "media library width should be persisted");

useUiPreferences.getState().setMediaStudioSize(1, 1);
equal(useUiPreferences.getState().mediaStudioWidth, MIN_MEDIA_STUDIO_WIDTH, "media studio width should have a floor");
equal(useUiPreferences.getState().mediaStudioHeight, MIN_MEDIA_STUDIO_HEIGHT, "media studio height should have a floor");

useUiPreferences.getState().setUiSize(120);
equal(useUiPreferences.getState().uiSize, 120, "UI size should update");
equal(persistedUiState().uiSize, 120, "UI size should be persisted");

useUiPreferences.getState().setUiSize(106);
equal(useUiPreferences.getState().uiSize, 110, "UI size should snap to 10% steps");

useUiPreferences.getState().setUiSize(9999);
equal(useUiPreferences.getState().uiSize, MAX_UI_SIZE, "UI size should be capped");

useUiPreferences.getState().setUiSize(1);
equal(useUiPreferences.getState().uiSize, MIN_UI_SIZE, "UI size should have a floor");

useUiPreferences.getState().setShowAccountUsageInTitleBar(false);
equal(useUiPreferences.getState().showAccountUsageInTitleBar, false, "title-bar account usage should update");
equal(persistedUiState().showAccountUsageInTitleBar, false, "title-bar account usage should be persisted");

useUiPreferences.getState().setUiSize(DEFAULT_UI_SIZE);
const zoomInDelta = uiSizeShortcutDelta({ key: "=", ctrlKey: true }, false);
if (zoomInDelta) useUiPreferences.getState().setUiSize(useUiPreferences.getState().uiSize + zoomInDelta * UI_SIZE_STEP);
equal(useUiPreferences.getState().uiSize, 110, "Ctrl+= should increase UI size by 10%");

const zoomOutDelta = uiSizeShortcutDelta({ key: "-", ctrlKey: true }, false);
if (zoomOutDelta) useUiPreferences.getState().setUiSize(useUiPreferences.getState().uiSize + zoomOutDelta * UI_SIZE_STEP);
equal(useUiPreferences.getState().uiSize, DEFAULT_UI_SIZE, "Ctrl+- should decrease UI size by 10%");

useUiPreferences.getState().setUiSize(MAX_UI_SIZE);
if (zoomInDelta) useUiPreferences.getState().setUiSize(useUiPreferences.getState().uiSize + zoomInDelta * UI_SIZE_STEP);
equal(useUiPreferences.getState().uiSize, MAX_UI_SIZE, "UI size shortcut should clamp at the max");

useUiPreferences.getState().setUiSize(MIN_UI_SIZE);
if (zoomOutDelta) useUiPreferences.getState().setUiSize(useUiPreferences.getState().uiSize + zoomOutDelta * UI_SIZE_STEP);
equal(useUiPreferences.getState().uiSize, MIN_UI_SIZE, "UI size shortcut should clamp at the min");

useUiPreferences.getState().setWindowAlwaysOnTop(true);
equal(useUiPreferences.getState().windowAlwaysOnTop, true, "always-on-top preference should update");
equal(localStorage.getItem("milim.window.alwaysOnTop"), "true", "always-on-top should mirror the legacy window key");

useUiPreferences.getState().setInterfaceSounds(true);
equal(useUiPreferences.getState().interfaceSounds, true, "interface sounds should update");
equal(persistedUiState().interfaceSounds, true, "interface sounds should be persisted");
useUiPreferences.getState().setSoundOnFinished(false);
useUiPreferences.getState().setSoundOnAttention(false);
useUiPreferences.getState().setSoundOnInteractions(true);
useUiPreferences.getState().setFinishedSound("bloom");
useUiPreferences.getState().setAttentionSound("droplet");
equal(persistedUiState().soundOnFinished, false, "finished sound preference should be persisted");
equal(persistedUiState().soundOnAttention, false, "attention sound preference should be persisted");
equal(persistedUiState().soundOnInteractions, true, "interaction sound preference should be persisted");
equal(persistedUiState().finishedSound, "bloom", "finished sound choice should be persisted");
equal(persistedUiState().attentionSound, "droplet", "attention sound choice should be persisted");

useUiPreferences.getState().setQuickActionMode("pinned");
useUiPreferences.getState().setPinnedQuickActions([
  { id: "one", label: "One", prompt: "First" },
  { id: "two", label: "Two", prompt: "Second" },
  { id: "three", label: "Three", prompt: "Third" },
  { id: "four", label: "Four", prompt: "Fourth" },
]);
equal(useUiPreferences.getState().pinnedQuickActions.length, 3, "pinned actions should be bounded to three");
useUiPreferences.getState().recordGlobalPrompt("same prompt");
useUiPreferences.getState().recordGlobalPrompt("same prompt");
equal(useUiPreferences.getState().globalPromptHistory.length, 1, "global prompt history should deduplicate prompts");
for (let index = 0; index < 110; index += 1) useUiPreferences.getState().recordGlobalPrompt(`prompt ${index}`);
equal(useUiPreferences.getState().globalPromptHistory.length, 100, "global prompt history should be bounded to 100");
useUiPreferences.getState().recordSuggestionUse("command:model");
equal(useUiPreferences.getState().suggestionUsage["command:model"].count, 1, "suggestion use should store bounded local counts");
useUiPreferences.getState().resetSuggestionHistory();
equal(Object.keys(useUiPreferences.getState().suggestionUsage).length, 0, "suggestion history should reset");

useUiPreferences.getState().setComposerSendShortcut("modEnter");
equal(useUiPreferences.getState().composerSendShortcut, "modEnter", "send shortcut should update");

useUiPreferences.getState().setComposerDensity("compact");
equal(useUiPreferences.getState().composerDensity, "compact", "composer density should update");

useUiPreferences.getState().setAutoTitleChats(false);
equal(useUiPreferences.getState().autoTitleChats, false, "auto-title preference should update");

useUiPreferences.getState().setAiThreadNames(true);
equal(useUiPreferences.getState().aiThreadNames, true, "AI thread names preference should update");
equal(persistedUiState().aiThreadNames, true, "AI thread names preference should be persisted");

useUiPreferences.getState().setAiThreadNameModel("  title-model  ");
equal(useUiPreferences.getState().aiThreadNameModel, "title-model", "AI thread name model should update");
equal(persistedUiState().aiThreadNameModel, "title-model", "AI thread name model should be persisted");

useUiPreferences.getState().setNewChatButtonAtBottom(true);
equal(useUiPreferences.getState().newChatButtonAtBottom, true, "new chat button placement should update");
equal(persistedUiState().newChatButtonAtBottom, true, "new chat button placement should be persisted");

useUiPreferences.getState().setDeveloperMode(true);
equal(useUiPreferences.getState().developerMode, true, "developer mode should update");

useUiPreferences.getState().setExperimentalHashlinePatch(true);
equal(useUiPreferences.getState().experimentalHashlinePatch, true, "hashline patching should update");
equal(persistedUiState().experimentalHashlinePatch, true, "hashline patching should be persisted");

useUiPreferences.getState().setChatLayoutStyle("bubbles");
equal(useUiPreferences.getState().chatLayoutStyle, "bubbles", "chat layout should update");

useUiPreferences.getState().setSidebarRailStyle("centered");
equal(useUiPreferences.getState().sidebarRailStyle, "centered", "collapsed sidebar rail style should update");
equal(persistedUiState().sidebarRailStyle, "centered", "collapsed sidebar rail style should be persisted");

useUiPreferences.getState().setSettledThreadsEnabled(true);
equal(useUiPreferences.getState().settledThreadsEnabled, true, "settled threads should update");
equal(persistedUiState().settledThreadsEnabled, true, "settled threads should be persisted");

useUiPreferences.getState().setShowEmptyChatRidgeline(false);
equal(useUiPreferences.getState().showEmptyChatRidgeline, false, "empty-chat ridgeline should update");
equal(persistedUiState().showEmptyChatRidgeline, false, "empty-chat ridgeline should be persisted");

useUiPreferences.getState().setAutoColorThreadNames(true);
equal(useUiPreferences.getState().autoColorThreadNames, true, "automatic thread colors should update");
equal(persistedUiState().autoColorThreadNames, true, "automatic thread colors should be persisted");

useUiPreferences.getState().setMessageWidth("wide");
equal(useUiPreferences.getState().messageWidth, "wide", "message width should update");

useUiPreferences.getState().setAvatarStyle("avatar");
equal(useUiPreferences.getState().avatarStyle, "avatar", "avatar style should update");
useUiPreferences.getState().setAvatarStyle("initials" as never);
equal(useUiPreferences.getState().avatarStyle, "avatar", "legacy initials preferences should migrate to avatars");

useUiPreferences.getState().setCodeBlockTheme("terminal");
equal(useUiPreferences.getState().codeBlockTheme, "terminal", "code theme should update");

useUiPreferences.getState().setBackgroundFit("tile");
equal(useUiPreferences.getState().backgroundFit, "tile", "background fit should update");

useUiPreferences.getState().setBackgroundTreatment("mono");
equal(useUiPreferences.getState().backgroundTreatment, "mono", "background treatment should update");

useUiPreferences.getState().setGitPanelExpanded(true);
equal(useUiPreferences.getState().gitPanelExpanded, true, "git panel state should update");
equal(persistedUiState().gitPanelExpanded, true, "git panel state should be persisted");

equal(useUiPreferences.getState().setAppShortcut("focusComposer", "Mod+Shift+L"), true, "app shortcut should update");
equal(useUiPreferences.getState().appShortcuts.focusComposer, "Mod+Shift+L", "custom app shortcut should be stored");
equal(
  (persistedUiState().appShortcuts as Record<string, string>).focusComposer,
  "Mod+Shift+L",
  "custom app shortcut should be persisted",
);
equal(useUiPreferences.getState().setAppShortcut("newChat", "Mod+Shift+L"), false, "duplicate app shortcut should be rejected");
equal(useUiPreferences.getState().appShortcuts.newChat, DEFAULT_APP_SHORTCUTS.newChat, "duplicate shortcut should not overwrite");
equal(useUiPreferences.getState().setAppShortcut("newChat", "n"), false, "bare letter app shortcut should be rejected");
equal(useUiPreferences.getState().appShortcuts.newChat, DEFAULT_APP_SHORTCUTS.newChat, "invalid shortcut should not overwrite");
useUiPreferences.getState().resetAppShortcuts();
equal(useUiPreferences.getState().appShortcuts.focusComposer, DEFAULT_APP_SHORTCUTS.focusComposer, "shortcut reset should restore defaults");

useUiPreferences.getState().resetLayoutWidths();
equal(useUiPreferences.getState().sidebarWidth, DEFAULT_SIDEBAR_WIDTH, "reset should restore sidebar width");
equal(useUiPreferences.getState().previewPanelWidth, 420, "reset should restore preview panel width");
equal(useUiPreferences.getState().mediaStudioWidth, DEFAULT_MEDIA_STUDIO_WIDTH, "reset should restore media studio width");
equal(useUiPreferences.getState().mediaStudioHeight, DEFAULT_MEDIA_STUDIO_HEIGHT, "reset should restore media studio height");
equal(useUiPreferences.getState().mediaComposerWidth, DEFAULT_MEDIA_COMPOSER_WIDTH, "reset should restore media composer width");
equal(useUiPreferences.getState().mediaLibraryWidth, DEFAULT_MEDIA_LIBRARY_WIDTH, "reset should restore media library width");

useUiPreferences.setState({
  sidebarOpen: true,
  sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
  previewPanelWidth: 420,
  uiSize: DEFAULT_UI_SIZE,
  showAccountUsageInTitleBar: true,
  windowAlwaysOnTop: false,
  interfaceSounds: false,
  soundOnFinished: true,
  soundOnAttention: true,
  soundOnInteractions: false,
  finishedSound: "ready",
  attentionSound: "error",
  composerSendShortcut: "enter",
  composerDensity: "comfortable",
  autoTitleChats: true,
  aiThreadNames: false,
  aiThreadNameModel: "",
  newChatButtonAtBottom: false,
  developerMode: false,
  experimentalHashlinePatch: false,
  chatLayoutStyle: "transcript",
  sidebarRailStyle: "regular",
  settledThreadsEnabled: false,
  showEmptyChatRidgeline: true,
  autoColorThreadNames: false,
  messageWidth: "standard",
  avatarStyle: "none",
  codeBlockTheme: "match",
  backgroundFit: "cover",
  backgroundTreatment: "clear",
  gitPanelExpanded: false,
  appShortcuts: { ...DEFAULT_APP_SHORTCUTS },
});
localStorage.setItem(
  "milim.ui",
  '{"state":{"sidebarOpen":false,"sidebarWidth":384,"previewPanelWidth":512,"mediaStudioWidth":1040,"mediaStudioHeight":740,"mediaComposerPlacement":"side","uiSize":130,"windowAlwaysOnTop":true,"interfaceSounds":"loud","soundOnFinished":"yes","soundOnAttention":0,"soundOnInteractions":"sometimes","finishedSound":"error","attentionSound":"ready","composerSendShortcut":"modEnter","composerDensity":"compact","autoTitleChats":false,"aiThreadNames":true,"aiThreadNameModel":"persisted-title-model","newChatButtonAtBottom":true,"interfaceMode":"workbench","developerMode":true,"experimentalHashlinePatch":true,"chatLayoutStyle":"compact","sidebarRailStyle":"split","settledThreadsEnabled":true,"autoColorThreadNames":true,"messageWidth":"full","avatarStyle":"role","codeBlockTheme":"high-contrast","backgroundFit":"contain","backgroundTreatment":"blur","thinkingBlocksOpen":true,"gitPanelExpanded":true,"appShortcuts":{"newChat":"Mod+Shift+N","focusSearch":"Mod+Shift+N","focusComposer":"x","stopGeneration":"F2","toggleSidebar":"Mod+B","previousThread":"Mod+Tab"}},"version":0}',
);
await useUiPreferences.persist.rehydrate();
equal(useUiPreferences.getState().sidebarOpen, false, "sidebar should rehydrate persisted closed state");
equal(useUiPreferences.getState().sidebarWidth, 384, "sidebar should rehydrate persisted width");
equal(useUiPreferences.getState().previewPanelWidth, 512, "preview panel should rehydrate persisted width");
equal(useUiPreferences.getState().mediaStudioWidth, 1040, "media studio should rehydrate persisted width");
equal(useUiPreferences.getState().mediaStudioHeight, 740, "media studio should rehydrate persisted height");
equal(useUiPreferences.getState().mediaComposerPlacement, "side", "media composer placement should rehydrate");
equal(useUiPreferences.getState().uiSize, 130, "UI size should rehydrate persisted value");
equal(useUiPreferences.getState().showAccountUsageInTitleBar, true, "missing title-bar account usage preference should keep its enabled default");
equal(useUiPreferences.getState().windowAlwaysOnTop, true, "always-on-top should rehydrate");
equal(useUiPreferences.getState().interfaceSounds, false, "malformed interface sounds should rehydrate as off");
equal(useUiPreferences.getState().soundOnFinished, true, "malformed finished sound toggle should use its default");
equal(useUiPreferences.getState().soundOnAttention, true, "malformed attention sound toggle should use its default");
equal(useUiPreferences.getState().soundOnInteractions, false, "malformed interaction sound toggle should use its default");
equal(useUiPreferences.getState().finishedSound, "ready", "malformed finished sound should use ready");
equal(useUiPreferences.getState().attentionSound, "error", "malformed attention sound should use error");
equal(useUiPreferences.getState().composerSendShortcut, "modEnter", "send shortcut should rehydrate");
equal(useUiPreferences.getState().composerDensity, "compact", "composer density should rehydrate");
equal(useUiPreferences.getState().autoTitleChats, false, "auto-title should rehydrate");
equal(useUiPreferences.getState().aiThreadNames, true, "AI thread names should rehydrate");
equal(useUiPreferences.getState().aiThreadNameModel, "persisted-title-model", "AI thread name model should rehydrate");
equal(useUiPreferences.getState().newChatButtonAtBottom, true, "new chat button placement should rehydrate");
equal("interfaceMode" in useUiPreferences.getState(), false, "obsolete persisted interface mode should be ignored");
equal(useUiPreferences.getState().developerMode, true, "developer mode should rehydrate");
equal(useUiPreferences.getState().experimentalHashlinePatch, true, "hashline patching should rehydrate");
equal(useUiPreferences.getState().chatLayoutStyle, "compact", "chat layout should rehydrate");
equal(useUiPreferences.getState().sidebarRailStyle, "split", "collapsed sidebar rail style should rehydrate");
equal(useUiPreferences.getState().settledThreadsEnabled, true, "settled threads should rehydrate");
equal(useUiPreferences.getState().showEmptyChatRidgeline, true, "missing empty-chat ridgeline preference should keep its enabled default");
equal(useUiPreferences.getState().autoColorThreadNames, true, "automatic thread colors should rehydrate");
equal(useUiPreferences.getState().messageWidth, "full", "message width should rehydrate");
equal(useUiPreferences.getState().avatarStyle, "role", "avatar style should rehydrate");
equal(useUiPreferences.getState().codeBlockTheme, "high-contrast", "code theme should rehydrate");
equal(useUiPreferences.getState().backgroundFit, "contain", "background fit should rehydrate");
equal(useUiPreferences.getState().backgroundTreatment, "blur", "background treatment should rehydrate");
equal("thinkingBlocksOpen" in useUiPreferences.getState(), false, "removed thinking block state should be ignored");
equal(useUiPreferences.getState().gitPanelExpanded, true, "git panel state should rehydrate");
equal(useUiPreferences.getState().appShortcuts.newChat, "Mod+Shift+N", "custom new chat shortcut should rehydrate");
equal(useUiPreferences.getState().appShortcuts.focusSearch, DEFAULT_APP_SHORTCUTS.focusSearch, "duplicate persisted shortcut should fall back");
equal(useUiPreferences.getState().appShortcuts.focusComposer, DEFAULT_APP_SHORTCUTS.focusComposer, "invalid persisted shortcut should fall back");
equal(useUiPreferences.getState().appShortcuts.stopGeneration, "F2", "custom stop shortcut should rehydrate");
equal(useUiPreferences.getState().appShortcuts.toggleSidebar, DEFAULT_APP_SHORTCUTS.toggleSidebar, "default sidebar shortcut should rehydrate");
equal(useUiPreferences.getState().appShortcuts.previousThread, "Ctrl+Tab", "old previous thread shortcut should migrate");

export {};
