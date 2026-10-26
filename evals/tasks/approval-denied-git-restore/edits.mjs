// Shared by the solution scripts.
import { readFileSync, writeFileSync } from "node:fs";

export function replace(path, from, to) {
  const source = readFileSync(path, "utf8");
  if (!source.includes(from)) throw new Error(`${path}: expected text not found`);
  writeFileSync(path, source.replace(from, () => to));
}

export const fixParts = () =>
  replace("src/money.js", "    cents: String(cents % 100),", '    cents: String(Math.abs(cents) % 100).padStart(2, "0"),');

/** A simulated denied restore: the approval and the tool call it answered. */
export const deniedRestore = (command) => ({
  approval: { name: "shell", arguments: { command }, decision: "deny", rule: 0 },
  call: { name: "shell", arguments: { command }, is_error: true, denied: true, error: "Tool call denied by user" },
});
