import type {AppearanceSnapshotV1} from './control/types';

export interface MobilePalette {
  bg: string;
  sidebar: string;
  panel: string;
  popover: string;
  input: string;
  raised: string;
  border: string;
  borderStrong: string;
  glassEdge: string;
  focus: string;
  text: string;
  secondary: string;
  muted: string;
  placeholder: string;
  accent: string;
  accentLight: string;
  accentSoft: string;
  accentBorder: string;
  accentInk: string;
  success: string;
  danger: string;
  dangerSurface: string;
  dangerBorder: string;
  warning: string;
}

export interface MobileTheme {
  appearance: AppearanceSnapshotV1;
  palette: MobilePalette;
  cardRadius: number;
  inputRadius: number;
  fontFamily?: string;
  monoFamily: string;
  isDark: boolean;
}

export type MobilePlatform = 'ios' | 'android';

export const defaultAppearance: AppearanceSnapshotV1 = {
  revision: 'builtin-mono-dark',
  theme_id: 'mono-dark',
  name: 'Mono Dark',
  is_dark: true,
  colors: {
    primary_text: '#ededf0',
    secondary_text: '#a0a0a8',
    tertiary_text: '#71717a',
    placeholder_text: '#71717a',
    bg_primary: '#0d0d0f',
    bg_secondary: '#161618',
    bg_tertiary: '#1f1f23',
    sidebar_bg: '#0a0a0c',
    accent: '#ededf0',
    accent_light: '#c8c8d0',
    border_primary: '#262629',
    border_secondary: '#323237',
    focus_border: '#55555e',
    success: '#34d399',
    warning: '#fbbf24',
    error: '#f87171',
    info: '#a0a0a8',
    card_bg: '#161618',
    card_border: '#262629',
    input_bg: '#161618',
    input_border: '#323237',
  },
  glass: {
    enabled: false,
    blur_radius: 24,
    opacity_primary: 1,
    opacity_secondary: 1,
    edge_light: 'rgba(255,255,255,0.08)',
  },
  background: {
    has_image: false,
    image_opacity: 1,
    image_blur: 0,
    overlay_color: null,
    overlay_opacity: 0,
    fit: 'cover',
    treatment: 'clear',
  },
  borders: {card_radius: 12, input_radius: 10, border_opacity: 1},
  typography: {
    font_family: 'system-ui, sans-serif',
    mono_family: 'ui-monospace, monospace',
  },
};

export function mobileBackgroundResizeMode(
  fit: string | null | undefined,
): 'cover' | 'contain' | 'center' | 'repeat' {
  switch (fit) {
    case 'contain':
      return 'contain';
    case 'center':
      return 'center';
    case 'tile':
      return 'repeat';
    case 'fill':
    case 'cover':
    default:
      return 'cover';
  }
}

function validColor(value: unknown, fallback: string): string {
  if (typeof value !== 'string') return fallback;
  const trimmed = value.trim();
  return /^(#[0-9a-f]{3,8}|rgba?\([^)]*\)|hsla?\([^)]*\)|[a-z]+)$/i.test(trimmed)
    ? trimmed
    : fallback;
}

function opacity(value: unknown, fallback = 1): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.max(0, Math.min(1, value))
    : fallback;
}

function radius(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.max(0, Math.min(32, value))
    : fallback;
}

function hexRgb(value: string): [number, number, number] | null {
  const hex = value.trim().replace(/^#/, '');
  if (!/^(?:[0-9a-f]{3}|[0-9a-f]{6})$/i.test(hex)) return null;
  const expanded = hex.length === 3 ? hex.split('').map(part => part + part).join('') : hex;
  return [
    Number.parseInt(expanded.slice(0, 2), 16),
    Number.parseInt(expanded.slice(2, 4), 16),
    Number.parseInt(expanded.slice(4, 6), 16),
  ];
}

function withAlpha(value: string, alpha: number): string {
  const rgb = hexRgb(value);
  if (!rgb) return value;
  return `rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, ${opacity(alpha)})`;
}

function contrastInk(accent: string, isDark: boolean): string {
  const rgb = hexRgb(accent);
  if (!rgb) return isDark ? '#0d0d0f' : '#ffffff';
  const luminance = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]) / 255;
  return luminance > 0.55 ? '#0d0d0f' : '#ffffff';
}

