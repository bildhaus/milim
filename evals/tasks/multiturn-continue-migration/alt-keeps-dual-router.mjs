// Another correct run: finishes the handlers but leaves the router's
// callback support in place.
import { recordTurn } from "../../lib/trajectory.mjs";
import { convert, FIRST, leftAnswer, router, SECOND } from "./passes.mjs";

convert(FIRST);
router("both");
recordTurn({ answer: leftAnswer });
convert(SECOND);
recordTurn({ answer: "Converted the remaining six handlers." });
