import { lazy, Suspense, useEffect, useMemo, useRef, useState, type CSSProperties, type FormEvent, type KeyboardEvent, type MouseEvent as ReactMouseEvent, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { cancelPreviewPicker, openExternalUrl, setActivePreviewTarget, startPreviewPicker, takePreviewPicker, type ChatArtifact, type PreviewAppPreflight, type PreviewAppStatus, type PreviewSurfaceCapability, type PreviewSurfaceKind, type PreviewSurfaceTarget } from "../api";
import type { ArtifactRevision, ArtifactRevisionGroup } from "../lib/artifactRevisions";
import { buildArtifactPreviewDocument, previewKindForArtifact } from "../lib/artifactPreview";
import { isFileArtifact, isPreviewableArtifact, normalizeArtifactBrowserUrl, resolveArtifactBrowserInput } from "../lib/artifacts";
import { useBrowserRecentVisits, type BrowserRecentVisit } from "../browser/recentVisits";
import type { PreviewControlActivity } from "../lib/previewActivity";
import { googleWorkspaceUrl } from "../lib/googleWorkspace";
import type { SessionBrowserSession, SessionBrowserTab } from "../sessions/store";
import { createPreviewWebview, listenForPreviewWebviewNavigation, listenForPreviewWebviewNewTab, listenForPreviewWebviewShortcut, listenForPreviewWebviewTitle, navigatePreviewWebview, reloadPreviewWebview, setPreviewWebviewBounds, setPreviewWebviewMuted, setPreviewWebviewVisibility, setPreviewWebviewZoom, type PreviewBrowserStorageMode, type PreviewWebviewLoadState, type PreviewWebviewNavigation, type PreviewWebviewShortcut, type PreviewWebviewTitle } from "../lib/previewWebview";
import { useSettings } from "../settings/store";
import { nextPreviewBrowserZoom, useUiPreferences } from "../ui/store";
import { shortcutLabel, shortcutMatchesEvent } from "../ui/shortcuts";
import { requestWorkspaceEditorLeave } from "../lib/workspaceEditorGuard";
import { useContextMenu } from "./ContextMenu";
import { ArrowLeft, ArrowRight, Bolt, Code, Copy, Download, ExternalLink, Eye, Globe, MoreHorizontal, Plus, Refresh, Sidebar, Square, Terminal, VolumeX, X } from "./icons";
import { GoogleWorkspacePreview } from "./GoogleWorkspacePreview";
import { Logo } from "./Logo";
import { PaneResizeHandle } from "./PaneResizeHandle";
import { WorkspaceCodePanel } from "./WorkspaceCodePanel";

const Markdown = lazy(() => import("./Markdown").then((mod) => ({ default: mod.Markdown })));

export type PreviewTab = "preview" | "code";
export type PreviewSource = "artifact" | "app" | "url";
export type PreviewBrowserPage = SessionBrowserTab;
export type PreviewBrowserSession = SessionBrowserSession;
type PreviewLogLevel = "log" | "info" | "warn" | "error";
type NativeWebviewHandle = {
  label: string;
  hide: () => Promise<void>;
  show: () => Promise<void>;
  once: <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;
  setPosition: (position: unknown) => Promise<void>;
  setSize: (size: unknown) => Promise<void>;
};
type NativeOverlayWindowHandle = NativeWebviewHandle & {
  setIgnoreCursorEvents: (ignore: boolean) => Promise<void>;
};
type NativeVisibleHandle = Pick<NativeWebviewHandle, "hide" | "show">;
type NativePreviewClaim = { label: string; claimToken: number };
type PreviewControlOverlayPoint = { x: number; y: number };

type PreviewControlOverlayPayload = {
  id: string;
  gesture: PreviewControlActivity["gesture"];
  label: string;
  status: PreviewControlActivity["status"];
  point?: PreviewControlOverlayPoint;
  dark: boolean;
  accent?: string;
  accentLight?: string;
  accentGlow?: string;
  focusBorder?: string;
};

type PreviewLogEntry = {
  id: number;
  level: PreviewLogLevel;
  label?: string;
  message: string;
  timestamp: number;
  stack?: string;
  source?: string;
  line?: number;
  column?: number;
};

const PREVIEW_LOG_EVENT = "milim-artifact-log";
const MAX_PREVIEW_LOGS = 200;
const LOG_DRAWER_MIN_HEIGHT = 48;
const LOG_DRAWER_DEFAULT_HEIGHT = 142;
const LOG_DRAWER_MAX_HEIGHT = 360;
const LOG_DRAWER_KEYBOARD_STEP = 24;
const PREVIEW_CONTROL_OVERLAY_CLOSE_MS = 3400;
const PREVIEW_CONTROL_OVERLAY_STORAGE_PREFIX = "milim-preview-control-activity:";
const PREVIEW_CONTROL_OVERLAY_LABEL = "artifact-overlay-preview";
const PREVIEW_CONTROL_OVERLAY_CHANNEL = "preview-control-overlay";
const MAX_PREVIEW_BROWSER_TABS = 24;
let previewControlOverlayOwner = 0;
const PREVIEW_TAB_IDS: Record<PreviewTab, string> = {
  preview: "inspector-tab-preview",
  code: "inspector-tab-code",
};
const PREVIEW_PANEL_IDS: Record<PreviewTab, string> = {
  preview: "inspector-panel-preview",
  code: "inspector-panel-code",
};
const IS_TAURI = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const APP_FLOATING_UI_SELECTOR = '[data-native-preview-blocker="true"], [data-native-preview-blocker="open"][open]';
const DOM_PREVIEW_CAPABILITIES: PreviewSurfaceCapability[] = ["dom_snapshot", "click", "type", "key", "scroll", "logs", "annotate", "source"];

export function nativePreviewBlockedByAppUi(root: Pick<ParentNode, "querySelector"> = document): boolean {
  return Boolean(root.querySelector(APP_FLOATING_UI_SELECTOR));
}

export function previewSurfaceIsInspectable(surface: PreviewSurfaceTarget | null): boolean {
  return Boolean(surface?.status === "ready" && surface.capabilities.includes("dom_snapshot"));
}

export function previewNativeWebviewLabel(
  surfaceKind: PreviewSurfaceKind,
  storageMode: PreviewBrowserStorageMode,
): string {
  const surface = surfaceKind === "native_browser" ? "url" : "app";
  return `artifact-browser-${surface}-${surface === "app" ? "private" : storageMode}-slot`;
}

async function publishPreviewSurface(
  surface: PreviewSurfaceTarget | null,
  onSurfaceChange?: (surface: PreviewSurfaceTarget | null) => void,
  threadId?: string,
) {
  onSurfaceChange?.(surface);
  await setActivePreviewTarget(surface, threadId).catch(() => undefined);
}

export function PreviewPanel({
  artifact,
  artifacts,
  fixArtifact,
  fixArtifacts,
  fixRevision,
  revision,
  revisionGroup,
  closing = false,
  noEnterMotion = false,
  previewDeferred = false,
  activeTab: controlledActiveTab,
  onClose,
  onSelectRevision,
  onSendArtifactFixPrompt,
  onPrepareArtifactFix,
  onActiveTabChange,
  previewSource: controlledPreviewSource,
  availablePreviewSources,
  onPreviewSourceChange,
  browserSession: controlledBrowserSession,
  onBrowserSessionChange,
  runtimeStatus,
  runtimeBusy = false,
  runtimePreflight,
  runtimePreflightBusy = false,
  runtimeStale = false,
  onRuntimePreflight,
  onRuntimeStart,
  onRuntimeStop,
  onRuntimeRestart,
  controlActivity,
  onSurfaceChange,
  threadId = "",
  modeSwitcher,
  style,
  workspaceFolder,
  onPreviewWorkspaceFile,
}: {
  artifact: ChatArtifact;
  artifacts?: readonly ChatArtifact[];
  fixArtifact?: ChatArtifact;
  fixArtifacts?: readonly ChatArtifact[];
  fixRevision?: ArtifactRevision;
  revision?: ArtifactRevision;
  revisionGroup?: ArtifactRevisionGroup;
  closing?: boolean;
  noEnterMotion?: boolean;
  previewDeferred?: boolean;
  activeTab?: PreviewTab;
  onClose: () => void;
  onSelectRevision?: (revision: ArtifactRevision) => void;
  /** @deprecated Use onPrepareArtifactFix so the controller can queue an editable draft. */
  onSendArtifactFixPrompt?: (prompt: string) => void;
  onPrepareArtifactFix?: (prompt: string) => void;
  onActiveTabChange?: (tab: PreviewTab) => void;
  previewSource?: PreviewSource;
  availablePreviewSources?: readonly PreviewSource[];
  onPreviewSourceChange?: (source: PreviewSource) => void;
  browserSession?: PreviewBrowserSession;
  onBrowserSessionChange?: (session: PreviewBrowserSession) => void;
  runtimeStatus?: PreviewAppStatus | null;
  runtimeBusy?: boolean;
  runtimePreflight?: PreviewAppPreflight | null;
  runtimePreflightBusy?: boolean;
  runtimeStale?: boolean;
  onRuntimePreflight?: () => void;
  onRuntimeStart?: () => void;
  onRuntimeStop?: () => void;
  onRuntimeRestart?: () => void;
  controlActivity?: PreviewControlActivity | null;
  onSurfaceChange?: (surface: PreviewSurfaceTarget | null) => void;
  threadId?: string;
  modeSwitcher?: ReactNode;
  style?: CSSProperties;
  workspaceFolder?: string;
  onPreviewWorkspaceFile?: (path: string) => void;
}) {
  const { openContextMenu } = useContextMenu();
  const browserStorageMode = useSettings((state) => state.browserStorageMode);
  const browserSetupSeen = useSettings((state) => state.browserSetupSeen);
  const setBrowserStorageMode = useSettings((state) => state.setBrowserStorageMode);
  const setBrowserSetupSeen = useSettings((state) => state.setBrowserSetupSeen);
  const browserRecentVisits = useBrowserRecentVisits((state) => state.visits);
  const recordBrowserVisit = useBrowserRecentVisits((state) => state.recordVisit);
  const clearBrowserRecentVisits = useBrowserRecentVisits((state) => state.clearVisits);
  const panelRef = useRef<HTMLElement | null>(null);
  const returnFocusRef = useRef<HTMLElement | null>(typeof document !== "undefined" && document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const tabRefs = useRef<Record<PreviewTab, HTMLButtonElement | null>>({ preview: null, code: null });
  const [localActiveTab, setLocalActiveTab] = useState<PreviewTab>(previewDeferred ? "code" : "preview");
  const activeTab = controlledActiveTab ?? localActiveTab;
  const [frameKey, setFrameKey] = useState(0);
  const [copied, setCopied] = useState(false);
  const [selectedCodeArtifactId, setSelectedCodeArtifactId] = useState(artifact.id);
  const [previewDocument, setPreviewDocument] = useState({ key: `${artifact.id}:${artifact.content}`, source: artifact.content });
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [logs, setLogs] = useState<PreviewLogEntry[]>([]);
  const [logsOpen, setLogsOpen] = useState(false);
  const [runtimeDetailsOpen, setRuntimeDetailsOpen] = useState(false);
  const [runtimePanelFocused, setRuntimePanelFocused] = useState(false);
  const [runtimeLogsClearedAt, setRuntimeLogsClearedAt] = useState(0);
  const [logDrawerHeight, setLogDrawerHeight] = useState(LOG_DRAWER_DEFAULT_HEIGHT);
  const [logResizing, setLogResizing] = useState(false);
  const [iframeReadyKey, setIframeReadyKey] = useState<string | null>(null);
  const [reviewCommentDraft, setReviewCommentDraft] = useState("");
  const [reviewCommentOpen, setReviewCommentOpen] = useState(false);
  const reviewCommentSubmitRef = useRef<((body: string) => void) | null>(null);
  const reviewCommentReturnFocusRef = useRef<HTMLElement | null>(null);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const pickerRunRef = useRef(0);
  const logIdRef = useRef(0);
  const logResizeStartRef = useRef<{ clientY: number; height: number } | null>(null);
  const previewWasDeferredRef = useRef(previewDeferred);
  const nativeBrowserClaimsRef = useRef(new Map<string, NativePreviewClaim>());
  const handledNewTabRequestsRef = useRef(new Set<number>());
  const pendingNativeUrlRef = useRef(new Map<string, string>());
  const runtimeStatusTriggerRef = useRef<HTMLButtonElement | null>(null);
  const runtimePanelRef = useRef<HTMLElement | null>(null);
  const title = artifact.filename ?? artifact.title;
  const source = useMemo(() => artifact.content, [artifact.content]);
  const isUrlPreview = artifact.mime === "text/uri-list";
  const previewUrl = isUrlPreview ? normalizeArtifactBrowserUrl(source) : null;
  const previewKind = isUrlPreview ? "html" : previewKindForArtifact(artifact);
  const [localBrowserSession, setLocalBrowserSession] = useState<PreviewBrowserSession>(() => initialBrowserSession(previewUrl));
  const browserSession = controlledBrowserSession ?? localBrowserSession;
  const browserSessionRef = useRef(browserSession);
  browserSessionRef.current = browserSession;
  const activeBrowserPage = browserSession.tabs.find((tab) => tab.id === browserSession.activeTabId) ?? browserSession.tabs[0] ?? initialBrowserPage(null, "main");
  const { url: browserUrl, input: browserInput, history: browserHistory, historyIndex: browserHistoryIndex } = activeBrowserPage;
  const [browserError, setBrowserError] = useState<string | null>(null);
  const [browserRecentOpen, setBrowserRecentOpen] = useState(false);
  const [browserRecentEditing, setBrowserRecentEditing] = useState(false);
  const [browserRecentIndex, setBrowserRecentIndex] = useState(-1);
  const artifactContext = useMemo(() => (artifacts?.length ? artifacts : [artifact]), [artifact, artifacts]);
  const codeFiles = useMemo(
    () => artifactContext.map((item) => ({ artifact: item, path: artifactLabel(item), entry: item.id === artifact.id })),
    [artifact.id, artifactContext],
  );
  const selectedCodeFile = codeFiles.find((file) => file.artifact.id === selectedCodeArtifactId) ?? codeFiles.find((file) => file.entry) ?? codeFiles[0];
  const selectedCodeArtifact = selectedCodeFile?.artifact ?? artifact;
  const selectedSource = selectedCodeArtifact.content;
  const runtimeLogs = useMemo(
    () => (runtimeStatus?.logs ?? []).filter((log) => log.ts > runtimeLogsClearedAt).slice(-MAX_PREVIEW_LOGS).map((log, index): PreviewLogEntry => ({
      id: log.seq == null ? -index - 1 : -log.seq - 1,
      level: log.stream === "system" ? "info" : "log",
      label: log.stream,
      message: log.line,
      timestamp: log.ts,
    })),
    [runtimeLogsClearedAt, runtimeStatus?.logs],
  );
  const visibleLogs = useMemo(() => [...logs, ...runtimeLogs].slice(-MAX_PREVIEW_LOGS), [logs, runtimeLogs]);
  const errorLogs = visibleLogs.filter((log) => log.level === "error");
  const fixLogs = visibleLogs.filter((log) => log.level === "error" || log.label === "stderr");
  const selectedPreviewSource = controlledPreviewSource ?? (isUrlPreview ? "url" : "artifact");
  const previewBrowserZoom = useUiPreferences((state) => state.previewBrowserZoom);
  const setPreviewBrowserZoom = useUiPreferences((state) => state.setPreviewBrowserZoom);
  const previewBrowserZoomSource = selectedPreviewSource === "app" ? "app" : "url";
  const previewBrowserZoomPercent = previewBrowserZoom[previewBrowserZoomSource];
  const browserRecentEnabled = selectedPreviewSource === "url" && browserStorageMode === "persistent";
  const filteredBrowserRecentVisits = useMemo(() => {
    const query = browserRecentEditing ? browserInput.trim().toLowerCase() : "";
    return browserRecentVisits.filter((visit) => !query || `${visit.title ?? ""} ${visit.url}`.toLowerCase().includes(query)).slice(0, 8);
  }, [browserInput, browserRecentEditing, browserRecentVisits]);
  const runtimeError = runtimeErrorMessage(runtimeStatus);
  const prepareArtifactFix = onPrepareArtifactFix ?? onSendArtifactFixPrompt;
  const canPrepareFix = Boolean(selectedPreviewSource !== "url" && prepareArtifactFix && (previewError || runtimeError || errorLogs.length));
  const canSwitchRevisions = Boolean(revision && revisionGroup && revisionGroup.revisions.length > 1 && onSelectRevision);
  const canGoBack = isUrlPreview && browserHistoryIndex > 0;
  const canGoForward = isUrlPreview && browserHistoryIndex >= 0 && browserHistoryIndex < browserHistory.length - 1;

  useEffect(() => {
    if (!browserRecentEnabled) setBrowserRecentOpen(false);
  }, [browserRecentEnabled]);
  const iframeSurfaceKey = `${artifact.id}:${frameKey}`;
  const previewBuildKey = `${artifact.id}:${source}`;
  const previewDocumentReady = previewDocumentReadyForSurface(
    isUrlPreview,
    previewDocument.key,
    previewBuildKey,
  );
  const previewSources = availablePreviewSources?.length ? availablePreviewSources : [selectedPreviewSource];
  const previewAvailable = selectedPreviewSource !== "artifact" || isUrlPreview || isPreviewableArtifact(artifact);
  const resolvedRuntimePreflight = runtimePreflight ?? runtimeStatus?.preflight ?? null;
  const runtimeIsStale = runtimeStale || Boolean(runtimeStatus?.stale);
  const runtimeHealthy = Boolean(runtimeStatus && previewRuntimeIsHealthy(runtimeStatus, runtimeIsStale));
  const showRuntimeControls = Boolean(runtimeStatus && (onRuntimePreflight || onRuntimeStart || runtimeStatus.active || resolvedRuntimePreflight));
  const showRuntimePanel = Boolean(
    showRuntimeControls &&
    runtimeStatus &&
    (runtimePanelFocused || runtimeDetailsOpen || !runtimeHealthy),
  );
  const previousRuntimeHealthyRef = useRef(runtimeHealthy);
  const contextSource = activeTab === "code" ? "artifact" : selectedPreviewSource;
  const inspectorTitle = activeTab === "code"
    ? title
    : selectedPreviewSource === "app"
      ? basename(resolvedRuntimePreflight?.cwd || runtimeStatus?.cwd || "") || "Generated app"
      : selectedPreviewSource === "url"
        ? browserUrl || "New URL"
        : title;

  async function setActiveTab(tab: PreviewTab, focus = false): Promise<void> {
    if (tab === "preview" && !previewAvailable) return;
    if (tab !== activeTab && activeTab === "code" && !(await requestWorkspaceEditorLeave("navigate"))) return;
    if (controlledActiveTab === undefined) setLocalActiveTab(tab);
    if (tab !== activeTab) onActiveTabChange?.(tab);
    if (focus) tabRefs.current[tab]?.focus();
  }

  function updateBrowserSession(next: PreviewBrowserSession) {
    browserSessionRef.current = next;
    if (!controlledBrowserSession) setLocalBrowserSession(next);
    onBrowserSessionChange?.(next);
  }

  function updateBrowserPage(page: PreviewBrowserPage) {
    const current = browserSessionRef.current;
    updateBrowserSession({
      ...current,
      tabs: current.tabs.map((tab) => tab.id === page.id ? page : tab),
    });
  }

  function selectBrowserTab(tabId: string) {
    if (!browserSession.tabs.some((tab) => tab.id === tabId)) return;
    updateBrowserSession({ ...browserSession, activeTabId: tabId });
    setBrowserError(null);
  }

  function openBrowserTab(url: string | null) {
    const normalized = url === "about:blank" || !url ? null : normalizeArtifactBrowserUrl(url);
    if (url && url !== "about:blank" && !normalized) {
      setBrowserError("Blocked new tab: preview URLs must use HTTPS or loopback HTTP.");
      return;
    }
    const tab = initialBrowserPage(normalized);
    const current = browserSessionRef.current;
    if (current.tabs.length >= MAX_PREVIEW_BROWSER_TABS) {
      setBrowserError(`Preview browser tabs are limited to ${MAX_PREVIEW_BROWSER_TABS}.`);
      return;
    }
    updateBrowserSession({
      ...current,
      tabs: [...current.tabs, tab],
      activeTabId: tab.id,
    });
    setBrowserError(null);
  }

  function openRequestedBrowserTab(requestId: number, url: string) {
    if (handledNewTabRequestsRef.current.has(requestId)) return;
    handledNewTabRequestsRef.current.add(requestId);
    openBrowserTab(url);
  }

  function closeBrowserTab(tabId: string) {
    const index = browserSession.tabs.findIndex((tab) => tab.id === tabId);
    if (index < 0) return;
    nativeBrowserClaimsRef.current.delete(tabId);
    pendingNativeUrlRef.current.delete(tabId);
    const tabs = browserSession.tabs.filter((tab) => tab.id !== tabId);
    if (!tabs.length) {
      const blank = initialBrowserPage(null);
      updateBrowserSession({ ...browserSession, tabs: [blank], activeTabId: blank.id });
      return;
    }
    const activeTabId = browserSession.activeTabId === tabId
      ? tabs[Math.min(index, tabs.length - 1)].id
      : browserSession.activeTabId;
    updateBrowserSession({ ...browserSession, tabs, activeTabId });
  }

  function closePanel() {
    onClose();
    window.requestAnimationFrame(() => returnFocusRef.current?.focus({ preventScroll: true }));
  }

  function focusRuntimeStatusTrigger() {
    window.requestAnimationFrame(() => runtimeStatusTriggerRef.current?.focus({ preventScroll: true }));
  }

  function runRuntimeAction(action?: () => void) {
    action?.();
    focusRuntimeStatusTrigger();
  }

  function toggleRuntimeDetails() {
    if (runtimeHealthy) {
      setRuntimeDetailsOpen((open) => !open);
      return;
    }
    runtimePanelRef.current?.focus({ preventScroll: true });
  }

  function handlePanelKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (!event.defaultPrevented && isUrlPreview && shortcutMatchesEvent("Mod+T", event)) {
      event.preventDefault();
      event.stopPropagation();
      blankBrowser();
      return;
    }
    if (!event.defaultPrevented && isUrlPreview && shortcutMatchesEvent("Mod+W", event)) {
      event.preventDefault();
      event.stopPropagation();
      closeBrowserTab(browserSession.activeTabId);
      return;
    }
    if (event.key !== "Escape" || event.defaultPrevented) return;
    event.preventDefault();
    event.stopPropagation();
    closePanel();
  }

  function handleTabKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    const tabs = isUrlPreview ? (["preview"] as const) : previewAvailable ? (["preview", "code"] as const) : (["code"] as const);
    const nextTab = nextPreviewTab(activeTab, event.key, tabs);
    if (!nextTab) return;
    event.preventDefault();
    void setActiveTab(nextTab, true);
  }

  useEffect(() => {
    const panel = panelRef.current;
    window.requestAnimationFrame(() => {
      const activeView = panel?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
      (activeView ?? panel)?.focus({ preventScroll: true });
    });
    return () => {
      if (document.activeElement === document.body || document.activeElement === null) {
        returnFocusRef.current?.focus({ preventScroll: true });
      }
    };
  }, []);

  useEffect(() => {
    setSelectedCodeArtifactId(artifact.id);
  }, [artifact.id]);

  useEffect(() => {
    if (!isUrlPreview) return;
    if (!controlledBrowserSession) setLocalBrowserSession(initialBrowserSession(previewUrl));
    setBrowserError(null);
  }, [artifact.id, controlledBrowserSession, isUrlPreview, previewUrl]);

  useEffect(() => {
    if (isUrlPreview && activeTab !== "preview") setActiveTab("preview");
  }, [activeTab, isUrlPreview]);

  useEffect(() => {
    if (!isUrlPreview && previewDeferred && activeTab === "preview") setActiveTab("code");
  }, [activeTab, isUrlPreview, previewDeferred]);

  useEffect(() => {
    if (!previewAvailable && activeTab === "preview") setActiveTab("code");
  }, [activeTab, previewAvailable]);

  useEffect(() => {
    if (previewWasDeferredRef.current && !previewDeferred) setActiveTab("preview");
    previewWasDeferredRef.current = previewDeferred;
  }, [previewDeferred]);

  useEffect(() => {
    if (codeFiles.length && !codeFiles.some((file) => file.artifact.id === selectedCodeArtifactId)) {
      setSelectedCodeArtifactId(codeFiles[0].artifact.id);
    }
  }, [codeFiles, selectedCodeArtifactId]);

  useEffect(() => {
    if (previewDeferred) return;
    let cancelled = false;
    setPreviewError(null);
    if (isUrlPreview) return;
    if (previewKind === "markdown") {
      setPreviewDocument({ key: previewBuildKey, source });
      return () => {
        cancelled = true;
      };
    }
    setPreviewDocument({ key: "", source: "" });
    void buildArtifactPreviewDocument(artifact, artifactContext)
      .then((document) => {
        if (cancelled) return;
        setPreviewDocument({ key: previewBuildKey, source: document.source });
      })
      .catch((e) => {
        if (cancelled) return;
        setPreviewDocument({ key: previewBuildKey, source: "" });
        setPreviewError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [artifact, artifactContext, isUrlPreview, previewBuildKey, previewDeferred, previewKind, source]);

  useEffect(() => {
    setLogs([]);
    setLogsOpen(false);
  }, [artifact.id, browserUrl, frameKey, source]);

  useEffect(() => {
    setIframeReadyKey(null);
  }, [artifact.id, frameKey, previewDocument.source]);

  useEffect(() => {
    if (runtimeStatus?.status === "error") setLogsOpen(true);
  }, [runtimeStatus?.status]);

  useEffect(() => {
    const becameHealthy = !previousRuntimeHealthyRef.current && runtimeHealthy;
    previousRuntimeHealthyRef.current = runtimeHealthy;
    if (!runtimeHealthy) {
      setRuntimeDetailsOpen(false);
      return;
    }
    if (becameHealthy && runtimePanelFocused) focusRuntimeStatusTrigger();
  }, [runtimeHealthy, runtimePanelFocused]);

  useEffect(() => {
    if (isUrlPreview && browserUrl) return;
    let surface: PreviewSurfaceTarget;
    if (isUrlPreview) {
      surface = { kind: "blank", title: "Browser", native: false, status: "not_inspectable", capabilities: [] };
    } else if (activeTab === "code") {
      surface = { kind: "code", title, native: false, status: "not_inspectable", capabilities: ["source"] };
    } else if (previewKind === "markdown") {
      surface = { kind: "markdown", title, native: false, status: "not_inspectable", capabilities: ["source"] };
    } else if (previewDeferred || !previewDocumentReady) {
      surface = { kind: "artifact_iframe", title, message: "Building artifact preview", native: false, status: "loading", capabilities: ["source"] };
    } else if (previewError) {
      surface = { kind: "artifact_iframe", title, message: previewError, native: false, status: "error", capabilities: ["logs", "source"] };
    } else if (iframeReadyKey === iframeSurfaceKey) {
      surface = { label: "main", kind: "artifact_iframe", title, url: title, native: false, status: "ready", capabilities: DOM_PREVIEW_CAPABILITIES };
    } else {
      surface = { kind: "artifact_iframe", title, native: false, status: "loading", capabilities: ["source"] };
    }
    void publishPreviewSurface(surface, onSurfaceChange, threadId);
    return () => {
      void publishPreviewSurface(null, onSurfaceChange, threadId);
    };
  }, [activeTab, browserUrl, iframeReadyKey, iframeSurfaceKey, isUrlPreview, onSurfaceChange, previewDeferred, previewDocumentReady, previewError, previewKind, threadId, title]);

  function requestReviewComment(onSubmit: (body: string) => void) {
    reviewCommentSubmitRef.current = onSubmit;
    reviewCommentReturnFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setReviewCommentDraft("");
    setReviewCommentOpen(true);
  }

  function closeReviewComment() {
    setReviewCommentOpen(false);
    reviewCommentSubmitRef.current = null;
    window.requestAnimationFrame(() => reviewCommentReturnFocusRef.current?.focus({ preventScroll: true }));
  }

  function submitReviewComment(event: FormEvent) {
    event.preventDefault();
    const body = reviewCommentDraft.trim();
    const submit = reviewCommentSubmitRef.current;
    if (!body || !submit) return;
    setReviewCommentOpen(false);
    reviewCommentSubmitRef.current = null;
    submit(body);
    window.requestAnimationFrame(() => reviewCommentReturnFocusRef.current?.focus({ preventScroll: true }));
  }

  useEffect(() => {
    function onMessage(event: MessageEvent) {
      if (event.source !== iframeRef.current?.contentWindow) return;
      if (event.data?.type === "milim-preview-annotation") {
        requestReviewComment((body) => {
          window.dispatchEvent(new CustomEvent("milim:add-review-comment", { detail: {
            id: `review-${Date.now()}-${Math.random().toString(16).slice(2)}`,
            surface: "preview",
            body,
            timestamp: Date.now(),
            preview: event.data.value,
          } }));
        });
        return;
      }
      const entry = normalizePreviewLog(event.data, ++logIdRef.current);
      if (!entry) return;
      setLogs((current) => [...current, entry].slice(-MAX_PREVIEW_LOGS));
      if (entry.level === "error") setLogsOpen(true);
    }
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  useEffect(() => () => {
    pickerRunRef.current += 1;
    void cancelPreviewPicker().catch(() => undefined);
    iframeRef.current?.contentWindow?.postMessage({ type: "milim-preview-annotation-cancel" }, "*");
  }, [frameKey, activeTab, selectedPreviewSource]);

  async function startPreviewAnnotation() {
    if (!isUrlPreview) {
      iframeRef.current?.contentWindow?.postMessage({ type: "milim-preview-annotation-start" }, "*");
      return;
    }
    const run = ++pickerRunRef.current;
    await startPreviewPicker();
    for (let attempt = 0; attempt < 120 && pickerRunRef.current === run; attempt += 1) {
      await new Promise((resolve) => window.setTimeout(resolve, 250));
      const selection = await takePreviewPicker();
      if (!selection) continue;
      requestReviewComment((body) => {
        window.dispatchEvent(new CustomEvent("milim:add-review-comment", { detail: {
          id: `review-${Date.now()}-${Math.random().toString(16).slice(2)}`,
          surface: "preview", body, timestamp: Date.now(), preview: selection,
        } }));
      });
      return;
    }
    await cancelPreviewPicker().catch(() => undefined);
  }

  async function copySource() {
    await navigator.clipboard?.writeText(isUrlPreview ? browserUrl ?? browserInput : activeTab === "code" ? selectedSource : source);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }

  function downloadSource() {
    const currentArtifact = activeTab === "code" ? selectedCodeArtifact : artifact;
    const blob = new Blob([currentArtifact.content], { type: currentArtifact.mime || "text/plain" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = downloadName(currentArtifact);
    link.click();
    URL.revokeObjectURL(url);
  }

  function openBrowserUrl(url: string) {
    setBrowserError(null);
    const nextHistory = browserHistory.slice(0, Math.max(browserHistoryIndex + 1, 0));
    if (nextHistory[nextHistory.length - 1] !== url) nextHistory.push(url);
    if (IS_TAURI && nativeBrowserClaimsRef.current.has(activeBrowserPage.id)) {
      pendingNativeUrlRef.current.set(activeBrowserPage.id, url);
    }
    updateBrowserPage({
      ...activeBrowserPage,
      url,
      input: url,
      history: nextHistory,
      historyIndex: nextHistory.length - 1,
      title: activeBrowserPage.url === url ? activeBrowserPage.title : undefined,
      faviconUrl: browserFaviconUrl(url),
    });
  }

  function submitBrowserUrl(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const normalized = resolveArtifactBrowserInput(browserInput);
    if (!normalized) {
      setBrowserError("Enter a search or a valid HTTPS or local URL.");
      return;
    }
    openBrowserUrl(normalized);
    setBrowserRecentOpen(false);
    setBrowserRecentEditing(false);
  }

  function openRecentBrowserVisit(visit: BrowserRecentVisit) {
    openBrowserUrl(visit.url);
    setBrowserRecentOpen(false);
    setBrowserRecentEditing(false);
    setBrowserRecentIndex(-1);
  }

  function handleBrowserAddressKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Escape" && browserRecentOpen) {
      event.preventDefault();
      event.stopPropagation();
      setBrowserRecentOpen(false);
      setBrowserRecentIndex(-1);
      return;
    }
    if (event.key === "Enter" && browserRecentOpen && browserRecentIndex >= 0) {
      const visit = filteredBrowserRecentVisits[browserRecentIndex];
      if (!visit) return;
      event.preventDefault();
      openRecentBrowserVisit(visit);
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    if (!browserRecentEnabled || !filteredBrowserRecentVisits.length) return;
    event.preventDefault();
    setBrowserRecentOpen(true);
    setBrowserRecentIndex((current) => event.key === "ArrowDown"
      ? (current + 1) % filteredBrowserRecentVisits.length
      : (current <= 0 ? filteredBrowserRecentVisits.length : current) - 1);
  }

  function navigateBrowser(delta: -1 | 1) {
    const nextIndex = browserHistoryIndex + delta;
    const nextUrl = browserHistory[nextIndex];
    if (!nextUrl) return;
    updateBrowserPage({ ...activeBrowserPage, url: nextUrl, input: nextUrl, history: browserHistory, historyIndex: nextIndex });
    setBrowserError(null);
  }

  function syncNativeNavigation(tabId: string, url: string, state: PreviewWebviewLoadState) {
    if (state === "error") return;
    const tab = browserSessionRef.current.tabs.find((item) => item.id === tabId);
    if (!tab) return;
    if (state === "ready" && selectedPreviewSource === "url" && browserStorageMode === "persistent") {
      recordBrowserVisit(url, tab.url === url ? tab.title : undefined);
    }
    const metadata = tab.url === url ? {} : { title: undefined, faviconUrl: browserFaviconUrl(url) };
    if (state !== "ready") {
      updateBrowserPage({ ...tab, ...metadata, url, input: url });
      setBrowserError(null);
      return;
    }
    if (pendingNativeUrlRef.current.has(tabId)) {
      pendingNativeUrlRef.current.delete(tabId);
      const nextHistory = [...tab.history];
      const index = Math.min(Math.max(tab.historyIndex, 0), Math.max(nextHistory.length - 1, 0));
      if (nextHistory.length) nextHistory[index] = url;
      else nextHistory.push(url);
      updateBrowserPage({ ...tab, ...metadata, url, input: url, history: nextHistory, historyIndex: index });
    } else if (tab.history[tab.historyIndex] !== url) {
      const nextHistory = tab.history.slice(0, Math.max(tab.historyIndex + 1, 0));
      if (nextHistory[nextHistory.length - 1] !== url) nextHistory.push(url);
      updateBrowserPage({ ...tab, ...metadata, url, input: url, history: nextHistory, historyIndex: nextHistory.length - 1 });
    } else if (tab.input !== url) {
      updateBrowserPage({ ...tab, input: url });
    }
    setBrowserError(null);
  }

  function handleNativeBrowserError(tabId: string, message: string) {
    pendingNativeUrlRef.current.delete(tabId);
    if (tabId === browserSessionRef.current.activeTabId) setBrowserError(message);
  }

  function syncNativeTitle(tabId: string, title: string, faviconUrl?: string) {
    const tab = browserSessionRef.current.tabs.find((item) => item.id === tabId);
    if (!tab) return;
    updateBrowserPage({
      ...tab,
      title: title.trim() || undefined,
      faviconUrl: faviconUrl || browserFaviconUrl(tab.url),
    });
    if (tab.url && selectedPreviewSource === "url" && browserStorageMode === "persistent") {
      recordBrowserVisit(tab.url, title);
    }
  }

  function blankBrowser() {
    openBrowserTab(null);
  }

  function reloadBrowser() {
    if (!browserUrl) return;
    const claim = nativeBrowserClaimsRef.current.get(activeBrowserPage.id);
    if (IS_TAURI && claim) {
      void reloadPreviewWebview(claim.label, claim.claimToken).catch((error) => setBrowserError(error instanceof Error ? error.message : String(error)));
      return;
    }
    setFrameKey((key) => key + 1);
  }

  function reloadBrowserTab(tab: PreviewBrowserPage) {
    if (!tab.url) return;
    const claim = nativeBrowserClaimsRef.current.get(tab.id);
    if (IS_TAURI && claim) {
      void reloadPreviewWebview(claim.label, claim.claimToken).catch((error) => setBrowserError(error instanceof Error ? error.message : String(error)));
      return;
    }
    if (tab.id !== browserSession.activeTabId) selectBrowserTab(tab.id);
    setFrameKey((key) => key + 1);
  }

  function toggleBrowserTabMuted(tab: PreviewBrowserPage) {
    updateBrowserPage({ ...tab, muted: tab.muted ? undefined : true });
  }

  function openBrowserUrlExternal(url = browserUrl) {
    if (!url) return;
    void openExternalUrl(url).catch((error) => console.warn("failed to open URL", error));
  }

  function copyBrowserUrl(url: string) {
    void navigator.clipboard?.writeText(url);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1200);
  }

  function openBrowserTabContextMenu(event: ReactMouseEvent, tab: PreviewBrowserPage) {
    openContextMenu(event, [
      {
        id: "new-browser-tab",
        label: "New tab",
        icon: <Plus size={13} />,
        detail: shortcutLabel("Mod+T"),
        action: blankBrowser,
      },
      {
        id: "reload-browser-tab",
        label: "Reload",
        icon: <Refresh size={13} />,
        disabled: !tab.url,
        action: () => reloadBrowserTab(tab),
      },
      {
        id: "toggle-browser-tab-muted",
        label: tab.muted ? "Unmute tab" : "Mute tab",
        icon: <VolumeX size={13} />,
        action: () => toggleBrowserTabMuted(tab),
      },
      {
        id: "open-browser-tab-external",
        label: "Open in system browser",
        icon: <ExternalLink size={13} />,
        disabled: !tab.url,
        separatorBefore: true,
        action: () => openBrowserUrlExternal(tab.url),
      },
      {
        id: "copy-browser-tab-url",
        label: "Copy URL",
        icon: <Copy size={13} />,
        disabled: !tab.url,
        action: () => tab.url && copyBrowserUrl(tab.url),
      },
      {
        id: "close-browser-tab",
        label: "Close tab",
        icon: <X size={13} />,
        separatorBefore: true,
        action: () => closeBrowserTab(tab.id),
      },
    ], browserTabLabel(tab));
  }

  function prepareFix() {
    if (!prepareArtifactFix) return;
    prepareArtifactFix(buildFixPrompt(
      fixArtifact ?? artifact,
      fixArtifacts?.length ? fixArtifacts.map(artifactLabel) : codeFiles.map((file) => file.path),
      fixRevision?.revisionNumber ?? revision?.revisionNumber,
      previewError,
      runtimeError,
      fixLogs,
    ));
  }

  function clearLogs() {
    setLogs([]);
    setRuntimeLogsClearedAt((runtimeStatus?.logs ?? []).reduce((latest, log) => Math.max(latest, log.ts), 0));
  }

  function openPreviewContextMenu(event: ReactMouseEvent) {
    openContextMenu(event, [
      ...(isUrlPreview ? [{
        id: "new-browser-tab",
        label: "New tab",
        icon: <Plus size={13} />,
        detail: shortcutLabel("Mod+T"),
        action: blankBrowser,
      }, {
        id: "browser-back",
        label: "Back",
        icon: <ArrowLeft size={13} />,
        disabled: !canGoBack,
        separatorBefore: true,
        action: () => navigateBrowser(-1),
      }, {
        id: "browser-forward",
        label: "Forward",
        icon: <ArrowRight size={13} />,
        disabled: !canGoForward,
        action: () => navigateBrowser(1),
      }, {
        id: "reload-browser",
        label: "Reload",
        icon: <Refresh size={13} />,
        disabled: !browserUrl,
        action: reloadBrowser,
      }] : []),
      ...(!isUrlPreview ? [{
        id: "preview-tab",
        label: "Show preview",
        icon: <Eye size={13} />,
        checked: activeTab === "preview",
        disabled: activeTab === "preview",
        action: () => setActiveTab("preview"),
      }, {
        id: "code-tab",
        label: "Show code",
        icon: <Code size={13} />,
        checked: activeTab === "code",
        disabled: activeTab === "code",
        action: () => setActiveTab("code"),
      }] : []),
      ...(activeTab === "preview" && previewKind === "html" && !isUrlPreview ? [{
        id: "reload-preview",
        label: "Reload preview",
        icon: <Refresh size={13} />,
        separatorBefore: true,
        action: () => setFrameKey((key) => key + 1),
      }] : []),
      ...(isUrlPreview && browserUrl ? [{
        id: "open-external",
        label: "Open in system browser",
        icon: <ExternalLink size={13} />,
        separatorBefore: true,
        action: () => openBrowserUrlExternal(),
      }] : []),
      {
        id: "copy",
        label: isUrlPreview ? "Copy URL" : "Copy source",
        icon: <Copy size={13} />,
        disabled: isUrlPreview && !browserUrl && !browserInput,
        separatorBefore: !isUrlPreview,
        action: () => void copySource(),
      },
      ...(isUrlPreview ? [{
        id: "close-browser-tab",
        label: "Close tab",
        icon: <X size={13} />,
        detail: shortcutLabel("Mod+W"),
        separatorBefore: true,
        action: () => closeBrowserTab(browserSession.activeTabId),
      }] : []),
      ...(!isUrlPreview ? [{
        id: "download",
        label: "Download source",
        icon: <Download size={13} />,
        action: downloadSource,
      }] : []),
      ...(canPrepareFix ? [{
        id: "prepare-fix",
        label: "Prepare fix",
        icon: <Bolt size={13} />,
        separatorBefore: true,
        action: prepareFix,
      }] : []),
      {
        id: "close",
        label: "Close preview",
        icon: <X size={13} />,
        separatorBefore: true,
        action: closePanel,
      },
    ], title);
  }

  function closeOverflowAfterAction(event: ReactMouseEvent<HTMLDivElement>) {
    if (!(event.target instanceof Element) || !event.target.closest("button")) return;
    event.currentTarget.closest("details")?.removeAttribute("open");
  }

  function maxLogDrawerHeight(): number {
    if (typeof window === "undefined") return LOG_DRAWER_MAX_HEIGHT;
    return Math.max(LOG_DRAWER_MIN_HEIGHT, Math.min(LOG_DRAWER_MAX_HEIGHT, Math.round(window.innerHeight * 0.45)));
  }

  function clampLogDrawerHeight(height: number): number {
    return Math.round(Math.min(Math.max(height, LOG_DRAWER_MIN_HEIGHT), maxLogDrawerHeight()));
  }

  function resizeLogDrawer(height: number) {
    setLogDrawerHeight(clampLogDrawerHeight(height));
  }

  function startLogResize(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0) return;
    event.preventDefault();
    logResizeStartRef.current = { clientY: event.clientY, height: logDrawerHeight };
    setLogResizing(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function moveLogResize(event: PointerEvent<HTMLDivElement>) {
    const start = logResizeStartRef.current;
    if (!start) return;
    resizeLogDrawer(start.height + start.clientY - event.clientY);
  }

  function endLogResize(event: PointerEvent<HTMLDivElement>) {
    if (!logResizeStartRef.current) return;
    logResizeStartRef.current = null;
    setLogResizing(false);
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  }

  function resizeLogDrawerWithKeyboard(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "ArrowUp") {
      event.preventDefault();
      resizeLogDrawer(logDrawerHeight + LOG_DRAWER_KEYBOARD_STEP);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      resizeLogDrawer(logDrawerHeight - LOG_DRAWER_KEYBOARD_STEP);
    } else if (event.key === "Home") {
      event.preventDefault();
      resizeLogDrawer(LOG_DRAWER_MIN_HEIGHT);
    } else if (event.key === "End") {
      event.preventDefault();
      resizeLogDrawer(maxLogDrawerHeight());
    }
  }

  return (
    <aside
      ref={panelRef}
      className={`preview-panel${closing ? " closing" : ""}${noEnterMotion ? " no-enter" : ""}`}
      data-testid="chat-preview-split"
      aria-label="Inspector"
      tabIndex={-1}
      style={style}
      onKeyDown={handlePanelKeyDown}
      onContextMenu={openPreviewContextMenu}
    >
      <div className="preview-header" data-testid="preview-header">
      <div className="preview-toolbar">
        <div className="preview-primary-navigation">
          {modeSwitcher ?? (
            <div className="preview-tabs" role="tablist" aria-label="Inspector views">
              {previewAvailable && (
                <button
                  ref={(element) => { tabRefs.current.preview = element; }}
                  id={PREVIEW_TAB_IDS.preview}
                  className={`preview-tab${activeTab === "preview" ? " active" : ""}`}
                  data-testid="preview-tab-preview"
                  role="tab"
                  tabIndex={activeTab === "preview" ? 0 : -1}
                  aria-controls={PREVIEW_PANEL_IDS.preview}
                  aria-selected={activeTab === "preview"}
                  onKeyDown={handleTabKeyDown}
                  onClick={() => setActiveTab("preview")}
                >
                  <Eye size={13} />
                  <span>Preview</span>
                </button>
              )}
              {!isUrlPreview && (
                <button
                  ref={(element) => { tabRefs.current.code = element; }}
                  id={PREVIEW_TAB_IDS.code}
                  className={`preview-tab${activeTab === "code" ? " active" : ""}`}
                  data-testid="preview-tab-code"
                  role="tab"
                  tabIndex={activeTab === "code" ? 0 : -1}
                  aria-controls={PREVIEW_PANEL_IDS.code}
                  aria-selected={activeTab === "code"}
                  onKeyDown={handleTabKeyDown}
                  onClick={() => setActiveTab("code")}
                >
                  <Code size={13} />
                  <span>Code</span>
                </button>
              )}
            </div>
          )}
        </div>
        <button className="preview-action preview-close" title="Close inspector" aria-label="Close inspector" onClick={closePanel}>
          <Sidebar size={16} />
        </button>
      </div>

      <div className="preview-context-toolbar">
        {!isUrlPreview && (
          <div className="preview-context-title" data-testid="preview-context-title">
            <span>{contextSource === "app" ? "App" : contextSource === "url" ? "URL" : "Artifact"}</span>
            <strong title={inspectorTitle}>{inspectorTitle}</strong>
          </div>
        )}
        {activeTab === "preview" && previewSources.length > 1 && (
          <>
            <div className="preview-source-selector" role="group" aria-label="Preview source" data-testid="preview-source-selector">
              {previewSources.map((item) => (
                <button
                  key={item}
                  className={item === selectedPreviewSource ? "active" : ""}
                  aria-pressed={item === selectedPreviewSource}
                  onClick={() => onPreviewSourceChange?.(item)}
                >
                  {previewSourceLabel(item)}
                </button>
              ))}
            </div>
            <label className="preview-source-select">
              <span>Source</span>
              <select
                aria-label="Preview source"
                value={selectedPreviewSource}
                onChange={(event) => onPreviewSourceChange?.(event.currentTarget.value as PreviewSource)}
              >
                {previewSources.map((item) => <option key={item} value={item}>{previewSourceLabel(item)}</option>)}
              </select>
            </label>
          </>
        )}
        {canSwitchRevisions && revision && revisionGroup && (
          <label className="preview-revision-control" title={`${revisionGroup.label} revision`}>
            <span>Version</span>
            <select
              data-testid="preview-revision-select"
              aria-label="Artifact revision"
              value={revision.revisionNumber}
              onChange={(event) => {
                const next = revisionGroup.revisions.find((item) => item.revisionNumber === Number(event.currentTarget.value));
                if (next) onSelectRevision?.(next);
              }}
            >
              {revisionGroup.revisions.map((itemRevision) => (
                <option key={itemRevision.revisionNumber} value={itemRevision.revisionNumber}>
                  v{itemRevision.revisionNumber}{itemRevision.revisionNumber === itemRevision.totalRevisions ? " latest" : ""}
                </option>
              ))}
            </select>
          </label>
        )}
        {activeTab === "preview" && runtimeStatus && (
          <div className="preview-runtime-toolbar">
            <PreviewRuntimeBadge
              status={runtimeStatus}
              stale={runtimeIsStale}
              expanded={showRuntimePanel}
              collapsible={runtimeHealthy}
              buttonRef={(element) => { runtimeStatusTriggerRef.current = element; }}
              onToggle={toggleRuntimeDetails}
            />
            {runtimeHealthy && (
              <button
                className={`preview-action preview-runtime-quick-stop${runtimeStatus.kind === "static" ? " labeled" : ""}`}
                data-testid="preview-runtime-quick-stop"
                title="Stop app preview"
                aria-label="Stop app preview"
                disabled={runtimeBusy || !onRuntimeStop}
                onClick={() => runRuntimeAction(onRuntimeStop)}
              >
                <Square size={12} />
                {runtimeStatus.kind === "static" && <span>Stop</span>}
              </button>
            )}
          </div>
        )}
        <div className="preview-actions preview-secondary-actions" aria-label="Inspector actions">
          {activeTab === "preview" && previewKind === "html" && ((!isUrlPreview && iframeReadyKey === iframeSurfaceKey) || (isUrlPreview && Boolean(browserUrl))) && (
            <button className="preview-action" title="Annotate preview element" aria-label="Annotate preview element" onClick={() => void startPreviewAnnotation()}>
              <Eye size={14} />
            </button>
          )}
          {activeTab === "preview" && previewKind === "html" && !isUrlPreview && (
            <button className="preview-action" title="Reload preview" aria-label="Reload preview" onClick={() => setFrameKey((key) => key + 1)}>
              <Refresh size={14} />
            </button>
          )}
          <button className="preview-action" title={copied ? "Copied" : isUrlPreview ? "Copy URL" : "Copy source"} aria-label={copied ? "Copied" : isUrlPreview ? "Copy URL" : "Copy source"} onClick={() => void copySource()}>
            <Copy size={14} />
          </button>
          {!isUrlPreview && (
            <button className="preview-action" title="Download source" aria-label="Download source" onClick={downloadSource}>
              <Download size={14} />
            </button>
          )}
        </div>
        <details className="preview-overflow" data-native-preview-blocker="open">
          <summary className="preview-action" title="More inspector actions" aria-label="More inspector actions">
            <MoreHorizontal size={14} />
          </summary>
          <div className="preview-overflow-menu" role="group" aria-label="Inspector actions" onClick={closeOverflowAfterAction}>
            {activeTab === "preview" && previewKind === "html" && !isUrlPreview && (
              <button onClick={() => setFrameKey((key) => key + 1)}><Refresh size={13} />Reload preview</button>
            )}
            <button onClick={() => void copySource()}><Copy size={13} />{isUrlPreview ? "Copy URL" : "Copy source"}</button>
            {!isUrlPreview && <button onClick={downloadSource}><Download size={13} />Download source</button>}
          </div>
        </details>
      </div>
      </div>

      <div
        id={PREVIEW_PANEL_IDS.preview}
        className="preview-tab-panel"
        role="tabpanel"
        aria-labelledby={PREVIEW_TAB_IDS.preview}
        hidden={activeTab !== "preview" || !previewAvailable}
      >
        {previewKind === "markdown" ? (
          <div className="preview-markdown">
            <Suspense fallback={<span className="typing">...</span>}>
              <Markdown content={source} />
            </Suspense>
          </div>
        ) : previewDeferred || !previewDocumentReady ? (
          <div className="preview-loading" role="status" aria-live="polite">
            <span className="typing">Building preview...</span>
          </div>
        ) : previewError ? (
          <div className="preview-error" role="alert">
            <strong>Preview failed</strong>
            <span>{previewError}</span>
            {canPrepareFix && (
              <button className="preview-quick-fix" data-testid="preview-prepare-fix" onClick={prepareFix}>
                <Bolt size={13} />
                <span>Prepare fix</span>
              </button>
            )}
          </div>
        ) : (
          <div className="preview-runtime-shell">
            {isUrlPreview && (
              <>
                <div className="preview-browser-tabs" data-testid="preview-browser-tabs" role="tablist" aria-label="Browser tabs">
                  {browserSession.tabs.map((tab) => (
                    <div
                      className={`preview-browser-tab${tab.id === browserSession.activeTabId ? " active" : ""}`}
                      key={tab.id}
                      onContextMenu={(event) => openBrowserTabContextMenu(event, tab)}
                    >
                      <button
                        type="button"
                        role="tab"
                        aria-selected={tab.id === browserSession.activeTabId}
                        aria-label={`${browserTabLabel(tab)}${tab.muted ? ", muted" : ""}`}
                        title={`${tab.url ?? "New tab"}${tab.muted ? " · Muted" : ""}`}
                        onClick={() => selectBrowserTab(tab.id)}
                      >
                        <span className="preview-browser-tab-icon" aria-hidden="true">
                          {tab.faviconUrl && <img key={tab.faviconUrl} src={tab.faviconUrl} alt="" onError={(event) => { event.currentTarget.hidden = true; }} />}
                          <Globe size={12} />
                        </span>
                        <span className="preview-browser-tab-label">{browserTabLabel(tab)}</span>
                        {tab.muted && (
                          <span className="preview-browser-tab-muted" data-testid="preview-browser-tab-muted" title="Tab muted" aria-hidden="true">
                            <VolumeX size={11} />
                          </span>
                        )}
                      </button>
                      <button type="button" className="preview-browser-tab-close" aria-label={`Close ${browserTabLabel(tab)}`} onClick={() => closeBrowserTab(tab.id)}>
                        <X size={11} />
                      </button>
                    </div>
                  ))}
                  <button type="button" className="preview-browser-tab-add" title="New tab" aria-label="Open new tab" onClick={blankBrowser}>
                    <Plus size={13} />
                  </button>
                </div>
                <div className="preview-browser-bar" data-testid="preview-browser-bar">
                  <div className="preview-browser-nav">
                    <button className="preview-browser-action" title="Back" aria-label="Back" disabled={!canGoBack} onClick={() => navigateBrowser(-1)}>
                      <ArrowLeft size={14} />
                    </button>
                    <button className="preview-browser-action" title="Forward" aria-label="Forward" disabled={!canGoForward} onClick={() => navigateBrowser(1)}>
                      <ArrowRight size={14} />
                    </button>
                    <button className="preview-browser-action" title="Reload" aria-label="Reload page" disabled={!browserUrl} onClick={reloadBrowser}>
                      <Refresh size={14} />
                    </button>
                  </div>
                  <form
                    className="preview-browser-form"
                    onSubmit={submitBrowserUrl}
                    onBlur={(event) => {
                      if (event.relatedTarget instanceof Node && event.currentTarget.contains(event.relatedTarget)) return;
                      setBrowserRecentOpen(false);
                      setBrowserRecentIndex(-1);
                    }}
                  >
                    <Globe size={14} aria-hidden="true" />
                    <input
                      data-testid="preview-browser-url"
                      value={browserInput}
                      onChange={(event) => {
                        updateBrowserPage({ ...activeBrowserPage, input: event.currentTarget.value });
                        setBrowserError(null);
                        setBrowserRecentEditing(true);
                        setBrowserRecentOpen(browserRecentEnabled);
                        setBrowserRecentIndex(-1);
                      }}
                      onFocus={() => {
                        setBrowserRecentEditing(false);
                        setBrowserRecentOpen(browserRecentEnabled);
                        setBrowserRecentIndex(-1);
                      }}
                      onKeyDown={handleBrowserAddressKeyDown}
                      placeholder="Search Google or enter a URL"
                      aria-label="Search Google or enter a URL"
                      role="combobox"
                      aria-autocomplete="list"
                      aria-expanded={browserRecentOpen && filteredBrowserRecentVisits.length > 0}
                      aria-controls="preview-browser-recents"
                      aria-activedescendant={browserRecentIndex >= 0 ? `preview-browser-recent-${browserRecentIndex}` : undefined}
                    />
                    {browserRecentOpen && filteredBrowserRecentVisits.length > 0 && (
                      <div
                        id="preview-browser-recents"
                        className="preview-browser-recents"
                        data-testid="preview-browser-recents"
                        data-native-preview-blocker="true"
                        role="listbox"
                        aria-label="Recently visited"
                        onMouseDown={(event) => event.preventDefault()}
                      >
                        <div className="preview-browser-recents-head">
                          <span>Recently visited</span>
                          <button
                            type="button"
                            onClick={() => {
                              clearBrowserRecentVisits();
                              setBrowserRecentOpen(false);
                            }}
                          >
                            Clear
                          </button>
                        </div>
                        {filteredBrowserRecentVisits.map((visit, index) => (
                          <button
                            id={`preview-browser-recent-${index}`}
                            className={index === browserRecentIndex ? "active" : ""}
                            key={visit.url}
                            type="button"
                            role="option"
                            aria-selected={index === browserRecentIndex}
                            onMouseEnter={() => setBrowserRecentIndex(index)}
                            onClick={() => openRecentBrowserVisit(visit)}
                          >
                            <span className="preview-browser-recent-icon" aria-hidden="true">
                              <img src={browserFaviconUrl(visit.url)} alt="" onError={(event) => { event.currentTarget.hidden = true; }} />
                              <Globe size={13} />
                            </span>
                            <span className="preview-browser-recent-copy">
                              <strong>{browserRecentLabel(visit)}</strong>
                              <small>{visit.url}</small>
                            </span>
                          </button>
                        ))}
                      </div>
                    )}
                  </form>
                  <button className="preview-browser-action preview-browser-open-external" title="Open in browser" aria-label="Open in system browser" disabled={!browserUrl} onClick={() => openBrowserUrlExternal()}>
                    <ExternalLink size={14} />
                  </button>
                </div>
              </>
            )}
            {selectedPreviewSource === "url" && !browserSetupSeen && (
              <div className="preview-browser-setup" data-testid="preview-browser-setup" role="status">
                <div>
                  <strong>Milim remembers browser sign-ins on this device</strong>
                  <span>Preview tools can interact with the active page under this chat&apos;s approval mode. External browser credentials are not imported.</span>
                </div>
                <div className="preview-browser-setup-actions">
                  <button className="btn-accent" type="button" onClick={() => setBrowserSetupSeen(true)}>Got it</button>
                  <button
                    className="btn-ghost"
                    type="button"
                    onClick={() => {
                      setBrowserStorageMode("private");
                      setBrowserSetupSeen(true);
                    }}
                  >
                    Use private browsing instead
                  </button>
                </div>
              </div>
            )}
            {showRuntimePanel && runtimeStatus && (
              <PreviewRuntimeStatus
                status={runtimeStatus}
                preflight={resolvedRuntimePreflight}
                busy={runtimeBusy}
                preflightBusy={runtimePreflightBusy}
                stale={runtimeIsStale}
                detailsExpanded={runtimeDetailsOpen}
                sectionRef={(element) => { runtimePanelRef.current = element; }}
                onFocusChange={setRuntimePanelFocused}
                onPreflight={onRuntimePreflight}
                onStart={onRuntimeStart ? () => runRuntimeAction(onRuntimeStart) : undefined}
                onStop={onRuntimeStop ? () => runRuntimeAction(onRuntimeStop) : undefined}
                onRestart={onRuntimeRestart ? () => runRuntimeAction(onRuntimeRestart) : undefined}
              />
            )}
            {browserError && <div className="preview-browser-error" role="alert">{browserError}</div>}
            {isUrlPreview ? (
              <div className="preview-browser-content">
                {browserSession.tabs.map((tab) => {
                  const googleTarget = selectedPreviewSource === "url" ? googleWorkspaceUrl(tab.url) : null;
                  const active = tab.id === browserSession.activeTabId;
                  return (
                  <div className={`preview-browser-tab-page${active ? " active" : ""}`} key={tab.id}>
                    {active ? tab.url ? googleTarget ? (
                      <GoogleWorkspacePreview
                        fileId={googleTarget.fileId}
                        fallbackUrl={tab.url}
                        active={active && !closing}
                        onMetadata={(title, faviconUrl) => syncNativeTitle(tab.id, title, faviconUrl)}
                        onSurfaceChange={active ? onSurfaceChange : undefined}
                        onOpenFile={openBrowserUrl}
                        onOpenFileInNewTab={openBrowserTab}
                      />
                    ) : (
                      <NativeArtifactBrowser
                        url={tab.url}
                        frameKey={active ? frameKey : 0}
                        title={tab.url}
                        mounted={!closing}
                        visible={active}
                        profileId={browserSession.profileId}
                        surfaceKind={selectedPreviewSource === "app" ? "runtime_browser" : "native_browser"}
                        storageMode={selectedPreviewSource === "url" ? browserStorageMode : "private"}
                        muted={tab.muted === true}
                        surfaceReady={selectedPreviewSource === "app" ? Boolean(runtimeStatus?.ready) : undefined}
                        surfaceError={selectedPreviewSource === "app" ? runtimeError : null}
                        zoomPercent={previewBrowserZoomPercent}
                        threadId={threadId}
                        onNativeClaimChange={(claim) => {
                          if (claim) nativeBrowserClaimsRef.current.set(tab.id, claim);
                          else nativeBrowserClaimsRef.current.delete(tab.id);
                        }}
                        onNavigation={(nextUrl, state) => syncNativeNavigation(tab.id, nextUrl, state)}
                        onNavigationError={(message) => handleNativeBrowserError(tab.id, message)}
                        onNewTab={openRequestedBrowserTab}
                        onShortcut={(shortcut) => {
                          if (shortcut.action === "new_tab") blankBrowser();
                          else if (shortcut.action === "close_tab") closeBrowserTab(tab.id);
                          else setPreviewBrowserZoom(
                            previewBrowserZoomSource,
                            nextPreviewBrowserZoom(
                              previewBrowserZoomPercent,
                              shortcut.action === "zoom_in" ? 1 : shortcut.action === "zoom_out" ? -1 : 0,
                            ),
                          );
                        }}
                        onTitle={(nextTitle) => syncNativeTitle(tab.id, nextTitle)}
                        onSurfaceChange={active ? onSurfaceChange : undefined}
                        controlActivity={active ? controlActivity : undefined}
                      />
                    ) : (
                      <div className="preview-browser-empty" data-testid="preview-browser-empty">
                        <Logo height={42} className="preview-browser-empty-logo" />
                        <strong>Open a preview URL</strong>
                        <span>Use the address bar for localhost or HTTPS pages.</span>
                      </div>
                    ) : null}
                  </div>
                  );
                })}
              </div>
            ) : (
              <div className="preview-frame-host">
                <iframe
                  ref={iframeRef}
                  key={`${artifact.id}:${frameKey}`}
                  className="preview-frame"
                  title={title}
                  sandbox="allow-forms allow-modals allow-popups allow-scripts"
                  referrerPolicy="no-referrer"
                  src="about:blank"
                  srcDoc={previewDocument.source}
                  onLoad={() => setIframeReadyKey(iframeSurfaceKey)}
                />
                {controlActivity && <PreviewControlOverlay key={controlActivity.id} activity={controlActivity} />}
              </div>
            )}
            <div className={`preview-log-drawer${logsOpen ? " open" : ""}`} data-testid="preview-log-drawer">
              {logsOpen && (
                <PaneResizeHandle
                  className={`preview-log-resize-handle${logResizing ? " dragging" : ""}`}
                  orientation="horizontal"
                  data-testid="preview-log-resize-handle"
                  aria-label="Resize logs"
                  aria-valuemin={LOG_DRAWER_MIN_HEIGHT}
                  aria-valuemax={maxLogDrawerHeight()}
                  aria-valuenow={logDrawerHeight}
                  onKeyDown={resizeLogDrawerWithKeyboard}
                  onPointerDown={startLogResize}
                  onPointerMove={moveLogResize}
                  onPointerUp={endLogResize}
                  onPointerCancel={endLogResize}
                />
              )}
              <div className="preview-log-head">
                <button
                  className="preview-log-toggle"
                  data-testid="preview-log-toggle"
                  aria-controls="preview-log-list"
                  aria-expanded={logsOpen}
                  onClick={() => setLogsOpen((open) => !open)}
                >
                  Logs <span>{visibleLogs.length}</span>
                </button>
                <div className="preview-log-actions">
                  {visibleLogs.length > 0 && (
                    <button className="preview-log-clear" data-testid="preview-log-clear" title="Clear logs" aria-label="Clear logs" onClick={clearLogs}>
                      <X size={13} />
                      <span>Clear</span>
                    </button>
                  )}
                  {canPrepareFix && (
                    <button className="preview-quick-fix" data-testid="preview-prepare-fix" onClick={prepareFix}>
                      <Bolt size={13} />
                      <span>Prepare fix</span>
                    </button>
                  )}
                </div>
              </div>
              {logsOpen && (
                <div
                  id="preview-log-list"
                  className="preview-log-list"
                  data-testid="preview-log-list"
                  role="log"
                  aria-live="polite"
                  aria-relevant="additions"
                  style={{ height: logDrawerHeight }}
                >
                  {visibleLogs.length ? visibleLogs.map((log) => <PreviewLogRow key={log.id} log={log} />) : <div className="preview-log-empty">No logs</div>}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
      <div id={PREVIEW_PANEL_IDS.code} className="preview-code-panel" role="tabpanel" aria-labelledby={PREVIEW_TAB_IDS.code} hidden={activeTab !== "code" || isUrlPreview}>
        <WorkspaceCodePanel
          workspaceFolder={workspaceFolder}
          files={codeFiles}
          selectedArtifactId={selectedCodeArtifactId}
          onSelectArtifact={setSelectedCodeArtifactId}
          onPreviewWorkspaceFile={onPreviewWorkspaceFile}
          runtimeBusy={runtimeBusy}
        />
      </div>
      {reviewCommentOpen && createPortal(
        <div
          className="git-modal-backdrop"
          data-native-preview-blocker="true"
          onMouseDown={(event) => event.target === event.currentTarget && closeReviewComment()}
        >
          <form
            className="git-modal git-pr-dialog"
            data-testid="review-comment-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="review-comment-title"
            onSubmit={submitReviewComment}
            onKeyDown={(event) => {
              if (event.key !== "Escape") return;
              event.preventDefault();
              event.stopPropagation();
              closeReviewComment();
            }}
          >
            <div className="git-modal-head">
              <strong id="review-comment-title">Review comment</strong>
              <button type="button" className="git-icon-btn" title="Close" aria-label="Close" onClick={closeReviewComment}>
                <X size={13} />
              </button>
            </div>
            <label>
              <span>Comment</span>
              <textarea
                autoFocus
                rows={4}
                value={reviewCommentDraft}
                onChange={(event) => setReviewCommentDraft(event.currentTarget.value)}
              />
            </label>
            <div className="git-diff-comment-actions">
              <button type="button" onClick={closeReviewComment}>Cancel</button>
              <button type="submit" disabled={!reviewCommentDraft.trim()}>Add comment</button>
            </div>
          </form>
        </div>,
        document.body,
      )}
    </aside>
  );
}

function PreviewControlOverlay({ activity }: { activity: PreviewControlActivity }) {
  const className = `preview-control-overlay ${activity.gesture} ${activity.status}`;
  return (
    <div className={className} data-testid="preview-control-overlay" aria-hidden="true">
      <span className="preview-control-scan" />
      {activity.gesture !== "inspect" && <span className="preview-control-cursor-glow" />}
      {activity.gesture !== "inspect" && (
        <svg className="preview-control-cursor" viewBox="0 0 24 24" fill="none" focusable="false">
          <path
            d="M20.5056 10.7754C21.1225 10.5355 21.431 10.4155 21.5176 10.2459C21.5926 10.099 21.5903 9.92446 21.5115 9.77954C21.4205 9.61226 21.109 9.50044 20.486 9.2768L4.59629 3.5728C4.0866 3.38983 3.83175 3.29835 3.66514 3.35605C3.52029 3.40621 3.40645 3.52004 3.35629 3.6649C3.29859 3.8315 3.39008 4.08635 3.57304 4.59605L9.277 20.4858C9.50064 21.1088 9.61246 21.4203 9.77973 21.5113C9.92465 21.5901 10.0991 21.5924 10.2461 21.5174C10.4157 21.4308 10.5356 21.1223 10.7756 20.5054L13.3724 13.8278C13.4194 13.707 13.4429 13.6466 13.4792 13.5957C13.5114 13.5506 13.5508 13.5112 13.5959 13.479C13.6468 13.4427 13.7072 13.4192 13.828 13.3722L20.5056 10.7754Z"
            fill="var(--preview-control-cursor-fill)"
            stroke="var(--preview-control-cursor-stroke)"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      )}
      {activity.gesture === "click" && <span className="preview-control-click-ring" />}
      {activity.gesture === "scroll" && <span className="preview-control-scroll-cue" />}
      {activity.gesture === "type" && <span className="preview-control-caret" />}
      <span className="preview-control-label">
        <span>{previewControlLabel(activity)}</span>
      </span>
    </div>
  );
}

function previewControlLabel(activity: PreviewControlActivity): string {
  if (activity.gesture === "inspect") return activity.detail || activity.label;
  return activity.label;
}

function PreviewRuntimeBadge({
  status,
  stale,
  expanded,
  collapsible,
  buttonRef,
  onToggle,
}: {
  status: PreviewAppStatus;
  stale: boolean;
  expanded: boolean;
  collapsible: boolean;
  buttonRef: (element: HTMLButtonElement | null) => void;
  onToggle: () => void;
}) {
  const statusLabel = previewRuntimeLabel(status, stale);
  const label = status.kind === "static" && status.ready && !stale && !runtimeErrorMessage(status)
    ? "Static preview"
    : statusLabel;
  const runtimeName = status.kind === "static" ? "Static preview" : "App runtime";
  const actionLabel = collapsible
    ? `${expanded ? "Hide" : "Show"} runtime details`
    : "Focus runtime details";
  return (
    <button
      ref={buttonRef}
      type="button"
      className={`preview-runtime-badge ${previewRuntimeTone(status, stale)}`}
      data-testid="preview-runtime-status"
      title={`${label}. ${actionLabel}`}
      aria-label={`${runtimeName} ${statusLabel}. ${actionLabel}`}
      aria-controls="preview-runtime-details"
      aria-expanded={expanded}
      onClick={onToggle}
    >
      <span className="preview-runtime-badge-dot" aria-hidden="true" />
      <span className="preview-runtime-badge-label" role="status" aria-live="polite">{label}</span>
    </button>
  );
}

function PreviewRuntimeStatus({
  status,
  preflight,
  busy,
  preflightBusy,
  stale,
  detailsExpanded,
  sectionRef,
  onFocusChange,
  onPreflight,
  onStart,
  onStop,
  onRestart,
}: {
  status: PreviewAppStatus;
  preflight: PreviewAppPreflight | null;
  busy: boolean;
  preflightBusy: boolean;
  stale: boolean;
  detailsExpanded: boolean;
  sectionRef: (element: HTMLElement | null) => void;
  onFocusChange: (focused: boolean) => void;
  onPreflight?: () => void;
  onStart?: () => void;
  onStop?: () => void;
  onRestart?: () => void;
}) {
  const active = previewRuntimeIsActive(status);
  const statusText = previewRuntimeLabel(status, stale);
  const error = runtimeErrorMessage(status);
  const message = runtimeStatusMessage(status, statusText, error);
  const compactStatic = status.kind === "static" && active && status.ready && !error;
  const runRequiresPreflight = Boolean(onPreflight && !preflight);
  return (
    <section
      ref={sectionRef}
      id="preview-runtime-details"
      className={`preview-managed-runtime ${previewRuntimeTone(status, stale)}${compactStatic ? " compact" : ""}`}
      data-testid="preview-managed-runtime"
      aria-label={status.kind === "static" ? "Static preview runtime" : "App preview runtime"}
      aria-busy={busy || preflightBusy}
      tabIndex={-1}
      onFocusCapture={() => onFocusChange(true)}
      onBlurCapture={(event) => {
        if (!(event.relatedTarget instanceof Node) || !event.currentTarget.contains(event.relatedTarget)) onFocusChange(false);
      }}
    >
      <div className="preview-managed-runtime-head">
        <div className="preview-managed-runtime-copy">
          <span className="preview-managed-runtime-dot" aria-hidden="true" />
          <div>
            <strong>{status.kind === "static" ? "Static preview" : "App runtime"}</strong>
            {!compactStatic && <span role="status" aria-live="polite">{statusText}</span>}
          </div>
        </div>
        <div className="preview-managed-runtime-actions">
          {onPreflight && (
            <button className="preview-runtime-button" data-testid="preview-runtime-preflight" disabled={busy || preflightBusy || active} onClick={onPreflight}>
              <Terminal size={13} />
              <span>{preflight ? "Refresh commands" : "Review commands"}</span>
            </button>
          )}
          {!active ? (
            <button
              className="preview-runtime-button primary"
              data-testid="preview-runtime-start"
              aria-label={runRequiresPreflight ? "Run preview unavailable until review is complete" : "Run app preview"}
              disabled={busy || preflightBusy || runRequiresPreflight || !onStart || stale}
              onClick={onStart}
            >
              <Bolt size={13} />
              <span>Run</span>
            </button>
          ) : (
            <>
              <button className="preview-runtime-button" data-testid="preview-runtime-stop" aria-label="Stop app preview" disabled={busy || !onStop} onClick={onStop}>
                <Square size={12} />
                <span>Stop</span>
              </button>
              {onRestart && (status.ready || stale || Boolean(error)) && (
                <button className="preview-runtime-button" data-testid="preview-runtime-restart" aria-label="Restart app preview" disabled={busy || !onRestart || stale} onClick={onRestart}>
                  <Refresh size={13} />
                  <span>Restart</span>
                </button>
              )}
            </>
          )}
        </div>
      </div>
      {preflight && (!active || detailsExpanded || stale || Boolean(error)) && (
        <div className="preview-runtime-preflight" data-testid="preview-runtime-preflight-details">
          <dl>
            <div><dt>Scope</dt><dd>{preflight.managed ? "Managed copy" : "Selected folder"}</dd></div>
            <div><dt>Folder</dt><dd title={preflight.cwd}>{preflight.cwd}</dd></div>
            <div><dt>Configuration</dt><dd>{preflight.configuration === "manifest" ? ".milim/preview.json" : "package.json"}</dd></div>
            <div><dt>{preflight.configuration === "manifest" ? "Launcher" : "Package manager"}</dt><dd>{preflight.package_manager}</dd></div>
            <div><dt>Install</dt><dd>{preflight.install_required ? "Required" : "Not required"}</dd></div>
            <div><dt>Install command</dt><dd><code>{preflight.install_command || "None"}</code></dd></div>
            <div><dt>Dev command</dt><dd><code>{preflight.dev_command || "Unavailable"}</code></dd></div>
            <div><dt>Readiness</dt><dd title={preflight.healthcheck_url}><code>{preflight.healthcheck_url}</code></dd></div>
            <div><dt>Source</dt><dd title={preflight.source_fingerprint}><code>{shortFingerprint(preflight.source_fingerprint)}</code></dd></div>
          </dl>
          {!preflight.managed && preflight.install_required && (
            <p className="preview-runtime-warning" role="note">Installing dependencies may modify the selected folder.</p>
          )}
        </div>
      )}
      {error && <p className="preview-runtime-message error" role="alert">{error}</p>}
      {!compactStatic && message && <p className="preview-runtime-message" role="status" aria-live="polite">{message}</p>}
    </section>
  );
}

function NativeArtifactBrowser({
  url,
  frameKey,
  title,
  mounted,
  visible,
  profileId,
  surfaceKind,
  storageMode,
  muted,
  surfaceReady,
  surfaceError,
  zoomPercent,
  threadId,
  onNativeClaimChange,
  onNavigation,
  onNavigationError,
  onNewTab,
  onShortcut,
  onTitle,
  onSurfaceChange,
  controlActivity,
}: {
  url: string;
  frameKey: number;
  title: string;
  mounted: boolean;
  visible: boolean;
  profileId: string;
  surfaceKind: PreviewSurfaceKind;
  storageMode: PreviewBrowserStorageMode;
  muted: boolean;
  surfaceReady?: boolean;
  surfaceError?: string | null;
  zoomPercent: number;
  threadId: string;
  onNativeClaimChange?: (claim: NativePreviewClaim | null) => void;
  onNavigation?: (url: string, state: PreviewWebviewLoadState) => void;
  onNavigationError?: (message: string) => void;
  onNewTab?: (requestId: number, url: string) => void;
  onShortcut?: (shortcut: PreviewWebviewShortcut) => void;
  onTitle?: (title: string) => void;
  onSurfaceChange?: (surface: PreviewSurfaceTarget | null) => void;
  controlActivity?: PreviewControlActivity | null;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const webviewRef = useRef<NativeWebviewHandle | null>(null);
  const claimRef = useRef<NativePreviewClaim | null>(null);
  const visibilitySyncRef = useRef(Promise.resolve());
  const overlayWindowRef = useRef<NativeOverlayWindowHandle | null>(null);
  const overlayOwnerRef = useRef(0);
  const overlayCleanupRef = useRef<(() => void) | null>(null);
  const overlayCloseTimerRef = useRef<number | null>(null);
  const overlayActiveRef = useRef(false);
  const navigationCallbackRef = useRef(onNavigation);
  const navigationErrorCallbackRef = useRef(onNavigationError);
  const newTabCallbackRef = useRef(onNewTab);
  const shortcutCallbackRef = useRef(onShortcut);
  const titleCallbackRef = useRef(onTitle);
  const claimCallbackRef = useRef(onNativeClaimChange);
  const visibleRef = useRef(visible);
  const zoomPercentRef = useRef(zoomPercent);
  const mutedRef = useRef(muted);
  const currentNativeUrlRef = useRef(url);
  const previousFrameKeyRef = useRef(frameKey);
  const [nativeError, setNativeError] = useState<string | null>(null);
  const [nativeNavigation, setNativeNavigation] = useState<{ label: string | null; url: string; state: PreviewWebviewLoadState }>({
    label: null,
    url,
    state: "loading",
  });
  const nativeLabel = previewNativeWebviewLabel(surfaceKind, storageMode);

  navigationCallbackRef.current = onNavigation;
  navigationErrorCallbackRef.current = onNavigationError;
  newTabCallbackRef.current = onNewTab;
  shortcutCallbackRef.current = onShortcut;
  titleCallbackRef.current = onTitle;
  claimCallbackRef.current = onNativeClaimChange;
  visibleRef.current = visible;
  zoomPercentRef.current = zoomPercent;
  mutedRef.current = muted;

  function clearOverlayCloseTimer() {
    if (overlayCloseTimerRef.current === null) return;
    window.clearTimeout(overlayCloseTimerRef.current);
    overlayCloseTimerRef.current = null;
  }

  function ownsOverlay(owner = overlayOwnerRef.current) {
    return owner !== 0 && previewControlOverlayOwner === owner;
  }

  async function hideOverlayWebview(owner = overlayOwnerRef.current) {
    if (!ownsOverlay(owner)) return;
    clearOverlayCloseTimer();
    overlayActiveRef.current = false;
    await setNativeWebviewHidden(
      overlayWindowRef.current,
      true,
      () => ownsOverlay(owner),
    ).catch(() => undefined);
  }

  async function releaseOverlayWebview() {
    clearOverlayCloseTimer();
    overlayActiveRef.current = false;
    overlayCleanupRef.current?.();
    overlayCleanupRef.current = null;
    const overlay = overlayWindowRef.current;
    overlayWindowRef.current = null;
    const owner = overlayOwnerRef.current;
    overlayOwnerRef.current = 0;
    await setNativeWebviewHidden(overlay, true, () => ownsOverlay(owner)).catch(() => undefined);
    if (previewControlOverlayOwner === owner) previewControlOverlayOwner += 1;
  }

  function queueNativeVisibility() {
    visibilitySyncRef.current = visibilitySyncRef.current.then(async () => {
      const claim = claimRef.current;
      if (!claim) return;
      const blocked = !visibleRef.current || nativePreviewBlockedByAppUi();
      await setPreviewWebviewVisibility(
        claim.label,
        claim.claimToken,
        !blocked,
        mutedRef.current,
      );
      const owner = overlayOwnerRef.current;
      await setNativeWebviewHidden(
        overlayWindowRef.current,
        blocked || !overlayActiveRef.current,
        () => ownsOverlay(owner),
      );
    }).catch((error) => {
      console.error("Could not update native preview visibility.", error);
    });
    return visibilitySyncRef.current;
  }

  useEffect(() => {
    if (!mounted) return;
    if (!IS_TAURI) return;

    let cancelled = false;
    let resizeObserver: ResizeObserver | null = null;
    let appUiObserver: MutationObserver | null = null;
    let unlistenNavigation: (() => void) | null = null;
    let unlistenNewTab: (() => void) | null = null;
    let unlistenShortcut: (() => void) | null = null;
    let unlistenTitle: (() => void) | null = null;
    let removeLayoutListeners: (() => void) | null = null;
    let ownedClaim: NativePreviewClaim | null = null;
    let ownedWebview: NativeWebviewHandle | null = null;
    const pendingEvents: {
      navigation: PreviewWebviewNavigation | null;
      title: PreviewWebviewTitle | null;
    } = { navigation: null, title: null };
    let raf = 0;

    async function releaseWebview() {
      const claim = ownedClaim;
      const webview = ownedWebview;
      ownedClaim = null;
      ownedWebview = null;
      await releaseOverlayWebview();
      if (claim && claimRef.current?.claimToken === claim.claimToken) {
        claimRef.current = null;
        if (webviewRef.current === webview) webviewRef.current = null;
        claimCallbackRef.current?.(null);
      }
      if (claim) {
        await setPreviewWebviewVisibility(
          claim.label,
          claim.claimToken,
          false,
          true,
        ).catch(() => undefined);
      }
    }

    void (async () => {
      const host = hostRef.current;
      if (!host) return;
      const hostElement = host;
      setNativeError(null);

      const { Webview } = await import("@tauri-apps/api/webview");
      if (cancelled) return;

      function bounds() {
        const rect = hostElement.getBoundingClientRect();
        return {
          x: Math.max(0, Math.round(rect.left)),
          y: Math.max(0, Math.round(rect.top)),
          width: Math.max(1, Math.round(rect.width)),
          height: Math.max(1, Math.round(rect.height)),
        };
      }

      const boundsKey = (rect: ReturnType<typeof bounds>) =>
        `${rect.x}:${rect.y}:${rect.width}:${rect.height}`;
      let lastBoundsKey: string | null = null;
      let pendingBounds: ReturnType<typeof bounds> | null = null;
      let boundsSync: Promise<void> | null = null;

      function syncBounds(webview = webviewRef.current): Promise<void> {
        if (!webview || !hostElement.isConnected || cancelled) return Promise.resolve();
        pendingBounds = bounds();
        if (boundsSync) return boundsSync;
        boundsSync = (async () => {
          while (pendingBounds && !cancelled) {
            const rect = pendingBounds;
            pendingBounds = null;
            const key = boundsKey(rect);
            if (key === lastBoundsKey) continue;
            try {
              const claim = claimRef.current;
              if (!claim) return;
              await setPreviewWebviewBounds(claim.label, claim.claimToken, rect);
              lastBoundsKey = key;
            } catch {
              // A later observation can retry the latest bounds.
            }
          }
        })().finally(() => {
          boundsSync = null;
          if (pendingBounds && !cancelled) void syncBounds(webview);
        });
        return boundsSync;
      }

      async function syncAppUiVisibility() {
        await queueNativeVisibility();
        if (!cancelled && visibleRef.current && !nativePreviewBlockedByAppUi()) {
          await syncBounds(webviewRef.current);
        }
      }

      const rect = bounds();
      lastBoundsKey = boundsKey(rect);
      const label = nativeLabel;
      currentNativeUrlRef.current = url;
      setNativeNavigation({ label, url, state: "loading" });
      const applyNavigation = (navigation: PreviewWebviewNavigation) => {
        const claim = ownedClaim;
        if (cancelled || navigation.label !== label) return;
        if (!claim) {
          pendingEvents.navigation = navigation;
          return;
        }
        if (navigation.claimToken !== claim.claimToken) return;
        if (navigation.state === "error") {
          const message = navigation.message || "This preview navigation was blocked.";
          setNativeError(message);
          setNativeNavigation({ label, url: navigation.url, state: "error" });
          navigationErrorCallbackRef.current?.(message);
          return;
        }
        currentNativeUrlRef.current = navigation.url;
        setNativeError(null);
        setNativeNavigation(navigation);
        navigationCallbackRef.current?.(navigation.url, navigation.state);
      };
      const applyTitle = (title: PreviewWebviewTitle) => {
        const claim = ownedClaim;
        if (cancelled || title.label !== label) return;
        if (!claim) {
          pendingEvents.title = title;
          return;
        }
        if (title.claimToken !== claim.claimToken) return;
        titleCallbackRef.current?.(title.title);
      };
      unlistenNavigation = await listenForPreviewWebviewNavigation(applyNavigation);
      unlistenNewTab = await listenForPreviewWebviewNewTab((request) => {
        const claim = ownedClaim;
        if (cancelled || !claim || request.openerLabel !== claim.label || request.claimToken !== claim.claimToken) return;
        newTabCallbackRef.current?.(request.requestId, request.url);
      });
      unlistenShortcut = await listenForPreviewWebviewShortcut((shortcut) => {
        const claim = ownedClaim;
        if (cancelled || !claim || shortcut.label !== claim.label || shortcut.claimToken !== claim.claimToken) return;
        shortcutCallbackRef.current?.(shortcut);
      });
      unlistenTitle = await listenForPreviewWebviewTitle(applyTitle);
      if (cancelled) {
        unlistenNavigation();
        unlistenNewTab();
        unlistenShortcut();
        unlistenTitle();
        return;
      }
      const created = await createPreviewWebview(label, url, rect, storageMode, profileId);
      if (!created) throw new Error("Could not create the native preview webview.");
      const claim = { label, claimToken: created.claimToken };
      ownedClaim = claim;
      claimRef.current = claim;
      claimCallbackRef.current?.(claim);
      if (created.reused) {
        currentNativeUrlRef.current = created.url;
        setNativeNavigation({ label, url: created.url, state: created.navigated ? "loading" : "ready" });
        if (!created.navigated) navigationCallbackRef.current?.(created.url, "ready");
      }
      if (pendingEvents.navigation?.claimToken === claim.claimToken) applyNavigation(pendingEvents.navigation);
      if (pendingEvents.title?.claimToken === claim.claimToken) applyTitle(pendingEvents.title);
      pendingEvents.navigation = null;
      pendingEvents.title = null;
      const webview = await Webview.getByLabel(label) as NativeWebviewHandle | null;
      if (!webview) throw new Error("Could not access the created preview webview.");
      ownedWebview = webview;
      webviewRef.current = webview;
      if (cancelled) {
        await releaseWebview();
        return;
      }
      await setPreviewWebviewZoom(label, claim.claimToken, zoomPercentRef.current / 100);
      await setPreviewWebviewMuted(label, claim.claimToken, mutedRef.current);
      await syncBounds(webview);
      appUiObserver = new MutationObserver(() => void syncAppUiVisibility());
      appUiObserver.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["data-native-preview-blocker", "open"] });
      await syncAppUiVisibility();
      resizeObserver = new ResizeObserver(() => void syncBounds(webview));
      resizeObserver.observe(hostElement);
      const onWindowLayout = () => void syncBounds(webview);
      window.addEventListener("resize", onWindowLayout);
      window.addEventListener("scroll", onWindowLayout, true);
      removeLayoutListeners = () => {
        window.removeEventListener("resize", onWindowLayout);
        window.removeEventListener("scroll", onWindowLayout, true);
      };
      let frame = 0;
      const syncAnimationFrame = () => {
        void syncBounds(webview);
        frame += 1;
        if (frame < 12 && !cancelled) raf = window.requestAnimationFrame(syncAnimationFrame);
      };
      raf = window.requestAnimationFrame(syncAnimationFrame);
    })()
      .catch((error) => {
        if (cancelled) return;
        void releaseWebview();
        const message = error instanceof Error ? error.message : String(error);
        setNativeError(message);
        navigationErrorCallbackRef.current?.(message);
      });

    return () => {
      cancelled = true;
      if (raf) window.cancelAnimationFrame(raf);
      resizeObserver?.disconnect();
      appUiObserver?.disconnect();
      removeLayoutListeners?.();
      unlistenNavigation?.();
      unlistenNewTab?.();
      unlistenShortcut?.();
      unlistenTitle?.();
      void releaseWebview();
    };
  }, [mounted, nativeLabel, profileId, storageMode, surfaceKind]);

  useEffect(() => {
    if (!IS_TAURI) return;
    const claim = claimRef.current;
    if (!claim) return;
    void setPreviewWebviewZoom(claim.label, claim.claimToken, zoomPercent / 100).catch((error) => {
      console.error("Could not update native preview zoom.", error);
    });
  }, [zoomPercent]);

  useEffect(() => {
    if (!IS_TAURI) return;
    const claim = claimRef.current;
    if (!claim) return;
    void setPreviewWebviewMuted(claim.label, claim.claimToken, muted).catch((error) => {
      console.error("Could not update native preview audio mute.", error);
    });
  }, [muted, nativeNavigation.label, nativeNavigation.state]);

  useEffect(() => {
    if (!IS_TAURI) return;
    void queueNativeVisibility();
  }, [visible]);

  useEffect(() => {
    const claim = claimRef.current;
    if (!IS_TAURI || !claim || currentNativeUrlRef.current === url) return;
    setNativeNavigation((current) => ({ ...current, url, state: "loading" }));
    void navigatePreviewWebview(claim.label, claim.claimToken, url).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      setNativeError(message);
      navigationErrorCallbackRef.current?.(message);
    });
  }, [nativeNavigation.label, url]);

  useEffect(() => {
    const claim = claimRef.current;
    if (!IS_TAURI || !claim) {
      previousFrameKeyRef.current = frameKey;
      return;
    }
    if (previousFrameKeyRef.current === frameKey) return;
    previousFrameKeyRef.current = frameKey;
    setNativeNavigation((current) => ({ ...current, state: "loading" }));
    void reloadPreviewWebview(claim.label, claim.claimToken).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      setNativeError(message);
      navigationErrorCallbackRef.current?.(message);
    });
  }, [frameKey, nativeNavigation.label]);

  useEffect(() => {
    if (!visible) return;
    if (!IS_TAURI) {
      void publishPreviewSurface({
        kind: surfaceKind,
        title,
        url: nativeNavigation.url,
        message: "Native preview inspection is available in the desktop app",
        native: false,
        status: "not_inspectable",
        capabilities: [],
      }, onSurfaceChange, threadId);
      return () => { void publishPreviewSurface(null, onSurfaceChange, threadId); };
    }
    const runtimeWaiting = surfaceReady === false;
    const error = nativeError || surfaceError;
    const ready = nativeNavigation.state === "ready" && !runtimeWaiting && !error;
    const surface: PreviewSurfaceTarget = {
      label: nativeNavigation.label,
      kind: surfaceKind,
      title,
      url: nativeNavigation.url,
      message: error || (runtimeWaiting ? "Waiting for the app runtime to become healthy" : ready ? undefined : "Loading preview page"),
      native: true,
      status: error ? "error" : ready ? "ready" : "loading",
      capabilities: ready ? DOM_PREVIEW_CAPABILITIES : [],
    };
    void publishPreviewSurface(surface, onSurfaceChange, threadId);
    return () => { void publishPreviewSurface(null, onSurfaceChange, threadId); };
  }, [visible, nativeError, nativeNavigation, onSurfaceChange, surfaceError, surfaceKind, surfaceReady, threadId, title]);

  useEffect(() => {
    if (!visible || !controlActivity || !IS_TAURI) return;
    let cancelled = false;
    const owner = ++previewControlOverlayOwner;
    overlayOwnerRef.current = owner;
    const channel = PREVIEW_CONTROL_OVERLAY_CHANNEL;
    const payload = previewControlOverlayPayload(controlActivity, hostRef.current ?? undefined);

    overlayActiveRef.current = true;
    publishPreviewControlOverlayActivity(channel, payload);
    clearOverlayCloseTimer();
    overlayCloseTimerRef.current = window.setTimeout(() => void hideOverlayWebview(owner), PREVIEW_CONTROL_OVERLAY_CLOSE_MS);

    void (async () => {
      const host = hostRef.current;
      if (!host) return;
      const hostElement = host;
      const [{ WebviewWindow }, { getCurrentWindow }, { LogicalPosition, LogicalSize }] = await Promise.all([
        import("@tauri-apps/api/webviewWindow"),
        import("@tauri-apps/api/window"),
        import("@tauri-apps/api/dpi"),
      ]);
      if (cancelled || !ownsOverlay(owner)) return;
      const mainWindow = getCurrentWindow();

      async function syncOverlayBounds() {
        if (!ownsOverlay(owner) || !overlayWindowRef.current || !hostElement.isConnected) return;
        const nextRect = await nativeBrowserWindowBounds(hostElement, mainWindow);
        await Promise.all([
          overlayWindowRef.current.setPosition(new LogicalPosition(nextRect.x, nextRect.y)),
          overlayWindowRef.current.setSize(new LogicalSize(nextRect.width, nextRect.height)),
        ]).catch(() => undefined);
      }

      if (!overlayWindowRef.current) {
        const rect = await nativeBrowserWindowBounds(hostElement, mainWindow);
        const existing = await WebviewWindow.getByLabel(PREVIEW_CONTROL_OVERLAY_LABEL) as NativeOverlayWindowHandle | null;
        const overlay = existing ?? new WebviewWindow(PREVIEW_CONTROL_OVERLAY_LABEL, {
          url: previewControlOverlayUrl(channel),
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height,
          transparent: true,
          decorations: false,
          backgroundColor: [0, 0, 0, 0],
          visible: false,
          focus: false,
          focusable: false,
          resizable: false,
          shadow: false,
          skipTaskbar: true,
          parent: mainWindow,
          dragDropEnabled: false,
        }) as NativeOverlayWindowHandle;
        overlayWindowRef.current = overlay;
        if (!existing) await waitForNativeCreated(overlay);
        if (cancelled || !ownsOverlay(owner)) {
          await setNativeWebviewHidden(overlayWindowRef.current, true, () => ownsOverlay(owner)).catch(() => undefined);
          return;
        }
        await overlay.setIgnoreCursorEvents(true);
        await syncOverlayBounds();
        const resizeObserver = new ResizeObserver(() => void syncOverlayBounds());
        resizeObserver.observe(hostElement);
        const onWindowLayout = () => void syncOverlayBounds();
        window.addEventListener("resize", onWindowLayout);
        window.addEventListener("scroll", onWindowLayout, true);
        const windowListeners = await Promise.all([
          mainWindow.onMoved(onWindowLayout),
          mainWindow.onResized(onWindowLayout),
          mainWindow.onScaleChanged(onWindowLayout),
        ]);
        const publishSoon = [
          window.setTimeout(() => publishPreviewControlOverlayActivity(channel, payload), 80),
          window.setTimeout(() => publishPreviewControlOverlayActivity(channel, payload), 220),
        ];
        overlayCleanupRef.current = () => {
          resizeObserver.disconnect();
          window.removeEventListener("resize", onWindowLayout);
          window.removeEventListener("scroll", onWindowLayout, true);
          for (const unlisten of windowListeners) unlisten();
          for (const timer of publishSoon) window.clearTimeout(timer);
        };
      } else {
        await syncOverlayBounds();
        publishPreviewControlOverlayActivity(channel, payload);
      }
      if (ownsOverlay(owner) && visibleRef.current && overlayActiveRef.current && !nativePreviewBlockedByAppUi()) {
        await overlayWindowRef.current?.show().catch(() => undefined);
      }
      clearOverlayCloseTimer();
      overlayCloseTimerRef.current = window.setTimeout(() => void hideOverlayWebview(owner), PREVIEW_CONTROL_OVERLAY_CLOSE_MS);
    })()
      .catch(() => {
        if (!cancelled) void hideOverlayWebview(owner);
      });

    return () => {
      cancelled = true;
    };
  }, [visible, controlActivity?.id]);

  return (
    <div ref={hostRef} className="preview-native-browser" data-testid="preview-native-browser" aria-label={title}>
      {!IS_TAURI && (
        <iframe
          key={`${url}:${frameKey}`}
          className="preview-frame"
          title={title}
          sandbox="allow-forms allow-modals allow-popups allow-same-origin allow-scripts"
          referrerPolicy="no-referrer"
          src={url}
          onLoad={() => {
            currentNativeUrlRef.current = url;
            setNativeNavigation({ label: null, url, state: "ready" });
            navigationCallbackRef.current?.(url, "ready");
          }}
        />
      )}
      {nativeError && !onNavigationError ? (
        <div className="preview-native-browser-error" role="alert">{nativeError}</div>
      ) : nativeNavigation.state !== "ready" && nativeNavigation.state !== "error" ? (
        <div className="preview-native-browser-status" role="status" aria-live="polite">Loading preview...</div>
      ) : null}
    </div>
  );
}

async function setNativeWebviewHidden(
  webview: NativeVisibleHandle | null,
  hidden: boolean,
  isCurrent: () => boolean = () => true,
) {
  if (!webview || !isCurrent()) return;
  const updateVisibility = () => hidden ? webview.hide() : webview.show();
  try {
    await updateVisibility();
  } catch {
    await new Promise((resolve) => window.setTimeout(resolve, 50));
    if (!isCurrent()) return;
    await updateVisibility();
  }
}

async function waitForNativeCreated(webview: NativeWebviewHandle) {
  await new Promise<void>((resolve, reject) => {
    let settled = false;
    let timer = 0;
    const finish = (error?: string) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timer);
      if (error) reject(new Error(error));
      else resolve();
    };
    timer = window.setTimeout(() => finish(), 600);
    void webview.once("tauri://created", () => finish()).catch(() => undefined);
    void webview.once<string>("tauri://error", (event) => finish(event.payload || "Could not create preview overlay.")).catch(() => undefined);
  });
}

function nativeBrowserBounds(host: HTMLElement) {
  const rect = host.getBoundingClientRect();
  return {
    x: Math.max(0, Math.round(rect.left)),
    y: Math.max(0, Math.round(rect.top)),
    width: Math.max(1, Math.round(rect.width)),
    height: Math.max(1, Math.round(rect.height)),
  };
}

async function nativeBrowserWindowBounds(
  host: HTMLElement,
  mainWindow: {
    outerPosition: () => Promise<{ x: number; y: number; toLogical?: (scaleFactor: number) => { x: number; y: number } }>;
    scaleFactor: () => Promise<number>;
  },
) {
  const rect = nativeBrowserBounds(host);
  const [position, scaleFactor] = await Promise.all([mainWindow.outerPosition(), mainWindow.scaleFactor()]);
  const origin = position.toLogical ? position.toLogical(scaleFactor) : { x: position.x / scaleFactor, y: position.y / scaleFactor };
  return {
    x: Math.max(0, Math.round(origin.x + rect.x)),
    y: Math.max(0, Math.round(origin.y + rect.y)),
    width: rect.width,
    height: rect.height,
  };
}

function previewControlOverlayUrl(channel: string): string {
  const params = new URLSearchParams({ channel });
  return `/preview-control-overlay.html?${params.toString()}`;
}

function previewControlOverlayPayload(activity: PreviewControlActivity, host?: HTMLElement): PreviewControlOverlayPayload {
  const payload: PreviewControlOverlayPayload = {
    id: activity.id,
    gesture: activity.gesture,
    status: activity.status,
    label: previewControlLabel(activity),
    dark: typeof document !== "undefined" && document.documentElement.getAttribute("data-dark") === "true",
  };
  const point = previewControlOverlayPoint(activity, host);
  if (point) payload.point = point;
  const styles = typeof document !== "undefined" ? getComputedStyle(document.documentElement) : null;
  const accent = styles?.getPropertyValue("--accent").trim();
  const accentLight = styles?.getPropertyValue("--accent-light").trim();
  const accentGlow = styles?.getPropertyValue("--accent-glow").trim();
  const focusBorder = styles?.getPropertyValue("--focus-border").trim();
  if (accent) payload.accent = accent;
  if (accentLight) payload.accentLight = accentLight;
  if (accentGlow) payload.accentGlow = accentGlow;
  if (focusBorder) payload.focusBorder = focusBorder;
  return payload;
}

function previewControlOverlayPoint(activity: PreviewControlActivity, host?: HTMLElement): PreviewControlOverlayPoint | undefined {
  if (!activity.point) return undefined;
  const { x, y, unit } = activity.point;
  if (unit === "ratio") return { x: clampPercent(x * 100), y: clampPercent(y * 100) };
  if (unit === "percent") return { x: clampPercent(x), y: clampPercent(y) };
  const rect = host?.getBoundingClientRect();
  if (!rect?.width || !rect.height) return undefined;
  return { x: clampPercent((x / rect.width) * 100), y: clampPercent((y / rect.height) * 100) };
}

function clampPercent(value: number): number {
  return Math.min(100, Math.max(0, Math.round(value * 100) / 100));
}

function publishPreviewControlOverlayActivity(channel: string, payload: PreviewControlOverlayPayload) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(`${PREVIEW_CONTROL_OVERLAY_STORAGE_PREFIX}${channel}`, JSON.stringify(payload));
  } catch {
    // Storage is best-effort; BroadcastChannel is enough when the overlay is already loaded.
  }
  if (!("BroadcastChannel" in window)) return;
  try {
    const broadcast = new BroadcastChannel(channel);
    broadcast.postMessage(payload);
    broadcast.close();
  } catch {
    // The overlay will still read the latest payload from localStorage on load.
  }
}

