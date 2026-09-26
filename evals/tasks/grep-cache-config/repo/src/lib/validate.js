export function requireFields(body, fields) {
  return fields.filter((field) => body?.[field] === undefined || body[field] === "");
}
