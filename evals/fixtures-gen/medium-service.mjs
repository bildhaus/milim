// Deterministic generator for a ~40-file HTTP service fixture. Variants plant
// exactly one bug so an agent has to search the tree to find it:
//   "quota"  - the usage handler hard-codes status 500 for QUOTA_EXCEEDED
//   "cache"  - the cache reads a config key that config.js never defines

const ENTITIES = [
  ["users", "user"],
  ["teams", "team"],
  ["projects", "project"],
  ["tasks", "task"],
  ["comments", "comment"],
  ["labels", "label"],
  ["invoices", "invoice"],
  ["payments", "payment"],
  ["customers", "customer"],
  ["products", "product"],
  ["orders", "order"],
  ["shipments", "shipment"],
  ["warehouses", "warehouse"],
  ["suppliers", "supplier"],
  ["reports", "report"],
  ["webhooks", "webhook"],
  ["sessions", "session"],
  ["tokens", "token"],
  ["notifications", "notification"],
  ["subscriptions", "subscription"],
  ["coupons", "coupon"],
  ["audits", "audit"],
  ["exports", "export"],
];

const REQUIRED = ["name", "title", "email", "amount", "code", "label", "url"];

const cap = (word) => word[0].toUpperCase() + word.slice(1);

function entityHandler([table, singular], index) {
  const Name = cap(singular);
  const field = REQUIRED[index % REQUIRED.length];
  return `import { errorResponse, ok } from "../lib/http.js";
import { requireFields } from "../lib/validate.js";
import { paginate } from "../lib/paginate.js";
import { db } from "../lib/db.js";
import { log } from "../lib/logger.js";

const TABLE = "${table}";

export function list${cap(table)}(req) {
  const rows = db.all(TABLE);
  return ok(paginate(rows, req.query.page ?? 1, req.query.pageSize ?? 20));
}

export function get${Name}(req) {
  const row = db.get(TABLE, req.params.id);
  if (!row) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok(row);
}

export function create${Name}(req) {
  const missing = requireFields(req.body, ["${field}"]);
  if (missing.length > 0) {
    return errorResponse("VALIDATION_FAILED", { missing });
  }
  const row = db.insert(TABLE, req.body);
  log("info", "${singular}.created", { id: row.id });
  return ok(row, 201);
}

export function delete${Name}(req) {
  if (!db.remove(TABLE, req.params.id)) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok({ deleted: true });
}
`;
}

function usageHandler(variant) {
  const exceeded =
    variant === "quota"
      ? `    return {
      status: 500,
      body: { error: { code: "QUOTA_EXCEEDED", message: "usage quota exceeded", details: { limit, used } } },
    };`
      : `    return errorResponse("QUOTA_EXCEEDED", { limit, used });`;
  return `import { errorResponse, ok } from "../lib/http.js";
import { config } from "../lib/config.js";
import { db } from "../lib/db.js";

export function recordUsage(req) {
  const account = req.params.account;
  const used = (db.get("usage", account)?.used ?? 0) + (req.body.units ?? 1);
  const limit = db.get("plans", account)?.limit ?? config.defaultQuota;
  if (used > limit) {
${exceeded}
  }
  db.upsert("usage", account, { used });
  return ok({ account, used, limit });
}

export function getUsage(req) {
  const account = req.params.account;
  return ok({ account, used: db.get("usage", account)?.used ?? 0 });
}
`;
}

function cacheLib(variant) {
  const ttl = variant === "cache" ? "config.cacheTTL" : "config.cacheTtlSeconds";
  return `import { config } from "./config.js";
import { now } from "./clock.js";

const entries = new Map();

/** Remember \`value\` for the configured TTL. */
export function remember(key, value) {
  const ttlMs = ${ttl} * 1000;
  if (!(ttlMs > 0)) {
    return value;
  }
  entries.set(key, { value, expiresAt: now() + ttlMs });
  return value;
}

/** The cached value for \`key\`, or undefined once it expires. */
export function recall(key) {
  const entry = entries.get(key);
  if (!entry || entry.expiresAt <= now()) {
    entries.delete(key);
    return undefined;
  }
  return entry.value;
}

export function clearCache() {
  entries.clear();
}
`;
}

