// Wrong: answers both questions from the same process but never stops it.
import { spawn } from "node:child_process";
import { recordTurn } from "../../lib/trajectory.mjs";

// Detached so it outlives this script, like a server the agent forgot.
const child = spawn(process.execPath, ["server.mjs"], { stdio: ["ignore", "pipe", "ignore"], detached: true });
const url = await new Promise((resolve) => {
  let output = "";
  child.stdout.on("data", (chunk) => {
    output += chunk;
    const match = /listening on (http:\/\/\S+)/.exec(output);
    if (match) resolve(match[1]);
  });
});
child.stdout.destroy();
child.unref();
const get = async (path) => (await fetch(`${url}${path}`)).json();
const health = await get("/health");
recordTurn({ answer: `Boot id ${health.boot_id}.` });
let last;
for (let index = 0; index < 3; index += 1) last = await get("/counter");
recordTurn({ answer: `Last value ${last.value}, boot id ${last.boot_id}.` });
recordTurn({ answer: "Stopped." });
