import { spawnSync } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const tmpName = `.tmp-tests-${process.pid}-${Date.now()}`;
const tmp = join(root, tmpName);
const tests = process.argv.slice(2);

if (!tests.length) {
  throw new Error("Usage: node tests/run-ts-test.mjs tests/foo.test.ts [...]");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with exit ${result.status}`);
  }
}

try {
  mkdirSync(tmp, { recursive: true });
  run(
    "npm",
    [
      "exec",
      "--",
      "tsc",
      "--target",
      "ES2022",
      "--module",
      "ES2022",
      "--moduleResolution",
      "bundler",
      "--skipLibCheck",
      "--jsx",
      "react-jsx",
      "--outDir",
      tmpName,
      "--noEmit",
      "false",
      ...tests,
    ],
    { shell: process.platform === "win32" },
  );

  for (const test of tests) {
    const compiled = join(tmp, test.replace(/\.[cm]?tsx?$/, ".js"));
    const moduleUrl = pathToFileURL(compiled).href;
    run(process.execPath, [
      "--input-type=module",
      "--eval",
      `await import(${JSON.stringify(moduleUrl)}); process.exit(0);`,
    ]);
  }
} finally {
  rmSync(tmp, { recursive: true, force: true });
}
