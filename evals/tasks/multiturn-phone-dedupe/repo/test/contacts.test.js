import { test } from "node:test";
import assert from "node:assert/strict";
import { sortContacts } from "../src/contacts.js";

test("sorts by name", () => {
  const list = [{ name: "Bo", phone: "1" }, { name: "Al", phone: "2" }];
  assert.deepEqual(sortContacts(list).map((contact) => contact.name), ["Al", "Bo"]);
  assert.equal(list[0].name, "Bo");
});
