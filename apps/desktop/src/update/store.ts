import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import {
  checkForUpdate,
  compareVersions,
  downloadUpdate,
  installUpdate,
  shouldRunAutoUpdateCheck,
  STARTUP_UPDATE_INTERVAL_MS,
  type UpdateDownloadProgress,
  type UpdateInfo,
} from "./service.js";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "available"
  | "downloading"
  | "ready"
  | "installing"
  | "disabled"
  | "error";

interface UpdateState {
  automaticCheck: boolean;
  automaticDownload: boolean;
  currentVersion: string | null;
  status: UpdateStatus;
  updateInfo: UpdateInfo | null;
  updatePath: string | null;
  downloadProgress: UpdateDownloadProgress | null;
  error: string | null;
  lastCheckedAt: number | null;
  ignoredVersion: string | null;
  recoveryChecked: boolean;
  loadCurrentVersion: () => Promise<void>;
  checkNow: (options?: { automatic?: boolean; startup?: boolean }) => Promise<UpdateInfo | null>;
  downloadNow: (info?: UpdateInfo | null) => Promise<string | null>;
  installNow: () => Promise<void>;
  ignoreVersion: (version: string) => void;
  setAutomaticCheck: (automaticCheck: boolean) => void;
  setAutomaticDownload: (automaticDownload: boolean) => void;
}

const LOCAL_UPDATE_STATE_KEY = "milim.local.updates";
const UPDATE_STATE_VERSION = 1;

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function isDevBuild(): boolean {
  return Boolean((import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV);
}

function getMachineLocalStorage(): Storage | null {
  try {
    return typeof globalThis.localStorage === "undefined" ? null : globalThis.localStorage;
  } catch {
    return null;
  }
}

function formatError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Update request failed.";
}

function isInstalledUpdate(currentVersion: string, updateInfo: UpdateInfo | null): boolean {
  return Boolean(updateInfo && compareVersions(currentVersion, updateInfo.version) >= 0);
}

async function readCurrentVersion(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

async function takeUpdateRecoveryError(): Promise<string | null> {
  if (!isTauriRuntime()) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<string | null>("take_update_recovery_error");
}

export const useUpdateStore = create<UpdateState>()(
  persist(
    (set, get) => ({
      automaticCheck: true,
      automaticDownload: true,
      currentVersion: null,
      status: "idle",
      updateInfo: null,
      updatePath: null,
      downloadProgress: null,
      error: null,
      lastCheckedAt: null,
      ignoredVersion: null,
      recoveryChecked: false,

      loadCurrentVersion: async () => {
        if (!get().recoveryChecked) {
          try {
            const recoveryError = await takeUpdateRecoveryError();
            if (recoveryError) {
              set({
                status: "error",
                error: recoveryError,
                lastCheckedAt: Date.now(),
                recoveryChecked: true,
              });
            } else {
              set({ recoveryChecked: true });
            }
          } catch {
            set({ recoveryChecked: true });
          }
        }
        const version = await readCurrentVersion();
        if (!version) return;
        if (isInstalledUpdate(version, get().updateInfo)) {
          const state = get();
          set({
            currentVersion: version,
            status: state.status === "error" ? state.status : "up-to-date",
            updateInfo: null,
            updatePath: null,
            downloadProgress: null,
            error: state.status === "error" ? state.error : null,
            ignoredVersion: null,
          });
          return;
        }
        set({ currentVersion: version });
      },

      checkNow: async (options) => {
        await get().loadCurrentVersion();
        if (
          options?.automatic &&
          !shouldRunAutoUpdateCheck(
            get().lastCheckedAt,
            Date.now(),
            options.startup ? STARTUP_UPDATE_INTERVAL_MS : undefined,
          )
        )
          return null;
        if (!isTauriRuntime()) {
          set({ status: "disabled", error: "Updates are only available in the desktop app.", lastCheckedAt: Date.now() });
          return null;
        }
        if (isDevBuild()) {
          set({ status: "disabled", error: "Updates are disabled in development builds.", lastCheckedAt: Date.now() });
          return null;
        }

        set({ status: "checking", downloadProgress: null, error: null });

        try {
          const info = await checkForUpdate();
          if (!info) {
            set({ status: "up-to-date", updateInfo: null, updatePath: null, downloadProgress: null, lastCheckedAt: Date.now() });
            return null;
          }
          if (options?.automatic && get().ignoredVersion === info.version) {
            set({ status: "available", updateInfo: info, updatePath: null, lastCheckedAt: Date.now() });
            return null;
          }
          set({ status: "available", updateInfo: info, updatePath: null, lastCheckedAt: Date.now() });
          if (options?.automatic && get().automaticDownload) {
            await get().downloadNow(info);
          }
          return info;
        } catch (error) {
          set({ status: "error", downloadProgress: null, error: formatError(error), lastCheckedAt: Date.now() });
          return null;
        }
      },

      downloadNow: async (info) => {
        const updateInfo = info ?? get().updateInfo;
        if (!updateInfo) return null;
        if (get().status === "ready" && get().updatePath) return get().updatePath;

        set({
          status: "downloading",
          downloadProgress: { phase: "downloading", downloadedBytes: 0, totalBytes: null },
          error: null,
          updateInfo,
        });
        try {
          const updatePath = await downloadUpdate(updateInfo, (downloadProgress) => set({ downloadProgress }));
          set({ status: "ready", updatePath, updateInfo, downloadProgress: null, ignoredVersion: null });
          return updatePath;
        } catch (error) {
          set({ status: "available", downloadProgress: null, error: formatError(error) });
          return null;
        }
      },

      installNow: async () => {
        const updatePath = get().updatePath;
        if (!updatePath) {
          set({ status: "error", error: "Update package not downloaded yet." });
          return;
        }
        set({
          status: "installing",
          downloadProgress: { phase: "restarting", downloadedBytes: 1, totalBytes: 1 },
          error: null,
        });
        try {
          await installUpdate(updatePath);
        } catch (error) {
          set({ status: "ready", downloadProgress: null, error: formatError(error) });
        }
      },

      ignoreVersion: (version) => set({ ignoredVersion: version }),
      setAutomaticCheck: (automaticCheck) => set({ automaticCheck }),
      setAutomaticDownload: (automaticDownload) => set({ automaticDownload }),
    }),
    {
      name: LOCAL_UPDATE_STATE_KEY,
      storage: createJSONStorage(() => getMachineLocalStorage() ?? localStorage),
      version: UPDATE_STATE_VERSION,
      migrate: (persisted, version) => {
        const saved = persisted as Partial<UpdateState>;
        return version < UPDATE_STATE_VERSION ? { ...saved, automaticDownload: true } : saved;
      },
      merge: (persisted, current) => {
        const saved = persisted as Partial<UpdateState> | undefined;
        return {
          ...current,
          ...saved,
          automaticCheck: typeof saved?.automaticCheck === "boolean" ? saved.automaticCheck : true,
          automaticDownload: typeof saved?.automaticDownload === "boolean" ? saved.automaticDownload : true,
        };
      },
      partialize: (state) => ({
        automaticCheck: state.automaticCheck,
        automaticDownload: state.automaticDownload,
        status: state.status,
        updateInfo: state.updateInfo,
        updatePath: state.updatePath,
        error: state.error,
        lastCheckedAt: state.lastCheckedAt,
        ignoredVersion: state.ignoredVersion,
      }),
    },
  ),
);
