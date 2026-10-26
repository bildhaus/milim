// Another correct run: the agent lists the failures in its own order
// (sku, cart, dates), so "second" is cart and "first" is sku.
import { recordTurn } from "../../lib/trajectory.mjs";
import { fixes, listing } from "./fixes.mjs";

recordTurn({ answer: listing(["sku", "cart", "dates"]) });
fixes.cart();
recordTurn({ answer: "Fixed the cart rounding." });
fixes.sku();
recordTurn({ answer: "Fixed the SKU comparison; the ISO week failure remains." });
