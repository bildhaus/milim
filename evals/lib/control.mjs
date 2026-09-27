// Control-API credentials shared by run.mjs and smoke.mjs.
//
//   MILIM_CONTROL_URL       base URL of the desktop or mobile/control listener
//   MILIM_DEVICE_KEY        paired-device bearer key (mobile/control listener), or
//   MILIM_API_TOKEN         desktop bearer token (main listener)
//   MILIM_E2E_CONTROL_FILE  instead of the three above: the file a debug
//                           desktop build writes `{api_url, token}` to when it
//                           is launched with the same variable; waited for
//
// Credentials are only ever returned to the caller; nothing here prints them.

import { readFile } from "node:fs/promises";

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

/** Wait for a debug desktop build to publish its API URL and token. */
export async function readControlFile(path, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const value = JSON.parse(await readFile(path, "utf8"));
      if (typeof value.api_url === "string" && typeof value.token === "string") {
        return { apiUrl: value.api_url.replace(/\/+$/, ""), token: value.token };
      }
      throw new Error("control file is missing api_url or token");
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    if (Date.now() > deadline) {
      throw new Error(`timed out after ${timeoutMs} ms waiting for the control file`);
    }
    await sleep(250);
  }
}

/**
 * Resolve `{ base, credential, source }` from the environment. An explicit
 * MILIM_CONTROL_URL wins; otherwise MILIM_E2E_CONTROL_FILE is read (and
 * waited for). `base` is empty when neither is set.
 */
export async function resolveControlCredentials({ env = process.env, timeoutMs = 30_000 } = {}) {
  const base = (env.MILIM_CONTROL_URL || "").replace(/\/+$/, "");
  if (base) {
    return { base, credential: env.MILIM_DEVICE_KEY || env.MILIM_API_TOKEN || "", source: "env" };
  }
  if (env.MILIM_E2E_CONTROL_FILE) {
    const control = await readControlFile(env.MILIM_E2E_CONTROL_FILE, timeoutMs);
    return { base: control.apiUrl, credential: control.token, source: "control-file" };
  }
  return { base: "", credential: "", source: "none" };
}

/**
 * Retry `attempt` while the host is not accepting connections yet (a freshly
 * launched app). `isConnectionError` decides which failures are retryable.
 */
export async function retryUntilReady(attempt, { timeoutMs, isConnectionError }) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      return await attempt();
    } catch (error) {
      if (Date.now() > deadline || !isConnectionError(error)) throw error;
      await sleep(250);
    }
  }
}
