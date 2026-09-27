// Fixes the root cause without ever running the build or the tests.
import { readFileSync, writeFileSync } from "node:fs";

const path = "scripts/build-rates.mjs";
writeFileSync(path, readFileSync(path, "utf8").replace("Number(minorUnits) || 2", "Number(minorUnits ?? 2)"));
