import { useEffect, useState } from "react";

const THEME_STORAGE_KEY = "milim-theme";
const themeModes = ["system", "dark", "light"] as const;

type ThemeMode = (typeof themeModes)[number];

function getStoredThemeMode(): ThemeMode {
  if (typeof window === "undefined") return "system";

  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    return themeModes.includes(stored as ThemeMode) ? (stored as ThemeMode) : "system";
  } catch {
    return "system";
  }
}

export function ThemeControl() {
  const [themeMode, setThemeMode] = useState<ThemeMode>(getStoredThemeMode);

  useEffect(() => {
    document.documentElement.dataset.theme = themeMode;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, themeMode);
    } catch {
      // Storage can be blocked; the active document still gets the selected theme.
    }
  }, [themeMode]);

  return (
    <div className="theme-control" aria-label="Color mode" data-theme-mode={themeMode} role="group">
      {themeModes.map((mode) => (
        <button
          type="button"
          aria-pressed={themeMode === mode}
          data-theme-mode={mode}
          onClick={() => setThemeMode(mode)}
          key={mode}
        >
          {mode}
        </button>
      ))}
    </div>
  );
}
