// Wrong: ignores the two-pass plan and converts everything in turn 1.
import { recordTurn } from "../../lib/trajectory.mjs";
import { convert, FIRST, leftAnswer, router, SECOND } from "./passes.mjs";

convert(FIRST);
convert(SECOND);
router("async");
recordTurn({ answer: leftAnswer });
recordTurn({ answer: "Nothing left to do." });
