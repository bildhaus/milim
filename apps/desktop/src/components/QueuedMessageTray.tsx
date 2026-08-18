import {
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import type { QueuedMessage } from "../sessions/store";
import type { ContextMenuItem } from "./ContextMenu";
import { ArrowRight, MoreHorizontal, Pencil, Trash } from "./icons";

function queuedAttachmentLabel(count: number): string {
  return count === 1 ? "1 attachment" : `${count} attachments`;
}

type QueuedDropTarget = {
  id: string;
  position: "before" | "after";
};

type QueuedPointerDrag = {
  id: string;
  pointerId: number;
  startX: number;
  startY: number;
  active: boolean;
  source: HTMLElement;
  captureTarget: HTMLButtonElement;
};

const QUEUED_DRAG_THRESHOLD = 4;

export type QueuedMessageTrayItem = QueuedMessage & {
  source: "canonical" | "legacy";
};

export function QueuedMessageTray({
  items,
  busy,
  canActivate,
  interruptingMessageId,
  openContextMenu,
  onActivate,
  onEdit,
  onMove,
  onRemove,
}: {
  items: QueuedMessageTrayItem[];
  busy: boolean;
  canActivate: boolean;
  interruptingMessageId?: string;
  openContextMenu: (
    event: MouseEvent,
    items: ContextMenuItem[],
    label?: string,
  ) => boolean;
  onActivate: (item: QueuedMessageTrayItem) => void;
  onEdit: (item: QueuedMessageTrayItem) => void;
  onMove: (
    item: QueuedMessageTrayItem,
    target: QueuedMessageTrayItem,
    position: "before" | "after",
  ) => void;
  onRemove: (item: QueuedMessageTrayItem) => void;
}) {
  const pointerDragRef = useRef<QueuedPointerDrag | null>(null);
  const dropTargetRef = useRef<QueuedDropTarget | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<QueuedDropTarget | null>(null);
  const [reorderStatus, setReorderStatus] = useState("");

  if (items.length === 0) return null;

  function setQueuedDropTarget(target: QueuedDropTarget | null) {
    dropTargetRef.current = target;
    setDropTarget(target);
  }

  function clearQueuedDrag() {
    const drag = pointerDragRef.current;
    if (drag) {
      drag.source.style.removeProperty("pointer-events");
      drag.source.style.removeProperty("translate");
      drag.source.style.removeProperty("will-change");
      if (drag.captureTarget.hasPointerCapture(drag.pointerId)) {
        drag.captureTarget.releasePointerCapture(drag.pointerId);
      }
    }
    pointerDragRef.current = null;
    setDraggingId(null);
    setQueuedDropTarget(null);
  }

  function dropTargetAt(clientX: number, clientY: number, sourceId: string) {
    const element = document.elementFromPoint(clientX, clientY);
    const row =
      element instanceof Element
        ? element.closest<HTMLElement>("[data-queued-message-id]")
        : null;
    const id = row?.dataset.queuedMessageId;
    if (!row || !id || id === sourceId) return null;
    const rect = row.getBoundingClientRect();
    return {
      id,
      position: clientY > rect.top + rect.height / 2 ? "after" : "before",
    } as QueuedDropTarget;
  }

  function startQueuedDrag(
    event: ReactPointerEvent<HTMLButtonElement>,
    id: string,
  ) {
    if (
      event.button !== 0 ||
      items.length < 2 ||
      Boolean(interruptingMessageId)
    )
      return;
    const source = event.currentTarget.closest<HTMLElement>(
      "[data-queued-message-id]",
    );
    if (!source) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    pointerDragRef.current = {
      id,
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      active: false,
      source,
      captureTarget: event.currentTarget,
    };
  }

  function moveQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const moved = Math.hypot(
      event.clientX - drag.startX,
      event.clientY - drag.startY,
    );
    if (!drag.active && moved < QUEUED_DRAG_THRESHOLD) return;
    if (!drag.active) {
      drag.active = true;
      drag.source.style.pointerEvents = "none";
      drag.source.style.willChange = "translate";
      setDraggingId(drag.id);
    }
    event.preventDefault();
    drag.source.style.translate = `0 ${event.clientY - drag.startY}px`;
    setQueuedDropTarget(dropTargetAt(event.clientX, event.clientY, drag.id));
  }

  function endQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    if (drag.active) {
      event.preventDefault();
      const target =
        dropTargetAt(event.clientX, event.clientY, drag.id) ??
        dropTargetRef.current;
      if (target) {
        const sourceItem = items.find((item) => item.id === drag.id);
        const targetItem = items.find((item) => item.id === target.id);
        if (sourceItem && targetItem) {
          onMove(sourceItem, targetItem, target.position);
        }
        const nextItems = items.filter((item) => item.id !== drag.id);
        const targetIndex = nextItems.findIndex(
          (item) => item.id === target.id,
        );
        const nextIndex =
          targetIndex + (target.position === "after" ? 1 : 0) + 1;
        setReorderStatus(
          `Queued message moved to position ${nextIndex} of ${items.length}.`,
        );
      }
    }
    clearQueuedDrag();
  }

  function cancelQueuedDrag(event: ReactPointerEvent<HTMLButtonElement>) {
    const drag = pointerDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    clearQueuedDrag();
  }

  function moveQueuedWithKeyboard(
    event: KeyboardEvent<HTMLButtonElement>,
    item: QueuedMessageTrayItem,
    index: number,
  ) {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const targetIndex = index + (event.key === "ArrowUp" ? -1 : 1);
    const target = items[targetIndex];
    if (!target) return;
    onMove(item, target, event.key === "ArrowUp" ? "before" : "after");
    setReorderStatus(
      `Queued message moved to position ${targetIndex + 1} of ${items.length}.`,
    );
  }

  return (
    <div className="queued-tray" data-testid="queued-message-tray">
      <div className="queued-list">
        {items.map((item, index) => {
          const text = item.content.trim();
          const attachmentCount = item.attachments?.length ?? 0;
          const rowDrop = dropTarget?.id === item.id ? dropTarget : null;
          const interrupting = interruptingMessageId === item.id;
          return (
            <div
              className={`queued-item${draggingId === item.id ? " dragging" : ""}${rowDrop ? ` drag-over drop-${rowDrop.position}` : ""}`}
              data-testid="queued-message"
              data-queued-message-id={item.id}
              key={item.id}
            >
              <button
                className="queued-drag-handle"
                type="button"
                aria-label={`Reorder queued message ${index + 1} of ${items.length}`}
                disabled={items.length < 2 || Boolean(interruptingMessageId)}
                onPointerDown={(event) => startQueuedDrag(event, item.id)}
                onPointerMove={moveQueuedDrag}
                onPointerUp={endQueuedDrag}
                onPointerCancel={cancelQueuedDrag}
                onKeyDown={(event) =>
                  moveQueuedWithKeyboard(event, item, index)
                }
              />
              <span
                className="queued-copy"
                title={text || queuedAttachmentLabel(attachmentCount)}
              >
                {text || "Attached files"}
              </span>
              {attachmentCount > 0 && (
                <span className="queued-meta">
                  {queuedAttachmentLabel(attachmentCount)}
                </span>
              )}
              <button
                className="queued-activate"
                type="button"
                title={
                  canActivate
                    ? busy
                      ? "Interrupt the current response and run this message"
                      : "Run this queued message next"
                    : "Choose a chat model to run queued messages"
                }
                disabled={!canActivate || Boolean(interruptingMessageId)}
                onClick={() => onActivate(item)}
              >
                <ArrowRight size={12} />
                <span>
                  {interrupting ? "Interrupting..." : busy ? "Interrupt" : "Run"}
                </span>
              </button>
              <button
                className="queued-action"
                type="button"
                title="Remove queued message"
                aria-label="Remove queued message"
                disabled={Boolean(interruptingMessageId)}
                onClick={() => onRemove(item)}
              >
                <Trash size={12} />
              </button>
              <button
                className="queued-action"
                type="button"
                title="More queued message actions"
                aria-label="More queued message actions"
                disabled={Boolean(interruptingMessageId)}
                onClick={(event) =>
                  openContextMenu(
                    event,
                    [
                      {
                        id: "edit",
                        label: "Edit queued message",
                        icon: <Pencil size={13} />,
                        action: () => onEdit(item),
                      },
                    ],
                    "Queued message actions",
                  )
                }
              >
                <MoreHorizontal size={13} />
              </button>
            </div>
          );
        })}
      </div>
      <span className="queued-reorder-status" role="status" aria-live="polite">
        {reorderStatus}
      </span>
    </div>
  );
}
