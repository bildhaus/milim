// Another correct run: the agent kept a copy of src/config.js from before
// its last edit and restores it verbatim (the file ends without a newline
// change, which the check tolerates as whitespace).
import { readFileSync, writeFileSync } from "node:fs";
import { recordTurn } from "../../lib/trajectory.mjs";
import { bumpRetries, renameEverywhere } from "./turns.mjs";

renameEverywhere();
const afterRename = readFileSync("src/config.js", "utf8");
recordTurn({ answer: "Renamed." });
bumpRetries();
recordTurn({ answer: "Bumped." });
writeFileSync("src/config.js", `${afterRename.trimEnd()}\n\n`);
recordTurn({ answer: "Undone." });
