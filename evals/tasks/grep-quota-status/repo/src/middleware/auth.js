import { errorResponse } from "../lib/http.js";

export function requireAuth(handler) {
  return (req) => (req.headers?.authorization ? handler(req) : errorResponse("UNAUTHORIZED"));
}
