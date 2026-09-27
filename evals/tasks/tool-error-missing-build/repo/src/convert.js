import { RATES } from "./generated/rates.js";

function rate(code) {
  const entry = RATES[code];
  if (!entry) throw new RangeError(`unknown currency ${code}`);
  return entry;
}

/**
 * Convert `amount`, given in the minor units of `from` (cents for USD, yen
 * for JPY), into the minor units of `to`, rounded to the nearest unit.
 */
export function convert(amount, from, to) {
  const source = rate(from);
  const target = rate(to);
  const usd = amount / 10 ** source.minorUnits / source.ratePerUsd;
  return Math.round(usd * target.ratePerUsd * 10 ** target.minorUnits);
}

/** Format an amount in minor units, e.g. `format(1999, "USD")` is "19.99 USD". */
export function format(amount, code) {
  const { minorUnits } = rate(code);
  return `${(amount / 10 ** minorUnits).toFixed(minorUnits)} ${code}`;
}
