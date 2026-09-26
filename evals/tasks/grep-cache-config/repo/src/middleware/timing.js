import { log } from "../lib/logger.js";

export function timed(name, handler) {
  return (req) => {
    const started = Date.now();
    const response = handler(req);
    log("debug", "request.timed", { name, ms: Date.now() - started });
    return response;
  };
}
