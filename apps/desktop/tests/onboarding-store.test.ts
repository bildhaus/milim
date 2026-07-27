class MemoryStorage implements Storage {
  private values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return Array.from(this.values.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

localStorage.setItem("milim.onboarding", JSON.stringify({
  state: {
    version: 1,
    status: "completed",
    selectedSetupPath: "codex",
    completedSteps: ["model", "defaults", "workbench", "finish"],
    developerShowOnboarding: false,
  },
  version: 0,
}));

const { ONBOARDING_DISMISS_SNOOZE_MS, ONBOARDING_STATE_VERSION, shouldCheckOnboardingModels, shouldShowOnboarding, useOnboarding } = await import("../src/onboarding/store.js");

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

equal(useOnboarding.getState().version, ONBOARDING_STATE_VERSION, "onboarding state should have a version");
equal(useOnboarding.getState().selectedSetupPath, "account_runtime", "legacy Codex setup should migrate to account runtimes");
assert(!useOnboarding.getState().completedSteps.some((step) => String(step) === "defaults"), "removed defaults step should be discarded");
assert(useOnboarding.getState().completedSteps.includes("context"), "legacy workbench step should migrate to workspace");
equal(useOnboarding.getState().status, "completed", "completed onboarding should stay completed after migration");

useOnboarding.getState().reset();
equal(useOnboarding.getState().status, "not_started", "onboarding should default to not started");
equal(useOnboarding.getState().developerShowOnboarding, false, "developer override should default off");
assert(shouldShowOnboarding("not_started", false, false), "first run should show when no models are ready");
assert(!shouldShowOnboarding("not_started", true, false), "first run should not show when models already exist");
assert(shouldShowOnboarding("completed", true, true), "developer override should show completed onboarding");
assert(!shouldCheckOnboardingModels("completed", false), "completed onboarding should not block startup on model discovery");
assert(!shouldCheckOnboardingModels("in_progress", false), "active onboarding should not block startup on model discovery");
assert(!shouldCheckOnboardingModels("not_started", true), "developer override should not block startup on model discovery");
assert(shouldCheckOnboardingModels("not_started", false), "first run should check models before deciding onboarding");
assert(!shouldShowOnboarding("dismissed", false, false, 1_000, 1_000), "dismiss should snooze onboarding immediately");
assert(
  shouldShowOnboarding("dismissed", false, false, 1_000, 1_000 + ONBOARDING_DISMISS_SNOOZE_MS),
  "dismissed onboarding should return after the snooze when no models are ready",
);

useOnboarding.getState().start();
equal(useOnboarding.getState().status, "in_progress", "starting should begin unified onboarding");

useOnboarding.getState().setSetupPath("hosted");
equal(useOnboarding.getState().selectedSetupPath, "hosted", "setup path should be stored");

useOnboarding.getState().setSetupPath("account_runtime");
equal(useOnboarding.getState().selectedSetupPath, "account_runtime", "account runtime setup path should be stored");

useOnboarding.getState().markStepComplete("model");
assert(useOnboarding.getState().completedSteps.includes("model"), "model step should be marked complete");

useOnboarding.getState().setDeveloperShowOnboarding(true);
equal(useOnboarding.getState().developerShowOnboarding, true, "developer override should turn on");
equal(useOnboarding.getState().status, "in_progress", "developer override should keep flow in progress");

useOnboarding.getState().complete();
equal(useOnboarding.getState().status, "completed", "complete should mark onboarding completed");
equal(useOnboarding.getState().developerShowOnboarding, false, "complete should clear developer override");
assert(useOnboarding.getState().completedSteps.includes("finish"), "finish step should be marked complete");

useOnboarding.getState().reset();
equal(useOnboarding.getState().status, "not_started", "reset should restore first-run status");
equal(useOnboarding.getState().completedSteps.length, 0, "reset should clear completed steps");

export {};
