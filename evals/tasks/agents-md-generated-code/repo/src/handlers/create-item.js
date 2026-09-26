import { items } from "../store.js";

export function handleCreateItem(request) {
  if (typeof request.body?.name !== "string") {
    return { status: 422, body: { error: "name is required" } };
  }
  items.push({ name: request.body.name });
  return { status: 201, body: { name: request.body.name } };
}
