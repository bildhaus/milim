import { DEFAULT_TYPOGRAPHY, type Theme } from "./types.js";

// Only two built-ins - clean neutral monochrome. Everything else (background
// image, accent + palette, fonts, glass) is user-customizable via the editor.

const monoDark: Theme = {
  id: "mono-dark",
  name: "Mono Dark",
  isDark: true,
  colors: {
    primaryText: "#ededf0",
    secondaryText: "#a0a0a8",
    tertiaryText: "#82828c",
    placeholderText: "#82828c",
    bgPrimary: "#0d0d0f",
    bgSecondary: "#161618",
    bgTertiary: "#1f1f23",
    sidebarBg: "#0a0a0c",
    accent: "#ededf0",
    accentLight: "#c8c8d0",
    borderPrimary: "#262629",
    borderSecondary: "#323237",
    focusBorder: "#55555e",
    success: "#34d399",
    warning: "#fbbf24",
    error: "#f87171",
    info: "#a0a0a8",
    cardBg: "#161618",
    cardBorder: "#262629",
    inputBg: "#161618",
    inputBorder: "#323237",
  },
  glass: { enabled: false, blurRadius: 24, opacityPrimary: 1, opacitySecondary: 1, edgeLight: "rgba(255,255,255,0.08)" },
  background: { imageOpacity: 1, overlayOpacity: 0 },
  borders: { cardRadius: 12, inputRadius: 10, borderOpacity: 1 },
  typography: DEFAULT_TYPOGRAPHY,
};

const monoLight: Theme = {
  id: "mono-light",
  name: "Mono Light",
  isDark: false,
  colors: {
    primaryText: "#18181b",
    secondaryText: "#55555c",
    tertiaryText: "#707078",
    placeholderText: "#707078",
    bgPrimary: "#ffffff",
    bgSecondary: "#f6f6f7",
    bgTertiary: "#ededee",
    sidebarBg: "#f3f3f4",
    accent: "#18181b",
    accentLight: "#3f3f46",
    borderPrimary: "#e4e4e7",
    borderSecondary: "#d4d4d8",
    focusBorder: "#a0a0a8",
    success: "#15803d",
    warning: "#a16207",
    error: "#dc2626",
    info: "#55555c",
    cardBg: "#ffffff",
    cardBorder: "#e6e6e9",
    inputBg: "#ffffff",
    inputBorder: "#d4d4d8",
  },
  glass: { enabled: false, blurRadius: 24, opacityPrimary: 1, opacitySecondary: 1, edgeLight: "rgba(255,255,255,0.6)" },
  background: { imageOpacity: 1, overlayOpacity: 0 },
  borders: { cardRadius: 12, inputRadius: 10, borderOpacity: 1 },
  typography: DEFAULT_TYPOGRAPHY,
};

export const BUILTIN_THEMES: Theme[] = [monoDark, monoLight];
export const DEFAULT_THEME_ID = monoDark.id;

export function themeById(id: string): Theme {
  return BUILTIN_THEMES.find((t) => t.id === id) ?? BUILTIN_THEMES[0];
}
