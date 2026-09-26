import { createUser } from "./users.js";

export function signup(form) {
  const user = createUser(form.name, form.email);
  return { user, welcome: `Welcome, ${user.name}!` };
}
