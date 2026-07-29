import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ModelInfo, ProviderInfo } from "../api";
import type { HotSwapAction, NativeSessionMode } from "../sessions/store";
import type { HotSwapAssessment } from "../lib/hotSwap";
import { ArrowRight, Eye, Refresh, Sparkles } from "./icons";
import { ModelPicker, type ModelPickerSelection } from "./ModelPicker";
import { SheetDialog } from "./SheetDialog";

const ACTION_LABEL: Record<HotSwapAction, string> = {
  switch: "Switch model",
  continue: "Continue with",
  review: "Review with",
  retry: "Retry with",
};

export function BatonMenu({
  retryDisabled,
  onAction,
}: {
  retryDisabled: boolean;
  onAction: (action: Exclude<HotSwapAction, "switch">) => void;
}) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 8, top: 8 });

  useEffect(() => {
    const closeOnOutsideClick = (event: MouseEvent) => {
      const target = event.target as Node;
      if (!open || triggerRef.current?.contains(target) || popoverRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", closeOnOutsideClick);
    return () => document.removeEventListener("mousedown", closeOnOutsideClick);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const close = () => setOpen(false);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [open]);

  function toggleMenu() {
    if (open) {
      setOpen(false);
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect();
    if (rect) {
      const width = 232;
      const estimatedHeight = 148;
      const left = Math.max(8, Math.min(window.innerWidth - width - 8, rect.right - width));
      const top = rect.top - estimatedHeight - 6 >= 8 ? rect.top - estimatedHeight - 6 : rect.bottom + 6;
      setPosition({ left, top });
    }
    setOpen(true);
  }

  function choose(action: Exclude<HotSwapAction, "switch">) {
    setOpen(false);
    onAction(action);
  }

  return (
    <span
      className={"baton-menu" + (open ? " open" : "")}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        setOpen(false);
        triggerRef.current?.focus();
      }}
    >
      <button
        type="button"
        className="msg-act msg-act-text baton-continue"
        data-testid="baton-continue"
        onClick={() => choose("continue")}
      >
        <ArrowRight size={13} />
        <span>Continue with...</span>
      </button>
      <button
        ref={triggerRef}
        type="button"
        className="msg-act baton-menu-trigger"
        title="More model handoff actions"
        aria-label="More model handoff actions"
        aria-haspopup="menu"
        aria-expanded={open}
        data-testid="baton-menu-trigger"
        onClick={toggleMenu}
      >
        <Sparkles size={13} />
      </button>
      {open && createPortal(
        <div
          ref={popoverRef}
          className="baton-menu-popover message-popover-layer"
          role="menu"
          aria-label="Model handoff actions"
          data-native-preview-blocker="true"
          style={position}
        >
          <button type="button" role="menuitem" onClick={() => choose("review")}>
            <Eye size={14} />
            <span><strong>Review with...</strong><small>Get a read-only second opinion</small></span>
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={retryDisabled}
            title={retryDisabled ? "Clean Retry requires a Git checkpoint" : undefined}
            onClick={() => choose("retry")}
          >
            <Refresh size={14} />
            <span>
              <strong>Retry with...</strong>
              <small>{retryDisabled ? "Needs a Git checkpoint" : "Branch before this turn"}</small>
            </span>
          </button>
        </div>,
        document.body,
      )}
    </span>
  );
}

export function BatonTargetSheet({
  action,
  models,
  model,
  providers,
  toolIntent,
  onSelect,
  onManageProviders,
  onClose,
}: {
  action: Exclude<HotSwapAction, "switch">;
  models: ModelInfo[];
  model: string;
  providers: ProviderInfo[];
  toolIntent: boolean;
  onSelect: (selection: ModelPickerSelection) => void;
  onManageProviders: () => void;
  onClose: () => void;
}) {
  return (
    <SheetDialog
      title={`${ACTION_LABEL[action]} another model`}
      className="sheet hot-swap-target-sheet"
      testId="hot-swap-target"
      onClose={onClose}
    >
      <div className="sheet-header">
        <div>
          <h2>{ACTION_LABEL[action]} another model</h2>
          <p className="sheet-sub">Choose the collaborator for this Baton action.</p>
        </div>
        <button className="icon-btn" type="button" aria-label="Close" onClick={onClose}>×</button>
      </div>
      <ModelPicker
        models={models}
        model={model}
        providers={providers}
        toolIntent={toolIntent}
        onModel={onSelect}
        onManageProviders={onManageProviders}
        onManageMcp={() => {}}
        onManageMemory={() => {}}
        onClose={onClose}
        showManagementActions={false}
      />
    </SheetDialog>
  );
}

export function HotSwapPreflightSheet({
  fromModel,
  targetModel,
  assessment,
  onConfirm,
  onClose,
}: {
  fromModel: string;
  targetModel: string;
  assessment: HotSwapAssessment;
  onConfirm: (nativeMode?: NativeSessionMode) => void;
  onClose: () => void;
}) {
  const [nativeMode, setNativeMode] = useState<NativeSessionMode>("fresh");
  const blocked = assessment.parity === "blocked";
  const resumeDisabled = assessment.issues.some(
    (issue) =>
      issue.code === "context_compaction_required" ||
      issue.code === "context_too_large",
  );
  return (
    <SheetDialog
      title="Hot Swap preflight"
      className="sheet hot-swap-sheet"
      testId="hot-swap-preflight"
      onClose={onClose}
    >
      <div className="sheet-header">
        <div>
          <h2>Hot Swap</h2>
          <p className="sheet-sub">{fromModel || "Current model"} to {targetModel}</p>
        </div>
        <button className="icon-btn" type="button" aria-label="Close" onClick={onClose}>×</button>
      </div>
      <div className={`hot-swap-parity ${assessment.parity}`}>
        {assessment.parity === "translated" ? "Translated handoff" : `${assessment.parity[0].toUpperCase()}${assessment.parity.slice(1)} handoff`}
      </div>
      <div className="hot-swap-issues">
        {assessment.issues.map((issue) => (
          <div className="hot-swap-issue" key={issue.code}>
            <strong>{issue.title}</strong>
            <span>{issue.detail}</span>
          </div>
        ))}
      </div>
      {assessment.nativeSessionStale && (
        <fieldset className="hot-swap-native-choice">
          <legend>Native session</legend>
          <label>
            <input type="radio" name="native-session-mode" checked={nativeMode === "fresh"} onChange={() => setNativeMode("fresh")} />
            <span><strong>Fresh:</strong> start from Milim's complete canonical context.</span>
          </label>
          <label>
            <input type="radio" name="native-session-mode" disabled={resumeDisabled} checked={nativeMode === "resume"} onChange={() => setNativeMode("resume")} />
            <span><strong>Resume:</strong> {resumeDisabled ? "unavailable because this handoff requires compaction." : "keep hidden native history and inject intervening turns."}</span>
          </label>
        </fieldset>
      )}
      <div className="sheet-actions hot-swap-actions">
        <button type="button" className="btn-ghost" onClick={onClose}>Cancel</button>
        <button
          type="button"
          className="btn-accent"
          disabled={blocked}
          onClick={() => onConfirm(assessment.nativeSessionStale ? nativeMode : undefined)}
        >
          {blocked ? "Target unavailable" : "Confirm Hot Swap"}
        </button>
      </div>
    </SheetDialog>
  );
}
