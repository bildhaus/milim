// Wrong: updates the schema but never runs the generator.
import { recordTurn } from "../../lib/trajectory.mjs";
import { addPhoneToSchema, addTest } from "./edits.mjs";

addPhoneToSchema();
addTest();
recordTurn({ answer: "Added phone to the schema." });
