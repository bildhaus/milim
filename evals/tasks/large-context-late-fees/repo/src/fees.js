/**
 * Late fee, in cents, for an invoice that is still unpaid on `asOf`.
 *
 * The rules were agreed in the weekly product syncs; see docs/meetings/.
 *
 * @param {{ amountCents: number, dueDate: string, customerType: "standard" | "nonprofit" | "enterprise" }} invoice
 *   `amountCents` is a non-negative integer and `dueDate` is YYYY-MM-DD.
 * @param {string} asOf - The day the fee is computed for, as YYYY-MM-DD.
 * @returns {number} The fee in cents.
 */
export function lateFee(invoice, asOf) {
  throw new Error("not implemented");
}
