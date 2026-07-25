import { contrastRatio } from "../theme/contrast.js";

const GOLDEN_ANGLE = 137.508;
const MIN_TEXT_CONTRAST = 4.5;

type ProjectColorSource = {
  folder: string;
  color?: string;
};

type ProjectColorTheme = {
  accent: string;
  sidebarBackground: string;
  auto: boolean;
};

type Rgb = { r: number; g: number; b: number };
type Hsl = { h: number; s: number; l: number };

export function normalizeProjectColor(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  let hex = value.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(hex)) {
    hex = hex.split("").map((char) => char + char).join("");
  }
  return /^[0-9a-f]{6}$/i.test(hex) ? `#${hex.toLowerCase()}` : undefined;
}

function hexToRgb(value: string): Rgb | null {
  const hex = normalizeProjectColor(value);
  if (!hex) return null;
  return {
    r: Number.parseInt(hex.slice(1, 3), 16),
    g: Number.parseInt(hex.slice(3, 5), 16),
    b: Number.parseInt(hex.slice(5, 7), 16),
  };
}

function rgbToHsl({ r, g, b }: Rgb): Hsl {
  const red = r / 255;
  const green = g / 255;
  const blue = b / 255;
  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const delta = max - min;
  const l = (max + min) / 2;
  if (delta === 0) return { h: 0, s: 0, l };
  const s = delta / (1 - Math.abs(2 * l - 1));
  const h = max === red
    ? 60 * (((green - blue) / delta) % 6)
    : max === green
      ? 60 * ((blue - red) / delta + 2)
      : 60 * ((red - green) / delta + 4);
  return { h: h < 0 ? h + 360 : h, s, l };
}

function hslToHex({ h, s, l }: Hsl): string {
  const chroma = (1 - Math.abs(2 * l - 1)) * s;
  const x = chroma * (1 - Math.abs(((h / 60) % 2) - 1));
  const offset = l - chroma / 2;
  let red = 0;
  let green = 0;
  let blue = 0;
  if (h < 60) [red, green] = [chroma, x];
  else if (h < 120) [red, green] = [x, chroma];
  else if (h < 180) [green, blue] = [chroma, x];
  else if (h < 240) [green, blue] = [x, chroma];
  else if (h < 300) [red, blue] = [x, chroma];
  else [red, blue] = [chroma, x];
  return `#${[red, green, blue]
    .map((channel) => Math.round((channel + offset) * 255).toString(16).padStart(2, "0"))
    .join("")}`;
}

function stableColorSlot(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % 24;
}

export function ensureProjectColorContrast(
  color: string,
  background: string,
): string | undefined {
  const foreground = normalizeProjectColor(color);
  const normalizedBackground = normalizeProjectColor(background);
  if (!foreground || !normalizedBackground) return foreground;
  if ((contrastRatio(foreground, normalizedBackground) ?? 0) >= MIN_TEXT_CONTRAST) {
    return foreground;
  }
  const hsl = rgbToHsl(hexToRgb(foreground)!);
  const lightContrast = contrastRatio("#ffffff", normalizedBackground) ?? 0;
  const darkContrast = contrastRatio("#000000", normalizedBackground) ?? 0;
  const towardLight = lightContrast >= darkContrast;
  for (let step = 1; step <= 20; step += 1) {
    const amount = step / 20;
    const candidate = hslToHex({
      ...hsl,
      l: towardLight ? hsl.l + (1 - hsl.l) * amount : hsl.l * (1 - amount),
    });
    if ((contrastRatio(candidate, normalizedBackground) ?? 0) >= MIN_TEXT_CONTRAST) {
      return candidate;
    }
  }
  return towardLight ? "#ffffff" : "#000000";
}

export function automaticProjectColor(
  folder: string,
  accent: string,
  sidebarBackground: string,
): string {
  const accentHsl = rgbToHsl(hexToRgb(accent) ?? { r: 124, g: 140, b: 255 });
  const color = hslToHex({
    h: (accentHsl.h + stableColorSlot(folder) * GOLDEN_ANGLE) % 360,
    s: Math.min(0.78, Math.max(0.6, accentHsl.s)),
    l: accentHsl.l,
  });
  return ensureProjectColorContrast(color, sidebarBackground) ?? color;
}

export function effectiveProjectColor(
  project: ProjectColorSource | null | undefined,
  theme: ProjectColorTheme,
): string | undefined {
  if (!project) return undefined;
  const custom = normalizeProjectColor(project.color);
  if (custom) return ensureProjectColorContrast(custom, theme.sidebarBackground);
  return theme.auto
    ? automaticProjectColor(project.folder, theme.accent, theme.sidebarBackground)
    : undefined;
}
