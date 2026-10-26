import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Store an avatar of `{ bytes }` (at most 1,000,000) for a user. Resolves to `{ status, body }`. */
export async function uploadAvatar(req) {
  const bytes = Number(req.body?.bytes);
  if (!(bytes > 0)) throw new HttpError(400, "bytes must be positive");
  if (bytes > 1_000_000) throw new HttpError(413, "avatar too large");
  const user = await db.promises.find("users", req.params.id);
  if (!user) throw new HttpError(404, "user not found");
  const updated = await db.promises.update("users", user.id, { avatarBytes: bytes });
  return { status: 200, body: updated };
}
