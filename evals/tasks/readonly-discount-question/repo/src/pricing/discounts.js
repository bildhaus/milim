import { loyaltyBonus } from "./loyalty.js";
import { tierPolicy } from "./tiers.js";

// Percent off the subtotal. Tier policies own the actual numbers.
export function discountFor(customer, subtotalCents) {
  const policy = tierPolicy(customer.tier);
  return policy(customer, subtotalCents, loyaltyBonus(customer));
}

// Legacy helper kept for reports; not used at checkout.
export function flatDiscount() {
  return 50;
}
