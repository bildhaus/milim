import { deepEqual, equal, throws } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import {
  AUTO_UPDATE_INTERVAL_MS,
  STARTUP_UPDATE_INTERVAL_MS,
  compareVersions,
  downloadUpdateWithServices,
  getAssetConfigForPlatform,
  getAssetDownloadUrl,
  getUpdateFileName,
  selectUpdateAssets,
  shouldRunAutoUpdateCheck,
  shouldShowUpdateCards,
  type GitHubReleaseAsset,
  type UpdateDownloadProgress,
  type UpdateDownloadServices,
  type UpdateInfo,
} from "../src/update/service.js";
import { UpdateProgress } from "../src/update/UpdateProgress.js";

const SHA_A = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SHA_B = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
const updateStoreSource = readFileSync("src/update/store.ts", "utf8");
const autoUpdaterSource = readFileSync("src/components/AutoUpdater.tsx", "utf8");
const topBarSource = readFileSync("src/components/TopBar.tsx", "utf8");

function asset(
  name: string,
  browserUrl = `https://github.com/oshtz/milim/releases/download/v1/${name}`,
  apiUrl = `https://api.github.com/repos/oshtz/milim/releases/assets/${encodeURIComponent(name)}`,
): GitHubReleaseAsset {
  return { name, url: apiUrl, browser_download_url: browserUrl };
}

function updateInfo(overrides: Partial<UpdateInfo> = {}): UpdateInfo {
  return {
    version: "0.1.30",
    notes: null,
    publishedAt: null,
    assetName: "milim_0.1.30_x64-portable.exe",
    downloadUrl: "https://example.test/milim_0.1.30_x64-portable.exe",
    checksumUrl: "https://example.test/milim_0.1.30_x64-portable.exe.sha256",
    ...overrides,
  };
}

function createDownloadServices(os: string, events: string[]): UpdateDownloadServices {
  return {
    getPlatform: async () => {
      events.push("platform");
      return os;
    },
    downloadUpdateFile: async (request, onProgress) => {
      events.push(`native:${request.fileName}:${request.assetName}:${request.downloadUrl}:${request.checksumUrl}`);
      events.push(`native-checksum:${request.checksumUrl.includes(".sha256") ? SHA_A : SHA_B}`);
      onProgress?.({ phase: "downloading", downloadedBytes: 5, totalBytes: 10 });
      onProgress?.({ phase: "verifying", downloadedBytes: 10, totalBytes: 10 });
      return `C:/Users/USER/AppData/Local/com.omershatz.milim/milim-updates/${request.fileName}`;
    },
    extractAppZip: async (zipPath) => {
      events.push(`extract:${zipPath}`);
      return `${zipPath}/milim.app`;
    },
  };
}

deepEqual(
  { name: getAssetConfigForPlatform("darwin").name, extension: getAssetConfigForPlatform("darwin").extension },
  { name: "milim.app.zip", extension: ".app.zip" },
);
equal(getAssetConfigForPlatform("win32").extension, ".exe");
throws(() => getAssetConfigForPlatform("linux"), /not supported/);
equal(getUpdateFileName(updateInfo(), "win32"), "milim-0.1.30.exe");
equal(getUpdateFileName(updateInfo(), "darwin"), "milim-0.1.30.app.zip");

equal(compareVersions("v0.1.30", "0.1.29"), 1);
equal(compareVersions("0.1.29-beta.1", "0.1.29"), 0);
equal(compareVersions("0.1.28", "0.1.29"), -1);

