import { validateUser } from "./generated/user.js";

/** Normalize and validate a user record; throws a TypeError listing every problem. */
export function parseUser(input) {
  const user = { ...input, email: typeof input.email === "string" ? input.email.trim().toLowerCase() : input.email };
  const errors = validateUser(user);
  if (errors.length > 0) throw new TypeError(errors.join("; "));
  return user;
}
