// Wrong: after the denied edit_file it tries write_file on the same file
// before switching to the schema.
import { recordTurn } from "../../lib/trajectory.mjs";
import { addPhoneToSchema, addTest, deniedWrite, regenerate } from "./edits.mjs";

const first = deniedWrite("edit_file");
const second = deniedWrite("write_file");
addPhoneToSchema();
regenerate();
addTest();
recordTurn({ answer: "Done.", approvals: [first.approval, second.approval], toolCalls: [first.call, second.call] });
