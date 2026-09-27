import { attempt, expectEqual, expectOnlyChanged, finish, load } from "../../lib/check.mjs";

// Final rules from docs/meetings/ (see evals/fixtures-gen/meeting-notes.mjs):
// 1.5% of the amount (2% until week 6), only once an invoice is more than
// 10 days past due (7 until week 10), at most $40.00, nonprofits exempt,
// enterprise a flat $25.00 after the grace period, cents rounded half up.
expectOnlyChanged((path) => path === "src/fees.js" || path.startsWith("test/"));

await attempt("lateFee", async () => {
  const { lateFee } = await load("src/fees.js");
  const standard = (amountCents, dueDate = "2024-05-01") => ({ amountCents, dueDate, customerType: "standard" });
  const cases = [
    [standard(10_000), "2024-04-20", 0, "not yet due"],
    [standard(10_000), "2024-05-01", 0, "due today"],
    [standard(10_000), "2024-05-09", 0, "8 days late is inside the extended grace period"],
    [standard(10_000), "2024-05-11", 0, "exactly 10 days late is still free"],
    [standard(10_000), "2024-05-12", 150, "11 days late pays 1.5%"],
    [standard(3_100), "2024-06-01", 47, "1.5% of $31.00 rounds half up to 47 cents"],
    [standard(12_345), "2024-06-01", 185, "1.5% of $123.45 is 185.175 cents"],
    [standard(500_000), "2024-06-01", 4_000, "capped at $40.00"],
    [standard(266_667), "2024-06-01", 4_000, "cap applies from 4000.005 cents"],
    [standard(266_600), "2024-06-01", 3_999, "just under the cap"],
    [standard(0), "2024-06-01", 0, "zero amount"],
    [standard(10_000, "2024-02-25"), "2024-03-07", 150, "11 days across a leap day"],
    [standard(10_000, "2024-02-25"), "2024-03-06", 0, "10 days across a leap day"],
    [{ amountCents: 900_000, dueDate: "2024-05-01", customerType: "nonprofit" }, "2024-09-01", 0, "nonprofits are exempt"],
    [{ amountCents: 900_000, dueDate: "2024-05-01", customerType: "enterprise" }, "2024-06-01", 2_500, "enterprise flat fee"],
    [{ amountCents: 1_000, dueDate: "2024-05-01", customerType: "enterprise" }, "2024-06-01", 2_500, "enterprise flat fee, not the percentage"],
    [{ amountCents: 900_000, dueDate: "2024-05-01", customerType: "enterprise" }, "2024-05-11", 0, "enterprise within the grace period"],
  ];
  for (const [invoice, asOf, expected, label] of cases) {
    expectEqual(lateFee(invoice, asOf), expected, `${label}: lateFee(${JSON.stringify(invoice)}, "${asOf}")`);
  }
});

finish();
