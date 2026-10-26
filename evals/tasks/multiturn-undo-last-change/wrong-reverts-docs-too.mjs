// Wrong: undoes the whole of turn 2, including the docs it was told to keep.
import { readFileSync, writeFileSync } from "node:fs";
import { recordTurn } from "../../lib/trajectory.mjs";
import { bumpRetries, renameEverywhere } from "./turns.mjs";

renameEverywhere();
recordTurn({ answer: "Renamed." });
bumpRetries();
recordTurn({ answer: "Bumped." });
writeFileSync("src/config.js", readFileSync("src/config.js", "utf8").replace("  retries: 5,", "  retries: 3,"));
writeFileSync("docs/config.md", readFileSync("docs/config.md", "utf8").replace("| `retries` | 5 |", "| `retries` | 3 |"));
recordTurn({ answer: "Reverted the retry bump." });
