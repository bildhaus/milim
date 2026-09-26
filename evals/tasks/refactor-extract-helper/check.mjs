import { attempt, exists, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, runNodeTests } from "../../lib/check.mjs";

const modules = ["src/invoice.js", "src/receipt.js", "src/statement.js"];

expect(read("test/render.test.js") === headFile("test/render.test.js"), "tests were modified");
expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/"));
expect(exists("src/money.js"), "src/money.js was not created");
runNodeTests("test");

for (const path of modules) {
  const source = read(path);
  expect(/import\s*\{[^}]*\bformatMoney\b[^}]*\}\s*from\s*["']\.\/money\.js["']/.test(source), `${path} does not import formatMoney from ./money.js`);
  expect(!/padStart\(2,\s*"0"\)/.test(source), `${path} still contains a local formatting copy`);
  expect(!/function (money|fmt)\b|const formatAmount\b/.test(source), `${path} still defines its old helper`);
}

await attempt("formatMoney", async () => {
  const { formatMoney } = await load("src/money.js");
  expect(typeof formatMoney === "function", "money.js must export formatMoney");
  expectEqual(formatMoney(123456789, "USD"), "$1,234,567.89", "large USD");
  expectEqual(formatMoney(-5, "EUR"), "-€0.05", "negative EUR");
  expectEqual(formatMoney(0, "USD"), "$0.00", "zero");
});

finish();
