import { spawnSync } from "node:child_process";
import { changedFiles, expect, expectEqual, finish, findPython, headFile, read, repo, skip } from "../../lib/check.mjs";

const python = findPython();
if (!python) skip("python3 is not installed; this task needs Python 3.8+");

const pythonEnv = { ...process.env, PYTHONDONTWRITEBYTECODE: "1", PYTHONPATH: repo };

expect(read("tests/test_invoice.py") === headFile("tests/test_invoice.py"), "tests/test_invoice.py was modified");
const stray = changedFiles().filter((path) => !path.startsWith("billing/") && !path.startsWith("tests/"));
expect(stray.length === 0, `changed outside billing/ and tests/: ${stray.join(", ")}`);

const tests = spawnSync(python, ["-m", "unittest", "discover", "-s", "tests"], { cwd: repo, encoding: "utf8", env: pythonEnv, timeout: 60_000 });
expect(tests.status === 0, `unit tests failed: ${`${tests.stdout}${tests.stderr}`.trim().split("\n").slice(-4).join(" | ")}`);

// Hidden cases, run in a fresh interpreter against the package.
const script = String.raw`
import json
from billing import invoice_total, parse_lines

text = 'description,quantity,unit_price\r\n"Tape, blue",2,1.15\r\nGlue,1,0.10\r\n\r\n'
lines = parse_lines(text)
out = {
    "descriptions": [line["description"] for line in lines],
    "quantities": [line["quantity"] for line in lines],
    "float_trap": invoice_total([{"description": "a", "quantity": 1, "unit_price": "1.15"}], "0.5"),
    "tie": invoice_total([{"description": "b", "quantity": 1, "unit_price": "2.25"}], "0.5"),
    "many": invoice_total([{"description": "c", "quantity": 1, "unit_price": "0.10"}] * 3, "0"),
    "large": invoice_total([{"description": "d", "quantity": 1, "unit_price": "98765432.25"}], "0.5"),
    "mixed": invoice_total(lines, "0.0825"),
}
print(json.dumps(out))
`;
const hidden = spawnSync(python, ["-c", script], { cwd: repo, encoding: "utf8", env: pythonEnv, timeout: 30_000 });
if (expect(hidden.status === 0, `hidden cases crashed: ${hidden.stderr.trim().split("\n").at(-1)}`)) {
  const got = JSON.parse(hidden.stdout.trim().split("\n").at(-1));
  expectEqual(got.descriptions, ["Tape, blue", "Glue"], "CRLF and quoted commas");
  expectEqual(got.quantities, [2, 1], "quantities are ints");
  expectEqual(got.float_trap, { subtotal: "1.15", tax: "0.58", total: "1.73" }, "1.15 * 0.5 = 0.575 rounds half up");
  expectEqual(got.tie, { subtotal: "2.25", tax: "1.13", total: "3.38" }, "2.25 * 0.5 = 1.125 rounds half up");
  expectEqual(got.many, { subtotal: "0.30", tax: "0.00", total: "0.30" }, "exact sums");
  expectEqual(got.large, { subtotal: "98765432.25", tax: "49382716.13", total: "148148148.38" }, "large amounts stay exact");
  expectEqual(got.mixed, { subtotal: "2.40", tax: "0.20", total: "2.60" }, "parsed invoice total");
}

finish();
