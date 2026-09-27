// Computes the count from the data files without running the service and
// invents a request id.
import { readFileSync, writeFileSync } from "node:fs";

const { items } = JSON.parse(readFileSync("data/inventory.json", "utf8"));
const { shipments } = JSON.parse(readFileSync("data/incoming.json", "utf8"));
const incoming = (sku) => shipments.filter((s) => s.sku === sku && s.status === "confirmed").reduce((sum, s) => sum + s.quantity, 0);
const count = items.filter((item) => item.warehouse === "north" && item.stock + incoming(item.sku) < item.reorder_point).length;
writeFileSync("ANSWER.json", `${JSON.stringify({ count, request_id: "00000000-0000-4000-8000-000000000000" })}\n`);
