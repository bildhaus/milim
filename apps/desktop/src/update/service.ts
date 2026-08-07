export type GitHubReleaseAsset = {
  name: string;
  url?: string;
  browser_download_url?: string;
};

type GitHubRelease = {
  tag_name: string;
  body?: string | null;
  published_at?: string | null;
  assets?: GitHubReleaseAsset[];
};

export type UpdateInfo = {
  version: string;
  notes: string | null;
  publishedAt: string | null;
  assetName: string;
  downloadUrl: string;
  checksumUrl: string;
};

export type UpdateDownloadProgress = {
  phase: "downloading" | "verifying" | "preparing" | "restarting";
  downloadedBytes: number;
  totalBytes: number | null;
};

type AssetConfig = {
  name: string;
  extension: string;
  matches: (asset: GitHubReleaseAsset) => boolean;
};

export const AUTO_UPDATE_INTERVAL_MS = 12 * 60 * 60 * 1000;
export const STARTUP_UPDATE_INTERVAL_MS = 120 * 60 * 1000;
const GITHUB_REPO = "bildhaus/milim";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function isDevBuild(): boolean {
  return Boolean((import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV);
}

export function normalizeVersion(version: string): string {
  return version.trim().replace(/^v/i, "").split("-")[0];
}

export function compareVersions(left: string, right: string): number {
  const leftParts = normalizeVersion(left).split(".").map((part) => Number(part) || 0);
  const rightParts = normalizeVersion(right).split(".").map((part) => Number(part) || 0);
  const maxLength = Math.max(leftParts.length, rightParts.length);

  for (let index = 0; index < maxLength; index++) {
    const leftValue = leftParts[index] ?? 0;
    const rightValue = rightParts[index] ?? 0;
    if (leftValue > rightValue) return 1;
    if (leftValue < rightValue) return -1;
  }

  return 0;
}

export function shouldRunAutoUpdateCheck(
  lastCheckedAt: number | null,
  now = Date.now(),
  intervalMs = AUTO_UPDATE_INTERVAL_MS,
): boolean {
  return !lastCheckedAt || now - lastCheckedAt >= intervalMs;
}

export function shouldShowUpdateCards(
  currentVersion: string | null,
  releaseVersion: string,
  seenVersion: string | null,
): boolean {
  return currentVersion === releaseVersion && seenVersion !== currentVersion;
}

export function getAssetConfigForPlatform(os: string): AssetConfig {
  if (os === "darwin") {
    return {
      name: "milim.app.zip",
      extension: ".app.zip",
      matches: (asset) => asset.name === "milim.app.zip",
    };
  }

  if (os === "win32") {
    return {
      name: "Windows portable EXE",
      extension: ".exe",
      matches: (asset) => {
        const name = asset.name.toLowerCase();
        return name.endsWith("_x64-portable.exe") || name.endsWith("-portable.exe");
      },
    };
  }

  throw new Error("Auto-update is not supported on this platform.");
}

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url, {
    method: "GET",
    headers: {
      Accept: "application/vnd.github+json",
    },
  });

  if (!response.ok) {
    if (response.status === 403) throw new Error("GitHub API rate limit exceeded. Try again later.");
    throw new Error(`GitHub API request failed (${response.status})`);
  }

  return response.json() as Promise<T>;
}

export function selectUpdateAssets(
  assets: GitHubReleaseAsset[],
  assetConfig: AssetConfig,
): { asset: GitHubReleaseAsset; checksumAsset: GitHubReleaseAsset } {
  const asset = assets.find(assetConfig.matches);
  if (!asset) throw new Error("No compatible update asset found for this platform.");

  const checksumAsset =
    assets.find((entry) => entry.name === `${asset.name}.sha256`) ??
    assets.find((entry) => entry.name.toLowerCase() === "sha256sums.txt") ??
    assets.find((entry) => entry.name.toLowerCase() === "checksums.txt");

  if (!checksumAsset) throw new Error(`No SHA-256 checksum asset found for ${asset.name}.`);
  return { asset, checksumAsset };
}

