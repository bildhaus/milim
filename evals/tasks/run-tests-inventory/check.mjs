import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

for (const path of ["test/inventory.test.js", "test/report.test.js"]) {
  expect(read(path) === headFile(path), `${path} was modified`);
}
expectOnlyChanged((path) => path.startsWith("src/"));
runNodeTests("test");

await attempt("hidden inventory cases", async () => {
  const { Inventory } = await load("src/inventory.js");
  const inventory = new Inventory();
  inventory.receive("B", 5);
  inventory.reserve("B", 2);
  inventory.ship("B", 1);
  expectEqual(inventory.available("B"), 3, "partial ship");
  let threw = false;
  try {
    inventory.reserve("B", 4);
  } catch {
    threw = true;
  }
  expect(threw, "over-reservation throws");
  expectEqual(inventory.available("B"), 3, "failed reservation keeps availability");
});

await attempt("hidden report cases", async () => {
  const { weeklyTotals } = await load("src/report.js");
  expectEqual(weeklyTotals([{ date: "2024-12-29", quantity: 1 }, { date: "2024-12-30", quantity: 2 }]), { "2024-12-23": 1, "2024-12-30": 2 }, "year boundary Sunday");
});

finish();