function PreviewLogRow({ log }: { log: PreviewLogEntry }) {
  const location = !log.label && log.source ? `${basename(log.source)}${log.line ? `:${log.line}` : ""}` : "";
  return (
    <div className={`preview-log-row ${log.level}`} data-testid="preview-log-row">
      <span className="preview-log-level">{log.label ?? log.level}</span>
      <span className="preview-log-message">{log.message}</span>
      {location && <span className="preview-log-location">{location}</span>}
    </div>
  );
}

function normalizePreviewLog(raw: unknown, id: number): PreviewLogEntry | null {
  if (!raw || typeof raw !== "object") return null;
  const data = raw as Record<string, unknown>;
  if (data.type !== PREVIEW_LOG_EVENT) return null;
  const level = typeof data.level === "string" && isPreviewLogLevel(data.level) ? data.level : "log";
  const message = typeof data.message === "string" ? data.message : String(data.message ?? "");
  if (!message.trim()) return null;
  return {
    id,
    level,
    message,
    timestamp: typeof data.timestamp === "number" ? data.timestamp : Date.now(),
    stack: typeof data.stack === "string" ? data.stack : undefined,
    source: typeof data.source === "string" ? data.source : undefined,
    line: typeof data.line === "number" ? data.line : undefined,
    column: typeof data.column === "number" ? data.column : undefined,
  };
}

