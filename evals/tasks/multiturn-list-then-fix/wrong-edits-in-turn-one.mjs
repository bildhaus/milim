// Wrong: "fixes" the second failure while it was asked only to list them.
import { recordTurn } from "../../lib/trajectory.mjs";
import { fixes, listing } from "./fixes.mjs";

fixes.dates();
recordTurn({ answer: listing(["cart", "dates", "sku"]) });
recordTurn({ answer: "isoWeek is already fixed." });
fixes.cart();
recordTurn({ answer: "Fixed cartTotal." });
