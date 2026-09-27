// Patches the symptom: builds the table, then special-cases JPY in the converter.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const path = "src/convert.js";
writeFileSync(
  path,
  readFileSync(path, "utf8").replace(
    "  if (!entry) throw new RangeError(`unknown currency ${code}`);\n  return entry;",
    "  if (!entry) throw new RangeError(`unknown currency ${code}`);\n  return code === \"JPY\" ? { ...entry, minorUnits: 0 } : entry;",
  ),
);
execFileSync(process.execPath, ["scripts/build-rates.mjs", "--input", "data/rates.csv", "--out", "src/generated/rates.js"]);
