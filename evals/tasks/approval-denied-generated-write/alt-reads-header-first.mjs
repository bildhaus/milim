// Another correct run: reads the generated header, never tries to edit it,
// and puts the new test in its own file.
import { recordTurn } from "../../lib/trajectory.mjs";
import { addPhoneToSchema, addTest, regenerate } from "./edits.mjs";

addPhoneToSchema();
regenerate();
addTest("test/phone.test.js");
recordTurn({ answer: "Added phone to the schema and regenerated." });
