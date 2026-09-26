import { priceCart } from "./pricing/cart.js";
import { loadCustomer } from "./customers/directory.js";

export function checkout(customerId, cart) {
  const customer = loadCustomer(customerId);
  return priceCart(cart, customer);
}
