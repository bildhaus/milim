// Shared by the solution scripts: start the dev server, wait for its URL,
// and query it.
import { spawn } from "node:child_process";

export async function startServer() {
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
    child.on("exit", (code) => reject(new Error(`server exited ${code}: ${output}`)));
  });
  const get = async (path) => (await fetch(`${url}${path}`)).json();
  const stop = async () => {
    const exited = new Promise((resolve) => child.once("exit", resolve));
    child.kill("SIGTERM");
    await exited;
  };
  return { get, stop };
}
