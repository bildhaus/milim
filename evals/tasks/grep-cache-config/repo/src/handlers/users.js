import { errorResponse, ok } from "../lib/http.js";
import { requireFields } from "../lib/validate.js";
import { paginate } from "../lib/paginate.js";
import { db } from "../lib/db.js";
import { log } from "../lib/logger.js";

const TABLE = "users";

export function listUsers(req) {
  const rows = db.all(TABLE);
  return ok(paginate(rows, req.query.page ?? 1, req.query.pageSize ?? 20));
}

export function getUser(req) {
  const row = db.get(TABLE, req.params.id);
  if (!row) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok(row);
}

export function createUser(req) {
  const missing = requireFields(req.body, ["name"]);
  if (missing.length > 0) {
    return errorResponse("VALIDATION_FAILED", { missing });
  }
  const row = db.insert(TABLE, req.body);
  log("info", "user.created", { id: row.id });
  return ok(row, 201);
}

export function deleteUser(req) {
  if (!db.remove(TABLE, req.params.id)) {
    return errorResponse("NOT_FOUND", { resource: TABLE, id: req.params.id });
  }
  return ok({ deleted: true });
}
