// Wrong: fixes every failure in turn 2 instead of only the second one.
import { recordTurn } from "../../lib/trajectory.mjs";
import { fixes, listing } from "./fixes.mjs";

recordTurn({ answer: listing(["cart", "dates", "sku"]) });
fixes.dates();
fixes.cart();
fixes.sku();
recordTurn({ answer: "Fixed all three." });
recordTurn({ answer: "Already done." });
