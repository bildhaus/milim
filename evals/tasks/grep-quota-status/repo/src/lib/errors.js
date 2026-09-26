export const ERRORS = {
  NOT_FOUND: { status: 404, message: "resource not found" },
  VALIDATION_FAILED: { status: 422, message: "request validation failed" },
  UNAUTHORIZED: { status: 401, message: "authentication required" },
  FORBIDDEN: { status: 403, message: "not allowed" },
  QUOTA_EXCEEDED: { status: 429, message: "usage quota exceeded" },
  CONFLICT: { status: 409, message: "conflicting change" },
  INTERNAL: { status: 500, message: "internal error" },
};
