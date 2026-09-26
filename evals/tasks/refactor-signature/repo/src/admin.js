import { createUser } from "./users.js";

export function promoteInvite(invite) {
  const user = createUser(invite.name, invite.email, true);
  return { user, permissions: user.isAdmin ? ["read", "write", "admin"] : ["read"] };
}

export function isPrivileged(user) {
  return user.isAdmin === true;
}