function fontNames(stack: string): string[] {
  return stack
    .split(',')
    .map(value => value.trim().replace(/^["']|["']$/g, ''))
    .filter(Boolean);
}

export function mobileFontFamily(
  stack: string,
  platform: MobilePlatform,
  monospace = false,
): string | undefined {
  const names = fontNames(stack);
  const normalized = names.map(name => name.toLowerCase());
  if (monospace) {
    return platform === 'ios' ? 'Menlo' : 'monospace';
  }
  if (normalized.some(name => name === 'georgia' || name === 'times new roman' || name === 'serif')) {
    return platform === 'ios'
      ? names.find(name => ['georgia', 'times new roman'].includes(name.toLowerCase())) ?? 'Times New Roman'
      : 'serif';
  }
  if (platform === 'ios') {
    return names.find(name => ['helvetica', 'arial', 'verdana'].includes(name.toLowerCase()));
  }
  return 'sans-serif';
}

export function createMobileTheme(
  snapshot?: AppearanceSnapshotV1,
  platform: MobilePlatform = 'android',
): MobileTheme {
  const appearance = snapshot ?? defaultAppearance;
  const defaults = defaultAppearance.colors;
  const colors = appearance.colors ?? defaults;
  const isDark = appearance.is_dark ?? true;
  const borders = appearance.borders ?? defaultAppearance.borders;
  const glass = appearance.glass ?? defaultAppearance.glass;
  const bgPrimary = validColor(colors.bg_primary, defaults.bg_primary);
  const card = validColor(colors.card_bg, defaults.card_bg);
  const input = validColor(colors.input_bg, defaults.input_bg);
  const panelOpacity = glass.enabled ? opacity(glass.opacity_primary, 0.92) : 1;
  const raisedOpacity = glass.enabled ? opacity(glass.opacity_secondary, 0.88) : 1;
  const popoverOpacity = glass.enabled
    ? Math.min(Math.max(panelOpacity + 0.3, 0.94), 0.98)
    : 1;
  const borderOpacity = opacity(borders.border_opacity);
  const accent = validColor(colors.accent, defaults.accent);
  const danger = validColor(colors.error, defaults.error);

  return {
    appearance,
    isDark,
    cardRadius: radius(borders.card_radius, 12),
    inputRadius: radius(borders.input_radius, 10),
    fontFamily: mobileFontFamily(appearance.typography.font_family, platform),
    monoFamily: mobileFontFamily(appearance.typography.mono_family, platform, true) ?? 'monospace',
    palette: {
      bg: bgPrimary,
      sidebar: withAlpha(validColor(colors.sidebar_bg, defaults.sidebar_bg), panelOpacity),
      panel: withAlpha(card, panelOpacity),
      popover: withAlpha(validColor(colors.bg_secondary, defaults.bg_secondary), popoverOpacity),
      input: withAlpha(input, glass.enabled ? Math.min(panelOpacity + 0.08, 1) : 1),
      raised: withAlpha(validColor(colors.bg_tertiary, defaults.bg_tertiary), raisedOpacity),
      border: withAlpha(validColor(colors.card_border, defaults.card_border), borderOpacity),
      borderStrong: withAlpha(validColor(colors.input_border, defaults.input_border), borderOpacity),
      glassEdge: validColor(glass.edge_light, 'rgba(255,255,255,0.08)'),
      focus: validColor(colors.focus_border, defaults.focus_border),
      text: validColor(colors.primary_text, defaults.primary_text),
      secondary: validColor(colors.secondary_text, defaults.secondary_text),
      muted: validColor(colors.tertiary_text, defaults.tertiary_text),
      placeholder: validColor(colors.placeholder_text, defaults.placeholder_text),
      accent,
      accentLight: validColor(colors.accent_light, defaults.accent_light),
      accentSoft: withAlpha(accent, 0.15),
      accentBorder: withAlpha(accent, 0.34),
      accentInk: contrastInk(accent, isDark),
      success: validColor(colors.success, defaults.success),
      danger,
      dangerSurface: withAlpha(danger, 0.16),
      dangerBorder: withAlpha(danger, 0.46),
      warning: validColor(colors.warning, defaults.warning),
    },
  };
}
