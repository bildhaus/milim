import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Update a user's `name` and/or `email`; other fields are ignored. Resolves to `{ status, body }`. */
export async function updateUser(req) {
  const user = await db.promises.find("users", req.params.id);
  if (!user) throw new HttpError(404, "user not found");
  const patch = {};
  if (typeof req.body?.name === "string") patch.name = req.body.name.trim();
  if (typeof req.body?.email === "string") {
    if (!req.body.email.includes("@")) throw new HttpError(400, "a valid email is required");
    patch.email = req.body.email.toLowerCase();
  }
  const updated = await db.promises.update("users", user.id, patch);
  return { status: 200, body: updated };
}
