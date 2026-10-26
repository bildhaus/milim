import { loadConfig } from "./config.js";

/** Request options for the transport, derived from the configuration. */
export function requestOptions(overrides = {}) {
  const config = loadConfig(overrides);
  return {
    url: config.baseUrl,
    timeout: config.timeoutMs,
    attempts: config.retries + 1,
    // Exponential backoff between attempts.
    delays: Array.from({ length: config.retries }, (_, attempt) => config.backoffMs * 2 ** attempt),
  };
}
