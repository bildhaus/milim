// Queries the service correctly but never stops it.
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";

const child = spawn(process.execPath, ["server.mjs"], { stdio: ["ignore", "pipe", "ignore"], detached: true });
const url = await new Promise((resolve) => {
  let output = "";
  child.stdout.on("data", (chunk) => {
    output += chunk;
    const match = /listening on (http:\/\/\S+)/.exec(output);
    if (match) resolve(match[1]);
  });
});
const report = await (await fetch(`${url}/stats/low-stock?warehouse=north`)).json();
writeFileSync("ANSWER.json", `${JSON.stringify({ count: report.count, request_id: report.request_id })}\n`);
child.stdout.destroy();
child.unref();
