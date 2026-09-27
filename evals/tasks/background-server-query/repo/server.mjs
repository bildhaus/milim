// Inventory stats service. See README.md.

import { randomUUID } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync } from "node:fs";
import { createServer } from "node:http";

const root = new URL("./", import.meta.url);
const readJson = (path) => JSON.parse(readFileSync(new URL(path, root), "utf8"));
const varDir = new URL("var/", root);
mkdirSync(varDir, { recursive: true });
const record = (file, entry) => appendFileSync(new URL(file, varDir), `${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`);

record("servers.jsonl", { pid: process.pid, event: "starting" });

/** Items with their confirmed incoming quantity. */
function buildIndex() {
  const { items } = readJson("data/inventory.json");
  const { shipments } = readJson("data/incoming.json");
  const incoming = new Map();
  for (const shipment of shipments) {
    if (shipment.status === "confirmed") incoming.set(shipment.sku, (incoming.get(shipment.sku) ?? 0) + shipment.quantity);
  }
  return items.map((item) => ({ ...item, incoming: incoming.get(item.sku) ?? 0 }));
}

const index = buildIndex();
const warehouses = new Set(index.map((item) => item.warehouse));

function route(method, url) {
  if (method !== "GET") return [405, { error: "method not allowed" }];
  if (url.pathname === "/health") return [200, { ok: true }];
  if (url.pathname === "/stats/low-stock") {
    const warehouse = url.searchParams.get("warehouse");
    if (!warehouse) return [400, { error: "the warehouse query parameter is required" }];
    if (!warehouses.has(warehouse)) return [404, { error: `unknown warehouse ${warehouse}` }];
    const skus = index
      .filter((item) => item.warehouse === warehouse && item.stock + item.incoming < item.reorder_point)
      .map((item) => item.sku);
    return [200, { warehouse, count: skus.length, skus }];
  }
  return [404, { error: "not found" }];
}

const server = createServer((request, response) => {
  const url = new URL(request.url, "http://localhost");
  const [status, payload] = route(request.method, url);
  const body = { ...payload, request_id: randomUUID() };
  record("requests.jsonl", {
    request_id: body.request_id,
    method: request.method,
    path: url.pathname,
    query: Object.fromEntries(url.searchParams),
    status,
  });
  response.writeHead(status, { "content-type": "application/json" });
  response.end(`${JSON.stringify(body)}\n`);
});

function shutdown(signal) {
  record("servers.jsonl", { pid: process.pid, event: "stopped", signal });
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1_000).unref();
}
process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

// Warming up takes a moment; requests are accepted once the URL is printed.
setTimeout(() => {
  server.listen(Number(process.env.PORT ?? 0), "127.0.0.1", () => {
    const { port } = server.address();
    record("servers.jsonl", { pid: process.pid, event: "listening", port });
    console.log(`inventory service listening on http://127.0.0.1:${port}`);
  });
}, 1_500);
