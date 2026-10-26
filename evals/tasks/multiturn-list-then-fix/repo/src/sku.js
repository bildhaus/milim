// SKU validation for catalog imports.

const SKU = /^[A-Z]{3}-\d{4}$/;

/** Whether `sku` looks like "ABC-1234" (case-insensitive, surrounding spaces allowed). */
export function isValidSku(sku) {
  return SKU.test(String(sku).trim().toUpperCase());
}

/**
 * SKUs that appear more than once in `rows` ({ sku }), compared without
 * case and surrounding whitespace, each reported once in canonical form
 * (trimmed, upper case) in order of first appearance.
 */
export function findDuplicateSkus(rows) {
  const seen = new Set();
  const duplicates = [];
  for (const row of rows) {
    const sku = row.sku;
    if (seen.has(sku) && !duplicates.includes(sku)) duplicates.push(sku);
    seen.add(sku);
  }
  return duplicates;
}
