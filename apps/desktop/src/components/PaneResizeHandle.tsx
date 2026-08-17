import { forwardRef, type ComponentPropsWithoutRef } from "react";

type PaneResizeHandleProps = Omit<
  ComponentPropsWithoutRef<"div">,
  "aria-orientation" | "role"
> & {
  orientation: "horizontal" | "vertical";
};

export const PaneResizeHandle = forwardRef<HTMLDivElement, PaneResizeHandleProps>(
  function PaneResizeHandle(
    { className = "", orientation, tabIndex = 0, ...props },
    ref,
  ) {
    return (
      <div
        {...props}
        ref={ref}
        className={`pane-resize-handle pane-resize-handle-${orientation}${className ? ` ${className}` : ""}`}
        role="separator"
        aria-orientation={orientation}
        tabIndex={tabIndex}
      />
    );
  },
);
