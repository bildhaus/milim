import { getCurrentWebview } from "@tauri-apps/api/webview";
import { invoke } from "@tauri-apps/api/core";
import {
  Component,
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ErrorInfo,
  type MouseEvent,
  type ReactNode,
} from "react";
import { useAgents } from "./agents/store";
import {
  deleteThreadTree,
  listModelsDetailed,
  loadStartupModels,
  openDiagnosticsFolder,
  openExternalUrl,
  recordFrontendError,
  requestDesktopQuit,
  restartDesktopApp,
} from "./api";
import { AutoUpdater } from "./components/AutoUpdater";
import { ChatView } from "./components/ChatView";
import { ContextMenuProvider, useContextMenu, type ContextMenuItem } from "./components/ContextMenu";
import type { GitPanelView } from "./components/GitPanel";
import { FolderOpen, Gear, Globe, Pencil, Plus, Refresh, Sidebar as SidebarIcon, X } from "./components/icons";
import { Logo } from "./components/Logo";
import { MobilePairingApprovalPrompt } from "./components/MobilePairingApprovalPrompt";
import { ResizeHandles } from "./components/ResizeHandles";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { UpdateCards } from "./components/UpdateCards";
import {
  shouldCheckOnboardingModels,
  shouldShowOnboarding,
  useOnboarding,
} from "./onboarding/store";
import { useSettings } from "./settings/store";
import {
  importLegacyLocalStorageOnce,
  flushDeferredUserStateWrites,
  installUserStateFlushHandlers,
} from "./persistence/userStateStorage.js";
import {
  hydrateSessionComposerDraftsFromUserState,
  purgeExpiredArchivesAfterHydration,
  useSessions,
} from "./sessions/store";
import { hydrateThemeFromUserState, useTheme } from "./theme/store";
import { animatedBackgroundSource } from "./theme/applyTheme";
import {
  installInterfaceSoundClicks,
  setInterfaceSoundsEnabled,
} from "./ui/sounds";
import { shortcutLabel } from "./ui/shortcuts";
import { createInteractiveChat } from "./lib/newChatCoordinator";
import {
  nativeBadgeThreadCount,
  setMilimUnreadBadge,
} from "./lib/nativeNotifications";
import { useUiPreferences } from "./ui/store";

const SettingsPage = lazy(() =>
  import("./settings/SettingsDialog").then((mod) => ({
    default: mod.SettingsPage,
  })),
);
const AgentsManager = lazy(() =>
  import("./components/AgentsManager").then((mod) => ({
    default: mod.AgentsManager,
  })),
);
const SkillsManager = lazy(() =>
  import("./components/SkillsManager").then((mod) => ({
    default: mod.SkillsManager,
  })),
);
const SchedulesManager = lazy(() =>
  import("./components/SchedulesManager").then((mod) => ({
    default: mod.SchedulesManager,
  })),
);
const MediaManager = lazy(() =>
  import("./components/MediaManager").then((mod) => ({
    default: mod.MediaManager,
  })),
);
const PullRequestsManager = lazy(() =>
  import("./components/PullRequestsManager").then((mod) => ({
    default: mod.PullRequestsManager,
  })),
);
const OnboardingFlow = lazy(() =>
  import("./components/OnboardingFlow").then((mod) => ({
    default: mod.OnboardingFlow,
  })),
);
const inTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const DOCS_URL = "https://docs.milim.ai/";
const APP_MENU_EVENT = "milim://menu-action";
const MODEL_FAVORITES_UPDATED_EVENT = "milim://model-favorites-updated";
const SETTINGS_STATE_KEY = "milim.settings";

function normalizedFavoriteModelIds(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return Array.from(new Set(
    value
      .filter((item): item is string => typeof item === "string")
      .map((item) => item.trim())
      .filter(Boolean),
  ));
}

function persistedFavoriteModelIds(value: string | null): string[] {
  if (!value) return [];
  try {
    const parsed = JSON.parse(value) as { state?: { favorites?: unknown } };
    return normalizedFavoriteModelIds(parsed.state?.favorites);
  } catch {
    return [];
  }
}

function applyCanonicalFavoriteModelIds(value: unknown) {
  const favorites = normalizedFavoriteModelIds(value);
  const current = useSettings.getState().favorites;
  if (
    current.length === favorites.length &&
    current.every((id, index) => id === favorites[index])
  ) return;
  useSettings.getState().setFavorites(favorites);
}

