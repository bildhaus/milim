import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent } from "react";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const INTERACTIVE_TITLEBAR_SELECTOR = [
  "a",
  "button",
  "input",
  "select",
  "textarea",
  '[role="button"]',
  "[data-window-drag-ignore]",
].join(",");

export function startWindowDrag(event: MouseEvent<HTMLElement>) {
  if (!inTauri || event.button !== 0) return;
  const target = event.target;
  if (!(target instanceof Element)) return;
  if (target.closest(INTERACTIVE_TITLEBAR_SELECTOR)) return;

  event.preventDefault();
  getCurrentWindow()
    .startDragging()
    .catch(() => {});
}
