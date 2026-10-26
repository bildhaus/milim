// Shared by the solution scripts: the fix for each failing test, applied to
// the repository in the current directory.
import { readFileSync, writeFileSync } from "node:fs";

function replace(path, from, to) {
  const source = readFileSync(path, "utf8");
  if (!source.includes(from)) throw new Error(`${path}: expected text not found`);
  writeFileSync(path, source.replace(from, () => to));
}

export const fixes = {
  cart: () => replace("src/cart.js", "return Math.floor(gross * (1 - discount));", "return Math.round(gross * (1 - discount));"),
  dates: () =>
    replace(
      "src/dates.js",
      `  const value = parse(date);
  const start = Date.UTC(value.getUTCFullYear(), 0, 1);
  const dayOfYear = Math.floor((value - start) / DAY_MS) + 1;
  return Math.ceil(dayOfYear / 7);`,
      `  // The Thursday of this ISO week decides the ISO year.
  const thursday = new Date(parse(date).getTime() + (4 - isoWeekday(date)) * DAY_MS);
  const yearStart = Date.UTC(thursday.getUTCFullYear(), 0, 1);
  return Math.floor((thursday - yearStart) / DAY_MS / 7) + 1;`,
    ),
  sku: () =>
    replace(
      "src/sku.js",
      "    const sku = row.sku;",
      "    const sku = String(row.sku).trim().toUpperCase();",
    ),
};

export const describe = {
  cart: "test/cart.test.js \"cart total rounds half-cent totals up\": cartTotal floors instead of rounding half up",
  dates: "test/dates.test.js \"early January can belong to the previous ISO year's last week\": isoWeek counts days from Jan 1 instead of using ISO weeks",
  sku: "test/sku.test.js \"duplicate SKUs that differ only by case or spaces are reported once\": findDuplicateSkus compares raw values",
};

/** A numbered turn-1 answer listing the failures in `order`. */
export const listing = (order) =>
  `Three tests fail:\n\n${order.map((key, index) => `${index + 1}. ${describe[key]}`).join("\n")}\n\nI have not changed anything.`;
