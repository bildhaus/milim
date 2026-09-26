const CUSTOMERS = new Map([
  ["c1", { id: "c1", tier: "gold", years: 3 }],
  ["c2", { id: "c2", tier: "silver", years: 1 }],
]);

export function loadCustomer(id) {
  const customer = CUSTOMERS.get(id);
  if (!customer) throw new Error(`unknown customer ${id}`);
  return customer;
}
