import { discountFor } from "./discounts.js";

export function priceCart(cart, customer) {
  const subtotal = cart.reduce((sum, line) => sum + line.unitCents * line.quantity, 0);
  const percent = discountFor(customer, subtotal);
  return { subtotal, percent, total: Math.round(subtotal * (1 - percent / 100)) };
}
