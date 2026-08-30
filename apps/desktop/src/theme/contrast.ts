import type { Theme } from "./types";

function hexRgb(color: string): [number, number, number] | null {
  const raw = color.trim().replace(/^#/, "");
  const hex =
    raw.length === 3
      ? raw
          .split("")
          .map((char) => char + char)
          .join("")
      : raw;
  if (!/^[0-9a-f]{6}$/i.test(hex)) return null;
  return [
    Number.parseInt(hex.slice(0, 2), 16),
    Number.parseInt(hex.slice(2, 4), 16),
    Number.parseInt(hex.slice(4, 6), 16),
  ];
}

function luminance(color: string): number | null {
  const rgb = hexRgb(color);
  if (!rgb) return null;
  const [r, g, b] = rgb.map((channel) => {
    const value = channel / 255;
    return value <= 0.03928
      ? value / 12.92
      : Math.pow((value + 0.055) / 1.055, 2.4);
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(foreground: string, background: string): number | null {
  const fg = luminance(foreground);
  const bg = luminance(background);
  if (fg == null || bg == null) return null;
  const light = Math.max(fg, bg);
  const dark = Math.min(fg, bg);
  return (light + 0.05) / (dark + 0.05);
}

export function themeContrastIssues(theme: Theme): string[] {
  const checks: Array<[string, string, string]> = [
    ["Primary text", theme.colors.primaryText, theme.colors.bgPrimary],
    ["Primary text on panels", theme.colors.primaryText, theme.colors.bgSecondary],
    ["Muted text on panels", theme.colors.secondaryText, theme.colors.bgSecondary],
    ["Tertiary text", theme.colors.tertiaryText, theme.colors.bgPrimary],
    ["Tertiary text on panels", theme.colors.tertiaryText, theme.colors.bgSecondary],
    ["Input text", theme.colors.primaryText, theme.colors.inputBg],
    ["Placeholder text", theme.colors.placeholderText, theme.colors.inputBg],
  ];
  return checks.flatMap(([label, foreground, background]) => {
    const ratio = contrastRatio(foreground, background);
    return ratio == null || ratio >= 4.5
      ? []
      : [`${label} contrast is ${ratio.toFixed(1)}:1`];
  });
}
