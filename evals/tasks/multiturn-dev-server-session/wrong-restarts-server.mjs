// Wrong: the server from turn 1 is gone by turn 2 (for example, killed when
// the turn's run ended), so turn 2 starts a new one and reports its boot id.
import { recordTurn } from "../../lib/trajectory.mjs";
import { startServer } from "./session.mjs";

const first = await startServer();
const health = await first.get("/health");
await first.stop();
recordTurn({ answer: `Boot id ${health.boot_id}.` });
const second = await startServer();
let last;
for (let index = 0; index < 3; index += 1) last = await second.get("/counter");
recordTurn({ answer: `Last value ${last.value}, boot id ${last.boot_id}.` });
await second.stop();
recordTurn({ answer: "Stopped." });