equal(AUTO_UPDATE_INTERVAL_MS, 12 * 60 * 60 * 1000);
equal(STARTUP_UPDATE_INTERVAL_MS, 120 * 60 * 1000);
equal(shouldRunAutoUpdateCheck(null, 10), true);
equal(shouldRunAutoUpdateCheck(10, 10 + AUTO_UPDATE_INTERVAL_MS - 1), false);
equal(shouldRunAutoUpdateCheck(10, 10 + AUTO_UPDATE_INTERVAL_MS), true);
equal(shouldRunAutoUpdateCheck(10, 10 + STARTUP_UPDATE_INTERVAL_MS - 1, STARTUP_UPDATE_INTERVAL_MS), false);
equal(shouldRunAutoUpdateCheck(10, 10 + STARTUP_UPDATE_INTERVAL_MS, STARTUP_UPDATE_INTERVAL_MS), true);
equal(shouldShowUpdateCards("0.2.0", "0.2.0", null), true);
equal(shouldShowUpdateCards("0.2.0", "0.2.0", "0.2.0"), false);
equal(shouldShowUpdateCards("0.2.1", "0.2.0", "0.1.45"), false);
equal(updateStoreSource.includes("if (get().currentVersion) return;"), false, "current app version should refresh from Tauri on launch");
equal(updateStoreSource.includes("currentVersion: state.currentVersion"), false, "current app version should not persist across installed binaries");
equal(
  updateStoreSource.includes("isInstalledUpdate(version, get().updateInfo)"),
  true,
  "installed update packages should be cleared after the new binary launches",
);
equal(
  updateStoreSource.includes("compareVersions(currentVersion, updateInfo.version) >= 0"),
  true,
  "stale update packages should be detected by semantic version comparison",
);
equal(updateStoreSource.includes("downloadProgress: state.downloadProgress"), false, "download progress should not persist");
equal(
  (updateStoreSource.match(/downloadProgress: null/g) ?? []).length >= 4,
  true,
  "download progress should reset after checks, success, and errors",
);
equal(autoUpdaterSource.includes("window.confirm"), false, "automatic update checks should not prompt on startup");
equal(autoUpdaterSource.includes("downloadNow"), false, "the scheduler should delegate automatic download policy to the update store");
equal(autoUpdaterSource.includes("installNow"), false, "automatic update checks should not install before the user clicks update");
equal(autoUpdaterSource.includes("ignoreVersion"), false, "canceling the top-bar update prompt should not hide the update");
equal(autoUpdaterSource.includes("void run(true)"), true, "startup should use the shorter startup update guard");
equal(autoUpdaterSource.includes("window.setInterval(() => void run()"), true, "background checks should keep the default automatic guard");
equal(
  updateStoreSource.includes("if (options?.automatic && get().automaticDownload)"),
  true,
  "automatic checks should stage updates when automatic downloads are enabled",
);
equal(
  updateStoreSource.includes('set({ status: "available", downloadProgress: null, error: formatError(error) })'),
  true,
  "failed background downloads should remain retryable",
);
equal(topBarSource.includes('data-testid="topbar-update"'), true, "available updates should render a top-bar update button");
equal(topBarSource.includes("window.confirm"), false, "top-bar update flow should use the themed app dialog");
equal(topBarSource.includes('role="dialog"'), true, "top-bar update flow should render an in-app confirmation dialog");
equal(topBarSource.includes('if (updateStatus === "ready")'), true, "ready updates should install on the first top-bar click");
equal(topBarSource.includes("Restart to update milim"), true, "ready updates should identify the restart action");
equal(topBarSource.includes("<Refresh size={14} />"), true, "ready updates should use the restart icon");
equal(
  topBarSource.lastIndexOf("await installNow()") > topBarSource.indexOf("await downloadNow(updateInfo)"),
  true,
  "the fallback top-bar update flow should download before installing",
);

const windowsSelected = selectUpdateAssets(
  [
    asset("milim_0.1.30_x64-portable.exe"),
    asset("SHA256SUMS.txt"),
    asset("milim_0.1.30_x64-portable.exe.sha256"),
  ],
  getAssetConfigForPlatform("win32"),
);
equal(windowsSelected.asset.name, "milim_0.1.30_x64-portable.exe");
equal(windowsSelected.checksumAsset.name, "milim_0.1.30_x64-portable.exe.sha256");

const macSelected = selectUpdateAssets([asset("milim.app.zip"), asset("SHA256SUMS.txt")], getAssetConfigForPlatform("darwin"));
equal(macSelected.asset.name, "milim.app.zip");
equal(macSelected.checksumAsset.name, "SHA256SUMS.txt");

equal(getAssetDownloadUrl(asset("milim.app.zip")), "https://github.com/oshtz/milim/releases/download/v1/milim.app.zip");
equal(
  getAssetDownloadUrl({ name: "milim.app.zip", url: "https://api.github.com/repos/oshtz/milim/releases/assets/1" }),
  "https://api.github.com/repos/oshtz/milim/releases/assets/1",
);
throws(() => getAssetDownloadUrl({ name: "milim.app.zip" }), /no download URL/);

const events: string[] = [];
const windowsProgress: UpdateDownloadProgress[] = [];
const downloaded = await downloadUpdateWithServices(
  updateInfo(),
  createDownloadServices("win32", events),
  (progress) => windowsProgress.push(progress),
);
equal(downloaded.endsWith("milim-0.1.30.exe"), true);
deepEqual(events, [
  "platform",
  "native:milim-0.1.30.exe:milim_0.1.30_x64-portable.exe:https://example.test/milim_0.1.30_x64-portable.exe:https://example.test/milim_0.1.30_x64-portable.exe.sha256",
  `native-checksum:${SHA_A}`,
]);
deepEqual(windowsProgress, [
  { phase: "downloading", downloadedBytes: 5, totalBytes: 10 },
  { phase: "verifying", downloadedBytes: 10, totalBytes: 10 },
]);

const macEvents: string[] = [];
const macProgress: UpdateDownloadProgress[] = [];
await downloadUpdateWithServices(
  updateInfo({
    assetName: "milim.app.zip",
    downloadUrl: "https://example.test/milim.app.zip",
    checksumUrl: "https://example.test/milim.app.zip.sha256",
  }),
  createDownloadServices("darwin", macEvents),
  (progress) => macProgress.push(progress),
);
equal(macEvents.at(-1)?.includes("extract:"), true);
deepEqual(macProgress.at(-1), { phase: "preparing", downloadedBytes: 10, totalBytes: null });

const determinateProgress = renderToStaticMarkup(UpdateProgress({
  progress: { phase: "downloading", downloadedBytes: 5, totalBytes: 10 },
}));
equal(determinateProgress.includes('aria-valuenow="50"'), true);
equal(determinateProgress.includes("50% · 5 B of 10 B"), true);
equal(determinateProgress.includes("width:50%"), true);

const indeterminateProgress = renderToStaticMarkup(UpdateProgress({
  progress: { phase: "downloading", downloadedBytes: 5, totalBytes: null },
}));
equal(indeterminateProgress.includes("progress-fill indeterminate"), true);
equal(indeterminateProgress.includes("aria-valuenow"), false);
