/**
 * Size registry for every resizable surface. Each pane stores one number (its
 * width, or its height for `axis: "y"`) in the persisted UI store. Entries are
 * sparse: a missing entry means "use the default", so resetting a pane deletes
 * its entry and future default changes still reach users who never resized it.
 *
 * The saved value is a preference. Surfaces render `min(preference, space)`
 * against their live container, so a small window never overwrites it.
 */

export type PaneAxis = "x" | "y";

export interface PaneSpec {
  /** Lowercase noun used in "Resize {label}". */
  label: string;
  axis: PaneAxis;
  min: number;
  max: number;
  /** `null` lets CSS size the surface until the user resizes it (sheets). */
  default: number | null;
  /** Dragging this far past `min` hides the pane; reversing the drag restores it. */
  collapseOvershoot?: number;
}

export const PANE_COLLAPSE_OVERSHOOT = 96;
export const PANE_KEYBOARD_STEP = 16;
export const PANE_KEYBOARD_LARGE_STEP = 64;
/** Matches the open/close transition, so a reopened pane animates before live resizing resumes. */
export const PANE_SNAP_ANIMATION_MS = 180;

export const SHEET_IDS = [
  "agents",
  "skills",
  "schedules",
  "providers",
  "memory",
  "mcp",
  "usage",
  "media",
  "pullRequests",
] as const;
export type SheetId = (typeof SHEET_IDS)[number];

/** Detail column kept beside a manager's resizable list rail. */
export const MANAGER_DETAIL_MIN_WIDTH = 320;

export const MIN_SHEET_WIDTH = 560;
export const MIN_SHEET_HEIGHT = 480;
const MAX_SHEET_WIDTH = 2400;
const MAX_SHEET_HEIGHT = 1600;
/** Viewport margin kept around a resized sheet. */
export const SHEET_VIEWPORT_MARGIN = 24;

const SHEET_LABELS: Record<SheetId, string> = {
  agents: "agents manager",
  skills: "skills manager",
  schedules: "schedules manager",
  providers: "providers manager",
  memory: "memory manager",
  mcp: "MCP servers manager",
  usage: "usage manager",
  media: "media studio",
  pullRequests: "pull requests panel",
};

