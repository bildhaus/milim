import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import type { WorkspaceLauncherId } from "../api";
import {
  normalizeWorkspaceLauncherHistory,
  rememberWorkspaceLauncherInHistory,
} from "../lib/workspaceLauncher.js";
import { userStateStorage, writeUserStateKey } from "../persistence/userStateStorage.js";
import {
  DEFAULT_APP_SHORTCUTS,
  normalizeAppShortcuts,
  normalizeShortcut,
  shortcutConflict,
  shortcutValidationIssue,
  type AppShortcutAction,
  type AppShortcuts,
} from "./shortcuts.js";

export type ComposerSendShortcut = "enter" | "modEnter";
export type ComposerDensity = "comfortable" | "compact";
export type ChatLayoutStyle = "transcript" | "bubbles" | "compact";
export type SidebarRailStyle = "regular" | "split" | "centered";
export type MessageWidth = "narrow" | "standard" | "wide" | "full";
export type AvatarStyle = "none" | "avatar" | "role";
export type CodeBlockTheme = "match" | "terminal" | "github" | "high-contrast";
export type BackgroundFit = "cover" | "contain" | "tile" | "center";
export type BackgroundTreatment = "clear" | "dim" | "blur" | "mono";
export type StartupBehavior = "restore" | "new-chat";
export type QuickActionMode = "smart" | "pinned" | "hidden";
export type AutocompleteMode = "automatic" | "manual" | "off";
export type PromptHistoryScope = "thread" | "global" | "off";
export type ThreadExportFormat = "json" | "markdown";
export type NewProjectChatWorkspace = "current" | "ask" | "worktree";
export type WorkspaceLauncherPreference = "remember" | WorkspaceLauncherId;
export type ComposerCompletionMode = "off" | "local" | "current";
export type AutocompleteSource = "commands" | "files" | "skills" | "mcp";
export type AutocompleteSources = Record<AutocompleteSource, boolean>;

export interface PinnedQuickAction {
  id: string;
  label: string;
  prompt: string;
  builtinId?: string;
}

export interface SuggestionUsage {
  count: number;
  lastUsedAt: number;
}
export const FINISHED_SOUND_OPTIONS = ["ready", "success", "chime", "bloom"] as const;
export const ATTENTION_SOUND_OPTIONS = ["error", "tick", "chime", "droplet"] as const;
export type FinishedSound = (typeof FINISHED_SOUND_OPTIONS)[number];
export type AttentionSound = (typeof ATTENTION_SOUND_OPTIONS)[number];
export type AppNoticeTone = "info" | "success" | "warning" | "error";

export interface AppNotice {
  id: string;
  tone: AppNoticeTone;
  message: string;
  createdAt: number;
}

const WINDOW_ALWAYS_ON_TOP_KEY = "milim.window.alwaysOnTop";

