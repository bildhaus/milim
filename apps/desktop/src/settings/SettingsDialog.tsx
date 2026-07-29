import { Fragment, type KeyboardEvent, useEffect, useMemo, useState } from "react";
import {
  listModelsDetailed,
  listWorkspaceLaunchers,
  exportMilimBackup,
  inspectMilimBackup,
  restoreMilimBackup,
  restartDesktopApp,
  openDiagnosticsFolder,
  chooseGoogleWorkspaceFiles,
  disconnectGoogleWorkspace,
  getGoogleWorkspaceStatus,
  getSecretStorageStatus,
  openExternalUrl,
  removeGoogleWorkspaceFile,
  type GoogleWorkspaceStatus,
  type ModelInfo,
  type SecretStorageStatus,
  type WorkspaceLauncher,
} from "../api";
import { flushDeferredUserStateWrites } from "../persistence/userStateStorage";
import { clearPreviewWebviewData } from "../lib/previewWebview";
import {
  GOOGLE_ACCOUNT_CONNECTIONS_URL,
  GOOGLE_CONNECT_DISCLOSURE,
  GOOGLE_REMOVE_MESSAGE,
  googleDisconnectMessage,
  googleWorkspaceFileUrl,
} from "../lib/googleWorkspace";
import { useAgents } from "../agents/store";
import { useSettings, type ConfiguredThreadDefaults } from "./store";
import { BUILTIN_QUICK_ACTIONS } from "../lib/emptyStarterSuggestions";
import { ensureNativeNotificationPermission } from "../lib/nativeNotifications";
import { isThreadNamingModel } from "../lib/threadTitles";
import {
  matchingSettingsEntries,
  type SettingSearchEntry,
  type SettingsSectionId,
} from "./search";
import {
  SettingsBlock,
  SettingsChoiceGroup,
  SettingsPanel,
} from "./SettingsPrimitives";
import {
  AppearanceAvatarChoices,
  AppearanceBackgroundImageChoices,
  AppearanceChatLayoutChoices,
  AppearanceCodeBlockThemeChoices,
  AppearanceMessageWidthChoices,
} from "./AppearancePreviewChoices";
import { useTheme } from "../theme/store";
import { themeContrastIssues } from "../theme/contrast";
import type { Theme } from "../theme/types";
import { useOnboarding } from "../onboarding/store";
import { DAY_MS, useSessions, type ArchiveRetentionDays, type Project, type Session } from "../sessions/store";
import { useUpdateStore, type UpdateStatus } from "../update/store";
import { UpdateProgress } from "../update/UpdateProgress";
import { showUpdateCardsForDebug } from "../components/UpdateCards";
import {
  APP_SHORTCUT_ACTIONS,
  APP_SHORTCUT_LABELS,
  shortcutConflict,
  shortcutFromKeyboardEvent,
  shortcutLabel,
  shortcutValidationIssue,
  type AppShortcutAction,
} from "../ui/shortcuts";
import {
  ATTENTION_SOUND_OPTIONS,
  DEFAULT_UI_SIZE,
  DEFAULT_PREVIEW_PANEL_WIDTH,
  DEFAULT_SIDEBAR_WIDTH,
  FINISHED_SOUND_OPTIONS,
  MAX_UI_SIZE,
  MIN_UI_SIZE,
  UI_SIZE_STEP,
  useUiPreferences,
  type AttentionSound,
  type ComposerDensity,
  type ComposerSendShortcut,
  type FinishedSound,
  type PinnedQuickAction,
} from "../ui/store";
import { playInterfaceSound } from "../ui/sounds";
import { Archive, Check, Code, Download, ExternalLink, FileText, FolderOpen, Gear, GitLogo, Pencil, PlusSquare, Refresh, Search, Sidebar, Smartphone, Sun, Trash, X } from "../components/icons";
import { MobileCompanionSettings } from "../components/MobileCompanionSettings";
import { SheetDialog } from "../components/SheetDialog";
import { ThemeEditor } from "../components/ThemeEditor";
import { Select, Slider, Toggle } from "../components/ui";

type SettingsSection = {
  id: SettingsSectionId;
  label: string;
  detail: string;
  icon: typeof Gear;
};
type SettingsBadge = { label: string; tone: "neutral" | "warn" };
type SettingsSectionActivation = { focusTab?: boolean; remember?: boolean };
type ShortcutRecordingTarget = AppShortcutAction;

const googleLogo = new URL("../assets/google.svg", import.meta.url).href;

const SOUND_LABELS: Record<AttentionSound | FinishedSound, string> = {
  ready: "Ready",
  success: "Success",
  chime: "Chime",
  bloom: "Bloom",
  error: "Error",
  tick: "Tick",
  droplet: "Droplet",
};

let lastSettingsSection: SettingsSectionId = "app";

const SETTINGS_SECTIONS: SettingsSection[] = [
  {
    id: "app",
    label: "General",
    detail: "Window behavior and layout",
    icon: Sidebar,
  },
  {
    id: "chat",
    label: "Chat",
    detail: "Composer behavior and thread naming",
    icon: Pencil,
  },
  {
    id: "appearance",
    label: "Appearance",
    detail: "Themes and custom styles",
    icon: Sun,
  },
  {
    id: "models",
    label: "Models & agents",
    detail: "Defaults and unavailable-model behavior",
    icon: Gear,
  },
  {
    id: "workspace",
    label: "Workspace",
    detail: "Openers, checkouts, and isolated worktrees",
    icon: FolderOpen,
  },
  {
    id: "history",
    label: "Data",
    detail: "Archives, exports, backup, and recovery",
    icon: Archive,
  },
  {
    id: "google",
    label: "Google Workspace",
    detail: "Connection and authorized Drive files",
    icon: FileText,
  },
  {
    id: "mobile",
    label: "Mobile",
    detail: "Phone companion relay and pairing",
    icon: Smartphone,
  },
  {
    id: "system",
    label: "System",
    detail: "Credential storage, shortcuts, and app commands",
    icon: Gear,
  },
  {
    id: "about",
    label: "About",
    detail: "Version and GitHub release updates",
    icon: GitLogo,
  },
  {
    id: "developer",
    label: "Developer",
    detail: "Developer-only setup and experimental controls",
    icon: Code,
  },
];

const SETTINGS_SECTION_GROUPS: Array<{ label: string; sections: SettingsSectionId[] }> = [
  { label: "Preferences", sections: ["app", "chat", "appearance"] },
  { label: "Workflows", sections: ["models", "workspace"] },
  { label: "Data & devices", sections: ["history", "google", "mobile"] },
  { label: "Application", sections: ["system", "about", "developer"] },
];
function onboardingSetupLabel(value: string | null): string {
  if (value === "local_detect") return "Local detection";
  if (value === "hosted") return "Hosted provider";
  return "Not chosen";
}

function timestampLabel(value: number | undefined): string {
  return value ? new Date(value).toLocaleString() : "Never";
}

function archiveDeleteLabel(archivedAt: number | undefined, retentionDays: ArchiveRetentionDays): string {
  if (!archivedAt) return "Not scheduled";
  return new Date(archivedAt + retentionDays * DAY_MS).toLocaleDateString();
}

function folderLabel(folder: string): string {
  return folder.split(/[\\/]/).filter(Boolean).pop() || folder || "Project";
}

function googleFileTypeLabel(mimeType: string): string {
  if (mimeType.endsWith(".spreadsheet")) return "Sheet";
  if (mimeType.endsWith(".document")) return "Doc";
  if (mimeType.endsWith(".presentation")) return "Slides";
  if (mimeType.endsWith(".folder")) return "Folder";
  return mimeType.split("/").pop() || "Drive file";
}

function updateStatusLabel(status: UpdateStatus): string {
  if (status === "checking") return "Checking";
  if (status === "up-to-date") return "Up to date";
  if (status === "available") return "Available";
  if (status === "downloading") return "Downloading";
  if (status === "ready") return "Ready";
  if (status === "installing") return "Installing";
  if (status === "disabled") return "Disabled";
  if (status === "error") return "Error";
  return "Not checked";
}

