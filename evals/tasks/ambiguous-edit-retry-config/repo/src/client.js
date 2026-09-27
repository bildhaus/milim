import { CLIENTS } from "../config/clients.js";

/** Settings for one named client, e.g. `clientSettings("production", "payments")`. */
export function clientSettings(env, name) {
  const settings = CLIENTS[env]?.[name];
  if (!settings) throw new RangeError(`no ${name} client in ${env}`);
  return settings;
}

/** Delays in ms before each retry: backoffMs, then doubling. */
export function retryDelays(env, name) {
  const { attempts, backoffMs } = clientSettings(env, name).retry;
  return Array.from({ length: attempts - 1 }, (_, index) => backoffMs * 2 ** index);
}
