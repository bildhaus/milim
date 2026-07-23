import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { clampContextMenuPosition, shouldPreserveNativeContextMenu, type ContextMenuPoint } from "../lib/contextMenu";
import { Check } from "./icons";

export type ContextMenuItem = {
  id: string;
  label: string;
  icon?: ReactNode;
  detail?: string;
  description?: string;
  disabled?: boolean;
  danger?: boolean;
  checked?: boolean;
  separatorBefore?: boolean;
  action: () => void | Promise<void>;
};

type ContextMenuState = {
  point: ContextMenuPoint;
  items: ContextMenuItem[];
  label?: string;
  trigger?: HTMLElement;
};

type ContextMenuApi = {
  openContextMenu: (event: MouseEvent, items: ContextMenuItem[], label?: string) => boolean;
  openMenuAt: (point: ContextMenuPoint, items: ContextMenuItem[], label?: string, trigger?: HTMLElement) => boolean;
  closeContextMenu: () => void;
};

const ContextMenuContext = createContext<ContextMenuApi | null>(null);
const ESTIMATED_MENU_SIZE = { width: 240, height: 260 };

export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const [menu, setMenu] = useState<ContextMenuState | null>(null);
  const [position, setPosition] = useState<ContextMenuPoint | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const focusFirstItemRef = useRef(false);

  const closeContextMenu = useCallback(() => setMenu(null), []);

  const openMenuAt = useCallback((point: ContextMenuPoint, items: ContextMenuItem[], label?: string, trigger?: HTMLElement) => {
    if (items.length === 0) return false;
    focusFirstItemRef.current = Boolean(trigger);
    setMenu({ point, items, label, trigger });
    setPosition(clampContextMenuPosition(point, ESTIMATED_MENU_SIZE, { width: window.innerWidth, height: window.innerHeight }));
    return true;
  }, []);

  const openContextMenu = useCallback((event: MouseEvent, items: ContextMenuItem[], label?: string) => {
    if (event.defaultPrevented || shouldPreserveNativeContextMenu(event.target) || items.length === 0) return false;
    event.preventDefault();
    event.stopPropagation();
    return openMenuAt({ x: event.clientX, y: event.clientY }, items, label);
  }, [openMenuAt]);

  useEffect(() => {
    if (!menu || !menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    const next = clampContextMenuPosition(menu.point, { width: rect.width, height: rect.height }, { width: window.innerWidth, height: window.innerHeight });
    setPosition((current) => current && current.x === next.x && current.y === next.y ? current : next);
    if (focusFirstItemRef.current) {
      focusFirstItemRef.current = false;
      menuRef.current.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    }
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const trigger = menu.trigger;
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeContextMenu();
        trigger?.focus();
        return;
      }
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
      const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
      if (!items.length) return;
      event.preventDefault();
      const current = items.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === "Home"
        ? 0
        : event.key === "End"
          ? items.length - 1
          : event.key === "ArrowDown"
            ? (current + 1) % items.length
            : (current <= 0 ? items.length : current) - 1;
      items[next]?.focus();
    }
    function onPointerDown(event: PointerEvent) {
      if (menuRef.current && event.target instanceof Node && menuRef.current.contains(event.target)) return;
      closeContextMenu();
    }
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("blur", closeContextMenu);
    window.addEventListener("resize", closeContextMenu);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("blur", closeContextMenu);
      window.removeEventListener("resize", closeContextMenu);
    };
  }, [closeContextMenu, menu]);

  const api = useMemo(() => ({ openContextMenu, openMenuAt, closeContextMenu }), [closeContextMenu, openContextMenu, openMenuAt]);

  return (
    <ContextMenuContext.Provider value={api}>
      {children}
      {menu && position && createPortal(
        <div
          ref={menuRef}
          className="app-context-menu"
          data-native-preview-blocker="true"
          data-testid="app-context-menu"
          role="menu"
          aria-label={menu.label ?? "Context menu"}
          style={{ left: position.x, top: position.y }}
        >
          {menu.items.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`app-context-menu-item${item.danger ? " danger" : ""}${item.separatorBefore ? " sep-before" : ""}`}
              role="menuitem"
              disabled={item.disabled}
              onClick={() => {
                if (item.disabled) return;
                closeContextMenu();
                void item.action();
              }}
            >
              <span className="app-context-menu-icon" aria-hidden="true">
                {item.checked ? <Check size={13} /> : item.icon}
              </span>
              <span className="app-context-menu-copy">
                <span className="app-context-menu-label">{item.label}</span>
                {item.description && <span className="app-context-menu-description">{item.description}</span>}
              </span>
              {item.detail && <span className="app-context-menu-detail">{item.detail}</span>}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </ContextMenuContext.Provider>
  );
}

export function useContextMenu(): ContextMenuApi {
  const api = useContext(ContextMenuContext);
  if (!api) throw new Error("useContextMenu must be used inside ContextMenuProvider");
  return api;
}
