// Another correct fix: destructure the fee with a default, and build the
// statement line in two steps.
import { applyRefund, describeRefund, totalsWithRefund } from "./edits.mjs";

applyRefund(`{
      const { amount, fee = 0 } = event;
      return balance + amount - fee;
    }`);
describeRefund(`{
      const line = \`refund \${cents(event.amount)}\`;
      return event.fee === undefined || event.fee === 0 ? line : \`\${line} (fee \${cents(event.fee)})\`;
    }`);
totalsWithRefund();
