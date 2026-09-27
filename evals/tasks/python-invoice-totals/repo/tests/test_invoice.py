import unittest

from billing import invoice_total, parse_lines

INVOICE = """description,quantity,unit_price
"Widget, large",3,19.99
Bolt,10,0.10
"Cable ""HD"", 2m",1,5.50
"""


class ParseLinesTest(unittest.TestCase):
    def test_quoted_descriptions(self):
        lines = parse_lines(INVOICE)
        self.assertEqual(
            [line["description"] for line in lines],
            ["Widget, large", "Bolt", 'Cable "HD", 2m'],
        )
        self.assertEqual(lines[0]["quantity"], 3)
        self.assertEqual(lines[1]["unit_price"], "0.10")


class InvoiceTotalTest(unittest.TestCase):
    def test_totals(self):
        totals = invoice_total(parse_lines(INVOICE), "0.0825")
        self.assertEqual(totals, {"subtotal": "66.47", "tax": "5.48", "total": "71.95"})

    def test_tax_rounds_half_up(self):
        lines = [{"description": "x", "quantity": 1, "unit_price": "1.25"}]
        # 1.25 * 0.1 = 0.125, which must round up to 0.13.
        self.assertEqual(invoice_total(lines, "0.1"), {"subtotal": "1.25", "tax": "0.13", "total": "1.38"})


if __name__ == "__main__":
    unittest.main()
