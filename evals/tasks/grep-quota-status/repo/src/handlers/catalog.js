import { ok } from "../lib/http.js";
import { recall, remember } from "../lib/cache.js";
import { db } from "../lib/db.js";

let lookups = 0;

export function lookupCount() {
  return lookups;
}

export function getCatalog(req) {
  const key = `catalog:${req.query.region ?? "default"}`;
  const cached = recall(key);
  if (cached) {
    return ok(cached);
  }
  lookups += 1;
  return ok(remember(key, { items: db.all("products"), region: req.query.region ?? "default" }));
}
