import { create } from "zustand";

export type AppConfirmationRequest = {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
  /** Shows a single-line text field; its value is returned by `promptApp`. */
  input?: { label: string; defaultValue?: string; placeholder?: string };
};

type AppConfirmationState = {
  request: AppConfirmationRequest | null;
};

let pendingResolution: ((accepted: boolean, value?: string) => void) | null = null;

export const useAppConfirmation = create<AppConfirmationState>(() => ({
  request: null,
}));

export function confirmApp(request: AppConfirmationRequest): Promise<boolean> {
  pendingResolution?.(false);
  return new Promise((resolve) => {
    pendingResolution = (accepted) => resolve(accepted);
    useAppConfirmation.setState({ request });
  });
}

/** Resolves to the entered text, or null when cancelled or superseded. */
export function promptApp(
  request: AppConfirmationRequest & { input: NonNullable<AppConfirmationRequest["input"]> },
): Promise<string | null> {
  pendingResolution?.(false);
  return new Promise((resolve) => {
    pendingResolution = (accepted, value) => resolve(accepted ? value ?? "" : null);
    useAppConfirmation.setState({ request });
  });
}

export function resolveAppConfirmation(accepted: boolean, value?: string) {
  const resolve = pendingResolution;
  pendingResolution = null;
  useAppConfirmation.setState({ request: null });
  resolve?.(accepted, value);
}
