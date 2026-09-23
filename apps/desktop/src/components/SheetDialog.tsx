import { useEffect, useRef, type CSSProperties, type KeyboardEvent, type ReactNode } from "react";
import type { SheetId } from "../lib/paneSizes";
import { PANE_RESIZE_TITLE, SHEET_EDGES, useResizableSheet } from "../ui/usePaneResize";

const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "input:not([disabled]):not([type='hidden'])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

export function SheetDialog({
  title,
  className = "sheet",
  overlayClassName = "sheet-overlay",
  testId,
  style,
  resizable,
  children,
  onClose,
}: {
  title: string;
  className?: string;
  overlayClassName?: string;
  testId?: string;
  style?: CSSProperties;
  /** Persisted, edge- and corner-resizable sheet; `testId` names the corner grip. */
  resizable?: { id: SheetId; testId?: string };
  children: ReactNode;
  onClose: () => void;
}) {
  const sheetRef = useRef<HTMLDivElement>(null);
  const resize = useResizableSheet(resizable?.id ?? null);

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const sheet = sheetRef.current;
    sheet?.focus({ preventScroll: true });
    return () => {
      previous?.focus({ preventScroll: true });
    };
  }, []);

  function focusableElements(root: HTMLElement | null): HTMLElement[] {
    if (!root) return [];
    return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter((element) => {
      const style = window.getComputedStyle(element);
      return style.visibility !== "hidden" && style.display !== "none";
    });
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.stopPropagation();
      onClose();
      return;
    }

    if (event.key !== "Tab") return;
    const sheet = sheetRef.current;
    const focusable = focusableElements(sheet);
    if (!sheet || focusable.length === 0) {
      event.preventDefault();
      sheet?.focus({ preventScroll: true });
      return;
    }

    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !sheet.contains(active))) {
      event.preventDefault();
      last.focus({ preventScroll: true });
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus({ preventScroll: true });
    }
  }

  const sheet = (
    <div
      ref={sheetRef}
      className={className}
      data-testid={testId}
      role="dialog"
      aria-modal="true"
      aria-label={title}
      tabIndex={-1}
      style={resize?.style ? { ...style, ...resize.style } : style}
      onKeyDown={onKeyDown}
    >
      {children}
      {resize && (
        <button
          className="sheet-resize-handle"
          data-testid={resizable?.testId ?? `${resizable?.id}-sheet-resize-handle`}
          type="button"
          aria-label={`Resize ${resize.label}`}
          title={`${PANE_RESIZE_TITLE} · Arrow keys resize`}
          onPointerDown={(event) => resize.startDrag("se", event, sheetRef.current)}
          onKeyDown={(event) => resize.onKeyDown(event, sheetRef.current)}
          onDoubleClick={() => resize.reset()}
        />
      )}
    </div>
  );

  return (
    <div
      className={overlayClassName}
      data-native-preview-blocker="true"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      {resize ? (
        <div className={`sheet-resize-frame${resize.dragging ? " resizing" : ""}`}>
          {sheet}
          {SHEET_EDGES.map((edge) => (
            <div
              key={edge}
              className={`sheet-resize-edge sheet-resize-edge-${edge}`}
              aria-hidden="true"
              title={PANE_RESIZE_TITLE}
              onPointerDown={(event) => resize.startDrag(edge, event, sheetRef.current)}
              onDoubleClick={() => resize.reset(edge === "e" || edge === "w" ? "width" : edge === "n" || edge === "s" ? "height" : undefined)}
            />
          ))}
        </div>
      ) : sheet}
    </div>
  );
}