function BackgroundLayer() {
  const source = animatedBackgroundSource(useTheme.getState().theme.background.image);
  return (
    <div className="bg-layer" aria-hidden="true">
      <img
        className="bg-layer-gif"
        data-theme-background-gif
        src={source}
        hidden={!source}
        alt=""
      />
    </div>
  );
}

function OnboardingGate() {
  const status = useOnboarding((s) => s.status);
  const developerShowOnboarding = useOnboarding(
    (s) => s.developerShowOnboarding,
  );
  const dismissedAt = useOnboarding((s) => s.dismissedAt);
  const accountRuntimeEnabled = useSettings((s) => s.accountRuntimeEnabled);
  const shouldCheckModels = shouldCheckOnboardingModels(
    status,
    developerShowOnboarding,
  );
  const [hydrated, setHydrated] = useState(useOnboarding.persist.hasHydrated());
  const [modelsReady, setModelsReady] = useState(false);
  const [modelsChecked, setModelsChecked] = useState(false);

  useEffect(() => {
    if (hydrated) return;
    if (useOnboarding.persist.hasHydrated()) {
      setHydrated(true);
      return;
    }
    return useOnboarding.persist.onFinishHydration(() => setHydrated(true));
  }, [hydrated]);

  const refreshModelReadiness = useCallback(async () => {
    try {
      const models = await listModelsDetailed(accountRuntimeEnabled);
      setModelsReady(models.length > 0);
    } catch {
      setModelsReady(false);
    } finally {
      setModelsChecked(true);
    }
  }, [accountRuntimeEnabled]);

  useEffect(() => {
    if (!hydrated || !shouldCheckModels) return;
    let cancelled = false;
    void loadStartupModels(
      (models) => {
        if (cancelled) return;
        setModelsReady(models.length > 0);
        setModelsChecked(true);
      },
      accountRuntimeEnabled,
    ).catch(() => {
      if (cancelled) return;
      setModelsReady(false);
      setModelsChecked(true);
    });
    return () => {
      cancelled = true;
    };
  }, [accountRuntimeEnabled, hydrated, shouldCheckModels]);

  if (!hydrated || (shouldCheckModels && !modelsChecked))
    return (
      <div
        className="onboarding-preflight"
        data-native-preview-blocker="true"
        data-testid="onboarding-preflight"
      >
        <svg
          className="onboarding-preflight-logo"
          viewBox="0 0 424.58 352.98"
          aria-hidden="true"
        >
          <use className="signal-medium" href="/milim-wordmark.svg#g-m1" />
          <use className="signal-short signal-d1" href="/milim-wordmark.svg#g-i1s" />
          <use className="bounce-dot bounce-d1" href="/milim-wordmark.svg#g-i1d" />
          <use className="signal-long signal-l" href="/milim-wordmark.svg#g-l" />
          <use className="signal-short signal-s2" href="/milim-wordmark.svg#g-i2s" />
          <use className="bounce-dot bounce-d2" href="/milim-wordmark.svg#g-i2d" />
          <use className="signal-medium signal-m2" href="/milim-wordmark.svg#g-m2" />
        </svg>
      </div>
    );
  if (
    !shouldShowOnboarding(
      status,
      modelsReady,
      developerShowOnboarding,
      dismissedAt,
    )
  )
    return <UpdateCards />;
  return <OnboardingFlow onModelsChanged={refreshModelReadiness} />;
}

