import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** One user by id. Calls back with `(error, { status, body })`. */
export function getUser(req, callback) {
  db.find("users", req.params.id, (error, user) => {
    if (error) return callback(error);
    if (!user) return callback(new HttpError(404, "user not found"));
    callback(null, { status: 200, body: user });
  });
}
