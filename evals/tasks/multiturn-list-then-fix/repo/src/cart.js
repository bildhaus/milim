// Cart totals. Prices are integer cents; a discount is a fraction (0.15 = 15%).

/** Total of `items` ({ cents, qty }) after `discount`, rounded half up to whole cents. */
export function cartTotal(items, discount = 0) {
  if (!(discount >= 0 && discount < 1)) throw new RangeError("discount must be in [0, 1)");
  const gross = items.reduce((sum, item) => sum + item.cents * item.qty, 0);
  return Math.floor(gross * (1 - discount));
}

/** Format integer cents as dollars, e.g. 1234 -> "$12.34". */
export function formatCents(cents) {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  return `${sign}$${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
}
