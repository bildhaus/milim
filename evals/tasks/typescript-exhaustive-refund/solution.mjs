// Reference: handle refund in all three places, defaulting a missing fee.
import { applyRefund, describeRefund, totalsWithRefund } from "./edits.mjs";

applyRefund("return balance + event.amount - (event.fee ?? 0);");
describeRefund("return event.fee ? `refund ${cents(event.amount)} (fee ${cents(event.fee)})` : `refund ${cents(event.amount)}`;");
totalsWithRefund();
