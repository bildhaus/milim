// Reference solution: start the service, wait for its URL, query it, write
// the answer, and stop it with SIGTERM.
import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";

const child = spawn(process.execPath, ["server.mjs"], { stdio: ["ignore", "pipe", "inherit"] });
const url = await new Promise((resolve, reject) => {
  let output = "";
  const timer = setTimeout(() => reject(new Error(`no URL after 20s: ${output}`)), 20_000);
  child.stdout.on("data", (chunk) => {
    output += chunk;
    const match = /listening on (http:\/\/\S+)/.exec(output);
    if (match) {
      clearTimeout(timer);
      resolve(match[1]);
    }
  });
  child.on("exit", (code) => reject(new Error(`service exited ${code}: ${output}`)));
});
const report = await (await fetch(`${url}/stats/low-stock?warehouse=north`)).json();
writeFileSync("ANSWER.json", `${JSON.stringify({ count: report.count, request_id: report.request_id }, null, 2)}\n`);
const exited = new Promise((resolve) => child.once("exit", resolve));
child.kill("SIGTERM");
await exited;
