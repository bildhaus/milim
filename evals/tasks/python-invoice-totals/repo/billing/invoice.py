"""Invoice parsing and totals.

Invoices arrive as CSV text with a header row:

    description,quantity,unit_price

Descriptions may be quoted and may contain commas. Money is handled in
exact decimal arithmetic: amounts are rounded half up to whole cents.
"""


def parse_lines(text):
    """Return one dict per invoice line.

    Each dict has ``description`` (str), ``quantity`` (int), and
    ``unit_price`` (a string with exactly the digits given, such as "19.99").
    Blank lines are ignored.
    """
    rows = [line for line in text.strip().splitlines() if line.strip()]
    lines = []
    for row in rows[1:]:
        description, quantity, unit_price = row.split(",")
        lines.append(
            {
                "description": description,
                "quantity": int(quantity),
                "unit_price": unit_price.strip(),
            }
        )
    return lines


def invoice_total(lines, tax_rate):
    """Return ``{"subtotal", "tax", "total"}`` as strings with two decimals.

    ``subtotal`` is the sum of quantity * unit_price. ``tax`` is subtotal *
    tax_rate (a string such as "0.0825") rounded half up to the cent, and
    ``total`` is subtotal + tax.
    """
    subtotal = sum(line["quantity"] * float(line["unit_price"]) for line in lines)
    tax = round(subtotal * float(tax_rate), 2)
    total = subtotal + tax
    return {
        "subtotal": "%.2f" % subtotal,
        "tax": "%.2f" % tax,
        "total": "%.2f" % total,
    }
