import { parseArgs } from "./lib/args.js";
import { runReport } from "./features/report.js";
import { runSync } from "./features/sync.js";

export function main(argv) {
  const args = parseArgs(argv);
  return args.command === "sync" ? runSync(args) : runReport(args);
}

export function version() {
  return "1.0.0";
}
