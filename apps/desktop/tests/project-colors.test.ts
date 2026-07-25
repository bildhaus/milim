import {
  automaticProjectColor,
  effectiveProjectColor,
  normalizeProjectColor,
} from "../src/lib/projectColors.js";
import { contrastRatio } from "../src/theme/contrast.js";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function equal<T>(actual: T, expected: T, message: string): void {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

equal(normalizeProjectColor("#A3C"), "#aa33cc", "short hex colors should normalize");
equal(normalizeProjectColor("invalid"), undefined, "invalid colors should be rejected");

const darkBackground = "#18181b";
const first = automaticProjectColor("C:\\workspace-a", "#8b5cf6", darkBackground);
const repeated = automaticProjectColor("C:\\workspace-a", "#8b5cf6", darkBackground);
const second = automaticProjectColor("C:\\workspace-b", "#8b5cf6", darkBackground);
equal(first, repeated, "automatic project colors should be stable");
assert(first !== second, "different projects should receive distinct palette colors");
assert(
  (contrastRatio(first, darkBackground) ?? 0) >= 4.5,
  "automatic project colors should meet text contrast",
);

const changedTheme = automaticProjectColor("C:\\workspace-a", "#22c55e", darkBackground);
assert(first !== changedTheme, "automatic project colors should follow the theme accent");

equal(
  effectiveProjectColor(
    { folder: "C:\\workspace-a", color: "#ffffff" },
    { accent: "#8b5cf6", sidebarBackground: darkBackground, auto: true },
  ),
  "#ffffff",
  "manual project colors should override automatic colors",
);
equal(
  effectiveProjectColor(
    { folder: "C:\\workspace-a" },
    { accent: "#8b5cf6", sidebarBackground: darkBackground, auto: false },
  ),
  undefined,
  "uncolored projects should remain default when automatic coloring is off",
);

export {};
