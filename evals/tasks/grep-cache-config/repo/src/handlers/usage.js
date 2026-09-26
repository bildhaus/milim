import { errorResponse, ok } from "../lib/http.js";
import { config } from "../lib/config.js";
import { db } from "../lib/db.js";

export function recordUsage(req) {
  const account = req.params.account;
  const used = (db.get("usage", account)?.used ?? 0) + (req.body.units ?? 1);
  const limit = db.get("plans", account)?.limit ?? config.defaultQuota;
  if (used > limit) {
    return errorResponse("QUOTA_EXCEEDED", { limit, used });
  }
  db.upsert("usage", account, { used });
  return ok({ account, used, limit });
}

export function getUsage(req) {
  const account = req.params.account;
  return ok({ account, used: db.get("usage", account)?.used ?? 0 });
}
