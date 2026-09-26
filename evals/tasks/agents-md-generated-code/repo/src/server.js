import { ROUTES } from "./routes.generated.js";

export function dispatch(request) {
  const route = ROUTES.find((entry) => entry.method === request.method && entry.path === request.path);
  return route ? route.handler(request) : { status: 404, body: { error: "not found" } };
}
