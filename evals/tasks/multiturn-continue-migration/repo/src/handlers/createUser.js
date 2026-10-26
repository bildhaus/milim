import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Create a user from `{ name, email }`. Calls back with `(error, { status, body })`. */
export function createUser(req, callback) {
  const { name, email } = req.body ?? {};
  if (typeof name !== "string" || !name.trim()) return callback(new HttpError(400, "name is required"));
  if (typeof email !== "string" || !email.includes("@")) return callback(new HttpError(400, "a valid email is required"));
  db.insert("users", { name: name.trim(), email: email.toLowerCase() }, (error, user) => {
    if (error) return callback(error);
    callback(null, { status: 201, body: user });
  });
}
