import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { requestDesktopHide } from "../api";
import { useUiPreferences } from "../ui/store";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function IconMin() {
  return <svg width="11" height="11" viewBox="0 0 16 16"><path d="M3 8h10" stroke="currentColor" strokeWidth="1.3" /></svg>;
}
function IconMax() {
  return <svg width="11" height="11" viewBox="0 0 16 16"><rect x="3.5" y="3.5" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" strokeWidth="1.3" /></svg>;
}
function IconRestore() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <rect x="5" y="5" width="7.5" height="7.5" rx="1.3" />
      <path d="M5 4.2V3.4A1.4 1.4 0 0 1 6.4 2h6.2A1.4 1.4 0 0 1 14 3.4v6.2a1.4 1.4 0 0 1-1.4 1.4h-.8" />
    </svg>
  );
}
function IconClose() {
  return <svg width="11" height="11" viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" /></svg>;
}

export function WindowControls() {
  const [maxed, setMaxed] = useState(false);
  const pushNotice = useUiPreferences((state) => state.pushNotice);

  useEffect(() => {
    if (!inTauri) return;
    const w = getCurrentWindow();
    let un: (() => void) | undefined;
    w.isMaximized().then(setMaxed).catch(() => {});
    w.onResized(() => w.isMaximized().then(setMaxed).catch(() => {}))
      .then((u) => (un = u))
      .catch(() => {});
    return () => un?.();
  }, []);

  const act = (label: string, fn: () => Promise<unknown>) => () => {
    if (!inTauri) return;
    void Promise.resolve()
      .then(fn)
      .catch((error) => {
        const detail = error instanceof Error ? error.message : String(error);
        console.error(`${label} failed.`, error);
        pushNotice({ tone: "error", message: `${label} failed: ${detail}` });
      });
  };

  return (
    <div className="win-controls">
      <button type="button" className="win-btn" title="Minimize" aria-label="Minimize window" onClick={act("Minimizing window", () => getCurrentWindow().minimize())}>
        <IconMin />
      </button>
      <button
        type="button"
        className="win-btn"
        title={maxed ? "Restore" : "Maximize"}
        aria-label={maxed ? "Restore window" : "Maximize window"}
        onClick={act("Changing window size", () => getCurrentWindow().toggleMaximize())}
      >
        {maxed ? <IconRestore /> : <IconMax />}
      </button>
      <button type="button" className="win-btn win-close" title="Close" aria-label="Close window" onClick={act("Hiding window", requestDesktopHide)}>
        <IconClose />
      </button>
    </div>
  );
}