function isPreviewLogLevel(value: string): value is PreviewLogLevel {
  return value === "log" || value === "info" || value === "warn" || value === "error";
}

function initialBrowserSession(url: string | null): PreviewBrowserSession {
  const tab = initialBrowserPage(url, "main");
  return {
    profileId: `session-${Math.random().toString(36).slice(2)}`,
    tabs: [tab],
    activeTabId: tab.id,
  };
}

function initialBrowserPage(url: string | null, id = `tab-${Math.random().toString(36).slice(2)}`): PreviewBrowserPage {
  return {
    id,
    url,
    input: url ?? "",
    history: url ? [url] : [],
    historyIndex: url ? 0 : -1,
    faviconUrl: browserFaviconUrl(url),
  };
}

function browserTabLabel(tab: PreviewBrowserPage): string {
  if (tab.title) return tab.title;
  if (!tab.url) return "New tab";
  try {
    return new URL(tab.url).hostname || tab.url;
  } catch {
    return tab.url;
  }
}

function browserRecentLabel(visit: BrowserRecentVisit): string {
  if (visit.title) return visit.title;
  try {
    return new URL(visit.url).hostname || visit.url;
  } catch {
    return visit.url;
  }
}

function browserFaviconUrl(value: string | null): string | undefined {
  if (!value) return undefined;
  try {
    return new URL("/favicon.ico", value).toString();
  } catch {
    return undefined;
  }
}