interface UiPreferencesState {
  sidebarOpen: boolean;
  sidebarWidth: number;
  previewPanelWidth: number;
  mediaStudioWidth: number;
  mediaStudioHeight: number;
  pullRequestsWidth: number;
  pullRequestsHeight: number;
  pullRequestsListWidth: number;
  uiSize: number;
  showAccountUsageInTitleBar: boolean;
  windowAlwaysOnTop: boolean;
  interfaceSounds: boolean;
  soundOnFinished: boolean;
  soundOnAttention: boolean;
  soundOnInteractions: boolean;
  finishedSound: FinishedSound;
  attentionSound: AttentionSound;
  composerSendShortcut: ComposerSendShortcut;
  composerDensity: ComposerDensity;
  autoTitleChats: boolean;
  aiThreadNames: boolean;
  aiThreadNameModel: string;
  newChatButtonAtBottom: boolean;
  developerMode: boolean;
  experimentalHashlinePatch: boolean;
  chatLayoutStyle: ChatLayoutStyle;
  sidebarRailStyle: SidebarRailStyle;
  showEmptyChatRidgeline: boolean;
  autoColorThreadNames: boolean;
  messageWidth: MessageWidth;
  avatarStyle: AvatarStyle;
  codeBlockTheme: CodeBlockTheme;
  backgroundFit: BackgroundFit;
  backgroundTreatment: BackgroundTreatment;
  gitPanelExpanded: boolean;
  toolsExpanded: boolean;
  startupBehavior: StartupBehavior;
  restoreOpenPanels: boolean;
  notifyRunFinished: boolean;
  notifyNeedsAttention: boolean;
  notifyOnlyWhenUnfocused: boolean;
  notificationIncludeThreadTitle: boolean;
  quickActionMode: QuickActionMode;
  pinnedQuickActions: PinnedQuickAction[];
  projectQuickActionOverrides: Record<string, PinnedQuickAction[]>;
  autocompleteMode: AutocompleteMode;
  autocompleteSources: AutocompleteSources;
  personalizedSuggestions: boolean;
  suggestionUsage: Record<string, SuggestionUsage>;
  promptHistoryScope: PromptHistoryScope;
  globalPromptHistory: string[];
  threadExportFormat: ThreadExportFormat;
  workspaceLauncherPreference: WorkspaceLauncherPreference;
  newProjectChatWorkspace: NewProjectChatWorkspace;
  composerCompletionMode: ComposerCompletionMode;
  remoteCompletionConfirmed: boolean;
  workspaceLauncherLastUsedByFolder: Record<string, WorkspaceLauncherId>;
  notices: AppNotice[];
  appShortcuts: AppShortcuts;
  setSidebarOpen: (sidebarOpen: boolean) => void;
  setSidebarWidth: (sidebarWidth: number) => void;
  setPreviewPanelWidth: (previewPanelWidth: number) => void;
  setMediaStudioSize: (width: number, height: number) => void;
  setPullRequestsSize: (width: number, height: number) => void;
  setPullRequestsListWidth: (width: number) => void;
  setUiSize: (uiSize: number) => void;
  setShowAccountUsageInTitleBar: (showAccountUsageInTitleBar: boolean) => void;
  setWindowAlwaysOnTop: (windowAlwaysOnTop: boolean) => void;
  setInterfaceSounds: (interfaceSounds: boolean) => void;
  setSoundOnFinished: (soundOnFinished: boolean) => void;
  setSoundOnAttention: (soundOnAttention: boolean) => void;
  setSoundOnInteractions: (soundOnInteractions: boolean) => void;
  setFinishedSound: (finishedSound: FinishedSound) => void;
  setAttentionSound: (attentionSound: AttentionSound) => void;
  setComposerSendShortcut: (composerSendShortcut: ComposerSendShortcut) => void;
  setComposerDensity: (composerDensity: ComposerDensity) => void;
  setAutoTitleChats: (autoTitleChats: boolean) => void;
  setAiThreadNames: (aiThreadNames: boolean) => void;
  setAiThreadNameModel: (aiThreadNameModel: string) => void;
  setNewChatButtonAtBottom: (newChatButtonAtBottom: boolean) => void;
  setDeveloperMode: (developerMode: boolean) => void;
  setExperimentalHashlinePatch: (experimentalHashlinePatch: boolean) => void;
  setChatLayoutStyle: (chatLayoutStyle: ChatLayoutStyle) => void;
  setSidebarRailStyle: (sidebarRailStyle: SidebarRailStyle) => void;
  setShowEmptyChatRidgeline: (showEmptyChatRidgeline: boolean) => void;
  setAutoColorThreadNames: (autoColorThreadNames: boolean) => void;
  setMessageWidth: (messageWidth: MessageWidth) => void;
  setAvatarStyle: (avatarStyle: AvatarStyle) => void;
  setCodeBlockTheme: (codeBlockTheme: CodeBlockTheme) => void;
  setBackgroundFit: (backgroundFit: BackgroundFit) => void;
  setBackgroundTreatment: (backgroundTreatment: BackgroundTreatment) => void;
  setGitPanelExpanded: (gitPanelExpanded: boolean) => void;
  setToolsExpanded: (toolsExpanded: boolean) => void;
  setStartupBehavior: (startupBehavior: StartupBehavior) => void;
  setRestoreOpenPanels: (restoreOpenPanels: boolean) => void;
  setNotifyRunFinished: (notifyRunFinished: boolean) => void;
  setNotifyNeedsAttention: (notifyNeedsAttention: boolean) => void;
  setNotifyOnlyWhenUnfocused: (notifyOnlyWhenUnfocused: boolean) => void;
  setNotificationIncludeThreadTitle: (notificationIncludeThreadTitle: boolean) => void;
  setQuickActionMode: (quickActionMode: QuickActionMode) => void;
  setPinnedQuickActions: (actions: PinnedQuickAction[]) => void;
  setProjectQuickActionOverride: (projectId: string, actions: PinnedQuickAction[] | null) => void;
  setAutocompleteMode: (autocompleteMode: AutocompleteMode) => void;
  setAutocompleteSource: (source: AutocompleteSource, enabled: boolean) => void;
  setPersonalizedSuggestions: (personalizedSuggestions: boolean) => void;
  recordSuggestionUse: (id: string) => void;
  resetSuggestionHistory: () => void;
  setPromptHistoryScope: (promptHistoryScope: PromptHistoryScope) => void;
  recordGlobalPrompt: (prompt: string) => void;
  clearGlobalPromptHistory: () => void;
  setThreadExportFormat: (threadExportFormat: ThreadExportFormat) => void;
  setWorkspaceLauncherPreference: (workspaceLauncherPreference: WorkspaceLauncherPreference) => void;
  setNewProjectChatWorkspace: (newProjectChatWorkspace: NewProjectChatWorkspace) => void;
  setComposerCompletionMode: (composerCompletionMode: ComposerCompletionMode) => void;
  setRemoteCompletionConfirmed: (remoteCompletionConfirmed: boolean) => void;
  rememberWorkspaceLauncher: (folder: string, launcherId: WorkspaceLauncherId) => void;
  pushNotice: (notice: { tone: AppNoticeTone; message: string }) => string;
  dismissNotice: (id: string) => void;
  setAppShortcut: (action: AppShortcutAction, shortcut: string) => boolean;
  resetAppShortcuts: () => void;
  resetLayoutWidths: () => void;
  toggleSidebar: () => void;
}

