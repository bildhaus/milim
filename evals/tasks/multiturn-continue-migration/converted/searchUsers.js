import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Users whose name contains `?q=` (at least two characters, any case). Resolves to `{ status, body }`. */
export async function searchUsers(req) {
  const q = String(req.query?.q ?? "").trim().toLowerCase();
  if (q.length < 2) throw new HttpError(400, "q must have at least two characters");
  const users = await db.promises.list("users", (user) => user.name.toLowerCase().includes(q));
  return { status: 200, body: users.map((user) => user.id) };
}
