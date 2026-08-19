import type { KeyboardEvent, ReactNode } from "react";
import { ArrowLeft } from "../components/icons";
import { WindowControls } from "../components/WindowControls";
import { startWindowDrag } from "../ui/windowDrag";

export function SettingsSurface({
  ariaLabel = "Settings",
  backLabel,
  backTestId,
  children,
  className = "",
  onBack,
  testId,
}: {
  ariaLabel?: string;
  backLabel: string;
  backTestId: string;
  children: ReactNode;
  className?: string;
  onBack: () => void;
  testId: string;
}) {
  function onKeyDown(event: KeyboardEvent<HTMLElement>) {
    if (event.defaultPrevented || event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    onBack();
  }

  return (
    <main
      className={`settings-page${className ? ` ${className}` : ""}`}
      data-native-preview-blocker="true"
      data-testid={testId}
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
    >
      <header
        className="settings-page-titlebar"
        data-tauri-drag-region
        onMouseDown={startWindowDrag}
      >
        <div className="settings-page-titlebar-leading">
          <button
            className="settings-page-back"
            data-testid={backTestId}
            type="button"
            autoFocus
            onClick={onBack}
          >
            <ArrowLeft size={14} aria-hidden="true" />
            <span>{backLabel}</span>
          </button>
        </div>
        <div className="settings-page-drag-region" data-tauri-drag-region />
        <WindowControls />
      </header>
      {children}
    </main>
  );
}