export const DEFAULT_SIDEBAR_WIDTH = 248;
export const MIN_SIDEBAR_WIDTH = 220;
export const MAX_SIDEBAR_WIDTH = 420;
export const DEFAULT_PREVIEW_PANEL_WIDTH = 420;
const MIN_PREVIEW_PANEL_WIDTH = 280;
export const DEFAULT_MEDIA_STUDIO_WIDTH = 1120;
export const DEFAULT_MEDIA_STUDIO_HEIGHT = 820;
export const MIN_MEDIA_STUDIO_WIDTH = 560;
export const MIN_MEDIA_STUDIO_HEIGHT = 480;
export const DEFAULT_PULL_REQUESTS_LIST_WIDTH = 520;
export const MIN_PULL_REQUESTS_LIST_WIDTH = 240;
const MAX_MEDIA_STUDIO_WIDTH = 2400;
const MAX_MEDIA_STUDIO_HEIGHT = 1600;
export const DEFAULT_UI_SIZE = 100;
export const MIN_UI_SIZE = 80;
export const MAX_UI_SIZE = 140;
export const UI_SIZE_STEP = 10;

export function normalizeSidebarWidth(width: number): number {
  if (!Number.isFinite(width)) return DEFAULT_SIDEBAR_WIDTH;
  return Math.round(Math.min(Math.max(width, MIN_SIDEBAR_WIDTH), MAX_SIDEBAR_WIDTH));
}

export function normalizePullRequestsListWidth(width: number): number {
  if (!Number.isFinite(width)) return DEFAULT_PULL_REQUESTS_LIST_WIDTH;
  return Math.round(Math.max(width, MIN_PULL_REQUESTS_LIST_WIDTH));
}

function normalizePreviewPanelWidth(width: number): number {
  if (!Number.isFinite(width)) return DEFAULT_PREVIEW_PANEL_WIDTH;
  return Math.round(Math.max(width, MIN_PREVIEW_PANEL_WIDTH));
}

export function normalizeMediaStudioSize(width: number, height: number): { width: number; height: number } {
  return {
    width: Math.round(Math.min(Math.max(Number.isFinite(width) ? width : DEFAULT_MEDIA_STUDIO_WIDTH, MIN_MEDIA_STUDIO_WIDTH), MAX_MEDIA_STUDIO_WIDTH)),
    height: Math.round(Math.min(Math.max(Number.isFinite(height) ? height : DEFAULT_MEDIA_STUDIO_HEIGHT, MIN_MEDIA_STUDIO_HEIGHT), MAX_MEDIA_STUDIO_HEIGHT)),
  };
}

export function normalizeUiSize(size: number): number {
  if (!Number.isFinite(size)) return DEFAULT_UI_SIZE;
  const stepped = Math.round(size / UI_SIZE_STEP) * UI_SIZE_STEP;
  return Math.min(Math.max(stepped, MIN_UI_SIZE), MAX_UI_SIZE);
}

function normalizeComposerSendShortcut(value: unknown): ComposerSendShortcut {
  return value === "modEnter" ? "modEnter" : "enter";
}

function normalizeComposerDensity(value: unknown): ComposerDensity {
  return value === "compact" ? "compact" : "comfortable";
}

function normalizeAiThreadNameModel(value: unknown): string {
  return typeof value === "string" && value.trim().toLowerCase() !== "mock-echo" ? value.trim() : "";
}

function normalizeChatLayoutStyle(value: unknown): ChatLayoutStyle {
  return value === "bubbles" || value === "compact" ? value : "transcript";
}

function normalizeSidebarRailStyle(value: unknown): SidebarRailStyle {
  return value === "split" || value === "centered" ? value : "regular";
}

function normalizeMessageWidth(value: unknown): MessageWidth {
  return value === "narrow" || value === "wide" || value === "full" ? value : "standard";
}

function normalizeAvatarStyle(value: unknown): AvatarStyle {
  if (value === "avatar" || value === "initials") return "avatar";
  return value === "role" ? "role" : "none";
}

function normalizeCodeBlockTheme(value: unknown): CodeBlockTheme {
  return value === "terminal" || value === "github" || value === "high-contrast" ? value : "match";
}

function normalizeBackgroundFit(value: unknown): BackgroundFit {
  return value === "contain" || value === "tile" || value === "center" ? value : "cover";
}

function normalizeBackgroundTreatment(value: unknown): BackgroundTreatment {
  return value === "dim" || value === "blur" || value === "mono" ? value : "clear";
}

function normalizeFinishedSound(value: unknown): FinishedSound {
  return typeof value === "string" && FINISHED_SOUND_OPTIONS.includes(value as FinishedSound)
    ? value as FinishedSound
    : "ready";
}

function normalizeAttentionSound(value: unknown): AttentionSound {
  return typeof value === "string" && ATTENTION_SOUND_OPTIONS.includes(value as AttentionSound)
    ? value as AttentionSound
    : "error";
}

const DEFAULT_AUTOCOMPLETE_SOURCES: AutocompleteSources = {
  commands: true,
  files: true,
  skills: true,
  mcp: true,
};

function normalizeEnum<T extends string>(value: unknown, values: readonly T[], fallback: T): T {
  return typeof value === "string" && values.includes(value as T) ? value as T : fallback;
}

function normalizePinnedQuickActions(value: unknown): PinnedQuickAction[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const actions: PinnedQuickAction[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) continue;
    const source = item as Partial<PinnedQuickAction>;
    const id = typeof source.id === "string" ? source.id.trim() : "";
    const label = typeof source.label === "string" ? source.label.trim().slice(0, 80) : "";
    const prompt = typeof source.prompt === "string" ? source.prompt.trim().slice(0, 4000) : "";
    if (!id || !label || !prompt || seen.has(id)) continue;
    seen.add(id);
    actions.push({ id, label, prompt, ...(typeof source.builtinId === "string" ? { builtinId: source.builtinId } : {}) });
    if (actions.length === 3) break;
  }
  return actions;
}

