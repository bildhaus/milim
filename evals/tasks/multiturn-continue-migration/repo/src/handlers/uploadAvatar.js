import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Store an avatar of `{ bytes }` (at most 1,000,000) for a user. Calls back with `(error, { status, body })`. */
export function uploadAvatar(req, callback) {
  const bytes = Number(req.body?.bytes);
  if (!(bytes > 0)) return callback(new HttpError(400, "bytes must be positive"));
  if (bytes > 1_000_000) return callback(new HttpError(413, "avatar too large"));
  db.find("users", req.params.id, (error, user) => {
    if (error) return callback(error);
    if (!user) return callback(new HttpError(404, "user not found"));
    db.update("users", user.id, { avatarBytes: bytes }, (updateError, updated) => {
      if (updateError) return callback(updateError);
      callback(null, { status: 200, body: updated });
    });
  });
}
