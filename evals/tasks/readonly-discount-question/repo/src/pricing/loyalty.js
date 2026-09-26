// One point per full year as a customer, up to 8.
export function loyaltyBonus(customer) {
  return Math.min(Math.floor(customer.years ?? 0), 8);
}