function normalizeQuickActionOverrides(value: unknown): Record<string, PinnedQuickAction[]> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const result: Record<string, PinnedQuickAction[]> = {};
  for (const [projectId, actions] of Object.entries(value)) {
    if (!projectId.trim()) continue;
    result[projectId] = normalizePinnedQuickActions(actions);
  }
  return result;
}

function normalizeAutocompleteSources(value: unknown): AutocompleteSources {
  const saved = value && typeof value === "object" && !Array.isArray(value) ? value as Partial<AutocompleteSources> : {};
  return Object.fromEntries(
    Object.entries(DEFAULT_AUTOCOMPLETE_SOURCES).map(([source, fallback]) => [
      source,
      typeof saved[source as AutocompleteSource] === "boolean" ? saved[source as AutocompleteSource] : fallback,
    ]),
  ) as AutocompleteSources;
}

function normalizeSuggestionUsage(value: unknown): Record<string, SuggestionUsage> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(
    Object.entries(value)
      .map(([id, usage]) => {
        if (!id.trim() || !usage || typeof usage !== "object" || Array.isArray(usage)) return null;
        const source = usage as Partial<SuggestionUsage>;
        if (!Number.isFinite(source.count) || !Number.isFinite(source.lastUsedAt)) return null;
        return [id, { count: Math.min(999, Math.max(1, Math.round(source.count!))), lastUsedAt: Math.max(0, Math.round(source.lastUsedAt!)) }] as const;
      })
      .filter((entry): entry is readonly [string, SuggestionUsage] => Boolean(entry))
      .sort((a, b) => b[1].lastUsedAt - a[1].lastUsedAt)
      .slice(0, 200),
  );
}

function normalizeGlobalPromptHistory(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value
    .filter((item): item is string => typeof item === "string")
    .map((item) => item.trim())
    .filter((item) => item && !seen.has(item) && Boolean(seen.add(item)))
    .slice(0, 100);
}

function normalizeWorkspaceLauncherPreference(value: unknown): WorkspaceLauncherPreference {
  return value === "vscode" || value === "zed" || value === "file_manager" || value === "terminal" || value === "git_bash" || value === "wsl" || value === "android_studio"
    ? value
    : "remember";
}

function persistWindowAlwaysOnTop(windowAlwaysOnTop: boolean): void {
  void Promise.resolve(writeUserStateKey(WINDOW_ALWAYS_ON_TOP_KEY, String(windowAlwaysOnTop))).catch(() => {});
}

