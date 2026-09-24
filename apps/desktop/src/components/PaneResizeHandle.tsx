import { forwardRef, useCallback, type ComponentPropsWithoutRef, type ForwardedRef } from "react";
import type { PaneResizeController } from "../ui/usePaneResize";

type PaneResizeHandleProps = Omit<
  ComponentPropsWithoutRef<"div">,
  "aria-orientation" | "role"
> & {
  /** Defaults to the controller's axis: panes that resize along x use a vertical separator. */
  orientation?: "horizontal" | "vertical";
  /** Shared resize behaviour from `usePaneResize`; explicit props override its handlers. */
  resize?: PaneResizeController;
};

function assignRef<T>(ref: ForwardedRef<T>, value: T | null) {
  if (typeof ref === "function") ref(value);
  else if (ref) ref.current = value;
}

export const PaneResizeHandle = forwardRef<HTMLDivElement, PaneResizeHandleProps>(
  function PaneResizeHandle(
    { className = "", orientation, resize, tabIndex, ...props },
    ref,
  ) {
    const { ref: controllerRef, ...controllerProps } = resize?.handleProps ?? { ref: undefined, tabIndex: undefined };
    const setRef = useCallback((element: HTMLDivElement | null) => {
      assignRef(ref, element);
      controllerRef?.(element);
    }, [controllerRef, ref]);
    const resolvedOrientation = orientation ?? (resize?.axis === "y" ? "horizontal" : "vertical");
    const dragging = resize?.dragging ? " dragging" : "";
    return (
      <div
        {...controllerProps}
        {...props}
        ref={setRef}
        className={`pane-resize-handle pane-resize-handle-${resolvedOrientation}${dragging}${className ? ` ${className}` : ""}`}
        role="separator"
        aria-orientation={resolvedOrientation}
        tabIndex={tabIndex ?? controllerProps.tabIndex ?? 0}
      />
    );
  },
);
