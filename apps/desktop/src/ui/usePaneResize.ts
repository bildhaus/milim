import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import {
  PANE_KEYBOARD_LARGE_STEP,
  PANE_KEYBOARD_STEP,
  PANE_SNAP_ANIMATION_MS,
  PANES,
  SHEET_VIEWPORT_MARGIN,
  clampPaneSize,
  paneBounds,
  paneDragStep,
  paneKeyboardTarget,
  sheetLabel,
  sheetPaneIds,
  type PaneAxis,
  type PaneBounds,
  type PaneId,
  type SheetId,
} from "../lib/paneSizes";
import { useUiPreferences } from "./store";

export const PANE_RESIZE_TITLE = "Drag to resize · Double-click to reset";

type DragCursor = "col-resize" | "row-resize" | "ew-resize" | "ns-resize" | "nwse-resize" | "nesw-resize";

/**
 * Shared pointer plumbing for every resize drag: pointer capture, one update
 * per animation frame, and a body class that pins the cursor, blocks text
 * selection, and stops iframes from swallowing the pointer (foundation.css).
 * Listeners live on window, so a handle that unmounts mid-drag (a pane that
 * snaps closed) keeps the drag alive. Returns an idempotent cleanup.
 */
export function beginPointerDrag(
  event: ReactPointerEvent<HTMLElement>,
  cursor: DragCursor,
  handlers: { move: (dx: number, dy: number) => void; end: () => void },
): () => void {
  const target = event.currentTarget;
  const { pointerId, clientX: originX, clientY: originY } = event;
  const body = document.body;
  let pending: { x: number; y: number } | null = null;
  let frame = 0;
  let active = true;

  const flush = () => {
    frame = 0;
    if (!pending) return;
    const point = pending;
    pending = null;
    handlers.move(point.x - originX, point.y - originY);
  };
  const onMove = (moveEvent: PointerEvent) => {
    if (moveEvent.pointerId !== pointerId) return;
    pending = { x: moveEvent.clientX, y: moveEvent.clientY };
    if (!frame) frame = window.requestAnimationFrame(flush);
  };
  const onEnd = (endEvent: PointerEvent) => {
    if (endEvent.pointerId !== pointerId) return;
    if (frame) window.cancelAnimationFrame(frame);
    flush();
    cleanup();
    handlers.end();
  };
  function cleanup() {
    if (!active) return;
    active = false;
    if (frame) window.cancelAnimationFrame(frame);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onEnd);
    window.removeEventListener("pointercancel", onEnd);
    body.classList.remove("pane-resizing");
    body.style.removeProperty("--pane-resize-cursor");
    try {
      if (target.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
    } catch {
      // The handle may already be detached (collapsed pane).
    }
  }

  event.preventDefault();
  event.stopPropagation();
  try {
    target.setPointerCapture(pointerId);
  } catch {
    // Synthetic pointers cannot be captured; window listeners still track them.
  }
  body.classList.add("pane-resizing");
  body.style.setProperty("--pane-resize-cursor", cursor);
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onEnd);
  window.addEventListener("pointercancel", onEnd);
  return cleanup;
}

type Bound = number | (() => number | undefined);

function readBound(bound: Bound | undefined): number | undefined {
  const value = typeof bound === "function" ? bound() : bound;
  return value !== undefined && Number.isFinite(value) ? value : undefined;
}

/**
 * Live max for a pane inside `ref`: the container size minus the space its
 * sibling needs. Unmeasured containers fall back to the spec max. Pair it with
 * a CSS `min(var(--pane), calc(100% - reserve))` so the render clamp stays live.
 */
export function containerMax(
  ref: RefObject<HTMLElement | null>,
  reserve: number,
  axis: PaneAxis = "x",
): () => number | undefined {
  return () => {
    const element = ref.current;
    if (!element) return undefined;
    const size = axis === "x" ? element.clientWidth : element.clientHeight;
    return size > 0 ? size - reserve : undefined;
  };
}

