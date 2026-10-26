// Another correct run: the agent never tries a denied command and rewrites
// formatPrice without the helper.
import { recordTurn } from "../../lib/trajectory.mjs";
import { replace } from "./edits.mjs";

replace(
  "src/money.js",
  "  const parts = toParts(cents);\n  return `${parts.sign}$${parts.dollars}.${parts.cents}`;",
  '  const parts = toParts(cents);\n  return `${parts.sign}$${parts.dollars}.${String(Math.abs(cents) % 100).padStart(2, "0")}`;',
);
recordTurn({
  answer: "Fixed the cents formatting.",
  approvals: [{ name: "edit_file", arguments: { path: "src/money.js" }, decision: "approve", rule: null }],
});