export function nextPreviewTab(current: PreviewTab, key: string, tabs: readonly PreviewTab[]): PreviewTab | null {
  if (!tabs.length) return null;
  if (key === "Home") return tabs[0];
  if (key === "End") return tabs[tabs.length - 1];
  const direction = key === "ArrowRight" || key === "ArrowDown" ? 1 : key === "ArrowLeft" || key === "ArrowUp" ? -1 : 0;
  if (!direction) return null;
  const index = Math.max(0, tabs.indexOf(current));
  return tabs[(index + direction + tabs.length) % tabs.length];
}

export function previewDocumentReadyForSurface(
  isUrlPreview: boolean,
  builtKey: string,
  requestedKey: string,
): boolean {
  return isUrlPreview || builtKey === requestedKey;
}

function previewSourceLabel(source: PreviewSource): string {
  if (source === "app") return "App";
  if (source === "url") return "URL";
  return "Artifact";
}

function previewRuntimeIsActive(status: PreviewAppStatus): boolean {
  return status.active ?? (Boolean(status.pid) || status.status === "installing" || status.status === "starting" || status.status === "running");
}

export function previewRuntimeIsHealthy(status: PreviewAppStatus, stale: boolean): boolean {
  return previewRuntimeIsActive(status) && Boolean(status.ready) && !stale && !runtimeErrorMessage(status);
}