export interface PaneResizeOptions {
  /** 1 when dragging right/down grows the pane, -1 when dragging left/up does. */
  direction?: 1 | -1;
  /** Live limits from the pane's container; functions are re-read every frame. */
  min?: Bound;
  max?: Bound;
  /** Receives `cssVar` during a drag so the owner does not re-render per frame. */
  targetRef?: RefObject<HTMLElement | null>;
  cssVar?: string;
  /** Rendered size when CSS can clamp the pane below its saved size; drags start from it. */
  measure?: () => number | undefined;
  /** Collapsible panes hide past `min - collapseOvershoot` and return when the drag reverses. */
  onCollapse?: () => void;
  onExpand?: () => void;
  onDragStart?: (size: number) => void;
  /** Called every frame with the unclamped size, before the clamped size is applied. */
  onDrag?: (raw: number) => void;
  onLiveSize?: (size: number) => void;
  onDragEnd?: (size: number, collapsed: boolean) => void;
  valueText?: (size: number) => string;
  controls?: string;
  label?: string;
  disabled?: boolean;
}

export interface PaneResizeHandleProps {
  ref: (element: HTMLDivElement | null) => void;
  "aria-label": string;
  "aria-controls"?: string;
  "aria-valuemin": number;
  "aria-valuemax": number;
  "aria-valuenow": number;
  "aria-valuetext"?: string;
  title: string;
  tabIndex: number;
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void;
  onDoubleClick: () => void;
}

export interface PaneResizeController {
  id: PaneId;
  axis: PaneAxis;
  /** Rendered size: the saved preference (or default) clamped to live bounds; the live value mid-drag. */
  size: number;
  /** The saved preference, or the default when nothing is saved. */
  preference: number;
  bounds: PaneBounds;
  dragging: boolean;
  /** Clamps to live bounds and saves. */
  resize: (size: number) => void;
  reset: () => void;
  /** Re-applies the in-flight drag after its container changed size under a still pointer. */
  reapply: () => void;
  handleProps: PaneResizeHandleProps;
}

type PaneDrag = {
  start: number;
  raw: number;
  size: number;
  collapsed: boolean;
  resumeTimer: number | null;
};

/**
 * One resize behaviour for every pane: pointer drag, keyboard (Arrow ±16px,
 * Shift+Arrow ±64px, Home/End, Enter), double-click reset, optional
 * snap-collapse, and one persisted write when the drag ends.
 */
