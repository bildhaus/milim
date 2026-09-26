import { errorResponse, ok } from "../lib/http.js";
import { requireFields } from "../lib/validate.js";
import { paginate } from "../lib/paginate.js";
import { db } from "../lib/db.js";
import { log } from "../lib/logger.js";

const TABLE = "sessions";

export function listSessions(req) {
  const rows = db.all(TABLE);
  return ok(paginate(rows, req.query.page ?? 1, req.query.pageSize ?? 20));
}

export function getSession(req) {
  const row = db.get(TABLE, req.params.id);
  if (!row) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok(row);
}

export function createSession(req) {
  const missing = requireFields(req.body, ["email"]);
  if (missing.length > 0) {
    return errorResponse("VALIDATION_FAILED", { missing });
  }
  const row = db.insert(TABLE, req.body);
  log("info", "session.created", { id: row.id });
  return ok(row, 201);
}

export function deleteSession(req) {
  if (!db.remove(TABLE, req.params.id)) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok({ deleted: true });
}
