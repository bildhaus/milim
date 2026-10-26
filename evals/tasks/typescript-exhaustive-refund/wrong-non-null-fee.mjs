// Wrong: silences "possibly undefined" with a non-null assertion, so a
// refund without a fee yields NaN.
import { applyRefund, describeRefund, totalsWithRefund } from "./edits.mjs";

applyRefund("return balance + event.amount - event.fee!;");
describeRefund("return `refund ${cents(event.amount)} (fee ${cents(event.fee ?? 0)})`;");
totalsWithRefund();
