import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import { userStateStorage } from "../persistence/userStateStorage.js";

export const ONBOARDING_STATE_VERSION = 1;
export const ONBOARDING_DISMISS_SNOOZE_MS = 24 * 60 * 60 * 1000;

export type OnboardingStatus = "not_started" | "in_progress" | "completed" | "dismissed";
export type OnboardingSetupPath = "local_detect" | "hosted" | "account_runtime";
export type OnboardingStepId = "model" | "context" | "finish";

interface OnboardingState {
  version: number;
  status: OnboardingStatus;
  selectedSetupPath: OnboardingSetupPath | null;
  completedSteps: OnboardingStepId[];
  developerShowOnboarding: boolean;
  dismissedAt?: number;
  completedAt?: number;
  startedAt?: number;
  start: () => void;
  setSetupPath: (path: OnboardingSetupPath) => void;
  markStepComplete: (step: OnboardingStepId) => void;
  complete: () => void;
  dismiss: () => void;
  reset: () => void;
  setDeveloperShowOnboarding: (show: boolean) => void;
}

const DEFAULT_STATE = {
  version: ONBOARDING_STATE_VERSION,
  status: "not_started" as OnboardingStatus,
  selectedSetupPath: null,
  completedSteps: [] as OnboardingStepId[],
  developerShowOnboarding: false,
};

function withStep(steps: OnboardingStepId[], step: OnboardingStepId): OnboardingStepId[] {
  return steps.includes(step) ? steps : [...steps, step];
}

function normalizeSteps(value: unknown): OnboardingStepId[] {
  if (!Array.isArray(value)) return [];
  const allowed = new Set<OnboardingStepId>(["model", "context", "finish"]);
  const out: OnboardingStepId[] = [];
  for (const item of value) {
    const normalized = item === "workbench" ? "context" : item;
    if (allowed.has(normalized as OnboardingStepId) && !out.includes(normalized as OnboardingStepId)) {
      out.push(normalized as OnboardingStepId);
    }
  }
  return out;
}

function normalizeSetupPath(value: unknown): OnboardingSetupPath | null {
  if (value === "codex") return "account_runtime";
  return value === "local_detect" || value === "hosted" || value === "account_runtime" ? value : null;
}

function normalizeStatus(value: unknown): OnboardingStatus {
  return value === "in_progress" || value === "completed" || value === "dismissed" ? value : "not_started";
}

export function shouldShowOnboarding(
  status: OnboardingStatus,
  modelsReady: boolean,
  developerShowOnboarding: boolean,
  dismissedAt?: number,
  now = Date.now(),
): boolean {
  if (developerShowOnboarding) return true;
  if (status === "in_progress") return true;
  if (status === "completed") return false;
  if (status === "dismissed") {
    return !modelsReady && typeof dismissedAt === "number" && now - dismissedAt >= ONBOARDING_DISMISS_SNOOZE_MS;
  }
  return !modelsReady;
}

export function shouldCheckOnboardingModels(status: OnboardingStatus, developerShowOnboarding: boolean): boolean {
  return !developerShowOnboarding && status !== "completed" && status !== "in_progress";
}

export const useOnboarding = create<OnboardingState>()(
  persist(
    (set) => ({
      ...DEFAULT_STATE,
      start: () =>
        set((state) => ({
          status: "in_progress",
          startedAt: state.startedAt ?? Date.now(),
          dismissedAt: undefined,
          completedAt: undefined,
        })),
      setSetupPath: (path) =>
        set((state) => ({
          status: "in_progress",
          selectedSetupPath: path,
          startedAt: state.startedAt ?? Date.now(),
        })),
      markStepComplete: (step) =>
        set((state) => ({
          status: state.status === "not_started" ? "in_progress" : state.status,
          completedSteps: withStep(state.completedSteps, step),
          startedAt: state.startedAt ?? Date.now(),
        })),
      complete: () =>
        set((state) => ({
          status: "completed",
          completedSteps: withStep(state.completedSteps, "finish"),
          developerShowOnboarding: false,
          completedAt: Date.now(),
          dismissedAt: undefined,
        })),
      dismiss: () =>
        set({
          status: "dismissed",
          developerShowOnboarding: false,
          dismissedAt: Date.now(),
        }),
      reset: () =>
        set({
          ...DEFAULT_STATE,
          status: "not_started",
          startedAt: undefined,
          dismissedAt: undefined,
          completedAt: undefined,
        }),
      setDeveloperShowOnboarding: (show) =>
        set((state) => ({
          developerShowOnboarding: show,
          status: show ? "in_progress" : state.status === "in_progress" ? "dismissed" : state.status,
          startedAt: show ? state.startedAt ?? Date.now() : state.startedAt,
          dismissedAt: show ? undefined : state.dismissedAt,
          completedAt: show ? undefined : state.completedAt,
        })),
    }),
    {
      name: "milim.onboarding",
      storage: createJSONStorage(() => userStateStorage),
      merge: (persisted, current) => {
        const saved = persisted as Partial<OnboardingState> | undefined;
        if (!saved || saved.version !== ONBOARDING_STATE_VERSION) return current;
        return {
          ...current,
          ...saved,
          version: ONBOARDING_STATE_VERSION,
          status: normalizeStatus(saved.status),
          selectedSetupPath: normalizeSetupPath(saved.selectedSetupPath),
          completedSteps: normalizeSteps(saved.completedSteps),
          developerShowOnboarding: typeof saved.developerShowOnboarding === "boolean" ? saved.developerShowOnboarding : false,
        };
      },
      partialize: (state) => ({
        version: ONBOARDING_STATE_VERSION,
        status: state.status,
        selectedSetupPath: state.selectedSetupPath,
        completedSteps: state.completedSteps,
        developerShowOnboarding: state.developerShowOnboarding,
        dismissedAt: state.dismissedAt,
        completedAt: state.completedAt,
        startedAt: state.startedAt,
      }),
    },
  ),
);
