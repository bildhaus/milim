import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Start a password reset for `{ email }`. Resolves to `{ status, body }`. */
export async function resetPassword(req) {
  const email = String(req.body?.email ?? "").toLowerCase();
  const users = await db.promises.list("users", (user) => user.email === email);
  if (users.length === 0) throw new HttpError(404, "no user with that email");
  return { status: 202, body: { sent: true, user: users[0].id } };
}
