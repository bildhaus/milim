import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** One user by id. Resolves to `{ status, body }`. */
export async function getUser(req) {
  const user = await db.promises.find("users", req.params.id);
  if (!user) throw new HttpError(404, "user not found");
  return { status: 200, body: user };
}
