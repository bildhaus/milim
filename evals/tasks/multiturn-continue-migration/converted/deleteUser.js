import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Delete a user. Resolves to `{ status, body }`. */
export async function deleteUser(req) {
  const user = await db.promises.find("users", req.params.id);
  if (!user) throw new HttpError(404, "user not found");
  await db.promises.remove("users", user.id);
  return { status: 204, body: null };
}
