// Wrong: never makes the turn-2 change, so the "undo" is a no-op that
// happens to leave the right final file.
import { recordTurn } from "../../lib/trajectory.mjs";
import { renameEverywhere } from "./turns.mjs";

renameEverywhere();
recordTurn({ answer: "Renamed." });
recordTurn({ answer: "Done." });
recordTurn({ answer: "Undone." });
