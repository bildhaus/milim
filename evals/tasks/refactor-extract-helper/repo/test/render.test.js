import { test } from "node:test";
import assert from "node:assert/strict";
import { renderInvoice } from "../src/invoice.js";
import { renderReceipt, renderRefund } from "../src/receipt.js";
import { renderStatement } from "../src/statement.js";

test("invoice", () => {
  const text = renderInvoice({
    number: "A-1",
    currency: "USD",
    items: [
      { description: "Widget", quantity: 3, unitCents: 125050 },
      { description: "Bolt", quantity: 10, unitCents: 5 },
    ],
  });
  assert.equal(text, "Invoice A-1\nWidget: $3,751.50\nBolt: $0.50\nTotal: $3,752.00");
});

test("receipt and refund", () => {
  const payment = { cents: 999, currency: "EUR", payer: "Ada" };
  assert.equal(renderReceipt(payment), "Received €9.99 from Ada");
  assert.equal(renderRefund(payment), "Refunded -€9.99 to Ada");
});

test("statement", () => {
  const text = renderStatement({
    owner: "Lin",
    currency: "USD",
    entries: [
      { date: "2024-01-01", cents: 100000000 },
      { date: "2024-01-02", cents: -250 },
    ],
  });
  assert.equal(text, "Statement for Lin\n2024-01-01  $1,000,000.00  $1,000,000.00\n2024-01-02  -$2.50  $999,997.50");
});
