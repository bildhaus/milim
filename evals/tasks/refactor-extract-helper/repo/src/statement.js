export function renderStatement(account) {
  let balance = 0;
  const rows = account.entries.map((entry) => {
    balance += entry.cents;
    return `${entry.date}  ${fmt(entry.cents, account.currency)}  ${fmt(balance, account.currency)}`;
  });
  return [`Statement for ${account.owner}`, ...rows].join("\n");
}

function fmt(cents, currency) {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const whole = Math.floor(abs / 100).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const fraction = String(abs % 100).padStart(2, "0");
  return `${sign}${currency === "EUR" ? "€" : "$"}${whole}.${fraction}`;
}