export function usePaneResize(id: PaneId, options: PaneResizeOptions = {}): PaneResizeController {
  const spec = PANES[id];
  const preference = useUiPreferences((state) => state.paneSizes[id]);
  const setPaneSize = useUiPreferences((state) => state.setPaneSize);
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const dragRef = useRef<PaneDrag | null>(null);
  const handleRef = useRef<HTMLDivElement | null>(null);
  const cleanupRef = useRef<(() => void) | null>(null);
  const [dragging, setDragging] = useState(false);

  const liveBounds = useCallback(
    () => paneBounds(id, { min: readBound(optionsRef.current.min), max: readBound(optionsRef.current.max) }),
    [id],
  );
  const bounds = liveBounds();
  const saved = preference ?? spec.default ?? spec.min;
  const size = dragRef.current?.size ?? clampPaneSize(saved, bounds.min, bounds.max);
  const sizeRef = useRef(size);
  sizeRef.current = size;

  useEffect(() => () => {
    const drag = dragRef.current;
    if (drag?.resumeTimer != null) window.clearTimeout(drag.resumeTimer);
    cleanupRef.current?.();
  }, []);

  const applyLive = useCallback((next: number) => {
    const current = optionsRef.current;
    if (current.cssVar) current.targetRef?.current?.style.setProperty(current.cssVar, `${next}px`);
    const handle = handleRef.current;
    if (handle) {
      const { min, max } = liveBounds();
      handle.setAttribute("aria-valuemin", String(min));
      handle.setAttribute("aria-valuemax", String(max));
      handle.setAttribute("aria-valuenow", String(next));
      if (current.valueText) handle.setAttribute("aria-valuetext", current.valueText(next));
    }
    current.onLiveSize?.(next);
  }, [liveBounds]);

  const resize = useCallback((next: number) => {
    const { min, max } = liveBounds();
    setPaneSize(id, clampPaneSize(next, min, max));
  }, [id, liveBounds, setPaneSize]);

  const reset = useCallback(() => setPaneSize(id, null), [id, setPaneSize]);

  const currentSize = useCallback(() => {
    const measured = optionsRef.current.measure?.();
    if (measured === undefined || !Number.isFinite(measured)) return sizeRef.current;
    const { min, max } = liveBounds();
    return clampPaneSize(measured, min, max);
  }, [liveBounds]);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const current = optionsRef.current;
    if (event.button !== 0 || current.disabled) return;
    cleanupRef.current?.();
    const start = currentSize();
    const drag: PaneDrag = { start, raw: start, size: start, collapsed: false, resumeTimer: null };
    const direction = current.direction ?? 1;
    const collapsible = Boolean(current.onCollapse);
    dragRef.current = drag;
    setDragging(true);
    current.onDragStart?.(drag.start);
    cleanupRef.current = beginPointerDrag(event, spec.axis === "x" ? "col-resize" : "row-resize", {
      move: (dx, dy) => {
        const live = optionsRef.current;
        const raw = drag.start + (spec.axis === "x" ? dx : dy) * direction;
        drag.raw = raw;
        if (paneDragStep(id, raw, liveBounds(), collapsible).kind === "collapse") {
          if (!drag.collapsed) {
            drag.collapsed = true;
            if (drag.resumeTimer != null) window.clearTimeout(drag.resumeTimer);
            drag.resumeTimer = null;
            // Let the pane animate closed instead of tracking the pointer.
            setDragging(false);
            live.onCollapse?.();
          }
          return;
        }
        if (drag.collapsed) {
          drag.collapsed = false;
          live.onExpand?.();
          drag.resumeTimer = window.setTimeout(() => {
            drag.resumeTimer = null;
            if (dragRef.current === drag && !drag.collapsed) setDragging(true);
          }, PANE_SNAP_ANIMATION_MS);
        }
        live.onDrag?.(raw);
        // Read the bounds after onDrag, which may change them (inspector overlay staging).
        const { min, max } = liveBounds();
        drag.size = clampPaneSize(raw, min, max);
        applyLive(drag.size);
      },
      end: () => {
        cleanupRef.current = null;
        if (drag.resumeTimer != null) window.clearTimeout(drag.resumeTimer);
        dragRef.current = null;
        setDragging(false);
        // A collapsed pane keeps its previous size for when it reopens.
        if (!drag.collapsed && drag.size !== drag.start) setPaneSize(id, drag.size);
        else applyLive(drag.start);
        optionsRef.current.onDragEnd?.(drag.size, drag.collapsed);
      },
    });
  }, [applyLive, currentSize, id, liveBounds, setPaneSize, spec.axis]);

  const reapply = useCallback(() => {
    const drag = dragRef.current;
    if (!drag || drag.collapsed) return;
    const { min, max } = liveBounds();
    drag.size = clampPaneSize(drag.raw, min, max);
    applyLive(drag.size);
  }, [applyLive, liveBounds]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if (optionsRef.current.disabled) return;
    const target = paneKeyboardTarget(
      event.key,
      event.shiftKey,
      currentSize(),
      spec.axis,
      optionsRef.current.direction ?? 1,
      liveBounds(),
    );
    if (target === null) return;
    event.preventDefault();
    if (target === "reset") reset();
    else resize(target);
  }, [currentSize, liveBounds, reset, resize, spec.axis]);

  const setHandle = useCallback((element: HTMLDivElement | null) => {
    handleRef.current = element;
  }, []);

  const defaultLabel = `Resize ${spec.label}`;
  const label = options.label ?? (options.onCollapse ? `${defaultLabel}; drag past the minimum to hide it` : defaultLabel);
  return {
    id,
    axis: spec.axis,
    size,
    preference: saved,
    bounds,
    dragging,
    resize,
    reset,
    reapply,
    handleProps: {
      ref: setHandle,
      "aria-label": label,
      "aria-controls": options.controls,
      "aria-valuemin": bounds.min,
      "aria-valuemax": bounds.max,
      "aria-valuenow": size,
      "aria-valuetext": options.valueText?.(size),
      title: PANE_RESIZE_TITLE,
      tabIndex: options.disabled ? -1 : 0,
      onPointerDown,
      onKeyDown,
      onDoubleClick: reset,
    },
  };
}

