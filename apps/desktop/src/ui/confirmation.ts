import { create } from "zustand";

export type AppConfirmationRequest = {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  tone?: "default" | "danger";
};

type AppConfirmationState = {
  request: AppConfirmationRequest | null;
};

let pendingResolution: ((accepted: boolean) => void) | null = null;

export const useAppConfirmation = create<AppConfirmationState>(() => ({
  request: null,
}));

export function confirmApp(request: AppConfirmationRequest): Promise<boolean> {
  pendingResolution?.(false);
  return new Promise((resolve) => {
    pendingResolution = resolve;
    useAppConfirmation.setState({ request });
  });
}

export function resolveAppConfirmation(accepted: boolean) {
  const resolve = pendingResolution;
  pendingResolution = null;
  useAppConfirmation.setState({ request: null });
  resolve?.(accepted);
}
