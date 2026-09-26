function money(cents, currency) {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const whole = Math.floor(abs / 100).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const fraction = String(abs % 100).padStart(2, "0");
  return `${sign}${currency === "EUR" ? "€" : "$"}${whole}.${fraction}`;
}

export function renderInvoice(invoice) {
  const lines = invoice.items.map(
    (item) => `${item.description}: ${money(item.quantity * item.unitCents, invoice.currency)}`,
  );
  const total = invoice.items.reduce((sum, item) => sum + item.quantity * item.unitCents, 0);
  return [`Invoice ${invoice.number}`, ...lines, `Total: ${money(total, invoice.currency)}`].join("\n");
}