const PANE_SPECS = {
  sidebar: { label: "thread sidebar", axis: "x", min: 220, max: 420, default: 248, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  inspector: { label: "side panel", axis: "x", min: 360, max: 4096, default: 420, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  context: { label: "context panel", axis: "x", min: 260, max: 560, default: 300, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  gitDiffNavigator: { label: "changed files", axis: "x", min: 160, max: 480, default: 210, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  codeRail: { label: "file rail", axis: "x", min: 148, max: 360, default: 188, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  previewLogs: { label: "logs", axis: "y", min: 48, max: 360, default: 142, collapseOvershoot: 48 },
  workersHistory: { label: "worker history", axis: "x", min: 180, max: 480, default: 240 },
  googleSlidesRail: { label: "slide thumbnails", axis: "x", min: 120, max: 360, default: 168, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  settingsNav: { label: "settings navigation", axis: "x", min: 200, max: 360, default: 232 },
  mediaComposer: { label: "media composer", axis: "x", min: 220, max: 420, default: 300, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  mediaLibrary: { label: "local library", axis: "x", min: 220, max: 420, default: 280, collapseOvershoot: PANE_COLLAPSE_OVERSHOOT },
  pullRequestsList: { label: "pull request list", axis: "x", min: 240, max: 1600, default: 520 },
  agentsRail: { label: "agent list", axis: "x", min: 220, max: 480, default: 306 },
  skillsRail: { label: "skill list", axis: "x", min: 220, max: 480, default: 292 },
  schedulesList: { label: "schedule list", axis: "x", min: 220, max: 480, default: 306 },
  providersRail: { label: "provider list", axis: "x", min: 200, max: 440, default: 264 },
  mcpRail: { label: "server list", axis: "x", min: 220, max: 480, default: 306 },
  memoryDetail: { label: "memory detail", axis: "x", min: 320, max: 720, default: 420 },
  ...sheetSpecs(),
} satisfies Record<string, PaneSpec>;

export type SheetPaneId = `${SheetId}Sheet.width` | `${SheetId}Sheet.height`;
export type PaneId = Exclude<keyof typeof PANE_SPECS, SheetPaneId> | SheetPaneId;
export type PaneSizes = Partial<Record<PaneId, number>>;

export const PANES: Readonly<Record<PaneId, PaneSpec>> = PANE_SPECS;
export const PANE_IDS = Object.keys(PANES) as PaneId[];

function sheetSpecs(): Record<SheetPaneId, PaneSpec> {
  const specs = {} as Record<SheetPaneId, PaneSpec>;
  for (const id of SHEET_IDS) {
    specs[`${id}Sheet.width`] = { label: `${SHEET_LABELS[id]} width`, axis: "x", min: MIN_SHEET_WIDTH, max: MAX_SHEET_WIDTH, default: null };
    specs[`${id}Sheet.height`] = { label: `${SHEET_LABELS[id]} height`, axis: "y", min: MIN_SHEET_HEIGHT, max: MAX_SHEET_HEIGHT, default: null };
  }
  return specs;
}

export function sheetPaneIds(id: SheetId): { width: SheetPaneId; height: SheetPaneId } {
  return { width: `${id}Sheet.width`, height: `${id}Sheet.height` };
}

export function sheetLabel(id: SheetId): string {
  return SHEET_LABELS[id];
}

export function isPaneId(value: string): value is PaneId {
  return Object.prototype.hasOwnProperty.call(PANES, value);
}

/** Clamps to `[min, max]`; when the live space is below `min`, `min` wins over `max`. */
export function clampPaneSize(value: number, min: number, max: number): number {
  return Math.round(Math.max(min, Math.min(value, max)));
}

/** Normalizes a stored preference against the static spec bounds. */
export function normalizePaneSize(id: PaneId, value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  const spec = PANES[id];
  return clampPaneSize(value, spec.min, spec.max);
}

export function paneDefaultSize(id: PaneId): number | null {
  return PANES[id].default;
}

/** Returns the stored preference, or the default when nothing was saved. */
export function paneSize(sizes: PaneSizes, id: PaneId): number | null {
  return sizes[id] ?? PANES[id].default;
}

/** Stores `value` sparsely: saving the default removes the entry. */
export function withPaneSize(sizes: PaneSizes, id: PaneId, value: number | null): PaneSizes {
  const next = { ...sizes };
  const normalized = value == null ? undefined : normalizePaneSize(id, value);
  if (normalized === undefined || normalized === PANES[id].default) delete next[id];
  else next[id] = normalized;
  return next;
}

export function normalizePaneSizes(value: unknown): PaneSizes {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  let sizes: PaneSizes = {};
  for (const [id, size] of Object.entries(value)) {
    if (isPaneId(id)) sizes = withPaneSize(sizes, id, typeof size === "number" ? size : null);
  }
  return sizes;
}

/**
 * Top-level keys that stored pane sizes before the registry existed, with the
 * default those builds always wrote. A legacy default is not a user choice, so
 * it is not carried over (sheets then stay CSS-sized, which renders the same).
 */
export const LEGACY_PANE_SIZE_KEYS = {
  sidebarWidth: ["sidebar", 248],
  previewPanelWidth: ["inspector", 420],
  mediaStudioWidth: ["mediaSheet.width", 1120],
  mediaStudioHeight: ["mediaSheet.height", 820],
  mediaComposerWidth: ["mediaComposer", 300],
  mediaLibraryWidth: ["mediaLibrary", 280],
  pullRequestsWidth: ["pullRequestsSheet.width", 1120],
  pullRequestsHeight: ["pullRequestsSheet.height", 820],
  pullRequestsListWidth: ["pullRequestsList", 520],
} as const satisfies Record<string, readonly [PaneId, number]>;

/**
 * Builds the registry from persisted state. Registry entries win; legacy keys
 * only fill panes the registry has not saved yet, so a pre-registry size is
 * carried over once and never lost.
 */
export function migratePaneSizes(saved: Record<string, unknown> | undefined): PaneSizes {
  let sizes: PaneSizes = {};
  for (const [legacyKey, [id, legacyDefault]] of Object.entries(LEGACY_PANE_SIZE_KEYS)) {
    const value = saved?.[legacyKey];
    if (typeof value === "number" && Math.round(value) !== legacyDefault) sizes = withPaneSize(sizes, id, value);
  }
  return { ...sizes, ...normalizePaneSizes(saved?.paneSizes) };
}

export interface PaneBounds {
  min: number;
  max: number;
}

/** Live bounds: spec bounds narrowed by the caller's container, never inverted. */
export function paneBounds(id: PaneId, live: Partial<PaneBounds> = {}): PaneBounds {
  const spec = PANES[id];
  const min = Math.max(spec.min, live.min ?? spec.min);
  const max = Math.max(min, Math.min(spec.max, live.max ?? spec.max));
  return { min, max };
}

export type PaneDragStep =
  | { kind: "resize"; size: number }
  | { kind: "collapse" };

/**
 * Resolves one drag frame. `raw` is the unclamped size the pointer asks for.
 * Collapsible panes snap closed once `raw` passes `min` by the overshoot.
 */
export function paneDragStep(id: PaneId, raw: number, bounds: PaneBounds, collapsible: boolean): PaneDragStep {
  const overshoot = PANES[id].collapseOvershoot;
  if (collapsible && overshoot !== undefined && raw < bounds.min - overshoot) return { kind: "collapse" };
  return { kind: "resize", size: clampPaneSize(raw, bounds.min, bounds.max) };
}

export type PaneKeyboardTarget = number | "reset" | null;

/**
 * Shared keyboard contract: Arrow keys move 16px (64px with Shift) along the
 * pane's axis, Home/End jump to the live min/max, and Enter resets.
 * `direction` is 1 when moving right/down grows the pane and -1 otherwise.
 */
export function paneKeyboardTarget(
  key: string,
  shiftKey: boolean,
  current: number,
  axis: PaneAxis,
  direction: 1 | -1,
  bounds: PaneBounds,
): PaneKeyboardTarget {
  const step = shiftKey ? PANE_KEYBOARD_LARGE_STEP : PANE_KEYBOARD_STEP;
  const grow = axis === "x" ? "ArrowRight" : "ArrowDown";
  const shrink = axis === "x" ? "ArrowLeft" : "ArrowUp";
  if (key === grow) return clampPaneSize(current + step * direction, bounds.min, bounds.max);
  if (key === shrink) return clampPaneSize(current - step * direction, bounds.min, bounds.max);
  if (key === "Home") return bounds.min;
  if (key === "End") return bounds.max;
  if (key === "Enter") return "reset";
  return null;
}
