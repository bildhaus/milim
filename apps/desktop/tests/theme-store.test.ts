import type { Theme } from "../src/theme/types";

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

const rootStyle = {
  colorScheme: "",
  setProperty(_key: string, _value: string): void {},
};

Object.defineProperty(globalThis, "document", {
  value: {
    documentElement: {
      dataset: {},
      style: rootStyle,
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, "localStorage", {
  value: new MemoryStorage(),
  configurable: true,
});

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

const customTheme: Theme = {
  id: "custom-persisted",
  name: "Custom Persisted",
  isDark: true,
  colors: {
    primaryText: "#f8fafc",
    secondaryText: "#cbd5e1",
    tertiaryText: "#64748b",
    placeholderText: "#64748b",
    bgPrimary: "#020617",
    bgSecondary: "#0f172a",
    bgTertiary: "#1e293b",
    sidebarBg: "#020617",
    accent: "#38bdf8",
    accentLight: "#7dd3fc",
    borderPrimary: "#334155",
    borderSecondary: "#475569",
    focusBorder: "#0ea5e9",
    success: "#22c55e",
    warning: "#f59e0b",
    error: "#ef4444",
    info: "#38bdf8",
    cardBg: "#0f172a",
    cardBorder: "#334155",
    inputBg: "#020617",
    inputBorder: "#334155",
  },
  glass: { enabled: false, blurRadius: 24, opacityPrimary: 1, opacitySecondary: 1, edgeLight: "rgba(255,255,255,0.08)" },
  background: { imageOpacity: 1, overlayOpacity: 0 },
  borders: { cardRadius: 8, inputRadius: 8, borderOpacity: 1 },
  typography: {
    fontFamily: "system-ui, sans-serif",
    monoFamily: "ui-monospace, monospace",
  },
};

localStorage.setItem("milim.customThemes", JSON.stringify([customTheme]));
localStorage.setItem("milim.themeId", customTheme.id);

const { hydrateThemeFromUserState, useTheme } = await import("../src/theme/store.js");
const { animatedBackgroundSource, themeCssVariables } = await import("../src/theme/applyTheme.js");

equal(
  animatedBackgroundSource("url(data:image/gif;base64,R0lGODlhAQABAIAAAAUEBA==)"),
  "data:image/gif;base64,R0lGODlhAQABAIAAAAUEBA==",
  "embedded GIF backgrounds should use the animated image layer",
);
equal(animatedBackgroundSource("url(data:image/png;base64,iVBORw0KGgo=)"), undefined, "static backgrounds should keep the CSS layer");

equal(useTheme.getState().themeId, customTheme.id, "legacy raw active custom theme should initialize");
equal(useTheme.getState().theme.name, customTheme.name, "active custom theme should resolve from persisted custom themes");

const glassPopoverVars = themeCssVariables({
  ...customTheme,
  glass: { ...customTheme.glass, enabled: true, blurRadius: 12, opacityPrimary: 0.95 },
});
equal(glassPopoverVars["--popover-blur"], "12px", "glass popovers should inherit theme blur");
equal(glassPopoverVars["--popover-bg"], "rgba(15, 23, 42, 0.88)", "glass popovers should stay translucent enough to show blur");

localStorage.setItem("milim.themeId", JSON.stringify("mono-light"));
await hydrateThemeFromUserState();
equal(useTheme.getState().themeId, "mono-light", "JSON encoded active built-in theme should hydrate");

localStorage.setItem("milim.themeId", JSON.stringify(customTheme.id));
await hydrateThemeFromUserState();
equal(useTheme.getState().themeId, customTheme.id, "JSON encoded active custom theme should hydrate");

const nextCustomTheme = { ...customTheme, id: "custom-next", name: "Custom Next" };
useTheme.getState().saveCustom(nextCustomTheme);
equal(localStorage.getItem("milim.themeId"), '"custom-next"', "saved custom theme should persist active ID as JSON string");
assert(localStorage.getItem("milim.customThemes")?.includes('"custom-next"'), "saved custom theme should persist in custom theme list");

export {};
