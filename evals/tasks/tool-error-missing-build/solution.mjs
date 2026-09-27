// Reference solution: the build script turned a minor-unit count of 0 into
// the default 2 (`Number("0") || 2`). Fix it, then build the table the way
// the README says.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const path = "scripts/build-rates.mjs";
const before = readFileSync(path, "utf8");
const after = before.replace(
  `    // Most currencies have two decimal places.
    minorUnits: Number(minorUnits) || 2,`,
  `    // Zero is a real value (JPY, KRW); only a blank column means the usual two.
    minorUnits: minorUnits === undefined || minorUnits.trim() === "" ? 2 : Number(minorUnits),`,
);
if (after === before) throw new Error("build script did not match");
writeFileSync(path, after);
execFileSync(process.execPath, ["scripts/build-rates.mjs", "--input", "data/rates.csv", "--out", "src/generated/rates.js"], { stdio: "inherit" });
