import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Users whose name contains `?q=` (at least two characters, any case). Calls back with `(error, { status, body })`. */
export function searchUsers(req, callback) {
  const q = String(req.query?.q ?? "").trim().toLowerCase();
  if (q.length < 2) return callback(new HttpError(400, "q must have at least two characters"));
  db.list("users", (user) => user.name.toLowerCase().includes(q), (error, users) => {
    if (error) return callback(error);
    callback(null, { status: 200, body: users.map((user) => user.id) });
  });
}
