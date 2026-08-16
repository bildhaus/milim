import type { Theme } from "./types.js";

export type AppearanceBackgroundFitV1 = "cover" | "contain" | "tile" | "center";
export type AppearanceBackgroundTreatmentV1 = "clear" | "dim" | "blur" | "mono";

export interface AppearancePreferencesV1 {
  backgroundFit: AppearanceBackgroundFitV1;
  backgroundTreatment: AppearanceBackgroundTreatmentV1;
}

export interface AppearanceSnapshotV1 {
  revision: string;
  theme_id: string;
  name: string;
  is_dark: boolean;
  colors: {
    primary_text: string;
    secondary_text: string;
    tertiary_text: string;
    placeholder_text: string;
    bg_primary: string;
    bg_secondary: string;
    bg_tertiary: string;
    sidebar_bg: string;
    accent: string;
    accent_light: string;
    border_primary: string;
    border_secondary: string;
    focus_border: string;
    success: string;
    warning: string;
    error: string;
    info: string;
    card_bg: string;
    card_border: string;
    input_bg: string;
    input_border: string;
  };
  glass: {
    enabled: boolean;
    blur_radius: number;
    opacity_primary: number;
    opacity_secondary: number;
    edge_light: string;
  };
  background: {
    has_image: boolean;
    image_opacity: number;
    image_blur: number;
    overlay_color: string | null;
    overlay_opacity: number;
    fit: AppearanceBackgroundFitV1;
    treatment: AppearanceBackgroundTreatmentV1;
  };
  borders: {
    card_radius: number;
    input_radius: number;
    border_opacity: number;
  };
  typography: {
    font_family: string;
    mono_family: string;
  };
}

function fnv1a(value: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `fnv1a-${(hash >>> 0).toString(16).padStart(8, "0")}`;
}

export function appearanceSnapshot(
  theme: Theme,
  preferences: AppearancePreferencesV1 = {
    backgroundFit: "cover",
    backgroundTreatment: "clear",
  },
): AppearanceSnapshotV1 {
  const value = {
    theme_id: theme.id,
    name: theme.name,
    is_dark: theme.isDark,
    colors: {
      primary_text: theme.colors.primaryText,
      secondary_text: theme.colors.secondaryText,
      tertiary_text: theme.colors.tertiaryText,
      placeholder_text: theme.colors.placeholderText,
      bg_primary: theme.colors.bgPrimary,
      bg_secondary: theme.colors.bgSecondary,
      bg_tertiary: theme.colors.bgTertiary,
      sidebar_bg: theme.colors.sidebarBg,
      accent: theme.colors.accent,
      accent_light: theme.colors.accentLight,
      border_primary: theme.colors.borderPrimary,
      border_secondary: theme.colors.borderSecondary,
      focus_border: theme.colors.focusBorder,
      success: theme.colors.success,
      warning: theme.colors.warning,
      error: theme.colors.error,
      info: theme.colors.info,
      card_bg: theme.colors.cardBg,
      card_border: theme.colors.cardBorder,
      input_bg: theme.colors.inputBg,
      input_border: theme.colors.inputBorder,
    },
    glass: {
      enabled: theme.glass.enabled,
      blur_radius: theme.glass.blurRadius,
      opacity_primary: theme.glass.opacityPrimary,
      opacity_secondary: theme.glass.opacitySecondary,
      edge_light: theme.glass.edgeLight,
    },
    background: {
      has_image: Boolean(theme.background.image),
      image_opacity: theme.background.imageOpacity,
      image_blur: theme.background.imageBlur ?? 0,
      overlay_color: theme.background.overlayColor ?? null,
      overlay_opacity: theme.background.overlayOpacity,
      fit: preferences.backgroundFit,
      treatment: preferences.backgroundTreatment,
    },
    borders: {
      card_radius: theme.borders.cardRadius,
      input_radius: theme.borders.inputRadius,
      border_opacity: theme.borders.borderOpacity,
    },
    typography: {
      font_family: theme.typography.fontFamily,
      mono_family: theme.typography.monoFamily,
    },
  };
  return { revision: fnv1a(JSON.stringify(value)), ...value };
}
