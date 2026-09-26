const formatAmount = (cents, currency) => {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const whole = Math.floor(abs / 100).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const fraction = String(abs % 100).padStart(2, "0");
  return `${sign}${currency === "EUR" ? "€" : "$"}${whole}.${fraction}`;
};

export function renderReceipt(payment) {
  return `Received ${formatAmount(payment.cents, payment.currency)} from ${payment.payer}`;
}

export function renderRefund(payment) {
  return `Refunded ${formatAmount(-payment.cents, payment.currency)} to ${payment.payer}`;
}