export function SettingsDialog({ onClose }: { onClose: () => void }) {
  const themes = useTheme((s) => s.themes);
  const custom = useTheme((s) => s.custom);
  const themeId = useTheme((s) => s.themeId);
  const current = useTheme((s) => s.theme);
  const activeBackgroundImage = current.background.image?.trim() ? current.background.image : undefined;
  const setTheme = useTheme((s) => s.setTheme);
  const sidebarOpen = useUiPreferences((s) => s.sidebarOpen);
  const sidebarWidth = useUiPreferences((s) => s.sidebarWidth);
  const previewPanelWidth = useUiPreferences((s) => s.previewPanelWidth);
  const uiSize = useUiPreferences((s) => s.uiSize);
  const showAccountUsageInTitleBar = useUiPreferences((s) => s.showAccountUsageInTitleBar);
  const windowAlwaysOnTop = useUiPreferences((s) => s.windowAlwaysOnTop);
  const interfaceSounds = useUiPreferences((s) => s.interfaceSounds);
  const soundOnFinished = useUiPreferences((s) => s.soundOnFinished);
  const soundOnAttention = useUiPreferences((s) => s.soundOnAttention);
  const soundOnInteractions = useUiPreferences((s) => s.soundOnInteractions);
  const finishedSound = useUiPreferences((s) => s.finishedSound);
  const attentionSound = useUiPreferences((s) => s.attentionSound);
  const composerSendShortcut = useUiPreferences((s) => s.composerSendShortcut);
  const composerDensity = useUiPreferences((s) => s.composerDensity);
  const autoTitleChats = useUiPreferences((s) => s.autoTitleChats);
  const aiThreadNames = useUiPreferences((s) => s.aiThreadNames);
  const aiThreadNameModel = useUiPreferences((s) => s.aiThreadNameModel);
  const newChatButtonAtBottom = useUiPreferences((s) => s.newChatButtonAtBottom);
  const developerMode = useUiPreferences((s) => s.developerMode);
  const experimentalHashlinePatch = useUiPreferences((s) => s.experimentalHashlinePatch);
  const chatLayoutStyle = useUiPreferences((s) => s.chatLayoutStyle);
  const sidebarRailStyle = useUiPreferences((s) => s.sidebarRailStyle);
  const settledThreadsEnabled = useUiPreferences((s) => s.settledThreadsEnabled);
  const showEmptyChatRidgeline = useUiPreferences((s) => s.showEmptyChatRidgeline);
  const autoColorThreadNames = useUiPreferences((s) => s.autoColorThreadNames);
  const messageWidth = useUiPreferences((s) => s.messageWidth);
  const avatarStyle = useUiPreferences((s) => s.avatarStyle);
  const codeBlockTheme = useUiPreferences((s) => s.codeBlockTheme);
  const backgroundFit = useUiPreferences((s) => s.backgroundFit);
  const backgroundTreatment = useUiPreferences((s) => s.backgroundTreatment);
  const startupBehavior = useUiPreferences((s) => s.startupBehavior);
  const restoreOpenPanels = useUiPreferences((s) => s.restoreOpenPanels);
  const notifyRunFinished = useUiPreferences((s) => s.notifyRunFinished);
  const notifyNeedsAttention = useUiPreferences((s) => s.notifyNeedsAttention);
  const notifyOnlyWhenUnfocused = useUiPreferences((s) => s.notifyOnlyWhenUnfocused);
  const notificationIncludeThreadTitle = useUiPreferences((s) => s.notificationIncludeThreadTitle);
  const quickActionMode = useUiPreferences((s) => s.quickActionMode);
  const pinnedQuickActions = useUiPreferences((s) => s.pinnedQuickActions);
  const projectQuickActionOverrides = useUiPreferences((s) => s.projectQuickActionOverrides);
  const autocompleteMode = useUiPreferences((s) => s.autocompleteMode);
  const autocompleteSources = useUiPreferences((s) => s.autocompleteSources);
  const personalizedSuggestions = useUiPreferences((s) => s.personalizedSuggestions);
  const promptHistoryScope = useUiPreferences((s) => s.promptHistoryScope);
  const globalPromptHistory = useUiPreferences((s) => s.globalPromptHistory);
  const threadExportFormat = useUiPreferences((s) => s.threadExportFormat);
  const workspaceLauncherPreference = useUiPreferences((s) => s.workspaceLauncherPreference);
  const newProjectChatWorkspace = useUiPreferences((s) => s.newProjectChatWorkspace);
  const composerCompletionMode = useUiPreferences((s) => s.composerCompletionMode);
  const remoteCompletionConfirmed = useUiPreferences((s) => s.remoteCompletionConfirmed);
  const appShortcuts = useUiPreferences((s) => s.appShortcuts);
  const setSidebarOpen = useUiPreferences((s) => s.setSidebarOpen);
  const setUiSize = useUiPreferences((s) => s.setUiSize);
  const setShowAccountUsageInTitleBar = useUiPreferences((s) => s.setShowAccountUsageInTitleBar);
  const setWindowAlwaysOnTop = useUiPreferences((s) => s.setWindowAlwaysOnTop);
  const setInterfaceSounds = useUiPreferences((s) => s.setInterfaceSounds);
  const setSoundOnFinished = useUiPreferences((s) => s.setSoundOnFinished);
  const setSoundOnAttention = useUiPreferences((s) => s.setSoundOnAttention);
  const setSoundOnInteractions = useUiPreferences((s) => s.setSoundOnInteractions);
  const setFinishedSound = useUiPreferences((s) => s.setFinishedSound);
  const setAttentionSound = useUiPreferences((s) => s.setAttentionSound);
  const setComposerSendShortcut = useUiPreferences((s) => s.setComposerSendShortcut);
  const setComposerDensity = useUiPreferences((s) => s.setComposerDensity);
  const setAutoTitleChats = useUiPreferences((s) => s.setAutoTitleChats);
  const setAiThreadNames = useUiPreferences((s) => s.setAiThreadNames);
  const setAiThreadNameModel = useUiPreferences((s) => s.setAiThreadNameModel);
  const setNewChatButtonAtBottom = useUiPreferences((s) => s.setNewChatButtonAtBottom);
  const setDeveloperMode = useUiPreferences((s) => s.setDeveloperMode);
  const setExperimentalHashlinePatch = useUiPreferences((s) => s.setExperimentalHashlinePatch);
  const setChatLayoutStyle = useUiPreferences((s) => s.setChatLayoutStyle);
  const setSidebarRailStyle = useUiPreferences((s) => s.setSidebarRailStyle);
  const setSettledThreadsEnabled = useUiPreferences((s) => s.setSettledThreadsEnabled);
  const setShowEmptyChatRidgeline = useUiPreferences((s) => s.setShowEmptyChatRidgeline);
  const setAutoColorThreadNames = useUiPreferences((s) => s.setAutoColorThreadNames);
  const setMessageWidth = useUiPreferences((s) => s.setMessageWidth);
  const setAvatarStyle = useUiPreferences((s) => s.setAvatarStyle);
  const setCodeBlockTheme = useUiPreferences((s) => s.setCodeBlockTheme);
  const setBackgroundFit = useUiPreferences((s) => s.setBackgroundFit);
  const setBackgroundTreatment = useUiPreferences((s) => s.setBackgroundTreatment);
  const setStartupBehavior = useUiPreferences((s) => s.setStartupBehavior);
  const setRestoreOpenPanels = useUiPreferences((s) => s.setRestoreOpenPanels);
  const setNotifyRunFinished = useUiPreferences((s) => s.setNotifyRunFinished);
  const setNotifyNeedsAttention = useUiPreferences((s) => s.setNotifyNeedsAttention);
  const setNotifyOnlyWhenUnfocused = useUiPreferences((s) => s.setNotifyOnlyWhenUnfocused);
  const setNotificationIncludeThreadTitle = useUiPreferences((s) => s.setNotificationIncludeThreadTitle);
  const setQuickActionMode = useUiPreferences((s) => s.setQuickActionMode);
  const setPinnedQuickActions = useUiPreferences((s) => s.setPinnedQuickActions);
  const setProjectQuickActionOverride = useUiPreferences((s) => s.setProjectQuickActionOverride);
  const setAutocompleteMode = useUiPreferences((s) => s.setAutocompleteMode);
  const setAutocompleteSource = useUiPreferences((s) => s.setAutocompleteSource);
  const setPersonalizedSuggestions = useUiPreferences((s) => s.setPersonalizedSuggestions);
  const resetSuggestionHistory = useUiPreferences((s) => s.resetSuggestionHistory);
  const setPromptHistoryScope = useUiPreferences((s) => s.setPromptHistoryScope);
  const clearGlobalPromptHistory = useUiPreferences((s) => s.clearGlobalPromptHistory);
  const setThreadExportFormat = useUiPreferences((s) => s.setThreadExportFormat);
  const setWorkspaceLauncherPreference = useUiPreferences((s) => s.setWorkspaceLauncherPreference);
  const setNewProjectChatWorkspace = useUiPreferences((s) => s.setNewProjectChatWorkspace);
  const setComposerCompletionMode = useUiPreferences((s) => s.setComposerCompletionMode);
  const setRemoteCompletionConfirmed = useUiPreferences((s) => s.setRemoteCompletionConfirmed);
  const resetLayoutWidths = useUiPreferences((s) => s.resetLayoutWidths);
  const setAppShortcut = useUiPreferences((s) => s.setAppShortcut);
  const resetAppShortcuts = useUiPreferences((s) => s.resetAppShortcuts);
  const onboardingStatus = useOnboarding((s) => s.status);
  const onboardingSetupPath = useOnboarding((s) => s.selectedSetupPath);
  const onboardingDeveloperShow = useOnboarding((s) => s.developerShowOnboarding);
  const onboardingCompletedAt = useOnboarding((s) => s.completedAt);
  const onboardingDismissedAt = useOnboarding((s) => s.dismissedAt);
  const setDeveloperShowOnboarding = useOnboarding((s) => s.setDeveloperShowOnboarding);
  const completeOnboarding = useOnboarding((s) => s.complete);
  const resetOnboarding = useOnboarding((s) => s.reset);
  const updateCurrentVersion = useUpdateStore((s) => s.currentVersion);
  const updateStatus = useUpdateStore((s) => s.status);
  const updateInfo = useUpdateStore((s) => s.updateInfo);
  const updatePath = useUpdateStore((s) => s.updatePath);
  const updateProgress = useUpdateStore((s) => s.downloadProgress);
  const updateError = useUpdateStore((s) => s.error);
  const updateLastCheckedAt = useUpdateStore((s) => s.lastCheckedAt);
  const loadCurrentVersion = useUpdateStore((s) => s.loadCurrentVersion);
  const checkForAppUpdate = useUpdateStore((s) => s.checkNow);
  const downloadAppUpdate = useUpdateStore((s) => s.downloadNow);
  const installAppUpdate = useUpdateStore((s) => s.installNow);
  const automaticCheck = useUpdateStore((s) => s.automaticCheck);
  const automaticDownload = useUpdateStore((s) => s.automaticDownload);
  const setAutomaticCheck = useUpdateStore((s) => s.setAutomaticCheck);
  const setAutomaticDownload = useUpdateStore((s) => s.setAutomaticDownload);
  const agents = useAgents((s) => s.agents);
  const newThreadBehavior = useSettings((s) => s.newThreadBehavior);
  const configuredThreadDefaults = useSettings((s) => s.configuredThreadDefaults);
  const unavailableModelPolicy = useSettings((s) => s.unavailableModelPolicy);
  const browserStorageMode = useSettings((s) => s.browserStorageMode);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const setNewThreadBehavior = useSettings((s) => s.setNewThreadBehavior);
  const setConfiguredThreadDefaults = useSettings((s) => s.setConfiguredThreadDefaults);
  const setUnavailableModelPolicy = useSettings((s) => s.setUnavailableModelPolicy);
  const setBrowserStorageMode = useSettings((s) => s.setBrowserStorageMode);
  const sessions = useSessions((s) => s.sessions);
  const projects = useSessions((s) => s.projects);
  const archiveRetentionDays = useSessions((s) => s.archiveRetentionDays);
  const setArchiveRetentionDays = useSessions((s) => s.setArchiveRetentionDays);
  const restoreSession = useSessions((s) => s.restoreSession);
  const removeSession = useSessions((s) => s.remove);
  const restoreProject = useSessions((s) => s.restoreProject);
  const removeProject = useSessions((s) => s.removeProject);
  const purgeExpiredArchives = useSessions((s) => s.purgeExpiredArchives);
  const activeFolder = useSessions((s) => s.sessions.find((session) => session.id === s.activeId)?.settings?.folder ?? "");
  const refreshAgents = useAgents((s) => s.refresh);

  const [editing, setEditing] = useState<{ base: Theme; isNew: boolean } | null>(null);
  const [activeSection, setActiveSection] = useState<SettingsSectionId>(lastSettingsSection);
  const [settingsQuery, setSettingsQuery] = useState("");
  const [highlightedSettingId, setHighlightedSettingId] = useState<string | null>(null);
  const [confirmArchiveDelete, setConfirmArchiveDelete] = useState<string | null>(null);
  const [threadNameModels, setThreadNameModels] = useState<ModelInfo[]>([]);
  const [catalogModels, setCatalogModels] = useState<ModelInfo[]>([]);
  const [workspaceLaunchers, setWorkspaceLaunchers] = useState<WorkspaceLauncher[]>([]);
  const [notificationError, setNotificationError] = useState<string | null>(null);
  const [backupStatus, setBackupStatus] = useState<string | null>(null);
  const [backupBusy, setBackupBusy] = useState(false);
  const [browserDataStatus, setBrowserDataStatus] = useState<string | null>(null);
  const [browserDataBusy, setBrowserDataBusy] = useState(false);
  const [confirmBrowserDataClear, setConfirmBrowserDataClear] = useState(false);
  const [googleWorkspace, setGoogleWorkspace] = useState<GoogleWorkspaceStatus | null>(null);
  const [googleWorkspaceBusy, setGoogleWorkspaceBusy] = useState(false);
  const [googleWorkspaceMessage, setGoogleWorkspaceMessage] = useState<string | null>(null);
  const [confirmGoogleConnect, setConfirmGoogleConnect] = useState(false);
  const [confirmGoogleDisconnect, setConfirmGoogleDisconnect] = useState(false);
  const [googleRevocationUnconfirmed, setGoogleRevocationUnconfirmed] = useState(false);
  const [secretStorage, setSecretStorage] = useState<SecretStorageStatus | null>(null);
  const [selectedBuiltinAction, setSelectedBuiltinAction] = useState(BUILTIN_QUICK_ACTIONS[0]?.id ?? "");
  const [editProjectQuickActions, setEditProjectQuickActions] = useState(false);
  const [recordingShortcut, setRecordingShortcut] = useState<ShortcutRecordingTarget | null>(null);
  const [shortcutError, setShortcutError] = useState<string | null>(null);
  const [diagnosticsError, setDiagnosticsError] = useState<string | null>(null);
  const archivedSessions = useMemo(
    () => sessions.filter((session) => session.archivedAt).slice().sort((a, b) => (b.archivedAt ?? 0) - (a.archivedAt ?? 0)),
    [sessions],
  );
  const archivedProjects = useMemo(
    () => projects.filter((project) => project.archivedAt).slice().sort((a, b) => (b.archivedAt ?? 0) - (a.archivedAt ?? 0)),
    [projects],
  );
  const archivedCount = archivedSessions.length + archivedProjects.length;
  const sectionBadges: Partial<Record<SettingsSectionId, SettingsBadge>> = {
    ...(archivedCount ? { history: { label: `${archivedCount} archived`, tone: "neutral" } } : {}),
    ...(updateStatus === "available"
      ? { about: { label: "Update available", tone: "warn" } }
      : updateStatus === "ready"
        ? { about: { label: "Restart to update", tone: "warn" } }
        : updateStatus === "error"
          ? { about: { label: "Update error", tone: "warn" } }
          : {}),
  };
  const settingsSearchResults = useMemo(
    () => matchingSettingsEntries(settingsQuery),
    [settingsQuery],
  );

  useEffect(() => {
    void loadCurrentVersion();
  }, [loadCurrentVersion]);

  useEffect(() => {
    let cancelled = false;
    listModelsDetailed(accountRuntimeEnabled)
      .then((items) => {
        if (!cancelled) {
          setCatalogModels(items);
          setThreadNameModels(items.filter(isThreadNamingModel));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    void refreshAgents().catch(() => {});
  }, [refreshAgents]);

  useEffect(() => {
    void refreshGoogleWorkspace();
    void getSecretStorageStatus()
      .then(setSecretStorage)
      .catch(() => setSecretStorage({
        mode: "unavailable",
        detail: "Credential-storage status could not be read.",
      }));
  }, []);

  useEffect(() => {
    if (!activeFolder.trim()) {
      setWorkspaceLaunchers([]);
      return;
    }
    let cancelled = false;
    void listWorkspaceLaunchers(activeFolder)
      .then((items) => {
        if (!cancelled) setWorkspaceLaunchers(items);
      })
      .catch(() => {
        if (!cancelled) setWorkspaceLaunchers([]);
      });
    return () => { cancelled = true; };
  }, [activeFolder]);

  useEffect(() => {
    if (!recordingShortcut) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => recordAppShortcut(recordingShortcut, event);
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [appShortcuts, recordingShortcut]);

  useEffect(() => {
    if (!confirmArchiveDelete) return;
    const timer = window.setTimeout(() => setConfirmArchiveDelete(null), 3000);
    return () => window.clearTimeout(timer);
  }, [confirmArchiveDelete]);

  useEffect(() => {
    if (!highlightedSettingId) return;
    const timer = window.setTimeout(() => setHighlightedSettingId(null), 1600);
    return () => window.clearTimeout(timer);
  }, [highlightedSettingId]);

  if (editing) {
    return <ThemeEditor base={editing.base} isNew={editing.isNew} onClose={() => setEditing(null)} />;
  }

  const threadNameModelOptions = [
    { value: "", label: "Use chat model" },
    ...threadNameModels.map((item) => ({ value: item.id, label: item.id })),
  ];
  if (aiThreadNameModel && isThreadNamingModel(aiThreadNameModel) && !threadNameModelOptions.some((option) => option.value === aiThreadNameModel)) {
    threadNameModelOptions.push({ value: aiThreadNameModel, label: aiThreadNameModel });
  }

  const customIds = new Set(custom.map((c) => c.id));
  const updateBusy = updateStatus === "checking" || updateStatus === "downloading" || updateStatus === "installing";
  const canCheckForUpdate = !updateBusy;
  const canDownloadUpdate = updateStatus === "available" && !!updateInfo;
  const canInstallUpdate = updateStatus === "ready" && !!updatePath;
  const latestVersionLabel = updateInfo ? `v${updateInfo.version}` : "Not checked";
  const currentVersionLabel = updateCurrentVersion ? `v${updateCurrentVersion}` : "Unknown";
  const archiveRetentionValue = String(archiveRetentionDays) as "7" | "14" | "30";
  const projectNameByFolder = new Map(projects.map((project) => [project.folder, project.name]));
  const activeSettingsSection = SETTINGS_SECTIONS.find((section) => section.id === activeSection) ?? SETTINGS_SECTIONS[0];
  const activeProject = projects.find((project) => project.folder === activeFolder);
  const editablePinnedActions = editProjectQuickActions && activeProject
    ? projectQuickActionOverrides[activeProject.id] ?? pinnedQuickActions
    : pinnedQuickActions;
  const modelOptions = [
    { value: "", label: "Choose when starting" },
    ...catalogModels.map((item) => ({ value: item.id, label: item.display_id || item.id })),
  ];
  const agentOptions = [
    { value: "", label: "Default chat" },
    ...agents.map((agent) => ({ value: agent.id, label: agent.name })),
  ];
  const launcherOptions = [
    { value: "remember", label: "Remember per project" },
    ...workspaceLaunchers.filter((launcher) => launcher.available).map((launcher) => ({ value: launcher.id, label: launcher.label })),
  ];

  async function setNotificationPreference(kind: "finished" | "attention", enabled: boolean) {
    setNotificationError(null);
    if (!enabled) {
      if (kind === "finished") setNotifyRunFinished(false);
      else setNotifyNeedsAttention(false);
      return;
    }
    try {
      if (!(await ensureNativeNotificationPermission())) {
        setNotificationError("Notification permission was denied. The setting remains off.");
        return;
      }
      if (kind === "finished") setNotifyRunFinished(true);
      else setNotifyNeedsAttention(true);
    } catch (error) {
      setNotificationError(error instanceof Error ? error.message : String(error));
    }
  }

  function updatePinnedAction(id: string, patch: Partial<PinnedQuickAction>) {
    savePinnedActions(editablePinnedActions.map((action) => action.id === id ? { ...action, ...patch } : action));
  }

  function savePinnedActions(actions: PinnedQuickAction[]) {
    if (editProjectQuickActions && activeProject) setProjectQuickActionOverride(activeProject.id, actions);
    else setPinnedQuickActions(actions);
  }

  function addBuiltinQuickAction() {
    const builtin = BUILTIN_QUICK_ACTIONS.find((action) => action.id === selectedBuiltinAction);
    if (!builtin || editablePinnedActions.length >= 3 || editablePinnedActions.some((action) => action.id === builtin.id)) return;
    savePinnedActions([...editablePinnedActions, {
      id: builtin.id,
      builtinId: builtin.id,
      label: builtin.label,
      prompt: builtin.prompt,
    }]);
  }

  function addCustomQuickAction() {
    if (editablePinnedActions.length >= 3) return;
    savePinnedActions([...editablePinnedActions, {
      id: `custom-${Date.now()}`,
      label: "Custom action",
      prompt: "Describe what you want Milim to help with.",
    }]);
  }

  function movePinnedAction(index: number, direction: -1 | 1) {
    const target = index + direction;
    if (target < 0 || target >= editablePinnedActions.length) return;
    const next = editablePinnedActions.slice();
    [next[index], next[target]] = [next[target], next[index]];
    savePinnedActions(next);
  }

  function updateConfiguredDefaults(patch: Partial<ConfiguredThreadDefaults>) {
    setConfiguredThreadDefaults(patch);
  }

  async function exportBackupFromSettings() {
    setBackupStatus(null);
    setBackupBusy(true);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: `milim-${new Date().toISOString().slice(0, 10)}.milim-backup.json`, filters: [{ name: "Milim backup", extensions: ["json"] }] });
      if (!path) return;
      await flushDeferredUserStateWrites();
      const result = await exportMilimBackup(path);
      setBackupStatus(`Exported ${result.summary.chats} chats and ${result.summary.projects} projects.`);
    } catch (error) {
      setBackupStatus(`Export failed: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBackupBusy(false);
    }
  }

  async function restoreBackupFromSettings() {
    setBackupStatus(null);
    setBackupBusy(true);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ multiple: false, directory: false, filters: [{ name: "Milim backup", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      const inspection = await inspectMilimBackup(path);
      const accepted = window.confirm(`Replace local Milim data with this v${inspection.appVersion} backup containing ${inspection.summary.chats} chats and ${inspection.summary.projects} projects? A recovery snapshot will be created first.`);
      if (!accepted) return;
      await flushDeferredUserStateWrites();
      const recoveryPath = await restoreMilimBackup(path);
      setBackupStatus(`Restore completed. Recovery snapshot: ${recoveryPath}`);
      await restartDesktopApp();
    } catch (error) {
      setBackupStatus(`Restore failed: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBackupBusy(false);
    }
  }

  async function clearBrowserDataFromSettings() {
    if (!confirmBrowserDataClear) {
      setConfirmBrowserDataClear(true);
      setBrowserDataStatus(null);
      return;
    }
    setBrowserDataBusy(true);
    setBrowserDataStatus(null);
    try {
      await flushDeferredUserStateWrites();
      await clearPreviewWebviewData();
      setBrowserDataStatus("Browser sign-ins and site data cleared.");
    } catch (error) {
      setBrowserDataStatus(`Clear failed: ${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setBrowserDataBusy(false);
      setConfirmBrowserDataClear(false);
    }
  }

  async function refreshGoogleWorkspace() {
    try {
      setGoogleWorkspace(await getGoogleWorkspaceStatus());
    } catch (error) {
      setGoogleWorkspace(null);
      setGoogleWorkspaceMessage(
        `Google Workspace status failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }

  async function chooseGoogleFilesFromSettings() {
    setGoogleRevocationUnconfirmed(false);
    if (!googleWorkspace?.connected && !confirmGoogleConnect) {
      setConfirmGoogleConnect(true);
      setGoogleWorkspaceMessage(null);
      return;
    }
    setGoogleWorkspaceBusy(true);
    setGoogleWorkspaceMessage("Finish choosing files in your browser.");
    try {
      const flow = await chooseGoogleWorkspaceFiles();
      setGoogleWorkspaceMessage(
        flow.files.length
          ? `Added ${flow.files.length} Google ${flow.files.length === 1 ? "file" : "files"}.`
          : "No files were added.",
      );
      await refreshGoogleWorkspace();
    } catch (error) {
      setGoogleWorkspaceMessage(
        `Google connection failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setGoogleWorkspaceBusy(false);
      setConfirmGoogleConnect(false);
    }
  }

  async function removeGoogleFileFromSettings(id: string) {
    setGoogleWorkspaceBusy(true);
    try {
      await removeGoogleWorkspaceFile(id);
      setGoogleWorkspaceMessage(GOOGLE_REMOVE_MESSAGE);
      await refreshGoogleWorkspace();
    } catch (error) {
      setGoogleWorkspaceMessage(
        `Remove failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setGoogleWorkspaceBusy(false);
    }
  }

  async function disconnectGoogleFromSettings() {
    if (!confirmGoogleDisconnect) {
      setConfirmGoogleDisconnect(true);
      return;
    }
    setGoogleWorkspaceBusy(true);
    try {
      const result = await disconnectGoogleWorkspace();
      setGoogleWorkspaceMessage(googleDisconnectMessage(result.revocation));
      setGoogleRevocationUnconfirmed(result.revocation === "unconfirmed");
      setConfirmGoogleDisconnect(false);
      await refreshGoogleWorkspace();
    } catch (error) {
      setGoogleWorkspaceMessage(
        `Disconnect failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setGoogleWorkspaceBusy(false);
    }
  }

  function openGoogleFileInMilim(url: string) {
    window.dispatchEvent(
      new CustomEvent("milim-open-browser-url", { detail: { url } }),
    );
    onClose();
  }

  function setArchiveRetentionFromSettings(value: "7" | "14" | "30") {
    setArchiveRetentionDays(Number(value) as ArchiveRetentionDays);
    useSessions.getState().purgeExpiredArchives();
  }

  function archivedSessionProjectLabel(session: Session): string {
    const folder = session.settings?.folder?.trim() ?? "";
    return folder ? projectNameByFolder.get(folder) ?? folderLabel(folder) : "Chats";
  }

  function projectThreadCount(project: Project): number {
    return sessions.filter((session) => !session.parentId && session.settings?.folder?.trim() === project.folder).length;
  }

  function deleteArchivedSession(id: string) {
    const key = `session:${id}`;
    if (confirmArchiveDelete !== key) {
      setConfirmArchiveDelete(key);
      return;
    }
    removeSession(id);
    setConfirmArchiveDelete(null);
  }

  function deleteArchivedProject(id: string) {
    const key = `project:${id}`;
    if (confirmArchiveDelete !== key) {
      setConfirmArchiveDelete(key);
      return;
    }
    removeProject(id);
    setConfirmArchiveDelete(null);
  }

  function purgeExpiredArchivesFromSettings() {
    purgeExpiredArchives();
    setConfirmArchiveDelete(null);
  }

  async function checkUpdatesFromSettings() {
    await checkForAppUpdate();
  }

  async function downloadUpdateFromSettings() {
    await downloadAppUpdate();
  }

  async function installUpdateFromSettings() {
    await installAppUpdate();
  }

  async function openLogsFromSettings() {
    setDiagnosticsError(null);
    try {
      await openDiagnosticsFolder();
    } catch (error) {
      setDiagnosticsError(error instanceof Error ? error.message : String(error));
    }
  }

  function recordAppShortcut(target: ShortcutRecordingTarget, event: globalThis.KeyboardEvent) {
    event.preventDefault();
    event.stopPropagation();
    const shortcut = shortcutFromKeyboardEvent(event);
    if (!shortcut) {
      setShortcutError("Use a modifier, Escape, or an F-key.");
      return;
    }
    const issue = shortcutValidationIssue(shortcut);
    if (issue) {
      setShortcutError(issue);
      return;
    }
    const conflict = shortcutConflict(appShortcuts, target, shortcut);
    if (conflict) {
      setShortcutError(`${shortcutLabel(shortcut)} is already used by ${APP_SHORTCUT_LABELS[conflict]}.`);
      return;
    }
    if (!setAppShortcut(target, shortcut)) {
      setShortcutError("Shortcut could not be saved.");
      return;
    }
    setRecordingShortcut(null);
    setShortcutError(null);
  }

  function startRecordingShortcut(target: ShortcutRecordingTarget) {
    setRecordingShortcut((current) => current === target ? null : target);
    setShortcutError(null);
  }

  function activateSettingsSection(sectionId: SettingsSectionId, options: SettingsSectionActivation = {}) {
    const { focusTab = false, remember = true } = options;
    setActiveSection(sectionId);
    if (remember) {
      lastSettingsSection = sectionId;
    }
    if (focusTab) {
      window.requestAnimationFrame(() => {
        document.getElementById(`settings-tab-${sectionId}`)?.focus({ preventScroll: true });
      });
    }
  }

  function openSettingSearchResult(entry: SettingSearchEntry) {
    activateSettingsSection(entry.section);
    setSettingsQuery("");
    setHighlightedSettingId(entry.id);
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        const target = document.querySelector<HTMLElement>(`[data-setting-id="${entry.id}"]`);
        target?.scrollIntoView({ block: "center", behavior: "smooth" });
        const focusable = target?.querySelector<HTMLElement>("button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex='-1'])");
        (focusable ?? target)?.focus({ preventScroll: true });
      });
    });
  }

  function selectSettingsSection(sectionId: SettingsSectionId, options: SettingsSectionActivation = {}) {
    activateSettingsSection(sectionId, options);
  }

  function onSettingsNavKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    const currentIndex = SETTINGS_SECTIONS.findIndex((section) => section.id === activeSection);
    if (currentIndex < 0) return;

    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % SETTINGS_SECTIONS.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + SETTINGS_SECTIONS.length) % SETTINGS_SECTIONS.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = SETTINGS_SECTIONS.length - 1;
    }

    if (nextIndex == null) return;
    event.preventDefault();
    selectSettingsSection(SETTINGS_SECTIONS[nextIndex].id, { focusTab: true });
  }

  const settingHighlightClass = (id: string) => highlightedSettingId === id ? " setting-highlight" : "";

  return (
    <SheetDialog title="Settings" className="sheet" testId="settings-dialog" onClose={onClose}>
        <div className="sheet-header">
          <h2>Settings</h2>
          <button
            className="icon-btn sheet-close"
            data-testid="close-settings"
            onClick={onClose}
            title="Close"
            aria-label="Close settings"
          >
            <X size={15} />
          </button>
        </div>

        <div className="settings-layout">
          <nav className="settings-nav" aria-label="Settings sections">
            <div className="settings-nav-search">
              <Search size={14} aria-hidden="true" />
              <input
                data-testid="settings-search"
                type="search"
                value={settingsQuery}
                onChange={(event) => setSettingsQuery(event.currentTarget.value)}
                placeholder="Search settings"
                aria-label="Search settings"
              />
              {settingsQuery ? (
                <button
                  className="settings-nav-search-clear"
                  type="button"
                  onClick={() => setSettingsQuery("")}
                  title="Clear search"
                  aria-label="Clear settings search"
                >
                  <X size={12} />
                </button>
              ) : null}
            </div>
            {settingsQuery.trim() ? (
              <div className="settings-search-results" aria-label="Matching settings">
                {settingsSearchResults.length ? settingsSearchResults.map((entry) => {
                  const section = SETTINGS_SECTIONS.find((item) => item.id === entry.section);
                  return (
                    <button
                      key={entry.id}
                      className="settings-search-result"
                      type="button"
                      onClick={() => openSettingSearchResult(entry)}
                    >
                      <span>{entry.label}</span>
                      <small>{section?.label ?? entry.section}</small>
                    </button>
                  );
                }) : <div className="settings-nav-empty">No settings match.</div>}
              </div>
            ) : (
              <div className="settings-nav-list" role="tablist" aria-label="Settings sections">
                {SETTINGS_SECTION_GROUPS.map((group) => (
                  <Fragment key={group.label}>
                    <div className="settings-nav-group">{group.label}</div>
                    {group.sections.map((sectionId) => {
                      const section = SETTINGS_SECTIONS.find((item) => item.id === sectionId)!;
                      const Icon = section.icon;
                      const selected = activeSection === section.id;
                      const badge = sectionBadges[section.id];
                      return (
                        <button
                          key={section.id}
                          id={`settings-tab-${section.id}`}
                          className={"settings-nav-item" + (selected ? " active" : "")}
                          type="button"
                          role="tab"
                          data-testid={`settings-section-${section.id}`}
                          aria-selected={selected}
                          aria-controls={`settings-panel-${section.id}`}
                          tabIndex={selected ? 0 : -1}
                          onClick={() => selectSettingsSection(section.id)}
                          onKeyDown={onSettingsNavKeyDown}
                        >
                          <span className="settings-nav-icon" aria-hidden="true">
                            {section.id === "google" ? <span className="settings-google-nav-icon" /> : <Icon size={15} />}
                          </span>
                          <span className="settings-nav-copy">
                            <span className="settings-nav-label">{section.label}</span>
                          </span>
                          {badge ? (
                            <span
                              className={`settings-nav-status ${badge.tone}`}
                              aria-label={`${section.label}: ${badge.label}`}
                              title={badge.label}
                            >
                              {badge.label}
                            </span>
                          ) : null}
                        </button>
                      );
                    })}
                  </Fragment>
                ))}
              </div>
            )}
          </nav>

          <div className="settings-detail">
            <div className="settings-detail-head">
              <div>
                <h3>{activeSettingsSection.label}</h3>
                <p>{activeSettingsSection.detail}</p>
              </div>
            </div>

            <div className="settings-content">
            {activeSection === "app" && (
        <section className="settings-section" id="settings-panel-app" role="tabpanel" aria-labelledby="settings-tab-app" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Startup" data-setting-id="app-startup" className={settingHighlightClass("app-startup").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup
                  value={startupBehavior}
                  onChange={setStartupBehavior}
                  testIdPrefix="startup-behavior"
                  options={[
                    { value: "restore", label: "Restore last chat", detail: "Return to the chat that was open." },
                    { value: "new-chat", label: "Open a new chat", detail: "Start fresh using your new-chat policy." },
                  ]}
                />
                <div className="setting-toggle-row">
                  <div><strong>Restore open panels</strong><span>Remember context and preview panel state between launches.</span></div>
                  <Toggle checked={restoreOpenPanels} onChange={setRestoreOpenPanels} ariaLabel="Restore open panels" />
                </div>
              </div>
            </SettingsBlock>
            <SettingsBlock title="Window & layout" data-setting-id="app-window-layout" className={settingHighlightClass("app-window-layout").trim()}>
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div>
                    <strong>Keep window on top</strong>
                    <span>Pin Milim above other windows and remember that choice.</span>
                  </div>
                  <Toggle
                    checked={windowAlwaysOnTop}
                    onChange={setWindowAlwaysOnTop}
                    ariaLabel="Keep window on top"
                    testId="general-always-on-top-toggle"
                  />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>Open sidebar</strong>
                    <span>Keep the chat list visible by default.</span>
                  </div>
                  <Toggle checked={sidebarOpen} onChange={setSidebarOpen} ariaLabel="Open sidebar" testId="general-sidebar-open-toggle" />
                </div>
                <div className="setting-field">
                  <div className="settings-action-row">
                    <div>
                      <strong>UI size</strong>
                      <span>Scale the whole app to {uiSize}%.</span>
                    </div>
                    <button className="btn-ghost" type="button" data-testid="general-reset-ui-size" disabled={uiSize === DEFAULT_UI_SIZE} onClick={() => setUiSize(DEFAULT_UI_SIZE)}>
                      <Refresh size={13} />
                      Reset
                    </button>
                  </div>
                  <Slider ariaLabel="UI size" min={MIN_UI_SIZE} max={MAX_UI_SIZE} step={UI_SIZE_STEP} value={uiSize} onChange={setUiSize} />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>Show account usage in title bar</strong>
                    <span>Show compact quota details for the active Codex or Claude runtime.</span>
                  </div>
                  <Toggle checked={showAccountUsageInTitleBar} onChange={setShowAccountUsageInTitleBar} ariaLabel="Show account usage in title bar" testId="general-titlebar-account-usage-toggle" />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>New chat at bottom</strong>
                    <span>Anchor the new chat button above the sidebar footer.</span>
                  </div>
                  <Toggle checked={newChatButtonAtBottom} onChange={setNewChatButtonAtBottom} ariaLabel="New chat at bottom" testId="general-new-chat-bottom-toggle" />
                </div>
                <div className="settings-action-row">
                  <div>
                    <strong>Panel widths</strong>
                    <span>
                      Sidebar {sidebarWidth}px / Preview {previewPanelWidth}px · Defaults {DEFAULT_SIDEBAR_WIDTH}px / {DEFAULT_PREVIEW_PANEL_WIDTH}px
                    </span>
                  </div>
                  <button className="btn-ghost" type="button" data-testid="general-reset-layout" onClick={resetLayoutWidths}>
                    Reset
                  </button>
                </div>
              </div>
            </SettingsBlock>
            <SettingsBlock title="Notifications" data-setting-id="app-notifications" className={settingHighlightClass("app-notifications").trim()}>
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div><strong>Run finished</strong><span>Show a native notification after a run and its queue finish.</span></div>
                  <Toggle checked={notifyRunFinished} onChange={(enabled) => void setNotificationPreference("finished", enabled)} ariaLabel="Run finished notifications" />
                </div>
                <div className="setting-toggle-row">
                  <div><strong>Needs attention</strong><span>Notify for approvals, proposed workers, and terminal errors.</span></div>
                  <Toggle checked={notifyNeedsAttention} onChange={(enabled) => void setNotificationPreference("attention", enabled)} ariaLabel="Needs attention notifications" />
                </div>
                <div className="setting-toggle-row">
                  <div><strong>Only when Milim is unfocused</strong><span>Suppress native notifications while you are using the app.</span></div>
                  <Toggle checked={notifyOnlyWhenUnfocused} onChange={setNotifyOnlyWhenUnfocused} ariaLabel="Only notify when Milim is unfocused" />
                </div>
                <div className="setting-toggle-row">
                  <div><strong>Include thread title</strong><span>Otherwise notification text stays generic.</span></div>
                  <Toggle checked={notificationIncludeThreadTitle} onChange={setNotificationIncludeThreadTitle} ariaLabel="Include thread title in notifications" />
                </div>
                {notificationError ? <p className="sheet-hint error" role="alert">{notificationError}</p> : null}
              </div>
            </SettingsBlock>
            <SettingsBlock title="Update policy" data-setting-id="app-update-policy" className={settingHighlightClass("app-update-policy").trim()}>
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div><strong>Check automatically</strong><span>Check at startup and periodically while Milim is open.</span></div>
                  <Toggle checked={automaticCheck} onChange={setAutomaticCheck} ariaLabel="Check for updates automatically" />
                </div>
                <div className="setting-toggle-row">
                  <div><strong>Download automatically</strong><span>Download verified updates after an automatic check. Installation always remains manual.</span></div>
                  <Toggle checked={automaticDownload} onChange={setAutomaticDownload} ariaLabel="Download updates automatically" />
                </div>
              </div>
            </SettingsBlock>
            </SettingsPanel>
        </section>
            )}

            {activeSection === "chat" && (
        <section className="settings-section" id="settings-panel-chat" role="tabpanel" aria-labelledby="settings-tab-chat" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Composer" data-setting-id="chat-composer" className={settingHighlightClass("chat-composer").trim()}>
              <div className="setting-stack">
                <div className="setting-field">
                  <span className="setting-mini-title">Send shortcut</span>
                  <SettingsChoiceGroup<ComposerSendShortcut>
                    value={composerSendShortcut}
                    onChange={setComposerSendShortcut}
                    testIdPrefix="chat-send-shortcut"
                    options={[
                      { value: "enter", label: "Enter", detail: "Enter sends. Shift+Enter adds a line." },
                      { value: "modEnter", label: "Ctrl / Cmd+Enter", detail: "Enter adds lines. Modifier sends." },
                    ]}
                  />
                </div>
                <div className="setting-field">
                  <span className="setting-mini-title">Composer density</span>
                  <SettingsChoiceGroup<ComposerDensity>
                    value={composerDensity}
                    onChange={setComposerDensity}
                    testIdPrefix="chat-composer-density"
                    options={[
                      { value: "comfortable", label: "Comfortable", detail: "More breathing room for normal drafting." },
                      { value: "compact", label: "Compact", detail: "Tighter composer for small screens." },
                    ]}
                  />
                </div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="New chats" data-setting-id="chat-new-thread" className={settingHighlightClass("chat-new-thread").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup
                  value={newThreadBehavior}
                  onChange={setNewThreadBehavior}
                  testIdPrefix="new-thread-behavior"
                  options={[
                    { value: "inherit", label: "Inherit current chat", detail: "Keep the current model and safety settings." },
                    { value: "configured", label: "Use configured defaults", detail: "Apply the defaults below to new chats." },
                  ]}
                />
                {newThreadBehavior === "configured" ? (
                  <>
                    <div className="setting-toggle-row"><div><strong>Memory</strong><span>Enable scoped memory for configured new chats.</span></div><Toggle checked={configuredThreadDefaults.memory} onChange={(memory) => updateConfiguredDefaults({ memory })} ariaLabel="Default memory" /></div>
                    <div className="setting-toggle-row"><div><strong>Sandbox</strong><span>Enable sandbox tools for configured new chats.</span></div><Toggle checked={configuredThreadDefaults.sandbox} onChange={(sandbox) => updateConfiguredDefaults({ sandbox })} ariaLabel="Default sandbox" /></div>
                    <div className="setting-field"><span className="setting-mini-title">Privacy</span><SettingsChoiceGroup value={configuredThreadDefaults.privacy} onChange={(privacy) => updateConfiguredDefaults({ privacy })} testIdPrefix="default-privacy" options={[
                      { value: "off", label: "Off", detail: "Send prompts unchanged." },
                      { value: "redact", label: "Redact", detail: "Remove detected secrets." },
                      { value: "block", label: "Block", detail: "Stop risky outbound prompts." },
                    ]} /></div>
                    <div className="setting-field"><span className="setting-mini-title">Tool approval</span><SettingsChoiceGroup value={configuredThreadDefaults.toolApproval} onChange={(toolApproval) => updateConfiguredDefaults({ toolApproval })} testIdPrefix="default-approval" options={[
                      { value: "review", label: "Review", detail: "Approve each mutating action." },
                      { value: "guarded", label: "Guarded", detail: "Only read-only tools are available." },
                      { value: "open", label: "Open", detail: "Run without approval in trusted workspaces." },
                    ]} /></div>
                    <div className="setting-field"><span className="setting-mini-title">Delegation</span><SettingsChoiceGroup value={configuredThreadDefaults.delegationPolicy} onChange={(delegationPolicy) => updateConfiguredDefaults({ delegationPolicy })} testIdPrefix="default-delegation" options={[
                      { value: "off", label: "Off", detail: "Do not delegate." },
                      { value: "ask", label: "Ask", detail: "Pause on delegation proposals." },
                      { value: "auto", label: "Automatic", detail: "Allow eligible delegation." },
                    ]} /></div>
                    <p className="sheet-hint">Computer Use, Plan Mode, goals, temporary instructions, and running state always reset. Changing the workspace folder resets approval to Review.</p>
                  </>
                ) : null}
              </div>
            </SettingsBlock>

            <SettingsBlock title="Quick actions" data-setting-id="chat-quick-actions" className={settingHighlightClass("chat-quick-actions").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup value={quickActionMode} onChange={setQuickActionMode} testIdPrefix="quick-action-mode" options={[
                  { value: "smart", label: "Smart", detail: "Use current Git and workspace context." },
                  { value: "pinned", label: "Pinned only", detail: "Show your saved prompts." },
                  { value: "hidden", label: "Hidden", detail: "Hide empty-chat actions." },
                ]} />
                <div className="settings-action-row">
                  <div><strong>Editing {editProjectQuickActions && activeProject ? activeProject.name : "global"} actions</strong><span>Up to three actions; selecting one only prefills the composer.</span></div>
                  {activeProject ? <button type="button" className="btn-ghost" onClick={() => setEditProjectQuickActions((value) => !value)}>{editProjectQuickActions ? "Edit global" : "Edit project"}</button> : null}
                </div>
                {editProjectQuickActions && activeProject && Object.prototype.hasOwnProperty.call(projectQuickActionOverrides, activeProject.id) ? (
                  <div className="settings-action-row"><div><strong>Project override</strong><span>Remove it to inherit global actions again.</span></div><button type="button" className="btn-ghost" onClick={() => setProjectQuickActionOverride(activeProject.id, null)}>Use global</button></div>
                ) : null}
                {editablePinnedActions.map((action, index) => (
                  <div className="quick-action-editor" key={action.id}>
                    <input aria-label={`Quick action ${index + 1} label`} value={action.label} onChange={(event) => updatePinnedAction(action.id, { label: event.currentTarget.value })} />
                    <textarea aria-label={`Quick action ${index + 1} prompt`} value={action.prompt} onChange={(event) => updatePinnedAction(action.id, { prompt: event.currentTarget.value })} />
                    <div className="quick-action-editor-controls">
                      <button className="btn-ghost" type="button" aria-label={`Move ${action.label} up`} disabled={index === 0} onClick={() => movePinnedAction(index, -1)}>Up</button>
                      <button className="btn-ghost" type="button" aria-label={`Move ${action.label} down`} disabled={index === editablePinnedActions.length - 1} onClick={() => movePinnedAction(index, 1)}>Down</button>
                      <button className="btn-ghost danger" type="button" aria-label={`Delete ${action.label}`} onClick={() => savePinnedActions(editablePinnedActions.filter((item) => item.id !== action.id))}>Delete</button>
                    </div>
                  </div>
                ))}
                <div className="setting-field-action">
                  <Select value={selectedBuiltinAction} options={BUILTIN_QUICK_ACTIONS.map((action) => ({ value: action.id, label: action.label }))} onChange={setSelectedBuiltinAction} />
                  <button className="btn-ghost" type="button" disabled={editablePinnedActions.length >= 3} onClick={addBuiltinQuickAction}>Add built-in</button>
                  <button className="btn-ghost" type="button" disabled={editablePinnedActions.length >= 3} onClick={addCustomQuickAction}>Add custom</button>
                </div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="Autocomplete" data-setting-id="chat-autocomplete" className={settingHighlightClass("chat-autocomplete").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup value={autocompleteMode} onChange={setAutocompleteMode} testIdPrefix="autocomplete-mode" options={[
                  { value: "automatic", label: "Automatic", detail: "/ and @ open suggestions while typing." },
                  { value: "manual", label: "Manual", detail: "Use the button or shortcut." },
                  { value: "off", label: "Off", detail: "Typed slash commands still execute." },
                ]} />
                {(["commands", "files", "skills", "mcp"] as const).map((source) => (
                  <div className="setting-toggle-row" key={source}><div><strong>{source === "mcp" ? "MCP tools" : source[0].toUpperCase() + source.slice(1)}</strong><span>Include {source} in composer results.</span></div><Toggle checked={autocompleteSources[source]} onChange={(enabled) => setAutocompleteSource(source, enabled)} ariaLabel={`Autocomplete source ${source}`} /></div>
                ))}
                <div className="setting-toggle-row"><div><strong>Personalized ranking</strong><span>Use bounded local ID counts; file paths and prompt contents are never stored.</span></div><Toggle checked={personalizedSuggestions} onChange={setPersonalizedSuggestions} ariaLabel="Personalized suggestion ranking" /></div>
                <div className="settings-action-row"><div><strong>Suggestion history</strong><span>Clear local ranking counts for commands, skills, MCP tools, and quick actions.</span></div><button className="btn-ghost" type="button" onClick={resetSuggestionHistory}>Reset</button></div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="Prompt history" data-setting-id="chat-prompt-history" className={settingHighlightClass("chat-prompt-history").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup value={promptHistoryScope} onChange={setPromptHistoryScope} testIdPrefix="prompt-history" options={[
                  { value: "thread", label: "Current chat", detail: "Recall prompts from this chat." },
                  { value: "global", label: "Across chats", detail: "Keep 100 deduplicated prompts locally." },
                  { value: "off", label: "Off", detail: "Disable arrow-key prompt recall." },
                ]} />
                <div className="settings-action-row"><div><strong>Global prompt history</strong><span>{globalPromptHistory.length} of 100 prompts stored locally.</span></div><button className="btn-ghost" type="button" disabled={!globalPromptHistory.length} onClick={clearGlobalPromptHistory}>Clear</button></div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="AI composer completion" data-setting-id="chat-ai-completion" className={settingHighlightClass("chat-ai-completion").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup value={composerCompletionMode} onChange={(mode) => {
                  if (mode === "current" && !remoteCompletionConfirmed) {
                    const accepted = window.confirm("Remote completion sends only the current composer text to the selected provider and may incur cost. Enable it?");
                    if (!accepted) return;
                    setRemoteCompletionConfirmed(true);
                  }
                  setComposerCompletionMode(mode);
                }} testIdPrefix="composer-completion" options={[
                  { value: "off", label: "Off", detail: "No background completion requests." },
                  { value: "local", label: "Local providers", detail: "Only loopback provider endpoints." },
                  { value: "current", label: "Current provider model", detail: "Provider models only; explicit remote opt-in." },
                ]} />
                <p className="sheet-hint">Completion receives only current composer text, never chat history, memory, tools, files, or workspace contents. Tab accepts ghost text; Escape dismisses it.</p>
              </div>
            </SettingsBlock>

            <SettingsBlock title="Threads" data-setting-id="chat-threads" className={settingHighlightClass("chat-threads").trim()}>
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div>
                    <strong>Auto-title new chats</strong>
                    <span>Rename a new chat from the first user message.</span>
                  </div>
                  <Toggle checked={autoTitleChats} onChange={setAutoTitleChats} ariaLabel="Auto-title new chats" testId="chat-auto-title-toggle" />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>AI thread names</strong>
                    <span>After the first reply, ask a model for a 2-5 word name.</span>
                  </div>
                  <Toggle checked={aiThreadNames} onChange={setAiThreadNames} ariaLabel="AI thread names" testId="chat-ai-title-toggle" />
                </div>
                {aiThreadNames && (
                  <div className="setting-field">
                    <span className="setting-mini-title">Naming model</span>
                    <Select
                      value={aiThreadNameModel}
                      options={threadNameModelOptions}
                      onChange={setAiThreadNameModel}
                      placeholder="Use chat model"
                      testId="chat-ai-title-model"
                    />
                    <p className="sheet-hint">
                      {autoTitleChats ? "Leave empty to use compatible chat models. Choose a provider model for Codex, Claude, or media chats." : "Auto-title new chats is off."}
                    </p>
                  </div>
                )}
              </div>
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "models" && (
        <section className="settings-section" id="settings-panel-models" role="tabpanel" aria-labelledby="settings-tab-models" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock data-setting-id="models-defaults" className={settingHighlightClass("models-defaults").trim()}>
              <div className="setting-stack">
                <div className="setting-field"><span className="setting-mini-title">Default chat model</span><Select value={configuredThreadDefaults.model} options={modelOptions} onChange={(model) => updateConfiguredDefaults({ model })} /></div>
                <div className="setting-field"><span className="setting-mini-title">Default agent</span><Select value={configuredThreadDefaults.activeAgentId ?? ""} options={agentOptions} onChange={(activeAgentId) => updateConfiguredDefaults({ activeAgentId: activeAgentId || null })} /></div>
                <div className="setting-field"><span className="setting-mini-title">Default worker model</span><Select value={configuredThreadDefaults.workerModel} options={modelOptions} onChange={(workerModel) => updateConfiguredDefaults({ workerModel })} /></div>
                <div className="setting-field">
                  <span className="setting-mini-title">If a configured model is unavailable</span>
                  <SettingsChoiceGroup value={unavailableModelPolicy} onChange={setUnavailableModelPolicy} testIdPrefix="unavailable-model" options={[
                    { value: "ask", label: "Ask", detail: "Clear it and open the model picker." },
                    { value: "favorite", label: "First favorite", detail: "Use the first available ordered favorite." },
                    { value: "blocked", label: "Remain blocked", detail: "Keep it and show the setup error." },
                  ]} />
                </div>
                <p className="sheet-hint">Milim never silently selects an arbitrary non-favorite remote model.</p>
              </div>
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "workspace" && (
        <section className="settings-section" id="settings-panel-workspace" role="tabpanel" aria-labelledby="settings-tab-workspace" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Opener" data-setting-id="workspace-opener" className={settingHighlightClass("workspace-opener").trim()}>
              <div className="setting-field">
                <span className="setting-mini-title">Preferred opener</span>
                <Select value={workspaceLauncherPreference} options={launcherOptions} onChange={(value) => setWorkspaceLauncherPreference(value as typeof workspaceLauncherPreference)} />
                <p className="sheet-hint">{activeFolder ? `Detected launchers for ${folderLabel(activeFolder)}.` : "Open a project to detect installed launchers."} If the selected launcher disappears, Milim uses its existing recommendation logic and shows a notice.</p>
              </div>
            </SettingsBlock>
            <SettingsBlock title="New project chats" data-setting-id="workspace-new-chat" className={settingHighlightClass("workspace-new-chat").trim()}>
              <SettingsChoiceGroup value={newProjectChatWorkspace} onChange={setNewProjectChatWorkspace} testIdPrefix="project-chat-workspace" options={[
                { value: "current", label: "Current checkout", detail: "Start in the selected project folder." },
                { value: "ask", label: "Ask", detail: "Choose checkout or worktree each time." },
                { value: "worktree", label: "Isolated worktree", detail: "Create one for interactive Git-project chats." },
              ]} />
              <p className="sheet-hint">If worktree creation fails, the chat stays in the current checkout and offers a retry.</p>
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "google" && (
        <section className="settings-section" id="settings-panel-google" role="tabpanel" aria-labelledby="settings-tab-google" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock data-setting-id="google-workspace" className={settingHighlightClass("google-workspace").trim()}>
              <div className="setting-stack">
                <div className="google-workspace-connect-card">
                  <div className="google-workspace-connect-copy">
                    <span className="google-workspace-logo" aria-hidden="true">
                      <img src={googleLogo} alt="" />
                    </span>
                    <div>
                      <strong>{googleWorkspace?.connected ? "Google Workspace connected" : "Connect Google Workspace"}</strong>
                      <span>Choose one or many Sheets, Docs, Slides, and Drive files for Milim.</span>
                    </div>
                  </div>
                  <button
                    className="btn-accent google-workspace-connect-button"
                    type="button"
                    disabled={googleWorkspaceBusy || !googleWorkspace?.available}
                    onClick={() => void chooseGoogleFilesFromSettings()}
                  >
                    {googleWorkspaceBusy
                      ? "Waiting..."
                      : !googleWorkspace
                        ? "Loading..."
                        : !googleWorkspace.available
                          ? "Unavailable"
                          : confirmGoogleConnect && !googleWorkspace.connected
                            ? "Continue to Google"
                            : googleWorkspace.connected
                              ? "Choose more files"
                              : "Connect Google Workspace"}
                  </button>
                </div>
                {confirmGoogleConnect && !googleWorkspace?.connected ? (
                  <p className="sheet-hint" role="note">{GOOGLE_CONNECT_DISCLOSURE}</p>
                ) : null}
                {!googleWorkspace ? (
                  <p className="sheet-hint">{googleWorkspaceMessage || "Loading Google Workspace status..."}</p>
                ) : !googleWorkspace.available ? (
                  <p className="sheet-hint">
                    {googleWorkspace.unavailable_reason || "Google Workspace is unavailable in this build."}
                  </p>
                ) : (
                  <>
                    {googleWorkspace.files.length ? (
                      <div className="google-workspace-file-list">
                        {googleWorkspace.files.map((file) => {
                          const url = googleWorkspaceFileUrl(file);
                          return (
                            <div className="google-workspace-file-row" key={file.id}>
                              {file.icon_link ? (
                                <img className="google-file-icon" src={file.icon_link} alt="" />
                              ) : file.mime_type.endsWith(".folder") ? (
                                <FolderOpen size={15} aria-hidden="true" />
                              ) : (
                                <FileText size={15} aria-hidden="true" />
                              )}
                              <div className="google-workspace-file-copy">
                                <strong>{file.name}</strong>
                                <span>
                                  {googleFileTypeLabel(file.mime_type)}
                                  {googleWorkspace.managed_folder_id === file.id ? " · Managed folder" : ""}
                                  {file.trashed ? " · In trash" : ""}
                                </span>
                              </div>
                              <div className="google-workspace-file-actions">
                                <button
                                  className="btn-ghost google-workspace-file-action"
                                  type="button"
                                  title={`Open ${file.name} in Milim`}
                                  aria-label={`Open ${file.name} in Milim`}
                                  onClick={() => openGoogleFileInMilim(url)}
                                >
                                  <span className="topbar-logo" aria-hidden="true" />
                                </button>
                                <button
                                  className="btn-ghost google-workspace-file-action"
                                  type="button"
                                  title={`Open ${file.name} in Google`}
                                  aria-label={`Open ${file.name} in Google`}
                                  onClick={() => void openExternalUrl(url)}
                                >
                                  <ExternalLink size={13} aria-hidden="true" />
                                </button>
                                <button
                                  className="btn-ghost danger google-workspace-file-action"
                                  type="button"
                                  title={`Remove ${file.name} from Milim`}
                                  aria-label={`Remove ${file.name} from Milim`}
                                  disabled={googleWorkspaceBusy}
                                  onClick={() => void removeGoogleFileFromSettings(file.id)}
                                >
                                  <Trash size={13} aria-hidden="true" />
                                </button>
                              </div>
                            </div>
                          );
                        })}
                      </div>
                    ) : (
                      <p className="sheet-hint">No Google files are authorized yet.</p>
                    )}
                    {googleWorkspace.connected ? (
                      <div className="settings-action-row">
                        <div>
                          <strong>Disconnect Google Workspace</strong>
                          <span>Ask Google to revoke Milim, then remove the local token and selected-file registry. Drive files are never deleted.</span>
                        </div>
                        <button
                          className={"btn-ghost danger" + (confirmGoogleDisconnect ? " confirm" : "")}
                          type="button"
                          disabled={googleWorkspaceBusy}
                          onClick={() => void disconnectGoogleFromSettings()}
                        >
                          {confirmGoogleDisconnect ? "Confirm?" : "Disconnect"}
                        </button>
                      </div>
                    ) : null}
                    {googleWorkspace.error ? <p className="sheet-hint error" role="alert">{googleWorkspace.error}</p> : null}
                  </>
                )}
                <p className="sheet-hint">Read access follows Guarded mode. Changes follow the chat's Review or Open approval mode. Tokens and the selected-file registry are encrypted locally and excluded from backups.</p>
                {googleWorkspaceMessage ? <p className={googleWorkspaceMessage.includes("failed") ? "sheet-hint error" : "sheet-hint"} role="status">{googleWorkspaceMessage}</p> : null}
                {googleRevocationUnconfirmed ? (
                  <button className="btn-ghost" type="button" onClick={() => void openExternalUrl(GOOGLE_ACCOUNT_CONNECTIONS_URL)}>
                    <ExternalLink size={12} />
                    Open Google Account
                  </button>
                ) : null}
              </div>
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "history" && (
        <section className="settings-section" id="settings-panel-history" role="tabpanel" aria-labelledby="settings-tab-history" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Browser data" data-setting-id="browser-data" className={settingHighlightClass("browser-data").trim()}>
              <div className="setting-stack">
                <SettingsChoiceGroup
                  value={browserStorageMode}
                  onChange={setBrowserStorageMode}
                  testIdPrefix="browser-storage"
                  ariaLabel="Browser storage"
                  options={[
                    { value: "persistent", label: "Remember sign-ins", detail: "Share one local Milim browser profile across chats and restarts." },
                    { value: "private", label: "Private", detail: "Discard cookies and site storage when the sidepanel browser closes." },
                  ]}
                />
                <p className="sheet-hint">Milim does not read or import Chrome, Safari, Firefox, or Edge passwords and cookies. Sign in once inside the Milim browser instead. Generated App previews always remain private.</p>
                <div className="settings-action-row">
                  <div>
                    <strong>Clear sign-ins and site data</strong>
                    <span>Remove cookies, local storage, cache, and other data from the persistent Milim browser profile.</span>
                  </div>
                  <button
                    className={"btn-ghost danger" + (confirmBrowserDataClear ? " confirm" : "")}
                    type="button"
                    disabled={browserDataBusy}
                    onClick={() => void clearBrowserDataFromSettings()}
                    data-testid="browser-data-clear"
                  >
                    {browserDataBusy ? "Clearing..." : confirmBrowserDataClear ? "Confirm?" : "Clear"}
                  </button>
                </div>
                {browserDataStatus ? <p className={browserDataStatus.startsWith("Clear failed") ? "sheet-hint error" : "sheet-hint"} role="status">{browserDataStatus}</p> : null}
              </div>
            </SettingsBlock>
            <SettingsBlock title="Export" data-setting-id="data-export" className={settingHighlightClass("data-export").trim()}>
              <div className="setting-field">
                <span className="setting-mini-title">Default single-thread format</span>
                <SettingsChoiceGroup value={threadExportFormat} onChange={setThreadExportFormat} testIdPrefix="thread-export-format" options={[
                  { value: "json", label: "JSON", detail: "Preserve structured Milim thread data." },
                  { value: "markdown", label: "Markdown", detail: "Create a readable conversation document." },
                ]} />
              </div>
            </SettingsBlock>
            <SettingsBlock title="Backup & restore" data-setting-id="data-backup" className={settingHighlightClass("data-backup").trim()}>
              <div className="setting-stack">
                <div className="settings-action-row"><div><strong>Export Milim backup</strong><span>Chats, projects, drafts, archive state, settings, themes, quick actions, and local personalization metadata.</span></div><button className="btn-ghost" type="button" disabled={backupBusy} onClick={() => void exportBackupFromSettings()}>Export</button></div>
                <div className="settings-action-row"><div><strong>Restore backup</strong><span>Validate, snapshot current data, then replace backed-up state in one transaction.</span></div><button className="btn-ghost" type="button" disabled={backupBusy} onClick={() => void restoreBackupFromSettings()}>Restore</button></div>
                <p className="sheet-hint">Credentials, browser profile data, MCP secrets, paired-device tokens, memory databases, generated media, update packages, worktrees, and running jobs are excluded.</p>
                {backupStatus ? <p className={backupStatus.includes("failed") ? "sheet-hint error" : "sheet-hint"} role="status">{backupStatus}</p> : null}
              </div>
            </SettingsBlock>
            <SettingsBlock title="Retention" data-setting-id="history-retention" className={settingHighlightClass("history-retention").trim()}>
              <div className="setting-stack">
                <div className="setting-field">
                  <span className="setting-mini-title">Delete archived items after</span>
                  <SettingsChoiceGroup<"7" | "14" | "30">
                    value={archiveRetentionValue}
                    onChange={setArchiveRetentionFromSettings}
                    testIdPrefix="archive-retention"
                    options={[
                      { value: "7", label: "7 days", detail: "Short cleanup window." },
                      { value: "14", label: "14 days", detail: "Two-week recovery window." },
                      { value: "30", label: "30 days", detail: "Maximum recovery window." },
                    ]}
                  />
                </div>
                <div className="settings-action-row">
                  <div>
                    <strong>Expired items</strong>
                    <span>Archived chats and projects older than {archiveRetentionDays} days are removed.</span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={purgeExpiredArchivesFromSettings}>
                    <Trash size={13} />
                    Purge now
                  </button>
                </div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="Projects" data-setting-id="history-projects" className={settingHighlightClass("history-projects").trim()}>
              {archivedProjects.length === 0 ? (
                <p className="sheet-hint">No archived projects.</p>
              ) : (
                <div className="archive-list">
                  {archivedProjects.map((project) => {
                    const deleteKey = `project:${project.id}`;
                    return (
                      <div className="settings-action-row archive-row" key={project.id}>
                        <div>
                          <strong>{project.name}</strong>
                          <span>{project.folder}</span>
                          <span>{projectThreadCount(project)} chats · deletes {archiveDeleteLabel(project.archivedAt, archiveRetentionDays)}</span>
                        </div>
                        <div className="archive-row-actions">
                          <button
                            className="btn-ghost archive-action-button"
                            type="button"
                            title={`Restore ${project.name}`}
                            aria-label={`Restore ${project.name}`}
                            onClick={() => restoreProject(project.id)}
                          >
                            <Refresh size={13} />
                          </button>
                          <button
                            className={"btn-ghost danger archive-action-button" + (confirmArchiveDelete === deleteKey ? " confirm" : "")}
                            type="button"
                            title={confirmArchiveDelete === deleteKey ? `Confirm delete ${project.name}` : `Delete ${project.name}`}
                            aria-label={confirmArchiveDelete === deleteKey ? `Confirm delete ${project.name}` : `Delete ${project.name}`}
                            onClick={() => deleteArchivedProject(project.id)}
                          >
                            {confirmArchiveDelete === deleteKey ? "Confirm?" : <Trash size={13} />}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </SettingsBlock>

            <SettingsBlock title="Chats" data-setting-id="history-chats" className={settingHighlightClass("history-chats").trim()}>
              {archivedSessions.length === 0 ? (
                <p className="sheet-hint">No archived chats.</p>
              ) : (
                <div className="archive-list">
                  {archivedSessions.map((session) => {
                    const deleteKey = `session:${session.id}`;
                    return (
                      <div className="settings-action-row archive-row" key={session.id}>
                        <div>
                          <strong>{session.title}</strong>
                          <span>{archivedSessionProjectLabel(session)}</span>
                          <span>Archived {timestampLabel(session.archivedAt)} · deletes {archiveDeleteLabel(session.archivedAt, archiveRetentionDays)}</span>
                        </div>
                        <div className="archive-row-actions">
                          <button
                            className="btn-ghost archive-action-button"
                            type="button"
                            title={`Restore ${session.title}`}
                            aria-label={`Restore ${session.title}`}
                            onClick={() => restoreSession(session.id)}
                          >
                            <Refresh size={13} />
                          </button>
                          <button
                            className={"btn-ghost danger archive-action-button" + (confirmArchiveDelete === deleteKey ? " confirm" : "")}
                            type="button"
                            title={confirmArchiveDelete === deleteKey ? `Confirm delete ${session.title}` : `Delete ${session.title}`}
                            aria-label={confirmArchiveDelete === deleteKey ? `Confirm delete ${session.title}` : `Delete ${session.title}`}
                            onClick={() => deleteArchivedSession(session.id)}
                          >
                            {confirmArchiveDelete === deleteKey ? "Confirm?" : <Trash size={13} />}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "appearance" && (
        <section className="settings-section" id="settings-panel-appearance" role="tabpanel" aria-labelledby="settings-tab-appearance" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Theme" data-setting-id="appearance-theme" className={settingHighlightClass("appearance-theme").trim()}>
              <div className="theme-grid">
                {themes.map((t) => {
                  const contrastIssues = themeContrastIssues(t);
                  const isCustomTheme = customIds.has(t.id);
                  return (
                    <div className="theme-card-wrap" key={t.id}>
                    <button
                      key={t.id}
                      className={"theme-card" + (t.id === themeId ? " active" : "") + (contrastIssues.length ? " low-contrast" : "")}
                      onClick={() => setTheme(t.id)}
                      onDoubleClick={() => isCustomTheme && setEditing({ base: t, isNew: false })}
                      title={contrastIssues[0]}
                    >
                      <span
                        className="theme-preview"
                        style={{
                          background: t.background.image
                            ? `${t.background.image}, ${t.colors.bgPrimary}`
                            : t.colors.bgPrimary,
                        }}
                      >
                        <span
                          className="theme-panel"
                          style={{ background: t.colors.bgSecondary, borderColor: t.colors.borderPrimary }}
                        >
                          <span className="theme-dot" style={{ background: t.colors.accent }} />
                          <span className="theme-bar" style={{ background: t.colors.tertiaryText }} />
                        </span>
                      </span>
                      <span className="theme-name">
                        {t.name}
                        {t.id === themeId && <Check size={13} />}
                      </span>
                    </button>
                    {isCustomTheme && (
                      <button
                        className="theme-edit-button"
                        type="button"
                        onClick={() => setEditing({ base: t, isNew: false })}
                        aria-label={`Edit ${t.name}`}
                        title={`Edit ${t.name}`}
                      >
                        <Pencil size={12} />
                      </button>
                    )}
                    </div>
                  );
                })}

                <button className="theme-card new-card" onClick={() => setEditing({ base: current, isNew: true })}>
                  <span className="theme-preview new">
                    <PlusSquare size={22} />
                  </span>
                  <span className="theme-name">Customize...</span>
                </button>
              </div>

              {custom.length > 0 && <p className="sheet-hint">Double-click a custom theme to edit or delete it.</p>}
            </SettingsBlock>
            <SettingsBlock title="Chat surface" data-setting-id="appearance-chat-surface" className={settingHighlightClass("appearance-chat-surface").trim()}>
              <div className="setting-stack">
                <div className="setting-field">
                  <span className="setting-mini-title">Layout</span>
                  <AppearanceChatLayoutChoices
                    value={chatLayoutStyle}
                    onChange={setChatLayoutStyle}
                  />
                </div>
                <div className="setting-field">
                  <span className="setting-mini-title">Message width</span>
                  <AppearanceMessageWidthChoices
                    value={messageWidth}
                    onChange={setMessageWidth}
                  />
                </div>
                <div className="setting-field">
                  <span className="setting-mini-title">Avatars</span>
                  <AppearanceAvatarChoices
                    value={avatarStyle}
                    onChange={setAvatarStyle}
                  />
                </div>
              </div>
            </SettingsBlock>
            <SettingsBlock title="Sidebar" data-setting-id="appearance-sidebar-colors" className={settingHighlightClass("appearance-sidebar-colors").trim()}>
              <div className="setting-stack">
                <div className="setting-field sidebar-rail-style-field">
                  <span className="setting-mini-title">Collapsed rail</span>
                  <SettingsChoiceGroup
                    value={sidebarRailStyle}
                    onChange={setSidebarRailStyle}
                    testIdPrefix="sidebar-rail-style"
                    ariaLabel="Collapsed sidebar rail style"
                    options={[
                      { value: "regular", label: "Regular", detail: "One full-height rail." },
                      { value: "split", label: "Split", detail: "Actions at the top and bottom." },
                      { value: "centered", label: "Centered", detail: "One compact group in the middle." },
                    ]}
                  />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>Automatically color project thread names</strong>
                    <span>Derive one stable color per project from the active theme accent. Custom project colors take priority.</span>
                  </div>
                  <Toggle
                    checked={autoColorThreadNames}
                    onChange={setAutoColorThreadNames}
                    ariaLabel="Automatically color project thread names"
                    testId="auto-color-thread-names-toggle"
                  />
                </div>
                <div className="setting-toggle-row">
                  <div>
                    <strong>Settled threads</strong>
                    <span>Move finished threads into a compact tier at the bottom of the sidebar.</span>
                  </div>
                  <Toggle
                    checked={settledThreadsEnabled}
                    onChange={setSettledThreadsEnabled}
                    ariaLabel="Settled threads"
                    testId="settled-threads-toggle"
                  />
                </div>
              </div>
            </SettingsBlock>
            <SettingsBlock title="Empty chat" data-setting-id="appearance-empty-chat-ridgeline" className={settingHighlightClass("appearance-empty-chat-ridgeline").trim()}>
              <div className="setting-toggle-row">
                <div>
                  <strong>Show activity ridgeline</strong>
                  <span>Display the local activity chart above the composer in an empty chat.</span>
                </div>
                <Toggle
                  checked={showEmptyChatRidgeline}
                  onChange={setShowEmptyChatRidgeline}
                  ariaLabel="Show empty-chat activity ridgeline"
                  testId="empty-chat-ridgeline-toggle"
                />
              </div>
            </SettingsBlock>
            <SettingsBlock title="Code blocks" data-setting-id="appearance-code-blocks" className={settingHighlightClass("appearance-code-blocks").trim()}>
              <AppearanceCodeBlockThemeChoices
                value={codeBlockTheme}
                onChange={setCodeBlockTheme}
              />
            </SettingsBlock>
            <SettingsBlock title="Interface sounds" data-setting-id="appearance-interface-sounds" className={settingHighlightClass("appearance-interface-sounds").trim()}>
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div>
                    <strong>Enable sounds</strong>
                    <span>Locally synthesized alerts, off by default.</span>
                  </div>
                  <Toggle
                    checked={interfaceSounds}
                    onChange={setInterfaceSounds}
                    ariaLabel="Enable interface sounds"
                    testId="interface-sounds-toggle"
                  />
                </div>
                {interfaceSounds && (
                  <>
                    <div className="setting-toggle-row">
                      <div>
                        <strong>Needs attention</strong>
                        <span>Tool approvals, proposed worker plans, and terminal errors.</span>
                      </div>
                      <Toggle checked={soundOnAttention} onChange={setSoundOnAttention} ariaLabel="Needs attention sounds" testId="attention-sounds-toggle" />
                    </div>
                    {soundOnAttention && (
                      <div className="setting-field">
                        <span className="setting-mini-title">Attention sound</span>
                        <div className="setting-field-action">
                          <Select
                            value={attentionSound}
                            options={ATTENTION_SOUND_OPTIONS.map((value) => ({ value, label: SOUND_LABELS[value] }))}
                            onChange={(value) => setAttentionSound(value as AttentionSound)}
                            testId="attention-sound-select"
                          />
                          <button type="button" className="btn-ghost" onClick={() => playInterfaceSound(attentionSound)}>Preview</button>
                        </div>
                      </div>
                    )}
                    <div className="setting-toggle-row">
                      <div>
                        <strong>Finished</strong>
                        <span>A visible active chat completes, including its queued messages.</span>
                      </div>
                      <Toggle checked={soundOnFinished} onChange={setSoundOnFinished} ariaLabel="Finished sounds" testId="finished-sounds-toggle" />
                    </div>
                    {soundOnFinished && (
                      <div className="setting-field">
                        <span className="setting-mini-title">Finished sound</span>
                        <div className="setting-field-action">
                          <Select
                            value={finishedSound}
                            options={FINISHED_SOUND_OPTIONS.map((value) => ({ value, label: SOUND_LABELS[value] }))}
                            onChange={(value) => setFinishedSound(value as FinishedSound)}
                            testId="finished-sound-select"
                          />
                          <button type="button" className="btn-ghost" onClick={() => playInterfaceSound(finishedSound)}>Preview</button>
                        </div>
                      </div>
                    )}
                    <div className="setting-toggle-row">
                      <div>
                        <strong>Interaction feedback</strong>
                        <span>Optional cues for toggles, menus, dismissals, and primary actions.</span>
                      </div>
                      <Toggle checked={soundOnInteractions} onChange={setSoundOnInteractions} ariaLabel="Interaction feedback sounds" testId="interaction-sounds-toggle" />
                    </div>
                  </>
                )}
              </div>
            </SettingsBlock>
            {activeBackgroundImage && (
              <SettingsBlock title="Background image" data-setting-id="appearance-background" className={settingHighlightClass("appearance-background").trim()}>
                <AppearanceBackgroundImageChoices
                  backgroundImage={activeBackgroundImage}
                  fit={backgroundFit}
                  treatment={backgroundTreatment}
                  onFitChange={setBackgroundFit}
                  onTreatmentChange={setBackgroundTreatment}
                />
              </SettingsBlock>
            )}
          </SettingsPanel>
        </section>
            )}

            {activeSection === "system" && (
        <section className="settings-section" id="settings-panel-system" role="tabpanel" aria-labelledby="settings-tab-system" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Credential storage" data-setting-id="system-secret-storage" className={settingHighlightClass("system-secret-storage").trim()}>
              <div className="settings-action-row">
                <div>
                  <strong>
                    {!secretStorage
                      ? "Checking credential storage"
                      : secretStorage.mode === "native"
                        ? "OS credential vault"
                        : secretStorage.mode === "restricted_file"
                          ? "Restricted local fallback"
                          : "Credential storage unavailable"}
                  </strong>
                  <span>{secretStorage?.detail ?? "Reading the desktop credential-storage status."}</span>
                </div>
              </div>
            </SettingsBlock>
            <SettingsBlock data-setting-id="system-shortcuts" className={settingHighlightClass("system-shortcuts").trim()}>
              <div className="setting-stack">
                {APP_SHORTCUT_ACTIONS.map((action) => (
                  <div className="shortcut-recorder-row" key={action}>
                    <div>
                      <strong>{APP_SHORTCUT_LABELS[action]}</strong>
                      <span>{recordingShortcut === action ? "Press a key combination..." : shortcutLabel(appShortcuts[action])}</span>
                    </div>
                    <button
                      className={"btn-ghost shortcut-recorder-button" + (recordingShortcut === action ? " active" : "")}
                      type="button"
                      data-shortcut-recorder="true"
                      data-testid={`app-shortcut-${action}`}
                      aria-pressed={recordingShortcut === action}
                      onClick={() => startRecordingShortcut(action)}
                    >
                      {recordingShortcut === action ? "Recording" : "Change"}
                    </button>
                  </div>
                ))}
                <div className="settings-action-row">
                  <div>
                    <strong>Shortcut defaults</strong>
                    <span>Restore Milim's default app-window shortcuts.</span>
                  </div>
                  <button
                    className="btn-ghost"
                    type="button"
                    data-testid="app-shortcuts-reset"
                    onClick={() => {
                      resetAppShortcuts();
                      setRecordingShortcut(null);
                      setShortcutError(null);
                    }}
                  >
                    Reset
                  </button>
                </div>
                {shortcutError && <p className="sheet-hint error">{shortcutError}</p>}
              </div>
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "mobile" && (
        <section className="settings-section" id="settings-panel-mobile" role="tabpanel" aria-labelledby="settings-tab-mobile" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock data-setting-id="mobile-companion" className={settingHighlightClass("mobile-companion").trim()}>
              <MobileCompanionSettings />
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "about" && (
        <section className="settings-section" id="settings-panel-about" role="tabpanel" aria-labelledby="settings-tab-about" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Version" data-setting-id="about-version" className={settingHighlightClass("about-version").trim()}>
              <div className="settings-action-row">
                <div>
                  <strong>Current version</strong>
                  <span>{currentVersionLabel}</span>
                </div>
              </div>
              <div className="settings-action-row">
                <div>
                  <strong>Latest version</strong>
                  <span>{latestVersionLabel}</span>
                </div>
              </div>
              <div className="settings-action-row">
                <div>
                  <strong>Last checked</strong>
                  <span>{updateLastCheckedAt ? new Date(updateLastCheckedAt).toLocaleString() : "Never"}</span>
                </div>
              </div>
            </SettingsBlock>

            <SettingsBlock title="Updates" data-setting-id="about-updates" className={settingHighlightClass("about-updates").trim()}>
              <div className="settings-action-row">
                <div>
                  <strong>{updateStatusLabel(updateStatus)}</strong>
                  <span>{updateError || (updatePath ? "Downloaded and ready to install." : "Checks GitHub Releases for portable app updates.")}</span>
                </div>
                <button className="btn-ghost" type="button" onClick={checkUpdatesFromSettings} disabled={!canCheckForUpdate}>
                  <Refresh size={13} />
                  Check
                </button>
              </div>
              {(updateStatus === "downloading" || updateStatus === "installing") ? (
                <UpdateProgress
                  className="settings-update-progress"
                  progress={updateProgress ?? {
                    phase: updateStatus === "installing" ? "restarting" : "downloading",
                    downloadedBytes: 0,
                    totalBytes: null,
                  }}
                />
              ) : null}
              {canDownloadUpdate ? (
                <div className="settings-action-row">
                  <div>
                    <strong>Download update</strong>
                    <span>Verify the package checksum before staging it locally.</span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={downloadUpdateFromSettings}>
                    <Download size={13} />
                    Download
                  </button>
                </div>
              ) : null}
              {canInstallUpdate ? (
                <div className="settings-action-row">
                  <div>
                    <strong>Restart to update</strong>
                    <span>milim will close, replace the app, and reopen.</span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={installUpdateFromSettings}>
                    Restart
                  </button>
                </div>
              ) : null}
              {updateInfo?.publishedAt ? <p>Released {new Date(updateInfo.publishedAt).toLocaleString()}.</p> : null}
              {updateInfo?.notes ? (
                <details className="settings-contract">
                  <summary>Release notes</summary>
                  <p>{updateInfo.notes}</p>
                </details>
              ) : null}
            </SettingsBlock>

            <SettingsBlock title="Diagnostics" data-setting-id="about-diagnostics" className={settingHighlightClass("about-diagnostics").trim()}>
              <div className="settings-action-row">
                <div>
                  <strong>Local logs</strong>
                  <span>Milim keeps two bounded log files on this device. Logs are never uploaded automatically.</span>
                </div>
                <button className="btn-ghost" type="button" data-testid="open-diagnostics" onClick={() => void openLogsFromSettings()}>
                  <FolderOpen size={13} />
                  Open logs
                </button>
              </div>
              {diagnosticsError && <p className="sheet-hint error" role="alert">{diagnosticsError}</p>}
            </SettingsBlock>
          </SettingsPanel>
        </section>
            )}

            {activeSection === "developer" && (
        <section className="settings-section" id="settings-panel-developer" role="tabpanel" aria-labelledby="settings-tab-developer" tabIndex={-1}>
          <SettingsPanel>
            <SettingsBlock title="Mode" data-setting-id="developer-mode" className={settingHighlightClass("developer-mode").trim()}>
              <div className="setting-toggle-row">
                <div>
                  <strong>Developer mode</strong>
                  <span>Show developer-only settings for testing setup flows.</span>
                </div>
                <Toggle checked={developerMode} onChange={setDeveloperMode} ariaLabel="Developer mode" testId="general-developer-mode-toggle" />
              </div>
            </SettingsBlock>

            {developerMode && (
            <SettingsBlock title="Experimental">
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div>
                    <strong>Hashline file patching</strong>
                    <span>Expose anchored read and patch tools to agent runs.</span>
                  </div>
                  <Toggle
                    checked={experimentalHashlinePatch}
                    onChange={setExperimentalHashlinePatch}
                    ariaLabel="Hashline file patching"
                    testId="developer-hashline-patch-toggle"
                  />
                </div>
              </div>
            </SettingsBlock>
            )}

            {developerMode && (
            <SettingsBlock title="Release UI" data-setting-id="developer-update-cards" className={settingHighlightClass("developer-update-cards").trim()}>
              <div className="settings-action-row">
                <div>
                  <strong>Update cards</strong>
                  <span>Preview the bundled cards for the current version without changing their viewed state.</span>
                </div>
                <button
                  className="btn-ghost"
                  type="button"
                  data-testid="developer-show-update-cards"
                  onClick={() => {
                    onClose();
                    showUpdateCardsForDebug();
                  }}
                >
                  Show cards
                </button>
              </div>
            </SettingsBlock>
            )}

            {developerMode && (
            <SettingsBlock title="Onboarding">
              <div className="setting-stack">
                <div className="setting-toggle-row">
                  <div>
                    <strong>Onboarding flow</strong>
                    <span>Open the first-run setup sheet for testing. Turning this off dismisses the active flow.</span>
                  </div>
                  <Toggle
                    checked={onboardingDeveloperShow || onboardingStatus === "in_progress"}
                    onChange={setDeveloperShowOnboarding}
                    ariaLabel="Onboarding flow"
                    testId="developer-onboarding-toggle"
                  />
                </div>
                <div className="settings-action-row">
                  <div>
                    <strong>Onboarding state</strong>
                    <span>
                      {onboardingStatus} / {onboardingSetupLabel(onboardingSetupPath)}
                    </span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={() => setDeveloperShowOnboarding(true)} data-testid="developer-open-onboarding">
                    Open now
                  </button>
                </div>
                <div className="settings-action-row">
                  <div>
                    <strong>Completion</strong>
                    <span>
                      Completed {timestampLabel(onboardingCompletedAt)} / dismissed {timestampLabel(onboardingDismissedAt)}
                    </span>
                  </div>
                  <button className="btn-ghost" type="button" onClick={completeOnboarding} data-testid="developer-complete-onboarding">
                    Mark complete
                  </button>
                </div>
                <div className="settings-action-row">
                  <div>
                    <strong>Reset first-run state</strong>
                    <span>Clear onboarding choices so automatic first-run gating can run again.</span>
                  </div>
                  <button className="btn-ghost danger" type="button" onClick={resetOnboarding} data-testid="developer-reset-onboarding">
                    Reset
                  </button>
                </div>
              </div>
            </SettingsBlock>
            )}
          </SettingsPanel>
        </section>
            )}
          </div>
          </div>
        </div>
      </SheetDialog>
  );
}
