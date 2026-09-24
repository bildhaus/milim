import { useContextMenu, type ContextMenuItem } from "./ContextMenu";
import { MoreHorizontal } from "./icons";

/**
 * A labelled "More actions" button that opens the shared app menu below it.
 * The menu takes focus, supports arrow keys, and returns focus on Escape.
 */
export function OverflowMenuButton({
  label,
  items,
  className = "",
  testId,
  disabled = false,
}: {
  label: string;
  items: ContextMenuItem[];
  className?: string;
  testId?: string;
  disabled?: boolean;
}) {
  const { openMenuAt } = useContextMenu();
  return (
    <button
      className={`icon-btn overflow-menu-button ${className}`.trim()}
      type="button"
      title={label}
      aria-label={label}
      aria-haspopup="menu"
      data-testid={testId}
      disabled={disabled || items.length === 0}
      onClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        openMenuAt(
          { x: Math.max(8, rect.right - 220), y: rect.bottom + 4 },
          items,
          label,
          event.currentTarget,
        );
      }}
    >
      <MoreHorizontal size={16} aria-hidden="true" />
    </button>
  );
}
