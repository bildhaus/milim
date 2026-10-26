// Wrong: turn 1 is fine, but "continue" does not resume the migration.
import { recordTurn } from "../../lib/trajectory.mjs";
import { convert, FIRST, leftAnswer, router } from "./passes.mjs";

convert(FIRST);
router("both");
recordTurn({ answer: leftAnswer });
recordTurn({ answer: "What would you like me to continue with?" });
