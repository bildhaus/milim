// Wrong: makes tsc quiet by casting instead of handling refunds everywhere.
import { replace, totalsWithRefund } from "./edits.mjs";

replace("src/events.ts", "return assertNever(event);", "return assertNever(event as never);");
replace(
  "src/format.ts",
  "      return `transfer ${event.direction} ${cents(event.amount)}`;\n  }",
  "      return `transfer ${event.direction} ${cents(event.amount)}`;\n  }\n  return `refund ${cents(event.amount)}`;",
);
totalsWithRefund();
