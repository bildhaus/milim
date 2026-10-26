// Reference: one server process for all three turns.
import { recordTurn } from "../../lib/trajectory.mjs";
import { startServer } from "./session.mjs";

const server = await startServer();
const health = await server.get("/health");
recordTurn({ answer: `The dev server is running; GET /health reports boot id \`${health.boot_id}\`.` });
let last;
for (let index = 0; index < 3; index += 1) last = await server.get("/counter");
recordTurn({ answer: `The last /counter call returned ${last.value}. The server still reports boot id ${last.boot_id}.` });
await server.stop();
recordTurn({ answer: "Stopped the dev server." });