export function getAssetDownloadUrl(asset: GitHubReleaseAsset): string {
  const url = asset.browser_download_url || asset.url;
  if (!url) throw new Error(`Release asset ${asset.name} has no download URL.`);
  return url;
}

export function getUpdateFileName(update: UpdateInfo, os: string): string {
  return os === "darwin" ? `milim-${update.version}.app.zip` : `milim-${update.version}.exe`;
}

export type UpdateDownloadFileRequest = {
  downloadUrl: string;
  checksumUrl: string;
  assetName: string;
  fileName: string;
};

export type UpdateDownloadServices = {
  getPlatform: () => Promise<string>;
  downloadUpdateFile: (
    request: UpdateDownloadFileRequest,
    onProgress?: (progress: UpdateDownloadProgress) => void,
  ) => Promise<string>;
  extractAppZip: (zipPath: string) => Promise<string>;
};

export async function downloadUpdateWithServices(
  update: UpdateInfo,
  services: UpdateDownloadServices,
  onProgress?: (progress: UpdateDownloadProgress) => void,
): Promise<string> {
  const os = await services.getPlatform();
  let latestProgress: UpdateDownloadProgress = {
    phase: "downloading",
    downloadedBytes: 0,
    totalBytes: null,
  };
  const reportProgress = (progress: UpdateDownloadProgress) => {
    latestProgress = progress;
    onProgress?.(progress);
  };
  const updatePath = await services.downloadUpdateFile({
    downloadUrl: update.downloadUrl,
    checksumUrl: update.checksumUrl,
    assetName: update.assetName,
    fileName: getUpdateFileName(update, os),
  }, reportProgress);
  if (os !== "darwin") return updatePath;
  reportProgress({ ...latestProgress, phase: "preparing", totalBytes: null });
  return services.extractAppZip(updatePath);
}

export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!isTauriRuntime() || isDevBuild()) return null;

  const { getVersion } = await import("@tauri-apps/api/app");
  const { invoke } = await import("@tauri-apps/api/core");
  const currentVersion = await getVersion();
  const release = await fetchJson<GitHubRelease>(`https://api.github.com/repos/${GITHUB_REPO}/releases/latest`);
  const latestVersion = normalizeVersion(release.tag_name || "");
  if (!latestVersion || compareVersions(latestVersion, currentVersion) <= 0) return null;

  const os = await invoke<string>("get_update_platform");
  const { asset, checksumAsset } = selectUpdateAssets(release.assets ?? [], getAssetConfigForPlatform(os));
  return {
    version: latestVersion,
    notes: release.body ?? null,
    publishedAt: release.published_at ?? null,
    assetName: asset.name,
    downloadUrl: getAssetDownloadUrl(asset),
    checksumUrl: getAssetDownloadUrl(checksumAsset),
  };
}

export async function downloadUpdate(
  update: UpdateInfo,
  onProgress?: (progress: UpdateDownloadProgress) => void,
): Promise<string> {
  if (!isTauriRuntime()) throw new Error("Updates require the desktop app.");

  const { Channel, invoke } = await import("@tauri-apps/api/core");
  return downloadUpdateWithServices(update, {
    getPlatform: () => invoke<string>("get_update_platform"),
    downloadUpdateFile: (request, reportProgress) => {
      const progress = new Channel<UpdateDownloadProgress>();
      progress.onmessage = (event) => reportProgress?.(event);
      return invoke<string>("download_update_file", { ...request, onProgress: progress });
    },
    extractAppZip: (zipPath) => invoke<string>("extract_app_zip", { zipPath }),
  }, onProgress);
}

export async function installUpdate(updatePath: string): Promise<void> {
  if (!isTauriRuntime()) throw new Error("Updates require the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("apply_update", { updatePath });
}