const LIB = {
  "src/lib/http.js": `import { ERRORS } from "./errors.js";

export function ok(body, status = 200) {
  return { status, body };
}

export function errorResponse(code, details = {}) {
  const spec = ERRORS[code] ?? ERRORS.INTERNAL;
  return { status: spec.status, body: { error: { code, message: spec.message, details } } };
}
`,
  "src/lib/errors.js": `export const ERRORS = {
  NOT_FOUND: { status: 404, message: "resource not found" },
  VALIDATION_FAILED: { status: 422, message: "request validation failed" },
  UNAUTHORIZED: { status: 401, message: "authentication required" },
  FORBIDDEN: { status: 403, message: "not allowed" },
  QUOTA_EXCEEDED: { status: 429, message: "usage quota exceeded" },
  CONFLICT: { status: 409, message: "conflicting change" },
  INTERNAL: { status: 500, message: "internal error" },
};
`,
  "src/lib/validate.js": `export function requireFields(body, fields) {
  return fields.filter((field) => body?.[field] === undefined || body[field] === "");
}
`,
  "src/lib/paginate.js": `export function paginate(rows, page, pageSize) {
  const start = (Number(page) - 1) * Number(pageSize);
  return { items: rows.slice(start, start + Number(pageSize)), total: rows.length };
}
`,
  "src/lib/db.js": `const tables = new Map();
let nextId = 1;

function table(name) {
  if (!tables.has(name)) tables.set(name, new Map());
  return tables.get(name);
}

export const db = {
  all: (name) => [...table(name).values()],
  get: (name, id) => table(name).get(String(id)),
  insert(name, row) {
    const id = String(nextId++);
    const stored = { ...row, id };
    table(name).set(id, stored);
    return stored;
  },
  upsert(name, id, row) {
    table(name).set(String(id), { ...row, id: String(id) });
  },
  remove: (name, id) => table(name).delete(String(id)),
  reset() {
    tables.clear();
    nextId = 1;
  },
};
`,
  "src/lib/logger.js": `const lines = [];

export function log(level, event, fields = {}) {
  lines.push({ level, event, ...fields });
}

export function drainLogs() {
  return lines.splice(0, lines.length);
}
`,
  "src/lib/config.js": `const env = globalThis.process?.env ?? {};

export const config = {
  port: Number(env.PORT ?? 8080),
  defaultQuota: Number(env.DEFAULT_QUOTA ?? 1000),
  cacheTtlSeconds: Number(env.CACHE_TTL_SECONDS ?? 60),
  logLevel: env.LOG_LEVEL ?? "info",
};
`,
  "src/lib/clock.js": `let offset = 0;

export function now() {
  return Date.now() + offset;
}

export function advance(ms) {
  offset += ms;
}
`,
  "src/lib/ids.js": `export function isId(value) {
  return /^[0-9]+$/.test(String(value));
}
`,
  "src/middleware/auth.js": `import { errorResponse } from "../lib/http.js";

export function requireAuth(handler) {
  return (req) => (req.headers?.authorization ? handler(req) : errorResponse("UNAUTHORIZED"));
}
`,
  "src/middleware/json.js": `export function parseJson(handler) {
  return (req) => handler({ ...req, body: typeof req.body === "string" ? JSON.parse(req.body) : req.body ?? {} });
}
`,
  "src/middleware/timing.js": `import { log } from "../lib/logger.js";

export function timed(name, handler) {
  return (req) => {
    const started = Date.now();
    const response = handler(req);
    log("debug", "request.timed", { name, ms: Date.now() - started });
    return response;
  };
}
`,
  "src/middleware/cors.js": `export function withCors(handler) {
  return (req) => {
    const response = handler(req);
    return { ...response, headers: { ...response.headers, "access-control-allow-origin": "*" } };
  };
}
`,
};

function catalogHandler() {
  return `import { ok } from "../lib/http.js";
import { recall, remember } from "../lib/cache.js";
import { db } from "../lib/db.js";

let lookups = 0;

export function lookupCount() {
  return lookups;
}

export function getCatalog(req) {
  const key = \`catalog:\${req.query.region ?? "default"}\`;
  const cached = recall(key);
  if (cached) {
    return ok(cached);
  }
  lookups += 1;
  return ok(remember(key, { items: db.all("products"), region: req.query.region ?? "default" }));
}
`;
}

