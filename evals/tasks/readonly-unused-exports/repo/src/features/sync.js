import { daysBetween } from "../lib/time.js";

export function runSync(args) {
  const since = new Date(args.flags.since ?? 0);
  return `synced ${daysBetween(since, new Date(since.getTime() + 86_400_000 * 3))} days`;
}

export function dryRun(args) {
  return `would sync since ${args.flags.since}`;
}
