import assert from "node:assert/strict";
import test from "node:test";
import { findDuplicateSkus, isValidSku } from "../src/sku.js";

test("valid SKUs look like ABC-1234", () => {
  assert.equal(isValidSku("abc-1234"), true);
  assert.equal(isValidSku("AB-1234"), false);
});

test("duplicate SKUs that differ only by case or spaces are reported once", () => {
  const rows = [{ sku: "ABC-0001" }, { sku: "abc-0001 " }, { sku: "XYZ-0002" }, { sku: "Abc-0001" }];
  assert.deepEqual(findDuplicateSkus(rows), ["ABC-0001"]);
});
