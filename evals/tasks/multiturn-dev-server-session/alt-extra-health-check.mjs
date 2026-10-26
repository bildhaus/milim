// Another correct run: turn 2 first confirms the server with /health, then
// calls /counter three times and reports the boot id in a different format.
import { recordTurn } from "../../lib/trajectory.mjs";
import { startServer } from "./session.mjs";

const server = await startServer();
const health = await server.get("/health");
recordTurn({ answer: `Boot id: ${health.boot_id}` });
const again = await server.get("/health");
const values = [];
for (let index = 0; index < 3; index += 1) values.push((await server.get("/counter")).value);
recordTurn({ answer: `Counter values: ${values.join(", ")} (last: ${values.at(-1)}).\nboot_id=${again.boot_id}, same process as before.` });
await server.stop();
recordTurn({ answer: "Done, the server is stopped." });
