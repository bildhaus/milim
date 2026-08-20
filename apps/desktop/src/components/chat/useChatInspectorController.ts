import { useEffect, useState } from "react";
import {
  getPreviewAppStatus,
  preflightPreviewApp,
  restartPreviewApp,
  startPreviewApp,
  startStaticPreview,
  stopPreviewApp,
  type PreviewAppPreflight,
  type PreviewAppStartOptions,
  type PreviewAppStatus,
  type PreviewStaticStartOptions,
  type PreviewSurfaceTarget,
} from "../../api";
import {
  previewRuntimeFoldersEqual,
  previewRuntimeKeyForThread,
} from "../../lib/previewRuntimeKeys";
import { useSessions, type SessionPreviewRuntime } from "../../sessions/store";

function documentVisible(): boolean {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

export function previewRuntimeText(value?: string | null): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

export function previewRuntimeFromStatus(
  status: PreviewAppStatus,
  previous?: SessionPreviewRuntime,
): SessionPreviewRuntime {
  const state = previewRuntimeText(status.status) ?? "idle";
  const url =
    previewRuntimeText(status.url) ??
    (status.active === true ||
    state === "staging" ||
    state === "installing" ||
    state === "starting" ||
    state === "stopping" ||
    state === "error"
      ? previous?.url
      : undefined);
  return {
    kind: status.kind,
    status: state,
    cwd: previewRuntimeText(status.cwd),
    url,
    pid:
      typeof status.pid === "number" && Number.isFinite(status.pid)
        ? status.pid
        : undefined,
    command: previewRuntimeText(status.command),
    message: previewRuntimeText(status.message),
    active: status.active,
    ready: status.ready,
    managed: status.managed,
    runId: previewRuntimeText(status.run_id),
    error: status.error ?? undefined,
    preflight: status.preflight ?? undefined,
  };
}

export function previewStatusFromRuntime(
  threadId: string,
  runtime?: SessionPreviewRuntime,
): PreviewAppStatus | null {
  if (!runtime) return null;
  return {
    thread_id: threadId,
    kind: runtime.kind ?? "app",
    status: runtime.status,
    cwd: runtime.cwd ?? "",
    url: runtime.url ?? null,
    pid: runtime.pid ?? null,
    command: runtime.command ?? null,
    message: runtime.message ?? null,
    active: runtime.active,
    ready: runtime.ready,
    managed: runtime.managed,
    run_id: runtime.runId ?? null,
    updated_at: runtime.updatedAt,
    error: runtime.error ?? null,
    preflight: runtime.preflight ?? null,
    logs: [],
  };
}

export function previewStatusMatchesFolder(
  status: Pick<PreviewAppStatus, "cwd"> | null | undefined,
  folder: string,
): boolean {
  const cwd = previewRuntimeText(folder);
  return !cwd || previewRuntimeFoldersEqual(status?.cwd, cwd);
}

export function useChatInspectorController({
  activeId,
  folder,
  sessionsHydrated,
  visible,
  onNotice,
}: {
  activeId: string;
  folder: string;
  sessionsHydrated: boolean;
  visible: boolean;
  onNotice: (notice: InspectorNotice) => void;
}) {
  const [activePreviewSurface, setActivePreviewSurface] =
    useState<PreviewSurfaceTarget | null>(null);
  const [previewAppStatus, setPreviewAppStatus] =
    useState<PreviewAppStatus | null>(null);
  const [previewAppPreflight, setPreviewAppPreflight] =
    useState<PreviewAppPreflight | null>(null);
  const [previewAppPreflightBusy, setPreviewAppPreflightBusy] = useState(false);
  const [previewAppBusy, setPreviewAppBusy] = useState<
    "start" | "stop" | "restart" | null
  >(null);
  const setSessionPreviewRuntime = useSessions((state) => state.setPreviewRuntime);
  const setPreviewRuntimeByKey = useSessions((state) => state.setPreviewRuntimeByKey);
  const activePreviewRuntimeKey = previewRuntimeKeyForThread(activeId, folder);

  function persistPreviewRuntimeStatus(status: PreviewAppStatus) {
    const state = useSessions.getState();
    const previous = folder.trim()
      ? state.previewRuntimesByKey[activePreviewRuntimeKey]
      : state.sessions.find((session) => session.id === activeId)?.previewRuntime;
    const runtime = previewRuntimeFromStatus(status, previous);
    if (folder.trim()) setPreviewRuntimeByKey(activePreviewRuntimeKey, runtime);
    else setSessionPreviewRuntime(activeId, runtime);
  }

  function applyPreviewAppStatus(status: PreviewAppStatus) {
    const freshStatus = { ...status, stale: false };
    setPreviewAppStatus(freshStatus);
    setPreviewAppPreflight(
      status.kind === "static"
        ? null
        : (status.preflight ?? previewAppPreflight),
    );
    persistPreviewRuntimeStatus(freshStatus);
  }

  async function preflightRuntime(options: PreviewAppStartOptions) {
    setPreviewAppPreflightBusy(true);
    try {
      const preflight = await preflightPreviewApp(
        activePreviewRuntimeKey,
        options,
      );
      setPreviewAppPreflight(preflight);
      setPreviewAppStatus((current) =>
        current ? { ...current, preflight, stale: false } : current,
      );
      onNotice({
        tone: "info",
        message: "Preview commands are ready to review.",
      });
      return preflight;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppPreflightBusy(false);
    }
  }

  async function startRuntime(options: PreviewAppStartOptions) {
    setPreviewAppBusy("start");
    try {
      const status = await startPreviewApp(activePreviewRuntimeKey, options);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusy(null);
    }
  }

  async function startStaticRuntime(options: PreviewStaticStartOptions) {
    setPreviewAppBusy("start");
    try {
      const status = await startStaticPreview(
        activePreviewRuntimeKey,
        options,
      );
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusy(null);
    }
  }

  async function stopRuntime() {
    setPreviewAppBusy("stop");
    try {
      const status = await stopPreviewApp(activePreviewRuntimeKey);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusy(null);
    }
  }

  async function restartRuntime(options: PreviewAppStartOptions) {
    setPreviewAppBusy("restart");
    try {
      const status = await restartPreviewApp(activePreviewRuntimeKey, options);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusy(null);
    }
  }

  useEffect(() => {
    const state = useSessions.getState();
    const runtime = folder.trim()
      ? state.previewRuntimesByKey[activePreviewRuntimeKey]
      : state.sessions.find((session) => session.id === activeId)?.previewRuntime;
    const status = previewStatusFromRuntime(activePreviewRuntimeKey, runtime);
    const matchingStatus = previewStatusMatchesFolder(status, folder)
      ? status
      : null;
    setPreviewAppStatus(matchingStatus);
    setPreviewAppPreflight(matchingStatus?.preflight ?? null);
  }, [activeId, activePreviewRuntimeKey, folder, sessionsHydrated]);

  useEffect(() => {
    if (!visible) return;
    let cancelled = false;
    async function pollPreviewApp() {
      if (!documentVisible()) return;
      try {
        const status = await getPreviewAppStatus(activePreviewRuntimeKey);
        if (cancelled) return;
        if (previewStatusMatchesFolder(status, folder)) {
          const freshStatus = { ...status, stale: false };
          setPreviewAppStatus(freshStatus);
          setPreviewAppPreflight(status.preflight ?? null);
          persistPreviewRuntimeStatus(freshStatus);
        } else {
          setPreviewAppStatus(null);
          if (folder.trim()) {
            setPreviewRuntimeByKey(activePreviewRuntimeKey, undefined);
          } else {
            setSessionPreviewRuntime(activeId, undefined);
          }
        }
      } catch {
        if (!cancelled) {
          setPreviewAppStatus((current) =>
            current?.thread_id === activePreviewRuntimeKey
              ? { ...current, stale: true }
              : current,
          );
        }
      }
    }
    void pollPreviewApp();
    const timer = window.setInterval(() => void pollPreviewApp(), 2500);
    const onVisible = () => {
      if (documentVisible()) void pollPreviewApp();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [
    activeId,
    activePreviewRuntimeKey,
    folder,
    visible,
    setPreviewRuntimeByKey,
    setSessionPreviewRuntime,
  ]);

  return {
    activePreviewRuntimeKey,
    activePreviewSurface,
    previewAppBusy,
    previewAppPreflight,
    previewAppPreflightBusy,
    previewAppStatus,
    preflightRuntime,
    restartRuntime,
    setActivePreviewSurface,
    setPreviewAppPreflight,
    setPreviewAppStatus,
    startRuntime,
    startStaticRuntime,
    stopRuntime,
  };
}

type InspectorNotice = {
  message: string;
  tone: "info" | "warning" | "error";
};
