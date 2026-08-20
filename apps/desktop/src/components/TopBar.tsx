import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent, ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { getCodexRateLimits, isClaudeModel, isCodexModel } from "../api";
import {
  codexLimitsFromRateLimitPayload,
  formatCompactProviderLimits,
  formatProviderLimits,
  formatThreadMetricsBreakdown,
  latestProviderLimits,
} from "../lib/usageMetrics";
import { deriveThreadTitle, shouldReplaceThreadTitle } from "../lib/threadTitles";
import { readUserStateKey } from "../persistence/userStateStorage.js";
import { useSessions } from "../sessions/store";
import { shortcutMatchesEvent, uiSizeShortcutDelta } from "../ui/shortcuts";
import { startWindowDrag } from "../ui/windowDrag";
import {
  DEFAULT_UI_SIZE,
  MAX_UI_SIZE,
  MIN_UI_SIZE,
  UI_SIZE_STEP,
  useUiPreferences,
} from "../ui/store";
import { useUpdateStore } from "../update/store";
import { UpdateProgress } from "../update/UpdateProgress";
import { Download, Pin, Refresh } from "./icons";
import { WindowControls } from "./WindowControls";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const PINNED_KEY = "milim.window.alwaysOnTop";
const ZOOM_CHIP_IDLE_MS = 3000;

