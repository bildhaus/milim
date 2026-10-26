// Reference: a-m plus a dual-mode router, then the rest and an async-only router.
import { recordTurn } from "../../lib/trajectory.mjs";
import { convert, FIRST, leftAnswer, router, SECOND } from "./passes.mjs";

convert(FIRST);
router("both");
recordTurn({ answer: leftAnswer });
convert(SECOND);
router("async");
recordTurn({ answer: "All twelve handlers are async now and the router only awaits them." });
