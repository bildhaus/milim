export type NativeNotificationKind = "finished" | "attention";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function ensureNativeNotificationPermission(): Promise<boolean> {
  if (!isTauriRuntime()) return false;
  const { isPermissionGranted, requestPermission } = await import("@tauri-apps/plugin-notification");
  if (await isPermissionGranted()) return true;
  return (await requestPermission()) === "granted";
}

export async function sendMilimNotification(
  kind: NativeNotificationKind,
  options: { threadTitle?: string; includeThreadTitle: boolean; onlyWhenUnfocused: boolean },
): Promise<void> {
  if (!isTauriRuntime()) return;
  if (options.onlyWhenUnfocused && document.hasFocus()) return;
  const { isPermissionGranted, sendNotification } = await import("@tauri-apps/plugin-notification");
  if (!(await isPermissionGranted())) return;
  const generic = kind === "finished" ? "A run finished." : "A chat needs your attention.";
  const titled = kind === "finished"
    ? `${options.threadTitle || "A chat"} finished.`
    : `${options.threadTitle || "A chat"} needs your attention.`;
  sendNotification({ title: "Milim", body: options.includeThreadTitle ? titled : generic });
}
