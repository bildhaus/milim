import {
  sessionProjectFolder,
  type Project,
  type Session,
} from "../sessions/store.js";
import { pendingAttentionKey } from "../ui/sounds.js";

export type NativeNotificationKind = "finished" | "attention";

type NativeBadgeState = {
  sessions: readonly Pick<
    Session,
    "id" | "messages" | "archivedAt" | "settings" | "retryWorkspace" | "threadWorkspace"
  >[];
  projects: readonly Pick<Project, "folder" | "archivedAt">[];
  unreadSessionIds: readonly string[];
  workerRuns: readonly {
    run: { id: string; parent_thread_id: string; status: string };
  }[];
};

type NativeBadgeWriter = (count: number) => Promise<void>;

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
  const archivedProjectFolders = new Set(
    state.projects
      .filter((project) => project.archivedAt)
      .map((project) => project.folder),
  );
  const visibleSessions = state.sessions.filter(
    (session) =>
      !session.archivedAt &&
      !archivedProjectFolders.has(sessionProjectFolder(session)),
  );
  const visibleSessionIds = new Set(visibleSessions.map((session) => session.id));
  const counted = new Set(
    state.unreadSessionIds.filter((id) => visibleSessionIds.has(id)),
  );
  const proposedWorkers = new Map<string, string>();
  for (const { run } of state.workerRuns) {
    if (run.status === "proposed" && !proposedWorkers.has(run.parent_thread_id))
      proposedWorkers.set(run.parent_thread_id, run.id);
  }
  for (const session of visibleSessions) {
    if (pendingAttentionKey(session.messages, proposedWorkers.get(session.id)))
      counted.add(session.id);
  }
  return counted.size;
}

export function createNativeBadgeUpdater(write: NativeBadgeWriter): NativeBadgeWriter {
  let latestUpdateId = 0;
  let writeQueue = Promise.resolve();
  return (count) => {
    const updateId = ++latestUpdateId;
    const normalizedCount = Math.max(0, Math.floor(count));
    const update = writeQueue
      .catch(() => {})
      .then(() => updateId === latestUpdateId ? write(normalizedCount) : undefined);
    writeQueue = update;
    return update;
  };
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

async function writeMilimUnreadBadge(normalizedCount: number): Promise<void> {
  const label = unreadBadgeLabel(normalizedCount);
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const appWindow = getCurrentWindow();

  if (navigator.userAgent.includes("Windows")) {
    if (!label) {
      await appWindow.setOverlayIcon();
      return;
    }
    const { Image } = await import("@tauri-apps/api/image");
    const icon = await Image.new(unreadBadgeRgba(label), 32, 32);
    try {
      await appWindow.setOverlayIcon(icon);
    } finally {
      await icon.close();
    }
  } else if (navigator.userAgent.includes("Mac")) {
    await appWindow.setBadgeCount(normalizedCount || undefined);
  }
}

const updateMilimUnreadBadge = createNativeBadgeUpdater(writeMilimUnreadBadge);

export function setMilimUnreadBadge(count: number): Promise<void> {
  return isTauriRuntime() ? updateMilimUnreadBadge(count) : Promise.resolve();
}
