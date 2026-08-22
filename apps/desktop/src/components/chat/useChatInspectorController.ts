import { useCallback, useEffect, useState } from "react";
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
  const activePreviewRuntimeKey = previewRuntimeKeyForThread(activeId, folder);
  const [previewSurfaceState, setPreviewSurfaceState] = useState<{
    threadId: string;
    target: PreviewSurfaceTarget;
  } | null>(null);
  const activePreviewSurface =
    previewSurfaceState?.threadId === activeId
      ? previewSurfaceState.target
      : null;
  const setActivePreviewSurface = useCallback(
    (target: PreviewSurfaceTarget | null) => {
      setPreviewSurfaceState((current) =>
        target
          ? { threadId: activeId, target }
          : current?.threadId === activeId
            ? null
            : current,
      );
    },
    [activeId],
  );
  const [previewAppStatus, setPreviewAppStatus] =
    useState<PreviewAppStatus | null>(null);
  const [previewAppPreflight, setPreviewAppPreflight] =
    useState<PreviewAppPreflight | null>(null);
  const [previewAppPreflightBusyKey, setPreviewAppPreflightBusyKey] =
    useState<string | null>(null);
  const [previewAppBusyState, setPreviewAppBusyState] = useState<{
    key: string;
    action: "start" | "stop" | "restart";
  } | null>(null);
  const previewAppPreflightBusy =
    previewAppPreflightBusyKey === activePreviewRuntimeKey;
  const previewAppBusy =
    previewAppBusyState?.key === activePreviewRuntimeKey
      ? previewAppBusyState.action
      : null;
  const setSessionPreviewRuntime = useSessions((state) => state.setPreviewRuntime);
  const setPreviewRuntimeByKey = useSessions((state) => state.setPreviewRuntimeByKey);

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
    const runtimeKey = activePreviewRuntimeKey;
    setPreviewAppPreflightBusyKey(runtimeKey);
    try {
      const preflight = await preflightPreviewApp(
        runtimeKey,
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
      setPreviewAppPreflightBusyKey((current) =>
        current === runtimeKey ? null : current,
      );
    }
  }

  async function startRuntime(options: PreviewAppStartOptions) {
    const runtimeKey = activePreviewRuntimeKey;
    setPreviewAppBusyState({ key: runtimeKey, action: "start" });
    try {
      const status = await startPreviewApp(runtimeKey, options);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusyState((current) =>
        current?.key === runtimeKey ? null : current,
      );
    }
  }

  async function startStaticRuntime(options: PreviewStaticStartOptions) {
    const runtimeKey = activePreviewRuntimeKey;
    setPreviewAppBusyState({ key: runtimeKey, action: "start" });
    try {
      const status = await startStaticPreview(
        runtimeKey,
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
      setPreviewAppBusyState((current) =>
        current?.key === runtimeKey ? null : current,
      );
    }
  }

  async function stopRuntime() {
    const runtimeKey = activePreviewRuntimeKey;
    setPreviewAppBusyState({ key: runtimeKey, action: "stop" });
    try {
      const status = await stopPreviewApp(runtimeKey);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusyState((current) =>
        current?.key === runtimeKey ? null : current,
      );
    }
  }

  async function restartRuntime(options: PreviewAppStartOptions) {
    const runtimeKey = activePreviewRuntimeKey;
    setPreviewAppBusyState({ key: runtimeKey, action: "restart" });
    try {
      const status = await restartPreviewApp(runtimeKey, options);
      applyPreviewAppStatus(status);
      return status;
    } catch (error) {
      onNotice({
        tone: "error",
        message: error instanceof Error ? error.message : String(error),
      });
      return null;
    } finally {
      setPreviewAppBusyState((current) =>
        current?.key === runtimeKey ? null : current,
      );
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
