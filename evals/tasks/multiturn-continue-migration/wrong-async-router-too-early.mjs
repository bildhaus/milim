// Wrong: switches the router to async-only in turn 1, breaking the six
// callback handlers that are still waiting for the second pass.
import { recordTurn } from "../../lib/trajectory.mjs";
import { convert, FIRST, leftAnswer, router, SECOND } from "./passes.mjs";

convert(FIRST);
router("async");
recordTurn({ answer: leftAnswer });
convert(SECOND);
recordTurn({ answer: "Done." });