function router() {
  const imports = ENTITIES.map(
    ([table, singular]) =>
      `import { create${cap(singular)}, delete${cap(singular)}, get${cap(singular)}, list${cap(table)} } from "./handlers/${table}.js";`,
  ).join("\n");
  const routes = ENTITIES.map(
    ([table, singular]) => `  ["GET /${table}", list${cap(table)}],
  ["GET /${table}/:id", get${cap(singular)}],
  ["POST /${table}", create${cap(singular)}],
  ["DELETE /${table}/:id", delete${cap(singular)}],`,
  ).join("\n");
  return `${imports}
import { getUsage, recordUsage } from "./handlers/usage.js";
import { getCatalog } from "./handlers/catalog.js";
import { errorResponse } from "./lib/http.js";
import { parseJson } from "./middleware/json.js";

export const ROUTES = [
${routes}
  ["GET /usage/:account", getUsage],
  ["POST /usage/:account", recordUsage],
  ["GET /catalog", getCatalog],
];

function match(pattern, method, path) {
  const [patternMethod, patternPath] = pattern.split(" ");
  if (patternMethod !== method) return null;
  const want = patternPath.split("/");
  const got = path.split("/");
  if (want.length !== got.length) return null;
  const params = {};
  for (let i = 0; i < want.length; i += 1) {
    if (want[i].startsWith(":")) params[want[i].slice(1)] = got[i];
    else if (want[i] !== got[i]) return null;
  }
  return params;
}

export function handle({ method, path, query = {}, body, headers = {} }) {
  for (const [pattern, handler] of ROUTES) {
    const params = match(pattern, method, path);
    if (params) return parseJson(handler)({ params, query, body, headers });
  }
  return errorResponse("NOT_FOUND", { path });
}
`;
}

const TESTS = {
  "test/router.test.js": `import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";
import { db } from "../src/lib/db.js";

beforeEach(() => db.reset());

test("unknown route is 404", () => {
  assert.equal(handle({ method: "GET", path: "/nope" }).status, 404);
});

test("create then get a user", () => {
  const created = handle({ method: "POST", path: "/users", body: { name: "Apollo" } });
  assert.equal(created.status, 201);
  const fetched = handle({ method: "GET", path: \`/users/\${created.body.id}\` });
  assert.equal(fetched.body.name, "Apollo");
});
`,
  "test/validation.test.js": `import { test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";

test("missing required field is 422", () => {
  const response = handle({ method: "POST", path: "/users", body: {} });
  assert.equal(response.status, 422);
  assert.equal(response.body.error.code, "VALIDATION_FAILED");
});
`,
  "test/usage.test.js": `import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { handle } from "../src/index.js";
import { db } from "../src/lib/db.js";

beforeEach(() => db.reset());

test("usage under the quota is recorded", () => {
  db.upsert("plans", "acme", { limit: 10 });
  const response = handle({ method: "POST", path: "/usage/acme", body: { units: 4 } });
  assert.equal(response.status, 200);
  assert.equal(response.body.used, 4);
});
`,
};

/** Map of relative path to file content for one variant. */
export function mediumServiceFiles(variant) {
  if (!["quota", "cache"].includes(variant)) throw new Error(`unknown variant ${variant}`);
  const files = {
    "package.json": `${JSON.stringify(
      { name: "orbit-api", private: true, type: "module", scripts: { test: "node --test test/" } },
      null,
      2,
    )}\n`,
    "README.md": `# orbit-api

A small in-memory REST service. Handlers live in \`src/handlers/\`, shared
helpers in \`src/lib/\`, and request wrappers in \`src/middleware/\`. Run the
tests with \`npm test\`.
`,
    "src/index.js": router(),
    "src/handlers/usage.js": usageHandler(variant),
    "src/handlers/catalog.js": catalogHandler(),
    "src/lib/cache.js": cacheLib(variant),
    ...LIB,
    ...TESTS,
  };
  ENTITIES.forEach((entity, index) => {
    files[`src/handlers/${entity[0]}.js`] = entityHandler(entity, index);
  });
  return files;
}
