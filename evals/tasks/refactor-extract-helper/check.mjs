import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { attempt, exists, expect, expectEqual, expectOnlyChanged, finish, headFile, load, read, repo, runNodeTests } from "../../lib/check.mjs";

const modules = ["src/invoice.js", "src/receipt.js", "src/statement.js"];

expect(read("test/render.test.js") === headFile("test/render.test.js"), "tests were modified");
expectOnlyChanged((path) => path.startsWith("src/") || path.startsWith("test/"));
expect(exists("src/money.js"), "src/money.js was not created");
runNodeTests("test");

for (const path of modules) {
  const source = read(path);
  expect(!/padStart\(2,\s*"0"\)/.test(source), `${path} still contains a local formatting copy`);
  expect(!/function (money|fmt)\b|const formatAmount\b/.test(source), `${path} still defines its old helper`);
}

// Every module must format through src/money.js, however it imports it
// (named, aliased, or namespace import): swap in a formatMoney that returns
// a marker and check each renderer's output carries it.
await attempt("modules use money.js", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "milim-eval-money-"));
  try {
    cpSync(join(repo, "src"), join(scratch, "src"), { recursive: true });
    writeFileSync(join(scratch, "package.json"), '{ "type": "module" }\n');
    writeFileSync(join(scratch, "src/money.js"), "export function formatMoney(cents, currency) {\n  return `<${cents} ${currency}>`;\n}\n");
    const url = (path) => pathToFileURL(join(scratch, path)).href;
    const { renderInvoice } = await import(url("src/invoice.js"));
    const { renderReceipt } = await import(url("src/receipt.js"));
    const { renderStatement } = await import(url("src/statement.js"));
    expect(
      renderInvoice({ number: "N", currency: "USD", items: [{ description: "W", quantity: 2, unitCents: 5 }] }).includes("<10 USD>"),
      "src/invoice.js does not format through money.js",
    );
    expect(renderReceipt({ cents: 7, currency: "EUR", payer: "P" }).includes("<7 EUR>"), "src/receipt.js does not format through money.js");
    expect(
      renderStatement({ owner: "O", currency: "USD", entries: [{ date: "d", cents: 3 }] }).includes("<3 USD>"),
      "src/statement.js does not format through money.js",
    );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

await attempt("formatMoney", async () => {
  const { formatMoney } = await load("src/money.js");
  expect(typeof formatMoney === "function", "money.js must export formatMoney");
  expectEqual(formatMoney(123456789, "USD"), "$1,234,567.89", "large USD");
  expectEqual(formatMoney(-5, "EUR"), "-€0.05", "negative EUR");
  expectEqual(formatMoney(0, "USD"), "$0.00", "zero");
});

finish();
