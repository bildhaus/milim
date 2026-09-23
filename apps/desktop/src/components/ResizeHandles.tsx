import { getCurrentWindow } from "@tauri-apps/api/window";
import type { MouseEvent } from "react";
import { inTauri } from "../api";


type Dir = "North" | "South" | "East" | "West" | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";

const EDGES: Array<[string, Dir]> = [
  ["n", "North"],
  ["s", "South"],
  ["e", "East"],
  ["w", "West"],
  ["ne", "NorthEast"],
  ["nw", "NorthWest"],
  ["se", "SouthEast"],
  ["sw", "SouthWest"],
];

/** Invisible edge/corner strips that drive native window resizing on the
 *  borderless window. No-op (and not rendered) outside Tauri. */
export function ResizeHandles() {
  if (!inTauri) return null;

  const start = (dir: Dir) => (e: MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    getCurrentWindow()
      .startResizeDragging(dir)
      .catch(() => {});
  };

  return (
    <>
      {EDGES.map(([cls, dir]) => (
        <div key={cls} className={"rz rz-" + cls} onMouseDown={start(dir)} />
      ))}
    </>
  );
}
