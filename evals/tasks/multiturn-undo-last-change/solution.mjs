// Reference: rename, bump, then revert only the bump in src/config.js.
import { readFileSync, writeFileSync } from "node:fs";
import { recordTurn } from "../../lib/trajectory.mjs";
import { bumpRetries, renameEverywhere } from "./turns.mjs";

renameEverywhere();
recordTurn({ answer: "Renamed timeoutMs to requestTimeoutMs in code, tests, and docs." });
bumpRetries();
recordTurn({ answer: "Default retries is now 5 in src/config.js and docs/config.md." });
const config = readFileSync("src/config.js", "utf8");
writeFileSync("src/config.js", config.replace("  retries: 5,", "  retries: 3,"));
recordTurn({ answer: "Reverted the retry default in src/config.js back to 3; docs unchanged." });
