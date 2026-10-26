// Wrong: `git checkout -- src/config.js` restores the committed file, which
// also reverts turn 1's rename.
import { execFileSync } from "node:child_process";
import { recordTurn } from "../../lib/trajectory.mjs";
import { bumpRetries, renameEverywhere } from "./turns.mjs";

renameEverywhere();
recordTurn({ answer: "Renamed." });
bumpRetries();
recordTurn({ answer: "Bumped." });
execFileSync("git", ["checkout", "--", "src/config.js"]);
recordTurn({ answer: "Restored src/config.js." });