function previewRuntimeLabel(status: PreviewAppStatus, stale: boolean): string {
  const base = status.ready
    ? "Ready"
    : status.status === "installing"
      ? "Installing"
      : status.status === "starting"
        ? "Starting"
        : status.status === "error"
          ? "Unhealthy"
          : previewRuntimeIsActive(status)
            ? "Active · not ready"
            : status.status === "staged"
              ? "Ready to run"
              : "Stopped";
  return stale ? `${base} · disconnected` : base;
}

function previewRuntimeTone(status: PreviewAppStatus, stale: boolean): string {
  if (stale) return "stale";
  if (status.error || status.status === "error") return "error";
  if (status.ready) return "running";
  if (previewRuntimeIsActive(status)) return "starting";
  return "stopped";
}

function runtimeErrorMessage(status?: PreviewAppStatus | null): string | null {
  if (!status) return null;
  if (!status.error && status.status !== "error") return null;
  const messages = [status.error?.message, status.message].filter((message, index, all): message is string => Boolean(message) && all.indexOf(message) === index);
  return messages.join("\n") || "The app preview runtime failed.";
}

function runtimeStatusMessage(status: PreviewAppStatus, statusText: string, error: string | null): string | null {
  const message = error ? "" : status.message?.trim() ?? "";
  if (!message) return null;
  const normalize = (value: string) => value.replace(/[.!]+$/, "").trim().toLowerCase();
  const normalized = normalize(message);
  if (normalized === normalize(statusText) || (status.ready && normalized === "running")) return null;
  return message;
}

