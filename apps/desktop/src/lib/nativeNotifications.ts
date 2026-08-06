import type { ChatMessage } from "../api.js";
import { pendingAttentionKey } from "../ui/sounds.js";

export type NativeNotificationKind = "finished" | "attention";

type NativeBadgeState = {
  sessions: readonly { id: string; messages: ChatMessage[] }[];
  unreadSessionIds: readonly string[];
  workerRuns: readonly {
    run: { id: string; parent_thread_id: string; status: string };
  }[];
};

let badgeUpdateId = 0;

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

export function nativeBadgeThreadCount(state: NativeBadgeState): number {
  const counted = new Set(state.unreadSessionIds);
  const proposedWorkers = new Map<string, string>();
  for (const { run } of state.workerRuns) {
    if (run.status === "proposed" && !proposedWorkers.has(run.parent_thread_id))
      proposedWorkers.set(run.parent_thread_id, run.id);
  }
  for (const session of state.sessions) {
    if (pendingAttentionKey(session.messages, proposedWorkers.get(session.id)))
      counted.add(session.id);
  }
  return counted.size;
}

export function unreadBadgeLabel(count: number): string | null {
  if (count < 1) return null;
  return count > 9 ? "9+" : String(Math.floor(count));
}

function unreadBadgeRgba(label: string): Uint8Array {
  const canvas = document.createElement("canvas");
  canvas.width = 32;
  canvas.height = 32;
  const context = canvas.getContext("2d");
  if (!context) throw new Error("Canvas is unavailable.");
  context.beginPath();
  context.arc(16, 16, 15, 0, Math.PI * 2);
  context.fillStyle = "#202124";
  context.fill();
  context.lineWidth = 2;
  context.strokeStyle = "#ffffff";
  context.stroke();
  context.fillStyle = "#ffffff";
  context.font = `700 ${label.length > 1 ? 15 : 21}px "Segoe UI", sans-serif`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillText(label, 16, 17);
  return new Uint8Array(context.getImageData(0, 0, 32, 32).data.buffer);
}

export async function setMilimUnreadBadge(count: number): Promise<void> {
  if (!isTauriRuntime()) return;
  const updateId = ++badgeUpdateId;
  const normalizedCount = Math.max(0, Math.floor(count));
  const label = unreadBadgeLabel(normalizedCount);
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  if (updateId !== badgeUpdateId) return;
  const appWindow = getCurrentWindow();

  if (navigator.userAgent.includes("Windows")) {
    if (!label) {
      await appWindow.setOverlayIcon();
      return;
    }
    const { Image } = await import("@tauri-apps/api/image");
    const icon = await Image.new(unreadBadgeRgba(label), 32, 32);
    try {
      if (updateId === badgeUpdateId) await appWindow.setOverlayIcon(icon);
    } finally {
      await icon.close();
    }
  } else if (navigator.userAgent.includes("Mac")) {
    await appWindow.setBadgeCount(normalizedCount || undefined);
  }
}
