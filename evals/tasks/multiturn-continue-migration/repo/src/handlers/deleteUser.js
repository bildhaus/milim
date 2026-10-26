import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Delete a user. Calls back with `(error, { status, body })`. */
export function deleteUser(req, callback) {
  db.find("users", req.params.id, (error, user) => {
    if (error) return callback(error);
    if (!user) return callback(new HttpError(404, "user not found"));
    db.remove("users", user.id, (removeError) => {
      if (removeError) return callback(removeError);
      callback(null, { status: 204, body: null });
    });
  });
}