function shortFingerprint(fingerprint: string): string {
  return fingerprint.length > 16 ? `${fingerprint.slice(0, 12)}...` : fingerprint;
}

export function buildFixPrompt(
  artifact: ChatArtifact,
  files: string[],
  revisionNumber: number | undefined,
  previewError: string | null,
  runtimeError: string | null,
  errors: PreviewLogEntry[],
): string {
  const details = [
    previewError ? `Preview build error:\n${previewError}` : "",
    runtimeError ? `Runtime error:\n${runtimeError}` : "",
    errors.slice(-5).map(formatErrorLog).join("\n\n"),
  ].filter(Boolean).join("\n\n");
  return [
    "Please fix the current artifact preview errors.",
    "",
    `Artifact: ${artifactLabel(artifact)}`,
    ...(revisionNumber ? [`Revision: v${revisionNumber}`] : []),
    "Files:",
    ...files.map((file) => `- ${file}`),
    "",
    "Errors:",
    "```",
    details || "No error details were captured.",
    "```",
    "",
    "Return the corrected artifact files as named fenced code blocks.",
  ].join("\n");
}

function formatErrorLog(log: PreviewLogEntry): string {
  const location = log.source ? ` (${basename(log.source)}${log.line ? `:${log.line}` : ""})` : "";
  return `${log.message}${location}${log.stack ? `\n${log.stack}` : ""}`;
}

function artifactLabel(artifact: ChatArtifact): string {
  if (!isFileArtifact(artifact) && !artifact.filename) return "Preview source";
  return artifact.filename ?? artifact.title;
}

function downloadName(artifact: ChatArtifact): string {
  const raw = artifact.filename ?? artifact.title;
  return raw.split(/[\\/]/).pop() || "preview.html";
}

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
