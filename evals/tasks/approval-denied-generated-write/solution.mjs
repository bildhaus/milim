// Reference: the direct edit is denied once; then schema, generator, test.
import { recordTurn } from "../../lib/trajectory.mjs";
import { addPhoneToSchema, addTest, deniedWrite, regenerate } from "./edits.mjs";

const denied = deniedWrite("edit_file");
addPhoneToSchema();
regenerate();
addTest();
recordTurn({
  answer: "The validator is generated; added phone to schema/user.json, ran npm run generate, and added a test.",
  approvals: [denied.approval, { name: "edit_file", arguments: { path: "schema/user.json" }, decision: "approve", rule: null }],
  toolCalls: [denied.call, { name: "edit_file", arguments: { path: "schema/user.json" } }, { name: "shell", arguments: { command: "npm run generate && npm test" } }],
});
