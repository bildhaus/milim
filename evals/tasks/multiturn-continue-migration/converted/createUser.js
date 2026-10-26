import { db } from "../db.js";
import { HttpError } from "../errors.js";

/** Create a user from `{ name, email }`. Resolves to `{ status, body }`. */
export async function createUser(req) {
  const { name, email } = req.body ?? {};
  if (typeof name !== "string" || !name.trim()) throw new HttpError(400, "name is required");
  if (typeof email !== "string" || !email.includes("@")) throw new HttpError(400, "a valid email is required");
  const user = await db.promises.insert("users", { name: name.trim(), email: email.toLowerCase() });
  return { status: 201, body: user };
}
