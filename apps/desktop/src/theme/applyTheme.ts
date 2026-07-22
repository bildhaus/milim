import type { Theme } from "./types";

export function animatedBackgroundSource(image: string | undefined): string | undefined {
  const match = image?.trim().match(/^url\(\s*(["']?)(data:image\/gif(?:;[^,]*)?,[^)]*)\1\s*\)$/i);
  return match?.[2];
}

/** Convert `#rrggbb` (or `#rgb`) + alpha -> `rgba(...)`. Non-hex passes through. */
function rgba(hex: string, alpha: number): string {
  let h = hex.trim();
  if (!h.startsWith("#")) return h;
  h = h.slice(1);
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export function themeCssVariables(t: Theme): Record<string, string> {
  const c = t.colors;
  const g = t.glass;
  const op = g.enabled ? g.opacityPrimary : 1;
  const op2 = g.enabled ? g.opacitySecondary : 1;
  const popoverOp = g.enabled ? Math.min(op + 0.08, 0.88) : 1;

  return {
    "--primary-text": c.primaryText,
    "--secondary-text": c.secondaryText,
    "--tertiary-text": c.tertiaryText,
    "--placeholder-text": c.placeholderText,
    "--bg-primary": c.bgPrimary,
    "--bg-secondary": c.bgSecondary,
    "--bg-tertiary": c.bgTertiary,
    "--accent": c.accent,
    "--accent-light": c.accentLight,
    "--accent-soft": rgba(c.accent, 0.14),
    "--accent-15": rgba(c.accent, 0.16),
    "--accent-glow": rgba(c.accent, 0.38),
    "--border-primary": c.borderPrimary,
    "--border-secondary": c.borderSecondary,
    "--focus-border": c.focusBorder,
    "--success": c.success,
    "--warning": c.warning,
    "--error": c.error,
    "--accent-contrast": t.isDark && isLight(c.accent) ? "#10131a" : t.isDark ? "#0b0b0b" : "#ffffff",

    // Derived panel surfaces - translucent + blurred when glass is enabled, so a
    // background image/gradient shows through; otherwise solid.
    "--sidebar-bg": rgba(c.sidebarBg, op),
    "--panel-bg": rgba(c.bgSecondary, op),
    "--popover-bg": rgba(c.bgSecondary, popoverOp),
    "--popover-border": c.borderPrimary,
    "--popover-blur": g.enabled ? `${g.blurRadius}px` : "0px",
    "--panel-hover": rgba(c.bgTertiary, Math.min(op + 0.1, 1)),
    "--card-bg": rgba(c.cardBg, op),
    "--card-border": c.cardBorder,
    "--chip-bg": rgba(c.bgTertiary, op2),
    "--chip-hover": rgba(c.bgTertiary, Math.min(op2 + 0.18, 1)),
    "--input-bg": rgba(c.inputBg, g.enabled ? Math.min(op + 0.08, 1) : 1),
    "--input-border": c.inputBorder,
    "--topbar-bg": g.enabled ? rgba(c.bgPrimary, 0.5) : c.bgPrimary,
    "--glass-edge": g.edgeLight,
    "--blur": g.enabled ? `${g.blurRadius}px` : "0px",

    // Background layer
    "--bg-image": t.background.image ?? "none",
    "--bg-image-opacity": String(t.background.imageOpacity),
    "--bg-image-blur": `${t.background.imageBlur ?? 0}px`,
    "--overlay-color": t.background.overlayColor ?? "#000000",
    "--overlay-opacity": String(t.background.overlayOpacity),

    // Borders + radii
    "--card-radius": `${t.borders.cardRadius}px`,
    "--input-radius": `${t.borders.inputRadius}px`,
    "--chip-radius": `${Math.max(0, t.borders.inputRadius - 2)}px`,
    "--popover-radius": `${t.borders.cardRadius}px`,

    // Typography
    "--font": t.typography.fontFamily,
    "--mono": t.typography.monoFamily,
  };
}

/** Write a theme onto `:root` as CSS variables. */
export function applyTheme(t: Theme): void {
  const s = document.documentElement.style;
  for (const [key, value] of Object.entries(themeCssVariables(t))) {
    s.setProperty(key, value);
  }

  const animatedBackground = animatedBackgroundSource(t.background.image);
  document.documentElement.dataset.animatedBackground = String(Boolean(animatedBackground));
  document.querySelectorAll?.<HTMLImageElement>("[data-theme-background-gif]").forEach((image) => {
    if (animatedBackground) {
      if (image.getAttribute("src") !== animatedBackground) image.src = animatedBackground;
      image.hidden = false;
    } else {
      image.removeAttribute("src");
      image.hidden = true;
    }
  });

  document.documentElement.dataset.dark = String(t.isDark);
  document.documentElement.style.colorScheme = t.isDark ? "dark" : "light";
}

/** Rough luminance check so a light accent gets dark text on it. */
function isLight(hex: string): boolean {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  const r = (n >> 16) & 255;
  const gg = (n >> 8) & 255;
  const b = n & 255;
  return (0.299 * r + 0.587 * gg + 0.114 * b) / 255 > 0.6;
}
