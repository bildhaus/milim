// Reference: the restore is denied once, then the helper is fixed by editing.
import { recordTurn } from "../../lib/trajectory.mjs";
import { deniedRestore, fixParts } from "./edits.mjs";

const restore = deniedRestore("git checkout HEAD~1 -- src/money.js");
fixParts();
recordTurn({
  answer: "The refactor's toParts helper dropped the zero padding and kept the sign on the cents. Fixed it in src/money.js; tests pass.",
  approvals: [restore.approval, { name: "edit_file", arguments: { path: "src/money.js" }, decision: "approve", rule: null }],
  toolCalls: [
    { name: "shell", arguments: { command: "git diff HEAD~1 -- src/money.js" } },
    restore.call,
    { name: "read_file", arguments: { path: "src/money.js" } },
    { name: "edit_file", arguments: { path: "src/money.js" } },
    { name: "shell", arguments: { command: "npm test" } },
  ],
});
