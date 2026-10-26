// Wrong: lists sku, cart, dates but then counts "second" and "first" in the
// test runner's order (dates, then cart) instead of its own list.
import { recordTurn } from "../../lib/trajectory.mjs";
import { fixes, listing } from "./fixes.mjs";

recordTurn({ answer: listing(["sku", "cart", "dates"]) });
fixes.dates();
recordTurn({ answer: "Fixed isoWeek." });
fixes.cart();
recordTurn({ answer: "Fixed cartTotal." });
