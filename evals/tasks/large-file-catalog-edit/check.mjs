import { attempt, expect, expectEqual, expectOnlyChanged, finish, headFile, lines, load, read } from "../../lib/check.mjs";

const PATH = "src/catalog.js";
expectOnlyChanged([PATH]);

const before = lines(headFile(PATH));
const after = lines(read(PATH));
expectEqual(after.length, before.length, "line count");
const changed = [];
for (let index = 0; index < Math.max(before.length, after.length); index += 1) {
  if (before[index] !== after[index]) changed.push(index);
}
const allowed = new Set(
  before.flatMap((line, index) => (/sku: "SKU-(0917|1203)"/.test(line) ? [index] : [])),
);
expectEqual(allowed.size, 2, "fixture target lines");
const stray = changed.filter((index) => !allowed.has(index));
expect(stray.length === 0, `lines changed outside the two targets: ${stray.slice(0, 5).map((index) => index + 1).join(", ")}`);

await attempt("catalog values", async () => {
  const { CATALOG, findBySku } = await load(PATH);
  expectEqual(CATALOG.length, 1600, "entry count");
  const price = findBySku("SKU-0917");
  const retired = findBySku("SKU-1203");
  expectEqual(price?.priceCents, 2499, "SKU-0917 price");
  expectEqual(retired?.stock, 0, "SKU-1203 stock");
  expectEqual(retired?.discontinued, true, "SKU-1203 discontinued");
  const original = /stock: (\d+)/.exec(before.find((line) => line.includes('"SKU-0917"')))[1];
  expectEqual(price?.stock, Number(original), "SKU-0917 stock untouched");
  expect(!("discontinued" in price), "SKU-0917 must not be discontinued");
});

finish();
