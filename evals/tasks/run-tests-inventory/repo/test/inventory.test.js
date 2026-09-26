import { test } from "node:test";
import assert from "node:assert/strict";
import { Inventory } from "../src/inventory.js";

test("reserving exactly the available stock is allowed", () => {
  const inventory = new Inventory();
  inventory.receive("A", 3);
  inventory.reserve("A", 3);
  assert.equal(inventory.available("A"), 0);
});

test("a failed reservation leaves nothing reserved", () => {
  const inventory = new Inventory();
  inventory.receive("A", 2);
  assert.throws(() => inventory.reserve("A", 5));
  assert.equal(inventory.available("A"), 2);
});

test("shipping removes units from stock", () => {
  const inventory = new Inventory();
  inventory.receive("A", 10);
  inventory.reserve("A", 4);
  inventory.ship("A", 4);
  assert.equal(inventory.available("A"), 6);
  inventory.reserve("A", 6);
  assert.equal(inventory.available("A"), 0);
});
