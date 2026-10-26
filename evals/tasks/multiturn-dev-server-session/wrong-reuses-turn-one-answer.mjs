// Wrong: turn 2 repeats the boot id from turn 1 without querying the
// server again, and guesses the counter.
import { recordTurn } from "../../lib/trajectory.mjs";
import { startServer } from "./session.mjs";

const server = await startServer();
const health = await server.get("/health");
recordTurn({ answer: `Boot id ${health.boot_id}.` });
recordTurn({ answer: `The counter is at 3 and the boot id is still ${health.boot_id}.` });
await server.stop();
recordTurn({ answer: "Stopped." });
