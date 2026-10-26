import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Update a user's `name` and/or `email`; other fields are ignored. Calls back with `(error, { status, body })`. */
export function updateUser(req, callback) {
  db.find("users", req.params.id, (error, user) => {
    if (error) return callback(error);
    if (!user) return callback(new HttpError(404, "user not found"));
    const patch = {};
    if (typeof req.body?.name === "string") patch.name = req.body.name.trim();
    if (typeof req.body?.email === "string") {
      if (!req.body.email.includes("@")) return callback(new HttpError(400, "a valid email is required"));
      patch.email = req.body.email.toLowerCase();
    }
    db.update("users", user.id, patch, (updateError, updated) => {
      if (updateError) return callback(updateError);
      callback(null, { status: 200, body: updated });
    });
  });
}
