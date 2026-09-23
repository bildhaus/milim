import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { resolveAppConfirmation, useAppConfirmation, type AppConfirmationRequest } from "../ui/confirmation";
import { SheetDialog } from "./SheetDialog";

export function AppConfirmationHost() {
  const request = useAppConfirmation((state) => state.request);
  if (!request || typeof document === "undefined") return null;
  return createPortal(<AppConfirmationDialog request={request} />, document.body);
}

function AppConfirmationDialog({ request }: { request: AppConfirmationRequest }) {
  const danger = request.tone === "danger";
  const [value, setValue] = useState(request.input?.defaultValue ?? "");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!request.input) return;
    const frame = window.requestAnimationFrame(() => {
      inputRef.current?.focus();
      inputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [request.input]);

  return (
    <SheetDialog
      title={request.title}
      className="git-modal app-confirmation-modal"
      overlayClassName="git-modal-backdrop app-confirmation-backdrop"
      testId="app-confirmation-dialog"
      onClose={() => resolveAppConfirmation(false)}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          resolveAppConfirmation(true, value);
        }}
      >
        <div className="git-modal-head">
          <strong>{request.title}</strong>
        </div>
        <p>{request.message}</p>
        {request.input && (
          <input
            ref={inputRef}
            className="app-confirmation-input"
            data-testid="app-confirmation-input"
            aria-label={request.input.label}
            placeholder={request.input.placeholder}
            value={value}
            onChange={(event) => setValue(event.currentTarget.value)}
          />
        )}
        <div className="app-confirmation-actions">
          <button className="btn-ghost" type="button" onClick={() => resolveAppConfirmation(false)}>
            {request.cancelLabel ?? "Cancel"}
          </button>
          <button className={danger ? "app-confirmation-confirm danger" : "btn-accent"} type="submit">
            {request.confirmLabel ?? "Continue"}
          </button>
        </div>
      </form>
    </SheetDialog>
  );
}
