// Price formatting for the storefront. Amounts are integer cents.

/** Sign, dollars (with thousands separators), and cents of an amount. */
function toParts(cents) {
  return {
    sign: cents < 0 ? "-" : "",
    dollars: Math.floor(Math.abs(cents) / 100).toLocaleString("en-US"),
    cents: String(cents % 100),
  };
}

/** 1234 -> "$12.34", 5 -> "$0.05", -5 -> "-$0.05", 123456 -> "$1,234.56". */
export function formatPrice(cents) {
  const parts = toParts(cents);
  return `${parts.sign}$${parts.dollars}.${parts.cents}`;
}

/** A price range such as "$5.00–$12.50"; equal ends collapse to one price. */
export function formatRange(low, high) {
  if (low > high) throw new RangeError("low must not exceed high");
  if (low === high) return formatPrice(low);
  return `${formatPrice(low)}–${formatPrice(high)}`;
}
