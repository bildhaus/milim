let nextId = 1;

export function resetIds() {
  nextId = 1;
}

export function createUser(name, email, isAdmin = false) {
  if (!name || !email.includes("@")) {
    throw new Error("invalid user");
  }
  return { id: nextId++, name: name.trim(), email: email.toLowerCase(), isAdmin };
}
