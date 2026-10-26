// Price formatting for the storefront. Amounts are integer cents.

const pad = (value) => String(value).padStart(2, "0");

/** 1234 -> "$12.34", 5 -> "$0.05", -5 -> "-$0.05", 123456 -> "$1,234.56". */
export function formatPrice(cents) {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  return `${sign}$${Math.floor(abs / 100).toLocaleString("en-US")}.${pad(abs % 100)}`;
}
