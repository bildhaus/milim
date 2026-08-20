import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const PREVIEW_WEBVIEW_NAVIGATION_EVENT =
  "milim://preview-webview-navigation";
export const PREVIEW_WEBVIEW_NEW_TAB_EVENT =
  "milim://preview-webview-new-tab";
export const PREVIEW_WEBVIEW_SHORTCUT_EVENT =
  "milim://preview-webview-shortcut";
export const PREVIEW_WEBVIEW_TITLE_EVENT =
  "milim://preview-webview-title";
export const PREVIEW_OPEN_URL_EVENT = "milim://preview-open-url";

export type PreviewWebviewLoadState =
  | "navigated"
  | "loading"
  | "ready"
  | "error";

export interface PreviewWebviewNavigation {
  label: string;
  url: string;
  state: PreviewWebviewLoadState;
  message?: string;
}

export interface PreviewWebviewNewTab {
  requestId: number;
  openerLabel: string;
  url: string;
}

export interface PreviewWebviewShortcut {
  label: string;
  action: "new_tab" | "close_tab" | "zoom_in" | "zoom_out" | "zoom_reset";
}

export interface PreviewWebviewTitle {
  label: string;
  title: string;
}

export interface PreviewOpenUrl {
  url: string;
}

export type PreviewBrowserStorageMode = "persistent" | "private";

export interface PreviewWebviewBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function createPreviewWebview(
  label: string,
  url: string,
  bounds: PreviewWebviewBounds,
  storageMode: PreviewBrowserStorageMode,
  profileId: string,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_create", { label, url, bounds, storageMode, profileId });
}

export async function clearPreviewWebviewData(): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_clear_data");
}

export async function closePreviewWebview(label: string): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_close", { label });
}

export async function navigatePreviewWebview(
  label: string,
  url: string,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_navigate", { label, url });
}

export async function reloadPreviewWebview(label: string): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_reload", { label });
}

export async function movePreviewWebviewHistory(
  label: string,
  delta: -1 | 1,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_history", { label, delta });
}

export async function currentPreviewWebviewUrl(
  label: string,
): Promise<string | null> {
  if (!IS_TAURI) return null;
  return await invoke<string>("preview_webview_url", { label });
}

export async function listenForPreviewWebviewNavigation(
  handler: (navigation: PreviewWebviewNavigation) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) return () => undefined;
  return await listen<PreviewWebviewNavigation>(
    PREVIEW_WEBVIEW_NAVIGATION_EVENT,
    (event) => handler(event.payload),
  );
}

export async function listenForPreviewWebviewNewTab(
  handler: (request: PreviewWebviewNewTab) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) return () => undefined;
  return await listen<PreviewWebviewNewTab>(
    PREVIEW_WEBVIEW_NEW_TAB_EVENT,
    (event) => handler(event.payload),
  );
}

export async function listenForPreviewWebviewShortcut(
  handler: (shortcut: PreviewWebviewShortcut) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) return () => undefined;
  return await listen<PreviewWebviewShortcut>(
    PREVIEW_WEBVIEW_SHORTCUT_EVENT,
    (event) => handler(event.payload),
  );
}

export async function listenForPreviewWebviewTitle(
  handler: (title: PreviewWebviewTitle) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) return () => undefined;
  return await listen<PreviewWebviewTitle>(
    PREVIEW_WEBVIEW_TITLE_EVENT,
    (event) => handler(event.payload),
  );
}

export async function listenForPreviewOpenUrl(
  handler: (request: PreviewOpenUrl) => void,
): Promise<UnlistenFn> {
  if (!IS_TAURI) return () => undefined;
  return await listen<PreviewOpenUrl>(PREVIEW_OPEN_URL_EVENT, (event) =>
    handler(event.payload),
  );
}