export const useUiPreferences = create<UiPreferencesState>()(
  persist(
    (set) => ({
      sidebarOpen: true,
      sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
      previewPanelWidth: DEFAULT_PREVIEW_PANEL_WIDTH,
      mediaStudioWidth: DEFAULT_MEDIA_STUDIO_WIDTH,
      mediaStudioHeight: DEFAULT_MEDIA_STUDIO_HEIGHT,
      pullRequestsWidth: DEFAULT_MEDIA_STUDIO_WIDTH,
      pullRequestsHeight: DEFAULT_MEDIA_STUDIO_HEIGHT,
      pullRequestsListWidth: DEFAULT_PULL_REQUESTS_LIST_WIDTH,
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
      showEmptyChatRidgeline: true,
      autoColorThreadNames: false,
      messageWidth: "standard",
      avatarStyle: "none",
      codeBlockTheme: "match",
      backgroundFit: "cover",
      backgroundTreatment: "clear",
      gitPanelExpanded: false,
      toolsExpanded: false,
      startupBehavior: "restore",
      restoreOpenPanels: true,
      notifyRunFinished: false,
      notifyNeedsAttention: false,
      notifyOnlyWhenUnfocused: true,
      notificationIncludeThreadTitle: false,
      quickActionMode: "smart",
      pinnedQuickActions: [],
      projectQuickActionOverrides: {},
      autocompleteMode: "automatic",
      autocompleteSources: { ...DEFAULT_AUTOCOMPLETE_SOURCES },
      personalizedSuggestions: true,
      suggestionUsage: {},
      promptHistoryScope: "thread",
      globalPromptHistory: [],
      threadExportFormat: "json",
      workspaceLauncherPreference: "remember",
      newProjectChatWorkspace: "current",
      composerCompletionMode: "off",
      remoteCompletionConfirmed: false,
      workspaceLauncherLastUsedByFolder: {},
      notices: [],
      appShortcuts: { ...DEFAULT_APP_SHORTCUTS },
      setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
      setSidebarWidth: (sidebarWidth) => set({ sidebarWidth: normalizeSidebarWidth(sidebarWidth) }),
      setPreviewPanelWidth: (previewPanelWidth) => set({ previewPanelWidth: normalizePreviewPanelWidth(previewPanelWidth) }),
      setMediaStudioSize: (width, height) => {
        const size = normalizeMediaStudioSize(width, height);
        set({ mediaStudioWidth: size.width, mediaStudioHeight: size.height });
      },
      setPullRequestsSize: (width, height) => {
        const size = normalizeMediaStudioSize(width, height);
        set({ pullRequestsWidth: size.width, pullRequestsHeight: size.height });
      },
      setPullRequestsListWidth: (width) =>
        set({ pullRequestsListWidth: normalizePullRequestsListWidth(width) }),
      setUiSize: (uiSize) => set({ uiSize: normalizeUiSize(uiSize) }),
      setShowAccountUsageInTitleBar: (showAccountUsageInTitleBar) => set({ showAccountUsageInTitleBar }),
      setWindowAlwaysOnTop: (windowAlwaysOnTop) => {
        persistWindowAlwaysOnTop(windowAlwaysOnTop);
        set({ windowAlwaysOnTop });
      },
      setInterfaceSounds: (interfaceSounds) => set({ interfaceSounds }),
      setSoundOnFinished: (soundOnFinished) => set({ soundOnFinished }),
      setSoundOnAttention: (soundOnAttention) => set({ soundOnAttention }),
      setSoundOnInteractions: (soundOnInteractions) => set({ soundOnInteractions }),
      setFinishedSound: (finishedSound) => set({ finishedSound: normalizeFinishedSound(finishedSound) }),
      setAttentionSound: (attentionSound) => set({ attentionSound: normalizeAttentionSound(attentionSound) }),
      setComposerSendShortcut: (composerSendShortcut) =>
        set({ composerSendShortcut: normalizeComposerSendShortcut(composerSendShortcut) }),
      setComposerDensity: (composerDensity) => set({ composerDensity: normalizeComposerDensity(composerDensity) }),
      setAutoTitleChats: (autoTitleChats) => set({ autoTitleChats }),
      setAiThreadNames: (aiThreadNames) => set({ aiThreadNames }),
      setAiThreadNameModel: (aiThreadNameModel) => set({ aiThreadNameModel: normalizeAiThreadNameModel(aiThreadNameModel) }),
      setNewChatButtonAtBottom: (newChatButtonAtBottom) => set({ newChatButtonAtBottom }),
      setDeveloperMode: (developerMode) => set({ developerMode }),
      setExperimentalHashlinePatch: (experimentalHashlinePatch) => set({ experimentalHashlinePatch }),
      setChatLayoutStyle: (chatLayoutStyle) => set({ chatLayoutStyle: normalizeChatLayoutStyle(chatLayoutStyle) }),
      setSidebarRailStyle: (sidebarRailStyle) => set({ sidebarRailStyle: normalizeSidebarRailStyle(sidebarRailStyle) }),
      setShowEmptyChatRidgeline: (showEmptyChatRidgeline) => set({ showEmptyChatRidgeline }),
      setAutoColorThreadNames: (autoColorThreadNames) => set({ autoColorThreadNames }),
      setMessageWidth: (messageWidth) => set({ messageWidth: normalizeMessageWidth(messageWidth) }),
      setAvatarStyle: (avatarStyle) => set({ avatarStyle: normalizeAvatarStyle(avatarStyle) }),
      setCodeBlockTheme: (codeBlockTheme) => set({ codeBlockTheme: normalizeCodeBlockTheme(codeBlockTheme) }),
      setBackgroundFit: (backgroundFit) => set({ backgroundFit: normalizeBackgroundFit(backgroundFit) }),
      setBackgroundTreatment: (backgroundTreatment) => set({ backgroundTreatment: normalizeBackgroundTreatment(backgroundTreatment) }),
      setGitPanelExpanded: (gitPanelExpanded) => set({ gitPanelExpanded }),
      setToolsExpanded: (toolsExpanded) => set({ toolsExpanded }),
      setStartupBehavior: (startupBehavior) => set({ startupBehavior: normalizeEnum<StartupBehavior>(startupBehavior, ["restore", "new-chat"], "restore") }),
      setRestoreOpenPanels: (restoreOpenPanels) => set({ restoreOpenPanels }),
      setNotifyRunFinished: (notifyRunFinished) => set({ notifyRunFinished }),
      setNotifyNeedsAttention: (notifyNeedsAttention) => set({ notifyNeedsAttention }),
      setNotifyOnlyWhenUnfocused: (notifyOnlyWhenUnfocused) => set({ notifyOnlyWhenUnfocused }),
      setNotificationIncludeThreadTitle: (notificationIncludeThreadTitle) => set({ notificationIncludeThreadTitle }),
      setQuickActionMode: (quickActionMode) => set({ quickActionMode: normalizeEnum<QuickActionMode>(quickActionMode, ["smart", "pinned", "hidden"], "smart") }),
      setPinnedQuickActions: (actions) => set({ pinnedQuickActions: normalizePinnedQuickActions(actions) }),
      setProjectQuickActionOverride: (projectId, actions) => set((state) => {
        const key = projectId.trim();
        if (!key) return {};
        const next = { ...state.projectQuickActionOverrides };
        if (actions == null) delete next[key];
        else next[key] = normalizePinnedQuickActions(actions);
        return { projectQuickActionOverrides: next };
      }),
      setAutocompleteMode: (autocompleteMode) => set({ autocompleteMode: normalizeEnum<AutocompleteMode>(autocompleteMode, ["automatic", "manual", "off"], "automatic") }),
      setAutocompleteSource: (source, enabled) => set((state) => ({ autocompleteSources: { ...state.autocompleteSources, [source]: enabled } })),
      setPersonalizedSuggestions: (personalizedSuggestions) => set({ personalizedSuggestions }),
      recordSuggestionUse: (id) => set((state) => {
        const key = id.trim();
        if (!key) return {};
        const existing = state.suggestionUsage[key];
        return {
          suggestionUsage: normalizeSuggestionUsage({
            ...state.suggestionUsage,
            [key]: { count: Math.min(999, (existing?.count ?? 0) + 1), lastUsedAt: Date.now() },
          }),
        };
      }),
      resetSuggestionHistory: () => set({ suggestionUsage: {} }),
      setPromptHistoryScope: (promptHistoryScope) => set({ promptHistoryScope: normalizeEnum<PromptHistoryScope>(promptHistoryScope, ["thread", "global", "off"], "thread") }),
      recordGlobalPrompt: (prompt) => set((state) => {
        const value = prompt.trim();
        if (!value) return {};
        return { globalPromptHistory: [value, ...state.globalPromptHistory.filter((item) => item !== value)].slice(0, 100) };
      }),
      clearGlobalPromptHistory: () => set({ globalPromptHistory: [] }),
      setThreadExportFormat: (threadExportFormat) => set({ threadExportFormat: normalizeEnum<ThreadExportFormat>(threadExportFormat, ["json", "markdown"], "json") }),
      setWorkspaceLauncherPreference: (workspaceLauncherPreference) => set({ workspaceLauncherPreference: normalizeWorkspaceLauncherPreference(workspaceLauncherPreference) }),
      setNewProjectChatWorkspace: (newProjectChatWorkspace) => set({ newProjectChatWorkspace: normalizeEnum<NewProjectChatWorkspace>(newProjectChatWorkspace, ["current", "ask", "worktree"], "current") }),
      setComposerCompletionMode: (composerCompletionMode) => set({ composerCompletionMode: normalizeEnum<ComposerCompletionMode>(composerCompletionMode, ["off", "local", "current"], "off") }),
      setRemoteCompletionConfirmed: (remoteCompletionConfirmed) => set({ remoteCompletionConfirmed }),
      rememberWorkspaceLauncher: (folder, launcherId) =>
        set((state) => ({
          workspaceLauncherLastUsedByFolder: rememberWorkspaceLauncherInHistory(
            state.workspaceLauncherLastUsedByFolder,
            folder,
            launcherId,
          ),
        })),
      pushNotice: (notice) => {
        const id = `notice-${Date.now()}-${Math.random().toString(36).slice(2)}`;
        set((state) => ({
          notices: [
            ...state.notices.slice(-3),
            { ...notice, id, createdAt: Date.now() },
          ],
        }));
        return id;
      },
      dismissNotice: (id) =>
        set((state) => ({
          notices: state.notices.filter((notice) => notice.id !== id),
        })),
      setAppShortcut: (action, shortcut) => {
        let accepted = false;
        set((state) => {
          const normalized = normalizeShortcut(shortcut);
          const current = normalizeAppShortcuts(state.appShortcuts);
          if (
            !normalized ||
            shortcutValidationIssue(normalized) ||
            shortcutConflict(current, action, normalized)
          ) {
            return {};
          }
          accepted = true;
          return { appShortcuts: { ...current, [action]: normalized } };
        });
        return accepted;
      },
      resetAppShortcuts: () => set({ appShortcuts: { ...DEFAULT_APP_SHORTCUTS } }),
      resetLayoutWidths: () =>
        set({
          sidebarWidth: DEFAULT_SIDEBAR_WIDTH,
          previewPanelWidth: DEFAULT_PREVIEW_PANEL_WIDTH,
          mediaStudioWidth: DEFAULT_MEDIA_STUDIO_WIDTH,
          mediaStudioHeight: DEFAULT_MEDIA_STUDIO_HEIGHT,
          pullRequestsWidth: DEFAULT_MEDIA_STUDIO_WIDTH,
          pullRequestsHeight: DEFAULT_MEDIA_STUDIO_HEIGHT,
          pullRequestsListWidth: DEFAULT_PULL_REQUESTS_LIST_WIDTH,
        }),
      toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
    }),
    {
      name: "milim.ui",
      storage: createJSONStorage(() => userStateStorage),
      merge: (persisted, current) => {
        const saved = persisted as (Partial<UiPreferencesState> & { thinkingBlocksOpen?: unknown; interfaceMode?: unknown; workbenchExpanded?: unknown }) | undefined;
        const savedState = saved ? { ...saved } : undefined;
        delete savedState?.thinkingBlocksOpen;
        delete savedState?.interfaceMode;
        delete savedState?.workbenchExpanded;
        return {
          ...current,
          ...savedState,
          sidebarOpen: typeof saved?.sidebarOpen === "boolean" ? saved.sidebarOpen : current.sidebarOpen,
          sidebarWidth: normalizeSidebarWidth(saved?.sidebarWidth ?? current.sidebarWidth),
          previewPanelWidth: normalizePreviewPanelWidth(saved?.previewPanelWidth ?? current.previewPanelWidth),
          mediaStudioWidth: normalizeMediaStudioSize(
            saved?.mediaStudioWidth ?? current.mediaStudioWidth,
            saved?.mediaStudioHeight ?? current.mediaStudioHeight,
          ).width,
          mediaStudioHeight: normalizeMediaStudioSize(
            saved?.mediaStudioWidth ?? current.mediaStudioWidth,
            saved?.mediaStudioHeight ?? current.mediaStudioHeight,
          ).height,
          pullRequestsWidth: normalizeMediaStudioSize(
            saved?.pullRequestsWidth ?? current.pullRequestsWidth,
            saved?.pullRequestsHeight ?? current.pullRequestsHeight,
          ).width,
          pullRequestsHeight: normalizeMediaStudioSize(
            saved?.pullRequestsWidth ?? current.pullRequestsWidth,
            saved?.pullRequestsHeight ?? current.pullRequestsHeight,
          ).height,
          pullRequestsListWidth: normalizePullRequestsListWidth(
            saved?.pullRequestsListWidth ?? current.pullRequestsListWidth,
          ),
          uiSize: normalizeUiSize(saved?.uiSize ?? current.uiSize),
          showAccountUsageInTitleBar: typeof saved?.showAccountUsageInTitleBar === "boolean" ? saved.showAccountUsageInTitleBar : current.showAccountUsageInTitleBar,
          windowAlwaysOnTop: typeof saved?.windowAlwaysOnTop === "boolean" ? saved.windowAlwaysOnTop : current.windowAlwaysOnTop,
          interfaceSounds: typeof saved?.interfaceSounds === "boolean" ? saved.interfaceSounds : false,
          soundOnFinished: typeof saved?.soundOnFinished === "boolean" ? saved.soundOnFinished : true,
          soundOnAttention: typeof saved?.soundOnAttention === "boolean" ? saved.soundOnAttention : true,
          soundOnInteractions: typeof saved?.soundOnInteractions === "boolean" ? saved.soundOnInteractions : false,
          finishedSound: normalizeFinishedSound(saved?.finishedSound),
          attentionSound: normalizeAttentionSound(saved?.attentionSound),
          composerSendShortcut: normalizeComposerSendShortcut(saved?.composerSendShortcut),
          composerDensity: normalizeComposerDensity(saved?.composerDensity),
          autoTitleChats: typeof saved?.autoTitleChats === "boolean" ? saved.autoTitleChats : current.autoTitleChats,
          aiThreadNames: typeof saved?.aiThreadNames === "boolean" ? saved.aiThreadNames : current.aiThreadNames,
          aiThreadNameModel: normalizeAiThreadNameModel(saved?.aiThreadNameModel),
          newChatButtonAtBottom: typeof saved?.newChatButtonAtBottom === "boolean" ? saved.newChatButtonAtBottom : current.newChatButtonAtBottom,
          developerMode: typeof saved?.developerMode === "boolean" ? saved.developerMode : current.developerMode,
          experimentalHashlinePatch: typeof saved?.experimentalHashlinePatch === "boolean" ? saved.experimentalHashlinePatch : current.experimentalHashlinePatch,
          chatLayoutStyle: normalizeChatLayoutStyle(saved?.chatLayoutStyle),
          sidebarRailStyle: normalizeSidebarRailStyle(saved?.sidebarRailStyle),
          showEmptyChatRidgeline: typeof saved?.showEmptyChatRidgeline === "boolean" ? saved.showEmptyChatRidgeline : true,
          autoColorThreadNames: typeof saved?.autoColorThreadNames === "boolean" ? saved.autoColorThreadNames : false,
          messageWidth: normalizeMessageWidth(saved?.messageWidth),
          avatarStyle: normalizeAvatarStyle(saved?.avatarStyle),
          codeBlockTheme: normalizeCodeBlockTheme(saved?.codeBlockTheme),
          backgroundFit: normalizeBackgroundFit(saved?.backgroundFit),
          backgroundTreatment: normalizeBackgroundTreatment(saved?.backgroundTreatment),
          gitPanelExpanded: typeof saved?.gitPanelExpanded === "boolean" ? saved.gitPanelExpanded : current.gitPanelExpanded,
          toolsExpanded:
            typeof saved?.toolsExpanded === "boolean"
              ? saved.toolsExpanded
              : typeof (saved as { workbenchExpanded?: unknown } | undefined)?.workbenchExpanded === "boolean"
                ? Boolean((saved as { workbenchExpanded: boolean }).workbenchExpanded)
                : current.toolsExpanded,
          startupBehavior: normalizeEnum(saved?.startupBehavior, ["restore", "new-chat"], "restore"),
          restoreOpenPanels: typeof saved?.restoreOpenPanels === "boolean" ? saved.restoreOpenPanels : true,
          notifyRunFinished: typeof saved?.notifyRunFinished === "boolean" ? saved.notifyRunFinished : false,
          notifyNeedsAttention: typeof saved?.notifyNeedsAttention === "boolean" ? saved.notifyNeedsAttention : false,
          notifyOnlyWhenUnfocused: typeof saved?.notifyOnlyWhenUnfocused === "boolean" ? saved.notifyOnlyWhenUnfocused : true,
          notificationIncludeThreadTitle: typeof saved?.notificationIncludeThreadTitle === "boolean" ? saved.notificationIncludeThreadTitle : false,
          quickActionMode: normalizeEnum(saved?.quickActionMode, ["smart", "pinned", "hidden"], "smart"),
          pinnedQuickActions: normalizePinnedQuickActions(saved?.pinnedQuickActions),
          projectQuickActionOverrides: normalizeQuickActionOverrides(saved?.projectQuickActionOverrides),
          autocompleteMode: normalizeEnum(saved?.autocompleteMode, ["automatic", "manual", "off"], "automatic"),
          autocompleteSources: normalizeAutocompleteSources(saved?.autocompleteSources),
          personalizedSuggestions: typeof saved?.personalizedSuggestions === "boolean" ? saved.personalizedSuggestions : true,
          suggestionUsage: normalizeSuggestionUsage(saved?.suggestionUsage),
          promptHistoryScope: normalizeEnum(saved?.promptHistoryScope, ["thread", "global", "off"], "thread"),
          globalPromptHistory: normalizeGlobalPromptHistory(saved?.globalPromptHistory),
          threadExportFormat: normalizeEnum(saved?.threadExportFormat, ["json", "markdown"], "json"),
          workspaceLauncherPreference: normalizeWorkspaceLauncherPreference(saved?.workspaceLauncherPreference),
          newProjectChatWorkspace: normalizeEnum(saved?.newProjectChatWorkspace, ["current", "ask", "worktree"], "current"),
          composerCompletionMode: normalizeEnum(saved?.composerCompletionMode, ["off", "local", "current"], "off"),
          remoteCompletionConfirmed: typeof saved?.remoteCompletionConfirmed === "boolean" ? saved.remoteCompletionConfirmed : false,
          workspaceLauncherLastUsedByFolder: normalizeWorkspaceLauncherHistory(saved?.workspaceLauncherLastUsedByFolder),
          notices: [],
          appShortcuts: normalizeAppShortcuts(saved?.appShortcuts),
        };
      },
      partialize: (state) => ({
        sidebarOpen: state.sidebarOpen,
        sidebarWidth: normalizeSidebarWidth(state.sidebarWidth),
        previewPanelWidth: normalizePreviewPanelWidth(state.previewPanelWidth),
        mediaStudioWidth: normalizeMediaStudioSize(state.mediaStudioWidth, state.mediaStudioHeight).width,
        mediaStudioHeight: normalizeMediaStudioSize(state.mediaStudioWidth, state.mediaStudioHeight).height,
        pullRequestsWidth: normalizeMediaStudioSize(state.pullRequestsWidth, state.pullRequestsHeight).width,
        pullRequestsHeight: normalizeMediaStudioSize(state.pullRequestsWidth, state.pullRequestsHeight).height,
        pullRequestsListWidth: normalizePullRequestsListWidth(state.pullRequestsListWidth),
        uiSize: normalizeUiSize(state.uiSize),
        showAccountUsageInTitleBar: state.showAccountUsageInTitleBar,
        windowAlwaysOnTop: state.windowAlwaysOnTop,
        interfaceSounds: state.interfaceSounds,
        soundOnFinished: state.soundOnFinished,
        soundOnAttention: state.soundOnAttention,
        soundOnInteractions: state.soundOnInteractions,
        finishedSound: normalizeFinishedSound(state.finishedSound),
        attentionSound: normalizeAttentionSound(state.attentionSound),
        composerSendShortcut: normalizeComposerSendShortcut(state.composerSendShortcut),
        composerDensity: normalizeComposerDensity(state.composerDensity),
        autoTitleChats: state.autoTitleChats,
        aiThreadNames: state.aiThreadNames,
        aiThreadNameModel: normalizeAiThreadNameModel(state.aiThreadNameModel),
        newChatButtonAtBottom: state.newChatButtonAtBottom,
        developerMode: state.developerMode,
        experimentalHashlinePatch: state.experimentalHashlinePatch,
        chatLayoutStyle: normalizeChatLayoutStyle(state.chatLayoutStyle),
        sidebarRailStyle: normalizeSidebarRailStyle(state.sidebarRailStyle),
        showEmptyChatRidgeline: state.showEmptyChatRidgeline,
        autoColorThreadNames: state.autoColorThreadNames,
        messageWidth: normalizeMessageWidth(state.messageWidth),
        avatarStyle: normalizeAvatarStyle(state.avatarStyle),
        codeBlockTheme: normalizeCodeBlockTheme(state.codeBlockTheme),
        backgroundFit: normalizeBackgroundFit(state.backgroundFit),
        backgroundTreatment: normalizeBackgroundTreatment(state.backgroundTreatment),
        gitPanelExpanded: state.gitPanelExpanded,
        toolsExpanded: state.toolsExpanded,
        startupBehavior: state.startupBehavior,
        restoreOpenPanels: state.restoreOpenPanels,
        notifyRunFinished: state.notifyRunFinished,
        notifyNeedsAttention: state.notifyNeedsAttention,
        notifyOnlyWhenUnfocused: state.notifyOnlyWhenUnfocused,
        notificationIncludeThreadTitle: state.notificationIncludeThreadTitle,
        quickActionMode: state.quickActionMode,
        pinnedQuickActions: normalizePinnedQuickActions(state.pinnedQuickActions),
        projectQuickActionOverrides: normalizeQuickActionOverrides(state.projectQuickActionOverrides),
        autocompleteMode: state.autocompleteMode,
        autocompleteSources: normalizeAutocompleteSources(state.autocompleteSources),
        personalizedSuggestions: state.personalizedSuggestions,
        suggestionUsage: normalizeSuggestionUsage(state.suggestionUsage),
        promptHistoryScope: state.promptHistoryScope,
        globalPromptHistory: normalizeGlobalPromptHistory(state.globalPromptHistory),
        threadExportFormat: state.threadExportFormat,
        workspaceLauncherPreference: state.workspaceLauncherPreference,
        newProjectChatWorkspace: state.newProjectChatWorkspace,
        composerCompletionMode: state.composerCompletionMode,
        remoteCompletionConfirmed: state.remoteCompletionConfirmed,
        workspaceLauncherLastUsedByFolder: normalizeWorkspaceLauncherHistory(state.workspaceLauncherLastUsedByFolder),
        appShortcuts: normalizeAppShortcuts(state.appShortcuts),
      }),
    },
  ),
);
