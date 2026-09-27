// Deterministic generators for the 1500+ line single-file fixtures.

/** Small linear congruential generator so output never depends on Math.random. */
export function rng(seed) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1_664_525) + 1_013_904_223) >>> 0;
    return state / 2 ** 32;
  };
}

const ADJECTIVES = ["Walnut", "Brass", "Linen", "Slate", "Cedar", "Copper", "Maple", "Onyx", "Ivory", "Ash"];
const NOUNS = ["Desk Lamp", "Bookshelf", "Side Table", "Planter", "Clock", "Mirror", "Stool", "Rug", "Vase", "Tray"];
const CATEGORIES = ["lighting", "storage", "furniture", "garden", "decor"];

/** A 1,600-entry product catalog with one entry per line. */
export function catalogFiles() {
  const next = rng(1_600);
  const rows = [];
  for (let index = 1; index <= 1_600; index += 1) {
    const sku = `SKU-${String(index).padStart(4, "0")}`;
    const name = `${ADJECTIVES[Math.floor(next() * ADJECTIVES.length)]} ${NOUNS[Math.floor(next() * NOUNS.length)]} ${index}`;
    const category = CATEGORIES[Math.floor(next() * CATEGORIES.length)];
    const priceCents = 500 + Math.floor(next() * 40_000);
    const stock = Math.floor(next() * 200);
    rows.push(`  { sku: "${sku}", name: "${name}", category: "${category}", priceCents: ${priceCents}, stock: ${stock} },`);
  }
  const source = `// Generated product catalog. One entry per line; keep that layout.
export const CATALOG = [
${rows.join("\n")}
];

export function findBySku(sku) {
  return CATALOG.find((entry) => entry.sku === sku);
}
`;
  return {
    "package.json": `${JSON.stringify({ name: "catalog-fixture", private: true, type: "module" }, null, 2)}\n`,
    "src/catalog.js": source,
  };
}

const UNITS = [
  ["Celsius", "Fahrenheit", "(value * 9) / 5 + 32", "°F"],
  ["Kilometers", "Miles", "value * 0.621371", "mi"],
  ["Kilograms", "Pounds", "value * 2.20462", "lb"],
  ["Liters", "Gallons", "value * 0.264172", "gal"],
  ["Meters", "Feet", "value * 3.28084", "ft"],
  ["Hours", "Minutes", "value * 60", "min"],
  ["Hectares", "Acres", "value * 2.47105", "ac"],
  ["Joules", "Calories", "value * 0.239006", "cal"],
];

function filler(index) {
  const [from, to, expression, suffix] = UNITS[index % UNITS.length];
  const digits = (index % 4) + 1;
  return `/**
 * Format a ${from.toLowerCase()} value as ${to.toLowerCase()} for report column ${index}.
 *
 * Rounds to ${digits} decimal place(s) and appends the "${suffix}" unit.
 * Non-finite input returns an em dash so tables stay aligned.
 *
 * @param {number} value
 * @returns {string}
 */
export function format${from}As${to}Column${index}(value) {
  if (!Number.isFinite(value)) {
    return "—";
  }
  const converted = ${expression};
  const rounded = converted.toFixed(${digits});
  return \`\${rounded} ${suffix}\`;
}
`;
}

const FORMAT_BYTES = `/**
 * Format a byte count with binary units: B, KiB, MiB, GiB, TiB.
 *
 * Uses powers of 1024 and one decimal place for every unit above B, so
 * 1536 becomes "1.5 KiB" and 1048576 becomes "1.0 MiB". Values below 1024
 * are whole bytes ("512 B"). Negative or non-finite input throws a
 * RangeError.
 *
 * @param {number} bytes
 * @returns {string}
 */
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes < 0) {
    throw new RangeError("bytes must be a non-negative finite number");
  }
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  if (bytes < 1024) {
    return \`\${Math.round(bytes)} B\`;
  }
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return \`\${value.toFixed(1)} \${units[unit]}\`;
}
`;

/** A 1,500+ line formatter module with one buggy function in the middle. */
export function formattersFiles() {
  const blocks = [];
  for (let index = 1; index <= 84; index += 1) {
    blocks.push(filler(index));
    if (index === 43) blocks.push(FORMAT_BYTES);
  }
  const source = `// Report column formatters. Each export is used by a dashboard column.

${blocks.join("\n")}`;
  return {
    "package.json": `${JSON.stringify({ name: "formatters-fixture", private: true, type: "module" }, null, 2)}\n`,
    "src/formatters.js": source,
  };
}
