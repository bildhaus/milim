import { items } from "../store.js";

export function handleListItems() {
  return { status: 200, body: { items: [...items] } };
}