export function TopBar({
  onOpenAppMenu,
  threadNavigation,
}: {
  onOpenAppMenu: (event: MouseEvent<HTMLButtonElement>) => void;
  threadNavigation?: ReactNode;
}) {
  const [pinnedReady, setPinnedReady] = useState(!inTauri);
  const [updateActionRunning, setUpdateActionRunning] = useState(false);
  const [confirmingUpdate, setConfirmingUpdate] = useState(false);
  const [zoomChipVisible, setZoomChipVisible] = useState(false);
  const [codexLimits, setCodexLimits] = useState<ReturnType<typeof latestProviderLimits>>([]);
  const zoomChipTimerRef = useRef<number | null>(null);
  const pinned = useUiPreferences((s) => s.windowAlwaysOnTop);
  const setPinned = useUiPreferences((s) => s.setWindowAlwaysOnTop);
  const uiSize = useUiPreferences((s) => s.uiSize);
  const setUiSize = useUiPreferences((s) => s.setUiSize);
  const showAccountUsage = useUiPreferences((s) => s.showAccountUsageInTitleBar);
  const updateStatus = useUpdateStore((s) => s.status);
  const updateInfo = useUpdateStore((s) => s.updateInfo);
  const updateProgress = useUpdateStore((s) => s.downloadProgress);
  const updateError = useUpdateStore((s) => s.error);
  const downloadNow = useUpdateStore((s) => s.downloadNow);
  const installNow = useUpdateStore((s) => s.installNow);
  const activeSession = useSessions((s) => s.sessions.find((session) => session.id === s.activeId));
  const messages = activeSession?.messages ?? [];
  const storedThreadTitle = activeSession?.title?.trim() || "New chat";
  const threadTitle = shouldReplaceThreadTitle(storedThreadTitle, messages)
    ? deriveThreadTitle(messages)
    : storedThreadTitle;
  const threadMetrics = formatThreadMetricsBreakdown(messages);
  const model = activeSession?.settings?.model?.trim() ?? "";
  const activeModelIsCodex = isCodexModel(model);
  const activeModelIsClaude = isClaudeModel(model);
  const providerLimits = activeModelIsCodex
    ? codexLimits
    : activeModelIsClaude
      ? latestProviderLimits(activeSession?.messages ?? [], "claude")
      : [];
  const providerLimitText = showAccountUsage ? formatCompactProviderLimits(providerLimits) : null;
  const providerLimitTitle = showAccountUsage ? formatProviderLimits(providerLimits) : null;
  const updateBusy = updateActionRunning || updateStatus === "downloading" || updateStatus === "installing";
  const showUpdateButton = !!updateInfo && (updateStatus === "available" || updateStatus === "ready" || updateBusy);
  const updateVersionLabel = updateInfo ? `v${updateInfo.version.replace(/^v/i, "")}` : "";
  const updateButtonLabel = updateBusy
    ? updateStatus === "downloading" ? `Downloading milim ${updateVersionLabel}` : `Installing milim ${updateVersionLabel}`
    : updateStatus === "ready" ? `Restart to update milim ${updateVersionLabel}` : `Install milim ${updateVersionLabel}`;
  const visibleUpdateProgress = updateProgress ?? {
    phase: updateStatus === "installing" ? "restarting" as const : "downloading" as const,
    downloadedBytes: 0,
    totalBytes: null,
  };

  useEffect(() => {
    if (!inTauri) return;
    let cancelled = false;
    const w = getCurrentWindow();
    void (async () => {
      const stored = (await Promise.resolve(readUserStateKey(PINNED_KEY))) === "true";
      try {
        const current = await w.isAlwaysOnTop();
        if (cancelled) return;
        const next = stored || current || useUiPreferences.getState().windowAlwaysOnTop;
        setPinned(next);
      } catch {
        if (cancelled) return;
        setPinned(stored);
      } finally {
        if (!cancelled) setPinnedReady(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [setPinned]);

  useEffect(() => {
    if (!inTauri || !pinnedReady) return;
    void getCurrentWindow()
      .setAlwaysOnTop(pinned)
      .catch(() => {});
  }, [pinned, pinnedReady]);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (!event.defaultPrevented && inTauri && event.metaKey && shortcutMatchesEvent("Mod+W", event)) {
        event.preventDefault();
        void getCurrentWindow().close().catch(() => {});
        return;
      }
      const delta = uiSizeShortcutDelta(event);
      if (!delta) return;
      event.preventDefault();
      changeUiSize(delta * UI_SIZE_STEP);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [setUiSize]);

  useEffect(() => () => clearZoomChipTimer(), []);

  useEffect(() => {
    if (!showAccountUsage || !activeModelIsCodex) {
      setCodexLimits([]);
      return;
    }
    let cancelled = false;
    const refresh = async () => {
      if (document.visibilityState === "hidden") return;
      try {
        const limits = codexLimitsFromRateLimitPayload(await getCodexRateLimits());
        if (!cancelled) setCodexLimits(limits);
      } catch {
        if (!cancelled) setCodexLimits([]);
      }
    };
    void refresh();
    const timer = setInterval(() => void refresh(), 60_000);
    const onVisible = () => void refresh();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [activeModelIsCodex, showAccountUsage]);

  function toggleAlwaysOnTop() {
    setPinned(!pinned);
  }

  function clearZoomChipTimer() {
    if (zoomChipTimerRef.current === null) return;
    window.clearTimeout(zoomChipTimerRef.current);
    zoomChipTimerRef.current = null;
  }

  function scheduleZoomChipDismissal() {
    clearZoomChipTimer();
    zoomChipTimerRef.current = window.setTimeout(() => {
      setZoomChipVisible(false);
      zoomChipTimerRef.current = null;
    }, ZOOM_CHIP_IDLE_MS);
  }

  function revealZoomChip() {
    setZoomChipVisible(true);
    scheduleZoomChipDismissal();
  }

  function changeUiSize(delta: number) {
    setUiSize(useUiPreferences.getState().uiSize + delta);
    revealZoomChip();
  }

  function resetUiSize() {
    setUiSize(DEFAULT_UI_SIZE);
    revealZoomChip();
  }

  async function runTopBarUpdate() {
    if (!updateInfo || updateBusy) return;
    if (updateStatus === "ready") {
      setConfirmingUpdate(true);
      setUpdateActionRunning(true);
      try {
        await installNow();
      } finally {
        setUpdateActionRunning(false);
      }
      return;
    }
    setConfirmingUpdate(true);
  }

  async function confirmTopBarUpdate() {
    if (!updateInfo || updateBusy) return;

    setUpdateActionRunning(true);
    try {
      const path = await downloadNow(updateInfo);
      if (path) await installNow();
    } finally {
      setUpdateActionRunning(false);
    }
  }

  return (
    <header className={"topbar" + (threadNavigation ? " topbar-with-thread-navigation" : "")} data-tauri-drag-region onMouseDown={startWindowDrag}>
      <div className="topbar-side topbar-left" aria-label="Current chat" data-tauri-drag-region>
        <button
          type="button"
          className="topbar-logo-button"
          data-testid="app-menu-trigger"
          title="Milim menu"
          aria-label="Open Milim menu"
          aria-haspopup="menu"
          onClick={onOpenAppMenu}
        >
          <span className="topbar-logo" aria-hidden="true" />
        </button>
        <span className="topbar-divider" aria-hidden="true" data-tauri-drag-region />
        <span className="topbar-thread" title={threadTitle} data-tauri-drag-region>
          {threadTitle}
        </span>
        {threadMetrics.label && (
          <span
            className="topbar-usage"
            data-testid="thread-usage"
            title={threadMetrics.title ?? undefined}
            aria-label={`Thread totals: ${threadMetrics.label}`}
            data-tauri-drag-region
          >
            {threadMetrics.label}
          </span>
        )}
        {providerLimitText && (
          <span
            className="topbar-account-usage"
            data-testid="account-usage-pill"
            title={providerLimitTitle ?? providerLimitText}
            aria-label={`Account quota: ${providerLimitTitle ?? providerLimitText}`}
            data-tauri-drag-region
          >
            {providerLimitText}
          </span>
        )}
      </div>

      {threadNavigation && (
        <div className="topbar-thread-navigation">
          {threadNavigation}
        </div>
      )}

      <div className="topbar-side topbar-right">
        {showUpdateButton && (
          <button
            type="button"
            className={"topbar-update-btn" + (updateBusy ? " busy" : "")}
            data-testid="topbar-update"
            title={updateButtonLabel}
            aria-label={updateButtonLabel}
            disabled={updateBusy}
            onClick={() => void runTopBarUpdate()}
          >
            {updateStatus === "ready" || updateStatus === "installing" ? <Refresh size={14} /> : <Download size={14} />}
          </button>
        )}
        {zoomChipVisible && (
          <div
            className="topbar-zoom-chip"
            role="group"
            aria-label="UI zoom controls"
            data-testid="ui-zoom-chip"
            data-window-drag-ignore
          >
            <span className="topbar-zoom-value" data-testid="ui-zoom-value" aria-live="polite">
              {uiSize}%
            </span>
            <button
              type="button"
              className="topbar-zoom-btn"
              data-testid="ui-zoom-decrease"
              title="Zoom out"
              aria-label="Zoom out"
              disabled={uiSize <= MIN_UI_SIZE}
              onClick={() => changeUiSize(-UI_SIZE_STEP)}
            >
              <span aria-hidden="true">−</span>
            </button>
            <button
              type="button"
              className="topbar-zoom-btn"
              data-testid="ui-zoom-increase"
              title="Zoom in"
              aria-label="Zoom in"
              disabled={uiSize >= MAX_UI_SIZE}
              onClick={() => changeUiSize(UI_SIZE_STEP)}
            >
              <span aria-hidden="true">+</span>
            </button>
            <button
              type="button"
              className="topbar-zoom-reset"
              data-testid="ui-zoom-reset"
              disabled={uiSize === DEFAULT_UI_SIZE}
              onClick={resetUiSize}
            >
              Reset
            </button>
          </div>
        )}
        <button
          type="button"
          className={"icon-btn" + (pinned ? " active" : "")}
          data-testid="pin-window"
          title={pinned ? "Keep on top: on" : "Keep on top"}
          aria-label={pinned ? "Disable keep on top" : "Enable keep on top"}
          aria-pressed={pinned}
          onClick={toggleAlwaysOnTop}
        >
          <Pin size={15} />
        </button>
        <WindowControls />
      </div>
      {typeof document !== "undefined" && confirmingUpdate && updateInfo && createPortal(
        <div
          className="git-modal-backdrop"
          data-native-preview-blocker="true"
          onMouseDown={(event) => {
            if (!updateActionRunning && event.target === event.currentTarget) setConfirmingUpdate(false);
          }}
        >
          <section className="git-modal update-confirm-modal" role="dialog" aria-modal="true" aria-labelledby="update-confirm-title">
            <div className="git-modal-head">
              <span>
                <Download size={14} />
                <strong id="update-confirm-title">
                  {updateActionRunning ? "Installing update" : updateError ? "Update failed" : "Install update"}
                </strong>
              </span>
            </div>
            {updateActionRunning ? (
              <UpdateProgress progress={visibleUpdateProgress} />
            ) : updateError ? (
              <>
                <p className="error">{updateError}</p>
                <div className="update-confirm-actions">
                  <button className="btn-ghost" type="button" onClick={() => setConfirmingUpdate(false)}>
                    Close
                  </button>
                  <button className="btn-accent" type="button" onClick={() => void confirmTopBarUpdate()}>
                    Retry
                  </button>
                </div>
              </>
            ) : (
              <>
                <p>Install milim {updateVersionLabel}? The app will download the update, close, replace itself, and reopen.</p>
                <div className="update-confirm-actions">
                  <button className="btn-ghost" type="button" onClick={() => setConfirmingUpdate(false)}>
                    Cancel
                  </button>
                  <button className="btn-accent" type="button" onClick={() => void confirmTopBarUpdate()}>
                    Update now
                  </button>
                </div>
              </>
            )}
          </section>
        </div>,
        document.body,
      )}
    </header>
  );
}