/** Keyboard step for callers that extend the shared keyboard contract. */
export function paneKeyboardStep(event: { shiftKey: boolean }): number {
  return event.shiftKey ? PANE_KEYBOARD_LARGE_STEP : PANE_KEYBOARD_STEP;
}

export type SheetEdge = "n" | "e" | "s" | "w" | "ne" | "nw" | "se" | "sw";

export const SHEET_EDGES: readonly SheetEdge[] = ["n", "e", "s", "w", "ne", "nw", "se", "sw"];

const SHEET_EDGE_CURSORS: Record<SheetEdge, DragCursor> = {
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  nw: "nwse-resize",
  se: "nwse-resize",
};

type SheetSize = { width: number; height: number };

function sheetAxisBounds(id: PaneId, viewport: number): PaneBounds {
  const spec = PANES[id];
  const max = Math.max(0, Math.min(spec.max, viewport - SHEET_VIEWPORT_MARGIN));
  return { min: Math.min(spec.min, max), max };
}

function sheetBounds(id: SheetId): { width: PaneBounds; height: PaneBounds } {
  const ids = sheetPaneIds(id);
  return {
    width: sheetAxisBounds(ids.width, window.innerWidth),
    height: sheetAxisBounds(ids.height, window.innerHeight),
  };
}

export interface SheetResizeController {
  label: string;
  /** Inline size for the sheet; axes without a saved size stay CSS-driven. */
  style: CSSProperties | undefined;
  dragging: boolean;
  startDrag: (edge: SheetEdge, event: ReactPointerEvent<HTMLElement>, sheet: HTMLElement | null) => void;
  onKeyDown: (event: KeyboardEvent<HTMLElement>, sheet: HTMLElement | null) => void;
  reset: (axis?: "width" | "height") => void;
}

/**
 * Resizable manager sheets. Sheets are centered, so an edge moves by half the
 * size change and each pointer pixel adds two. The live size is written to the
 * sheet element while dragging and saved once on release; the viewport clamp
 * lives in CSS, so a small window never overwrites the saved size.
 */
