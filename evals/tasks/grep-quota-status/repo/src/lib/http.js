import { ERRORS } from "./errors.js";

export function ok(body, status = 200) {
  return { status, body };
}

export function errorResponse(code, details = {}) {
  const spec = ERRORS[code] ?? ERRORS.INTERNAL;
  return { status: spec.status, body: { error: { code, message: spec.message, details } } };
}
