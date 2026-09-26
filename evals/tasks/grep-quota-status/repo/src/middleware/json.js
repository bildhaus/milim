export function parseJson(handler) {
  return (req) => handler({ ...req, body: typeof req.body === "string" ? JSON.parse(req.body) : req.body ?? {} });
}
