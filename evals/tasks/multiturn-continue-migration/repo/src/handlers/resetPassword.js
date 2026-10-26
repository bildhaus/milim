import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Start a password reset for `{ email }`. Calls back with `(error, { status, body })`. */
export function resetPassword(req, callback) {
  const email = String(req.body?.email ?? "").toLowerCase();
  db.list("users", (user) => user.email === email, (error, users) => {
    if (error) return callback(error);
    if (users.length === 0) return callback(new HttpError(404, "no user with that email"));
    callback(null, { status: 202, body: { sent: true, user: users[0].id } });
  });
}
