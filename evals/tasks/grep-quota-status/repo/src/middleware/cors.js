export function withCors(handler) {
  return (req) => {
    const response = handler(req);
    return { ...response, headers: { ...response.headers, "access-control-allow-origin": "*" } };
  };
}
