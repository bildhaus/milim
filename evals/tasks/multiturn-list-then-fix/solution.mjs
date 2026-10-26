// Reference: list in test-run order (cart, dates, sku), fix dates, then cart.
import { recordTurn } from "../../lib/trajectory.mjs";
import { fixes, listing } from "./fixes.mjs";

recordTurn({ answer: listing(["cart", "dates", "sku"]) });
fixes.dates();
recordTurn({ answer: "Fixed isoWeek; the cart and SKU failures remain." });
fixes.cart();
recordTurn({ answer: "Fixed cartTotal rounding; the SKU failure remains." });
