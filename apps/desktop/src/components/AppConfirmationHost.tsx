import { createPortal } from "react-dom";
import { resolveAppConfirmation, useAppConfirmation } from "../ui/confirmation";
import { SheetDialog } from "./SheetDialog";

export function AppConfirmationHost() {
  const request = useAppConfirmation((state) => state.request);
  if (!request || typeof document === "undefined") return null;

  const danger = request.tone === "danger";
  return createPortal(
    <SheetDialog
      title={request.title}
      className="git-modal app-confirmation-modal"
      overlayClassName="git-modal-backdrop app-confirmation-backdrop"
      testId="app-confirmation-dialog"
      onClose={() => resolveAppConfirmation(false)}
    >
      <div className="git-modal-head">
        <strong>{request.title}</strong>
      </div>
      <p>{request.message}</p>
      <div className="app-confirmation-actions">
        <button className="btn-ghost" type="button" onClick={() => resolveAppConfirmation(false)}>
          {request.cancelLabel ?? "Cancel"}
        </button>
        <button
          className={danger ? "app-confirmation-confirm danger" : "btn-accent"}
          type="button"
          onClick={() => resolveAppConfirmation(true)}
        >
          {request.confirmLabel ?? "Continue"}
        </button>
      </div>
    </SheetDialog>,
    document.body,
  );
}
