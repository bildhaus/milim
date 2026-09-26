export class Inventory {
  constructor() {
    this.stock = new Map();
    this.reserved = new Map();
  }

  receive(sku, quantity) {
    this.stock.set(sku, (this.stock.get(sku) ?? 0) + quantity);
  }

  available(sku) {
    return (this.stock.get(sku) ?? 0) - (this.reserved.get(sku) ?? 0);
  }

  /** Reserve `quantity` units, or throw without reserving anything. */
  reserve(sku, quantity) {
    this.reserved.set(sku, (this.reserved.get(sku) ?? 0) + quantity);
    if (this.available(sku) <= 0) {
      throw new Error(`insufficient stock for ${sku}`);
    }
  }

  /** Ship reserved units: they leave both stock and the reservation. */
  ship(sku, quantity) {
    const reserved = this.reserved.get(sku) ?? 0;
    if (quantity > reserved) {
      throw new Error(`only ${reserved} reserved for ${sku}`);
    }
    this.reserved.set(sku, reserved - quantity);
  }
}
