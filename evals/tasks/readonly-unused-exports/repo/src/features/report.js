import { column, mean } from "../lib/numbers.js";
import { isoDay } from "../lib/time.js";

export function runReport(args) {
  const values = (args.flags.values ?? "").split(",").filter(Boolean).map(Number);
  return `${isoDay(new Date(0))} ${column(mean(values))}`;
}

export function exportCsv(rows) {
  return rows.map((row) => row.join(",")).join("\n");
}
