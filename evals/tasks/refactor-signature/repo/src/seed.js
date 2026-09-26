import { createUser } from "./users.js";

export function seedUsers() {
  return [
    createUser("Root", "ROOT@example.com", true),
    createUser("Guest", "guest@example.com", false),
    createUser(" Dana ", "Dana@Example.com"),
  ];
}
