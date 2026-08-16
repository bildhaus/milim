import { create } from "zustand";
import { readUserStateKey, writeUserStateKey } from "../persistence/userStateStorage.js";
import { useUiPreferences } from "../ui/store.js";
import { appearanceSnapshot } from "./appearanceSnapshot.js";
import { applyTheme } from "./applyTheme.js";
import { BUILTIN_THEMES, DEFAULT_THEME_ID, themeById } from "./themes.js";
import type { Theme } from "./types";

const ACTIVE_KEY = "milim.themeId";
const CUSTOM_KEY = "milim.customThemes";
const APPEARANCE_KEY = "milim.appearanceSnapshot";
let pendingAppearanceWrite: Promise<void> | null = null;

function loadCustom(): Theme[] {
  return parseCustomThemes(readUserStateKeySync(CUSTOM_KEY));
}
function loadActiveThemeId(): string | null {
  return parseActiveThemeId(readUserStateKeySync(ACTIVE_KEY));
}
function readUserStateKeySync(key: string): string | null {
  const value = readUserStateKey(key);
  if (typeof value === "string") return value;
  if (value === null) return null;
  return null;
}
function parseActiveThemeId(raw: string | null): string | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return typeof parsed === "string" ? parsed : null;
  } catch {
    return raw;
  }
}
function parseCustomThemes(raw: string | null): Theme[] {
  try {
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? (parsed as Theme[]) : [];
  } catch {
    return [];
  }
}
function persistCustom(list: Theme[]) {
  void writeUserStateKey(CUSTOM_KEY, JSON.stringify(list));
}
function persistActiveThemeId(id: string) {
  void writeUserStateKey(ACTIVE_KEY, JSON.stringify(id));
}
function trackAppearanceWrite(task: Promise<void>) {
  pendingAppearanceWrite = task;
  void task
    .catch((error) => console.warn("Failed to publish appearance:", error))
    .finally(() => {
      if (pendingAppearanceWrite === task) pendingAppearanceWrite = null;
    });
}
function persistAppearance(theme: Theme) {
  const { backgroundFit, backgroundTreatment } = useUiPreferences.getState();
  const value = JSON.stringify(
    appearanceSnapshot(theme, { backgroundFit, backgroundTreatment }),
  );
  const write = () => Promise.resolve(writeUserStateKey(APPEARANCE_KEY, value));
  if (!pendingAppearanceWrite) {
    const result = writeUserStateKey(APPEARANCE_KEY, value);
    if (result) trackAppearanceWrite(result);
    return;
  }
  trackAppearanceWrite(pendingAppearanceWrite.catch(() => {}).then(write));
}
function resolve(id: string, custom: Theme[]): Theme {
  return custom.find((t) => t.id === id) ?? themeById(id);
}

interface ThemeState {
  themeId: string;
  theme: Theme;
  builtins: Theme[];
  custom: Theme[];
  themes: Theme[];
  setTheme: (id: string) => void;
  /** Apply a theme live without persisting (for the editor preview). */
  preview: (t: Theme) => void;
  /** Re-apply the currently selected theme (cancel a preview). */
  revert: () => void;
  /** Save (insert/update) a custom theme and make it active. */
  saveCustom: (t: Theme) => void;
  deleteCustom: (id: string) => void;
}

export const useTheme = create<ThemeState>((set, get) => {
  const custom = loadCustom();
  const initialId = loadActiveThemeId() ?? DEFAULT_THEME_ID;
  const initial = resolve(initialId, custom);
  applyTheme(initial); // before first paint

  return {
    themeId: initial.id,
    theme: initial,
    builtins: BUILTIN_THEMES,
    custom,
    themes: [...BUILTIN_THEMES, ...custom],

    setTheme: (id) => {
      const t = resolve(id, get().custom);
      applyTheme(t);
      persistActiveThemeId(t.id);
      persistAppearance(t);
      set({ themeId: t.id, theme: t });
    },

    preview: (t) => applyTheme(t),
    revert: () => applyTheme(get().theme),

    saveCustom: (t) => {
      const list = get().custom.filter((x) => x.id !== t.id);
      list.push(t);
      persistCustom(list);
      applyTheme(t);
      persistActiveThemeId(t.id);
      persistAppearance(t);
      set({ custom: list, themes: [...BUILTIN_THEMES, ...list], themeId: t.id, theme: t });
    },

    deleteCustom: (id) => {
      const list = get().custom.filter((x) => x.id !== id);
      persistCustom(list);
      const next = get().themeId === id ? BUILTIN_THEMES[0] : resolve(get().themeId, list);
      applyTheme(next);
      persistActiveThemeId(next.id);
      persistAppearance(next);
      set({ custom: list, themes: [...BUILTIN_THEMES, ...list], themeId: next.id, theme: next });
    },
  };
});

useUiPreferences.subscribe((state, previous) => {
  if (
    state.backgroundFit !== previous.backgroundFit ||
    state.backgroundTreatment !== previous.backgroundTreatment
  ) {
    persistAppearance(useTheme.getState().theme);
  }
});

export async function hydrateThemeFromUserState(): Promise<void> {
  const [customRaw, activeRaw] = await Promise.all([
    Promise.resolve(readUserStateKey(CUSTOM_KEY)),
    Promise.resolve(readUserStateKey(ACTIVE_KEY)),
  ]);
  const custom = parseCustomThemes(customRaw);
  const activeId = parseActiveThemeId(activeRaw);
  const next = resolve(activeId ?? useTheme.getState().themeId, custom);
  applyTheme(next);
  persistAppearance(next);
  useTheme.setState({ custom, themes: [...BUILTIN_THEMES, ...custom], themeId: next.id, theme: next });
}