export function useResizableSheet(id: SheetId | null): SheetResizeController | null {
  const ids = id ? sheetPaneIds(id) : null;
  const width = useUiPreferences((state) => (ids ? state.paneSizes[ids.width] : undefined));
  const height = useUiPreferences((state) => (ids ? state.paneSizes[ids.height] : undefined));
  const setPaneSizes = useUiPreferences((state) => state.setPaneSizes);
  const [dragging, setDragging] = useState(false);
  const cleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => () => cleanupRef.current?.(), []);

  const save = useCallback((size: Partial<SheetSize>) => {
    if (!ids) return;
    setPaneSizes({
      ...(size.width !== undefined ? { [ids.width]: size.width } : {}),
      ...(size.height !== undefined ? { [ids.height]: size.height } : {}),
    });
  }, [ids?.width, ids?.height, setPaneSizes]);

  const reset = useCallback((axis?: "width" | "height") => {
    if (!ids) return;
    setPaneSizes({
      ...(axis !== "height" ? { [ids.width]: null } : {}),
      ...(axis !== "width" ? { [ids.height]: null } : {}),
    });
  }, [ids?.width, ids?.height, setPaneSizes]);

  const startDrag = useCallback((edge: SheetEdge, event: ReactPointerEvent<HTMLElement>, sheet: HTMLElement | null) => {
    if (!id || !sheet || event.button !== 0) return;
    cleanupRef.current?.();
    const rect = sheet.getBoundingClientRect();
    const start: SheetSize = { width: Math.round(rect.width), height: Math.round(rect.height) };
    const latest = { ...start };
    const inline = { width: sheet.style.width, height: sheet.style.height };
    const sx = edge.includes("e") ? 1 : edge.includes("w") ? -1 : 0;
    const sy = edge.includes("s") ? 1 : edge.includes("n") ? -1 : 0;
    setDragging(true);
    cleanupRef.current = beginPointerDrag(event, SHEET_EDGE_CURSORS[edge], {
      move: (dx, dy) => {
        const bounds = sheetBounds(id);
        if (sx) {
          latest.width = clampPaneSize(start.width + sx * dx * 2, bounds.width.min, bounds.width.max);
          sheet.style.width = `${latest.width}px`;
        }
        if (sy) {
          latest.height = clampPaneSize(start.height + sy * dy * 2, bounds.height.min, bounds.height.max);
          sheet.style.height = `${latest.height}px`;
        }
      },
      end: () => {
        cleanupRef.current = null;
        setDragging(false);
        const changed = {
          width: sx && latest.width !== start.width ? latest.width : undefined,
          height: sy && latest.height !== start.height ? latest.height : undefined,
        };
        // Unchanged axes return to their rendered (possibly CSS-driven) size.
        if (changed.width === undefined) sheet.style.width = inline.width;
        if (changed.height === undefined) sheet.style.height = inline.height;
        save(changed);
      },
    });
  }, [id, save]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLElement>, sheet: HTMLElement | null) => {
    if (!id || !sheet) return;
    const rect = sheet.getBoundingClientRect();
    const current: SheetSize = { width: Math.round(rect.width), height: Math.round(rect.height) };
    const bounds = sheetBounds(id);
    let next: Partial<SheetSize> | "reset" | null = null;
    if (event.key === "Enter") next = "reset";
    else if (event.key === "Home") next = { width: bounds.width.min, height: bounds.height.min };
    else if (event.key === "End") next = { width: bounds.width.max, height: bounds.height.max };
    else {
      const byWidth = paneKeyboardTarget(event.key, event.shiftKey, current.width, "x", 1, bounds.width);
      const byHeight = paneKeyboardTarget(event.key, event.shiftKey, current.height, "y", 1, bounds.height);
      if (typeof byWidth === "number") next = { width: byWidth };
      else if (typeof byHeight === "number") next = { height: byHeight };
    }
    if (!next) return;
    event.preventDefault();
    event.stopPropagation();
    if (next === "reset") reset();
    else save(next);
  }, [id, reset, save]);

  if (!id) return null;
  const style: CSSProperties = {};
  if (width !== undefined) style.width = width;
  if (height !== undefined) style.height = height;
  return {
    label: sheetLabel(id),
    style: width !== undefined || height !== undefined ? style : undefined,
    dragging,
    startDrag,
    onKeyDown,
    reset,
  };
}

/**
 * Two-column split whose rail is a persisted pane: returns the container ref
 * and style (the rail width custom property) plus the resize controller.
 * `reserve` is the space the other column keeps.
 */
export function useSplitPane<T extends HTMLElement = HTMLDivElement>(
  id: PaneId,
  cssVar: string,
  reserve: number,
  options: PaneResizeOptions = {},
): { containerRef: RefObject<T>; style: CSSProperties; resize: PaneResizeController } {
  const containerRef = useRef<T>(null);
  const resize = usePaneResize(id, {
    max: containerMax(containerRef, reserve),
    targetRef: containerRef,
    cssVar,
    ...options,
  });
  return { containerRef, style: { [cssVar]: `${resize.size}px` } as CSSProperties, resize };
}
