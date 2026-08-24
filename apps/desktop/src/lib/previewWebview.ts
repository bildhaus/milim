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
  claimToken: number;
  url: string;
  state: PreviewWebviewLoadState;
  message?: string;
}

export interface PreviewWebviewNewTab {
  requestId: number;
  openerLabel: string;
  claimToken: number;
  url: string;
}

export interface PreviewWebviewShortcut {
  label: string;
  claimToken: number;
  action: "new_tab" | "close_tab" | "zoom_in" | "zoom_out" | "zoom_reset";
}

export interface PreviewWebviewTitle {
  label: string;
  claimToken: number;
  title: string;
}

export interface PreviewOpenUrl {
  threadId: string;
  url: string;
}

export type PreviewBrowserStorageMode = "persistent" | "private";

export interface PreviewWebviewBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PreviewWebviewCreateResult {
  reused: boolean;
  url: string;
  claimToken: number;
  navigated: boolean;
}

const IS_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function createPreviewWebview(
  label: string,
  url: string,
  bounds: PreviewWebviewBounds,
  storageMode: PreviewBrowserStorageMode,
  profileId: string,
): Promise<PreviewWebviewCreateResult | null> {
  if (!IS_TAURI) return null;
  return await invoke<PreviewWebviewCreateResult>("preview_webview_create", { label, url, bounds, storageMode, profileId });
}

export async function clearPreviewWebviewData(): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_clear_data");
}

export async function setPreviewWebviewVisibility(
  label: string,
  claimToken: number,
  visible: boolean,
  muted: boolean,
): Promise<boolean> {
  if (!IS_TAURI) return false;
  return await invoke<boolean>("preview_webview_set_visibility", { label, claimToken, visible, muted });
}

export async function navigatePreviewWebview(
  label: string,
  claimToken: number,
  url: string,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_navigate", { label, claimToken, url });
}

export async function reloadPreviewWebview(label: string, claimToken: number): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_reload", { label, claimToken });
}

export async function setPreviewWebviewMuted(
  label: string,
  claimToken: number,
  muted: boolean,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_set_muted", { label, claimToken, muted });
}

export async function setPreviewWebviewZoom(
  label: string,
  claimToken: number,
  scaleFactor: number,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_set_zoom", { label, claimToken, scaleFactor });
}

export async function setPreviewWebviewBounds(
  label: string,
  claimToken: number,
  bounds: PreviewWebviewBounds,
): Promise<void> {
  if (!IS_TAURI) return;
  await invoke("preview_webview_set_bounds", { label, claimToken, bounds });
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