function AppNoticeHost() {
  const notices = useUiPreferences((s) => s.notices);
  const dismissNotice = useUiPreferences((s) => s.dismissNotice);
  useEffect(() => {
    if (!notices.length) return;
    const timer = window.setTimeout(() => {
      const oldest = notices[0];
      if (oldest) dismissNotice(oldest.id);
    }, 6000);
    return () => window.clearTimeout(timer);
  }, [dismissNotice, notices]);
  if (!notices.length) return null;
  return (
    <div className="app-notices" data-native-preview-blocker="true" aria-live="polite" aria-atomic="false">
      {notices.map((notice) => (
        <div
          key={notice.id}
          className={`app-notice ${notice.tone}`}
          role={notice.tone === "error" ? "alert" : "status"}
        >
          <span>{notice.message}</span>
          <button
            className="icon-btn"
            type="button"
            title="Dismiss"
            aria-label="Dismiss notice"
            onClick={() => dismissNotice(notice.id)}
          >
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}

function AppRecoveryScreen({
  title,
  description,
  detail,
}: {
  title: string;
  description: string;
  detail: string;
}) {
  const [actionError, setActionError] = useState<string | null>(null);

  function restart() {
    setActionError(null);
    void flushDeferredUserStateWrites()
      .catch((error) =>
        recordFrontendError(
          "Failed to flush user state before restart",
          error instanceof Error ? error.stack || error.message : undefined,
        ),
      )
      .finally(() => restartDesktopApp())
      .catch((error) =>
        setActionError(error instanceof Error ? error.message : String(error)),
      );
  }

  function openLogs() {
    setActionError(null);
    void openDiagnosticsFolder().catch((error) =>
      setActionError(error instanceof Error ? error.message : String(error)),
    );
  }

  const { backgroundFit, backgroundTreatment } = useUiPreferences.getState();
  return (
    <div className={`app app-error-state bg-fit-${backgroundFit} bg-treatment-${backgroundTreatment}`}>
      <BackgroundLayer />
      <div className="app-error-backdrop" aria-hidden="true" />
      <main className="app-error-layout" aria-labelledby="app-error-title">
        <header className="app-error-brand">
          <Logo height={26} />
          <span aria-hidden="true" />
          <p>Recovery</p>
        </header>

        <div className="app-error-body">
          <section className="app-error-copy">
            <div role="alert">
              <p className="app-error-kicker">
                <span aria-hidden="true" />
                Runtime interrupted
              </p>
              <h1 id="app-error-title">{title}</h1>
              <p className="app-error-description">{description}</p>
            </div>

            <div className="app-error-actions">
              <button className="app-error-reload" type="button" onClick={restart}>
                <Refresh size={15} aria-hidden="true" />
                Restart Milim
              </button>
              <button className="app-error-reload secondary" type="button" onClick={openLogs}>
                <FolderOpen size={15} aria-hidden="true" />
                Open logs
              </button>
              <span>Saved work stays on this device</span>
            </div>
            {actionError && <p className="sheet-hint error" role="alert">{actionError}</p>}

            <details className="app-error-details">
              <summary>Technical details</summary>
              <code>{detail}</code>
            </details>
          </section>
        </div>
      </main>
    </div>
  );
}

class AppErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Milim UI error", error, info);
    void recordFrontendError(error.message, [error.stack, info.componentStack].filter(Boolean).join("\n")).catch(() => {});
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <AppRecoveryScreen
        title="Milim needs a quick restart."
        description="The interface stopped unexpectedly. Restart to return to your workspace. Your saved chats and settings will stay put."
        detail={this.state.error.message || "Unknown render error."}
      />
    );
  }
}

export function App() {
  return (
    <AppErrorBoundary>
      <ContextMenuProvider>
        <AppContent />
      </ContextMenuProvider>
    </AppErrorBoundary>
  );
}

function AppContent() {
  // Subscribing here ensures the theme store initializes (and applies CSS vars)
  // before first paint.
  useTheme((s) => s.themeId);
  const { openContextMenu, openMenuAt } = useContextMenu();
  const refreshAgents = useAgents((s) => s.refresh);
  const sessionIds = useSessions((s) =>
    s.sessions.map((session) => session.id).join("\0"),
  );
  const nativeBadgeCount = useSessions(nativeBadgeThreadCount);
  const lastSessionIdsRef = useRef<Set<string> | null>(null);
  const [sessionsHydrated, setSessionsHydrated] = useState(() =>
    useSessions.persist.hasHydrated(),
  );
  const [uiHydrated, setUiHydrated] = useState(() => useUiPreferences.persist.hasHydrated());
  const [settingsHydrated, setSettingsHydrated] = useState(() => useSettings.persist.hasHydrated());
  const startupAppliedRef = useRef(false);
  useEffect(() => {
    installUserStateFlushHandlers();
    return purgeExpiredArchivesAfterHydration();
  }, []);
  useEffect(() => {
    if (sessionsHydrated) return;
    return useSessions.persist.onFinishHydration(() =>
      setSessionsHydrated(true),
    );
  }, [sessionsHydrated]);
  useEffect(() => {
    if (uiHydrated) return;
    return useUiPreferences.persist.onFinishHydration(() => setUiHydrated(true));
  }, [uiHydrated]);
  useEffect(() => {
    if (settingsHydrated) return;
    return useSettings.persist.onFinishHydration(() => setSettingsHydrated(true));
  }, [settingsHydrated]);
  useEffect(() => {
    if (!inTauri || !settingsHydrated) return;
    let cancelled = false;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(async ({ listen }) => {
        const unlisten = await listen<unknown>(MODEL_FAVORITES_UPDATED_EVENT, (event) => {
          applyCanonicalFavoriteModelIds(event.payload);
        });
        if (cancelled) {
          unlisten();
          return;
        }
        dispose = unlisten;
        const persisted = await invoke<string | null>("user_state_get", {
          key: SETTINGS_STATE_KEY,
        });
        if (!cancelled) {
          applyCanonicalFavoriteModelIds(persistedFavoriteModelIds(persisted));
        }
      })
      .catch((error) => console.warn("Failed to sync model favorites:", error));
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, [settingsHydrated]);
  useEffect(() => {
    if (!sessionsHydrated || !uiHydrated || startupAppliedRef.current) return;
    startupAppliedRef.current = true;
    const preferences = useUiPreferences.getState();
    const sessions = useSessions.getState();
    if (preferences.startupBehavior === "new-chat") sessions.newUserChat();
    if (!preferences.restoreOpenPanels) {
      const activeId = useSessions.getState().activeId;
      sessions.setContextPanelOpen(activeId, false);
      sessions.setInspectorOpen(activeId, false);
    }
  }, [sessionsHydrated, uiHydrated]);
  useEffect(() => {
    const ids = new Set(sessionIds ? sessionIds.split("\0") : []);
    if (!sessionsHydrated) return;
    const previous = lastSessionIdsRef.current;
    lastSessionIdsRef.current = ids;
    if (!previous) return;
    previous.forEach((id) => {
      if (!ids.has(id)) void deleteThreadTree(id).catch(() => {});
    });
  }, [sessionIds, sessionsHydrated]);
  useEffect(() => {
    if (!sessionsHydrated) return;
    void setMilimUnreadBadge(nativeBadgeCount).catch(() => {});
  }, [nativeBadgeCount, sessionsHydrated]);
  useEffect(() => {
    refreshAgents();
  }, [refreshAgents]);
  useEffect(() => {
    void importLegacyLocalStorageOnce()
      .then(() =>
        Promise.all([
          hydrateThemeFromUserState(),
          hydrateSessionComposerDraftsFromUserState(),
        ]),
      )
      .catch((e) => console.warn("Failed to hydrate user state:", e));
  }, []);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const mainRef = useRef<HTMLDivElement>(null);
  const settingsReturnFocusRef = useRef<HTMLElement | null>(null);
  const [runtimeFailed, setRuntimeFailed] = useState(false);
  const [agentsOpen, setAgentsOpen] = useState(false);
  const [skillsOpen, setSkillsOpen] = useState(false);
  const [skillsRevision, setSkillsRevision] = useState(0);
  const [schedulesOpen, setSchedulesOpen] = useState(false);
  const [mediaOpen, setMediaOpen] = useState(false);
  const [pullRequestsOpen, setPullRequestsOpen] = useState(false);
  const [mcpManagerRequest, setMcpManagerRequest] = useState(0);
  const [chatSearchRequest, setChatSearchRequest] = useState(0);
  const [composerDraft, setComposerDraft] = useState<{
    id: number;
    text: string;
  } | null>(null);
  const [gitPanelRequest, setGitPanelRequest] = useState<{
    id: number;
    sessionId?: string;
    view: GitPanelView;
  } | null>(null);
  const sidebarOpen = useUiPreferences((s) => s.sidebarOpen);
  const toggleSidebar = useUiPreferences((s) => s.toggleSidebar);
  const appShortcuts = useUiPreferences((s) => s.appShortcuts);
  const pushNotice = useUiPreferences((s) => s.pushNotice);
  const uiSize = useUiPreferences((s) => s.uiSize);
  const interfaceSounds = useUiPreferences((s) => s.interfaceSounds);
  const soundOnInteractions = useUiPreferences((s) => s.soundOnInteractions);
  const chatLayoutStyle = useUiPreferences((s) => s.chatLayoutStyle);
  const threadNavigationPlacement = useUiPreferences((s) => s.threadNavigationPlacement);
  const sidebarRailStyle = useUiPreferences((s) => s.sidebarRailStyle);
  const messageWidth = useUiPreferences((s) => s.messageWidth);
  const avatarStyle = useUiPreferences((s) => s.avatarStyle);
  const codeBlockTheme = useUiPreferences((s) => s.codeBlockTheme);
  const backgroundFit = useUiPreferences((s) => s.backgroundFit);
  const backgroundTreatment = useUiPreferences((s) => s.backgroundTreatment);
  const appClassName = [
    "app",
    `chat-layout-${chatLayoutStyle}`,
    `thread-navigation-${threadNavigationPlacement}`,
    `sidebar-rail-${sidebarRailStyle}`,
    `message-width-${messageWidth}`,
    `avatar-style-${avatarStyle}`,
    `code-theme-${codeBlockTheme}`,
    `bg-fit-${backgroundFit}`,
    `bg-treatment-${backgroundTreatment}`,
  ].join(" ");

  useEffect(() => setInterfaceSoundsEnabled(interfaceSounds), [interfaceSounds]);
  useLayoutEffect(() => {
    const main = mainRef.current;
    if (!main) return;
    main.inert = settingsOpen;
    return () => {
      main.inert = false;
    };
  }, [settingsOpen]);
  useEffect(() => {
    if (!interfaceSounds || !soundOnInteractions) return;
    return installInterfaceSoundClicks();
  }, [interfaceSounds, soundOnInteractions]);

  function focusComposer() {
    document
      .querySelector<HTMLTextAreaElement>('[data-testid="composer-input"]')
      ?.focus();
  }

  function openSettings() {
    if (!settingsReturnFocusRef.current && document.activeElement instanceof HTMLElement) {
      settingsReturnFocusRef.current = document.activeElement;
    }
    setSettingsOpen(true);
  }

  function closeSettings() {
    const returnTarget = settingsReturnFocusRef.current;
    settingsReturnFocusRef.current = null;
    setSettingsOpen(false);
    window.requestAnimationFrame(() => {
      if (returnTarget?.isConnected) returnTarget.focus({ preventScroll: true });
    });
  }

  function startNewChat() {
    void createInteractiveChat();
    focusComposer();
  }

  function reportAppActionError(action: string, error: unknown) {
    pushNotice({
      tone: "error",
      message: `${action} failed: ${error instanceof Error ? error.message : String(error)}`,
    });
  }

  function openDocumentation() {
    void openExternalUrl(DOCS_URL).catch((error) => reportAppActionError("Opening documentation", error));
  }

  function openDiagnostics() {
    void openDiagnosticsFolder().catch((error) => reportAppActionError("Opening diagnostics", error));
  }

  function quitMilim() {
    void requestDesktopQuit().catch((error) => reportAppActionError("Quitting Milim", error));
  }

  function appMenuItems(): ContextMenuItem[] {
    return [
      {
        id: "new-chat",
        label: "New chat",
        icon: <Plus size={13} />,
        detail: shortcutLabel(appShortcuts.newChat),
        action: startNewChat,
      },
      {
        id: "focus-composer",
        label: "Focus composer",
        icon: <Pencil size={13} />,
        detail: shortcutLabel(appShortcuts.focusComposer),
        action: focusComposer,
      },
      ...(threadNavigationPlacement === "sidebar" ? [{
          id: "toggle-sidebar",
          label: sidebarOpen ? "Hide sidebar" : "Show sidebar",
          icon: <SidebarIcon size={13} />,
          detail: shortcutLabel(appShortcuts.toggleSidebar),
          separatorBefore: true,
          action: toggleSidebar,
        }] : []),
      {
        id: "settings",
        label: "Settings",
        icon: <Gear size={13} />,
        action: openSettings,
      },
      {
        id: "documentation",
        label: "Documentation",
        icon: <Globe size={13} />,
        separatorBefore: true,
        action: openDocumentation,
      },
      ...(inTauri ? [
        {
          id: "diagnostics",
          label: "Open diagnostics",
          icon: <FolderOpen size={13} />,
          action: openDiagnostics,
        },
        {
          id: "quit",
          label: "Quit Milim",
          separatorBefore: true,
          action: quitMilim,
        },
      ] : []),
    ];
  }

  function openAppContextMenu(event: MouseEvent<HTMLDivElement>) {
    openContextMenu(event, appMenuItems(), "App");
  }

  function openAppMenu(event: MouseEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.stopPropagation();
    const trigger = event.currentTarget;
    const rect = trigger.getBoundingClientRect();
    openMenuAt({ x: rect.left, y: rect.bottom + 4 }, appMenuItems(), "Milim menu", trigger);
  }

  useEffect(() => {
    if (!inTauri) return;
    try {
      void getCurrentWebview()
        .setZoom(uiSize / 100)
        .catch(() => {});
    } catch {
      /* not in tauri */
    }
  }, [uiSize]);

  useEffect(() => {
    if (!inTauri) return;
    void invoke("set_sidebar_toggle_enabled", {
      enabled: threadNavigationPlacement === "sidebar",
    }).catch(() => {});
  }, [threadNavigationPlacement]);

  useEffect(() => {
    if (!inTauri) return;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen("milim://runtime-failed", () => setRuntimeFailed(true)))
      .then((unlisten) => {
        dispose = unlisten;
      })
      .catch(() => {});
    return () => dispose?.();
  }, []);

  useEffect(() => {
    if (!inTauri) return;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen<string>(APP_MENU_EVENT, (event) => {
        switch (event.payload) {
          case "new-chat":
            startNewChat();
            break;
          case "settings":
            openSettings();
            break;
          case "toggle-sidebar":
            if (useUiPreferences.getState().threadNavigationPlacement === "sidebar") {
              useUiPreferences.getState().toggleSidebar();
            }
            break;
          case "documentation":
            openDocumentation();
            break;
          case "diagnostics":
            openDiagnostics();
            break;
        }
      }))
      .then((unlisten) => {
        dispose = unlisten;
      })
      .catch(() => {});
    return () => dispose?.();
  }, []);

  if (runtimeFailed) {
    return (
      <AppRecoveryScreen
        title="Milim's local service stopped."
        description="Restart Milim to restore the local service. Saved chats and settings will stay put."
        detail="The embedded milim server exited unexpectedly."
      />
    );
  }

  const threadNavigation = (
    <Sidebar
      open={sidebarOpen}
      placement={threadNavigationPlacement}
      onToggle={toggleSidebar}
      onSearchChats={() => setChatSearchRequest((value) => value + 1)}
      onOpenSettings={openSettings}
      onManageSkills={() => setSkillsOpen(true)}
      onManageSchedules={() => setSchedulesOpen(true)}
      onManageMedia={() => setMediaOpen(true)}
      onManagePullRequests={() => setPullRequestsOpen(true)}
      onManageMcp={() => setMcpManagerRequest((value) => value + 1)}
      onGitAction={(text) => setComposerDraft({ id: Date.now(), text })}
      onOpenGitPanel={(sessionId, view = "changes") =>
        setGitPanelRequest({ id: Date.now(), sessionId, view })
      }
    />
  );

  return (
    <div className={appClassName} onContextMenu={openAppContextMenu}>
      <BackgroundLayer />
      <div
        ref={mainRef}
        className="main"
        aria-hidden={settingsOpen || undefined}
      >
        {threadNavigationPlacement === "sidebar" && threadNavigation}
        <div className="content">
          <TopBar
            onOpenAppMenu={openAppMenu}
            threadNavigation={threadNavigationPlacement === "top" ? threadNavigation : undefined}
          />
          <ChatView
            onManageAgents={() => setAgentsOpen(true)}
            onOpenSchedules={() => setSchedulesOpen(true)}
            onOpenSettings={openSettings}
            composerDraft={composerDraft}
            gitPanelRequest={gitPanelRequest}
            mcpManagerRequest={mcpManagerRequest}
            chatSearchRequest={chatSearchRequest}
            skillsRevision={skillsRevision}
            onComposerDraftConsumed={(id) =>
              setComposerDraft((current) =>
                current?.id === id ? null : current,
              )
            }
          />
          {threadNavigationPlacement === "bottom" && threadNavigation}
        </div>
      </div>
      <Suspense fallback={null}>
        <OnboardingGate />
        {settingsOpen && <SettingsPage onClose={closeSettings} />}
        {agentsOpen && <AgentsManager onClose={() => setAgentsOpen(false)} />}
        {skillsOpen && (
          <SkillsManager
            onClose={() => {
              setSkillsOpen(false);
              setSkillsRevision((value) => value + 1);
            }}
          />
        )}
        {schedulesOpen && (
          <SchedulesManager onClose={() => setSchedulesOpen(false)} />
        )}
        {mediaOpen && (
          <MediaManager onClose={() => setMediaOpen(false)} />
        )}
        {pullRequestsOpen && (
          <PullRequestsManager onClose={() => setPullRequestsOpen(false)} />
        )}
      </Suspense>
      <AppNoticeHost />
      <MobilePairingApprovalPrompt />
      <AutoUpdater />
      <ResizeHandles />
    </div>
  );
}
