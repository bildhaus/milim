// Development server for the visits dashboard. See README.md.
//
// Every boot gets a random boot id, and the visit counter starts from a
// value restored from a random snapshot, so both identify this process.

import { randomBytes, randomInt } from "node:crypto";
import { appendFileSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";

const varDir = new URL("var/", import.meta.url);
mkdirSync(varDir, { recursive: true });
const record = (file, entry) => appendFileSync(new URL(file, varDir), `${JSON.stringify({ at_ms: Date.now(), ...entry })}\n`);

const bootId = randomBytes(4).toString("hex");
const startedAt = Date.now();
let visits = randomInt(100, 1000);
record("boots.jsonl", { event: "boot", pid: process.pid, boot_id: bootId });

function route(method, path) {
  if (method !== "GET") return [405, { error: "method not allowed" }];
  if (path === "/health") return [200, { ok: true, boot_id: bootId, uptime_ms: Date.now() - startedAt }];
  if (path === "/counter") {
    visits += 1;
    return [200, { boot_id: bootId, value: visits }];
  }
  return [404, { error: "not found" }];
}

const server = createServer((request, response) => {
  const path = new URL(request.url, "http://localhost").pathname;
  const [status, body] = route(request.method, path);
  record("requests.jsonl", { pid: process.pid, boot_id: bootId, method: request.method, path, status, body });
  response.writeHead(status, { "content-type": "application/json" });
  response.end(`${JSON.stringify(body)}\n`);
});

function shutdown(signal) {
  record("boots.jsonl", { event: "stopped", pid: process.pid, boot_id: bootId, signal });
  server.close(() => process.exit(0));
  setTimeout(() => process.exit(0), 1_000).unref();
}
process.on("SIGTERM", () => shutdown("SIGTERM"));
process.on("SIGINT", () => shutdown("SIGINT"));

// Simulates the dev build before the server accepts connections.
setTimeout(() => {
  server.listen(0, "127.0.0.1", () => {
    console.log(`dev server listening on http://127.0.0.1:${server.address().port}`);
  });
}, 1_000);
